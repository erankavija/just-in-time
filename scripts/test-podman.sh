#!/bin/bash
# Test script for Podman deployment

set -e

echo "🐳 Testing JIT with Podman..."
echo "=============================="
echo

# Check Podman is available
if ! command -v podman &> /dev/null; then
    echo "❌ Podman not found. Please install: sudo apt install podman"
    exit 1
fi

echo "✅ Podman version: $(podman --version)"
echo

# Build images
echo "🔨 Building Docker images with Podman..."
echo

echo "1️⃣  Building CLI image..."
podman build -t jit-cli:test -f docker/Dockerfile.cli .
echo "   ✅ CLI image built"
echo

echo "2️⃣  Building API server image..."
podman build -t jit-api:test -f docker/Dockerfile.api .
echo "   ✅ API image built"
echo

echo "3️⃣  Building Web UI image..."
podman build -t jit-web:test -f docker/Dockerfile.web .
echo "   ✅ Web image built"
echo

# List images
echo "📦 Built images:"
podman images | grep "jit-.*:test"
echo

# Test CLI
echo "🧪 Testing CLI..."
podman run --rm jit-cli:test --version
echo "   ✅ CLI works"
echo

# Create pod for API + Web
echo "🚀 Creating pod with API + Web..."
POD_NAME="jit-test-pod"

# Stop and remove existing pod if it exists
podman pod exists $POD_NAME 2>/dev/null && podman pod rm -f $POD_NAME

# Create pod with port mappings
podman pod create --name $POD_NAME -p 3000:3000 -p 8080:80
echo "   ✅ Pod created: $POD_NAME"
echo

# Start API server in pod
echo "🔧 Starting API server..."
podman run -d --pod $POD_NAME \
  --name jit-api-test \
  -v jit-test-data:/data:z \
  -e JIT_DATA_DIR=/data \
  jit-api:test
echo "   ✅ API server started"
echo

# Wait for API to be ready
echo "⏳ Waiting for API to be ready..."
for i in {1..30}; do
    if curl -s http://localhost:3000/api/health > /dev/null 2>&1; then
        echo "   ✅ API is responding"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "   ❌ API didn't start in time"
        podman logs jit-api-test
        exit 1
    fi
    sleep 1
done
echo

# Start Web UI in pod
echo "🌐 Starting Web UI..."
podman run -d --pod $POD_NAME \
  --name jit-web-test \
  jit-web:test
echo "   ✅ Web UI started"
echo

# Test API endpoint
echo "🧪 Testing API endpoints..."
echo -n "   Health check: "
curl -s http://localhost:3000/api/health | grep -q "ok" && echo "✅" || echo "❌"

echo -n "   Status endpoint: "
curl -s http://localhost:3000/api/status > /dev/null && echo "✅" || echo "❌"

echo -n "   Issues endpoint: "
curl -s http://localhost:3000/api/issues > /dev/null && echo "✅" || echo "❌"
echo

# Test Web UI
echo "🧪 Testing Web UI..."
echo -n "   Homepage loads: "
curl -s http://localhost:8080/ | grep -q "<!DOCTYPE html>" && echo "✅" || echo "❌"
echo

# Show running containers
echo "📊 Running containers in pod:"
podman pod ps
echo
podman ps --pod
echo

# Show logs sample
echo "📋 API Server logs (last 10 lines):"
podman logs --tail 10 jit-api-test
echo

echo "✨ All tests passed!"
echo
echo "📌 Services running:"
echo "   API:    http://localhost:3000"
echo "   Web UI: http://localhost:8080"
echo
echo "📋 Management commands:"
echo "   View logs:     podman logs -f jit-api-test"
echo "   CLI command:   podman run --rm -v jit-test-data:/data:z jit-cli:test issue list"
echo "   Stop all:      podman pod stop $POD_NAME"
echo "   Remove all:    podman pod rm -f $POD_NAME"
echo "   Remove volume: podman volume rm jit-test-data"
echo
