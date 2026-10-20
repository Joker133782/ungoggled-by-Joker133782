use crate::{App, assets, settings, wifi};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Duration};
use tower_http::services::ServeDir;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;
fn bad(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn guard(h: &HeaderMap) -> ApiResult<()> {
    crate::guard(h).map_err(|_| {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Missing control header"})),
        )
    })
}
pub fn router(app: App, web: std::path::PathBuf) -> Router {
    Router::new()
        .route(
            "/api/status",
            get(|State(a): State<App>| async move { Json(a.snapshot()) }),
        )
        .route(
            "/api/history",
            get(|State(a): State<App>| async move { Json(a.history.lock().unwrap().snapshot()) }),
        )
        .route("/api/diagnostics", get(|| async { Json(crate::doctor()) }))
        .route("/api/start", post(crate::start))
        .route("/api/stop", post(crate::stop))
        .route("/api/restart", post(crate::restart))
        .route("/api/system/reboot", post(reboot))
        .route("/api/system/shutdown", post(shutdown))
        .route("/api/system/time", post(set_time))
        .route("/api/hardware/test-beeper", post(test_beeper))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/images", get(images).post(upload))
        .route("/api/images/{id}", get(image_file).delete(remove_image))
        .route("/api/preview.jpg", get(preview))
        .route("/api/wifi", get(get_wifi).post(save_wifi))
        .route("/api/usb-videos", get(usb_videos))
        .route("/api/usb-videos/status", get(usb_video_status))
        .route("/api/usb-videos/pause", post(pause_usb_video))
        .route("/api/usb-videos/seek", post(seek_usb_video))
        .route("/api/usb-videos/stop", post(stop_usb_video))
        .route("/api/usb-videos/{id}/play", post(play_usb_video))
        .route("/api/usb-videos/{id}/thumbnail", get(usb_video_thumbnail))
        .route(
            "/api/recording",
            get(recording_status).post(start_recording),
        )
        .route("/api/recording/stop", post(stop_recording))
        .route("/api/recording/save-buffer", post(save_recording_buffer))
        .route(
            "/api/recording/dismiss-warning",
            post(dismiss_recording_warning),
        )
        .route("/api/recordings", get(list_recordings))
        .route(
            "/api/recordings/{name}",
            get(recording_file).delete(remove_recording),
        )
        .route("/api/recordings/{name}/play", post(play_recording))
        .route("/api/stream.ts", get(stream_ts))
        .layer(DefaultBodyLimit::max(assets::MAX_VIDEO_UPLOAD + 65536))
        .merge(crate::update::api::routes())
        .fallback_service(ServeDir::new(web))
        .with_state(app)
}
async fn reboot(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    power_action(a, h, "reboot").await
}
async fn shutdown(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    power_action(a, h, "poweroff").await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockChange {
    unix_ms: u64,
    timezone: Option<String>,
}
async fn set_time(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<ClockChange>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let worker = a.clone();
    tokio::task::spawn_blocking(move || {
        let a = worker;
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        crate::clock::apply(c.unix_ms, c.timezone.as_deref())
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    a.time_synced.store(true, Ordering::Relaxed);
    let clock = crate::clock::now();
    Ok(Json(json!({
        "saved": true,
        "time_unix_ms": clock.as_ref().map(|t| t.unix_ms),
        "time": clock.as_ref().map(|t| t.display()),
        "timezone": clock.as_ref().map(|t| t.tz.clone()),
    })))
}
async fn power_action(a: App, h: HeaderMap, verb: &'static str) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let _operations = a.operations.lock().unwrap();
    crate::update::api::ensure_idle().map_err(bad)?;
    // Return before the connection this response travels over goes down.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = tokio::process::Command::new("systemctl")
            .arg(verb)
            .status()
            .await;
    });
    Ok(Json(json!({"accepted":true})))
}
async fn test_beeper(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    if a.settings.get().beeper_pin.is_none() {
        return Err(bad("Enable the beeper and set its GPIO pin first"));
    }
    a.beeper_test.store(true, Ordering::Relaxed);
    Ok(Json(json!({"accepted":true})))
}
async fn get_settings(State(a): State<App>) -> Json<settings::Settings> {
    Json(a.settings.get())
}
async fn save_settings(
    State(a): State<App>,
    h: HeaderMap,
    Json(s): Json<settings::Settings>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    // Only re-validate when the mode is actually changing, and only while a
    // display currently supplies a mode list. Otherwise saving an unrelated
    // setting (a GPIO pin, the preview toggle, ...) would fail as soon as the
    // display advertising the already-saved mode isn't attached any more.
    if s.hdmi_mode != a.settings.get().hdmi_mode && s.hdmi_mode != "auto" {
        let status = a.snapshot();
        if let Some(modes) = status["hdmi_modes"].as_array()
            && !modes.is_empty()
            && !modes.iter().any(|v| v.as_str() == Some(&s.hdmi_mode))
        {
            return Err(bad(
                "This HDMI mode is not advertised by the connected display",
            ));
        }
    }
    tokio::task::spawn_blocking(move || {
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        a.settings.save(s)
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"saved":true})))
}
async fn images(State(a): State<App>) -> Json<Value> {
    Json(json!({"images":assets::list(&a.settings.dir.join("images"))}))
}
async fn upload(
    State(a): State<App>,
    h: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let field = multipart
        .next_field()
        .await
        .map_err(bad)?
        .ok_or_else(|| bad("Select an image or video"))?;
    let bytes = field.bytes().await.map_err(bad)?;
    let result = tokio::task::spawn_blocking(move || {
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        assets::upload(&a.settings.dir.join("images"), &bytes)
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(result))
}
async fn image_file(State(a): State<App>, Path(id): Path<String>) -> Response {
    if !settings::valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let kind = if assets::is_video_id(&id) {
        "video/h264"
    } else {
        "image/png"
    };
    match tokio::task::spawn_blocking(move || std::fs::read(a.settings.dir.join("images").join(id)))
        .await
    {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, kind),
                (header::CACHE_CONTROL, "private, max-age=3600"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn remove_image(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    if !settings::valid_id(&id) {
        return Err(bad("Invalid image"));
    }
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        if a.settings.get().fallback_image.as_deref() == Some(&id) {
            anyhow::bail!("Select another fallback before deleting this image");
        }
        assets::remove(&a.settings.dir.join("images"), &id)?;
        Ok(())
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"deleted":true})))
}
async fn preview(State(a): State<App>) -> Response {
    let frame = a.preview.lock().unwrap();
    if let Some((time, bytes)) = &*frame
        && time.elapsed() < Duration::from_secs(2)
    {
        return (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            Bytes::copy_from_slice(bytes),
        )
            .into_response();
    }
    (
        StatusCode::NO_CONTENT,
        [(header::CACHE_CONTROL, "no-store")],
    )
        .into_response()
}
async fn usb_videos(State(a): State<App>) -> ApiResult<Json<Value>> {
    let videos = tokio::task::spawn_blocking(move || a.storage.list_videos())
        .await
        .map_err(bad)?;
    Ok(Json(json!({"videos":videos})))
}
async fn usb_video_thumbnail(State(a): State<App>, Path(id): Path<String>) -> Response {
    let Some(path) = a.storage.thumbnail(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn play_usb_video(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let path = a
        .storage
        .resolve(&id)
        .ok_or_else(|| bad("Unknown video; rescan the USB drive"))?;
    *a.usb_playback.lock().unwrap() = Some(path);
    Ok(Json(json!({"accepted":true})))
}
async fn stop_usb_video(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    *a.usb_playback.lock().unwrap() = None;
    Ok(Json(json!({"accepted":true})))
}
/// Position and duration come from mpv's IPC socket, so they are only asked
/// for while something is actually playing; otherwise this is a cheap check
/// of the requested-playback path with no process to talk to.
async fn usb_video_status(State(a): State<App>) -> Json<Value> {
    let name = a
        .usb_playback
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let Some(name) = name else {
        return Json(json!({"playing":false}));
    };
    let socket = a.usb_mpv_socket.clone();
    let (paused, position, duration) = tokio::task::spawn_blocking(move || {
        let get = |prop: &str| {
            crate::video::mpv_command(&socket, json!(["get_property", prop]))
                .ok()
                .and_then(|r| r.get("data").cloned())
        };
        (
            get("pause").and_then(|v| v.as_bool()),
            get("time-pos").and_then(|v| v.as_f64()),
            get("duration").and_then(|v| v.as_f64()),
        )
    })
    .await
    .unwrap_or_default();
    Json(
        json!({"playing":true,"name":name,"paused":paused,"position":position,"duration":duration}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseChange {
    paused: bool,
}
async fn pause_usb_video(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<PauseChange>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let socket = a.usb_mpv_socket.clone();
    tokio::task::spawn_blocking(move || {
        crate::video::mpv_command(&socket, json!(["set_property", "pause", c.paused]))
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"accepted":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeekChange {
    position: f64,
}
async fn seek_usb_video(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<SeekChange>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    if !c.position.is_finite() || c.position < 0.0 {
        return Err(bad("Invalid position"));
    }
    let socket = a.usb_mpv_socket.clone();
    tokio::task::spawn_blocking(move || {
        // mpv's IPC command array rejects a combined "absolute+exact" flag
        // string; plain "absolute" is exact by default for absolute seeks.
        crate::video::mpv_command(&socket, json!(["seek", c.position, "absolute"]))
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"accepted":true})))
}
fn recordings_dir(a: &App) -> std::path::PathBuf {
    a.settings.dir.join("recordings")
}
async fn recording_status(State(a): State<App>) -> Json<Value> {
    Json(a.recording.status())
}
async fn start_recording(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let dir = recordings_dir(&a);
    let result = tokio::task::spawn_blocking(move || a.recording.start(&dir))
        .await
        .map_err(bad)?
        .map_err(bad)?;
    *a.disk_warning.lock().unwrap() = None;
    Ok(Json(result))
}
async fn stop_recording(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    Ok(Json(
        a.recording.stop().unwrap_or(json!({"recording":false})),
    ))
}
async fn dismiss_recording_warning(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    *a.disk_warning.lock().unwrap() = None;
    Ok(Json(json!({"accepted":true})))
}
async fn save_recording_buffer(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let dir = recordings_dir(&a);
    let result = tokio::task::spawn_blocking(move || a.recording.save_buffer(&dir))
        .await
        .map_err(bad)?
        .map_err(bad)?;
    Ok(Json(result))
}
async fn list_recordings(State(a): State<App>) -> Json<Value> {
    let dir = recordings_dir(&a);
    let recordings = tokio::task::spawn_blocking(move || crate::recording::list(&dir))
        .await
        .unwrap_or_default();
    Json(json!({"recordings":recordings}))
}
async fn recording_file(State(a): State<App>, Path(name): Path<String>) -> Response {
    if !crate::recording::valid_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = recordings_dir(&a).join(&name);
    match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "video/h264".to_owned()),
                (header::CACHE_CONTROL, "private, max-age=3600".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{name}\""),
                ),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn remove_recording(
    State(a): State<App>,
    h: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let dir = recordings_dir(&a);
    tokio::task::spawn_blocking(move || crate::recording::remove(&dir, &name))
        .await
        .map_err(bad)?
        .map_err(bad)?;
    Ok(Json(json!({"deleted":true})))
}
async fn play_recording(
    State(a): State<App>,
    h: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    if !crate::recording::valid_name(&name) {
        return Err(bad("Invalid recording"));
    }
    let path = recordings_dir(&a).join(&name);
    if !path.is_file() {
        return Err(bad("Unknown recording"));
    }
    *a.usb_playback.lock().unwrap() = Some(path);
    Ok(Json(json!({"accepted":true})))
}
async fn get_wifi(State(a): State<App>) -> Json<Value> {
    let mut s = wifi::status();
    s["applying"] = json!(a.wifi_busy.load(Ordering::Relaxed));
    s["error"] = json!(a.wifi_error.lock().unwrap().clone());
    Json(s)
}
async fn save_wifi(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<wifi::Change>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    wifi::validate(&c).map_err(bad)?;
    let _operations = a.operations.lock().unwrap();
    crate::update::api::ensure_idle().map_err(bad)?;
    if a.wifi_busy.swap(true, Ordering::SeqCst) {
        return Err(bad("Wi-Fi change already in progress"));
    }
    *a.wifi_error.lock().unwrap() = None;
    // Return before restarting the AP that carries this HTTP connection.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let result = tokio::task::spawn_blocking(move || wifi::apply(c)).await;
        if let Err(e) = result
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            *a.wifi_error.lock().unwrap() = Some(e);
        }
        a.wifi_busy.store(false, Ordering::Relaxed);
    });
    Ok(Json(
        json!({"accepted":true,"message":"Wi-Fi changes in two seconds. Reconnect using the new credentials."}),
    ))
}
/// MPEG-TS for the spectator page (`/watch.html`) over one long-lived HTTP
/// connection; each connected viewer gets its own copy from the one relay
/// `stream::Controller` runs, not a separate encode per viewer.
async fn stream_ts(State(a): State<App>) -> Response {
    let Some(rx) = a.stream.subscribe() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Streaming is turned off in the web interface",
        )
            .into_response();
    };
    let body = Body::from_stream(tokio_stream::wrappers::BroadcastStream::new(rx));
    ([(header::CONTENT_TYPE, "video/mp2t")], body).into_response()
}
