//! Record the live goggles feed to local storage (the SD card), independent
//! of HDMI output and the USB-drive video browser. Four ways to capture,
//! all writing the same raw Annex-B H.264 the live decoder already
//! receives, with no re-encoding, matching how fallback videos are stored:
//! an explicit start/stop recording, an always-on ~30 s ring buffer saved
//! on demand or automatically when the feed drops (like the goggles' own
//! instant-replay), and auto-recording a whole flight (buffered in RAM,
//! under the runtime tmpfs, while the feed is live; written to the SD card,
//! keeping the 5 most recent, as soon as the feed drops).
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Recording stops (and a new one refuses to start) below this much free
/// space, so the filesystem never actually fills up.
pub const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BUFFER_DURATION: Duration = Duration::from_secs(30);
/// Bound the ring buffer in bytes too, in case frames arrive far faster than
/// expected; generous for any realistic goggles bitrate over 30 seconds.
const BUFFER_MAX_BYTES: usize = 64 * 1024 * 1024;
/// How many auto-recorded flights to keep on the SD card; the oldest is
/// deleted once a new one is saved beyond this count.
const MAX_FLIGHTS: usize = 5;
/// How many crash-buffer clips to keep; a much smaller rotation than
/// flights since these are just a safety net, not the whole recording.
const MAX_CRASH_CLIPS: usize = 3;

struct Active {
    file: File,
    name: String,
    started: Instant,
    bytes: u64,
}
/// A flight currently being buffered in RAM (tmpfs), not yet on the SD card.
struct AutoRecording {
    file: File,
    tmp_path: PathBuf,
}
#[derive(Default)]
struct Buffer {
    entries: VecDeque<(Instant, Vec<u8>)>,
    bytes: usize,
}
impl Buffer {
    fn push(&mut self, nal: &[u8]) {
        self.entries.push_back((Instant::now(), nal.to_vec()));
        self.bytes += nal.len();
        let cutoff = Instant::now() - BUFFER_DURATION;
        while self
            .entries
            .front()
            .is_some_and(|(t, _)| *t < cutoff || self.bytes > BUFFER_MAX_BYTES)
        {
            let (_, old) = self.entries.pop_front().unwrap();
            self.bytes -= old.len();
        }
    }
    /// Bytes from the first SPS onward, so playback can start cleanly; `None`
    /// if nothing has arrived yet or no SPS is within the buffered window.
    fn snapshot(&self) -> Option<Vec<u8>> {
        let start = self
            .entries
            .iter()
            .position(|(_, n)| n.get(3).map(|b| b & 31) == Some(7))?;
        let mut out = Vec::new();
        for (_, n) in self.entries.iter().skip(start) {
            out.extend_from_slice(n);
        }
        Some(out)
    }
}

#[derive(Default)]
struct Inner {
    active: Option<Active>,
    buffer: Buffer,
    auto: Option<AutoRecording>,
    /// Whether live input was flowing as of the last `crash_tick` call, to
    /// detect the drop that should trigger an auto-save.
    crash_was_live: bool,
}

#[derive(Clone)]
pub struct Controller {
    inner: Arc<Mutex<Inner>>,
    /// Under the runtime directory (tmpfs): gone on every reboot or power
    /// loss, so cached thumbnails never accumulate on the SD card.
    thumb_dir: PathBuf,
}
impl Controller {
    pub fn new(thumb_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&thumb_dir);
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            thumb_dir,
        }
    }
    /// Start a new continuous recording. Fails without creating a file if
    /// there is already too little free space.
    pub fn start(&self, dir: &Path) -> anyhow::Result<Value> {
        fs::create_dir_all(dir)?;
        if free_bytes(dir).is_some_and(|f| f < MIN_FREE_BYTES) {
            anyhow::bail!("Less than 2 GiB free; free up space before recording");
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let name = format!("recording-{stamp}.h264");
        let file = File::create(dir.join(&name))?;
        let mut inner = self.inner.lock().unwrap();
        inner.active = Some(Active {
            file,
            name: name.clone(),
            started: Instant::now(),
            bytes: 0,
        });
        Ok(json!({"recording":true,"name":name}))
    }
    /// Stop the active recording, if any, and report how big it ended up.
    pub fn stop(&self) -> Option<Value> {
        let active = self.inner.lock().unwrap().active.take()?;
        Some(json!({
            "name": active.name,
            "bytes": active.bytes,
            "seconds": active.started.elapsed().as_secs(),
        }))
    }
    pub fn status(&self) -> Value {
        match &self.inner.lock().unwrap().active {
            Some(a) => json!({
                "recording": true,
                "name": a.name,
                "bytes": a.bytes,
                "seconds": a.started.elapsed().as_secs(),
            }),
            None => json!({"recording": false}),
        }
    }
    /// Feed one NAL unit from the live goggles stream: always into the
    /// rolling buffer, and into the active recording file if there is one.
    pub fn feed(&self, nal: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.push(nal);
        if let Some(active) = &mut inner.active
            && active.file.write_all(nal).is_ok()
        {
            active.bytes += nal.len() as u64;
        }
        if let Some(auto) = &mut inner.auto {
            let _ = auto.file.write_all(nal);
        }
    }
    /// Keep auto-recording in sync with whether live input is currently
    /// flowing, when the `auto_record` setting is on: starts buffering a
    /// flight in `tmp_dir` (tmpfs) when input appears, and saves it to
    /// `sd_dir` (pruned to the most recent `MAX_FLIGHTS`) as soon as input
    /// stops. Cheap to call every tick either way.
    pub fn auto_tick(&self, tmp_dir: &Path, sd_dir: &Path, enabled: bool, input_live: bool) {
        let mut inner = self.inner.lock().unwrap();
        if !enabled {
            if let Some(auto) = inner.auto.take() {
                finalize_flight(auto, sd_dir);
            }
            return;
        }
        match (inner.auto.is_some(), input_live) {
            (false, true) => {
                if let Ok(auto) = start_auto_recording(tmp_dir) {
                    inner.auto = Some(auto);
                }
            }
            (true, false) => {
                let auto = inner.auto.take().unwrap();
                finalize_flight(auto, sd_dir);
            }
            _ => {}
        }
    }
    /// Keep the crash buffer in sync with live input, when the
    /// `crash_buffer` setting is on. The ~30 s ring buffer itself always
    /// runs regardless of this setting (it also backs the manual "save last
    /// 30 seconds" button); this only decides whether a drop in live input
    /// auto-saves it, like the goggles' own instant-replay, keeping the
    /// most recent `MAX_CRASH_CLIPS`. Cheap to call every tick either way.
    pub fn crash_tick(&self, dir: &Path, enabled: bool, input_live: bool) {
        let snapshot = {
            let mut inner = self.inner.lock().unwrap();
            let just_dropped = enabled && inner.crash_was_live && !input_live;
            inner.crash_was_live = input_live;
            just_dropped.then(|| inner.buffer.snapshot()).flatten()
        };
        if let Some(bytes) = snapshot {
            let _ = save_crash_clip(dir, &bytes);
        }
    }
    /// Check free space while a recording is active; stop it and return a
    /// human-readable reason if it just dropped below the minimum. A no-op,
    /// cheap to call every tick, while nothing is recording.
    pub fn enforce_space(&self, dir: &Path) -> Option<String> {
        self.inner.lock().unwrap().active.as_ref()?;
        if free_bytes(dir).is_some_and(|f| f >= MIN_FREE_BYTES) {
            return None;
        }
        let stopped = self.stop()?;
        Some(format!(
            "Recording stopped: less than 2 GiB free ({})",
            stopped["name"].as_str().unwrap_or("recording")
        ))
    }
    /// Save whatever is currently in the ~30 s ring buffer as its own file.
    pub fn save_buffer(&self, dir: &Path) -> anyhow::Result<Value> {
        fs::create_dir_all(dir)?;
        let bytes = {
            let inner = self.inner.lock().unwrap();
            inner
                .buffer
                .snapshot()
                .ok_or_else(|| anyhow::anyhow!("Nothing recent to save yet"))?
        };
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let name = format!("clip-{stamp}.h264");
        crate::settings::atomic_write(&dir.join(&name), &bytes)?;
        Ok(json!({"name":name,"bytes":bytes.len()}))
    }
    /// List recordings, clips and saved flights in `dir`, generating any
    /// thumbnail missing from the cache along the way (so the first list
    /// after a new file appears is slower than later ones).
    pub fn list(&self, dir: &Path) -> Vec<Value> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !valid_name(&name) {
                    return None;
                }
                let meta = e.metadata().ok()?;
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                let thumbnail = crate::storage::ensure_thumbnail(
                    &e.path(),
                    &self.thumb_dir.join(format!("{name}.jpg")),
                );
                Some(json!({"name":name,"size":meta.len(),"modified":modified,"thumbnail":thumbnail}))
            })
            .collect();
        entries.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
        entries
    }
    /// The cached thumbnail for a recording, if one has been generated.
    pub fn thumbnail(&self, name: &str) -> Option<PathBuf> {
        let path = valid_name(name).then(|| self.thumb_dir.join(format!("{name}.jpg")))?;
        path.exists().then_some(path)
    }
}

/// Create a new tmpfs scratch file for a flight about to start.
fn start_auto_recording(tmp_dir: &Path) -> std::io::Result<AutoRecording> {
    fs::create_dir_all(tmp_dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = tmp_dir.join(format!("flight-{stamp}.h264"));
    let file = File::create(&tmp_path)?;
    Ok(AutoRecording { file, tmp_path })
}
/// Copy a finished flight from the tmpfs scratch file to the SD card and
/// prune older flights beyond `MAX_FLIGHTS`; always removes the scratch
/// file, whether or not the copy succeeded.
fn finalize_flight(auto: AutoRecording, sd_dir: &Path) {
    drop(auto.file);
    if fs::create_dir_all(sd_dir).is_ok() {
        let name = auto
            .tmp_path
            .file_name()
            .map(|n| sd_dir.join(n))
            .filter(|_| fs::metadata(&auto.tmp_path).is_ok_and(|m| m.len() > 0));
        if let Some(dest) = name
            && fs::copy(&auto.tmp_path, &dest).is_ok()
        {
            prune_flights(sd_dir);
        }
    }
    let _ = fs::remove_file(&auto.tmp_path);
}
fn prune_flights(dir: &Path) {
    prune_by_prefix(dir, "flight-", MAX_FLIGHTS);
}

/// Save a crash-buffer snapshot as its own file and prune older ones beyond
/// `MAX_CRASH_CLIPS`.
fn save_crash_clip(dir: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let name = format!("crash-{stamp}.h264");
    crate::settings::atomic_write(&dir.join(&name), bytes)?;
    prune_by_prefix(dir, "crash-", MAX_CRASH_CLIPS);
    Ok(())
}

/// Delete the oldest files in `dir` whose name starts with `prefix`, beyond
/// the most recent `keep`. Nanosecond timestamps in the name sort
/// chronologically as strings.
fn prune_by_prefix(dir: &Path, prefix: &str, keep: usize) {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(prefix).then_some(name)
        })
        .collect();
    names.sort();
    for old in names.iter().rev().skip(keep) {
        let _ = fs::remove_file(dir.join(old));
    }
}

fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

pub fn valid_name(name: &str) -> bool {
    (name.starts_with("recording-")
        || name.starts_with("clip-")
        || name.starts_with("flight-")
        || name.starts_with("crash-"))
        && name.ends_with(".h264")
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}
pub fn remove(dir: &Path, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_name(name), "Invalid recording name");
    Ok(fs::remove_file(dir.join(name))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ungoggled-recording-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }
    fn test_controller(label: &str) -> (Controller, PathBuf) {
        let thumbs = scratch(&format!("{label}-thumbs"));
        (Controller::new(thumbs), scratch(label))
    }

    #[test]
    fn rejects_unsafe_or_unrecognized_names() {
        assert!(valid_name("recording-123.h264"));
        assert!(valid_name("clip-456.h264"));
        assert!(valid_name("flight-789.h264"));
        assert!(valid_name("crash-321.h264"));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("recording-123.mp4"));
        assert!(!valid_name("other-123.h264"));
    }

    #[test]
    fn records_and_stops_on_demand() {
        let (c, dir) = test_controller("start-stop");
        assert!(!c.status()["recording"].as_bool().unwrap());
        c.start(&dir).unwrap();
        assert!(c.status()["recording"].as_bool().unwrap());
        c.feed(&[0, 0, 1, 0x67, 1, 2, 3]);
        c.feed(&[0, 0, 1, 0x65, 4, 5]);
        let summary = c.stop().unwrap();
        assert_eq!(summary["bytes"], 13);
        assert!(!c.status()["recording"].as_bool().unwrap());
        let name = summary["name"].as_str().unwrap();
        assert_eq!(
            fs::read(dir.join(name)).unwrap(),
            [0, 0, 1, 0x67, 1, 2, 3, 0, 0, 1, 0x65, 4, 5]
        );
        let listed = c.list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], name);
        remove(&dir, name).unwrap();
        assert!(c.list(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn buffer_snapshot_starts_at_the_first_sps() {
        let (c, dir) = test_controller("buffer");
        // A stray slice before any SPS must not appear in the saved clip.
        c.feed(&[0, 0, 1, 0x65, 0xaa]);
        c.feed(&[0, 0, 1, 0x67, 1, 2]);
        c.feed(&[0, 0, 1, 0x68, 3]);
        c.feed(&[0, 0, 1, 0x65, 4]);
        let saved = c.save_buffer(&dir).unwrap();
        let name = saved["name"].as_str().unwrap().to_owned();
        let bytes = fs::read(dir.join(&name)).unwrap();
        assert_eq!(
            bytes,
            [0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_buffer_fails_cleanly_when_nothing_was_fed() {
        let (c, dir) = test_controller("empty-buffer");
        assert!(c.save_buffer(&dir).is_err());
    }

    #[test]
    fn auto_record_saves_on_feed_loss_and_does_nothing_when_disabled() {
        let (c, sd) = test_controller("auto");
        let tmp = scratch("auto-tmp");
        // Disabled: never starts, even with live input.
        c.auto_tick(&tmp, &sd, false, true);
        c.feed(&[0, 0, 1, 0x67, 1]);
        assert!(c.list(&sd).is_empty());
        // Enabled + live: starts buffering in the tmp dir, not the SD dir.
        c.auto_tick(&tmp, &sd, true, true);
        c.feed(&[0, 0, 1, 0x67, 1, 2]);
        c.feed(&[0, 0, 1, 0x65, 3, 4]);
        assert!(c.list(&sd).is_empty(), "still in tmp, not yet on sd_dir");
        // Feed drops: the flight is saved to sd_dir.
        c.auto_tick(&tmp, &sd, true, false);
        let listed = c.list(&sd);
        assert_eq!(listed.len(), 1);
        let name = listed[0]["name"].as_str().unwrap().to_owned();
        assert!(name.starts_with("flight-"));
        assert_eq!(
            fs::read(sd.join(&name)).unwrap(),
            [0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x65, 3, 4]
        );
        // The tmp scratch file is cleaned up after saving.
        assert!(fs::read_dir(&tmp).unwrap().next().is_none());
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn crash_buffer_saves_on_drop_only_when_enabled() {
        let (c, dir) = test_controller("crash");
        // Disabled: a drop in live input saves nothing.
        c.crash_tick(&dir, false, true);
        c.feed(&[0, 0, 1, 0x67, 1, 2]);
        c.crash_tick(&dir, false, false);
        assert!(c.list(&dir).is_empty());
        // Enabled: the drop auto-saves the current ring-buffer snapshot.
        c.crash_tick(&dir, true, true);
        c.feed(&[0, 0, 1, 0x65, 3, 4]);
        c.crash_tick(&dir, true, false);
        let listed = c.list(&dir);
        assert_eq!(listed.len(), 1);
        let name = listed[0]["name"].as_str().unwrap().to_owned();
        assert!(name.starts_with("crash-"));
        // Staying dropped must not save again.
        c.crash_tick(&dir, true, false);
        assert_eq!(c.list(&dir).len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crash_buffer_prunes_to_the_most_recent_clips() {
        let (c, dir) = test_controller("crash-prune");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..(MAX_CRASH_CLIPS + 2) {
            c.crash_tick(&dir, true, true);
            c.feed(&[0, 0, 1, 0x67, i as u8]);
            c.crash_tick(&dir, true, false);
        }
        let listed = c.list(&dir);
        assert_eq!(listed.len(), MAX_CRASH_CLIPS);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn auto_record_prunes_to_the_most_recent_flights() {
        let (c, sd) = test_controller("prune");
        let tmp = scratch("prune-tmp");
        fs::create_dir_all(&sd).unwrap();
        for i in 0..(MAX_FLIGHTS + 2) {
            c.auto_tick(&tmp, &sd, true, true);
            c.feed(&[0, 0, 1, 0x67, i as u8]);
            c.auto_tick(&tmp, &sd, true, false);
        }
        let listed = c.list(&sd);
        assert_eq!(listed.len(), MAX_FLIGHTS);
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }
}
