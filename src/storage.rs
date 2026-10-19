//! Browse video files on USB mass-storage devices (action cameras) plugged
//! into a regular USB-A host port, so a clip can be played back on the HDMI
//! output. Deliberately separate from the goggles USB-C gadget port: Pi 4's
//! USB-C controller runs in one fixed role at a time, and dynamically
//! switching it between the goggles' accessory/gadget mode and acting as a
//! host for mass storage is not something this project relies on.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "m4v", "insv", "lrv"];
/// Keep scans fast and bounded even on a card full of unrelated files.
const MAX_FILES: usize = 4000;
const MAX_DEPTH: usize = 6;

#[derive(Clone)]
struct Mounted {
    device: String,
    mountpoint: PathBuf,
}

#[derive(Clone)]
pub struct Store {
    mounted: Arc<Mutex<Vec<Mounted>>>,
    videos: Arc<Mutex<HashMap<String, PathBuf>>>,
}
impl Store {
    /// Rescan every currently mounted drive and return the videos found.
    /// Assigns each file a stable opaque id, resolvable with `resolve`.
    pub fn list_videos(&self) -> Vec<Value> {
        let mounted = self.mounted.lock().unwrap().clone();
        let mut registry = self.videos.lock().unwrap();
        registry.clear();
        let mut out = Vec::new();
        for drive in &mounted {
            for path in find_videos(&drive.mountpoint, MAX_DEPTH, MAX_FILES) {
                let id = video_id(&path);
                let name = path
                    .strip_prefix(&drive.mountpoint)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.push(json!({"id":id,"name":name,"device":drive.device,"size":size}));
                registry.insert(id, path);
            }
        }
        out
    }
    /// Resolve an id from the most recent `list_videos` call back to a path.
    pub fn resolve(&self, id: &str) -> Option<PathBuf> {
        self.videos.lock().unwrap().get(id).cloned()
    }
}

/// Poll for removable USB block devices, mount new ones read-only under
/// `runtime_dir/usb-media`, and unmount+remove ones that disappeared. Never
/// touches the device the root filesystem lives on.
pub fn start(runtime_dir: PathBuf, shutdown: Arc<std::sync::atomic::AtomicBool>) -> Store {
    let store = Store {
        mounted: Arc::new(Mutex::new(Vec::new())),
        videos: Arc::new(Mutex::new(HashMap::new())),
    };
    let base = runtime_dir.join("usb-media");
    let _ = fs::create_dir_all(&base);
    let watched = store.mounted.clone();
    thread::spawn(move || {
        use std::sync::atomic::Ordering;
        while !shutdown.load(Ordering::Relaxed) {
            reconcile(&base, &watched);
            thread::sleep(Duration::from_secs(2));
        }
        for drive in watched.lock().unwrap().drain(..) {
            unmount(&drive);
        }
    });
    store
}

fn reconcile(base: &Path, mounted: &Arc<Mutex<Vec<Mounted>>>) {
    let excluded = root_devices();
    let present: Vec<String> = removable_block_devices()
        .into_iter()
        .filter(|d| !excluded.contains(d))
        .collect();
    let mut guard = mounted.lock().unwrap();
    guard.retain(|m| {
        if present.contains(&m.device) {
            true
        } else {
            unmount(m);
            false
        }
    });
    for device in present {
        if guard.iter().any(|m| m.device == device) {
            continue;
        }
        let mountpoint = base.join(&device);
        if fs::create_dir_all(&mountpoint).is_err() {
            continue;
        }
        let ok = Command::new("mount")
            .args(["-o", "ro,noexec,nosuid,nodev"])
            .arg("--")
            .arg(format!("/dev/{device}"))
            .arg(&mountpoint)
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            guard.push(Mounted { device, mountpoint });
        } else {
            let _ = fs::remove_dir(&mountpoint);
        }
    }
}
fn unmount(m: &Mounted) {
    let _ = Command::new("umount").arg(&m.mountpoint).output();
    let _ = fs::remove_dir(&m.mountpoint);
}

/// Block device names (e.g. "sda1") that are removable and not loop/dm/md.
fn removable_block_devices() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/sys/class/block") else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            !name.starts_with("loop") && !name.starts_with("dm-") && !name.starts_with("md")
        })
        .filter(|name| is_removable(name))
        .collect()
}
fn is_removable(name: &str) -> bool {
    let own = fs::read_to_string(format!("/sys/class/block/{name}/removable"));
    let parent = fs::read_to_string(format!("/sys/class/block/{name}/../removable"));
    own.or(parent).is_ok_and(|s| s.trim() == "1")
}
/// The whole-disk device backing `/`, plus any of its partitions, by prefix.
fn root_devices() -> Vec<String> {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    let Some(root) = mounts.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let dev = parts.next()?;
        let mountpoint = parts.next()?;
        (mountpoint == "/" && dev.starts_with("/dev/")).then(|| dev.trim_start_matches("/dev/"))
    }) else {
        return Vec::new();
    };
    let disk = root.trim_end_matches(|c: char| c.is_ascii_digit());
    let disk = disk
        .strip_suffix('p')
        .filter(|_| disk.contains("mmcblk"))
        .unwrap_or(disk);
    fs::read_dir("/sys/class/block")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n == disk || n.starts_with(disk))
        .collect()
}

fn is_video_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
fn video_id(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn find_videos(root: &Path, max_depth: usize, max_files: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= max_files {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if out.len() >= max_files {
                break;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
            } else if file_type.is_file()
                && path
                    .file_name()
                    .is_some_and(|n| is_video_file(&n.to_string_lossy()))
            {
                out.push(path);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_known_video_extensions_case_insensitively() {
        assert!(is_video_file("CLIP0001.MP4"));
        assert!(is_video_file("clip.mov"));
        assert!(is_video_file("clip.insv"));
        assert!(!is_video_file("clip.txt"));
        assert!(!is_video_file("noextension"));
    }

    #[test]
    fn video_id_is_stable_and_distinct() {
        let a = video_id(Path::new("/mnt/usb-sda1/DCIM/100MEDIA/clip.mp4"));
        let b = video_id(Path::new("/mnt/usb-sda1/DCIM/100MEDIA/clip.mp4"));
        let c = video_id(Path::new("/mnt/usb-sda1/DCIM/100MEDIA/other.mp4"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn finds_videos_within_depth_and_skips_other_files() {
        let dir =
            std::env::temp_dir().join(format!("ungoggled-storage-test-{}", std::process::id()));
        let nested = dir.join("DCIM").join("100MEDIA");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("clip.MP4"), b"x").unwrap();
        fs::write(nested.join("notes.txt"), b"x").unwrap();
        fs::write(dir.join("root.mov"), b"x").unwrap();
        let too_deep = nested.join("a/b/c/c/c/c/too_deep.mp4");
        fs::create_dir_all(too_deep.parent().unwrap()).unwrap();
        fs::write(&too_deep, b"x").unwrap();

        let found = find_videos(&dir, 3, 100);
        assert!(found.contains(&nested.join("clip.MP4")));
        assert!(found.contains(&dir.join("root.mov")));
        assert!(!found.iter().any(|p| p.ends_with("notes.txt")));
        assert!(!found.contains(&too_deep), "exceeds max depth");

        fs::remove_dir_all(&dir).unwrap();
    }
}
