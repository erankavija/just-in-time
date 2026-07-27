# Issue 82b17394: Implement CLI-level Lease Enforcement

**Status:** In Progress, Claimed  
**Lease:** 5140a44c-9239-4c45-8698-c9c713eae9fe (600s)  
**Depends on:** 5e1d5f02 (COMPLETE - enforcement modes config)

## Objective

Add lease requirement checks before structural operations that modify existing issues.
Respect configurable enforcement modes (strict/warn/off).

## Key Requirements

1. **EXEMPTED Operations** (no lease required):
   - `jit issue create` - allocates new ULID, cannot conflict
   - Read operations - queries, show, list, search

2. **ENFORCED Operations** (require active lease):
   - `jit issue update` - modify existing issue
   - `jit issue delete` - delete existing issue
   - `jit dep add/rm` - modify dependencies
   - `jit issue assign/unassign` - modify assignment
   - State transitions (ready, in_progress, done, etc.)
   - Gate operations on existing issues

3. **Enforcement Modes** (from 5e1d5f02):
   - `strict` - Block operation, return error
   - `warn` - Log warning, allow operation
   - `off` - Bypass all checks

## Implementation Approach

### 1. Add `require_active_lease()` to CommandExecutor

```rust
impl<S: IssueStore> CommandExecutor<S> {
    fn require_active_lease(&self, issue_id: &str) -> Result<()> {
        let mode = self.config_manager.get_enforcement_mode()?;
        
        match mode {
            EnforcementMode::Off => Ok(()),
            EnforcementMode::Warn | EnforcementMode::Strict => {
                // Check if lease exists for this issue
                let has_lease = self.check_active_lease(issue_id)?;
                
                if !has_lease {
                    let msg = format!(
                        "No active lease for issue {}.\nAcquire lease: jit claim acquire {}",
                        issue_id, issue_id
                    );
                    
                    match mode {
                        EnforcementMode::Warn => {
                            eprintln!("⚠️  Warning: {}", msg);
                            Ok(())
                        }
                        EnforcementMode::Strict => {
                            anyhow::bail!("{}", msg)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    Ok(())
                }
            }
        }
    }
    
    fn check_active_lease(&self, issue_id: &str) -> Result<bool> {
        // Load claims index from .git/jit/claims.index.json
        // Check if current agent has active lease for issue_id
        // Return true if active lease exists, false otherwise
    }
}
```

### 2. Add Enforcement Calls

Update these operations to call `require_active_lease()` first:
- `update_issue()` - before modifying
- `delete_issue()` - before deleting
- `add_dependency()` / `remove_dependency()` - before changing deps
- `assign_issue()` / `unassign_issue()` - before assignment changes
- State transition methods

### 3. Wire ConfigManager to CommandExecutor

CommandExecutor needs access to ConfigManager to get enforcement mode.
Check current constructor and add config_manager field.

## TDD Approach

1. **RED**: Write tests for enforcement in strict/warn/off modes
2. **GREEN**: Implement `require_active_lease()` and `check_active_lease()`
3. **REFACTOR**: Add enforcement calls to all structural operations
4. **VERIFY**: All tests pass, zero clippy warnings

## Acceptance Criteria

- [ ] `require_active_lease()` method added
- [ ] `check_active_lease()` loads claims index and validates
- [ ] Enforcement respects strict/warn/off modes
- [ ] Issue create EXEMPTED (no lease check)
- [ ] All structural operations enforced
- [ ] Tests for all three modes
- [ ] Tests verify error messages are actionable
- [ ] Zero clippy warnings
- [ ] All tests pass
