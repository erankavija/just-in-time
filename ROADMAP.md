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

## Phase 2: Quality Gates & Query Interface ✅

**Goal:** Gate enforcement and clean query interface for external orchestrators.

**Action Items:**
- [x] Gate registry management (`data/gates.json`)
- [x] Gate operations: `gate add`, `gate pass`, `gate fail`
- [x] Blocked state: consider gates + dependencies
- [x] State transitions with gate validation
- [x] Event log: append-only `data/events.jsonl`
- [x] Event types: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- [x] Query interface: `query ready`, `query blocked`, `query assignee`, `query state`, `query priority`
- [x] **CLI Consistency** (2025-11-30):
  - All mutation commands support `--json` flag for machine-readable output
  - Consistent argument order: `<id>` first, then flags
  - Human-readable text output without `--json`
- [x] **Coordinator removal** (2025-11-30):
  - Removed coordinator daemon from core (732 lines)
  - Extracted to separate `jit-dispatch` orchestrator
  - Clean architectural separation
- [x] Test infrastructure: TestHarness for fast in-process testing (10-100x faster)
- [x] **Tests:** 115 → 123 tests (8 new CLI consistency tests)

**Tests:**
- Gate blocking logic ✓
- State transition validation ✓
- Event log integrity ✓
- Query interface validation ✓
- CLI consistency (JSON output, argument order) ✓

**Reference:** See `docs/design.md` sections: Quality Gating, Monitoring & Observability, `TESTING.md` for test strategy

## Phase 3: Orchestrator & External Integrations (In Progress)

**Goal:** Separate orchestrator tool and enhanced monitoring.

**TDD Requirements:**
- **TESTS MUST BE WRITTEN BEFORE IMPLEMENTATION** ✅ Enforced!
- All new functions must have unit tests before code is written
- Target: >80% overall coverage for all modules
- Current: **258 tests** (97 unit + 8 harness + 16 integration + 7 query + 8 CLI consistency + 6 no-coordinator + 9 orchestrator + 13 memory storage + 94 other)

**Action Items:**
- [x] Graph export: `export --format dot|mermaid` (✓ tests added)
- [x] Event queries: `events tail`, `events query` (✓ tests added)
- [x] Search and filters: complex query syntax (✓ TDD: 9 unit + 7 integration tests)
- [x] **jit-dispatch orchestrator** (2025-11-30):
  - [x] Config file loading (dispatch.toml)
  - [x] Agent pool management with capacity tracking
  - [x] Periodic polling of `jit query ready`
  - [x] Priority-based dispatch (critical > high > normal > low)
  - [x] Multi-agent coordination
  - [x] CLI: `start` (daemon mode), `once` (single cycle)
  - [x] **Tests:** 9 orchestrator tests (6 unit + 3 integration)
  - [ ] Stalled work detection (future)
- [x] **Storage abstraction** (2025-12-02) ✅ **COMPLETED**
  - [x] Extract `IssueStore` trait for pluggable backends
  - [x] Refactor `Storage` → `JsonFileStorage`
  - [x] Update `CommandExecutor` to use generic storage
  - [x] Add 6 trait conformance tests
  - [x] Zero-cost abstraction with generics
  - [x] Add `InMemoryStorage` for fast testing (27 tests, 10-100x speedup)
  - [x] **See:** `docs/storage-abstraction.md` for detailed plan
- [x] **Generic DAG refactoring** (2025-12-02) ✅ **COMPLETED**
  - [x] Extract `GraphNode` trait for generic dependency graphs
  - [x] Make `DependencyGraph<T: GraphNode>` generic
  - [x] Create `visualization.rs` module for Issue-specific exports
  - [x] Separate graph algorithms from domain/presentation logic
  - [x] Prepare for future web UI (clean graph data structure)
  - [x] Added 5 new generic graph tests (102 total tests)
  - [x] **See:** `docs/generic-dag-refactoring.md` for detailed plan
- [x] **CLI Consistency & MCP Integration - Phase 1.1** (2025-12-02) ✅ **COMPLETED**
  - [x] Phase 1.1: Universal JSON output for all commands
  - [x] JsonOutput<T> and JsonError foundation types
  - [x] Add --json to: issue commands (list, show, search)
  - [x] Add --json to: status, validate
  - [x] Add --json to: query commands (ready, blocked, assignee, state, priority)
  - [x] Add --json to: graph commands (show, roots, downstream)
  - [x] Add --json to: registry commands (list, show)
  - [x] Create StatusSummary struct for structured status data
  - [x] **22 commands** now support --json flag
  - [x] **308 tests passing** (was 293, +15 new tests)
  - [x] 3 commits: query, graph, registry JSON support
  - [x] **Time invested:** ~4 hours
- [ ] **CLI Consistency - Phase 1.2** (2025-12-02+) 🚧 **NEXT**
  - [ ] Structured error responses with suggestions
  - [ ] JsonError usage in command handlers
  - [ ] Error codes (ISSUE_NOT_FOUND, CYCLE_DETECTED, etc.)
  - [ ] Suggestions for common errors
  - [ ] **Estimated:** 6-8 hours
- [ ] **CLI Consistency - Phase 1.3** (Future)
  - [ ] Standardized exit codes for automation
  - [ ] **Estimated:** 4-6 hours
- [ ] **CLI Consistency - Phase 1.4** (Future)
  - [ ] Command schema export (`--schema json`)
  - [ ] **Estimated:** 6-8 hours
- [ ] **CLI Consistency - Phase 1.5** (Future)
  - [ ] Batch operations support
  - [ ] **Estimated:** 10-12 hours
- [ ] **Phase 2: MCP Server** (Future)
  - [ ] MCP server for AI agents (TypeScript wrapper)
  - [ ] 15-20 MCP tools covering all operations
  - [ ] **See:** `docs/cli-and-mcp-strategy.md` for detailed plan
  - [ ] **Estimated:** 24-32 hours
- [ ] **Knowledge Management System** (2025-12-02+) 📋 **PLANNED**
  - [ ] Document references in issues (design docs, notes, artifacts)
  - [ ] Git integration for version-aware references
  - [ ] Validation of document links and commit hashes
  - [ ] Web UI with interactive graph visualization
  - [ ] Inline markdown document rendering
  - [ ] Historical document viewer (time machine)
  - [ ] Full-text search across issues and documents
  - [ ] Archive system for project knowledge preservation
  - [ ] **See:** `docs/knowledge-management-vision.md` for detailed plan
  - [ ] **Timeline:** 4 sprints, ~100-120 hours total effort
- [ ] Bulk operations (TDD: write tests first)
- [ ] CI integration: read artifacts to auto-pass gates (TDD: write tests first)
- [ ] Pull-based agent mode (TDD: write tests first)
- [ ] Metrics reporting: `metrics report --format csv` (TDD: write tests first)
- [ ] Webhooks for orchestrator events (TDD: write tests first)

**Tests:**
- Export format validation ✓
- Query syntax correctness
- Gate automation tests
- Webhook delivery tests

**Reference:** See `docs/design.md` sections: Monitoring & Observability, Extensibility Hooks

## Code Quality & Housekeeping (Ongoing)

**Goal:** Maintain clean, well-documented, maintainable codebase.

**Current Status (2025-12-02):**
- ✅ All 11 modules have module-level docs
- ✅ Zero rustdoc warnings in default mode
- ✅ Zero clippy warnings
- ✅ 312 tests passing
- ⚠️ `main.rs` at 807 lines (trending up due to JSON error handling)
- ⚠️ `commands.rs` at 1,980 lines (acceptable but monitor)
- ⚠️ 99 items missing docs in strict mode (mostly `commands.rs` methods)

**Action Items:**
- [ ] **Refactor main.rs** (Priority: Medium, Before: 1,000 lines)
  - [ ] Extract JSON output helpers/macros to reduce boilerplate
  - [ ] Consider `handle_json_result!(expr, json_flag)` macro
  - [ ] Alternative: Split into command handler modules if >1,000 lines
  - [ ] **Estimated:** 2-3 hours
- [ ] **Document CommandExecutor public API** (Priority: Medium, Before: Phase 4)
  - [ ] Add doc comments to all 40 public methods in `commands.rs`
  - [ ] Include examples for complex methods
  - [ ] Document error cases and edge conditions
  - [ ] **Estimated:** 2-3 hours
  - [ ] **Target:** Zero warnings with `cargo rustdoc -- -D missing_docs`
- [ ] **Monitor code growth** (Priority: Low, Ongoing)
  - [ ] Keep `main.rs` under 1,000 lines
  - [ ] Consider splitting `commands.rs` if >2,500 lines
  - [ ] Track: `find crates -name "*.rs" -exec wc -l {} + | sort -rn | head -5`
- [ ] **Documentation audit** (Priority: Low, Before: v1.0)
  - [ ] Ensure all public APIs have doc comments with examples
  - [ ] Add usage examples to module docs
  - [ ] Update `README.md` with latest features
  - [ ] **Estimated:** 3-4 hours

**Benefits:**
- Easier onboarding for contributors
- Reduces technical debt accumulation
- Prevents future refactoring efforts (2-3 hours now vs 10+ hours later)
- Better IDE support and discoverability

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
