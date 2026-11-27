#!/bin/bash
# facts. launcher for Linux. All logic lives in .system/facts-linux (source: facts-rs/).
ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
SYSTEM_DIR="$ROOT_DIR/.system"

# Kill an engine left running by a crashed session.
pkill -x llama-server 2>/dev/null

# exFAT drives often lose the executable bit.
chmod +x "$SYSTEM_DIR/facts-linux" "$SYSTEM_DIR"/linux/llama-* 2>/dev/null

printf "\033]0;facts.\007"
clear
if [ ! -x "$SYSTEM_DIR/facts-linux" ]; then
    echo ""
    echo "  [ERROR] .system/facts-linux is missing. Your drive may be corrupted."
    echo "  Need help? Visit opensourceeverything.io and use the support chat."
    echo ""
    read -p "  Press Enter to exit..."
    exit 1
fi
"$SYSTEM_DIR/facts-linux"
read -p "  Press Enter to close..."
