# Documentation Reorganization - File Classification Analysis

**Issue:** 165cf162-1cb1-491d-8c92-b2fb571e7f4c  
**Date:** 2025-12-22  
**Total Files:** 63 markdown files + example-config.toml

## Classification Strategy

Based on the documentation organization strategy, files are classified into:

1. **dev/active/** - Designs linked to open issues (active development)
2. **dev/architecture/** - Core system architecture (permanent internal reference)
3. **dev/vision/** - Future exploration and planning
4. **dev/studies/** - Completed investigations, analyses, and explorations
5. **dev/sessions/** - Development session notes
6. **docs/** - Keep docs/README.md only (product documentation index)

---

## Classification Results

### dev/archive/ (Completed Work from Pre-JIT Era)

Documents that are completed and no longer actively referenced (can be archived immediately):

**Refactorings (→ dev/archive/refactorings/):**
1. `orchestrator-separation.md` - ✅ Completed 2025-11-30
2. `refactoring-plan-binary-to-library.md` - ✅ Completed 2025-12-15
3. `storage-abstraction.md` - ✅ Completed 2025-12-02
4. `generic-dag-refactoring.md` - ✅ Completed 2025-12-02
5. `state-model-refactoring.md` - Approved, may be in progress (check if done)

**Bug Fixes (→ dev/archive/bug-fixes/):**
1. `gate-enforcement-bug-analysis.md` - Completed bug analysis
2. `test-warnings-resolution.md` - Completed resolution

**Features (→ dev/archive/features/):**
1. `json-schema-api.md` - Status: Complete
2. `file-locking-design.md` - Complete (has companion usage guide)
3. `file-locking-usage.md` - Complete (part of file locking feature)
4. `search-implementation.md` - Completed implementation

**Studies (→ dev/archive/studies/):**
1. `label-hierarchy-audit-plan.md` - ✅ Completed audit
2. `label-hierarchy-implementation-plan.md` - ✅ Completed implementation
3. `type-hierarchy-enforcement-proposal.md` - Proposal (check if superseded)
4. `type-hierarchy-implementation-summary.md` - ✅ Summary of completed work
5. `week1-completion-report.md` - Completed report
6. `document-search-performance.md` - Status: Production Implementation (analysis done)
7. `gate-preview-analysis.md` - Completed analysis

**Sessions (→ dev/archive/sessions/):**
1. `session-2025-12-02-complete.md` - December 2 session
2. `session-2025-12-02-mcp-server.md` - December 2 session
3. `session-2025-12-03-frontend.md` - December 3 session
4. `session-2025-12-14-membership-validation.md` - December 14 session
5. `session-2025-12-15-phase-h-implementation.md` - ✅ Complete, all tests passing
6. `session-notes-hierarchy-bug-fix.md` - 🚨 INCOMPLETE (keep in dev/sessions for now)

**Note:** session-2025-12-21-short-hash-progress.md is recent and linked to active work, keep in dev/sessions/

**Total Archive Candidates: ~25 files**

### dev/active/ (Active Designs - Linked to Open Issues)

Files referenced in open issues (active work):

1. `agent-validation-design.md` ✓
2. `bulk-operations-plan.md` ✓
3. `ci-gate-integration-design.md` ✓
4. `documentation-lifecycle-design.md` ✓
5. `documentation-lifecycle-phase2-design.md` ✓
6. `gate-examples.md` ✓
7. `json-output-standardization-plan.md` ✓
8. `observability-design.md` ✓
9. `production-polish-design.md` ✓
10. `production-stability-design.md` ✓
11. `quiet-mode-plan.md` ✓
12. `rejection-state-design.md` ✓
13. `state-transition-feedback-design.md` ✓
14. `transitive-reduction-validation-plan.md` ✓

**Total: 14 files**

### dev/architecture/ (Core System Architecture)

Permanent internal architecture reference documents:

1. `design.md` → **core-system-design.md** (rename for clarity)
2. `storage-abstraction.md` ✓
3. `web-ui-architecture.md` ✓
4. `graph-filtering-architecture.md` ✓
5. `cli-and-mcp-strategy.md` ✓
6. `file-locking-design.md` ✓
7. `file-locking-usage.md` ✓
8. `orchestrator-separation.md` (completed, foundational)

**Total: 8 files**

### dev/vision/ (Future Planning)

Forward-looking exploration and vision documents:

1. `knowledge-management-vision.md` ✓
2. `document-graph-implementation-plan.md` ✓
3. `document-viewer-implementation-plan.md` ✓
4. `auth-design.md` (future feature)
5. `billing-design.md` (future feature)

**Total: 5 files**

### dev/studies/ (Active Investigations & Reference)

Ongoing investigations, analyses, explorations, and active reference documents:

1. `documentation-organization-strategy.md` (already in dev/studies/) ✓
2. `architecture-pitfalls.md` - Active reference ✓
3. `clippy-suppressions.md` - Active reference ✓
4. `dependency-vs-labels-clarity.md` - Active reference ✓
5. `generic-hierarchy-model.md` - Exploration ✓
6. `label-conventions.md` - Active reference/guide ✓
7. `label-enforcement-proposal.md` - Proposal ✓
8. `label-quick-reference.md` - Active reference guide ✓
9. `labels-config-consolidation.md` - Design plan ✓
10. `short-hash-implementation-plan.md` - May be linked to active work ✓

**Total: 10 files** (down from 24, most moved to archive)

### dev/sessions/ (Recent Development Session Notes)

Active/recent session notes (session-2025-12-21 and incomplete sessions):

1. `session-2025-12-21-short-hash-progress.md` - Recent, linked to active work ✓
2. `session-notes-hierarchy-bug-fix.md` - 🚨 INCOMPLETE, needs work ✓

**Total: 2 files** (others archived)

**Note:** Older completed sessions (Dec 2-15) moved to dev/archive/sessions/

### docs/ (Product Documentation)

Keep in place for now:

1. `README.md` - Main documentation index ✓
2. `diagrams/` - Keep diagrams directory

**Note:** User-facing guides to be created in future phases:
- `getting-started-complete.md` - Could become docs/tutorials/getting-started.md (Phase 2)
- `research-workflow-examples.md` - Could become docs/how-to/research-projects.md (Phase 2)
- `agent-context-mcp.md` - Agent-specific reference (could go to docs/reference/ or dev/)
- `agent-project-initialization-guide.md` - Agent-specific guide (could go to docs/how-to/ or dev/)

**Total: 2 items (README.md + diagrams/)**

### Other Files

1. `example-config.toml` → Move to `docs/reference/example-config.toml`

---

## Special Consideration Files

These files need decision on placement:

### Agent-Specific Documentation
- `agent-context-mcp.md` - MCP agent quick reference
- `agent-project-initialization-guide.md` - Agent initialization guide

**Recommendation:** Move to `dev/` since they're contributor/agent-facing, not end-user product docs.
- → `dev/architecture/agent-context-mcp.md`
- → `dev/architecture/agent-project-initialization-guide.md`

### Getting Started Guides
- `getting-started-complete.md` - Comprehensive getting started guide
- `research-workflow-examples.md` - Domain-specific examples

**Recommendation:** These are product-facing but incomplete. Options:
1. Move to `dev/studies/` temporarily until Phase 2 creates proper tutorials
2. Move directly to `docs/tutorials/` and refine in Phase 2

**Decision:** Move to `dev/studies/` for now, migrate to `docs/tutorials/` in Phase 2.

---

## Summary Statistics

| Category | Count | Destination |
|----------|-------|-------------|
| **Archive** | ~25 | `dev/archive/{refactorings,bug-fixes,features,studies,sessions}` |
| Active Designs | 14 | `dev/active/` |
| Architecture | 8 | `dev/architecture/` |
| Vision | 5 | `dev/vision/` |
| Studies | 10 | `dev/studies/` |
| Sessions | 2 | `dev/sessions/` |
| Product Docs | 2 | `docs/` (README.md, diagrams/) |
| Special Cases | 4 | See recommendations above |
| Config | 1 | `docs/reference/` |

**Total:** 71 items classified (including archived items)

---

## Next Steps

1. ✅ Review classification with stakeholder
2. Execute file moves based on approved classification
3. Update all internal links in moved files
4. Update issue document references in `.jit/issues/*.json`
5. Create index files (`docs/index.md`, `dev/index.md`)
6. Configure archival policy in `.jit/config.toml`
7. Update top-level docs (README.md, CONTRIBUTOR-QUICKSTART.md)
8. Verify no broken links

---

## Files Requiring Link Updates (High Priority)

Files with many internal cross-references that will need link updates:
- `docs/README.md` - Central navigation hub
- Active design files referencing other docs
- Session notes referencing design files
