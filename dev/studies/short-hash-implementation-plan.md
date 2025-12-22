# Implementation Plan: Short Hash Issue IDs

## Overview

Enable CLI to accept short UUID prefixes (like git short commit hashes) instead of requiring full UUIDs for all commands.

## Current State

- JIT uses UUID v4 for issue IDs (e.g., `9db27a3a-86c5-4d79-9582-9ad68364ea36`)
- All CLI commands require full UUID
- Issue files stored as `{uuid}.json` in `.jit/issues/`
- `load_issue()` only accepts exact matches

## Goal

Support git-style short hash resolution: accept partial UUID prefix (minimum 4 chars), auto-resolve to full hash.

## Task Breakdown

### 1. Add ID Resolution Logic (Core Functionality)

**Location**: `crates/jit/src/storage/mod.rs` (trait) and implementations

**Function signature**:
```rust
fn resolve_issue_id(&self, partial_id: &str) -> Result<String>
```

**Logic**:
- Validate input: minimum 4 characters, valid hex UUID prefix format
- Normalize input: lowercase, strip hyphens for matching
- Load all issue IDs from index
- Filter IDs matching prefix
- Return full ID if unique match
- Error cases:
  - Too short (< 4 chars): `"Issue ID prefix must be at least 4 characters"`
  - No match: `"Issue not found: {partial_id}"`
  - Ambiguous (multiple matches): `"Ambiguous ID '{partial_id}' matches multiple issues: {list}"`

**Edge cases**:
- Full UUID: pass through unchanged (backward compatible)
- With hyphens: `9db27a3a-86c5` should work
- Without hyphens: `9db27a3a86c5` should work
- Case insensitive: `9DB27A3A` should work

### 2. Update Storage Trait

**File**: `crates/jit/src/storage/mod.rs`

Add method to `IssueStore` trait:
```rust
pub trait IssueStore: Clone {
    // ... existing methods ...
    
    /// Resolve a partial issue ID to its full UUID
    ///
    /// Accepts either a full UUID or a unique prefix (minimum 4 characters).
    /// Returns the full UUID if a unique match is found.
    ///
    /// # Errors
    ///
    /// - Prefix too short (< 4 chars)
    /// - No matching issue found
    /// - Multiple issues match (ambiguous)
    fn resolve_issue_id(&self, partial_id: &str) -> Result<String>;
}
```

### 3. Implement for Storage Backends

#### JsonFileStorage (`crates/jit/src/storage/json.rs`)

```rust
fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
    // Normalize input
    let normalized = partial_id.to_lowercase().replace('-', "");
    
    // Full UUID check (fast path)
    if normalized.len() == 32 {
        // Try loading to verify it exists
        return self.load_issue(partial_id)
            .map(|issue| issue.id)
            .context("Issue not found");
    }
    
    // Minimum length check
    if normalized.len() < 4 {
        bail!("Issue ID prefix must be at least 4 characters");
    }
    
    // Load index and filter
    let index = self.load_index()?;
    let matches: Vec<String> = index.all_ids
        .iter()
        .filter(|id| id.replace('-', "").to_lowercase().starts_with(&normalized))
        .cloned()
        .collect();
    
    match matches.len() {
        0 => bail!("Issue not found: {}", partial_id),
        1 => Ok(matches[0].clone()),
        _ => bail!("Ambiguous ID '{}' matches multiple issues: {}", 
                   partial_id, matches.join(", "))
    }
}
```

#### InMemoryStorage (`crates/jit/src/storage/memory.rs`)

Similar implementation, iterate over stored issues instead of index.

### 4. Update CLI Commands

**Pattern to apply**: Wrap `load_issue()` calls with resolution

```rust
// OLD
let issue = self.storage.load_issue(&id)?;

// NEW
let full_id = self.storage.resolve_issue_id(&id)?;
let issue = self.storage.load_issue(&full_id)?;
```

**Commands to update** (in `crates/jit/src/commands/`):

- `issue.rs`: `show_issue`, `update_issue`, `delete_issue`, `assign_issue`, `unassign_issue`, `claim_issue`, `release_issue`, `reject_issue`
- `dependency.rs`: `add_dependency`, `remove_dependency`
- `gate.rs`: `add_gate_to_issue`, `pass_gate`, `fail_gate`
- `gate_check.rs`: `check_gate`, `check_all_gates`
- `document.rs`: `add_document`, `remove_document`, `list_documents`, `show_document`, `document_history`, `document_diff`
- `graph.rs`: `show_graph`, `show_downstream`
- `breakdown.rs`: `breakdown_issue`

**Note**: Only update commands that take issue ID from user input. Internal operations using IDs from the database should continue using full UUIDs.

### 5. Add Tests

#### Unit Tests (`crates/jit/src/storage/json.rs`, `crates/jit/src/storage/memory.rs`)

```rust
#[test]
fn test_resolve_full_uuid() {
    // Full UUID should pass through
}

#[test]
fn test_resolve_short_prefix() {
    // 4-8 char prefix should resolve
}

#[test]
fn test_resolve_with_hyphens() {
    // Handle hyphens in input
}

#[test]
fn test_resolve_case_insensitive() {
    // Uppercase/lowercase should work
}

#[test]
fn test_resolve_too_short() {
    // < 4 chars should error
}

#[test]
fn test_resolve_not_found() {
    // Non-matching prefix should error
}

#[test]
fn test_resolve_ambiguous() {
    // Multiple matches should error with list
}
```

#### Integration Tests (new file: `crates/jit/tests/short_hash_tests.rs`)

```rust
#[test]
fn test_cli_show_with_short_hash() {
    // jit issue show <short-hash>
}

#[test]
fn test_cli_update_with_short_hash() {
    // jit issue update <short-hash> --state done
}

#[test]
fn test_cli_dep_add_with_short_hashes() {
    // jit dep add <short1> <short2>
}

#[test]
fn test_ambiguous_short_hash_error() {
    // Create issues with overlapping prefixes
    // Verify error message lists candidates
}
```

#### Property-Based Tests (`proptest`)

```rust
proptest! {
    #[test]
    fn test_resolve_random_prefixes(prefix_len in 4usize..16) {
        // Generate random UUIDs
        // Test various prefix lengths resolve correctly
    }
}
```

### 6. Update Documentation

#### README.md

Add example in "Commands" section:
```bash
# Show issue with short hash (minimum 4 characters)
jit issue show 9db27a3
jit issue update 003f9f8 --state done
```

#### EXAMPLE.md

Update examples to show both full and short IDs:
```bash
# Both work:
jit issue claim 9db27a3a-86c5-4d79-9582-9ad68364ea36 copilot:worker-1
jit issue claim 9db27a3 copilot:worker-1
```

#### CLI Help Text

Update `crates/jit/src/cli.rs` command help:
```rust
/// Show issue details
Show {
    /// Issue ID (full UUID or unique prefix, minimum 4 characters)
    id: String,
    
    #[arg(long)]
    json: bool,
},
```

## Implementation Approach (TDD)

1. **Write tests first** for `resolve_issue_id()` (all edge cases)
2. **Implement minimal logic** to pass tests
3. **Add to storage trait** and both implementations
4. **Update one CLI command** as proof of concept
5. **Run tests**, fix issues
6. **Systematically update** remaining CLI commands
7. **Run full test suite** (`cargo test`)
8. **Run clippy** (`cargo clippy`) - zero warnings required
9. **Format code** (`cargo fmt`)
10. **Update documentation**

## Success Criteria

✅ `jit issue show 9db2` works if unique prefix  
✅ Clear error for too short: `"Issue ID prefix must be at least 4 characters"`  
✅ Clear error for ambiguous: `"Ambiguous ID '9db2' matches: 9db27a3a-..., 9db28f1b-..."`  
✅ Clear error for not found: `"Issue not found: 9db2"`  
✅ Full UUID continues to work (backward compatible)  
✅ All existing tests pass  
✅ New tests cover short hash resolution  
✅ Zero clippy warnings  
✅ Code formatted with `cargo fmt`  
✅ Documentation updated

## Performance Considerations

- Resolution requires loading index (already cached in most operations)
- Full UUID fast path avoids unnecessary index scan
- Overhead negligible compared to file I/O
- No performance degradation for existing full UUID usage

## Backward Compatibility

✅ Fully backward compatible - full UUIDs continue to work unchanged  
✅ No breaking changes to storage format  
✅ No breaking changes to API  

## Error Messages

All error messages follow git's approach - clear, actionable, and helpful:

```
Error: Issue ID prefix must be at least 4 characters

Error: Issue not found: xyz123

Error: Ambiguous ID '9db2' matches multiple issues:
  9db27a3a-86c5-4d79-9582-9ad68364ea36 | Support short hash issue IDs in CLI
  9db28f1b-2c3d-4e5f-8a9b-1c2d3e4f5a6b | Add gate preview command
```

## Dependencies

No new crate dependencies required - uses existing:
- `anyhow` for error handling
- `uuid` (already used for ID generation)

## Estimated Effort

- Core resolution logic: 2-3 hours
- CLI command updates: 3-4 hours  
- Tests: 2-3 hours
- Documentation: 1 hour
- **Total: 8-11 hours**

## Related Issues

This task is part of Epic: Phase 5.2: Agent-Friendly Additions (14303b30-75b2-4b21-8963-bc6563b95b91)
