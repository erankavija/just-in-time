# Core Model

> **Status:** Draft - Story c8254dbf  
> **Diátaxis Type:** Explanation

## Issues

Issues are the fundamental unit of work in JIT. Everything you track - features, bugs, research questions, learning goals - is represented as an issue.

### What Are Issues?

**Issues** are universal work items that:
- Represent any trackable unit of work (domain-agnostic)
- Serve as the primary entity in the JIT system
- Can be organized hierarchically or independently
- Support arbitrary dependency relationships

**Relationship to traditional systems:**
- Similar to "tickets" (Jira), "issues" (GitHub), "cards" (Trello)
- But more flexible - not tied to software development terminology
- Usable for research, knowledge work, personal projects, any domain

### Core Properties

Every issue has the following properties:

#### ID - Unique Identifier
```
Full: abc12345-6789-4def-1234-567890abcdef (UUID)
Short: abc12345 (first 8 characters, case-insensitive)
```

- **Format**: UUID for global uniqueness
- **Short hash support**: Use minimum 4 characters (like git)
- **Collision-free**: UUIDs prevent conflicts across repositories

**Examples:**
```bash
jit issue show abc12345           # Short hash
jit issue show abc12345-6789      # Longer prefix
jit dep add 003f 9db2             # Minimal (4 chars)
```

#### Title - Human-Readable Summary

Short, descriptive summary of the work (single line):

```
"Fix login redirect bug"
"Add dark mode support"
"Research: Compare database options"
"Learn: Complete Python tutorial"
```

**Best practices:**
- Keep under 80 characters
- Start with verb (action-oriented)
- Be specific enough to differentiate from similar work

#### Description - Detailed Explanation

Markdown-formatted detailed explanation including:
- **What**: Specific work to be done
- **Why**: Context and motivation
- **Acceptance criteria**: How to know it's complete
- **Notes**: Any additional context, links, or constraints

**Example:**
```markdown
## Problem
Users get redirected to /home after login instead of their 
intended destination.

## Solution
Store the intended URL in session before redirecting to login page.
After successful auth, redirect to stored URL or default to /home.

## Acceptance Criteria
- [ ] Pre-login URL captured in session
- [ ] Post-login redirect uses stored URL
- [ ] Falls back to /home if no stored URL
- [ ] Works across browser sessions
```

#### State - Current Lifecycle Position

The issue's current position in the workflow. See [States](#states) for complete state machine.

**Primary states:**
- `backlog` - Created but not ready for work
- `ready` - All dependencies done, can start work
- `in_progress` - Currently being worked on
- `done` - Completed successfully

#### Priority - Importance Level

Four priority levels:
- `critical` - Urgent, blocks other work
- `high` - Important, should be done soon
- `normal` - Default priority
- `low` - Nice to have, do when time permits

**Priority affects:**
- Query ordering (higher priority listed first)
- Agent decision-making (claim higher priority first)
- Work scheduling and planning

**Note:** Priority does not affect state transitions or blocking.

#### Assignee - Current Owner

Who is working on this issue (optional):

**Format**: `{type}:{identifier}`

**Examples:**
```
human:alice          # Human developer
agent:copilot-1      # AI agent instance
ci:github-actions    # CI system
team:backend         # Group assignment
```

See [Assignees](#assignees) for complete specification.

#### Dependencies - Work Order

List of issue IDs that must complete before this issue can be done.

**Semantics:** "This issue depends on those issues"
- Blocks completion (state transition to `done`)
- Does not block starting work (can `in_progress` while dependencies pending)
- Enforces DAG structure (no cycles)

See [Dependencies](#dependencies-vs-labels-understanding-the-difference) for complete explanation.

#### Gates - Quality Requirements

List of gate keys that must pass before issue can progress.

**Prechecks:** Block transition to `in_progress`
**Postchecks:** Block transition to `done`

**Examples:**
```json
"gates_required": ["tests", "code-review", "security-scan"]
```

See [Gates](#gates) for complete gate system.

#### Labels - Organizational Tags

Flexible categorization using `namespace:value` format.

**Common namespaces:**
```
type:task               # Issue type
epic:auth               # Epic membership
milestone:v1.0          # Milestone membership
component:backend       # System component
priority:high           # Alternative to priority field
```

See [Labels](#labels) for complete labeling system.

#### Documents - Attached References

List of design documents, notes, and artifacts linked to this issue.

**Example:**
```json
"documents": [
  {
    "path": "dev/design/auth-design.md",
    "label": "Design Document",
    "doc_type": "design",
    "commit": "abc123..."
  }
]
```

Documents can be versioned via git, archived when work completes, and validated for broken links.

#### Context - Agent Metadata

Flexible key-value storage for agent-specific data (optional).

**Use cases:**
- Store intermediate state during long-running tasks
- Track agent-specific preferences or settings
- Cache computed values across operations

**Example:**
```json
"context": {
  "last_build_time": "2026-02-02T20:00:00Z",
  "retry_count": "2",
  "checkpoint": "step-3-complete"
}
```

### Issue Lifecycle

Issues progress through states as work advances:

```
Creation → Backlog → Ready → In Progress → Done
                        ↓           ↓
                    Gated ←─────────┘
                        ↓
                      Done
```

**1. Creation**
```bash
jit issue create \
  --title "Implement feature X" \
  --description "..." \
  --priority high \
  --label "type:task"
```

New issues start in `backlog` state.

**2. Transition to Ready**

Issues become `ready` when:
- ✓ All dependencies in terminal state (`done` or `rejected`)
- ✓ All precheck gates passed

**3. Work Begins**

```bash
jit issue claim $ISSUE agent:worker-1
# Automatically transitions to in_progress
```

**4. Completion**

```bash
# Check postchecks
jit gate check-all $ISSUE

# Mark complete
jit issue update $ISSUE --state done
```

Issue moves to `gated` if postchecks unpassed, otherwise to `done`.

See [States](#states) for complete state machine details.

### JSON Structure

Issues are stored as JSON files in `.jit/issues/{id}.json`:

```json
{
  "id": "abc12345-6789-4def-1234-567890abcdef",
  "title": "Implement user authentication",
  "description": "Add JWT-based authentication system with...",
  "state": "in_progress",
  "priority": "high",
  "assignee": "agent:worker-1",
  "dependencies": [
    "xyz78901-2345-6abc-7890-def123456789"
  ],
  "gates_required": ["tests", "code-review"],
  "gates_status": {
    "tests": {
      "status": "passed",
      "updated_by": "ci:github-actions",
      "updated_at": "2026-02-02T20:00:00Z"
    },
    "code-review": {
      "status": "pending",
      "updated_by": null,
      "updated_at": "2026-02-01T15:00:00Z"
    }
  },
  "context": {
    "build_status": "success",
    "coverage": "94%"
  },
  "documents": [
    {
      "path": "dev/design/auth-design.md",
      "label": "Authentication Design",
      "doc_type": "design",
      "commit": "a1b2c3d4"
    }
  ],
  "labels": [
    "type:task",
    "epic:auth",
    "milestone:v1.0",
    "component:backend"
  ]
}
```

**Storage guarantees:**
- Atomic writes (write-temp-rename pattern)
- No partial writes from crashes
- JSON validation on read
- Git-optional (works without version control)

### Relationship to Other Concepts

Issues are the central concept that ties together all other JIT features:

**Dependencies** control workflow:
```
Issue A depends on Issue B
  → A cannot complete until B is done
  → Determines what work is available (ready vs blocked)
```

**Gates** ensure quality:
```
Issue requires ["tests", "review"]
  → Cannot complete without passing gates
  → Enforces process and standards
```

**Labels** organize work:
```
Issue has label "epic:auth"
  → Groups related work together
  → Enables filtering and reporting
  → Provides hierarchy and context
```

**Assignees** prevent conflicts:
```
Issue assigned to "agent:worker-1"
  → Indicates ownership
  → Combined with claims for atomicity
  → Enables coordination across agents
```

**States** track progress:
```
Issue state: in_progress
  → Shows current workflow position
  → Determines valid transitions
  → Affects query results (available, blocked, done)
```

### Domain-Agnostic Examples

Issues work for any domain:

**Software Development:**
```json
{
  "title": "Add user registration endpoint",
  "labels": ["type:task", "epic:auth", "component:api"],
  "gates_required": ["tests", "code-review"],
  "priority": "high"
}
```

**Research:**
```json
{
  "title": "Literature review: Neural architecture search",
  "labels": ["type:task", "project:nas-research", "phase:background"],
  "gates_required": ["peer-review"],
  "priority": "normal"
}
```

**Knowledge Work:**
```json
{
  "title": "Learn: Complete Rust async programming chapter",
  "labels": ["type:task", "goal:learn-rust", "topic:async"],
  "gates_required": ["exercises-complete"],
  "priority": "low"
}
```

**Project Management:**
```json
{
  "title": "Finalize Q1 budget proposal",
  "labels": ["type:task", "milestone:q1-planning", "team:finance"],
  "gates_required": ["manager-approval", "stakeholder-review"],
  "priority": "critical"
}
```

### See Also

- [States](#states) - Complete state machine and transitions
- [Dependencies](#dependencies-vs-labels-understanding-the-difference) - Workflow control with DAG
- [Gates](#gates) - Quality enforcement and process integration
- [Labels](#labels) - Organizational taxonomy
- [Assignees](#assignees) - Ownership and coordination
- [System Guarantees](guarantees.md) - Atomicity, consistency, failure handling

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

Labels provide organizational membership using `namespace:value` format for filtering and grouping.

### Label Format (CRITICAL)

**ALL labels MUST use**: `namespace:value`

```
✅ CORRECT:
  type:task, epic:auth, milestone:v1.0, component:backend

❌ WRONG:
  auth (missing namespace)
  milestone-v1.0 (wrong separator)
  Type:task (uppercase namespace)
  type: task (space after colon)
```

**Validation rules:**
- Namespace: lowercase, no spaces, alphanumeric + hyphens
- Value: any characters (allows spaces, uppercase)
- Separator: exactly one colon (`:`)
- No leading/trailing whitespace

### Required Labels

**Every issue must have exactly ONE**:
```
type:*
├─ type:milestone    Release/time-bound goal
├─ type:epic         Large feature  
├─ type:task         Concrete work item
├─ type:research     Time-boxed investigation
└─ type:bug          Defect to fix
```

**Exception:** Use `--orphan` flag to explicitly allow issues without type label.

### Common Label Namespaces

**Organization labels** (optional but recommended):
```
milestone:*    Groups work under releases (e.g., milestone:v1.0)
epic:*         Groups tasks under features (e.g., epic:auth)
component:*    Technical area (e.g., component:backend, component:web)
priority:*     (Built-in, not a label) - critical, high, normal, low
```

**Workflow labels:**
```
needs-review:true      Requires review
blocked-by:external    External dependency
resolution:duplicate   Why rejected
```

**Strategic labels** (high-level organization):

JIT defines certain namespaces as "strategic" in configuration:
- `milestone:*` - Release grouping
- `epic:*` - Feature grouping
- `goal:*` - Objective grouping
- `theme:*` - Initiative grouping

Strategic issues (those with strategic labels) appear in special queries:
```bash
# Find all strategic issues
jit query strategic
```

### Label Usage

**Creating issues with labels:**
```bash
# Single label
jit issue create --title "Fix bug" --label "type:bug"

# Multiple labels
jit issue create \
  --title "Implement login" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend"
```

**Adding labels to existing issues:**
```bash
# Add single label
jit issue update abc123 --label "needs-review:true"

# Add multiple labels
jit issue update abc123 \
  --label "component:frontend" \
  --label "priority:high"
```

**Removing labels:**
```bash
# Remove single label
jit issue update abc123 --remove-label "needs-review:true"

# Remove multiple labels
jit issue update abc123 \
  --remove-label "milestone:v0.9" \
  --remove-label "component:legacy"
```

### Label Queries

**Exact match:**
```bash
# Find all auth epic issues
jit query all --label "epic:auth"

# Find all v1.0 milestone issues
jit query all --label "milestone:v1.0"
```

**Wildcard (namespace match):**
```bash
# Find all issues with ANY milestone
jit query all --label "milestone:*"

# Find all issues with ANY epic
jit query all --label "epic:*"

# Find all component-tagged issues
jit query all --label "component:*"
```

**Boolean queries:**
```bash
# Complex filters
jit query all --filter "label:epic:auth AND label:component:backend"
jit query all --filter "label:milestone:v1.0 OR label:milestone:v1.1"
jit query all --filter "label:type:task AND NOT label:epic:*"
```

### Label vs Dependency Semantics

See [Dependencies vs Labels](#dependencies-vs-labels-understanding-the-difference) section above for detailed comparison.

**Summary:**
- **Labels** = Organizational membership (grouping)
- **Dependencies** = Execution order (workflow)
- Both often flow same direction, but different purposes
- Use both for maximum clarity

**Example:**
```bash
# Task belongs to auth epic (label)
jit issue create --title "JWT utils" --label "epic:auth"

# Epic requires task to complete (dependency)
jit dep add <epic-id> <task-id>

# Query by label: "epic:auth" → Shows all auth work
# Query ready: → Shows task if unblocked, epic if task done
```

### Label Namespaces Discovery

**List all namespaces in use:**
```bash
jit label namespaces
# Output:
# type
# epic
# milestone
# component
# needs-review
```

**List all values for a namespace:**
```bash
jit label values milestone
# Output:
# v0.9
# v1.0
# v1.1

jit label values epic
# Output:
# auth
# api
# web-ui
```

These commands help discover existing labels without manual inspection.

## Assignees

Assignees indicate who (human or agent) is actively working on an issue. JIT supports atomic claiming to prevent race conditions in multi-agent scenarios.

### Assignee Format

**All assignees use**: `{type}:{identifier}`

**Supported types:**
- `human:{name}` - Human developer (e.g., `human:alice`, `human:bob`)
- `agent:{id}` - AI agent (e.g., `agent:copilot-session-1`, `agent:worker-2`)
- `bot:{name}` - Automated bot (e.g., `bot:dependabot`, `bot:automation`)

**Examples:**
```bash
# Human assignee
jit issue assign abc123 human:alice

# Agent assignee
jit issue claim abc123 agent:worker-1

# Bot assignee
jit issue assign abc123 bot:ci-automation
```

### Claim Semantics (Atomic Operations)

**Why atomic claiming matters:**

In multi-agent scenarios, multiple agents may query for ready work simultaneously. Without atomic operations, race conditions occur:

```
Time   Agent 1                Agent 2
-----------------------------------------
T1     query ready → [abc123]
T2                           query ready → [abc123]
T3     claim abc123 ✓
T4                           claim abc123 ✓  ← Duplicate work!
```

**JIT's solution: File-based atomic claiming**

Claiming uses atomic file operations (rename is atomic in POSIX):
1. Read issue file
2. Verify no assignee exists
3. Write temp file with new assignee
4. **Atomic rename** (succeeds for one agent, fails for others)

```bash
# Both agents try simultaneously
Agent 1: jit issue claim abc123 agent:worker-1  # ✓ Succeeds
Agent 2: jit issue claim abc123 agent:worker-2  # ✗ Fails: already claimed
```

### Claim vs Assign

**`jit issue claim`** - Atomic operation (race-safe)
- Verifies issue is unassigned
- Claims for specified assignee
- Returns error if already assigned
- Use for multi-agent coordination

**`jit issue assign`** - Force assignment (overwrites)
- Assigns regardless of current state
- Can reassign from one agent to another
- Use for manual intervention

**Examples:**
```bash
# Agent claims (atomic, safe)
jit issue claim abc123 agent:worker-1
# Error if already claimed

# Human reassigns (force, override)
jit issue assign abc123 human:alice
# Succeeds even if claimed by agent
```

### Claim Next Ready Issue

For agents that just want "next available work":

```bash
# Claim next ready issue by priority
jit issue claim-next agent:worker-1

# With filter
jit issue claim-next agent:worker-1 --filter "label:epic:auth"
```

**Behavior:**
1. Queries ready issues (unassigned, state=ready, unblocked)
2. Sorts by priority (critical → high → normal → low)
3. Atomically claims first available issue
4. Returns claimed issue ID

**Race handling:**
- If multiple agents claim-next simultaneously, each gets different issue
- Atomic file operations ensure no duplicates
- If no ready issues, returns error

### Release Semantics

Agents can release issues they cannot complete:

```bash
# Release an issue
jit issue release abc123 --reason "timeout"
```

**Behavior:**
- Clears assignee
- Adds event to audit log
- Issue becomes available for other agents
- Reason recorded for observability

**Common reasons:**
- `timeout` - Exceeded time budget
- `error` - Encountered blocking error
- `reassign` - Redirecting to different agent
- `manual` - Human intervention required

### Unassign

Simpler alternative when no reason needed:

```bash
# Clear assignee
jit issue unassign abc123
```

Equivalent to `assign` with no assignee value.

### Multi-Agent Coordination Patterns

**Pattern 1: Decentralized Polling**
```bash
# Each agent independently polls and claims
while true; do
  # Claim next ready work atomically
  ISSUE=$(jit issue claim-next agent:worker-$ID --json | jq -r '.data.id')
  
  if [ -n "$ISSUE" ]; then
    # Do work...
    work_on_issue "$ISSUE"
    
    # Complete when done
    jit issue update "$ISSUE" --state done
  else
    # No work available, wait
    sleep 10
  fi
done
```

**Pattern 2: Filtered Work Distribution**
```bash
# Agent 1: Backend specialist
jit issue claim-next agent:backend-specialist --filter "label:component:backend"

# Agent 2: Frontend specialist
jit issue claim-next agent:frontend-specialist --filter "label:component:frontend"

# Agent 3: Generalist
jit issue claim-next agent:generalist
```

**Pattern 3: Priority-Based Assignment**
```bash
# High-priority agent gets critical work
jit issue claim-next agent:priority-worker --filter "priority:critical OR priority:high"

# Low-priority agent gets normal work
jit issue claim-next agent:background-worker --filter "priority:normal OR priority:low"
```

**Pattern 4: Timeout and Recovery**
```bash
# Work on issue with timeout
ISSUE=$(jit issue claim-next agent:worker-1)
timeout 300 work_on_issue "$ISSUE" || {
  # Timeout exceeded, release for others
  jit issue release "$ISSUE" --reason "timeout"
}
```

### Current Limitations and Future Directions

**Current capabilities:**
- Atomic claiming via file operations
- Decentralized polling (no coordinator daemon)
- Simple assignee format with type prefix
- Manual release on timeout/error

**Potential future enhancements** (not yet implemented):
- **Coordinator daemon** (`jit-dispatch`) - Central work distributor with:
  - Active push to agents (no polling)
  - Health monitoring and automatic reassignment
  - Load balancing across agents
  - Stalled work detection
  - Agent capability matching
- **Assignee priorities** - Preferred agent for issue types
- **Work-in-progress limits** - Max concurrent issues per agent
- **Agent heartbeats** - Detect crashed agents
- **Automatic timeout** - Release after inactivity threshold

**Note:** Current design works well for 1-10 agents polling every 10-30 seconds. Coordinator daemon would optimize for larger agent pools (10-100 agents) with lower latency requirements.

For practical coordination examples, see [How-To: Software Development](../how-to/software-development.md#multi-agent-workflows).
