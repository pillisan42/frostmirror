use anyhow::Result;
use axum::Router;
use frostmirror_core::config::FrostmirrorConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::api;
use crate::registry;
use crate::watcher::IncomingWatcher;
use crate::web;

/// Shared application state.
pub struct AppState {
    pub mirror_dir: PathBuf,
    pub incoming_dir: PathBuf,
    pub config_path: PathBuf,
    pub depends_path: PathBuf,
    pub base_url: String,
    pub watcher_active: RwLock<bool>,

    // Live-mirror (proxy) mode. When `proxy_mode` is false the registry
    // returns 404 for missing files exactly like the offline workflow.
    pub proxy_mode: bool,
    pub proxy_index_url: String,
    pub proxy_dl_url: String,
    pub proxy_dist_url: String,
    pub http_client: reqwest::Client,
    /// Serializes manifest.json updates from concurrent proxy fetches.
    pub manifest_lock: Mutex<()>,
    /// Single-flight guard for snapshot exports — true while one is running.
    pub export_running: Mutex<bool>,
}

pub type SharedState = Arc<AppState>;

/// The frostmirror HTTP server.
pub struct Server {
    pub mirror_dir: PathBuf,
    pub incoming_dir: PathBuf,
    pub config_path: PathBuf,
    pub depends_path: PathBuf,
    pub bind_addr: String,
    pub base_url: String,
    pub watch_incoming: bool,
}

impl Server {
    pub fn from_env() -> Self {
        let mirror_dir = std::env::var("FROSTMIRROR_MIRROR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/mirror"));
        let incoming_dir = std::env::var("FROSTMIRROR_INCOMING")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/incoming"));

        Self {
            mirror_dir,
            incoming_dir,
            config_path: PathBuf::from("/config/frostmirror.toml"),
            depends_path: PathBuf::from("/config/depends.toml"),
            bind_addr: std::env::var("FROSTMIRROR_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            base_url: std::env::var("FROSTMIRROR_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            watch_incoming: false,
        }
    }

    pub async fn run(self) -> Result<()> {
        // Ensure directories exist
        std::fs::create_dir_all(&self.mirror_dir)?;
        std::fs::create_dir_all(&self.incoming_dir)?;
        std::fs::create_dir_all(self.incoming_dir.join("done"))?;
        std::fs::create_dir_all(self.incoming_dir.join("failed"))?;
        std::fs::create_dir_all(self.incoming_dir.join("exports"))?;

        // Load proxy + behavior settings from frostmirror.toml so they survive
        // restarts and are editable from the web UI's Configuration page.
        let cfg = FrostmirrorConfig::load(&self.config_path).unwrap_or_default();

        let http_client = reqwest::Client::builder()
            .user_agent("frostmirror-serve/0.1.0")
            .build()
            .expect("failed to build HTTP client");

        let state = Arc::new(AppState {
            mirror_dir: self.mirror_dir.clone(),
            incoming_dir: self.incoming_dir.clone(),
            config_path: self.config_path.clone(),
            depends_path: self.depends_path.clone(),
            base_url: self.base_url.clone(),
            watcher_active: RwLock::new(self.watch_incoming),
            proxy_mode: cfg.proxy_mode,
            proxy_index_url: cfg.proxy_index_url.clone(),
            proxy_dl_url: cfg.proxy_dl_url.clone(),
            proxy_dist_url: cfg.proxy_dist_url.clone(),
            http_client,
            manifest_lock: Mutex::new(()),
            export_running: Mutex::new(false),
        });

        // Start incoming watcher if requested
        if self.watch_incoming {
            let watcher = IncomingWatcher::new(
                self.incoming_dir.clone(),
                self.mirror_dir.clone(),
            )
            .with_config_dir(
                self.config_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("/config"))
                    .to_path_buf(),
            );
            tokio::spawn(async move {
                if let Err(e) = watcher.watch().await {
                    tracing::error!("incoming watcher error: {}", e);
                }
            });
        }

        let app = Router::new()
            .merge(web::routes())
            .merge(api::routes(state.clone()))
            .merge(registry::routes(state.clone()));

        let listener = tokio::net::TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("frostmirror serving on {}", self.bind_addr);
        tracing::info!("base URL: {}", self.base_url);
        tracing::info!("mirror: {}", self.mirror_dir.display());
        tracing::info!("incoming watcher: {}", if self.watch_incoming { "active" } else { "disabled" });
        tracing::info!(
            "proxy mode: {}",
            if state.proxy_mode { "active" } else { "disabled" }
        );

        axum::serve(listener, app).await?;
        Ok(())
    }
}
