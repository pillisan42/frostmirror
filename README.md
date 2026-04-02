# frostmirror

A lightweight, dependency-scoped Rust mirror tool designed for air-gapped environments.

Unlike tools that mirror all of crates.io (panamax) or all of rustup (romt), frostmirror only fetches the crates required to build your specific project. It resolves the full transitive dependency graph, downloads exactly what is needed, and packages everything into a single timestamped `.pkg` bundle compressed with brotli. Bundles are designed for incremental transfer across an air gap -- only the delta since the last bundle needs to be transported.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [Core Concepts](#core-concepts)
- [CLI Reference](#cli-reference)
- [Docker Usage](#docker-usage)
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

This section walks you through the most common scenario: producing a `.pkg` bundle on an internet-connected machine, transferring it to an air-gapped machine, and using it to install Rust crates offline.

### Step 1 -- Define your dependencies

Create a `depends.toml` file listing the crates your project needs:

```toml
[dependencies]
tokio = "1.50.0"
serde = "1.0.210"
axum = "0.7.5"

[platforms]
targets = ["x86_64-unknown-linux-gnu"]
toolchain = "stable"
```

You only need to list direct dependencies. frostmirror resolves the entire transitive dependency tree automatically.

### Step 2 -- Fetch (online machine)

```bash
frostmirror fetch --config depends.toml --output ./releases/
```

This produces a file like `20260402-2130-crates.pkg` in `./releases/`. The bundle contains every `.crate` file, sparse index slices, rustup binaries, and a cargo config -- everything needed to build offline.

### Step 3 -- Transfer across the air gap

Copy the `.pkg` file to a USB drive (or any other medium):

```bash
cp ./releases/20260402-2130-crates.pkg /media/usb/
```

### Step 4 -- Import (offline machine)

On the air-gapped machine, drop the `.pkg` file into the incoming directory:

```bash
cp /media/usb/20260402-2130-crates.pkg ./incoming/
```

If the registry container is running with `--watch-incoming`, the import happens automatically. Otherwise, import manually:

```bash
frostmirror import 20260402-2130-crates.pkg --mirror /mirror
```

### Step 5 -- Build your project offline

Configure cargo on the offline machine to use the frostmirror registry:

```bash
# ~/.cargo/config.toml
cat > ~/.cargo/config.toml << 'EOF'
[source.frostmirror]
registry = "http://frostmirror.internal:8080/index"

[source.crates-io]
replace-with = "frostmirror"
EOF

cargo build
```

That's it. `cargo build` now resolves all crates from your local frostmirror registry.

---

## Installation

### From source

```bash
git clone https://github.com/example/frostmirror
cd frostmirror
cargo install --path crates/frostmirror
```

### With cargo

```bash
cargo install frostmirror
```

### Docker

```bash
docker build -t frostmirror:latest .
```

---

## Core Concepts

### `depends.toml`

The single source of truth for what gets mirrored. You declare your direct dependencies and target platforms; frostmirror resolves everything else.

```toml
[dependencies]
tokio = "1.50.0"
serde = "1.0.210"
reqwest = "0.12.4"

[platforms]
targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
toolchain = "stable"
```

**Version strings** follow semver. If multiple transitive dependencies require different versions of the same crate, frostmirror includes all required versions -- it deduplicates by `(name, version)` pairs.

### `.pkg` bundle format

Each bundle is a brotli-compressed archive with a custom binary format:

| Section | Contents |
|---|---|
| `manifest.json` | Resolved dep graph, SHA-256 hashes, parent `.pkg` reference |
| `rustup/` | `rustup-init` binaries for declared target platforms |
| `crates/` | `.crate` files for all resolved packages |
| `index/` | Sparse index slices for crates present in this bundle |
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

Delta bundles include a `parent` field in their manifest referencing the previous `.pkg`, enabling chain validation on import.

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
1. SHA-256 manifest check runs
2. If valid: merge into mirror atomically, move `.pkg` to `done/`
3. If invalid: move to `failed/`, log the error, mirror is untouched

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
# Import a bundle
frostmirror import 20260402-2130-crates.pkg --mirror /data/mirror

# Import with default mirror path
frostmirror import ./releases/20260402-2130-crates.pkg
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
# Basic serve
frostmirror serve --mirror /data/mirror

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

**Example:**

```bash
frostmirror verify 20260402-2130-crates.pkg
# OK -- 47 crates, 2 rustup artifacts
```

Verification checks:
- Bundle decompression and format validity
- Manifest SHA-256 hash
- Per-crate file SHA-256 against manifest entries

### `frostmirror status`

Display current mirror state.

```bash
frostmirror status [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `--mirror <DIR>` | `/mirror` | Mirror directory |

**Example:**

```bash
frostmirror status --mirror /data/mirror
# Crate count:  47
# Total size:   12458923 bytes
# Last import:  2026-04-02T21:30:00+00:00
```

### `frostmirror gc`

Garbage collect crates no longer referenced by the current manifest.

```bash
frostmirror gc [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `--mirror <DIR>` | `/mirror` | Mirror directory |

**Example:**

```bash
frostmirror gc --mirror /data/mirror
# Removed 3 crates, freed 1248576 bytes
```

Removed dependencies are **never pruned automatically** -- this prevents breaking builds that still reference older crates. You must run `gc` explicitly.

---

## Docker Usage

### Development

```bash
# Start dev environment with hot-reload
docker compose -f compose.dev.yml up dev

# Run tests
docker compose -f compose.dev.yml run --rm test
```

### Online machine -- Docker fetch

```bash
# Full fetch
FROSTMIRROR_MODE=full docker compose -f compose.fetch.yml run --rm fetch

# Incremental fetch
FROSTMIRROR_MODE=incremental docker compose -f compose.fetch.yml run --rm fetch
```

The `.pkg` files are written to the `pkg-out` Docker volume. Copy them to your transfer medium:

```bash
# Find where Docker stores the volume
docker volume inspect frostmirror_pkg-out --format '{{ .Mountpoint }}'

# Or copy from a temporary container
docker run --rm -v frostmirror_pkg-out:/pkgs -v $(pwd):/out busybox \
  cp /pkgs/*.pkg /out/
```

### Offline machine -- Air-gapped registry

```bash
# Start the registry (runs permanently)
docker compose -f compose.airgap.yml up -d

# Check status
curl http://localhost:8080/api/status
```

The container uses `network_mode: none` -- no outbound network at all. Drop `.pkg` files into `./incoming/` and they are imported automatically.

---

## Air-Gap Workflow

### Complete example: from zero to offline builds

**On the online machine:**

```bash
# 1. Create depends.toml for your project
cat > depends.toml << 'EOF'
[dependencies]
tokio = "1.50.0"
serde = "1.0.210"
serde_json = "1"
clap = "4"

[platforms]
targets = ["x86_64-unknown-linux-gnu"]
toolchain = "stable"
EOF

# 2. Fetch everything
frostmirror fetch --output ./releases/

# 3. Verify before transport
frostmirror verify ./releases/20260402-2130-crates.pkg
```

**Transfer:**

```bash
# Copy to USB (or any transfer medium)
cp ./releases/20260402-2130-crates.pkg /media/usb/
```

**On the offline machine:**

```bash
# 4. Start the registry (first time only)
docker compose -f compose.airgap.yml up -d

# 5. Drop the bundle
cp /media/usb/20260402-2130-crates.pkg ./incoming/

# 6. Wait a moment, then verify the import
curl http://localhost:8080/api/status
# {"crate_count":47,"total_size":12458923,...}

# 7. Configure cargo on each developer machine
cat > ~/.cargo/config.toml << 'EOF'
[source.frostmirror]
registry = "http://frostmirror.internal:8080/index"

[source.crates-io]
replace-with = "frostmirror"
EOF

# 8. Build your project
cargo build  # resolves everything from frostmirror
```

### Subsequent updates

When dependencies change, only the delta needs to cross the air gap:

```bash
# Online machine
frostmirror fetch --incremental --output ./releases/
# Produces a small delta .pkg

# Transfer just the new .pkg
cp ./releases/20260403-1000-crates.pkg /media/usb/

# Offline machine -- drop it in
cp /media/usb/20260403-1000-crates.pkg ./incoming/
# Auto-imported, old crates preserved
```

---

## Web UI

The web UI is served at the root URL (`http://frostmirror.internal:8080/`). No extra service or port required.

### Pages

| Page | URL | Description |
|---|---|---|
| **Dashboard** | `/` | Crate count, mirror size, last import, watcher state, failed count |
| **Dependencies** | `/deps` | Edit `depends.toml` with a table UI, live TOML preview |
| **Configuration** | `/config` | Registry URL, bind address, targets, behavior toggles |
| **Packages** | `/packages` | Import history, bundle sizes, GC button |
| **Client Setup** | `/setup` | Generated shell commands and downloadable config files |

The Dashboard auto-refreshes every 30 seconds. The `failed` count on the dashboard is the primary operational alert -- if it is non-zero, inspect the files in `./incoming/failed/`.

---

## API Reference

All API endpoints are served by the same process as the registry.

| Method | Endpoint | Description |
|---|---|---|
| `GET` | `/api/status` | Mirror health, crate count, last import time |
| `GET` | `/api/packages` | Import history list |
| `GET` | `/api/config` | Current frostmirror.toml as JSON |
| `POST` | `/api/config` | Write new config |
| `GET` | `/api/deps` | Current depends.toml as JSON |
| `POST` | `/api/deps` | Write new depends.toml |
| `GET` | `/api/incoming` | Watcher state, done/failed counts |
| `POST` | `/api/gc` | Trigger garbage collection |
| `GET` | `/api/setup/cargo-config` | Download cargo config.toml |
| `GET` | `/api/setup/rustup-env.sh` | Download shell env script |
| `GET` | `/api/setup/rustup-env.ps1` | Download PowerShell env script |

### Example: check mirror status with curl

```bash
curl -s http://frostmirror.internal:8080/api/status | python3 -m json.tool
```

```json
{
    "crate_count": 47,
    "total_size": 12458923,
    "total_size_human": "11.9 MB",
    "last_import": "2026-04-02T21:30:00+00:00",
    "watcher_active": true,
    "done_count": 2,
    "failed_count": 0
}
```

### Example: update dependencies via API

```bash
curl -X POST http://frostmirror.internal:8080/api/deps \
  -H "Content-Type: application/json" \
  -d '{
    "dependencies": {
      "tokio": "1.50.0",
      "serde": "1.0.210",
      "reqwest": "0.12.4"
    },
    "platforms": {
      "targets": ["x86_64-unknown-linux-gnu"],
      "toolchain": "stable"
    }
  }'
```

### Example: trigger garbage collection

```bash
curl -X POST http://frostmirror.internal:8080/api/gc
# {"removed":3,"freed_bytes":1248576}
```

---

## Client Configuration

### Cargo

Add to `~/.cargo/config.toml` on each developer machine:

```toml
[source.frostmirror]
registry = "http://frostmirror.internal:8080/index"

[source.crates-io]
replace-with = "frostmirror"
```

Or download the ready-made file from the web UI at `/setup`, or via:

```bash
curl http://frostmirror.internal:8080/api/setup/cargo-config > ~/.cargo/config.toml
```

### Rustup

Set environment variables in `.bashrc` (or equivalent):

```bash
export RUSTUP_DIST_SERVER=http://frostmirror.internal:8080
export RUSTUP_UPDATE_ROOT=http://frostmirror.internal:8080/rustup
```

Install rustup from the mirror:

```bash
curl http://frostmirror.internal:8080/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
  -o rustup-init
chmod +x rustup-init
./rustup-init
```

### PowerShell (Windows)

```powershell
$env:RUSTUP_DIST_SERVER = "http://frostmirror.internal:8080"
$env:RUSTUP_UPDATE_ROOT = "http://frostmirror.internal:8080/rustup"
```

Or download the script: `http://frostmirror.internal:8080/api/setup/rustup-env.ps1`

---

## Environment Variables

All settings can be controlled via environment variables, making Docker and CI integration straightforward.

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
| `FROSTMIRROR_PKG_DIR` | import | `/pkgs` | Where to read `.pkg` files |
| `FROSTMIRROR_BASE_URL` | serve | `http://localhost:8080` | Base URL embedded in client configs |
| `FROSTMIRROR_BIND` | serve | `0.0.0.0:8080` | HTTP bind address |
| `FROSTMIRROR_MIRROR` | serve | `/mirror` | Mirror data directory |
| `FROSTMIRROR_INCOMING` | serve | `/incoming` | Incoming `.pkg` drop directory |
| `RUST_LOG` | all | `info` | Log verbosity (`debug`, `info`, `warn`, `error`) |

---

## Project Architecture

frostmirror is organized as a Cargo workspace with five crates:

```
frostmirror/
├── crates/
│   ├── frostmirror-core/       # Library: dep resolution, bundle format, manifest, diff
│   ├── frostmirror-fetch/      # Library: sparse index client, crate/rustup downloaders
│   ├── frostmirror-import/     # Library: .pkg extraction, atomic mirror merge, GC
│   ├── frostmirror-serve/      # Library: HTTP server, sparse registry, web UI, watcher
│   └── frostmirror/            # Binary: CLI entrypoint
├── docker/                     # Dockerfiles and entrypoint
├── tests/integration/          # Docker-based integration tests
└── depends.toml                # Self-hosted: frostmirror mirrors its own deps
```

### Crate responsibilities

| Crate | Role |
|---|---|
| `frostmirror-core` | Bundle format (brotli + custom binary archive), manifest with SHA-256 integrity, semver resolver against sparse index, diff logic for incremental updates |
| `frostmirror-fetch` | Async downloads from crates.io sparse index and rustup dist servers, produces `.pkg` bundles |
| `frostmirror-import` | Decompresses and verifies `.pkg` bundles, atomically merges contents into the mirror store |
| `frostmirror-serve` | Axum-based HTTP server implementing the Cargo sparse registry protocol, rustup dist serving, REST API, and embedded web UI |
| `frostmirror` | CLI binary wiring everything together via clap |

`frostmirror-core` is published as a standalone library so other tools can embed the resolution and bundle format logic.

---

## Development

### Prerequisites

- Rust 1.77+
- Docker and Docker Compose (for integration tests)

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

### Docker development with hot-reload

```bash
docker compose -f compose.dev.yml up dev
```

### Integration tests

The integration tests spin up real Docker containers -- one online (with a mock registry), one fully air-gapped -- and pass `.pkg` bundles across a shared volume:

```bash
# Build the test image
docker build -t frostmirror:test .

# Run all 6 tests
docker compose -f compose.test.yml run --rm test-runner

# Run a single test
docker compose -f compose.test.yml run --rm test-runner \
  cargo test --test integration test_02_incremental

# Keep containers up after failure for inspection
docker compose -f compose.test.yml up --abort-on-container-exit
```

**Test suite:**

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

### "no matching version" during fetch

The resolver could not find a version satisfying your requirement. Check that:
- The crate name is spelled correctly in `depends.toml`
- The version exists on crates.io and is not yanked
- Your version string is valid semver (e.g. `"1.0"`, `">=0.12, <0.13"`, `"=1.50.0"`)

### Incremental fetch falls back to full

This happens when no previous manifest is found in the history directory. It is normal on the first run or after clearing `~/.frostmirror/history/`. The warning is informational.

### Bundle verification fails on import

The `.pkg` file may be corrupted (truncated transfer, bad disk). The file is moved to `./incoming/failed/`. Re-transfer the original `.pkg` and try again.

To verify a bundle before transporting it:

```bash
frostmirror verify 20260402-2130-crates.pkg
```

### `cargo build` says "no matching package found"

Make sure:
1. The crate and version are included in your `depends.toml`
2. A `.pkg` containing that crate has been imported
3. Your `~/.cargo/config.toml` points to the correct frostmirror URL

Check what the mirror has:

```bash
curl http://frostmirror.internal:8080/api/status
```

### Web UI shows failed count > 0

Inspect the failed bundles:

```bash
ls ./incoming/failed/
```

Common causes: corrupted transfer, incompatible bundle version, disk full. Fix the issue, then re-drop a valid `.pkg`.

### Mirror taking too much disk space

Run garbage collection to remove crates no longer in the current manifest:

```bash
frostmirror gc --mirror /data/mirror
```

Or trigger it from the web UI (Packages page) or API:

```bash
curl -X POST http://frostmirror.internal:8080/api/gc
```
