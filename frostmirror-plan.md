# frostmirror — project plan

## Overview

**frostmirror** is a lightweight, dependency-scoped Rust mirror tool designed for air-gapped environments. Unlike panamax (mirrors all of crates.io) and romt (mirrors all of rustup), frostmirror only fetches the crates required to build a specific project, as declared in a `depends.toml` file. It resolves the full transitive dependency graph, downloads exactly what is needed, and packages everything into a single timestamped `.pkg` bundle compressed with brotli. The bundle is designed for incremental transfer across an air gap — only the delta since the last bundle needs to be transported each update cycle.

---

## Core concepts

### `depends.toml`

The single source of truth for what gets mirrored. Format:

```toml
[dependencies]
tokio = "1.50.0"
onnxruntime = "0.1.0"

[platforms]
targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
toolchain = "stable"
```

Version strings follow semver. If multiple transitive dependencies require different versions of the same crate, frostmirror automatically includes all required versions — it walks the full dependency graph and deduplicates by `(name, version)` pairs.

### `.pkg` bundle format

Filename scheme: `YYYYMMDD-HHMM-crates.pkg`
Example: `20260402-2130-crates.pkg`

A single brotli-compressed archive containing:

| Section | Contents |
|---|---|
| `manifest.json` | Resolved dep graph, content hashes, parent `.pkg` reference |
| `rustup/` | `rustup-init` binaries for declared target platforms |
| `crates/` | `.crate` files for all resolved packages |
| `index/` | Sparse index slices for crates present in this bundle |
| `config.toml` | Ready-to-use cargo source replacement config |

The `parent` field in the manifest references the previous `.pkg` filename, enabling incremental chain validation on import.

### Incremental update strategy

1. **First transfer**: full `.pkg` (large, happens once)
2. **Subsequent updates**: `frostmirror fetch --incremental` diffs the newly resolved graph against the previous manifest, downloads only new `(name, version)` tuples, and produces a small delta `.pkg`
3. **On the offline machine**: drop the `.pkg` into `./incoming/` — the container detects it automatically and imports it

Removed dependencies are never pruned automatically; pruning requires an explicit `frostmirror gc` command. This prevents accidentally breaking builds that still reference removed crates.

---

## Fetch modes

The fetch side (online, produces `.pkg` files) supports two equally valid modes. The serve side (offline, runs the registry) is Docker-only.

### Native CLI mode

No Docker daemon required. Install once, run anywhere with internet access.

```bash
cargo install frostmirror
```

State is kept in `~/.frostmirror/` (overridable via `FROSTMIRROR_HOME` or `--home`):

```
~/.frostmirror/
├── history/
│   └── 20260401-2000-manifest.json    # previous run's resolved dep graph
└── config.toml                         # default platforms, toolchain, output dir
```

Usage:

```bash
# First time — full bundle
frostmirror fetch --output ./releases/

# After editing depends.toml — delta only
frostmirror fetch --incremental --output ./releases/

# Verify a bundle before transporting it
frostmirror verify 20260402-2130-crates.pkg
```

If no history exists, `--incremental` automatically falls back to a full fetch and warns.

### Docker CLI mode

Same binary, same behavior. State is kept in named Docker volumes instead of `~/.frostmirror/`. Suited for CI pipelines and shared team environments.

```bash
# Full fetch
FROSTMIRROR_MODE=full docker compose -f compose.fetch.yml run --rm fetch

# Incremental
FROSTMIRROR_MODE=incremental docker compose -f compose.fetch.yml run --rm fetch
```

---

## CLI reference

```
frostmirror fetch                   # full fetch from depends.toml
frostmirror fetch --incremental     # delta only vs previous .pkg
frostmirror import <file.pkg>       # apply bundle to local mirror (native)
frostmirror serve                   # HTTP server for cargo + rustup
frostmirror serve --watch-incoming  # serve + auto-import on file drop
frostmirror status                  # show mirror state, dep counts
frostmirror verify <file.pkg>       # check integrity before importing
frostmirror gc                      # remove unused crates from local store
```

---

## Crate workspace structure

```
frostmirror/
├── crates/
│   ├── frostmirror-core/       # dep resolution, bundle format, diff logic
│   ├── frostmirror-fetch/      # download engine (crates.io sparse index, rustup dist)
│   ├── frostmirror-import/     # .pkg extraction and mirror merge
│   ├── frostmirror-serve/      # HTTP registry (sparse protocol) + rustup dist server
│   └── frostmirror/            # CLI binary — native entrypoint + Docker entrypoint
├── tests/
│   └── integration/            # see Integration tests section
├── docker/
│   ├── Dockerfile              # multi-stage: builder → runtime
│   ├── Dockerfile.dev          # dev image with full toolchain + cargo-watch
│   ├── Dockerfile.mock-registry
│   └── entrypoint.sh           # CMD dispatch: fetch|import|serve|gc|verify
├── compose.dev.yml
├── compose.fetch.yml           # online machine, Docker fetch (optional)
├── compose.airgap.yml          # offline machine, Docker registry (required)
├── compose.test.yml            # integration test environment
├── depends.toml                # frostmirror is self-hosted
└── Cargo.toml
```

`frostmirror-core` is published as a library crate so other tools can embed the dep-resolution and bundle format logic.

---

## Docker configuration

### `Dockerfile` — multi-stage production build

```dockerfile
# stage 1: builder
FROM rust:1.77-slim AS builder

WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p frostmirror && \
    cp target/release/frostmirror /frostmirror-bin

# stage 2: runtime (~15 MB final image)
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /frostmirror-bin /usr/local/bin/frostmirror
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

VOLUME ["/pkgs", "/mirror", "/config"]
EXPOSE 8080

ENTRYPOINT ["/entrypoint.sh"]
CMD ["serve"]
```

### `compose.dev.yml` — development workflow

```yaml
services:
  dev:
    build:
      context: .
      dockerfile: docker/Dockerfile.dev
    volumes:
      - .:/workspace
      - cargo-cache:/usr/local/cargo/registry
      - build-cache:/workspace/target
    working_dir: /workspace
    environment:
      - RUST_LOG=debug
      - RUST_BACKTRACE=1
    command: cargo watch -x "build -p frostmirror"

  test:
    build:
      context: .
      dockerfile: docker/Dockerfile.dev
    volumes:
      - .:/workspace
      - cargo-cache:/usr/local/cargo/registry
      - build-cache:/workspace/target
    working_dir: /workspace
    command: cargo test --workspace

volumes:
  cargo-cache:
  build-cache:
```

### `compose.fetch.yml` — online machine, Docker fetch (optional)

```yaml
services:
  fetch:
    image: frostmirror:latest
    command: fetch
    environment:
      - FROSTMIRROR_MODE=${FROSTMIRROR_MODE:-full}
      - FROSTMIRROR_PLATFORMS=${FROSTMIRROR_PLATFORMS:-x86_64-unknown-linux-gnu}
      - FROSTMIRROR_TOOLCHAIN=${FROSTMIRROR_TOOLCHAIN:-stable}
      - FROSTMIRROR_OUTPUT=/pkgs
    volumes:
      - ./depends.toml:/config/depends.toml:ro
      - pkg-out:/pkgs
      - pkg-history:/pkg-history
    network_mode: bridge

volumes:
  pkg-out:
  pkg-history:
```

### `compose.airgap.yml` — offline registry (Docker required)

```yaml
services:
  frostmirror:
    image: frostmirror:latest
    command: serve --watch-incoming
    environment:
      - FROSTMIRROR_BASE_URL=http://frostmirror.internal:8080
      - FROSTMIRROR_BIND=0.0.0.0:8080
    volumes:
      - ./incoming:/incoming     # host dir — drop .pkg files here
      - mirror-store:/mirror
    ports:
      - "8080:8080"
    network_mode: none           # hard air-gap: no outbound network
    restart: unless-stopped

volumes:
  mirror-store:
```

`network_mode: none` removes all network interfaces except loopback. The port mapping still works because it is handled by the Docker host's network stack.

---

## Air-gap update workflow

The `serve --watch-incoming` flag spawns a background thread watching `/incoming/` via inotify. When a `.pkg` file's write completes:

1. SHA-256 manifest check and parent chain validation runs
2. If verification passes: merge into `/mirror` atomically (write to temp path, then rename), move `.pkg` to `/incoming/done/`
3. If verification fails: move to `/incoming/failed/`, log the error — the running registry is untouched
4. The HTTP serve layer picks up new index entries on the next request — no restart needed

```
./incoming/
├── (drop .pkg files here)
├── done/      ← successfully imported
└── failed/    ← failed verification, inspect before retrying
```

### Full update cycle

| Step | Native CLI path | Docker CLI path |
|---|---|---|
| Produce `.pkg` | `frostmirror fetch --incremental` | `FROSTMIRROR_MODE=incremental docker compose -f compose.fetch.yml run --rm fetch` |
| Transfer | Copy `.pkg` to USB | Copy `.pkg` to USB |
| Import | Drop into `./incoming/` | Drop into `./incoming/` |
| Done | Container auto-imports | Container auto-imports |

The Docker service on the offline machine starts once (`docker compose -f compose.airgap.yml up -d`) and runs permanently. Every future update is a single file drop.

---

## Environment variables reference

| Variable | Service | Default | Description |
|---|---|---|---|
| `FROSTMIRROR_MODE` | fetch | `full` | `full` or `incremental` |
| `FROSTMIRROR_PLATFORMS` | fetch | `x86_64-unknown-linux-gnu` | Comma-separated target triples |
| `FROSTMIRROR_TOOLCHAIN` | fetch | `stable` | Rust toolchain channel or pinned version |
| `FROSTMIRROR_OUTPUT` | fetch | `./output` | Where to write `.pkg` files (native) |
| `FROSTMIRROR_HOME` | fetch | `~/.frostmirror` | State directory (native mode) |
| `FROSTMIRROR_PKG_DIR` | import | `/pkgs` | Where to read `.pkg` files from |
| `FROSTMIRROR_BASE_URL` | serve | `http://localhost:8080` | Embedded in `config.toml` written to clients |
| `FROSTMIRROR_BIND` | serve | `0.0.0.0:8080` | HTTP bind address |
| `RUST_LOG` | all | `info` | Log verbosity |

---

## Web UI

The web UI is served by `frostmirror-serve` at `/`. No extra service or port — the registry API lives under `/index/` and `/rustup/`. Static assets are embedded into the binary via `rust-embed`.

### Pages

**Dashboard** — read-only status pulled from `/api/status`: crate count, mirror size on disk, last import timestamp, incoming watcher state, and `done/` / `failed/` file counts. The `failed/` count is the primary operational alert.

**Dependencies** — edit `depends.toml` through a table UI. Each row shows the crate name, a version input, transitive dep count, and a resolution status badge (resolved / conflict / unresolved). Conflict badges surface version range incompatibilities detected by the resolver. A live TOML preview updates as rows are edited. Saving writes the file to disk; it does not trigger a fetch.

**Configuration** — writes to `frostmirror.toml`. Controls: registry base URL, bind address, toolchain channel, target platform checkboxes, and four behavior toggles (watch incoming, verify checksums, keep failed packages, prune on import). All toggles correspond directly to env vars in the compose file.

**Packages** — full import history from the `done/` archive with bundle type (full / delta), size, date, and status. GC button removes archived bundles beyond the configured retention count. The current bundle is always protected.

**Client setup** — generates shell commands and config file snippets for a new client machine, using the base URL from Configuration. Download buttons produce ready-made files: `cargo config.toml`, `rustup-env.sh`, `rustup-env.ps1`.

### API surface

```
GET  /api/status                   mirror health, crate count, last import time
GET  /api/packages                 import history list
GET  /api/config                   current frostmirror.toml as JSON
POST /api/config                   write new config (triggers in-process reload)
GET  /api/deps                     current depends.toml parsed to JSON
POST /api/deps                     write new depends.toml
GET  /api/incoming                 watcher state + done/failed counts
POST /api/gc                       trigger GC run
GET  /api/setup/cargo-config       download cargo config.toml
GET  /api/setup/rustup-env.sh      download shell env script
GET  /api/setup/rustup-env.ps1     download PowerShell env script
```

All writes go through the same code paths used by the CLI.

---

## Client configuration

Once the registry is running, configure client machines as follows.

### Environment variables (add to `.bashrc` or equivalent)

```bash
export RUSTUP_DIST_SERVER=http://frostmirror.internal:8080
export RUSTUP_UPDATE_ROOT=http://frostmirror.internal:8080/rustup
```

### Install rustup from the mirror

```bash
curl http://frostmirror.internal:8080/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
  -o rustup-init
chmod +x rustup-init && ./rustup-init
```

### Configure cargo (`~/.cargo/config.toml`)

```toml
[source.frostmirror]
registry = "http://frostmirror.internal:8080/index"

[source.crates-io]
replace-with = "frostmirror"
```

---

## Integration tests

Tests spin up real containers — one online, one fully isolated — and pass a `.pkg` across the shared volume boundary. A `mock-registry` container serves fixture `.crate` files so tests never hit the real crates.io.

### Infrastructure

```
tests/
└── integration/
    ├── fixtures/
    │   ├── depends-minimal.toml       # 2 direct deps, ~10 transitive
    │   ├── depends-extended.toml      # adds 3 more crates (for delta test)
    │   ├── depends-conflict.toml      # forces a version conflict
    │   ├── crates/                    # pre-built .crate fixture files
    │   └── mock-index/               # sparse index entries for fixtures
    ├── helpers.rs                    # wait_for_healthy(), assert_crate_served(), docker_exec()
    ├── test_01_full_fetch.rs
    ├── test_02_incremental.rs
    ├── test_03_corruption.rs
    ├── test_04_version_conflict.rs
    ├── test_05_cargo_offline.rs
    └── test_06_rustup_offline.rs
```

### `compose.test.yml`

```yaml
networks:
  online-net:
    driver: bridge

volumes:
  pkg-transfer:
  mirror-store:
  cargo-home:

services:
  mock-registry:
    build:
      context: .
      dockerfile: docker/Dockerfile.mock-registry
    networks: [online-net]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:80/health"]
      interval: 2s
      retries: 10

  online:
    image: frostmirror:test
    depends_on:
      mock-registry: { condition: service_healthy }
    environment:
      - FROSTMIRROR_REGISTRY_URL=http://mock-registry/index
      - FROSTMIRROR_DIST_URL=http://mock-registry/dist
      - FROSTMIRROR_OUTPUT=/transfer
      - FROSTMIRROR_HISTORY=/transfer/history
      - RUST_LOG=debug
    volumes:
      - ./tests/integration/fixtures:/fixtures:ro
      - pkg-transfer:/transfer
    networks: [online-net]

  airgap:
    image: frostmirror:test
    environment:
      - FROSTMIRROR_INCOMING=/transfer
      - FROSTMIRROR_MIRROR=/mirror
      - FROSTMIRROR_BIND=0.0.0.0:8080
      - FROSTMIRROR_BASE_URL=http://airgap:8080
      - RUST_LOG=debug
    volumes:
      - pkg-transfer:/transfer
      - mirror-store:/mirror
    network_mode: none
    ports:
      - "18080:8080"
    command: serve --watch-incoming

  test-runner:
    image: frostmirror:test
    depends_on:
      - online
      - airgap
    environment:
      - AIRGAP_BASE_URL=http://host.docker.internal:18080
    volumes:
      - ./tests:/tests:ro
      - pkg-transfer:/transfer
      - cargo-home:/usr/local/cargo
      - /var/run/docker.sock:/var/run/docker.sock
    networks: [online-net]
    command: cargo test --test integration -- --test-threads=1
```

Tests run sequentially (`--test-threads=1`) because T2–T6 depend on prior state. Files are named `test_01_` through `test_06_` so the test harness runs them in order.

### Test case specifications

#### T1 — full fetch and import

- **Given**: `depends-minimal.toml` (tokio 1.50.0, serde 1.0.210), empty airgap mirror
- **Steps**: exec `frostmirror fetch --config /fixtures/depends-minimal.toml` in online container; wait for `.pkg` in `/transfer`; wait for airgap to import
- **Assert**:
  - `GET /index/to/ki/tokio` → 200, contains `"1.50.0"`
  - `GET /crates/tokio/1.50.0/download` → 200
  - `/transfer/done/` contains exactly one `.pkg`
  - `/transfer/failed/` is empty
  - manifest lists all expected `(name, version)` pairs
  - airgap container has zero outbound connections (enforced by `network_mode: none`)

#### T2 — incremental delta

- **Given**: T1 complete; `depends-extended.toml` adds reqwest 0.12.4 and axum 0.7.5
- **Steps**: exec `frostmirror fetch --incremental` in online container; wait for second `.pkg`; wait for airgap to import
- **Assert**:
  - second `.pkg` is smaller than first
  - second `.pkg` manifest `parent` field equals first `.pkg` filename
  - new crates (reqwest, axum) served by airgap
  - original crates (tokio, serde) still served — merge was additive
  - `/transfer/done/` contains exactly two `.pkg` files
  - total crate count = T1 count + new transitive count

#### T3 — corrupted `.pkg` rejected, mirror intact

- **Given**: T1 mirror state; a `.pkg` with its last 512 bytes zeroed out
- **Steps**: copy corrupted file directly into `/transfer`; wait 5 seconds
- **Assert**:
  - `/transfer/failed/` contains the corrupted file
  - `/transfer/done/` count unchanged from T1
  - airgap crate count unchanged from T1
  - `last_import` timestamp unchanged
  - `GET /index/to/ki/tokio` → still 200

#### T4 — version conflict included

- **Given**: `depends-conflict.toml` where dep-a requires `serde = "1.0.0"` and dep-b requires `serde = "1.0.210"`
- **Steps**: exec fetch with conflict config; wait for `.pkg` and import
- **Assert**:
  - fetch exits 0 (conflict is resolved, not an error)
  - `.pkg` contains both `serde-1.0.0.crate` and `serde-1.0.210.crate`
  - airgap index for serde contains both version entries
  - `GET .../crates/serde/1.0.0/download` → 200
  - `GET .../crates/serde/1.0.210/download` → 200

#### T5 — `cargo build` succeeds fully offline

- **Given**: T1 mirror state; a fresh `CARGO_HOME` with no cached registry; a minimal `Cargo.toml` depending on tokio and serde
- **Steps**: point `~/.cargo/config.toml` at `http://airgap:8080/index`; run `cargo build`
- **Assert**:
  - `cargo build` exits 0
  - no outbound HTTP to `crates.io` or `static.crates.io` (only airgap:8080 in access logs)
  - compiled binary exists in `target/debug/`

#### T6 — `rustup` install succeeds fully offline

- **Given**: T1 mirror state (includes rustup-init and stable toolchain); a clean container with no Rust toolchain
- **Steps**: set `RUSTUP_DIST_SERVER` and `RUSTUP_UPDATE_ROOT` to airgap URL; download and run `rustup-init`
- **Assert**:
  - `rustup-init` exits 0
  - `rustc --version` prints a version string
  - `cargo --version` prints a version string
  - no outbound requests to `rustup.rs` or `static.rust-lang.org`

### Running the tests

```bash
# Build the test image
docker build -t frostmirror:test .

# Run the full suite
docker compose -f compose.test.yml run --rm test-runner

# Run a single test
docker compose -f compose.test.yml run --rm test-runner \
  cargo test --test integration test_02_incremental

# Keep containers up for manual inspection after a failure
docker compose -f compose.test.yml up --abort-on-container-exit
```

### CI integration (GitHub Actions)

```yaml
- name: Build test image
  run: docker build -t frostmirror:test .

- name: Run integration tests
  run: docker compose -f compose.test.yml run --rm test-runner

- name: Collect logs on failure
  if: failure()
  run: docker compose -f compose.test.yml logs > integration-logs.txt

- uses: actions/upload-artifact@v4
  if: failure()
  with:
    name: integration-logs
    path: integration-logs.txt
```

### Test helper API (`helpers.rs`)

```rust
/// Poll until airgap /api/status shows a new last_import timestamp.
pub async fn wait_for_import(base_url: &str, previous_ts: u64, timeout: Duration)
    -> Result<StatusResponse>

/// Assert a crate is served: index entry exists and download returns a valid archive.
pub async fn assert_crate_served(base_url: &str, name: &str, version: &str)
    -> Result<()>

/// Assert a crate is not present in the index.
pub async fn assert_crate_absent(base_url: &str, name: &str, version: &str)
    -> Result<()>

/// Read the manifest from a .pkg file and return the (name, version) set.
pub fn pkg_manifest_crates(pkg_path: &Path) -> Result<HashSet<(String, String)>>

/// Exec a command inside a named Docker container and return stdout.
pub fn docker_exec(container: &str, args: &[&str]) -> Result<String>

/// Wait until a URL returns 200, polling every 500ms up to timeout.
pub async fn wait_for_healthy(url: &str, timeout: Duration) -> Result<()>
```

---

## Key implementation notes

**Dependency resolution**: use the crates.io sparse index (`https://index.crates.io`) rather than cloning the full git index. The `guppy` crate can handle resolution rather than reimplementing it from scratch. Recurse from each entry in `depends.toml`, collect all required version ranges, resolve to concrete versions, and deduplicate by `(name, version)`.

**Bundle integrity**: every `.pkg` contains a SHA-256 manifest of all included `.crate` files. Verify before writing anything to disk. The `parent` field enables incremental chain validation — detect and reject a delta applied on top of the wrong base.

**Serve layer**: implement the Cargo sparse registry protocol (`/index/config.json` + per-crate paths) plus the rustup dist layout. The mirror is a flat-file store so no in-memory cache invalidation is needed on import.

**Brotli compression**: use the `brotli` crate. For the bundle format itself, use a simple custom archive (length-prefixed sections with a header table) rather than tar/zip — both ends are controlled so a bespoke format is more robust and easier to validate.

**Atomic import**: write new crate files and index entries to a temporary path inside the mirror volume, then rename into place. Rename on the same filesystem is atomic on Linux. This guarantees the running serve process never observes a partial import state.

---

## Comparison with existing tools

| | panamax | romt | frostmirror |
|---|---|---|---|
| Mirror scope | All of crates.io | Full rustup | Project deps only |
| Typical bundle size | GB+ | GB+ | MB – low GB |
| Incremental transfer | No | No | Yes — delta only |
| Air-gap workflow | Manual rsync | Partial | First-class |
| Dep-graph aware | Via `cargo vendor` | No | Native |
| Self-hosted toolchain | Yes | Yes | Yes |
| Web UI | No | No | Yes |
| Update action on airgap | Manual command | Manual command | Drop file |
