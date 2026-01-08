# CLI Command Quality Improvements

**Issue:** 4b2cb4cd-0351-48d1-bb90-7c0d14909623  
**Status:** Backlog  
**Author:** Copilot  
**Date:** 2026-01-08

## Context

During implementation of claim and worktree CLI commands (issues ee20327b, 77206cf6, d91b72ab, 3f2cf966, 6e020dc9, 2ba4d4c7, 8be14c59), seven code quality issues were identified. While the implementations are functionally correct and pass all tests, these patterns represent technical debt and potential future bugs.

## Improvements

### 1. Test Quality: Conditional Assertions

**Problem:**  
Tests that use conditional assertions can silently pass when the bug they're supposed to catch exists.

**Example from `worktree_paths.rs`:**
```rust
#[test]
fn test_detect_in_main_worktree() {
    if let Ok(paths) = WorktreePaths::detect() {
        // BUG: This condition is backwards!
        if !paths.is_worktree() {
            assert_eq!(paths.common_dir, paths.worktree_root.join(".git"));
        }
    }
}
```

When `is_worktree()` had a bug (always returning true), the condition `!paths.is_worktree()` was false, so the assertion never ran. Test passed, bug remained.

**Root Cause:**  
The bug itself prevented the test from executing. Classic Heisenbug in testing.

**Solutions:**

**Option A: Clippy rule (recommended)**
```rust
// Forbid conditional assertions in tests
#![deny(clippy::assertions_in_conditional)]
```

**Option B: Restructure test**
```rust
#[test]
fn test_detect_in_main_worktree() {
    let paths = WorktreePaths::detect().unwrap();
    
    // Test the actual condition
    let expected_is_main = paths.common_dir == paths.worktree_root.join(".git");
    assert_eq!(paths.is_worktree(), !expected_is_main, 
        "is_worktree() should be false when common_dir matches worktree_root/.git");
}
```

**Option C: Property-based testing**
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_worktree_invariant(
        common_dir in any::<PathBuf>(),
        worktree_root in any::<PathBuf>()
    ) {
        let paths = WorktreePaths { common_dir, worktree_root, ... };
        
        // Invariant: is_worktree() is true IFF common_dir != worktree_root/.git
        assert_eq!(
            paths.is_worktree(),
            paths.common_dir != paths.worktree_root.join(".git")
        );
    }
}
```

**Recommendation:** Use Option B immediately (restructure existing tests) + add Option C for critical invariants.

---

### 2. Unused Storage Parameter Pattern

**Problem:**  
Most `execute_*` functions in claim and worktree commands take a `storage: &S` parameter but don't use it.

**Examples:**
```rust
pub fn execute_claim_list<S: IssueStore>(_storage: &S) -> Result<Vec<Lease>>
pub fn execute_claim_renew<S: IssueStore>(_storage: &S, ...) -> Result<Lease>
pub fn execute_worktree_info<S: IssueStore>(_storage: &S) -> Result<WorktreeInfo>
```

**Why it exists:**  
For API consistency with other commands that DO use storage (issue CRUD operations).

**Trade-offs:**

| Keep it | Remove it |
|---------|-----------|
| ✅ Uniform API across all commands | ✅ Honest API (no unused params) |
| ✅ Easy to add storage use later | ✅ Clearer to readers |
| ❌ Confusing (why pass if not used?) | ❌ Breaks API uniformity |
| ❌ Easy to accidentally use wrong storage | ❌ Harder to add storage use later |

**Recommendation:**  
Remove the parameter from commands that genuinely don't need it. The API inconsistency is acceptable - claim/worktree commands operate on control plane (`.git/jit/`), not data plane (`.jit/issues/`).

**Implementation:**
1. Remove `_storage` parameter from `execute_claim_*` and `execute_worktree_*`
2. Update call sites in `main.rs`
3. Update tests

**Caveat:** If future requirements need storage in these commands, we'll add it back. That's fine - API evolution is normal.

---

### 3. Git Detection Silent Fallbacks

**Problem:**  
`get_current_branch()` returns `"main"` on error, hiding git failures.

**Current code:**
```rust
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to get current git branch")?;

    if !output.status.success() {
        return Ok("main".to_string()); // Silent fallback!
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
```

**Why it's dangerous:**
- User not in git repo → silently uses "main" → wrong worktree identity → lease conflicts
- Detached HEAD state → silent "main" → wrong branch association
- Git error → masked → debugging nightmare

**Solution:**
```rust
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to execute git command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Git command failed: {}. Are you in a git repository?", stderr.trim());
    }

    let branch = String::from_utf8(output.stdout)
        .context("Git output is not valid UTF-8")?
        .trim()
        .to_string();
    
    if branch.is_empty() {
        bail!("Git returned empty branch name");
    }

    Ok(branch)
}
```

**Testing:**
```rust
#[test]
fn test_get_current_branch_outside_git_repo() {
    let temp = TempDir::new().unwrap();
    env::set_current_dir(&temp).unwrap();
    
    let result = get_current_branch();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("git repository"));
}
```

---

### 4. Path Handling Inconsistency

**Problem:**  
Inconsistent handling of relative vs absolute paths leads to bugs.

**Example Bug (fixed in commit 48d3b83):**
```rust
// Git returns ".git" (relative)
let common_dir = PathBuf::from(git_output.trim());

// Later comparison uses absolute path
if common_dir != worktree_root.join(".git") {  // ".git" != "/path/.git" ❌
    // Always true, even in main worktree!
}
```

**Principle to establish:**  
**Always canonicalize paths from external sources before storage or comparison.**

External sources:
- Git commands (`git rev-parse`, `git worktree list`)
- User input (CLI arguments, config files)
- Environment variables (`GITDIR`, paths)

**Pattern to follow:**
```rust
// BAD: Use raw path from git
let common_dir = PathBuf::from(git_output.trim());

// GOOD: Canonicalize immediately
let common_dir_raw = PathBuf::from(git_output.trim());
let common_dir = if common_dir_raw.is_absolute() {
    common_dir_raw
} else {
    env::current_dir()?.join(common_dir_raw).canonicalize()?
};
```

**Where to apply:**
1. `WorktreePaths::detect()` - ✅ Fixed
2. `load_or_create_worktree_identity()` - Check needed
3. Document path handling (user-specified paths)
4. Anywhere using `Command` output for paths

**Testing:**
Add property tests:
```rust
proptest! {
    #[test]
    fn path_comparisons_work_with_relative_and_absolute(
        relative in r"[a-z]+(/[a-z]+)*",
        absolute in r"/[a-z]+(/[a-z]+)*"
    ) {
        // Test that our canonicalization handles both
    }
}
```

---

### 5. Test Helper Duplication

**Problem:**  
Multiple modules duplicate `setup_test_repo()`:

```rust
// In commands/claim.rs
fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> { ... }

// In commands/worktree.rs  
fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> { ... }

// In storage/claim_coordinator.rs
fn setup_test_repo() -> Result<...> { ... }
```

**Impact:**
- Code duplication (DRY violation)
- Inconsistent setup across tests
- Changes need to be replicated

**Solution:**  
Extract to shared test utilities module.

**Implementation:**
```rust
// crates/jit/src/test_utils.rs (or tests/common/mod.rs)
#![cfg(test)]

use anyhow::Result;
use std::fs;
use tempfile::TempDir;
use crate::storage::{JsonFileStorage, WorktreePaths};

/// Standard test repository setup
pub fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
    let temp = TempDir::new()?;
    
    // Create .jit directory
    let jit_root = temp.path().join(".jit");
    fs::create_dir_all(&jit_root)?;
    
    // Create .git directory
    let git_dir = temp.path().join(".git");
    fs::create_dir_all(&git_dir)?;
    
    // Initialize storage
    let storage = JsonFileStorage::new(&jit_root);
    storage.init()?;
    
    Ok((temp, storage))
}

/// Create test WorktreePaths from TempDir
pub fn create_test_paths(temp: &TempDir) -> WorktreePaths {
    WorktreePaths {
        common_dir: temp.path().join(".git"),
        worktree_root: temp.path().to_path_buf(),
        local_jit: temp.path().join(".jit"),
        shared_jit: temp.path().join(".git/jit"),
    }
}

/// Create a test issue
pub fn create_test_issue(
    storage: &JsonFileStorage,
    title: &str
) -> Result<String> {
    let issue = Issue::new(title.to_string(), "Test description".to_string());
    let issue_id = issue.id.clone();
    storage.save_issue(&issue)?;
    Ok(issue_id)
}
```

**Migration:**
1. Create `test_utils.rs`
2. Replace duplicated helpers with `use crate::test_utils::*`
3. Run tests to verify
4. Remove duplicated code

---

### 6. Missing Integration Tests

**Problem:**  
Good unit test coverage (24 claim tests, coordinator tests), but no end-to-end CLI testing.

**What's missing:**
- Actual `jit` binary execution
- Exit code validation
- Output format validation (text and JSON)
- Error message verification
- Cross-command workflows

**Example of what we DON'T test:**
```bash
# This workflow is not tested end-to-end:
$ jit claim acquire abc123 --ttl 600
✓ Acquired lease: 01HXJK...

$ jit claim list --json
{"leases": [...]}

$ jit claim release 01HXJK...
✓ Released lease: 01HXJK...
```

**Solution:**  
Add integration tests in `tests/` directory.

**Implementation:**
```rust
// tests/cli_claim_integration.rs

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_claim_acquire_workflow() {
    let temp = TempDir::new().unwrap();
    let jit_root = temp.path().join(".jit");
    
    // Initialize repo
    Command::cargo_bin("jit")
        .unwrap()
        .current_dir(&temp)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));
    
    // Create issue
    let output = Command::cargo_bin("jit")
        .unwrap()
        .current_dir(&temp)
        .args(&["issue", "create", "--title", "Test", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();
    
    // Acquire claim
    Command::cargo_bin("jit")
        .unwrap()
        .current_dir(&temp)
        .args(&["claim", "acquire", issue_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Acquired lease"));
    
    // Verify with list
    Command::cargo_bin("jit")
        .unwrap()
        .current_dir(&temp)
        .args(&["claim", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(issue_id));
}

#[test]
fn test_claim_acquire_already_claimed_error() {
    // Setup...
    
    Command::cargo_bin("jit")
        .unwrap()
        .args(&["claim", "acquire", issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already claimed"));
}
```

**Dependencies:**
```toml
[dev-dependencies]
assert_cmd = "2.0"
predicates = "3.0"
```

**Coverage targets:**
- Happy path for each command
- Error conditions
- JSON output validation
- Cross-command workflows

---

### 7. Error Messages Not Actionable

**Problem:**  
Error messages state the problem but don't suggest solutions.

**Examples:**

**Current:**
```
Error: Lease abc123 not found
```

**Improved:**
```
Error: Lease abc123 not found

Possible causes:
  • Lease has expired
  • Incorrect lease ID
  
Try: jit claim list --json | jq -r '.data.leases[] | .lease_id'
```

**Current:**
```
Error: Cannot acquire claim on issue xyz: already claimed by agent:other
```

**Improved:**
```
Error: Cannot acquire claim on issue xyz: already claimed by agent:other

The issue is locked by another agent. To proceed:
  • Wait for the lease to expire
  • Contact the agent owner to release it
  • Use force-evict (admin only): jit claim force-evict <lease-id> --reason "..."
  
View lease details: jit claim status --issue xyz --json
```

**Pattern:**
```rust
// Helper for actionable errors
struct ActionableError {
    error: String,
    causes: Vec<String>,
    remediation: Vec<String>,
}

impl ActionableError {
    fn to_error_message(&self) -> String {
        let mut msg = format!("Error: {}\n", self.error);
        
        if !self.causes.is_empty() {
            msg.push_str("\nPossible causes:\n");
            for cause in &self.causes {
                msg.push_str(&format!("  • {}\n", cause));
            }
        }
        
        if !self.remediation.is_empty() {
            msg.push_str("\nTo fix:\n");
            for remedy in &self.remediation {
                msg.push_str(&format!("  • {}\n", remedy));
            }
        }
        
        msg
    }
}
```

**Where to apply:**
1. Claim acquisition errors (already claimed, issue not found)
2. Lease operations (not found, not owned)
3. Worktree detection failures
4. Git command failures

**Trade-off:**  
More verbose errors, but much better UX.

---

## Implementation Plan

### Phase 1: Quick Wins (1-2 hours)
1. Fix conditional test assertions (Issue #1)
2. Improve git error messages (Issue #3)
3. Document path canonicalization principle (Issue #4)

### Phase 2: Refactoring (2-4 hours)
4. Extract test helpers to shared module (Issue #5)
5. Remove unused storage parameters (Issue #2)

### Phase 3: Enhancement (4-8 hours)
6. Add integration tests (Issue #6)
7. Improve error messages with actionable hints (Issue #7)

## Success Criteria

- [ ] All existing tests still pass
- [ ] No regression in functionality
- [ ] At least 3 integration tests added
- [ ] Error messages include actionable hints for top 5 error scenarios
- [ ] Path handling principle documented in `.copilot-instructions.md`
- [ ] Test helpers consolidated (< 50 lines total duplication)
- [ ] Unused parameters removed or justified in comments

## References

- Original implementation: commits 3ef9bff through ddcf41a
- Bug fix example: commit 48d3b83 (path canonicalization)
- Refactoring examples: commits 11756a6, 543e802
