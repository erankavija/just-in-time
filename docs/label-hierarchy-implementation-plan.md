# Label-Based Issue Hierarchy - Complete Implementation Plan

**Date**: 2025-12-07  
**Status**: Ready for implementation  
**Approach**: Labels + dependency graph (no separate milestone file)

---

## Executive Summary

**Core Principle**: Issue hierarchy is defined through labels, not separate storage files.

- **Milestone** = Issue with `label:milestone:*`
- **Epic** = Issue with `label:epic:*`
- **Task** = Issue with specific epic/component labels
- **Progress** = Derived from dependency graph (`jit graph downstream`)
- **Strategic view** = Filter by milestone/epic labels

**Estimated Total Time**: 8-12 hours

---

## Design Decisions Made

### ✅ DECIDED: Labels, not separate file
- Milestones are issues (leverage existing graph)
- No `milestones.json` file needed
- Progress calculated from dependencies

### ✅ DECIDED: Enforced label format
- Pattern: `namespace:value`
- Regex: `^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$`
- Reject malformed labels with suggestions

### ✅ DECIDED: Namespace registry
- Store in `.jit/label-namespaces.json`
- Properties: description, unique, strategic
- Extensible for custom namespaces

### ✅ DECIDED: Standard namespaces
- `milestone:*` - Release goals (strategic)
- `epic:*` - Large features (strategic)
- `component:*` - Technical areas
- `type:*` - Work type (unique)
- `team:*` - Owning team (unique)

### ✅ DECIDED: Strategic vs tactical
- Strategic = issues with `milestone:*` or `epic:*` labels
- Tactical = all issues (default view)
- Web UI toggle between views

---

## Design Decisions - ALL CONFIRMED ✅

### ✅ Validation: Strict (Option A)
- Reject labels that don't match regex
- Warn if namespace not in registry (but allow)
- Warn if similar values exist (e.g., `v1.0` vs `1.0`)
- Better for AI agents

### ✅ Registry: Required (Option A)
- `.jit/label-namespaces.json` created on `jit init`
- Contains standard namespaces (milestone, epic, component, type, team)
- Users can add custom namespaces via CLI
- Labels using non-registry namespaces: warn but allow

### ✅ Breakdown Inheritance: All labels (Option A)
- Subtasks inherit ALL labels from parent
- Agents can remove unwanted labels after
- Simpler implementation

### ✅ Target Dates: Context field (Option A)
```bash
jit issue update $MILESTONE --context "target_date:2026-03-01"
```
- No schema changes needed
- Target dates are optional metadata
- Can add dedicated field later if needed

### ✅ Query Syntax: Exact + wildcard (Option A)
```bash
jit query label "milestone:v1.0"      # Exact match
jit query label "milestone:*"         # All milestones
jit query label "epic:*" --or label "milestone:*"  # Strategic view
```
- Clear semantics
- Fast performance
- Can add regex support later if needed

---

## Implementation Plan

### Phase 1: Core Labels (3-4 hours)

**Goal**: Add labels to issues, basic CLI commands

#### 1.1 Domain Model
- [ ] Add `labels: Vec<String>` to `Issue` struct
- [ ] Update serialization (backward compatible)
- [ ] Schema version bump (if needed)

**Files changed:**
- `crates/jit/src/domain.rs`
- Migration: Existing issues get `labels: []`

**Tests**: 5-10 unit tests

#### 1.2 Label Validation
- [ ] Implement `validate_label()` with regex
- [ ] Error messages with suggestions
- [ ] Helper to parse `namespace:value`

**Files changed:**
- `crates/jit/src/labels.rs` (new module)

**Tests**: 10-15 validation tests

#### 1.3 CLI Commands - Basic
- [ ] `jit issue create --label "key:value"`
- [ ] `jit issue update <id> --label "key:value"`
- [ ] `jit issue update <id> --remove-label "key:value"`

**Files changed:**
- `crates/jit/src/cli.rs`
- `crates/jit/src/commands.rs`

**Tests**: 10-15 CLI integration tests

#### 1.4 Query by Label
- [ ] `jit query label "pattern"`
- [ ] Support exact match
- [ ] Support wildcard: `milestone:*`

**Files changed:**
- `crates/jit/src/commands.rs`
- `crates/jit/src/storage.rs` (add label index)

**Tests**: 8-10 query tests

**Phase 1 Deliverable**: Basic labels work, can create/query labeled issues

---

### Phase 2: Namespace Registry (2-3 hours)

**Goal**: Define standard namespaces, validate usage

#### 2.1 Registry Storage
- [ ] Create `label-namespaces.json` schema
- [ ] Load/save registry
- [ ] Auto-create standard namespaces on `jit init`

**Files changed:**
- `crates/jit/src/storage.rs`
- `.jit/label-namespaces.json` (new file)

**Schema**:
```json
{
  "schema_version": 1,
  "namespaces": {
    "milestone": { "description": "...", "unique": false, "strategic": true },
    "epic": { "description": "...", "unique": false, "strategic": true },
    "component": { "description": "...", "unique": false, "strategic": false },
    "type": { "description": "...", "unique": true, "strategic": false },
    "team": { "description": "...", "unique": true, "strategic": false }
  }
}
```

**Tests**: 5-8 storage tests

#### 2.2 Namespace Validation
- [ ] Check uniqueness constraint
- [ ] Warn on unknown namespace
- [ ] Validate on issue update

**Files changed:**
- `crates/jit/src/labels.rs`

**Tests**: 8-10 validation tests

#### 2.3 CLI Commands - Namespaces
- [ ] `jit label namespaces list`
- [ ] `jit label namespace show <name>`
- [ ] `jit label namespace add <name> --description "..." --unique`
- [ ] `jit label values <namespace>` (list existing values)

**Files changed:**
- `crates/jit/src/cli.rs`
- `crates/jit/src/commands.rs`

**Tests**: 8-10 CLI tests

**Phase 2 Deliverable**: Registry defines standard namespaces, validated

---

### Phase 3: Breakdown & Strategic Queries (2-3 hours)

**Goal**: Labels flow through breakdown, strategic view queries

#### 3.1 Update Breakdown
- [ ] Copy labels from parent to subtasks
- [ ] Document behavior in help text
- [ ] Handle label inheritance rules (based on decision above)

**Files changed:**
- `crates/jit/src/commands.rs` (modify `breakdown_issue`)

**Tests**: 5-8 breakdown tests

#### 3.2 Strategic Query Helpers
- [ ] `jit query strategic` (wrapper: query milestone:*|epic:*)
- [ ] Helper to show milestone progress
- [ ] Helper to list all milestones/epics

**Files changed:**
- `crates/jit/src/cli.rs`
- `crates/jit/src/commands.rs`

Optional commands:
```bash
jit milestone-list  # Wrapper: query label "milestone:*"
jit milestone-status <id>  # Wrapper: graph downstream + count
jit epic-list
jit epic-status <id>
```

**Tests**: 6-8 query tests

**Phase 3 Deliverable**: Breakdown works, strategic queries available

---

### Phase 4: MCP Integration (1-2 hours)

**Goal**: AI agents can use labels via MCP

#### 4.1 MCP Tools
- [ ] `label_add` - Add labels to issue
- [ ] `label_remove` - Remove labels from issue
- [ ] `label_query` - Query issues by label
- [ ] `label_list_namespaces` - Discovery
- [ ] `label_values` - List values in namespace

**Files changed:**
- `mcp-server/src/tools.ts`
- `mcp-server/src/handlers.ts`

**Tests**: 8-10 MCP tests

#### 4.2 Schema Generation
- [ ] Auto-generate MCP schema from namespace registry
- [ ] Include examples in tool definitions
- [ ] Add validation hints

**Files changed:**
- `mcp-server/src/schema.ts`

**Tests**: 3-5 schema tests

#### 4.3 Documentation
- [ ] Update MCP README with label usage
- [ ] Add examples to tool descriptions
- [ ] Document label format requirements

**Files changed:**
- `mcp-server/README.md`

**Phase 4 Deliverable**: AI agents can use labels reliably

---

### Phase 5: Web UI (3-4 hours)

**Goal**: Visual strategic/tactical views, label badges

#### 5.1 API Endpoints
- [ ] `GET /api/issues?label=pattern` - Filter by label
- [ ] `GET /api/labels/namespaces` - List namespaces
- [ ] `GET /api/labels/values/:namespace` - List values
- [ ] `POST /api/issues/:id/labels` - Add label
- [ ] `DELETE /api/issues/:id/labels/:label` - Remove label

**Files changed:**
- `crates/jit-server/src/main.rs`

**Tests**: 8-10 API tests

#### 5.2 Frontend Components
- [ ] Label badge component (shows labels on nodes)
- [ ] Strategic view toggle button
- [ ] Filter issues by label
- [ ] Label selector dropdown (for adding)

**Files changed:**
- `web/src/components/LabelBadge.tsx` (new)
- `web/src/components/IssueGraph.tsx` (add filter)
- `web/src/components/IssueDetail.tsx` (show labels)
- `web/src/App.tsx` (add strategic toggle)

**Tests**: 8-10 component tests

#### 5.3 Strategic View Logic
- [ ] Filter graph nodes to milestone/epic labels only
- [ ] Show rollup stats in tooltips
- [ ] Expand/collapse epic nodes (optional)

**Files changed:**
- `web/src/hooks/useGraphData.ts`
- `web/src/utils/graphLayout.ts`

**Tests**: 5-8 integration tests

**Phase 5 Deliverable**: Web UI shows strategic/tactical views

---

### Phase 6: Validation & Polish (1-2 hours)

**Goal**: Audit tool, migration helpers, documentation

#### 6.1 Audit Tool
- [ ] `jit label audit` - Find malformed labels
- [ ] Suggest fixes for common mistakes
- [ ] Exit code for CI integration

**Files changed:**
- `crates/jit/src/commands.rs`

**Tests**: 5-8 audit tests

#### 6.2 Migration Helpers
- [ ] Migrate from `context["milestone"]` to labels
- [ ] Migrate from `context["epic"]` to labels
- [ ] Report conversion stats

**Files changed:**
- `crates/jit/src/commands.rs`

**Tests**: 3-5 migration tests

#### 6.3 Documentation
- [ ] Update README with label examples
- [ ] Update getting-started guide
- [ ] Create label best practices doc
- [ ] Update ROADMAP

**Files changed:**
- `README.md`
- `docs/getting-started-complete.md`
- `docs/label-best-practices.md` (new)
- `ROADMAP.md`

**Phase 6 Deliverable**: Production-ready, documented

---

## Testing Strategy

### Unit Tests (60+ tests)
- Label validation (15 tests)
- Namespace registry (10 tests)
- Query logic (12 tests)
- Breakdown label inheritance (8 tests)
- Storage (10 tests)
- Misc (5 tests)

### Integration Tests (40+ tests)
- CLI commands (20 tests)
- API endpoints (10 tests)
- MCP tools (10 tests)

### Component Tests (15+ tests)
- Web UI components (10 tests)
- Graph filtering (5 tests)

### E2E Tests (Optional, 5 tests)
- Create milestone → epic → tasks workflow
- Strategic view toggle
- Agent label usage via MCP

**Total Tests**: 115-125 new tests

---

## Success Criteria

### Phase 1 Complete
- [ ] Can add/remove labels via CLI
- [ ] Can query issues by label (exact + wildcard)
- [ ] Labels serialize correctly
- [ ] 25+ tests passing

### Phase 2 Complete
- [ ] Namespace registry exists
- [ ] Standard namespaces defined
- [ ] Uniqueness validation works
- [ ] 18+ tests passing

### Phase 3 Complete
- [ ] Breakdown copies labels
- [ ] Strategic query helpers work
- [ ] Can list milestones/epics
- [ ] 12+ tests passing

### Phase 4 Complete
- [ ] MCP tools available
- [ ] Agents can add/query labels
- [ ] Schema validation works
- [ ] 12+ tests passing

### Phase 5 Complete
- [ ] Web UI shows label badges
- [ ] Strategic view toggle works
- [ ] Can filter by label
- [ ] 18+ tests passing

### Phase 6 Complete
- [ ] Audit tool finds issues
- [ ] Migration scripts work
- [ ] Documentation complete
- [ ] 10+ tests passing

### Overall Complete
- [ ] All 115+ tests passing
- [ ] Zero clippy warnings
- [ ] Zero rustdoc warnings
- [ ] Documentation updated
- [ ] Example workflows in getting-started guide

---

## Risk Assessment

### Low Risk
✅ Label storage - Just Vec<String> field, simple serialization  
✅ Query logic - String matching, already have similar code  
✅ CLI commands - Standard clap patterns  

### Medium Risk
⚠️ Namespace registry - New file format, need schema versioning  
⚠️ Label validation - Must be user-friendly, not annoying  
⚠️ Web UI filtering - Graph performance with many nodes  

### High Risk
🔴 **Agent adoption** - Will agents understand the format?  
   Mitigation: Clear MCP tool descriptions, examples, validation  

🔴 **Label proliferation** - Users might create chaos  
   Mitigation: Namespace registry, validation, audit tool  

---

## Dependencies

### Required Before Starting
- [x] Rust 1.80+
- [x] Existing issue system working
- [x] Dependency graph working
- [x] CLI framework (clap) in place

### Optional (Can Add Later)
- [ ] Web UI (can ship CLI-only first)
- [ ] MCP tools (can ship without AI integration)

---

## Rollout Plan

### Version 0.2.0 (CLI + Backend)
- Phase 1: Core labels
- Phase 2: Namespace registry
- Phase 3: Strategic queries
- Ship: CLI-only release
- Document: Label conventions, examples

### Version 0.3.0 (AI Integration)
- Phase 4: MCP tools
- Ship: AI agents can use labels
- Document: MCP label usage guide

### Version 0.4.0 (Web UI)
- Phase 5: Web UI enhancements
- Ship: Strategic/tactical view toggle
- Document: Web UI usage

### Version 0.5.0 (Polish)
- Phase 6: Validation & migration
- Ship: Production-ready
- Document: Best practices

---

## Estimated Timeline

| Phase | Duration | Dependencies |
|-------|----------|--------------|
| Phase 1 | 3-4 hours | None |
| Phase 2 | 2-3 hours | Phase 1 |
| Phase 3 | 2-3 hours | Phase 1 |
| Phase 4 | 1-2 hours | Phase 1, 2 |
| Phase 5 | 3-4 hours | Phase 1, 2 |
| Phase 6 | 1-2 hours | All phases |
| **Total** | **12-18 hours** | Sequential |

**Minimum viable (CLI-only)**: Phases 1-3 = 7-10 hours  
**With AI support**: Phases 1-4 = 8-12 hours  
**Complete**: All phases = 12-18 hours

---

## Implementation Ready ✅

**All decisions confirmed:**
1. ✅ Validation: Strict (Option A)
2. ✅ Registry: Required (Option A)
3. ✅ Breakdown: Inherit all labels (Option A)
4. ✅ Target dates: Context field (Option A)
5. ✅ Query: Exact + wildcard (Option A)

**Status: Ready to implement. Awaiting confirmation to start.**

---

## Next Steps

1. **Make decisions** on 5 open questions above
2. **Create feature branch**: `git checkout -b feature/label-hierarchy`
3. **Start Phase 1**: Add labels to Issue struct
4. **Test incrementally**: Each phase independently testable
5. **Ship iteratively**: CLI → MCP → Web UI

---

## Appendix: File Changes Summary

### New Files
- `crates/jit/src/labels.rs` (validation, parsing)
- `.jit/label-namespaces.json` (registry)
- `docs/label-best-practices.md` (documentation)
- `web/src/components/LabelBadge.tsx` (UI component)

### Modified Files
- `crates/jit/src/domain.rs` (add labels field)
- `crates/jit/src/cli.rs` (new commands)
- `crates/jit/src/commands.rs` (label operations, breakdown)
- `crates/jit/src/storage.rs` (load/save registry)
- `crates/jit-server/src/main.rs` (API endpoints)
- `mcp-server/src/tools.ts` (MCP tools)
- `web/src/components/IssueGraph.tsx` (filtering)
- `web/src/App.tsx` (strategic toggle)

### Documentation Updates
- `README.md` (label examples)
- `docs/getting-started-complete.md` (label workflow)
- `docs/design.md` (label specification)
- `ROADMAP.md` (update status)

**Total File Changes**: ~15 files

---

## Ready to Implement?

All design decisions are documented. Once the 5 open questions are answered, implementation can proceed without ambiguity.

**The design is complete. We just need your decisions on the 5 questions above.**
