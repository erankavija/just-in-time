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
- `EXAMPLE.md` - Workflows and examples

### Phase 5: Quality Gate System (In Design)

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

**Next: Implementation (Phase 5.1)**
- [ ] Core infrastructure: versioned Gate and GateRunResult structs
- [ ] Exec checker with timeout, output capture, Git commit recording
- [ ] Postcheck execution on issue completion
- [ ] Precheck execution on starting work  
- [ ] CLI commands: define, check, pass/fail gates
- [ ] Structured result storage in `.jit/gate-runs/`
- [ ] Event logging for all gate operations

**Future Extensions (Phase 5.2+)**:
- [ ] Additional checker types (docker, http, artifact)
- [ ] Gate dependencies and parallelization
- [ ] Conditional gates
- [ ] PR/branch protection integration
- [ ] External CI result ingestion

---

## Current Focus

**System Status**: Production-ready for core use case

**In Design Phase**: Quality gate system for automated checks and TDD workflows

**Completed Features:**
- Issue tracking with dependency graphs
- Label hierarchy with type validation
- Knowledge management with document linking
- Interactive web UI with strategic views
- MCP server for AI agent integration
- Multi-agent safe concurrent operations

**Recommended Next Steps:**
1. Implement gate system MVP (Phase 5.1)
2. Use gates in production to validate design
3. Gather feedback on agent usability
4. Implement advanced gate features as needed

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
