# Just-In-Time Roadmap

## Completed Phases ✅

### Phase 0: Design
Core design document, file layout, CLI specification, and Rust implementation foundation.

### Phase 1: Core Issue Management
Basic issue tracker with dependency graph enforcement, cycle detection, and DAG validation.

### Phase 2: Quality Gates & Query Interface
Gate enforcement, event logging, query interface, and JSON output for automation.

### Phase 3: Orchestrator & External Integrations
Separate jit-dispatch orchestrator, storage abstraction, generic DAG, CLI consistency, and graph export.

**Deferred Items:**
- [ ] Stalled work detection
- [ ] Bulk operations
- [ ] Pull-based agent mode
- [ ] Metrics reporting
- [ ] Webhooks for orchestrator events

**CI/Quality Gates - In Design Phase:**
- [x] **Gate Design**: Comprehensive design for automated quality gates (see docs/ci-gate-integration-design.md)
  - Prechecks (before work) and postchecks (after work)
  - Manual gates (reminders, checklists) and automated gates (script execution)
  - Versioned schemas for future-proofing
  - TDD workflow support with example configurations
- [ ] **Gate Implementation**: MVP with exec checker, state transition hooks, and result storage
- [ ] **Gate Examples**: Rust, Python, JavaScript templates (see docs/gate-examples.md)

### Phase 4: Advanced Features & Production Hardening

**Completed:**
- ✅ **Label System**: Labels-based hierarchy (milestone, epic, task) with validation, querying, and strategic views
- ✅ **Type Hierarchy**: Configurable validation with templates, auto-fix, membership validation, and warning system
- ✅ **Knowledge Management**: Document references, git integration, full-text search with ripgrep, document history and diff
- ✅ **Web UI**: Interactive graph visualization, document viewer with markdown/LaTeX, search with highlighting, strategic/tactical toggle, label filtering
- ✅ **MCP Server**: TypeScript wrapper with 47 tools, schema auto-generation
- ✅ **CI/CD**: GitHub Actions workflows, Docker containerization, multi-stage builds
- ✅ **File Locking**: Multi-agent safe concurrent access with fs4

**In Progress / Deferred:**

#### Label System Post-Merge
- [ ] Performance benchmarks
- [ ] AI agent validation testing

#### CLI Consistency
- [ ] Batch operations support

#### MCP Server
- [ ] Integration testing with AI-compatible tools

#### Knowledge Management - Advanced
- [ ] Document graph visualization (see docs/document-graph-implementation-plan.md)
- [ ] Archive system

#### Web UI Enhancements
- [ ] State transition buttons (change issue state from UI)
- [ ] Real-time updates (polling or WebSocket)
- [ ] Export graph as PNG/SVG
- [ ] Keyboard shortcuts
- [ ] Mobile responsive layout
- [ ] Better graph layout algorithms (elk.js)

#### Production Readiness
- [ ] Plugin system for custom gates
- [ ] Prometheus metrics export
- [ ] Web dashboard
- [ ] Alert system
- [ ] Cross-repository issue linking
- [ ] Performance optimization (if needed)
- [ ] Comprehensive error recovery

---

## Documentation

### Core Documentation
- `docs/design.md` - Comprehensive design
- `docs/cli-and-mcp-strategy.md` - CLI consistency and MCP architecture
- `docs/knowledge-management-vision.md` - Long-term vision

### Label System
- `docs/label-conventions.md` - Format rules and agent usage
- `docs/label-quick-reference.md` - Quick guide
- `docs/type-hierarchy-enforcement-proposal.md` - Complete design
- `docs/label-hierarchy-audit-plan.md` - Audit and validation

### Implementation
- `docs/storage-abstraction.md` - Storage layer architecture
- `docs/generic-dag-refactoring.md` - DAG abstraction
- `docs/file-locking-usage.md` - Multi-agent concurrency
- `docs/search-implementation.md` - Full-text search
- `docs/web-ui-architecture.md` - Frontend architecture

### Guides
- `INSTALL.md` - Installation methods
- `DEPLOYMENT.md` - Production deployment
- `TESTING.md` - Test strategy
- `docs/tutorials/` - Quickstart and workflow examples

### Phase 5: Quality Gate System

**Design Complete:**
- ✅ **Gate Framework Design**: Comprehensive design for automated quality gates
  - Two stages: prechecks (before work) and postchecks (after work)  
  - Two modes: manual (reminders, checklists) and automated (script execution)
  - Versioned schemas for future extensions (docker, http, artifact checkers)
  - Commit-aware result storage for future PR/branch protection
  - See: `docs/ci-gate-integration-design.md`

- ✅ **Example Configurations**: Practical gate configurations
  - TDD workflow examples (Rust, Python, JavaScript)
  - Context validation patterns (manual and automated)
  - Security gates (audit, secret detection)
  - Quick setup templates
  - See: `docs/gate-examples.md`

**Phase 5.1: Core Infrastructure (✅ Complete)**
- ✅ Core infrastructure: versioned Gate and GateRunResult structs
- ✅ Exec checker with timeout, output capture, Git commit recording (graceful degradation)
- ✅ Postcheck execution on issue completion (auto-transitions to Done if all pass)
- ✅ Precheck execution on starting work (blocks transition if fails)
- ✅ Gate checking methods: check_gate, check_all_gates, run_prechecks, run_postchecks
- ✅ Structured result storage in `.jit/gate-runs/`
- ✅ State transition integration (update_issue_state with hooks)
- ✅ Working directory logic (repo root for production, current_dir for tests)
- ✅ Comprehensive test coverage (17 gate CLI tests + 173 existing = 190 total tests passing)
- ✅ CLI commands: define, list, show, remove, check, check-all gates
- ✅ Enhanced gate definition command with stage/mode/checker
- ✅ Event logging for gate check operations
- ✅ JSON output support for all gate commands

**Phase 5.2: Agent-Friendly Additions**
- [ ] **Precheck Preview**: `jit gate preview <issue-id>` - run prechecks without state change
  - Cache result for 5 minutes
  - Gives agents discoverability without triggering lazy eval
  - Returns `--json` with predicted precheck results
- [ ] **Enhanced Observability**:
  - `jit gate history <issue>` - show all gate runs for an issue
  - `jit gate runs <gate-key>` - show all runs of a specific gate
  - Filter by status, time range
- [ ] **Improved Error Messages**:
  - Show relevant output excerpts on failure
  - Suggest fixes based on common patterns
  - Link to full logs in `.jit/gate-runs/`

**Phase 5.3: Advanced Features (Future)**
- [ ] Additional checker types (docker, http, artifact)
- [ ] Gate dependencies and parallelization
- [ ] Conditional gates (when conditions, use `skipped` status)
- [ ] PR/branch protection integration
- [ ] External CI result ingestion
- [ ] Gate templates/presets (`jit gate define --preset rust-tdd`)
- [ ] Multi-approver gates (RBAC layer)

---

## Current Focus

**System Status**: Production-ready with complete gate system (Phase 5.1)

**Latest Completion**: Phase 5.1 - Quality gate system CLI (2025-12-18)

**Completed Features:**
- Issue tracking with dependency graphs
- Label hierarchy with type validation
- Knowledge management with document linking
- Interactive web UI with strategic views
- MCP server for AI agent integration
- Multi-agent safe concurrent operations
- Quality gate system with automated checkers

**Recommended Next Steps:**
1. **Use in production**: Apply gate system to real workflows (TDD, CI/CD integration)
2. **Documentation**: Update user guides with gate examples
3. **Agent testing**: Validate gate usability with AI agents
4. **Phase 5.2**: Implement agent-friendly additions (gate preview, enhanced observability)
5. **Phase 5.3**: Add advanced features based on production feedback

---

**Core Features Complete:**
- Issue tracking with dependency graphs
- Label hierarchy with type validation
- Knowledge management with document linking
- Interactive web UI with strategic views
- MCP server for AI agent integration
- Multi-agent safe concurrent operations

**Recommended Next Steps:**
1. Use the system in production to identify priority enhancements
2. Gather feedback on AI agent usability
3. Prioritize remaining features based on actual needs

---

## Reference

For detailed implementation history and completion status, see:
- `ROADMAP-backup-YYYYMMDD.md` - Detailed historical roadmap
- `docs/week1-completion-report.md` - Recent audit completion
- `docs/label-hierarchy-audit-plan.md` - Feature audit details
