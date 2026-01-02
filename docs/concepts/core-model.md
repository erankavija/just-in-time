# Core Model

> **Status:** Draft - Story c8254dbf  
> **Diátaxis Type:** Explanation

## Issues

<!-- What are issues? Properties, lifecycle -->

## Dependencies vs Labels: Understanding the Difference

Dependencies and labels both organize work, but serve fundamentally different purposes. They often flow in the same direction (task → epic → milestone) but have distinct semantics.

### Labels: What Belongs Where (Grouping)

Labels provide **organizational membership** using `namespace:value` format:

```
Task: Implement Login
  label: "epic:auth"     → This task belongs to auth epic
  label: "type:task"     → This is a task-level work item
  label: "component:api" → This affects the API component
```

**Key properties:**
- Hierarchical grouping (milestone > epic > task)
- Multiple labels allowed (one per namespace if unique)
- Used for filtering and reporting
- Static relationship (doesn't change based on state)

### Dependencies: What Blocks What (Workflow)

Dependencies create **execution order** in a directed acyclic graph (DAG):

```
Epic: Auth System
  depends on: [Login Task, Password Task, Session Task]
  → Epic cannot complete until all tasks are done
```

**Key properties:**
- Arbitrary DAG structure (not limited to hierarchy)
- Determines what work is available (ready vs blocked)
- Dynamic relationship (affects state transitions)
- Transitive reduction (minimal edges preferred)

### Same Direction, Different Meanings

```
┌─────────────────────────────────────────────────────────┐
│                Task: Implement Login                    │
│                                                         │
│  Label: "epic:auth"                                    │
│  └─→ "This task belongs to auth epic" (membership)    │
│                                                         │
│  Dependency of: Epic                                    │
│  └─→ "Epic requires this task to complete" (order)    │
└─────────────────────────────────────────────────────────┘
                         ↓ flows into
┌─────────────────────────────────────────────────────────┐
│                Epic: Auth System                        │
│                                                         │
│  Label: "milestone:v1.0"                               │
│  └─→ "This epic belongs to v1.0" (membership)         │
│                                                         │
│  Dependency of: Milestone                               │
│  └─→ "Milestone requires this epic" (order)           │
└─────────────────────────────────────────────────────────┘
```

Both flow the same way, but labels organize while dependencies control workflow.

### Asymmetry: Dependencies Are More Flexible

Labels follow strict hierarchy (task → epic → milestone), but dependencies allow arbitrary DAG relationships:

```
Sequential releases (dependencies work, labels don't):

v1.0 Release (completed) ──→ blocks ──→ v2.0 Planning Task

Valid dependency: Future work waits for past release
Invalid label: v1.0 cannot "belong to" v2.0 task
```

### When to Use What

**Use Labels:**
- Organizing related work into groups
- Filtering by scope or domain
- Reporting progress by epic/milestone
- Querying specific subsets

**Use Dependencies:**
- Enforcing work order (A must complete before B)
- Blocking work until prerequisites ready
- Determining what's available to work on
- Controlling state transitions

**Use Both (Common Pattern):**
Most workflows use both for maximum clarity:

```
Task: Login Endpoint
  labels: ["epic:auth", "type:task", "component:backend"]
  dependencies: []  # No blockers, can start immediately

Epic: Auth System
  labels: ["milestone:v1.0", "type:epic"]
  dependencies: [Login Task, Password Task, Session Task]

Query by label: "epic:auth" → Shows all auth work
Query ready: → Shows Login Task (epic blocked by dependency)
```

## Dependencies

<!-- DAG model, blocking, transitive reduction -->

## Gates

Quality gates are checkpoints that enforce process requirements before issues can progress through the workflow.

### What Are Gates?

**Gates** are quality control mechanisms that:
- Define quality standards for work completion
- Automate or remind about process steps
- Prevent premature completion of incomplete work
- Integrate quality checks directly into workflow

**Mental model:** Gates are like guardrails on a road - they keep work on track and prevent accidents (bugs, technical debt, incomplete features).

### Gate Lifecycle

Gates exist in three states:

1. **Required** - Gate is attached to an issue but not yet checked
2. **Passed** - Gate check succeeded (automated) or approved (manual)
3. **Failed** - Gate check failed (automated only)

**State transitions:**
```
Required → Passed    (check succeeds or manual approval)
Required → Failed    (automated check fails)
Failed → Passed      (fix issue, re-run check)
```

### Gate Types: Prechecks vs Postchecks

Gates run at two stages in the issue lifecycle:

**Prechecks** - Run before work begins (`backlog/ready → in_progress`)
- Verify prerequisites met
- Remind about process (e.g., TDD: write tests first)
- Validate approach before implementation

**Postchecks** - Run before completion (`in_progress → done`)
- Verify quality standards (tests pass, linting clean)
- Require reviews or approvals
- Validate deliverables complete

**Example workflow:**
```
backlog → [PRECHECK: tdd-reminder] → in_progress
          ↓
          work happens
          ↓
in_progress → [POSTCHECK: tests, clippy, code-review] → gated → done
```

### Gate Modes: Manual vs Automated

**Manual Gates** - Require human judgment
- Examples: code review, design approval, security audit
- Passed explicitly: `jit gate pass $ISSUE code-review --by human:alice`
- Used for subjective quality checks

**Automated Gates** - Run programmatic checks
- Examples: tests, linters, builds, security scans
- Run automatically: `jit gate check $ISSUE tests`
- Used for objective, repeatable verification
- Require checker command and timeout configuration

### Gate Status Tracking

Each gate on an issue tracks:
- **Status**: required, passed, or failed
- **Updated by**: Who/what passed the gate (e.g., `human:alice`, `ci:github-actions`)
- **Updated at**: Timestamp of last status change

**Query gate status:**
```bash
jit issue show $ISSUE --json | jq '.data.gates_status'
```

### Gate Enforcement and Auto-Transitions

Gates integrate with the state machine to enforce quality:

**Attempting to complete work:**
```bash
jit issue update $ISSUE --state done
```

**If all gates passed:**
- Issue transitions directly to `done`

**If any gates not passed:**
- Issue transitions to `gated` (waiting for gate approval)
- Clear error message shows which gates are blocking
- Auto-transitions to `done` when last gate passes

**Example:**
```bash
$ jit issue update abc123 --state done
Error: Gate validation failed: Cannot transition to 'done' - 2 gate(s) not passed: tests, code-review
→ Issue automatically transitioned to 'gated' (awaiting gate approval)

# Fix and pass gates
$ jit gate check abc123 tests
✓ tests passed

$ jit gate pass abc123 code-review --by human:alice
✓ code-review passed
→ Issue automatically transitioned to 'done' (all gates passed)
```

### Gate Bypass for Terminal States

**Critical design property:** Transitioning to `rejected` bypasses all gate enforcement.

**Rationale:**
- Issues can be rejected at any time (duplicate found, requirements changed)
- Requiring gates to pass before rejecting doesn't make semantic sense
- `rejected` is an escape hatch for "this work won't happen"

**Example:**
```bash
# Issue has failing gates
jit issue update $ISSUE --state done
# Error: Gate validation failed

# But can always reject
jit issue reject $ISSUE --reason "duplicate"
# Success - bypasses gates
```

Terminal state `done` requires gates, but `rejected` does not.

### Gate Registry

Gates are defined globally in the **gate registry** (`.jit/gates.json`):
- Each gate has a unique key (e.g., `tests`, `code-review`)
- Gate definitions are reusable across issues
- Changes to gate definitions don't affect existing gate status

**Define once, use many times:**
```bash
# Define in registry
jit gate define tests --title "Tests Pass" --mode auto --checker-command "cargo test"

# Apply to multiple issues
jit issue update --filter "epic:auth" --add-gate tests
```

### Relationship to State Machine

Gates influence state transitions:

```
                    ┌──────────┐
                    │ Backlog  │
                    └────┬─────┘
                         │
                    [prechecks]
                         │
                         v
                    ┌──────────┐
                    │  Ready   │
                    └────┬─────┘
                         │
                         v
                  ┌─────────────┐
                  │ In Progress │
                  └──────┬──────┘
                         │
                    [postchecks]
                         │
                         v
                    ┌─────────┐
                    │  Gated  │ ← Waiting for gates
                    └────┬────┘
                         │
                  [all gates pass]
                         │
                         v
                    ┌────────┐
                    │  Done  │
                    └────────┘
```

**Key behaviors:**
- **Prechecks** gate entry to `in_progress`
- **Postchecks** gate entry to `done`
- **Gated state** exists specifically for gate waiting
- **Auto-transition** from `gated → done` when gates pass

### Design Philosophy

**Gates encode process, not just validation:**
- Manual gates remind about important steps (write tests first)
- Automated gates enforce quality standards (tests pass)
- Together, they create a workflow that's hard to shortcut

**Gates are optional and flexible:**
- Issues can have zero gates (simple tracking)
- Issues can have many gates (strict quality control)
- Gates are defined per-issue (different standards for different work)

**Gates provide transparency:**
- Clear why work is blocked (which gates need to pass)
- Audit trail of who approved what (gate status history)
- Programmatic queryability (find issues awaiting specific gates)

### Current Limitations and Future Directions

**Current capabilities:**
- Gates apply uniformly at state transitions
- Checker commands run in shell with simple pass/fail
- Gate status is binary: required/passed/failed

**Potential future enhancements** (not yet implemented):
- **Conditional gates**: Apply gates based on issue properties (e.g., only require security-scan for epic:auth issues)
- **Gate dependencies**: Gates that must pass in specific order
- **Parallel gate execution**: Run multiple automated gates concurrently for speed
- **Rich gate output**: Structured results beyond exit codes (metrics, warnings, artifacts)
- **Gate templates**: Pre-configured gate sets for common workflows
- **Per-label gate policies**: Different gate requirements based on issue labels

**Note:** Current design intentionally keeps gates simple and flexible. These extensions would be added based on real-world usage patterns, maintaining backward compatibility.

For domain-specific gate examples beyond software development, see [Custom Gates - Beyond Software Development](../how-to/custom-gates.md#beyond-software-development).

## States

Issues progress through a lifecycle with the following states:

### State Machine

```
       ┌─────────┐
       │ Backlog │ (default initial state)
       └────┬────┘
            │
            v
       ┌─────────┐
       │  Ready  │ (dependencies satisfied, no assignee)
       └────┬────┘
            │
            v
    ┌──────────────┐
    │ In Progress  │ (assignee claimed)
    └──────┬───────┘
           │
           v
    ┌──────────┐
    │  Gated   │ (waiting for gates to pass)
    └────┬─────┘
         │
         v
    ┌────────┐
    │  Done  │ (terminal - successfully completed)
    └────────┘
    
    From any state:
         │
         v
    ┌──────────┐
    │ Rejected │ (terminal - won't implement)
    └──────────┘
```

### State Descriptions

**Backlog**: Issue is not yet ready to work on. Dependencies are incomplete or issue is explicitly marked as future work.

**Ready**: Issue is unblocked (all dependencies satisfied), has no assignee, and is available to claim. This is the state agents query to find work.

**In Progress**: Issue is actively being worked on by an assignee.

**Gated**: Issue has attempted to transition to Done, but quality gates have not all passed. Issue auto-transitions to Done when all required gates pass.

**Done**: Terminal state indicating successful completion. Issue cannot transition out of this state.

**Rejected**: Terminal state indicating the issue was closed without implementation. Common reasons: duplicate, won't-fix, invalid, out-of-scope.

### Terminal States

JIT has two terminal states that represent different outcomes:

**Done** - Work was successfully completed
- All gates passed
- Implementation delivered
- Issue fulfilled its purpose

**Rejected** - Work was not completed
- Closed without implementation
- Common reasons: duplicate, won't-fix, invalid, out-of-scope
- Optional `resolution:*` label provides closure reason

Once an issue reaches a terminal state (Done or Rejected), it cannot transition to any other state.

### State Transitions

**Auto-transitions:**
- `Backlog → Ready`: When all dependencies complete
- `Gated → Done`: When all required gates pass

**Manual transitions:**
- `Ready → In Progress`: Via `jit issue claim` or `jit issue assign`
- `In Progress → Done`: Via `jit issue update --state done` (if gates allow)
- `In Progress → Gated`: Automatic when transitioning to Done with unmet gates
- `Any State → Rejected`: Via `jit issue reject` (bypasses gates)

### Gate Bypass for Rejected

**Critical design property:** Transitioning to `Rejected` bypasses all gate enforcement.

**Rationale:**
- Issues can be rejected at any time (duplicate discovered, requirements changed, etc.)
- Requiring gates to pass before rejecting doesn't make sense
- `Rejected` is an escape hatch for "this work won't happen"

**Example:**
```bash
# Issue has failing gates, cannot transition to Done
jit issue update $ISSUE --state done
# Error: Gate validation failed

# But can always reject
jit issue reject $ISSUE --reason "duplicate"
# Success - bypasses gates
```

### Resolution Labels

When rejecting issues, optionally add `resolution:*` labels to document why:

**Common resolution labels:**
- `resolution:duplicate` - Duplicate of another issue
- `resolution:wont-fix` - Valid request, but won't implement
- `resolution:invalid` - Not a valid issue
- `resolution:out-of-scope` - Outside project scope
- `resolution:obsolete` - No longer relevant

**Usage:**
```bash
jit issue reject $ISSUE --reason "duplicate"
# Adds label: resolution:duplicate
```

## Labels

<!-- Namespace:value format, hierarchy, strategic vs tactical -->

## Assignees

<!-- Format, types, claim/release semantics -->
