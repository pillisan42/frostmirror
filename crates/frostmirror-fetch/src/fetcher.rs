use anyhow::{bail, Context, Result};
use frostmirror_core::bundle::{self, BundleBuilder};
use frostmirror_core::depends::DependsToml;
use frostmirror_core::manifest::{BundleType, Manifest};
use frostmirror_core::resolver::{ResolvedCrate, ResolvedGraph};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

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

        // Resolve dependencies using cargo's own resolver
        let graph = self.resolve_with_cargo(&depends).await?;
        tracing::info!("resolved {} total crates via cargo", graph.crate_count());

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
        }

        // Fetch index entries for ALL resolved crates (not just the delta).
        // Since we use cargo's own resolver, this set is exactly what cargo
        // will need on the air-gap side — no iterative expansion required.
        let all_crate_names: BTreeSet<String> = graph
            .crate_set()
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        tracing::info!(
            "fetching index entries for {} resolved crate(s)",
            all_crate_names.len()
        );

        let index_fetches: Vec<_> = all_crate_names
            .iter()
            .map(|name| {
                let idx = &sparse_index;
                let n = name.clone();
                async move {
                    let result = idx.fetch_raw_index(&n).await;
                    (n, result)
                }
            })
            .collect();

        for (name, result) in futures::future::join_all(index_fetches).await {
            match result {
                Ok(raw_index) => {
                    builder.add_index_entry(&name, raw_index.as_bytes().to_vec());
                }
                Err(e) => {
                    tracing::warn!("failed to fetch index for {}: {}", name, e);
                }
            }
        }

        // Download rustup artifacts.
        // Full: all targets. Delta: only new targets.
        let rustup_targets: Vec<String> = if bundle_type == BundleType::Full {
            depends.platforms.targets.clone()
        } else {
            let prev_targets: HashSet<String> = previous_manifest
                .as_ref()
                .map(|m| m.rustup.iter().map(|r| r.target.clone()).collect())
                .unwrap_or_default();
            depends
                .platforms
                .targets
                .iter()
                .filter(|t| !prev_targets.contains(*t))
                .cloned()
                .collect()
        };

        if !rustup_targets.is_empty() {
            tracing::info!(
                "downloading rustup-init for {} target(s)",
                rustup_targets.len()
            );
        }
        for target in &rustup_targets {
            match rustup::download_rustup_init(
                &self.client,
                self.config.dist_url.as_deref(),
                target,
            )
            .await
            {
                Ok(data) => {
                    let sha = bundle::sha256_hex(&data);
                    let filename = rustup::rustup_init_filename(target).to_string();
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

        // Generate cargo config.toml for clients
        let base_url = std::env::var("FROSTMIRROR_BASE_URL")
            .unwrap_or_else(|_| "http://frostmirror.internal:8080".to_string());
        let cargo_config = format!(
            r#"[source.frostmirror]
registry = "sparse+{base_url}/index/"

[source.crates-io]
replace-with = "frostmirror"
"#
        );
        builder.add_config(&cargo_config);

        // Seal and write
        manifest.seal();
        builder.add_manifest(&manifest)?;

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

    /// Resolve dependencies using a two-pass strategy:
    ///
    /// **Pass 1 — Combined:** Put all deps in one Cargo.toml and run
    /// `cargo generate-lockfile`. This produces the unified versions that
    /// cargo would pick when all deps coexist in a single project — which
    /// is how most users will consume the mirror. If this fails (e.g.
    /// conflicting version requirements), we skip to pass 2.
    ///
    /// **Pass 2 — Per-dependency:** Resolve each dependency independently
    /// to catch versions that only appear when a crate is resolved alone
    /// (conflicting requirements across unrelated projects).
    ///
    /// The final set is the union of both passes, deduplicated by
    /// (name, version). This guarantees the mirror has every version any
    /// downstream project might need, whether it uses one dep or all of them.
    async fn resolve_with_cargo(&self, depends: &DependsToml) -> Result<ResolvedGraph> {
        let mut all_crates = BTreeSet::new();
        let mut all_roots = Vec::new();
        // Flatten multi-version entries into individual (name, spec) pairs
        let flat_deps = depends.flat_deps();
        let dep_count = flat_deps.len();

        // Pass 1: combined resolution (catches unified transitive versions).
        // For multi-version entries we try each version in its own combined
        // project — we can't put conflicting versions of the same crate in
        // one Cargo.toml, so we group by unique name and run combined
        // resolution once per group of non-conflicting deps.
        tracing::info!(
            "pass 1: resolving all {} dependencies combined",
            dep_count
        );

        match self.resolve_combined(depends).await {
            Ok(combined) => {
                tracing::info!(
                    "combined resolution: {} crates",
                    combined.crate_count()
                );
                all_roots = combined.roots.clone();
                all_crates.extend(combined.crates);
            }
            Err(e) => {
                tracing::warn!(
                    "combined resolution failed (likely version conflicts), \
                     falling back to per-dependency only: {}",
                    e
                );
            }
        }

        // Pass 2: per-dependency resolution (catches conflict-specific versions
        // and multi-version entries)
        tracing::info!(
            "pass 2: resolving {} dependency specs individually",
            dep_count
        );

        for (i, (name, spec)) in flat_deps.iter().enumerate() {
            tracing::info!(
                "[{}/{}] resolving {} = {}",
                i + 1,
                dep_count,
                name,
                spec.to_cargo_toml_value()
            );

            match self.resolve_single_dep(name, spec).await {
                Ok(resolved) => {
                    if let Some(root) = resolved.crates.iter().find(|c| c.name == *name) {
                        if !all_roots.iter().any(|r| r.name == *name) {
                            all_roots.push(root.clone());
                        }
                    }
                    all_crates.extend(resolved.crates);
                }
                Err(e) => {
                    tracing::warn!("failed to resolve {} = {}: {}", name, spec.to_cargo_toml_value(), e);
                }
            }
        }

        let graph = ResolvedGraph {
            crates: all_crates,
            roots: all_roots,
        };

        tracing::info!(
            "total: {} unique crates across {} dependencies",
            graph.crate_count(),
            dep_count
        );

        Ok(graph)
    }

    /// Resolve all dependencies together in a single Cargo project.
    async fn resolve_combined(&self, depends: &DependsToml) -> Result<ResolvedGraph> {
        let tmp_dir = tempfile::tempdir().context("failed to create temp dir")?;
        let project_dir = tmp_dir.path();

        let mut cargo_toml = String::new();
        cargo_toml.push_str("[package]\n");
        cargo_toml.push_str("name = \"frostmirror-resolve\"\n");
        cargo_toml.push_str("version = \"0.0.0\"\n");
        cargo_toml.push_str("edition = \"2021\"\n");
        cargo_toml.push_str("publish = false\n\n");
        cargo_toml.push_str("[dependencies]\n");
        for (name, entry) in &depends.dependencies {
            // For multi-version entries, use the first spec in the combined project.
            // The per-dependency pass (pass 2) handles each version individually.
            let spec = entry.specs()[0];
            cargo_toml.push_str(&format!("{} = {}\n", name, spec.to_cargo_toml_value()));
        }

        std::fs::write(project_dir.join("Cargo.toml"), &cargo_toml)?;
        std::fs::create_dir_all(project_dir.join("src"))?;
        std::fs::write(project_dir.join("src/lib.rs"), "")?;

        let project_path = project_dir.to_path_buf();
        let lockfile_content = tokio::task::spawn_blocking(move || -> Result<String> {
            let output = std::process::Command::new("cargo")
                .arg("generate-lockfile")
                .current_dir(&project_path)
                .env("CARGO_TERM_COLOR", "never")
                .output()
                .context("failed to run cargo generate-lockfile")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("cargo generate-lockfile failed:\n{}", stderr);
            }

            std::fs::read_to_string(project_path.join("Cargo.lock"))
                .context("failed to read Cargo.lock")
        })
        .await??;

        parse_cargo_lock(&lockfile_content)
    }

    /// Resolve a single dependency by creating a temporary Cargo project and
    /// running `cargo generate-lockfile`.
    async fn resolve_single_dep(
        &self,
        name: &str,
        spec: &frostmirror_core::depends::DepSpec,
    ) -> Result<ResolvedGraph> {
        let tmp_dir = tempfile::tempdir().context("failed to create temp dir")?;
        let project_dir = tmp_dir.path();

        let cargo_toml = format!(
            r#"[package]
name = "frostmirror-resolve"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
{name} = {value}
"#,
            value = spec.to_cargo_toml_value()
        );

        std::fs::write(project_dir.join("Cargo.toml"), &cargo_toml)?;
        std::fs::create_dir_all(project_dir.join("src"))?;
        std::fs::write(project_dir.join("src/lib.rs"), "")?;

        let project_path = project_dir.to_path_buf();
        let lockfile_content = tokio::task::spawn_blocking(move || -> Result<String> {
            let output = std::process::Command::new("cargo")
                .arg("generate-lockfile")
                .current_dir(&project_path)
                .env("CARGO_TERM_COLOR", "never")
                .output()
                .context("failed to run cargo generate-lockfile")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("cargo generate-lockfile failed:\n{}", stderr);
            }

            std::fs::read_to_string(project_path.join("Cargo.lock"))
                .context("failed to read Cargo.lock")
        })
        .await??;

        parse_cargo_lock(&lockfile_content)
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

/// Parse a Cargo.lock file and extract the resolved dependency graph.
///
/// Cargo.lock format (v3/v4):
/// ```toml
/// [[package]]
/// name = "serde"
/// version = "1.0.210"
/// source = "registry+https://github.com/rust-lang/crates.io-index"
/// checksum = "..."
/// ```
fn parse_cargo_lock(content: &str) -> Result<ResolvedGraph> {
    #[derive(serde::Deserialize)]
    struct LockFile {
        #[serde(default)]
        package: Vec<LockPackage>,
    }

    #[derive(serde::Deserialize)]
    struct LockPackage {
        name: String,
        version: String,
        #[serde(default)]
        source: Option<String>,
    }

    let lockfile: LockFile =
        toml::from_str(content).context("failed to parse Cargo.lock")?;

    let mut crates = BTreeSet::new();
    let mut roots = Vec::new();

    for pkg in &lockfile.package {
        // Skip the synthetic resolve project itself
        if pkg.name == "frostmirror-resolve" {
            continue;
        }

        // Only include crates from a registry (skip path/git deps)
        match &pkg.source {
            Some(src) if src.starts_with("registry+") => {}
            None => continue,    // path dependency (our synthetic project)
            Some(_) => continue, // git or other source
        }

        crates.insert(ResolvedCrate {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
        });
    }

    // The first-level deps of frostmirror-resolve are the roots
    // (we don't strictly need this, but it's nice for the manifest)
    for pkg in &lockfile.package {
        if pkg.name == "frostmirror-resolve" {
            continue;
        }
        if pkg
            .source
            .as_ref()
            .map(|s| s.starts_with("registry+"))
            .unwrap_or(false)
        {
            roots.push(ResolvedCrate {
                name: pkg.name.clone(),
                version: pkg.version.clone(),
            });
        }
    }

    Ok(ResolvedGraph { crates, roots })
}
