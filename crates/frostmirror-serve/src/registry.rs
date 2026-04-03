use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use std::path::PathBuf;

use crate::server::SharedState;

/// Routes for the Cargo sparse registry protocol and rustup dist serving.
///
/// The index is mounted via `nest` so its routes are isolated and cannot
/// conflict with routes defined in the web or API routers.
pub fn routes(state: SharedState) -> Router {
    // Sub-router for /index — uses fallback to handle arbitrary crate paths
    let index_router = Router::new()
        .route("/config.json", get(index_config))
        .fallback(index_fallback)
        .with_state(state.clone());

    // Sub-router for /crates — uses fallback to handle download requests.
    // This avoids issues with axum route matching and URL encoding.
    let crates_router = Router::new()
        .fallback(crates_fallback)
        .with_state(state.clone());

    Router::new()
        .nest("/index", index_router)
        .nest("/crates", crates_router)
        // Rustup dist
        .route("/rustup/dist/{target}/{filename}", get(rustup_dist))
        .with_state(state)
}

/// Dynamically generated sparse index config.json.
async fn index_config(State(state): State<SharedState>) -> impl IntoResponse {
    let config = serde_json::json!({
        "dl": format!("{}/crates/{{crate}}/{{version}}/download", state.base_url),
        "api": state.base_url
    });

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::to_string(&config).unwrap(),
    )
}

/// Fallback handler for all index paths.
async fn index_fallback(State(state): State<SharedState>, req: Request) -> Response {
    let uri_path = req.uri().path();
    let relative = uri_path.strip_prefix('/').unwrap_or(uri_path);

    if relative.is_empty() {
        return (StatusCode::NOT_FOUND, "index: empty path").into_response();
    }

    if relative.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    let file_path = state.mirror_dir.join("index").join(relative);
    serve_file(&file_path, "text/plain").await
}

/// Fallback handler for crate downloads.
///
/// Handles requests like `/crates/aho-corasick/1.1.4/download`.
/// Because this is nested under `/crates`, axum strips that prefix.
/// We receive e.g. `/aho-corasick/1.1.4/download`.
async fn crates_fallback(State(state): State<SharedState>, req: Request) -> Response {
    let uri_path = req.uri().path();
    let relative = uri_path.strip_prefix('/').unwrap_or(uri_path);

    tracing::debug!("crate download request: {}", relative);

    if relative.is_empty() || relative.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    // Expected format: {name}/{version}/download
    // Serve the file directly from mirror/crates/{relative}
    let file_path = state.mirror_dir.join("crates").join(relative);
    serve_file(&file_path, "application/octet-stream").await
}

async fn rustup_dist(
    State(state): State<SharedState>,
    Path((target, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let file_path = state
        .mirror_dir
        .join("rustup")
        .join("dist")
        .join(&target)
        .join(&filename);

    serve_file(&file_path, "application/octet-stream").await
}

async fn serve_file(path: &PathBuf, content_type: &str) -> Response {
    tracing::debug!("serving file: {}", path.display());

    match tokio::fs::read(path).await {
        Ok(data) => {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", content_type.parse().unwrap());
            headers.insert("content-length", data.len().to_string().parse().unwrap());
            (StatusCode::OK, headers, Body::from(data)).into_response()
        }
        Err(e) => {
            tracing::debug!("file not found: {} ({})", path.display(), e);
            (StatusCode::NOT_FOUND, format!("not found: {}", path.display())).into_response()
        }
    }
}
