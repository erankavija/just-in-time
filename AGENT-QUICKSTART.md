# Agent Quick Start

**Goal:** Get productive as an AI agent in 5 minutes. Understand JIT structure, label conventions, and MCP tools.

## 1. Core Concepts (2 minutes)

JIT is a **CLI-first issue tracker** designed for **AI agents**:
- **Dependency DAG** - "Task B needs Task A done first"
- **Quality Gates** - "Tests must pass before marking done"
- **Agent-friendly** - JSON output, atomic operations, MCP tools
- **Hierarchical labels** - `namespace:value` format for organization

Everything is in `.jit/` (like `.git/`). Plain JSON files. Version controlled.

### Issue States
```
open → ready → in_progress → done
         ↓
      blocked (has incomplete dependencies or failing gates)
```

### Label Format (CRITICAL)

**ALL labels MUST use**: `namespace:value`

```
✅ CORRECT:
  type:task, epic:auth, milestone:v1.0, component:backend

❌ WRONG:
  auth (missing namespace)
  milestone-v1.0 (wrong separator)
  Type:task (uppercase namespace)
```

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

**Organization labels** (optional but recommended):
```
milestone:*    Groups work under releases (e.g., milestone:v1.0)
epic:*         Groups tasks under features (e.g., epic:auth)
component:*    Technical area (e.g., component:backend, component:web)
```

---

## 2. Find Your Next Task (1 minute)

### Via CLI
```bash
# See what's ready to work on (prioritized)
jit query ready --json | jq -r '.issues[] | "\(.priority) | \(.id[0:8]) | \(.title)"'

# Claim a task atomically
jit issue claim <short-hash> agent:your-name

# Check what's blocking progress
jit query blocked --json
```

### Via MCP Tools

**Use MCP tools exclusively** - don't fall back to CLI/bash for efficiency.

```javascript
// Find ready work
const ready = await jit_query_ready({ json: true });

// Claim atomically
await jit_issue_claim({
  id: "01abc123",
  assignee: "agent:copilot-session-1"
});

// Check dependencies
await jit_issue_show({ id: "01abc123", json: true });
```

---

## 3. MCP Tool Reference

### Parameter Names (Use These!)

**MCP tool parameters match CLI exactly:**

```javascript
jit_issue_create({
  title: "string",
  description: "string",        // Full word (consistent with CLI)
  label: ["type:task", ...],    // Array, singular form
  gate: ["tests", "review"],    // Array, singular form  
  priority: "high"
})
```

### Common MCP Tools

**Issue Management:**
- `jit_issue_create` - Create new issue
- `jit_issue_show` - Get issue details
- `jit_issue_list` - List issues with filters
- `jit_issue_update` - Modify issue (state, labels, priority)
- `jit_issue_claim` - Atomically claim unassigned issue
- `jit_issue_claim_next` - Claim next ready issue by priority
- `jit_issue_search` - Full-text search

**Dependencies:**
- `jit_dep_add` - Add dependency (FROM depends on TO)
- `jit_dep_rm` - Remove dependency

**Gates:**
- `jit_gate_list` - List gate definitions
- `jit_gate_add` - Add gate requirement to issue
- `jit_gate_check` - Run automated gate
- `jit_gate_pass` - Mark gate as passed
- `jit_gate_fail` - Mark gate as failed

**Queries:**
- `jit_query_ready` - Issues ready to work on
- `jit_query_blocked` - Blocked issues with reasons
- `jit_query_state` - Filter by state
- `jit_query_priority` - Filter by priority
- `jit_query_label` - Filter by label pattern

**Graph:**
- `jit_graph_show` - Show dependency tree
- `jit_graph_roots` - Find root issues (no dependencies)
- `jit_graph_downstream` - Show what's blocked by this issue

### Efficiency Tips

✅ **Parallel operations**: Use Promise.all() for creating multiple issues  
✅ **Chain MCP calls**: Structured JSON responses are easy to parse  
✅ **Use short hashes**: `jit issue show 01abc` works  
✅ **Check `--json` output**: All commands support machine-readable format

---

## 4. Work on Tasks (following TDD)

```bash
# 1. Write tests first
cargo test <feature_name>  # Should fail

# 2. Implement minimal code to pass
cargo test <feature_name>  # Should pass

# 3. Run quality checks
cargo test --workspace --quiet
cargo clippy --workspace --all-targets
cargo fmt --all

# 4. Pass gates
jit gate pass <short-hash> tests
jit gate pass <short-hash> clippy  
jit gate pass <short-hash> fmt

# 5. Mark done (auto-transitions if all gates passed)
jit issue update <short-hash> --state done
```

### Gate Workflow

Gates are quality checkpoints that must pass before completion:

```bash
# Add gates to an issue
jit issue create \
  --title "Implement parser" \
  --label "type:task" \
  --gate tests --gate clippy --gate code-review

# Check gate status
jit issue show <id> --json | jq '.data.gates_status'

# Run automated gate
jit gate check <id> tests

# Manual gates
jit gate pass <id> code-review --by "reviewer:alice"
```

**Gate strictness:** Gates may use stricter checks than manual commands (e.g., `clippy` gate uses `-D warnings`). Check gate definition: `jit gate show <gate-key>`.

---

## 5. Create Issues & Structure

### Basic Pattern

```bash
# Milestone (the release)
jit issue create \
  --title "Release v1.0" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical

# Epic (feature in release)
jit issue create \
  --title "User Authentication" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high

# Task (work in epic)
jit issue create \
  --title "Implement JWT utilities" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate tests --gate code-review

# Add dependencies (FROM depends on TO)
jit dep add <milestone-id> <epic-id>  # Milestone blocked by epic
jit dep add <epic-id> <task-id>       # Epic blocked by task
```

### Via MCP Tools

```javascript
// Create milestone
const milestone = await jit_issue_create({
  title: "Release v1.0",
  label: ["type:milestone", "milestone:v1.0"],
  priority: "critical"
});

// Create epic
const epic = await jit_issue_create({
  title: "User Authentication",
  label: ["type:epic", "epic:auth", "milestone:v1.0"],
  priority: "high"
});

// Create task with gates
const task = await jit_issue_create({
  title: "Implement JWT utilities",
  label: ["type:task", "epic:auth", "milestone:v1.0", "component:backend"],
  priority: "high",
  gate: ["tests", "code-review"]
});

// Add dependencies
await jit_dep_add({
  from_id: milestone.data.id,
  to_id: epic.data.id
});
await jit_dep_add({
  from_id: epic.data.id,
  to_id: task.data.id
});
```

---

## 6. Multi-Agent Coordination

JIT supports multiple agents working concurrently through atomic claim operations:

```bash
# Agent claims next available ready issue (atomic)
jit issue claim-next agent:worker-1

# Or claim specific issue atomically
jit issue claim <short-hash> agent:worker-1

# Check what's assigned to you
jit query assignee agent:worker-1 --json

# Release a task if you can't complete it
jit issue release <id> --reason "timeout"

# Monitor overall progress
jit status
jit query blocked  # What's stuck and why
jit query ready    # What's available to claim
```

**How it works:**
- `claim` and `claim-next` are atomic file operations (no race conditions)
- Multiple agents can safely poll for ready work
- Each agent claims issues with unique assignee ID (e.g., `agent:worker-1`, `agent:worker-2`)
- Agents coordinate through the shared `.jit/` repository state

**Note:** Coordinator daemon (`jit-dispatch`) is planned but not yet implemented. Current multi-agent support is decentralized - agents independently poll and claim work.

---

## 7. Document Lifecycle

Attach design docs and session notes to issues:

```bash
# Add document reference
jit doc add <issue-id> docs/design/auth-spec.md \
  --label "Design Specification" \
  --doc-type design

# View document
jit doc show <issue-id> docs/design/auth-spec.md

# List documents for issue
jit doc list <issue-id> --json

# View document history
jit doc history <issue-id> docs/design/auth-spec.md

# Archive when done
jit doc archive docs/design/auth-spec.md --type features
```

---

## 8. Validation & Safety

```bash
# Validate repository integrity
jit validate

# Fix issues automatically (with preview)
jit validate --fix --dry-run
jit validate --fix

# Check dependency graph
jit graph show --all

# View audit log
jit events tail -n 50
jit events query --event-type state_changed --limit 20
```

---

## Key Files to Know

- **README.md** - Project overview, why JIT exists
- **ROADMAP.md** - Where we are, where we're going
- **TESTING.md** - TDD approach, test strategy
- **.copilot-instructions.md** - Coding standards, patterns to follow
- **Tutorials** - Quickstart and complete workflow examples
- **docs/reference/labels.md** - Complete label reference
- **dev/index.md** - Development documentation guide

---

## Common Patterns

### Issue has design doc?
Read it first - contains acceptance criteria and implementation plan.

### Issue has no design doc?
Check its epic's dependencies - epics should have design docs or references. Then check issue description for requirements.

### Session notes missing?
Not all issues have them. Check the epic's documents for architectural context.

### Tests failing?
That's expected if you're doing TDD right. Implement to make them pass.

### Need to understand code?
```bash
# Find where something is used
rg "function_name" crates/

# Find examples of a pattern
rg "resolve_issue_id" --type rust
```

---

## Pro Tips

- **Use short hashes**: `jit issue show 01abc` instead of full UUID
- **Check blocked reasons**: `jit query blocked` shows why issues can't start
- **Follow the gates**: They enforce quality (TDD, tests, clippy, fmt, code-review)
- **Read session notes**: Issues in progress often have `dev/sessions/session-*.md` attached
- **Commit often**: Small focused commits with clear messages
- **No hacks**: Code quality matters - if you're tempted to shortcut, add a TODO issue instead

---

## Important Rules

**Gate strictness:** Gates may use stricter checks than manual commands (e.g., `clippy` gate uses `-D warnings`). Check gate definition: `jit gate show <gate-key>`.

**Pre-existing issues:** You must fix ALL warnings/errors that block gates, even if they existed before your changes. Pre-existence is never an excuse.

**Follow-up issues:** If you discover unrelated work or nice-to-have improvements, propose to create follow-up issues and link them to appropriate epics. Don't expand current issue scope.

**Dependencies matter most:** Use `jit dep add` to express "task B needs task A done first". Epic labels are helpful for organization but dependencies are the critical relationship.

---

## When Stuck

1. Read the linked design doc
2. Check recent commits for similar work
3. Look at test files for examples
4. Ask for guidance!

**That's it!** You're ready to work with JIT as an AI agent. Pick a ready task and start coding.
