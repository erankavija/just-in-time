#!/bin/bash
# Validation script for CI/CD setup

set -e

echo "🔍 Validating CI/CD Setup..."
echo

# Check files exist
echo "📁 Checking required files..."
files=(
    ".github/workflows/ci.yml"
    ".github/workflows/release-publish.yml"
    ".github/workflows/release-artifacts.yml"
    ".github/workflows/docker.yml"
    "Dockerfile"
    "docker-compose.yml"
    ".dockerignore"
    "test-vectors/container-image/test_image_contract.py"
    "scripts/test-podman.sh"
    "INSTALL.md"
    "docs/how-to/deployment.md"
)

for file in "${files[@]}"; do
    if [ -f "$file" ]; then
        echo "  ✅ $file"
    else
        echo "  ❌ $file MISSING"
        exit 1
    fi
done

echo
echo "🔧 Checking Rust workspace..."
if cargo build --workspace --quiet 2>/dev/null; then
    echo "  ✅ Rust workspace builds successfully"
else
    echo "  ❌ Rust build failed"
    exit 1
fi

echo
echo "🌐 Checking MCP server..."
if [ -f "mcp-server/package.json" ]; then
    echo "  ✅ MCP server package.json found"
else
    echo "  ❌ MCP server package.json missing"
    exit 1
fi

echo
echo "⚛️  Checking Web UI..."
if [ -f "web/package.json" ]; then
    echo "  ✅ Web UI package.json found"
else
    echo "  ❌ Web UI package.json missing"
    exit 1
fi

echo
echo "🐳 Checking the container setup..."

# The Compose service maps JIT_UID and JIT_GID onto the container user. Absent
# them the image runs as its fixed 10001:10001 default, which reaches the
# repository mount only when that identity may read, write, and search it, so
# a deployment derives the two values from the repository's owner instead.
JIT_REPO="${JIT_REPO:-$PWD}"
JIT_UID="$(stat -c '%u' "$JIT_REPO")"
JIT_GID="$(stat -c '%g' "$JIT_REPO")"
export JIT_REPO JIT_UID JIT_GID
echo "  ℹ️  Repository mount: $JIT_REPO owned by JIT_UID=$JIT_UID JIT_GID=$JIT_GID"

if docker compose version &> /dev/null; then
    docker compose config > /dev/null
    echo "  ✅ docker-compose.yml resolves under docker compose with the derived mount identity"
elif command -v podman-compose &> /dev/null; then
    podman-compose config > /dev/null
    echo "  ✅ docker-compose.yml resolves under podman-compose with the derived mount identity"
else
    echo "  ℹ️  No Compose CLI on PATH; skipping the docker-compose.yml resolution check"
fi

echo
echo "📝 Checking documentation..."
docs=("INSTALL.md" "docs/how-to/deployment.md" "README.md")
for doc in "${docs[@]}"; do
    if [ -f "$doc" ]; then
        lines=$(wc -l < "$doc")
        echo "  ✅ $doc ($lines lines)"
    else
        echo "  ❌ $doc missing"
        exit 1
    fi
done

echo
echo "✨ All validations passed!"
echo
echo "📋 Next steps:"
echo "  1. git add ."
echo "  2. git commit -m 'Add CI/CD pipeline and Docker support'"
echo "  3. git push"
echo "  4. Create release: git tag -a v<product-version> from" \
    "crates/jit/Cargo.toml, then git push --tags"
echo "  5. Serve a repository: JIT_REPO=<repo> JIT_UID=\$(stat -c '%u' <repo>)" \
    "JIT_GID=\$(stat -c '%g' <repo>) docker compose up -d"
echo "  6. Smoke the image with Podman: ./scripts/test-podman.sh"
echo
