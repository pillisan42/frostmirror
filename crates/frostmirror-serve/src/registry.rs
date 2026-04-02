use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use std::path::PathBuf;

use crate::server::SharedState;

/// Routes for the Cargo sparse registry protocol and rustup dist serving.
pub fn routes(state: SharedState) -> Router {
    Router::new()
        // Sparse index: config.json
        .route("/index/config.json", get(index_config))
        // Sparse index: crate entries (1-char, 2-char, 3-char, 4+-char paths)
        .route("/index/1/{name}", get(index_entry))
        .route("/index/2/{name}", get(index_entry))
        .route("/index/3/{prefix}/{name}", get(index_entry))
        .route("/index/{a}/{b}/{name}", get(index_entry))
        // Crate downloads
        .route("/crates/{name}/{version}/download", get(download_crate))
        // Rustup dist
        .route("/rustup/dist/{target}/{filename}", get(rustup_dist))
        .with_state(state)
}

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

async fn index_entry(
    State(state): State<SharedState>,
    Path(_params): Path<Vec<(String, String)>>,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    // Reconstruct the path relative to /index/
    let uri_path = uri.path();
    let relative = uri_path.strip_prefix("/index/").unwrap_or(uri_path);
    let file_path = state.mirror_dir.join("index").join(relative);

    serve_file(&file_path, "text/plain").await
}

async fn download_crate(
    State(state): State<SharedState>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let file_path = state
        .mirror_dir
        .join("crates")
        .join(&name)
        .join(&version)
        .join("download");

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
    match tokio::fs::read(path).await {
        Ok(data) => {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", content_type.parse().unwrap());
            headers.insert("content-length", data.len().to_string().parse().unwrap());
            (StatusCode::OK, headers, Body::from(data)).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
