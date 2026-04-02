use anyhow::{Context, Result};
use frostmirror_core::resolver::IndexEntry;
use frostmirror_core::bundle::crate_index_path;

/// Client for the crates.io sparse index.
pub struct SparseIndex {
    client: reqwest::Client,
    base_url: String,
}

impl SparseIndex {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("frostmirror/0.1.0")
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Default crates.io sparse index URL.
    pub fn crates_io() -> Self {
        Self::new("https://index.crates.io")
    }

    /// Fetch all index entries (all versions) for a named crate.
    pub async fn fetch_crate_index(&self, name: &str) -> Result<Vec<IndexEntry>> {
        let path = crate_index_path(name);
        let url = format!("{}/{}", self.base_url, path);

        tracing::debug!("fetching index for {} from {}", name, url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to fetch index for {}", name))?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "index fetch for {} returned HTTP {}: {}",
                name,
                resp.status(),
                url
            );
        }

        let body = resp.text().await?;
        parse_index_entries(&body, name)
    }

    /// Fetch the raw index text for embedding in a bundle.
    pub async fn fetch_raw_index(&self, name: &str) -> Result<String> {
        let path = crate_index_path(name);
        let url = format!("{}/{}", self.base_url, path);

        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("index fetch for {} returned HTTP {}", name, resp.status());
        }

        Ok(resp.text().await?)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Parse the ndjson sparse index format into IndexEntry structs.
pub fn parse_index_entries(body: &str, _name: &str) -> Result<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<IndexEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!("failed to parse index line: {}: {}", e, line);
            }
        }
    }
    Ok(entries)
}

/// Download a `.crate` file from crates.io.
pub async fn download_crate(
    client: &reqwest::Client,
    dl_base: &str,
    name: &str,
    version: &str,
) -> Result<Vec<u8>> {
    let url = format!("{}/{}/{}/download", dl_base, name, version);
    tracing::debug!("downloading {}-{} from {}", name, version, url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to download {}-{}", name, version))?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "download of {}-{} returned HTTP {}",
            name,
            version,
            resp.status()
        );
    }

    let bytes = resp.bytes().await?.to_vec();
    Ok(bytes)
}
