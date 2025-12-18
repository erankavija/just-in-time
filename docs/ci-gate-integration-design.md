# CI Gate Integration Design

**Status**: Planning  
**Date**: 2025-12-17  
**Version**: 1.0

## Overview

Add automated checking capabilities to the quality gate system, enabling both manual checklist-style gates and automated script execution. The system must be simple and intuitive for AI agents to understand and use.

## Core Principles

1. **Two gate variants**: Manual checklists and automated scripts
2. **Lifecycle stages**: Gates run at different points (before work, after work)
3. **AI-friendly**: Clear semantics, predictable behavior, obvious failure modes
4. **Unified framework**: Single gate concept with different execution strategies

## Gate Types

### 1. Manual Checklist Gates

Traditional quality gates that require explicit human/agent approval:

```json
{
  "key": "code-review",
  "title": "Code Review",
  "description": "Human review of code changes",
  "stage": "postcheck",
  "auto": false
}
```

**Usage:**
```bash
# Manually pass the gate
jit gate pass <issue-id> code-review --by human:alice
```

### 2. Automated Script Gates

Gates that execute programs and auto-pass/fail based on exit code:

```json
{
  "key": "unit-tests",
  "title": "Unit Tests",
  "description": "Run project unit tests",
  "stage": "postcheck",
  "auto": true,
  "checker": {
    "type": "command",
    "command": "cargo test --lib",
    "timeout_seconds": 300,
    "working_dir": ".",
    "env": {
      "RUST_BACKTRACE": "1"
        }
  }
}
```

**Behavior:**
- Exit code 0 → gate passes automatically
- Non-zero exit code → gate fails
- Timeout → gate fails
- All output captured for debugging

## Lifecycle Stages

Gates execute at different points in the issue lifecycle:

### Precheck Stage

**When**: Before work starts (transitioning `ready` → `in_progress`)  
**Purpose**: Validate preconditions are met before beginning work  
**Use cases**:
- TDD: Verify test files exist before implementation
- Design: Ensure design doc approved before coding
- Environment: Check dependencies installed, tools available
- Acceptance criteria: Validate issue has sufficient detail

**Example:**
```bash
# Issue with precheck
ISSUE=$(jit issue create --title "Add login feature" \
  --gate tdd-setup:precheck \
  --gate unit-tests:postcheck)

# Try to start work
jit issue update $ISSUE --state in_progress
# → Runs tdd-setup precheck
# → Only allows transition if precheck passes
```

### Postcheck Stage

**When**: After work completes (transitioning `in_progress` → `gated`)  
**Purpose**: Validate work meets quality standards  
**Use cases**:
- Tests: Run unit/integration tests
- Linting: Check code style, warnings
- Security: Run vulnerability scans
- Review: Manual code review

**Example:**
```bash
# Complete work
jit issue complete $ISSUE
# → Automatically runs all postcheck gates
# → If all pass → done
# → If any fail → stays gated
```

## State Transition Model

### Overview

```
    ┌──────────┐
    │ backlog  │  Has incomplete dependencies
    └─────┬────┘
          │
          │ (dependencies complete)
          ↓
    ┌──────────┐
    │  ready   │  Available to claim/start
    └─────┬────┘
          │
          │ start work → PRECHECKS (on-demand)
          │
          ├─── fail ───┐
          │            ↓
          │       stays ready
          │       (fix & retry)
          │
          │ pass
          ↓
    ┌──────────────┐
    │ in_progress  │  Work being done
    └──────┬───────┘
           │
           │ complete → POSTCHECKS (automatic)
           ↓
    ┌──────────┐
    │  gated   │  Awaiting quality gates
    └─────┬────┘
          │
          ├─── all pass ───→ done
          │
          └─── any fail ───→ stays gated
                             (fix & re-check)
```

### Detailed Transition Flow

```
backlog (incomplete dependencies)
  ↓
[dependencies complete]
  ↓
ready (available to claim and start)
  ↓
[agent tries to start work]
  ↓
[run PRECHECKS on-demand]
  ↓
pass? ──→ in_progress
  ↓              ↓
 fail       [work happens]
  ↓              ↓
stays        [complete work]
ready            ↓
(retry)    [run POSTCHECKS]
                 ↓
            all pass? ──→ done
                 ↓
                fail
                 ↓
              gated
         (fix & re-check)
```

**Key Design Decision: Lazy Precheck Evaluation**

Prechecks run **on-demand** when an agent attempts to transition to `in_progress`, not proactively when dependencies complete.

**Rationale:**
- **Simplicity**: No new states needed
- **Efficiency**: Only check when work is actually starting (don't waste resources on unclaimed issues)
- **Clear semantics**: `ready` = "dependencies complete, available to claim"
- **Retry-friendly**: If prechecks fail, issue stays `ready` and agent can retry after fixing

**Trade-offs:**
- Agents don't know about precheck issues until they try to start
- Cannot pre-filter issues by precheck status
- Acceptable because prechecks should be fast (setup validation, not long-running tests)

## Gate Definition Schema

```json
{
  "gates": {
    "<gate-key>": {
      "key": "string (unique identifier)",
      "title": "string (human-readable name)",
      "description": "string (what this gate checks)",
      "stage": "precheck | postcheck",
      "auto": "boolean (true = can auto-execute, false = manual only)",
      "checker": {
        "type": "command",
        "command": "string (shell command to run)",
        "timeout_seconds": "integer (max execution time)",
        "working_dir": "string (optional, default: repo root)",
        "env": {
          "KEY": "value (optional environment variables)"
        }
      }
    }
  }
}
```

**Field semantics for AI agents:**
- `stage`: When does this gate run? `precheck` = before work, `postcheck` = after work
- `auto`: Can a computer pass this gate automatically? `true` = yes (has checker), `false` = no (human only)
- `checker`: Present only if `auto: true`, defines how to automatically check

## Example Gate Configurations

### TDD Precheck (Automated)

```json
{
  "key": "tdd-setup",
  "title": "TDD Test Setup",
  "description": "Verify test file exists before implementation begins",
  "stage": "precheck",
  "auto": true,
  "checker": {
    "type": "command",
    "command": "./scripts/verify-tdd-setup.sh",
    "timeout_seconds": 10
  }
}
```

### Unit Tests (Automated Postcheck)

```json
{
  "key": "unit-tests",
  "title": "Unit Tests",
  "description": "All unit tests must pass",
  "stage": "postcheck",
  "auto": true,
  "checker": {
    "type": "command",
    "command": "cargo test --lib",
    "timeout_seconds": 300
  }
}
```

### Code Review (Manual Postcheck)

```json
{
  "key": "code-review",
  "title": "Code Review",
  "description": "Human review of code quality and design",
  "stage": "postcheck",
  "auto": false
}
```

### Design Approval (Manual Precheck)

```json
{
  "key": "design-approved",
  "title": "Design Approved",
  "description": "Technical design reviewed and approved",
  "stage": "precheck",
  "auto": false
}
```

## CLI Commands

### Gate Definition

```bash
# Define automated postcheck gate
jit gate define <key> \
  --title "Title" \
  --description "What this checks" \
  --stage postcheck \
  --auto \
  --checker-command "cargo test"

# Define manual precheck gate
jit gate define <key> \
  --title "Title" \
  --stage precheck

# List all gates
jit gate list

# Show gate details
jit gate show <key>
```

### Adding Gates to Issues

```bash
# Create issue with gates
jit issue create --title "Feature X" \
  --gate tdd-setup \
  --gate unit-tests \
  --gate review

# Add gate to existing issue
jit gate add <issue-id> <gate-key>
```

### Manual Gate Operations

```bash
# Manually pass a gate
jit gate pass <issue-id> <gate-key> --by agent:worker-1

# Manually fail a gate
jit gate fail <issue-id> <gate-key> --by ci:github-actions

# Check single gate status
jit gate status <issue-id> <gate-key>
```

### Automated Checking

```bash
# Run specific gate checker
jit gate check <issue-id> <gate-key>

# Run all checkable gates for an issue
jit gate check-all <issue-id>

# Test a gate checker (doesn't update issue)
jit gate test <gate-key>
```

## Automatic Execution Triggers

### Starting Work (Prechecks)

```bash
jit issue update <issue-id> --state in_progress
```

**Behavior (lazy evaluation):**
1. Agent requests transition to `in_progress`
2. Find all gates with `stage: precheck`
3. Run checkers for gates with `auto: true`
4. Check status of gates with `auto: false` (must already be passed manually)
5. If all prechecks pass: transition succeeds, state becomes `in_progress`
6. If any fail: transition rejected, state stays `ready`, show detailed error

**Example output on failure:**
```
Running prechecks...
✗ tdd-setup failed (exit 1, 0.1s)
  ERROR: No test file found for issue abc123
  TDD requires tests before implementation
  Create test file with failing tests first

Cannot transition to in_progress: 1 precheck failed
Issue abc123 remains in 'ready' state

Fix the issue and retry:
  jit issue update abc123 --state in_progress
```

**Precheck triggering:**
- Prechecks run **on-demand** when transition to `in_progress` is attempted
- NOT run proactively when dependencies complete
- This keeps state model simple and avoids wasting resources on unclaimed issues

### Completing Work (Postchecks)

```bash
jit issue complete <issue-id>
# OR
jit issue update <issue-id> --state gated
```

**Behavior:**
1. Transition to `gated` state
2. Find all gates with `stage: postcheck`
3. Run checkers for gates with `auto: true`
4. Check status of gates with `auto: false` (stay pending)
5. If all postchecks pass: auto-transition to `done`
6. If any fail: stay in `gated` state
7. Show results of all checks

## Issue Context for Checkers

Checkers receive issue metadata via environment variables:

```bash
JIT_ISSUE_ID=abc123
JIT_ISSUE_TITLE="Implement login API"
JIT_ISSUE_STATE=in_progress
JIT_ISSUE_PRIORITY=high
JIT_ISSUE_ASSIGNEE=copilot:worker-1
```

Checkers can also read full issue JSON via stdin (future enhancement).

## Example: TDD Workflow

### 1. Define Gates

```bash
# TDD precheck - verify tests exist
jit gate define tdd-setup \
  --title "TDD Test Setup" \
  --stage precheck \
  --auto \
  --checker-command "./scripts/verify-tdd-setup.sh"

# Unit test postcheck - verify tests pass
jit gate define unit-tests \
  --title "Unit Tests Pass" \
  --stage postcheck \
  --auto \
  --checker-command "cargo test --lib"
```

### 2. Create Issue

```bash
ISSUE=$(jit issue create --title "Add user authentication" \
  --gate tdd-setup \
  --gate unit-tests \
  --gate code-review)
```

### 3. Try to Start (Precheck Runs On-Demand)

```bash
jit issue update $ISSUE --state in_progress
```

**Output:**
```
Running prechecks...
✗ tdd-setup failed (exit 1, 0.1s)
  ERROR: No test file found for issue abc123
  TDD requires tests before implementation
  Create test file with failing tests first

Cannot transition to in_progress: 1 precheck failed
Issue abc123 remains in 'ready' state
```

### 4. Create Failing Test (TDD Red Phase)

```bash
cat > tests/test_auth.rs << 'EOF'
#[test]
fn test_user_login() {
    panic!("Not implemented");
}
EOF
```

### 5. Retry Start (Precheck Passes This Time)

```bash
jit issue update $ISSUE --state in_progress
```

**Output:**
```
Running prechecks...
✓ tdd-setup passed (exit 0, 0.1s)

Issue abc123 → in_progress
```

### 6. Implement Feature (TDD Green Phase)

```bash
# ... write code to make test pass ...
```

### 7. Complete Work (Postchecks Run)

```bash
jit issue complete $ISSUE
```

**Output:**
```
Running gate checks for issue abc123...
✓ unit-tests passed (exit 0, 2.3s)
  Command: cargo test --lib
  142 tests passed

code-review gate pending (manual)
Issue abc123 → gated (waiting for manual gates)
```

### 8. Manual Review

```bash
jit gate pass $ISSUE code-review --by human:alice
```

**Output:**
```
All gates passed!
Issue abc123 → done
```

## AI Agent Guidelines

### For Lead Agents Creating Issues

```bash
# Always specify appropriate gates based on issue type

# For features: TDD + tests + review
jit issue create --title "New feature" \
  --gate tdd-setup \
  --gate unit-tests \
  --gate code-review

# For bug fixes: tests + review
jit issue create --title "Fix bug" \
  --gate unit-tests \
  --gate code-review

# For refactoring: tests + lint + review
jit issue create --title "Refactor X" \
  --gate unit-tests \
  --gate lint \
  --gate code-review
```

### For Worker Agents Implementing

```bash
# 1. Claim issue
jit issue claim $ISSUE copilot:worker-1

# 2. Try to start work
jit issue update $ISSUE --state in_progress
# → If prechecks fail, fix them first
# → If TDD precheck fails, write tests first

# 3. Do the work
# ... implement feature ...

# 4. Complete work
jit issue complete $ISSUE
# → Postchecks run automatically
# → Fix failures and re-run: jit gate check-all $ISSUE
```

### Understanding Gate Failure

When a gate fails, the output shows:
- **Gate key**: Which gate failed
- **Exit code**: What the checker returned
- **Duration**: How long it took
- **Output**: stdout/stderr from checker
- **Next steps**: How to fix and retry

Example:
```
✗ unit-tests failed (exit 1, 5.2s)
  Command: cargo test --lib
  Error: test test_login ... FAILED
  
  Fix the failing tests and run:
  jit gate check-all abc123
```

## Implementation Plan

### Phase 1: Core Infrastructure
- [ ] Add `stage` field to `Gate` struct (default: `postcheck`)
- [ ] Add `checker` field to `Gate` struct (optional)
- [ ] Implement `GateChecker` struct for command execution
- [ ] Implement command executor with timeout, env vars, working dir
- [ ] Add checker result capture (exit code, stdout, stderr, duration)

### Phase 2: Postcheck Implementation
- [ ] Hook postcheck execution into `complete_issue` command
- [ ] Hook postcheck execution into `update_state(Gated)` transition
- [ ] Auto-transition to `done` if all postchecks pass
- [ ] Add `jit gate check <issue> <gate>` command
- [ ] Add `jit gate check-all <issue>` command
- [ ] Event logging for all auto-checks

### Phase 3: Precheck Implementation
- [ ] Hook precheck execution into `update_state(InProgress)` transition
- [ ] Block transition if any precheck fails
- [ ] Clear error messages for precheck failures
- [ ] Support manual prechecks (must be passed before starting)

### Phase 4: Enhanced CLI
- [ ] Add `jit gate define` command with `--stage`, `--checker-command` flags
- [ ] Add `jit gate test <key>` for testing checkers
- [ ] Add `jit issue complete <id>` convenience command
- [ ] Improve output formatting for check results
- [ ] Add `--json` output for all gate commands

### Phase 5: Documentation & Examples
- [ ] TDD precheck script example
- [ ] Common gate configurations (tests, lint, security)
- [ ] AI agent workflow examples
- [ ] Troubleshooting guide

## Future Enhancements

### Additional Checker Types
- `type: "script"` - Run script file with issue JSON on stdin
- `type: "docker"` - Run checker in container
- `type: "http"` - Call HTTP endpoint with issue data
- `type: "artifact"` - Read result from file (for external CI)

### Advanced Features
- Parallel gate execution
- Gate dependencies (gate B only runs if gate A passes)
- Conditional gates (only run if labels/context match)
- Gate result caching
- Coordinator auto-retry for transient failures
- Gate execution history and trends

### Configuration Options
```json
{
  "gate_execution": {
    "parallel": true,
    "max_concurrent": 4,
    "retry_transient_failures": true,
    "cache_results": false
  }
}
```

## Design Decisions

### 1. Precheck Triggering: On-Demand (Lazy Evaluation)

**Decision:** Prechecks run when agent attempts `ready → in_progress` transition, NOT proactively when dependencies complete.

**Rationale:**
- Keep state model simple (`ready` = dependencies done, no new states needed)
- Avoid wasting resources on unclaimed issues
- Clear retry model: fix issue, retry transition
- "Blocked" state remains clear (blocked by dependencies only)

**Trade-off:** Agents discover precheck failures on attempt, not proactively. This is acceptable because:
- Prechecks should be fast validation (seconds, not minutes)
- Clear error messages guide agents to fix and retry
- Most issues won't have failing prechecks

### 2. Manual Precheck Override

**Decision:** No `--force` flag in initial implementation.

**Rationale:**
- Prechecks enforce process (like TDD) - override defeats purpose
- Can add later if emergency situations require it
- Keeps semantics simple for AI agents

### 3. Gate Templates/Presets

**Decision:** Not in initial implementation, future enhancement.

**Example for future:**
```bash
jit gate define --preset rust-tdd
# Would create: tdd-setup (precheck), unit-tests, lint, clippy
```

### 4. Checker Context Passing

**Decision:** Start with environment variables, add stdin later if needed.

**Current approach:**
```bash
JIT_ISSUE_ID=abc123
JIT_ISSUE_TITLE="Feature X"
JIT_ISSUE_STATE=ready
```

**Future enhancement:** Pass full issue JSON on stdin for complex checkers.

## References

- Original design: `docs/design.md`
- Gate implementation: `crates/jit/src/commands/gate.rs`
- Domain model: `crates/jit/src/domain.rs`
- Roadmap: `ROADMAP.md` (Phase 3 - deferred CI integration)

## State Transition Reference

### All Valid Transitions

| From | To | Trigger | Gate Check |
|------|-----|---------|------------|
| backlog | ready | Dependencies complete | None |
| ready | in_progress | Start work | **PRECHECK** (on-demand) |
| ready | archived | Manual archive | None |
| in_progress | gated | Complete work | **POSTCHECK** (automatic) |
| in_progress | ready | Unclaim (abort work) | None |
| in_progress | archived | Manual archive | None |
| gated | done | All gates pass | Validation only |
| gated | in_progress | Reopen for more work | None |
| gated | archived | Manual archive | None |
| done | archived | Manual archive | None |
| done | in_progress | Reopen (rare) | None |

### State Meanings

| State | Meaning | Typical Duration |
|-------|---------|------------------|
| **backlog** | Has incomplete dependencies | Until dependencies complete |
| **ready** | Dependencies done, can start work | Until claimed/started |
| **in_progress** | Work actively happening | Hours to days |
| **gated** | Work done, awaiting gates | Minutes (auto) to days (manual) |
| **done** | Complete and verified | Permanent |
| **archived** | No longer relevant | Permanent |

### Key Notes

1. **Prechecks are lazy**: Only run when agent attempts to start work
2. **Postchecks are eager**: Run immediately when work is marked complete
3. **"Blocked" is derived**: Not a state, calculated from dependencies
4. **Ready means ready**: Dependencies complete, prechecks validate on start
5. **Gated is temporary**: Auto-transitions to done if all gates pass

## Change Log

- 2025-12-17: Initial design document created
- 2025-12-17: Clarified precheck triggering model (on-demand/lazy evaluation)
  - Prechecks run when transitioning to `in_progress`, not proactively
  - Issue stays in `ready` state if prechecks fail (no new state needed)
  - "Blocked" state remains clear: blocked by dependencies only
- 2025-12-17: Added comprehensive state transition diagrams and reference tables
