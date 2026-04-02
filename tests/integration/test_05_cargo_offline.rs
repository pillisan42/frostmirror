//! T5 — cargo build succeeds fully offline
//!
//! Given: T1 mirror state; fresh CARGO_HOME; minimal Cargo.toml depending on tokio + serde
//! Steps: point cargo config at airgap; run cargo build
//! Assert: cargo build exits 0, binary exists

mod helpers;

use helpers::*;
use std::time::Duration;

#[tokio::test]
async fn test_05_cargo_build_offline() {
    let airgap_url =
        std::env::var("AIRGAP_BASE_URL").unwrap_or_else(|_| "http://localhost:18080".to_string());

    // Create a minimal project and configure cargo to use frostmirror
    docker_exec(
        "test-runner",
        &[
            "bash",
            "-c",
            &format!(
                r#"
                mkdir -p /tmp/test-project/src
                cat > /tmp/test-project/Cargo.toml << 'TOML'
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = "1.50.0"
serde = "1.0.210"
TOML

                cat > /tmp/test-project/src/main.rs << 'RS'
fn main() {{
    println!("hello from frostmirror");
}}
RS

                mkdir -p /usr/local/cargo
                cat > /usr/local/cargo/config.toml << CONF
[source.frostmirror]
registry = "{url}/index"

[source.crates-io]
replace-with = "frostmirror"
CONF

                cd /tmp/test-project && cargo build 2>&1
                "#,
                url = airgap_url
            ),
        ],
    )
    .expect("cargo build should succeed");

    // Verify binary exists
    docker_exec(
        "test-runner",
        &["test", "-f", "/tmp/test-project/target/debug/test-project"],
    )
    .expect("compiled binary should exist");
}
