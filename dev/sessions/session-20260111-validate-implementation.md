# Session Notes: Divergence and Lease Validation Implementation

**Date:** 2026-01-11  
**Issue:** f8e58e7d - Add divergence and lease validation to jit validate  
**Status:** Implementation complete, code review identified issues requiring fixes  
**Assignee:** agent:copilot-cli

## Session Overview

Implemented `--divergence` and `--leases` flags for the `jit validate` command. Basic functionality works and manual testing confirms the features operate correctly, but code review revealed critical gaps that must be addressed before completion.

## What Was Accomplished

### 1. Divergence Validation ✅

**Implemented:** `jit validate --divergence`

Validates that the current branch shares common history with `origin/main` by comparing:
- `git merge-base HEAD origin/main` (common ancestor)
- `git rev-parse origin/main` (current main commit)

If these differ, the branch has diverged.

**Example output (success):**
```bash
$ jit validate --divergence
✓ Branch is up-to-date with origin/main
```

**Example output (failure):**
```bash
$ jit validate --divergence
❌ Divergence validation failed:
Branch has diverged from origin/main
Merge base: abc123
Main commit: def456
Fix: git rebase origin/main
```

**JSON output:**
```json
{
  "success": true,
  "data": {
    "valid": true,
    "validations": [
      {
        "validation": "divergence",
        "valid": true,
        "message": ""
      }
    ]
  }
}
```

### 2. Lease Validation ⚠️ (Partially Complete)

**Implemented:** `jit validate --leases`

Validates active leases in `.git/jit/claims.index.json`:
- ✅ Checks if leases have expired (TTL exceeded)
- ✅ Checks if referenced issues still exist
- ❌ **MISSING:** Checks if referenced worktrees still exist (documented but not implemented)

**Manual testing found real issue:**
```bash
$ jit validate --leases
❌ Lease validation failed:
Found 1 invalid lease(s):
Lease fe9695a2-975f-4d4e-9f51-8aa477024dfc (Issue 8a19eb0c): Expired 2 days ago
  Fix: jit claim release 8a19eb0c
```

### 3. CLI Integration ✅

Added flags to `Commands::Validate`:
```rust
Validate {
    json: bool,
    fix: bool,
    dry_run: bool,
    divergence: bool,  // NEW
    leases: bool,      // NEW
}
```

Supports:
- Individual validation: `--divergence` or `--leases`
- Combined: `--divergence --leases`
- JSON output: `--divergence --json`
- Human-readable output with actionable fix suggestions

### 4. Helper Functions ✅

**`format_duration(duration: chrono::Duration) -> String`**
- Formats durations in human-readable form
- Examples: "30 seconds", "5 minutes", "2 hours", "3 days"

**`validate_divergence(&self) -> Result<()>`**
- Public method on CommandExecutor
- Returns Ok if up-to-date, Err with helpful message if diverged

**`validate_leases(&self) -> Result<Vec<String>>`**
- Public method on CommandExecutor  
- Returns vector of invalid lease descriptions with fix suggestions

## Critical Issues Identified (Code Review)

### Issue 1: Missing Worktree Validation ⚠️ HIGH PRIORITY

**Documentation says:**
```rust
/// Checks claims.index.json for:
/// - Expired leases (TTL exceeded)
/// - Leases referencing non-existent worktrees  // ← DOCUMENTED
/// - Leases referencing non-existent issues
```

**But implementation only checks:**
1. ✅ Expired leases
2. ❌ Missing worktrees ← **NOT IMPLEMENTED**
3. ✅ Missing issues

**Why this matters:**
When a worktree is deleted (e.g., `git worktree remove`), the lease becomes orphaned. Current implementation doesn't detect this.

**Example scenario:**
1. Agent claims issue in worktree `wt:feature-x`
2. Worktree is deleted: `git worktree remove feature-x`
3. Lease still references `wt:feature-x` in claims.index.json
4. `jit validate --leases` should flag this but currently doesn't

**Fix required:**
```rust
// After checking expiry and issue existence
let worktree_exists = check_worktree_exists(&lease.worktree_id)?;
if !worktree_exists {
    invalid_leases.push(format!(
        "Lease {} (Issue {}): Worktree {} no longer exists\n  Fix: jit claim force-evict {}",
        lease.lease_id,
        &lease.issue_id[..8.min(lease.issue_id.len())],
        lease.worktree_id,
        &lease.issue_id[..8.min(lease.issue_id.len())]
    ));
}
```

**Helper function needed:**
```rust
fn check_worktree_exists(worktree_id: &str) -> Result<bool> {
    // Parse git worktree list --porcelain
    // Load each worktree's .jit/worktree.json
    // Check if any match the worktree_id
    // Return true if found, false otherwise
}
```

### Issue 2: Incorrect Issue Path Resolution 🐛 HIGH PRIORITY

**Current code (line 819-821):**
```rust
let issue_path = paths
    .local_jit
    .join(format!("issues/{}.json", lease.issue_id));
if !issue_path.exists() {
    invalid_leases.push(...); // Flag as missing
}
```

**Problem:** Uses `paths.local_jit` which is the **current worktree's** `.jit/` directory.

**Why this is wrong:**
- Issues may exist in git commits (canonical source)
- Issues may be in main worktree `.jit/`
- Issues may be in other worktrees (for claimed issues)
- Checking only local worktree gives **false positives**

**Example failure scenario:**
1. Issue abc123 is committed to git
2. Current worktree doesn't have abc123 in `.jit/issues/`
3. Validation incorrectly reports: "Issue no longer exists"
4. Issue actually exists in git history

**Correct approach:**
Use the storage layer which has proper resolution:
```rust
// Check if issue exists via storage (checks git + filesystem)
match self.storage.load_issue(&lease.issue_id) {
    Ok(_) => {
        // Issue exists - no problem
    }
    Err(_) => {
        invalid_leases.push(format!(
            "Lease {} (Issue {}): Issue no longer exists\n  Fix: jit claim release {}",
            lease.lease_id,
            &lease.issue_id[..8.min(lease.issue_id.len())],
            &lease.issue_id[..8.min(lease.issue_id.len())]
        ));
    }
}
```

**Impact:** Currently deployed code will give false positives when validating leases in worktrees that don't have local copies of all issues.

### Issue 3: Tests Are Placeholders ❌ HIGH PRIORITY

**Current test implementation:**
```rust
#[test]
fn test_validate_leases_no_index() {
    // This test would require proper git setup with WorktreePaths
    // For now, just test that the function signature is correct
    // Real testing would need integration test with actual git repo
}

#[test]
fn test_validate_leases_all_valid() {
    // Would require proper git setup - skip for unit test
    // Integration tests will cover this
}

#[test]
fn test_validate_leases_expired() {
    // Would require proper git setup - skip for unit test
}

#[test]
fn test_validate_leases_missing_issue() {
    // Would require proper git setup - skip for unit test
}
```

**All 4 lease validation tests are empty placeholders!**

**Problem:** Violates TDD principle:
1. Tests should drive implementation
2. Tests should verify correctness
3. Code without tests is unverified

**What's needed:**

**Minimum viable tests:**
1. Test `format_duration()` with various inputs
2. Test divergence validation with mocked git output (or skip if too complex)
3. Test lease validation logic with in-memory data structures

**Better approach:**
Create integration tests that:
- Set up temporary git repository
- Create claims.index.json with known leases
- Verify validation detects specific issues
- Check error messages contain expected content

### Issue 4: No Divergence Tests ❌ MEDIUM PRIORITY

No tests exist for `validate_divergence()` function.

**Why hard to test:**
- Requires actual git repository
- Requires `origin/main` remote
- Hard to mock in unit tests

**Options:**
1. **Skip unit tests** - Accept that this is integration-tested manually
2. **Mock Command::new()** - Complex, not worth it for this case
3. **Integration test only** - Acceptable for git-dependent code

**Recommendation:** Document that divergence validation is integration-tested only (manual + real usage). Focus test effort on lease validation.

### Issue 5: Imperative Loop Style ⚠️ LOW PRIORITY

**Current code:**
```rust
let mut invalid_leases = Vec::new();
let now = Utc::now();

for lease in &index.leases {
    // Check if lease has expired
    if let Some(expires_at) = lease.expires_at {
        if expires_at < now {
            invalid_leases.push(...);
            continue;
        }
    }
    // Check if issue still exists
    if !issue_path.exists() {
        invalid_leases.push(...);
    }
}
```

**More functional approach:**
```rust
let invalid_leases = index
    .leases
    .iter()
    .filter_map(|lease| validate_single_lease(lease, &self.storage, &now))
    .collect::<Vec<_>>();

fn validate_single_lease(
    lease: &Lease,
    storage: &impl IssueStore,
    now: &DateTime<Utc>,
) -> Option<String> {
    // Check expiry
    if let Some(expires_at) = lease.expires_at {
        if expires_at < *now {
            return Some(format!(...));
        }
    }
    
    // Check worktree exists
    if !worktree_exists(&lease.worktree_id) {
        return Some(format!(...));
    }
    
    // Check issue exists
    if storage.load_issue(&lease.issue_id).is_err() {
        return Some(format!(...));
    }
    
    None // Valid lease
}
```

**Benefits:**
- More idiomatic Rust (functional style)
- Easier to test (test `validate_single_lease` in isolation)
- Clear separation of concerns

**Impact:** Low - current code works, but refactoring improves quality.

## Files Modified

1. **`crates/jit/src/cli.rs`** - Added `divergence` and `leases` flags
2. **`crates/jit/src/commands/validate.rs`** - Implemented validation functions
3. **`crates/jit/src/main.rs`** - CLI handler for new flags

## Current Status

### ✅ Working
- Divergence validation executes correctly
- Lease validation detects expired leases
- JSON output format correct
- Human-readable output with fix suggestions
- Clippy passes, fmt passes

### ❌ Broken/Missing
- Worktree existence validation (documented but not implemented)
- Issue path resolution uses wrong approach (false positives likely)
- Tests are placeholders (no actual verification)

### ⚠️ Technical Debt
- Imperative loop instead of functional style
- No integration tests
- No divergence tests

## Next Session TODO

### Critical Fixes (Must Complete Before Done)

1. **Add worktree validation** (~30 min)
   - Implement `check_worktree_exists(worktree_id: &str) -> Result<bool>`
   - Parse git worktree list output
   - Load worktree identities from `.jit/worktree.json`
   - Match against lease.worktree_id
   - Add to validation loop

2. **Fix issue path resolution** (~15 min)
   - Replace `paths.local_jit.join(...)` with `self.storage.load_issue(...)`
   - Use storage layer's proper resolution (git + filesystem)
   - Handle error case correctly

3. **Add real tests** (~45 min)
   - Test `format_duration()` with various inputs
   - Test lease expiry detection (mock data)
   - Test missing issue detection (mock data)
   - Test missing worktree detection (mock data)
   - Consider property-based tests for edge cases

### Optional Improvements

4. **Refactor to functional style** (~30 min)
   - Extract `validate_single_lease()` function
   - Use `filter_map()` instead of imperative loop
   - Improve testability

5. **Add integration test** (~60 min)
   - Set up git repo in temp dir
   - Create claims.index.json
   - Run validation
   - Verify output

## Lessons Learned

1. **Code review before commit** - Would have caught missing worktree validation
2. **Don't stub tests** - Placeholder tests give false confidence
3. **Test critical paths** - Issue resolution logic should have been verified
4. **Check against spec** - Documentation said "3 checks", implementation did 2

## Design Decisions Made

1. **Return Vec<String> for invalid leases** - Simple, human-readable, easy to test
2. **Use git commands directly** - Simpler than parsing git internals
3. **Separate --divergence and --leases** - Allow independent validation
4. **Include fix suggestions** - More helpful than just error messages

## Manual Testing Performed

```bash
# Divergence validation (success)
$ jit validate --divergence
✓ Branch is up-to-date with origin/main

# Lease validation (found real issue!)
$ jit validate --leases
❌ Lease validation failed:
Found 1 invalid lease(s):
Lease fe9695a2-975f-4d4e-9f51-8aa477024dfc (Issue 8a19eb0c): Expired 2 days ago
  Fix: jit claim release 8a19eb0c

# JSON output
$ jit validate --divergence --json | jq .
{
  "success": true,
  "data": {
    "valid": true,
    "validations": [
      {
        "validation": "divergence",
        "valid": true,
        "message": ""
      }
    ]
  }
}

# Help output
$ jit validate --help
Validate repository integrity

Usage: jit validate [OPTIONS]

Options:
      --json        
  -q, --quiet       Suppress non-essential output (for scripting)
      --fix         Attempt to automatically fix validation issues
      --dry-run     Show what would be fixed without applying changes (requires --fix)
      --divergence  Validate branch hasn't diverged from main
      --leases      Validate active leases are consistent and not stale
  -h, --help        Print help
```

## Summary

Implementation provides **functional but incomplete** validation. The divergence validation works correctly. Lease validation partially works but has critical gaps:

1. **Missing feature:** Worktree validation
2. **Bug:** Wrong issue resolution approach  
3. **Quality:** No real tests

These issues must be fixed before marking task complete. All three are straightforward to address and should take ~90 minutes total.

**Recommendation:** Do not pass gates or mark done until critical fixes are applied.
