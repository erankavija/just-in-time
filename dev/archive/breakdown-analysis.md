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

### Solution 1: Use Gate Presets Instead of Inheritance

**Key insight:** Gate templates/presets are now available (issue 56b7e503 - completed)

**Why not inherit gates by default:**
- Parent gates may not apply to child work (different scope/granularity)
- Epic with "security-audit" gate doesn't mean each task needs full security audit
- Story with "stakeholder-review" gate may not need review on individual tasks
- Different levels of work often need different quality standards

**Better approach: Use gate presets**

```bash
# Break down with explicit child type
jit issue breakdown $STORY --child-type task \
  --subtask "Login endpoint" \
  --subtask "Password hashing"

# Apply appropriate gate preset to all subtasks
jit gate preset apply rust-tdd $SUBTASK1 $SUBTASK2 ...

# Or apply different presets per subtask
jit gate preset apply rust-tdd $SUBTASK1
jit gate preset apply minimal $SUBTASK2    # Simpler subtask
```

**Available presets:**
- `rust-tdd`: TDD workflow (5 gates: tdd-reminder, tests, clippy, fmt, code-review)
- `minimal`: Minimal workflow (1 gate: code-review)
- Custom presets: Users can define in `.jit/config/gate-presets/`

**Advantages:**
- ✅ More flexible than simple inheritance
- ✅ Better semantic match for different work types
- ✅ Leverages existing gate preset infrastructure
- ✅ User retains full control
- ✅ Can mix presets for different subtasks
- ✅ Can customize with --except, --no-precheck, etc.

### Solution 2: Optional --gate-preset Flag

**If we want to make it more ergonomic:**

```bash
# Break down and apply preset in one command
jit issue breakdown $STORY --child-type task \
  --gate-preset rust-tdd \
  --subtask "Login endpoint" \
  --subtask "Password hashing"

# Or use parent's gates (opt-in)
jit issue breakdown $STORY --child-type task \
  --inherit-gates \
  --subtask "Login endpoint"

# Or no gates (default)
jit issue breakdown $STORY --child-type task \
  --subtask "Login endpoint"
```

**Implementation:**
```rust
pub fn breakdown_issue(
    &self,
    parent_id: &str,
    child_type: &str,
    subtasks: Vec<(String, String)>,
    gate_option: GateOption,  // ← New parameter
) -> Result<Vec<String>> {
    // ... create subtasks with empty gates by default ...
    
    // Apply gate option after creation
    match gate_option {
        GateOption::None => {},  // Default - no gates
        GateOption::Inherit => {
            // Copy parent's gates to each subtask
            for subtask_id in &subtask_ids {
                for gate in &parent.gates_required {
                    self.add_gate_to_issue(subtask_id, gate)?;
                }
            }
        }
        GateOption::Preset(preset_name) => {
            // Apply preset to each subtask
            let preset_manager = PresetManager::new(self.storage.root());
            let preset = preset_manager.get_preset(&preset_name)?;
            for subtask_id in &subtask_ids {
                preset_manager.apply_preset_to_issue(subtask_id, preset)?;
            }
        }
    }
    
    Ok(subtask_ids)
}
```

**CLI flags:**
```rust
Breakdown {
    parent_id: String,
    
    #[arg(long, required = true)]
    child_type: String,
    
    #[arg(long)]
    subtask: Vec<String>,
    
    // Gate options (mutually exclusive)
    #[arg(long, conflicts_with_all = ["inherit_gates"])]
    gate_preset: Option<String>,
    
    #[arg(long, conflicts_with_all = ["gate_preset"])]
    inherit_gates: bool,
    
    // ... rest unchanged
}
```

**Default behavior:** No gates on subtasks (user applies separately)

### Recommendation

**Implement complete solution (not phased):**
- Add required `--child-type` argument
- Add optional `--gate-preset <name>` flag
- Add optional `--inherit-gates` flag (conflicts with --gate-preset)
- Default: no gates (user must choose explicitly)

**This provides:**
- ✅ Required explicit child type
- ✅ Optional preset application for convenience
- ✅ Optional inheritance for edge cases
- ✅ Clear default (no gates = explicit choice required)

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

### Complete Implementation (All Features)

**Changes:**
1. Add `--child-type` as required CLI argument
2. Add `--gate-preset <name>` as optional CLI argument
3. Add `--inherit-gates` as optional CLI flag
4. Make --gate-preset and --inherit-gates mutually exclusive
5. Update `breakdown_issue()` to handle gate options
6. Replace parent's `type:` label with `type:{child_type}`
7. Integrate with PresetManager for preset application
8. Add comprehensive tests
9. Update documentation

**Effort:** ~3 hours
**Risk:** Medium (integration with preset system)
**Breaking:** Yes - new required argument (--child-type), but pre-1.0

**Result - Three usage patterns:**
```bash
# 1. No gates (default) - apply manually later
jit issue breakdown $STORY --child-type task \
  --subtask "A" --subtask "B"

# 2. Apply preset to all subtasks
jit issue breakdown $STORY --child-type task \
  --gate-preset rust-tdd \
  --subtask "A" --subtask "B"

# 3. Inherit parent's gates
jit issue breakdown $STORY --child-type task \
  --inherit-gates \
  --subtask "A" --subtask "B"
```

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
