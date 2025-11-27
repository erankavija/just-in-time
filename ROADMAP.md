# Just-In-Time Roadmap

## Phase 0: Design ✅
- [x] Core design document
- [x] File layout and data model
- [x] CLI surface specification
- [x] Copilot instructions
- [x] Language choice (Rust)

## Phase 1: Core Issue Management

**Goal:** Basic issue tracker with dependency graph enforcement.

**Action Items:**
- [x] Initialize Rust project (`cargo init`, add dependencies)
- [x] Implement `jit init` - create `data/` structure
- [x] Core domain types (Issue, State, Priority, Gate)
- [x] Storage layer with atomic writes
- [x] Issue CRUD: `create`, `list`, `show`, `update`, `delete`
- [x] Dependency graph with cycle detection (DFS)
- [x] Commands: `dep add`, `dep rm`
- [x] Derived state: compute blocked from dependencies
- [x] Assignment: `assign`, `claim`, `unassign`, `claim-next`
- [x] Graph queries: `graph show`, `graph roots`, `graph downstream`
- [x] Validation: `jit validate` (DAG integrity check)

**Tests:**
- Unit tests for cycle detection
- Property tests for DAG invariant
- Integration tests for CLI commands

**Reference:** See `docs/design.md` sections: Core Domain Model, Dependency Graph, CLI Surface

## Phase 2: Quality Gates & Orchestration ✅

**Goal:** Gate enforcement and coordinator daemon for agent dispatch.

**Action Items:**
- [x] Gate registry management (`data/gates.json`)
- [x] Gate operations: `gate add`, `gate pass`, `gate fail`
- [x] Blocked state: consider gates + dependencies
- [x] State transitions with gate validation
- [x] Event log: append-only `data/events.jsonl`
- [x] Event types: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- [x] Coordinator daemon: `coordinator start`, `stop`, `status`
- [x] Agent pool configuration (`data/coordinator.json`)
- [x] Dispatch logic: priority-based work assignment
- [x] Monitoring: `status`, `agent list`, `metrics`

**Tests:**
- Gate blocking logic
- State transition validation
- Event log integrity

**Reference:** See `docs/design.md` sections: Quality Gating, Agent Coordination & Orchestration, Monitoring & Observability

## Phase 3: Advanced Observability & Automation

**Goal:** Enhanced monitoring and external integrations.

**TDD Requirements:**
- **TESTS MUST BE WRITTEN BEFORE IMPLEMENTATION**
- All new functions must have unit tests before code is written
- Target: >80% code coverage for all modules
- Current coverage: 40.34% (313/776 lines) - NEEDS IMPROVEMENT

**Action Items:**
- [x] Graph export: `export --format dot|mermaid` (✓ tests added)
- [x] Event queries: `events tail`, `events query` (✓ tests added)
- [ ] **BACKFILL MISSING TESTS** for existing functions:
  - [ ] delete_issue, assign_issue, unassign_issue
  - [ ] add_dependency, remove_dependency (cycle detection tested)
  - [ ] add_gate, pass_gate, fail_gate (basic ops tested, need comprehensive)
  - [ ] show_graph, show_downstream, show_roots (need comprehensive)
  - [ ] validate, status (not tested)
  - [ ] list_gates, add/remove/show gate definitions (registry tested, not commands)
  - [ ] export_graph (not tested at command level)
- [ ] Search and filters: complex query syntax (TDD: write tests first)
- [ ] Bulk operations (TDD: write tests first)
- [ ] CI integration: read artifacts to auto-pass gates (TDD: write tests first)
- [ ] Pull-based agent mode (TDD: write tests first)
- [ ] Metrics reporting: `metrics report --format csv` (TDD: write tests first)
- [ ] Webhooks for coordinator events (TDD: write tests first)

**Tests:**
- Export format validation ✓
- Query syntax correctness
- Gate automation tests
- Webhook delivery tests

**Reference:** See `docs/design.md` sections: Monitoring & Observability, Extensibility Hooks

## Phase 4: Production Readiness

**Goal:** Concurrency safety and production features.

**TDD Requirements:**
- **TESTS MUST BE WRITTEN BEFORE IMPLEMENTATION**
- Target: >90% code coverage for production features
- All concurrency scenarios must have tests
- Error handling must be fully tested

**Action Items:**
- [ ] File locking for multi-agent safety (TDD: write concurrency tests first)
- [ ] Plugin system for custom gates (TDD: write plugin API tests first)
- [ ] Prometheus metrics export (TDD: write metric format tests first)
- [ ] Web dashboard (optional) (TDD: write API tests first)
- [ ] Alert system: `alert add --condition "..."` (TDD: write alert tests first)
- [ ] Cross-repository issue linking (TDD: write link validation tests first)
- [ ] Performance optimization (if needed) (add performance benchmarks)
- [ ] Comprehensive error recovery (TDD: write error scenario tests first)

**Tests:**
- Concurrency stress tests (race conditions, deadlocks)
- Plugin API validation
- Error recovery scenarios
- Performance benchmarks

**Reference:** See `docs/design.md` sections: Implementation Phasing, Extensibility Hooks

## Dependencies

- Phase 1 → Phase 2 (core needed for gates)
- Phase 2 → Phase 3 (events needed for observability)
- Phase 3 → Phase 4 (stable features before hardening)

## Success Metrics

- **Phase 1:** Can track issues with dependencies, detect cycles
- **Phase 2:** Coordinator dispatches agents, gates block transitions
- **Phase 3:** Full observability, CI/CD integration working
- **Phase 4:** Production-grade reliability, multi-agent coordination

## Quick Start

1. Start with Phase 1, Core Issue Management
2. Follow TDD: tests first, minimal implementation
3. Run `cargo clippy` and `cargo fmt` frequently
4. See `copilot-instructions.md` for coding guidelines
5. See `docs/design.md` for detailed specifications
