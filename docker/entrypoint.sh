#!/bin/bash
set -e

CMD="${1:-serve}"
shift 2>/dev/null || true

case "$CMD" in
    fetch)
        MODE="${FROSTMIRROR_MODE:-full}"
        ARGS=""
        if [ "$MODE" = "incremental" ]; then
            ARGS="--incremental"
        fi
        exec frostmirror fetch \
            --config "${FROSTMIRROR_CONFIG:-/config/depends.toml}" \
            --output "${FROSTMIRROR_OUTPUT:-/pkgs}" \
            $ARGS "$@"
        ;;
    import)
        exec frostmirror import "$@"
        ;;
    serve)
        exec frostmirror serve \
            --bind "${FROSTMIRROR_BIND:-0.0.0.0:8080}" \
            --base-url "${FROSTMIRROR_BASE_URL:-http://localhost:8080}" \
            --mirror "${FROSTMIRROR_MIRROR:-/mirror}" \
            --incoming "${FROSTMIRROR_INCOMING:-/incoming}" \
            "$@"
        ;;
    verify)
        exec frostmirror verify "$@"
        ;;
    gc)
        exec frostmirror gc \
            --mirror "${FROSTMIRROR_MIRROR:-/mirror}" \
            "$@"
        ;;
    status)
        exec frostmirror status \
            --mirror "${FROSTMIRROR_MIRROR:-/mirror}" \
            "$@"
        ;;
    *)
        echo "Unknown command: $CMD"
        echo "Usage: entrypoint.sh {fetch|import|serve|verify|gc|status}"
        exit 1
        ;;
esac
