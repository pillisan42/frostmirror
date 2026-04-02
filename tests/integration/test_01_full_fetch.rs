//! T1 — Full fetch and import
//!
//! Given: depends-minimal.toml (tokio 1.50.0, serde 1.0.210), empty airgap mirror
//! Steps: exec frostmirror fetch in online container; wait for .pkg in /transfer; wait for airgap import
//! Assert: index entries exist, crates downloadable, done/ has one .pkg, failed/ empty

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_01_full_fetch_and_import() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    // Wait for airgap to be healthy
    wait_for_healthy(&format!("{}/api/status", airgap_url), Duration::from_secs(30))
        .await
        .expect("airgap should become healthy");

    // Exec full fetch in the online container
    docker_exec(
        "online",
        &[
            "frostmirror",
            "fetch",
            "--config",
            "/fixtures/depends-minimal.toml",
            "--output",
            "/transfer",
        ],
    )
    .expect("fetch should succeed");

    // Wait for airgap to auto-import
    let status = wait_for_import(&airgap_url, None, Duration::from_secs(60))
        .await
        .expect("import should complete");

    // Assert: crates are served
    assert_crate_served(&airgap_url, "tokio", "1.50.0")
        .await
        .expect("tokio should be served");
    assert_crate_served(&airgap_url, "serde", "1.0.210")
        .await
        .expect("serde should be served");

    // Assert: done/ has one .pkg, failed/ is empty
    assert_eq!(status.done_count, 1, "should have exactly one imported pkg");
    assert_eq!(status.failed_count, 0, "should have no failed pkgs");
}
