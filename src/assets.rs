use anyhow::{Context, Result, bail};
use image::{GenericImageView, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MAX_UPLOAD: usize = 12 * 1024 * 1024;
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_UPLOAD {
        bail!("Image must be smaller than 12 MiB");
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
    ) {
        bail!("Use PNG, JPEG or WebP");
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().context("Cannot decode image")?;
    let img = img.resize(1920, 1080, image::imageops::FilterType::Triangle);
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}
// ---------------------------------------------------------------------------
// The idle-screen library (`<data>/images`): built-ins plus uploads, each
// with a list thumbnail under `thumbs/` and, for uploads, a `.meta.json`
// sidecar holding the original file name.
// ---------------------------------------------------------------------------

/// The permanent idle-screen entries, in list order, with their display
/// names. Refreshed on every start (`settings::Store::open`), never
/// deletable, and not counted against the upload limits below.
pub const BUILTINS: [(&str, &str); 4] = [
    ("default-loop.h264", "Logo loop"),
    ("default-loop-3d.h264", "Logo loop 3D"),
    ("default-comic.h264", "FPV comic"),
    ("no-signal.png", "No signal"),
];
/// Uploads allowed besides the built-ins. Built-in videos used to count
/// against a cap of four, which left room for one video of your own.
pub const MAX_USER_VIDEOS: usize = 3;
pub const MAX_USER_IMAGES: usize = 16;
/// Width of an image's list thumbnail (height follows the aspect ratio).
const THUMB_WIDTH: u32 = 320;

pub fn is_builtin(id: &str) -> bool {
    BUILTINS.iter().any(|(b, _)| *b == id)
}
/// Where an entry's list thumbnail lives (a JPEG).
pub fn thumb_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("thumbs").join(format!("{id}.jpg"))
}
fn meta_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(Path::new(id).with_extension("meta.json"))
}
#[derive(Serialize, Deserialize)]
struct Meta {
    name: String,
}
/// Just the file name a browser sent: no path, no control characters,
/// at most 80 characters.
fn clean_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next()?.trim();
    let name: String = base.chars().filter(|c| !c.is_control()).take(80).collect();
    (!name.is_empty()).then_some(name)
}
/// Best effort: without it the entry is just listed as "Your image/video".
fn save_name(dir: &Path, id: &str, raw: Option<&str>) {
    if let Some(name) = raw.and_then(clean_name)
        && let Ok(bytes) = serde_json::to_vec(&Meta { name })
    {
        let _ = crate::settings::atomic_write(&meta_path(dir, id), &bytes);
    }
}
fn user_count(dir: &Path, extension: &str) -> usize {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|f| {
            let id = f.file_name().to_string_lossy().into_owned();
            f.path().extension().is_some_and(|e| e == extension) && !is_builtin(&id)
        })
        .count()
}
pub fn limits(dir: &Path) -> Value {
    json!({
        "videos": {"used": user_count(dir, "h264"), "max": MAX_USER_VIDEOS},
        "images": {"used": user_count(dir, "png"), "max": MAX_USER_IMAGES},
    })
}

pub fn upload(dir: &Path, bytes: &[u8], name: Option<&str>) -> Result<Value> {
    if looks_like_mp4(bytes) {
        return upload_video(dir, bytes, name);
    }
    if user_count(dir, "png") >= MAX_USER_IMAGES {
        bail!(
            "Image library is full ({MAX_USER_IMAGES} of your own images). Delete an unused image first."
        );
    }
    let png = normalize(bytes)?;
    let id = format!(
        "image-{}.png",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    crate::settings::atomic_write(&dir.join(&id), &png)?;
    save_name(dir, &id, name);
    // Best effort: the thumbnail endpoint makes one on demand if this fails.
    let _ = image_thumbnail(&dir.join(&id), &thumb_path(dir, &id));
    Ok(json!({"id":id,"url":format!("/api/images/{id}"),"kind":"image"}))
}
/// Built-ins first (in `BUILTINS` order), then uploads, newest first.
pub fn list(dir: &Path) -> Vec<Value> {
    let mut entries: Vec<(usize, u64, Value)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if !crate::settings::valid_id(&id) {
                return None;
            }
            let path = e.path();
            let video = is_video_id(&id);
            let meta = fs::metadata(&path).ok()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let builtin = BUILTINS.iter().position(|(b, _)| *b == id);
            let name = match builtin {
                Some(i) => BUILTINS[i].1.to_owned(),
                None => fs::read(meta_path(dir, &id))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
                    .map(|m| m.name)
                    .unwrap_or_else(|| if video { "Your video" } else { "Your image" }.into()),
            };
            let (width, height, fps) = if video {
                let info = video_info(&path);
                (Some(info.width), Some(info.height), Some(info.fps))
            } else {
                let size = image::image_dimensions(&path).ok();
                (size.map(|s| s.0), size.map(|s| s.1), None)
            };
            let entry = json!({
                "id": id,
                "url": format!("/api/images/{id}"),
                "thumbnail": format!("/api/images/{id}/thumbnail"),
                "kind": if video { "video" } else { "image" },
                "name": name,
                "builtin": builtin.is_some(),
                "size": meta.len(),
                "modified": modified,
                "width": width,
                "height": height,
                "fps": fps,
            });
            Some((builtin.unwrap_or(usize::MAX), modified, entry))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    entries.into_iter().map(|(_, _, v)| v).collect()
}
/// The entry's list thumbnail, making it first if it doesn't exist yet: a
/// downscaled JPEG for an image, a decoded frame for a video. `None` if the
/// entry is unknown or no thumbnail can be made (a placeholder shows then).
pub fn thumbnail(dir: &Path, id: &str) -> Option<PathBuf> {
    if !crate::settings::valid_id(id) {
        return None;
    }
    let source = dir.join(id);
    if !source.is_file() {
        return None;
    }
    let thumb = thumb_path(dir, id);
    if thumb.is_file() {
        return Some(thumb);
    }
    // Don't retry a source that just failed on every list view; do retry
    // after a while, in case it failed for a passing reason (a busy Pi).
    let failed = thumb.with_extension("failed");
    if fs::metadata(&failed)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(600))
    {
        return None;
    }
    fs::create_dir_all(thumb.parent()?).ok()?;
    let made = if is_video_id(id) {
        // One decode at a time, however many posters a list view asks for.
        static DECODING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one_at_a_time = DECODING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        thumb.is_file() || h264_poster(&source, &thumb)
    } else {
        image_thumbnail(&source, &thumb)
    };
    // Recreated rather than rewritten, so its age always counts from now.
    let _ = fs::remove_file(&failed);
    if made {
        Some(thumb)
    } else {
        let _ = fs::write(&failed, b"");
        None
    }
}
/// Downscales a stored image to a `THUMB_WIDTH`-wide JPEG.
fn image_thumbnail(source: &Path, dest: &Path) -> bool {
    let Ok(img) = image::open(source) else {
        return false;
    };
    let small = img.thumbnail(THUMB_WIDTH, THUMB_WIDTH).to_rgb8();
    let mut out = Cursor::new(Vec::new());
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&small)
        .is_err()
    {
        return false;
    }
    dest.parent().is_some_and(|p| fs::create_dir_all(p).is_ok())
        && crate::settings::atomic_write(dest, &out.into_inner()).is_ok()
}
/// One decoded frame of a stored (raw Annex-B) H.264 loop as a JPEG, via a
/// software GStreamer decode. mpv's frame grab (`storage::ensure_thumbnail`)
/// can't open a raw .h264 — confirmed on the Pi's mpv 0.40 — so this is the
/// fallback for uploads and the way to give older uploads a poster.
fn h264_poster(source: &Path, dest: &Path) -> bool {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = dest.with_extension(format!("part-{}-{stamp}", std::process::id()));
    let ok = Command::new("nice")
        // A poster must never compete with live video for CPU.
        .args([
            "-n",
            "10",
            "timeout",
            "30",
            "gst-launch-1.0",
            "-q",
            "filesrc",
        ])
        .arg(format!("location={}", source.display()))
        .args([
            "!",
            "h264parse",
            "!",
            "avdec_h264",
            "!",
            "videoconvert",
            "!",
        ])
        .args(["videoscale", "!"])
        .arg(format!(
            "video/x-raw,width={THUMB_WIDTH},pixel-aspect-ratio=1/1"
        ))
        .args(["!", "jpegenc", "snapshot=true", "!", "filesink"])
        .arg(format!("location={}", part.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
        && fs::metadata(&part).is_ok_and(|m| m.len() > 0)
        && fs::rename(&part, dest).is_ok();
    let _ = fs::remove_file(&part);
    ok
}
// ---------------------------------------------------------------------------
// Looped fallback videos.
//
// An uploaded MP4 is demuxed once, without re-encoding, into an Annex-B H.264
// elementary stream with SPS/PPS before every keyframe. Playback then feeds it
// through exactly the same decoder pipeline as the live goggles stream.
// ---------------------------------------------------------------------------

pub const MAX_VIDEO_UPLOAD: usize = 64 * 1024 * 1024;
const CONVERT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}
impl VideoInfo {
    pub fn frame_duration(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.fps)
    }
}

pub fn is_video_id(id: &str) -> bool {
    id.ends_with(".h264")
}
/// ISO base media files (MP4, MOV) carry an `ftyp` box at offset 4.
pub fn looks_like_mp4(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[4..8] == b"ftyp"
}
fn sidecar(video: &Path) -> std::path::PathBuf {
    video.with_extension("json")
}
/// Timing and size recorded at upload. Missing or damaged metadata falls back
/// to 30 fps so an older or hand-copied file still plays.
pub fn video_info(video: &Path) -> VideoInfo {
    fs::read(sidecar(video))
        .ok()
        .and_then(|b| serde_json::from_slice::<VideoInfo>(&b).ok())
        .filter(|i| (1.0..=120.0).contains(&i.fps))
        .unwrap_or(VideoInfo {
            width: 1920,
            height: 1080,
            fps: 30.0,
        })
}
pub fn remove(dir: &Path, id: &str) -> Result<()> {
    if is_builtin(id) {
        bail!("Built-in idle screens can't be deleted");
    }
    let path = dir.join(id);
    fs::remove_file(&path)?;
    if is_video_id(id) {
        let _ = fs::remove_file(sidecar(&path));
    }
    let thumb = thumb_path(dir, id);
    let _ = fs::remove_file(thumb.with_extension("failed"));
    let _ = fs::remove_file(thumb);
    let _ = fs::remove_file(meta_path(dir, id));
    Ok(())
}
fn upload_video(dir: &Path, bytes: &[u8], name: Option<&str>) -> Result<Value> {
    if bytes.len() > MAX_VIDEO_UPLOAD {
        bail!("Video must be smaller than 64 MiB");
    }
    if user_count(dir, "h264") >= MAX_USER_VIDEOS {
        bail!(
            "Video library is full ({MAX_USER_VIDEOS} of your own videos). Delete an unused video first."
        );
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let id = format!("video-{stamp}.h264");
    let input = dir.join(format!(".upload-{stamp}.mp4"));
    let part = dir.join(format!(".upload-{stamp}.part"));
    let result = (|| -> Result<VideoInfo> {
        fs::write(&input, bytes)?;
        let info = convert_video(&input, &part)?;
        fs::File::open(&part)?.sync_all()?;
        crate::settings::atomic_write(&sidecar(&dir.join(&id)), &serde_json::to_vec(&info)?)?;
        fs::rename(&part, dir.join(&id))?;
        fs::File::open(dir)?.sync_all()?;
        Ok(info)
    })();
    // The list poster comes from the original MP4 while it still exists:
    // mpv's frame grab handles MP4 (checked on the Pi) but not the raw .h264
    // kept afterwards, which only the slower GStreamer fallback can read.
    if result.is_ok() {
        let thumb = thumb_path(dir, &id);
        let _ = fs::create_dir_all(dir.join("thumbs"));
        if !crate::storage::ensure_thumbnail_sized(&input, &thumb, THUMB_WIDTH) {
            let _ = h264_poster(&dir.join(&id), &thumb);
        }
        save_name(dir, &id, name);
    }
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&part);
    let info = result?;
    Ok(json!({"id":id,"url":format!("/api/images/{id}"),"kind":"video","video":info}))
}
/// Demux the first video track to Annex-B H.264 with GStreamer. No pixels are
/// decoded or re-encoded, so this is fast even on the Pi and keeps quality.
fn convert_video(input: &Path, output: &Path) -> Result<VideoInfo> {
    let mut child = Command::new("gst-launch-1.0")
        .arg("-v")
        .arg("filesrc")
        .arg(format!("location={}", input.display()))
        // No queue between demuxer and parser: with one, gst-launch can hang
        // instead of exiting after a link error (e.g. an HEVC upload).
        .args(["!", "qtdemux", "name=d", "d.video_0", "!"])
        .args(["h264parse", "config-interval=-1", "!"])
        .args(["video/x-h264,stream-format=byte-stream,alignment=au", "!"])
        .arg("filesink")
        .arg(format!("location={}", output.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start GStreamer for video conversion")?;
    let failed = Arc::new(AtomicBool::new(false));
    let collect = |pipe: Box<dyn Read + Send>| {
        let failed = failed.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if line.starts_with("ERROR") {
                    failed.store(true, Ordering::Relaxed);
                }
                text.push_str(&line);
                text.push('\n');
            }
            text
        })
    };
    let out = collect(Box::new(child.stdout.take().unwrap()));
    let err = collect(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let mut failed_at: Option<Instant> = None;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        // Never wait out the full timeout once GStreamer has reported an error.
        if failed.load(Ordering::Relaxed)
            && failed_at.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(2)
        {
            let _ = child.kill();
            break child.wait()?;
        }
        if started.elapsed() > CONVERT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "Video conversion took longer than {} s",
                CONVERT_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = format!(
        "{}\n{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    if !status.success() {
        if log.contains("not-linked") {
            bail!(
                "The video is not H.264. Export it as MP4 with H.264 (not HEVC/H.265), \
                 for example with HandBrake."
            );
        }
        if log.contains("no playable streams") || log.contains("not supported") {
            bail!("This file has no playable video track");
        }
        bail!("Video conversion failed");
    }
    let (width, height, fps) = log
        .lines()
        .find_map(crate::video::input_caps)
        .context("Could not read the video format")?;
    if width > 1920 || height > 1080 {
        bail!("Video is {width}×{height}; use at most 1920×1080 (1080p)");
    }
    if fs::metadata(output).map(|m| m.len()).unwrap_or(0) == 0 {
        bail!("The video track is empty");
    }
    Ok(VideoInfo {
        width,
        height,
        fps: fps.filter(|f| (1.0..=120.0).contains(f)).unwrap_or(30.0),
    })
}

pub fn pixels(path: Option<&Path>, width: u32, height: u32) -> Vec<u8> {
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([16, 18, 22, 255]));
    if let Some(img) = path.and_then(|p| image::open(p).ok()) {
        let scaled = img.resize(width, height, image::imageops::FilterType::Triangle);
        let (w, h) = scaled.dimensions();
        image::imageops::overlay(
            &mut canvas,
            &scaled,
            ((width - w) / 2).into(),
            ((height - h) / 2).into(),
        );
    }
    // DRM XRGB8888 is B,G,R,X in little endian memory.
    let mut data = canvas.into_raw();
    for p in data.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        p[3] = 255;
    }
    data
}
/// Pre-installed on every Raspberry Pi OS image (a `fontconfig` dependency
/// pulled in regardless, so this never needs bundling or downloading);
/// missing only on a non-standard rootfs, where the caller just gets a plain
/// background instead of a crash.
const STATUS_FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}
fn draw_line(
    canvas: &mut image::RgbaImage,
    font: &ab_glyph::FontRef,
    scale: f32,
    text: &str,
    y: f32,
) {
    use ab_glyph::{Font, ScaleFont, point};
    let scaled = font.as_scaled(scale);
    let total_w: f32 = text
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let mut x = (canvas.width() as f32 - total_w) / 2.0;
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(x, y));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (
                    bounds.min.x as i32 + gx as i32,
                    bounds.min.y as i32 + gy as i32,
                );
                if px < 0 || py < 0 || px as u32 >= canvas.width() || py as u32 >= canvas.height() {
                    return;
                }
                let bg = canvas.get_pixel(px as u32, py as u32).0;
                canvas.put_pixel(
                    px as u32,
                    py as u32,
                    image::Rgba([
                        lerp(bg[0], 235, coverage),
                        lerp(bg[1], 235, coverage),
                        lerp(bg[2], 235, coverage),
                        255,
                    ]),
                );
            });
        }
        x += scaled.h_advance(id);
    }
}
/// Renders a centered block of status lines (current Wi-Fi mode, SSID, IP —
/// see `wifi::client_status`) onto a plain background, as a PNG ready for
/// the same fallback-image path any other image takes (`display::pixels`
/// scales it to the connected screen's real resolution). `None` when the
/// font isn't where every Raspberry Pi OS image keeps it; the caller falls
/// back to showing nothing rather than a half-drawn image.
pub fn render_network_status(lines: &[String]) -> Option<Vec<u8>> {
    let font_bytes = fs::read(STATUS_FONT).ok()?;
    let font = ab_glyph::FontRef::try_from_slice(&font_bytes).ok()?;
    let (width, height) = (1920u32, 1080u32);
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([16, 18, 22, 255]));
    let scale = 64.0;
    let line_height = scale * 1.4;
    let start_y = (height as f32 - line_height * lines.len() as f32) / 2.0 + scale;
    for (i, line) in lines.iter().enumerate() {
        draw_line(
            &mut canvas,
            &font,
            scale,
            line,
            start_y + line_height * i as f32,
        );
    }
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_upload_and_letterboxes() {
        assert!(normalize(b"not an image").is_err());
        assert_eq!(pixels(None, 2, 1), [22, 18, 16, 255, 22, 18, 16, 255]);
        let source = image::DynamicImage::new_rgb8(3840, 2160);
        let mut b = Cursor::new(Vec::new());
        source.write_to(&mut b, image::ImageFormat::Png).unwrap();
        let small = image::load_from_memory(&normalize(b.get_ref()).unwrap()).unwrap();
        assert_eq!(small.dimensions(), (1920, 1080));
    }
    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ungoggled-assets-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("thumbs")).unwrap();
        dir
    }
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut b, image::ImageFormat::Png)
            .unwrap();
        b.into_inner()
    }
    #[test]
    fn upload_names_are_reduced_to_a_plain_file_name() {
        assert_eq!(
            clean_name("C:\\fakepath\\clip.mp4").as_deref(),
            Some("clip.mp4")
        );
        assert_eq!(clean_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(clean_name("a\nb\tc.png").as_deref(), Some("abc.png"));
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(&"x".repeat(200)).unwrap().len(), 80);
    }
    #[test]
    fn builtins_come_first_with_names_and_cannot_be_deleted() {
        let dir = scratch("builtins");
        fs::write(dir.join("no-signal.png"), png(64, 36)).unwrap();
        fs::write(dir.join("default-loop.h264"), b"not really h264").unwrap();
        let uploaded = upload(&dir, &png(1280, 720), Some("Club banner.png")).unwrap();
        let id = uploaded["id"].as_str().unwrap().to_owned();
        let listed = list(&dir);
        let ids: Vec<&str> = listed.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["default-loop.h264", "no-signal.png", id.as_str()]);
        assert_eq!(listed[0]["name"], "Logo loop");
        assert_eq!(listed[0]["kind"], "video");
        assert_eq!(listed[0]["builtin"], true);
        assert_eq!(listed[1]["name"], "No signal");
        assert_eq!(listed[2]["name"], "Club banner.png");
        assert_eq!(listed[2]["builtin"], false);
        // Uploads are stored scaled to fit 1920 × 1080 (see `normalize`).
        assert_eq!(listed[2]["width"], 1920);
        assert_eq!(listed[2]["height"], 1080);
        assert!(remove(&dir, "no-signal.png").is_err());
        assert!(remove(&dir, "default-loop.h264").is_err());
        assert!(dir.join("no-signal.png").exists());
        remove(&dir, &id).unwrap();
        assert!(!dir.join(&id).exists());
        assert!(!thumb_path(&dir, &id).exists());
        assert!(!meta_path(&dir, &id).exists());
        fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn upload_limits_count_only_your_own_files() {
        let dir = scratch("limits");
        for (id, _) in BUILTINS {
            fs::write(dir.join(id), b"x").unwrap();
        }
        let l = limits(&dir);
        assert_eq!(l["videos"]["used"], 0);
        assert_eq!(l["videos"]["max"], MAX_USER_VIDEOS);
        assert_eq!(l["images"]["used"], 0);
        upload(&dir, &png(32, 32), None).unwrap();
        assert_eq!(limits(&dir)["images"]["used"], 1);
        fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn image_thumbnails_are_small_jpegs_made_at_upload_or_on_demand() {
        let dir = scratch("thumbs");
        let uploaded = upload(&dir, &png(3840, 2160), None).unwrap();
        let id = uploaded["id"].as_str().unwrap();
        let thumb = thumb_path(&dir, id);
        let img = image::load_from_memory(&fs::read(&thumb).unwrap()).unwrap();
        assert_eq!(img.dimensions(), (320, 180));
        assert_eq!(&fs::read(&thumb).unwrap()[..2], &[0xff, 0xd8], "JPEG");
        // An image from before thumbnails existed gets one on first request.
        fs::remove_file(&thumb).unwrap();
        assert_eq!(thumbnail(&dir, id).as_deref(), Some(thumb.as_path()));
        assert!(thumbnail(&dir, "../etc.png").is_none());
        assert!(thumbnail(&dir, "missing.png").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn renders_network_status_as_a_valid_image() {
        let png =
            render_network_status(&["Verbunden mit: TestNetz".into(), "IP: 192.168.1.42".into()])
                .expect("DejaVu Sans Bold is present on every Raspberry Pi OS image");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(img.dimensions(), (1920, 1080));
        // Drawing something changed at least one pixel away from the plain background.
        assert!(
            img.to_rgba8()
                .pixels()
                .any(|p| *p != image::Rgba([16, 18, 22, 255]))
        );
    }
}
