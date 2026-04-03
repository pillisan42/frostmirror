#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# build.sh — Build all frostmirror Docker images
#
# Produces:
#   frostmirror:latest        Used by compose.fetch.yml and compose.airgap.yml
#   frostmirror:test          Used by compose.test.yml (online, airgap, test-runner)
#   frostmirror-mock:latest   Used by compose.test.yml (mock-registry)
#
# Usage:
#   ./scripts/build.sh                  Build all images
#   ./scripts/build.sh --production     Build only the production image
#   ./scripts/build.sh --test           Build production + test images
#   ./scripts/build.sh --no-cache       Build without Docker cache
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[build]${NC} $*"; }
ok()   { echo -e "${GREEN}[build]${NC} $*"; }
warn() { echo -e "${YELLOW}[build]${NC} $*"; }
err()  { echo -e "${RED}[build]${NC} $*" >&2; }

BUILD_PRODUCTION=true
BUILD_TEST=true
BUILD_MOCK=true
EXTRA_ARGS=()

for arg in "$@"; do
    case "$arg" in
        --production)
            BUILD_TEST=false
            BUILD_MOCK=false
            ;;
        --test)
            BUILD_MOCK=true
            BUILD_TEST=true
            ;;
        --no-cache)
            EXTRA_ARGS+=("--no-cache")
            ;;
        --help|-h)
            echo "Usage: $0 [--production|--test] [--no-cache]"
            echo ""
            echo "Options:"
            echo "  --production   Build only frostmirror:latest"
            echo "  --test         Build frostmirror:latest + frostmirror:test + mock-registry"
            echo "  --no-cache     Build without Docker layer cache"
            echo ""
            echo "With no flags, all three images are built."
            exit 0
            ;;
        *)
            err "Unknown option: $arg"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

# ── Production image ─────────────────────────────────────────────────
if [ "$BUILD_PRODUCTION" = true ]; then
    log "Building frostmirror:latest ..."
    docker build \
        -t frostmirror:latest \
        -f docker/Dockerfile \
        "${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}" \
        .
    ok "frostmirror:latest built"
fi

# ── Test image (same binary, tagged separately) ─────────────────────
if [ "$BUILD_TEST" = true ]; then
    log "Tagging frostmirror:test ..."
    docker tag frostmirror:latest frostmirror:test
    ok "frostmirror:test tagged"
fi

# ── Mock registry image ─────────────────────────────────────────────
if [ "$BUILD_MOCK" = true ]; then
    log "Building frostmirror-mock:latest ..."
    docker build \
        -t frostmirror-mock:latest \
        -f docker/Dockerfile.mock-registry \
        "${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}" \
        .
    ok "frostmirror-mock:latest built"
fi

# ── Summary ──────────────────────────────────────────────────────────
echo ""
ok "Build complete. Images:"
docker images --format "  {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.ID}}" \
    | grep -E "^  frostmirror" || true
