use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Manifest embedded in each `.pkg` bundle.
/// Contains the resolved dependency graph, content hashes, and parent chain reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Timestamp when this bundle was created (RFC 3339).
    pub created: String,
    /// Filename of the parent `.pkg` bundle, if this is an incremental delta.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Whether this is a full or delta bundle.
    pub bundle_type: BundleType,
    /// Platforms this bundle was built for.
    pub targets: Vec<String>,
    /// Toolchain channel.
    pub toolchain: String,
    /// All crates included in this bundle, keyed by `name-version`.
    pub crates: BTreeMap<String, CrateEntry>,
    /// Rustup artifacts included.
    #[serde(default)]
    pub rustup: Vec<RustupEntry>,
    /// Toolchain distribution files (channel manifests + component archives).
    #[serde(default)]
    pub dist: Vec<DistEntry>,
    /// SHA-256 of the entire manifest (computed over all other fields).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleType {
    Full,
    Delta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateEntry {
    pub name: String,
    pub version: String,
    /// SHA-256 of the `.crate` file.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustupEntry {
    pub target: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

/// A toolchain distribution file (channel manifest or component archive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistEntry {
    /// Relative path under `dist/`, e.g. `channel-rust-stable.toml`
    /// or `2024-01-09/rustc-1.75.0-x86_64-unknown-linux-gnu.tar.xz`.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

impl Manifest {
    pub fn new(
        bundle_type: BundleType,
        parent: Option<String>,
        targets: Vec<String>,
        toolchain: String,
    ) -> Self {
        Self {
            created: chrono::Utc::now().to_rfc3339(),
            parent,
            bundle_type,
            targets,
            toolchain,
            crates: BTreeMap::new(),
            rustup: Vec::new(),
            dist: Vec::new(),
            manifest_hash: None,
        }
    }

    pub fn add_crate(&mut self, name: String, version: String, sha256: String, size: u64) {
        let key = format!("{}-{}", name, version);
        self.crates.insert(
            key,
            CrateEntry {
                name,
                version,
                sha256,
                size,
            },
        );
    }

    pub fn add_rustup(&mut self, target: String, filename: String, sha256: String, size: u64) {
        self.rustup.push(RustupEntry {
            target,
            filename,
            sha256,
            size,
        });
    }

    pub fn add_dist(&mut self, path: String, sha256: String, size: u64) {
        self.dist.push(DistEntry { path, sha256, size });
    }

    /// Get all (name, version) pairs in this manifest.
    pub fn crate_set(&self) -> BTreeSet<(String, String)> {
        self.crates
            .values()
            .map(|c| (c.name.clone(), c.version.clone()))
            .collect()
    }

    /// Compute the SHA-256 hash of the manifest (excluding the manifest_hash field).
    pub fn compute_hash(&self) -> String {
        let mut m = self.clone();
        m.manifest_hash = None;
        let json = serde_json::to_string(&m).expect("manifest serialization cannot fail");
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Set the manifest hash.
    pub fn seal(&mut self) {
        self.manifest_hash = Some(self.compute_hash());
    }

    /// Verify the manifest hash.
    pub fn verify_hash(&self) -> Result<()> {
        let expected = self
            .manifest_hash
            .as_ref()
            .context("manifest has no hash")?;
        let computed = self.compute_hash();
        if expected != &computed {
            bail!(
                "manifest hash mismatch: expected {}, computed {}",
                expected,
                computed
            );
        }
        Ok(())
    }

    /// Compute the delta between this manifest and a previous one.
    /// Returns the set of (name, version) pairs that are new in `self`.
    pub fn diff(&self, previous: &Manifest) -> BTreeSet<(String, String)> {
        let prev_set = previous.crate_set();
        let curr_set = self.crate_set();
        curr_set.difference(&prev_set).cloned().collect()
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("failed to serialize manifest")
    }

    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("failed to parse manifest")
    }
}
