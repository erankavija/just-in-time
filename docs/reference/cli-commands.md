# CLI Commands Reference

> **Diátaxis Type:** Reference

## MCP Tools Reference

JIT provides Model Context Protocol (MCP) tools for AI agent integration. These tools wrap CLI commands with standardized interfaces.

### What is MCP?

MCP (Model Context Protocol) is a standard interface for AI agents to interact with tools. JIT's MCP server provides all CLI functionality through structured tool calls.

**Key benefits:**
- Standardized parameter names and formats
- Structured JSON responses
- Type-safe tool definitions
- Framework-agnostic (works with any MCP client)

### Installation and Setup

```bash
# MCP server is in mcp-server/ directory
cd mcp-server

# Install dependencies
npm install

# Start server (for MCP clients)
node index.js

# Configure in your MCP client (e.g., Claude Desktop, VSCode)
# See mcp-server/README.md for client configuration
```

### Parameter Naming Convention

**MCP tool parameters match CLI exactly:**

```javascript
// CLI: jit issue create --title "..." --description "..."
jit_issue_create({
  title: "string",
  description: "string",  // Full word (consistent with CLI)
  label: ["type:task"],   // Array, singular form
  gate: ["tests"],        // Array, singular form
  priority: "high"
})
```

**Key conventions:**
- Parameters use full words (description, not desc)
- Arrays use singular form (label not labels, gate not gates)
- Flags become boolean properties (json: true, quiet: true)
- Hyphens become underscores (add_gate not add-gate)

### Core MCP Tools

#### Issue Management

**`jit_issue_create`** - Create new issue
```javascript
{
  title: string,              // Required
  description?: string,
  label?: string[],           // ["type:task", "epic:auth"]
  gate?: string[],            // ["tests", "code-review"]
  priority?: string,          // "critical" | "high" | "normal" | "low"
  orphan?: boolean,           // Allow without type label
  json?: boolean              // Return JSON response
}
```

**`jit_issue_show`** - Get issue details
```javascript
{
  id: string,                 // Issue ID or short hash
  json?: boolean
}
```

**`jit_issue_list`** - List issues with filters
```javascript
{
  state?: string,             // "ready" | "in_progress" | "done" etc.
  assignee?: string,          // "agent:worker-1"
  priority?: string,
  json?: boolean
}
```

**`jit_issue_update`** - Modify issue
```javascript
{
  id?: string,                // Single issue mode
  filter?: string,            // Batch mode (mutually exclusive with id)
  state?: string,
  priority?: string,
  assignee?: string,
  unassign?: boolean,
  label?: string[],           // Add labels
  remove_label?: string[],
  add_gate?: string[],
  remove_gate?: string[],
  json?: boolean
}
```

**`jit_issue_claim`** - Atomically claim unassigned issue
```javascript
{
  id: string,
  assignee: string,           // "agent:copilot-session-1"
  json?: boolean
}
```

**`jit_issue_claim_next`** - Claim next ready issue by priority
```javascript
{
  assignee: string,
  filter?: string,            // Optional filter
  json?: boolean
}
```

**`jit_issue_release`** - Release issue from assignee
```javascript
{
  id: string,
  reason: string,             // "timeout" | "error" | "reassign"
  json?: boolean
}
```

**`jit_issue_reject`** - Reject issue (bypasses gates)
```javascript
{
  id: string,
  reason?: string,            // Adds resolution:* label
  json?: boolean
}
```

**`jit_issue_search`** - Full-text search
```javascript
{
  query: string,              // Search title, description, ID
  state?: string,
  assignee?: string,
  priority?: string,
  json?: boolean
}
```

**`jit_issue_breakdown`** - Break issue into subtasks
```javascript
{
  parent_id: string,
  subtask: string[],          // Subtask titles
  description?: string[],     // Optional descriptions
  json?: boolean
}
```

#### Dependencies

**`jit_dep_add`** - Add dependency (FROM depends on TO)
```javascript
{
  from_id: string,            // Issue that is blocked
  to_ids: string[],           // Dependencies required
  json?: boolean
}
```

**`jit_dep_rm`** - Remove dependency
```javascript
{
  from_id: string,
  to_ids: string[],
  json?: boolean
}
```

#### Gates

**`jit_gate_define`** - Define new gate in registry
```javascript
{
  key: string,                // Unique identifier
  title: string,
  description: string,
  stage?: string,             // "precheck" | "postcheck"
  mode?: string,              // "manual" | "auto"
  checker_command?: string,   // For automated gates
  timeout?: number,           // Seconds
  working_dir?: string,
  json?: boolean
}
```

**`jit_gate_list`** - List all gate definitions
```javascript
{
  json?: boolean
}
```

**`jit_gate_show`** - Show gate definition
```javascript
{
  key: string,
  json?: boolean
}
```

**`jit_gate_add`** - Add gates to issue
```javascript
{
  id: string,
  gate_keys: string[],        // ["tests", "clippy", "code-review"]
  json?: boolean
}
```

**`jit_gate_remove`** - Remove gate from issue
```javascript
{
  id: string,
  gate_key: string,
  json?: boolean
}
```

**`jit_gate_check`** - Run automated gate checker
```javascript
{
  id: string,
  gate_key: string,
  json?: boolean
}
```

**`jit_gate_check_all`** - Run all automated gates
```javascript
{
  id: string,
  json?: boolean
}
```

**`jit_gate_pass`** - Mark gate as passed
```javascript
{
  id: string,
  gate_key: string,
  by?: string,                // "human:alice" | "ci:github"
  json?: boolean
}
```

**`jit_gate_fail`** - Mark gate as failed
```javascript
{
  id: string,
  gate_key: string,
  by?: string,
  json?: boolean
}
```

#### Queries

**`jit_query_ready`** - Issues ready to work on
```javascript
{
  json?: boolean
}
```

**`jit_query_blocked`** - Blocked issues with reasons
```javascript
{
  json?: boolean
}
```

**`jit_query_state`** - Filter by state
```javascript
{
  state: string,              // "backlog" | "ready" | "in_progress" | etc.
  json?: boolean
}
```

**`jit_query_priority`** - Filter by priority
```javascript
{
  priority: string,           // "critical" | "high" | "normal" | "low"
  json?: boolean
}
```

**`jit_query_label`** - Filter by label pattern
```javascript
{
  pattern: string,            // "epic:auth" | "milestone:*"
  json?: boolean
}
```

**`jit_query_assignee`** - Filter by assignee
```javascript
{
  assignee: string,           // "agent:worker-1"
  json?: boolean
}
```

**`jit_query_strategic`** - Strategic issues (milestone/epic/goal)
```javascript
{
  json?: boolean
}
```

**`jit_query_closed`** - Done or rejected issues
```javascript
{
  json?: boolean
}
```

#### Graph

**`jit_graph_show`** - Show dependency tree
```javascript
{
  id?: string,                // Optional - shows all if omitted
  json?: boolean
}
```

**`jit_graph_roots`** - Find root issues (no dependencies)
```javascript
{
  json?: boolean
}
```

**`jit_graph_downstream`** - Show what's blocked by this issue
```javascript
{
  id: string,
  json?: boolean
}
```

**`jit_graph_export`** - Export graph in various formats
```javascript
{
  format: string,             // "dot" | "mermaid"
  output?: string             // File path (optional)
}
```

#### Documents

**`jit_doc_add`** - Add document reference to issue
```javascript
{
  id: string,
  path: string,
  label?: string,
  doc_type?: string,          // "design" | "implementation" | "notes"
  commit?: string,            // Git commit
  skip_scan?: boolean,
  json?: boolean
}
```

**`jit_doc_list`** - List documents for issue
```javascript
{
  id: string,
  json?: boolean
}
```

**`jit_doc_show`** - Show document content
```javascript
{
  id: string,
  path: string,
  at?: string,                // Git commit
  json?: boolean
}
```

**`jit_doc_remove`** - Remove document reference
```javascript
{
  id: string,
  path: string,
  json?: boolean
}
```

#### Status and Validation

**`jit_status`** - Overall status
```javascript
{
  json?: boolean
}
```

**`jit_validate`** - Validate repository integrity
```javascript
{
  fix?: boolean,              // Auto-fix issues
  dry_run?: boolean,          // Preview fixes
  json?: boolean
}
```

#### Search

**`jit_search`** - Full-text search across issues and documents
```javascript
{
  query: string,
  glob?: string,              // File pattern
  regex?: boolean,
  case_sensitive?: boolean,
  context?: number,           // Lines of context
  limit?: number,
  json?: boolean
}
```

### MCP Response Format

All MCP tools return structured responses:

**Success response:**
```javascript
{
  success: true,
  data: {
    id: "abc123...",
    title: "Issue title",
    state: "ready",
    // ... other fields
  }
}
```

**Error response:**
```javascript
{
  success: false,
  error: {
    message: "Issue not found",
    code: "NOT_FOUND"
  }
}
```

**List response:**
```javascript
{
  success: true,
  data: {
    issues: [...],
    count: 42
  }
}
```

### Usage Examples (JavaScript/TypeScript)

**Basic workflow:**
```typescript
// Create issue
const created = await jit_issue_create({
  title: "Implement user authentication",
  label: ["type:task", "epic:auth", "component:backend"],
  gate: ["tests", "code-review"],
  priority: "high",
  json: true
});
const issueId = created.data.id;

// Add dependencies
await jit_dep_add({
  from_id: epicId,
  to_ids: [issueId]
});

// Query ready work
const ready = await jit_query_ready({ json: true });
console.log(`${ready.data.count} issues ready`);

// Claim atomically
await jit_issue_claim({
  id: issueId,
  assignee: "agent:copilot-session-1"
});

// Do work...

// Pass gates
await jit_gate_check({ id: issueId, gate_key: "tests" });
await jit_gate_pass({ 
  id: issueId, 
  gate_key: "code-review",
  by: "human:reviewer"
});

// Complete
await jit_issue_update({ 
  id: issueId, 
  state: "done" 
});
```

**Multi-agent coordination:**
```typescript
// Agent polling loop
async function agentLoop(agentId: string) {
  while (true) {
    // Claim next ready issue atomically
    const claimed = await jit_issue_claim_next({
      assignee: `agent:${agentId}`,
      json: true
    });
    
    if (claimed.success) {
      const issueId = claimed.data.id;
      console.log(`Agent ${agentId} claimed ${issueId}`);
      
      // Do work
      await performWork(issueId);
      
      // Complete
      await jit_issue_update({ id: issueId, state: "done" });
    } else {
      // No work available
      await sleep(10000);
    }
  }
}
```

**Parallel operations:**
```typescript
// Create multiple issues in parallel
const tasks = [
  "Implement JWT utilities",
  "Add password hashing",
  "Create session management"
];

const created = await Promise.all(
  tasks.map(title => 
    jit_issue_create({
      title,
      label: ["type:task", "epic:auth"],
      gate: ["tests", "code-review"],
      json: true
    })
  )
);

const issueIds = created.map(r => r.data.id);
console.log(`Created ${issueIds.length} issues`);
```

### Efficiency Tips for Agents

**✅ Use MCP tools exclusively**
- Don't fall back to CLI/bash for efficiency
- MCP tools are optimized for structured responses
- Avoid shell parsing overhead

**✅ Parallel operations with Promise.all()**
```typescript
// Good: Parallel
await Promise.all([
  jit_gate_check({ id, gate_key: "tests" }),
  jit_gate_check({ id, gate_key: "clippy" }),
  jit_gate_check({ id, gate_key: "fmt" })
]);

// Avoid: Sequential
await jit_gate_check({ id, gate_key: "tests" });
await jit_gate_check({ id, gate_key: "clippy" });
await jit_gate_check({ id, gate_key: "fmt" });
```

**✅ Chain MCP calls**
```typescript
// Structured JSON responses are easy to chain
const ready = await jit_query_ready({ json: true });
const firstIssue = ready.data.issues[0];
await jit_issue_claim({ id: firstIssue.id, assignee: agentId });
```

**✅ Use short hashes**
```typescript
// Works with short prefixes (4+ chars)
await jit_issue_show({ id: "01abc" });  // Instead of full UUID
await jit_gate_pass({ id: "003f", gate_key: "tests" });
```

**✅ Check JSON output structure**
```typescript
// Inspect response for available fields
const issue = await jit_issue_show({ id, json: true });
console.log(issue.data);
// { id, title, state, priority, assignee, dependencies, gates_status, ... }
```

### Testing MCP Tools

```bash
# Test MCP server
cd mcp-server
npm test

# Test specific tool
node test-tool.js jit_issue_create

# Test with MCP inspector (if available)
npx @modelcontextprotocol/inspector
```

### See Also

- [MCP Server README](../../mcp-server/README.md) - Setup and configuration
- [Core Model](../concepts/core-model.md) - Understanding issues, gates, dependencies
- [How-To: Custom Gates](../how-to/custom-gates.md) - Gate usage patterns
- [Quickstart Tutorial](../tutorials/quickstart.md) - Getting started

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

### Rejecting Issues

Use `jit issue reject` to close an issue without implementation:

```bash
# Reject with reason
jit issue reject $ISSUE --reason "duplicate"

# Reject without reason
jit issue reject $ISSUE

# Quiet mode for scripting
jit issue reject $ISSUE --reason "wont-fix" --quiet
```

**Key behaviors:**
- **Bypasses gates:** Can reject from any state, even with failing gates
- **Terminal state:** Cannot transition out of Rejected
- **Optional reason:** `--reason` flag adds `resolution:*` label
- **Immediate:** No validation or gate checks

**Common rejection reasons:**
- `duplicate` - Duplicate of another issue
- `wont-fix` - Valid request, but won't implement
- `invalid` - Not a valid issue
- `out-of-scope` - Outside project scope
- `obsolete` - No longer relevant

**Example workflow:**
```bash
# Discover duplicate during work
jit issue show $ISSUE
# Found duplicate: #ABC123

# Reject with reason
jit issue reject $ISSUE --reason "duplicate"

# Query rejected issues
jit query all --state rejected --json | jq -r '.issues[] | {id, title, labels}'
```

**State transition examples:**

```bash
# From Ready → Rejected (skip work entirely)
jit issue reject $READY_ISSUE --reason "out-of-scope"

# From In Progress → Rejected (abandon in-progress work)
jit issue reject $WIP_ISSUE --reason "duplicate"

# From Gated → Rejected (bypass failing gates)
jit issue reject $GATED_ISSUE --reason "wont-fix"
# Note: This bypasses gates, unlike transitioning to Done

# Cannot transition from Done → Rejected (terminal states are final)
jit issue reject $DONE_ISSUE
# Error: Cannot transition from terminal state
```

## Gate Commands

Gates are quality checkpoints that enforce process requirements. See [How-To: Custom Gates](../how-to/custom-gates.md) for practical examples and [Core Model - Gates](../concepts/core-model.md#gates) for conceptual understanding.

### `jit gate define`

Define a new gate in the registry for reuse across issues.

**Usage:**
```bash
jit gate define <KEY> --title <TITLE> --description <DESCRIPTION> [OPTIONS]
```

**Arguments:**
- `KEY` - Unique identifier (e.g., `tests`, `code-review`, `security-scan`)

**Required Options:**
- `--title <TITLE>` - Human-readable name
- `--description <DESCRIPTION>` - What this gate checks

**Optional:**
- `--stage <STAGE>` - When gate runs: `precheck` or `postcheck` (default: `postcheck`)
- `--mode <MODE>` - How gate is checked: `manual` or `auto` (default: `manual`)
- `--checker-command <COMMAND>` - Command to run for automated gates
- `--timeout <SECONDS>` - Checker timeout in seconds (default: 300)
- `--working-dir <PATH>` - Working directory for checker (relative to repo root)

**Examples:**
```bash
# Manual code review gate
jit gate define code-review \
  --title "Code Review" \
  --description "Another developer must review code" \
  --stage postcheck \
  --mode manual

# Automated test gate
jit gate define tests \
  --title "All Tests Pass" \
  --description "Full test suite must succeed" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

# TDD precheck reminder
jit gate define tdd-reminder \
  --title "Write Tests First" \
  --description "Reminder to follow TDD practice" \
  --stage precheck \
  --mode manual
```

### `jit gate add`

Add gate requirements to an issue. Gates must be defined in registry first.

**Usage:**
```bash
jit gate add <ISSUE_ID> <GATE_KEY>...
```

**Arguments:**
- `ISSUE_ID` - Issue to add gates to
- `GATE_KEY...` - One or more gate keys from registry

**Examples:**
```bash
# Add single gate
jit gate add abc123 code-review

# Add multiple gates at once
jit gate add abc123 tests clippy fmt

# Add gate to multiple issues with filter
jit issue update --filter "label:epic:auth" --add-gate tests
```

### `jit gate list`

List all gates defined in the registry.

**Usage:**
```bash
jit gate list [--json] [--quiet]
```

**Output:**
```
Gates:
  tests - All Tests Pass (Postcheck, Auto)
  clippy - Clippy Lints Pass (Postcheck, Auto)
  fmt - Code Formatted (Postcheck, Auto)
  code-review - Code Review (Postcheck, Manual)
  tdd-reminder - TDD Reminder (Precheck, Manual)
```

### `jit gate show`

Show detailed information about a specific gate definition.

**Usage:**
```bash
jit gate show <GATE_KEY> [--json] [--quiet]
```

**Example:**
```bash
$ jit gate show tests

Gate: tests
  Title: All Tests Pass
  Description: Full test suite must pass
  Stage: Postcheck
  Mode: Auto
  Checker:
    Command: cargo test --lib
    Timeout: 300s
```

### `jit gate check`

Run an automated gate checker for a specific issue.

**Usage:**
```bash
jit gate check <ISSUE_ID> <GATE_KEY> [--json] [--quiet]
```

**Behavior:**
- Runs the gate's checker command
- Updates gate status based on exit code (0 = passed, non-zero = failed)
- Only works for automated gates (manual gates cannot be checked)

**Examples:**
```bash
# Check single gate
jit gate check abc123 tests
# ✓ tests passed (exit code 0)

# Check fails
jit gate check abc123 clippy
# ✗ clippy failed (exit code 1)
# Output: found 3 warnings...
```

### `jit gate check-all`

Run all automated gates for an issue.

**Usage:**
```bash
jit gate check-all <ISSUE_ID> [--json]
```

**Behavior:**
- Runs checker commands for all automated gates on the issue
- Manual gates are skipped
- Reports summary of results

**Example:**
```bash
$ jit gate check-all abc123

Running 3 automated gate(s)...
✓ tests passed
✓ fmt passed
✗ clippy failed

Summary: 2 passed, 1 failed
```

### `jit gate pass`

Manually mark a gate as passed.

**Usage:**
```bash
jit gate pass <ISSUE_ID> <GATE_KEY> [--by <WHO>]
```

**Options:**
- `--by <WHO>` - Record who passed the gate (e.g., `human:alice`, `ci:github-actions`)

**Examples:**
```bash
# Pass manual gate
jit gate pass abc123 code-review --by "human:alice"

# Pass without attribution
jit gate pass abc123 tdd-reminder

# Pass automated gate manually (override checker)
jit gate pass abc123 tests --by "human:admin"
```

**Behavior:**
- Updates gate status to `passed`
- Records who passed it and timestamp
- If this was the last blocking gate, issue auto-transitions from `gated → done`

### `jit gate fail`

Manually mark a gate as failed.

**Usage:**
```bash
jit gate fail <ISSUE_ID> <GATE_KEY> [--by <WHO>]
```

**Example:**
```bash
# Fail automated gate manually
jit gate fail abc123 tests --by "ci:github-actions"
```

**Note:** Typically only used for automated gates run in CI/CD. Manual gates are usually only passed, not failed.

### `jit gate remove`

Remove a gate requirement from an issue.

**Usage:**
```bash
jit issue update <ISSUE_ID> --remove-gate <GATE_KEY>
```

**Example:**
```bash
# Remove single gate
jit issue update abc123 --remove-gate code-review

# Remove multiple gates
jit issue update abc123 --remove-gate tests --remove-gate clippy
```

### Gate Status in Issue Queries

**View gate status:**
```bash
# Show all gate information for issue
jit issue show abc123 --json | jq '.data.gates_status'

# Example output:
{
  "tests": {
    "status": "passed",
    "updated_by": "ci:github-actions",
    "updated_at": "2026-01-02T10:30:00Z"
  },
  "code-review": {
    "status": "passed",
    "updated_by": "human:alice",
    "updated_at": "2026-01-02T11:00:00Z"
  }
}
```

**Find issues with specific gate status:**
```bash
# Find all gated issues (waiting for gates)
jit query all --state gated

# Use jq to filter by specific gate
jit query all --json | jq '.issues[] | select(.gates_status.tests.status == "failed")'
```

### Exit Codes

All gate commands use standard exit codes:

- `0` - Success
- `1` - Error (gate not found, issue not found, invalid input)
- `2` - Validation error (cannot add duplicate gate, etc.)
- `4` - Gate check failed (for `jit gate check` when checker returns non-zero)

## Dependency Commands

<!-- jit dep add/rm -->

## Query Commands

<!-- jit query available, blocked, etc. -->

## Document Commands

<!-- jit doc add/show/list/archive -->

## Graph Commands

<!-- jit graph deps/export/roots/downstream -->

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
jit query all --quiet | grep "Bug"
jit query available --quiet | head -5

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
jit query all --json --quiet | jq -r '.data.issues[] | select(.priority == "High") | .id'

# Query and filter
jit query available --json --quiet | jq -r '.data.issues[0].id'

# Get status counts
jit status --json --quiet | jq -r '.data.summary.by_state'
```

### Graceful Pipe Handling

JIT handles broken pipes gracefully (no panics):

```bash
# Safe to pipe to head/tail
jit query all | head -1          # Clean exit, no error
jit query available --quiet | head -3  # Works perfectly

# Chain with grep
jit query all --quiet | grep -i "bug"

# Use with while loops
jit query available --quiet | while read -r line; do
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

jit query available --quiet | while read -r line; do
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
READY=$(jit query available --json --quiet | jq -r '.data.count')
IN_PROGRESS=$(jit query all --state in_progress --json --quiet | jq -r '.data.count')
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
