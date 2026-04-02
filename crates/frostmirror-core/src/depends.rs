use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The `depends.toml` file: single source of truth for what gets mirrored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependsToml {
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub platforms: Platforms,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_depends() {
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
        assert_eq!(depends.dependencies["tokio"], "1.50.0");
        assert_eq!(depends.platforms.targets.len(), 2);
    }
}
