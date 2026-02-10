# How-To: Custom Gates

> **Diátaxis Type:** How-To Guide

Quality gates enforce process requirements before issues can be completed. This guide shows how to define and use custom gates for your workflow.

## Create Your First Gate

### Manual Gate (Simple)

Manual gates are reminders that require human judgment:

```bash
# Define a code review gate
jit gate define code-review \
  --title "Code Review" \
  --description "Code must be reviewed by another developer" \
  --stage postcheck \
  --mode manual

# Add to an issue
jit gate add $ISSUE code-review

# Later, mark as passed
jit gate pass $ISSUE code-review --by "human:reviewer"
```

**Use manual gates for:**
- Code reviews
- Design approvals
- Security audits
- Documentation review

### Automated Gate (With Checker)

Automated gates run scripts to verify conditions:

```bash
# Define a test gate with automated checker
jit gate define tests \
  --title "All Tests Pass" \
  --description "Full test suite must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

# Add to an issue
jit gate add $ISSUE tests

# Run the checker
jit gate check $ISSUE tests
# ✓ tests passed (exit code 0)

# Or check all gates at once
jit gate check-all $ISSUE
```

**Use automated gates for:**
- Running tests
- Linters (clippy, eslint, etc.)
- Build verification
- Security scans

## Manual vs Automated Gates

### When to Use Manual Gates

**Appropriate for:**
- Subjective quality checks (code review, design approval)
- Human judgment required (security review, UX evaluation)
- External dependencies (stakeholder sign-off)
- Process reminders (TDD: write tests first)

**Example: TDD Reminder**
```bash
jit gate define tdd-reminder \
  --title "TDD Reminder" \
  --description "Write tests before implementation" \
  --stage precheck \
  --mode manual

# Reminds developers to write tests first
# No automation - relies on process discipline
```

### When to Use Automated Gates

**Appropriate for:**
- Objective, programmatic checks (tests pass, code compiles)
- Repeatable verification (lint rules, formatting)
- Fast feedback loops (under 5 minutes)
- CI/CD integration

**Example: Clippy Linter**
```bash
jit gate define clippy \
  --title "Clippy Lints Pass" \
  --description "No clippy warnings" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy --all-targets -- -D warnings" \
  --timeout 120
```

### Combining Both

Most workflows use both manual and automated gates:

```bash
# Automated quality checks
jit gate add $ISSUE tests clippy fmt

# Manual process gate
jit gate add $ISSUE code-review

# Automated gates run automatically, manual gate requires sign-off
```

## Write Gate Checker Scripts

Automated gates execute shell commands. Follow these patterns for reliable checkers.

### Exit Codes

Gates use standard exit codes:

- **0** - Gate passed
- **Non-zero** - Gate failed

```bash
#!/bin/bash
# Example checker script

# Run tests
if cargo test --quiet; then
  echo "✓ All tests passed"
  exit 0
else
  echo "✗ Tests failed"
  exit 1
fi
```

### Best Practices

**1. Make checkers fast** (target: under 5 minutes)
```bash
# Good: Focused test subset
cargo test --lib

# Avoid: Slow integration tests in gate
# cargo test --all  # Too slow for quick feedback
```

**2. Provide clear output**
```bash
# Good: Specific error message
echo "✗ Clippy found 3 warnings in src/main.rs"

# Avoid: Generic failure
echo "Failed"
```

**3. Use working directory option for multi-crate repos**
```bash
jit gate define backend-tests \
  --title "Backend Tests" \
  --description "Backend test suite" \
  --mode auto \
  --checker-command "cargo test" \
  --working-dir "crates/backend"
```

**4. Set appropriate timeouts**
```bash
# Fast checks: 60-120 seconds
--timeout 60   # Linters, formatters

# Test suites: 300-600 seconds
--timeout 300  # Unit tests
--timeout 600  # Integration tests
```

### Example: Multi-Step Checker

```bash
#!/bin/bash
# scripts/quality-gate.sh - Composite checker

set -e  # Exit on first error

echo "Running quality checks..."

# Step 1: Format check
echo "1/3 Checking formatting..."
cargo fmt --check

# Step 2: Linter
echo "2/3 Running clippy..."
cargo clippy --all-targets -- -D warnings

# Step 3: Tests
echo "3/3 Running tests..."
cargo test --lib

echo "✓ All quality checks passed"
exit 0
```

Register the script as a gate:
```bash
jit gate define quality \
  --title "Quality Checks" \
  --description "Format, lint, and test" \
  --mode auto \
  --checker-command "./scripts/quality-gate.sh" \
  --timeout 300
```

## Prechecks vs Postchecks

Gates can run at two stages in the workflow:

### Prechecks (Before Work Begins)

Run when issue transitions **to** `in_progress` state.

**Purpose:** Ensure prerequisites are met before starting work.

**Use for:**
- TDD reminders (write tests first)
- Design approval required
- Prerequisites verified (dependencies installed, environment configured)

**Example: TDD Precheck**
```bash
jit gate define tdd-precheck \
  --title "TDD: Tests Exist" \
  --description "Verify test file exists before implementation" \
  --stage precheck \
  --mode auto \
  --checker-command "test -f tests/feature_test.rs"
```

**Workflow:**
```bash
# Issue requires TDD precheck
jit issue claim $ISSUE agent:me

# Precheck runs automatically
# If fails: Issue transitions to gated (must pass before starting)
# If passes: Issue transitions to in_progress
```

### Postchecks (After Work Completes)

Run when issue transitions **to** `done` state.

**Purpose:** Verify work quality before completion.

**Use for:**
- Tests pass
- Code review complete
- Documentation updated
- Build succeeds

**Example: Test Postcheck**
```bash
jit gate define tests \
  --title "All Tests Pass" \
  --description "Test suite must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib"
```

**Workflow:**
```bash
# Complete work, attempt to finish
jit issue update $ISSUE --state done

# Postcheck runs automatically
# If fails: Issue transitions to gated (fix and retry)
# If passes: Issue transitions to done
```

### Choosing Stage

| Gate Type | Stage | Reason |
|-----------|-------|--------|
| TDD reminder | Precheck | Ensure tests written before code |
| Design approval | Precheck | Validate approach before implementation |
| Tests pass | Postcheck | Verify implementation works |
| Code review | Postcheck | Quality check after completion |
| Linter | Postcheck | Enforce style after writing |
| Security scan | Postcheck | Verify no vulnerabilities introduced |

## Gate Presets and Templates

Gate presets are pre-configured bundles of quality gates that dramatically reduce setup time. Instead of defining and adding gates individually, apply entire workflows in seconds.

### Using Builtin Presets

JIT includes two builtin presets for common workflows:

**List available presets:**
```bash
jit gate preset list
```

**Output:**
```
[builtin] rust-tdd - Test-driven development workflow for Rust projects (5 gates)
[builtin] minimal - Minimal workflow with just code review (1 gate)
```

**View preset details:**
```bash
jit gate preset show rust-tdd
```

**Output:**
```
Preset: rust-tdd
Description: Test-driven development workflow for Rust projects

Gates:
  tdd-reminder - Write tests first (TDD) (precheck:manual)
  tests - All tests pass (postcheck:auto)
    Command: cargo test
    Timeout: 300s
  clippy - Clippy lints pass (postcheck:auto)
    Command: cargo clippy --all-targets -- -D warnings
    Timeout: 120s
  fmt - Code formatted (postcheck:auto)
    Command: cargo fmt --check
    Timeout: 30s
  code-review - Code review completed (postcheck:manual)
```

**Apply preset to issue:**
```bash
# Create issue
jit issue create --title "Implement user authentication"

# Apply rust-tdd preset (adds all 5 gates at once)
jit gate preset apply rust-tdd abc123
```

**Done!** The issue now has:
- TDD reminder (precheck)
- Tests, clippy, fmt (automated postchecks)
- Code review (manual postcheck)

### Customizing Preset Application

Filter which gates to apply using command options:

**Skip precheck gates:**
```bash
# For hotfixes or situations where TDD isn't required
jit gate preset apply rust-tdd abc123 --no-precheck
# Adds: tests, clippy, fmt, code-review (skips tdd-reminder)
```

**Skip postcheck gates:**
```bash
# For planning or precheck-only workflows
jit gate preset apply rust-tdd abc123 --no-postcheck
# Adds: tdd-reminder only
```

**Exclude specific gates:**
```bash
# Skip clippy if not using linter
jit gate preset apply rust-tdd abc123 --except clippy

# Skip multiple gates
jit gate preset apply rust-tdd abc123 --except clippy --except fmt
# Adds: tdd-reminder, tests, code-review
```

**Override timeouts:**
```bash
# Increase timeout for slow CI
jit gate preset apply rust-tdd abc123 --timeout 600
# All automated gates get 600s timeout instead of defaults
```

**Combine filters:**
```bash
# Hotfix workflow: no precheck, no linter, fast timeout
jit gate preset apply rust-tdd abc123 --no-precheck --except clippy --timeout 60
# Adds: tests (60s), fmt (60s), code-review
```

### Batch Operations

Apply presets to multiple issues at once:

**Multiple issues directly:**
```bash
jit gate preset apply minimal abc123 def456 ghi789
# Applies to all three issues
```

**From query results:**
```bash
# Apply to all issues in an epic
jit query all --filter "label:epic:auth" | xargs jit gate preset apply rust-tdd

# Apply to all ready issues
jit query available | xargs jit gate preset apply minimal
```

### Creating Custom Presets

Capture your team's workflow as a reusable preset:

**Step 1: Configure one issue perfectly**
```bash
# Create issue and add desired gates
jit issue create --title "Reference issue"
jit gate add abc123 tests clippy code-review docs security-scan

# Or apply builtin and customize
jit gate preset apply rust-tdd abc123 --except fmt
jit gate add abc123 security-scan
```

**Step 2: Save as custom preset**
```bash
jit gate preset create abc123 team-standard
```

**Output:**
```
Created preset 'team-standard' at .jit/config/gate-presets/team-standard.json
```

**Step 3: Use everywhere**
```bash
# List shows custom preset
jit gate preset list
# [custom] team-standard - Custom preset created from issue abc123 (5 gates)

# Apply to any issue
jit gate preset apply team-standard def456
```

### Custom Preset Storage

Custom presets are stored as JSON files in `.jit/config/gate-presets/`:

```bash
# View custom preset file
cat .jit/config/gate-presets/team-standard.json
```

```json
{
  "name": "team-standard",
  "description": "Custom preset created from issue abc123",
  "gates": [
    {
      "key": "tests",
      "title": "All tests pass",
      "description": "cargo test must pass",
      "stage": "postcheck",
      "mode": "auto",
      "checker": {
        "type": "exec",
        "command": "cargo test",
        "timeout_seconds": 300,
        "working_dir": null,
        "env": {}
      }
    },
    ...
  ]
}
```

**Managing custom presets:**
- Edit JSON files directly for fine-tuning
- Delete files to remove presets
- Share files with team via git
- Custom presets override builtin with same name

### Practical Workflows

**Quick Start New Issue:**
```bash
jit issue create --title "New feature"
jit gate preset apply rust-tdd $ISSUE_ID
# Ready to work with full quality pipeline
```

**Team Onboarding:**
```bash
# Document team standards
jit gate preset create reference-issue team-workflow

# Team members apply to their issues
jit gate preset apply team-workflow their-issue
# Instant consistency across team
```

**Different Requirements by Type:**
```bash
# Full workflow for features
jit gate preset apply rust-tdd feature-issue

# Minimal for docs
jit gate preset apply minimal doc-issue --except code-review
jit gate add doc-issue spell-check

# Custom for infrastructure
jit gate preset apply team-infra infra-issue
```

**Migration from Manual Setup:**
```bash
# Old way (slow, error-prone):
jit gate define tests --mode auto --checker-command "cargo test" ...
jit gate define clippy --mode auto --checker-command "cargo clippy" ...
jit gate define fmt --mode auto --checker-command "cargo fmt --check" ...
jit gate add $ISSUE tests clippy fmt code-review
# 4+ commands, easy to forget gates

# New way (fast, consistent):
jit gate preset apply rust-tdd $ISSUE
# 1 command, guaranteed completeness
```

### Comparing with Manual Gate Definitions

Below shows the manual approach for reference, but **use presets instead** for consistency and speed:

**Manual approach (old):**
```bash
# Define each gate individually
jit gate define tests \
  --title "All Tests Pass" \
  --description "cargo test must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

jit gate define clippy \
  --title "Clippy Clean" \
  --description "No clippy warnings" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy --all-targets -- -D warnings" \
  --timeout 120

# ... repeat for each gate

# Add to each issue
jit gate add $ISSUE tests clippy fmt code-review
```

**Preset approach (new):**
```bash
# One command
jit gate preset apply rust-tdd $ISSUE
```

**Benefits:**
- **10x faster**: 1 command vs 10+ commands
- **No mistakes**: Preset definitions are tested and proven
- **Consistent**: Same gates on every issue
- **Shareable**: Team uses identical workflows
- **Customizable**: Filter options for special cases

## Common Workflows

For complete workflow examples including TDD and CI/CD integration, see [Software Development](software-development.md).

### Workflow 1: Code Quality Pipeline

```bash
# Define quality gates
jit gate define fmt --mode auto --stage postcheck --checker-command "cargo fmt --check"
jit gate define clippy --mode auto --stage postcheck --checker-command "cargo clippy -- -D warnings"
jit gate define tests --mode auto --stage postcheck --checker-command "cargo test"

# Apply to all issues in epic (batch mode still uses --add-gate)
jit issue update --filter "label:epic:auth" --add-gate fmt --add-gate clippy --add-gate tests

# Developer completes work
jit issue update $ISSUE --state done
# All three gates run automatically
# Issue transitions to gated if any fail
# Auto-transitions to done when all pass
```

### Workflow 3: Manual + Automated Review

```bash
# Automated checks
jit gate add $ISSUE tests clippy

# Manual review
jit gate add $ISSUE code-review

# Complete work
jit issue update $ISSUE --state done
# Automated gates run, manual gate remains unpassed
# Issue in gated state

# Reviewer approves
jit gate pass $ISSUE code-review --by "human:alice"
# Issue auto-transitions to done
```

## Troubleshooting Gate Failures

### Common Issues and Solutions

#### "Repository not initialized"

```bash
Error: .jit directory not found
```

**Solution:** Run `jit init` in your project directory first.

#### "Cycle detected"

```bash
Error: Adding dependency would create a cycle
```

**Solution:** Check your dependency graph with `jit graph export --format dot` or `jit graph deps --depth 0` and remove circular references. Dependencies must form a directed acyclic graph (DAG).

#### "Invalid label format"

```bash
Error: Invalid label format: 'milestone-v1.0'
Expected format: 'namespace:value'
```

**Solution:** Use colon separator: `--label "milestone:v1.0"`

#### "Missing type label"

```bash
Warning: Issue created without type label
```

**Solution:** Add type label: `jit issue update $ISSUE --label "type:task"`

#### "Orphaned task"

```bash
Warning: Issue is an orphaned task (no epic:* or milestone:* label)
```

**Solution:** Add parent label: `jit issue update $ISSUE --label "epic:auth"` or use `--orphan` flag to explicitly allow orphaned issues.

### Validation and Recovery

```bash
# Check repository health
jit validate

# Automatically fix issues
jit validate --fix

# Preview fixes without applying
jit validate --fix --dry-run
```

### Getting Help

```bash
# Command-specific help
jit issue create --help
jit dep add --help
jit gate define --help

# List available commands
jit --help

# Check configured label namespaces
jit label namespaces

# View existing label values
jit label values milestone
jit label values epic
```

## Advanced Topics

### Gate System Extensibility

The current gate system is intentionally simple and flexible. Future enhancements being considered:

**Potential extensions** (not yet implemented):
- **Conditional gates**: Apply different gates based on issue labels or properties
- **Gate dependencies**: Enforce gate ordering (e.g., tests before code-review)
- **Parallel execution**: Run multiple automated gates concurrently
- **Rich output**: Structured results with metrics, warnings, and artifacts
- **Gate templates**: Pre-configured gate bundles for common workflows
- **Custom gate stages**: Beyond precheck/postcheck for complex pipelines

These would be added based on real-world usage patterns while maintaining backward compatibility.

### Adapting Gates to Your Domain

Gates are domain-agnostic quality checkpoints. The examples in this guide focus on software development, but the patterns apply broadly:

**Research**: Literature review, peer review, data validation, statistical significance
**Writing**: Outline approval, editor review, spell check, word count targets, fact-checking
**Design**: Stakeholder approval, user testing, accessibility checks, brand compliance
**Operations**: Change approval, rollback plan, monitoring setup, incident review

The key insight: **any workflow with quality requirements can use gates**.

## See Also

- [Core Model - Gates](../concepts/core-model.md#gates) - Conceptual understanding
- [CLI Reference - Gate Commands](../reference/cli-commands.md#gate-commands) - Complete command syntax
- [First Workflow Tutorial](../tutorials/first-workflow.md) - Gate usage in practice
