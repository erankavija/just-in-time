# Design: CLI Issue Tracker (Just-In-Time)

Version: 0.1
Date: 2025-11-27

## Goal
Provide a compact, robust design for a repository-local CLI issue tracker emphasizing:
- Dependency graph modeling and enforcement (primary)
- Quality gating and gating enforcement (primary)
- Process-orientation: deterministic, machine-friendly outputs (secondary: human-friendly views)

This document captures the domain model, file layout, behaviors, CLI surface, validation rules, phasing plan, open questions, and security considerations so the implementation can be done iteratively.

---

## Guiding Principles
1. Process > People: prioritize deterministic outputs and automation rather than a rich human UI.
2. Simplicity: keep metadata minimal — only what’s needed for dependency resolution and gating.
3. Extensibility: design to accept future integrations (CI, exporters).
4. Transparency: all state stored as versionable plaintext files.
5. Determinism: derived states must be reproducible from stored artifacts.

---

## High-level concept
- Repository-local store: one file per issue under data/issues/<id>.json plus small global files.
- CLI tool ('jit') performs CRUD, dependency edits, validation, gate operations, and shows dependency trees.
- System enforces DAG property for dependencies and enforces that issues cannot be marked ready/done while gates or dependencies block them.

---

## Core Domain Model

Issue (stored per-file)
- id: string (ULID; stable and sortable)
- title: string - one-line summary
- description: string - task details, acceptance criteria, context for agents/humans
- state: enum {open, ready, in_progress, done, archived}
  - Note: "blocked" is derived from dependencies/gates, not stored
- priority: enum {low, normal, high, critical}
- assignee: string (optional) - format: {type}:{identifier}
  - Examples: "human:vkaskivuo", "copilot:session-abc", "ci:github-actions", null (unassigned)
- dependencies: list of issue ids (prerequisites)
- gates_required: list of gate keys
- gates_status: map: gate_key -> {status: enum {pending, passed, failed}, updated_by, updated_at}
- context: map[string, string] - flexible key-value for agent-specific data
  - Example: {"epic": "auth", "pr_url": "...", "agent_notes": "..."}

Gate Definition (global registry, data/gates.json)
- key: string (unique)
- title: string
- description: string
- auto: boolean (true = automation can pass it; false = manual)
- example_integration: optional string (how automation might satisfy this)

System metadata (data/index.json)
- schema_version: integer
- next_id_hint: optional (if using incremental ids; nil if ULIDs)
- all_ids: list of ids (or computed from filesystem)

Derived fields:
- dependents: computed (reverse edges)
- blocked status: derived from dependencies and gate statuses but can be stored for convenience if carefully reconciled

---

## Data layout (skeleton)
- README.md
- docs/design.md
- data/
  - index.json
  - gates.json
  - issues/ (per-issue files, one file per issue id)
- cli/
  - README.md (placeholder)
- scripts/ (future helpers)
- .gitignore

Per-issue file example: data/issues/01FABCDE...json

Design choice: Option B — per-issue files.
Rationale: fewer conflicts in collaborative edits, smaller diffs, easy partial reviews.

---

## Dependency graph handling
- Dependencies form a Directed Acyclic Graph (DAG).
- Any operation that would create a cycle is rejected.
- Cycle detection: DFS or Kahn’s algorithm at mutation time.
- Derived rules:
  - If any dependency is not `done`, the dependent issue is considered `blocked` for purposes of readiness.
  - `ready` becomes achievable only when dependencies are `done` and required gates are `passed`.
- Commands must expose reasons for blocked/not-ready (list failing gates, incomplete deps).

---

## Quality gating
- Gate lifecycle statuses: pending (default) → passed | failed.
- An issue cannot transition to `ready` or `done` until all gates listed in gates_required have status `passed`.
- Gate registry defines whether automation may mark a gate passed (auto: true).
- CLI will offer commands for marking gates passed/failed, and future integrations will allow CI to write gate status artifacts that the CLI reads to auto-pass gates.

Example common gates:
- review (manual)
- unit-tests (auto)
- integration-tests (auto)
- security-scan (auto)

---

## CLI surface (Phase 0 = specification only)
Initialization and scaffolding
- jit init
Issue management
- jit issue create --title "..." [--desc "..."] [--priority high] [--gate review --gate unit-tests]
- jit issue list [--state open|ready|done] [--assignee agent-x] [--priority high]
- jit issue show <id>
- jit issue update <id> [--title ...] [--desc ...] [--priority ...]
- jit issue delete <id>
Assignment (for agent coordination)
- jit issue assign <id> --to <assignee>
- jit issue claim <id> --to <assignee>     (atomic: only succeeds if unassigned)
- jit issue unassign <id>
- jit issue claim-next [--filter "..."] --to <assignee>  (find and claim first ready issue)
Dependencies
- jit issue dep add <id> --on <depId>
- jit issue dep rm <id> --on <depId>
Gates
- jit gate add <id> <gate_key>
- jit gate pass <id> <gate_key> [--by actor]
- jit gate fail <id> <gate_key> [--by actor]
Graph commands
- jit graph show <id>         (upstream tree)
- jit graph downstream <id>   (issues that depend on <id>)
- jit graph roots             (issues with no dependencies)
Monitoring & observability
- jit status                  (overview: X open, Y in-progress, Z blocked with reasons)
- jit agent list              (show active agents and their assignments)
- jit metrics [--since 1h]    (issues completed, gates passed/failed, cycle time)
Validation & tooling
- jit validate                (full integrity check; returns non-zero on errors)
- jit export --format dot|mermaid
Orchestration (Phase 2+)
- jit coordinator start       (launch coordinator daemon)
- jit coordinator stop        (stop coordinator daemon)
Machine outputs
- All commands support --json to return structured output for automation.

---

## Error & validation model
- Hard errors: dependency cycles, missing gate definitions, invalid ids, attempts to mark done when gates/deps block.
- Soft warnings: deprecated fields present, schema mismatches (non-fatal unless incompatible).
- CLI exits non-zero on hard errors and returns machine-readable diagnostics with `--json`.

---

## Implementation phasing

Phase 0 (this PR)
- Add README and docs/design.md
- Add data/gates.json and data/index.json with samples
- Add cli/ placeholder and .gitignore

Phase 1: Core Issue Management
- Implement storage init and per-issue files
- Implement issue create/list/show/update/delete
- Implement add/remove dependency with cycle detection (DAG enforcement)
- Issue assignment commands (assign, claim, unassign)
- Derived-state evaluation (compute blocked status from dependencies/gates)

Phase 2: Quality Gates & Orchestration
- Implement gate assignment and status transitions (pass/fail)
- Event log: append-only data/events.jsonl for audit trail
  - Events: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- Coordinator daemon: monitors ready issues, spawns/dispatches agents
  - Coordinator config: data/coordinator.json (agent pool, dispatch rules)
- Basic monitoring commands: status, agent list, metrics

Phase 3: Advanced Observability & Automation
- Graph export (dot, mermaid)
- Event streaming and webhooks for external integrations
- Automation integration: read CI artifacts to auto-pass gates
- Search expressions and bulk operations
- Web dashboard (optional): simple UI to visualize issue state and dependencies

Phase 4: Production Readiness
- File locking and concurrency controls for multi-agent safety
- Plugin hooks for custom gates and integrations
- Prometheus metrics export
- Cross-repository workflows and issue linking
- Alert system: notify on blocked issues, failed gates, stalled work

---

## Technology options (pros/cons)
- Rust: strong type-safety, excellent error handling, single binary distribution, superior performance. Recommended.
  - Ecosystem: clap (CLI), serde/serde_json (serialization), ulid crate
  - Trade-off: slightly slower initial iteration vs Go, but safety guarantees ideal for graph/validation logic
- Go: single compiled binary, simpler for team contributions, good tooling (Cobra for CLI).
- Python: fastest for prototyping; less ideal as a distributable single binary.

Recommendation: Rust (clap for CLI, serde for JSON) for type safety and performance.

---

## Security & integrity
- All IDs validated to prevent path traversal.
- Atomic writes (write temp + rename) for file updates.
- Consider lightweight file locking to avoid race conditions in CI-heavy environments.

---

## Agent Coordination & Orchestration

### Coordinator Architecture (Phase 2)
The coordinator is a push-based orchestrator that monitors issue state and dispatches agents on-demand.

**Coordinator responsibilities:**
- Watch for state changes (new issues, dependencies resolved, gates passed)
- Identify ready-to-work issues (state=ready, assignee=null, all deps done, no blocking gates)
- Spawn/dispatch agents from configured pool
- Monitor agent health and reassign stalled work
- Record events to audit log

**Coordinator configuration (data/coordinator.json):**
```json
{
  "agent_pool": [
    {
      "id": "copilot-1",
      "type": "copilot",
      "command": "github-copilot-cli work-on",
      "max_concurrent": 2
    },
    {
      "id": "ci-runner",
      "type": "ci",
      "command": "scripts/ci-agent.sh",
      "max_concurrent": 5
    }
  ],
  "dispatch_rules": {
    "priority_order": ["critical", "high", "normal", "low"],
    "stall_timeout_minutes": 30
  }
}
```

**Coordinator lifecycle:**
```bash
# Start coordinator daemon
jit coordinator start [--daemon] [--config data/coordinator.json]

# Stop coordinator
jit coordinator stop

# Coordinator status
jit coordinator status  # Show running agents, pending work queue
```

**Event log format (data/events.jsonl):**
Append-only newline-delimited JSON for audit trail and event sourcing.
```json
{"ts":"2025-11-27T19:00:00Z","event":"issue.created","id":"01JD...","title":"..."}
{"ts":"2025-11-27T19:00:05Z","event":"issue.claimed","id":"01JD...","assignee":"copilot-1"}
{"ts":"2025-11-27T19:05:00Z","event":"gate.passed","id":"01JD...","gate":"unit-tests"}
{"ts":"2025-11-27T19:10:00Z","event":"issue.completed","id":"01JD...","assignee":"copilot-1"}
```

### Alternative: Pull-based Agents (Phase 3)
For environments without coordinator infrastructure, agents can poll for work:
```bash
# Agent polling loop
while true; do
  jit issue claim-next --filter "priority:high|critical" --to copilot-1 --json
  if [ $? -eq 0 ]; then
    # Work on issue, update state and gates
    jit issue complete <id>
  fi
  sleep 10
done
```

---

## Monitoring & Observability

### Built-in Commands (Phase 2)
```bash
# System overview
jit status
# Output:
# Open: 12  Ready: 3  In Progress: 5  Blocked: 4  Done: 23
# Top blockers:
#   - 01JD123: waiting on dep 01JD456 (in_progress)
#   - 01JD789: gate 'security-scan' failed

# Agent status
jit agent list
# Output:
# AGENT         STATUS    ASSIGNED       STARTED
# copilot-1     active    01JD123        5m ago
# copilot-2     idle      -              -
# ci-runner     active    01JD456        12m ago

# Metrics
jit metrics --since 1h
# Output:
# Completed: 5 issues
# Gates passed: 23  Gates failed: 2
# Avg cycle time: 15m
```

### Event Log Queries (Phase 3)
```bash
# Tail live events
jit events tail

# Query event history
jit events query --since 1h --event gate.failed
jit events query --assignee copilot-1

# Generate reports
jit metrics report --format csv --output metrics.csv
```

### Advanced Monitoring (Phase 4)
- Prometheus metrics endpoint: `jit metrics export --prometheus`
- Web dashboard: `jit dashboard start --port 8080`
- Alerting: `jit alert add --condition "blocked_count > 5" --notify webhook:https://...`

---

## Extensibility hooks
- Gate policies referencing artifacts (coverage thresholds).
- Event log for audit streaming and external integrations.
- Plugin executables under a `plugins/` contract.
- Webhooks for coordinator events.

---

## Decisions
1. **Language**: Rust (clap, serde, ulid crate)
2. **ID format**: ULID to avoid central coordination and race conditions
3. **Ready state**: explicit but auto-evaluated from dependencies + gate statuses
4. **Gate registry**: global registry (data/gates.json) to prevent typos and provide metadata
5. **Assignee format**: `{type}:{identifier}` (e.g., "copilot:session-1", "human:alice")
6. **Orchestration**: Push-based coordinator (Phase 2) with pull-based fallback (Phase 3)
7. **Event log**: Append-only JSONL for audit trail and event sourcing
8. **Timestamps**: minimal metadata; rely on git history where possible

Open questions for later phases:
- Locking strategy: cross-platform lock vs advisory locks (implement in Phase 4)
- Coordinator persistence: In-memory vs durable queue for dispatch state

---

## Review checklist for this PR
- [x] Language choice agreed (Rust)
- [ ] File layout accepted
- [ ] Core entity attributes approved
- [ ] Dependency cycle policy accepted
- [x] ID strategy decided (ULID)
- [ ] Gate model approved
- [ ] Phase plan accepted

---

## Example file formats

data/gates.json (sample)
{
  "review": {
    "key": "review",
    "title": "Code review",
    "description": "Manual code review approval",
    "auto": false
  },
  "unit-tests": {
    "key": "unit-tests",
    "title": "Unit tests",
    "description": "Unit test run must pass (automation)",
    "auto": true
  }
}

data/index.json (sample)
{
  "schema_version": 1,
  "next_id_hint": null,
  "all_ids": []
}

---

## Next actions for implementers
- Decide on ID format and final CLI language.
- Implement Phase 1 commands and tests that validate DAG enforcement.
- Iterate gates integration with sample CI artifacts.

Thank you — review this document and tell me which items you want to finalize before I prepare a Phase 1 implementation PR.