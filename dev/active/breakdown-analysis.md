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

### Solution 2: Require Explicit Child Type (Recommended)

**Problem with automatic transformation:**
- Type hierarchy is fully configurable (can have custom types)
- Multiple types can exist at same hierarchy level (bug, enhancement, task all at level 4)
- Breakdown doesn't always go to next level (could be same level, or skip levels)
- No way to know user's intent without explicit input

**Better approach: Make --child-type a required argument**

```bash
# Explicit and clear
jit issue breakdown $STORY --child-type task \
  --subtask "Login endpoint" \
  --subtask "Password hashing"

# Works with any hierarchy
jit issue breakdown $EPIC --child-type story ...
jit issue breakdown $MILESTONE --child-type epic ...

# Works for same-level breakdown
jit issue breakdown $BUG --child-type bug ...  # Break bug into sub-bugs

# Works with custom types
jit issue breakdown $FEATURE --child-type requirement ...
```

**Implementation:**
```rust
pub fn breakdown_issue(
    &self,
    parent_id: &str,
    child_type: &str,  // ← New required parameter
    subtasks: Vec<(String, String)>,
) -> Result<Vec<String>> {
    let parent = self.storage.load_issue(&full_parent_id)?;
    
    for (title, desc) in subtasks {
        // Replace type: label with user-specified type
        let mut child_labels = parent.labels.clone();
        child_labels.retain(|l| !l.starts_with("type:"));
        child_labels.push(format!("type:{}", child_type));
        
        let subtask_id = self.create_issue(
            title,
            desc,
            parent.priority,
            parent.gates_required.clone(),
            child_labels,
        )?;
        subtask_ids.push(subtask_id);
    }
    // ... rest unchanged
}
```

**CLI change:**
```rust
Breakdown {
    parent_id: String,
    
    /// Type for child issues (e.g., 'task', 'story', 'bug')
    #[arg(long, required = true)]
    child_type: String,
    
    #[arg(long)]
    subtask: Vec<String>,
    
    // ... rest unchanged
}
```

**Advantages:**
- ✅ Works with any hierarchy configuration
- ✅ Clear and explicit (no magic)
- ✅ Supports same-level breakdown
- ✅ Supports custom type hierarchies
- ✅ No complex transformation logic needed
- ✅ User retains full control

**Breaking change?** Yes - new required argument, but acceptable pre-1.0

## Implementation Plan

### Phase 1: Fix Gate Inheritance (Quick Win)

**Changes:**
1. Update `breakdown_issue()` to pass `parent.gates_required.clone()`
2. Update CLI help text to document inheritance behavior
3. Add test for gate inheritance
4. Update documentation

**Effort:** ~30 minutes
**Risk:** Low (purely additive functionality)
**Breaking:** Yes, but pre-1.0

### Phase 2: Add Required --child-type Argument

**Changes:**
1. Add `--child-type` as required CLI argument
2. Update `breakdown_issue()` signature to accept `child_type: &str`
3. Replace parent's `type:` label with `type:{child_type}`
4. Preserve all other labels
5. Add tests for type replacement
6. Update documentation and error messages

**Effort:** ~1.5 hours
**Risk:** Low (simple string replacement)
**Breaking:** Yes - new required argument, but pre-1.0

### Phase 3: Update Documentation

**Changes:**
1. Update CLI help text with --child-type examples
2. Update how-to guides that mention breakdown
3. Add note about explicit type specification
4. Document gate inheritance behavior

**Effort:** ~30 minutes

### Phase 4: Quality Gates

**Changes:**
1. All tests pass
2. Clippy clean
3. Format correct
4. Update dev/active documents with final solution

**Effort:** ~30 minutes

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
