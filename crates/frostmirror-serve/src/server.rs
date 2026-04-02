use anyhow::Result;
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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

        let state = Arc::new(AppState {
            mirror_dir: self.mirror_dir.clone(),
            incoming_dir: self.incoming_dir.clone(),
            config_path: self.config_path.clone(),
            depends_path: self.depends_path.clone(),
            base_url: self.base_url.clone(),
            watcher_active: RwLock::new(self.watch_incoming),
        });

        // Start incoming watcher if requested
        if self.watch_incoming {
            let watcher = IncomingWatcher::new(
                self.incoming_dir.clone(),
                self.mirror_dir.clone(),
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

        axum::serve(listener, app).await?;
        Ok(())
    }
}
