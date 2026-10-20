//! Record the live goggles feed to local storage (the SD card), independent
//! of HDMI output and the USB-drive video browser. Two ways to capture:
//! an explicit start/stop recording, and an always-on ring buffer that can
//! save whatever happened in roughly the last 30 seconds on demand. Both
//! write the same raw Annex-B H.264 the live decoder already receives, with
//! no re-encoding, matching how fallback videos are stored.
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

struct Active {
    file: File,
    path: PathBuf,
    name: String,
    started: Instant,
    bytes: u64,
}
struct Buffer {
    entries: VecDeque<(Instant, Vec<u8>)>,
    bytes: usize,
}
impl Default for Buffer {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }
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
}

#[derive(Clone, Default)]
pub struct Controller(Arc<Mutex<Inner>>);
impl Controller {
    /// Start a new continuous recording. Fails without creating a file if
    /// there is already too little free space.
    pub fn start(&self, dir: &Path) -> anyhow::Result<Value> {
        fs::create_dir_all(dir)?;
        if free_bytes(dir).is_some_and(|f| f < MIN_FREE_BYTES) {
            anyhow::bail!("Less than 2 GiB free; free up space before recording");
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let name = format!("recording-{stamp}.h264");
        let path = dir.join(&name);
        let file = File::create(&path)?;
        let mut inner = self.0.lock().unwrap();
        inner.active = Some(Active {
            file,
            path,
            name: name.clone(),
            started: Instant::now(),
            bytes: 0,
        });
        Ok(json!({"recording":true,"name":name}))
    }
    /// Stop the active recording, if any, and report how big it ended up.
    pub fn stop(&self) -> Option<Value> {
        let active = self.0.lock().unwrap().active.take()?;
        Some(json!({
            "name": active.name,
            "bytes": active.bytes,
            "seconds": active.started.elapsed().as_secs(),
        }))
    }
    pub fn status(&self) -> Value {
        match &self.0.lock().unwrap().active {
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
        let mut inner = self.0.lock().unwrap();
        inner.buffer.push(nal);
        if let Some(active) = &mut inner.active
            && active.file.write_all(nal).is_ok()
        {
            active.bytes += nal.len() as u64;
        }
    }
    /// Check free space while a recording is active; stop it and return a
    /// human-readable reason if it just dropped below the minimum. A no-op,
    /// cheap to call every tick, while nothing is recording.
    pub fn enforce_space(&self, dir: &Path) -> Option<String> {
        if self.0.lock().unwrap().active.is_none() {
            return None;
        }
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
            let inner = self.0.lock().unwrap();
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
    (name.starts_with("recording-") || name.starts_with("clip-"))
        && name.ends_with(".h264")
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}
pub fn list(dir: &Path) -> Vec<Value> {
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
            Some(json!({"name":name,"size":meta.len(),"modified":modified}))
        })
        .collect();
    entries.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    entries
}
pub fn remove(dir: &Path, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_name(name), "Invalid recording name");
    Ok(fs::remove_file(dir.join(name))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_unrecognized_names() {
        assert!(valid_name("recording-123.h264"));
        assert!(valid_name("clip-456.h264"));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("recording-123.mp4"));
        assert!(!valid_name("other-123.h264"));
    }

    #[test]
    fn records_and_stops_on_demand() {
        let dir = std::env::temp_dir().join(format!("ungoggled-recording-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c = Controller::default();
        assert!(!c.status()["recording"].as_bool().unwrap());
        c.start(&dir).unwrap();
        assert!(c.status()["recording"].as_bool().unwrap());
        c.feed(&[0, 0, 1, 0x67, 1, 2, 3]);
        c.feed(&[0, 0, 1, 0x65, 4, 5]);
        let summary = c.stop().unwrap();
        assert_eq!(summary["bytes"], 12);
        assert!(!c.status()["recording"].as_bool().unwrap());
        let name = summary["name"].as_str().unwrap();
        assert_eq!(
            fs::read(dir.join(name)).unwrap(),
            [0, 0, 1, 0x67, 1, 2, 3, 0, 0, 1, 0x65, 4, 5]
        );
        let listed = list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], name);
        remove(&dir, name).unwrap();
        assert!(list(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn buffer_snapshot_starts_at_the_first_sps() {
        let dir = std::env::temp_dir().join(format!("ungoggled-recording-buf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c = Controller::default();
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
        let dir = std::env::temp_dir().join(format!("ungoggled-recording-empty-{}", std::process::id()));
        let c = Controller::default();
        assert!(c.save_buffer(&dir).is_err());
    }
}
