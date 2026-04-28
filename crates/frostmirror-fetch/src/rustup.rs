use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

const DEFAULT_DIST_URL: &str = "https://static.rust-lang.org";

/// Return the correct rustup-init binary name for a target triple.
/// Windows targets use `rustup-init.exe`, everything else uses `rustup-init`.
pub fn rustup_init_filename(target: &str) -> &'static str {
    if target.contains("windows") {
        "rustup-init.exe"
    } else {
        "rustup-init"
    }
}

/// Download rustup-init for a given target triple.
pub async fn download_rustup_init(
    client: &reqwest::Client,
    dist_url: Option<&str>,
    target: &str,
) -> Result<Vec<u8>> {
    let base = dist_url.unwrap_or(DEFAULT_DIST_URL);
    let filename = rustup_init_filename(target);
    let url = format!("{}/rustup/dist/{}/{}", base, target, filename);
    tracing::info!("downloading {} for {} from {}", filename, target, url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to download rustup-init for {}", target))?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "rustup-init download for {} returned HTTP {}",
            target,
            resp.status()
        );
    }

    Ok(resp.bytes().await?.to_vec())
}

/// Download the channel manifest (e.g., channel-rust-stable.toml).
pub async fn download_channel_manifest(
    client: &reqwest::Client,
    dist_url: Option<&str>,
    channel: &str,
) -> Result<String> {
    let base = dist_url.unwrap_or(DEFAULT_DIST_URL);
    let url = format!("{}/dist/channel-rust-{}.toml", base, channel);
    tracing::info!("downloading channel manifest from {}", url);

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("channel manifest download returned HTTP {}", resp.status());
    }

    Ok(resp.text().await?)
}

/// Download a toolchain component archive.
pub async fn download_component(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>> {
    tracing::debug!("downloading component from {}", url);
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("component download returned HTTP {}", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

// ---------- Channel manifest parsing ----------

/// Parsed representation of a rustup channel manifest (e.g. channel-rust-stable.toml).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ChannelManifest {
    pub date: String,
    pub pkg: BTreeMap<String, Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub version: Option<String>,
    pub target: BTreeMap<String, TargetInfo>,
}

#[derive(Debug, Deserialize)]
pub struct TargetInfo {
    pub available: bool,
    pub url: Option<String>,
    pub hash: Option<String>,
    pub xz_url: Option<String>,
    pub xz_hash: Option<String>,
}

/// A component to download, extracted from the channel manifest.
#[derive(Debug, Clone)]
pub struct ComponentDownload {
    /// The full download URL.
    pub url: String,
    /// The path relative to the dist server root, e.g.
    /// `2024-01-09/rustc-1.75.0-x86_64-unknown-linux-gnu.tar.xz`.
    pub dist_path: String,
}

/// Parse a channel manifest TOML and extract all component download URLs
/// for the given targets. Prefers `xz_url` over `url` for smaller downloads.
pub fn parse_channel_manifest(
    manifest_toml: &str,
    targets: &[String],
    dist_url: Option<&str>,
) -> Result<Vec<ComponentDownload>> {
    let manifest: ChannelManifest =
        toml::from_str(manifest_toml).context("failed to parse channel manifest TOML")?;
    let base = dist_url.unwrap_or(DEFAULT_DIST_URL);

    let mut downloads = Vec::new();

    for (_pkg_name, pkg) in &manifest.pkg {
        for target_name in targets {
            if let Some(info) = pkg.target.get(target_name.as_str()) {
                if !info.available {
                    continue;
                }
                // Prefer xz, fall back to gz
                let url = info
                    .xz_url
                    .as_deref()
                    .or(info.url.as_deref());
                if let Some(url) = url {
                    let dist_path = extract_dist_path(url, base);
                    downloads.push(ComponentDownload {
                        url: url.to_string(),
                        dist_path,
                    });
                }
            }
        }
    }

    Ok(downloads)
}

/// Extract the path relative to `/dist/` from a full URL.
/// e.g. `https://static.rust-lang.org/dist/2024-01-09/rustc-...tar.xz`
/// → `2024-01-09/rustc-...tar.xz`
fn extract_dist_path(url: &str, base: &str) -> String {
    let dist_prefix = format!("{}/dist/", base);
    if let Some(rest) = url.strip_prefix(&dist_prefix) {
        rest.to_string()
    } else {
        // Fallback: try to find `/dist/` anywhere in the URL
        if let Some(idx) = url.find("/dist/") {
            url[idx + 6..].to_string()
        } else {
            // Last resort: use the filename
            url.rsplit('/').next().unwrap_or("unknown").to_string()
        }
    }
}
