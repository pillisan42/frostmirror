use anyhow::{Context, Result};
use frostmirror_core::bundle::{self, BundleBuilder};
use frostmirror_core::depends::DependsToml;
use frostmirror_core::manifest::{BundleType, Manifest};
use frostmirror_core::resolver::{IndexEntry, ResolvedGraph, Resolver};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::index::{self, SparseIndex};
use crate::rustup;

/// Configuration for a fetch run.
pub struct FetchConfig {
    pub depends_path: PathBuf,
    pub output_dir: PathBuf,
    pub incremental: bool,
    pub history_dir: PathBuf,
    /// Override the crates.io index URL (for testing with mock registries).
    pub registry_url: Option<String>,
    /// Override the crate download base URL.
    pub dl_url: Option<String>,
    /// Override the rustup distribution URL.
    pub dist_url: Option<String>,
}

impl FetchConfig {
    pub fn from_env(depends_path: PathBuf, output_dir: PathBuf, incremental: bool) -> Self {
        let home = frostmirror_core::FrostmirrorConfig::home_dir();
        Self {
            depends_path,
            output_dir,
            incremental,
            history_dir: std::env::var("FROSTMIRROR_HISTORY")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join("history")),
            registry_url: std::env::var("FROSTMIRROR_REGISTRY_URL").ok(),
            dl_url: std::env::var("FROSTMIRROR_DL_URL").ok(),
            dist_url: std::env::var("FROSTMIRROR_DIST_URL").ok(),
        }
    }
}

/// The main fetch engine.
pub struct Fetcher {
    config: FetchConfig,
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new(config: FetchConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("frostmirror/0.1.0")
            .build()
            .expect("failed to build HTTP client");
        Self { config, client }
    }

    /// Execute the fetch: resolve deps, download crates, produce a `.pkg` bundle.
    pub async fn run(&self) -> Result<PathBuf> {
        // Load depends.toml
        let depends = DependsToml::load(&self.config.depends_path)?;
        tracing::info!(
            "loaded {} direct dependencies for {} targets",
            depends.dependencies.len(),
            depends.platforms.targets.len()
        );

        // Load previous manifest if incremental
        let previous_manifest = if self.config.incremental {
            self.load_latest_manifest()?
        } else {
            None
        };

        if self.config.incremental && previous_manifest.is_none() {
            tracing::warn!("no previous manifest found, falling back to full fetch");
        }

        let bundle_type = if previous_manifest.is_some() {
            BundleType::Delta
        } else {
            BundleType::Full
        };

        let parent_filename = previous_manifest
            .as_ref()
            .and_then(|_| self.latest_manifest_pkg_name());

        // Resolve dependencies
        let graph = self.resolve_dependencies(&depends).await?;
        tracing::info!("resolved {} total crates", graph.crate_count());

        // Determine which crates to download
        let to_download = if let Some(ref prev) = previous_manifest {
            let prev_set = prev.crate_set();
            let curr_set = graph.crate_set();
            let delta: BTreeSet<_> = curr_set.difference(&prev_set).cloned().collect();
            tracing::info!(
                "incremental: {} new crates (delta from {})",
                delta.len(),
                prev_set.len()
            );
            delta
        } else {
            graph.crate_set()
        };

        // Create manifest
        let mut manifest = Manifest::new(
            bundle_type,
            parent_filename,
            depends.platforms.targets.clone(),
            depends.platforms.toolchain.clone(),
        );

        // Build the bundle
        let mut builder = BundleBuilder::new();

        // Download and add crates
        let dl_base = self
            .config
            .dl_url
            .as_deref()
            .unwrap_or("https://static.crates.io/crates");

        let sparse_index = if let Some(ref url) = self.config.registry_url {
            SparseIndex::new(url)
        } else {
            SparseIndex::crates_io()
        };

        for (name, version) in &to_download {
            // Download .crate file
            let crate_data =
                index::download_crate(&self.client, dl_base, name, version).await?;
            let sha = bundle::sha256_hex(&crate_data);
            manifest.add_crate(
                name.clone(),
                version.clone(),
                sha,
                crate_data.len() as u64,
            );
            builder.add_crate_file(name, version, crate_data);

            // Fetch and add index entry
            match sparse_index.fetch_raw_index(name).await {
                Ok(raw_index) => {
                    builder.add_index_entry(name, raw_index.into_bytes());
                }
                Err(e) => {
                    tracing::warn!("failed to fetch index for {}: {}", name, e);
                }
            }
        }

        // Download rustup artifacts (only for full fetches)
        if bundle_type == BundleType::Full {
            for target in &depends.platforms.targets {
                match rustup::download_rustup_init(
                    &self.client,
                    self.config.dist_url.as_deref(),
                    target,
                )
                .await
                {
                    Ok(data) => {
                        let sha = bundle::sha256_hex(&data);
                        let filename = "rustup-init".to_string();
                        manifest.add_rustup(
                            target.clone(),
                            filename.clone(),
                            sha,
                            data.len() as u64,
                        );
                        builder.add_rustup_file(target, &filename, data);
                    }
                    Err(e) => {
                        tracing::warn!("failed to download rustup-init for {}: {}", target, e);
                    }
                }
            }
        }

        // Generate cargo config.toml for clients
        let base_url = std::env::var("FROSTMIRROR_BASE_URL")
            .unwrap_or_else(|_| "http://frostmirror.internal:8080".to_string());
        let cargo_config = format!(
            r#"[source.frostmirror]
registry = "{base_url}/index"

[source.crates-io]
replace-with = "frostmirror"
"#
        );
        builder.add_config(&cargo_config);

        // Seal and write
        manifest.seal();
        builder.add_manifest(&manifest)?;

        // Write bundle
        std::fs::create_dir_all(&self.config.output_dir)?;
        let filename = bundle::pkg_filename();
        let output_path = self.config.output_dir.join(&filename);
        builder.write_to_file(&output_path)?;

        tracing::info!("wrote bundle to {}", output_path.display());

        // Save manifest to history
        std::fs::create_dir_all(&self.config.history_dir)?;
        let history_name = filename.replace("-crates.pkg", "-manifest.json");
        let history_path = self.config.history_dir.join(history_name);
        std::fs::write(&history_path, manifest.to_json()?)?;

        Ok(output_path)
    }

    async fn resolve_dependencies(&self, depends: &DependsToml) -> Result<ResolvedGraph> {
        let sparse_index = if let Some(ref url) = self.config.registry_url {
            SparseIndex::new(url)
        } else {
            SparseIndex::crates_io()
        };

        // Build a cached index lookup — fetch once per crate
        let cache: Arc<Mutex<std::collections::HashMap<String, Vec<IndexEntry>>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));

        // Pre-fetch all direct dependency indexes
        for name in depends.dependencies.keys() {
            let entries = sparse_index.fetch_crate_index(name).await?;
            cache.lock().unwrap().insert(name.clone(), entries);
        }

        let cache_clone = cache.clone();
        let index_base = sparse_index.base_url().to_string();

        let mut resolver = Resolver::new(move |name: &str| {
            let cache = cache_clone.lock().unwrap();
            if let Some(entries) = cache.get(name) {
                return Ok(entries.clone());
            }
            // Synchronous fetch for transitive deps — we use a blocking client
            drop(cache);
            let path = frostmirror_core::bundle::crate_index_path(name);
            let url = format!("{}/{}", index_base, path);
            let resp = reqwest::blocking::get(&url)
                .with_context(|| format!("failed to fetch index for {}", name))?;
            if !resp.status().is_success() {
                anyhow::bail!("index for {} returned HTTP {}", name, resp.status());
            }
            let body = resp.text()?;
            let entries = index::parse_index_entries(&body, name)?;
            cache_clone.lock().unwrap().insert(name.to_string(), entries.clone());
            Ok(entries)
        });

        resolver.resolve(&depends.dependencies)
    }

    fn load_latest_manifest(&self) -> Result<Option<Manifest>> {
        if !self.config.history_dir.exists() {
            return Ok(None);
        }

        let mut manifests: Vec<_> = std::fs::read_dir(&self.config.history_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        manifests.sort_by_key(|e| e.file_name());

        if let Some(latest) = manifests.last() {
            let content = std::fs::read_to_string(latest.path())?;
            let manifest = Manifest::from_json(&content)?;
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }

    fn latest_manifest_pkg_name(&self) -> Option<String> {
        if !self.config.history_dir.exists() {
            return None;
        }

        let mut files: Vec<_> = std::fs::read_dir(&self.config.history_dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        files.sort_by_key(|e| e.file_name());
        files.last().map(|e| {
            e.file_name()
                .to_string_lossy()
                .replace("-manifest.json", "-crates.pkg")
        })
    }
}
