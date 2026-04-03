# Frostmirror

<div style="width:15%; margin: auto;">

![Dual Mode](/resources/frostmirror_icon.svg)

</div>

A lightweight, dependency-scoped Rust mirror tool designed for air-gapped environments.

<div style="width:50%; margin: auto;">

![Dual Mode](/resources/frostmirror_dual_mode.svg)

</div>
Unlike tools that mirror all of crates.io (panamax) or all of rustup (romt), frostmirror only fetches the crates required to build your specific project. It delegates dependency resolution to cargo itself, downloads exactly what is needed, and packages everything into a single timestamped `.pkg` bundle compressed with brotli. Bundles are designed for incremental transfer across an air gap -- only the delta since the last bundle needs to be transported.

<div style="width:50%; margin: auto;">

![Update flow](/resources/frostmirror_airgap_update_flow.svg)

</div>
This projects was greatly inspired from Panamax and Romt. But crates.io is getting so heavy it’s no more possible to use it. 
This project was created with Claude code. I publish it to help others people that may have the same problem has me.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [Core Concepts](#core-concepts)
- [CLI Reference](#cli-reference)
- [Docker Usage](#docker-usage)
- [Docker Image Scripts](#docker-image-scripts)
- [Air-Gap Workflow](#air-gap-workflow)
- [Web UI](#web-ui)
- [API Reference](#api-reference)
- [Client Configuration](#client-configuration)
- [Environment Variables](#environment-variables)
- [Project Architecture](#project-architecture)
- [Development](#development)
- [Troubleshooting](#troubleshooting)

---

## Quick Start

### Step 1 -- Define your dependencies

Create a `depends.toml` file listing the crates your project needs:

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "net", "macros"] }
serde = { version = "1", features = ["derive"] }
axum = "0.7"

[platforms]
targets = ["x86_64-unknown-linux-gnu"]
toolchain = "stable"
```

You only need to list direct dependencies. frostmirror delegates to `cargo generate-lockfile` to resolve the entire transitive dependency tree -- features, optional deps, and platform-specific deps are all handled by cargo's own resolver.

### Step 2 -- Fetch (online machine)

```bash
frostmirror fetch --config depends.toml --output ./releases/
```

This produces a file like `20260402-2130-crates.pkg` in `./releases/`. The bundle contains every `.crate` file, sparse index entries, rustup binaries, and a cargo config.

### Step 3 -- Transfer across the air gap

```bash
cp ./releases/20260402-2130-crates.pkg /media/usb/
```

### Step 4 -- Import (offline machine)

Drop the `.pkg` file into the incoming directory:

```bash
cp /media/usb/20260402-2130-crates.pkg ./incoming/
```

If the registry container is running with `--watch-incoming`, the import happens automatically. Otherwise, import manually:

```bash
frostmirror import 20260402-2130-crates.pkg --mirror /mirror
```

### Step 5 -- Build your project offline

```toml
# ~/.cargo/config.toml
[source.frostmirror]
registry = "sparse+http://frostmirror.internal:8080/index/"

[source.crates-io]
replace-with = "frostmirror"
```

```bash
cargo build  # resolves everything from frostmirror
```

---

## Installation

### From source

```bash
git clone https://github.com/pillisan42/frostmirror
cd frostmirror
cargo install --path crates/frostmirror
```

### With cargo

> not available right now (I will figure it out later)

```bash
cargo install frostmirror
```

### Docker

```bash
docker build -t frostmirror:latest -f docker/Dockerfile .
```

---

## Core Concepts

### `depends.toml`

The single source of truth for what gets mirrored. Three formats are supported and can be mixed freely:

```toml
[dependencies]
# Simple -- just a version string
anyhow = "1"

# Extended -- version + features (same syntax as Cargo.toml)
tokio = { version = "1", features = ["rt-multi-thread", "net", "macros"] }
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4"] }

# Extended -- disable default features
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }

# Multiple versions of the same crate -- use an array
# Useful when mirroring for multiple projects with conflicting requirements
serde_json = ["1.0.60", "1.0.120"]

[platforms]
targets = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]
toolchain = "stable"
```

**Version strings** follow semver (e.g. `"1"`, `"1.50.0"`, `">=0.12, <0.13"`).

**Features** are passed directly to cargo's resolver. If a crate has optional dependencies activated through features (like `ratatui` activating `ratatui-crossterm` via its `crossterm` default feature), they are automatically included.

**Multiple versions** of the same crate are supported via the array syntax. Each version is resolved independently so conflicting requirements across projects don't cause errors.

### Dependency resolution

frostmirror delegates resolution entirely to **cargo itself**. For each dependency in `depends.toml`, frostmirror:

1. Creates a temporary Cargo project with that dependency
2. Runs `cargo generate-lockfile` to produce an exact `Cargo.lock`
3. Parses the lock file to extract all `(name, version)` pairs
4. Merges results across all dependencies, deduplicating by `(name, version)`

This two-pass strategy ensures complete coverage:

| Pass | What it does | What it catches |
|---|---|---|
| **Combined** | All deps in one `Cargo.toml` | Unified transitive versions (version unification) |
| **Per-dependency** | Each dep in its own `Cargo.toml` | Conflict-specific versions, multi-version entries |

The result is the union of both passes. Because cargo does the resolution, all features, optional deps, platform-specific deps, and version unification are handled exactly as they would be in a real `cargo build`.

### `.pkg` bundle format

Each bundle is a brotli-compressed archive with a custom binary format:

| Section | Contents |
|---|---|
| `manifest.json` | Resolved dep graph, SHA-256 hashes, parent `.pkg` reference |
| `rustup/` | `rustup-init` binaries for declared target platforms |
| `crates/` | `.crate` files for all resolved packages |
| `index/` | Sparse index entries for all resolved crates |
| `config.toml` | Ready-to-use cargo source replacement config |

**Filename scheme:** `YYYYMMDD-HHMM-crates.pkg` (e.g. `20260402-2130-crates.pkg`)

### Incremental updates

After the initial full bundle, subsequent fetches only download new crates:

```bash
# First time -- full bundle (may be large)
frostmirror fetch --output ./releases/

# After editing depends.toml -- delta only
frostmirror fetch --incremental --output ./releases/
```

Incremental bundles include:
- **New `.crate` files** -- only crates not in the previous manifest
- **Full index entries** -- for all resolved crates (not just new ones), so cargo can resolve correctly on the air-gap side
- **Rustup binaries** -- for any new target platforms added since the last bundle

If no history exists, `--incremental` automatically falls back to a full fetch with a warning.

### Incoming watcher

On the offline machine, the serve command can watch for new `.pkg` files:

```
./incoming/
    (drop .pkg files here)
    done/      <-- successfully imported bundles
    failed/    <-- bundles that failed verification
```

When a `.pkg` file appears in `./incoming/`:
1. Waits for the file write to complete (size stability check)
2. SHA-256 manifest check and per-crate hash verification
3. If valid: merge into mirror atomically, move `.pkg` to `done/`
4. If invalid: move to `failed/`, log the error, mirror is untouched

The watcher runs on a dedicated thread so it never blocks the HTTP server.

---

## CLI Reference

### `frostmirror fetch`

Resolve dependencies and produce a `.pkg` bundle.

```bash
frostmirror fetch [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `-c, --config <PATH>` | `depends.toml` | Path to the depends.toml file |
| `-o, --output <DIR>` | `./output` | Output directory for .pkg files |
| `--incremental` | off | Only download crates not in the previous bundle |

**Examples:**

```bash
# Full fetch with default config
frostmirror fetch

# Full fetch with custom paths
frostmirror fetch --config /path/to/depends.toml --output /path/to/output/

# Incremental fetch (delta only)
frostmirror fetch --incremental --output ./releases/
```

**Note:** `cargo` must be installed on the machine running `fetch`, since frostmirror uses `cargo generate-lockfile` for dependency resolution.

### `frostmirror import`

Import a `.pkg` bundle into the local mirror store.

```bash
frostmirror import <FILE> [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `--mirror <DIR>` | `/mirror` | Mirror directory |

**Examples:**

```bash
frostmirror import 20260402-2130-crates.pkg --mirror /data/mirror
```

### `frostmirror serve`

Start the HTTP registry server with optional file-drop auto-import.

```bash
frostmirror serve [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `--bind <ADDR>` | `0.0.0.0:8080` | HTTP bind address |
| `--base-url <URL>` | `http://localhost:8080` | Base URL for generated configs |
| `--mirror <DIR>` | `/mirror` | Mirror directory |
| `--incoming <DIR>` | `/incoming` | Incoming directory for .pkg files |
| `--watch-incoming` | off | Auto-import .pkg files dropped into incoming/ |

**Examples:**

```bash
# Serve with auto-import and custom URL
frostmirror serve \
  --bind 0.0.0.0:3000 \
  --base-url http://mirrors.corp.internal:3000 \
  --mirror /data/mirror \
  --incoming /data/incoming \
  --watch-incoming
```

### `frostmirror verify`

Check the integrity of a `.pkg` bundle before transporting or importing it.

```bash
frostmirror verify <FILE>
```

```bash
frostmirror verify 20260402-2130-crates.pkg
# OK -- 183 crates, 2 rustup artifacts
```

### `frostmirror status`

Display current mirror state.

```bash
frostmirror status --mirror /data/mirror
# Crate count:  183
# Total size:   45231872 bytes
# Last import:  2026-04-02T21:30:00+00:00
```

### `frostmirror gc`

Garbage collect crates no longer referenced by the current manifest.

```bash
frostmirror gc --mirror /data/mirror
# Removed 3 crates, freed 1248576 bytes
```

Removed dependencies are **never pruned automatically**. You must run `gc` explicitly.

---

## Docker Usage

### Development

```bash
docker compose -f compose.dev.yml up dev    # hot-reload
docker compose -f compose.dev.yml run --rm test  # tests
```

### Online machine -- Docker fetch

```bash
FROSTMIRROR_MODE=full docker compose -f compose.fetch.yml run --rm fetch
FROSTMIRROR_MODE=incremental docker compose -f compose.fetch.yml run --rm fetch
```

### Offline machine -- Air-gapped registry

```bash
docker compose -f compose.airgap.yml up -d
curl http://localhost:8080/api/status
```

The container uses `network_mode: none` -- no outbound network at all. Drop `.pkg` files into `./incoming/` and they are imported automatically.

---

## Docker Image Scripts

Three helper scripts in `scripts/` handle the full lifecycle of Docker images.

| Script | Purpose | Run on |
|---|---|---|
| `scripts/build.sh` | Build all Docker images from source | Online machine |
| `scripts/export.sh` | Save images to a compressed `.tar.gz` archive | Online machine |
| `scripts/import.sh` | Load images from the archive into Docker | Offline machine |

### `scripts/build.sh`

```bash
./scripts/build.sh                # all images
./scripts/build.sh --production   # only frostmirror:latest
./scripts/build.sh --no-cache     # clean rebuild
```

The production image uses a multi-stage build: `rust:1.86-slim` for compilation, `debian:bookworm-slim` for the final runtime (~15 MB).

### `scripts/export.sh`

```bash
./scripts/export.sh                          # all images -> ./export/
./scripts/export.sh --production             # only production image
./scripts/export.sh --output /media/usb/     # write to USB drive
```

### `scripts/import.sh`

```bash
./scripts/import.sh /media/usb/frostmirror-images-20260402-2130.tar.gz
docker compose -f compose.airgap.yml up -d
```

---

## Air-Gap Workflow

### Complete example: from zero to offline builds

**On the online machine:**

```bash
# 1. Build and export the Docker image
./scripts/build.sh --production
./scripts/export.sh --production --output /media/usb/

# 2. Create depends.toml
cat > depends.toml << 'EOF'
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }

[platforms]
targets = ["x86_64-unknown-linux-gnu"]
toolchain = "stable"
EOF

# 3. Fetch everything
frostmirror fetch --output ./releases/

# 4. Verify before transport
frostmirror verify ./releases/20260402-2130-crates.pkg
```

**Transfer:**

```bash
cp ./releases/20260402-2130-crates.pkg /media/usb/
```

**On the offline machine:**

```bash
# 5. Load the Docker image (first time, or when updating)
./scripts/import.sh /media/usb/frostmirror-images-20260402-2130.tar.gz

# 6. Start the registry (first time only)
docker compose -f compose.airgap.yml up -d

# 7. Drop the bundle
cp /media/usb/20260402-2130-crates.pkg ./incoming/

# 8. Verify the import
curl http://localhost:8080/api/status

# 9. Configure cargo on each developer machine
cat > ~/.cargo/config.toml << 'EOF'
[source.frostmirror]
registry = "sparse+http://frostmirror.internal:8080/index/"

[source.crates-io]
replace-with = "frostmirror"
EOF

# 10. Build your project
cargo build
```

### Subsequent updates

```bash
# Online: delta only
frostmirror fetch --incremental --output ./releases/

# Transfer just the new .pkg
cp ./releases/20260403-1000-crates.pkg /media/usb/

# Offline: drop it in
cp /media/usb/20260403-1000-crates.pkg ./incoming/
# Auto-imported, old crates preserved, new targets included
```

### Mirroring for multiple projects

If different projects on the air-gap need conflicting dependency versions, list them all in `depends.toml`:

```toml
[dependencies]
# Project A uses an older serde
serde = ["=1.0.100", { version = "1", features = ["derive"] }]
# Project B uses ratatui
ratatui = "0.30.0"
# Both share tokio
tokio = { version = "1", features = ["full"] }
```

Each entry is resolved independently, so conflicting requirements don't cause errors. The mirror contains all required versions.

---

## Web UI

The web UI is served at the root URL (`http://frostmirror.internal:8080/`). No extra service or port required.

| Page | URL | Description |
|---|---|---|
| **Dashboard** | `/` | Crate count, mirror size, last import, watcher state, failed count |
| **Dependencies** | `/deps` | Edit `depends.toml` with a table UI, live TOML preview |
| **Configuration** | `/config` | Registry URL, bind address, targets, behavior toggles |
| **Packages** | `/packages` | Import history, bundle sizes, GC button |
| **Client Setup** | `/setup` | Generated shell commands and downloadable config files |

The Dashboard auto-refreshes every 30 seconds. The `failed` count is the primary operational alert -- if non-zero, inspect `./incoming/failed/`.

---

## API Reference

All API endpoints are served by the same process as the registry.

| Method | Endpoint | Description |
|---|---|---|
| `GET` | `/api/status` | Mirror health, crate count, last import time |
| `GET` | `/api/packages` | Import history list |
| `GET` | `/api/config` | Current frostmirror.toml as JSON |
| `POST` | `/api/config` | Write new config (triggers in-process reload) |
| `GET` | `/api/deps` | Current depends.toml as JSON |
| `POST` | `/api/deps` | Write new depends.toml |
| `GET` | `/api/incoming` | Watcher state, done/failed counts |
| `POST` | `/api/gc` | Trigger garbage collection |
| `GET` | `/api/setup/cargo-config` | Download cargo config.toml |
| `GET` | `/api/setup/rustup-env.sh` | Download shell env script |
| `GET` | `/api/setup/rustup-env.ps1` | Download PowerShell env script |

### Examples

```bash
# Check mirror status
curl -s http://localhost:8080/api/status | python3 -m json.tool

# Update dependencies (supports simple, extended, and array formats)
curl -X POST http://localhost:8080/api/deps \
  -H "Content-Type: application/json" \
  -d '{
    "dependencies": {
      "tokio": {"version": "1", "features": ["full"]},
      "serde": ["1.0.100", {"version": "1", "features": ["derive"]}],
      "anyhow": "1"
    }
  }'

# Trigger garbage collection
curl -X POST http://localhost:8080/api/gc
```

---

## Client Configuration

### Cargo

Add to `~/.cargo/config.toml` on each developer machine:

```toml
[source.frostmirror]
registry = "sparse+http://frostmirror.internal:8080/index/"

[source.crates-io]
replace-with = "frostmirror"
```

The `sparse+` prefix is required -- it tells cargo to use the HTTP sparse protocol instead of trying to git-clone the URL.

Or download the ready-made file:

```bash
curl http://frostmirror.internal:8080/api/setup/cargo-config > ~/.cargo/config.toml
```

### Rustup

```bash
export RUSTUP_DIST_SERVER=http://frostmirror.internal:8080
export RUSTUP_UPDATE_ROOT=http://frostmirror.internal:8080/rustup
```

Install rustup from the mirror:

```bash
# Linux/macOS
curl http://frostmirror.internal:8080/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
  -o rustup-init
chmod +x rustup-init && ./rustup-init

# Windows (PowerShell)
Invoke-WebRequest http://frostmirror.internal:8080/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe `
  -OutFile rustup-init.exe
.\rustup-init.exe
```

Note: Windows targets use `rustup-init.exe`, Linux/macOS targets use `rustup-init`.

### PowerShell (Windows)

```powershell
$env:RUSTUP_DIST_SERVER = "http://frostmirror.internal:8080"
$env:RUSTUP_UPDATE_ROOT = "http://frostmirror.internal:8080/rustup"
```

---

## Environment Variables

| Variable | Used by | Default | Description |
|---|---|---|---|
| `FROSTMIRROR_MODE` | fetch | `full` | `full` or `incremental` |
| `FROSTMIRROR_PLATFORMS` | fetch | `x86_64-unknown-linux-gnu` | Comma-separated target triples |
| `FROSTMIRROR_TOOLCHAIN` | fetch | `stable` | Rust toolchain channel |
| `FROSTMIRROR_OUTPUT` | fetch | `./output` | Where to write `.pkg` files |
| `FROSTMIRROR_HOME` | fetch | `~/.frostmirror` | State directory (history, config) |
| `FROSTMIRROR_REGISTRY_URL` | fetch | crates.io | Override sparse index URL |
| `FROSTMIRROR_DL_URL` | fetch | static.crates.io | Override crate download URL |
| `FROSTMIRROR_DIST_URL` | fetch | static.rust-lang.org | Override rustup dist URL |
| `FROSTMIRROR_HISTORY` | fetch | `~/.frostmirror/history` | Manifest history directory |
| `FROSTMIRROR_BASE_URL` | serve | `http://localhost:8080` | Base URL embedded in client configs |
| `FROSTMIRROR_BIND` | serve | `0.0.0.0:8080` | HTTP bind address |
| `FROSTMIRROR_MIRROR` | serve | `/mirror` | Mirror data directory |
| `FROSTMIRROR_INCOMING` | serve | `/incoming` | Incoming `.pkg` drop directory |
| `RUST_LOG` | all | `info` | Log verbosity (`debug`, `info`, `warn`, `error`) |

---

## Project Architecture

```
frostmirror/
├── crates/
│   ├── frostmirror-core/       # Library: bundle format, manifest, diff logic
│   ├── frostmirror-fetch/      # Library: cargo-based resolver, crate/rustup downloaders
│   ├── frostmirror-import/     # Library: .pkg extraction, atomic mirror merge, GC
│   ├── frostmirror-serve/      # Library: HTTP server, sparse registry, web UI, watcher
│   └── frostmirror/            # Binary: CLI entrypoint
├── docker/                     # Dockerfiles and entrypoint
├── scripts/                    # Build, export, and import Docker images
├── tests/integration/          # Docker-based integration tests
└── depends.toml                # Self-hosted: frostmirror mirrors its own deps
```

| Crate | Role |
|---|---|
| `frostmirror-core` | Bundle format (brotli + custom binary archive), manifest with SHA-256 integrity, `depends.toml` parser with features/multi-version support |
| `frostmirror-fetch` | Delegates to `cargo generate-lockfile` for resolution, downloads `.crate` files and index entries, produces `.pkg` bundles |
| `frostmirror-import` | Decompresses and verifies `.pkg` bundles, atomically merges contents into the mirror store |
| `frostmirror-serve` | Axum-based HTTP server implementing the Cargo sparse registry protocol (`nest` + `fallback` routing), rustup dist serving, REST API, embedded web UI, incoming watcher on dedicated thread |
| `frostmirror` | CLI binary wiring everything together via clap |

---

## Development

### Prerequisites

- Rust 1.86+ (required for Cargo.lock v4 format)
- Docker and Docker Compose (for integration tests and production images)

### Build

```bash
cargo build --workspace
```

### Run unit tests

```bash
cargo test --workspace
```

### Run with verbose logging

```bash
RUST_LOG=debug cargo run -p frostmirror -- fetch --config depends.toml
```

### Integration tests

```bash
./scripts/build.sh
docker compose -f compose.test.yml run --rm test-runner
```

| Test | What it validates |
|---|---|
| T1 -- Full fetch | End-to-end: fetch, bundle, auto-import, crates served |
| T2 -- Incremental | Delta bundle is smaller, parent chain valid, merge is additive |
| T3 -- Corruption | Corrupted `.pkg` rejected, mirror state unchanged |
| T4 -- Version conflict | Both versions of a crate included when graph requires them |
| T5 -- Cargo offline | `cargo build` succeeds using only the frostmirror registry |
| T6 -- Rustup offline | `rustup-init` installs a toolchain from the mirror |

---

## Troubleshooting

### `cargo generate-lockfile` fails during fetch

frostmirror requires `cargo` to be installed on the machine running `fetch`. The fetcher creates temporary Cargo projects and runs `cargo generate-lockfile` to resolve dependencies. If cargo is not in `PATH`, the fetch will fail.

### "no matching package found" on the air-gap

Check that:
1. The crate and version are in your `depends.toml` (or are transitive deps of something that is)
2. A `.pkg` containing that crate has been imported
3. Your `~/.cargo/config.toml` uses the `sparse+` prefix: `registry = "sparse+http://..."`

Without `sparse+`, cargo tries to git-clone the URL instead of using the HTTP sparse protocol, which will always fail.

Run with debug logging on the server to see what cargo is requesting:

```bash
RUST_LOG=debug frostmirror serve --mirror /data/mirror
```

### "failed to download" -- crate file returns 404

The index entry exists but the `.crate` file is missing from the mirror. This can happen if:
- The crate was resolved in a previous `depends.toml` but the `.pkg` containing it was never imported
- The crate version was unified differently by cargo (e.g. your project resolves `aho-corasick 1.1.4` but the mirror only has `1.1.3`)

Fix: re-run `frostmirror fetch` (full, not incremental) to rebuild the bundle with the current resolution, then re-import.

### Incremental fetch falls back to full

This happens when no previous manifest is found in the history directory (`~/.frostmirror/history/`). Normal on the first run. The warning is informational.

### Bundle verification fails on import

The `.pkg` file may be corrupted (truncated transfer, bad disk). It is moved to `./incoming/failed/`. Re-transfer the original and try again.

```bash
frostmirror verify 20260402-2130-crates.pkg
```

### Web UI shows failed count > 0

Inspect `./incoming/failed/`. Common causes: corrupted transfer, disk full. Fix the issue, then re-drop a valid `.pkg`.

### Mirror taking too much disk space

```bash
frostmirror gc --mirror /data/mirror
# Or via API:
curl -X POST http://localhost:8080/api/gc
```

### Windows rustup-init returns 404

Windows targets use `rustup-init.exe` (not `rustup-init`). Make sure your `depends.toml` lists the Windows target:

```toml
[platforms]
targets = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]
```

frostmirror automatically uses the correct filename per platform.
