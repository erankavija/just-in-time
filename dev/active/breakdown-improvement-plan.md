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
  - [ ] Test: `test_breakdown_replaces_type_label_with_child_type`
  - [ ] Test: `test_breakdown_preserves_non_type_labels`
  - [ ] Test: `test_breakdown_works_with_custom_types`
  
- [ ] **Task 2: Implement gate inheritance**
  - [ ] Update `breakdown_issue()` to pass `parent.gates_required.clone()`
  - [ ] Verify tests pass
  
- [ ] **Task 3: Add required --child-type argument**
  - [ ] Add `child_type: String` parameter to CLI Breakdown command
  - [ ] Mark as required with `#[arg(long, required = true)]`
  - [ ] Update `breakdown_issue()` signature: `child_type: &str`
  - [ ] Implement type label replacement logic
  - [ ] Verify tests pass
  
- [ ] **Task 4: Update documentation**
  - [ ] Update CLI help text for `jit issue breakdown`
  - [ ] Add examples showing --child-type usage
  - [ ] Update any how-to guides that mention breakdown
  - [ ] Document gate inheritance and type replacement
  
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

### Type Label Replacement (Required Argument)

**File:** `crates/jit/src/cli.rs`

```rust
Breakdown {
    /// Parent issue ID to break down
    parent_id: String,
    
    /// Type for child issues (e.g., 'task', 'story', 'bug')
    /// Must match a type: label value from your hierarchy
    #[arg(long, required = true)]
    child_type: String,
    
    /// Subtask titles (use multiple times)
    #[arg(long)]
    subtask: Vec<String>,
    
    // ... rest unchanged
}
```

**File:** `crates/jit/src/commands/breakdown.rs`

```rust
pub fn breakdown_issue(
    &self,
    parent_id: &str,
    child_type: &str,  // ← New required parameter
    subtasks: Vec<(String, String)>,
) -> Result<Vec<String>> {
    let full_parent_id = self.storage.resolve_issue_id(parent_id)?;
    let parent = self.storage.load_issue(&full_parent_id)?;
    let original_deps = parent.dependencies.clone();

    let mut subtask_ids = Vec::new();
    for (title, desc) in subtasks {
        // Replace parent's type: label with child type
        let mut child_labels = parent.labels.clone();
        child_labels.retain(|l| !l.starts_with("type:"));
        child_labels.push(format!("type:{}", child_type));
        
        let subtask_id = self.create_issue(
            title,
            desc,
            parent.priority,
            parent.gates_required.clone(),  // ← Inherit gates
            child_labels,                    // ← Type replaced
        )?;
        subtask_ids.push(subtask_id);
    }
    
    // ... rest unchanged (dependency management)
    
    Ok(subtask_ids)
}
```

**Usage:**
```bash
# Break down story into tasks
jit issue breakdown $STORY --child-type task \
  --subtask "Implement login endpoint" \
  --subtask "Add password hashing"

# Break down epic into stories
jit issue breakdown $EPIC --child-type story \
  --subtask "User authentication" \
  --subtask "Session management"

# Break down bug into sub-bugs (same level)
jit issue breakdown $BUG --child-type bug \
  --subtask "Reproduce issue" \
  --subtask "Fix root cause"

# Works with custom types
jit issue breakdown $FEATURE --child-type requirement \
  --subtask "REQ-001" \
  --subtask "REQ-002"
```

## Testing Strategy

### Test Scenarios

**Scenario 1: Story breakdown with gates and explicit child type**
```rust
// Parent: type:story with tests+review gates
// Command: --child-type task
// Expected: Subtasks have type:task with tests+review gates
```

**Scenario 2: Epic breakdown**
```rust
// Parent: type:epic
// Command: --child-type story
// Expected: Subtasks have type:story
```

**Scenario 3: Preserve other labels**
```rust
// Parent: type:story, epic:auth, milestone:v1, component:backend
// Command: --child-type task
// Expected: Subtasks have type:task, epic:auth, milestone:v1, component:backend
```

**Scenario 4: Same-level breakdown**
```rust
// Parent: type:bug with component:api
// Command: --child-type bug
// Expected: Subtasks have type:bug, component:api
```

**Scenario 5: Custom type**
```rust
// Parent: type:feature with custom labels
// Command: --child-type requirement
// Expected: Subtasks have type:requirement plus inherited labels
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
