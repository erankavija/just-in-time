# Just-In-Time Roadmap

## Phase 0: Design ✅
- [x] Core design document
- [x] File layout and data model
- [x] CLI surface specification
- [x] Copilot instructions
- [x] Language choice (Rust)

## Phase 1: Core Issue Management ✅

**Goal:** Basic issue tracker with dependency graph enforcement.

- [x] Initialize Rust project
- [x] Implement `jit init` - create `.jit/` structure
- [x] Core domain types (Issue, State, Priority, Gate)
- [x] Storage layer with atomic writes
- [x] Issue CRUD: `create`, `list`, `show`, `update`, `delete`
- [x] Dependency graph with cycle detection (DFS)
- [x] Commands: `dep add`, `dep rm`
- [x] Derived state: compute blocked from dependencies
- [x] Assignment: `assign`, `claim`, `unassign`, `claim-next`
- [x] Graph queries: `graph show`, `graph roots`, `graph downstream`
- [x] Validation: `jit validate` (DAG integrity check)

## Phase 2: Quality Gates & Query Interface ✅

**Goal:** Gate enforcement and clean query interface for external orchestrators.

- [x] Gate registry management (`.jit/gates.json`)
- [x] Gate operations: `gate add`, `gate pass`, `gate fail`
- [x] Blocked state: consider gates + dependencies
- [x] State transitions with gate validation
- [x] Event log: append-only `.jit/events.jsonl`
- [x] Event types: issue.created, issue.claimed, gate.passed, gate.failed, issue.completed
- [x] Query interface: `query ready`, `query blocked`, `query assignee`, `query state`, `query priority`
- [x] CLI Consistency:
  - All mutation commands support `--json` flag for machine-readable output
  - Consistent argument order: `<id>` first, then flags
  - Human-readable text output without `--json`
- [x] Coordinator removal:
  - Removed coordinator daemon from core
  - Extracted to separate `jit-dispatch` orchestrator
  - Clean architectural separation
- [x] Test infrastructure: TestHarness for fast in-process testing

## Phase 3: Orchestrator & External Integrations ✅

**Goal:** Separate orchestrator tool and enhanced monitoring.

- [x] Graph export: `export --format dot|mermaid`
- [x] Event queries: `events tail`, `events query`
- [x] Search and filters: complex query syntax
- [x] **jit-dispatch orchestrator:**
  - Config file loading (dispatch.toml)
  - Agent pool management with capacity tracking
  - Periodic polling of `jit query ready`
  - Priority-based dispatch (critical > high > normal > low)
  - Multi-agent coordination
  - CLI: `start` (daemon mode), `once` (single cycle)
- [x] **Storage abstraction:**
  - Extract `IssueStore` trait for pluggable backends
  - Refactor `Storage` → `JsonFileStorage`
  - Update `CommandExecutor` to use generic storage
  - Zero-cost abstraction with generics
  - Add `InMemoryStorage` for fast testing
- [x] **Generic DAG refactoring:**
  - Extract `GraphNode` trait for generic dependency graphs
  - Make `DependencyGraph<T: GraphNode>` generic
  - Create `visualization.rs` module for Issue-specific exports
  - Separate graph algorithms from domain/presentation logic
- [x] **CLI Consistency - Phase 1.1:** Universal JSON output
  - Add --json to: issue commands (list, show, search)
  - Add --json to: status, validate
  - Add --json to: query commands (ready, blocked, assignee, state, priority)
  - Add --json to: graph commands (show, roots, downstream)
  - Add --json to: registry commands (list, show)
  - Create StatusSummary struct for structured status data
  - 22 commands now support --json flag
- [x] **CLI Consistency - Phase 1.2:** Structured error responses
  - JsonError usage in command handlers
  - Error codes (ISSUE_NOT_FOUND, GATE_NOT_FOUND, CYCLE_DETECTED, etc.)
  - Suggestions for common errors
  - --json flag added to dep and gate commands
  - Exit code 1 for errors with JSON output
- [x] **CLI Consistency - Phase 1.3:** Standardized exit codes
  - Exit code enum with clear mappings (0, 1, 2, 3, 4, 5, 6, 10)
  - Error message to exit code mapping helper
  - Documented exit codes in --help
  - Enhanced validation (broken deps, invalid gates, cycles)

**Deferred:**
- [ ] Stalled work detection
- [ ] Bulk operations
- [ ] CI integration: read artifacts to auto-pass gates
- [ ] Pull-based agent mode
- [ ] Metrics reporting: `metrics report --format csv`
- [ ] Webhooks for orchestrator events

## Code Quality & Housekeeping ✅

**Goal:** Maintain clean, well-documented, maintainable codebase.

**Current Status:**
- ✅ All 11 modules have module-level docs
- ✅ Zero rustdoc warnings in default mode
- ✅ Zero clippy warnings
- ✅ 332 tests passing
- ✅ main.rs at 843 lines (under 1,000 threshold)
- ✅ commands.rs at 2,134 lines (critical methods documented)
- ✅ output_macros.rs created (4 helper macros)
- ✅ Key CommandExecutor methods documented with examples

**Completed:**
- [x] Refactor main.rs
  - Created `output_macros.rs` with 4 macros
  - Demonstrated usage, ready for broader adoption
  - main.rs reduced from 853 → 843 lines
- [~] Document CommandExecutor public API
  - Documented 5 critical methods (create_issue, list_issues, add_dependency, claim_issue, validate_silent)
  - All doc tests passing
  - Remaining 35+ methods can be documented as needed
- [x] Monitor code growth
  - main.rs: 843 lines (✓ under 1,000)
  - commands.rs: 2,134 lines (✓ under 2,500)
  - Overall quality maintained

## Phase 4: Future Enhancements

**Goal:** Advanced features and optimizations.

### CLI Consistency

- [x] **Phase 1.4:** Command schema export
  - Implemented `--schema` flag for AI introspection
  - Generated JSON schemas from command definitions
  - 8 unit tests + 7 integration tests
  - Documentation in `docs/json-schema-api.md`
- [ ] **Phase 1.5:** Batch operations support

### MCP Server

- [x] TypeScript MCP server wrapping CLI
- [x] 29 MCP tools covering all operations
- [ ] Integration with Claude Desktop (documented, not tested)
- [ ] See `mcp-server/README.md` for usage

### Knowledge Management System

- [ ] Document references in issues (design docs, notes, artifacts)
- [ ] Git integration for version-aware references
- [ ] Validation of document links and commit hashes
- [ ] Web UI with interactive graph visualization
- [ ] Inline markdown document rendering
- [ ] Historical document viewer (time machine)
- [ ] Full-text search across issues and documents
- [ ] Archive system for project knowledge preservation
- [ ] See `docs/knowledge-management-vision.md` for detailed plan

### Production Readiness

- [ ] **File locking for multi-agent safety** (Priority: High - race conditions observed with MCP server)
  - [x] Research locking strategy (flock vs advisory locks vs process-based locking) - **Decision: fs4 with advisory locks**
  - [x] Add locking abstraction to storage layer (lock_file/unlock_file methods) - **FileLocker + LockGuard**
  - [x] Implement file-level locking for atomic operations (index.json, individual issues) - **Phase 1.1 Complete (2025-12-03)**
    - Created `FileLocker` with timeout support
    - Created `LockGuard` with RAII pattern
    - Exclusive and shared locks implemented
    - Try-lock non-blocking variants
    - 6 comprehensive unit tests
    - All 338 tests passing
  - [x] Update JsonFileStorage to acquire locks before write operations - **Phase 1.2 Complete (2025-12-03)**
    - Uses separate .lock files to avoid conflicts with atomic writes
    - Lock ordering: index first, then issue (prevents deadlocks)
    - All operations protected: save, load, delete, list, gates, events
    - 7 concurrent tests including 50-thread stress test
    - All 362 tests passing, zero clippy warnings
  - [x] Add lock timeout configuration (via environment variables) - **JIT_LOCK_TIMEOUT**
  - [x] Add tests for concurrent access patterns (parallel creates, updates, dependency adds) - **7 comprehensive tests**
  - [x] Document locking semantics and performance implications - **docs/file-locking-usage.md (2025-12-03)**
  - [ ] Add retry logic with exponential backoff for lock contention - **Phase 2 (deferred)**
  - [ ] Test with MCP server and multiple concurrent clients - **Phase 2 (next priority)**
- [ ] Plugin system for custom gates
- [ ] Prometheus metrics export
- [ ] Web dashboard (optional)
- [ ] Alert system: `alert add --condition "..."`
- [ ] Cross-repository issue linking
- [ ] Performance optimization (if needed)
- [ ] Comprehensive error recovery

## Dependencies

- Phase 1 → Phase 2 (core needed for gates)
- Phase 2 → Phase 3 (events needed for observability)
- Phase 3 → Phase 4 (stable features before hardening)

## Success Metrics

- **Phase 1:** Can track issues with dependencies, detect cycles ✅
- **Phase 2:** Gates block transitions, events logged ✅
- **Phase 3:** Full observability, external integration working ✅
- **Phase 4:** Production-grade reliability, advanced features

## Quick Start

1. See `copilot-instructions.md` for coding guidelines
2. See `docs/design.md` for detailed specifications
3. Run `cargo test` to verify tests
4. Run `cargo clippy` for linting
5. Follow TDD: tests first, minimal implementation

## Reference Documentation

- `docs/design.md` - Comprehensive design document
- `docs/cli-and-mcp-strategy.md` - CLI consistency and MCP server plan
- `docs/storage-abstraction.md` - Storage layer architecture
- `docs/generic-dag-refactoring.md` - DAG abstraction details
- `docs/knowledge-management-vision.md` - Long-term vision
- `TESTING.md` - Test strategy and infrastructure
