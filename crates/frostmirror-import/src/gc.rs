use anyhow::{Context, Result};
use frostmirror_core::manifest::Manifest;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Garbage collector for the mirror store.
/// Removes crates that are no longer referenced by the current manifest.
pub struct GarbageCollector {
    mirror_dir: PathBuf,
}

impl GarbageCollector {
    pub fn new(mirror_dir: PathBuf) -> Self {
        Self { mirror_dir }
    }

    /// Run garbage collection.
    ///
    /// Reads the current manifest to determine which crates are still needed,
    /// then removes any crate files not in that set.
    pub fn run(&self) -> Result<GcResult> {
        let manifest_path = self.mirror_dir.join("manifest.json");
        if !manifest_path.exists() {
            anyhow::bail!("no manifest found in mirror — nothing to garbage collect");
        }

        let content = std::fs::read_to_string(&manifest_path)
            .context("failed to read manifest")?;
        let manifest: Manifest = serde_json::from_str(&content)
            .context("failed to parse manifest")?;

        let needed: BTreeSet<(String, String)> = manifest.crate_set();

        let crates_dir = self.mirror_dir.join("crates");
        if !crates_dir.exists() {
            return Ok(GcResult {
                removed: 0,
                freed_bytes: 0,
            });
        }

        let mut removed = 0u64;
        let mut freed_bytes = 0u64;

        // Walk crates/<name>/<version>/download
        let name_dirs: Vec<_> = std::fs::read_dir(&crates_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        for name_dir in name_dirs {
            let crate_name = name_dir.file_name().to_string_lossy().to_string();

            let version_dirs: Vec<_> = std::fs::read_dir(name_dir.path())?
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();

            for version_dir in version_dirs {
                let version = version_dir.file_name().to_string_lossy().to_string();

                if !needed.contains(&(crate_name.clone(), version.clone())) {
                    // Remove this version directory
                    let size = dir_size(&version_dir.path())?;
                    std::fs::remove_dir_all(version_dir.path())?;
                    removed += 1;
                    freed_bytes += size;
                    tracing::info!("gc: removed {}-{} ({} bytes)", crate_name, version, size);
                }
            }

            // Remove empty name directories
            if is_dir_empty(&name_dir.path())? {
                std::fs::remove_dir(name_dir.path())?;
            }
        }

        Ok(GcResult {
            removed,
            freed_bytes,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GcResult {
    pub removed: u64,
    pub freed_bytes: u64,
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn is_dir_empty(path: &Path) -> Result<bool> {
    Ok(std::fs::read_dir(path)?.next().is_none())
}
