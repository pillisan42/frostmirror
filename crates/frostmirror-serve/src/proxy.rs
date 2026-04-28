use anyhow::{Context, Result};
use std::path::Path;

/// Fetch `upstream_url` and atomically write the response body to `cache_path`.
///
/// Concurrent fetches for the same path are safe: each writes to a unique temp
/// file then renames over the destination. Both fetches retrieve identical
/// bytes from upstream, so whichever rename wins, the cached file is correct.
///
/// Returns the bytes so the caller can serve the response without re-reading
/// from disk on a cache miss.
pub async fn fetch_and_cache(
    client: &reqwest::Client,
    upstream_url: &str,
    cache_path: &Path,
) -> Result<Vec<u8>> {
    tracing::info!("proxy fetch: {}", upstream_url);

    let resp = client
        .get(upstream_url)
        .send()
        .await
        .with_context(|| format!("upstream request failed: {}", upstream_url))?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "upstream returned HTTP {} for {}",
            resp.status(),
            upstream_url
        );
    }

    let bytes = resp
        .bytes()
        .await
        .context("failed to read upstream response body")?
        .to_vec();

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let nonce: u64 = rand_u64();
    let tmp_path = cache_path.with_extension(format!(
        "tmp.{}-{}",
        std::process::id(),
        nonce
    ));

    tokio::fs::write(&tmp_path, &bytes)
        .await
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    if let Err(e) = tokio::fs::rename(&tmp_path, cache_path).await {
        // Best-effort cleanup of the temp file; the original error is what matters.
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(anyhow::Error::new(e).context(format!(
            "failed to publish cache file {}",
            cache_path.display()
        )));
    }

    Ok(bytes)
}

/// Cheap nonce — we only need uniqueness within the lifetime of a process,
/// not cryptographic randomness. Avoids pulling in the `rand` crate.
fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tid = std::thread::current().id();
    let tid_hash = format!("{:?}", tid);
    let mut h: u64 = nanos;
    for b in tid_hash.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u64);
    }
    h
}
