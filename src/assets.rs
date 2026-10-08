use anyhow::{Context, Result, bail};
use image::{GenericImageView, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Cursor, Read},
    path::Path,
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
pub fn upload(dir: &Path, bytes: &[u8]) -> Result<Value> {
    if looks_like_mp4(bytes) {
        return upload_video(dir, bytes);
    }
    if fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|f| f.path().extension().is_some_and(|e| e == "png"))
        .count()
        >= 16
    {
        bail!("Image library is full (16 images). Delete an unused image first.");
    }
    let png = normalize(bytes)?;
    let id = format!(
        "image-{}.png",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    crate::settings::atomic_write(&dir.join(&id), &png)?;
    Ok(json!({"id":id,"url":format!("/api/images/{id}")}))
}
pub fn list(dir: &Path) -> Vec<Value> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if !crate::settings::valid_id(&id) {
                return None;
            }
            let kind = if is_video_id(&id) { "video" } else { "image" };
            Some(json!({"id":id,"url":format!("/api/images/{id}"),"kind":kind}))
        })
        .collect();
    entries.sort_by_key(|v| v["id"].as_str().unwrap().to_owned());
    entries
}
// ---------------------------------------------------------------------------
// Looped fallback videos.
//
// An uploaded MP4 is demuxed once, without re-encoding, into an Annex-B H.264
// elementary stream with SPS/PPS before every keyframe. Playback then feeds it
// through exactly the same decoder pipeline as the live goggles stream.
// ---------------------------------------------------------------------------

pub const MAX_VIDEO_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_VIDEOS: usize = 4;
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
    let path = dir.join(id);
    fs::remove_file(&path)?;
    if is_video_id(id) {
        let _ = fs::remove_file(sidecar(&path));
    }
    Ok(())
}
fn upload_video(dir: &Path, bytes: &[u8]) -> Result<Value> {
    if bytes.len() > MAX_VIDEO_UPLOAD {
        bail!("Video must be smaller than 64 MiB");
    }
    let count = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|f| f.path().extension().is_some_and(|e| e == "h264"))
        .count();
    if count >= MAX_VIDEOS {
        bail!("Video library is full ({MAX_VIDEOS} videos). Delete an unused video first.");
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
}
