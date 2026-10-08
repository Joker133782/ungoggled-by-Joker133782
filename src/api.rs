use crate::{App, assets, settings, wifi};
use axum::{
    Json, Router,
    body::Bytes,
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
