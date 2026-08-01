#!/usr/bin/env bash
# ==============================================================================
# cjson-rs — Split-Screen Terminal GUI Launcher
# Hackathon: Port Mortem 2026 (Code Resurrection — C -> Rust Track)
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo ""
echo "===================================================================="
echo "  Launching cjson-rs Split-Screen Terminal GUI..."
echo "  Left Panel: ANSI C (cJSON.c)  |  Right Panel: Safe Rust (cjson-rs)"
echo "===================================================================="

# Check if Python 3 is available
if ! command -v python3 &> /dev/null; then
    echo "[ERROR] python3 is required to run the local HTTP server."
    echo "You can still open gui/index.html directly in any web browser!"
    exit 1
fi

# Try opening the browser in the background after 1 second
(
    sleep 1
    URL="http://localhost:8080"
    if command -v open &> /dev/null; then
        open "$URL" 2>/dev/null || true
    elif command -v xdg-open &> /dev/null; then
        xdg-open "$URL" 2>/dev/null || true
    else
        echo "Please open $URL in your web browser."
    fi
) &

# Launch Python server
exec python3 gui/server.py
