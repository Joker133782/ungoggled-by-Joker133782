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
        if s == "event" { Self::Event } else { Self::High }
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
            && relay.stdin.write_all(nal).is_err()
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
    thread::spawn(move || {
        for _ in BufReader::new(stderr).lines().map_while(Result::ok) {}
    });
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
