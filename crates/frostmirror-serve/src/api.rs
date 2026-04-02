use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use frostmirror_core::config::FrostmirrorConfig;
use frostmirror_core::depends::DependsToml;
use frostmirror_import::{GarbageCollector, Importer};
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
    let importer = Importer::new(state.mirror_dir.clone());
    let status = importer.status().unwrap_or(frostmirror_import::importer::MirrorStatus {
        crate_count: 0,
        total_size: 0,
        last_import: None,
    });

    let done_count = count_files_in(&state.incoming_dir.join("done")).unwrap_or(0);
    let failed_count = count_files_in(&state.incoming_dir.join("failed")).unwrap_or(0);
    let watcher_active = *state.watcher_active.read().await;

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
    let mut packages = Vec::new();

    // List done packages
    if let Ok(entries) = std::fs::read_dir(state.incoming_dir.join("done")) {
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
    if let Ok(entries) = std::fs::read_dir(state.incoming_dir.join("failed")) {
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
    dependencies: std::collections::BTreeMap<String, String>,
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
    let gc = GarbageCollector::new(state.mirror_dir.clone());
    match gc.run() {
        Ok(result) => (StatusCode::OK, Json(serde_json::to_value(result).unwrap())),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_cargo_config(State(state): State<SharedState>) -> impl IntoResponse {
    let config = format!(
        r#"[source.frostmirror]
registry = "{base_url}/index"

[source.crates-io]
replace-with = "frostmirror"
"#,
        base_url = state.base_url
    );

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
