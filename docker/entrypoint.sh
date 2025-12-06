#!/bin/sh
set -e

# Initialize JIT repository if not exists
if [ ! -d "$JIT_DATA_DIR/.jit" ]; then
    echo "Initializing JIT repository at $JIT_DATA_DIR..."
    cd "$JIT_DATA_DIR"
    jit init
fi

case "$1" in
    all)
        echo "Starting all services..."
        # Start nginx in background
        nginx
        # Start API server in background
        jit-server &
        # Keep container running
        tail -f /dev/null
        ;;
    api)
        echo "Starting API server..."
        exec jit-server
        ;;
    web)
        echo "Starting Web UI (nginx)..."
        exec nginx -g 'daemon off;'
        ;;
    cli)
        shift
        exec jit "$@"
        ;;
    *)
        exec "$@"
        ;;
esac
