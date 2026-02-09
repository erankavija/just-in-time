# Implementation Plan: Fix `jit issue breakdown` Ergonomics

## Problem Statement

Items 10 & 11 from issue 32f804f1 identify two ergonomics issues:
1. **Gates not inherited** - Manual `jit gate add` needed for every subtask
2. **Type labels not transformed** - Subtasks get `type:story` instead of `type:task`

## Proposed Solution

Fix both issues in a single PR with TDD approach:
- Inherit gates from parent by default
- Transform `type:story` → `type:task` during breakdown

## Implementation Tasks

- [ ] **Task 1: Write failing tests (TDD)**
  - [ ] Test: `test_breakdown_inherits_gates_from_parent`
  - [ ] Test: `test_breakdown_transforms_story_type_to_task`
  - [ ] Test: `test_breakdown_preserves_non_type_labels`
  - [ ] Test: `test_breakdown_transforms_epic_type_to_story`
  
- [ ] **Task 2: Implement gate inheritance**
  - [ ] Update `breakdown_issue()` to pass `parent.gates_required.clone()`
  - [ ] Verify tests pass
  
- [ ] **Task 3: Implement type label transformation**
  - [ ] Add helper function `transform_type_label(labels: &[String]) -> Vec<String>`
  - [ ] Logic: `type:story` → `type:task`, `type:epic` → `type:story`
  - [ ] Preserve all other labels unchanged
  - [ ] Apply in `breakdown_issue()` before creating subtasks
  - [ ] Verify tests pass
  
- [ ] **Task 4: Update documentation**
  - [ ] Update CLI help text for `jit issue breakdown`
  - [ ] Update any how-to guides that mention breakdown
  - [ ] Add note about automatic inheritance/transformation
  
- [ ] **Task 5: Quality gates**
  - [ ] All tests pass
  - [ ] Clippy clean
  - [ ] Format correct

## Technical Design

### Gate Inheritance (Simple)

**File:** `crates/jit/src/commands/breakdown.rs`

```rust
// Before:
let subtask_id = self.create_issue(
    title,
    desc,
    parent.priority,
    vec![],                    // ← Empty
    parent.labels.clone(),
)?;

// After:
let subtask_id = self.create_issue(
    title,
    desc,
    parent.priority,
    parent.gates_required.clone(),  // ← Inherit gates
    transformed_labels,             // ← Transformed labels
)?;
```

### Type Label Transformation

**File:** `crates/jit/src/commands/breakdown.rs`

```rust
/// Transform type labels for child issues in breakdown
fn transform_type_label(labels: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut found_type = false;
    
    for label in labels {
        if label.starts_with("type:") {
            found_type = true;
            // Transform based on parent type
            match label.as_str() {
                "type:epic" => result.push("type:story".to_string()),
                "type:story" => result.push("type:task".to_string()),
                // Keep milestone, bug, task as-is
                other => result.push(other.to_string()),
            }
        } else {
            // Preserve all non-type labels
            result.push(label.clone());
        }
    }
    
    // If no type label found, don't add one
    result
}
```

**Usage:**
```rust
for (title, desc) in subtasks {
    let transformed_labels = transform_type_label(&parent.labels);
    let subtask_id = self.create_issue(
        title,
        desc,
        parent.priority,
        parent.gates_required.clone(),
        transformed_labels,
    )?;
    subtask_ids.push(subtask_id);
}
```

## Testing Strategy

### Test Scenarios

**Scenario 1: Story breakdown with gates**
```rust
// Parent: type:story with tests+review gates
// Expected: Subtasks have type:task with tests+review gates
```

**Scenario 2: Epic breakdown**
```rust
// Parent: type:epic
// Expected: Subtasks have type:story
```

**Scenario 3: Preserve other labels**
```rust
// Parent: type:story, epic:auth, milestone:v1, component:backend
// Expected: Subtasks have type:task, epic:auth, milestone:v1, component:backend
```

**Scenario 4: No type label**
```rust
// Parent: No type label (just component:backend)
// Expected: Subtasks have same labels (no type added)
```

## Breaking Changes

**Yes, but acceptable pre-1.0:**
1. Subtasks now inherit gates (previously empty)
2. Type labels are transformed (previously copied verbatim)

**User impact:**
- Positive: Less manual work after breakdown
- Positive: Correct semantic hierarchy
- Neutral: Pre-1.0, no migration needed

## Estimated Effort

- Task 1 (Tests): 1 hour
- Task 2 (Gates): 15 minutes
- Task 3 (Labels): 45 minutes
- Task 4 (Docs): 30 minutes
- Task 5 (QA): 15 minutes

**Total: ~3 hours**

## Success Criteria

- [x] Confirm current behavior through testing
- [ ] All new tests pass
- [ ] Gate inheritance works correctly
- [ ] Type transformation works correctly
- [ ] Other labels preserved
- [ ] All existing tests still pass
- [ ] Clippy clean
- [ ] Documentation updated
- [ ] Commit with clear message
