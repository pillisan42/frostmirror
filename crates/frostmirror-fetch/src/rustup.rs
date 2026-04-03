use anyhow::{Context, Result};

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
