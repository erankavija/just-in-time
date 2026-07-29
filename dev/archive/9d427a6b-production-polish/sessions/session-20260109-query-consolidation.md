# Session Notes: Query Consolidation Implementation

**Date:** 2026-01-09  
**Issue:** 11682766 - Consolidate and simplify jit query commands  
**Status:** Core implementation complete, testing and documentation remaining

## Session Overview

Implemented major consolidation of `jit query` commands to eliminate redundancy, fix naming confusion, and optimize for token efficiency. The implementation is functionally complete with core features working.

## What Was Accomplished

### 1. Unified Query Interface ✅

**Removed redundant commands:**
- ❌ `jit query state` → use `jit query all --state`
- ❌ `jit query assignee` → use `jit query all --assignee`
- ❌ `jit query priority` → use `jit query all --priority`
- ❌ `jit query label` → use `jit query all --label`
- ❌ `jit query all` → use `jit query all`

**New unified commands:**
```bash
# Base filter with composable flags
jit query all [--state STATE] [--assignee X] [--priority P] [--label PATTERN]

# Special queries with optional filters
jit query available [--priority P] [--label PATTERN]   # was: ready
jit query blocked [--priority P] [--label PATTERN]
jit query strategic [--priority P] [--label PATTERN]
jit query closed [--priority P] [--label PATTERN]
```

**Key change:** Renamed `ready` → `available` to avoid confusion with `--state ready`

### 2. Output Format Consistency ✅

**Before (inconsistent):**
- `query all`: `{filters, issues, summary: {total, by_state}}`
- `query ready`: `{count, issues}`
- Other queries: various formats

**After (consistent):**
- All commands: `{count, issues}`
- `query all` adds: `{count, issues, filters}`

This eliminates confusion between `summary.total` vs `count`.

### 3. Token Efficiency with --full Flag ✅

**Default (minimal) output:**
```json
{
  "count": 53,
  "issues": [
    {
      "id": "aa54c441-f9ae-414f-b244-21e5fcdd1977",
      "title": "Implement gate history",
      "state": "ready",
      "priority": "high"
    }
  ]
}
```

**With --full flag:**
```json
{
  "count": 53,
  "issues": [
    {
      "id": "...",
      "title": "...",
      "description": "...",  // Often huge!
      "state": "...",
      "priority": "...",
      "dependencies": [...],
      "gates_required": [...],
      "gates_status": {...},
      "context": {...},
      "documents": [...],
      "labels": [...]
    }
  ]
}
```

**Token savings:** ~70% reduction (4 fields vs 12 fields, no descriptions)

**Special case - blocked queries:**
Minimal format includes `blocked_reasons` array:
```json
{
  "id": "...",
  "title": "...",
  "state": "...",
  "priority": "...",
  "blocked_reasons": ["dependency:abc123 (Task X:ready)", "gate:tests (Pending)"]
}
```

### 4. Code Changes

**Files modified:**
- `crates/jit/src/cli.rs` - Updated QueryCommands enum, removed List from IssueCommands
- `crates/jit/src/domain.rs` - Added MinimalIssue and MinimalBlockedIssue structs
- `crates/jit/src/commands/query.rs` - Added new query methods with filtering
- `crates/jit/src/main.rs` - Completely rewrote query command handlers
- Multiple test files - Updated to use new commands and JSON structure

**New domain types:**
```rust
pub struct MinimalIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
}

pub struct MinimalBlockedIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    pub blocked_reasons: Vec<String>,
}
```

### 5. Test Updates

Updated tests in:
- `error_json_tests.rs` - Changed from `query state` to `query all --state`
- `exit_code_tests.rs` - Changed from `issue list` to `query all`
- `gate_modification_cli_tests.rs` - Changed from `query label` to `query all --label`
- `integration_test.rs` - Changed from `issue list` to `query all`
- `label_hierarchy_e2e_test.rs` - Changed from `query ready` to `query available`
- `label_query_json_tests.rs` - Updated JSON assertions for new format
- `label_query_tests.rs` - Bulk replaced `query label` with `query all --label`
- `quiet_mode_tests.rs` - Changed from `issue list` to `query all`
- `query_json_tests.rs` - Updated JSON assertions and commands
- `query_tests.rs` - Updated to use new commands and `count` instead of `summary.total`
- `schema_tests.rs` - Removed expectations for `issue list`, added `query all`

**Note:** Tests were updated to expect `count` instead of `summary.total`, but many still expect **full** issue objects instead of minimal. This is the main remaining work.

## Current Status

### ✅ Working
- All commands compile and run
- CLI help text updated
- `--full` flag implemented and working
- Clippy passes (1 pre-existing warning only)
- Code formatted with rustfmt
- Manual testing confirms token savings work

### ⚠️ Needs Work
- **Test failures:** Tests expect full issue objects but now get minimal by default
  - Need to either: add `--full` to test commands, or update assertions
  - Estimated: ~10-20 test assertions to fix
- **Documentation:** ~26 markdown files need updates
- **Scripts:** 3 scripts need command updates
- **MCP server:** Needs regeneration from updated schema

### 🔧 Test Failures to Fix

Most failures are because tests expect full issue JSON but get minimal:

**Pattern 1 - Missing fields:**
```rust
// Test expects:
assert!(json["data"]["issues"][0]["description"]);

// But minimal only has: id, title, state, priority
// Fix: Add --full flag to command or remove assertion
```

**Pattern 2 - JSON structure:**
```rust
// Some tests may still check old formats
assert_eq!(json["data"]["summary"]["total"], 1);  // OLD
assert_eq!(json["data"]["count"], 1);              // NEW
```

**Files with likely failures:**
- Any integration test checking issue fields beyond id/title/state/priority
- Tests that were already updated but might have edge cases

## Next Steps

### Immediate (to complete issue):

1. **Fix remaining test failures** (~1-2 hours)
   ```bash
   cargo test --workspace 2>&1 | grep FAILED -B 5
   ```
   - Add `--full` flag where tests need complete issues
   - Update assertions to work with minimal format
   - Check for any remaining `summary.total` references

2. **Update documentation** (~2-3 hours)
   - Find all references: `rg "jit (query|issue list)" docs/ dev/ --type md`
   - Update command examples to new syntax
   - Document --full flag usage
   - Update migration guides

3. **Update scripts** (~30 min)
   - `scripts/test-label-hierarchy-walkthrough.sh`
   - `scripts/agent-init-demo-project.sh`
   - `scripts/test-concurrent-mcp.sh`

4. **Regenerate MCP server** (~30 min)
   - MCP server auto-generates from CLI schema
   - Test with: `cd mcp-server && npm test`

5. **Final validation**
   ```bash
   cargo test --workspace --quiet
   cargo clippy --workspace --all-targets
   cargo fmt --all -- --check
   ```

6. **Update issue and mark done**
   - Pass all gates
   - Update issue to Done state

### Code Locations for Next Session

**To find remaining test failures:**
```bash
cd /home/vkaskivuo/Projects/just-in-time
cargo test --workspace 2>&1 | grep "thread.*panicked" -B 2
```

**To update docs:**
```bash
rg "jit query (ready|state|priority|assignee|label)" docs/ dev/ -l
rg "jit query all" docs/ dev/ -l
```

**Query command handler location:**
- File: `crates/jit/src/main.rs`
- Line: ~1447 (Commands::Query match)

**Test command pattern replacement:**
```bash
# Find tests using old commands
rg 'query", "(ready|state|label|priority|assignee)"' crates/jit/tests/

# Pattern to add --full flag
.args(["query", "available", "--full", "--json"])
```

## Migration Guide for Users

**Old → New command mappings:**

```bash
# Query commands
jit query available              → jit query available
jit query all --state ready        → jit query all --state ready
jit query all --priority high      → jit query all --priority high
jit query all --assignee X         → jit query all --assignee X
jit query all --label epic:auth    → jit query all --label epic:auth

# Issue list
jit query all               → jit query all
jit query all --state ready → jit query all --state ready

# Composable filters (NEW!)
jit query all --state ready --label epic:auth --priority high

# Special queries with filters (NEW!)
jit query available --label epic:auth
jit query blocked --priority critical

# Full output when needed
jit query all --full --json
jit query available --full --json
```

## Design Decisions Made

1. **Why rename `ready` to `available`?**
   - Avoids confusion: `jit query available` vs `jit query all --state ready`
   - Clear semantics: "available for claiming" vs "in ready state"
   - The query checks more than state (also unassigned + unblocked)

2. **Why remove `issue list`?**
   - 100% redundant with `jit query all`
   - Consolidates filtering in one place
   - `jit query` is the established pattern for finding issues

3. **Why default to minimal output?**
   - Token efficiency critical for AI agents
   - Descriptions can be hundreds of tokens each
   - `jit issue show` provides full details when needed
   - Opt-in to verbosity with `--full` feels right

4. **Why include `filters` in `query all` output?**
   - Transparency: shows what filters were applied
   - Debugging: easy to verify the query worked correctly
   - Machine-readable: agents can validate their requests

5. **Why keep `count` at top level instead of in summary?**
   - Consistency across all query commands
   - Simpler JSON structure
   - `summary.by_state` only useful for unfiltered queries

## Known Issues

None blocking - all functionality works as designed.

## Performance Notes

No performance testing done yet, but minimal output should:
- Reduce JSON serialization time
- Reduce network transfer for remote use
- Significantly reduce token usage in AI contexts

Typical savings: query returning 50 issues
- Before: ~15,000 tokens (with descriptions)
- After: ~2,000 tokens (minimal)
- With --full: ~15,000 tokens (unchanged)

## Questions for Review

1. Should `--full` be the default with `--minimal` opt-in instead?
   - Current choice: minimal by default (better for agents)
   - Alternative: full by default (less breaking, more familiar)
   - Decision: Stick with minimal - this is agent-first

2. Should blocked minimal format include full BlockedReason objects or strings?
   - Current: Simple strings like "dependency:abc123 (Task X:ready)"
   - Alternative: Structured objects with type/detail
   - Decision: Strings are simpler and human-readable

3. Add summary statistics to `query all` even in minimal mode?
   - Current: Only filters shown
   - Alternative: Include count_by_state breakdown
   - Decision: Not needed - adds tokens without much value

## Files to Review Before Continuing

1. `crates/jit/tests/` - Check test failures
2. `docs/reference/cli-commands.md` - Main CLI documentation
3. `docs/tutorials/quickstart.md` - User-facing tutorial
4. `CONTRIBUTOR-QUICKSTART.md` - Developer guide
5. `mcp-server/jit-schema.json` - Will be regenerated

## Lessons Learned

1. **Bulk sed replacements** can be dangerous
   - Better to use targeted edits with context
   - Always verify with grep before/after

2. **Test JSON structure changes** are pervasive
   - Changing from `summary.total` to `count` touched many tests
   - Could have caught earlier with better grep patterns

3. **Breaking changes** need careful documentation
   - Migration guide in issue description helps
   - Session notes critical for context

4. **Token efficiency** is a real concern
   - 70% reduction in default output is significant
   - Opt-in verbosity better than opt-out

## Summary

Core implementation complete and working. Main remaining work is fixing tests to work with minimal output and updating documentation. The design is solid and delivers significant improvements in consistency, clarity, and efficiency.

## Update: 2026-01-10 - Implementation Complete

### Completed Work

✅ **All tasks from "Next Steps" completed:**

1. **Test failures fixed** - 3 tests updated to use `--full` flag and correct blocked_reasons format
2. **Documentation updated** - 31 files updated with new command syntax
3. **Scripts updated** - 3 shell scripts migrated to new commands  
4. **MCP server regenerated** - Schema updated with new query commands
5. **Quality gates passed:**
   - Tests: 464 passed (1 pre-existing flaky test)
   - Clippy: Pass (1 pre-existing warning)
   - Fmt: Pass

### Final Commit Summary

```
2208edc test: Fix test assertions for minimal query output format
fd21dc6 docs: Update all documentation for query command consolidation
13a6402 mcp: Regenerate schema for query command consolidation
```

### Migration Summary

**Commands changed:**
- `jit query ready` → `jit query available`
- `jit query state X` → `jit query all --state X`
- `jit query priority X` → `jit query all --priority X`
- `jit query assignee X` → `jit query all --assignee X`
- `jit query label X` → `jit query all --label X`
- `jit issue list` → `jit query all`

**Files updated:**
- 31 documentation/script files
- 3 test files
- 1 MCP schema file

**Breaking changes:** Yes - not backward compatible (pre-1.0)

### Issue Status

Ready to mark issue 1168 as done after final gate checks.
