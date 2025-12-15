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

## Core Improvements ✅

### State Model Refactoring ✅

**Goal:** Clearer state semantics and explicit quality gate blocking.

- [x] Renamed `Open` → `Backlog` for clearer semantics
- [x] Added `Gated` state for work awaiting quality approval
- [x] Separated blocking logic: dependencies block starting, gates block completion
- [x] Auto-transitions: Backlog→Ready (deps done), Gated→Done (gates pass)
- [x] Updated all components: Rust core (167 tests), MCP server (7 tests), Web UI (11 tests), REST API (6 tests)
- [x] Backward compatible: accepts 'open' as alias for 'backlog'
- [x] Comprehensive documentation: `docs/state-model-refactoring.md`
- [x] Test suites: MCP protocol testing, Web UI type/component testing

**Implementation:**
- Updated `State` enum: `Backlog | Ready | InProgress | Gated | Done | Archived`
- Split `is_blocked()` (dependencies only) from `has_unpassed_gates()`
- Updated all visualizations, CLI output, MCP schema, Web UI types/colors/emojis
- Added `web/TESTING.md`, `mcp-server/test.js` with full test coverage

## Code Quality & Housekeeping ✅

**Goal:** Maintain clean, well-documented, maintainable codebase.

**Status:**
- ✅ All modules have module-level docs
- ✅ Zero rustdoc warnings in default mode
- ✅ Zero clippy warnings (with allow(dead_code) on incomplete features)
- ✅ **518 tests passing** (across entire workspace)
  - Core library: 150 unit tests + 140 integration tests
  - Type hierarchy: 10 membership validation tests
  - Various modules: 218 additional tests
- ✅ **Transitive reduction implemented** - storage-level minimal edge set
- ✅ **Issue breakdown command** - foolproof task decomposition for agents
- ✅ main.rs well-maintained (under 1,000 lines)
- ✅ commands.rs modularized into 12 focused modules (1,918 lines total, 55% reduction)
- ✅ jit-server crate with search API endpoint
- ✅ Thread-safe InMemoryStorage (refactored for async)
- ✅ Responsive web UI with search functionality
- ✅ **54 web tests passing** (14 new strategic view tests)
- ✅ **Strategic/tactical view toggle** - filtering, downstream stats, seamless switching
- ✅ **Label system (95% complete)** - validation, querying, strategic views, web UI badges
- ✅ **Type hierarchy enforcement (Phases A-E complete)** - type validation + membership validation
- ✅ Comprehensive documentation: 
  - file-locking-usage.md (400 lines)
  - knowledge-management-vision.md (537 lines)
  - web-ui-architecture.md (466 lines)
  - search-implementation.md (596 lines)
  - type-hierarchy-enforcement-proposal.md (920 lines)
  - session-2025-12-14-membership-validation.md (453 lines)

**Completed:**
- [x] Refactor main.rs
  - Created `output_macros.rs` with 4 macros
  - Demonstrated usage, ready for broader adoption
- [~] Document CommandExecutor public API
  - Documented 5 critical methods (create_issue, list_issues, add_dependency, claim_issue, validate_silent)
  - All doc tests passing
  - Remaining 35+ methods can be documented as needed
- [x] Monitor code growth
  - Overall quality maintained

## Phase 4: Advanced Features & Production Hardening

**Goal:** Knowledge management and production-grade reliability.

### Issue Hierarchy & Strategic Views ✅ (All Phases Complete) 🔍 AUDIT IN PROGRESS

**Goal:** Labels-based hierarchy for epics, milestones, and strategic/tactical views.

**Design Complete:**
- [x] Design: Labels define hierarchy (not separate storage)
- [x] Implementation plan: 6 phases, 12-18 hours
- [x] All design decisions confirmed
- [x] See: `docs/label-hierarchy-implementation-plan.md` (complete plan)
- [x] See: `docs/label-conventions.md` (format rules & agent usage)
- [x] See: `docs/label-hierarchy-audit-plan.md` (comprehensive audit plan - 2025-12-15)

**Implementation Phases:**
- [x] **Phase 1.1:** Domain model (30 min) - **COMPLETE** ✅ 2025-12-08
  - Added `labels: Vec<String>` to Issue struct
  - 4 new domain tests
  - All 187 tests passing
- [x] **Phase 1.2:** Label validation (1 hour) - **COMPLETE** ✅ 2025-12-08
  - Created `labels.rs` module with validation
  - Regex: `^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$`
  - Helpful error messages with suggestions
  - 13 validation tests
- [x] **Phase 1.3:** CLI commands (2 hours) - **COMPLETE** ✅ 2025-12-08
  - `jit issue create --label "namespace:value"`
  - `jit issue update <id> --label "..." --remove-label "..."`
  - Updated CommandExecutor methods
  - All 196 tests passing
- [x] **Phase 1.4:** Query by label (1-2 hours) - **COMPLETE** ✅ 2025-12-08
  - `jit query label "pattern"`
  - Support exact match and wildcard: `milestone:*`
  - 8 comprehensive tests
  - JSON output support
- [x] **Phase 2:** Namespace registry (2-3 hours) - **COMPLETE** ✅ 2025-12-08
  - `.jit/label-namespaces.json` with standard namespaces
  - Created on `jit init` with defaults (milestone, epic, component, type, team)
  - Validate uniqueness constraints (type, team) on issue creation
  - Domain model: LabelNamespace, LabelNamespaces types
  - Storage: load/save namespace registry (JSON + InMemory)
  - CommandExecutor methods: get_issue, add_label, list_label_values, add_label_namespace
  - 9 comprehensive tests
  - Zero clippy warnings
- [x] **Phase 3:** Breakdown & strategic queries (2-3 hours) - **COMPLETE** ✅ 2025-12-08
  - Updated `breakdown_issue` to copy parent labels to all subtasks
  - `query_strategic`: dynamically queries all namespaces with strategic=true
  - CLI: `jit query strategic [--json]`
  - Flexible design: custom namespaces can be marked strategic
  - Label inheritance enables hierarchical planning (milestone → epic → tasks)
  - 12 comprehensive tests (query + breakdown scenarios)
  - 550 total tests passing, zero clippy warnings
- [x] **Phase 4:** MCP integration (1-2 hours) - **COMPLETE** ✅ 2025-12-08
  - CLI commands: `jit label namespaces`, `jit label values`, `jit label add-namespace`
  - Schema auto-generates MCP tools from CLI definitions
  - Fixed MCP server to read positional args from schema (no hardcoded map)
  - Fixed multi-word subcommand handling (add-namespace)
  - MCP tools: jit_query_label, jit_query_strategic, jit_label_namespaces, jit_label_values, jit_label_add_namespace
  - 5 comprehensive MCP tests, all passing
  - Zero-maintenance: new CLI commands automatically become MCP tools
- [x] **Phase 5.1:** Label badges in Web UI (2 hours) - **COMPLETE** ✅ 2025-12-08
  - Created LabelBadge component with namespace:value formatting
  - Color-coded badges by namespace (milestone=blue, epic=yellow, etc.)
  - Added labels to GraphView nodes (max 2 badges + count)
  - Added labels to IssueDetail view (all labels shown)
  - Backend: Updated GraphNode API to include labels field
  - TypeScript: Updated Issue and GraphNode types
  - All 38 web tests + 490+ Rust tests passing
- [x] **Phase 5.2:** Strategic/tactical view toggle (1.5 hours) - **COMPLETE** ✅ 2025-12-09
  - Filter graph nodes to milestone/epic labels only (Option A)
  - Downstream stats with DFS calculation (Option C)
  - Toggle button in header: 📋 Tactical / 🎯 Strategic
  - Display rollup stats on nodes: "↓ 5 tasks • ✓ 3 • ⚠ 1"
  - 14 new tests for strategic view filtering
  - All 54 web tests + 192 Rust tests passing
  - TDD implementation with full test coverage
- [x] **Phase 5.3:** Label filtering (1-2 hours) - **COMPLETE** ✅ 2025-12-10
  - Flexible graph filter architecture supporting multiple filter types
  - Label filters gray out non-matching nodes (preserves context)
  - Strategic filter hides non-strategic nodes (different semantic)
  - Filter composition: strategic + label filters work together
  - LabelFilter component with autocomplete and wildcard support
  - 37 new tests (28 filter logic + 9 UI), all 95 web tests passing
  - Zero TypeScript errors, production build successful
- [x] **Phase 6:** Label validation (30 min) - **COMPLETE** ✅ 2025-12-09
  - Integrated label validation into `jit validate` command
  - Validates label format (namespace:value) for all issues
  - Checks namespaces exist in registry
  - Enforces uniqueness constraints (type, team)
  - 3 comprehensive tests (malformed, unknown namespace, duplicate unique)
  - All 578 tests passing, zero clippy warnings
  - **Note:** Separate "label audit" tool unnecessary - integrated into existing validate

**🔍 Audit Phase (Est: 18-23 hours) - 56% COMPLETE** - **2025-12-15**

**Current Status:** Week 1 COMPLETE ✅ (10 hours actual / 10 hours planned)

**Test Coverage:** ✅ **667 tests passing** (561 Rust + 95 Web + 11 MCP)  
**Documentation:** ✅ **5,603+ lines** across 9 documents + updated EXAMPLE.md + new diagrams
**Clippy Warnings:** ✅ **ZERO** (all warnings fixed)  

**Critical Issues (Must Fix Before Merge):**
- [x] 🔴 **5 clippy warnings**: All fixed - ZERO warnings ✅
- [x] 🔴 **Binary/library duplication**: Fixed - clean architecture ✅
- [x] 🔴 **Missing documentation**: Complete with examples ✅
- [x] 🔴 **E2E tests**: Added 5 comprehensive workflow tests ✅
- [x] 🔴 **Manual walkthrough**: Automated script created & validated ✅

**Quality Improvements (Should Fix Before Release):**
- [x] 🟡 Update EXAMPLE.md with label examples ✅
- [x] 🟡 Add E2E workflow test (3.5h actual) ✅
- [ ] 🟡 Performance benchmarks (1h) - DEFERRED to post-merge
- [ ] 🟡 Test with AI agent (Claude/GPT-4) (2h) - NEXT

**Audit Plan Phases:**
1. ✅ Feature completeness review (complete)
2. ✅ Documentation audit (COMPLETE - all docs updated + new diagrams)
3. ✅ Test coverage analysis (excellent: 667 tests)
4. ✅ Code quality review (COMPLETE - zero clippy warnings, clean architecture)
5. ✅ E2E testing (5 comprehensive tests + automated walkthrough script)
6. ⏳ Onboarding & usability (AI agent testing pending)
7. ⏳ Production readiness (performance benchmarks deferred)

**Progress: Week 1 COMPLETE ✅ - 10h / 10h planned (100%)**
- Day 1-2: Code quality fixes (4.5h) ✅
- Day 3-4: Documentation updates (2h) ✅
- Day 5: E2E tests + walkthrough + diagrams (3.5h) ✅

**Week 1 Deliverables:**
- 5 new E2E tests covering complete label hierarchy workflows
- Automated walkthrough script (`scripts/test-label-hierarchy-walkthrough.sh`)
- Comprehensive documentation on labels vs dependencies orthogonality
- ASCII diagrams explaining mental models
- Research workflow examples
- Zero clippy warnings, all critical blockers resolved

**See:** `docs/label-hierarchy-audit-plan.md` and `docs/week1-completion-report.md` for audit details

**Key Features:**
- Milestone = Issue with `label:milestone:v1.0`
- Epic = Issue with `label:epic:auth`
- Strategic view = Filter to milestone/epic labels
- Progress = Derived from `graph downstream`
- No separate milestone file needed

**Standard Namespaces:**
- `milestone:*` - Release goals (strategic)
- `epic:*` - Large features (strategic)
- `component:*` - Technical areas
- `type:*` - Work type (unique)
- `team:*` - Owning team (unique)

**Design Decisions (All Confirmed):**
1. Strict validation (reject malformed labels)
2. Required registry (auto-create on `jit init`)
3. Inherit all labels in breakdown
4. Target dates via context field
5. Query: exact match + wildcard (`milestone:*`)

### CLI Consistency

- [x] **Phase 1.4:** Command schema export
  - Implemented `--schema` flag for AI introspection
  - Generated JSON schemas from command definitions
  - 8 unit tests + 7 integration tests
  - Documentation in `docs/json-schema-api.md`
- [ ] **Phase 1.5:** Batch operations support

### MCP Server ✅

- [x] TypeScript MCP server wrapping CLI
- [x] 47 MCP tools covering all operations (added label + doc tools)
- [x] Schema auto-generation from CLI definitions
- [x] Fixed json flag handling (prevented double --json)
- [x] All 11 MCP tests passing
- [ ] Integration testing with MCP-compatible AI tools
- [ ] See `mcp-server/README.md` for usage

### Knowledge Management System ✅ (Phase 1-3.1 Complete)

**Phase 1.1: Document References** ✅
- [x] Document references in issues (design docs, notes, artifacts)
  - Added `DocumentReference` type to domain model
  - Fields: path, commit, label, doc_type
  - Builder methods: new(), at_commit(), with_label(), with_type()
- [x] CLI commands: `jit doc add/list/remove/show`
- [x] Updated `jit issue show` to display document references
- [x] Full JSON output support
- [x] 6 new domain tests, all 384 tests passing

**Phase 1.3: Git Integration & Validation** ✅
- [x] Git integration for version-aware references
  - Added `git2 = "0.18"` dependency
  - Extended `jit validate` to check document references
- [x] Validation of document links and commit hashes
  - Validates files exist at HEAD or specified commits
  - Validates commit hashes are valid
  - Graceful fallback for non-git repos
- [x] Document content viewing
  - `jit doc show` reads file content from git
  - Supports reading at HEAD or specific commits
- [x] **Graceful degradation without git** - **COMPLETE** ✅ 2025-12-08
  - Document content: filesystem fallback (returns "working-tree")
  - Document history: returns empty list (no error)
  - Document diff: returns graceful error message
  - All features work with or without git repository

**Phase 2.1: REST API Server** ✅
- [x] Web API server with Axum framework
  - GET /api/health - Health check
  - GET /api/issues - List all issues
  - GET /api/issues/:id - Get single issue
  - GET /api/graph - Dependency graph (nodes + edges)
  - GET /api/status - Repository status summary
- [x] CORS enabled for local development
- [x] Thread-safe InMemoryStorage (Arc<Mutex<>>)
- [x] 6 API integration tests
- [x] Server listens on http://localhost:3000

**Phase 2.2: Frontend Foundation** ✅
- [x] React + TypeScript project with Vite
- [x] Interactive graph visualization (React Flow)
- [x] Web UI with interactive graph visualization
- [x] Issue detail panel
- [x] Inline markdown document rendering with LaTeX support (KaTeX)
- [x] API client with axios (dynamic hostname)
- [x] Complete component structure
- [x] Dev server running on http://localhost:5173
- [x] API server integration (fixed storage path, CORS, type imports)
- [x] Markdown showcase with rich content (headers, tables, code, emojis)
- [x] Terminal-style dark theme with light mode toggle
- [x] Left-to-right DAG layout using dagre algorithm
- [x] Resizable split pane layout (drag separator)
- [x] Rounded node boxes with state-based coloring
- [x] Smooth curved edges with proper L->R dependency flow
- [x] Sans-serif font for markdown content, monospace for UI
- [x] Complex DAG support (multiple dependencies/dependents)
- [x] State legend with color coding
- [x] Priority indicators

**Phase 2.3: Responsive Search UI** ✅
- [x] **Backend search API**
  - GET /api/search endpoint with query parameters
  - Integrates ripgrep backend for deep content search
  - Query params: q, limit, case_sensitive, regex
  - JSON response with results, query, total, duration_ms
  - 3 new API endpoint tests
- [x] **Frontend search components**
  - SearchBar component with instant feedback
  - useSearch custom hook with hybrid client + server strategy
  - Client-side filtering (<16ms) for instant results
  - Debounced server search (300ms, min 3 chars)
  - Search results dropdown with click-to-navigate
  - ⚡ badge for instant client results
  - Result deduplication by issue ID
  - Graceful error handling with fallback
- [x] **Client-side search logic**
  - Relevance scoring: ID (20pts) > Title (10pts) > Description (5pts)
  - All search terms must match (AND logic)
  - Case-insensitive by default
  - Results sorted by score descending
  - 11 unit tests covering edge cases
- [x] **Developer experience**
  - Storage validation on startup (CLI and server)
  - Clear error messages when .jit not found
  - Instructions to run 'jit init' or set JIT_DATA_DIR
  - Environment variable support for custom repo location

**Phase 2.4: Document Viewer & Additional UI Features** ✅ (Complete)
- [x] **Inline document content viewer (Phase 2.4.1)** ✅
  - [x] Backend API endpoints for document content/history/diff
  - [x] Frontend API client with TypeScript types
  - [x] DocumentViewer React component with markdown rendering
  - [x] DocumentHistory component with commit timeline
  - [x] Modal overlay for document viewing
  - [x] Integration with IssueDetail component
  - [x] Terminal-style CSS theming
  - [x] Comprehensive test coverage (13 new tests)
  - [x] See `docs/document-viewer-implementation-plan.md` for details
- [x] **Enhanced markdown rendering (Phase 2.4.2)** ✅
  - [x] Syntax highlighting with react-syntax-highlighter (100+ languages)
  - [x] VS Code Dark+ theme matching terminal aesthetic
  - [x] Mermaid diagram rendering (flowcharts, sequences, class diagrams)
  - [x] GitHub Flavored Markdown support (tables, strikethrough, task lists)
  - [x] Custom dark theme for Mermaid diagrams
  - [x] React 19 compatibility fixes for test suite
  - [x] All 38 tests passing with zero warnings
  - [x] Improved UX: single clickable Documents section
- [ ] State transition buttons (change issue state from UI) (Deferred)
- [ ] Real-time updates (polling or WebSocket) (Deferred)
- [ ] Export graph as PNG/SVG (Deferred)
- [ ] Keyboard shortcuts (Cmd+K for search focus) (Deferred)
- [ ] Mobile responsive layout (Deferred)
- [ ] Better graph layout algorithms (elk.js) (Deferred)

**Phase 3: Advanced Features** 🚧 (In Progress)
- [x] **Full-text search with ripgrep (Phase 3.1)** ✅
  - CLI: `jit search <query> [--regex] [--glob "*.json"]`
  - Search across issues and referenced documents
  - Regex and glob pattern filtering
  - MCP tool: `search_issues`
  - 20+ tests (unit + integration + MCP)
  - Graceful degradation when ripgrep not installed
  - JSON output support for automation
  - Zero dependencies (uses system ripgrep)
- [x] **Responsive search UI (Phase 3.1b)** ✅
  - Web UI search bar with instant client-side results
  - Hybrid client + server search strategy
  - 16 tests covering search logic and integration
  - Future: Optional Tantivy backend for large repos (>1000 issues)
- [x] **Historical document viewer (Phase 3.2)** ✅
  - CLI commands: `jit doc history`, `jit doc diff`, `jit doc show --at`
  - 9 integration tests, all 490+ tests passing
  - Automatic schema generation from clap (eliminated 1,325 lines)
- [x] **Web UI document viewer (Phase 2.4.1)** ✅ - **2024-12-05**
  - REST API endpoints: `/api/issues/:id/documents/:path/{content,history,diff}`
  - DocumentViewer React component with markdown rendering + LaTeX
  - DocumentHistory component with commit timeline navigation
  - Modal overlay integration in IssueDetail
  - 13 new tests (5 backend + 8 frontend), all passing
  - Implementation plan: `docs/document-viewer-implementation-plan.md`
  - Total estimated time: 9-12 hours (actual: ~8 hours with TDD)
- [ ] **Document graph visualization (Phase 3.3)** - **DEFERRED**
  - Parse markdown links to build document-to-document graph
  - Detect circular references in documentation
  - Reverse lookup: which issues reference a document
  - Combined graph visualization (issues + documents)
  - See `docs/document-graph-implementation-plan.md` for detailed design
  - Estimated: 12-16 hours across 4 sub-phases
- [ ] Archive system (Phase 3.4)
- [ ] See `docs/knowledge-management-vision.md`, `docs/search-implementation.md`, `docs/web-ui-architecture.md`, `docs/document-viewer-implementation-plan.md`, and `docs/document-graph-implementation-plan.md` for detailed plans

### Production Readiness

- [x] **CI/CD, Packaging & Containerization** - **COMPLETE** ✅ - **2024-12-06**
  - [x] GitHub Actions workflows (ci.yml, docker.yml, release.yml, security-audit.yml)
  - [x] Comprehensive testing pipeline (Rust 490+ tests, MCP 11 tests, Web UI 38 tests)
  - [x] Docker configuration (all-in-one + specialized images for CLI, API, Web)
  - [x] Docker Compose setup with health checks
  - [x] Local CI testing with act + manual test scripts
  - [x] Installation documentation (INSTALL.md, DEPLOYMENT.md, PODMAN.md)
  - [x] Optimized workflows (path filters, separate security audits, no cache for act)
  - [x] All components tested: TypeScript fixes, MCP boolean flags, PATH setup, ripgrep installation
  - **Status:** Production-ready CI/CD pipeline, tested with local act runner
  
- [x] **File locking for multi-agent safety** - **COMPLETE** ✅
  - [x] Research locking strategy (flock vs advisory locks vs process-based locking) - **Decision: fs4 with advisory locks**
  - [x] Add locking abstraction to storage layer (lock_file/unlock_file methods) - **FileLocker + LockGuard**
  - [x] Implement file-level locking for atomic operations (index.json, individual issues) - **Phase 1.1 Complete**
    - Created `FileLocker` with timeout support and built-in retry (10ms polling)
    - Created `LockGuard` with RAII pattern
    - Exclusive and shared locks implemented
    - Try-lock non-blocking variants
    - 6 comprehensive unit tests
    - All 338 tests passing
  - [x] Update JsonFileStorage to acquire locks before write operations - **Phase 1.2 Complete**
    - Uses separate .lock files to avoid conflicts with atomic writes
    - Lock ordering: index first, then issue (prevents deadlocks)
    - All operations protected: save, load, delete, list, gates, events
    - 7 concurrent tests including 50-thread stress test
    - All 362 tests passing, zero clippy warnings
  - [x] Add lock timeout configuration (via environment variables) - **JIT_LOCK_TIMEOUT**
  - [x] Add tests for concurrent access patterns (parallel creates, updates, dependency adds) - **7 comprehensive tests**
  - [x] Document locking semantics and performance implications - **docs/file-locking-usage.md**
  - [x] Concurrent testing with realistic workload - **scripts/test-concurrent-mcp.sh (50 creates + 200 reads)**
  - [~] Add retry logic with exponential backoff - **Deferred: FileLocker already has built-in retry via polling**
  
  **Status:** Production-ready for concurrent multi-agent access. Tested with 50 concurrent operations, zero data corruption.
- [ ] Plugin system for custom gates
- [ ] Prometheus metrics export
- [ ] Web dashboard (optional)
- [ ] Alert system: `alert add --condition "..."`
- [ ] Cross-repository issue linking
- [ ] Performance optimization (if needed)
- [ ] Comprehensive error recovery

## Infrastructure & Deployment ✅

- [x] **GitHub Actions CI/CD** ✅
  - ci.yml: Rust tests (490+), MCP tests (11), Web UI tests (38), lint, build
  - docker.yml: Build and push images to GHCR (cli, api, web, all-in-one)
  - release.yml: Create releases with binaries, checksums, artifacts
  - security-audit.yml: Weekly dependency audits (Rust + npm)
  - Path filters to skip documentation-only changes
  - Optimizations: 40-60% faster CI pipeline
- [x] **Docker & Containerization** ✅
  - All-in-one image: CLI + API + MCP + Web UI (~300 MB)
  - Specialized images: CLI (24 MB), API (24 MB), Web (30 MB)
  - Docker Compose with health checks
  - Multi-stage builds with Alpine Linux
  - Static binaries with musl
- [x] **Local Testing** ✅
  - test-ci-manual.sh: Direct command testing (5-8 min)
  - test-ci-local.sh: Act-based GitHub Actions simulation
  - test-podman.sh: Container testing with Podman
  - validate-setup.sh: Pre-commit validation
- [x] **Documentation** ✅
  - INSTALL.md: Multiple installation methods
  - DEPLOYMENT.md: Production deployment guide
  - PODMAN.md: Podman-specific guide with SQLite migration
  - Comprehensive troubleshooting sections

## Dependencies

- Phase 1 → Phase 2 (core needed for gates)
- Phase 2 → Phase 3 (events needed for observability)
- Phase 3 → Phase 4 (stable features before hardening)
- Infrastructure: Ready for production deployment

## Success Metrics

- **Phase 1:** Can track issues with dependencies, detect cycles ✅
- **Phase 2:** Gates block transitions, events logged ✅
  - **Web UI:** Interactive graph visualization with document viewer ✅
  - **REST API:** Complete CRUD + search + documents endpoints ✅
  - **Document viewer:** View, history, diff support in web UI ✅
- **Phase 3:** Full observability, external integration working ✅
  - **File locking:** Multi-agent safe, tested with 50 concurrent operations ✅
  - **MCP server:** 29 tools, TypeScript wrapper complete ✅
  - **Documentation:** Comprehensive usage guides ✅
  - **Search:** Full-text search with ripgrep integration ✅
  - **Historical documents:** CLI and web UI support ✅
- **Phase 4:** Production-grade reliability, advanced features ✅
  - **Label system:** All phases complete (1-6 + 5.3) ✅ **2025-12-10**
    - Core labels, namespace registry, strategic queries, MCP tools ✅
    - Label badges in web UI ✅
    - Strategic/tactical view toggle with downstream stats ✅
    - Label validation integrated into `jit validate` ✅
    - **Label filtering in web UI with gray-out approach** ✅
      - Flexible filter architecture supporting multiple filter types
      - Label filters dim non-matching nodes (preserves context)
      - Strategic filters hide non-strategic nodes
      - 37 new tests (28 filter logic + 9 UI)
      - Fixed all React Testing Library warnings
      - Zero TypeScript errors, production build successful
    - 95 web tests + 578 Rust tests passing ✅
    - **Documentation clarified type vs membership labels** ✅
      - docs/label-quick-reference.md: Quick guide for agents
      - docs/label-enforcement-proposal.md: Programmatic validation proposal (~3-4h)
      - Updated label-conventions.md with clear distinction
  - **Type hierarchy enforcement:** 🚧 **IN PROGRESS** - **2025-12-14**
    - [x] **Phase A: Core Module (2 hours)** - Pure validation library ✅
      - [x] Create `crates/jit/src/type_hierarchy.rs` module (370 lines)
      - [x] `HierarchyConfig` struct with configurable levels (default 4-level)
      - [x] Error types with `thiserror` (HierarchyError, ConfigError)
      - [x] Pure validation functions (extract_type, validate_hierarchy)
      - [x] Type normalization (lowercase, trim whitespace)
      - [x] 12 unit tests (config validation, extraction, hierarchy logic)
      - [x] 6 property-based tests (transitivity, no cycles, monotonic levels, normalization)
      - [x] Zero clippy warnings, all functions documented
      - **Key design**: Orthogonal to DAG (validates organizational membership, not logical dependencies)
    - [x] **Phase B: CLI Integration (1.5 hours)** - Command integration ✅
      - [x] `add_dependency`: Enforce hierarchy constraints BEFORE cycle detection
      - [x] Graceful degradation: Skip validation if either issue lacks type label
      - [x] Clear error messages: "Type hierarchy violation: Type 'epic' depends on lower-level type 'task' (level 2 -> 4)"
      - [x] 5 integration tests (same-level, lower-to-higher, higher-to-lower, no-type, mixed)
      - [x] CLI verification with bash script (epic→task rejected, task→epic allowed)
      - [x] 220 total tests passing (215 baseline + 5 new)
    - [x] **Phase C.0: Code Modularization (2-3 hours)** - Technical debt reduction ✅
      - [x] Split commands.rs (4,278 lines) into logical modules:
        - [x] `commands/issue.rs` (355 lines) - Issue CRUD operations
        - [x] `commands/dependency.rs` (89 lines) - Dependency graph operations
        - [x] `commands/gate.rs` (130 lines) - Gate operations
        - [x] `commands/graph.rs` (62 lines) - Graph visualization
        - [x] `commands/query.rs` (185 lines) - Query operations
        - [x] `commands/validate.rs` (222 lines) - Validation logic
        - [x] `commands/labels.rs` (79 lines) - Label operations
        - [x] `commands/breakdown.rs` (48 lines) - Issue breakdown operations
        - [x] `commands/document.rs` (524 lines) - Document operations
        - [x] `commands/events.rs` (42 lines) - Event log operations
        - [x] `commands/search.rs` (59 lines) - Search operations
        - [x] `commands/mod.rs` (123 lines) - CommandExecutor struct and re-exports
      - [x] Clean module structure with proper visibility (pub(super) for helpers)
      - [x] Zero regressions - all 496 tests pass
      - [x] Zero clippy errors
      - [x] Total: 1,918 lines across 12 files (55% reduction from original)
      - **Rationale:** Prevent commands.rs from becoming unmaintainable, improve discoverability
      - **Implementation:** No hacks or shortcuts - clean Rust idioms throughout
    - [x] **Phase C.1: Config File Consolidation (1 hour)** - Single source of truth ✅
      - [x] Renamed `.jit/label-namespaces.json` → `.jit/labels.json`
      - [x] Extended LabelNamespaces struct to include `type_hierarchy: Option<HashMap<String, u8>>`
      - [x] Schema version bumped to 2, type_hierarchy includes default 4-level hierarchy
      - [x] Storage layer updated to use `labels.json` filename
      - [x] Added `get_type_hierarchy()` helper with fallback to defaults
      - [x] All 496 tests passing, zero clippy errors
      - [x] CLI verified: `jit label namespaces --json` includes type_hierarchy
      - **Rationale:** Single source of truth, prevent config drift, clearer naming
      - **Implementation:** No backward compatibility - clean break (no users yet)
    - [x] **Phase C.2: Hierarchy Config Commands (1 hour)** - Discoverability ✅
      - [x] `jit config show-hierarchy`: Display current type hierarchy with levels
      - [x] `jit config list-templates`: Show available hierarchy templates  
      - [x] `jit init --hierarchy-template <name>`: Initialize with template
      - [x] Templates: default (4-level), extended (5-level), agile (4-level), minimal (2-level)
      - [x] Template storage: Built-in code in hierarchy_templates module
      - [x] Config loading: get_type_hierarchy() helper with fallback to defaults
      - [x] 5 tests for templates (all, get, extended, minimal, nonexistent)
      - [x] All 506 tests passing, zero clippy errors
      - **Implementation:** Clean module-based design, no file I/O for templates
    - [x] **Phase D: Type Label Validation** - Type validation only ✅ **2025-12-14**
      - [x] `validate`: Detect unknown type labels
      - [x] `validate --fix`: Repair typos using Levenshtein distance
      - [x] Dry-run mode: `--fix --dry-run` shows changes without applying
      - [x] JSON output support (--fix --json)
      - [x] Quiet mode for JSON output
      - [x] 7 integration tests for type validation
      - [x] All tests passing, zero clippy warnings (with allow(dead_code) on incomplete features)
      - **CRITICAL FIX**: Removed incorrect dependency validation (commit a8fefe2)
        - Dependencies are NOT restricted by type hierarchy
        - Type hierarchy only validates organizational membership
        - See `docs/session-notes-hierarchy-bug-fix.md` for details
    - [x] **Phase E: Membership Validation** - Label-based references ✅ **2025-12-14**
      - [x] Implemented `detect_membership_issues()` function
      - [x] Validates `epic:auth` references point to actual issues with `type:epic`
      - [x] Validates `milestone:*`, `story:*`, and custom membership labels
      - [x] Self-references allowed (epic identifying itself)
      - [x] Multiple membership labels supported
      - [x] Added `label_associations` field to `HierarchyConfig`
      - [x] Dynamic namespace registration from `label_associations`
      - [x] Removed ALL hard-coded namespace names (commit d6891c5)
      - [x] 10 comprehensive integration tests
      - [x] Custom type names work automatically (e.g., "theme" instead of "epic")
      - [x] Type aliases supported (e.g., "release" and "milestone" both use `milestone:*`)
      - [x] 167 total tests passing
      - **Key Achievement**: Fully configuration-driven, zero hard-coding
    - **Documentation**: 
      - docs/type-hierarchy-enforcement-proposal.md: Full design (~920 lines)
      - docs/type-hierarchy-implementation-summary.md: Implementation guide (~330 lines)
      - docs/session-notes-hierarchy-bug-fix.md: Critical bug fix documentation
      - docs/session-2025-12-14-membership-validation.md: Phase E implementation notes
    - **Estimated total effort**: ~12 hours actual (Phases A-E complete!)
  - **Code Quality Improvements** - Identified 2025-12-14
    - [ ] **Argument ordering consistency** (low priority)
      - [ ] Consider standardizing create_issue arg order: (title, desc, labels, priority, gates)
      - [ ] Document convention for future APIs
      - [ ] Note: Breaking change, defer until major version bump
    - [ ] **Error type improvements** (medium priority)
      - [ ] Consider custom error types for DAG operations (GraphError)
      - [ ] Structured errors for programmatic handling (MCP, API)
      - [ ] Keep anyhow for CLI, add thiserror for library errors
      - [ ] Benefits: Better error recovery, typed error handling
    - [ ] **Test organization** (low priority, future)
      - [ ] If test suite grows >5000 lines, split into `tests/commands/` modules
      - [ ] Keep integration tests in main test module for now
  - **Next priorities after type hierarchy:**
    - **Phase F: Warning-Level Validations** ✅ **COMPLETE** - **2025-12-14**
      - [x] Add `ValidationWarning` enum (MissingStrategicLabel, OrphanedLeaf)
      - [x] Implement `validate_strategic_labels()` - check epic has epic:* label
      - [x] Implement `validate_orphans()` - detect tasks without epic/milestone labels
      - [x] 17 comprehensive tests (10 default + 7 custom hierarchies)
      - [x] Fully configuration-driven validation
      - Actual: ~1 hour (estimated 2 hours)
    - **Phase G: CLI Warning Integration** ✅ **COMPLETE** - **2025-12-14**
      - [x] Add `--force` flag to `jit issue create` to bypass warnings
      - [x] Add `--orphan` flag to acknowledge intentional orphans
      - [x] Check strategic consistency on issue creation (warn if missing label)
      - [x] Check orphan status on task creation (warn if no parent)
      - [x] Display warnings with clear, actionable messages
      - [x] Update `jit validate` command to report warnings
      - [x] JSON output for warnings in validate
      - [x] 4 integration tests (check_warnings method)
      - Actual: ~1.5 hours (matched estimate)
    - **Phase H: Configuration Support** ✅ **COMPLETE** - **2025-12-15**
      - [x] Add config.toml parsing for type_hierarchy section
      - [x] Support strictness levels (strict/loose/permissive)
      - [x] Support warn_orphaned_leaves, warn_strategic_consistency flags
      - [x] Add root() method to IssueStore trait
      - [x] Implement config loading in CommandExecutor
      - [x] 5 integration tests + 5 unit tests in config module
      - [x] Example config.toml in docs/
      - [x] All 556 tests passing, zero clippy errors
      - Actual: ~1.5 hours (matched estimate)
    - **Next High Priority: Code Quality & Architecture**
      - **Phase I: Binary-to-Library Refactoring** ✅ **COMPLETE** - **2025-12-15**
        - [x] Remove module duplication between main.rs and lib.rs
        - [x] Use `use jit::{...}` imports instead of `mod` declarations in binary
        - [x] Keep only `mod output_macros;` in main.rs (binary-specific)
        - [x] Update all imports to use `jit::` prefix
        - [x] Test compilation time: ~15.6s clean release build (baseline established)
        - [x] Verify all 556 tests still pass
        - [x] Update documentation with completion status
        - Actual: ~2 hours (matched estimate)
        - Priority: HIGH (reduces confusion, follows Rust best practices) ✅ DONE
        - See: `docs/refactoring-plan-binary-to-library.md` for full plan
      - **Issue #1: Inconsistent new() Methods** (defer to Phase J)
        - [ ] Add `try_new()` variants for validation where appropriate
        - [ ] `JsonFileStorage::try_new()` - validates path exists
        - [ ] Document pattern: `new()` never fails, `try_new()` validates
        - Estimated: 30 minutes
        - Priority: LOW (nice-to-have, non-breaking)
      - **Issue #2: create_issue() Argument Order** (defer to v1.0)
        - [ ] Standardize to: (title, desc, labels, priority, gates)
        - [ ] Update all callsites (grep "create_issue")
        - [ ] Breaking change - requires major version bump
        - Estimated: 1 hour
        - Priority: LOW (breaking change, defer to v1.0)
      - **Issue #3: Config Caching** (defer until needed)
        - [ ] Add optional config cache to CommandExecutor
        - [ ] Profile to confirm it's actually a bottleneck first
        - Estimated: 1 hour
        - Priority: LOW (premature optimization)
    - **Future enhancements:**
      - Option A: Document graph visualization (12-16 hours)
      - Option B: Plugin architecture for custom gates
      - Option C: Performance benchmarks and optimization
      - Option D: Circular membership detection
      - Option E: Membership hierarchy constraints (story can't belong to task)
- **CI/CD Infrastructure:** Complete and tested ✅
  - All workflows validated with YAML syntax checks
  - Tested locally with act and manual scripts
  - Ready for first push to GitHub and production release

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
