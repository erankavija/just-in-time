#!/bin/bash
set -e

# Manual CI Testing Script
# Runs the same checks as GitHub Actions CI without needing act

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

echo "🧪 Manual CI Testing (same checks as GitHub Actions)"
echo "===================================================="
echo ""

# Test 1: Rust formatting
echo "1️⃣  Checking Rust formatting..."
cargo fmt --all -- --check
echo "✅ Rust formatting OK"
echo ""

# Test 2: Clippy lints
echo "2️⃣  Running Clippy (zero warnings policy)..."
cargo clippy --all-targets --all-features -- -D warnings
echo "✅ Clippy OK"
echo ""

# Test 3: Rust build
echo "3️⃣  Building Rust workspace..."
cargo build --all-features --verbose
echo "✅ Rust build OK"
echo ""

# Test 4: Rust tests
echo "4️⃣  Running Rust tests..."
cargo test --all-features --verbose
echo "✅ Rust tests OK (490+ tests)"
echo ""

# Test 5: MCP Server tests
echo "5️⃣  Testing MCP Server..."
(cd mcp-server && npm test)
echo "✅ MCP Server tests OK"
echo ""

# Test 6: Web UI linting
echo "6️⃣  Linting Web UI..."
(cd web && npm run lint)
echo "✅ Web UI linting OK"
echo ""

# Test 7: Web UI tests
echo "7️⃣  Running Web UI tests..."
(cd web && npm test)
echo "✅ Web UI tests OK"
echo ""

# Test 8: Web UI build
echo "8️⃣  Building Web UI..."
(cd web && npm run build)
echo "✅ Web UI build OK"
echo ""

# Test 9: Security audit (Rust)
echo "9️⃣  Running cargo audit..."
if ! command -v cargo-audit &> /dev/null; then
    echo "⚠️  cargo-audit not installed, skipping..."
    echo "   Install with: cargo install cargo-audit"
else
    cargo audit
    echo "✅ Cargo audit OK"
fi
echo ""

# Test 10: Security audit (npm - MCP)
echo "🔟 Running npm audit (MCP Server)..."
cd mcp-server
npm audit --audit-level=moderate || echo "⚠️  Vulnerabilities found (check manually)"
cd ..
echo ""

# Test 11: Security audit (npm - Web)
echo "1️⃣1️⃣  Running npm audit (Web UI)..."
cd web
npm audit --audit-level=moderate || echo "⚠️  Vulnerabilities found (check manually)"
cd ..
echo ""

echo "=================================================="
echo "✅ All CI checks complete!"
echo ""
echo "Summary:"
echo "  - Rust: fmt ✅, clippy ✅, build ✅, tests ✅"
echo "  - MCP Server: tests ✅, audit ⚠️"
echo "  - Web UI: lint ✅, tests ✅, build ✅, audit ⚠️"
echo ""
echo "Ready to push to GitHub! 🚀"
