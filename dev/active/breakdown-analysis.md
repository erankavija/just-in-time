# Analysis: `jit issue breakdown` Ergonomics Issues

## Executive Summary

The `jit issue breakdown` command has two significant ergonomics issues that create friction in hierarchical workflows:

1. **Gates not inherited** - Subtasks don't inherit quality gates from parent
2. **Type label not transformed** - `type:story` copied to subtasks instead of `type:task`

## Current Behavior (Tested)

### Test Setup
```bash
# Created story with:
- Labels: type:story, epic:security, priority:high
- Gates: tests, review
- Priority: high

# Breakdown into 3 subtasks
jit issue breakdown $STORY_ID \
  --subtask "Implement login endpoint" \
  --subtask "Add password hashing" \
  --subtask "Create session management"
```

### Observed Results

**What gets inherited:**
- ✅ Priority: copied from parent
- ✅ Labels: ALL copied (including `type:story`)
- ✅ Dependencies: parent's deps become subtask deps
- ✅ Dependency graph: parent depends on subtasks, original deps removed

**What doesn't get inherited:**
- ❌ Gates: `gates_required` is empty array for all subtasks

### Code Analysis

**Location:** `crates/jit/src/commands/breakdown.rs`

```rust
pub fn breakdown_issue(
    &self,
    parent_id: &str,
    subtasks: Vec<(String, String)>,
) -> Result<Vec<String>> {
    let parent = self.storage.load_issue(&full_parent_id)?;
    
    for (title, desc) in subtasks {
        let subtask_id = self.create_issue(
            title,
            desc,
            parent.priority,
            vec![],                    // ← Empty gates array
            parent.labels.clone(),     // ← ALL labels copied verbatim
        )?;
        subtask_ids.push(subtask_id);
    }
    // ... dependency management
}
```

## Problems Identified

### Problem 1: No Gate Inheritance

**Impact:** Medium-High
- Every subtask needs manual `jit gate add` calls
- Easy to forget gates, leading to inconsistent quality standards
- Tedious for large breakdowns (N subtasks × M gates = N×M operations)

**Example:**
```bash
# Create story with 2 gates
jit issue create --title "Feature" --gate tests --gate review

# Break down into 5 subtasks
jit issue breakdown $STORY --subtask "A" --subtask "B" ... 

# Now need 10 manual gate operations!
jit gate add $SUBTASK1 tests review
jit gate add $SUBTASK2 tests review
jit gate add $SUBTASK3 tests review
jit gate add $SUBTASK4 tests review
jit gate add $SUBTASK5 tests review
```

**Why it matters:**
- Gates represent quality standards (tests, reviews, security checks)
- If parent requires gates, subtasks likely need same standards
- Manual addition is error-prone and doesn't scale

### Problem 2: Type Label Not Transformed

**Impact:** Medium
- Subtasks get `type:story` when they should be `type:task`
- Violates semantic hierarchy (stories contain tasks)
- Requires manual batch update after breakdown
- Confuses queries and reports

**Example:**
```bash
# Story has type:story
jit issue show $STORY --json | jq .labels
# ["type:story", "epic:auth"]

# After breakdown, subtasks also have type:story
jit issue show $SUBTASK1 --json | jq .labels
# ["type:story", "epic:auth"]  ← Wrong! Should be type:task

# Manual fix needed:
jit issue update --filter "labels.story:auth-login" --label type:task
# But this adds type:task, doesn't remove type:story!
```

**Why it matters:**
- Semantic correctness for reporting and queries
- `jit query all --label "type:task"` won't find these issues
- Label-based automation may behave incorrectly
- Hierarchy validation may flag inconsistencies

## Additional Observations

### Labels Not Shown in Human Output

**Issue:** `jit issue show` doesn't display labels in human-readable format

```bash
jit issue show $SUBTASK1
# Output shows: Title, Description, State, Priority, Assignee, Dependencies, Gates
# Missing: Labels!
```

This is a separate display issue but makes the type:story problem harder to notice.

## Root Causes

1. **Simplistic label copying** - No transformation logic based on hierarchy
2. **No gate inheritance logic** - Intentionally left empty (vec![])
3. **No configuration** - No flags to control inheritance behavior

## Proposed Solutions

### Solution 1: Inherit Gates by Default (Recommended)

**Rationale:**
- Quality standards apply to work at any level
- Opt-out easier than opt-in (can remove gates if not needed)
- Matches user expectations ("if story needs tests, tasks need tests")

**Implementation:**
```rust
let subtask_id = self.create_issue(
    title,
    desc,
    parent.priority,
    parent.gates_required.clone(),  // ← Inherit gates
    parent.labels.clone(),
)?;
```

**Breaking change?** Yes, but acceptable pre-1.0

### Solution 2: Transform Type Labels Based on Hierarchy

**Approach A: Simple downgrade (story → task)**
```rust
let mut inherited_labels = parent.labels.clone();

// If parent has type:story, replace with type:task for subtasks
if inherited_labels.iter().any(|l| l == "type:story") {
    inherited_labels.retain(|l| !l.starts_with("type:"));
    inherited_labels.push("type:task".to_string());
}
```

**Approach B: Hierarchical transformation (config-driven)**
```rust
// Use hierarchy config to determine child type
let child_type = match get_type_label(&parent.labels) {
    Some("type:epic") => "type:story",
    Some("type:story") => "type:task",
    Some("type:milestone") => "type:epic",
    _ => None,  // No transformation
};

if let Some(child_type) = child_type {
    inherited_labels.retain(|l| !l.starts_with("type:"));
    inherited_labels.push(child_type.to_string());
}
```

**Recommendation:** Start with Approach A (simple), consider Approach B later

### Solution 3: Add CLI Flags for Control (Future)

```bash
# Opt-out of gate inheritance
jit issue breakdown $PARENT --no-inherit-gates ...

# Override type transformation
jit issue breakdown $PARENT --child-type task ...

# Selective label inheritance
jit issue breakdown $PARENT --inherit-label epic:* --inherit-label milestone:* ...
```

**Recommendation:** Not for initial fix, but consider for v1.0+

## Implementation Plan

### Phase 1: Fix Gate Inheritance (Quick Win)

**Changes:**
1. Update `breakdown_issue()` to pass `parent.gates_required.clone()`
2. Update CLI help text to document inheritance behavior
3. Add test for gate inheritance
4. Update documentation

**Effort:** ~1 hour
**Risk:** Low (purely additive functionality)
**Breaking:** Yes, but pre-1.0

### Phase 2: Fix Type Label Transformation

**Changes:**
1. Add helper function `transform_type_label()`
2. Apply transformation in `breakdown_issue()`
3. Add tests for story→task, epic→story
4. Update documentation

**Effort:** ~2 hours
**Risk:** Low (well-defined transformation)
**Breaking:** Yes, but pre-1.0

### Phase 3: Fix Label Display (Nice-to-have)

**Changes:**
1. Update `jit issue show` human output to include labels
2. Decide on format (inline? separate section?)
3. Update tests

**Effort:** ~1 hour
**Risk:** Low
**Breaking:** No (output-only change)

## Testing Strategy

### Unit Tests
```rust
#[test]
fn test_breakdown_inherits_gates() {
    // Parent with gates
    // Breakdown
    // Assert subtasks have same gates
}

#[test]
fn test_breakdown_transforms_story_to_task() {
    // Parent with type:story
    // Breakdown
    // Assert subtasks have type:task
}

#[test]
fn test_breakdown_preserves_other_labels() {
    // Parent with epic:*, milestone:*, component:*
    // Breakdown
    // Assert all non-type labels preserved
}
```

### Integration Tests
```rust
#[test]
fn test_breakdown_e2e_with_gates() {
    // Create story with gates
    // Breakdown
    // Verify subtasks inherit gates
    // Complete subtask (gates should block)
}
```

## Backward Compatibility

**Breaking changes:**
1. Subtasks will now have gates (previously empty)
2. Subtasks will have different type labels (task instead of story)

**Migration path:** None needed (pre-1.0)

**For 1.0+:** Would need deprecation warnings and opt-in flags

## Success Metrics

After implementation:
- ✅ No manual gate operations needed after breakdown
- ✅ Subtasks have correct type labels
- ✅ Tests validate inheritance behavior
- ✅ Documentation reflects new behavior

## Recommendation

**Implement Phase 1 & 2 together:**
- Both are small, well-defined changes
- Fix the two main pain points
- Can be done in one PR
- Minimal risk

**Defer Phase 3:**
- Label display is a separate concern
- Can be addressed later
- Not blocking for breakdown ergonomics
