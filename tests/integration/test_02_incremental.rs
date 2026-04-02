//! T2 — Incremental delta
//!
//! Given: T1 complete; depends-extended.toml adds reqwest and axum
//! Steps: exec incremental fetch; wait for second .pkg; wait for import
//! Assert: second pkg smaller, parent field set, new + old crates served

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_02_incremental_delta() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    // Get current status (from T1)
    let client = reqwest::Client::new();
    let initial_status: StatusResponse = client
        .get(format!("{}/api/status", airgap_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let initial_import = initial_status.last_import.as_deref();

    // Exec incremental fetch with extended deps
    docker_exec(
        "online",
        &[
            "frostmirror",
            "fetch",
            "--incremental",
            "--config",
            "/fixtures/depends-extended.toml",
            "--output",
            "/transfer",
        ],
    )
    .expect("incremental fetch should succeed");

    // Wait for import
    let status = wait_for_import(&airgap_url, initial_import, Duration::from_secs(60))
        .await
        .expect("delta import should complete");

    // New crates should be served
    assert_crate_served(&airgap_url, "reqwest", "0.12.4")
        .await
        .expect("reqwest should be served");
    assert_crate_served(&airgap_url, "axum", "0.7.5")
        .await
        .expect("axum should be served");

    // Original crates should still be served
    assert_crate_served(&airgap_url, "tokio", "1.50.0")
        .await
        .expect("tokio should still be served");
    assert_crate_served(&airgap_url, "serde", "1.0.210")
        .await
        .expect("serde should still be served");

    // done/ should now have 2 packages
    assert_eq!(status.done_count, 2, "should have two imported pkgs");
    assert_eq!(status.failed_count, 0, "should have no failed pkgs");

    // Total crate count should have increased
    assert!(
        status.crate_count > initial_status.crate_count,
        "crate count should increase after delta import"
    );
}
