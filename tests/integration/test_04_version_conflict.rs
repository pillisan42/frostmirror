//! T4 — Version conflict included
//!
//! Given: depends-conflict.toml where dep-a requires serde 1.0.0 and dep-b requires serde 1.0.210
//! Steps: exec fetch with conflict config; wait for .pkg and import
//! Assert: both serde versions present in the mirror

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_04_version_conflict_both_included() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    let client = reqwest::Client::new();
    let before: StatusResponse = client
        .get(format!("{}/api/status", airgap_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let prev_import = before.last_import.as_deref();

    // Exec fetch with conflict config
    docker_exec(
        "online",
        &[
            "frostmirror",
            "fetch",
            "--config",
            "/fixtures/depends-conflict.toml",
            "--output",
            "/transfer",
        ],
    )
    .expect("fetch with conflict config should succeed (exit 0)");

    // Wait for import
    let _status = wait_for_import(&airgap_url, prev_import, Duration::from_secs(60))
        .await
        .expect("conflict pkg import should complete");

    // Both serde versions should be served
    assert_crate_served(&airgap_url, "serde", "1.0.0")
        .await
        .expect("serde 1.0.0 should be served");
    assert_crate_served(&airgap_url, "serde", "1.0.210")
        .await
        .expect("serde 1.0.210 should be served");
}
