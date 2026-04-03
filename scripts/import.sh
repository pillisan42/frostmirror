#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# import.sh — Import frostmirror Docker images from a portable tar archive
#
# Loads images exported by export.sh into the local Docker daemon.
# After loading, the images are ready for use with docker compose.
#
# Usage:
#   ./scripts/import.sh <archive>
#   ./scripts/import.sh frostmirror-images-20260402-2130.tar.gz
#   ./scripts/import.sh /media/usb/frostmirror-images-20260402-2130.tar.gz
#
# After import:
#   docker compose -f compose.airgap.yml up -d       # start the registry
#   docker compose -f compose.fetch.yml run --rm fetch  # run a fetch
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[import]${NC} $*"; }
ok()   { echo -e "${GREEN}[import]${NC} $*"; }
warn() { echo -e "${YELLOW}[import]${NC} $*"; }
err()  { echo -e "${RED}[import]${NC} $*" >&2; }

# ── Argument parsing ─────────────────────────────────────────────────
if [ $# -lt 1 ] || [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
    echo "Usage: $0 <archive>"
    echo ""
    echo "Loads frostmirror Docker images from a .tar.gz archive into"
    echo "the local Docker daemon."
    echo ""
    echo "Arguments:"
    echo "  <archive>   Path to the .tar.gz file created by export.sh"
    echo ""
    echo "Examples:"
    echo "  $0 frostmirror-images-20260402-2130.tar.gz"
    echo "  $0 /media/usb/frostmirror-images-20260402-2130.tar.gz"
    echo ""
    echo "After import, start the air-gapped registry with:"
    echo "  docker compose -f compose.airgap.yml up -d"
    exit 0
fi

ARCHIVE="$1"

# ── Validate input ───────────────────────────────────────────────────
if [ ! -f "$ARCHIVE" ]; then
    err "File not found: $ARCHIVE"
    exit 1
fi

ARCHIVE_SIZE=$(stat -c%s "$ARCHIVE" 2>/dev/null || stat -f%z "$ARCHIVE" 2>/dev/null)
ARCHIVE_MB=$(( ARCHIVE_SIZE / 1024 / 1024 ))

if [ "$ARCHIVE_SIZE" -lt 1024 ]; then
    err "Archive is suspiciously small (${ARCHIVE_SIZE} bytes). Is this the right file?"
    exit 1
fi

# ── Check Docker daemon ──────────────────────────────────────────────
if ! docker info > /dev/null 2>&1; then
    err "Docker daemon is not running or not accessible."
    err "Start Docker and try again."
    exit 1
fi

# ── Record existing images for diff ──────────────────────────────────
BEFORE=$(docker images --format '{{.Repository}}:{{.Tag}} {{.ID}}' \
    | grep -E "^frostmirror" | sort || true)

# ── Import ───────────────────────────────────────────────────────────
log "Loading images from: $ARCHIVE (${ARCHIVE_MB} MB)"
echo ""

gunzip -c "$ARCHIVE" | docker load

echo ""

# ── Show what was loaded ─────────────────────────────────────────────
AFTER=$(docker images --format '{{.Repository}}:{{.Tag}} {{.ID}}' \
    | grep -E "^frostmirror" | sort || true)

ok "Import complete! Loaded images:"
echo ""
docker images --format "  {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}" \
    | grep -E "^  frostmirror" || true

# ── Show newly added/updated images ─────────────────────────────────
if [ "$BEFORE" != "$AFTER" ]; then
    echo ""
    DIFF=$(comm -13 <(echo "$BEFORE") <(echo "$AFTER") | awk '{print $1}')
    if [ -n "$DIFF" ]; then
        ok "New or updated:"
        while IFS= read -r img; do
            ok "  + $img"
        done <<< "$DIFF"
    fi
fi

# ── Next steps ───────────────────────────────────────────────────────
echo ""
ok "Next steps:"
echo ""
echo "  # Start the air-gapped registry (offline machine)"
echo "  docker compose -f compose.airgap.yml up -d"
echo ""
echo "  # Or run a fetch (online machine)"
echo "  docker compose -f compose.fetch.yml run --rm fetch"
echo ""
echo "  # Or run integration tests"
echo "  docker compose -f compose.test.yml run --rm test-runner"
