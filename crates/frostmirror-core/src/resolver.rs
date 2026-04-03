use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

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
    pub features2: BTreeMap<String, Vec<String>>,
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

/// An entry in the resolve queue: a crate we need to resolve plus whether
/// to use its default features.
#[derive(Debug, Clone)]
struct ResolveRequest {
    name: String,
    version_req: String,
    use_default_features: bool,
}

/// Iterative dependency resolver that works in levels.
///
/// Instead of using a callback for index lookups (which would require blocking
/// HTTP inside an async runtime), this resolver works in a pull-based way:
///
/// 1. Call `seed()` with your direct dependencies
/// 2. Call `pending_crates()` to get crate names that need index data
/// 3. Fetch those indexes (async, in the caller)
/// 4. Call `provide_index()` for each fetched index
/// 5. Call `advance()` to process one level of the graph
/// 6. Repeat 2-5 until `pending_crates()` returns empty
/// 7. Call `finish()` to get the resolved graph
pub struct Resolver {
    /// Cached index entries, keyed by crate name.
    index_cache: HashMap<String, Vec<IndexEntry>>,
    /// Queue of crates to resolve.
    queue: VecDeque<ResolveRequest>,
    /// Already resolved crates.
    resolved: BTreeSet<ResolvedCrate>,
    /// Already visited (name, version) pairs to avoid cycles.
    visited: HashSet<(String, String)>,
    /// Root crates (direct deps).
    roots: Vec<ResolvedCrate>,
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            index_cache: HashMap::new(),
            queue: VecDeque::new(),
            resolved: BTreeSet::new(),
            visited: HashSet::new(),
            roots: Vec::new(),
        }
    }

    /// Seed the resolver with direct dependencies from `depends.toml`.
    pub fn seed(&mut self, deps: &BTreeMap<String, String>) {
        for (name, version_req) in deps {
            self.queue.push_back(ResolveRequest {
                name: name.clone(),
                version_req: version_req.clone(),
                use_default_features: true,
            });
        }
    }

    /// Provide index entries for a crate (fetched by the caller).
    pub fn provide_index(&mut self, name: &str, entries: Vec<IndexEntry>) {
        self.index_cache.insert(name.to_string(), entries);
    }

    /// Return the names of crates that are queued but whose index data
    /// is not yet in the cache. The caller should fetch these and call
    /// `provide_index` before calling `advance`.
    pub fn pending_crates(&self) -> Vec<String> {
        let mut needed: BTreeSet<String> = BTreeSet::new();
        for req in &self.queue {
            let dep_name = req.name.clone();
            if !self.index_cache.contains_key(&dep_name) {
                needed.insert(dep_name);
            }
        }
        needed.into_iter().collect()
    }

    /// Returns true if there are still crates to resolve.
    pub fn has_work(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Process all currently queued crates. This resolves their versions,
    /// computes enabled features, and enqueues their transitive deps.
    /// After this call, `pending_crates()` may return new names to fetch.
    pub fn advance(&mut self) -> Result<()> {
        let current_queue: Vec<ResolveRequest> = self.queue.drain(..).collect();

        for req in current_queue {
            let entries = match self.index_cache.get(&req.name) {
                Some(e) => e,
                None => {
                    tracing::warn!("no index data for {} (skipping)", req.name);
                    continue;
                }
            };

            let matched = match find_best_match(entries, &req.name, &req.version_req) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("{}", e);
                    continue;
                }
            };

            let key = (matched.name.clone(), matched.version.clone());
            if self.visited.contains(&key) {
                continue;
            }
            self.visited.insert(key);

            let resolved_crate = ResolvedCrate {
                name: matched.name.clone(),
                version: matched.version.clone(),
            };

            // Track roots (first level)
            if self.resolved.is_empty() || self.roots.iter().any(|r| r.name == req.name) {
                if !self.roots.iter().any(|r| r.name == req.name) {
                    // This is a direct dep being resolved for the first time
                } else {
                    // Already in roots
                }
            }

            self.resolved.insert(resolved_crate.clone());

            // Compute features and activated optional deps
            let enabled_features =
                compute_enabled_features(&matched, req.use_default_features);
            let activated_optional_deps =
                compute_activated_deps(&matched, &enabled_features);

            // Enqueue transitive deps
            for dep in &matched.deps {
                if dep.kind.as_deref() == Some("dev") || dep.kind.as_deref() == Some("build") {
                    continue;
                }

                if dep.optional && !activated_optional_deps.contains(&dep.name) {
                    continue;
                }

                let dep_name = dep.package.as_deref().unwrap_or(&dep.name);

                self.queue.push_back(ResolveRequest {
                    name: dep_name.to_string(),
                    version_req: dep.req.clone(),
                    use_default_features: dep.default_features,
                });
            }
        }

        Ok(())
    }

    /// Consume the resolver and return the final resolved graph.
    pub fn finish(self, direct_deps: &BTreeMap<String, String>) -> ResolvedGraph {
        // Reconstruct roots from direct deps
        let roots: Vec<ResolvedCrate> = direct_deps
            .keys()
            .filter_map(|name| {
                self.resolved
                    .iter()
                    .find(|c| c.name == *name)
                    .cloned()
            })
            .collect();

        ResolvedGraph {
            crates: self.resolved,
            roots,
        }
    }
}

/// Compute the full set of enabled feature names for an index entry,
/// starting from "default" (if requested) and recursively expanding
/// feature-to-feature references.
fn compute_enabled_features(entry: &IndexEntry, use_default: bool) -> HashSet<String> {
    let mut enabled = HashSet::new();

    let mut all_features: BTreeMap<String, Vec<String>> = entry.features.clone();
    for (k, v) in &entry.features2 {
        all_features
            .entry(k.clone())
            .or_default()
            .extend(v.clone());
    }

    if use_default {
        if let Some(defaults) = all_features.get("default") {
            let mut stack: Vec<String> = defaults.clone();
            while let Some(feat) = stack.pop() {
                if enabled.insert(feat.clone()) {
                    if let Some(sub) = all_features.get(&feat) {
                        stack.extend(sub.clone());
                    }
                }
            }
        }
    }

    enabled
}

/// Determine which optional dependencies are activated by the set of enabled
/// features.
fn compute_activated_deps(entry: &IndexEntry, enabled_features: &HashSet<String>) -> HashSet<String> {
    let mut activated = HashSet::new();

    let optional_dep_names: HashSet<&str> = entry
        .deps
        .iter()
        .filter(|d| d.optional)
        .map(|d| d.name.as_str())
        .collect();

    let mut all_features: BTreeMap<String, Vec<String>> = entry.features.clone();
    for (k, v) in &entry.features2 {
        all_features
            .entry(k.clone())
            .or_default()
            .extend(v.clone());
    }

    for feat_name in enabled_features {
        if optional_dep_names.contains(feat_name.as_str()) {
            activated.insert(feat_name.clone());
        }

        if let Some(values) = all_features.get(feat_name) {
            for value in values {
                if let Some(dep_ref) = value.strip_prefix("dep:") {
                    activated.insert(dep_ref.to_string());
                } else if optional_dep_names.contains(value.as_str()) {
                    activated.insert(value.clone());
                } else if value.contains('/') {
                    let dep_part = value.split('/').next().unwrap();
                    if optional_dep_names.contains(dep_part) {
                        activated.insert(dep_part.to_string());
                    }
                }
            }
        }
    }

    activated
}

/// Find the best matching version for a version requirement.
fn find_best_match(entries: &[IndexEntry], name: &str, version_req: &str) -> Result<IndexEntry> {
    let req = semver::VersionReq::parse(version_req)
        .or_else(|_| semver::VersionReq::parse(&format!("={}", version_req)))
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
