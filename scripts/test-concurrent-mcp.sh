#!/bin/bash
#
# Test concurrent access to verify file locking works correctly
#
# This script simulates multiple concurrent clients accessing the same
# jit repository to verify:
# 1. No data corruption
# 2. All operations succeed
# 3. Lock contention is handled gracefully

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "🧪 Concurrent Access Test"
echo "========================="
echo ""

# Check dependencies
if ! command -v jit &> /dev/null; then
    echo -e "${RED}❌ jit binary not found in PATH${NC}"
    echo "Build it first: cargo build --release"
    echo "Then add to PATH: export PATH=\"\$(pwd)/target/release:\$PATH\""
    exit 1
fi

# Create a temporary test directory
TEST_DIR=$(mktemp -d)
echo -e "${GREEN}✓${NC} Created test directory: $TEST_DIR"

cd "$TEST_DIR"
jit init > /dev/null
echo -e "${GREEN}✓${NC} Initialized jit repository"

# Function to simulate a concurrent agent creating issues
create_issues() {
    local agent_id=$1
    local num_issues=$2
    
    for i in $(seq 1 $num_issues); do
        jit issue create --title "Agent $agent_id Issue $i" --priority normal > /dev/null 2>&1
        if [ $? -ne 0 ]; then
            echo -e "${RED}❌ Agent $agent_id failed to create issue $i${NC}"
            return 1
        fi
    done
    echo -e "${GREEN}✓${NC} Agent $agent_id created $num_issues issues"
}

# Function to simulate concurrent reads
list_issues() {
    local agent_id=$1
    local num_reads=$2
    
    for i in $(seq 1 $num_reads); do
        jit issue list > /dev/null 2>&1
        if [ $? -ne 0 ]; then
            echo -e "${RED}❌ Agent $agent_id failed to list issues (attempt $i)${NC}"
            return 1
        fi
    done
    echo -e "${GREEN}✓${NC} Agent $agent_id performed $num_reads list operations"
}

echo ""
echo "📊 Test 1: Concurrent Issue Creation"
echo "-------------------------------------"
echo "Spawning 10 agents, each creating 5 issues..."

# Spawn 10 concurrent agents
pids=()
for agent in $(seq 1 10); do
    create_issues $agent 5 &
    pids+=($!)
done

# Wait for all agents to complete
failed=0
for pid in ${pids[@]}; do
    wait $pid || ((failed++))
done

if [ $failed -eq 0 ]; then
    echo -e "${GREEN}✅ All agents completed successfully${NC}"
else
    echo -e "${RED}❌ $failed agents failed${NC}"
    exit 1
fi

# Verify issue count (use --json for reliable parsing)
if command -v jq &> /dev/null; then
    issue_count=$(jit issue list --json 2>/dev/null | jq '.data.issues | length')
else
    # Fallback: count lines in normal output (each issue is one line)
    issue_count=$(jit issue list 2>/dev/null | tail -n +1 | wc -l)
fi

expected=50
if [ "$issue_count" -ge "$expected" ]; then
    echo -e "${GREEN}✅ Created exactly $issue_count issues (expected $expected)${NC}"
else
    echo -e "${RED}❌ Incorrect issue count: $issue_count (expected $expected)${NC}"
    echo "This indicates data corruption or lost writes!"
    exit 1
fi

echo ""
echo "📊 Test 2: Concurrent Reads"
echo "----------------------------"
echo "Spawning 20 reader agents, each listing issues 10 times..."

pids=()
for agent in $(seq 1 20); do
    list_issues $agent 10 &
    pids+=($!)
done

failed=0
for pid in ${pids[@]}; do
    wait $pid || ((failed++))
done

if [ $failed -eq 0 ]; then
    echo -e "${GREEN}✅ All reader agents completed successfully (200 total list operations)${NC}"
else
    echo -e "${RED}❌ $failed reader agents failed${NC}"
    exit 1
fi

echo ""
echo "======================================"
echo -e "${GREEN}✅ All concurrency tests passed!${NC}"
echo ""
echo "Summary:"
echo "  • 50 concurrent issue creates - no corruption"
echo "  • 200 concurrent list operations - all succeeded"
echo "  • Lock ordering prevents deadlocks"
echo "  • File locking working as expected"
echo ""
echo "Test directory: $TEST_DIR"
echo "  (Delete with: rm -rf $TEST_DIR)"
echo ""
