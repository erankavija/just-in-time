# Gate Preview Command Analysis

**Issue:** #3342dc7c-5ef2-44c3-a65e-d3c5c81277e9  
**Decision:** Closed - Low value, unnecessary complexity  
**Date:** 2025-12-20

## Proposed Feature
Add `jit gate preview <issue>` to run prechecks without state change. Cache results for 5 minutes. Returns JSON with predicted precheck results for agent discoverability.

## Current Workflow (Works Well)
1. Try to claim issue → rejected if precheck gates not passed
2. Pass manual precheck gates (e.g., `jit gate pass <issue> tdd-reminder`)
3. Claim issue successfully
4. Do work
5. Run automated postchecks (tests, clippy, fmt)
6. Pass manual postcheck (code-review)
7. Mark as done

## Value Assessment

### Potential Use Cases
1. **Agent Discovery**: Agent wants to know "can I claim this?" before attempting
2. **Pre-flight Check**: Check if all prechecks pass before starting work
3. **Batch Processing**: Check multiple issues to find claimable ones

### Why Current Approach is Sufficient
- `jit issue claim` already runs prechecks and **fails fast** with clear error message
- Error message is actionable: `"Pass it first with: jit gate pass <issue> <gate>"`
- For agents: one failed claim attempt has negligible overhead
- For manual check: `jit issue show` displays `gates_required` and `gates_status`
- The claim operation itself IS the preview - it's idempotent and informative

## Problems with Proposed Solution

### Low Value
1. **Duplicates existing functionality** - claim already does this check
2. **Marginal benefit** - failing a claim is not expensive
3. **Agents don't need it** - they can just try to claim and handle the error
4. **No clear user pain** - current workflow works smoothly

### High Complexity
1. **Caching layer** - Adds state management with 5-minute TTL
2. **Cache invalidation** - What if gates change? Gate registry updated?
3. **Duplicate code path** - Must maintain preview logic separate from actual claim
4. **Testing burden** - Need to verify cache behavior, expiration, invalidation
5. **Race conditions** - Cached "can claim" might be stale when actual claim happens

## Better Alternative (If Needed)

Instead of a new command, enhance `jit issue show` with computed fields:

```json
{
  "id": "abc123",
  "state": "ready",
  "gates_required": ["tdd-reminder", "tests"],
  "gates_status": {
    "tdd-reminder": {"status": "pending"},
    "tests": {"status": "pending"}
  },
  "can_claim": false,
  "blocking_prechecks": ["tdd-reminder"]
}
```

Benefits:
- ✅ No new command
- ✅ No caching layer
- ✅ Minimal code changes
- ✅ No cache invalidation issues
- ✅ Works for both humans and agents
- ✅ Always accurate (computed on demand)

## Real-World Experience

During implementation of #241b7002, we experienced the precheck workflow:

```bash
$ jit issue claim 241b7002 agent:copilot
Error: Manual precheck 'tdd-reminder' has not been passed. 
Pass it first with: jit gate pass 241b7002 tdd-reminder

$ jit gate pass 241b7002 tdd-reminder
Passed gate 'tdd-reminder' for issue 241b7002

$ jit issue claim 241b7002 agent:copilot
Claimed issue: 241b7002
```

**Observation:** The error message was perfectly clear and actionable. No preview command was needed.

## Recommendation

**CLOSE THIS ISSUE** - The current workflow already solves the problem elegantly:
- Claim attempts are cheap
- Error messages are informative
- No caching complexity needed
- No maintenance burden

If agent discoverability becomes a real pain point in the future, revisit with the simpler `jit issue show` enhancement approach.

## Related Design Principles

From the project guidelines:
> **Correctness first, then performance** - this is not a performance-critical system

Optimizing for a single extra claim attempt (which fails instantly) violates this principle. The added complexity of caching doesn't justify the marginal performance gain.
