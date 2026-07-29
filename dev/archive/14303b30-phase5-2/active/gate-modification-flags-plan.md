# Implementation Plan: Gate Modification Flags

**Issue:** 93d1caf7 - Add gate modification flags to issue update command  
**Epic:** phase5.2 (Agent-Friendly Additions)  
**Priority:** High  
**Estimated Effort:** 2-3 hours

## Objective

Add `--add-gate` and `--remove-gate` flags to `jit issue update` command, matching the existing pattern for labels (`--label`/`--remove-label`). This creates API consistency and reduces the number of commands needed for common workflows.

## Current State

**Issue create** has gate support:
```rust
#[arg(short, long, value_delimiter = ',')]
gate: Vec<String>,
```

**Issue update** lacks gate modification:
- Must use separate `jit gate add` and `jit gate remove` commands
- Inconsistent with label pattern (which has `--label`/`--remove-label`)

## Design Decisions

### 1. Flag Naming
- `--add-gate` / `--remove-gate` (explicit, clear intent)
- Matches `--remove-label` pattern
- Could also support `--gate` for adding (but prefer explicit)

### 2. Multiple Gates Support
Support both patterns for consistency:
```bash
# Multiple flags
jit issue update <id> --add-gate tests --add-gate clippy

# Comma-separated (if using value_delimiter)
jit issue update <id> --add-gate tests,clippy
```

### 3. Validation Strategy
- Validate gate keys exist in registry **before** modifying issue
- Return clear error for non-existent gates
- Allow duplicates to be idempotent (no-op if gate already exists)

### 4. Batch Mode Support
Gates work in both single-issue and batch mode:
```bash
# Single issue
jit issue update abc123 --add-gate tests

# Batch mode
jit issue update --filter "label:epic:auth" --add-gate code-review
```

## Implementation Plan

### Phase 1: Add CLI Flags (TDD - Red Phase)

**Files to modify:**
- `crates/jit/src/cli.rs` - Add flags to Update struct

**Changes:**
```rust
Update {
    // ... existing fields ...
    
    /// Add gate(s) to issue (gate keys from registry, repeatable)
    #[arg(long, value_delimiter = ',')]
    add_gate: Vec<String>,
    
    /// Remove gate(s) from issue (repeatable)
    #[arg(long, value_delimiter = ',')]
    remove_gate: Vec<String>,
    
    // ... rest of fields ...
}
```

**Tests to write first:**
- `tests/gate_modification_cli_tests.rs` (new file)
  1. `test_add_single_gate`
  2. `test_add_multiple_gates`
  3. `test_remove_gate`
  4. `test_add_and_remove_combined`
  5. `test_add_gate_invalid_key_error`
  6. `test_batch_add_gates`
  7. `test_gates_in_json_output`

**Expected:** All tests fail (command not implemented yet)

### Phase 2: Implement Single-Issue Mode (TDD - Green Phase)

**Files to modify:**
- `crates/jit/src/main.rs` - Handle gate flags in update handler
- `crates/jit/src/commands/mod.rs` - Add gate modification logic to CommandExecutor

**Implementation in main.rs:**
```rust
IssueCommands::Update {
    id,
    filter,
    // ... existing fields ...
    add_gate,
    remove_gate,
    json,
} => {
    // ... existing validation ...
    
    if let Some(id) = &id {
        // Single-issue mode
        let resolved_id = executor.resolve_issue_id(id)?;
        
        // Validate gates exist in registry
        for gate_key in &add_gate {
            executor.validate_gate_exists(gate_key)?;
        }
        
        // Add gates
        for gate_key in &add_gate {
            executor.add_gate_to_issue(&resolved_id, gate_key)?;
        }
        
        // Remove gates
        for gate_key in &remove_gate {
            executor.remove_gate_from_issue(&resolved_id, gate_key)?;
        }
        
        // ... rest of update logic ...
    }
}
```

**New methods in CommandExecutor:**
```rust
/// Validate that a gate key exists in the registry
pub fn validate_gate_exists(&self, gate_key: &str) -> Result<()> {
    let registry = self.registry_manager.load()?;
    if !registry.gates.contains_key(gate_key) {
        return Err(anyhow!(
            "Gate '{}' not found in registry. Use 'jit gate define' to create it.",
            gate_key
        ));
    }
    Ok(())
}

/// Add a gate to an issue
pub fn add_gate_to_issue(&self, issue_id: &str, gate_key: &str) -> Result<()> {
    let mut issue = self.storage.load_issue(issue_id)?;
    
    // Idempotent: skip if already present
    if !issue.gates_required.contains(&gate_key.to_string()) {
        issue.gates_required.push(gate_key.to_string());
        self.storage.save_issue(&issue)?;
        
        // Log event
        self.event_logger.log_gate_added(issue_id, gate_key)?;
    }
    
    Ok(())
}

/// Remove a gate from an issue
pub fn remove_gate_from_issue(&self, issue_id: &str, gate_key: &str) -> Result<()> {
    let mut issue = self.storage.load_issue(issue_id)?;
    
    issue.gates_required.retain(|g| g != gate_key);
    self.storage.save_issue(&issue)?;
    
    // Log event
    self.event_logger.log_gate_removed(issue_id, gate_key)?;
    
    Ok(())
}
```

**Expected:** Single-issue tests pass

### Phase 3: Implement Batch Mode Support

**Files to modify:**
- `crates/jit/src/commands/bulk_update.rs` - Add gate fields to UpdateOperations

**Changes to UpdateOperations:**
```rust
pub struct UpdateOperations {
    pub priority: Option<Priority>,
    pub state: Option<State>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub assignee: Option<String>,
    pub unassign: bool,
    
    // NEW: Gate operations
    pub add_gates: Vec<String>,
    pub remove_gates: Vec<String>,
}
```

**Update apply_operations_to_issue:**
```rust
fn apply_operations_to_issue(&self, ...) -> Result<bool> {
    // ... existing validation ...
    
    // Validate gates exist in registry
    for gate_key in &operations.add_gates {
        self.validate_gate_exists(gate_key)?;
    }
    
    // ... load issue ...
    
    // Add gates
    for gate_key in &operations.add_gates {
        if !updated.gates_required.contains(gate_key) {
            updated.gates_required.push(gate_key.clone());
            modified = true;
        }
    }
    
    // Remove gates
    for gate_key in &operations.remove_gates {
        let original_len = updated.gates_required.len();
        updated.gates_required.retain(|g| g != gate_key);
        if updated.gates_required.len() != original_len {
            modified = true;
        }
    }
    
    // ... rest of logic ...
}
```

**Update main.rs batch handler:**
```rust
let operations = UpdateOperations {
    // ... existing fields ...
    add_gates: add_gate.clone(),
    remove_gates: remove_gate.clone(),
};
```

**Expected:** Batch mode tests pass

### Phase 4: Update Event Logging

**Files to modify:**
- `crates/jit/src/events.rs` - Add GateAdded/GateRemoved events (if not already present)

**Check if these events exist, if not add:**
```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum Event {
    // ... existing variants ...
    
    #[serde(rename = "gate_added")]
    GateAdded {
        id: String,
        issue_id: String,
        gate_key: String,
        timestamp: String,
    },
    
    #[serde(rename = "gate_removed")]
    GateRemoved {
        id: String,
        issue_id: String,
        gate_key: String,
        timestamp: String,
    },
}
```

### Phase 5: Regenerate MCP Schema

**Commands:**
```bash
cargo build --release
./target/release/jit --schema > mcp-server/jit-schema.json
```

**Verify:** Schema includes `add-gate` and `remove-gate` flags

### Phase 6: Update Documentation

**Files to update:**
- Update EXAMPLE.md with gate modification examples
- Potentially update gate-related quickstart sections

**Example snippet:**
```bash
# Add gates to existing issue
jit issue update abc123 --add-gate tests --add-gate clippy

# Remove gate
jit issue update abc123 --remove-gate manual-review

# Batch: Add code-review to all ready tasks
jit issue update --filter "state:ready AND label:type:task" --add-gate code-review
```

## Test Coverage

### Unit Tests (in `commands/mod.rs`)
- `test_validate_gate_exists_success`
- `test_validate_gate_exists_not_found`
- `test_add_gate_to_issue_new`
- `test_add_gate_to_issue_duplicate_idempotent`
- `test_remove_gate_from_issue`

### Integration Tests (in `tests/gate_modification_cli_tests.rs`)
1. Add single gate
2. Add multiple gates (multiple flags)
3. Add multiple gates (comma-separated)
4. Remove gate
5. Add and remove in same command
6. Error: Invalid gate key
7. Batch mode: Add gates to multiple issues
8. JSON output includes updated gates_required
9. Combine with other update operations (state, labels)

### Edge Cases
- Adding gate that already exists (should be idempotent)
- Removing gate that doesn't exist (should be no-op)
- Empty gate key
- Gate key with special characters

## Quality Gates

Following TDD approach:

1. ✅ **TDD Reminder** - Pass before starting
2. ✅ **Tests** - All tests passing (362 existing + ~14 new = 376 total)
3. ✅ **Clippy** - Zero warnings
4. ✅ **Fmt** - All code formatted
5. ✅ **Code Review** - Manual review of changes

## Success Criteria

- [ ] `--add-gate` and `--remove-gate` flags work in single-issue mode
- [ ] Flags work in batch mode (with `--filter`)
- [ ] Multiple gates supported (comma-separated and multiple flags)
- [ ] Invalid gate keys return clear error messages
- [ ] Operations are idempotent (adding duplicate is no-op)
- [ ] JSON output includes updated `gates_required` array
- [ ] Events logged for gate additions/removals
- [ ] MCP schema updated with new flags
- [ ] All existing tests still pass
- [ ] 14 new tests added and passing
- [ ] Zero clippy warnings
- [ ] Code formatted

## Estimated Timeline

- **Phase 1 (Red):** 30 minutes - Write failing tests
- **Phase 2 (Green):** 45 minutes - Implement single-issue mode
- **Phase 3 (Green):** 30 minutes - Implement batch mode
- **Phase 4:** 15 minutes - Event logging
- **Phase 5:** 10 minutes - MCP schema update
- **Phase 6:** 20 minutes - Documentation

**Total:** 2.5 hours

## Dependencies

None - this is a standalone feature addition.

## Risks & Mitigation

**Risk:** Breaking existing gate add/remove commands  
**Mitigation:** Keep existing `jit gate add/remove` commands, just add convenience flags

**Risk:** Inconsistent behavior between single and batch mode  
**Mitigation:** Share validation logic (`validate_gate_exists`)

**Risk:** Event logging inconsistencies  
**Mitigation:** Reuse existing event logging patterns

## Follow-up Work

After this is complete:
- Update AGENT-QUICKSTART.md with gate modification examples
- Consider deprecating separate `jit gate add/remove` in favor of these flags (post-v1.0)

---

**Ready to proceed?** Start with Phase 1 (TDD Red) by writing the integration tests.
