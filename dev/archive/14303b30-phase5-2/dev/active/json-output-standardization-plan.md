# Item 12: JSON Output Standardization - Implementation Plan

## Overview

Standardize JSON output structures across all JIT commands to provide a consistent, predictable API for automation and agent consumption.

**Status:** Requires implementation + design decisions
**Estimated effort:** 4-6 hours
**Risk:** Low (mostly additive changes, some breaking for machine consumers)

---

## Design Principles

All list/query commands should follow consistent patterns:

### Pattern 1: Simple List Response
```json
{
  "success": true,
  "data": {
    "<items_name>": [...],  // Named field (e.g., "issues", "gates", "namespaces")
    "count": N              // Total count
  },
  "metadata": {...}
}
```

### Pattern 2: Filtered/Contextual List Response
```json
{
  "success": true,
  "data": {
    "<items_name>": [...],
    "count": N,
    "<context_field>": "..." // e.g., "assignee", "namespace", "issue_id"
  },
  "metadata": {...}
}
```

### Pattern 3: Single Item Response
```json
{
  "success": true,
  "data": {
    "id": "...",
    "title": "...",
    // ... all item fields directly in data
  },
  "metadata": {...}
}
```

### Pattern 4: Summary/Aggregate Response
```json
{
  "success": true,
  "data": {
    "field1": N,
    "field2": N,
    // ... summary fields directly in data
  },
  "metadata": {...}
}
```

---

## Detailed Changes

### 1. Fix `gate list` ❌ BREAKING CHANGE

**Current behavior:**
```bash
$ jit gate list --json
{
  "data": [
    {"key": "tests", "title": "Run tests", ...},
    {"key": "clippy", "title": "Run clippy", ...}
  ]
}
```

**Problem:** Data is an array, not an object with named field + count.

**Proposed fix:**
```bash
$ jit gate list --json
{
  "data": {
    "gates": [
      {"key": "tests", "title": "Run tests", ...},
      {"key": "clippy", "title": "Run clippy", ...}
    ],
    "count": 2
  }
}
```

**Changes required:**
- `crates/jit/src/output.rs`: Add `GateListResponse` type (already exists, just needs to be used)
- `crates/jit/src/commands/gate.rs`: Wrap response in structured type
- `crates/jit/tests/registry_json_tests.rs`: Update assertions

**Breaking change:** Yes - any script parsing `jit gate list --json` will break

---

### 2. Fix `label namespaces` ❌ BREAKING CHANGE

**Current behavior:**
```bash
$ jit label namespaces --json
{
  "data": {
    "namespaces": ["epic", "milestone", "type", "component"],
    "type_hierarchy": {"milestone": 1, "epic": 2, ...},
    "label_associations": {...},
    "strategic_types": ["milestone", "epic"],
    "schema_version": 1
  }
}
```

**Problems:**
1. Exposes internal configuration structure
2. Returns more data than user requested
3. Inconsistent with other list commands (no `count`)
4. Mixes user data with metadata

**Proposed fix:**
```bash
$ jit label namespaces --json
{
  "data": {
    "namespaces": ["epic", "milestone", "type", "component"],
    "count": 4
  }
}
```

**Changes required:**
- `crates/jit/src/output.rs`: Add `NamespacesResponse` type
- `crates/jit/src/commands/label.rs`: Create minimal response
- Tests: Update any assertions checking namespace output

**Breaking change:** Yes - structure completely changes

---

### 3. Fix `query all` - Remove filters field ❌ BREAKING CHANGE

**Current behavior:**
```bash
$ jit query all --state ready --json
{
  "data": {
    "issues": [...],
    "count": 5,
    "filters": {
      "state": "ready"
    }
  }
}
```

**Problem:** Other query commands don't have `filters` field

**Proposed fix:**
```bash
$ jit query all --state ready --json
{
  "data": {
    "issues": [...],
    "count": 5
  }
}
```

**Changes required:**
- `crates/jit/src/commands/query.rs`: Remove `filters` from response
- `crates/jit/tests/query_json_tests.rs`: Update assertions

**Breaking change:** Yes, but only for `query all`

---

### 4. Fix `search` - Rename "total" to "count" ❌ BREAKING CHANGE

**Current behavior:**
```bash
$ jit search "query" --json
{
  "data": {
    "results": [...],
    "total": 5,
    "query": "query"
  }
}
```

**Problem:** Uses "total" instead of "count" (all other commands use "count")

**Proposed fix:**
```bash
$ jit search "query" --json
{
  "data": {
    "results": [...],
    "count": 5,
    "query": "query"
  }
}
```

**Changes required:**
- `crates/jit/src/commands/search.rs`: Rename field in response struct
- Tests: Update assertions

**Breaking change:** Yes - field name changes

---

### 5. Fix `worktree list` - Add count field ⚠️ ADDITIVE CHANGE

**Current behavior:**
```bash
$ jit worktree list --json
{
  "data": {
    "worktrees": [...]
  }
}
```

**Problem:** Missing `count` field that all other list commands have

**Proposed fix:**
```bash
$ jit worktree list --json
{
  "data": {
    "worktrees": [...],
    "count": 3
  }
}
```

**Changes required:**
- `crates/jit/src/commands/worktree.rs`: Add count to response
- Tests: Add assertions for count field

**Breaking change:** No - purely additive (adds field, doesn't change existing)

---

## Backward Compatibility Analysis

### Breaking Changes Summary

| Command | Change | Impact | Breaking? |
|---------|--------|--------|-----------|
| `gate list` | Wrap array in object | HIGH | ✅ Yes |
| `label namespaces` | Remove internal fields | HIGH | ✅ Yes |
| `query all` | Remove filters field | LOW | ✅ Yes |
| `search` | Rename "total" to "count" | LOW | ✅ Yes |
| `worktree list` | Add count field | NONE | ❌ No (additive) |

**Total breaking changes:** 4 commands
**Total fixes:** 5 commands

### Migration Path

**Version Strategy:**
- Bump to 0.3.0 (minor version, breaking API changes)
- Document all breaking changes in CHANGELOG
- Provide migration examples

**Changelog entry:**
```markdown
## [0.2.1] - 2026-02-11

### Breaking Changes - JSON API Standardization

**Goal:** Consistent JSON output structures across all commands for better automation.

**`jit gate list --json`**
- Response structure changed from array to object
- Before: `{"data": [...]}`
- After: `{"data": {"gates": [...], "count": N}}`
- Migration: Access via `.data.gates` instead of `.data`

**`jit label namespaces --json`**
- Simplified response to requested data only (removed internal config fields)
- Before: `{"data": {"namespaces": [...], "type_hierarchy": {...}, "strategic_types": [...], ...}}`
- After: `{"data": {"namespaces": [...], "count": N}}`
- Migration: Access `.data.namespaces` (same path), other fields removed

**`jit query all --json`**
- Removed redundant `filters` field for consistency with other query commands
- Before: `{"data": {"issues": [...], "count": N, "filters": {...}}}`
- After: `{"data": {"issues": [...], "count": N}}`
- Migration: Remove any code accessing `.data.filters`

**`jit search --json`**
- Renamed `total` field to `count` for consistency
- Before: `{"data": {"results": [...], "total": N, "query": "..."}}`
- After: `{"data": {"results": [...], "count": N, "query": "..."}}`
- Migration: Change `.data.total` to `.data.count`

### Non-Breaking Changes

**`jit worktree list --json`**
- Added `count` field (additive change, backward compatible)
- Before: `{"data": {"worktrees": [...]}}`
- After: `{"data": {"worktrees": [...], "count": N}}`
```

---

## Implementation Checklist

### Phase 1: Add Response Types (30 min)
- [ ] Add `GateListResponse` usage in output.rs (type exists, just use it)
- [ ] Add `NamespacesResponse` struct in output.rs
- [ ] Verify `IssueListResponse` used by query commands

### Phase 2: Update Commands (1-2 hours)
- [ ] `crates/jit/src/commands/gate.rs`
  - [ ] Change `gate_list()` to return `GateListResponse`
  - [ ] Wrap gates vector with count
- [ ] `crates/jit/src/commands/label.rs`
  - [ ] Change `list_namespaces()` to return `NamespacesResponse`
  - [ ] Extract only namespaces array + count
- [ ] `crates/jit/src/commands/query.rs`
  - [ ] Remove `filters` field from `query_all()` response
  - [ ] Update response construction
- [ ] `crates/jit/src/commands/search.rs`
  - [ ] Rename `total` field to `count` in response struct
  - [ ] Update serialization
- [ ] `crates/jit/src/commands/worktree.rs`
  - [ ] Add `count` field to worktree list response
  - [ ] Calculate and include count

### Phase 3: Update Tests (1-2 hours)
- [ ] `crates/jit/tests/registry_json_tests.rs`
  - [ ] Update `test_gate_list_json()` assertions
  - [ ] Change from `.data[0]` to `.data.gates[0]`
  - [ ] Add assertion for `.data.count`
- [ ] `crates/jit/tests/query_json_tests.rs`
  - [ ] Update `test_query_all_json()` if filters removed
  - [ ] Verify no other tests depend on filters field
- [ ] Search for any other tests using `label namespaces --json`
  - [ ] `rg "label namespaces.*json" crates/jit/tests/`
  - [ ] Update assertions to expect new structure

### Phase 4: Integration Testing (30 min)
- [ ] Test all affected commands manually
  - [ ] `jit gate list --json | jq .`
  - [ ] `jit label namespaces --json | jq .`
  - [ ] `jit query all --json | jq .`
- [ ] Verify other commands still work
  - [ ] `jit query available --json`
  - [ ] `jit doc list <issue> --json`
  - [ ] `jit graph deps <issue> --json`

### Phase 5: Documentation (30 min)
- [ ] Update CHANGELOG.md with breaking changes
- [ ] Update docs/reference/cli-commands.md with new JSON examples
- [ ] Add migration notes if needed

### Phase 6: Version Bump (15 min)
- [ ] Update version in `Cargo.toml` files to 0.2.1
- [ ] Update `OUTPUT_VERSION` in `output.rs` to "0.2.1"
- [ ] Run `cargo build` to update `Cargo.lock`
- [ ] Update CHANGELOG.md with breaking changes section

---

## Testing Strategy

### Unit Tests
```rust
#[test]
fn test_gate_list_response_structure() {
    let gates = vec![/* ... */];
    let response = GateListResponse {
        gates: gates.clone(),
        count: gates.len(),
    };
    let output = JsonOutput::success(response, "gate list");
    let json = output.to_json_string().unwrap();
    
    // Verify structure
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["data"]["count"], gates.len());
    assert!(parsed["data"]["gates"].is_array());
}

#[test]
fn test_namespaces_response_structure() {
    let namespaces = vec!["epic", "milestone", "type"];
    let response = NamespacesResponse {
        namespaces: namespaces.clone(),
        count: namespaces.len(),
    };
    let output = JsonOutput::success(response, "label namespaces");
    let json = output.to_json_string().unwrap();
    
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["data"]["count"], 3);
    assert_eq!(parsed["data"]["namespaces"], json!(namespaces));
    assert!(parsed["data"]["type_hierarchy"].is_null()); // Should NOT exist
}
```

### Integration Tests
```bash
# Verify JSON structure matches pattern
jit gate list --json | jq -e '.data | has("gates")' || echo "FAIL"
jit gate list --json | jq -e '.data | has("count")' || echo "FAIL"

jit label namespaces --json | jq -e '.data | has("namespaces")' || echo "FAIL"
jit label namespaces --json | jq -e '.data | has("count")' || echo "FAIL"
jit label namespaces --json | jq -e '.data | has("type_hierarchy") | not' || echo "FAIL"
```

---

## Risk Assessment

**Low Risk Areas:**
- Top-level `JsonOutput` wrapper already consistent ✅
- Most commands already follow correct patterns ✅
- Changes are isolated to specific commands ✅

**Medium Risk Areas:**
- Breaking changes for automation/agents using affected commands
- Need to coordinate version bump + changelog
- Tests might be scattered across multiple files

**Mitigation:**
- Clear changelog with migration path
- Version bump signals breaking change
- Comprehensive test updates
- Manual verification of all affected commands

---

## Open Questions / Design Decisions Needed

### ✅ Decision 1: `label namespaces` response structure - DECIDED
- **Choice:** Option A - Minimal format `{namespaces: [], count: N}`
- **Rationale:** Simpler, matches `label values` pattern, strategic types available via config

### ✅ Decision 2: `query all` filters field - DECIDED
- **Choice:** Option A - Remove filters field for consistency
- **Rationale:** Users know what filters they passed, consistency matters more

### ✅ Decision 3: Version bump strategy - DECIDED
- **Choice:** 0.2.1 - Patch bump to minimize version noise
- **Rationale:** Pre-1.0, keep versions simple, note breaking changes in changelog

### ✅ Investigation 4: Additional Inconsistencies Found

**Commands checked:**
- `jit issue search` ✅ - Has `{issues: [], count: N, query: "..."}` - CORRECT pattern
- `jit search` ⚠️ - Has `{results: [], total: N, query: "..."}` - Uses "total" not "count"
- `jit claim list` ✅ - Has `{leases: [], count: N}` - CORRECT pattern
- `jit worktree list` ❌ - Has `{worktrees: []}` - MISSING count field
- `jit events tail` ℹ️ - No JSON output (not implemented)
- `jit events query` ℹ️ - No JSON output (not implemented)

**Additional fixes needed:**
1. `jit search` - Change "total" to "count" for consistency
2. `jit worktree list` - Add "count" field

---

## Success Criteria

- [ ] All list commands return `{<items>: [], count: N}` structure
- [ ] No internal config exposed in API responses
- [ ] Consistent patterns across similar commands
- [ ] All tests pass
- [ ] Documentation updated
- [ ] Changelog includes migration guide
- [ ] Version bumped appropriately

---

## Timeline Estimate

- **Analysis & Planning:** 1 hour (DONE ✅)
- **Implementation:** 2-3 hours
- **Testing:** 1-2 hours
- **Documentation:** 30-60 min
- **Total:** 4.5-6.5 hours

---

## Files to Modify

**Source:**
- `crates/jit/src/output.rs` - Add response types
- `crates/jit/src/commands/gate.rs` - Fix gate list
- `crates/jit/src/commands/label.rs` - Fix label namespaces
- `crates/jit/src/commands/query.rs` - Remove filters from query all
- `crates/jit/src/commands/search.rs` - Rename total to count
- `crates/jit/src/commands/worktree.rs` - Add count field

**Tests:**
- `crates/jit/tests/registry_json_tests.rs` - Gate list tests
- `crates/jit/tests/query_json_tests.rs` - Query all tests
- `crates/jit/tests/worktree_cli_tests.rs` - Worktree tests (if exists)
- Any tests using label namespaces (search needed)
- Any tests using search command (search needed)

**Documentation:**
- `CHANGELOG.md` - Breaking changes with migration guide
- `docs/reference/cli-commands.md` - JSON examples (if exists)

**Version:**
- `Cargo.toml` (all packages) - Version bump to 0.2.1
- `crates/jit/src/output.rs` - OUTPUT_VERSION constant to "0.2.1"
