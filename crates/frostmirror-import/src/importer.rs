use anyhow::{Context, Result};
use frostmirror_core::bundle::{BundleReader, SectionKind};
use frostmirror_core::manifest::Manifest;
use std::path::{Path, PathBuf};

/// Handles importing `.pkg` bundles into the local mirror store.
pub struct Importer {
    mirror_dir: PathBuf,
    config_dir: Option<PathBuf>,
}

impl Importer {
    pub fn new(mirror_dir: PathBuf) -> Self {
        Self {
            mirror_dir,
            config_dir: None,
        }
    }

    /// Route Config sections in imported bundles to this directory. When unset
    /// (default), Config sections from a snapshot bundle are skipped — this
    /// preserves existing offline behavior where fetch-produced bundles never
    /// carry config files.
    pub fn with_config_dir(mut self, config_dir: PathBuf) -> Self {
        self.config_dir = Some(config_dir);
        self
    }

    pub fn mirror_dir(&self) -> &Path {
        &self.mirror_dir
    }

    /// Import a `.pkg` bundle into the mirror.
    ///
    /// 1. Decompress and parse the bundle
    /// 2. Verify integrity (manifest hash + per-crate SHA-256)
    /// 3. Write all sections to a temp directory
    /// 4. Atomically rename into the mirror
    pub fn import(&self, pkg_path: &Path) -> Result<ImportResult> {
        tracing::info!("importing bundle: {}", pkg_path.display());

        // Read and decompress
        let bundle = BundleReader::read_file(pkg_path)
            .with_context(|| format!("failed to read bundle {}", pkg_path.display()))?;

        // Verify integrity
        BundleReader::verify(&bundle).context("bundle verification failed")?;

        let manifest = &bundle.manifest;
        let crate_count = manifest.crates.len();
        let rustup_count = manifest.rustup.len();

        // Create a temp directory inside the mirror for atomic writes
        let temp_dir = tempfile::tempdir_in(&self.mirror_dir)
            .context("failed to create temp dir in mirror")?;

        // Write all sections to temp
        for section in &bundle.sections {
            match section.kind {
                SectionKind::Manifest => {
                    // Store manifest alongside the mirror
                    let dest = temp_dir.path().join("manifest.json");
                    std::fs::write(&dest, &section.data)?;
                }
                SectionKind::Crate => {
                    // crates/<name>/<version>/download
                    let dest = temp_dir.path().join(&section.path);
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&dest, &section.data)?;
                }
                SectionKind::Index => {
                    // index/<path>
                    let dest = temp_dir.path().join(&section.path);
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&dest, &section.data)?;
                }
                SectionKind::Rustup => {
                    // rustup/dist/<target>/rustup-init
                    let dest = temp_dir.path().join(&section.path);
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&dest, &section.data)?;
                    // Make executable on Unix
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
                    }
                }
                SectionKind::Dist => {
                    // dist/<path> (channel manifests + component archives)
                    let dest = temp_dir.path().join(&section.path);
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&dest, &section.data)?;
                }
                SectionKind::Config => {
                    // Config sections live outside the mirror tree, so we
                    // bypass the temp/atomic-merge dance and write directly to
                    // the configured config dir. If no config_dir is set, skip
                    // — this keeps the legacy behavior for fetch-produced
                    // bundles that don't carry configuration.
                    let Some(config_dir) = self.config_dir.as_ref() else {
                        continue;
                    };
                    std::fs::create_dir_all(config_dir)?;

                    // Two forms are supported:
                    //   - "config.toml"          → legacy single-file form
                    //   - "config/<filename>"    → snapshot multi-file form
                    let filename = if section.path == "config.toml" {
                        "frostmirror.toml"
                    } else if let Some(rest) = section.path.strip_prefix("config/") {
                        if rest.is_empty() || rest.contains("..") || rest.contains('/') {
                            anyhow::bail!("invalid config section path: {}", section.path);
                        }
                        rest
                    } else {
                        anyhow::bail!("unrecognized config section path: {}", section.path);
                    };
                    let dest = config_dir.join(filename);
                    std::fs::write(&dest, &section.data)?;
                }
            }
        }

        // Atomic merge: move files from temp into the real mirror directory.
        // We walk the temp dir and rename each file into its final location.
        self.merge_into_mirror(temp_dir.path())?;

        // Store a cumulative mirror manifest. The bundle's own manifest only
        // describes what the bundle ships (potentially a delta); the mirror
        // manifest must describe everything actually on disk so that GC,
        // status, and append-on-proxy stay correct across import chains.
        let manifest_path = self.mirror_dir.join("manifest.json");
        let merged = merge_with_existing(&manifest_path, manifest)?;
        std::fs::write(&manifest_path, merged.to_json()?)?;

        // Write the index config.json for sparse protocol
        self.write_index_config()?;

        tracing::info!(
            "imported {} crates, {} rustup artifacts",
            crate_count,
            rustup_count
        );

        Ok(ImportResult {
            crate_count,
            rustup_count,
            bundle_type: manifest.bundle_type,
        })
    }

    /// Recursively move all files from src into the mirror directory,
    /// creating directories as needed. Uses rename for atomicity when possible.
    fn merge_into_mirror(&self, src: &Path) -> Result<()> {
        for entry in walkdir::WalkDir::new(src) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let relative = entry.path().strip_prefix(src)?;
                let dest = self.mirror_dir.join(relative);

                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // Try rename first (atomic on same filesystem)
                if std::fs::rename(entry.path(), &dest).is_err() {
                    // Fallback to copy + remove
                    std::fs::copy(entry.path(), &dest)?;
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    /// Write the sparse index config.json that cargo requires.
    fn write_index_config(&self) -> Result<()> {
        let base_url = std::env::var("FROSTMIRROR_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string());

        let config = serde_json::json!({
            "dl": format!("{}/crates/{{crate}}/{{version}}/download", base_url),
            "api": base_url
        });

        let index_dir = self.mirror_dir.join("index");
        std::fs::create_dir_all(&index_dir)?;
        let config_path = index_dir.join("config.json");
        std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;

        Ok(())
    }

    /// Get the current mirror status.
    pub fn status(&self) -> Result<MirrorStatus> {
        let crates_dir = self.mirror_dir.join("crates");
        let mut crate_count = 0u64;
        let mut total_size = 0u64;

        if crates_dir.exists() {
            for entry in walkdir::WalkDir::new(&crates_dir) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    crate_count += 1;
                    total_size += entry.metadata()?.len();
                }
            }
        }

        let manifest_path = self.mirror_dir.join("manifest.json");
        let last_import = if manifest_path.exists() {
            let content = std::fs::read_to_string(&manifest_path)?;
            let manifest: frostmirror_core::manifest::Manifest = serde_json::from_str(&content)?;
            Some(manifest.created)
        } else {
            None
        };

        Ok(MirrorStatus {
            crate_count,
            total_size,
            last_import,
        })
    }
}

/// Merge the bundle's manifest into whatever manifest already lives at
/// `manifest_path`. The bundle's entries layer on top — newer SHAs/sizes win
/// on key collision — and the bundle's metadata (`created`, `bundle_type`,
/// `parent`, `targets`, `toolchain`) becomes the merged manifest's metadata
/// so consumers see the latest import as the active one. `rustup` and `dist`
/// vectors dedupe by their stable identifier so repeated imports don't grow
/// duplicate entries.
fn merge_with_existing(manifest_path: &Path, bundle: &Manifest) -> Result<Manifest> {
    let mut merged = if manifest_path.exists() {
        let content = std::fs::read_to_string(manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        Manifest::from_json(&content).unwrap_or_else(|_| bundle.clone())
    } else {
        bundle.clone()
    };

    merged.created = bundle.created.clone();
    merged.bundle_type = bundle.bundle_type;
    merged.parent = bundle.parent.clone();
    merged.targets = bundle.targets.clone();
    merged.toolchain = bundle.toolchain.clone();

    for (key, entry) in &bundle.crates {
        merged.crates.insert(key.clone(), entry.clone());
    }

    for entry in &bundle.rustup {
        merged
            .rustup
            .retain(|e| !(e.target == entry.target && e.filename == entry.filename));
        merged.rustup.push(entry.clone());
    }

    for entry in &bundle.dist {
        merged.dist.retain(|e| e.path != entry.path);
        merged.dist.push(entry.clone());
    }

    merged.seal();
    Ok(merged)
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub crate_count: usize,
    pub rustup_count: usize,
    pub bundle_type: frostmirror_core::manifest::BundleType,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MirrorStatus {
    pub crate_count: u64,
    pub total_size: u64,
    pub last_import: Option<String>,
}
