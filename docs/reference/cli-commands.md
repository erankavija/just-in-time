# CLI Commands Reference

> **Status:** Draft - Story 5bad7437  
> **Diátaxis Type:** Reference

## Global Options

### `--json`
Output data in JSON format for machine consumption.

### `--quiet` / `-q`
Suppress non-essential output (success messages, headers, warnings). Preserves data output and errors.

**What --quiet suppresses:**
- Success messages ("Created issue...", "Updated...")
- Informational headers ("Ready issues:", "Total: 5")
- Warnings and validation hints
- Progress indicators

**What --quiet preserves:**
- Issue lists and query results
- Issue IDs (from `issue create`)
- Essential data output
- Errors (always shown to stderr)

## Issue Commands

### Bulk Operations

Update multiple issues with a single command using `--filter`:

```bash
# Batch state transitions - complete entire milestone
jit issue update --filter "label:milestone:v1.0 AND state:ready" --state done

# Batch label management - tag all backend tasks
jit issue update --filter "label:component:backend" --label "needs-review:true"

# Batch reassignment - hand off work to another agent
jit issue update --filter "assignee:agent:worker-1" --assignee "agent:worker-2"

# Batch priority adjustment - escalate critical path
jit issue update --filter "label:epic:auth" --priority critical

# Complex queries with AND/OR/NOT
jit issue update --filter "state:ready AND NOT assignee:* AND priority:high" --assignee "agent:worker-1"

# Remove labels from multiple issues
jit issue update --filter "label:milestone:v0.9" --remove-label "milestone:v0.9"
```

**Bulk operation semantics:**
- **Literal updates:** Sets exactly what you specify (no auto-transitions)
- **Atomic per-issue:** Each issue update is atomic (write temp + rename)
- **Predictable:** Safer for large-scale changes
- **Clear reporting:** Shows modified/skipped/error counts

**When to use bulk vs single-issue update:**
- **Single-issue:** Smart orchestration with prechecks, postchecks, auto-transitions
- **Bulk:** Explicit, predictable batch changes across many issues

<!-- Additional jit issue commands -->

## Gate Commands

<!-- Complete jit gate reference -->

## Dependency Commands

<!-- jit dep add/rm -->

## Query Commands

<!-- jit query ready, blocked, etc. -->

## Document Commands

<!-- jit doc add/show/list/archive -->

## Graph Commands

<!-- jit graph show/export/roots/downstream -->

## Status and Validation

<!-- jit status, jit validate -->

## Configuration

<!-- jit config commands -->

## Scripting and Automation

### Quiet Mode for Scripts

Use `--quiet` to suppress non-essential output:

```bash
# Create issue and capture ID
ISSUE_ID=$(jit issue create --title "Fix login bug" --orphan --quiet)
echo "Created issue: $ISSUE_ID"

# Update without confirmation messages
jit issue update $ISSUE_ID --state done --quiet

# Pipe to other commands without headers
jit issue list --quiet | grep "Bug"
jit query ready --quiet | head -5

# Dependency operations silently succeed
jit dep add $ISSUE1 $ISSUE2 --quiet
```

### JSON Mode for Parsing

Combine `--quiet` with `--json` for machine-readable output:

```bash
# Parse with jq
ISSUE_ID=$(jit issue create --title "Add feature" --orphan --quiet --json | jq -r '.data.id')

# Extract specific fields
jit issue show $ISSUE_ID --json --quiet | jq -r '.data.title'

# Process lists
jit issue list --json --quiet | jq -r '.data.issues[] | select(.priority == "High") | .id'

# Query and filter
jit query ready --json --quiet | jq -r '.data.issues[0].id'

# Get status counts
jit status --json --quiet | jq -r '.data.summary.by_state'
```

### Graceful Pipe Handling

JIT handles broken pipes gracefully (no panics):

```bash
# Safe to pipe to head/tail
jit issue list | head -1          # Clean exit, no error
jit query ready --quiet | head -3  # Works perfectly

# Chain with grep
jit issue list --quiet | grep -i "bug"

# Use with while loops
jit query ready --quiet | while read -r line; do
  echo "Processing: $line"
done
```

### Example Scripts

**Bulk issue creation:**
```bash
#!/bin/bash
# Create multiple issues from a file

while IFS=',' read -r title priority component; do
  ID=$(jit issue create \
    --title "$title" \
    --priority "$priority" \
    --label "component:$component" \
    --orphan \
    --quiet)
  echo "Created: $ID - $title"
done < issues.csv
```

**Automated workflow:**
```bash
#!/bin/bash
# Find ready issues and process them

jit query ready --quiet | while read -r line; do
  # Extract issue ID (first field)
  ISSUE_ID=$(echo "$line" | awk '{print $1}')
  
  # Claim for automation
  jit issue claim "$ISSUE_ID" "bot:automation" --quiet
  
  # Process...
  echo "Processing $ISSUE_ID"
  
  # Mark done
  jit issue update "$ISSUE_ID" --state done --quiet
done
```

**CI/CD integration:**
```bash
#!/bin/bash
# Pass gates automatically from CI

ISSUE_ID=$1

# Run tests
if cargo test; then
  jit gate pass "$ISSUE_ID" tests --quiet
  echo "✓ Tests passed for $ISSUE_ID"
else
  jit gate fail "$ISSUE_ID" tests --quiet
  echo "✗ Tests failed for $ISSUE_ID"
  exit 1
fi

# Run linter
if cargo clippy -- -D warnings; then
  jit gate pass "$ISSUE_ID" clippy --quiet
  echo "✓ Clippy passed for $ISSUE_ID"
else
  jit gate fail "$ISSUE_ID" clippy --quiet
  echo "✗ Clippy failed for $ISSUE_ID"
  exit 1
fi
```

**Status reporting:**
```bash
#!/bin/bash
# Generate daily status report

echo "=== JIT Status Report ==="
echo "Date: $(date)"
echo ""

# Get counts
READY=$(jit query ready --json --quiet | jq -r '.data.count')
IN_PROGRESS=$(jit query state in_progress --json --quiet | jq -r '.data.count')
BLOCKED=$(jit query blocked --json --quiet | jq -r '.data.count')
DONE_TODAY=$(jit events query --event-type state_changed --limit 100 --json | \
  jq -r '[.[] | select(.new_state == "done")] | length')

echo "Ready: $READY"
echo "In Progress: $IN_PROGRESS"
echo "Blocked: $BLOCKED"
echo "Completed Today: $DONE_TODAY"
```

### Exit Codes

JIT uses standard exit codes for scripting:

- `0` - Success
- `1` - Error (invalid input, file not found, etc.)
- `2` - Validation error (cycle detected, gate failed, etc.)

```bash
# Check exit codes
if jit issue create --title "Test" --orphan --quiet; then
  echo "Created successfully"
else
  echo "Failed with exit code: $?"
fi
```
