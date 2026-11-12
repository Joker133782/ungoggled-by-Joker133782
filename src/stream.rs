//! Relay the live goggles feed over the Wi-Fi access point for spectators,
//! as MPEG-TS over plain HTTP: real H.264 (unlike the JPEG browser preview),
//! playable in a phone browser via `mpegts.js` (MSE), with much lower
//! latency than HLS because there is no multi-second segment buffering.
//! Verified end to end in a real browser against this exact `mpegtsmux`
//! pipeline; not run against the live goggles feed on real hardware.
use crate::sync::LockExt;
use anyhow::{Context, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

/// Spectator-side chunks never need to be huge; keep enough queued that a
/// brief stall doesn't drop a keyframe interval's worth of data.
const CHANNEL_CAPACITY: usize = 256;
/// How many NAL units may wait for the relay's stdin. The video loop never
/// waits for the relay: if it falls behind (a software transcode that cannot
/// keep up) the units are dropped instead, and the picture of the spectators
/// resumes at the next keyframe. Otherwise a slow spectator stream would
/// slow the live HDMI picture down with it.
const FEED_QUEUE: usize = 128;
/// After the relay has failed, wait this long before starting another one.
const RESTART_AFTER_FAILURE: Duration = Duration::from_secs(15);

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
    /// Units for the thread that owns the relay's stdin.
    queue: SyncSender<Vec<u8>>,
    /// Dropping units because the queue was full; held until the next SPS so
    /// the stream resumes at a clean start instead of in the middle of a
    /// picture group.
    skipping: bool,
    quality: Quality,
    tx: broadcast::Sender<Vec<u8>>,
    /// The last line the relay printed to its stderr, for the log if it dies.
    last_message: Arc<Mutex<String>>,
}

#[derive(Default)]
struct State {
    relay: Option<Relay>,
    /// No new relay before this time (it failed a moment ago).
    retry_at: Option<Instant>,
}
impl State {
    fn failed(&mut self) {
        if let Some(relay) = self.relay.take() {
            let message = relay.last_message.lock_ok().clone();
            eprintln!(
                "Spectator stream stopped{}",
                if message.is_empty() {
                    String::new()
                } else {
                    format!(": {message}")
                }
            );
        }
        self.retry_at = Some(Instant::now() + RESTART_AFTER_FAILURE);
    }
}

#[derive(Clone, Default)]
pub struct Controller(Arc<Mutex<State>>);
impl Controller {
    /// Make sure a relay at this quality is running, (re)starting it if it
    /// is not running yet or is running at a different quality. One that
    /// failed is not started again for a while: it would fail the same way,
    /// and every start costs the video loop that calls this.
    pub fn ensure(&self, quality: Quality) {
        let mut state = self.0.lock_ok();
        if state.relay.as_ref().is_some_and(|r| r.quality == quality) {
            return;
        }
        state.relay = None;
        if state.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        match spawn(quality) {
            Ok(relay) => {
                state.relay = Some(relay);
                state.retry_at = None;
            }
            Err(e) => {
                eprintln!("Spectator stream: {e:#}");
                state.failed();
            }
        }
    }
    pub fn stop(&self) {
        let mut state = self.0.lock_ok();
        state.relay = None;
        state.retry_at = None;
    }
    /// Hand one NAL from the live goggles stream to the relay, if one is
    /// running. Never waits for the relay.
    pub fn feed(&self, nal: &[u8]) {
        let mut state = self.0.lock_ok();
        let Some(relay) = &mut state.relay else {
            return;
        };
        let is_sps = nal.get(3).map(|b| b & 31) == Some(7);
        if relay.skipping && !is_sps {
            return;
        }
        match relay.queue.try_send(patch_sps_level(nal).into_owned()) {
            Ok(()) => relay.skipping = false,
            Err(TrySendError::Full(_)) => relay.skipping = true,
            Err(TrySendError::Disconnected(_)) => state.failed(),
        }
    }
    /// A receiver for MPEG-TS bytes, if a relay is currently running.
    pub fn subscribe(&self) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.0
            .lock()
            .unwrap()
            .relay
            .as_ref()
            .map(|r| r.tx.subscribe())
    }
    pub fn running(&self) -> bool {
        self.0.lock_ok().relay.is_some()
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
    start(relay_command(quality), quality)
}
fn relay_command(quality: Quality) -> Command {
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
    cmd
}
/// Runs `cmd` as the relay: units go to its stdin, its stdout is broadcast.
fn start(mut cmd: Command, quality: Quality) -> Result<Relay> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start the streaming relay (gst-launch-1.0)")?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let last_message = Arc::new(Mutex::new(String::new()));
    let seen = last_message.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                *seen.lock_ok() = line.chars().take(300).collect();
            }
        }
    });
    // The only place that waits for the relay to take its input. When the
    // relay dies the write fails, the thread ends and the queue disconnects,
    // which `feed` notices.
    let (queue, units) = mpsc::sync_channel::<Vec<u8>>(FEED_QUEUE);
    thread::spawn(move || {
        while let Ok(unit) = units.recv() {
            if stdin.write_all(&unit).is_err() {
                return;
            }
        }
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
        queue,
        skipping: false,
        quality,
        tx,
        last_message,
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

    /// A relay that is `program` with these arguments instead of GStreamer.
    fn fake(program: &str, args: &[&str]) -> Controller {
        let mut cmd = Command::new(program);
        cmd.args(args);
        let c = Controller::default();
        c.0.lock_ok().relay = Some(start(cmd, Quality::High).unwrap());
        c
    }
    fn nal(kind: u8, tag: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0, 0, 1, kind, tag];
        v.resize(len.max(5), tag);
        v
    }

    #[test]
    fn units_reach_the_relay_and_its_output_reaches_subscribers() {
        let c = fake("cat", &[]);
        let mut rx = c.subscribe().expect("relay running");
        let unit = nal(0x65, 9, 40);
        c.feed(&unit);
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < unit.len() && Instant::now() < deadline {
            if let Ok(chunk) = rx.try_recv() {
                got.extend(chunk);
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(got, unit);
    }

    #[test]
    fn a_relay_that_stops_reading_never_holds_up_the_caller() {
        // `sleep` never reads its stdin: the pipe fills, the writer thread
        // blocks, the queue fills. The video loop must not feel any of it.
        let c = fake("sleep", &["30"]);
        let started = Instant::now();
        for i in 0..2000u32 {
            let kind = if i % 50 == 0 { 0x67 } else { 0x41 };
            c.feed(&nal(kind, 1, 100_000));
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "feeding took {:?}",
            started.elapsed()
        );
        assert!(c.running(), "falling behind is not a failure");
    }

    #[test]
    fn after_dropping_units_the_stream_resumes_at_the_next_sps() {
        let c = fake("sleep", &["30"]);
        for _ in 0..2000 {
            c.feed(&nal(0x41, 1, 100_000));
        }
        assert!(c.0.lock_ok().relay.as_ref().unwrap().skipping);
        // Slices are dropped until a unit that starts a picture group arrives.
        c.feed(&nal(0x41, 2, 10));
        assert!(c.0.lock_ok().relay.as_ref().unwrap().skipping);
        c.feed(&nal(0x67, 3, 10));
        // The queue is still full, so the SPS is dropped too and it keeps
        // skipping; what matters is that a slice never ends the skipping.
        assert!(c.running());
    }

    #[test]
    fn a_dead_relay_is_dropped_and_not_restarted_at_once() {
        let c = fake("false", &[]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while c.running() && Instant::now() < deadline {
            c.feed(&nal(0x41, 1, 10));
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!c.running(), "the dead relay was removed");
        let retry_at = c.0.lock_ok().retry_at.expect("a pause is set");
        assert!(retry_at > Instant::now() + RESTART_AFTER_FAILURE / 2);
        // `ensure` honours the pause: it must not try to start gst-launch.
        c.ensure(Quality::High);
        assert!(!c.running());
        // Stopping the stream (the setting turned off) clears the pause.
        c.stop();
        assert!(c.0.lock_ok().retry_at.is_none());
    }

    #[test]
    fn non_sps_nals_are_untouched() {
        let slice = [0, 0, 1, 0x65, 1, 2, 3];
        assert_eq!(&*patch_sps_level(&slice), &slice);
        let short = [0, 0, 1, 0x67, 1, 2];
        assert_eq!(&*patch_sps_level(&short), &short);
    }
}
