# CLI Commands Reference

> **Diátaxis Type:** Reference

## CLI JSON contracts

CLI commands that accept `--json` print machine-readable JSON to stdout. Success
responses are the command payload itself, not a `{ "success": true, "data": ... }`
envelope. Commands that return objects may include a top-level `message` field
for human-readable context.

Example successful issue update (abbreviated):

```json
{
  "id": "5c581575-bef8-4ee6-be83-7598fd22b557",
  "title": "Improve state and gate blocking remediation",
  "state": "done",
  "priority": "high",
  "assignee": "agent:copilot",
  "dependencies": [],
  "gates_required": ["cargo-ci", "code-review"],
  "gates_status": {
    "cargo-ci": {
      "status": "passed",
      "updated_by": "auto:executor",
      "updated_at": "2026-04-28T18:25:16.699033997Z"
    }
  },
  "labels": ["type:task", "epic:usability"],
  "message": "Updated issue 5c581575 to Done"
}
```

Error responses use a stable top-level `error` object:

```json
{
  "error": {
    "code": "ISSUE_NOT_FOUND",
    "message": "Issue not found: abc123",
    "details": {
      "issue_id": "abc123"
    },
    "suggestions": [
      "Run 'jit query all' to see available issues",
      "Check if the issue ID is correct"
    ]
  }
}
```

Blocked state transitions use the same error envelope and add structured blocker
and remediation fields in `error.details`. This applies to `issue update --json`,
`issue claim --json`, and `issue claim-next --json`.

Dependency-blocked example:

```json
{
  "error": {
    "code": "BLOCKED",
    "message": "Cannot transition to 'ready': issue blocked by 1 incomplete dependencies",
    "details": {
      "issue_id": "blocked-work-id",
      "requested_state": "ready",
      "actual_state": "backlog",
      "blockers": [
        {
          "type": "dependency",
          "issue_id": "prerequisite-id",
          "short_id": "prereq12",
          "title": "Blocked prerequisite",
          "state": "ready"
        }
      ],
      "remediation": [
        "jit graph deps blocked-work-id",
        "jit issue show prerequisite-id"
      ]
    },
    "suggestions": [
      "jit graph deps blocked-work-id",
      "jit issue show prerequisite-id"
    ]
  }
}
```

## Command and flag aliases

A few convenience aliases exist for the names agents reach for most often. They
behave identically to their canonical forms:

| Alias | Canonical | Notes |
|-------|-----------|-------|
| `jit dependency ...` | `jit dep ...` | Dependency management commands |
| `jit document ...` | `jit doc ...` | Document reference commands |
| `jit issue list` | `jit query all` | Same filters/flags (`-s`/`-a`/`-p`/`-l`, `--full`, `--json`); identical output |
| `jit issue update <id> --add-label <label>` | `... --label <label>` | `--add-label` is an accepted alias for `--label` |

```bash
# These pairs are equivalent
jit dependency add a b      # == jit dep add a b
jit document list <id>      # == jit doc list <id>
jit issue list --json       # == jit query all --json
jit issue update <id> --add-label area:foo  # == --label area:foo
```

Gate-blocked example:

```json
{
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "Gate validation failed: Cannot transition to 'done': 1 gate(s) not passed",
    "details": {
      "issue_id": "work-id",
      "requested_state": "done",
      "actual_state": "gated",
      "blockers": [
        {
          "type": "gate",
          "gate_key": "code-review",
          "status": "pending"
        }
      ],
      "remediation": [
        "jit gate check-all work-id",
        "jit gate pass work-id code-review"
      ]
    },
    "suggestions": [
      "jit gate check-all work-id",
      "jit gate pass work-id code-review"
    ]
  }
}
```

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
  pass_context?: boolean,     // Pass issue/gate/history context to checker
  prompt?: string,            // Inline prompt for context-aware checkers
  prompt_file?: string,       // Path to prompt file (relative to repo root)
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

**`jit_gate_check`** - Show the latest recorded run for a gate
```javascript
{
  id: string,
  gate_key: string,
  json?: boolean
}
```

**`jit_gate_check_all`** - Show the latest recorded runs for all automated gates
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

**`jit_version`** - Show CLI version and local build provenance
```javascript
{
  json?: boolean
}
```

When `json` is true, the response includes `package`, `version`,
`git_commit`, `git_short_commit`, `git_dirty`, `build_profile`,
`build_timestamp`, and `target`.

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

MCP tools wrap CLI payloads in a transport envelope. The `data` field contains
the same payload shape the corresponding CLI command prints with `--json`.

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
    code: "BLOCKED",
    message: "Cannot transition to 'ready': issue blocked by 1 incomplete dependencies",
    details: {
      issue_id: "blocked-work-id",
      requested_state: "ready",
      actual_state: "backlog",
      blockers: [
        {
          type: "dependency",
          issue_id: "prerequisite-id",
          short_id: "prereq12",
          title: "Blocked prerequisite",
          state: "ready"
        }
      ],
      remediation: [
        "jit graph deps blocked-work-id",
        "jit issue show prerequisite-id"
      ]
    },
    suggestions: [
      "jit graph deps blocked-work-id",
      "jit issue show prerequisite-id"
    ]
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
const issueId = createdid;

// Add dependencies
await jit_dep_add({
  from_id: epicId,
  to_ids: [issueId]
});

// Query ready work
const ready = await jit_query_ready({ json: true });
console.log(`${readycount} issues ready`);

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
      const issueId = claimedid;
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

const issueIds = created.map(r => rid);
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
const firstIssue = readyissues[0];
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
// { id, short_id, title, state, priority, assignee, dependencies, gates, ... }
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

### Multi-Value Arguments

**Flagged arguments** (e.g., `--label`, `--gate`) support **both** comma-separated and repeated flags:

```bash
# Comma-separated (compact)
jit issue create --title "Task" --label epic:auth,type:task,component:core

# Repeated flags (explicit)
jit issue create --title "Task" --label epic:auth --label type:task --label component:core

# Mixed (also works)
jit issue create --title "Task" --label epic:auth,type:task --label component:core
```

**Applies to:** `--label`, `--gate`, `--add-gate`, `--remove-label`, `--remove-gate`, `--subtask`, `--description`, `--except`

**Positional arguments** (e.g., `jit gate add <id> <gates>...`, `jit dep add <from> <to>...`) are **space-separated only**:

```bash
# Correct: space-separated positional args
jit gate add abc123 tests clippy fmt
jit dep add epic123 task1 task2 task3

# Incorrect: comma-separated positional args (will fail)
jit gate add abc123 tests,clippy,fmt  ❌
```

This follows industry standard (cargo, kubectl, git).

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

### `--version`
Print the CLI package version plus local build provenance:

```bash
jit --version
# jit 0.2.1 (commit 44ee4610, dirty=false, profile release)
```

Use `jit version` when you need the full provenance record.

## Version and Provenance

### `jit version`

Show the running `jit` binary's local build metadata. This command does not
require a `.jit/` repository and does not contact GitHub or compare against the
current checkout.

```bash
jit version
```

Human-readable output includes:

- `Version` — crate package version
- `Commit` — short and full Git commit hash, or `unknown` when unavailable
- `Dirty` — whether the source tree was dirty at build time, or `unknown`
- `Profile` — Cargo build profile such as `debug` or `release`
- `Built` — build timestamp as Unix epoch seconds, or `unknown`
- `Target` — Cargo target triple

### `jit version --json`

Return the same provenance as machine-readable JSON:

```bash
jit version --json
```

```json
{
  "package": "jit",
  "version": "0.2.1",
  "git_commit": "44ee4610bf33e7f35f4c87056c46a6cff3d13f5a",
  "git_short_commit": "44ee4610",
  "git_dirty": false,
  "build_profile": "release",
  "build_timestamp": "1777327815",
  "target": "x86_64-unknown-linux-gnu"
}
```

`git_dirty` is `true` or `false` when known, and `null` when build-time Git
state could not be determined.

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

### Batch-Create with Dependency Wiring (`jit issue batch-create`)

`jit issue batch-create --from-json <file>` creates a whole set of issues and
their dependency edges from one declarative JSON file, replacing hand-written
`create` + `dependency add` loops. Entries reference each other by a symbolic
`key`, so you describe the dependency graph directly instead of threading
generated IDs through follow-up commands.

```bash
jit issue batch-create --from-json plan.json
jit issue batch-create --from-json plan.json --json
```

**File schema** — a JSON array of objects:

| Field         | Required | Default              | Notes                                              |
|---------------|----------|----------------------|----------------------------------------------------|
| `key`         | yes      | —                    | Symbolic key, unique within the file               |
| `title`       | yes      | —                    | Issue title                                        |
| `description` | no       | `""`                 | Issue body                                         |
| `type`        | no       | project default type | Applied as a `type:<t>` label                      |
| `priority`    | no       | `normal`             | `low` / `normal` / `high` / `critical`             |
| `labels`      | no       | `[]`                 | Each must be `namespace:value`                     |
| `gates`       | no       | `[]`                 | Each must be a registered gate key                 |
| `depends_on`  | no       | `[]`                 | Symbolic `key`s of other entries in the same file  |

Example `plan.json`:

```json
[
  { "key": "spec", "title": "Write the spec", "type": "story" },
  { "key": "impl", "title": "Implement it", "type": "task", "depends_on": ["spec"] },
  { "key": "test", "title": "Test it", "type": "task", "depends_on": ["impl"] }
]
```

**Pre-validation (ALL before any write).** The entire file is validated before
a single issue is created. Validation collects EVERY problem (it does not stop
at the first) and, on any failure, creates **zero** issues and exits with code
`2` (invalid argument), listing each offending entry. Checks performed:

- duplicate `key`s,
- `depends_on` references to keys not defined in the file,
- cycles in the symbolic `depends_on` graph,
- priority strings that do not parse,
- `type` values not in the project's `[type_hierarchy]` (when one is configured),
- labels that are not `namespace:value`,
- gates not present in the gate registry.

**Atomicity caveat.** Pre-validation is atomic: a malformed file changes
nothing. The **write phase is NOT atomic** — once creation begins, a failure
partway through reports the partial `{key: id}` map produced so far plus the
failing step and exits non-zero. There is no rollback; recover manually (inspect
or delete the partially-created issues, fix the file, and re-run).

**Output.** On success the command returns the `{key: full_id}` map. With
`--json` the output is EXACTLY that map as the top-level JSON object — every
entry is a symbolic key mapping to the created issue's full id, with no envelope
or `message` field (keys are emitted sorted). Human output lists each `key -> id`.

```json
{
  "impl": "54a9f64c-7117-483e-b59d-3ccc16c5b55e",
  "spec": "48453126-9f6b-4f7a-a5a6-cb30ef833f92"
}
```

### Searching Issues (`jit issue search`)

`jit issue search` matches issues by a text query and/or filter flags. The text
query searches the title, description, and ID.

```bash
# Text query only
jit issue search auth

# Label filter, NO positional query
jit issue search --label type:epic

# Repeatable --label is ANDed: an issue must carry EVERY label
jit issue search --label type:task --label area:auth

# Combine a query with filters (both narrow the result)
jit issue search task --state ready --json
```

**Flag rules:**
- The positional query is **optional** whenever at least one filter flag is
  given (`--label`, `--state`/`-s`, `--assignee`/`-a`, `--priority`/`-p`). With a
  filter present the search matches all issues and the filters narrow it.
- Providing **neither** a query **nor** any filter is a usage error (exit code
  `2`): "provide a search query or at least one filter".
- `--label`/`-l` (format `namespace:value`) is repeatable. Multiple labels are
  **ANDed**: an issue is returned only when it carries every requested label. A
  malformed label is a usage error.
- `--full` returns full issue objects; the default `--json` shape uses compact
  `MinimalIssue` summaries. The `query` field is `null` when no positional query
  was given.

### Inspecting Issues (`jit issue show`)

`jit issue show` accepts one or more issue ids and supports field projection so
agents can read a single value without piping `--json` through `jq`/`python`.

```bash
# Full human-readable view (default)
jit issue show abc123

# Compact response without the description or enriched dependencies (--json only)
jit issue show abc123 --summary --json

# Project a single top-level field as PLAIN TEXT (unquoted)
jit issue show abc123 --field state          # -> ready
jit issue show abc123 --field title          # -> Implement login

# Project several fields as one COMPACT JSON object (requested key order preserved)
jit issue show abc123 --fields state,title   # -> {"state":"ready","title":"Implement login"}

# Show multiple issues as a JSON array (argument order preserved)
jit issue show abc123 def456 --json          # -> [ {...}, {...} ]
```

**Field projection (`--field` / `--fields`):**
- Projected names are the serialized keys of the `issue show --json` object
  (`id`, `short_id`, `title`, `state`, `priority`, `assignee`, `dependencies`,
  `gates`, `labels`, `description`, `created_at`, `updated_at`, …).
- `--field <name>` prints the field as plain text: string fields print raw
  (unquoted), scalar fields (number/bool/null) print their value, and
  array/object fields fall back to compact JSON for that field.
- `--fields a,b,c` prints `{"a":...,"b":...,"c":...}` as a single compact JSON
  object, keeping the requested order.
- An unknown field name (or any unknown name in `--fields`) is a usage error
  (exit code `2`).

**Flag rules:**
- `--field` and `--fields` are mutually exclusive.
- `--field`/`--fields` require **exactly one** issue id; passing them with two or
  more ids is rejected as a usage error (exit code `2`).
- Passing two or more ids with `--json` returns a JSON **array** of full issue
  objects in argument order. A single id with `--json` stays a single object.

### Assigning and Claiming Issues

There are two ways to put an assignee on an issue:

- **`jit issue assign <id> <assignee>`** sets the assignee and makes no state
  change. The issue stays in whatever state it is in (`backlog`, `ready`, ...).
- **`jit issue claim <id> <assignee>`** assigns the issue *and* transitions a
  `ready` issue to `in_progress` (the "start work" path). It is atomic and
  refuses an already-assigned issue.

```bash
# Assign without starting work (no state change)
jit issue assign $ISSUE agent:worker-1

# Claim: assign and transition ready -> in_progress
jit issue claim $ISSUE agent:worker-1

# Claim but skip the state transition (equivalent to `issue assign`)
jit issue claim $ISSUE agent:worker-1 --assign-only
```

**`--assign-only`:** Assign the issue without transitioning its state to
`in_progress`. Use it when you want to take ownership of work that is not yet
`ready` (e.g. still `backlog` behind dependencies) without forcing a transition.

**Dependency-blocked claims:** `jit issue claim` fails (exit 4) when the issue is
still `backlog` behind incomplete dependencies, because it cannot transition to
`in_progress`. The error (in both human and `--json` output) names how to assign
without starting work:

```text
To assign without starting work (no state change): jit issue assign <issue-id> <assignee>
```

In `--json` this hint appears in `error.suggestions` and
`error.details.remediation`. To take ownership of a dependency-blocked issue,
use `jit issue assign <id> <assignee>` (or `jit issue claim <id> <assignee>
--assign-only`).

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
- `--auto` - Convenience flag for `--mode auto` (overrides `--mode` when both are given)
- `--checker-command <COMMAND>` - Command to run for automated gates
- `--timeout <SECONDS>` - Checker timeout in seconds (default: 300)
- `--working-dir <PATH>` - Working directory for checker (relative to repo root)
- `--pass-context` - Pass structured context (issue data, run history, prompt) to checker via `JIT_CONTEXT_FILE`
- `--prompt <TEXT>` - Inline prompt/instructions included in context
- `--prompt-file <PATH>` - Path to prompt file (relative to repo root), read at check time; takes precedence over `--prompt`
- `--env <KEY=VALUE>` - Environment variable to pass to the checker process (repeatable)

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

# Context-aware gate with prompt
jit gate define review \
  --title "AI Review" \
  --description "AI-powered code review" \
  --mode auto \
  --pass-context \
  --prompt-file "docs/review-prompt.md" \
  --checker-command "./scripts/ai-review.sh" \
  --env REVIEWER_AGENT="codex review -"
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

**JSON output:**
```json
{
  "gates": [
    {
      "key": "tests",
      "title": "All Tests Pass",
      "description": "Full test suite must pass",
      "auto": true,
      "stage": "postcheck",
      "mode": "auto"
    }
  ],
  "count": 1,
  "message": "1 gate definition(s)"
}
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
- Shows the most recent recorded run for the gate
- Does not execute the checker or update gate state
- Only works for automated gates that have recorded runs

**Examples:**
```bash
# Show the latest recorded run for a single gate
jit gate check abc123 tests
# Gate 'tests' last run: passed (exit code: 0)

# Show a failed recorded run
jit gate check abc123 clippy
# Gate 'clippy' last run: failed (exit code: 1)
# stdout/stderr from the stored run are shown inline
```

### `jit gate check-all`

Show the latest recorded runs for all automated gates on an issue.

**Usage:**
```bash
jit gate check-all <ISSUE_ID> [--json]
```

**Behavior:**
- Shows the latest recorded run for each automated gate on the issue
- Does not execute any checker commands
- Reports which automated gates do not have recorded runs yet

**Example:**
```bash
$ jit gate check-all abc123

Gate run results for issue abc123:
Gate 'tests' last run: passed (exit code: 0)
Gate 'fmt' last run: passed (exit code: 0)
Gate 'clippy' has not been run yet for issue abc123. Use 'jit gate pass' to run it.
```

### `jit gate pass`

Manually mark a gate as passed.

**Usage:**
```bash
jit gate pass <ISSUE_ID> <GATE_KEY> [--by <WHO>] [--force]
```

**Options:**
- `--by <WHO>` - Record who passed the gate (e.g., `human:alice`, `ci:github-actions`)
- `--force` - Re-run the checker even if the gate already passed at the current HEAD commit

**Examples:**
```bash
# Pass manual gate
jit gate pass abc123 code-review --by "human:alice"

# Pass without attribution
jit gate pass abc123 tdd-reminder

# Pass automated gate manually (override checker)
jit gate pass abc123 tests --by "human:admin"

# Force a re-run even if it already passed at HEAD
jit gate pass abc123 tests --force
```

**Behavior:**
- For a manual gate: updates gate status to `passed`, records who passed it and timestamp.
- For an automated (auto) gate: runs the checker and only marks the gate passed if the checker passes.
- If this was the last blocking gate, issue auto-transitions from `gated → done`.

**Skip when already passed at HEAD:**
- If the gate's latest run already passed at the current `HEAD` commit, `jit gate
  pass` skips the (often expensive) checker, exits `0`, and reports
  `already_passed: true` in `--json`. The non-`--json` path prints a concise
  "already passed at HEAD, skipping (use --force to re-run)" line.
- The skip compares the current `HEAD` against the commit stamped on the latest
  recorded run; both must be present and equal. When there is no git repository
  or no commit (`HEAD` unresolvable), the run is never skipped — the prior pass
  cannot be proven current.
- `--force` bypasses the check and re-runs the checker unconditionally.
- On a normal run (manual attestation, or a freshly executed checker), `--json`
  reports `already_passed: false`.

**Exit-code taxonomy** (auto and manual gates):

| Code | Meaning |
|------|---------|
| `0`  | pass — checker passed, or manual attestation recorded |
| `2`  | bad arguments — the gate is not required for this issue |
| `3`  | issue not found |
| `4`  | checker failure — the checker ran and the verdict was `fail` |
| `10` | runner error — timeout, command-not-found, or crash (infrastructure failure) |

The codes `4` and `10` are split by the carried checker status: a clean non-zero
verdict (e.g. tests failed) is `4`; a checker that could not produce a verdict
(killed by timeout/signal, no exit code) is `10`. Pre-verdict argument (`2`) and
lookup (`3`) errors are classified before the run path and are never reported as
a runner error.

**`--json` verdict field:**

`jit gate pass --json` carries a `verdict` field describing the run-path outcome:

- `pass` — top-level field on the success response.
- `fail` — under `error.details` when the checker ran and failed (code `4`).
- `error` — under `error.details` when the runner failed (code `10`).

Pre-verdict errors (codes `2` and `3`) are argument/lookup errors, not gate
verdicts, so they carry no `verdict` field.

```bash
# Success response
jit gate pass abc123 tests --json
# {
#   "issue_id": "abc123",
#   "gate_key": "tests",
#   "status": "passed",
#   "verdict": "pass",
#   "message": "Passed gate 'tests' for issue abc123"
# }

# Checker failure (exit 4): error.details.verdict == "fail"
# Runner error    (exit 10): error.details.verdict == "error"
```

### `jit gate pass-all`

Pass all of an issue's required gates in one command, **fail-fast**.

**Usage:**
```bash
jit gate pass-all <ISSUE_ID> [--by <WHO>] [--force]
```

**Options:**
- `--by <WHO>` - Record who passed the gates (e.g., `human:alice`, `ci:github-actions`)
- `--force` - Re-run every gate's checker even if it already passed at the current HEAD commit

**Behavior:**
- Runs each required gate in declaration order, delegating to `jit gate pass`, so
  every gate inherits the same exit-code taxonomy, `verdict` semantics, and the
  **skip-if-passed-at-HEAD** behaviour (an already-passed gate is not re-run;
  its entry reports `already_passed: true`).
- **Fail-fast:** on the FIRST gate that does not pass, the command stops
  immediately and exits with that gate's code from the
  [`jit gate pass`](#jit-gate-pass) taxonomy (`0` pass / `2` bad-args / `3`
  not-found / `4` checker-failed / `10` runner-error). Later gates are never
  attempted.
- An issue with **no required gates** succeeds with exit `0` and an empty
  `gates` array.
- `--json` emits a top-level `verdict: "pass"` plus a `gates` array, one entry
  per gate (`gate_key`, `status`, `verdict`, `already_passed`). On the first
  failure it emits the same JSON-error shape as `jit gate pass` (with
  `error.details.verdict` `fail` or `error` and `error.details.gate_key` naming
  the offending gate).

```bash
# All gates pass (one already passed at HEAD, one freshly run)
jit gate pass-all abc123 --json
# {
#   "issue_id": "abc123",
#   "status": "passed",
#   "verdict": "pass",
#   "gates": [
#     { "gate_key": "tests",  "status": "passed", "verdict": "pass", "already_passed": true },
#     { "gate_key": "clippy", "status": "passed", "verdict": "pass", "already_passed": false }
#   ],
#   "message": "Passed 2 required gate(s) for issue abc123"
# }

# Fail-fast: first failing gate sets the exit code; later gates do not run.
jit gate pass-all abc123          # exit 4 if a checker fails, 10 on runner error
```

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

`jit issue show <id> --json` emits a `gates` array — one entry per required
gate, enriched from that gate's latest run. `status` is `pending`, `passed`, or
`failed`; `last_run_at` and `exit_code` come from the gate's latest run and are
both `null` when no run has been recorded (required-but-never-run, or a manual
gate attested without a run).

```bash
# Show all gate information for issue
jit issue show abc123 --json | jq '.gates'

# Example output:
[
  {
    "key": "tests",
    "status": "passed",
    "last_run_at": "2026-01-02T10:30:00Z",
    "exit_code": 0
  },
  {
    "key": "code-review",
    "status": "pending",
    "last_run_at": null,
    "exit_code": null
  }
]
```

**Find issues with specific gate status:**
```bash
# Find all gated issues (waiting for gates)
jit query all --state gated

# Use jq to filter by specific gate (query all returns the stored issue shape,
# which keeps the gates_status map)
jit query all --json | jq '.issues[] | select(.gates_status.tests.status == "failed")'
```

### Exit Codes

All gate commands use standard exit codes:

- `0` - Success
- `2` - Invalid argument (e.g. gate not required for the issue, duplicate gate)
- `3` - Resource not found (issue or gate)
- `4` - Validation/checker failure (e.g. `jit gate pass` checker verdict `fail`)
- `10` - Runner/external error (e.g. `jit gate pass` checker timeout or crash)

See [`jit gate pass`](#jit-gate-pass) above for the full pass-specific taxonomy
and the `--json` `verdict` field.

## Gate Preset Commands

Gate presets are pre-configured bundles of quality gates that can be quickly applied to issues. Presets encode best practices and reduce setup time from minutes to seconds.

### `jit gate preset list`

List all available gate presets (builtin and custom).

**Usage:**
```bash
jit gate preset list [--json]
```

**Output:**
```
[builtin] rust-tdd - Test-driven development workflow for Rust projects (5 gates)
[builtin] minimal - Minimal workflow with just code review (1 gate)
[custom] my-workflow - Custom preset created from issue abc123 (3 gates)
```

**Example:**
```bash
# List all presets
jit gate preset list

# JSON output
jit gate preset list --json
```

### `jit gate preset show`

Display detailed information about a specific preset, including all gates and their configurations.

**Usage:**
```bash
jit gate preset show <NAME> [--json]
```

**Arguments:**
- `NAME` - Preset name (e.g., `rust-tdd`, `minimal`)

**Output:**
```
Preset: rust-tdd
Description: Test-driven development workflow for Rust projects

Gates:
  tdd-reminder - Write tests first (TDD) (precheck:manual)
  tests - All tests pass (postcheck:auto)
    Command: cargo test
    Timeout: 300s
  clippy - Clippy lints pass (postcheck:auto)
    Command: cargo clippy --all-targets -- -D warnings
    Timeout: 120s
  fmt - Code formatted (postcheck:auto)
    Command: cargo fmt --check
    Timeout: 30s
  code-review - Code review completed (postcheck:manual)
```

**Examples:**
```bash
# Show preset details
jit gate preset show rust-tdd

# Show custom preset
jit gate preset show my-workflow

# JSON output
jit gate preset show rust-tdd --json
```

### `jit gate preset apply`

Apply preset gates to one or more issues. Gates from the preset are added to the issue's required gates list. If a gate doesn't exist in the registry, it is automatically defined.

**Usage:**
```bash
jit gate preset apply <NAME> <ISSUE_ID>... [OPTIONS]
```

**Arguments:**
- `NAME` - Preset name to apply
- `ISSUE_ID...` - One or more issue IDs (can specify multiple for batch operations)

**Options:**
- `--timeout <SECONDS>` - Override checker timeout for all automated gates
- `--no-precheck` - Skip precheck gates from preset
- `--no-postcheck` - Skip postcheck gates from preset
- `--except <GATE>` - Exclude specific gates (repeatable)
- `--json` - Output JSON format
- `--quiet` - Suppress non-essential output

**Examples:**
```bash
# Apply preset to single issue
jit gate preset apply rust-tdd abc123

# Apply to multiple issues (batch mode)
jit gate preset apply minimal abc123 def456 ghi789

# Apply from query results
jit query all | xargs jit gate preset apply rust-tdd

# Apply with filtering - skip precheck gates
jit gate preset apply rust-tdd abc123 --no-precheck

# Skip specific gates
jit gate preset apply rust-tdd abc123 --except clippy --except fmt

# Override timeout for all automated gates
jit gate preset apply rust-tdd abc123 --timeout 600

# Combine filters
jit gate preset apply rust-tdd abc123 --no-precheck --except clippy --timeout 120
```

**Batch Output:**
```
Applied preset 'rust-tdd' to 2 issue(s):
  abc123 - gates added: tdd-reminder, tests, clippy, fmt, code-review
  def456 - gates added: tdd-reminder, tests, clippy, fmt, code-review

Errors (1):
  xyz999 - Issue not found: xyz999
```

**Notes:**
- Gates are automatically added to registry if they don't exist
- Timeout override applies to all automated gates in the preset
- Exit code is 1 if any errors occur in batch mode
- Use `--json` for machine-readable output

### `jit gate preset create`

Create a custom preset from an issue's current gates. Captures all gates required by the issue and saves them as a reusable preset.

**Usage:**
```bash
jit gate preset create <ISSUE_ID> <NAME> [--json]
```

**Arguments:**
- `ISSUE_ID` - Issue to copy gates from
- `NAME` - Name for the new preset

**Options:**
- `--json` - Output JSON format
- `--quiet` - Suppress non-essential output

**Output:**
```
Created preset 'my-workflow' at /path/to/.jit/config/gate-presets/my-workflow.json
```

**Examples:**
```bash
# Create preset from issue
jit gate preset create abc123 my-workflow

# Create team standard
jit gate preset create abc123 team-standard

# JSON output
jit gate preset create abc123 my-workflow --json
```

**Validation:**
- Issue must have at least one gate
- Preset name must be valid (no special characters)

**Storage:**
Custom presets are stored in `.jit/config/gate-presets/<name>.json` and are automatically loaded alongside builtin presets. Custom presets with the same name as a builtin preset override the builtin.

### Builtin Presets

JIT includes eight builtin presets embedded in the binary:

**`rust-tdd`** - Test-driven development workflow for Rust (5 gates)
- `tdd-reminder` - Manual reminder to write tests first (precheck)
- `tests` - Automated test suite check (postcheck, 300s timeout)
- `clippy` - Automated linter check (postcheck, 120s timeout)
- `fmt` - Automated formatter check (postcheck, 30s timeout)
- `code-review` - Manual code review requirement (postcheck)

**`python-tdd`** - Test-driven development workflow for Python (5 gates)
- `tdd-reminder` - Manual reminder to write tests first (precheck)
- `pytest` - Automated test suite check (postcheck, 300s timeout)
- `black` - Automated formatter check (postcheck, 30s timeout)
- `mypy` - Automated type checking (postcheck, 120s timeout)
- `code-review` - Manual code review requirement (postcheck)

**`js-tdd`** - Test-driven development workflow for JavaScript/TypeScript (4 gates)
- `tdd-reminder` - Manual reminder to write tests first (precheck)
- `jest` - Automated test suite check (postcheck, 300s timeout)
- `eslint` - Automated linter check (postcheck, 120s timeout)
- `code-review` - Manual code review requirement (postcheck)

**`security-audit`** - Security review workflow (3 gates)
- `security-review` - Manual security vulnerability review (precheck)
- `secret-detection` - Automated secret detection via gitleaks (postcheck, 20s timeout)
- `dependency-audit` - Automated dependency vulnerability audit (postcheck, 60s timeout)

**`minimal`** - Minimal workflow with just code review (1 gate)
- `code-review` - Manual code review requirement (postcheck)

The remaining three are the [planning-bracket](../concepts/planning-bracket.md) gates, attached automatically when a breakable container is bracketed:

**`plan-review`** - Agent plan-quality review on the planning node `P` (1 gate)
- `plan-review` - AI review of the plan/design before fan-out (postcheck, auto)

**`coverage-preview`** - Deterministic coverage check on the breakdown node `B` (1 gate)
- `coverage-preview` - Scoped `jit validate` over the drafted decomposition; blocks when a `[hard]` criterion is uncovered (postcheck, auto)

**`breakdown-review`** - Agent decomposition-quality review on the breakdown node `B` (1 gate)
- `breakdown-review` - AI review of the breakdown against the design and content standards: per-child content standards, dependency-DAG coherence, right-sized depth (postcheck, auto)

**Note:** Builtin presets can be overridden by creating a custom preset with the same name in `.jit/config/gate-presets/`.

### Custom Presets

Custom presets are stored as JSON files in `.jit/config/gate-presets/`:

**File Structure:**
```json
{
  "name": "my-workflow",
  "description": "Custom preset created from issue abc123",
  "gates": [
    {
      "key": "tests",
      "title": "All tests pass",
      "description": "cargo test must pass",
      "stage": "postcheck",
      "mode": "auto",
      "checker": {
        "type": "exec",
        "command": "cargo test",
        "timeout_seconds": 300,
        "working_dir": null,
        "env": {}
      }
    }
  ]
}
```

**Management:**
- Custom presets appear in `jit gate preset list` with `[custom]` indicator
- Custom presets with the same name override their builtin counterpart
- Edit JSON files directly or recreate with `jit gate preset create`
- Delete files to remove custom presets

### Preset Workflow Examples

**Quick Start with Builtin:**
```bash
# Apply standard workflow to new issue
jit issue create --title "Add user login"
jit gate preset apply rust-tdd abc123
# Issue now has all 5 quality gates
```

**Create Team Standard:**
```bash
# Set up one issue with desired gates
jit gate add abc123 tests clippy code-review docs

# Save as team standard
jit gate preset create abc123 team-standard

# Apply to all issues in epic
jit query all --filter "label:epic:v2.0" | xargs jit gate preset apply team-standard
```

**Customize for Special Cases:**
```bash
# Apply without precheck for hotfix
jit gate preset apply rust-tdd hotfix-123 --no-precheck

# Apply with faster timeout for CI
jit gate preset apply rust-tdd abc123 --timeout 60

# Apply subset of gates
jit gate preset apply rust-tdd abc123 --except tdd-reminder --except clippy
```

### Exit Codes

- `0` - Success
- `1` - Error (preset not found, issue not found, validation failed)
- Exit code 1 in batch mode if any issues fail

## Dependency Commands

<!-- jit dep add/rm -->

## Query Commands

### Default (bare) form

`jit query` with no subcommand returns all issues, equivalent to `jit query all`. All
four filters work on the bare form:

```bash
jit query                                    # all issues
jit query --state ready                      # filter by state
jit query --assignee agent:worker-1          # filter by assignee
jit query --priority critical                # filter by priority
jit query --label component:api              # filter by label pattern
jit query --state in_progress --json         # combine with --json
```

### Subcommands

| Subcommand | Alias | Description |
|------------|-------|-------------|
| `jit query all` | — | All issues with optional `--state`/`--assignee`/`--priority`/`--label` filters |
| `jit query available` | `ready` | Unassigned, unblocked, state=ready issues |
| `jit query ready` | — | Visible alias of `available` |
| `jit query blocked` | — | Blocked issues with blocking reasons |
| `jit query strategic` | — | Issues carrying labels from strategic namespaces |
| `jit query closed` | — | Issues in Done or Rejected state |

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
ISSUE_ID=$(jit issue create --title "Add feature" --orphan --quiet --json | jq -r 'id')

# Extract specific fields
jit issue show $ISSUE_ID --json --quiet | jq -r 'title'

# Process lists
jit query all --json --quiet | jq -r 'issues[] | select(.priority == "High") | .id'

# Query and filter
jit query available --json --quiet | jq -r 'issues[0].id'

# Get status counts
jit status --json --quiet | jq -r 'summary.by_state'
```

List-style JSON responses use a named collection plus `count` at the top level:

```json
{
  "issues": [],
  "count": 0,
  "message": "Found 0 issue(s)"
}
```

The standardized collection names are command-specific: `issues`, `gates`,
`namespaces`, `results`, or `worktrees`. Top-level `search` uses `count` rather
than `total`, and `label namespaces --json` returns only `namespaces`, `count`,
and the optional `message` field instead of internal configuration details.

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
READY=$(jit query available --json --quiet | jq -r 'count')
IN_PROGRESS=$(jit query all --state in_progress --json --quiet | jq -r 'count')
BLOCKED=$(jit query blocked --json --quiet | jq -r 'count')
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
