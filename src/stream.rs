//! Relay the live goggles feed over the Wi-Fi access point for spectators,
//! as MPEG-TS over plain HTTP: real H.264 (unlike the JPEG browser preview),
//! playable in a phone browser via `mpegts.js` (MSE), with much lower
//! latency than HLS because there is no multi-second segment buffering.
//! Verified end to end in a real browser against this exact `mpegtsmux`
//! pipeline; not run against the live goggles feed on real hardware.
use anyhow::{Context, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};
use tokio::sync::broadcast;

/// Spectator-side chunks never need to be huge; keep enough queued that a
/// brief stall doesn't drop a keyframe interval's worth of data.
const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// Relay the original goggles stream unmodified: best quality, but
    /// costs roughly the same bandwidth per viewer as the live HDMI feed.
    High,
    /// Decode and re-encode at a lower resolution/bitrate, trading quality
    /// for letting the Pi's own Wi-Fi carry more simultaneous viewers.
    Event,
}
impl Quality {
    pub fn parse(s: &str) -> Self {
        if s == "event" {
            Self::Event
        } else {
            Self::High
        }
    }
}

struct Relay {
    child: Child,
    stdin: ChildStdin,
    quality: Quality,
    tx: broadcast::Sender<Vec<u8>>,
}

#[derive(Clone, Default)]
pub struct Controller(Arc<Mutex<Option<Relay>>>);
impl Controller {
    /// Make sure a relay at this quality is running, (re)starting it if it
    /// is not running yet or is running at a different quality.
    pub fn ensure(&self, quality: Quality) {
        let mut guard = self.0.lock().unwrap();
        if guard.as_ref().is_some_and(|r| r.quality == quality) {
            return;
        }
        *guard = None;
        match spawn(quality) {
            Ok(relay) => *guard = Some(relay),
            Err(e) => eprintln!("Spectator stream: {e:#}"),
        }
    }
    pub fn stop(&self) {
        *self.0.lock().unwrap() = None;
    }
    /// Feed one NAL from the live goggles stream, if a relay is running.
    pub fn feed(&self, nal: &[u8]) {
        let mut guard = self.0.lock().unwrap();
        if let Some(relay) = &mut *guard
            && relay.stdin.write_all(&patch_sps_level(nal)).is_err()
        {
            *guard = None;
        }
    }
    /// A receiver for MPEG-TS bytes, if a relay is currently running.
    pub fn subscribe(&self) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.0.lock().unwrap().as_ref().map(|r| r.tx.subscribe())
    }
    pub fn running(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

/// Goggles declare H.264 level 5.2 even for plain 1920x1080p30 (see the same
/// note on the live HDMI decode in `video.rs`). That path fixes it with
/// GStreamer's `capssetter`, which only rewrites negotiation metadata
/// between elements — fine when the next element is `v4l2h264dec`, but these
/// bytes instead end up muxed verbatim into the MPEG-TS sent to a phone's own
/// browser, which parses the real SPS and routinely refuses level 5.2
/// outright. So rewrite the one byte for real. `level_idc` is byte-aligned
/// right after the NAL header, before any Exp-Golomb-coded field, so no
/// bitstream parsing is needed. Level 4.2 has ample headroom for 1080p30 and
/// is broadly supported by phone decoders, unlike 5.1/5.2.
fn patch_sps_level(nal: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if nal.len() > 6 && nal[3] & 31 == 7 {
        let mut v = nal.to_vec();
        v[6] = 42;
        std::borrow::Cow::Owned(v)
    } else {
        std::borrow::Cow::Borrowed(nal)
    }
}

fn spawn(quality: Quality) -> Result<Relay> {
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.args(["fdsrc", "fd=0", "!", "h264parse", "config-interval=-1", "!"]);
    match quality {
        Quality::High => {}
        Quality::Event => {
            // Software decode here, deliberately: the live display pipeline
            // already owns the Pi's one hardware decoder, which typically
            // cannot serve two concurrent sessions.
            cmd.args([
                "avdec_h264",
                "!",
                "videoconvert",
                "!",
                "videoscale",
                "!",
                "videorate",
                "!",
                "video/x-raw,width=854,height=480,framerate=15/1",
                "!",
                "x264enc",
                "tune=zerolatency",
                "speed-preset=ultrafast",
                "bitrate=1500",
                "key-int-max=30",
                "!",
                // x264enc has no "profile" property on current GStreamer
                // (it only appears as a caps field); without constraining
                // it here, x264enc's default profile choice is not
                // guaranteed and can land on something exotic (High 4:4:4
                // Predictive was observed) that most decoders, including
                // Firefox's, reject outright. A capsfilter on the field is
                // the correct way to pin it; "main" is broadly supported.
                "video/x-h264,profile=main",
                "!",
                "h264parse",
                "!",
            ]);
        }
    }
    cmd.args(["mpegtsmux", "!", "fdsink", "fd=1"]);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start the streaming relay (gst-launch-1.0)")?;
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    thread::spawn(
        move || {
            for _ in BufReader::new(stderr).lines().map_while(Result::ok) {}
        },
    );
    let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
    let sender = tx.clone();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            // No subscribers is not an error; just nothing to deliver yet.
            let _ = sender.send(buf[..n].to_vec());
        }
    });
    Ok(Relay {
        child,
        stdin,
        quality,
        tx,
    })
}
impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sps_level_is_patched_down() {
        // profile_idc=0x64 (High), constraint=0x00, level_idc=0x34 (5.2).
        let sps = [0, 0, 1, 0x67, 0x64, 0, 0x34, 0xac];
        let patched = patch_sps_level(&sps);
        assert_eq!(patched[6], 42);
        assert_eq!(&patched[..6], &sps[..6]);
        assert_eq!(&patched[7..], &sps[7..]);
    }

    #[test]
    fn non_sps_nals_are_untouched() {
        let slice = [0, 0, 1, 0x65, 1, 2, 3];
        assert_eq!(&*patch_sps_level(&slice), &slice);
        let short = [0, 0, 1, 0x67, 1, 2];
        assert_eq!(&*patch_sps_level(&short), &short);
    }
}
