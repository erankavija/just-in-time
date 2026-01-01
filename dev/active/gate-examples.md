# Gate Configuration Examples

This document provides practical examples of gate configurations for common workflows.

## TDD Workflow with Quality Checks

### Setup Gates (Once Per Project)

```bash
# 1. TDD reminder - manual checkpoint (agent self-acknowledges)
jit gate define tdd-reminder \
  --title "TDD Reminder" \
  --description "Write failing tests before implementation" \
  --stage precheck \
  --mode manual

# 2. Unit tests - automated postcheck
jit gate define unit-tests \
  --title "Unit Tests Pass" \
  --description "All unit tests must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

# 3. Clippy - automated postcheck
jit gate define clippy \
  --title "Clippy Lints" \
  --description "No clippy warnings allowed" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy -- -D warnings" \
  --timeout 180

# 4. Format - automated postcheck
jit gate define rustfmt \
  --title "Code Formatting" \
  --description "Code must be formatted with rustfmt" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo fmt -- --check" \
  --timeout 60
```

### Agent Workflow

```bash
# 1. Create issue with gates
ISSUE=$(jit issue create --title "Add user authentication" \
  --gate tdd-reminder \
  --gate unit-tests \
  --gate clippy \
  --gate rustfmt)

# 2. Try to start work (precheck reminder appears)
jit issue update $ISSUE --state in_progress
# ⚠️  Reminder: tdd-reminder
#    Write failing tests before implementation
#    
# Acknowledge: jit gate pass abc123 tdd-reminder --by copilot:worker-1

# 3. Agent acknowledges TDD reminder
jit gate pass $ISSUE tdd-reminder --by copilot:worker-1

# 4. Start work
jit issue update $ISSUE --state in_progress
# ✓ tdd-reminder acknowledged
# Issue abc123 → in_progress

# 5. Complete work (postchecks run automatically)
jit issue complete $ISSUE
# Running postchecks...
# ✓ unit-tests passed (2.3s)
# ✓ clippy passed (1.8s)
# ✓ rustfmt passed (0.2s)
# All gates passed! Issue abc123 → done
```

## Context Validation Examples

### Option 1: Manual Context Checklist

Multiple reminders for agent to review before starting:

```bash
jit gate define context-check \
  --title "Context Review" \
  --description "Requirements clear, acceptance criteria defined, tests planned" \
  --stage precheck \
  --mode manual

jit gate define design-review \
  --title "Design Reviewed" \
  --description "Architecture approach confirmed" \
  --stage precheck \
  --mode manual
```

**Agent workflow:**
```bash
# Agent goes through checklist
jit gate pass $ISSUE context-check --by copilot:worker-1
jit gate pass $ISSUE design-review --by copilot:worker-1
jit issue update $ISSUE --state in_progress
```

### Option 2: Automated Context Validation

Validate that issue has sufficient detail:

```bash
jit gate define context-complete \
  --title "Context Complete" \
  --description "Issue has description, acceptance criteria, and test plan" \
  --stage precheck \
  --mode auto \
  --checker-command "./scripts/validate-issue-context.sh" \
  --timeout 10
```

**Validation script:**
```bash
#!/bin/bash
# scripts/validate-issue-context.sh

ISSUE_ID=$JIT_ISSUE_ID
ISSUE_JSON=$(jit issue show $ISSUE_ID --json)
DESC=$(echo "$ISSUE_JSON" | jq -r '.description')

# Check description length
if [ ${#DESC} -lt 50 ]; then
    echo "ERROR: Issue description too short (${#DESC} chars)"
    echo "Please add detailed description with context"
    exit 1
fi

# Check for acceptance criteria
if ! echo "$DESC" | grep -qi "acceptance criteria"; then
    echo "ERROR: No acceptance criteria found"
    echo "Add section: ## Acceptance Criteria"
    exit 1
fi

echo "✓ Issue context validated"
echo "  - Description: ${#DESC} chars"
echo "  - Acceptance criteria: present"
exit 0
```

### Option 3: Design Document Check

Ensure design doc is attached before starting:

```bash
jit gate define design-doc-exists \
  --title "Design Document" \
  --description "Design doc attached and reviewed" \
  --stage precheck \
  --mode auto \
  --checker-command "./scripts/check-design-doc.sh" \
  --timeout 5
```

**Validation script:**
```bash
#!/bin/bash
# scripts/check-design-doc.sh

ISSUE_ID=$JIT_ISSUE_ID
DOC_COUNT=$(jit doc list $ISSUE_ID --json | jq '.documents | length')

if [ "$DOC_COUNT" -eq 0 ]; then
    echo "ERROR: No design document attached"
    echo ""
    echo "Attach design doc:"
    echo "  jit doc add $ISSUE_ID docs/design.md --label 'Design'"
    exit 1
fi

echo "✓ Design document attached ($DOC_COUNT doc(s))"
exit 0
```

### Option 4: Git Branch Check

Ensure work is on proper feature branch:

```bash
jit gate define branch-naming \
  --title "Branch Naming" \
  --description "Working on feature branch, not main" \
  --stage precheck \
  --mode auto \
  --checker-command "./scripts/check-branch.sh" \
  --timeout 5
```

**Validation script:**
```bash
#!/bin/bash
# scripts/check-branch.sh

ISSUE_ID=$JIT_ISSUE_ID
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)

# Don't allow work on main/master
if [[ "$CURRENT_BRANCH" == "main" || "$CURRENT_BRANCH" == "master" ]]; then
    echo "ERROR: Cannot work directly on $CURRENT_BRANCH"
    echo ""
    echo "Create feature branch:"
    echo "  git checkout -b feature/$ISSUE_ID"
    exit 1
fi

echo "✓ Working on feature branch: $CURRENT_BRANCH"
exit 0
```

## Language-Specific Examples

### Rust Project

```bash
# Setup script for Rust projects
cat > scripts/setup-rust-gates.sh << 'EOF'
#!/bin/bash

jit gate define tdd-reminder \
  --title "TDD Reminder" \
  --stage precheck --mode manual

jit gate define unit-tests \
  --title "Unit Tests" \
  --stage postcheck --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

jit gate define integration-tests \
  --title "Integration Tests" \
  --stage postcheck --mode auto \
  --checker-command "cargo test --test '*'" \
  --timeout 600

jit gate define clippy \
  --title "Clippy" \
  --stage postcheck --mode auto \
  --checker-command "cargo clippy -- -D warnings" \
  --timeout 180

jit gate define rustfmt \
  --title "Format" \
  --stage postcheck --mode auto \
  --checker-command "cargo fmt -- --check" \
  --timeout 60

jit gate define cargo-check \
  --title "Cargo Check" \
  --stage postcheck --mode auto \
  --checker-command "cargo check --all-targets" \
  --timeout 120

echo "✓ Rust gates configured!"
jit gate list
EOF

chmod +x scripts/setup-rust-gates.sh
./scripts/setup-rust-gates.sh
```

### Python Project

```bash
jit gate define pytest \
  --title "Pytest" \
  --stage postcheck --mode auto \
  --checker-command "pytest tests/" \
  --timeout 300

jit gate define mypy \
  --title "Type Checking" \
  --stage postcheck --mode auto \
  --checker-command "mypy src/" \
  --timeout 120

jit gate define black \
  --title "Code Formatting" \
  --stage postcheck --mode auto \
  --checker-command "black --check src/" \
  --timeout 60

jit gate define pylint \
  --title "Linting" \
  --stage postcheck --mode auto \
  --checker-command "pylint src/ --fail-under=8.0" \
  --timeout 180
```

### JavaScript/TypeScript Project

```bash
jit gate define jest \
  --title "Jest Tests" \
  --stage postcheck --mode auto \
  --checker-command "npm test" \
  --timeout 300

jit gate define eslint \
  --title "ESLint" \
  --stage postcheck --mode auto \
  --checker-command "npm run lint" \
  --timeout 120

jit gate define prettier \
  --title "Prettier" \
  --stage postcheck --mode auto \
  --checker-command "npm run format:check" \
  --timeout 60

jit gate define typescript \
  --title "TypeScript" \
  --stage postcheck --mode auto \
  --checker-command "npm run type-check" \
  --timeout 180
```

## Security-Focused Gates

```bash
# Dependency vulnerability scanning
jit gate define security-audit \
  --title "Security Audit" \
  --description "Check for known vulnerabilities" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo audit" \
  --timeout 60

# Secret detection
jit gate define secret-scan \
  --title "Secret Scan" \
  --description "Detect accidentally committed secrets" \
  --stage postcheck \
  --mode auto \
  --checker-command "./scripts/check-secrets.sh" \
  --timeout 30

# SAST (Static Application Security Testing)
jit gate define sast \
  --title "SAST Scan" \
  --description "Static security analysis" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy -- -W clippy::all" \
  --timeout 300
```

## Performance Gates

```bash
# Benchmark regression check
jit gate define benchmark \
  --title "Performance Benchmark" \
  --description "Ensure no performance regression" \
  --stage postcheck \
  --mode auto \
  --checker-command "./scripts/benchmark-check.sh" \
  --timeout 600

# Binary size check
jit gate define binary-size \
  --title "Binary Size" \
  --description "Check binary size stays under limit" \
  --stage postcheck \
  --mode auto \
  --checker-command "./scripts/check-binary-size.sh" \
  --timeout 60
```

## Best Practices

### 1. Precheck Philosophy

**Use prechecks for:**
- Reminders and checklists (lightweight, agent self-acknowledges)
- Context validation (ensure issue has sufficient detail)
- Workflow enforcement (branch naming, design docs)

**Keep prechecks fast** (< 10 seconds):
- They run on every work start attempt
- Slow prechecks frustrate agents

### 2. Postcheck Philosophy

**Use postchecks for:**
- Quality enforcement (tests, linting, formatting)
- Security scanning
- Performance validation

**Postchecks can be slower**:
- Run once when work completes
- Can take minutes for comprehensive test suites

### 3. Manual vs Automated

**Manual gates** (agent self-pass):
- TDD reminders
- Context checklists
- Human approvals (code review)

**Automated gates** (script execution):
- Test execution
- Linting and formatting
- Security scans
- Any deterministic check

### 4. Separation of Concerns

```bash
# Precheck: Lightweight reminder
jit gate define tdd-reminder --stage precheck --mode manual

# Postcheck: Rigorous enforcement
jit gate define unit-tests --stage postcheck --mode auto
```

**Philosophy**: Prechecks remind, postchecks enforce.

## Quick Setup Templates

### Minimal (TDD + Tests)

```bash
jit gate define tdd-reminder --stage precheck --mode manual
jit gate define unit-tests --stage postcheck --mode auto \
  --checker-command "cargo test --lib" --timeout 300
```

### Standard (TDD + Quality)

```bash
jit gate define tdd-reminder --stage precheck --mode manual
jit gate define unit-tests --stage postcheck --mode auto \
  --checker-command "cargo test --lib" --timeout 300
jit gate define clippy --stage postcheck --mode auto \
  --checker-command "cargo clippy -- -D warnings" --timeout 180
jit gate define rustfmt --stage postcheck --mode auto \
  --checker-command "cargo fmt -- --check" --timeout 60
```

### Comprehensive (TDD + Quality + Security)

```bash
jit gate define tdd-reminder --stage precheck --mode manual
jit gate define context-check --stage precheck --mode manual
jit gate define unit-tests --stage postcheck --mode auto \
  --checker-command "cargo test --lib" --timeout 300
jit gate define integration-tests --stage postcheck --mode auto \
  --checker-command "cargo test --test '*'" --timeout 600
jit gate define clippy --stage postcheck --mode auto \
  --checker-command "cargo clippy -- -D warnings" --timeout 180
jit gate define rustfmt --stage postcheck --mode auto \
  --checker-command "cargo fmt -- --check" --timeout 60
jit gate define security-audit --stage postcheck --mode auto \
  --checker-command "cargo audit" --timeout 60
```

## See Also

- [CI Gate Integration Design](ci-gate-integration-design.md) - Complete design specification
- [Example Workflows](../../docs/tutorials/first-workflow.md) - End-to-end agent coordination examples
