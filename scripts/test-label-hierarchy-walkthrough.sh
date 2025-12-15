#!/usr/bin/env bash
# Manual walkthrough test for label hierarchy feature
# This script simulates a new user creating a milestone -> epic -> tasks hierarchy

set -e

echo "==================================================================="
echo "Label Hierarchy Feature - Manual Walkthrough Test"
echo "==================================================================="
echo ""

# Setup test directory
TEST_DIR="/tmp/jit-label-test-$(date +%s)"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

echo "Test directory: $TEST_DIR"
echo ""

# Initialize repository
echo "STEP 1: Initialize repository"
echo "$ jit init"
jit init
echo "✓ Repository initialized"
echo ""

# Check label namespaces
echo "STEP 2: Check default label namespaces"
echo "$ jit label namespaces"
jit label namespaces
echo ""

# Create milestone
echo "STEP 3: Create milestone for v1.0 release"
echo '$ jit issue create -t "Release v1.0" --label "type:milestone" --label "milestone:v1.0" --priority critical'
MILESTONE_ID=$(jit issue create -t "Release v1.0" --label "type:milestone" --label "milestone:v1.0" --priority critical | awk '{print $NF}')
echo "✓ Created milestone: $MILESTONE_ID"
echo ""

# Create epic
echo "STEP 4: Create epic for authentication system"
echo '$ jit issue create -t "Authentication System" --label "type:epic" --label "epic:auth" --label "milestone:v1.0" --priority high'
EPIC_ID=$(jit issue create -t "Authentication System" --label "type:epic" --label "epic:auth" --label "milestone:v1.0" --priority high | awk '{print $NF}')
echo "✓ Created epic: $EPIC_ID"
echo ""

# Create tasks
echo "STEP 5: Create tasks for the epic"
echo '$ jit issue create -t "Implement login endpoint" --label "type:task" --label "epic:auth" --label "component:backend" --priority high'
TASK1_ID=$(jit issue create -t "Implement login endpoint" --label "type:task" --label "epic:auth" --label "component:backend" --priority high | awk '{print $NF}')
echo "✓ Created task 1: $TASK1_ID"

echo '$ jit issue create -t "Add password hashing" --label "type:task" --label "epic:auth" --label "component:backend" --priority high'
TASK2_ID=$(jit issue create -t "Add password hashing" --label "type:task" --label "epic:auth" --label "component:backend" --priority high | awk '{print $NF}')
echo "✓ Created task 2: $TASK2_ID"

echo '$ jit issue create -t "Create login UI" --label "type:task" --label "epic:auth" --label "component:frontend" --priority normal'
TASK3_ID=$(jit issue create -t "Create login UI" --label "type:task" --label "epic:auth" --label "component:frontend" --priority normal | awk '{print $NF}')
echo "✓ Created task 3: $TASK3_ID"
echo ""

# Add dependencies
echo "STEP 6: Add dependencies (milestone -> epic -> tasks)"
echo "$ jit dep add $EPIC_ID $TASK1_ID"
jit dep add "$EPIC_ID" "$TASK1_ID"
echo "$ jit dep add $EPIC_ID $TASK2_ID"
jit dep add "$EPIC_ID" "$TASK2_ID"
echo "$ jit dep add $EPIC_ID $TASK3_ID"
jit dep add "$EPIC_ID" "$TASK3_ID"
echo "$ jit dep add $MILESTONE_ID $EPIC_ID"
jit dep add "$MILESTONE_ID" "$EPIC_ID"
echo "✓ Dependencies added"
echo ""

# Query by label - exact match
echo "STEP 7: Query by label (exact match: milestone:v1.0)"
echo "$ jit query label 'milestone:v1.0'"
jit query label "milestone:v1.0"
echo ""

# Query by label - wildcard
echo "STEP 8: Query by label (wildcard: epic:*)"
echo "$ jit query label 'epic:*'"
jit query label "epic:*"
echo ""

# Query by component
echo "STEP 9: Query by component (component:backend)"
echo "$ jit query label 'component:backend'"
jit query label "component:backend"
echo ""

# Strategic view
echo "STEP 10: Query strategic issues (issues with strategic namespace labels)"
echo "$ jit query strategic"
jit query strategic
echo ""

# Validate repository
echo "STEP 11: Validate repository integrity"
echo "$ jit validate"
jit validate
echo ""

# Show issue with labels
echo "STEP 12: Show epic details with labels"
echo "$ jit issue show $EPIC_ID"
jit issue show "$EPIC_ID"
echo ""

# List label values
echo "STEP 13: List values for epic namespace"
echo "$ jit label values epic"
jit label values epic
echo ""

# Check ready/blocked status
echo "STEP 14: Check ready and blocked issues"
echo "$ jit query ready"
echo "Ready issues (no blocking dependencies):"
jit query ready
echo ""
echo "$ jit query blocked"
echo "Blocked issues (have blocking dependencies):"
jit query blocked
echo ""

# Complete workflow
echo "STEP 15: Complete workflow - mark tasks as done"
echo "$ jit issue update $TASK1_ID --state done"
jit issue update "$TASK1_ID" --state done
echo "$ jit issue update $TASK2_ID --state done"
jit issue update "$TASK2_ID" --state done
echo "$ jit issue update $TASK3_ID --state done"
jit issue update "$TASK3_ID" --state done
echo "✓ All tasks completed"
echo ""

echo "STEP 16: Check if epic is now unblocked"
echo "$ jit query ready"
jit query ready
echo ""

# Graph visualization
echo "STEP 17: Visualize dependency graph"
echo "$ jit graph show"
jit graph show
echo ""

# JSON output
echo "STEP 18: Test JSON output for automation"
echo "$ jit query strategic --json | jq '. | length'"
STRATEGIC_COUNT=$(jit query strategic --json | jq '. | length')
echo "Strategic issues count: $STRATEGIC_COUNT"
echo ""

echo "==================================================================="
echo "✓ Manual walkthrough completed successfully!"
echo "==================================================================="
echo ""
echo "Summary:"
echo "  • Created 1 milestone, 1 epic, 3 tasks"
echo "  • Added labels for organization (milestone, epic, component)"
echo "  • Established dependency chain"
echo "  • Queried by labels (exact and wildcard)"
echo "  • Used strategic view to filter"
echo "  • Validated repository integrity"
echo "  • Completed tasks and verified unblocking"
echo "  • Tested JSON output"
echo ""
echo "Test directory: $TEST_DIR"
echo "To inspect: cd $TEST_DIR"
echo ""
