# Implementation Plan: Fix `jit issue breakdown` Ergonomics

## Problem Statement

Items 10 & 11 from issue 32f804f1 identify two ergonomics issues:
1. **Gates not inherited** - Manual `jit gate add` needed for every subtask
2. **Type labels not transformed** - Subtasks get `type:story` instead of `type:task`

## Proposed Solution

Complete implementation with all features integrated (not phased):
- Add required `--child-type` argument (explicit type transformation)
- Three gate handling patterns: none (default), `--gate-preset`, `--inherit-gates`
- Leverage existing PresetManager for quality standards
- No automatic gate inheritance (different work levels need different gates)

## Implementation Tasks

- [ ] **Task 1: Write failing tests (TDD)**
  - [ ] Test: `test_breakdown_replaces_type_label_with_child_type`
  - [ ] Test: `test_breakdown_preserves_non_type_labels`
  - [ ] Test: `test_breakdown_works_with_custom_types`
  - [ ] Test: `test_breakdown_no_gates_by_default`
  - [ ] Test: `test_breakdown_with_gate_preset`
  - [ ] Test: `test_breakdown_with_inherit_gates`
  - [ ] Test: `test_breakdown_gate_preset_and_inherit_mutually_exclusive`
  
- [ ] **Task 2: Add CLI arguments**
  - [ ] Add `child_type: String` as required argument
  - [ ] Add `gate_preset: Option<String>` as optional argument
  - [ ] Add `inherit_gates: bool` as optional flag
  - [ ] Make gate_preset and inherit_gates mutually exclusive
  - [ ] Update help text with examples
  
- [ ] **Task 3: Implement breakdown with gate options**
  - [ ] Update `breakdown_issue()` signature to accept gate options
  - [ ] Implement type label replacement
  - [ ] Implement GateOption enum (None, Preset, Inherit)
  - [ ] Integration with PresetManager for preset application
  - [ ] Implement gate inheritance logic
  - [ ] Verify tests pass
  
- [ ] **Task 4: Update documentation**
  - [ ] Update CLI help text with all three patterns
  - [ ] Add workflow examples for each pattern
  - [ ] Update any how-to guides that mention breakdown
  - [ ] Document gate handling strategy
  
- [ ] **Task 5: Quality gates**
  - [ ] All tests pass
  - [ ] Clippy clean
  - [ ] Format correct

## Gate Handling Strategy

**Three patterns (all first-class):**

**1. No gates (default - explicit later):**
```bash
jit issue breakdown $STORY --child-type task \
  --subtask "Login" --subtask "Password"

# User decides per subtask
jit gate preset apply rust-tdd $SUBTASK1
jit gate preset apply minimal $SUBTASK2
```

**2. Apply preset to all subtasks:**
```bash
jit issue breakdown $STORY --child-type task \
  --gate-preset rust-tdd \
  --subtask "Login" --subtask "Password"

# All subtasks get rust-tdd gates (5 gates)
```

**3. Inherit from parent:**
```bash
jit issue breakdown $STORY --child-type task \
  --inherit-gates \
  --subtask "Login" --subtask "Password"

# All subtasks get parent's exact gates
```

**Implementation notes:**
- Default is no gates (safest, most flexible)
- --gate-preset leverages existing PresetManager
- --inherit-gates is simple cloning
- Flags are mutually exclusive (enforced by clap)

## Technical Design

### Gate Handling (Three Patterns)

**File:** `crates/jit/src/commands/breakdown.rs`

```rust
pub enum GateOption {
    None,                    // Default - no gates
    Preset(String),         // Apply preset to all subtasks
    Inherit,                // Copy parent's gates
}

// In breakdown_issue():
for (title, desc) in subtasks {
    let subtask_id = self.create_issue(
        title,
        desc,
        parent.priority,
        vec![],                // ← Always start with no gates
        transformed_labels,
    )?;
    
    // Apply gate option after creation
    match &gate_option {
        GateOption::None => {},
        GateOption::Preset(name) => {
            preset_manager.apply_preset(&subtask_id, name)?;
        }
        GateOption::Inherit => {
            for gate in &parent.gates_required {
                self.add_gate_to_issue(&subtask_id, gate)?;
            }
        }
    }
    
    subtask_ids.push(subtask_id);
}
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
    
    /// Apply gate preset to all subtasks
    #[arg(long, conflicts_with = "inherit_gates")]
    gate_preset: Option<String>,
    
    /// Inherit parent's gates to all subtasks
    #[arg(long, conflicts_with = "gate_preset")]
    inherit_gates: bool,
    
    // ... rest unchanged
}
```

**File:** `crates/jit/src/commands/breakdown.rs`

```rust
pub fn breakdown_issue(
    &self,
    parent_id: &str,
    child_type: &str,
    subtasks: Vec<(String, String)>,
    gate_option: GateOption,  // ← New parameter for gate handling
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
            vec![],           // ← No gates initially
            child_labels,     // ← Type replaced
        )?;
        
        // Apply gate option after creation
        match &gate_option {
            GateOption::None => {},
            GateOption::Preset(name) => {
                let preset_manager = PresetManager::new(self.storage.root());
                preset_manager.apply_preset_to_issue(&subtask_id, name)?;
            }
            GateOption::Inherit => {
                for gate in &parent.gates_required {
                    self.add_gate_to_issue(&subtask_id, gate)?;
                }
            }
        }
        
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

**Scenario 1: No gates (default)**
```rust
// Parent: type:story with tests+review gates
// Command: --child-type task
// Expected: Subtasks have type:task with NO gates
```

**Scenario 2: With gate preset**
```rust
// Parent: type:story
// Command: --child-type task --gate-preset rust-tdd
// Expected: Subtasks have type:task with rust-tdd gates (5 gates)
```

**Scenario 3: With inherit gates**
```rust
// Parent: type:story with [tests, review] gates
// Command: --child-type task --inherit-gates
// Expected: Subtasks have type:task with [tests, review] gates
```

**Scenario 4: Preserve other labels**
```rust
// Parent: type:story, epic:auth, milestone:v1, component:backend
// Command: --child-type task
// Expected: Subtasks have type:task, epic:auth, milestone:v1, component:backend
```

**Scenario 5: Custom types**
```rust
// Parent: type:feature
// Command: --child-type requirement
// Expected: Subtasks have type:requirement
```

**Scenario 6: Mutual exclusivity**
```rust
// Command: --child-type task --gate-preset rust-tdd --inherit-gates
// Expected: Error - flags are mutually exclusive
```

## Breaking Changes

**Yes, but acceptable pre-1.0:**
1. New required argument: `--child-type`
2. Type labels are transformed (previously copied verbatim)

**User impact:**
- Positive: Explicit type control (no magic transformations)
- Positive: Optional gate convenience via --gate-preset or --inherit-gates
- Positive: Clear default (no gates unless requested)
- Breaking: Must specify --child-type (was previously implicit)
- Neutral: Pre-1.0, no migration needed

## Estimated Effort

- Task 1 (Tests): 1.5 hours (7 tests)
- Task 2 (CLI): 30 minutes
- Task 3 (Implementation): 1.5 hours (preset integration)
- Task 4 (Docs): 30 minutes
- Task 5 (QA): 15 minutes

**Total: ~4 hours**

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
