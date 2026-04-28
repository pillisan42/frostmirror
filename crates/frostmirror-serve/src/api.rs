use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use frostmirror_core::config::FrostmirrorConfig;
use frostmirror_core::depends::DependsToml;
use frostmirror_import::{Exporter, GarbageCollector, Importer};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::server::SharedState;

pub fn routes(state: SharedState) -> Router {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/packages", get(get_packages))
        .route("/api/config", get(get_config).post(post_config))
        .route("/api/deps", get(get_deps).post(post_deps))
        .route("/api/incoming", get(get_incoming))
        .route("/api/gc", post(post_gc))
        .route("/api/export", post(post_export).get(get_exports))
        .route("/api/export/download/{filename}", get(download_export))
        .route("/api/setup/cargo-config", get(get_cargo_config))
        .route("/api/setup/rustup-env.sh", get(get_rustup_env_sh))
        .route("/api/setup/rustup-env.ps1", get(get_rustup_env_ps1))
        .with_state(state)
}

#[derive(Serialize)]
struct StatusResponse {
    crate_count: u64,
    total_size: u64,
    total_size_human: String,
    last_import: Option<String>,
    watcher_active: bool,
    done_count: u64,
    failed_count: u64,
}

async fn get_status(State(state): State<SharedState>) -> impl IntoResponse {
    let mirror_dir = state.mirror_dir.clone();
    let incoming_dir = state.incoming_dir.clone();
    let watcher_active = *state.watcher_active.read().await;

    // Run blocking I/O (walkdir, file stat) off the async runtime
    let result = tokio::task::spawn_blocking(move || {
        let importer = Importer::new(mirror_dir);
        let status = importer.status().unwrap_or(frostmirror_import::importer::MirrorStatus {
            crate_count: 0,
            total_size: 0,
            last_import: None,
        });
        let done_count = count_files_in(&incoming_dir.join("done")).unwrap_or(0);
        let failed_count = count_files_in(&incoming_dir.join("failed")).unwrap_or(0);
        (status, done_count, failed_count)
    })
    .await;

    let (status, done_count, failed_count) = result.unwrap_or_else(|_| {
        (
            frostmirror_import::importer::MirrorStatus {
                crate_count: 0,
                total_size: 0,
                last_import: None,
            },
            0,
            0,
        )
    });

    Json(StatusResponse {
        crate_count: status.crate_count,
        total_size: status.total_size,
        total_size_human: human_size(status.total_size),
        last_import: status.last_import,
        watcher_active,
        done_count,
        failed_count,
    })
}

#[derive(Serialize)]
struct PackageEntry {
    filename: String,
    size: u64,
    status: String,
}

async fn get_packages(State(state): State<SharedState>) -> impl IntoResponse {
    let incoming_dir = state.incoming_dir.clone();

    let packages = tokio::task::spawn_blocking(move || {
        let mut packages = Vec::new();

        // List done packages
        if let Ok(entries) = std::fs::read_dir(incoming_dir.join("done")) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    packages.push(PackageEntry {
                        filename: entry.file_name().to_string_lossy().to_string(),
                        size: meta.len(),
                        status: "imported".to_string(),
                    });
                }
            }
        }

        // List failed packages
        if let Ok(entries) = std::fs::read_dir(incoming_dir.join("failed")) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    packages.push(PackageEntry {
                        filename: entry.file_name().to_string_lossy().to_string(),
                        size: meta.len(),
                        status: "failed".to_string(),
                    });
                }
            }
        }

        packages.sort_by(|a, b| b.filename.cmp(&a.filename));
        packages
    })
    .await
    .unwrap_or_default();

    Json(packages)
}

async fn get_config(State(state): State<SharedState>) -> impl IntoResponse {
    match FrostmirrorConfig::load(&state.config_path) {
        Ok(config) => (StatusCode::OK, Json(serde_json::to_value(config).unwrap())).into_response(),
        Err(_) => {
            let config = FrostmirrorConfig::default();
            (StatusCode::OK, Json(serde_json::to_value(config).unwrap())).into_response()
        }
    }
}

async fn post_config(
    State(state): State<SharedState>,
    Json(config): Json<FrostmirrorConfig>,
) -> impl IntoResponse {
    match config.save(&state.config_path) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_deps(State(state): State<SharedState>) -> impl IntoResponse {
    match DependsToml::load(&state.depends_path) {
        Ok(deps) => (StatusCode::OK, Json(serde_json::to_value(deps).unwrap())).into_response(),
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "dependencies": {},
                "platforms": { "targets": ["x86_64-unknown-linux-gnu"], "toolchain": "stable" }
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct DepsUpdate {
    dependencies: std::collections::BTreeMap<String, frostmirror_core::depends::DepEntry>,
    #[serde(default)]
    platforms: Option<frostmirror_core::depends::Platforms>,
}

async fn post_deps(
    State(state): State<SharedState>,
    Json(update): Json<DepsUpdate>,
) -> impl IntoResponse {
    let deps = DependsToml {
        dependencies: update.dependencies,
        platforms: update.platforms.unwrap_or_default(),
    };

    match deps.to_toml_string() {
        Ok(toml_str) => match std::fs::write(&state.depends_path, toml_str) {
            Ok(_) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            ),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

#[derive(Serialize)]
struct IncomingStatus {
    watcher_active: bool,
    done_count: u64,
    failed_count: u64,
    pending_files: Vec<String>,
}

async fn get_incoming(State(state): State<SharedState>) -> impl IntoResponse {
    let watcher_active = *state.watcher_active.read().await;
    let done_count = count_files_in(&state.incoming_dir.join("done")).unwrap_or(0);
    let failed_count = count_files_in(&state.incoming_dir.join("failed")).unwrap_or(0);

    let mut pending_files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&state.incoming_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".pkg") {
                pending_files.push(name);
            }
        }
    }

    Json(IncomingStatus {
        watcher_active,
        done_count,
        failed_count,
        pending_files,
    })
}

async fn post_gc(State(state): State<SharedState>) -> impl IntoResponse {
    let mirror_dir = state.mirror_dir.clone();

    let result = tokio::task::spawn_blocking(move || {
        let gc = GarbageCollector::new(mirror_dir);
        gc.run()
    })
    .await;

    match result {
        Ok(Ok(gc_result)) => (StatusCode::OK, Json(serde_json::to_value(gc_result).unwrap())),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("gc task failed: {}", e)})),
        ),
    }
}

#[derive(Deserialize, Default)]
struct CargoConfigQuery {
    /// When set to "false", append `[http]\ncheck-revoke = false` so cargo
    /// skips certificate revocation checks (workaround for misconfigured
    /// SSL/CRL endpoints, common on Windows clients).
    #[serde(default)]
    check_revoke: Option<String>,
}

async fn get_cargo_config(
    State(state): State<SharedState>,
    Query(query): Query<CargoConfigQuery>,
) -> impl IntoResponse {
    let mut config = format!(
        r#"[source.frostmirror]
registry = "sparse+{base_url}/index/"

[source.crates-io]
replace-with = "frostmirror"
"#,
        base_url = state.base_url
    );

    if query.check_revoke.as_deref() == Some("false") {
        config.push_str(
            "\n[http]\n# Skip TLS revocation checks. Use only when CRL/OCSP\n# endpoints are unreachable or the mirror uses a self-signed cert.\ncheck-revoke = false\n",
        );
    }

    (
        StatusCode::OK,
        [
            ("content-type", "text/plain"),
            (
                "content-disposition",
                "attachment; filename=\"config.toml\"",
            ),
        ],
        config,
    )
}

async fn get_rustup_env_sh(State(state): State<SharedState>) -> impl IntoResponse {
    let script = format!(
        r#"#!/bin/bash
# frostmirror rustup environment setup
export RUSTUP_DIST_SERVER={base_url}
export RUSTUP_UPDATE_ROOT={base_url}/rustup
"#,
        base_url = state.base_url
    );

    (
        StatusCode::OK,
        [
            ("content-type", "text/x-shellscript"),
            (
                "content-disposition",
                "attachment; filename=\"rustup-env.sh\"",
            ),
        ],
        script,
    )
}

async fn get_rustup_env_ps1(State(state): State<SharedState>) -> impl IntoResponse {
    let script = format!(
        r#"# frostmirror rustup environment setup (PowerShell)
$env:RUSTUP_DIST_SERVER = "{base_url}"
$env:RUSTUP_UPDATE_ROOT = "{base_url}/rustup"
"#,
        base_url = state.base_url
    );

    (
        StatusCode::OK,
        [
            ("content-type", "text/plain"),
            (
                "content-disposition",
                "attachment; filename=\"rustup-env.ps1\"",
            ),
        ],
        script,
    )
}

#[derive(Serialize)]
struct ExportSummary {
    filename: String,
    size: u64,
    crate_count: usize,
    rustup_count: usize,
    dist_count: usize,
}

async fn post_export(State(state): State<SharedState>) -> impl IntoResponse {
    // Single-flight guard: refuse if an export is already running. This avoids
    // two concurrent walks competing for disk and producing redundant files.
    {
        let mut running = state.export_running.lock().await;
        if *running {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "export already running"})),
            )
                .into_response();
        }
        *running = true;
    }

    let mirror_dir = state.mirror_dir.clone();
    let config_dir = state
        .config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("/config"));
    let exports_dir = state.incoming_dir.join("exports");
    let _ = std::fs::create_dir_all(&exports_dir);

    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("snapshot-{}.pkg", timestamp);
    let output = exports_dir.join(&filename);

    // Run the export on a blocking thread — walking a large mirror and
    // hashing every file must not stall the async runtime.
    let result = tokio::task::spawn_blocking(move || {
        let exporter = Exporter::new(mirror_dir).with_config_dir(config_dir);
        exporter.export(&output)
    })
    .await;

    *state.export_running.lock().await = false;

    match result {
        Ok(Ok(r)) => (
            StatusCode::OK,
            Json(serde_json::to_value(ExportSummary {
                filename,
                size: r.total_size,
                crate_count: r.crate_count,
                rustup_count: r.rustup_count,
                dist_count: r.dist_count,
            }).unwrap()),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("export task failed: {}", e)})),
        )
            .into_response(),
    }
}

#[derive(Serialize)]
struct ExportListEntry {
    filename: String,
    size: u64,
    created: Option<String>,
}

async fn get_exports(State(state): State<SharedState>) -> impl IntoResponse {
    let exports_dir = state.incoming_dir.join("exports");
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&exports_dir) {
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".pkg") {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let created = meta
                .modified()
                .ok()
                .and_then(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into());
            entries.push(ExportListEntry {
                filename: name,
                size: meta.len(),
                created,
            });
        }
    }
    entries.sort_by(|a, b| b.filename.cmp(&a.filename));
    Json(entries)
}

async fn download_export(
    State(state): State<SharedState>,
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> Response {
    // Reject any non-direct-child path. The filename comes straight from the
    // URL — no `..`, no slashes, no NULs.
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
        || !filename.ends_with(".pkg")
    {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let path = state.incoming_dir.join("exports").join(&filename);
    match tokio::fs::read(&path).await {
        Ok(data) => {
            let len = data.len();
            let headers = [
                ("content-type", "application/octet-stream".to_string()),
                ("content-length", len.to_string()),
                (
                    "content-disposition",
                    format!("attachment; filename=\"{}\"", filename),
                ),
            ];
            (StatusCode::OK, headers, Body::from(data)).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "snapshot not found").into_response(),
    }
}

fn count_files_in(dir: &Path) -> anyhow::Result<u64> {
    let mut count = 0;
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return format!("{:.1} {}", size, unit);
        }
        size /= 1024.0;
    }
    format!("{:.1} PB", size)
}
