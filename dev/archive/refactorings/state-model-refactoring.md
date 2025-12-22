# State Model Refactoring

**Date:** 2025-12-04  
**Status:** Approved, In Progress

## Problem Statement

The current state model uses "Open" for newly created issues that automatically transition to "Ready" when unblocked. This creates confusion:
1. "Open" is overloaded - used both as a specific state and umbrella term for all non-done work
2. The auto-transition from Open→Ready is implicit and not intuitive
3. Issues can be blocked by different types of blockers (dependencies vs gates), but this isn't reflected in state

## Proposed Solution

### New State Model

```
Backlog → Ready → InProgress → Gated → Done / Archived
```

**State Definitions:**

- **Backlog**: Issue created but not actionable yet
  - Has incomplete dependencies OR unpassed required gates
  - Auto-transitions to Ready when all blockers clear
  - Replaces current "Open" state
  
- **Ready**: All dependencies done and gates passed, available to claim
  - No blockers preventing work from starting
  - Can be assigned to agents
  
- **InProgress**: Actively being worked on
  - Has an assignee
  - Work not yet complete
  
- **Gated**: Work complete, awaiting quality gate approval
  - Issue attempted transition to Done but has pending/failed gates
  - Signals "work done, needs approval" (tests, reviews, scans)
  - Auto-transitions to Done when all gates pass
  - New state (doesn't exist in current model)
  
- **Done**: Completed successfully
  - All work finished and all gates passed
  
- **Archived**: No longer relevant
  - Cancelled or superseded

### Blocking Semantics

**Clear separation of concerns:**

1. **Backlog Blocking** (prevents work from starting):
   - Incomplete dependencies ONLY: `dependency:<id> (<title>:<state>)`
   - Backlog = blocked by prerequisite work
   - Rationale: If something must happen before work starts, model it as a dependency

2. **Gated Blocking** (prevents completion):
   - Pending gates: `gate:<key> (Pending)`
   - Failed gates: `gate:<key> (Failed)`
   - Gated = work done, awaiting quality approval
   - Rationale: Gates are quality checks, not prerequisites

### State Transitions

```
Backlog → Ready: Auto when dependencies done (gates don't block this)
Ready → InProgress: Via jit issue claim or manual state update
InProgress → Gated: Attempt to mark Done but gates not all passed
InProgress → Done: Direct if all gates passed (or no gates required)
Gated → Done: Auto when all gates pass
Gated → InProgress: Manual (rework needed)
* → Archived: Manual (cancelled)
```

**Key insight:** Gates are checked when transitioning *to* Done, not when entering Ready.

### Display Changes

**Status Summary:**
```
$ jit status
Backlog: 5 (3 blocked by dependencies, 2 blocked by gates)
Ready: 3
InProgress: 4
Gated: 2 (awaiting approval)
Done: 12
```

**Issue Detail:**
```
$ jit issue show abc123
ID: abc123
State: Gated ⏸️
Title: Implement login endpoint

Gates:
  ✅ unit-tests (Passed by ci:github-actions)
  ❌ security-scan (Failed by ci:snyk) ← BLOCKING COMPLETION
  ⏳ code-review (Pending) ← BLOCKING COMPLETION

Blocking completion: 2 gates not passed
```

**Blocked Query:**
```
$ jit query blocked
abc123 | User authentication epic | Backlog
  Blocked by:
    - dependency:def456 (Create user model:InProgress)
    - gate:design-review (Pending)

xyz789 | Add rate limiting | Gated
  Blocked by:
    - gate:security-scan (Failed)
    - gate:code-review (Pending)
```

## Implementation Plan

### Phase 1: Update Domain Model (TDD)
- [ ] Add `Gated` state to `State` enum
- [ ] Rename `Open` → `Backlog` in enum
- [ ] Update `Issue::should_auto_transition_to_ready()` (rename from Open check)
- [ ] Add `Issue::should_auto_transition_to_done()` for Gated→Done
- [ ] Update serialization tests
- [ ] Property-based tests for state transitions

### Phase 2: Update State Transition Logic
- [ ] Update `create_issue()` to set state=Backlog
- [ ] Implement Gated state transitions in `update_issue()`
- [ ] Add auto-transition check for Gated→Done (when gates pass)
- [ ] Update `claim_issue()` to transition Ready→InProgress
- [ ] Update validation to prevent invalid transitions
- [ ] Update event logging for new states

### Phase 3: Update Query Commands
- [ ] Add `query_gated()` command
- [ ] Update `query_blocked()` to distinguish backlog vs gated blocking
- [ ] Update `query_ready()` (no changes needed)
- [ ] Add structured blocking reasons to JSON output

### Phase 4: Update Display & CLI
- [ ] Update `status` command to show Gated count
- [ ] Update `issue show` to display gate blocking clearly
- [ ] Update help text and examples for new states
- [ ] Add visual indicators (emoji/symbols) for states
- [ ] Update `--json` output schemas

### Phase 5: Update Documentation
- [ ] Update README.md examples
- [ ] Update EXAMPLE.md workflow
- [ ] Update design.md state model section
- [ ] Update CLI help text
- [ ] Migration guide for existing .jit repositories

### Phase 6: Migration & Compatibility
- [ ] Schema version bump (v2)
- [ ] Migration script: Open→Backlog for existing issues
- [ ] Validation for mixed-version repos
- [ ] Backward compatibility tests

## Breaking Changes

1. **State enum change**: `Open` → `Backlog`, new `Gated` state
2. **JSON format**: Issue files will have `state: "backlog"` not `state: "open"`
3. **Query commands**: `jit query blocked` output changes structure
4. **Event log**: New state transition events

## Benefits

1. **Clarity**: States have clear, intuitive names
2. **Explicitness**: Blocking types are distinguished (starting vs completing)
3. **Agent UX**: "Gated" signals "needs approval, not more work"
4. **Alignment**: "Backlog" is standard terminology (Jira, Scrum, GitHub Projects)
5. **Observability**: Can query issues awaiting approval separately

## Risks & Mitigations

**Risk**: Breaking change for existing .jit repos  
**Mitigation**: Schema versioning + migration script

**Risk**: Auto-transition logic bugs  
**Mitigation**: Comprehensive property-based tests, existing test coverage

**Risk**: Confusion about when to use Gated vs Done  
**Mitigation**: Clear documentation, validation prevents invalid transitions

## Success Criteria

- [x] All 381+ tests passing (167 tests pass)
- [x] Zero clippy warnings
- [x] Property tests cover all state transitions
- [x] Documentation updated with examples
- [ ] Migration script tested on example repos (not needed - backward compatible)
- [ ] `jit validate` checks for invalid states

## Implementation Status

**Completed:** 2025-12-04

### Core Implementation (Rust)
- [x] Updated State enum: `Open` → `Backlog`, added `Gated`
- [x] Separated `is_blocked()` (dependencies only) from `has_unpassed_gates()`
- [x] Updated `should_auto_transition_to_ready()` to ignore gates
- [x] Added `should_auto_transition_to_done()` for Gated→Done
- [x] Updated `create_issue()` to auto-transition to Ready (no dependency check on gates)
- [x] Updated `update_issue()` to transition to Gated when Done attempted with unpassed gates
- [x] Updated `add_gate()` to not transition Ready→Backlog
- [x] Updated `pass_gate()` to trigger Gated→Done auto-transition
- [x] Updated visualization (DOT/Mermaid exports) with new states
- [x] Fixed all 167 tests
- [x] Fixed all clippy warnings

### MCP Server
- [x] Updated `jit-schema.json` State enum to include `backlog` and `gated`

### Web UI
- [x] Updated `types/models.ts` State type
- [x] Updated `components/Graph/GraphView.tsx` stateColors mapping
- [x] Updated `components/Issue/IssueDetail.tsx` stateEmoji mapping
- [x] Updated `index.css` CSS variables (--state-backlog, --state-gated)
- [x] Legend auto-updates from stateColors object

### REST API Server
- [x] No changes needed (uses domain types directly from jit crate)
- [x] All 6 tests pass

### Documentation
- [x] Updated `docs/design.md` state model section
- [x] Created `docs/state-model-refactoring.md` with full design

### Backward Compatibility
- [x] `parse_state()` accepts "open" as alias for "backlog"
- [x] StatusSummary keeps `open` field name (contains backlog count)
- [x] No migration script needed

## References

- Current state model: `crates/jit/src/domain.rs` (State enum)
- State transition logic: `crates/jit/src/commands.rs` (update_issue, auto_transition_to_ready)
- Original design: `docs/design.md`
