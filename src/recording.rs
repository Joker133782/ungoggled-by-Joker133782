//! Record the live goggles feed to local storage (the SD card), independent
//! of HDMI output and the USB-drive video browser. Four ways to capture,
//! all writing the same raw Annex-B H.264 the live decoder already
//! receives, with no re-encoding, matching how fallback videos are stored:
//! an explicit start/stop recording, an always-on ~30 s ring buffer saved
//! on demand or automatically when the feed drops (like the goggles' own
//! instant-replay), and auto-recording a whole flight (buffered in RAM,
//! under the runtime tmpfs, while the feed is live; written to the SD card,
//! keeping the 5 most recent, as soon as the feed drops).
use crate::sync::LockExt;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::JoinHandle,
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
/// A flight (or one part of a long one) currently being buffered in RAM
/// (tmpfs), not yet on the SD card.
///
/// The tmpfs is a fraction of the Pi's RAM and also holds mpv's socket, the
/// thumbnails and GStreamer's registry, while a flight can easily be a
/// gigabyte: so a part ends at `cap` bytes, is saved on its own, and the
/// flight goes on in the next part (`flight-<stamp>-2.h264`, ...).
struct AutoRecording {
    file: File,
    tmp_path: PathBuf,
    /// The start of the flight (its first part), shared by all its parts.
    stamp: u128,
    part: u32,
    bytes: u64,
    cap: u64,
    /// A write failed (the tmpfs is full): the part is saved as far as it
    /// got and the next one is started.
    failed: bool,
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
    /// Replaces the part size derived from the free tmpfs space (tests).
    cap_override: Option<u64>,
}

#[derive(Clone)]
pub struct Controller {
    inner: Arc<Mutex<Inner>>,
    /// Saves to the SD card or a USB drive run on their own threads: a flight
    /// is hundreds of MB, and copying it must neither stall the video loop
    /// nor hold the lock that `feed` and the status request need.
    saving: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Under the runtime directory (tmpfs): gone on every reboot or power
    /// loss, so cached thumbnails never accumulate on the SD card.
    thumb_dir: PathBuf,
}
impl Controller {
    pub fn new(thumb_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&thumb_dir);
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            saving: Arc::new(Mutex::new(Vec::new())),
            thumb_dir,
        }
    }
    fn save_in_background(&self, job: impl FnOnce() + Send + 'static) {
        let handle = std::thread::spawn(job);
        let mut saving = self
            .saving
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        saving.retain(|h| !h.is_finished());
        saving.push(handle);
    }
    /// Waits until every save started so far has finished (at shutdown, so
    /// the service is not stopped half way through copying a flight).
    pub fn wait_for_saves(&self) {
        let handles: Vec<_> = self
            .saving
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect();
        for handle in handles {
            let _ = handle.join();
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
        let mut inner = self.inner.lock_ok();
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
        let active = self.inner.lock_ok().active.take()?;
        Some(json!({
            "name": active.name,
            "bytes": active.bytes,
            "seconds": active.started.elapsed().as_secs(),
        }))
    }
    pub fn status(&self) -> Value {
        match &self.inner.lock_ok().active {
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
        let mut inner = self.inner.lock_ok();
        inner.buffer.push(nal);
        if let Some(active) = &mut inner.active
            && active.file.write_all(nal).is_ok()
        {
            active.bytes += nal.len() as u64;
        }
        if let Some(auto) = &mut inner.auto
            && !auto.failed
        {
            match auto.file.write_all(nal) {
                Ok(()) => auto.bytes += nal.len() as u64,
                Err(_) => auto.failed = true,
            }
        }
    }
    /// Keep auto-recording in sync with whether live input is currently
    /// flowing, when the `auto_record` setting is on: starts buffering a
    /// flight in `tmp_dir` (tmpfs) when input appears, and saves it to
    /// `sd_dir` (pruned to the most recent `MAX_FLIGHTS`) as soon as input
    /// stops. A part that has grown to its size limit is saved meanwhile and
    /// the flight goes on in a new one. Cheap to call every tick either way;
    /// the saving itself runs on another thread (see `wait_for_saves`).
    /// Returns something the owner should be told, if there is.
    pub fn auto_tick(
        &self,
        tmp_dir: &Path,
        sd_dir: &Path,
        enabled: bool,
        input_live: bool,
    ) -> Option<String> {
        let mut message = None;
        let finished = {
            let mut inner = self.inner.lock_ok();
            let mut finished = Vec::new();
            if !enabled || !input_live {
                finished.extend(inner.auto.take());
            } else {
                if inner
                    .auto
                    .as_ref()
                    .is_some_and(|a| a.failed || a.bytes >= a.cap)
                {
                    let old = inner.auto.take().unwrap();
                    if old.failed {
                        message = Some(
                            "Flight recording: the temporary memory is full or not writable; \
                             the flight is saved in parts"
                                .to_owned(),
                        );
                    }
                    inner.auto = start_auto_recording(
                        tmp_dir,
                        Some((old.stamp, old.part + 1)),
                        inner.cap_override,
                    )
                    .ok();
                    finished.push(old);
                }
                if inner.auto.is_none()
                    && let Ok(auto) = start_auto_recording(tmp_dir, None, inner.cap_override)
                {
                    inner.auto = Some(auto);
                }
            }
            finished
        };
        for auto in finished {
            let sd_dir = sd_dir.to_path_buf();
            self.save_in_background(move || finalize_flight(auto, &sd_dir));
        }
        message
    }
    /// Keep the crash buffer in sync with live input, when the
    /// `crash_buffer` setting is on. The ~30 s ring buffer itself always
    /// runs regardless of this setting (it also backs the manual "save last
    /// 30 seconds" button); this only decides whether a drop in live input
    /// auto-saves it, like the goggles' own instant-replay, keeping the
    /// most recent `MAX_CRASH_CLIPS`. Cheap to call every tick either way.
    pub fn crash_tick(&self, dir: &Path, enabled: bool, input_live: bool) {
        let snapshot = {
            let mut inner = self.inner.lock_ok();
            let just_dropped = enabled && inner.crash_was_live && !input_live;
            inner.crash_was_live = input_live;
            just_dropped.then(|| inner.buffer.snapshot()).flatten()
        };
        if let Some(bytes) = snapshot {
            let dir = dir.to_path_buf();
            self.save_in_background(move || {
                let _ = save_crash_clip(&dir, &bytes);
            });
        }
    }
    /// Check free space while a recording is active; stop it and return a
    /// human-readable reason if it just dropped below the minimum. A no-op,
    /// cheap to call every tick, while nothing is recording.
    pub fn enforce_space(&self, dir: &Path) -> Option<String> {
        self.inner.lock_ok().active.as_ref()?;
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
            let inner = self.inner.lock_ok();
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
                let thumb = self.thumb_dir.join(format!("{name}.jpg"));
                let marker = self.thumb_dir.join(format!("{name}.failed"));
                let thumbnail = if thumb.exists() {
                    true
                } else if failed_at_size(&marker, meta.len()) {
                    // Tried already: a file mpv cannot open would otherwise cost
                    // up to 20 s on every single list request.
                    false
                } else {
                    let made = crate::storage::ensure_thumbnail(&e.path(), &thumb);
                    if !made {
                        let _ = fs::write(&marker, meta.len().to_string());
                    }
                    made
                };
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

/// How much of the free tmpfs space one part may take: a third, so the part
/// being written and one still being copied away both fit with room to spare.
fn part_cap(dir: &Path) -> u64 {
    const MIB: u64 = 1024 * 1024;
    free_bytes(dir).map_or(64 * MIB, |free| (free / 3).clamp(8 * MIB, 256 * MIB))
}
/// Create a new tmpfs scratch file: for a flight about to start, or (`next`:
/// the flight's stamp and the part number) for its next part.
fn start_auto_recording(
    tmp_dir: &Path,
    next: Option<(u128, u32)>,
    cap_override: Option<u64>,
) -> std::io::Result<AutoRecording> {
    fs::create_dir_all(tmp_dir)?;
    let (stamp, part) = next.unwrap_or_else(|| {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        (stamp, 1)
    });
    let name = if part == 1 {
        format!("flight-{stamp}.h264")
    } else {
        format!("flight-{stamp}-{part}.h264")
    };
    let tmp_path = tmp_dir.join(name);
    let file = File::create(&tmp_path)?;
    Ok(AutoRecording {
        file,
        tmp_path,
        stamp,
        part,
        bytes: 0,
        cap: cap_override.unwrap_or_else(|| part_cap(tmp_dir)),
        failed: false,
    })
}
/// Copy a finished flight (part) from the tmpfs scratch file to the SD card
/// and prune older flights beyond `MAX_FLIGHTS`; always removes the scratch
/// file, whether or not the copy succeeded. The copy goes to a name that is
/// not a recording until it is complete, so a copy that is cut short (the
/// power, a service stop) never looks like a finished file.
fn finalize_flight(auto: AutoRecording, sd_dir: &Path) {
    let AutoRecording { file, tmp_path, .. } = auto;
    drop(file);
    let name = tmp_path
        .file_name()
        .map(PathBuf::from)
        .filter(|_| fs::metadata(&tmp_path).is_ok_and(|m| m.len() > 0));
    if let Some(name) = name
        && fs::create_dir_all(sd_dir).is_ok()
    {
        let dest = sd_dir.join(&name);
        let mut unfinished = dest.clone().into_os_string();
        unfinished.push(".part");
        let unfinished = PathBuf::from(unfinished);
        if fs::copy(&tmp_path, &unfinished).is_ok() && fs::rename(&unfinished, &dest).is_ok() {
            prune_flights(sd_dir);
        } else {
            let _ = fs::remove_file(&unfinished);
        }
    }
    let _ = fs::remove_file(&tmp_path);
}
/// Keeps the files of the `MAX_FLIGHTS` most recent flights (all parts of a
/// flight count as one) and deletes the rest.
fn prune_flights(dir: &Path) {
    let mut flights: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name
            .strip_prefix("flight-")
            .filter(|_| name.ends_with(".h264"))
        else {
            continue;
        };
        let stamp = rest.split(['-', '.']).next().unwrap_or_default();
        flights.entry(stamp.to_owned()).or_default().push(name);
    }
    // Nanosecond stamps sort chronologically as strings.
    for stamp in flights.keys().rev().skip(MAX_FLIGHTS) {
        for name in &flights[stamp] {
            let _ = fs::remove_file(dir.join(name));
        }
    }
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

/// A thumbnail attempt that failed is remembered with the file's size at the
/// time: a file that is still being written (or was replaced) is tried again
/// once its size differs.
fn failed_at_size(marker: &Path, size: u64) -> bool {
    fs::read_to_string(marker)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        == Some(size)
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
        c.wait_for_saves();
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
        c.wait_for_saves();
        let listed = c.list(&dir);
        assert_eq!(listed.len(), 1);
        let name = listed[0]["name"].as_str().unwrap().to_owned();
        assert!(name.starts_with("crash-"));
        // Staying dropped must not save again.
        c.crash_tick(&dir, true, false);
        c.wait_for_saves();
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
            c.wait_for_saves();
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
            c.wait_for_saves();
            // Flight names are nanosecond timestamps: keep them distinct.
            std::thread::sleep(Duration::from_millis(2));
        }
        let listed = c.list(&sd);
        assert_eq!(listed.len(), MAX_FLIGHTS);
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    fn sps(tag: u8, filler: usize) -> Vec<u8> {
        let mut nal = vec![0, 0, 1, 0x67, tag];
        nal.extend(std::iter::repeat_n(0xab, filler));
        nal
    }

    #[test]
    fn a_long_flight_is_saved_in_parts_that_follow_each_other() {
        let (c, sd) = test_controller("parts");
        let tmp = scratch("parts-tmp");
        c.inner.lock_ok().cap_override = Some(40);
        c.auto_tick(&tmp, &sd, true, true);
        let first = sps(1, 20);
        let second = sps(2, 20);
        let third = sps(3, 5);
        c.feed(&first);
        c.feed(&second);
        // The first part is full: the next tick saves it and carries on.
        c.auto_tick(&tmp, &sd, true, true);
        c.feed(&third);
        c.auto_tick(&tmp, &sd, true, false);
        c.wait_for_saves();
        let mut names: Vec<String> = c
            .list(&sd)
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "{names:?}");
        let (one, two) = if names[0].contains("-2.") {
            (&names[1], &names[0])
        } else {
            (&names[0], &names[1])
        };
        assert!(valid_name(one) && valid_name(two));
        assert_eq!(
            one.trim_end_matches(".h264"),
            two.trim_end_matches("-2.h264"),
            "same flight"
        );
        assert_eq!(fs::read(sd.join(one)).unwrap(), [first, second].concat());
        assert_eq!(fs::read(sd.join(two)).unwrap(), third);
        assert!(fs::read_dir(&tmp).unwrap().next().is_none(), "scratch gone");
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn all_parts_of_a_flight_count_as_one_when_pruning() {
        let dir = scratch("prune-parts");
        fs::create_dir_all(&dir).unwrap();
        for stamp in 100..100 + (MAX_FLIGHTS as u32 + 2) {
            fs::write(dir.join(format!("flight-{stamp}.h264")), b"x").unwrap();
            fs::write(dir.join(format!("flight-{stamp}-2.h264")), b"x").unwrap();
            fs::write(dir.join(format!("flight-{stamp}-3.h264")), b"x").unwrap();
        }
        fs::write(dir.join("recording-1.h264"), b"x").unwrap();
        prune_flights(&dir);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), MAX_FLIGHTS * 3 + 1, "{left:?}");
        assert!(left.contains(&"recording-1.h264".to_owned()));
        assert!(left.contains(&format!("flight-{}-3.h264", 100 + MAX_FLIGHTS as u32 + 1)));
        assert!(!left.contains(&"flight-100.h264".to_owned()));
        assert!(!left.contains(&"flight-101-2.h264".to_owned()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_scratch_disk_ends_the_part_and_says_so() {
        let (c, sd) = test_controller("full");
        let tmp = scratch("full-tmp");
        fs::create_dir_all(&tmp).unwrap();
        // /dev/full accepts the open and fails every write with ENOSPC, like
        // a full tmpfs.
        let broken = AutoRecording {
            file: fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
            tmp_path: tmp.join("flight-5.h264"),
            stamp: 5,
            part: 1,
            bytes: 0,
            cap: u64::MAX,
            failed: false,
        };
        fs::write(&broken.tmp_path, b"first part so far").unwrap();
        c.inner.lock_ok().auto = Some(broken);
        c.feed(&sps(1, 10));
        let message = c.auto_tick(&tmp, &sd, true, true);
        assert!(message.is_some_and(|m| m.contains("saved in parts")));
        c.auto_tick(&tmp, &sd, true, false);
        c.wait_for_saves();
        let saved: Vec<_> = c.list(&sd);
        assert_eq!(saved.len(), 1, "what the first part held is kept");
        assert_eq!(
            fs::read(sd.join(saved[0]["name"].as_str().unwrap())).unwrap(),
            b"first part so far"
        );
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_copy_that_is_not_finished_never_looks_like_a_recording() {
        let (c, sd) = test_controller("unfinished");
        let tmp = scratch("unfinished-tmp");
        fs::create_dir_all(&tmp).unwrap();
        // A destination that cannot be written: the copy fails half way.
        let auto = start_auto_recording(&tmp, None, Some(1 << 20)).unwrap();
        let mut file = auto.file.try_clone().unwrap();
        file.write_all(&sps(1, 10)).unwrap();
        fs::create_dir_all(&sd).unwrap();
        let name = auto.tmp_path.file_name().unwrap().to_owned();
        // A directory in the way of the final name makes the rename fail.
        fs::create_dir_all(sd.join(&name)).unwrap();
        finalize_flight(auto, &sd);
        let mut leftovers: Vec<String> = fs::read_dir(&sd)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        leftovers.sort();
        assert_eq!(leftovers, [name.to_string_lossy().into_owned()]);
        assert!(fs::read_dir(&tmp).unwrap().next().is_none());
        assert!(
            c.list(&sd)
                .iter()
                .all(|r| !r["name"].as_str().unwrap().ends_with(".part"))
        );
        fs::remove_dir_all(&sd).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn waiting_for_saves_with_nothing_pending_returns_at_once() {
        let (c, _dir) = test_controller("idle-wait");
        c.wait_for_saves();
    }

    #[test]
    fn a_recording_that_cannot_be_thumbnailed_is_not_retried_until_it_changes() {
        let dir = std::env::temp_dir().join(format!("ungoggled-rec-thumb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let thumbs = dir.join("thumbs");
        let recs = dir.join("recs");
        fs::create_dir_all(&recs).unwrap();
        let ctl = Controller::new(thumbs.clone());
        let file = recs.join("flight-1.h264");
        fs::write(&file, b"not a video").unwrap();
        let first = ctl.list(&recs);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["thumbnail"], false);
        let marker = thumbs.join("flight-1.h264.failed");
        assert_eq!(fs::read_to_string(&marker).unwrap(), "11");
        assert!(failed_at_size(&marker, 11));
        assert!(
            !failed_at_size(&marker, 12),
            "a changed file is tried again"
        );
        assert!(!failed_at_size(&thumbs.join("nothing.failed"), 11));
        let _ = fs::remove_dir_all(&dir);
    }
}
