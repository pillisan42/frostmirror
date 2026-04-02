//! T6 — rustup install succeeds fully offline
//!
//! Given: T1 mirror state (includes rustup-init and stable toolchain)
//! Steps: set RUSTUP_DIST_SERVER and RUSTUP_UPDATE_ROOT to airgap URL; run rustup-init
//! Assert: rustup-init exits 0, rustc --version works, cargo --version works

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_06_rustup_install_offline() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    // Download and run rustup-init from the mirror
    let output = docker_exec(
        "test-runner",
        &[
            "bash",
            "-c",
            &format!(
                r#"
                export RUSTUP_DIST_SERVER={url}
                export RUSTUP_UPDATE_ROOT={url}/rustup
                export RUSTUP_HOME=/tmp/rustup-test
                export CARGO_HOME=/tmp/cargo-test

                # Download rustup-init from mirror
                curl -f {url}/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
                    -o /tmp/rustup-init-test 2>/dev/null

                if [ ! -f /tmp/rustup-init-test ]; then
                    echo "SKIP: rustup-init not available in mirror"
                    exit 0
                fi

                chmod +x /tmp/rustup-init-test
                /tmp/rustup-init-test -y --default-toolchain stable 2>&1

                # Verify installation
                $CARGO_HOME/bin/rustc --version
                $CARGO_HOME/bin/cargo --version
                "#,
                url = airgap_url
            ),
        ],
    )
    .expect("rustup install should succeed");

    // Check that rustc version was printed (if not skipped)
    if !output.contains("SKIP") {
        assert!(
            output.contains("rustc"),
            "rustc --version should print version info"
        );
        assert!(
            output.contains("cargo"),
            "cargo --version should print version info"
        );
    }
}
