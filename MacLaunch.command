#!/bin/bash
# facts. launcher for macOS. All logic lives in .system/facts-macos (source: facts-rs/).
ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
SYSTEM_DIR="$ROOT_DIR/.system"

# Kill an engine left running by a crashed session.
killall llama-server 2>/dev/null

# Unsigned files from a download are quarantined by Gatekeeper; clear that and make them runnable.
xattr -r -d com.apple.quarantine "$SYSTEM_DIR" 2>/dev/null
chmod -R +x "$SYSTEM_DIR" 2>/dev/null

printf "\033]0;facts.\007"
clear
if [ ! -x "$SYSTEM_DIR/facts-macos" ]; then
    echo ""
    echo "  [ERROR] .system/facts-macos is missing. Your drive may be corrupted."
    echo "  Need help? Visit opensourceeverything.io"
    echo ""
    read -p "  Press Enter to exit..."
    exit 1
fi
"$SYSTEM_DIR/facts-macos"
