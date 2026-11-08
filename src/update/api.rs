use super::{Lock, Paths, Phase};
use crate::App;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tokio::io::AsyncWriteExt;
type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn bad(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn guard(h: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    crate::guard(h).map_err(|s| (s, Json(json!({"error":"Missing control header"}))))
}
pub fn routes() -> Router<App> {
    Router::new()
        .route(
            "/api/update",
            get(|| async { Json(super::information(&Paths::default())) }),
        )
        .route(
            "/api/update/upload",
            post(upload).layer(DefaultBodyLimit::max(super::package::MAX_UPLOAD + 65536)),
        )
        .route("/api/update/install", post(install))
        .route(
            "/api/update/releases",
            get(|| async { Json(super::releases_information(&Paths::default())) }),
        )
        .route("/api/update/rollback", post(rollback))
}
pub fn ensure_idle() -> anyhow::Result<()> {
    anyhow::ensure!(
        !Paths::default().state()?.phase.busy(),
        "Settings are locked while the update is being installed"
    );
    Ok(())
}
async fn upload(headers: HeaderMap, multipart: Multipart) -> Response {
    guard(&headers)?;
    if !super::supported() {
        return Err(bad("Web updates require an installed Pi service"));
    }
    let paths = Paths::default();
    let _lock = Lock::acquire(&paths).map_err(bad)?;
    let (id, job) = super::new_job(&paths).map_err(bad)?;
    let archive = job.join("upload.tar.gz");
    let result = async {
        receive_upload(multipart, &archive).await?;
        let id = id.clone();
        let paths = paths.clone();
        let state = tokio::task::spawn_blocking(move || {
            let _lock = _lock; // Keep ownership if the HTTP client disconnects.
            super::prepare(&paths, &id, &archive, super::package::architecture())
        })
        .await
        .map_err(bad)?
        .map_err(bad)?;
        Ok(Json(json!(state)))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(job).await;
    }
    result
}
// Separate transport parsing from staging/installation so malformed requests can
// be tested without root, systemd, or Pi hardware.
async fn receive_upload(
    mut multipart: Multipart,
    archive: &std::path::Path,
) -> Result<(), (StatusCode, Json<Value>)> {
    let mut field = multipart
        .next_field()
        .await
        .map_err(bad)?
        .ok_or_else(|| bad("Choose an update file"))?;
    let mut file = tokio::fs::File::create(&archive).await.map_err(bad)?;
    let mut total = 0;
    while let Some(chunk) = field.chunk().await.map_err(bad)? {
        total += chunk.len();
        if total > super::package::MAX_UPLOAD {
            return Err(bad("Update exceeds 64 MiB"));
        }
        file.write_all(&chunk).await.map_err(bad)?;
    }
    // Multer permits only one live Field handle, even after its data ends.
    drop(field);
    file.sync_all().await.map_err(bad)?;
    drop(file);
    if multipart.next_field().await.map_err(bad)?.is_some() {
        return Err(bad("Upload one update file at a time"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    id: String,
    /// An escape hatch for a health check that is itself broken — see
    /// `install::switch`. Only meaningful as a retry of a job that already
    /// failed its health check; defaults off for every ordinary install.
    #[serde(default)]
    skip_health_check: bool,
}
async fn install(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Install>,
) -> Response {
    guard(&headers)?;
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let _operations = app.operations.lock().unwrap();
        anyhow::ensure!(
            !app.wifi_busy.load(Ordering::Relaxed),
            "Wait for the Wi-Fi change to finish"
        );
        let paths = Paths::default();
        let _lock = Lock::acquire(&paths)?;
        let phase = paths.state()?.phase;
        anyhow::ensure!(
            matches!(phase, Phase::Ready | Phase::RolledBack | Phase::Failed),
            "No update ready to install"
        );
        super::launch(&paths, &request.id, request.skip_health_check)
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"accepted":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rollback {
    release: String,
}
async fn rollback(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Rollback>,
) -> Response {
    guard(&headers)?;
    let id = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let _operations = app.operations.lock().unwrap();
        anyhow::ensure!(
            !app.wifi_busy.load(Ordering::Relaxed),
            "Wait for the Wi-Fi change to finish"
        );
        let paths = Paths::default();
        let _lock = Lock::acquire(&paths)?;
        super::request_rollback(&paths, &request.release)
    })
    .await
    .map_err(bad)?
    .map_err(bad)?;
    Ok(Json(json!({"accepted":true,"id":id})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, extract::FromRequest, http::Request};
    #[tokio::test]
    async fn multipart_accepts_one_file_and_rejects_multiple() {
        let fixture = crate::update::tests::Fixture::new();
        for count in [1, 2] {
            let mut body = String::new();
            for _ in 0..count {
                body.push_str("--test-boundary\r\nContent-Disposition: form-data; name=\"update\"; filename=\"release.gz\"\r\nContent-Type: application/gzip\r\n\r\nupdate bytes\r\n");
            }
            body.push_str("--test-boundary--\r\n");
            let request = Request::builder()
                .header(
                    "Content-Type",
                    "multipart/form-data; boundary=test-boundary",
                )
                .body(Body::from(body))
                .unwrap();
            let multipart = Multipart::from_request(request, &()).await.unwrap();
            let file = fixture.0.join("upload.gz");
            let result = receive_upload(multipart, &file).await;
            if count == 1 {
                result.unwrap();
                assert_eq!(std::fs::read(file).unwrap(), b"update bytes");
            } else {
                assert_eq!(
                    result.unwrap_err().1.0["error"],
                    "Upload one update file at a time"
                );
            }
        }
    }
}
