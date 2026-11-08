//! One display owner for the lifetime of the service. USB workers only deliver
//! compressed bytes; reconnecting them never removes the fallback framebuffer.
use crate::{WorkerArgs, playspeed::PlaySpeed, settings::Store};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            net::{UnixDatagram, UnixStream},
            process::CommandExt,
        },
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
pub type Report = Arc<dyn Fn(Value) + Send + Sync>;
pub type Preview = Arc<Mutex<Option<(Instant, Vec<u8>)>>>;

// Sequence every datagram so a full socket cannot silently corrupt an H.264 NAL.
pub struct Output {
    socket: Option<UnixDatagram>,
    sequence: AtomicU64,
}
impl Output {
    pub fn connect(path: &Path) -> Self {
        let socket = UnixDatagram::unbound()
            .and_then(|s| {
                s.connect(path)?;
                s.set_nonblocking(true)?;
                Ok(s)
            })
            .ok();
        Self {
            socket,
            sequence: AtomicU64::new(0),
        }
    }
    pub fn send(&self, payload: Vec<u8>) {
        let Some(socket) = &self.socket else {
            return;
        };
        for bytes in payload.chunks(32 * 1024) {
            let mut packet = self
                .sequence
                .fetch_add(1, Ordering::Relaxed)
                .to_le_bytes()
                .to_vec();
            packet.extend_from_slice(bytes);
            let _ = socket.send(&packet);
        }
    }
}
struct Chunk {
    data: Vec<u8>,
    received: Instant,
}
/// One-shot playback of a file from a USB mass-storage drive, as opposed to
/// the live goggles decoder or the looping fallback video. Unlike those
/// gst-launch pipelines, this one is controllable at runtime (pause/seek)
/// over mpv's JSON IPC socket, so the web interface can offer a scrub bar.
struct UsbPlayer {
    child: Child,
}
impl UsbPlayer {
    /// True once mpv has exited. With `--idle=yes` that only happens on a
    /// real error or an explicit `quit` command, never just from reaching
    /// the end of the file, so a finished clip still answers IPC queries.
    fn finished(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }
    /// True once mpv has played past the last frame. `--idle=yes` keeps it
    /// running at that point rather than exiting (so `finished` alone never
    /// catches this) — without this check, a clip left to play to the end
    /// rather than being stopped from the web UI left the render loop
    /// standing aside for USB playback forever, never falling through to
    /// the idle screen again until the Pi was restarted (confirmed in the
    /// field, 2026-11-01).
    fn eof(&self, socket: &Path) -> bool {
        mpv_command(socket, json!(["get_property", "eof-reached"]))
            .ok()
            .and_then(|r| r.get("data").and_then(Value::as_bool))
            .unwrap_or(false)
    }
}
impl Drop for UsbPlayer {
    fn drop(&mut self) {
        // A plain kill: the kernel frees mpv's DRM framebuffers and its
        // V4L2/CMA buffers when the process dies. (A graceful IPC `quit` was
        // tried on 2026-11-02 against a "CMA leak" that turned out to be page
        // cache; see FINDINGS.MD.)
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
/// Fixed path for mpv's control socket while a USB video plays. Fixed rather
/// than per-session because only one USB video ever plays at a time.
pub fn usb_mpv_socket(video_socket: &Path) -> std::path::PathBuf {
    video_socket.with_file_name("usb-mpv.sock")
}
/// Send one JSON IPC command to mpv and return its reply. `command` is the
/// command array, e.g. `json!(["get_property", "time-pos"])`. See
/// <https://github.com/mpv-player/mpv/blob/master/DOCS/man/ipc.rst>.
pub fn mpv_command(socket: &Path, command: Value) -> anyhow::Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = serde_json::to_vec(&json!({"command": command}))?;
    request.push(b'\n');
    stream.write_all(&request)?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(serde_json::from_str(&reply)?)
}
/// The connector name mpv's `--drm-connector` wants (e.g. "HDMI-A-1") for
/// the numeric KMS connector id `kmssink`'s `connector-id` uses elsewhere in
/// this file; `None` leaves mpv to auto-pick the connected connector.
fn drm_connector_name(id: u32) -> Option<String> {
    std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .find_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let found: u32 = std::fs::read_to_string(e.path().join("connector_id"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            (found == id).then(|| {
                name.split_once('-')
                    .map_or(name.clone(), |(_, rest)| rest.to_owned())
            })
        })
}
/// One poll of mpv's clock for the speed readout of a USB video (see
/// `playspeed`): the status fields to publish, or `None` while there is no
/// reading yet (just started, just seeked, mpv not answering). `output_fps` is
/// filled too, so the history graph and the HDMI card show what reaches the
/// screen.
fn usb_speed_status(speed: &mut PlaySpeed, socket: &Path) -> Option<Value> {
    let get = |prop: &str| {
        mpv_command(socket, json!(["get_property", prop]))
            .ok()
            .and_then(|r| r.get("data").cloned())
    };
    let position = get("time-pos").and_then(|v| v.as_f64());
    let paused = get("pause").and_then(|v| v.as_bool()).unwrap_or(false);
    let clip_fps = get("container-fps").and_then(|v| v.as_f64());
    let reading = speed.update(Instant::now(), position, paused, clip_fps)?;
    Some(json!({
        "usb_speed": reading.speed,
        "usb_fps": reading.fps,
        "usb_clip_fps": clip_fps,
        "output_fps": reading.fps.unwrap_or(0.0),
    }))
}
/// The same fields emptied: no USB video is playing (any more).
fn usb_speed_cleared() -> Value {
    json!({"usb_speed":null,"usb_fps":null,"usb_clip_fps":null})
}
/// `drm_mode`: the HDMI mode already on screen (`Display::mpv_mode`), kept
/// for playback; `None` lets mpv pick the monitor's preferred mode.
fn spawn_usb_player(
    args: &WorkerArgs,
    path: &Path,
    drm_mode: Option<&str>,
    stretch: bool,
) -> anyhow::Result<UsbPlayer> {
    let socket = usb_mpv_socket(&args.video_socket);
    let _ = std::fs::remove_file(&socket);
    let mut cmd = Command::new("mpv");
    // Not --no-terminal: that silences every message, which is how a video
    // output that failed to start went unnoticed (mpv then just plays the
    // audio track). mpv writes its messages to stdout, inherited into the
    // journal below; --quiet drops the \r-redrawn status line.
    cmd.args([
        "--no-input-terminal",
        "--quiet",
        "--no-msg-color",
        "--idle=yes",
        // Action-cam footage beyond the Pi 4's decode throughput (a DJI
        // Action 2's 4096x3072 60 fps mode decodes at ~34 frames/s while it
        // is also being scanned out, ~37 with the small draw plane below,
        // ~53 at most with nothing on screen at the stock
        // 500 MHz HEVC clock) cannot play in real time: every frame
        // is a reference frame, so nothing can be skipped before decoding.
        // With audio as the master clock the video falls further behind
        // every second (A/V desync, then a full demuxer queue), and dropping
        // frames at the video output only makes it choppier without
        // speeding up decoding (measured 2026-11-06, FINDINGS.MD). Without
        // audio mpv shows every frame, slower but smooth. This is HDMI
        // review playback (the audio would only reach the 3.5 mm jack), so
        // that is the right trade.
        "--audio=no",
    ]);
    // "vd" logs which decoder/hwdec mpv actually opened, "vo" the DRM
    // output's mode, plane and hardware-frame-import choices.
    cmd.args(["--msg-level=vd=v,vo=v"]);
    cmd.arg(format!("--input-ipc-server={}", socket.display()));
    if args.output == "hdmi" {
        // Zero-copy: decoded frames go straight to a display (HVS) plane,
        // which also scales them. `--vo=drm` can only scale in software
        // after copying every frame back into RAM: measured on a Pi 4 with a
        // DJI Osmo Nano 3840x2880 50 fps 10-bit HEVC clip, that played at
        // 0.14x speed (~6 frames/s on screen, very choppy, yet mpv reported
        // no dropped frames), vs. 0.86x with every frame shown here. 30 fps
        // 4K HEVC and 1080p60 H.264 play in real time. See FINDINGS.MD.
        cmd.args([
            "--vo=gpu",
            "--gpu-context=drm",
            "--gpu-hwdec-interop=drmprime-overlay",
            // By default the video goes on an overlay plane at zpos 0,
            // below the primary plane, which vc4 rejects: every atomic
            // commit failed with EINVAL. Swap the planes' roles instead.
            "--drm-drmprime-video-plane=primary",
            "--drm-draw-plane=overlay",
            // The draw plane only carries mpv's OSD here (unused), yet at the
            // output size it is a full-screen ARGB buffer that the GPU redraws
            // and the display reads every refresh. That memory traffic slows
            // the HEVC decoder: a quarter-size surface (scaled up by the
            // display) gave 0.59x -> 0.62x for a DJI Action 2 4096x3072 clip,
            // 0.79x -> 0.82x for an O4 3840x2880 clip (2026-11-07,
            // FINDINGS.MD). Not smaller: a clip mpv has to decode in software
            // is rendered into this plane at this size. (A line-skip zoom that
            // went with it was dropped: it falsifies the colours, FINDINGS.MD.)
            "--drm-draw-surface-size=960x540",
            // HEVC on the stateless rpi-hevc-dec, H.264 on the stateful
            // bcm2835-codec, both as zero-copy DRM PRIME frames; anything
            // else falls back to software decoding.
            "--hwdec=drm,v4l2m2m",
            // Frame threading keeps several frames in flight in the HEVC
            // decoder: ~2x the throughput of the default single thread,
            // bit-exact with software decoding.
            "--vd-lavc-o=threads=4",
            // A clip the decoder can't quite sustain (4K at 50 fps) plays a
            // little slower but smoothly, instead of mpv dropping most
            // frames to keep up with the audio clock.
            "--framedrop=no",
            // Cheap GPU rendering for anything decoded in software.
            "--profile=fast",
            // The display plane can't rotate by 90/270 degrees, so mpv
            // honouring a clip's rotation flag (an action cam mounted
            // sideways) squeezed the unrotated picture into a portrait
            // rectangle. Show such clips as recorded, undistorted.
            "--video-rotate=no",
        ]);
        if stretch {
            // Fill the whole screen instead of keeping the clip's shape.
            cmd.arg("--keepaspect=no");
        }
        if let Some(mode) = drm_mode {
            cmd.arg(format!("--drm-mode={mode}"));
        }
        if let Some(id) = args.connector
            && let Some(name) = drm_connector_name(id)
        {
            cmd.arg(format!("--drm-connector={name}"));
        }
    } else {
        cmd.args(["--hwdec=auto", "--vo=null"]);
    }
    cmd.arg(path);
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!(
                "Cannot start mpv ({e}). Is it installed? `apt install mpv`, or re-flash \
                 the SD card with an image that includes it."
            )
        })?;
    Ok(UsbPlayer { child })
}
struct Player {
    child: Child,
    heartbeat: Arc<Mutex<Option<Instant>>>,
    active: Arc<AtomicBool>,
}
impl Drop for Player {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Whose frames a decoder pipeline shows. Only the live goggles stream reports
/// input format, HDMI playback state and preview frames.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Live,
    Fallback,
}

/// A fallback video fed in an endless loop through the live decoder pipeline,
/// paced at its recorded frame rate as if it came from the goggles.
struct Loop {
    stop: Arc<AtomicBool>,
    feeder: Option<thread::JoinHandle<()>>,
    player: Option<Player>,
    started: Instant,
}
impl Loop {
    /// False once the pipeline exited or stopped rendering, so it is restarted.
    fn healthy(&mut self) -> bool {
        let Some(p) = self.player.as_mut() else {
            return false;
        };
        if p.child.try_wait().ok().flatten().is_some() {
            return false;
        }
        if self.feeder.as_ref().is_some_and(|f| f.is_finished()) {
            return false;
        }
        let beat = *p.heartbeat.lock().unwrap();
        // The first pipeline after boot loads its plugins from a cold SD card.
        let grace = self.started.elapsed() < Duration::from_secs(10);
        grace || beat.is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }
}
impl Drop for Loop {
    fn drop(&mut self) {
        // Stop the feeder first, then kill and reap the decoder. Only after
        // this returns is the hardware decoder and HDMI overlay free again.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(f) = self.feeder.take() {
            let _ = f.join();
        }
        self.player = None;
    }
}

/// Split an Annex-B file into NAL units (each including its start code).
fn nal_units(bytes: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut annex = crate::h264::AnnexB::default();
    let mut units = annex.feed(bytes).map_err(anyhow::Error::msg)?;
    // A trailing start code flushes the final unit.
    units.extend(annex.feed(&[0, 0, 1]).map_err(anyhow::Error::msg)?);
    if !units.iter().any(|n| n.get(3).map(|b| b & 31) == Some(7)) {
        anyhow::bail!("Fallback video has no SPS");
    }
    Ok(units)
}
/// The first slice of a picture starts a new frame: VCL NAL (1 or 5) with
/// first_mb_in_slice == 0, which is a leading 1 bit in Exp-Golomb coding.
fn starts_frame(nal: &[u8]) -> bool {
    matches!(nal.get(3).map(|v| v & 31), Some(1 | 5)) && nal.get(4).is_some_and(|v| v & 0x80 != 0)
}

fn spawn_loop(args: &WorkerArgs, video: &Path, report: Report) -> anyhow::Result<Loop> {
    let info = crate::assets::video_info(video);
    let units = nal_units(&std::fs::read(video)?)?;
    let mut player = spawn_player(args, false, report, Arc::default(), Role::Fallback, None)?;
    let mut stdin = player
        .child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Decoder input missing"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let frame = info.frame_duration();
    let feeder = thread::spawn(move || {
        let mut deadline = Instant::now();
        loop {
            for nal in &units {
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                if starts_frame(nal) {
                    let now = Instant::now();
                    // After a stall, resume at real time instead of racing to catch up.
                    if now > deadline + Duration::from_millis(500) {
                        deadline = now;
                    }
                    while Instant::now() < deadline {
                        if stopped.load(Ordering::Relaxed) {
                            return;
                        }
                        let left = deadline.saturating_duration_since(Instant::now());
                        thread::sleep(left.min(Duration::from_millis(20)));
                    }
                    deadline += frame;
                }
                if write_deadline(&mut stdin, nal).is_err() {
                    return;
                }
            }
        }
    });
    Ok(Loop {
        stop,
        feeder: Some(feeder),
        player: Some(player),
        started: Instant::now(),
    })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}
/// The pixel aspect ratio that makes a `w`x`h` picture fill a screen of
/// `out_w`x`out_h` pixels when the display sink keeps the picture's own shape
/// (kmssink has no "stretch" switch, but it honours this ratio). `None` when
/// the picture already has the screen's shape (within 1 %) or the numbers are
/// implausible (a stretch of more than 4x in either direction).
fn fill_par(w: u32, h: u32, out_w: u32, out_h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return None;
    }
    let (n, d) = (
        u64::from(out_w) * u64::from(h),
        u64::from(out_h) * u64::from(w),
    );
    let g = gcd(n, d);
    let (n, d) = (n / g, d / g);
    if n.abs_diff(d) * 100 <= d || n > d * 4 || d > n * 4 {
        return None;
    }
    Some((u32::try_from(n).ok()?, u32::try_from(d).ok()?))
}
/// Whether a change from `current` to `next` settings has to rebuild the
/// output right now (stopping the live decoder, the idle loop and the display
/// with it). Settings this loop never reads (`terminal_enabled`) or applies
/// another way (`stretch_usb`) don't count.
/// A different idle screen alone waits while there is a `live` picture:
/// rebuilding the display under live video would interrupt it just to change
/// what is shown once there is none (the same reasoning as the disk-full
/// notice in `start`); it is applied as soon as the live picture ends.
fn must_rebuild(
    current: &crate::settings::Settings,
    next: &crate::settings::Settings,
    live: bool,
) -> bool {
    let video = |s: &crate::settings::Settings| crate::settings::Settings {
        terminal_enabled: false,
        // Applied to a running USB player over its IPC socket, or when the
        // next clip starts; no reason to tear down the live decoder.
        stretch_usb: false,
        fallback_image: None,
        ..s.clone()
    };
    video(current) != video(next) || (current.fallback_image != next.fallback_image && !live)
}
// Each parameter is independent shared state the render thread owns for its
// whole lifetime; grouping them into a struct wouldn't make any of them less
// of a parameter, just hide the same count behind one more layer.
/// The USB drive mounted for `record_to_usb`, if the setting is on and one
/// is currently plugged in, else the SD card's `recordings/` directory.
fn recordings_dir(
    store: &Store,
    storage: &crate::storage::Store,
    current: &crate::settings::Settings,
) -> std::path::PathBuf {
    if current.record_to_usb
        && let Some(mountpoint) = storage.recording_mountpoint()
    {
        mountpoint.join("ungoggled-recordings")
    } else {
        store.dir.join("recordings")
    }
}
#[allow(clippy::too_many_arguments)]
pub fn start(
    args: WorkerArgs,
    store: Store,
    storage: crate::storage::Store,
    report: Report,
    preview: Preview,
    shutdown: Arc<AtomicBool>,
    usb_playback: Arc<Mutex<Option<std::path::PathBuf>>>,
    recording: crate::recording::Controller,
    disk_warning: Arc<Mutex<Option<std::path::PathBuf>>>,
    network_notice: Arc<Mutex<Option<std::path::PathBuf>>>,
    stream: crate::stream::Controller,
) -> anyhow::Result<thread::JoinHandle<()>> {
    if args.video_socket.exists() {
        std::fs::remove_file(&args.video_socket)?;
    }
    let warning_image = args.video_socket.with_file_name("sd-card-full.png");
    let _ = std::fs::write(&warning_image, include_bytes!("../assets/sd-card-full.png"));
    // tmpfs: a flight in progress lives in RAM, not on the SD card, until it
    // is finalized there once the feed drops.
    let auto_record_dir = args.video_socket.with_file_name("auto-recording");
    let socket = UnixDatagram::bind(&args.video_socket)?;
    socket.set_read_timeout(Some(Duration::from_millis(200)))?;
    let (tx, rx) = mpsc::sync_channel::<Chunk>(1024);
    let queued = Arc::new(AtomicUsize::new(0));
    let lost = Arc::new(AtomicBool::new(false));
    let receive_stop = shutdown.clone();
    let q = queued.clone();
    let loss = lost.clone();
    thread::spawn(move || receive(socket, tx, q, loss, receive_stop));
    Ok(thread::spawn(move || {
        // The CLI --connector is the ultimate fallback (mostly for testing);
        // the setting, when present, wins. Overwriting args.connector keeps
        // every existing use of it (Display::prepare, spawn_player,
        // spawn_loop, spawn_usb_player) in sync for free.
        let cli_connector = args.connector;
        let mut args = args;
        let mut child: Option<Player> = None;
        let mut fallback: Option<Loop> = None;
        let mut usb_player: Option<UsbPlayer> = None;
        let mut current_usb: Option<std::path::PathBuf> = None;
        let mut usb_speed = PlaySpeed::default();
        let mut usb_speed_at = Instant::now() - Duration::from_secs(1);
        let mut usb_speed_shown = false;
        // Whether the running USB player was started (or switched) to fill
        // the screen; follows the `stretch_usb` setting.
        let mut usb_stretched = false;
        let mut fallback_retry = Instant::now() - Duration::from_secs(5);
        let mut display: Option<crate::display::Display> = None;
        // The mode of the last display prepared for the current settings,
        // which USB playback keeps (the display itself is dropped for it).
        let mut display_mode: Option<String> = None;
        let mut current = store.get();
        args.connector = current.hdmi_connector.or(cli_connector);
        let mut annex = crate::h264::AnnexB::default();
        let mut parameters = crate::h264::Parameters::default();
        let mut last_input: Option<Instant> = None;
        let mut last_frame: Option<Instant> = None;
        let mut prepare_at = Instant::now() - Duration::from_secs(5);
        let mut stats_at = Instant::now();
        let mut frames = 0_u64;
        let mut starts = 0_u64;
        // The live decoder was started stretched, for a picture of this size,
        // and whether it has shown a frame since. The display chip refuses some
        // stretches (the HVS runs out of scaling memory: drmModeSetPlane fails
        // with ENOSPC, so the decoder never shows a picture); a picture size
        // that failed this way is shown unstretched from then on.
        let mut child_stretch: Option<(u32, u32)> = None;
        let mut child_showed = false;
        let mut stretch_rejected: Option<(u32, u32)> = None;
        let mut shown_warning: Option<std::path::PathBuf> = None;
        while !shutdown.load(Ordering::Relaxed) {
            let settings = store.get();
            let live = last_input.is_some_and(|t| t.elapsed() <= Duration::from_millis(1500));
            // `current` keeps the old settings until a rebuild is due, so a
            // deferred idle-screen change is picked up again on a later tick.
            if must_rebuild(&current, &settings, live) {
                child = None;
                fallback = None;
                display = None;
                display_mode = None;
                current = settings.clone();
                args.connector = current.hdmi_connector.or(cli_connector);
                child_stretch = None;
                stretch_rejected = None;
                annex = Default::default();
                *preview.lock().unwrap() = None;
                prepare_at = Instant::now() - Duration::from_secs(5);
            }
            if current.stream_enabled {
                stream.ensure(crate::stream::Quality::parse(&current.stream_quality));
            } else if stream.running() {
                stream.stop();
            }
            // Re-read every tick, not just on settings change: the USB drive
            // itself can appear or disappear independently of the setting.
            let recordings_dir = recordings_dir(&store, &storage, &current);
            if display.as_ref().is_some_and(|d| d.needs_upgrade()) {
                child = None;
                fallback = None;
                display = None;
                display_mode = None;
            }
            // Swap in the "SD card full" notice (or swap it back out) only
            // once there is no live input to glitch; rebuilding the display
            // while the goggles are still streaming would briefly interrupt
            // the live picture just to change what's underneath it.
            let warning = disk_warning
                .lock()
                .unwrap()
                .clone()
                .or_else(|| network_notice.lock().unwrap().clone());
            let input_gone_now =
                last_input.is_none_or(|t| t.elapsed() > Duration::from_millis(1500));
            if input_gone_now && warning != shown_warning {
                display = None;
                // Otherwise a fallback-video loop already running on its own
                // overlay plane would just keep covering the image this
                // warning/notice is about to show on the primary plane.
                fallback = None;
                fallback_retry = Instant::now();
                prepare_at = Instant::now() - Duration::from_secs(5);
                shown_warning = warning;
            }
            if args.output == "hdmi"
                && display.is_none()
                && current_usb.is_none()
                && prepare_at.elapsed() > Duration::from_secs(3)
            {
                prepare_at = Instant::now();
                // A fallback video plays on the overlay; the primary plane stays dark.
                let image = shown_warning.clone().or_else(|| {
                    current
                        .fallback_image
                        .as_ref()
                        .filter(|id| !crate::assets::is_video_id(id))
                        .map(|id| store.dir.join("images").join(id))
                });
                match crate::display::Display::prepare(
                    args.connector,
                    &current.hdmi_mode,
                    image.as_deref(),
                ) {
                    Ok(d) => {
                        report(d.info.clone());
                        display_mode = Some(d.mpv_mode.clone());
                        child = None;
                        display = Some(d);
                    }
                    Err(e) => {
                        display_mode = None;
                        report(json!({"hdmi":"unavailable","display_error":format!("{e:#}")}))
                    }
                }
            }
            // An explicit request to play a USB-drive video takes over the
            // display from both live goggles input and the fallback loop,
            // until it finishes, errors, or is stopped from the web interface.
            let requested_usb = usb_playback.lock().unwrap().clone();
            if requested_usb != current_usb {
                child = None;
                fallback = None;
                usb_player = None;
                // The previous clip's numbers must not linger while the next
                // one is still being measured.
                usb_speed.reset();
                if usb_speed_shown {
                    usb_speed_shown = false;
                    report(usb_speed_cleared());
                }
                if requested_usb.is_some() {
                    // mpv sets the mode and owns the screen while it plays:
                    // release our fallback/notice framebuffer (and its CMA
                    // memory), and don't re-prepare the display meanwhile
                    // (see the current_usb check above), so the two never
                    // fight over the CRTC. Not a DRM-master problem: Display
                    // drops master right after its modeset.
                    display = None;
                    prepare_at = Instant::now();
                }
                current_usb = requested_usb;
            }
            if usb_player.as_mut().is_some_and(UsbPlayer::finished)
                || usb_player
                    .as_ref()
                    .is_some_and(|p| p.eof(&usb_mpv_socket(&args.video_socket)))
            {
                usb_player = None;
                current_usb = None;
                *usb_playback.lock().unwrap() = None;
            }
            if let Some(path) = current_usb.clone()
                && usb_player.is_none()
                && args.output == "hdmi"
                && display.is_none()
            {
                match spawn_usb_player(&args, &path, display_mode.as_deref(), settings.stretch_usb)
                {
                    Ok(p) => {
                        usb_stretched = settings.stretch_usb;
                        usb_player = Some(p);
                    }
                    Err(e) => {
                        report(json!({"message":format!("USB video playback: {e:#}")}));
                        current_usb = None;
                        *usb_playback.lock().unwrap() = None;
                    }
                }
            }
            if current_usb.is_some() {
                // Drop whatever the goggles send meanwhile; nothing is
                // decoding it while a USB clip owns the display.
                while let Ok(c) = rx.try_recv() {
                    queued.fetch_sub(c.data.len(), Ordering::Relaxed);
                }
                // What reaches the screen: mpv's own counters stay at 0 even
                // when it crawls, so derive it from how fast its clock runs.
                if usb_player.is_some() && usb_speed_at.elapsed() >= Duration::from_secs(1) {
                    usb_speed_at = Instant::now();
                    if settings.stretch_usb != usb_stretched {
                        let socket = usb_mpv_socket(&args.video_socket);
                        let keep = json!(["set_property", "keepaspect", !settings.stretch_usb]);
                        if mpv_command(&socket, keep).is_ok() {
                            usb_stretched = settings.stretch_usb;
                        }
                    }
                    if let Some(status) =
                        usb_speed_status(&mut usb_speed, &usb_mpv_socket(&args.video_socket))
                    {
                        usb_speed_shown = true;
                        report(status);
                    }
                }
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            if usb_speed_shown {
                usb_speed_shown = false;
                usb_speed.reset();
                report(usb_speed_cleared());
            }
            if child
                .as_mut()
                .is_some_and(|p| p.child.try_wait().ok().flatten().is_some())
            {
                child = None;
            }
            let heartbeat = child.as_ref().and_then(|p| *p.heartbeat.lock().unwrap());
            if let Some(t) = heartbeat {
                last_frame = Some(t);
                child_showed = true;
            }
            let input_gone = last_input.is_none_or(|t| t.elapsed() > Duration::from_millis(1500));
            let decoder_stalled =
                child.is_some() && last_frame.is_some_and(|t| t.elapsed() > Duration::from_secs(3));
            if input_gone || decoder_stalled {
                child = None;
            }
            if input_gone {
                // A decoder that never got going because the goggles went away
                // says nothing about the display; every new connection tries
                // the stretch again.
                child_stretch = None;
                stretch_rejected = None;
            }
            if fallback.as_mut().is_some_and(|l| !l.healthy()) {
                fallback = None;
                fallback_retry = Instant::now();
            }
            // Loop the fallback video only while the goggles send nothing, so
            // a brief live decoder restart never flickers to the loop.
            let fallback_video = current
                .fallback_image
                .as_ref()
                .filter(|id| crate::assets::is_video_id(id))
                .map(|id| store.dir.join("images").join(id));
            if let Some(video) = &fallback_video
                && child.is_none()
                && fallback.is_none()
                && input_gone
                && args.output == "hdmi"
                && display.is_some()
                && shown_warning.is_none()
                && fallback_retry.elapsed() > Duration::from_secs(3)
            {
                fallback_retry = Instant::now();
                match spawn_loop(&args, video, report.clone()) {
                    Ok(l) => fallback = Some(l),
                    Err(e) => report(json!({"message":format!("Fallback video: {e:#}")})),
                }
            }
            if child.is_none() {
                *preview.lock().unwrap() = None;
                report(
                    json!({"hdmi":if args.output=="hdmi" && display.is_some(){"fallback"}else{"idle"},"output_fps":0,"fallback_video":fallback.is_some()}),
                );
            }
            if stats_at.elapsed() >= Duration::from_secs(1) {
                report(json!({"input_fps":frames as f64/stats_at.elapsed().as_secs_f64()}));
                stats_at = Instant::now();
                frames = 0;
                if let Some(reason) = recording.enforce_space(&recordings_dir) {
                    report(json!({"message":reason}));
                    *disk_warning.lock().unwrap() = Some(warning_image.clone());
                }
                recording.auto_tick(
                    &auto_record_dir,
                    &recordings_dir,
                    current.auto_record,
                    !input_gone,
                );
                recording.crash_tick(&recordings_dir, current.crash_buffer, !input_gone);
            }
            let chunk = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(c) => c,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            queued.fetch_sub(chunk.data.len(), Ordering::Relaxed);
            if args.output == "none" {
                continue;
            }
            if last_input.is_some_and(|t| {
                chunk.received.saturating_duration_since(t) > Duration::from_millis(750)
            }) {
                child = None;
                annex = Default::default();
            }
            last_input = Some(chunk.received);
            if lost.swap(false, Ordering::Relaxed) {
                child = None;
                annex = Default::default();
                report(json!({"message":"Video queue overrun; recovering at next keyframe"}));
                while let Ok(c) = rx.try_recv() {
                    queued.fetch_sub(c.data.len(), Ordering::Relaxed);
                }
                continue;
            }
            let units = match annex.feed(&chunk.data) {
                Ok(v) => v,
                Err(e) => {
                    child = None;
                    report(json!({"message":e}));
                    continue;
                }
            };
            for nal in units {
                // Independent of the live decoder's own state: record exactly
                // what the goggles send, including through a decoder restart.
                recording.feed(&nal);
                stream.feed(&nal);
                if matches!(nal.get(3).map(|v| v & 31), Some(1 | 5))
                    && nal.get(4).is_some_and(|v| v & 0x80 != 0)
                {
                    frames += 1;
                }
                if parameters.changed(&nal) {
                    child = None;
                }
                if child.is_none() {
                    if nal.get(3).map(|b| b & 31) != Some(7) {
                        continue;
                    }
                    // Hand the decoder and HDMI overlay over from the loop.
                    fallback = None;
                    let mut playback_args = args.clone();
                    if args.output == "hdmi" && display.is_none() {
                        playback_args.output = "test".into();
                    }
                    // The previous decoder was stretched and never showed a
                    // frame: the display chip refused this stretch.
                    if let Some(size) = child_stretch.take()
                        && !child_showed
                    {
                        stretch_rejected = Some(size);
                        report(json!({"message":format!(
                            "The display cannot stretch a {}x{} picture; showing it unstretched",
                            size.0, size.1
                        )}));
                    }
                    // Stretch to the screen: the picture's size comes from the
                    // SPS that starts this decoder, the screen's from the mode.
                    let size = crate::h264::sps_size(&nal);
                    let pixel_aspect = current
                        .stretch_live
                        .then(|| {
                            let info = &display.as_ref()?.info;
                            let out = |key: &str| u32::try_from(info[key].as_u64()?).ok();
                            let (w, h) = size?;
                            fill_par(w, h, out("hdmi_width")?, out("hdmi_height")?)
                        })
                        .flatten()
                        .filter(|_| size != stretch_rejected);
                    match spawn_player(
                        &playback_args,
                        current.preview_enabled,
                        report.clone(),
                        preview.clone(),
                        Role::Live,
                        pixel_aspect,
                    ) {
                        Ok(p) => {
                            child = Some(p);
                            child_stretch = pixel_aspect.and(size);
                            child_showed = false;
                            starts += 1;
                            last_frame = Some(Instant::now());
                            report(json!({"decoder_starts":starts,"hdmi":"starting"}));
                        }
                        Err(e) => {
                            report(json!({"hdmi":"error","message":format!("Decoder: {e:#}")}));
                            continue;
                        }
                    }
                }
                if let Some(p) = &mut child
                    && let Err(e) = write_deadline(p.child.stdin.as_mut().unwrap(), &nal)
                {
                    child = None;
                    report(json!({"message":format!("Decoder input: {e}")}));
                }
            }
        }
        drop(fallback);
        drop(child);
        drop(display);
        stream.stop();
        // Save a flight in progress rather than discard it on shutdown.
        let recordings_dir = recordings_dir(&store, &storage, &current);
        recording.auto_tick(&auto_record_dir, &recordings_dir, false, false);
        let _ = std::fs::remove_file(&args.video_socket);
    }))
}
fn receive(
    socket: UnixDatagram,
    tx: SyncSender<Chunk>,
    queued: Arc<AtomicUsize>,
    lost: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    let mut buf = vec![0; 32776];
    let mut expected = None;
    while !stop.load(Ordering::Relaxed) {
        let n = match socket.recv(&mut buf) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => break,
        };
        if n < 8 {
            continue;
        }
        let seq = u64::from_le_bytes(buf[..8].try_into().unwrap());
        if expected.is_some_and(|s| s != seq) {
            lost.store(true, Ordering::Relaxed);
        }
        expected = Some(seq.wrapping_add(1));
        let len = n - 8;
        if queued.fetch_add(len, Ordering::Relaxed) + len > 2 * 1024 * 1024 {
            queued.fetch_sub(len, Ordering::Relaxed);
            lost.store(true, Ordering::Relaxed);
            continue;
        }
        if tx
            .try_send(Chunk {
                data: buf[8..n].to_vec(),
                received: Instant::now(),
            })
            .is_err()
        {
            queued.fetch_sub(len, Ordering::Relaxed);
            lost.store(true, Ordering::Relaxed);
        }
    }
}
fn spawn_player(
    args: &WorkerArgs,
    show_preview: bool,
    report: Report,
    preview: Preview,
    role: Role,
    // Stretches the picture over the screen, see `fill_par`.
    pixel_aspect: Option<(u32, u32)>,
) -> anyhow::Result<Player> {
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.args([
        "-v",
        "fdsrc",
        "fd=0",
        "blocksize=65536",
        "do-timestamp=true",
        "!",
        "h264parse",
        "config-interval=-1",
        "!",
        "video/x-h264,stream-format=byte-stream,alignment=au",
        "!",
    ]);
    if args.decoder == "v4l2h264dec" {
        // Goggles advertise level 5.2 even for 1080p30. Pi's V4L2 plugin caps
        // stop at 5.1 and reject the stream before hardware sees it. Adjust only
        // negotiation metadata; SPS and every encoded video byte stay intact.
        cmd.args(["capssetter", "caps=video/x-h264,level=(string)5.1", "!"]);
    }
    cmd.args([
        &args.decoder,
        "!",
        "tee",
        "name=decoded",
        "decoded.",
        "!",
        "queue",
        "max-size-buffers=2",
        "max-size-bytes=0",
        "max-size-time=0",
        "leaky=downstream",
        "!",
    ]);
    // Only the picture going to the screen; the preview branch keeps its own.
    if let Some((n, d)) = pixel_aspect {
        cmd.args([
            "capssetter",
            &format!("caps=video/x-raw,pixel-aspect-ratio=(fraction){n}/{d}"),
            "!",
        ]);
    }
    let mut sink = if args.output == "hdmi" {
        // Avoid libdrm probing unrelated drivers during every stream restart.
        "kmssink driver-name=vc4 sync=false".to_owned()
    } else {
        "fakesink sync=false".to_owned()
    };
    if args.output == "hdmi"
        && let Some(id) = args.connector
    {
        sink += &format!(" connector-id={id}");
    }
    cmd.args([
        "fpsdisplaysink",
        &format!("video-sink={sink}"),
        "text-overlay=false",
        "sync=false",
        "fps-update-interval=500",
    ]);
    let (reader, writer) = UnixStream::pair()?;
    if show_preview {
        cmd.args([
            "decoded.",
            "!",
            "queue",
            "max-size-buffers=1",
            "max-size-bytes=0",
            "max-size-time=0",
            "leaky=downstream",
            "!",
            "videorate",
            "drop-only=true",
            "!",
            "video/x-raw,framerate=5/1",
            "!",
            "videoscale",
            "!",
            "videoconvert",
            "!",
            "video/x-raw,width=640,height=360,format=I420",
            "!",
            "jpegenc",
            "quality=65",
            "!",
            "fdsink",
            "fd=3",
            "sync=false",
            "async=false",
        ]);
        let fd = writer.as_raw_fd();
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    drop(writer);
    let active = Arc::new(AtomicBool::new(true));
    if show_preview {
        let active = active.clone();
        thread::spawn(move || read_jpegs(reader, preview, active));
    }
    let fd = child.stdin.as_ref().unwrap().as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("Cannot configure decoder pipe");
    }
    let bus = child.stdout.take().unwrap();
    let heartbeat = Arc::new(Mutex::new(None));
    let beat = heartbeat.clone();
    let running = active.clone();
    let hdmi = args.output == "hdmi";
    let live = role == Role::Live;
    thread::spawn(move || {
        for line in BufReader::new(bus).lines().map_while(Result::ok) {
            if !running.load(Ordering::Relaxed) {
                break;
            }
            if let Some((w, h, fps)) = input_caps(&line).filter(|_| live) {
                report(json!({"input_width":w,"input_height":h,"input_nominal_fps":fps}));
            }
            if let Some((frames, fps)) = frame_stats(&line) {
                *beat.lock().unwrap() = Some(Instant::now());
                if hdmi && live {
                    report(json!({"hdmi":"playing","output_frames":frames,"output_fps":fps}));
                }
            }
        }
    });
    Ok(Player {
        child,
        heartbeat,
        active,
    })
}
fn read_jpegs(mut reader: UnixStream, preview: Preview, active: Arc<AtomicBool>) {
    let mut buffer = Vec::new();
    let mut bytes = [0; 16384];
    while let Ok(n) = reader.read(&mut bytes) {
        if n == 0 || !active.load(Ordering::Relaxed) {
            break;
        }
        buffer.extend_from_slice(&bytes[..n]);
        while let Some(end) = buffer.windows(2).position(|b| b == [0xff, 0xd9]) {
            let frame: Vec<_> = buffer.drain(..end + 2).collect();
            if frame.starts_with(&[0xff, 0xd8]) {
                *preview.lock().unwrap() = Some((Instant::now(), frame));
            }
        }
        if buffer.len() > 2 * 1024 * 1024 {
            buffer.clear();
        }
    }
}
fn write_deadline(writer: &mut impl Write, mut bytes: &[u8]) -> std::io::Result<()> {
    let start = Instant::now();
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_millis(1000) {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn frame_stats(line: &str) -> Option<(u64, f64)> {
    let (_, rest) = line.split_once("last-message = rendered: ")?;
    let n = rest.split(',').next()?.trim().parse().ok()?;
    let (_, rest) = rest.split_once("current: ")?;
    let fps: f64 = rest.split(',').next()?.trim().parse().ok()?;
    fps.is_finite().then_some((n, fps))
}
pub(crate) fn input_caps(line: &str) -> Option<(u32, u32, Option<f64>)> {
    if !line.contains("h264parse0.GstPad:src: caps =") {
        return None;
    }
    let value = |key: &str| {
        line.split_once(key)
            .and_then(|(_, s)| s.split(',').next())
            .map(str::trim)
    };
    let fps = value("framerate=(fraction)")
        .and_then(|s| s.split_once('/'))
        .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)))
        .and_then(|(n, d)| (d > 0.0).then_some(n / d));
    Some((
        value("width=(int)")?.parse().ok()?,
        value("height=(int)")?.parse().ok()?,
        fps,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stretch_usb_never_rebuilds_but_stretch_live_does() {
        let current = crate::settings::Settings::default();
        let usb = crate::settings::Settings {
            stretch_usb: true,
            ..current.clone()
        };
        assert!(!must_rebuild(&current, &usb, false));
        assert!(!must_rebuild(&current, &usb, true));
        let live = crate::settings::Settings {
            stretch_live: true,
            ..current.clone()
        };
        assert!(must_rebuild(&current, &live, true));
    }
    #[test]
    fn fill_par_stretches_only_what_does_not_fit() {
        assert_eq!(fill_par(1440, 1080, 1920, 1080), Some((4, 3)));
        assert_eq!(fill_par(640, 480, 1920, 1080), Some((4, 3)));
        assert_eq!(fill_par(1920, 1080, 1920, 1080), None);
        assert_eq!(fill_par(1280, 720, 1920, 1080), None);
        assert_eq!(fill_par(1440, 1080, 1024, 768), None);
        // 16:9 onto a 4:3 screen is squeezed.
        assert_eq!(fill_par(1920, 1080, 1024, 768), Some((3, 4)));
        assert_eq!(fill_par(0, 1080, 1920, 1080), None);
        // Implausible shapes are left alone.
        assert_eq!(fill_par(100, 1000, 1920, 1080), None);
    }
    #[test]
    fn a_new_idle_screen_never_interrupts_live_video() {
        let current = crate::settings::Settings::default();
        let idle = crate::settings::Settings {
            fallback_image: Some("no-signal.png".into()),
            ..current.clone()
        };
        assert!(!must_rebuild(&current, &idle, true), "deferred while live");
        assert!(
            must_rebuild(&current, &idle, false),
            "applied right away otherwise"
        );
        let terminal = crate::settings::Settings {
            terminal_enabled: true,
            ..current.clone()
        };
        assert!(!must_rebuild(&current, &terminal, true));
        assert!(!must_rebuild(&current, &terminal, false));
        let mode = crate::settings::Settings {
            hdmi_mode: "1280x720@60".into(),
            ..idle.clone()
        };
        assert!(
            must_rebuild(&current, &mode, true),
            "other changes still apply"
        );
        assert!(!must_rebuild(&current, &current.clone(), false));
    }
    #[test]
    fn stats_distinguish_nominal_rate_and_output() {
        assert_eq!(
            frame_stats("last-message = rendered: 42, dropped: 0, current: 29.98, average: 30.01"),
            Some((42, 29.98))
        );
        assert_eq!(
            frame_stats("last-message = rendered: 0, current: NaN"),
            None
        );
        assert_eq!(
            input_caps(
                "h264parse0.GstPad:src: caps = video/x-h264, width=(int)1920, height=(int)1080, framerate=(fraction)30000/1001"
            ),
            Some((1920, 1080, Some(30000.0 / 1001.0)))
        );
    }
    #[test]
    fn fallback_file_splits_into_nals_and_frames() {
        // SPS, PPS, IDR slice (first_mb 0), second slice of the same picture
        // (first_mb != 0), then a P slice starting the next picture.
        let file = [
            &[0, 0, 0, 1, 0x67, 0x64][..],
            &[0, 0, 0, 1, 0x68, 0xee][..],
            &[0, 0, 1, 0x65, 0x88, 0x01][..],
            &[0, 0, 1, 0x65, 0x40, 0x02][..],
            &[0, 0, 1, 0x41, 0x9a, 0x03][..],
        ]
        .concat();
        let units = nal_units(&file).unwrap();
        assert_eq!(units.len(), 5);
        assert_eq!(units[4], [0, 0, 1, 0x41, 0x9a, 0x03]);
        let frames: Vec<bool> = units.iter().map(|n| starts_frame(n)).collect();
        assert_eq!(frames, [false, false, true, false, true]);
        assert!(nal_units(&[0, 0, 1, 0x65, 0x88]).is_err(), "SPS required");
    }

    /// End-to-end with real GStreamer: upload converts MP4 without re-encoding,
    /// rejects HEVC and >1080p, and the loop keeps rendering past the clip end.
    /// `UNGOGGLED_TEST_VIDEOS=<dir with h264.mp4 hevc.mp4 uhd.mp4> cargo test -- --ignored`
    #[test]
    #[ignore]
    fn fallback_video_loops_with_gstreamer() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            worker: WorkerArgs,
        }
        let src = std::path::PathBuf::from(std::env::var("UNGOGGLED_TEST_VIDEOS").unwrap());
        let dir = std::env::temp_dir().join(format!("ungoggled-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let read = |n: &str| std::fs::read(src.join(n)).unwrap();
        let hevc = crate::assets::upload(&dir, &read("hevc.mp4"), None).unwrap_err();
        assert!(hevc.to_string().contains("not H.264"), "{hevc}");
        let uhd = crate::assets::upload(&dir, &read("uhd.mp4"), None).unwrap_err();
        assert!(uhd.to_string().contains("1920"), "{uhd}");
        let ok = crate::assets::upload(&dir, &read("h264.mp4"), Some("h264.mp4")).unwrap();
        assert_eq!(ok["kind"], "video");
        let id = ok["id"].as_str().unwrap();
        let video = dir.join(id);
        let info = crate::assets::video_info(&video);
        assert_eq!((info.width, info.height), (1920, 1080));
        assert!((info.fps - 30.0).abs() < 0.01);
        let listed = crate::assets::list(&dir);
        assert_eq!(listed.len(), 1, "temporary files must not be listed");
        let args = Cli::parse_from(["t", "--output", "test", "--decoder", "avdec_h264"]).worker;
        let mut l = spawn_loop(&args, &video, Arc::new(|_| {})).unwrap();
        // The 4 s clip must keep playing beyond its end.
        for _ in 0..9 {
            thread::sleep(Duration::from_secs(1));
            assert!(l.healthy(), "loop stopped rendering");
        }
        let stop = Instant::now();
        drop(l);
        assert!(stop.elapsed() < Duration::from_secs(2), "slow hand-over");
        crate::assets::remove(&dir, id).unwrap();
        assert!(!dir.join(id).with_extension("json").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
