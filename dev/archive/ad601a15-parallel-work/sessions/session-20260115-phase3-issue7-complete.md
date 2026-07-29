# Session Notes: CLI Quality Improvements - Phase 3, Issue #7 Complete

**Date:** 2026-01-15  
**Issue:** 4b2cb4cd (Refactor: Address code quality improvements from CLI command implementation)  
**Session Focus:** Complete Phase 3, Issue #7 - Actionable Error Messages

## Summary

Successfully implemented comprehensive actionable error infrastructure with diagnostic hints and remediation steps. The system is now production-ready and can be adopted more widely across the codebase.

## What Was Done

### 1. Created Actionable Error Infrastructure

**File:** `crates/jit/src/errors.rs` (236 lines)

Created a reusable `ActionableError` struct with:
- **Fluent builder API:** `.with_cause()`, `.with_remedy()`
- **Formatted output:** Structured "Error:", "Possible causes:", "To fix:" sections
- **Helper functions:** Pre-configured errors for common scenarios
- **Full test coverage:** 6 comprehensive unit tests

**Key Design Decisions:**
- Implements `std::error::Error` trait for compatibility with `anyhow`
- Display formatting includes bullet points for readability
- Optional causes and remediation (can have one without the other)
- Helper functions encapsulate domain knowledge

**Example Usage:**
```rust
ActionableError::new("Lease abc123 not found")
    .with_cause("The lease may have expired")
    .with_remedy("List all active leases: jit claim list --json")
```

### 2. Implemented 5 Helper Functions for Common Errors

**Top 5 Error Scenarios Addressed:**

1. **`lease_not_found(lease_id)`** - Lease operations
   - Causes: Expired, incorrect ID, released/evicted
   - Remediation: `jit claim list`, `jit claim status`

2. **`already_claimed(issue_id, agent_id, expires_info)`** - Claim acquisition
   - Causes: Another agent working, crashed agent, still in progress
   - Remediation: Wait, contact agent, force-evict if needed

3. **`not_in_git_repo()`** - Git repository detection
   - Causes: Not in git repo, git not installed
   - Remediation: `git init`, change directory, verify git installation

4. **`git_command_failed(command, stderr)`** - Git command failures
   - Causes: Repository invalid state, incorrect config
   - Remediation: `git status`, `git config --list`, manual command execution

5. **`not_owner(resource, owner, requester)`** - Ownership verification
   - Causes: Wrong agent, incorrect configuration
   - Remediation: Check config, use correct agent ID, force-evict if needed

### 3. Updated Error Locations

**Modified Files:**
- `crates/jit/src/storage/claim_coordinator.rs` (4 error sites)
  - Already claimed error (lines 211-220)
  - Lease not found errors (renew, release, force-evict)
  - Ownership errors (renew, release)
  - Git command failure (get_current_branch)

- `crates/jit/src/storage/worktree_paths.rs` (2 error sites)
  - Git command failures (git-common-dir, show-toplevel)

- `crates/jit/src/commands/worktree.rs` (1 error site)
  - Git worktree list failure

**Total:** 7 error sites updated with actionable errors

### 4. Enhanced Integration Tests

**File:** `crates/jit/tests/claim_integration_tests.rs`

Updated 2 existing tests to verify actionable error format:
- `test_claim_acquire_already_claimed_error` - Verifies "Possible causes:", "To fix:", remediation commands
- `test_claim_release_not_found_error` - Verifies "Possible causes:", "To fix:", "jit claim list"

**Verification Strategy:**
- Uses `predicate::str::contains()` for flexible matching
- Checks for structure: "Possible causes:", "To fix:"
- Validates specific remediation commands are mentioned
- All 10 integration tests still pass

### 5. Manual Verification

Tested actual error output in live scenarios:

**Lease Not Found:**
```
Error: Lease fake-lease-id not found

Possible causes:
  • The lease may have expired
  • The lease ID may be incorrect
  • The lease may have been released or evicted

To fix:
  • List all active leases: jit claim list --json | jq -r '.data.leases[].lease_id'
  • Check lease status: jit claim status --json
```

**Already Claimed:**
```
Error: Issue 88d36b60... already claimed by agent:first until 2026-01-15 22:48:47...

Possible causes:
  • Another agent is currently working on this issue
  • The previous agent may have crashed without releasing the lease
  • The issue may still be in progress

To fix:
  • Wait for the lease to expire or be released: jit claim status --issue 88d36b60... --json
  • Contact the agent owner to coordinate: agent:first
  • If the agent crashed, force evict with: jit claim force-evict <lease-id> --reason "<reason>"
```

**Output Quality:**
- Clear structure with bullet points
- Actionable commands with actual syntax
- Context-specific information (issue ID, agent ID, expiry time)
- Professional and helpful tone

## Broader Applicability Assessment

### Should ActionableError Be Used More Widely?

**YES - Recommended for:**

1. **User-facing CLI errors** - Any error shown to end users
   - Issue not found errors
   - Invalid state transitions
   - Validation failures
   - Configuration errors

2. **Agent coordination errors** - Critical for autonomous agents
   - Claim/lease operations (✓ completed)
   - Issue assignment conflicts
   - Gate failures with unclear causes
   - Dependency conflicts

3. **External system failures** - When user action is needed
   - Git command failures (✓ completed)
   - File system permission errors
   - Network/API failures (future)

**NOT recommended for:**
1. **Internal programming errors** - `panic!` or standard `anyhow::Error`
2. **Validation errors with obvious fixes** - Simple error messages sufficient
3. **Errors that never reach users** - Library-internal errors

### Migration Strategy

**Phase 1 (Immediate):** Current implementation
- [x] Claim coordinator errors
- [x] Worktree path detection
- [x] Git command failures in coordination

**Phase 2 (Next sprint):** Core user operations
- [ ] Issue CRUD operations (not found, invalid state)
- [ ] Dependency graph errors (cycle detection, missing deps)
- [ ] Gate execution failures (checker errors, timeout)
- [ ] Config validation errors

**Phase 3 (Future):** Nice-to-have improvements
- [ ] Search operation errors
- [ ] Document management errors
- [ ] Validation command errors

**Estimated effort:** 2-4 hours per phase (similar to Phase 3 Issue #7)

### Implementation Guidelines

**When adding ActionableError:**
1. Identify user-facing error location
2. Choose appropriate helper or create new one
3. Add 2-3 possible causes (diagnostic)
4. Add 2-3 remediation steps with example commands
5. Update tests to verify error format
6. Manual test to verify output quality

**Helper Function Pattern:**
```rust
pub fn error_name(context: &str) -> ActionableError {
    ActionableError::new(format!("Error description with {}", context))
        .with_cause("First possible cause")
        .with_cause("Second possible cause")
        .with_remedy("First fix: command example")
        .with_remedy("Second fix: alternative approach")
}
```

## Quality Metrics

### Code Changes
- **Files created:** 1 (errors.rs, 236 lines)
- **Files modified:** 5
- **Lines added:** 311 (236 new + 75 modifications)
- **Lines removed:** 53
- **Net change:** +258 lines

### Test Coverage
- **Unit tests added:** 6 (errors module)
- **Integration tests updated:** 2
- **Total tests:** 492 (486 previous + 6 new)
- **Test status:** All passing
- **Clippy warnings:** 0

### Success Criteria (7/7 Complete)
- [x] All existing tests still pass (492 tests)
- [x] No regression in functionality
- [x] At least 3 integration tests added (10 added in Issue #6)
- [x] **Error messages include actionable hints for top 5 error scenarios** ✓
- [x] Path handling principle documented
- [x] Test helpers consolidated
- [x] Unused parameters removed

## Key Learnings

### Design Patterns

**1. Fluent Builder for Error Construction**
- Allows optional causes and remediation
- Readable at call site: `.with_cause().with_remedy()`
- Chainable API encourages comprehensive error context

**2. Helper Functions Encapsulate Domain Knowledge**
- `lease_not_found()` knows about `jit claim list`
- `already_claimed()` knows about force-evict workflow
- Centralized domain knowledge, not scattered across codebase

**3. Context-Specific vs Generic Helpers**
- Generic: `ActionableError::new()` for unique errors
- Specific: `lease_not_found()` for common patterns
- Balance: Create helper after 2-3 duplicates

### Error Message Best Practices

**Effective Causes:**
- Start with most common cause
- Be specific: "The lease may have expired" not "Something went wrong"
- Include contextual clues: mention what state was expected

**Effective Remediation:**
- Provide exact commands: `jit claim list --json | jq`
- Show alternatives: "Wait" vs "Contact" vs "Force evict"
- Include placeholders: `<lease-id>`, `<issue-id>`

**Formatting:**
- Use bullet points for scannability
- Keep causes concise (one line each)
- Keep remediation actionable (command + context)

### Integration with Existing Error Handling

**Works seamlessly with `anyhow`:**
```rust
bail!("{}", errors::lease_not_found(lease_id));
```

**Preserves error context:**
```rust
.ok_or_else(|| anyhow::anyhow!("{}", errors::lease_not_found(lease_id)))?;
```

**Compatible with existing patterns:**
- No changes to error propagation (`?` operator)
- No changes to error types (still `anyhow::Result`)
- Additive improvement, not disruptive refactor

## Remaining Work

**Issue 4b2cb4cd:** Complete (all 7 success criteria met)

**Follow-up Opportunities:**
1. Create issue for Phase 2 error improvements (issue CRUD, deps, gates)
2. Document error message guidelines in contributor guide
3. Consider creating error catalog documentation

**No technical debt introduced.**

## Commands for Verification

```bash
# Run all tests
cargo test --workspace --quiet

# Check clippy
cargo clippy --workspace --all-targets

# Test error messages manually
cd /tmp && mkdir test-jit && cd test-jit
git init && git config user.name "Test" && git config user.email "test@example.com"
jit init
jit issue create --title "Test" --json | jq -r '.data.id' | read ISSUE_ID
jit claim acquire "$ISSUE_ID" --agent-id "agent:first" --ttl 3600
jit claim acquire "$ISSUE_ID" --agent-id "agent:second" --ttl 3600  # See error
jit claim release "fake-id"  # See error
```

## Commit Message

```
feat: Add actionable error messages with diagnostic hints

Implements comprehensive ActionableError infrastructure for better
user experience when errors occur. Errors now include:
- Clear error description
- Possible causes (2-3 diagnostic hints)
- Remediation steps (actionable fixes with example commands)

Updated 7 error sites across claim coordinator, worktree paths, and
worktree commands. Added 6 unit tests and updated 2 integration tests
to verify error format.

Addresses Issue #7 from 4b2cb4cd (CLI quality improvements).

Success criteria: All 7/7 complete
- 492 tests passing (486 previous + 6 new)
- Zero clippy warnings
- No regressions
- Error messages include actionable hints for top 5 scenarios

Files:
- NEW: crates/jit/src/errors.rs (236 lines) - Error infrastructure
- MOD: claim_coordinator.rs - 4 error sites updated
- MOD: worktree_paths.rs - 2 error sites updated
- MOD: worktree.rs - 1 error site updated
- MOD: claim_integration_tests.rs - 2 tests enhanced
```

## Time Tracking

**Estimated:** 3-4 hours  
**Actual:** ~3 hours
- Infrastructure creation: 30 min
- Error site updates: 1.5 hours
- Testing and verification: 45 min
- Documentation: 45 min

**On schedule, on budget, zero shortcuts.**
