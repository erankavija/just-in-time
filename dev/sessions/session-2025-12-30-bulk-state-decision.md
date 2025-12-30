# Design Decision: Bulk State Transition Behavior

**Date:** 2025-12-30  
**Issue:** 40f594a7 - Design decision: State transition behavior in bulk operations  
**Decision:** Option B - Document Differences (Literal/Dumb Bulk)  
**Status:** ✅ DECIDED

## Problem Statement

Bulk update operations bypass `update_issue_state()` and its associated business logic, creating an inconsistency between single-issue and bulk operations.

**Single-issue state updates include:**
- Precheck execution (Ready → InProgress transition)
- Postcheck execution (Gated state with auto-transition to Done)
- Auto-transition to Gated (when attempting Done with unpassed gates)
- Dependency validation

**Bulk state updates currently:**
- Validate dependencies and gates (prevent invalid transitions)
- Do NOT run prechecks/postchecks
- Do NOT auto-transition to Gated
- Set exactly the state specified (literal assignment)

## Options Considered

### Option A: Shared Logic (Smart Bulk)
Extract state transition logic into shared function used by both single and bulk.

**Pros:** Perfect consistency, less duplication  
**Cons:** Unpredictable results, performance impact, complexity  
**Effort:** 3-4 hours

### Option B: Document Differences (Literal Bulk) ⭐ **SELECTED**
Keep bulk operations literal, document the difference clearly.

**Pros:** Predictable, simple, fast, safer for large operations  
**Cons:** Inconsistent with single-issue, documentation burden  
**Effort:** 30 minutes

### Option C: Hybrid Approach
Support both modes via `--smart` / `--literal` flag.

**Pros:** Maximum flexibility  
**Cons:** High complexity, confusing, two code paths to maintain  
**Effort:** 4-5 hours

## Decision: Option B (Literal Bulk)

### Rationale

1. **Simplicity Wins**
   - Current implementation already works
   - No refactoring risk
   - Minimal effort to document

2. **Bulk State Transitions Are Rare**
   - Most bulk operations will be label management, assignments
   - State transitions typically done per-issue with careful consideration
   - When bulk state changes are needed, explicit behavior is safer

3. **Predictability for Bulk Operations**
   - When updating 50+ issues, you want to know exactly what happens
   - "Set all to Gated" should mean "all → Gated", not "some → Done"
   - No hidden auto-transitions or gate executions

4. **Performance Considerations**
   - Running prechecks/postchecks for many issues could be slow
   - Gate execution failures create complex error scenarios
   - Literal updates are fast and atomic per-issue

5. **Industry Precedent**
   - SQL: `UPDATE issues SET state = 'done'` → literal assignment
   - jq: `.[] | .state = "done"` → literal assignment
   - sed, awk, etc. → literal replacements
   - **Bulk tools use literal semantics by design**

6. **Composability**
   - Users can layer operations for complex workflows:
     ```bash
     jit issue update --filter "epic:auth" --state gated
     jit gate check-all --filter "epic:auth"  # Explicit gate execution
     ```
   - More flexible than one monolithic "smart" operation

## Implementation

### Code Changes

Added comprehensive documentation in `bulk_update.rs` at state transition site:

```rust
// DESIGN DECISION: Bulk operations use literal state transitions.
//
// Unlike single-issue updates (`update_issue_state()`), bulk updates do NOT:
// - Run prechecks automatically (Ready → InProgress)
// - Run postchecks automatically (Gated state)
// - Auto-transition to Gated when attempting Done with unpassed gates
//
// Rationale:
// 1. Predictability: Users get exactly the state they specify
// 2. Performance: Avoiding gate execution for many issues
// 3. Safety: Explicit control for large-scale changes
// 4. Composability: Users can layer operations
// 5. Precedent: Bulk tools (SQL UPDATE, jq, sed) use literal semantics
//
// Validation still occurs (dependencies, gate requirements) but no
// automatic gate execution or state orchestration.
```

### What Bulk Operations DO

✅ **Validate dependencies** - Block Ready/Done if dependencies incomplete  
✅ **Validate gate requirements** - Block Done if gates unpassed  
✅ **Validate field formats** - Reject invalid labels, assignees  
✅ **Log events** - Both IssueStateChanged and IssueUpdated events  
✅ **Atomic per-issue updates** - Each issue updated independently  

### What Bulk Operations DO NOT Do

❌ **Run prechecks** - No automatic gate execution on Ready → InProgress  
❌ **Run postchecks** - No automatic gate execution on Gated state  
❌ **Auto-transition to Gated** - Attempting Done with unpassed gates → error, not auto-Gated  
❌ **Orchestrate complex workflows** - No multi-step state changes  

## Behavior Comparison

### Single-Issue: "Smart" Behavior

```bash
# Attempt to mark as done with unpassed gates
jit issue update abc123 --state done

# What happens:
# 1. Checks dependencies → OK
# 2. Checks gates → FAILED (gates unpassed)
# 3. Auto-transitions issue to Gated
# 4. Returns error but issue is now Gated
```

### Bulk: "Literal" Behavior

```bash
# Attempt to mark multiple issues as done
jit issue update --filter "epic:auth" --state done

# What happens for each issue:
# 1. Checks dependencies → OK or error (skips issue)
# 2. Checks gates → OK or error (skips issue)
# 3. If validation passes: state → Done (exactly as requested)
# 4. If validation fails: error reported, issue unchanged
#
# No auto-transitions, no gate execution, no orchestration
```

## User Guidance

### When to Use Bulk State Transitions

**Good use cases:**
- Mass state resets: `--filter "milestone:v0.9" --state archived`
- Bulk rejection: `--filter "type:wontfix" --state rejected`
- Coordinated planning: `--filter "epic:new-feature" --state backlog`

**Bad use cases:**
- Completing work (gates need individual attention)
- Complex workflows (use single-issue for orchestration)

### Composing Operations

For complex workflows, layer multiple bulk operations:

```bash
# 1. Transition to gated
jit issue update --filter "epic:auth AND state:in_progress" --state gated

# 2. Run postchecks explicitly (future feature)
jit gate check-all --filter "state:gated"

# 3. Transition successful ones to done
jit issue update --filter "state:gated AND gates:all-passed" --state done
```

## Future Considerations

### Potential Enhancements (Post-v1.0)

If users frequently need "smart" bulk behavior, could add:

1. **Bulk gate execution command**
   ```bash
   jit gate check-all --filter "epic:auth"
   ```

2. **Optional `--smart` flag** (if demand exists)
   ```bash
   jit issue update --filter "..." --state done --smart
   ```
   Would run prechecks/postchecks, but default remains literal

3. **Workflow shortcuts**
   ```bash
   jit workflow complete --filter "epic:auth"
   # Equivalent to: gated → postchecks → done
   ```

**Decision:** Defer these unless users request them. Start simple.

## Documentation Updates Needed

### Code Comments
✅ Added comprehensive comments in `bulk_update.rs`

### User Documentation (Future)
When CLI integration (Phase 4) is complete, update:

- **EXAMPLE.md** - Add bulk operations section with state transition examples
- **AGENT-QUICKSTART.md** - Note bulk behavior differences
- **FAQ** - "Why don't bulk operations run gates automatically?"

## Acceptance Criteria

From issue 40f594a7 (Option B checklist):

- ✅ Add clear doc comments in code explaining the difference
- ⏳ Update EXAMPLE.md with bulk vs single comparison (deferred to Phase 4)
- ⏳ Add FAQ section explaining design rationale (deferred to Phase 4)
- ✅ No code changes needed (behavior already correct)
- ✅ Tests verify current behavior (validation tests passing)

**Note:** User documentation updates deferred until CLI integration is complete (Phase 4). Code documentation is complete now.

## Impact

**Consistency:** ❌ Bulk differs from single-issue (acceptable tradeoff)  
**Predictability:** ✅ High - users get exactly what they request  
**Performance:** ✅ Fast - no gate execution overhead  
**Simplicity:** ✅ Current implementation, just documented  
**Safety:** ✅ Explicit control for large-scale operations  

## Lessons Learned

1. **Simplicity is a feature** - The simplest solution is often the right one
2. **Document decisions early** - Clear rationale prevents future confusion
3. **Rare use cases don't drive design** - Optimize for common cases
4. **Consistency ≠ uniformity** - Different tools can behave differently if well-documented

## Related Issues

- **Parent:** f5ce80bc (Implement bulk operations support)
- **Sibling:** d1c51bbd (Add field-level validation) - ✅ Complete
- **Blocks:** Phase 4 (CLI integration) can now proceed

---

**Decision Maker:** User (vkaskivuo)  
**Session End:** 2025-12-30 23:30 UTC  
**Outcome:** Design decision documented, code commented, issue ready to close
