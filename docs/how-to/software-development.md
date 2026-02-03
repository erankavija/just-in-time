# How-To: Software Development

> **Status:** Draft - Story 5326b331  
> **Diátaxis Type:** How-To Guide

Practical recipes for common software development workflows using JIT.

## Use JIT for Feature Development

### Recipe: Build a Feature with Task Breakdown

**When to use:** Implementing a multi-step feature that requires coordinated work.

**Step 1: Create the epic**

```bash
# Create epic for the feature
EPIC_ID=$(jit issue create \
  --title "User Authentication System" \
  --description "Implement JWT-based authentication with login/logout" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --json | jq -r '.id')

echo "Created epic: $EPIC_ID"
```

**Step 2: Break down into tasks**

```bash
# Task 1: JWT token generation
TASK1=$(jit issue create \
  --title "JWT token generation and validation" \
  --description "Implement JWT signing and verification logic" \
  --label "type:task" \
  --label "epic:auth" \
  --label "component:backend" \
  --gate tests \
  --gate code-review \
  --priority high \
  --json | jq -r '.id')

# Task 2: Login endpoint
TASK2=$(jit issue create \
  --title "POST /auth/login endpoint" \
  --description "Accept credentials, validate, return JWT" \
  --label "type:task" \
  --label "epic:auth" \
  --label "component:backend" \
  --gate tests \
  --gate code-review \
  --priority high \
  --json | jq -r '.id')

# Task 3: Logout endpoint
TASK3=$(jit issue create \
  --title "POST /auth/logout endpoint" \
  --description "Invalidate JWT token" \
  --label "type:task" \
  --label "epic:auth" \
  --label "component:backend" \
  --gate tests \
  --priority normal \
  --json | jq -r '.id')
```

**Step 3: Model dependencies**

```bash
# Login depends on JWT token logic
jit dep add $TASK2 $TASK1

# Logout depends on login (reuses token validation)
jit dep add $TASK3 $TASK2

# Check what's ready to start
jit query available --filter "labels.epic:auth"
# Only TASK1 shows (others blocked)
```

**Step 4: Track progress**

```bash
# View the epic's dependency tree
jit graph show $EPIC_ID

# Check overall epic progress
jit query all --filter "labels.epic:auth" | grep -E "(ready|in_progress|done)"

# Find what's blocking
jit query blocked --filter "labels.epic:auth"
```

### When to Use Epics vs Standalone Tasks

**Use epics when:**
- Feature requires 3+ coordinated tasks
- Work spans multiple components (frontend + backend)
- Need to track feature progress as a unit
- Multiple developers working on same feature

**Use standalone tasks when:**
- Single focused change (bug fix, small enhancement)
- Work is independent of other tasks
- Quick iteration needed

### Querying Epic Progress

```bash
# All work for auth epic
jit query all --filter "labels.epic:auth"

# Ready work in auth epic
jit query available --filter "labels.epic:auth"

# Completed auth tasks
jit query all --state done --filter "labels.epic:auth"
```

## Implement TDD Workflow with Gates

### Recipe: Enforce Test-Driven Development

**Step 1: Define TDD gates**

```bash
# Precheck: Reminder to write tests first
jit gate define tdd-reminder \
  --title "Write Tests First" \
  --description "Did you write tests before implementation?" \
  --stage precheck \
  --mode manual

# Postcheck: Automated test execution
jit gate define tests \
  --title "All Tests Pass" \
  --description "Run full test suite" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

# Postcheck: Code formatting
jit gate define fmt \
  --title "Code Formatted" \
  --description "Rustfmt check" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo fmt -- --check"

# Postcheck: Linter
jit gate define clippy \
  --title "Clippy Lints Pass" \
  --description "No clippy warnings" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy -- -D warnings"

# Manual: Code review
jit gate define code-review \
  --title "Code Review" \
  --description "Peer review completed" \
  --stage postcheck \
  --mode manual
```

**Step 2: Apply gates to your work**

```bash
# Create issue with all TDD gates
TASK=$(jit issue create \
  --title "Implement user registration" \
  --gate tdd-reminder \
  --gate tests \
  --gate fmt \
  --gate clippy \
  --gate code-review \
  --priority high \
  --json | jq -r '.id')
```

**Step 3: Follow TDD workflow**

```bash
# 1. Claim the issue
jit issue claim $TASK agent:developer-1

# 2. Write tests FIRST (precheck reminder)
# ... write failing tests ...
jit gate pass $TASK tdd-reminder --by agent:developer-1

# 3. Implement code to make tests pass
# ... write implementation ...

# 4. Run tests (manual verification before marking done)
cargo test --lib
# If pass, continue. If fail, fix and repeat.

# 5. Mark as done - triggers automated postchecks
jit issue update $TASK --state done

# Gates auto-run:
# - tests: cargo test --lib
# - fmt: cargo fmt -- --check
# - clippy: cargo clippy -- -D warnings

# 6. Manual code review
jit gate pass $TASK code-review --by agent:reviewer

# Check final status
jit issue show $TASK
```

### Understanding Gate Stages

**Precheck gates** (before work starts):
- Design review
- TDD reminder
- Architecture approval
- Manually passed before implementation

**Postcheck gates** (before marking done):
- Automated tests
- Linters and formatters
- Performance benchmarks
- Code review

### Full TDD Cycle Example

```bash
# Create task with complete gate set
jit issue create \
  --title "Add user profile endpoint" \
  --gate tdd-reminder \
  --gate tests \
  --gate clippy \
  --gate fmt \
  --gate code-review

# TDD cycle:
# 1. Write failing test
# 2. Pass tdd-reminder gate
# 3. Implement minimum code to pass
# 4. Refactor if needed
# 5. Mark done (auto-runs tests, clippy, fmt)
# 6. Get code review, pass code-review gate
```

### Reusable Gate Templates

For consistent quality across features:

```bash
# Create template script
cat > scripts/add-backend-gates.sh << 'EOF'
#!/bin/bash
ISSUE=$1
jit gate add $ISSUE tests
jit gate add $ISSUE clippy
jit gate add $ISSUE fmt
jit gate add $ISSUE code-review
EOF

chmod +x scripts/add-backend-gates.sh

# Apply to any backend task
./scripts/add-backend-gates.sh $TASK_ID
```

## Manage Release Milestones

### Recipe: Track v1.0 Release

**Step 1: Label work for the milestone**

```bash
# Create issues with milestone label
jit issue create \
  --title "Core authentication" \
  --label "milestone:v1.0" \
  --label "type:epic" \
  --priority critical

jit issue create \
  --title "Basic dashboard UI" \
  --label "milestone:v1.0" \
  --label "type:epic" \
  --priority high
```

**Step 2: Query milestone progress**

```bash
# All v1.0 work
jit query all --filter "labels.milestone:v1.0"

# v1.0 work by state
jit query all --state done --filter "labels.milestone:v1.0"
jit query all --state in_progress --filter "labels.milestone:v1.0"
jit query available --filter "labels.milestone:v1.0"

# Find blockers
jit query blocked --filter "labels.milestone:v1.0"
```

**Step 3: Track release readiness**

```bash
# Overall status
jit status

# Milestone-specific status
jit query all --filter "labels.milestone:v1.0" --json | \
  jq '[.issues[] | .state] | group_by(.) | map({state: .[0], count: length})'

# Output:
# [
#   {"state": "ready", "count": 5},
#   {"state": "in_progress", "count": 3},
#   {"state": "done", "count": 12}
# ]
```

**Step 4: Critical path analysis**

```bash
# Find longest dependency chain
jit graph export --format mermaid > v1-deps.mmd

# Find issues blocking multiple others
jit query all --filter "labels.milestone:v1.0" --json | \
  jq -r '.issues[] | select(.dependencies | length > 0) | .id' | \
  while read issue; do
    COUNT=$(jit graph downstream $issue --json | jq '.dependents | length')
    echo "$COUNT dependents: $issue"
  done | sort -rn | head -5
```

### Multi-Milestone Management

```bash
# Create multiple milestones
jit issue create --title "MVP Release" --label "milestone:v1.0"
jit issue create --title "Beta Features" --label "milestone:v1.1"
jit issue create --title "Performance" --label "milestone:v2.0"

# Query by milestone
jit query all --filter "labels.milestone:v1.0"
jit query all --filter "labels.milestone:v1.1"

# Cross-milestone dependencies
jit dep add $V2_ISSUE $V1_ISSUE  # v2.0 depends on v1.0 work
```

### Release Checklist Workflow

```bash
# Create release checklist
jit issue create \
  --title "v1.0 Release Checklist" \
  --label "type:epic" \
  --label "milestone:v1.0" \
  --priority critical

# Break down into tasks
jit issue create --title "All tests passing" --label "epic:release"
jit issue create --title "Documentation updated" --label "epic:release"
jit issue create --title "Changelog written" --label "epic:release"
jit issue create --title "Version bumped" --label "epic:release"

# Track completion
jit query all --filter "labels.epic:release"
```

## Track Bug Fixes

### Recipe: Bug Workflow

**Step 1: Report the bug**

```bash
BUG_ID=$(jit issue create \
  --title "Login fails on Safari browser" \
  --description "Steps to reproduce:
1. Open Safari 17.0
2. Navigate to /login
3. Enter valid credentials
4. Click submit
5. Error: 'Invalid token format'

Expected: Successful login
Actual: Token validation error" \
  --label "type:bug" \
  --label "component:web" \
  --label "severity:high" \
  --priority critical \
  --json | jq -r '.id')
```

**Step 2: Triage and prioritize**

```bash
# Query all bugs by severity
jit query all --filter "labels.type:bug AND labels.severity:critical"
jit query all --filter "labels.type:bug AND labels.severity:high"

# Assign to developer
jit issue assign $BUG_ID agent:frontend-dev
```

**Step 3: Link to affected feature**

```bash
# If bug affects existing feature, add epic label
jit issue update $BUG_ID --label "epic:auth"

# Query all auth-related bugs
jit query all --filter "labels.type:bug AND labels.epic:auth"
```

**Step 4: Add regression test gate**

```bash
# Ensure regression test is written
jit gate define regression-test \
  --title "Regression Test" \
  --description "Test case added to prevent recurrence" \
  --mode manual

jit gate add $BUG_ID regression-test
jit gate add $BUG_ID tests
jit gate add $BUG_ID code-review
```

**Step 5: Fix and verify**

```bash
# Work on fix
jit issue update $BUG_ID --state in_progress

# After fix, pass regression gate
jit gate pass $BUG_ID regression-test --by agent:frontend-dev

# Mark done (auto-runs tests)
jit issue update $BUG_ID --state done
```

### Bug Severity Labels

```bash
# Critical: System down, data loss
jit issue create --title "Database corruption on crash" \
  --label "type:bug" --label "severity:critical"

# High: Major feature broken
jit issue create --title "Payment processing fails" \
  --label "type:bug" --label "severity:high"

# Medium: Feature partially works
jit issue create --title "Export button disabled on mobile" \
  --label "type:bug" --label "severity:medium"

# Low: Cosmetic issues
jit issue create --title "Typo in error message" \
  --label "type:bug" --label "severity:low"
```

### Bug Triage Queries

```bash
# Unassigned critical bugs
jit query available --filter "labels.type:bug AND labels.severity:critical"

# Stale bugs (in backlog > 7 days)
jit query all --state backlog --filter "labels.type:bug" --json | \
  jq -r '.issues[] | select(.created_at < (now - 604800)) | "\(.id) - \(.title)"'

# Bugs by component
jit query all --filter "labels.type:bug AND labels.component:backend"
jit query all --filter "labels.type:bug AND labels.component:frontend"
```

## Coordinate Multiple Contributors

For team coordination, claiming work, leases, and parallel work patterns, see the dedicated guide:

→ **[Multi-Agent Coordination](multi-agent-coordination.md)**

## Integrate with CI/CD

### Recipe: Run Gates in CI Pipeline

**Step 1: Configure automated gates**

```bash
# Define CI-friendly gates
jit gate define ci-tests \
  --title "CI Test Suite" \
  --mode auto \
  --checker-command "make test" \
  --working-dir "." \
  --timeout 600

jit gate define ci-lint \
  --title "Lint Check" \
  --mode auto \
  --checker-command "make lint"

jit gate define ci-build \
  --title "Build Check" \
  --mode auto \
  --checker-command "make build"
```

**Step 2: CI pipeline script**

```bash
#!/bin/bash
# .github/workflows/jit-gates.sh

set -e

# Get issue ID from PR/commit message
ISSUE_ID=$(git log -1 --pretty=%B | grep -oP 'Issue: \K[a-f0-9-]+' || echo "")

if [ -z "$ISSUE_ID" ]; then
  echo "No issue ID found in commit message"
  exit 0
fi

# Run all automated gates
jit gate check-all $ISSUE_ID --json > gate-results.json

# Check if all passed
FAILED=$(jq -r '[.[] | select(.status == "failed")] | length' gate-results.json)

if [ "$FAILED" -gt 0 ]; then
  echo "❌ $FAILED gate(s) failed"
  jq -r '.[] | select(.status == "failed") | "  - \(.gate_key): \(.error)"' gate-results.json
  exit 1
else
  echo "✅ All gates passed"
  exit 0
fi
```

**Step 3: GitHub Actions workflow**

```yaml
# .github/workflows/jit-gates.yml
name: JIT Quality Gates

on:
  pull_request:
  push:
    branches: [main]

jobs:
  gates:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install JIT
        run: |
          curl -sSL https://install-jit.dev | sh
          echo "$HOME/.jit/bin" >> $GITHUB_PATH
      
      - name: Run quality gates
        run: |
          bash .github/workflows/jit-gates.sh
```

**Step 4: Commit message format**

```bash
# Include issue ID in commit for CI tracking
git commit -m "feat: Add user profile endpoint

Issue: 9db27a3a-86c5-4d79-9582-9ad68364ea36

- Implement GET /users/:id
- Add validation
- Add tests"
```

### Pre-commit Hooks

```bash
# .git/hooks/pre-commit
#!/bin/bash

# Get staged files
STAGED=$(git diff --cached --name-only)

# Run local gates before commit
if command -v jit &> /dev/null; then
  # Find issue ID from branch name
  BRANCH=$(git branch --show-current)
  ISSUE_ID=$(echo $BRANCH | grep -oP '[a-f0-9]{8}' || echo "")
  
  if [ -n "$ISSUE_ID" ]; then
    echo "Running local gates for issue $ISSUE_ID..."
    jit gate check $ISSUE_ID fmt || {
      echo "❌ Format check failed. Run 'cargo fmt' first."
      exit 1
    }
  fi
fi
```

### Manual Gate Passing in CI

```bash
# After PR approval, mark code-review gate as passed
# (Typically done by reviewer or automation)
jit gate pass $ISSUE_ID code-review --by human:reviewer-name
```

### Status Badges

```bash
# Generate status for README
STATUS=$(jit status --json | jq -r '"\(.done)/\(.total) issues completed"')
echo "![JIT Progress](https://img.shields.io/badge/Progress-$STATUS-blue)"
```

## See Also

- [First Workflow Tutorial](../tutorials/first-workflow.md) - Complete walkthrough
- [Multi-Agent Coordination](multi-agent-coordination.md) - Team and agent coordination
- [Dependency Management](dependency-management.md) - Advanced dependency patterns
- [CLI Reference](../reference/cli-commands.md) - Full command documentation
