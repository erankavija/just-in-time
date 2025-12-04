# Design: CLI Issue Tracker (Just-In-Time)

Version: 0.2
Date: 2025-12-01

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
- Repository-local store: one file per issue under .jit/issues/<id>.json plus small global files.
- CLI tool ('jit') performs CRUD, dependency edits, validation, gate operations, and shows dependency trees.
- System enforces DAG property for dependencies and enforces that issues cannot be marked ready/done while gates or dependencies block them.
- Separate orchestrator tool ('jit-dispatch') handles agent coordination and work distribution.

---

## Core Domain Model

Issue (stored per-file)
- id: string (ULID; stable and sortable)
- title: string - one-line summary
- description: string - task details, acceptance criteria, context for agents/humans
- state: enum {backlog, ready, in_progress, gated, done, archived}
  - backlog: Created but has incomplete dependencies
  - ready: Dependencies done, available to start work (gates don't block this)
  - in_progress: Currently being worked on
  - gated: Work complete, awaiting quality gate approval
  - done: Completed successfully
  - archived: No longer relevant
  - Note: "blocked" is derived from dependencies only, not stored
- priority: enum {low, normal, high, critical}
- assignee: string (optional) - format: {type}:{identifier}
  - Examples: "human:vkaskivuo", "copilot:session-abc", "ci:github-actions", null (unassigned)
- dependencies: list of issue ids (prerequisites)
- gates_required: list of gate keys
- gates_status: map: gate_key -> {status: enum {pending, passed, failed}, updated_by, updated_at}
- context: map[string, string] - flexible key-value for agent-specific data
  - Example: {"epic": "auth", "pr_url": "...", "agent_notes": "..."}

Gate Definition (global registry, .jit/gates.json)
- key: string (unique)
- title: string
- description: string
- auto: boolean (true = automation can pass it; false = manual)
- example_integration: optional string (how automation might satisfy this)

System metadata (.jit/index.json)
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
- .jit/
  - index.json
  - gates.json
  - issues/ (per-issue files, one file per issue id)
- cli/
  - README.md (placeholder)
- scripts/ (future helpers)
- .gitignore

Per-issue file example: .jit/issues/01FABCDE...json

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
Query interface
- jit query ready             (issues ready for work: state=ready, unassigned, no blocks)
- jit query blocked           (issues blocked by dependencies or gates)
- jit query assignee <name>   (issues assigned to specific agent)
- jit query state <state>     (filter by state)
- jit query priority <level>  (filter by priority)
Events
- jit events tail             (live event stream)
- jit events query            (historical event search)
Validation & tooling
- jit validate                (full integrity check; returns non-zero on errors)
- jit export --format dot|mermaid
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
- Add .jit/gates.json and .jit/index.json with samples
- Add cli/ placeholder and .gitignore

Phase 1: Core Issue Management
- Implement storage init and per-issue files
- Implement issue create/list/show/update/delete
- Implement add/remove dependency with cycle detection (DAG enforcement)
- Issue assignment commands (assign, claim, unassign)
- Derived-state evaluation (compute blocked status from dependencies/gates)

Phase 2: Quality Gates & Query Interface
- Implement gate assignment and status transitions (pass/fail)
- Event log: append-only .jit/events.jsonl for audit trail
  - Events: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- Query interface for ready/blocked/filtered issues
- CLI consistency: all commands support --json for machine-readable output
- Test infrastructure: TestHarness for fast in-process testing

Phase 3: Orchestrator & External Integrations
- Separate jit-dispatch orchestrator tool (extracted from core)
  - Config file loading (dispatch.toml)
  - Agent pool management with capacity tracking
  - Priority-based dispatch algorithm
  - Commands: start (daemon), once (single cycle)
- Graph export (dot, mermaid)
- Event queries and tail commands
- **Storage abstraction (NEXT)**: Trait-based backend system for pluggable storage
  - Extract IssueStore trait for multiple backend support
  - Refactor current Storage to JsonFileStorage
  - Enable future SQLite, in-memory, or custom backends
  - See docs/storage-abstraction.md for detailed plan
- Bulk operations for batch updates
- Automation integration: read CI artifacts to auto-pass gates
- Pull-based agent mode (polling alternative)

Phase 4: Production Readiness
- File locking and concurrency controls for multi-agent safety
- Plugin system for custom gates
- Prometheus metrics export
- Web dashboard (optional): simple UI to visualize issue state and dependencies
- Alert system: notify on blocked issues, failed gates, stalled work
- Cross-repository workflows and issue linking
- Stalled work detection for jit-dispatch

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

### Architecture Overview (Updated 2025-11-30)
Agent orchestration is handled by a **separate tool** (`jit-dispatch`) to maintain clean architectural separation.
The core `jit` CLI focuses on issue tracking and querying, while `jit-dispatch` handles work distribution.

### jit-dispatch Orchestrator (Phase 3)
Push-based orchestrator that monitors issue state and dispatches agents on-demand.

**Orchestrator responsibilities:**
- Poll `jit query ready` to identify available work
- Dispatch work to agents based on priority and capacity
- Track agent assignments and capacity limits
- Support multiple concurrent agents
- Respect agent max_concurrent limits

**Configuration (dispatch.toml):**
```toml
[dispatch]
poll_interval_secs = 10

[[agents]]
id = "copilot-1"
type = "copilot"
command = "github-copilot-cli"
args = ["work-on"]
max_concurrent = 2

[[agents]]
id = "ci-runner"
type = "ci"
command = "./scripts/ci-agent.sh"
max_concurrent = 5
```

**Orchestrator commands:**
```bash
# Run single dispatch cycle
jit-dispatch once --config dispatch.toml --jit-path ./jit

# Start daemon (continuous polling)
jit-dispatch start --config dispatch.toml --jit-path ./jit [--daemon]
```

**How jit-dispatch works:**
1. Reads configuration (agent pool, capacity limits)
2. Polls `jit query ready --json` to find available work
3. Sorts by priority (critical → high → normal → low)
4. Dispatches to agents respecting max_concurrent limits
5. Uses `jit issue claim <id> --to <agent-id>` to assign work
6. Repeats based on poll_interval

**Event log format (.jit/events.jsonl):**
The core `jit` CLI writes append-only events for audit trail:
```json
{"ts":"2025-11-27T19:00:00Z","event":"issue.created","id":"01JD...","title":"..."}
{"ts":"2025-11-27T19:00:05Z","event":"issue.claimed","id":"01JD...","assignee":"copilot-1"}
{"ts":"2025-11-27T19:05:00Z","event":"gate.passed","id":"01JD...","gate":"unit-tests"}
{"ts":"2025-11-27T19:10:00Z","event":"issue.completed","id":"01JD...","assignee":"copilot-1"}
```

### Pull-based Agents (Phase 3 Alternative)
For simpler environments, agents can poll for work directly:
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

### Query Interface (Phase 2 - Implemented)
```bash
# Find ready work
jit query ready [--json]
# Returns: unassigned issues with state=ready, no blocking deps/gates

# Find blocked issues
jit query blocked [--json]
# Returns: issues blocked by dependencies or failed gates

# Filter by assignee
jit query assignee copilot-1 [--json]

# Filter by state
jit query state in_progress [--json]

# Filter by priority
jit query priority critical [--json]
```

### Event Log Queries (Phase 3 - Implemented)
```bash
# Tail live events
jit events tail

# Query event history
jit events query --since 1h --event gate.failed
jit events query --assignee copilot-1
```

### Advanced Monitoring (Phase 3+)
```bash
# Graph visualization
jit export --format dot > graph.dot
jit export --format mermaid > graph.md

# Metrics reporting (Future)
jit metrics report --format csv --output metrics.csv
```

### Production Monitoring (Phase 4)
- Prometheus metrics export: `jit metrics export --prometheus`
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
4. **Gate registry**: global registry (.jit/gates.json) to prevent typos and provide metadata
5. **Assignee format**: `{type}:{identifier}` (e.g., "copilot:session-1", "human:alice")
6. **Architecture**: Separate core tracker (`jit`) from orchestrator (`jit-dispatch`) for clean boundaries
7. **Orchestration**: Push-based via jit-dispatch (Phase 3) with pull-based fallback available
8. **Event log**: Append-only JSONL for audit trail and event sourcing
9. **Timestamps**: minimal metadata; rely on git history where possible
10. **CLI consistency**: All mutation commands support `--json` for machine-readable output
11. **Testing**: Three-layer strategy (unit/harness/integration) with TestHarness for fast iteration

Open questions for later phases:
- Locking strategy: cross-platform lock vs advisory locks (implement in Phase 4)
- Stalled work detection: timeout thresholds and reassignment policies

---

## Status (as of 2025-12-01)

**Completed Phases:**
- ✅ Phase 0: Design
- ✅ Phase 1: Core Issue Management (full CRUD, dependencies, DAG enforcement)
- ✅ Phase 2: Quality Gates & Query Interface (gates, events, queries, TestHarness)

**Current Phase:**
- 🔄 Phase 3: Orchestrator & External Integrations (jit-dispatch extracted, graph export done)

**Test Coverage:**
- 132 total tests (78 unit + 8 harness + 16 integration + 7 query + 8 CLI consistency + 6 refactor + 9 orchestrator)
- TestHarness provides 10-100x faster testing vs process-based tests
- See `TESTING.md` for detailed test strategy

---

## Example file formats

.jit/gates.json (sample)
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

.jit/index.json (sample)
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