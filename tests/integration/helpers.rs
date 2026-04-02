use anyhow::Result;
use frostmirror_core::bundle::BundleReader;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    pub crate_count: u64,
    pub total_size: u64,
    pub last_import: Option<String>,
    pub watcher_active: bool,
    pub done_count: u64,
    pub failed_count: u64,
}

/// Poll until airgap /api/status shows a new last_import timestamp.
pub async fn wait_for_import(
    base_url: &str,
    previous_ts: Option<&str>,
    timeout: Duration,
) -> Result<StatusResponse> {
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!("timed out waiting for import");
        }

        if let Ok(resp) = client
            .get(format!("{}/api/status", base_url))
            .send()
            .await
        {
            if let Ok(status) = resp.json::<StatusResponse>().await {
                if let Some(ref ts) = status.last_import {
                    match previous_ts {
                        Some(prev) if ts != prev => return Ok(status),
                        None => return Ok(status),
                        _ => {}
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Assert a crate is served: index entry exists and download returns a valid archive.
pub async fn assert_crate_served(base_url: &str, name: &str, version: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let index_path = frostmirror_core::bundle::crate_index_path(name);

    // Check index entry
    let resp = client
        .get(format!("{}/index/{}", base_url, index_path))
        .send()
        .await?;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "index entry for {} should exist",
        name
    );
    let body = resp.text().await?;
    assert!(
        body.contains(version),
        "index for {} should contain version {}",
        name,
        version
    );

    // Check crate download
    let resp = client
        .get(format!(
            "{}/crates/{}/{}/download",
            base_url, name, version
        ))
        .send()
        .await?;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "crate download for {}-{} should succeed",
        name,
        version
    );
    let bytes = resp.bytes().await?;
    assert!(!bytes.is_empty(), "crate download should not be empty");

    Ok(())
}

/// Assert a crate is not present in the index.
pub async fn assert_crate_absent(base_url: &str, name: &str, _version: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let index_path = frostmirror_core::bundle::crate_index_path(name);

    let resp = client
        .get(format!("{}/index/{}", base_url, index_path))
        .send()
        .await?;
    assert_eq!(
        resp.status().as_u16(),
        404,
        "index entry for {} should not exist",
        name
    );

    Ok(())
}

/// Read the manifest from a .pkg file and return the (name, version) set.
pub fn pkg_manifest_crates(pkg_path: &Path) -> Result<HashSet<(String, String)>> {
    let bundle = BundleReader::read_file(pkg_path)?;
    Ok(bundle
        .manifest
        .crates
        .values()
        .map(|c| (c.name.clone(), c.version.clone()))
        .collect())
}

/// Exec a command inside a named Docker container and return stdout.
pub fn docker_exec(container: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("docker")
        .arg("exec")
        .arg(container)
        .args(args)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker exec failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Wait until a URL returns 200, polling every 500ms up to timeout.
pub async fn wait_for_healthy(url: &str, timeout: Duration) -> Result<()> {
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!("timed out waiting for {} to become healthy", url);
        }

        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
