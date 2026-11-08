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
    /// Mounted read-write when the `record_to_usb` setting is on (so
    /// recordings can be written there), read-only otherwise — remounted
    /// when the setting changes.
    read_write: bool,
}

#[derive(Clone)]
pub struct Store {
    mounted: Arc<Mutex<Vec<Mounted>>>,
    videos: Arc<Mutex<HashMap<String, PathBuf>>>,
    /// `Some((done, total))` while thumbnails are being generated in the
    /// background, so the web interface can show that it is still working
    /// instead of looking stuck on a drive with many clips.
    scan: Arc<Mutex<Option<(usize, usize)>>>,
    /// Guards against starting a second background thumbnail pass while one
    /// from an earlier `list_videos` call is still working through its list.
    thumbnailing: Arc<Mutex<bool>>,
    /// Under the runtime directory (tmpfs): gone on every reboot or power
    /// loss, so a card full of old clips never leaves stale thumbnails on
    /// the SD card.
    thumb_dir: PathBuf,
    /// `video::App::usb_playback` — checked before generating any
    /// thumbnail. Thumbnails are decoded in software so they never take
    /// the CMA memory playback needs (see `ensure_thumbnail`), but still
    /// stand aside while a video is playing so they don't compete with it
    /// for CPU either, and pick up anything still missing on a later rescan.
    usb_playback: Arc<Mutex<Option<PathBuf>>>,
}
impl Store {
    /// Friendly names of every USB mass-storage drive mounted right now,
    /// regardless of whether `list_videos` has scanned it yet — lets the web
    /// interface confirm a drive was recognized before (or without) scanning
    /// it for videos.
    pub fn drives(&self) -> Vec<String> {
        self.mounted
            .lock()
            .unwrap()
            .iter()
            .map(|d| friendly_device_name(&d.device).unwrap_or_else(|| d.device.clone()))
            .collect()
    }
    /// `(files found so far, files with a thumbnail decided)` while a scan
    /// from `list_videos` is in progress, `None` otherwise.
    pub fn scan_progress(&self) -> Option<(usize, usize)> {
        *self.scan.lock().unwrap()
    }
    /// The first drive currently mounted read-write (i.e. mounted because
    /// `record_to_usb` is on), for recordings to land on instead of the SD
    /// card. `None` when the setting is off or nothing is plugged in yet, in
    /// which case the caller falls back to the SD card.
    pub fn recording_mountpoint(&self) -> Option<PathBuf> {
        self.mounted
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.read_write)
            .map(|m| m.mountpoint.clone())
    }
    /// Rescan every currently mounted drive and return the videos found,
    /// newest first. Assigns each file a stable opaque id, resolvable with
    /// `resolve`. Fast: file metadata only, no decoding — any thumbnail
    /// still missing from the cache is generated afterward in the
    /// background (see `scan_progress`), so a drive full of action-cam
    /// footage never makes the list itself wait on it.
    pub fn list_videos(&self) -> Vec<Value> {
        let mounted = self.mounted.lock().unwrap().clone();
        let mut registry = self.videos.lock().unwrap();
        registry.clear();
        let mut found: Vec<_> = mounted
            .iter()
            .flat_map(|drive| {
                find_videos(&drive.mountpoint, MAX_DEPTH, MAX_FILES)
                    .into_iter()
                    .map(move |path| (drive, path))
            })
            .collect();
        found.sort_by_key(|(_, path)| {
            std::cmp::Reverse(fs::metadata(path).and_then(|m| m.modified()).ok())
        });
        let mut out = Vec::with_capacity(found.len());
        let mut missing_thumbnail = Vec::new();
        for (drive, path) in &found {
            let id = video_id(path);
            let name = path
                .strip_prefix(&drive.mountpoint)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();
            let meta = fs::metadata(path).ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let thumb_path = self.thumb_dir.join(format!("{id}.jpg"));
            let thumbnail = thumb_path.exists();
            if !thumbnail {
                missing_thumbnail.push((path.clone(), thumb_path));
            }
            let device =
                friendly_device_name(&drive.device).unwrap_or_else(|| drive.device.clone());
            out.push(
                json!({"id":id,"name":name,"device":device,"size":size,"mtime":mtime,"thumbnail":thumbnail}),
            );
            registry.insert(id, path.clone());
        }
        drop(registry);
        self.spawn_thumbnailing(missing_thumbnail);
        out
    }
    /// Generate every thumbnail still missing, one file at a time, without
    /// blocking the caller. A no-op if a background pass from an earlier
    /// call is still running, or a USB video is currently playing — either
    /// way, a later rescan picks up anything still missing.
    fn spawn_thumbnailing(&self, missing: Vec<(PathBuf, PathBuf)>) {
        if missing.is_empty() || self.usb_playback.lock().unwrap().is_some() {
            return;
        }
        let mut running = self.thumbnailing.lock().unwrap();
        if *running {
            return;
        }
        *running = true;
        drop(running);
        let scan = self.scan.clone();
        let thumbnailing = self.thumbnailing.clone();
        let usb_playback = self.usb_playback.clone();
        let total = missing.len();
        *scan.lock().unwrap() = Some((0, total));
        thread::spawn(move || {
            for (done, (video, dest)) in missing.into_iter().enumerate() {
                if usb_playback.lock().unwrap().is_some() {
                    break;
                }
                ensure_thumbnail(&video, &dest);
                *scan.lock().unwrap() = Some((done + 1, total));
            }
            *scan.lock().unwrap() = None;
            *thumbnailing.lock().unwrap() = false;
        });
    }
    /// Resolve an id from the most recent `list_videos` call back to a path.
    pub fn resolve(&self, id: &str) -> Option<PathBuf> {
        self.videos.lock().unwrap().get(id).cloned()
    }
    /// The cached thumbnail for an id, if one has been generated.
    pub fn thumbnail(&self, id: &str) -> Option<PathBuf> {
        let path = valid_id(id).then(|| self.thumb_dir.join(format!("{id}.jpg")))?;
        path.exists().then_some(path)
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
/// Grab one frame a few seconds in as a small JPEG with mpv, caching it by
/// id so a rescan only pays for videos it has not thumbnailed yet.
/// Best-effort: a file mpv cannot open just stays without a thumbnail.
pub(crate) fn ensure_thumbnail(video: &Path, dest: &Path) -> bool {
    ensure_thumbnail_sized(video, dest, 160)
}
/// `ensure_thumbnail`, `width` pixels wide (the idle-screen posters are also
/// shown larger than in a list).
pub(crate) fn ensure_thumbnail_sized(video: &Path, dest: &Path, width: u32) -> bool {
    if dest.exists() {
        return true;
    }
    // mpv's image VO writes a numbered frame into a directory rather than to
    // an exact path, so render into a scratch directory next to the cache
    // and move the single result out. GStreamer's decodebin was tried first
    // and dropped: decodebin/qtdemux spent ~20s just indexing a real 3.7 GiB
    // 4K clip before decoding anything (vs. mpv's own demuxer seeking near
    // the target time directly), ~2-7s end to end either way here — see
    // FINDINGS.MD.
    let scratch = dest.with_extension("thumbdir");
    let _ = fs::remove_dir_all(&scratch);
    if fs::create_dir_all(&scratch).is_err() {
        return false;
    }
    let outdir = format!("--vo-image-outdir={}", scratch.display());
    // mpv scales correctly out of the box, including a real 4:3 4096x3072
    // source's pixel aspect ratio (GStreamer's videoscale needed an explicit
    // PAR override for that case).
    let scale = format!("--vf=scale={width}:-2");
    let ok = Command::new("nice")
        // `nice`: a thumbnail must never compete with live video for CPU.
        // `timeout`: a safety net, not expected to be hit — grabbing frame 0
        // (below) needs no seek, so even a demanding 4K60 clip decodes in a
        // couple of seconds on a Pi 4.
        .args(["-n", "10", "timeout", "20", "mpv"])
        .args([
            "--no-config",
            // Frame 0, not a few seconds in: it's reliably a keyframe, so
            // this needs no seek/decode-forward through a GOP — a seek-based
            // thumbnail once took so long on a real 4K60 clip that it could
            // lose the race against the background scan reporting itself
            // done (see git history for the fuller story).
            "--frames=1",
            // Hardware decode would take its buffers from the same CMA pool
            // as USB playback, the live decoder and the display: a
            // thumbnail still decoding when playback started once left mpv
            // unable to allocate its own buffers, so it silently played the
            // audio only (see FINDINGS.MD). A single still frame gains
            // nothing from hardware decode anyway.
            "--hwdec=no",
            "--ao=null",
            "--no-terminal",
            "--vo=image",
            "--vo-image-format=jpg",
            &scale,
            &outdir,
        ])
        .arg(video)
        .output()
        .is_ok_and(|o| o.status.success());
    let moved = ok
        .then(|| fs::read_dir(&scratch).ok())
        .flatten()
        .and_then(|mut entries| entries.next())
        .and_then(|e| e.ok())
        .is_some_and(|entry| fs::rename(entry.path(), dest).is_ok());
    let _ = fs::remove_dir_all(&scratch);
    moved
}

/// Poll for removable USB block devices, mount new ones under
/// `runtime_dir/usb-media` (read-only unless `settings.record_to_usb` is on),
/// and unmount+remove ones that disappeared. Never touches the device the
/// root filesystem lives on.
pub fn start(
    runtime_dir: PathBuf,
    settings: crate::settings::Store,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    usb_playback: Arc<Mutex<Option<PathBuf>>>,
) -> Store {
    let thumb_dir = runtime_dir.join("usb-thumbs");
    let _ = fs::create_dir_all(&thumb_dir);
    let store = Store {
        mounted: Arc::new(Mutex::new(Vec::new())),
        videos: Arc::new(Mutex::new(HashMap::new())),
        scan: Arc::new(Mutex::new(None)),
        thumbnailing: Arc::new(Mutex::new(false)),
        thumb_dir,
        usb_playback,
    };
    let base = runtime_dir.join("usb-media");
    let _ = fs::create_dir_all(&base);
    let watched = store.mounted.clone();
    thread::spawn(move || {
        use std::sync::atomic::Ordering;
        while !shutdown.load(Ordering::Relaxed) {
            reconcile(&base, &watched, settings.get().record_to_usb);
            thread::sleep(Duration::from_secs(2));
        }
        for drive in watched.lock().unwrap().drain(..) {
            unmount(&drive);
        }
    });
    store
}

fn reconcile(base: &Path, mounted: &Arc<Mutex<Vec<Mounted>>>, read_write: bool) {
    let excluded = root_devices();
    let present: Vec<String> = removable_block_devices()
        .into_iter()
        .filter(|d| !excluded.contains(d))
        .collect();
    let mut guard = mounted.lock().unwrap();
    guard.retain(|m| {
        if !present.contains(&m.device) {
            unmount(m);
            false
        } else if m.read_write != read_write {
            // record_to_usb changed since this drive was mounted: drop it so
            // the loop below mounts it fresh in the now-desired mode.
            unmount(m);
            false
        } else {
            true
        }
    });
    for device in present {
        if guard.iter().any(|m| m.device == device) {
            continue;
        }
        let mountpoint = base.join(&device);
        // Already mounted from a previous run that did not unmount cleanly
        // (an unclean shutdown, e.g.): adopt it rather than trying to mount
        // over it, which would just fail and leave the device untracked. Its
        // actual mode is unknown; assume it matches what's currently wanted
        // rather than forcing an unnecessary remount on every boot.
        if let Some(existing) = existing_mountpoint(&device) {
            guard.push(Mounted {
                device,
                mountpoint: existing,
                read_write,
            });
            continue;
        }
        if fs::create_dir_all(&mountpoint).is_err() {
            continue;
        }
        let opts = if read_write {
            "rw,noexec,nosuid,nodev"
        } else {
            "ro,noexec,nosuid,nodev"
        };
        let ok = Command::new("mount")
            .args(["-o", opts])
            .arg("--")
            .arg(format!("/dev/{device}"))
            .arg(&mountpoint)
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            guard.push(Mounted {
                device,
                mountpoint,
                read_write,
            });
        } else {
            let _ = fs::remove_dir(&mountpoint);
        }
    }
}
/// Where `/dev/<device>` is already mounted, per the kernel's own mount
/// table, if anywhere.
fn existing_mountpoint(device: &str) -> Option<PathBuf> {
    let target = format!("/dev/{device}");
    fs::read_to_string("/proc/mounts")
        .ok()?
        .lines()
        .find_map(|l| {
            let mut parts = l.split_whitespace();
            if parts.next()? != target {
                return None;
            }
            parts.next().map(PathBuf::from)
        })
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
/// The USB device's own descriptor strings (e.g. "DJI DJIAction2"), not the
/// SCSI/USB-storage gadget's INQUIRY vendor/model, which many cameras leave
/// as a generic "Linux File-Stor Gadget". Walks up from the block device's
/// `/sys/block/<disk>/device` symlink to the ancestor that carries
/// `manufacturer`/`product` files, i.e. the actual USB device node.
fn friendly_device_name(device: &str) -> Option<String> {
    let disk = device.trim_end_matches(|c: char| c.is_ascii_digit());
    let real = fs::canonicalize(format!("/sys/block/{disk}/device")).ok()?;
    real.ancestors().find_map(|dir| {
        let product = fs::read_to_string(dir.join("product")).ok()?;
        let product = product.trim();
        if product.is_empty() {
            return None;
        }
        let manufacturer = fs::read_to_string(dir.join("manufacturer")).ok();
        Some(match manufacturer {
            Some(m) if !product.starts_with(m.trim()) && !m.trim().is_empty() => {
                format!("{} {product}", m.trim())
            }
            _ => product.to_owned(),
        })
    })
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
    fn rejects_thumbnail_ids_that_are_not_plain_hex() {
        assert!(!valid_id("../../etc/passwd"));
        assert!(!valid_id("short"));
        assert!(!valid_id("zzzzzzzzzzzzzzzz"));
        assert!(valid_id("0123456789abcdef"));
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

    #[test]
    fn thumbnailing_stands_aside_while_a_usb_video_is_playing() {
        let thumb_dir =
            std::env::temp_dir().join(format!("ungoggled-thumbtest-{}", std::process::id()));
        fs::create_dir_all(&thumb_dir).unwrap();
        let store = Store {
            mounted: Arc::new(Mutex::new(Vec::new())),
            videos: Arc::new(Mutex::new(HashMap::new())),
            scan: Arc::new(Mutex::new(None)),
            thumbnailing: Arc::new(Mutex::new(false)),
            thumb_dir: thumb_dir.clone(),
            usb_playback: Arc::new(Mutex::new(Some(PathBuf::from("/mnt/usb-sda1/playing.mp4")))),
        };
        let missing = vec![(
            PathBuf::from("/mnt/usb-sda1/other.mp4"),
            thumb_dir.join("x.jpg"),
        )];

        // A video is playing: no pass starts at all, not even a "missing" one.
        store.spawn_thumbnailing(missing.clone());
        assert!(store.scan_progress().is_none());
        assert!(!*store.thumbnailing.lock().unwrap());

        // Once playback ends, a pass is free to run.
        *store.usb_playback.lock().unwrap() = None;
        store.spawn_thumbnailing(missing);
        assert!(*store.thumbnailing.lock().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while store.scan_progress().is_some() {
            assert!(std::time::Instant::now() < deadline, "pass never finished");
            thread::sleep(Duration::from_millis(5));
        }

        fs::remove_dir_all(&thumb_dir).unwrap();
    }
}
