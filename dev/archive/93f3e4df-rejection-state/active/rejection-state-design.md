# Simplified Closure Plan: Done vs Rejected

## Current Insight

You're right - we don't need multiple closure states. Let's keep it simple:

```
Terminal States:
- Done      = Successfully implemented and delivered
- Rejected  = Won't implement (any reason)
```

## Why This is Better

**Simpler Mental Model:**
- Success path: `backlog → ready → in_progress → gated → done`
- Rejection path: `backlog → ready → rejected` (or from any state)
- No need to distinguish between wont-fix/duplicate/invalid

**Label Flexibility:**
If we need to track *why* something was rejected:
```
State: Rejected
Labels: ["resolution:wont-fix"] or ["resolution:duplicate"] etc.
```

This gives us:
- ✅ Simple state model (2 terminal states, not 5)
- ✅ Flexibility via labels (can add new reasons without code changes)
- ✅ Clear semantics (done = success, rejected = not doing it)
- ✅ Both bypass gates

## Proposed Implementation

### 1. Add Single Rejected State

```rust
pub enum State {
    Backlog,
    Ready,
    InProgress,
    Gated,
    Done,      // Successfully implemented
    Rejected,  // Won't implement (bypasses gates)
}
```

### 2. Gate Bypass Logic

```rust
fn should_check_gates(new_state: &State) -> bool {
    match new_state {
        State::Done => true,      // Must pass gates
        State::Rejected => false, // Bypass gates
        _ => false,
    }
}
```

### 3. CLI Commands

```bash
# Simple rejection (no reason needed)
jit issue update <id> --state rejected

# With reason label (optional)
jit issue update <id> --state rejected --label resolution:wont-fix

# Or convenience command
jit issue reject <id>  # shortcuts to --state rejected
```

### 4. Query Support

```bash
jit query all --state rejected
jit query all --state done

# Query terminal states
jit query closed  # returns both Done and Rejected
```

### 5. Status Reporting

```bash
jit status
# Output:
#   Open: 8
#   Ready: 15
#   In Progress: 0
#   Done: 6
#   Rejected: 1
#   Total: 30
```

## Implementation Tasks

**Epic: "Issue Lifecycle - Rejection State"**

Tasks:
1. Add `Rejected` state to enum
2. Implement gate bypass for `Rejected` state
3. Update state transition validation
4. Update `jit status` to show rejected count
5. Add `jit query closed` for terminal states
6. Optional: Add `jit issue reject <id>` convenience command
7. Add tests for rejection workflow
8. Update documentation

**Effort:** ~1-2 hours (much simpler than multi-state approach)

## Migration

Existing issues: No migration needed
- `resolution:*` labels remain optional metadata
- Can manually transition the one stuck issue when ready

## Comparison: Done vs Rejected

| Aspect | Done | Rejected |
|--------|------|----------|
| **Meaning** | Successfully delivered | Won't implement |
| **Gates** | Must pass all gates | Bypasses gates |
| **Query** | `jit query all --state done` | `jit query all --state rejected` |
| **Metrics** | Counts as completion | Counts as closure |
| **Labels** | Optional | Optional (resolution:*) |

## Do We Need resolution:* Labels?

**Answer: Optional/Nice-to-have**

State alone is sufficient:
- `State: Done` = implemented
- `State: Rejected` = not implementing

Labels add detail if desired:
- `State: Rejected, Labels: [resolution:wont-fix]` = explicitly won't fix
- `State: Rejected, Labels: [resolution:duplicate]` = tracked elsewhere
- `State: Rejected` (no label) = rejected for unspecified reason

**Recommendation:** Keep labels for organizational purposes but don't require them.

## Open Questions

### Q1: Can rejected issues be reopened?
**A:** Yes, allow `rejected → ready` or `rejected → backlog` transitions.

### Q2: Should rejection require a comment/reason?
**A:** No. Description field can be used. Labels are optional metadata.

### Q3: Do we distinguish "rejected" from "closed"?
**A:** Both `done` and `rejected` are "closed" (terminal). The distinction is success vs. non-implementation.

### Q4: What about "deferred" or "on-hold"?
**A:** Use `backlog` state with `priority: low` or a label like `status:deferred`. Don't need a new state.

## Advantages Over Multi-State Approach

1. **Simpler:** 2 terminal states instead of 5
2. **Easier to understand:** done/rejected is clearer than done/wont-fix/duplicate/invalid/obsolete
3. **Less code:** Fewer enum variants, simpler matching
4. **Flexible:** Labels handle nuance without state complexity
5. **Faster to implement:** Single new state, not four

## Example Workflows

### Reject as Won't Fix
```bash
jit issue update 3342dc7c --state rejected --label resolution:wont-fix
# Gates bypassed, transitions directly to Rejected
```

### Reject as Duplicate
```bash
jit issue update abc123 --state rejected --label resolution:duplicate
# Optional: link to canonical issue in description
```

### Reject Without Reason
```bash
jit issue update xyz789 --state rejected
# No label needed, state alone is sufficient
```

### Query All Closures
```bash
jit query closed
# Returns all Done and Rejected issues
```

## Decision Point

**Do we need any states beyond `Done` and `Rejected`?**

Consider:
- **Cancelled** (planned but no longer needed) → Use `Rejected` + label if distinction matters
- **Obsolete** (became irrelevant) → Use `Rejected` + label
- **On-Hold/Deferred** → Use `Backlog` + label, not terminal state

**Recommendation:** Start with just `Rejected`. Add more only if clear need emerges.
