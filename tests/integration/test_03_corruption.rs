//! T3 — Corrupted .pkg rejected, mirror intact
//!
//! Given: T1 mirror state; a .pkg with its last 512 bytes zeroed out
//! Steps: copy corrupted file into /transfer; wait 5 seconds
//! Assert: failed/ contains the corrupted file, mirror unchanged

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_03_corruption_rejected() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    // Get current status before corruption
    let client = reqwest::Client::new();
    let before: StatusResponse = client
        .get(format!("{}/api/status", airgap_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Create a corrupted .pkg: take the first done pkg, zero last 512 bytes
    docker_exec(
        "airgap",
        &[
            "bash",
            "-c",
            r#"
            PKG=$(ls /transfer/done/*.pkg | head -1)
            cp "$PKG" /tmp/corrupted.pkg
            FILESIZE=$(stat -c%s /tmp/corrupted.pkg)
            dd if=/dev/zero of=/tmp/corrupted.pkg bs=1 seek=$((FILESIZE-512)) count=512 conv=notrunc 2>/dev/null
            cp /tmp/corrupted.pkg /transfer/corrupted-test.pkg
            "#,
        ],
    )
    .expect("corruption setup should succeed");

    // Wait for the watcher to process
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Check status after
    let after: StatusResponse = client
        .get(format!("{}/api/status", airgap_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Crate count should be unchanged
    assert_eq!(
        after.crate_count, before.crate_count,
        "crate count should not change after corrupted import"
    );

    // Last import should be unchanged
    assert_eq!(
        after.last_import, before.last_import,
        "last_import should not change after corrupted import"
    );

    // Failed count should increase
    assert!(
        after.failed_count > before.failed_count,
        "failed count should increase"
    );

    // Tokio should still be served
    assert_crate_served(&airgap_url, "tokio", "1.50.0")
        .await
        .expect("tokio should still be served after corruption");
}
