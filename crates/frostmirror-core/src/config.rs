use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime configuration for frostmirror, stored in `frostmirror.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrostmirrorConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_toolchain")]
    pub toolchain: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default = "default_true")]
    pub watch_incoming: bool,
    #[serde(default = "default_true")]
    pub verify_checksums: bool,
    #[serde(default = "default_true")]
    pub keep_failed_packages: bool,
    #[serde(default)]
    pub prune_on_import: bool,
    /// When true, the registry server fetches missing files from upstream on
    /// demand and caches them locally. Off by default — the offline workflow
    /// is unchanged. Can be overridden via `FROSTMIRROR_PROXY_MODE=true`.
    #[serde(default = "default_proxy_mode")]
    pub proxy_mode: bool,
    #[serde(default = "default_proxy_index_url")]
    pub proxy_index_url: String,
    #[serde(default = "default_proxy_dl_url")]
    pub proxy_dl_url: String,
    #[serde(default = "default_proxy_dist_url")]
    pub proxy_dist_url: String,
}

fn default_base_url() -> String {
    std::env::var("FROSTMIRROR_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

fn default_bind() -> String {
    std::env::var("FROSTMIRROR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string())
}

fn default_toolchain() -> String {
    std::env::var("FROSTMIRROR_TOOLCHAIN").unwrap_or_else(|_| "stable".to_string())
}

fn default_true() -> bool {
    true
}

fn default_proxy_mode() -> bool {
    matches!(
        std::env::var("FROSTMIRROR_PROXY_MODE")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn default_proxy_index_url() -> String {
    "https://index.crates.io".to_string()
}

fn default_proxy_dl_url() -> String {
    "https://static.crates.io/crates".to_string()
}

fn default_proxy_dist_url() -> String {
    "https://static.rust-lang.org".to_string()
}

impl Default for FrostmirrorConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            bind: default_bind(),
            toolchain: default_toolchain(),
            targets: vec!["x86_64-unknown-linux-gnu".to_string()],
            watch_incoming: true,
            verify_checksums: true,
            keep_failed_packages: true,
            prune_on_import: false,
            proxy_mode: default_proxy_mode(),
            proxy_index_url: default_proxy_index_url(),
            proxy_dl_url: default_proxy_dl_url(),
            proxy_dist_url: default_proxy_dist_url(),
        }
    }
}

impl FrostmirrorConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read config at {}", path.display()))?;
            toml::from_str(&content).context("failed to parse frostmirror.toml")
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("failed to serialize config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Resolve the frostmirror home directory.
    pub fn home_dir() -> PathBuf {
        if let Ok(home) = std::env::var("FROSTMIRROR_HOME") {
            PathBuf::from(home)
        } else {
            dirs_home().join(".frostmirror")
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
