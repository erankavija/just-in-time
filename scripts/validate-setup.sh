#!/bin/bash
# Validation script for CI/CD setup

set -e

echo "🔍 Validating CI/CD Setup..."
echo

# Check files exist
echo "📁 Checking required files..."
files=(
    ".github/workflows/ci.yml"
    ".github/workflows/release.yml"
    ".github/workflows/docker.yml"
    "Dockerfile"
    "docker-compose.yml"
    ".dockerignore"
    "docker/Dockerfile.api"
    "docker/Dockerfile.web"
    "docker/Dockerfile.cli"
    "docker/nginx.conf"
    "docker/entrypoint.sh"
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
echo "🐳 Checking Docker setup..."
if command -v docker &> /dev/null; then
    echo "  ✅ Docker is installed"
    
    # Validate docker-compose.yml syntax
    if docker-compose config > /dev/null 2>&1; then
        echo "  ✅ docker-compose.yml is valid"
    else
        echo "  ⚠️  docker-compose validation failed (may need newer version)"
    fi
else
    echo "  ⚠️  Docker not installed (optional for development)"
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
echo "  4. Create release: tag v<product-version> from" \
    "crates/jit/Cargo.toml, then git push --tags"
echo "  5. Test Docker: docker-compose up -d"
echo
