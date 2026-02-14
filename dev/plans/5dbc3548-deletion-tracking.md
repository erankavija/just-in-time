# Implementation Plan: Bug 5dbc3548 - Deletion Tracking

## Overview

Fix issue deletion bug where deleted issues reappear in queries by adding explicit deletion tracking to the index, while preserving multi-worktree functionality.

## Problem Statement

**Current bug:** When you delete an issue in main worktree, it reappears in `jit query all` until the deletion is committed to git.

**Root cause:** 
- `load_aggregated_index()` merges IDs from local + git + main worktree
- Deleted issue is removed from local but still in git index
- `load_issue()` falls back to git HEAD and successfully loads it

**Critical constraint:** Must preserve secondary worktree ability to see committed issues (documented workflow).

## Solution: Deletion Tracking in Index

Add `deleted_ids: Vec<String>` field to `index.json` to explicitly track locally deleted issues.

**Schema v2:**
```json
{
  "schema_version": 2,
  "all_ids": ["issue-1", "issue-2"],
  "deleted_ids": ["issue-3"]
}
```

**Breaking change:** Existing repositories will need to reinitialize index (run `jit validate --fix` or delete `.jit/index.json` to regenerate).

## Implementation Phases (TDD)

### Phase 1: Schema Update (SIMPLIFIED) ✅ COMPLETE
**Goal:** Add deleted_ids field, bump schema version

**Completed:**
- ✅ Updated `Index` struct to include `deleted_ids: Vec<String>`
- ✅ Bumped `CURRENT_SCHEMA_VERSION` to 2
- ✅ Updated `init()` to create v2 index
- ✅ Tests: New repos create v2 index, save/load works
- ✅ Commit: 62ddad2

### Phase 2: Deletion Tracking Logic ✅ COMPLETE  
**Goal:** Track deletions in local index

**Completed:**
- ✅ Updated `delete_issue()` to add ID to `deleted_ids[]`
- ✅ Updated `load_aggregated_index()` to filter deleted IDs
- ✅ Tests: Deletion tracking works, aggregated index filters correctly
- ✅ Integration test: test_bug_5dbc3548_deleted_issue_not_in_query
- ✅ Bug is FIXED - deleted issues don't reappear
- ✅ All tests pass (622 lib + integration + worktree)
- ✅ Commit: 62ddad2

### Phase 3: Secondary Worktree Deletion Safety 🔄 IN PROGRESS
**Goal:** Prevent unsafe deletions across worktrees

**Deletion rules:**
1. **Secondary worktree:** Deletion always blocked (error message)
2. **Main worktree with active leases in secondary:** Require `JIT_ALLOW_DELETION=1`
3. **Main worktree without secondary leases:** Deletion allowed normally

**Tasks:**
- [ ] Detect if in secondary worktree (check git worktree info)
- [ ] Block deletion in secondary with clear error
- [ ] In main worktree: check for active leases in other worktrees
- [ ] If leases exist: require `JIT_ALLOW_DELETION=1` env var
- [ ] Enhanced error messages with context
- [ ] Tests:
  - [ ] Secondary: deletion blocked with helpful error
  - [ ] Main with secondary lease: requires env var
  - [ ] Main with env var set: deletion succeeds
  - [ ] Main without secondary leases: deletion works normally

**Files to modify:**
- `crates/jit/src/commands/issue.rs` (`delete_issue()` pre-checks)
- `crates/jit/src/storage/json.rs` (worktree detection helpers)
- `crates/jit/tests/cross_worktree_integration_tests.rs`

**Error messages:**
```
Secondary worktree: 
"Issue deletion not allowed in secondary worktree.
Delete from main worktree instead."

Main with active leases:
"Cannot delete issue: active lease exists in secondary worktree.
Set JIT_ALLOW_DELETION=1 to override (use with caution)."
```

### Phase 4: Comprehensive Testing
**Goal:** Verify fix works in all scenarios

**Tests to add:**
- [ ] **Single worktree:**
  - [ ] Create → commit → delete → query (should not appear)
  - [ ] Create → delete → query (should not appear, no git)
  - [ ] Delete → commit deletion → query (should not appear)
  
- [ ] **Multi-worktree:**
  - [ ] Main: create → commit → Secondary: can see it ✅
  - [ ] Main: delete → Secondary: still sees it (not committed)
  - [ ] Main: delete → commit → Secondary: doesn't see it ✅
  - [ ] Secondary: create local → can see locally → not in main
  - [ ] **Secondary: try delete → blocked with error** ✅
  - [ ] **Main: delete with secondary lease → blocked without env var** ✅
  - [ ] **Main: delete with env var → succeeds** ✅
  
- [ ] **Edge cases:**
  - [ ] Delete already deleted issue (idempotent)
  - [ ] Delete then recreate same ID (should work)
  - [ ] Validate doesn't see deleted issues
  - [ ] Graph operations ignore deleted issues
  - [ ] Lease check works across worktrees

**Files to modify:**
- `crates/jit/tests/integration_test.rs`
- `crates/jit/tests/cross_worktree_integration_tests.rs`

### Phase 5: Secondary Worktree Deletion Safety (Optional Enhancement)
**Goal:** Additional safety for edge cases

**This phase was moved to Phase 3 as core functionality**

## Testing Strategy

**TDD Workflow:**
1. Write failing test
2. Run test (verify it fails)
3. Implement minimal code
4. Run test (verify it passes)
5. Run full suite (verify no regressions)
6. Refactor if needed

**Test Coverage:**
- Unit tests for schema migration
- Integration tests for deletion behavior
- Cross-worktree integration tests
- Edge case handling

## Acceptance Criteria

- [ ] Bug 5dbc3548 is fixed: deleted issues don't reappear in queries
- [ ] Secondary worktrees can still see committed issues (workflow preserved)
- [ ] All existing tests pass
- [ ] New tests cover deletion scenarios
- [ ] Schema v2 index format works
- [ ] Clear error/regeneration if v1 index encountered (breaking change is acceptable)
- [ ] Code passes clippy with zero warnings
- [ ] Code is formatted with cargo fmt

## Files That Will Be Modified

```
crates/jit/src/storage/json.rs          # Core implementation
crates/jit/tests/integration_test.rs     # Deletion tests
crates/jit/tests/cross_worktree_integration_tests.rs  # Worktree tests
```

## Breaking Changes

**Index schema v1 → v2:** Repositories on v1 will need index regeneration. Options:
1. Automatic: Detect v1, regenerate from `.jit/issues/`
2. Manual: Error message instructs user to run `jit validate --fix`
3. Simple: Delete `.jit/index.json`, it will regenerate on next command

**Recommendation:** Option 1 (automatic regeneration) for best UX.

## Rollout Plan

1. Implement Phase 1 (schema) - breaking change, auto-regenerate v1 indexes
2. Implement Phase 2 (deletion tracking) - fixes the core bug
3. Implement Phase 3 (deletion safety) - prevents unsafe deletions across worktrees
4. Implement Phase 4 (comprehensive tests) - full confidence
5. Commit with reference: `jit:5dbc3548`
6. Update issue state to Done

## Notes

- Deletion tracking is per-worktree until committed
- When you commit, other worktrees will see deletions via git
- This matches the mental model: deletions are like any other change
- No breaking changes to existing functionality
