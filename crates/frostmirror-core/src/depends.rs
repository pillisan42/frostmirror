use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The `depends.toml` file: single source of truth for what gets mirrored.
///
/// Supports simple, extended, and multi-version dependency specifications:
///
/// ```toml
/// [dependencies]
/// # Simple: just a version
/// anyhow = "1"
///
/// # Extended: version + features
/// tokio = { version = "1", features = ["rt-multi-thread", "net", "macros"] }
///
/// # Extended: with default-features control
/// reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
///
/// # Multiple versions of the same crate — use an array
/// serde = ["1.0.0", { version = "1.0.210", features = ["derive"] }]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependsToml {
    pub dependencies: BTreeMap<String, DepEntry>,
    #[serde(default)]
    pub platforms: Platforms,
}

/// A dependency entry — either a single spec or multiple specs (for
/// mirroring multiple versions of the same crate).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepEntry {
    /// Single version: `name = "1.0"` or `name = { version = "1.0", ... }`
    Single(DepSpec),
    /// Multiple versions: `name = ["1.0", { version = "2.0", features = [...] }]`
    Multiple(Vec<DepSpec>),
}

impl DepEntry {
    /// Iterate over all specs in this entry.
    pub fn specs(&self) -> Vec<&DepSpec> {
        match self {
            DepEntry::Single(s) => vec![s],
            DepEntry::Multiple(m) => m.iter().collect(),
        }
    }
}

/// A dependency specification — either a plain version string or a table
/// with version, features, and default-features.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Simple: `"1.0"`
    Simple(String),
    /// Extended: `{ version = "1.0", features = ["derive"] }`
    Extended(DepSpecExtended),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepSpecExtended {
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default = "default_true", rename = "default-features")]
    pub default_features: bool,
}

fn default_true() -> bool {
    true
}

impl DepSpec {
    pub fn version(&self) -> &str {
        match self {
            DepSpec::Simple(v) => v,
            DepSpec::Extended(e) => &e.version,
        }
    }

    pub fn features(&self) -> &[String] {
        match self {
            DepSpec::Simple(_) => &[],
            DepSpec::Extended(e) => &e.features,
        }
    }

    pub fn default_features(&self) -> bool {
        match self {
            DepSpec::Simple(_) => true,
            DepSpec::Extended(e) => e.default_features,
        }
    }

    /// Generate the Cargo.toml dependency value string for this spec.
    pub fn to_cargo_toml_value(&self) -> String {
        match self {
            DepSpec::Simple(v) => format!("\"{}\"", v),
            DepSpec::Extended(e) => {
                let mut parts = vec![format!("version = \"{}\"", e.version)];
                if !e.default_features {
                    parts.push("default-features = false".to_string());
                }
                if !e.features.is_empty() {
                    let feats: Vec<String> =
                        e.features.iter().map(|f| format!("\"{}\"", f)).collect();
                    parts.push(format!("features = [{}]", feats.join(", ")));
                }
                format!("{{ {} }}", parts.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platforms {
    #[serde(default = "default_targets")]
    pub targets: Vec<String>,
    #[serde(default = "default_toolchain")]
    pub toolchain: String,
}

impl Default for Platforms {
    fn default() -> Self {
        Self {
            targets: default_targets(),
            toolchain: default_toolchain(),
        }
    }
}

fn default_targets() -> Vec<String> {
    vec!["x86_64-unknown-linux-gnu".to_string()]
}

fn default_toolchain() -> String {
    "stable".to_string()
}

impl DependsToml {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read depends.toml at {}", path.display()))?;
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse depends.toml")
    }

    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize depends.toml")
    }

    /// Flatten all dependency entries into a list of (name, spec) pairs.
    /// Multi-version entries produce multiple pairs with the same name.
    pub fn flat_deps(&self) -> Vec<(String, DepSpec)> {
        let mut result = Vec::new();
        for (name, entry) in &self.dependencies {
            for spec in entry.specs() {
                result.push((name.clone(), spec.clone()));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let input = r#"
[dependencies]
tokio = "1.50.0"
serde = "1.0.210"

[platforms]
targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
toolchain = "stable"
"#;
        let depends = DependsToml::parse(input).unwrap();
        assert_eq!(depends.dependencies.len(), 2);
        let flat = depends.flat_deps();
        assert_eq!(flat.len(), 2);
    }

    #[test]
    fn test_parse_extended() {
        let input = r#"
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "net", "macros"] }
serde = { version = "1", features = ["derive"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
anyhow = "1"
"#;
        let depends = DependsToml::parse(input).unwrap();
        assert_eq!(depends.dependencies.len(), 4);
        let flat = depends.flat_deps();
        assert_eq!(flat.len(), 4);

        let tokio_spec = &flat.iter().find(|(n, _)| n == "tokio").unwrap().1;
        assert_eq!(tokio_spec.version(), "1");
        assert_eq!(tokio_spec.features().len(), 3);
    }

    #[test]
    fn test_parse_multi_version() {
        let input = r#"
[dependencies]
anyhow = "1"
serde = ["1.0.0", { version = "1.0.210", features = ["derive"] }]
"#;
        let depends = DependsToml::parse(input).unwrap();
        // 2 keys in the map
        assert_eq!(depends.dependencies.len(), 2);
        // but 3 total specs when flattened
        let flat = depends.flat_deps();
        assert_eq!(flat.len(), 3);

        let serde_specs: Vec<_> = flat.iter().filter(|(n, _)| n == "serde").collect();
        assert_eq!(serde_specs.len(), 2);
        assert_eq!(serde_specs[0].1.version(), "1.0.0");
        assert_eq!(serde_specs[1].1.version(), "1.0.210");
        assert_eq!(serde_specs[1].1.features(), &["derive"]);
    }

    #[test]
    fn test_to_cargo_toml_value() {
        let simple = DepSpec::Simple("1.0".to_string());
        assert_eq!(simple.to_cargo_toml_value(), "\"1.0\"");

        let extended = DepSpec::Extended(DepSpecExtended {
            version: "1".to_string(),
            features: vec!["derive".to_string()],
            default_features: true,
        });
        assert_eq!(
            extended.to_cargo_toml_value(),
            "{ version = \"1\", features = [\"derive\"] }"
        );

        let no_defaults = DepSpec::Extended(DepSpecExtended {
            version: "0.12".to_string(),
            features: vec!["rustls-tls".to_string()],
            default_features: false,
        });
        assert_eq!(
            no_defaults.to_cargo_toml_value(),
            "{ version = \"0.12\", default-features = false, features = [\"rustls-tls\"] }"
        );
    }
}
