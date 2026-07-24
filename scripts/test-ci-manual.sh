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
# Audits fail closed: a missing tool or reported vulnerabilities are CI
# failures, not warnings (@/charter/D-11). Failures are collected so every
# audit still runs and reports, but the script exits non-zero afterward
# and never prints the success summary.
AUDIT_FAILED=0

echo "9️⃣  Running cargo audit..."
if ! command -v cargo-audit &> /dev/null; then
    echo "❌ cargo-audit not installed (required for security audit)"
    echo "   Install with: cargo install cargo-audit"
    AUDIT_FAILED=1
elif cargo audit; then
    echo "✅ Cargo audit OK"
else
    echo "❌ Cargo audit found issues"
    AUDIT_FAILED=1
fi
echo ""

# Test 10: Security audit (npm - MCP)
echo "🔟 Running npm audit (MCP Server)..."
cd mcp-server
if npm audit --audit-level=low; then
    echo "✅ npm audit (MCP Server) OK"
else
    echo "❌ npm audit (MCP Server) found vulnerabilities"
    AUDIT_FAILED=1
fi
cd ..
echo ""

# Test 11: Security audit (npm - Web)
echo "1️⃣1️⃣  Running npm audit (Web UI)..."
cd web
if npm audit --audit-level=low; then
    echo "✅ npm audit (Web UI) OK"
else
    echo "❌ npm audit (Web UI) found vulnerabilities"
    AUDIT_FAILED=1
fi
cd ..
echo ""

if [ "$AUDIT_FAILED" -ne 0 ]; then
    echo "=================================================="
    echo "❌ CI checks failed: security audit missing or failing"
    echo ""
    exit 1
fi

echo "=================================================="
echo "✅ All CI checks complete!"
echo ""
echo "Summary:"
echo "  - Rust: fmt ✅, clippy ✅, build ✅, tests ✅, audit ✅"
echo "  - MCP Server: tests ✅, audit ✅"
echo "  - Web UI: lint ✅, tests ✅, build ✅, audit ✅"
echo ""
echo "Ready to push to GitHub! 🚀"
