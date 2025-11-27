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

## Phase 2: Quality Gates & Orchestration

**Goal:** Gate enforcement and coordinator daemon for agent dispatch.

**Action Items:**
- [ ] Gate registry management (`data/gates.json`)
- [ ] Gate operations: `gate add`, `gate pass`, `gate fail`
- [ ] Blocked state: consider gates + dependencies
- [ ] State transitions with gate validation
- [ ] Event log: append-only `data/events.jsonl`
- [ ] Event types: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- [ ] Coordinator daemon: `coordinator start`, `stop`, `status`
- [ ] Agent pool configuration (`data/coordinator.json`)
- [ ] Dispatch logic: priority-based work assignment
- [ ] Monitoring: `status`, `agent list`, `metrics`

**Tests:**
- Gate blocking logic
- State transition validation
- Event log integrity

**Reference:** See `docs/design.md` sections: Quality Gating, Agent Coordination & Orchestration, Monitoring & Observability

## Phase 3: Advanced Observability & Automation

**Goal:** Enhanced monitoring and external integrations.

**Action Items:**
- [ ] Graph export: `export --format dot|mermaid`
- [ ] Event queries: `events tail`, `events query`
- [ ] Search and filters: complex query syntax
- [ ] Bulk operations
- [ ] CI integration: read artifacts to auto-pass gates
- [ ] Pull-based agent mode (polling fallback)
- [ ] Metrics reporting: `metrics report --format csv`
- [ ] Webhooks for coordinator events

**Tests:**
- Export format validation
- Query syntax correctness

**Reference:** See `docs/design.md` sections: Monitoring & Observability, Extensibility Hooks

## Phase 4: Production Readiness

**Goal:** Concurrency safety and production features.

**Action Items:**
- [ ] File locking for multi-agent safety
- [ ] Plugin system for custom gates
- [ ] Prometheus metrics export
- [ ] Web dashboard (optional)
- [ ] Alert system: `alert add --condition "..."`
- [ ] Cross-repository issue linking
- [ ] Performance optimization (if needed)
- [ ] Comprehensive error recovery

**Tests:**
- Concurrency stress tests
- Plugin API validation

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
