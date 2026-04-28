use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use frostmirror_core::manifest::{BundleType, Manifest};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::proxy;
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

    // Sub-router for /dist — serves channel manifests and toolchain component archives.
    let dist_router = Router::new()
        .fallback(dist_fallback)
        .with_state(state.clone());

    Router::new()
        .nest("/index", index_router)
        .nest("/crates", crates_router)
        .nest("/dist", dist_router)
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
    let upstream_url = format!(
        "{}/{}",
        state.proxy_index_url.trim_end_matches('/'),
        relative
    );
    serve_or_proxy(&state, &file_path, "text/plain", upstream_url, UpstreamKind::Index).await
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
    let upstream_url = format!(
        "{}/{}",
        state.proxy_dl_url.trim_end_matches('/'),
        relative
    );
    serve_or_proxy(
        &state,
        &file_path,
        "application/octet-stream",
        upstream_url,
        UpstreamKind::Crate {
            relative: relative.to_string(),
        },
    )
    .await
}

/// Fallback handler for toolchain distribution files.
///
/// Serves channel manifests (e.g. `channel-rust-stable.toml`) and component
/// archives (e.g. `2024-01-09/rustc-1.75.0-x86_64-unknown-linux-gnu.tar.xz`)
/// from the `mirror/dist/` directory.
async fn dist_fallback(State(state): State<SharedState>, req: Request) -> Response {
    let uri_path = req.uri().path();
    let relative = uri_path.strip_prefix('/').unwrap_or(uri_path);

    if relative.is_empty() || relative.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    let file_path = state.mirror_dir.join("dist").join(relative);

    // Determine content type from extension
    let content_type = if relative.ends_with(".toml") {
        "text/plain"
    } else if relative.ends_with(".sha256") {
        "text/plain"
    } else {
        "application/octet-stream"
    };

    let upstream_url = format!(
        "{}/dist/{}",
        state.proxy_dist_url.trim_end_matches('/'),
        relative
    );
    serve_or_proxy(&state, &file_path, content_type, upstream_url, UpstreamKind::Dist).await
}

async fn rustup_dist(
    State(state): State<SharedState>,
    Path((target, filename)): Path<(String, String)>,
) -> Response {
    if target.contains("..") || filename.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    let file_path = state
        .mirror_dir
        .join("rustup")
        .join("dist")
        .join(&target)
        .join(&filename);

    let upstream_url = format!(
        "{}/rustup/dist/{}/{}",
        state.proxy_dist_url.trim_end_matches('/'),
        target,
        filename
    );
    serve_or_proxy(
        &state,
        &file_path,
        "application/octet-stream",
        upstream_url,
        UpstreamKind::Rustup,
    )
    .await
}

#[derive(Debug)]
enum UpstreamKind {
    Index,
    Crate { relative: String },
    Dist,
    Rustup,
}

/// Serve `path` from disk, or — when `proxy_mode` is enabled and the file is
/// missing — fetch from `upstream_url`, cache it to `path`, and serve the
/// freshly downloaded bytes.
async fn serve_or_proxy(
    state: &SharedState,
    path: &PathBuf,
    content_type: &str,
    upstream_url: String,
    kind: UpstreamKind,
) -> Response {
    // Cache hit: serve from disk.
    if let Ok(data) = tokio::fs::read(path).await {
        return ok_response(data, content_type);
    }

    // Cache miss + proxy off → 404 (matches the offline workflow).
    if !state.proxy_mode {
        tracing::debug!("file not found: {}", path.display());
        return (
            StatusCode::NOT_FOUND,
            format!("not found: {}", path.display()),
        )
            .into_response();
    }

    // Cache miss + proxy on → fetch upstream.
    let bytes = match proxy::fetch_and_cache(&state.http_client, &upstream_url, path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("proxy fetch failed for {}: {:#}", upstream_url, e);
            return (
                StatusCode::BAD_GATEWAY,
                format!("upstream fetch failed: {}", e),
            )
                .into_response();
        }
    };

    // For crates, persist the entry into manifest.json so GC keeps it.
    if let UpstreamKind::Crate { relative } = &kind {
        if let Some((name, version)) = parse_crate_relative(relative) {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let sha = hex::encode(hasher.finalize());
            let size = bytes.len() as u64;
            if let Err(e) = append_crate_to_manifest(state, &name, &version, sha, size).await {
                // Non-fatal: the crate is cached and serving fine, but log
                // loudly so operators notice manifest drift.
                tracing::error!(
                    "failed to record proxy-cached crate {}-{} in manifest: {:#}",
                    name, version, e
                );
            }
        }
    }

    ok_response(bytes, content_type)
}

fn ok_response(data: Vec<u8>, content_type: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", content_type.parse().unwrap());
    headers.insert("content-length", data.len().to_string().parse().unwrap());
    (StatusCode::OK, headers, Body::from(data)).into_response()
}

/// Parse `{name}/{version}/download` into `(name, version)`.
fn parse_crate_relative(relative: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = relative.split('/').collect();
    if parts.len() == 3 && parts[2] == "download" && !parts[0].is_empty() && !parts[1].is_empty() {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Append a proxy-cached crate to mirror/manifest.json under the manifest_lock.
async fn append_crate_to_manifest(
    state: &SharedState,
    name: &str,
    version: &str,
    sha256: String,
    size: u64,
) -> anyhow::Result<()> {
    let _guard = state.manifest_lock.lock().await;
    let manifest_path = state.mirror_dir.join("manifest.json");

    let mut manifest = if manifest_path.exists() {
        let content = tokio::fs::read_to_string(&manifest_path).await?;
        Manifest::from_json(&content)?
    } else {
        Manifest::new(BundleType::Full, None, Vec::new(), "stable".to_string())
    };

    manifest.add_crate(name.to_string(), version.to_string(), sha256, size);
    manifest.seal();

    let json = manifest.to_json()?;
    tokio::fs::write(&manifest_path, json).await?;
    Ok(())
}
