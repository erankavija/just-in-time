# CI Gate Integration Design

**Status**: Planning  
**Date**: 2025-12-17  
**Version**: 1.1  
**Last Updated**: 2025-12-18

## Overview

Add automated checking capabilities to the quality gate system, enabling both manual checklist-style gates and automated script execution. The system must be simple and intuitive for AI agents to understand and use.

## Design Philosophy

### Simple v1, Future-Proof Foundation

**Core principle**: Single concept, stable contracts
- One "gate" abstraction with versioned schema
- One "result" schema for all outcomes
- Add capabilities by extending, not changing contracts
- Issue-first today, commit-aware for future PR/branch protection

**Intentional v1 Scope (Keep It Simple)**:
- Only `exec` checker type (shell commands)
- Sequential execution (no parallelism or ordering)
- Two stages: precheck and postcheck
- Basic timeouts and output capture
- Clear warning: "this runs commands in your environment"

**Non-Goals for v1** (add later if needed):
- No parallel gate execution or dependency ordering
- No conditional gates or matrices
- No sandboxing/containers (just timeouts)
- No RBAC beyond "manual pass/fail with --by"
- No background schedulers (trigger on state changes only)

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

## Gate Definition Schema (Versioned, Stable)

```json
{
  "version": 1,
  "gates": {
    "unit-tests": {
      "version": 1,
      "key": "unit-tests",
      "title": "Unit Tests",
      "description": "Run project unit tests",
      "stage": "postcheck",
      "mode": "auto",
      "checker": {
        "type": "exec",
        "command": "cargo test --lib",
        "timeout_seconds": 300,
        "working_dir": ".",
        "env": {
          "RUST_BACKTRACE": "1"
        }
      },
      "reserved": {}
    },
    "code-review": {
      "version": 1,
      "key": "code-review",
      "title": "Code Review",
      "description": "Human review of code quality",
      "stage": "postcheck",
      "mode": "manual",
      "reserved": {}
    }
  }
}
```

**Field semantics for AI agents:**
- `version`: Schema version (enables future evolution without breaking changes)
- `key`: Unique identifier (stable, treat as API)
- `stage`: When does this gate run? `precheck` = before work, `postcheck` = after work
- `mode`: Can a computer pass this gate automatically? `auto` = yes (has checker), `manual` = no (human only)
- `checker`: Present only if `mode: auto`, defines how to automatically check
- `checker.type`: Always `"exec"` in v1 (future: `"docker"`, `"http"`, `"artifact"`)
- `reserved`: Empty object for future extensions without schema breaking

**Future-proofing:**
- Adding new checker types: Just add new `type` values, old `exec` checkers still work
- Adding gate dependencies: Add `depends_on: []` field later, existing gates ignore it
- Adding conditionals: Add `when: {}` field later, use `status: "skipped"` in results
- Schema evolution: Bump `version`, handle both old and new formats

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

## Gate Result Schema (Normalized, Commit-Aware)

Every gate execution produces a structured result, stored for audit and future analysis:

```json
{
  "schema_version": 1,
  "run_id": "01HQZX...",
  "gate_key": "unit-tests",
  "stage": "postcheck",
  "subject": {
    "type": "issue",
    "repo": "erankavija/just-in-time",
    "issue_id": "abc123",
    "commit": "7f8a9b2c...",
    "branch": "feature/auth"
  },
  "status": "passed",
  "started_at": "2025-12-18T12:00:00Z",
  "completed_at": "2025-12-18T12:00:02Z",
  "duration_ms": 2345,
  "executor": {
    "mode": "auto",
    "runner_id": "local",
    "env_profile": "default"
  },
  "evidence": {
    "exit_code": 0,
    "stdout_path": ".jit/gate-runs/01HQZX.../stdout.log",
    "stderr_path": ".jit/gate-runs/01HQZX.../stderr.log",
    "command": "cargo test --lib"
  },
  "by": "auto:runner-1",
  "message": "All 142 tests passed",
  "reserved": {}
}
```

**Status values:**
- `passed`: Check succeeded
- `failed`: Check failed (expected failure, e.g., tests failed)
- `error`: Unexpected error (timeout, command not found, crash)
- `skipped`: Not applicable (future: for conditional gates)
- `pending`: Not yet run (for manual gates)

**Subject field (commit-aware)**:
- Records Git commit and branch even in v1
- Enables future PR/commit-centric workflows without schema changes
- Issue-first today, but data model supports more tomorrow

**Storage**:
- Gate run results: `.jit/gate-runs/<run-id>/`
  - `result.json` - structured result
  - `stdout.log` - command output
  - `stderr.log` - error output

**Future extensions:**
- PR/branch protection: Already recording commit/branch
- Multi-subject runs: `subject.type` can become `"pr"`, `"commit"`, etc.
- External CI ingestion: Same schema for artifact/http checker results

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

### Phase 1: MVP - Core Infrastructure (Simple & Stable)

**Scope**: Minimum viable product with stable contracts

- [ ] **Data Model**
  - Add `version` field to Gate struct (default: 1)
  - Add `mode` field: `"manual"` or `"auto"` (replaces `auto: bool`)
  - Add `reserved` field (empty HashMap) for future extensions
  - Implement GateRunResult struct with full schema above
  - Store run results in `.jit/gate-runs/<run-id>/`

- [ ] **Checker Infrastructure**
  - Implement `exec` checker only (type: `"exec"`)
  - Command executor with timeout, working directory, env vars
  - Capture exit code, stdout, stderr, duration
  - Per-issue lock to prevent concurrent runs
  - Record Git commit/branch if available (for future-proofing)

- [ ] **Postcheck Implementation**
  - Hook into `complete_issue` command
  - Hook into `update_state(Gated)` transition
  - Sequential execution (no parallelism)
  - Auto-transition to `done` if all postchecks pass
  - Store structured results in `.jit/gate-runs/`

- [ ] **Precheck Implementation**
  - Hook into `update_state(InProgress)` transition
  - Block transition if any precheck fails
  - Clear error messages showing which checks failed
  - Support manual prechecks (must be passed before starting)

- [ ] **CLI Commands**
  - `jit gate define <key>` with `--stage`, `--mode`, `--checker-command`
  - `jit gate list` and `jit gate show <key>`
  - `jit gate check <issue> <gate>` - run single gate
  - `jit gate check-all <issue>` - run all auto gates
  - `jit gate pass/fail <issue> <gate> --by <who>` - manual gates
  - All commands support `--json` for machine-readable output
  - Clear warning: "This runs commands in your environment"

- [ ] **Security & Safety**
  - Timeout enforcement (kill process after timeout)
  - Clear warning on first gate definition
  - No sandboxing in v1 (document limitation)
  - Log all command executions to event log

- [ ] **Testing**
  - Unit tests for checker execution
  - Integration tests for state transitions
  - Property tests for concurrent gate execution prevention
  - Test timeout handling
  - Test result persistence and retrieval

### Phase 2: Agent-Friendly Additions (Still Simple)

- [ ] **Precheck Preview**
  - `jit gate preview <issue-id>` - run prechecks without state change
  - Cache result for 5 minutes
  - Gives agents discoverability without triggering lazy eval
  - Returns `--json` with predicted precheck results

- [ ] **Enhanced Observability**
  - `jit gate history <issue>` - show all gate runs for an issue
  - `jit gate runs <gate-key>` - show all runs of a specific gate
  - Filter by status, time range

- [ ] **Improved Error Messages**
  - Show relevant output excerpts on failure
  - Suggest fixes based on common patterns
  - Link to full logs in `.jit/gate-runs/`

### Phase 3: Advanced Features (Future Extensions)

**Only implement if needed:**

- [ ] Parallel gate execution (add `allow_parallel` field)
- [ ] Gate dependencies (add `depends_on` field)
- [ ] Conditional gates (add `when` conditions, use `skipped` status)
- [ ] Docker checker type (`{"type": "docker", "image": "..."}`)
- [ ] HTTP checker type (`{"type": "http", "url": "..."}`)
- [ ] Artifact checker type (`{"type": "artifact", "path": "..."}`)
- [ ] Gate templates/presets (`jit gate define --preset rust-tdd`)
- [ ] Coordinator auto-retry for transient failures
- [ ] Multi-approver gates (RBAC layer)
- [ ] PR/branch protection integration

## Minimal Internal Data Model

**Keep it simple, make it stable:**

```rust
// Gate definition (versioned)
pub struct Gate {
    pub version: u32,           // Schema version (default: 1)
    pub key: String,            // Stable identifier
    pub title: String,
    pub description: String,
    pub stage: GateStage,       // precheck | postcheck
    pub mode: GateMode,         // manual | auto
    pub checker: Option<GateChecker>,
    pub reserved: HashMap<String, serde_json::Value>,  // Future extensions
}

pub enum GateStage {
    Precheck,
    Postcheck,
}

pub enum GateMode {
    Manual,
    Auto,
}

// Checker (typed, extensible)
pub struct GateChecker {
    pub checker_type: CheckerType,
    // Type-specific fields
    pub command: Option<String>,        // For exec type
    pub timeout_seconds: u64,
    pub working_dir: Option<String>,
    pub env: HashMap<String, String>,
}

pub enum CheckerType {
    Exec,
    // Future: Docker, Http, Artifact
}

// Gate run result (normalized, commit-aware)
pub struct GateRunResult {
    pub schema_version: u32,
    pub run_id: String,
    pub gate_key: String,
    pub stage: GateStage,
    pub subject: GateSubject,
    pub status: GateRunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub executor: ExecutorInfo,
    pub evidence: ExecutionEvidence,
    pub by: Option<String>,
    pub message: Option<String>,
    pub reserved: HashMap<String, serde_json::Value>,
}

pub struct GateSubject {
    pub subject_type: String,    // "issue" (future: "pr", "commit")
    pub repo: String,
    pub issue_id: String,
    pub commit: Option<String>,  // Git SHA (future-proofing)
    pub branch: Option<String>,  // Git branch (future-proofing)
}

pub enum GateRunStatus {
    Passed,
    Failed,
    Error,
    Skipped,   // Future: for conditional gates
    Pending,
}

pub struct ExecutionEvidence {
    pub exit_code: Option<i32>,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub command: Option<String>,
}
```

**Storage layout:**
```
.jit/
  gates.json              # Gate definitions
  gate-runs/              # Run results
    01HQZX.../
      result.json         # Structured result
      stdout.log          # Command output
      stderr.log          # Error output
  issues/                 # Issue files (existing)
    abc123.json
      "gates_required": ["unit-tests", "review"]
      "gates_status": {
        "unit-tests": {
          "status": "passed",
          "last_run_id": "01HQZX...",
          "updated_by": "auto:runner-1",
          "updated_at": "2025-12-18T12:00:00Z"
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
- Roadmap: `ROADMAP.md` (Phase 5 - Quality Gate System)

## Practical Examples

See [gate-examples.md](gate-examples.md) for comprehensive examples including:
- TDD workflow with quality checks (Rust, Python, JavaScript)
- Context validation patterns (manual checklists, automated validation)
- Security-focused gates (audit, secret detection, SAST)
- Performance gates (benchmarks, binary size)
- Quick setup templates (minimal, standard, comprehensive)

**Key Philosophy**: Prechecks remind, postchecks enforce.

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
- 2025-12-18: Refined design for simplicity and future-proofing
  - Versioned, stable schemas for gates and results
  - Commit-aware subject field for future PR/branch workflows
  - Typed checker interface (only `exec` in v1, extensible later)
  - Simplified scope: no parallelism, dependencies, or conditionals in v1
  - Clear MVP scope vs. future extensions
  - Structured result storage in `.jit/gate-runs/`
