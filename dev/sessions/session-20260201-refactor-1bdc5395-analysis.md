# Refactor Analysis - Issue 1bdc5395

## Status Summary

### 1. ✅ Test quality: Conditional assertions
**COMPLETED** - Fixed in commit 6f6683e
- Removed `test_detect_in_main_worktree` that had conditional logic
- Refactored agent_config tests to use mock environment readers
- No more race conditions from global state modification

### 2. ⚠️ Unused storage parameter pattern
**PARTIALLY VALID** - Found 3 instances in claim.rs
```rust
// Lines 126, 171, 256 in commands/claim.rs
fn execute_claim_list<S: IssueStore>(_storage: &S, ...) 
fn execute_claim_renew<S: IssueStore>(_storage: &S, ...)
fn execute_claim_force_evict<S: IssueStore>(_storage: &S, ...)
```
**Action**: Remove unused parameter or document why it's needed for trait consistency

### 3. ❌ Git detection silent fallbacks
**CONFIRMED** - Critical issue
```rust
// commands/claim.rs:13-24 and commands/worktree.rs:43-56
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")...
    if !output.status.success() {
        return Ok("main".to_string()); // SILENT FALLBACK!
    }
}
```
**Impact**: Hides errors, returns "main" even when not in git repo
**Action**: Return error instead of silent fallback

### 4. ✅ Path handling inconsistency  
**COMPLETED** - Fixed in WorktreePaths::detect()
- Now canonicalizes relative paths from git (commit 98c1672)
- Consistent pattern established

### 5. ⚠️ Test helper duplication
**NEEDS INVESTIGATION**
- Need to check for duplicated `setup_test_repo()` functions
- Extract to shared test_utils if found

### 6. ❌ Missing integration tests
**VALID CONCERN**
- Have cross_worktree_integration_tests.rs (6 tests)
- But no CLI binary integration tests (exit codes, output format)
- **Action**: Add tests that run `jit` binary with Command

### 7. ❌ Error messages not actionable
**NEEDS INVESTIGATION**
- Need to audit error messages for actionable hints
- Example: "Lease not found" → "Lease not found. List active leases with: jit claim list"

## Recommended Approach

**High Priority (Fix Now):**
1. Fix silent fallback in get_current_branch() - CRITICAL
2. Remove unused _storage parameters - SIMPLE

**Medium Priority (This Issue):**
3. Audit and improve error messages - MEDIUM EFFORT
4. Check for test helper duplication - SIMPLE

**Low Priority (Defer):**
5. Add CLI integration tests - SEPARATE ISSUE (larger scope)

## Estimated Effort
- Items 1-2: 30 minutes
- Items 3-4: 1 hour
- Item 5: 2-3 hours (recommend separate issue)

