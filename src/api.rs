use crate::{App, assets, auth::Check, settings, terminal, wifi};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, Request, State, ws::WebSocketUpgrade,
    },
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::atomic::Ordering, time::Duration};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

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
fn session_token(h: &HeaderMap) -> Option<String> {
    let raw = h.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == "session").then(|| v.to_owned())
    })
}
fn error(status: StatusCode, message: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":message.to_string()})))
}
fn locked_out(remaining: Duration) -> (StatusCode, Json<Value>) {
    error(
        StatusCode::TOO_MANY_REQUESTS,
        format!(
            "Too many wrong passwords. Try again in {} minute(s).",
            remaining.as_secs().div_ceil(60)
        ),
    )
}
/// `auth::Store::check`, off the async runtime.
async fn check_password(a: &App, ip: std::net::IpAddr, password: String) -> ApiResult<Check> {
    // Answered straight away, so a locked-out address hammering away doesn't
    // queue up behind the checks (`check` decides again under its lock).
    if let Some(remaining) = a.auth.locked_out(ip) {
        return Ok(Check::LockedOut(remaining));
    }
    let auth = a.auth.clone();
    tokio::task::spawn_blocking(move || auth.check(ip, &password))
        .await
        .map_err(bad)
}
fn session_cookie(token: &str) -> String {
    // No `Secure`: the control interface is also reachable over plain HTTP
    // when `--tls-listen ""` disables TLS (see README), and a `Secure`
    // cookie would silently never be sent there.
    format!("session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000")
}
/// Gates everything except `/watch.html`, `/api/stream.ts` and the few
/// `/api/auth/*` routes a spectator or a not-yet-logged-in owner can reach —
/// see `router()`. A session cookie is the only thing checked here; the
/// `x-dji-control` header guard on individual handlers below is unrelated
/// (CSRF, not who's allowed in) and stays in place independently.
async fn require_auth(State(a): State<App>, req: Request, next: Next) -> Response {
    if a.auth
        .authenticated(session_token(req.headers()).as_deref())
    {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Not logged in"})),
        )
            .into_response()
    }
}
/// Same as `require_auth`, except a request from loopback is let through
/// regardless of session — see the comment on `/api/status`'s own route
/// registration in `router()` for why this one route needs that.
async fn require_auth_or_loopback(
    State(a): State<App>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if addr.ip().is_loopback()
        || a.auth
            .authenticated(session_token(req.headers()).as_deref())
    {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Not logged in"})),
        )
            .into_response()
    }
}
async fn get_status(State(a): State<App>) -> Json<Value> {
    Json(a.snapshot())
}
#[derive(Deserialize)]
struct Password {
    password: String,
}
async fn auth_status(State(a): State<App>, h: HeaderMap) -> Json<Value> {
    Json(json!({
        "configured": a.auth.configured(),
        "authenticated": a.auth.authenticated(session_token(&h).as_deref()),
    }))
}
/// Doubles as first-time setup: with no password configured yet, whatever is
/// submitted here becomes it (the frontend shows this as "Set a password"
/// rather than "Log in" by checking `/api/auth` first).
async fn login(
    State(a): State<App>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(p): Json<Password>,
) -> ApiResult<Response> {
    let ip = addr.ip();
    if let Some(remaining) = a.auth.locked_out(ip) {
        return Err(locked_out(remaining));
    }
    if a.auth.configured() {
        match check_password(&a, ip, p.password).await? {
            Check::Right => {}
            Check::Wrong => return Err(error(StatusCode::UNAUTHORIZED, "Wrong password")),
            Check::LockedOut(remaining) => return Err(locked_out(remaining)),
        }
    } else {
        if p.password.trim().is_empty() {
            return Err(bad("Password cannot be empty"));
        }
        a.auth.set_password(&p.password).map_err(bad)?;
    }
    let token = a.auth.login().map_err(bad)?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&token))],
        Json(json!({"ok":true})),
    )
        .into_response())
}
async fn logout(State(a): State<App>, h: HeaderMap) -> Json<Value> {
    if let Some(token) = session_token(&h) {
        a.auth.logout(&token);
    }
    Json(json!({"ok":true}))
}
#[derive(Deserialize)]
struct ChangePassword {
    current: String,
    new: String,
}
/// The current password is checked under the same lockout as the login page:
/// otherwise anyone holding a session (a phone left unlocked) could guess it
/// here without limit, and the same password opens the terminal.
async fn change_password(
    State(a): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(p): Json<ChangePassword>,
) -> ApiResult<Response> {
    if p.new.trim().is_empty() {
        return Err(bad("New password cannot be empty"));
    }
    match check_password(&a, addr.ip(), p.current).await? {
        Check::Right => {}
        Check::Wrong => {
            return Err(error(StatusCode::UNAUTHORIZED, "Current password is wrong"));
        }
        Check::LockedOut(remaining) => return Err(locked_out(remaining)),
    }
    a.auth.set_password(&p.new).map_err(bad)?;
    let token = a.auth.login().map_err(bad)?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&token))],
        Json(json!({"ok":true})),
    )
        .into_response())
}
pub fn router(app: App, web: std::path::PathBuf) -> Router {
    let protected = Router::new()
        .route("/api/history", get(history))
        .route("/api/diagnostics", get(|| async { Json(crate::doctor()) }))
        .route(
            "/api/hdmi-connectors",
            get(|| async { Json(crate::hdmi_connector_options()) }),
        )
        .route("/api/tls-cert.crt", get(tls_cert))
        .route("/api/support-bundle", get(support_bundle))
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
        .route("/api/images/{id}/thumbnail", get(image_thumbnail))
        .route("/api/preview.jpg", get(preview))
        .route("/api/wifi", get(get_wifi).post(save_wifi))
        .route("/api/wifi/share", get(share_wifi))
        .route(
            "/api/wifi/client",
            get(get_wifi_client)
                .post(join_wifi_client)
                .delete(leave_wifi_client),
        )
        .route("/api/wifi/scan", get(scan_wifi))
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
        .route("/api/recordings/{name}/thumbnail", get(recording_thumbnail))
        .route("/api/auth/change-password", post(change_password))
        // A root shell: see terminal.rs for every safeguard. Protected like
        // everything else, and then some — the session cookie alone isn't
        // enough to open one.
        .route("/api/terminal", get(terminal_status))
        .route("/api/terminal/session", post(terminal_session))
        .route("/api/terminal/ws", get(terminal_ws))
        .layer(DefaultBodyLimit::max(assets::MAX_VIDEO_UPLOAD + 65536))
        .merge(crate::update::api::routes())
        // Applied here, before the merge below, so it covers every route
        // above (including the just-merged update routes) but not the
        // public routes merged in next — see `require_auth`.
        .route_layer(middleware::from_fn_with_state(app.clone(), require_auth));
    // A spectator given the `/watch.html` link never logs in: only these,
    // the `/static/*` build assets and the app shell below stay reachable
    // without a session.
    let public = Router::new()
        .route("/api/stream.ts", get(stream_ts))
        .route("/api/auth", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout));
    // `/api/status` also accepts an unauthenticated request, but only from
    // loopback: the update mechanism's own health check
    // (`update/install.rs::healthy`, `curl http://127.0.0.1/api/status`) is
    // a local process on the Pi with no session cookie to send, and
    // unconditionally requiring one here failed that check 100% of the
    // time, rolling every single update back (confirmed in the field,
    // 2026-11-01 — the cost of not having tested the real update path, only
    // hand-deploys that bypass it). A Wi-Fi client's TCP connection can
    // never appear as loopback; only a process already running on the Pi
    // can, so this doesn't reopen read access to spectators or anyone else
    // on the network — just the same read-only status this endpoint always
    // served, to the one caller that was always meant to reach it
    // unauthenticated.
    let status_router = Router::new()
        .route("/api/status", get(get_status))
        .route_layer(middleware::from_fn_with_state(
            app.clone(),
            require_auth_or_loopback,
        ));
    protected
        .merge(public)
        .merge(status_router)
        // `/static/*` filenames are content-hashed by the build, so they
        // never change under a given URL and can be cached indefinitely;
        // everything else (chiefly index.html) must always be revalidated,
        // or a browser that cached it before a deploy never notices the new
        // build exists and keeps running stale JS indefinitely.
        .nest_service(
            "/static",
            Router::new()
                .fallback_service(ServeDir::new(web.join("static")))
                .layer(SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                )),
        )
        .fallback_service(Router::new().fallback_service(ServeDir::new(web)).layer(
            SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            ),
        ))
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
    let always_ap_changed = s.wifi_always_ap != a.settings.get().wifi_always_ap;
    let always_ap = s.wifi_always_ap;
    tokio::task::spawn_blocking(move || {
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        let result = a.settings.save(s);
        // Keeps a client profile saved from an earlier join in sync with a
        // toggle flipped afterwards; a no-op if none has been created yet.
        if result.is_ok() && always_ap_changed {
            wifi::set_client_autoconnect(always_ap);
        }
        result
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct HistoryQuery {
    since: Option<f64>,
}
async fn history(State(a): State<App>, Query(q): Query<HistoryQuery>) -> Json<Value> {
    Json(a.history.lock().unwrap().snapshot(q.since))
}
async fn images(State(a): State<App>) -> ApiResult<Json<Value>> {
    let dir = a.settings.dir.join("images");
    tokio::task::spawn_blocking(move || {
        Json(json!({"images":assets::list(&dir),"limits":assets::limits(&dir)}))
    })
    .await
    .map_err(bad)
}
/// A small JPEG for the idle-screen list, made on first request if the
/// upload didn't already leave one (see `assets::thumbnail`).
async fn image_thumbnail(State(a): State<App>, Path(id): Path<String>) -> Response {
    let dir = a.settings.dir.join("images");
    let thumb = tokio::task::spawn_blocking(move || {
        assets::thumbnail(&dir, &id).and_then(|p| std::fs::read(p).ok())
    })
    .await;
    match thumb {
        Ok(Some(bytes)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                // Ids are never reused for different content, and the
                // built-in posters only change with an update.
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
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
    let name = field.file_name().map(str::to_owned);
    let bytes = field.bytes().await.map_err(bad)?;
    let result = tokio::task::spawn_blocking(move || {
        let _lock = a.operations.lock().unwrap();
        crate::update::api::ensure_idle()?;
        assets::upload(&a.settings.dir.join("images"), &bytes, name.as_deref())
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(result))
}
/// Lets a device trust the self-signed TLS certificate as a CA (Android:
/// Settings → Security → Encryption & credentials → Install a certificate),
/// which turns the one-time "connection isn't private" warning into a
/// properly trusted connection and unlocks Chrome's real "Install app"
/// prompt instead of just a home-screen shortcut. The certificate itself
/// isn't sensitive — it has no matching private key exposure, it's exactly
/// what every visiting browser already receives during the TLS handshake.
/// A downloadable diagnostics bundle for reporting a problem — gathers
/// recent logs, update history, hardware diagnostics and the current
/// settings/status into one `.tar.gz`, so it can be pulled even from a
/// phone and shared without SSH access.
async fn support_bundle(State(a): State<App>) -> Response {
    let settings = serde_json::to_value(a.settings.get()).unwrap_or_default();
    let status = a.snapshot();
    match tokio::task::spawn_blocking(move || crate::support::build(&settings, &status)).await {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "application/gzip"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"ungoggled-diagnostics.tar.gz\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn tls_cert(State(a): State<App>) -> Response {
    match tokio::fs::read(a.settings.dir.join("tls-cert.pem")).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"ungoggled-ca.crt\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
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
            anyhow::bail!("Pick another idle screen before deleting this one");
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
    let already_loaded = a.usb_playback.lock().unwrap().as_deref() == Some(path.as_path());
    if already_loaded {
        // mpv stays running (and idle) after reaching EOF by design, so this
        // same path is indistinguishable from "nothing changed" to the
        // player-spawn loop, which only reacts to a *change* — replaying a
        // just-finished (or still-playing) clip needs telling the already-
        // running mpv directly instead.
        let socket = a.usb_mpv_socket.clone();
        let target = path.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            crate::video::mpv_command(&socket, json!(["loadfile", target]))
        })
        .await
        .map_err(bad)?
        .map_err(bad)?;
    } else {
        *a.usb_playback.lock().unwrap() = Some(path);
    }
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
    let drives = a.storage.drives();
    let scan = a
        .storage
        .scan_progress()
        .map(|(done, total)| json!({"done":done,"total":total}));
    let name = a
        .usb_playback
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let Some(name) = name else {
        return Json(json!({"playing":false,"drives":drives,"scan":scan}));
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
        json!({"playing":true,"name":name,"paused":paused,"position":position,"duration":duration,"drives":drives,"scan":scan}),
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
    if a.settings.get().record_to_usb
        && let Some(mountpoint) = a.storage.recording_mountpoint()
    {
        return mountpoint.join("ungoggled-recordings");
    }
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
    let recordings = tokio::task::spawn_blocking(move || a.recording.list(&dir))
        .await
        .unwrap_or_default();
    Json(json!({"recordings":recordings}))
}
async fn recording_thumbnail(State(a): State<App>, Path(name): Path<String>) -> Response {
    let Some(path) = a.recording.thumbnail(&name) else {
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
/// The access point's SSID and password, for the spectator share sheet's
/// Wi-Fi QR code. Its own route rather than a field of `/api/wifi`, which the
/// Settings tab polls every few seconds: the password only crosses the
/// network when someone actually opens the share sheet.
async fn share_wifi() -> ApiResult<Response> {
    let share = tokio::task::spawn_blocking(wifi::share)
        .await
        .map_err(bad)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(share)).into_response())
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
async fn scan_wifi(State(a): State<App>) -> ApiResult<Json<Value>> {
    // Shares wifi_busy with join/leave so a scan (nmcli itself) never runs
    // concurrently with those against the same radio.
    if a.wifi_busy.swap(true, Ordering::SeqCst) {
        return Err(bad("Wi-Fi change already in progress"));
    }
    let result = tokio::task::spawn_blocking(wifi::scan).await;
    a.wifi_busy.store(false, Ordering::Relaxed);
    result.map_err(bad)?.map(Json).map_err(bad)
}
async fn get_wifi_client(State(a): State<App>) -> Json<Value> {
    let mut s = wifi::client_status();
    s["applying"] = json!(a.wifi_busy.load(Ordering::Relaxed));
    s["error"] = json!(a.wifi_error.lock().unwrap().clone());
    Json(s)
}
async fn join_wifi_client(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<wifi::JoinRequest>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    wifi::validate_join(&c).map_err(bad)?;
    let _operations = a.operations.lock().unwrap();
    crate::update::api::ensure_idle().map_err(bad)?;
    if a.wifi_busy.swap(true, Ordering::SeqCst) {
        return Err(bad("Wi-Fi change already in progress"));
    }
    *a.wifi_error.lock().unwrap() = None;
    let always_ap = a.settings.get().wifi_always_ap;
    // Return before switching away from the AP that carries this HTTP connection.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let result = tokio::task::spawn_blocking(move || wifi::join(&c, always_ap)).await;
        if let Err(e) = result
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            *a.wifi_error.lock().unwrap() = Some(e);
        }
        a.wifi_busy.store(false, Ordering::Relaxed);
    });
    Ok(Json(
        json!({"accepted":true,"message":"Joining the network in two seconds. Reconnect from a device on that network."}),
    ))
}
async fn leave_wifi_client(State(a): State<App>, h: HeaderMap) -> ApiResult<Json<Value>> {
    guard(&h)?;
    let _operations = a.operations.lock().unwrap();
    crate::update::api::ensure_idle().map_err(bad)?;
    if a.wifi_busy.swap(true, Ordering::SeqCst) {
        return Err(bad("Wi-Fi change already in progress"));
    }
    *a.wifi_error.lock().unwrap() = None;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let result = tokio::task::spawn_blocking(wifi::leave_client).await;
        if let Err(e) = result
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            *a.wifi_error.lock().unwrap() = Some(e);
        }
        a.wifi_busy.store(false, Ordering::Relaxed);
    });
    Ok(Json(
        json!({"accepted":true,"message":"Returning to the access point in two seconds."}),
    ))
}
// ---------------------------------------------------------------------------
// Terminal: a root shell in the browser. Every safeguard is listed at the top
// of terminal.rs; the ones that need the request live here.
// ---------------------------------------------------------------------------
async fn terminal_status(State(a): State<App>) -> Json<Value> {
    let limits = terminal::Limits::from_env();
    Json(json!({
        "enabled": a.settings.get().terminal_enabled,
        "active": a.terminals.active(),
        "idle_seconds": limits.idle.as_secs(),
        "max_seconds": limits.max.as_secs(),
    }))
}
fn terminal_off() -> (StatusCode, Json<Value>) {
    error(
        StatusCode::FORBIDDEN,
        "The terminal is turned off (Settings → Terminal)",
    )
}
fn terminal_busy() -> (StatusCode, Json<Value>) {
    error(
        StatusCode::CONFLICT,
        "A terminal is already open (one at a time). Close it, or wait until it times out.",
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUp {
    password: String,
}
/// Step-up: the password once more, however fresh the session, in exchange
/// for a ticket that opens one terminal over `terminal_ws`. Wrong passwords
/// count against the same lockout as the login page.
async fn terminal_session(
    State(a): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    Json(p): Json<StepUp>,
) -> ApiResult<Json<Value>> {
    guard(&h)?;
    if !a.settings.get().terminal_enabled {
        return Err(terminal_off());
    }
    crate::update::api::ensure_idle().map_err(|e| error(StatusCode::CONFLICT, e))?;
    // `require_auth` already accepted this cookie; the ticket is bound to it.
    let session =
        session_token(&h).ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Not logged in"))?;
    // Checked again when the ticket is issued; this just spares a password
    // attempt that couldn't lead anywhere.
    if a.terminals.active() {
        return Err(terminal_busy());
    }
    let ip = addr.ip();
    match check_password(&a, ip, p.password).await? {
        Check::Right => {}
        Check::Wrong => {
            eprintln!("terminal: wrong password from {ip}");
            return Err(error(StatusCode::UNAUTHORIZED, "Wrong password"));
        }
        Check::LockedOut(remaining) => return Err(locked_out(remaining)),
    }
    let ticket = crate::auth::random_token().map_err(bad)?;
    a.terminals
        .issue(&session, ticket.clone())
        .map_err(|_| terminal_busy())?;
    Ok(Json(json!({
        "ticket": ticket,
        "expires_in": terminal::TICKET_TTL.as_secs(),
    })))
}
#[derive(Deserialize)]
struct TicketQuery {
    ticket: String,
}
/// The terminal's WebSocket. A browser can't put the `X-DJI-Control` header
/// on a WebSocket handshake, so the `Origin` check stands in for it here: it
/// is what stops a page on some other site from opening one with this
/// browser's cookie.
async fn terminal_ws(
    State(a): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    uri: Uri,
    Query(q): Query<TicketQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let ip = addr.ip();
    if !terminal::same_origin(&h, &uri) {
        eprintln!("terminal: refused a handshake from another origin ({ip})");
        return error(StatusCode::FORBIDDEN, "Wrong origin").into_response();
    }
    if !a.settings.get().terminal_enabled {
        return terminal_off().into_response();
    }
    if let Err(e) = crate::update::api::ensure_idle() {
        return error(StatusCode::CONFLICT, e).into_response();
    }
    let Some(session) = session_token(&h) else {
        return error(StatusCode::UNAUTHORIZED, "Not logged in").into_response();
    };
    let slot = match a.terminals.open(&q.ticket, &session) {
        Ok(slot) => slot,
        Err(terminal::Refused::Busy) => {
            eprintln!("terminal: refused a second terminal from {ip}");
            return terminal_busy().into_response();
        }
        Err(terminal::Refused::InvalidTicket) => {
            eprintln!("terminal: refused an unknown, used or expired ticket from {ip}");
            return error(
                StatusCode::FORBIDDEN,
                "This terminal link is no longer valid. Open the terminal again.",
            )
            .into_response();
        }
    };
    let limits = terminal::Limits::from_env();
    let app = a.clone();
    let stop = move || {
        if app.shutdown.load(Ordering::Relaxed) {
            Some("shutdown")
        } else if !app.settings.get().terminal_enabled {
            Some("disabled")
        } else if !app.auth.authenticated(Some(&session)) {
            Some("signed-out")
        } else {
            None
        }
    };
    // The slot moves into the upgrade callback: if the upgrade never
    // completes, dropping the callback frees it again.
    ws.max_message_size(terminal::MAX_FRAME)
        .max_frame_size(terminal::MAX_FRAME)
        .on_upgrade(move |socket| terminal::serve(socket, slot, ip, limits, stop))
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
