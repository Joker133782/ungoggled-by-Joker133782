mod api;
mod assets;
mod clock;
mod display;
mod functionfs;
mod g2;
mod g2_sequence;
mod gadget;
mod h264;
mod hardware;
mod history;
mod protocol;
mod session;
mod settings;
mod update;
mod video;
mod wifi;

use anyhow::Result;
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
    },
    #[command(hide = true)]
    UpdateRecover,
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
    #[arg(long,default_value="auto",value_parser=["auto","goggles3","goggles2"],env="UNGOGGLED_GOGGLES")]
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
        Commands::UpdateWorker { id } => update::install(&id),
        Commands::UpdateRecover => update::recover(),
        Commands::MigrateSettings { data_dir, output } => {
            println!(
                "{}",
                serde_json::to_string(&settings::migrate(&data_dir, &output)?)?
            );
            Ok(())
        }
        Commands::Worker(args) => {
            let result = if args.transport == "functionfs" {
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
            web_dir,
            no_autostart,
            data_dir,
            runtime_dir,
            mut worker,
        } => {
            std::fs::create_dir_all(&runtime_dir)?;
            worker.video_socket = runtime_dir.join("video.sock");
            let app = App {
                status: Arc::new(Mutex::new(
                    json!({"phase":"stopped","video_bytes":0,"bitrate_mbps":0,"hdmi":"idle"}),
                )),
                last_output: Arc::new(Mutex::new(None)),
                enabled: Arc::new(AtomicBool::new(!no_autostart)),
                restart: Arc::new(AtomicU64::new(0)),
                shutdown: Arc::new(AtomicBool::new(false)),
                since: Instant::now(),
                settings: settings::Store::open(data_dir)?,
                history: Arc::new(Mutex::new(history::History::new())),
                preview: Arc::new(Mutex::new(None)),
                operations: Arc::new(Mutex::new(())),
                wifi_busy: Arc::new(AtomicBool::new(false)),
                wifi_error: Arc::new(Mutex::new(None)),
                beeper_test: Arc::new(AtomicBool::new(false)),
                time_synced: Arc::new(AtomicBool::new(false)),
            };
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            let observed = app.clone();
            let report: video::Report = Arc::new(move |event| observed.event(event));
            let renderer = video::start(
                worker.clone(),
                app.settings.clone(),
                report.clone(),
                app.preview.clone(),
                app.shutdown.clone(),
            )?;
            let supervisor = supervise(app.clone(), worker);
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
                while !samples.shutdown.load(Ordering::Relaxed) {
                    interval.tick().await;
                    let s = samples.snapshot();
                    samples.history.lock().unwrap().push(&s);
                }
            });
            let router = api::router(app.clone(), web_dir);
            eprintln!("ungoggled control interface: http://{listen}");
            let stop_app = app.clone();
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let mut term =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                            .unwrap();
                    tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
                    stop_app.shutdown.store(true, Ordering::Relaxed);
                })
                .await?;
            app.shutdown.store(true, Ordering::Relaxed);
            let _ = supervisor.join();
            let _ = renderer.join();
            let _ = hardware.join();
            sampler.abort();
            Ok(())
        }
    }
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
