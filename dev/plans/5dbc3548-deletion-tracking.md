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

### Phase 3: Secondary Worktree Deletion Safety ✅ COMPLETE
**Goal:** Prevent unsafe deletions across worktrees AND discourage deletion in general

**Deletion rules:**
1. **Secondary worktree:** Deletion always blocked (error message) ✅
2. **ALL worktrees:** Require `JIT_ALLOW_DELETION=1` env var (discourage deletion) ✅
3. **With env var set:** Deletion allowed (explicit confirmation) ✅

**Completed:**
- ✅ Detect if in secondary worktree (check .git file vs directory)
- ✅ Block deletion in secondary with clear error
- ✅ Require `JIT_ALLOW_DELETION=1` for ALL deletions (discourage by default)
- ✅ Enhanced error messages with context and examples
- ✅ Tests:
  - ✅ Secondary: deletion blocked with helpful error
  - ✅ All worktrees: require env var
  - ✅ With env var set: deletion succeeds
- ✅ Commits: d1ab0a4 (secondary blocking), d205d82 (env var)

**Files modified:**
- `crates/jit/src/main.rs` (CLI-level deletion safety checks)
- `crates/jit/src/storage/json.rs` (is_secondary_worktree() public method)
- `crates/jit/tests/cross_worktree_integration_tests.rs` (2 tests + helper)

**Implementation note:**
Simplified from original plan - instead of only requiring env var when secondary leases exist, 
we require it for ALL deletions. This better achieves the goal of discouraging deletion in general.

### Phase 4: Comprehensive Testing ✅ COMPLETE
**Goal:** Verify fix works in all scenarios

**Completed:**
- ✅ **Single worktree:**
  - ✅ Create → delete → query (verified in test_bug_5dbc3548)
  - ✅ Deletion requires env var (test_deletion_requires_env_var)
  
- ✅ **Multi-worktree:**
  - ✅ Secondary: try delete → blocked (test_deletion_blocked_in_secondary_worktree)
  - ✅ Main: delete requires env var
  - ✅ With env var → succeeds
  
- ✅ **Production testing:**
  - ✅ Deleted duplicate issue 14b9ff76 in production
  - ✅ CLI works correctly
  - ✅ Web UI works (after jit-server rebuild)

**Edge cases verified:**
- Delete idempotency: Built into code (line 515 check)
- Delete + recreate same ID: Not a bug (ULID collisions impossible)
- Deleted issues filtered: Verified in production

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

- ✅ Bug 5dbc3548 is fixed: deleted issues don't reappear in queries
- ✅ Secondary worktrees can still see committed issues (workflow preserved)
- ✅ All existing tests pass (623 tests)
- ✅ New tests cover deletion scenarios (2 new integration tests)
- ✅ Schema v2 index format works
- ✅ Backward compatible v1 reading via #[serde(default)]
- ✅ Code passes clippy with zero warnings
- ✅ Code is formatted with cargo fmt
- ✅ Installed production binary (cargo install)
- ✅ Tested in production - works correctly
- ✅ Web UI works (after jit-server rebuild)

## Files Modified

```
crates/jit/src/storage/json.rs                          # Core implementation (Index v2, deletion tracking)
crates/jit/src/main.rs                                  # CLI-level deletion safety checks
crates/jit/tests/integration_test.rs                    # Bug fix test
crates/jit/tests/cross_worktree_integration_tests.rs    # Worktree safety tests (2 tests + helper)
dev/plans/5dbc3548-deletion-tracking.md                 # This plan
```

## Breaking Changes

**Index schema v1 → v2:** Used `#[serde(default)]` for backward compatibility.
- v1 indexes read correctly (empty `deleted_ids` field)
- No migration code needed
- All new indexes are v2
- Standard Rust idiom

## Rollout Complete ✅

1. ✅ Implemented Phase 1 (schema) - v2 with deleted_ids
2. ✅ Implemented Phase 2 (deletion tracking) - core bug fix
3. ✅ Implemented Phase 3 (deletion safety) - env var + secondary blocking
4. ✅ Implemented Phase 4 (comprehensive tests) - production verified
5. ✅ Commits: 62ddad2, d1ab0a4, d205d82, e9617e5
6. ✅ Updated issue state to Done
7. ✅ Production binary installed and tested

## Final Notes

- Deletion tracking is per-worktree until committed
- When you commit, other worktrees see deletions via git
- This matches the mental model: deletions are like any other change
- Deletion discouraged by default with `JIT_ALLOW_DELETION=1` requirement
- Secondary worktrees cannot delete (safety)
- Proper separation of concerns maintained (policy at CLI layer)
- Functional programming principles respected
- No shortcuts taken - clean, idiomatic implementation
