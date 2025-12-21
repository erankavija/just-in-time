# Implementation Plan: Bulk Operations for Gates, Labels, and Dependencies

## Problem Statement

Currently, adding multiple gates, labels, or dependencies requires multiple command invocations:

**Current (tedious)**:
```bash
jit gate add $ISSUE tests
jit gate add $ISSUE clippy
jit gate add $ISSUE fmt
jit gate add $ISSUE code-review
jit gate add $ISSUE tdd-reminder
```

**Desired (efficient)**:
```bash
jit gate add $ISSUE tests clippy fmt code-review tdd-reminder
# or
jit issue update $ISSUE --add-gate tests --add-gate clippy --add-gate fmt
```

This wastes time, creates many transactions, and makes scripts verbose.

## Current State

### Commands Affected
1. **Gate operations**: `jit gate add` - one gate at a time
2. **Label operations**: `jit issue update --label` - one label at a time
3. **Dependency operations**: No bulk support

### CLI Definitions
- `jit gate add <issue> <gate>` - single gate
- `jit issue update --label <label>` - repeatable but requires multiple flags
- `jit issue create --label <label>` - repeatable (works well!)
- `jit dep add <from> <to>` - single dependency

## Goal

Support bulk operations for common multi-value operations:

1. **Multiple gates in one command**
2. **Multiple labels in one command** (already works, verify consistency)
3. **Multiple dependencies in one command**
4. **Atomic transactions** (all succeed or all fail)

## Task Breakdown

### 1. Update Gate Commands

**File**: `crates/jit/src/cli.rs`

Change gate add to accept multiple gates:

```rust
#[derive(Subcommand)]
pub enum GateCommands {
    /// Add gate requirement(s) to an issue
    Add {
        /// Issue ID
        id: String,
        
        /// Gate key(s) from registry (e.g., 'tests', 'code-review')
        /// Can specify multiple: jit gate add <issue> gate1 gate2 gate3
        #[arg(required = true)]
        gate_keys: Vec<String>,
        
        #[arg(long)]
        json: bool,
    },
    // ... rest
}
```

**File**: `crates/jit/src/commands/gate.rs`

Update implementation:

```rust
pub fn add_gates_to_issue(&self, issue_id: &str, gate_keys: &[String]) -> Result<GateAddResult> {
    let registry = self.storage.load_gate_registry()?;
    let mut issue = self.storage.load_issue(issue_id)?;
    
    let mut added = Vec::new();
    let mut already_exist = Vec::new();
    let mut not_found = Vec::new();
    
    for gate_key in gate_keys {
        // Validate gate exists
        if !registry.gates.contains_key(gate_key) {
            not_found.push(gate_key.clone());
            continue;
        }
        
        // Check if already required
        if issue.gates_required.contains(gate_key) {
            already_exist.push(gate_key.clone());
            continue;
        }
        
        // Add it
        issue.gates_required.push(gate_key.clone());
        
        // Initialize status
        if !issue.gates_status.contains_key(gate_key) {
            issue.gates_status.insert(
                gate_key.clone(),
                GateState {
                    status: GateStatus::Pending,
                    updated_by: None,
                    updated_at: Utc::now(),
                },
            );
        }
        
        added.push(gate_key.clone());
    }
    
    // Atomic: only save if no errors
    if !not_found.is_empty() {
        return Err(anyhow!("Gates not found in registry: {}", not_found.join(", ")));
    }
    
    // Save issue
    self.storage.save_issue(&issue)?;
    
    // Log events
    for gate_key in &added {
        let event = Event::new_gate_added(&issue.id, gate_key);
        self.storage.append_event(&event)?;
    }
    
    Ok(GateAddResult {
        added,
        already_exist,
    })
}

pub struct GateAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
}
```

**File**: `crates/jit/src/main.rs`

Update command handler:

```rust
Commands::Gate(GateCommands::Add { id, gate_keys, json }) => {
    let result = executor.add_gates_to_issue(&id, &gate_keys)?;
    
    if json {
        let response = JsonResponse::success(result, "gate add");
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        if !result.added.is_empty() {
            println!("Added {} gate(s) to issue {}:", result.added.len(), id);
            for gate in &result.added {
                println!("  - {}", gate);
            }
        }
        if !result.already_exist.is_empty() {
            println!("Already required ({}):", result.already_exist.len());
            for gate in &result.already_exist {
                println!("  - {}", gate);
            }
        }
    }
    ExitCode::SUCCESS
}
```

### 2. Add Bulk Remove for Gates

**CLI**:
```rust
/// Remove gate requirement(s) from an issue
Remove {
    /// Issue ID
    id: String,
    
    /// Gate key(s) to remove
    #[arg(required = true)]
    gate_keys: Vec<String>,
    
    #[arg(long)]
    json: bool,
},
```

**Implementation**:
```rust
pub fn remove_gates_from_issue(&self, issue_id: &str, gate_keys: &[String]) -> Result<GateRemoveResult> {
    let mut issue = self.storage.load_issue(issue_id)?;
    
    let mut removed = Vec::new();
    let mut not_found = Vec::new();
    
    for gate_key in gate_keys {
        if issue.gates_required.contains(gate_key) {
            issue.gates_required.retain(|g| g != gate_key);
            issue.gates_status.remove(gate_key);
            removed.push(gate_key.clone());
        } else {
            not_found.push(gate_key.clone());
        }
    }
    
    self.storage.save_issue(&issue)?;
    
    // Log events
    for gate_key in &removed {
        let event = Event::new_gate_removed(&issue.id, gate_key);
        self.storage.append_event(&event)?;
    }
    
    Ok(GateRemoveResult { removed, not_found })
}
```

### 3. Add Bulk Dependencies

**CLI**:
```rust
#[derive(Subcommand)]
pub enum DepCommands {
    /// Add dependencies (issue depends on multiple others)
    Add {
        /// Issue that will depend on others (FROM)
        from_id: String,
        
        /// Issues that must be done first (TO)
        /// Can specify multiple: jit dep add FROM to1 to2 to3
        #[arg(required = true)]
        to_ids: Vec<String>,
        
        #[arg(long)]
        json: bool,
    },
    
    /// Remove dependencies
    Rm {
        /// Issue to modify (FROM)
        from_id: String,
        
        /// Dependencies to remove (TO)
        #[arg(required = true)]
        to_ids: Vec<String>,
        
        #[arg(long)]
        json: bool,
    },
}
```

**Implementation**:
```rust
pub fn add_dependencies(&self, from_id: &str, to_ids: &[String]) -> Result<DepAddResult> {
    let mut results = DepAddResults::default();
    
    for to_id in to_ids {
        match self.add_dependency(from_id, to_id) {
            Ok(DependencyAddResult::Added) => {
                results.added.push(to_id.clone());
            }
            Ok(DependencyAddResult::AlreadyExists) => {
                results.already_exist.push(to_id.clone());
            }
            Ok(DependencyAddResult::Skipped { reason }) => {
                results.skipped.push((to_id.clone(), reason));
            }
            Err(e) => {
                // Cycle detection or other error
                results.errors.push((to_id.clone(), e.to_string()));
            }
        }
    }
    
    // If all failed, return error
    if results.added.is_empty() && !results.errors.is_empty() {
        return Err(anyhow!("All dependencies failed: {}", 
            results.errors.iter()
                .map(|(id, err)| format!("{}: {}", id, err))
                .collect::<Vec<_>>()
                .join("; ")));
    }
    
    Ok(results)
}

#[derive(Default, Serialize)]
pub struct DepAddResults {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>,  // (id, reason)
    pub errors: Vec<(String, String)>,    // (id, error)
}
```

### 4. Verify Label Bulk Operations

Labels already support multiple values via `--label` flag (repeatable):

```bash
jit issue create --title "Test" --label "type:task" --label "epic:auth" --label "milestone:v1"
```

**Verify this works correctly**:
- Multiple `--label` flags
- Comma-separated values with `value_delimiter = ','`
- Document which syntax to use

**Update help text** to clarify:
```rust
/// Labels (format: namespace:value)
/// Can specify multiple: --label type:task --label epic:auth
/// Or comma-separated: --label type:task,epic:auth
#[arg(short, long, value_delimiter = ',')]
label: Vec<String>,
```

### 5. Add Tests

**File**: `crates/jit/tests/bulk_operations_tests.rs`

```rust
#[test]
fn test_add_multiple_gates_at_once() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    h.executor.add_gates_to_issue(&issue, &[
        "tests".to_string(),
        "clippy".to_string(),
        "fmt".to_string(),
    ]).unwrap();
    
    let loaded = h.storage.load_issue(&issue).unwrap();
    assert_eq!(loaded.gates_required.len(), 3);
    assert!(loaded.gates_required.contains(&"tests".to_string()));
    assert!(loaded.gates_required.contains(&"clippy".to_string()));
    assert!(loaded.gates_required.contains(&"fmt".to_string()));
}

#[test]
fn test_add_gates_some_already_exist() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    // Add one gate first
    h.executor.add_gates_to_issue(&issue, &["tests".to_string()]).unwrap();
    
    // Try to add tests again plus new ones
    let result = h.executor.add_gates_to_issue(&issue, &[
        "tests".to_string(),
        "clippy".to_string(),
    ]).unwrap();
    
    assert_eq!(result.added.len(), 1);  // Only clippy
    assert_eq!(result.already_exist.len(), 1);  // tests
}

#[test]
fn test_add_gates_atomic_failure() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    // Try to add gates, one doesn't exist
    let result = h.executor.add_gates_to_issue(&issue, &[
        "tests".to_string(),
        "nonexistent".to_string(),
    ]);
    
    // Should fail entirely
    assert!(result.is_err());
    
    // No gates should have been added
    let loaded = h.storage.load_issue(&issue).unwrap();
    assert_eq!(loaded.gates_required.len(), 0);
}

#[test]
fn test_remove_multiple_gates() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    // Add multiple gates
    h.executor.add_gates_to_issue(&issue, &[
        "tests".to_string(),
        "clippy".to_string(),
        "fmt".to_string(),
    ]).unwrap();
    
    // Remove two
    h.executor.remove_gates_from_issue(&issue, &[
        "tests".to_string(),
        "clippy".to_string(),
    ]).unwrap();
    
    let loaded = h.storage.load_issue(&issue).unwrap();
    assert_eq!(loaded.gates_required.len(), 1);
    assert!(loaded.gates_required.contains(&"fmt".to_string()));
}

#[test]
fn test_add_multiple_dependencies() {
    let h = TestHarness::new();
    let parent = h.create_issue("Parent");
    let child1 = h.create_issue("Child 1");
    let child2 = h.create_issue("Child 2");
    let child3 = h.create_issue("Child 3");
    
    let result = h.executor.add_dependencies(&parent, &[
        child1.clone(),
        child2.clone(),
        child3.clone(),
    ]).unwrap();
    
    assert_eq!(result.added.len(), 3);
    
    let loaded = h.storage.load_issue(&parent).unwrap();
    assert_eq!(loaded.dependencies.len(), 3);
}

#[test]
fn test_add_dependencies_partial_failure() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    
    // Create A → B (already exists)
    h.executor.add_dependency(&a, &b).unwrap();
    
    // Try to add A → B (exists), A → C (new), A → nonexistent (error)
    let result = h.executor.add_dependencies(&a, &[
        b.clone(),
        c.clone(),
        "nonexistent".to_string(),
    ]).unwrap();
    
    assert_eq!(result.added.len(), 1);  // C
    assert_eq!(result.already_exist.len(), 1);  // B
    assert_eq!(result.errors.len(), 1);  // nonexistent
}

#[test]
fn test_bulk_labels_via_cli() {
    let h = TestHarness::new();
    
    let output = h.run_command(&[
        "issue", "create",
        "--title", "Test",
        "--label", "type:task,epic:auth,milestone:v1.0",
    ]);
    
    let issue_id = h.parse_issue_id(&output);
    let issue = h.storage.load_issue(&issue_id).unwrap();
    
    assert_eq!(issue.labels.len(), 3);
    assert!(issue.labels.contains(&"type:task".to_string()));
    assert!(issue.labels.contains(&"epic:auth".to_string()));
    assert!(issue.labels.contains(&"milestone:v1.0".to_string()));
}
```

### 6. Update CLI Help and Documentation

**Help text examples**:
```bash
$ jit gate add --help
Add gate requirement(s) to an issue

Usage: jit gate add <ID> <GATE_KEYS>...

Arguments:
  <ID>             Issue ID
  <GATE_KEYS>...   Gate key(s) from registry
                   
Examples:
  # Add single gate
  jit gate add abc123 tests
  
  # Add multiple gates
  jit gate add abc123 tests clippy fmt code-review

$ jit dep add --help
Add dependencies (issue depends on multiple others)

Usage: jit dep add <FROM_ID> <TO_IDS>...

Arguments:
  <FROM_ID>    Issue that will depend on others
  <TO_IDS>...  Issue(s) that must be done first
  
Examples:
  # Single dependency
  jit dep add epic-123 task-456
  
  # Multiple dependencies
  jit dep add epic-123 task-1 task-2 task-3
```

**README.md**:
```markdown
## Bulk Operations

### Multiple Gates
```bash
# Add multiple gates at once
jit gate add $ISSUE tests clippy fmt code-review

# Remove multiple gates
jit gate rm $ISSUE tests clippy
```

### Multiple Dependencies
```bash
# Epic depends on multiple tasks
jit dep add $EPIC $TASK1 $TASK2 $TASK3

# Remove multiple dependencies
jit dep rm $EPIC $TASK1 $TASK2
```

### Multiple Labels
```bash
# Comma-separated
jit issue create --title "Test" --label "type:task,epic:auth,milestone:v1"

# Multiple flags
jit issue create --title "Test" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1"
```
```

## Implementation Approach

1. **Update gate add/remove** CLI definitions (30 min)
2. **Implement bulk gate operations** (2 hours)
3. **Update dep add/remove** CLI definitions (30 min)
4. **Implement bulk dependency operations** (2 hours)
5. **Verify label bulk operations** work (30 min)
6. **Add comprehensive tests** (3 hours)
7. **Update help text and documentation** (1 hour)
8. **Full test suite** + clippy + fmt (1 hour)

**Total: 10-11 hours**

## Success Criteria

✅ `jit gate add <issue> gate1 gate2 gate3` works  
✅ `jit gate rm <issue> gate1 gate2` works  
✅ `jit dep add <from> to1 to2 to3` works  
✅ `jit dep rm <from> to1 to2` works  
✅ Labels work with comma-separated or multiple flags  
✅ Atomic operations (all succeed or all fail for gates)  
✅ Partial success for dependencies (some may fail)  
✅ Clear feedback on what succeeded/failed  
✅ Tests cover all bulk operations  
✅ Documentation updated  
✅ Zero clippy warnings  

## Benefits

- **Faster workflows**: One command instead of many
- **Fewer transactions**: Atomic operations
- **Better scripting**: Easier to add multiple items
- **Consistent UX**: Same pattern across commands
- **Reduced errors**: Fewer commands to type

## Edge Cases

1. **Empty list**: Error (require at least one item)
2. **Duplicates in input**: De-duplicate automatically
3. **Some items fail**: Report which succeeded/failed
4. **Cycle detection**: Stop before adding any deps
5. **Invalid gates**: Fail entire operation (atomic)

## Backward Compatibility

- Single-item operations still work (backward compatible)
- No breaking changes to existing commands
- Simply extends to accept multiple values

## Dependencies

No new dependencies required.

## Related Issues

- Improves bulk issue creation workflows
- Makes scripting easier
- Complements quiet mode for automation
