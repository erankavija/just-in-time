# Implementation Plan: Unified Issue Update Command

**Issue:** f5ce80bc - Implement bulk operations support  
**Epic:** production-stability  
**Milestone:** v1.0  

## Overview

Unify single-issue and multi-issue update operations under one command: `jit issue update`. The command accepts either an explicit issue ID (single issue mode) or a filter query (batch mode), making batch operations a natural extension of the familiar update command.

**Key Design Principle:**
> An issue ID is just a shorthand for a filter: `update <id>` ≡ `update --filter "id:<id>"`

**Examples:**
```bash
# Single issue (explicit ID)
jit issue update 44104a30 --state done

# Multiple issues (filter query)
jit issue update --filter "label:milestone:v0.9" --state done
```

This design eliminates the asymmetry between single and batch operations while maintaining clarity and safety.

## Goals

Enable efficient updates for both individual and multiple issues:
- Single issue updates (existing behavior, enhanced)
- Batch state transitions across filtered issues
- Batch label management (add/remove to many issues)
- Batch assignments to agents
- Batch priority adjustments
- Workspace cleanup and reorganization

All through one unified command interface.

## Design Philosophy

### Atomicity Model: "Best Effort" with Per-Issue Atomicity

**Why NOT "all-or-nothing" transactions?**
1. File-based storage doesn't support cross-file transactions
2. One failed issue would block 100 successful updates
3. Users lose all work on any error
4. Impractical for large issue sets

**Chosen approach:**
- ✅ Each issue update is atomic (temp file + rename)
- ✅ Operations are serialized (no partial writes)
- ✅ Comprehensive error reporting (which succeeded, which failed)
- ✅ Users can retry just the failures
- ❌ No cross-issue transaction guarantee

**This matches our pattern from dependency bulk operations (44104a30).**

## Command Design

### Unified Update Command

```bash
# Single issue mode (ID provided)
jit issue update <ID> [OPTIONS]

# Batch mode (filter provided)
jit issue update --filter <QUERY> [OPTIONS]

Options:
  --filter QUERY         Boolean query for issue selection (batch mode)
  --state STATE          Change state to STATE
  --label LABEL          Add label (repeatable)
  --remove-label LABEL   Remove label (repeatable)
  --assignee ASSIGNEE    Set assignee
  --unassign             Clear assignee
  --priority PRIORITY    Set priority
  --dry-run              Preview without applying (default for >10 matches)
  --yes                  Skip confirmation prompt
  --json                 JSON output
```

**Mutual Exclusivity:**
- ID and `--filter` are mutually exclusive
- Must provide either ID or `--filter`, not both
- Clear error message if neither or both provided

**Mental Model:**
> Issue ID is shorthand for a precise filter: `update <id>` ≡ `update --filter "id:<id>"`

**Examples:**
```bash
# Single issue (current behavior, unchanged)
jit issue update 44104a30 --state done --label milestone:v1.0

# Multiple issues (new capability)
jit issue update --filter "state:ready label:epic:auth" --state in_progress

# All high-priority unassigned
jit issue update --filter "priority:high unassigned" --assignee agent:worker-1
```

### Query Language (Simple Boolean)

**Supported filters:**
- `state:STATE` - Filter by state (ready, in_progress, done, etc.)
- `label:PATTERN` - Filter by label (exact or wildcard: `epic:*`, `milestone:v1.0`)
- `priority:PRIORITY` - Filter by priority (low, normal, high, critical)
- `assignee:ASSIGNEE` - Filter by assignee
- `unassigned` - Issues with no assignee
- `blocked` - Issues with blocking dependencies

**Boolean operations:**
- **AND** (implicit): `state:ready label:epic:auth` (space-separated)
- **OR** (explicit): `state:ready OR state:in_progress`
- **Parentheses**: `(priority:high OR priority:critical) AND state:ready`
- **NOT**: `state:ready NOT label:blocked`

**Examples:**
```bash
# All ready tasks in auth epic
jit issue update --filter "state:ready label:epic:auth label:type:task" --state in_progress

# High or critical priority issues
jit issue update --filter "priority:high OR priority:critical" --label urgent

# Ready issues not blocked
jit issue update --filter "state:ready NOT blocked" --assignee agent:worker-1

# Complex query
jit issue update --filter "(state:ready OR state:in_progress) AND label:milestone:v1.0 NOT assignee:*" --priority high
```

## Safety Features

### 1. Dry-Run Mode

**Default behavior:**
- Automatically enabled if >10 issues matched
- Shows preview of changes
- Requires `--yes` to proceed

```bash
$ jit bulk-update --state done --filter "label:milestone:v0.9"

Would modify 47 issues:
  State changes (47):
    • 01abc123 Feature A: in_progress → done
    • 02def456 Feature B: ready → done
    • 03ghi789 Feature C: gated → done  ⚠️  Warning: 2 gates pending
    ...

Run with --yes to apply changes
```

### 2. Validation Checks

**Pre-flight validation:**
- Query syntax validation
- Filter produces results
- Operations are valid for matched issues

**Per-issue validation:**
- State transitions respect dependencies (blocked issues can't go to ready/done)
- Gate requirements checked (can't transition to done with failed gates)
- Label hierarchy constraints enforced
- Type hierarchy rules respected

### 3. Confirmation Prompts

**Interactive prompt for destructive operations (when >10 issues):**
```bash
About to modify 47 issues. Continue? [y/N]
```

**Skip with `--yes` flag for automation:**
```bash
jit issue update --filter "epic:auth" --state done --yes
```

## Single vs Batch Mode Behavior

### Mode Detection

```rust
pub enum UpdateMode {
    Single(String),           // ID provided
    Batch(QueryFilter),       // --filter provided
}

// Validation
if id.is_some() && filter.is_some() {
    return Err("Cannot specify both ID and --filter");
}
if id.is_none() && filter.is_none() {
    return Err("Must specify either ID or --filter");
}

let mode = if let Some(id) = id {
    UpdateMode::Single(id)
} else if let Some(filter) = filter {
    UpdateMode::Batch(QueryFilter::parse(&filter)?)
} else {
    unreachable!() // Validated above
};
```

### Behavioral Differences

| Behavior | Single Mode | Batch Mode |
|----------|-------------|------------|
| **Dry-run** | Never automatic | Auto if >10 matches |
| **Confirmation** | Never required | Required if >10 matches (unless --yes) |
| **Error handling** | Fail immediately | Best effort, report all |
| **Output** | "Updated issue X" | Summary with counts |
| **Return code** | 0=success, 1=error | 0=any success, 1=all failed |

### Safety Guardrails (Batch Mode Only)

- Auto dry-run for >10 matches
- Confirmation prompt required (unless `--yes`)
- Clear preview showing impact
- Per-issue validation before applying

## Result Reporting

### Data Structure

```rust
#[derive(Debug, Serialize)]
pub struct BulkUpdateResult {
    pub matched: Vec<String>,           // IDs that matched filter
    pub modified: Vec<String>,          // Successfully updated
    pub skipped: Vec<(String, String)>, // (id, reason)
    pub errors: Vec<(String, String)>,  // (id, error message)
    pub summary: BulkUpdateSummary,
}

#[derive(Debug, Serialize)]
pub struct BulkUpdateSummary {
    pub total_matched: usize,
    pub total_modified: usize,
    pub total_skipped: usize,
    pub total_errors: usize,
}
```

### Human-Readable Output

```bash
✓ Modified 95 issues
ℹ Skipped 3 issues:
  • abc123: Already in state 'done'
  • def456: Already has label 'milestone:v1.0'
  • ghi789: No changes needed

✗ Failed 2 issues:
  • jkl012: Blocked by dependencies [mno345, pqr678]
  • stu901: Gate 'tests' is pending (required for done state)

Summary: 95/100 succeeded (95%)
```

### JSON Output

```json
{
  "success": true,
  "data": {
    "matched": ["abc123", "def456", ...],
    "modified": ["abc123", "def456", ...],
    "skipped": [
      ["ghi789", "Already in state 'done'"]
    ],
    "errors": [
      ["jkl012", "Blocked by dependencies"]
    ],
    "summary": {
      "total_matched": 100,
      "total_modified": 95,
      "total_skipped": 3,
      "total_errors": 2
    }
  },
  "metadata": {
    "command": "bulk-update",
    "timestamp": "2025-12-30T00:00:00Z"
  }
}
```

## Supported Operations

### 1. State Changes

```bash
# Single issue
jit issue update 44104a30 --state in_progress

# Batch: Mark all ready tasks in epic as in-progress
jit issue update --filter "state:ready label:epic:auth label:type:task" --state in_progress

# Batch: Complete entire milestone
jit issue update --filter "label:milestone:v0.9" --state done --yes
```

**Validation:**
- Cannot transition blocked issues to ready/done
- Cannot transition to done if gates pending/failed
- Respects state machine rules

### 2. Label Management

```bash
# Single issue
jit issue update 44104a30 --label milestone:v1.1

# Batch: Add label to all issues
jit issue update --filter "state:ready OR state:in_progress" --label milestone:v1.1

# Batch: Remove deprecated label
jit issue update --filter "label:old-epic:deprecated" --remove-label old-epic:deprecated

# Batch: Multiple label operations
jit issue update --filter "state:done label:milestone:v0.9" \
  --label milestone:v1.0 \
  --remove-label milestone:v0.9
```

**Validation:**
- Label hierarchy constraints enforced
- Type hierarchy rules respected
- Orphan detection (if enabled)

### 3. Assignment Operations

```bash
# Single issue
jit issue update 44104a30 --assignee agent:worker-1

# Batch: Assign all ready high-priority tasks to agent
jit issue update --filter "state:ready priority:high unassigned" --assignee agent:worker-1

# Batch: Unassign stalled issues
jit issue update --filter "state:in_progress assignee:agent:stalled-worker" --unassign

# Batch: Reassign from one agent to another
jit issue update --filter "assignee:agent:old-worker" --assignee agent:new-worker
```

**Validation:**
- Cannot assign already-assigned issues (use --force to override)
- Claim semantics respected (atomic claim vs assignment)

### 4. Priority Changes

```bash
# Single issue
jit issue update 44104a30 --priority high

# Batch: Bump priority for milestone issues
jit issue update --filter "label:milestone:v1.0 priority:normal" --priority high

# Batch: Downgrade old issues
jit issue update --filter "state:done label:milestone:v0.8" --priority low
```

**Validation:**
- Always safe (cosmetic change)

## Implementation Phases

### Phase 1: Query Engine (2-3 hours)

**File:** `crates/jit/src/query/mod.rs`

```rust
pub struct QueryFilter {
    pub conditions: Vec<QueryCondition>,
    pub operator: BooleanOp,
}

pub enum QueryCondition {
    State(State),
    Label(String),  // Supports wildcards: epic:*
    Priority(Priority),
    Assignee(String),
    Unassigned,
    Blocked,
}

pub enum BooleanOp {
    And,
    Or,
    Not,
}

impl QueryFilter {
    pub fn parse(query: &str) -> Result<Self>;
    pub fn matches(&self, issue: &Issue) -> bool;
}
```

**Tests:**
- Parse simple queries
- Parse complex queries with AND/OR/NOT
- Parse queries with parentheses
- Match issues against filters
- Wildcard label matching

### Phase 2: Dry-Run Preview (1-2 hours)

**File:** `crates/jit/src/commands/bulk.rs`

```rust
pub fn preview_bulk_update(
    &self,
    filter: &QueryFilter,
    operations: &UpdateOperations,
) -> Result<BulkUpdatePreview> {
    let matched = self.query_issues(filter)?;
    
    let mut preview = BulkUpdatePreview::default();
    
    for issue_id in matched {
        let issue = self.storage.load_issue(&issue_id)?;
        let changes = compute_changes(&issue, operations);
        preview.add(issue, changes);
    }
    
    Ok(preview)
}
```

**Output:**
- List of matched issues
- Show what would change per issue
- Warning for risky operations
- Require confirmation

### Phase 3: Bulk Update Execution (3-4 hours)

**File:** `crates/jit/src/commands/bulk.rs`

```rust
pub fn bulk_update(
    &self,
    filter: &QueryFilter,
    operations: &UpdateOperations,
    dry_run: bool,
    force: bool,
) -> Result<BulkUpdateResult> {
    let matched = self.query_issues(filter)?;
    
    // Show preview if dry-run or large batch
    if dry_run || (!force && matched.len() > 10) {
        return Ok(self.preview_bulk_update(filter, operations)?);
    }
    
    let mut result = BulkUpdateResult::default();
    result.matched = matched.clone();
    
    for issue_id in matched {
        match self.apply_operations(&issue_id, operations) {
            Ok(modified) => {
                if modified {
                    result.modified.push(issue_id);
                } else {
                    result.skipped.push((issue_id, "No changes needed".to_string()));
                }
            }
            Err(e) => {
                result.errors.push((issue_id, e.to_string()));
            }
        }
    }
    
    result.compute_summary();
    Ok(result)
}

fn apply_operations(
    &self,
    issue_id: &str,
    operations: &UpdateOperations,
) -> Result<bool> {
    let mut issue = self.storage.load_issue(issue_id)?;
    let mut modified = false;
    
    // Apply state change
    if let Some(new_state) = operations.state {
        if issue.state != new_state {
            self.validate_state_transition(&issue, new_state)?;
            issue.state = new_state;
            modified = true;
        }
    }
    
    // Apply label additions
    for label in &operations.add_labels {
        if !issue.labels.contains(label) {
            issue.labels.push(label.clone());
            modified = true;
        }
    }
    
    // Apply label removals
    for label in &operations.remove_labels {
        if issue.labels.contains(label) {
            issue.labels.retain(|l| l != label);
            modified = true;
        }
    }
    
    // Apply assignee change
    if let Some(ref assignee) = operations.assignee {
        if issue.assignee.as_ref() != Some(assignee) {
            issue.assignee = Some(assignee.clone());
            modified = true;
        }
    } else if operations.unassign && issue.assignee.is_some() {
        issue.assignee = None;
        modified = true;
    }
    
    // Apply priority change
    if let Some(priority) = operations.priority {
        if issue.priority != priority {
            issue.priority = priority;
            modified = true;
        }
    }
    
    if modified {
        self.storage.save_issue(&issue)?;
        // Log events for significant changes
        if operations.state.is_some() {
            let event = Event::new_issue_state_changed(
                issue.id.clone(),
                // Note: We'd need to track old state for proper event logging
            );
            self.storage.append_event(&event)?;
        }
    }
    
    Ok(modified)
}
```

### Phase 4: CLI Integration (1-2 hours)

**File:** `crates/jit/src/cli.rs`

```rust
/// Update an issue or multiple issues
Update {
    /// Issue ID (for single issue mode)
    id: Option<String>,
    
    /// Boolean query filter (for batch mode)
    /// Mutually exclusive with ID
    #[arg(long, conflicts_with = "id")]
    filter: Option<String>,
    
    /// Change state
    #[arg(long)]
    state: Option<String>,
    
    /// Add label (repeatable)
    #[arg(long)]
    label: Vec<String>,
    
    /// Remove label (repeatable)
    #[arg(long)]
    remove_label: Vec<String>,
    
    /// Set assignee
    #[arg(long)]
    assignee: Option<String>,
    
    /// Clear assignee
    #[arg(long)]
    unassign: bool,
    
    /// Set priority
    #[arg(long)]
    priority: Option<String>,
    
    /// Preview without applying (batch mode)
    #[arg(long)]
    dry_run: bool,
    
    /// Skip confirmation prompt (batch mode)
    #[arg(long)]
    yes: bool,
    
    #[arg(long)]
    json: bool,
},
```

**File:** `crates/jit/src/main.rs`

```rust
IssueCommands::Update {
    id,
    filter,
    state,
    label,
    remove_label,
    assignee,
    unassign,
    priority,
    dry_run,
    yes,
    json,
} => {
    // Validate: exactly one of ID or filter
    if id.is_none() && filter.is_none() {
        return Err(anyhow!("Must specify either ID or --filter"));
    }
    if id.is_some() && filter.is_some() {
        return Err(anyhow!("Cannot specify both ID and --filter"));
    }
    
    let operations = UpdateOperations {
        state: state.map(|s| parse_state(&s)).transpose()?,
        add_labels: label,
        remove_labels: remove_label,
        assignee,
        unassign,
        priority: priority.map(|p| parse_priority(&p)).transpose()?,
    };
    
    let result = if let Some(id) = id {
        // Single issue mode
        executor.update_issue_single(&id, &operations)?
    } else if let Some(filter_str) = filter {
        // Batch mode
        let filter = QueryFilter::parse(&filter_str)?;
        executor.update_issue_batch(&filter, &operations, dry_run, yes)?
    } else {
        unreachable!() // Validated above
    };
    
    if json {
        let response = JsonOutput::success(result, "issue update");
        println!("{}", response.to_json_string()?);
    } else {
        print_update_result(&result);
    }
    
    ExitCode::SUCCESS
}
```

### Phase 5: Comprehensive Testing (3-4 hours)

**File:** `crates/jit/tests/bulk_update_tests.rs`

```rust
#[test]
fn test_bulk_update_state_simple_query() {
    let h = TestHarness::new();
    let id1 = h.create_ready_issue("Task 1");
    let id2 = h.create_ready_issue("Task 2");
    
    let filter = QueryFilter::parse("state:ready").unwrap();
    let ops = UpdateOperations {
        state: Some(State::InProgress),
        ..Default::default()
    };
    
    let result = h.executor.bulk_update(&filter, &ops, false, true).unwrap();
    
    assert_eq!(result.modified.len(), 2);
    assert!(result.errors.is_empty());
}

#[test]
fn test_bulk_update_with_validation_errors() {
    let h = TestHarness::new();
    let blocked = h.create_blocked_issue("Blocked");
    let ready = h.create_ready_issue("Ready");
    
    let filter = QueryFilter::parse("state:*").unwrap();
    let ops = UpdateOperations {
        state: Some(State::Done),
        ..Default::default()
    };
    
    let result = h.executor.bulk_update(&filter, &ops, false, true).unwrap();
    
    assert_eq!(result.modified.len(), 1);  // Only ready succeeded
    assert_eq!(result.errors.len(), 1);    // Blocked failed
    assert!(result.errors[0].1.contains("blocked"));
}

#[test]
fn test_bulk_update_labels() { /* ... */ }

#[test]
fn test_bulk_update_dry_run() { /* ... */ }

#[test]
fn test_query_filter_and_operation() { /* ... */ }

#[test]
fn test_query_filter_or_operation() { /* ... */ }

#[test]
fn test_query_filter_wildcard_labels() { /* ... */ }
```

**Property-based tests:**
```rust
proptest! {
    #[test]
    fn test_bulk_update_never_corrupts_issues(
        operations in arb_update_operations(),
        filter in arb_query_filter(),
    ) {
        // Generate random operations and filters
        // Apply bulk update
        // Verify all issues are still valid
        // Verify no data loss
    }
}
```

## Error Handling

### Query Parse Errors

```bash
$ jit bulk-update --state done --filter "invalid syntax !! @#"

Error: Invalid query syntax
  at position 15: unexpected token '!!'

Hint: Use boolean operators: AND, OR, NOT
      Supported filters: state:, label:, priority:, assignee:, blocked, unassigned
```

### Validation Errors (Per-Issue)

**Blocked issues:**
```
✗ Cannot transition abc123 to 'done': blocked by dependencies [def456, ghi789]
```

**Gate failures:**
```
✗ Cannot transition abc123 to 'done': gate 'tests' is pending
```

**Label hierarchy violations:**
```
✗ Cannot add label 'type:epic' to abc123: conflicts with existing 'type:task'
```

### Partial Failures

**Always continue processing:**
- One error doesn't stop the entire operation
- All successful updates are committed
- Failed issues are reported with reasons
- Users can fix issues and retry with same filter

## Performance Considerations

### Batching Strategy

**Current approach:**
- Load and update issues one at a time
- Each issue gets atomic file operation
- Simple, correct, predictable

**Optimization (if needed later):**
- Batch-load matched issues (single directory scan)
- Process in memory
- Batch-write results
- Trade-off: More memory, faster execution

**Recommendation:** Start simple, optimize if proven bottleneck.

### Query Optimization

**Filter evaluation order:**
1. State filters (fastest, indexed in memory)
2. Label filters (fast, in-memory)
3. Priority/assignee filters (fast, in-memory)
4. Dependency-based filters (slower, requires graph traversal)

**Early termination:**
- Count matches before executing
- Warn if >1000 issues matched
- Suggest more specific query

## Security & Safety

### Dangerous Operations

**State transitions bypassing gates:**
- ⚠️  Bulk update to `done` state bypasses gate requirements
- **Mitigation:** Add `--force-gates` flag (explicit opt-in)
- **Default:** Respect gate requirements, skip issues with pending gates

**Mass deletions:**
- ❌ **Not supported in bulk operations** (too dangerous)
- Require explicit `jit issue delete` per issue

**Mass label removal:**
- ⚠️  Could break hierarchy or remove critical labels
- **Mitigation:** Dry-run preview shows impact
- Require confirmation for >10 issues

### Audit Trail

**Event logging:**
- Log bulk operation start
- Log per-issue state changes (existing events)
- Log bulk operation summary

```jsonl
{"type":"bulk_update_started","filter":"state:ready","operations":"state:done","matched_count":47}
{"type":"issue_state_changed","issue_id":"abc123","from":"ready","to":"done"}
...
{"type":"bulk_update_completed","modified":45,"errors":2,"duration_ms":1234}
```

## Success Criteria

✅ Query language supports AND/OR/NOT operations  
✅ Dry-run mode shows preview for >10 issues  
✅ Per-issue atomic file operations  
✅ Comprehensive error reporting (modified/skipped/errors)  
✅ State changes respect dependency graph  
✅ Gate requirements enforced (unless --force-gates)  
✅ Label operations respect hierarchy  
✅ JSON output format consistent  
✅ Human-readable output with summaries  
✅ Confirmation prompts for large operations  
✅ Property-based tests for safety  
✅ All CLI commands documented with examples  

## Example Workflows

### 1. Complete Milestone

```bash
# Preview (single issue)
jit issue update 44104a30 --state done --dry-run

# Preview (batch)
jit issue update --filter "label:milestone:v0.9" --state done --dry-run

# Apply (batch)
jit issue update --filter "label:milestone:v0.9" --state done --yes
```

### 2. Reassign Stalled Work

```bash
# Find stalled issues (separate query)
jit query stalled --json | jq -r '.data.issues[].id'

# Unassign (batch)
jit issue update --filter "state:in_progress assignee:agent:stalled" --unassign

# Reassign high-priority to new agent (batch)
jit issue update --filter "state:ready priority:high unassigned" --assignee agent:new-worker
```

### 3. Workspace Cleanup

```bash
# Archive old completed work (batch)
jit issue update --filter "state:done label:milestone:v0.8" --state archived

# Remove deprecated labels (batch)
jit issue update --filter "label:old-component:*" --remove-label old-component:deprecated
```

### 4. Milestone Preparation

```bash
# Tag all ready issues for next milestone (batch)
jit issue update --filter "state:ready OR state:in_progress" --label milestone:v1.1

# Bump priority for milestone blockers (batch)
jit issue update --filter "label:milestone:v1.1 label:blocker" --priority critical
```

## Dependencies

**New crates (optional):**
- `nom` or `pest` for query parsing (lightweight parser combinator)
- OR: Hand-written recursive descent parser (no dependencies)

**Recommendation:** Start with simple hand-written parser, add parser library if query language becomes complex.

## Estimated Effort

- **Phase 1** (Query Engine): 2-3 hours
- **Phase 2** (Dry-Run): 1-2 hours
- **Phase 3** (Execution): 3-4 hours
- **Phase 4** (CLI Integration): 1-2 hours
- **Phase 5** (Testing): 3-4 hours
- **Documentation**: 1 hour

**Total: 11-16 hours**

## Related Issues

- **44104a30**: Single-issue bulk operations (DONE) - provides pattern for result reporting
- **32f804f1**: CLI ergonomics improvements - bulk operations is item #8
- **713ff59d**: Metrics reporting - could use query engine for filtering

## Notes

This design prioritizes **pragmatism and safety** over theoretical purity:
- Per-issue atomicity matches file-based storage model
- Best-effort approach maximizes successful operations
- Clear error reporting enables retry workflows
- Dry-run and confirmations prevent accidents
- Consistent with existing dependency bulk operations pattern

The query language is intentionally simple - complex queries can be composed from multiple simpler operations.
