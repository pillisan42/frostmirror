use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A single resolved crate with its concrete version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResolvedCrate {
    pub name: String,
    pub version: String,
}

/// The full resolved dependency graph: all crates needed to satisfy `depends.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedGraph {
    /// All resolved crates, deduplicated by (name, version).
    pub crates: BTreeSet<ResolvedCrate>,
    /// Direct dependencies as declared in depends.toml.
    pub roots: Vec<ResolvedCrate>,
}

/// Sparse index entry for a single crate version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    #[serde(rename = "vers")]
    pub version: String,
    #[serde(default)]
    pub deps: Vec<IndexDep>,
    #[serde(rename = "cksum")]
    pub checksum: String,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub yanked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDep {
    pub name: String,
    pub req: String,
    pub features: Vec<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "default_true")]
    pub default_features: bool,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
}

fn default_true() -> bool {
    true
}

impl ResolvedGraph {
    pub fn new() -> Self {
        Self {
            crates: BTreeSet::new(),
            roots: Vec::new(),
        }
    }

    /// Number of unique crates in the graph.
    pub fn crate_count(&self) -> usize {
        self.crates.len()
    }

    /// Get the set of (name, version) tuples.
    pub fn crate_set(&self) -> BTreeSet<(String, String)> {
        self.crates
            .iter()
            .map(|c| (c.name.clone(), c.version.clone()))
            .collect()
    }
}

/// Resolve dependencies from the sparse index.
///
/// This is a simplified resolver that:
/// 1. Fetches the index entry for each direct dependency
/// 2. Finds the best matching version
/// 3. Recursively resolves transitive dependencies
/// 4. Deduplicates by (name, version)
///
/// For a full production resolver, consider using `guppy` or `cargo`'s resolver.
pub struct Resolver {
    /// Callback to fetch index entries for a crate.
    /// Returns all versions' index lines for the named crate.
    index_lookup: Box<dyn Fn(&str) -> Result<Vec<IndexEntry>> + Send>,
    resolved: BTreeSet<ResolvedCrate>,
    /// Track what we've already attempted to resolve to avoid cycles.
    visited: BTreeSet<(String, String)>,
}

impl Resolver {
    pub fn new(index_lookup: impl Fn(&str) -> Result<Vec<IndexEntry>> + Send + 'static) -> Self {
        Self {
            index_lookup: Box::new(index_lookup),
            resolved: BTreeSet::new(),
            visited: BTreeSet::new(),
        }
    }

    /// Resolve all dependencies from the given direct dependency map.
    pub fn resolve(&mut self, deps: &BTreeMap<String, String>) -> Result<ResolvedGraph> {
        let mut roots = Vec::new();

        for (name, version_req) in deps {
            let entries = (self.index_lookup)(name)?;
            let matched = find_best_match(&entries, name, version_req)?;

            let resolved_crate = ResolvedCrate {
                name: name.clone(),
                version: matched.version.clone(),
            };
            roots.push(resolved_crate.clone());

            self.resolve_recursive(&matched)?;
        }

        Ok(ResolvedGraph {
            crates: self.resolved.clone(),
            roots,
        })
    }

    fn resolve_recursive(&mut self, entry: &IndexEntry) -> Result<()> {
        let key = (entry.name.clone(), entry.version.clone());
        if self.visited.contains(&key) {
            return Ok(());
        }
        self.visited.insert(key);

        self.resolved.insert(ResolvedCrate {
            name: entry.name.clone(),
            version: entry.version.clone(),
        });

        // Resolve non-optional, non-dev, non-build dependencies
        for dep in &entry.deps {
            if dep.optional {
                continue;
            }
            if dep.kind.as_deref() == Some("dev") || dep.kind.as_deref() == Some("build") {
                continue;
            }

            let dep_name = dep.package.as_deref().unwrap_or(&dep.name);
            let entries = match (self.index_lookup)(dep_name) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("failed to resolve dep {}: {}", dep_name, e);
                    continue;
                }
            };

            match find_best_match(&entries, dep_name, &dep.req) {
                Ok(matched) => {
                    self.resolve_recursive(&matched)?;
                }
                Err(e) => {
                    tracing::warn!("no matching version for {} {}: {}", dep_name, dep.req, e);
                }
            }
        }

        Ok(())
    }
}

/// Find the best matching version for a version requirement.
/// Uses a simplified semver matching strategy.
fn find_best_match(entries: &[IndexEntry], name: &str, version_req: &str) -> Result<IndexEntry> {
    let req = semver::VersionReq::parse(version_req)
        .or_else(|_| {
            // Try parsing as exact version
            semver::VersionReq::parse(&format!("={}", version_req))
        })
        .map_err(|e| anyhow::anyhow!("invalid version req '{}' for {}: {}", version_req, name, e))?;

    let mut candidates: Vec<&IndexEntry> = entries
        .iter()
        .filter(|e| !e.yanked)
        .filter(|e| {
            semver::Version::parse(&e.version)
                .map(|v| req.matches(&v))
                .unwrap_or(false)
        })
        .collect();

    // Sort by version descending to pick the latest match
    candidates.sort_by(|a, b| {
        let va = semver::Version::parse(&a.version).unwrap();
        let vb = semver::Version::parse(&b.version).unwrap();
        vb.cmp(&va)
    });

    candidates
        .into_iter()
        .next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no matching version for {} {}", name, version_req))
}
