# Gate Enforcement Bug Analysis

**Date**: 2025-12-20  
**Issue**: 544fd21b-ece7-4d81-993a-062fd3545459  
**Discovered During**: Issue 0ce16136 (Improve gate CLI help text clarity)

## Problem Summary

During the completion of the gate help text improvement issue, we discovered that the gate system has a critical enforcement bug: prechecks are not enforced when claiming issues.

## Detailed Findings

### 1. Precheck Bypass in `claim_issue`

**Location**: `crates/jit/src/commands/issue.rs::claim_issue()`

**Bug**: The function directly modifies the issue state without going through the gate enforcement pipeline:

```rust
// Current (BROKEN) implementation:
pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
    let mut issue = self.storage.load_issue(id)?;
    
    if issue.assignee.is_some() {
        return Err(anyhow!("Issue is already assigned"));
    }
    
    let old_state = issue.state;
    issue.assignee = Some(assignee.clone());
    
    // Transition to InProgress if Ready
    if issue.state == State::Ready {
        issue.state = State::InProgress;  // ❌ BYPASSES PRECHECKS!
    }
    
    self.storage.save_issue(&issue)?;
    // ... event logging ...
}
```

**Correct Pattern**: The `update_issue_state()` method properly enforces gates:

```rust
// In update_issue_state():
if old_state == State::Ready && new_state == State::InProgress {
    self.run_prechecks(id)?;  // ✅ Enforces prechecks
}
```

### 2. Impact

This bug breaks the TDD workflow:
- Issues with precheck gates (like `tdd-reminder`) can be claimed and work started
- The precheck gates are never validated
- Agents/users can bypass quality checkpoints

**Example**: Issue 0ce16136 had a `tdd-reminder` precheck gate, but I was able to claim it and start work without passing the gate first.

### 3. What Works Correctly

The gate execution system itself is properly implemented:
- ✅ `jit gate check <issue> <gate-key>` - runs single automated gate
- ✅ `jit gate check-all <issue>` - runs all automated gates
- ✅ `update_issue_state()` - properly enforces prechecks on Ready→InProgress
- ✅ Automated gates execute checker commands and auto-pass/fail based on exit codes

The problem is **only** that `claim_issue` doesn't use the proper state transition path.

## Proposed Fix

Refactor `claim_issue` to use `update_issue_state()` for state transitions:

```rust
pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
    let mut issue = self.storage.load_issue(id)?;
    
    if issue.assignee.is_some() {
        return Err(anyhow!("Issue is already assigned"));
    }
    
    let old_state = issue.state;
    issue.assignee = Some(assignee.clone());
    self.storage.save_issue(&issue)?;
    
    // Log assignment
    let event = Event::new_issue_claimed(issue.id.clone(), assignee);
    self.storage.append_event(&event)?;
    
    // Transition to InProgress if Ready (this will enforce prechecks)
    if old_state == State::Ready {
        self.update_issue_state(id, State::InProgress)?;
    }
    
    Ok(())
}
```

**Key Changes**:
1. Save issue with assignee first
2. Use `update_issue_state()` for state transition (which runs prechecks)
3. Handle the case where prechecks fail (claim succeeds, but state remains Ready)

## Testing Recommendations

1. Create test with precheck gate
2. Attempt to claim the issue
3. Verify prechecks are enforced (claim should fail or succeed with state=Ready if precheck fails)
4. Verify automated prechecks execute their checker commands
5. Verify manual prechecks block until manually passed

## Related Issues

- Issue 0ce16136: Improve gate CLI help text clarity (where bug was discovered)
- Epic 4a00b2b0: AI Agent Validation (this is an agent-friendliness issue)

## Observations

This bug demonstrates the importance of:
1. **Consistent code paths**: State transitions should always go through the same enforcement logic
2. **Integration testing**: Unit tests exist for `run_prechecks` but the bypass in `claim_issue` wasn't caught
3. **Real-world usage**: Discovered only when actually using the gate system in practice
