# State Transition Feedback Design

**Date**: 2025-12-20  
**Issue**: 4ae60108-d2b0-4fbc-829a-7df6203b1850  
**Discovered During**: Dogfooding automated gate enforcement (issue 78460609)

## Problem Summary

When attempting to transition an issue to `done` state while gates remain unpassed, the system silently modifies the request and transitions to `gated` instead, returning exit code 0 (success). This violates the principle of least surprise and creates problems for automation.

## Current Behavior

```bash
$ jit issue update 78460609-595f-49fc-9255-1af163680e6e --state done
Updated issue: 78460609-595f-49fc-9255-1af163680e6e
$ echo $?
0

$ jit issue show 78460609-595f-49fc-9255-1af163680e6e | grep State
State: Gated  # NOT Done!
```

**What happened:**
1. User requested state = `done`
2. System detected unpassed gate (`tdd-reminder`)
3. System silently transitioned to `gated` instead
4. Command returned success (exit 0)
5. No indication that request was modified

## Why This Is Problematic

### 1. **Automation Hazard**
```bash
# Dangerous pipeline:
jit issue update $ISSUE --state done && deploy-to-production

# If this succeeds but issue is actually 'gated', deployment happens
# even though work isn't truly complete!
```

### 2. **Principle of Least Surprise Violation**
- User explicitly requests state `done`
- System does something different
- No error or warning
- Exit code indicates success

This is confusing for both humans and AI agents.

### 3. **Agent-Unfriendly**
AI agents expect clear contracts:
- Exit 0 = command succeeded as requested
- Exit non-zero = constraint prevented success
- Don't silently do something different and pretend it succeeded

### 4. **Discovered Via Dogfooding**
During completion of issue 78460609, I (AI agent) ran the command and didn't notice the issue was `gated` not `done` until checking the web UI. The silent success masked the gate enforcement.

## Current Implementation

From `crates/jit/src/commands/issue.rs`:

```rust
State::Done => {
    // Check both dependencies and gates
    let issues = self.storage.list_issues()?;
    let issue_refs: Vec<&Issue> = issues.iter().collect();
    let resolved: HashMap<String, &Issue> =
        issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

    if issue.is_blocked(&resolved) {
        return Err(anyhow!(
            "Cannot transition to Done: issue blocked by incomplete dependencies"
        ));
    }

    // If gates not passed, transition to Gated instead
    if issue.has_unpassed_gates() {
        issue.state = State::Gated;  // ⚠️ Silent modification
    } else {
        issue.state = State::Done;
    }
}
```

**Note:** Dependencies block with an error, but gates silently modify. Inconsistent!

## Proposed Solution

### Keep Automatic Transition BUT Return Error

The `gated` state is useful - it represents "work complete, awaiting gate approval". We should keep the automatic transition but make it explicit:

```rust
State::Done => {
    // Check dependencies
    if issue.is_blocked(&resolved) {
        return Err(anyhow!(
            "Cannot transition to Done: issue blocked by incomplete dependencies"
        ));
    }

    // Check gates
    if issue.has_unpassed_gates() {
        issue.state = State::Gated;  // Still auto-transition
        
        // But return error with clear feedback
        let unpassed: Vec<_> = issue.gates_required.iter()
            .filter(|g| !issue.gates_status.get(*g).map_or(false, |s| s.status == GateStatus::Passed))
            .collect();
            
        return Err(anyhow!(
            "Cannot transition to 'done' - {} gate(s) not passed: {}\n\
             → Issue automatically transitioned to 'gated' (awaiting gate approval)\n\
             The issue will auto-transition to 'done' when all gates pass.",
            unpassed.len(),
            unpassed.join(", ")
        ));
    } else {
        issue.state = State::Done;
    }
}
```

### User Experience

**Terminal Output:**
```bash
$ jit issue update <id> --state done
Error: Cannot transition to 'done' - 1 gate(s) not passed: tdd-reminder

→ Issue automatically transitioned to 'gated' (awaiting gate approval)
The issue will auto-transition to 'done' when all gates pass.

To complete this issue:
  Pass gate: jit gate pass <id> tdd-reminder

$ echo $?
4  # Validation failure exit code
```

**JSON Output:**
```json
{
  "success": false,
  "error": {
    "code": "GATE_VALIDATION_FAILED",
    "message": "Cannot transition to 'done' - 1 gate(s) not passed: tdd-reminder",
    "details": {
      "requested_state": "done",
      "actual_state": "gated",
      "unpassed_gates": ["tdd-reminder"]
    }
  }
}
```

## Benefits

1. **Scripts can detect constraint violations:**
   ```bash
   if ! jit issue update $ID --state done; then
       echo "Gates still pending, not ready for deployment"
       exit 1
   fi
   ```

2. **Clear feedback for users and agents:**
   - Know exactly why transition failed
   - Understand what state issue is actually in
   - Get actionable next steps

3. **Consistent with dependency blocking:**
   - Dependencies block with error → Gates block with error
   - Same principle applied consistently

4. **Maintains automatic transition:**
   - `gated` state still serves its purpose
   - Auto-transition to `done` when gates pass still works
   - System does the right thing, but communicates it clearly

## Alternative Considered: Explicit Gated Command

We could add a separate command for submitting work:
```bash
jit issue submit <id>  # Runs postchecks, transitions to gated
```

And make `--state done` only work if all gates passed.

**Rejected because:**
- More complex API surface
- Current auto-transition is helpful
- Just needs better communication

## Exit Code Semantics

Following established pattern in `crates/jit/src/cli.rs`:
- Exit 0: Success
- Exit 1: Generic error
- Exit 2: Invalid arguments
- Exit 3: Resource not found
- **Exit 4: Validation failed** ← Use this for gate blocking

## Implementation Checklist

- [ ] Modify `update_issue_state()` to return error when gates block `done` transition
- [ ] Keep automatic transition to `gated` (don't change state machine)
- [ ] Return exit code 4 for validation failure
- [ ] Format error message with unpassed gates list
- [ ] Support JSON error output with details
- [ ] Update tests to expect error instead of silent success
- [ ] Add test for auto-transition to `done` after gates pass
- [ ] Update documentation about state transitions
- [ ] Verify consistency with dependency blocking behavior

## Related Issues

- Issue 78460609: Prevent manual pass/fail of automated gates (where bug was discovered)
- Issue 544fd21b: Fix claim_issue to enforce prechecks
- Epic 4a00b2b0: AI Agent Validation (this is critical for agent usability)

## Observations

This issue perfectly demonstrates why AI agent validation testing is valuable:
1. **Discovered via dogfooding** - Using the system as intended exposed the UX flaw
2. **Silent failures are agent-hostile** - AI agents rely on exit codes and clear feedback
3. **Principle of least surprise matters** - Both humans and agents need predictable behavior
4. **Small details have big impact** - A missing error code can break entire automation pipelines

The fix is small (return error instead of silent success) but the impact on usability is significant.
