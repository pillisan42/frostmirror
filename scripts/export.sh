#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# export.sh — Export frostmirror Docker images to a portable tar archive
#
# This script saves one or more Docker images into a single compressed
# .tar.gz file that can be transferred to an air-gapped machine and
# loaded with import.sh.
#
# Usage:
#   ./scripts/export.sh                             Export all images
#   ./scripts/export.sh --production                Export only frostmirror:latest
#   ./scripts/export.sh --output /media/usb/        Write to a custom directory
#   ./scripts/export.sh --output /media/usb/fm.tar.gz   Write to an exact file
#
# Output:
#   frostmirror-images-YYYYMMDD-HHMM.tar.gz    (default in ./export/)
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[export]${NC} $*"; }
ok()   { echo -e "${GREEN}[export]${NC} $*"; }
warn() { echo -e "${YELLOW}[export]${NC} $*"; }
err()  { echo -e "${RED}[export]${NC} $*" >&2; }

MODE="all"
OUTPUT=""
IMAGES=()

for arg in "$@"; do
    case "$arg" in
        --production)
            MODE="production"
            ;;
        --test)
            MODE="test"
            ;;
        --output)
            # Next arg is the path — handled below
            ;;
        --help|-h)
            echo "Usage: $0 [--production|--test] [--output <path>]"
            echo ""
            echo "Modes:"
            echo "  (default)      Export frostmirror:latest + frostmirror:test + frostmirror-mock:latest"
            echo "  --production   Export only frostmirror:latest"
            echo "  --test         Export frostmirror:latest + frostmirror:test + frostmirror-mock:latest"
            echo ""
            echo "Output:"
            echo "  --output <dir>    Write the archive into this directory"
            echo "  --output <file>   Write to this exact file path"
            echo ""
            echo "The archive can be loaded on another machine with: ./scripts/import.sh <archive>"
            exit 0
            ;;
        *)
            # Treat as --output value if OUTPUT is empty and previous arg was --output
            OUTPUT="$arg"
            ;;
    esac
done

# Handle --output with separate value
if [ -z "$OUTPUT" ]; then
    # Check if --output was the second-to-last arg
    args=("$@")
    for i in "${!args[@]}"; do
        if [ "${args[$i]}" = "--output" ] && [ $((i + 1)) -lt ${#args[@]} ]; then
            OUTPUT="${args[$((i + 1))]}"
        fi
    done
fi

# ── Select images ────────────────────────────────────────────────────
case "$MODE" in
    production)
        IMAGES=("frostmirror:latest")
        ;;
    test|all)
        IMAGES=("frostmirror:latest" "frostmirror:test" "frostmirror-mock:latest")
        ;;
esac

# ── Verify images exist ─────────────────────────────────────────────
MISSING=()
for img in "${IMAGES[@]}"; do
    if ! docker image inspect "$img" > /dev/null 2>&1; then
        MISSING+=("$img")
    fi
done

if [ ${#MISSING[@]} -gt 0 ]; then
    err "The following images are not built yet:"
    for img in "${MISSING[@]}"; do
        err "  - $img"
    done
    echo ""
    err "Run ./scripts/build.sh first, then try again."
    exit 1
fi

# ── Determine output path ───────────────────────────────────────────
TIMESTAMP=$(date -u +"%Y%m%d-%H%M")

if [ -z "$OUTPUT" ]; then
    OUTPUT_DIR="$PROJECT_ROOT/export"
    OUTPUT_FILE="$OUTPUT_DIR/frostmirror-images-${TIMESTAMP}.tar.gz"
elif [ -d "$OUTPUT" ] || [[ "$OUTPUT" != *.tar* && "$OUTPUT" != *.tgz ]]; then
    # It's a directory
    OUTPUT_DIR="$OUTPUT"
    OUTPUT_FILE="$OUTPUT_DIR/frostmirror-images-${TIMESTAMP}.tar.gz"
else
    # It's a file path
    OUTPUT_DIR="$(dirname "$OUTPUT")"
    OUTPUT_FILE="$OUTPUT"
fi

mkdir -p "$OUTPUT_DIR"

# ── Export ───────────────────────────────────────────────────────────
log "Exporting ${#IMAGES[@]} image(s):"
for img in "${IMAGES[@]}"; do
    SIZE=$(docker image inspect "$img" --format '{{.Size}}' 2>/dev/null || echo "0")
    SIZE_MB=$(( SIZE / 1024 / 1024 ))
    log "  - $img  (${SIZE_MB} MB uncompressed)"
done

log "Saving to: $OUTPUT_FILE"
echo ""

docker save "${IMAGES[@]}" | gzip -6 > "$OUTPUT_FILE"

ARCHIVE_SIZE=$(stat -c%s "$OUTPUT_FILE" 2>/dev/null || stat -f%z "$OUTPUT_FILE" 2>/dev/null)
ARCHIVE_MB=$(( ARCHIVE_SIZE / 1024 / 1024 ))

echo ""
ok "Export complete!"
ok "  Archive:  $OUTPUT_FILE"
ok "  Size:     ${ARCHIVE_MB} MB"
ok "  Images:   ${IMAGES[*]}"
echo ""
ok "Transfer this file to the target machine and run:"
ok "  ./scripts/import.sh $OUTPUT_FILE"
