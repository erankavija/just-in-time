#!/bin/bash
set -e

# Local CI Testing Script
# Tests GitHub Actions workflows locally using act + podman

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🧪 Testing GitHub Actions locally with act + podman"
echo "=================================================="

# Start podman socket if not running
PODMAN_SOCK="/tmp/podman-ci.sock"
if [ ! -S "$PODMAN_SOCK" ]; then
    echo "Starting Podman socket at $PODMAN_SOCK..."
    podman system service --time=0 "unix://$PODMAN_SOCK" &
    PODMAN_PID=$!
    sleep 2
fi

export DOCKER_HOST="unix://$PODMAN_SOCK"

cd "$PROJECT_ROOT"

# Create act config if it doesn't exist
if [ ! -f ~/.config/act/actrc ]; then
    mkdir -p ~/.config/act
    cat > ~/.config/act/actrc <<EOF
-P ubuntu-latest=catthehacker/ubuntu:act-latest
--container-daemon-socket $PODMAN_SOCK
EOF
    echo "Created act config at ~/.config/act/actrc"
fi

echo ""
echo "Available workflows:"
act -l

echo ""
echo "Choose a test to run:"
echo "  1) Dry run - Show what would execute (fast)"
echo "  2) Test Rust components only"
echo "  3) Test MCP Server only"
echo "  4) Test Web UI only"
echo "  5) Run all CI jobs (slow, ~10-15 min)"
echo "  6) Exit"
echo ""
read -p "Enter choice [1-6]: " choice

case $choice in
    1)
        echo "Running dry run..."
        act -j test-rust --dryrun
        ;;
    2)
        echo "Testing Rust components..."
        echo "Note: This will take 5-10 minutes on first run (downloads images)"
        act -j test-rust
        ;;
    3)
        echo "Testing MCP Server..."
        act -j test-mcp-server
        ;;
    4)
        echo "Testing Web UI..."
        act -j test-web-ui
        ;;
    5)
        echo "Running all CI jobs..."
        echo "This will take 10-15 minutes. Press Ctrl+C to cancel."
        sleep 3
        act -W .github/workflows/ci.yml
        ;;
    6)
        echo "Exiting..."
        exit 0
        ;;
    *)
        echo "Invalid choice"
        exit 1
        ;;
esac

# Cleanup
if [ -n "$PODMAN_PID" ]; then
    echo ""
    echo "Stopping Podman socket..."
    kill $PODMAN_PID 2>/dev/null || true
fi

echo ""
echo "✅ Local CI test complete!"
