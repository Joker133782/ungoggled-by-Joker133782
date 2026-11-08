mod api;
mod assets;
mod auth;
mod clock;
mod display;
mod functionfs;
mod g2;
mod g2_sequence;
mod gadget;
mod h264;
mod hardware;
mod history;
mod metrics;
mod playspeed;
mod protocol;
mod recording;
mod session;
mod settings;
mod storage;
mod stream;
mod support;
mod system_fixes;
mod terminal;
mod tls;
mod update;
mod v1v2;
mod video;
mod wifi;

use anyhow::{Context, Result};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "DJI Goggles 3 USB accessory receiver and HDMI bridge"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = "0.0.0.0:80")]
        listen: String,
        /// Serves the same interface over HTTPS too, with a self-signed
        /// certificate generated on first use; set empty to disable.
        #[arg(long, default_value = "0.0.0.0:443")]
        tls_listen: String,
        #[arg(long, default_value = "web/dist")]
        web_dir: PathBuf,
        #[arg(long)]
        no_autostart: bool,
        #[arg(long, default_value = "/var/lib/dji-hdmi")]
        data_dir: PathBuf,
        #[arg(long, default_value = "/run/dji-hdmi")]
        runtime_dir: PathBuf,
        #[command(flatten)]
        worker: WorkerArgs,
    },
    #[command(hide = true)]
    Worker(WorkerArgs),
    Doctor,
    /// Verify an update archive without installing it.
    CheckUpdate {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "aarch64", value_parser = ["aarch64"])]
        architecture: String,
    },
    /// Install an unpacked application update (manual bootstrap).
    UpdateInstall {
        #[arg(long)]
        source: PathBuf,
    },
    #[command(hide = true)]
    UpdateWorker {
        #[arg(long)]
        id: String,
        #[arg(long)]
        skip_health_check: bool,
    },
    #[command(hide = true)]
    UpdateRecover,
    /// Apply or inspect the boot configuration and HEVC decoder module fixes
    /// (run as root by prepare-pi.sh before every service start).
    SystemFixes {
        #[arg(value_parser = ["apply", "status", "remove-module", "enable-module"])]
        action: String,
        #[arg(long, default_value = "/")]
        root: PathBuf,
        /// Kernel release to install for instead of the running one (image build).
        #[arg(long)]
        kernel_release: Option<String>,
        /// Not a running system (image build): never loads modules.
        #[arg(long)]
        offline: bool,
        /// Fail unless every fix is in place (image build).
        #[arg(long)]
        strict: bool,
    },
    #[command(hide = true)]
    MigrateSettings {
        #[arg(long)]
        data_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Args, Clone)]
pub struct WorkerArgs {
    #[arg(long, default_value = "/run/dji-hdmi/video.sock")]
    video_socket: PathBuf,
    #[arg(long,default_value="functionfs",value_parser=["functionfs","gadgetfs"])]
    transport: String,
    #[arg(long, default_value = "/dev/gadget")]
    gadget_dir: String,
    #[arg(long)]
    controller: Option<String>,
    #[arg(long,default_value="hdmi",value_parser=["hdmi","test","none"])]
    output: String,
    #[arg(long,default_value="v4l2h264dec",value_parser=["v4l2h264dec","omxh264dec","avdec_h264"])]
    decoder: String,
    #[arg(long)]
    connector: Option<u32>,
    #[arg(long)]
    capture: Option<PathBuf>,
    #[arg(long, default_value_t = 32_000_000)]
    capture_limit: u64,
    #[arg(long, default_value_t = 22345)]
    control_port: u16,
    #[arg(long,default_value_t=11520,value_parser=clap::value_parser!(u16).range(11520..=11521),env="UNGOGGLED_ACCESSORY_PID")]
    accessory_pid: u16,
    /// Goggles protocol: `auto` tries the last working one first and falls back
    /// to the other when no video arrives; `goggles3` or `goggles2` force one.
    /// `v1v2` is a separate, experimental USB-host receiver for the original
    /// DJI FPV Goggles (plugged into a USB-A host port, not the gadget port)
    /// and is never selected by `auto`.
    #[arg(long,default_value="auto",value_parser=["auto","goggles3","goggles2","v1v2"],env="UNGOGGLED_GOGGLES")]
    goggles: String,
    /// Which protocol worked for which goggles (by their AOA strings).
    #[arg(long, default_value = "/var/lib/dji-hdmi/goggles-protocol.json")]
    goggles_memory: PathBuf,
    #[arg(long, hide = true)]
    cold_accessory: bool,
    /// After USB registration, send DUML clock commands from the Pi wall clock.
    #[arg(
        long,
        default_value_t = true,
        action = clap::ArgAction::Set,
        num_args = 0..=1,
        default_missing_value = "true",
        env = "UNGOGGLED_SYNC_TIME"
    )]
    sync_time: bool,
}

#[derive(Clone)]
struct App {
    status: Arc<Mutex<Value>>,
    last_output: Arc<Mutex<Option<Instant>>>,
    enabled: Arc<AtomicBool>,
    restart: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    since: Instant,
    settings: settings::Store,
    history: Arc<Mutex<history::History>>,
    preview: video::Preview,
    operations: Arc<Mutex<()>>,
    wifi_busy: Arc<AtomicBool>,
    wifi_error: Arc<Mutex<Option<String>>>,
    beeper_test: Arc<AtomicBool>,
    /// Latched once the clock has been set from a browser or NTP since boot.
    time_synced: Arc<AtomicBool>,
    storage: storage::Store,
    /// Path of a USB-drive video to play on the HDMI output, if any.
    usb_playback: Arc<Mutex<Option<PathBuf>>>,
    /// Where mpv's IPC socket lives while a USB video plays.
    usb_mpv_socket: PathBuf,
    recording: recording::Controller,
    /// Overrides the configured fallback image (e.g. with a "SD card full"
    /// notice) until the next successful recording start or dismissal.
    disk_warning: Arc<Mutex<Option<PathBuf>>>,
    /// Shows the current Wi-Fi mode/SSID/IP on the HDMI output while the
    /// client profile (see `wifi::join`) is the one active, so joining a
    /// network doesn't leave no way to find the Pi again on it; `None` the
    /// rest of the time. Kept current by a background task in `main`.
    network_notice: Arc<Mutex<Option<PathBuf>>>,
    stream: stream::Controller,
    /// Gates every control-plane route; `/watch.html` and `/api/stream.ts`
    /// deliberately stay outside it so spectators never need a login.
    auth: auth::Store,
    /// Step-up tickets and the single open terminal (`terminal.rs`).
    terminals: terminal::Terminals,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Commands::CheckUpdate { file, architecture } => {
            let staging =
                std::env::temp_dir().join(format!("ungoggled-verify-{}", std::process::id()));
            let result = update::package::unpack(&file, &staging, &architecture);
            // Never delete a pre-existing path if create_dir failed.
            if let Ok(manifest) = &result {
                std::fs::remove_dir_all(&staging)?;
                println!(
                    "Verified ungoggled {} for {} ({})",
                    manifest.version,
                    architecture,
                    manifest.release_id()?
                );
            }
            result.map(|_| ())
        }
        Commands::UpdateInstall { source } => update::manual(&source),
        Commands::UpdateWorker {
            id,
            skip_health_check,
        } => update::install(&id, skip_health_check),
        Commands::UpdateRecover => update::recover(),
        Commands::SystemFixes {
            action,
            root,
            kernel_release,
            offline,
            strict,
        } => system_fixes::command(&action, &root, kernel_release, offline, strict),
        Commands::MigrateSettings { data_dir, output } => {
            println!(
                "{}",
                serde_json::to_string(&settings::migrate(&data_dir, &output)?)?
            );
            Ok(())
        }
        Commands::Worker(args) => {
            let result = if args.goggles == "v1v2" {
                v1v2::run(&args)
            } else if args.transport == "functionfs" {
                functionfs::run(&args)
            } else {
                gadget::run(&args)
            };
            if let Err(e) = &result {
                println!("{}", json!({"phase":"error","message":format!("{e:#}")}));
            }
            result
        }
        Commands::Doctor => {
            println!("{}", serde_json::to_string_pretty(&doctor())?);
            Ok(())
        }
        Commands::Serve {
            listen,
            tls_listen,
            web_dir,
            no_autostart,
            data_dir,
            runtime_dir,
            mut worker,
        } => {
            std::fs::create_dir_all(&runtime_dir)?;
            system_fixes::watch();
            worker.video_socket = runtime_dir.join("video.sock");
            let usb_mpv_socket = video::usb_mpv_socket(&worker.video_socket);
            let shutdown = Arc::new(AtomicBool::new(false));
            let settings = settings::Store::open(data_dir.clone())?;
            let auth = auth::Store::open(&data_dir)?;
            let usb_playback = Arc::new(Mutex::new(None));
            let storage = storage::start(
                runtime_dir.clone(),
                settings.clone(),
                shutdown.clone(),
                usb_playback.clone(),
            );
            let app = App {
                status: Arc::new(Mutex::new(
                    json!({"phase":"stopped","video_bytes":0,"bitrate_mbps":0,"hdmi":"idle"}),
                )),
                last_output: Arc::new(Mutex::new(None)),
                enabled: Arc::new(AtomicBool::new(!no_autostart)),
                restart: Arc::new(AtomicU64::new(0)),
                shutdown,
                since: Instant::now(),
                settings,
                history: Arc::new(Mutex::new(history::History::new())),
                preview: Arc::new(Mutex::new(None)),
                operations: Arc::new(Mutex::new(())),
                wifi_busy: Arc::new(AtomicBool::new(false)),
                wifi_error: Arc::new(Mutex::new(None)),
                beeper_test: Arc::new(AtomicBool::new(false)),
                time_synced: Arc::new(AtomicBool::new(false)),
                storage,
                usb_playback,
                usb_mpv_socket,
                recording: recording::Controller::new(runtime_dir.join("recording-thumbs")),
                disk_warning: Arc::new(Mutex::new(None)),
                network_notice: Arc::new(Mutex::new(None)),
                stream: stream::Controller::default(),
                auth,
                terminals: terminal::Terminals::default(),
            };
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            let observed = app.clone();
            let report: video::Report = Arc::new(move |event| observed.event(event));
            let renderer = video::start(
                worker.clone(),
                app.settings.clone(),
                app.storage.clone(),
                report.clone(),
                app.preview.clone(),
                app.shutdown.clone(),
                app.usb_playback.clone(),
                app.recording.clone(),
                app.disk_warning.clone(),
                app.network_notice.clone(),
                app.stream.clone(),
            )?;
            let supervisor = supervise(app.clone(), worker);
            let wifi_shutdown = app.shutdown.clone();
            let wifi_watch_busy = app.wifi_busy.clone();
            let wifi_watch_error = app.wifi_error.clone();
            thread::spawn(move || wifi::watch(&wifi_shutdown, &wifi_watch_busy, &wifi_watch_error));
            let notice = app.clone();
            let wifi_notice_task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(2));
                let mut shown: Option<Vec<String>> = None;
                while !notice.shutdown.load(Ordering::Relaxed) {
                    interval.tick().await;
                    let status = tokio::task::spawn_blocking(wifi::client_status)
                        .await
                        .ok()
                        .filter(|s| s["active"].as_bool() == Some(true));
                    let lines = status.as_ref().map(|s| {
                        let ssid = s["ssid"].as_str().unwrap_or("?");
                        let ip = s["ip"]
                            .as_str()
                            .map(|ip| ip.split('/').next().unwrap_or(ip).to_owned());
                        vec![
                            format!("Connected to: {ssid}"),
                            match ip {
                                Some(ip) => format!("IP: {ip}"),
                                None => "Connecting…".to_owned(),
                            },
                        ]
                    });
                    if lines == shown {
                        continue;
                    }
                    shown = lines.clone();
                    *notice.network_notice.lock().unwrap() = match &lines {
                        Some(lines) => assets::render_network_status(lines).and_then(|png| {
                            // tmpfs, like the other runtime-generated images
                            // (sd-card-full.png) — this is status, not content.
                            let path = notice.usb_mpv_socket.with_file_name("network-status.png");
                            std::fs::write(&path, png).ok()?;
                            Some(path)
                        }),
                        None => None,
                    };
                }
            });
            let hardware_status = app.clone();
            let hardware = hardware::start(
                Arc::new(move || hardware_status.snapshot()),
                app.settings.clone(),
                app.shutdown.clone(),
                app.beeper_test.clone(),
                report,
            );
            let samples = app.clone();
            let sampler = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut resources = metrics::Sampler::default();
                while !samples.shutdown.load(Ordering::Relaxed) {
                    interval.tick().await;
                    let s = samples.snapshot();
                    let m = resources.sample();
                    samples.history.lock().unwrap().push(&s, &m);
                }
            });
            let router = api::router(app.clone(), web_dir);
            let tls_addr = if tls_listen.is_empty() {
                None
            } else {
                Some(
                    tls_listen
                        .parse::<std::net::SocketAddr>()
                        .with_context(|| format!("Invalid --tls-listen address {tls_listen}"))?,
                )
            };
            // With TLS on, plain HTTP only redirects to it: a PWA install
            // prompt (and anyone bookmarking the page) needs the secure
            // origin, not whichever scheme happened to be typed first.
            let http_router = match tls_addr {
                Some(addr) => redirect_router(addr.port()),
                None => router.clone(),
            };
            eprintln!("ungoggled control interface: http://{listen}");
            let tls_task = if let Some(tls_addr) = tls_addr {
                let tls_config = tls::ensure(&app.settings.dir).await?;
                eprintln!("ungoggled control interface: https://{tls_listen}");
                let tls_router = router.clone();
                let handle = axum_server::Handle::new();
                tokio::spawn(wait_for_shutdown_signal(handle.clone()));
                Some(tokio::spawn(async move {
                    axum_server::bind_rustls(tls_addr, tls_config)
                        .handle(handle)
                        .serve(
                            tls_router
                                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
                        )
                        .await
                }))
            } else {
                None
            };
            let stop_app = app.clone();
            axum::serve(
                listener,
                http_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                wait_for_shutdown().await;
                stop_app.shutdown.store(true, Ordering::Relaxed);
            })
            .await?;
            app.shutdown.store(true, Ordering::Relaxed);
            if let Some(t) = tls_task {
                let _ = t.await;
            }
            let _ = supervisor.join();
            let _ = renderer.join();
            let _ = hardware.join();
            sampler.abort();
            wifi_notice_task.abort();
            Ok(())
        }
    }
}
async fn wait_for_shutdown() {
    let mut term =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
}
async fn wait_for_shutdown_signal(handle: axum_server::Handle) {
    wait_for_shutdown().await;
    handle.graceful_shutdown(Some(Duration::from_secs(2)));
}
/// Every plain-HTTP request becomes a redirect to the same host on
/// `https_port`, so there is one canonical secure origin: Chrome's PWA
/// install prompt (and anything else that cares about a secure context)
/// only ever sees that one, regardless of which URL someone typed or
/// bookmarked first.
fn redirect_router(https_port: u16) -> axum::Router {
    async fn redirect(
        axum::extract::State(port): axum::extract::State<u16>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> axum::response::Redirect {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|h| h.split(':').next().unwrap_or(h))
            .unwrap_or("localhost");
        let suffix = if port == 443 {
            String::new()
        } else {
            format!(":{port}")
        };
        axum::response::Redirect::permanent(&format!("https://{host}{suffix}{uri}"))
    }
    axum::Router::new()
        .fallback(redirect)
        .with_state(https_port)
}
impl App {
    fn event(&self, event: Value) {
        if let Value::Object(event) = event {
            if event.contains_key("output_frames") {
                *self.last_output.lock().unwrap() = Some(Instant::now());
            }
            self.status
                .lock()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .extend(event);
        }
    }
    fn snapshot(&self) -> Value {
        let mut s = self.status.lock().unwrap().clone();
        if let Some(last) = *self.last_output.lock().unwrap() {
            s["output_age_ms"] = json!(last.elapsed().as_millis() as u64);
            if last.elapsed() > Duration::from_secs(3) && s["hdmi"] == "playing" {
                s["hdmi"] = json!("waiting for frames");
                s["output_fps"] = json!(0);
            }
        }
        s["enabled"] = json!(self.enabled.load(Ordering::Relaxed));
        s["uptime_seconds"] = json!(self.since.elapsed().as_secs());
        s["temperature_c"] = json!(history::temperature());
        s["version"] = json!(env!("CARGO_PKG_VERSION"));
        s["hdmi_connected"] = json!(hdmi_connectors().any(|c| monitor_detected(&c)));
        s["usb_state"] = json!(
            functionfs::first_controller()
                .as_deref()
                .and_then(functionfs::udc_state)
        );
        s["build_id"] = json!(update::running_build());
        s["disk_warning"] = json!(self.disk_warning.lock().unwrap().is_some());
        s["reboot_required"] = json!(system_fixes::reboot_required());
        if let Some(clock) = clock::now() {
            s["time_unix_ms"] = json!(clock.unix_ms);
            s["time"] = json!(clock.display());
            s["timezone"] = json!(clock.tz);
        }
        if !self.time_synced.load(Ordering::Relaxed) && clock::ntp_synchronized() {
            self.time_synced.store(true, Ordering::Relaxed);
        }
        s["time_synced"] = json!(self.time_synced.load(Ordering::Relaxed));
        s
    }
}
fn guard(headers: &HeaderMap) -> Result<(), StatusCode> {
    // Requiring a custom header blocks cross-site forms; no CORS is enabled.
    if headers.get("x-dji-control").is_some_and(|v| v == "1") {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
async fn start(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    guard(&headers)?;
    update::api::ensure_idle().map_err(|_| StatusCode::CONFLICT)?;
    app.enabled.store(true, Ordering::Relaxed);
    Ok(Json(json!({"accepted":true})))
}
async fn stop(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    guard(&headers)?;
    update::api::ensure_idle().map_err(|_| StatusCode::CONFLICT)?;
    app.enabled.store(false, Ordering::Relaxed);
    Ok(Json(json!({"accepted":true})))
}
async fn restart(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    guard(&headers)?;
    update::api::ensure_idle().map_err(|_| StatusCode::CONFLICT)?;
    app.enabled.store(true, Ordering::Relaxed);
    app.restart.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({"accepted":true})))
}

fn supervise(app: App, args: WorkerArgs) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !app.shutdown.load(Ordering::Relaxed) {
            if !app.enabled.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            let generation = app.restart.load(Ordering::Relaxed);
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "worker",
                "--transport",
                &args.transport,
                "--gadget-dir",
                &args.gadget_dir,
                "--output",
                &args.output,
                "--decoder",
                &args.decoder,
                "--control-port",
                &args.control_port.to_string(),
                "--accessory-pid",
                &args.accessory_pid.to_string(),
                "--goggles",
                &args.goggles,
            ]);
            cmd.arg("--goggles-memory").arg(&args.goggles_memory);
            cmd.arg("--video-socket").arg(&args.video_socket);
            if let Some(c) = &args.controller {
                cmd.args(["--controller", c]);
            }
            if let Some(c) = args.connector {
                cmd.args(["--connector", &c.to_string()]);
            }
            if let Some(c) = &args.capture {
                cmd.arg("--capture")
                    .arg(c)
                    .arg("--capture-limit")
                    .arg(args.capture_limit.to_string());
            }
            if args.cold_accessory {
                cmd.arg("--cold-accessory");
            }
            cmd.args(["--sync-time", if args.sync_time { "true" } else { "false" }]);
            cmd.process_group(0)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            // Ensure abrupt parent death does not leave a USB owner behind.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            app.event(json!({"phase":"starting","video_bytes":0,"bitrate_mbps":0}));
            match cmd.spawn() {
                Ok(mut child) => {
                    let state = app.status.clone();
                    let last_output = app.last_output.clone();
                    let stdout = child.stdout.take().unwrap();
                    let reader = thread::spawn(move || {
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            if let Ok(Value::Object(event)) = serde_json::from_str::<Value>(&line) {
                                if event.contains_key("output_frames") {
                                    *last_output.lock().unwrap() = Some(Instant::now());
                                }
                                state.lock().unwrap().as_object_mut().unwrap().extend(event);
                            }
                        }
                    });
                    loop {
                        if app.shutdown.load(Ordering::Relaxed)
                            || !app.enabled.load(Ordering::Relaxed)
                            || app.restart.load(Ordering::Relaxed) != generation
                        {
                            break;
                        }
                        match child.try_wait() {
                            Ok(Some(s)) => {
                                app.status.lock().unwrap()["message"] =
                                    json!(format!("Receiver exited: {s}"));
                                break;
                            }
                            Err(e) => {
                                app.status.lock().unwrap()["message"] = json!(e.to_string());
                                break;
                            }
                            _ => {}
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    // Kill receiver and decoder together, then reap before replacing.
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGTERM);
                    }
                    let _ = child.wait();
                    let _ = reader.join();
                }
                Err(e) => {
                    app.status.lock().unwrap()["message"] = json!(e.to_string());
                }
            }
            {
                let mut s = app.status.lock().unwrap();
                s["phase"] = json!(if app.enabled.load(Ordering::Relaxed) {
                    "retrying"
                } else {
                    "stopped"
                });
                s["bitrate_mbps"] = json!(0);
            }
            for _ in 0..20 {
                if app.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    })
}

fn hdmi_connectors() -> impl Iterator<Item = std::path::PathBuf> {
    std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("HDMI"))
        .map(|e| e.path())
}

// The image forces an HDMI mode, so the connector reads "connected" with nothing
// plugged in. A display that answers with an EDID is the one actually attached.
fn monitor_detected(connector: &std::path::Path) -> bool {
    std::fs::read_to_string(connector.join("status"))
        .unwrap_or_default()
        .trim()
        == "connected"
        && std::fs::read(connector.join("edid"))
            .unwrap_or_default()
            .len()
            >= 128
}

/// HDMI-A connectors for the "Output port" setting: id to send, a friendly
/// label matching the Pi 4's case silkscreen, and whether a display answers
/// right now. Only listed when there's more than one, so single-HDMI
/// hardware never shows a pointless one-choice dropdown.
pub fn hdmi_connector_options() -> Vec<Value> {
    let mut options: Vec<_> = hdmi_connectors()
        .filter_map(|c| {
            let name = c.file_name()?.to_string_lossy().into_owned();
            let id: u32 = std::fs::read_to_string(c.join("connector_id"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            // "card1-HDMI-A-1" -> port index 1; the Pi 4 port closest to the
            // USB-C power input is always HDMI-A-1, labeled "HDMI0" on the case.
            let port: u32 = name.rsplit('-').next()?.parse().ok()?;
            Some(json!({
                "id": id,
                "name": format!("HDMI{}", port.saturating_sub(1)),
                "connected": monitor_detected(&c),
            }))
        })
        .collect();
    options.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    options
}

fn doctor() -> Value {
    let controllers: Vec<_> = std::fs::read_dir("/sys/class/udc")
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .collect();
    let connectors: Vec<_> = hdmi_connectors()
        .map(|c| {
            json!({
                "name":c.file_name().unwrap_or_default().to_string_lossy(),
                "status":std::fs::read_to_string(c.join("status")).unwrap_or_default().trim(),
                "edid_bytes":std::fs::read(c.join("edid")).unwrap_or_default().len(),
                "monitor_detected":monitor_detected(&c),
                "modes":std::fs::read_to_string(c.join("modes")).unwrap_or_default().lines().collect::<Vec<_>>(),
                "id":std::fs::read_to_string(c.join("connector_id")).unwrap_or_default().trim()
            })
        }).collect();
    json!({"controllers":controllers,"connectors":connectors,"gadgetfs_mounted":std::fs::read_to_string("/proc/mounts").unwrap_or_default().contains(" gadgetfs "),"functionfs_mounted":std::fs::read_to_string("/proc/mounts").unwrap_or_default().contains(" functionfs "),"architecture":std::env::consts::ARCH})
}
