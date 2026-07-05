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
| `jit issue list` | `jit query all` | Same filters/flags (`-s`/`-a`/`-p`/`-l`, `--full`, `--json`); identical output. `-l`/`--label` is repeatable and ANDed |
| `jit issue update <id> --add-label <label>` | `... --label <label>` | `--add-label` is an accepted alias for `--label` |

```bash
# These pairs are equivalent
jit dependency add a b      # == jit dep add a b
jit document list <id>      # == jit doc list <id>
jit issue list --json       # == jit query all --json
jit issue update <id> --add-label area:foo  # == --label area:foo
```

### Wrong-verb hints (not aliases)

Each command group has exactly one canonical spelling for removal: `jit issue
delete`, `jit dep rm`, `jit gate remove`, `jit doc remove`. Other groups'
spellings are NOT aliased onto each other — `dep remove`, `dep delete`, `issue
rm`, `issue remove`, `gate rm`, `gate delete`, `doc rm`, and `doc delete` all
fail (exit code 2). The error names the group's canonical command instead of
clap's generic "unrecognized subcommand" message, e.g.:

```
$ jit dep remove abc123 def456
Error: 'jit dep remove' is not a jit command. Use 'jit dep rm' instead.
```

The same applies to two guesses at editing an issue and to labeling an issue
through the wrong command:

| Wrong guess | Hint names |
|-------------|-----------|
| `jit issue complete <id>` | `jit issue update <id> --state done` |
| `jit issue edit <id>` | `jit issue update <id>` |
| `jit label add <id> <label>` | `jit issue update <id> --label <namespace:value>` |
| `jit label rm`/`remove <id> <label>` | `jit issue update <id> --remove-label <namespace:value>` |

`jit label` itself only inspects the namespace registry (`jit label
namespaces`, `jit label values`) — it never touches an issue's labels. See
`jit label --help`.

Under `--json`, the same hint is in `error.message` with code
`INVALID_ARGUMENT`, exit code 2 — identical to any other usage error.

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
          "key": "code-review",
          "status": "pending"
        }
      ],
      "remediation": [
        "jit gate status-all work-id",
        "jit gate evaluate work-id code-review"
      ]
    },
    "suggestions": [
      "jit gate status-all work-id",
      "jit gate evaluate work-id code-review"
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

**`jit_gate_status`** - Show the latest recorded run for a gate (read-only)
```javascript
{
  id: string,
  gate_key: string,
  json?: boolean
}
```

**`jit_gate_status_all`** - Report readiness of every required gate; nonzero unless all passed (read-only)
```javascript
{
  id: string,
  json?: boolean
}
```

**`jit_gate_evaluate`** - Evaluate a gate: run its checker (auto) or record attestation (manual)
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
  format: string,             // "dot" | "mermaid" | "json"
  full?: boolean,             // Complete issue records per node (json only)
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
await jit_gate_status({ id: issueId, gate_key: "tests" });
await jit_gate_evaluate({ 
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
  jit_gate_status({ id, gate_key: "tests" }),
  jit_gate_status({ id, gate_key: "clippy" }),
  jit_gate_status({ id, gate_key: "fmt" })
]);

// Avoid: Sequential
await jit_gate_status({ id, gate_key: "tests" });
await jit_gate_status({ id, gate_key: "clippy" });
await jit_gate_status({ id, gate_key: "fmt" });
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
await jit_gate_evaluate({ id: "003f", gate_key: "tests" });
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

# Show multiple issues as the list envelope (argument order preserved)
jit issue show abc123 def456 --json          # -> {"count":2,"issues":[ {...}, {...} ]}
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

**Dangling dependencies:** the response includes a `dangling_dependency_ids`
array listing any ids in the issue's stored `dependencies` whose target issue
no longer exists. It is omitted when empty. Deleting an issue strips its id
from every dependent, so this array is normally absent; it surfaces only
pre-existing corruption (e.g. a repository hand-edited or written by an older
binary) rather than silently hiding those ids from the `dependencies` view.

**Unmet dependencies:** the `issue show --json` object also carries an
`unmet_dependencies` array: the subset of `dependencies` that are not yet **met**.
A dependency is met exactly when it is in a terminal state (`done` or
`rejected`) — the same readiness test `jit query ready` uses to decide whether an
issue is blocked — so a `rejected` dependency counts as met and is **not** listed.
Each entry is a subset of the matching `dependencies` entry: `{id, short_id,
title, state}`. The array is always present (empty `[]` when every dependency is
met or there are none), so callers no longer recompute the filter client-side.

```bash
jit issue show abc123 --json | jq '.unmet_dependencies'
# -> [ {"id":"def456...","short_id":"def45678","title":"Build parser","state":"in_progress"} ]
```

**Flag rules:**
- `--field` and `--fields` are mutually exclusive.
- `--field`/`--fields` require **exactly one** issue id; passing them with two or
  more ids is rejected as a usage error (exit code `2`).
- Passing two or more ids with `--json` returns the list envelope
  `{"count": N, "issues": [...]}` with full issue objects in argument order. A
  single id with `--json` stays a single object.

### Compact status (`jit issue status`)

`jit issue status <id>...` prints the one-line "where does this issue stand"
view — state, per-gate status, and still-unmet dependencies — that agents
otherwise rebuild by piping `issue show --json` through `jq`. It accepts one or
more ids and emits one row (text) or object (`--json`) per id, in argument order.

```bash
# Text (default): one greppable line per id
jit issue status abc123
# -> abc12345 [backlog] gates: tests=passed,review=pending unmet: def45678 title: Wire up login

# Empty sections read `none`
jit issue status ghi789
# -> ghi78901 [ready] gates: none unmet: none title: Standalone task

# Multiple ids: one line each
jit issue status abc123 def456

# JSON: a compact object per id
jit issue status abc123 --json
# -> {"short_id":"abc12345","state":"backlog","gates":[{"key":"tests","status":"passed"}],
#     "unmet_dependencies":["def45678"],"title":"Wire up login"}

# Two or more ids with --json use the list envelope
jit issue status abc123 def456 --json
# -> {"count":2,"issues":[ {...}, {...} ]}
```

**Text format** (stable and greppable, fixed field order):

```text
<short_id> [<state>] gates: <key>=<status>,... unmet: <short_id>,... title: <title>
```

- `<state>` and each gate `<status>` use their canonical lowercase names
  (`backlog`/`ready`/…, `pending`/`passed`/`failed`).
- The `gates:` and `unmet:` sections render `none` when empty, so the separators
  stay constant regardless of content.

**JSON shape:** `{short_id, state, gates:[{key,status}], unmet_dependencies:[short_id,...], title}`.
Note `unmet_dependencies` here is an array of **short ids** (the compact view),
whereas the full `issue show --json` carries the richer `{id, short_id, title,
state}` objects. Both use the same readiness-consistent unmet filter (terminal =
met). A single id yields a bare object; two or more wrap in the
`{"count": N, "issues": [...]}` list envelope.

`jit issue status` does not accept the `--field`/`--fields` projection flags;
those belong to `issue show`. Passing one is a usage error (exit code `2`).

### Container children (`jit issue children`)

`jit issue children <id>` lists a container's **direct children** — its
immediate dependencies (depth 1) — each rendered exactly like `issue status`.
This replaces the loop where an agent reads a container, then runs a per-child
`show`/`status` to see where each child stands.

**Containment follows the dependency DAG.** A container's children are the
issues it directly depends on; the dependency edges are authoritative for
membership. Membership labels (e.g. an `epic:*` or `milestone:*` grouping label)
are advisory and are **not** consulted here — to aggregate a label bucket use
[`jit query count`](#state-aggregation-jit-query-count). Any issue can be a
"container": a non-container leaf simply has no dependencies and lists nothing.
This depth-1 view lists only immediate children; for a deep rollup use
`jit graph deps <id> --depth <n>`.

```bash
# Text (default): one `issue status` line per child, ascending short-id order
jit issue children epic123
# -> aa11bb22 [done] gates: none unmet: none title: Parser
# -> cc33dd44 [in_progress] gates: tests=pending unmet: none title: Lexer

# JSON: container header + the list envelope over compact status objects
jit issue children epic123 --json
```

**JSON shape:**

```json
{
  "container": { "short_id": "ep1c2345", "title": "Auth epic", "state": "in_progress" },
  "count": 2,
  "issues": [
    { "short_id": "aa11bb22", "state": "done", "gates": [], "unmet_dependencies": [], "title": "Parser" },
    { "short_id": "cc33dd44", "state": "in_progress", "gates": [{ "key": "tests", "status": "pending" }], "unmet_dependencies": [], "title": "Lexer" }
  ]
}
```

- `container` is `{short_id, title, state}` for the queried issue.
- `issues` is the same compact projection as `issue status` (one object per
  child), and `count` equals its length (the standard `{count, issues}` list
  envelope, plus the `container` header). `count`/`issues` cover **resolvable**
  children only. Children are ordered by ascending short id.
- `dangling` (optional) lists any dependency id that resolves to no stored
  issue — a broken edge, e.g. from a raw storage mutation. Following the
  `issue show` `dangling_dependency_ids` precedent, such an edge is surfaced
  here rather than silently dropped; the key is omitted when there are none, and
  text mode appends a `dangling: <id>,<id>` line. A genuine storage error (not a
  missing id) is not treated as dangling — it propagates as a normal error.
- A bad id under `--json` returns the refined error envelope
  (`ISSUE_NOT_FOUND` / `INVALID_ID_PREFIX` / `AMBIGUOUS_ID`) and the matching
  exit code, exactly like `issue show`.

### Container progress (`jit issue progress`)

`jit issue progress <id>` aggregates a container's **direct children** (depth 1)
into counts by state plus a done/total delivery rollup — the numbers agents
otherwise compute by looping a per-child `show` and tallying by hand.

Membership follows the dependency DAG, exactly as for `issue children` (labels
are advisory; use `jit query count` for a label bucket). For a deep rollup use
`jit graph deps <id> --depth <n>`.

```bash
jit issue progress epic123
# -> ep1c2345 [in_progress] title: Auth epic
# -> by state: backlog=0 ready=1 in_progress=1 gated=0 done=2 rejected=1 archived=0
# -> done 2/5 (40%)  open 2  rejected 1

jit issue progress epic123 --json
```

**JSON shape:**

```json
{
  "container": { "short_id": "ep1c2345", "title": "Auth epic", "state": "in_progress" },
  "count": 7,
  "by_state": [
    { "state": "backlog", "count": 0 },
    { "state": "ready", "count": 1 },
    { "state": "in_progress", "count": 1 },
    { "state": "gated", "count": 0 },
    { "state": "done", "count": 2 },
    { "state": "rejected", "count": 1 },
    { "state": "archived", "count": 0 }
  ],
  "total": 5, "done": 2, "rejected": 1, "open": 2, "percent": 40
}
```

- `by_state` has **one entry per lifecycle state**, in canonical order, with a
  zero count for any state no child is in — a stable, complete shape. `count` is
  the number of state buckets (the `{count, by_state}` list envelope). The
  rollup fields are flattened alongside the `container` header.
- **Totals cover resolvable children only.** `total` and every count are over
  the direct children that resolve to a stored issue. A dependency id pointing
  at a missing issue is surfaced in `dangling` (optional, omitted when empty;
  text appends a `dangling: <id>` line) rather than counted or dropped — the
  same `issue show` precedent as `issue children`. A real storage error (not a
  missing id) propagates normally.
- **Terminal-state semantics** (tied to `State::is_terminal`, i.e.
  `done`/`rejected`): `done` and `rejected` are reported separately because a
  rejected child is terminal but **not delivered**. `open` is every non-terminal
  child (`total − done − rejected`; `archived`, which is not terminal, counts as
  open). The `done/total` ratio and `percent` (rounded; `0` when `total` is `0`)
  measure delivery — `done` against `total`.
- A bad id under `--json` returns the refined error envelope and matching exit
  code, like `issue show`.

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

### `jit gate update`

Edit an existing gate definition in the registry without hand-editing the
registry file. Only the fields you pass change; every other field keeps its
current value. The gate KEY is the gate's identity and cannot be changed. This
edits the registry definition only; per-issue gate status (`gates_status`) is
left untouched. The write is atomic (temp-file + rename).

**Usage:**
```bash
jit gate update <KEY> [OPTIONS]
```

**Arguments:**
- `KEY` - Exact registry key of the gate to update

**Optional (each leaves its field unchanged when omitted):**
- `--title <TITLE>` - New human-readable name
- `--description <DESCRIPTION>` - New description
- `--stage <STAGE>` - New stage: `precheck` or `postcheck`
- `--mode <MODE>` - New mode: `manual` or `auto`
- `--auto` - Convenience flag for `--mode auto` (overrides `--mode` when both are given)
- `--checker-command <COMMAND>` - New checker command for automated gates
- `--timeout <SECONDS>` - New checker timeout in seconds
- `--working-dir <PATH>` - New checker working directory (relative to repo root)
- `--clear-working-dir` - Clear the checker working directory (mutually exclusive with `--working-dir`)
- `--pass-context <BOOL>` - Set whether structured context is passed to the checker: `--pass-context true` or `--pass-context false`
- `--prompt <TEXT>` - New inline prompt/instructions
- `--clear-prompt` - Clear the inline prompt (mutually exclusive with `--prompt`)
- `--prompt-file <PATH>` - New prompt file path (relative to repo root)
- `--clear-prompt-file` - Clear the prompt file (mutually exclusive with `--prompt-file`)
- `--env <KEY=VALUE>` - Checker environment variable (repeatable); when provided, replaces the gate's existing environment set
- `--clear-env` - Clear the checker environment set (mutually exclusive with `--env`)
- `--priority <N>` - New execution priority (lower runs first)
- `--json` - Emit the updated gate definition as JSON

Every mutable field is reachable from the CLI: each clearable checker field has
both a set flag and a `--clear-*` flag, so editing a gate never requires
hand-editing the registry file. Passing a set flag together with its `--clear-*`
twin is an `INVALID_ARGUMENT` error.

Switching a gate to `auto` (via `--mode auto` or `--auto`) requires the gate to
have a checker command; supply `--checker-command` in the same call if the gate
had none. Switching to `manual` drops the checker. At least one field must be
provided; an update with no fields is an `INVALID_ARGUMENT` error. Updating a key
that is not in the registry is a `GATE_NOT_FOUND` error.

The write goes through the canonical atomic-write primitive (temp file + rename)
and appends a `gate_definition_updated` event to the event log.

**Examples:**
```bash
# Rename a gate
jit gate update tests --title "All Tests Pass"

# Raise the checker timeout, leaving everything else as-is
jit gate update tests --timeout 600

# Repoint an automated gate's checker command
jit gate update tests --checker-command "cargo test --workspace"

# Clear a gate's prompt file and disable context passing
jit gate update review --clear-prompt-file --pass-context false
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

### `jit gate status`

The unified gate-run inspection surface (inspection only, non-mutating). Legacy
alias: `check`. It offers four views over the stored run records: the latest
run (default), prior runs (history), the raw report text (flat), and the
structured findings (findings). All are read-only and reuse already-recorded
runs; none executes a checker or mutates gate state. Unlike
[`status-all`](#jit-gate-status-all), the singular `status` never exits nonzero
on a pending or failed gate — it is pure inspection.

**Usage:**
```bash
# Latest-run view (default): one gate, its most recent recorded run
jit gate status <ISSUE_ID> <GATE_KEY> [--json] [--quiet]
jit gate status <ISSUE_ID> --gate <GATE_KEY> [--json] [--quiet]

# History view: prior runs newest-first, gate key OPTIONAL (acts as a filter)
jit gate status <ISSUE_ID> --all [--gate <GATE_KEY>] [--status <STATUS>] [--json]
jit gate status <ISSUE_ID> --limit <N> [--gate <GATE_KEY>] [--status <STATUS>] [--json]

# Flat view: the latest run's stored report text, verbatim
jit gate status <ISSUE_ID> <GATE_KEY> --stdout [--tail <N>] [--json]
jit gate status <ISSUE_ID> <GATE_KEY> --stderr [--tail <N>] [--json]

# Findings view: the latest run's structured findings + verdict, one per line
jit gate status <ISSUE_ID> <GATE_KEY> --findings [--json]
```

For the latest-run and flat views the gate key may be supplied as a positional
argument or via `--gate <key>`; exactly one form must be used (supplying both or
neither is an error). In the history view the gate key is optional and, when
present, filters the listing to that gate.

**Options:**
- `--gate <KEY>` - Gate key (flag form). Required (with the positional as the
  alternative) for the latest-run and flat views; an optional filter in the
  history view.
- `--all` - History view: list every prior run, newest-first.
- `--limit <N>` - History view: list the most recent `N` runs, newest-first.
- `--status <STATUS>` - History view: keep only runs with this outcome
  (`passed`, `failed`, `error`, `pending`, `skipped`).
- `--stdout` - Flat view: print the latest run's stored stdout verbatim.
- `--stderr` - Flat view: print the latest run's stored stderr verbatim.
- `--tail <N>` - Flat view: keep only the last `N` lines of the printed text.
- `--findings` - Findings view: print only the latest run's structured findings
  and verdict (requires a gate key).
- `--json` - Machine-readable output (supported by every view).

The history view (`--all` / `--limit`) emits the list envelope
`{"count": N, "results": [...]}`, where `count` is the number of runs returned
after filtering. (The latest-run and flat views return a single run object.)

**Structured findings.** When a run's checker emitted a machine-readable
findings block (see
[Structured Findings](../how-to/custom-gates.md#structured-findings-machine-readable-output)),
its parsed form rides along as a `findings` object on the run in the latest-run,
history, and `status-all` JSON. The object has `verdict`, `summary`, and a
`findings` array of `{id, severity, summary, file?, line?}`. It is retained even
in the lean (passing) `status-all` projection that drops raw stdout, and it is
absent for plain-text checkers. Raw stdout is always kept alongside.

The `--findings` view reports only this structure for the latest run:

- Text: a header line `<gate> verdict: <v> summary: <s> findings: <N>` followed
  by one finding per line, `<id> [<severity>] <summary> (<file>:<line>)`. A run
  with no block renders `verdict: n/a findings: 0 (no machine-readable findings
  block)`.
- `--json`: `{"key", "run_id", "has_findings", "verdict", "summary",
  "findings":[...]}`. `has_findings` is `false` (and `verdict`/`summary` absent,
  `findings` empty) when the run carried no block.

History flags (`--all` / `--limit`), flat-output flags
(`--stdout` / `--stderr` / `--tail`), and the findings flag (`--findings`) are
mutually exclusive, and `--status` applies only to the history view. Each
violation is reported as an `INVALID_ARGUMENT` error (machine-readable under
`--json`).

**Examples:**
```bash
# Latest-run view (positional and flag forms are equivalent)
jit gate status abc123 tests
# Gate 'tests' last run: passed (exit code: 0)
jit gate status abc123 --gate tests

# History view: all prior runs of every gate, newest-first
jit gate status abc123 --all

# History view: the 5 most recent failed runs of the 'tests' gate
jit gate status abc123 --limit 5 --gate tests --status failed

# Flat view: the latest 'clippy' run's stderr, last 40 lines, undecorated
jit gate status abc123 clippy --stderr --tail 40

# Findings view: the latest 'code-review' run's structured findings + verdict
jit gate status abc123 code-review --findings
# code-review verdict: fail summary: 2 issues found findings: 2
# F1 [high] missing error context (src/x.rs:42)
# F2 [low] prefer iterator combinator
```

### `jit gate status-all`

Report the readiness of every required gate on an issue (inspection only,
non-mutating). Legacy alias: `check-all`.

**Usage:**
```bash
jit gate status-all <ISSUE_ID> [--json] [--full]
```

**Behavior:**
- Considers EVERY required gate on the issue — automated AND manual. A required
  manual gate that has not been attested counts as pending.
- Does not execute any checker commands; it only reports recorded state.
- Exits `0` only when every required gate has passed; otherwise exits `4`. A
  pending (auto never run, manual never attested) or failed gate is not green,
  and both map to the single nonzero code. This readiness contract is a single
  behaviour with no flag.
- With `--json`, the output is the list envelope
  `{"count": N, "gates": [...], …}`. `count` is the length of
  `gates` (one entry per required gate); it is the collection size, not
  a readiness tally. `total` / `passed` / `not_run` count all required gates —
  `total` is their number, `passed` how many are green, `not_run` the keys still
  pending. Each `gates` entry carries a required gate's `key` and status (`passed`
  / `failed` / `pending`) so a caller can tell a failed gate from a pending one.
  `all_passed` mirrors the exit contract. `--full` includes stdout/stderr for
  passing automated runs (failing runs always include them).

The singular [`jit gate status`](#jit-gate-status) carries the history
(`--all` / `--limit` / `--status`) and flat (`--stdout` / `--stderr` / `--tail`)
views over a single gate's run records; `status-all` is the cross-gate
readiness snapshot only.

**Example:**
```bash
$ jit gate status-all abc123          # exits 4 while any gate is not passed

Gate readiness for issue abc123:
Gate 'tests' last run: passed (exit code: 0)
Gate 'fmt' last run: passed (exit code: 0)
Gate 'clippy' has not been run yet for issue abc123. Use 'jit gate evaluate' to run it.
```

### `jit gate evaluate`

Run the checker (auto gates) or record attestation (manual gates) for a gate on
an issue. This produces a verdict (which may be *fail*), so it is not an
override. Legacy alias: `pass`; short alias: `eval`.

**Usage:**
```bash
jit gate evaluate <ISSUE_ID> <GATE_KEY> [--by <WHO>] [--force]
jit gate evaluate <ISSUE_ID> --gate <GATE_KEY> [--by <WHO>] [--force]
```

The gate key may be supplied as a positional argument or via `--gate <key>`. Exactly one form must be used; supplying both or neither is an error.

**Options:**
- `--gate <KEY>` - Gate key (flag form, alternative to the positional argument)
- `--by <WHO>` - Record who passed the gate (e.g., `human:alice`, `ci:github-actions`)
- `--force` - Re-run the checker even if the gate already passed at the current HEAD commit

**Examples:**
```bash
# Evaluate a manual gate — record attestation (positional form)
jit gate evaluate abc123 code-review --by "human:alice"

# Same command using the flag form
jit gate evaluate abc123 --gate code-review --by "human:alice"

# Evaluate without attribution
jit gate evaluate abc123 tdd-reminder

# Evaluate an automated gate — runs its checker (no manual override)
jit gate evaluate abc123 tests

# Force a re-run even if it already passed at HEAD
jit gate evaluate abc123 tests --force
jit gate evaluate abc123 --gate tests --force
```

**Behavior:**
- For a manual gate: updates gate status to `passed`, records who passed it and timestamp.
- For an automated (auto) gate: runs the checker and only marks the gate passed if the checker passes.
- If this was the last blocking gate, issue auto-transitions from `gated → done`.

**Skip when already passed at HEAD:**
- If the gate's latest run already passed at the current `HEAD` commit, `jit gate
  evaluate` skips the (often expensive) checker, exits `0`, and reports
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

`jit gate evaluate --json` carries a `verdict` field describing the run-path outcome:

- `pass` — top-level field on the success response.
- `fail` — under `error.details` when the checker ran and failed (code `4`).
- `error` — under `error.details` when the runner failed (code `10`).

Pre-verdict errors (codes `2` and `3`) are argument/lookup errors, not gate
verdicts, so they carry no `verdict` field.

```bash
# Success response
jit gate evaluate abc123 tests --json
# {
#   "issue_id": "abc123",
#   "key": "tests",
#   "status": "passed",
#   "verdict": "pass",
#   "message": "Passed gate 'tests' for issue abc123"
# }

# Checker failure (exit 4): error.details.verdict == "fail"
# Runner error    (exit 10): error.details.verdict == "error"
```

### `jit gate evaluate-all`

Evaluate all of an issue's required gates in one command, **fail-fast**. Legacy
alias: `pass-all`.

**Usage:**
```bash
jit gate evaluate-all <ISSUE_ID> [--by <WHO>] [--force]
```

**Options:**
- `--by <WHO>` - Record who passed the gates (e.g., `human:alice`, `ci:github-actions`)
- `--force` - Re-run every gate's checker even if it already passed at the current HEAD commit

**Behavior:**
- Runs each required gate in declaration order, delegating to `jit gate evaluate`, so
  every gate inherits the same exit-code taxonomy, `verdict` semantics, and the
  **skip-if-passed-at-HEAD** behaviour (an already-passed gate is not re-run;
  its entry reports `already_passed: true`).
- **Fail-fast:** on the FIRST gate that does not pass, the command stops
  immediately and exits with that gate's code from the
  [`jit gate evaluate`](#jit-gate-evaluate) taxonomy (`0` pass / `2` bad-args / `3`
  not-found / `4` checker-failed / `10` runner-error). Later gates are never
  attempted.
- An issue with **no required gates** succeeds with exit `0` and an empty
  `gates` array.
- `--json` emits a top-level `verdict: "pass"` plus a `gates` array, one entry
  per gate (`key`, `status`, `verdict`, `already_passed`). On the first
  failure it emits the same JSON-error shape as `jit gate evaluate` (with
  `error.details.verdict` `fail` or `error` and `error.details.key` naming
  the offending gate).

```bash
# All gates pass (one already passed at HEAD, one freshly run)
jit gate evaluate-all abc123 --json
# {
#   "issue_id": "abc123",
#   "status": "passed",
#   "verdict": "pass",
#   "gates": [
#     { "key": "tests",  "status": "passed", "verdict": "pass", "already_passed": true },
#     { "key": "clippy", "status": "passed", "verdict": "pass", "already_passed": false }
#   ],
#   "message": "Passed 2 required gate(s) for issue abc123"
# }

# Fail-fast: first failing gate sets the exit code; later gates do not run.
jit gate evaluate-all abc123          # exit 4 if a checker fails, 10 on runner error
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
- `4` - Validation/checker failure (e.g. `jit gate evaluate` checker verdict `fail`)
- `10` - Runner/external error (e.g. `jit gate evaluate` checker timeout or crash)

See [`jit gate evaluate`](#jit-gate-evaluate) above for the full pass-specific taxonomy
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

### `jit dep add`

Add one or more dependencies to an issue. `FROM` is blocked until every listed
`TO` completes. Dependencies are orthogonal to labels: issues don't need
matching labels to depend on each other.

**Usage:**
```bash
jit dep add <FROM_ID> <TO_ID>... [--reduce] [--json]
```

**Arguments:**
- `FROM_ID` — the issue that becomes blocked
- `TO_ID...` — one or more issues that must complete first

**All-or-nothing (jit:c8518f2a):** every requested edge is validated — id
resolution, then cycle detection and (by default) a check that the edge
doesn't leave the graph transitively redundant — against the graph with every
edge of the call applied at once, BEFORE anything is written. If any edge
fails, none of them are added and no event is logged for any of them, even
edges that would have succeeded on their own. The error names every rejected
edge, not only the first.

By default, an edge that would shadow an existing direct edge (or is itself
already reachable through other dependencies) is rejected, naming the
offending edge pair (exit code `4`). Pass `--reduce` to add the edge anyway and
drop the now-redundant edge(s) in the same operation, leaving the graph
transitively reduced — `jit validate` would otherwise flag the same violation
later. `jit validate --fix` performs the equivalent cleanup after the fact.

**Examples:**
```bash
# Single dependency
jit dep add epic-123 task-456

# Multiple dependencies in one call
jit dep add epic-123 task-1 task-2 task-3

# A redundant edge is rejected by default...
jit dep add downstream-issue shadowed-target
# Error: Refusing to add dependency ...: it would leave the graph not
# transitively reduced. Redundant edge(s): .... Re-run with `jit dep add
# --reduce` to drop the now-redundant edge(s) in the same operation, ...

# ...--reduce adds it and drops the shadowed edge instead
jit dep add downstream-issue shadowed-target --reduce
```

**JSON error output (rejected batch):**
```json
{
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "Refusing to add dependency ...",
    "details": {
      "from_id": "epic-123",
      "rejected": [
        { "from": "epic-123", "to": "task-2", "code": "VALIDATION_FAILED", "message": "..." }
      ]
    }
  }
}
```

A batch mixing an id-resolution failure (too-short/ambiguous prefix, not
found) with a graph-validation failure (cycle, or a rejected redundant edge)
exits with the resolution failure's code — resolution runs before graph
validation, so it wins the batch's exit code — while `details.rejected` still
lists every rejected edge.

### `jit dep rm`

Remove one or more dependencies from an issue. Unlike `dep add`, each `TO_ID`
is matched directly against `FROM_ID`'s own stored dependencies (by exact id or
by a 4+ character prefix), not resolved through the repository-wide index —
this keeps a dangling edge (whose target issue was deleted) removable.

**Usage:**
```bash
jit dep rm <FROM_ID> <TO_ID>... [--json]
```

**Examples:**
```bash
# Single dependency
jit dep rm epic-123 task-456

# Multiple dependencies in one call
jit dep rm epic-123 task-1 task-2
```

Removing a dependency can unblock the issue; a removal that clears the last
incomplete dependency auto-transitions a `backlog` issue to `ready`.

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
jit query --label epic:auth --label component:api  # repeatable --label is ANDed
jit query --state in_progress --json         # combine with --json
```

These filters belong to the bare form only. Supplying one before a subcommand
(e.g. `jit query --state ready available`) is a usage error (exit code `2`)
rather than a silent no-op, because the parent-level filter would otherwise be
dropped. Put the filter on the subcommand (`jit query available --priority high`)
or drop the subcommand to use the bare form.

`--label`/`-l` (format `namespace:value`, or `namespace:*` for wildcard) is
repeatable everywhere it appears in the query family — on the bare form and on
every subcommand below, as well as on `jit issue list`. Repeated occurrences
are ANDed: an issue is returned only if it matches every pattern given.

### Subcommands

| Subcommand | Alias | Description |
|------------|-------|-------------|
| `jit query all` | — | All issues with optional `--state`/`--assignee`/`--priority`/`--label` filters |
| `jit query available` | `ready` | Unassigned, unblocked, state=ready issues |
| `jit query ready` | — | Visible alias of `available` |
| `jit query blocked` | — | Blocked issues with blocking reasons |
| `jit query strategic` | — | Issues carrying labels from strategic namespaces |
| `jit query closed` | — | Issues in Done or Rejected state |
| `jit query count` | — | Counts by a dimension over a label bucket, with a done/total rollup |
| `jit query divergence` | — | Membership labels not backed by the DAG (advisory) |

### State aggregation (`jit query count`)

`jit query count --by state [--label ns:v ...]` aggregates a **label bucket**
into counts by state plus a done/total delivery rollup — the same rollup
[`jit issue progress`](#container-progress-jit-issue-progress) produces over a
container's children, but with membership defined by labels instead of the DAG.

The bucket is every issue matching **all** `--label` patterns (repeatable and
ANDed, as everywhere in the query family); with no `--label`, the whole
repository is aggregated. This is the advisory-grouping counterpart to
`issue progress`: use `issue progress` when containment is a dependency edge,
`query count` when it is a shared label.

```bash
jit query count --by state
# -> by state: backlog=3 ready=4 in_progress=2 gated=0 done=8 rejected=1 archived=0
# -> done 8/18 (44%)  open 6  rejected 1

jit query count --by state --label milestone:m1 --json
```

**JSON shape:** the same rollup as `issue progress`, without the `container`
header:

```json
{
  "count": 7,
  "by_state": [
    { "state": "backlog", "count": 3 },
    { "state": "ready", "count": 4 },
    { "state": "in_progress", "count": 2 },
    { "state": "gated", "count": 0 },
    { "state": "done", "count": 8 },
    { "state": "rejected", "count": 1 },
    { "state": "archived", "count": 0 }
  ],
  "total": 18, "done": 8, "rejected": 1, "open": 6, "percent": 44
}
```

- `--by` is required and typed: `state` is the only dimension today; an unknown
  value is a usage error (exit code `2`).
- `by_state` lists every lifecycle state (zero-count states included), `count`
  is the number of state buckets (the `{count, by_state}` list envelope), and
  the `done`/`rejected`/`open`/`percent` terminal-state semantics are identical
  to `issue progress` (done and rejected distinct; open = non-terminal;
  `done/total` measures delivery).

### Membership divergence (`jit query divergence`)

`jit query divergence [--json]` reports each issue that carries a membership
label (`epic:foo`, `milestone:v1.0`, …) while the dependency DAG does **not**
place it inside the container that owns that label — i.e. the label claims a
membership the [authoritative DAG](../concepts/hierarchy-resolution.md) does not
back. This is the canonical resolver for the label-vs-DAG disagreement that
`jit validate` also surfaces as an advisory count.

It is advisory and read-only: only the "label claims membership, DAG disagrees"
direction is flagged. A DAG descendant that does not repeat its container's label
is **not** reported (labels are advisory; children normally rely on the DAG), and
a label with no owning container is left to
[membership-reference validation](validation-rules.md).

```bash
jit query divergence
# -> 2 membership label(s) not backed by the DAG:
# ->   027d7cbf | milestone:v1.0 | Declare the gate contract ...

jit query divergence --json
```

**JSON shape:** the list envelope `{"count": N, "divergences": [...]}`:

```json
{
  "count": 1,
  "divergences": [
    {
      "id": "027d7cbf-bee1-4b4e-9912-bc144bc14cce",
      "short_id": "027d7cbf",
      "title": "Issue title",
      "label": "milestone:v1.0",
      "namespace": "milestone",
      "value": "v1.0"
    }
  ]
}
```

`jit validate --json` also carries `divergence_count` and a
`membership_divergences` array (the same entries); the count is **advisory** and
never changes the validate exit status.

## Document Commands

<!-- jit doc add/show/list/archive -->

## Graph Commands

<!-- jit graph deps/roots/downstream -->

### `jit graph export`

Export the whole-repository dependency graph.

```
jit graph export [--format dot|mermaid|json] [--full] [--output <file>]
```

| Flag | Description |
|------|-------------|
| `--format` | Output format: `dot` (default), `mermaid`, or `json`. |
| `--full` | Emit complete issue records per node. **JSON only** — combining it with `dot`/`mermaid` is a usage error (exit 2). |
| `--output` | Write to a file instead of stdout. |

`dot` and `mermaid` render the graph for Graphviz / Mermaid. `json` emits a
`{ "nodes": [...], "edges": [...] }` document for programmatic consumers, in one
of two node shapes; the `edges` list (`{ "from": <id>, "to": <dep-id> }`, one per
dependency edge) is identical in both.

**Summary shape (default `--format json`)** — lean nodes for orchestration
loops:

```json
{
  "nodes": [
    {
      "id": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17",
      "short_id": "003f9f83",
      "title": "Issue title",
      "state": "ready",
      "priority": "normal",
      "labels": ["type:task", "epic:auth"]
    }
  ],
  "edges": [
    { "from": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17", "to": "<dep-uuid>" }
  ]
}
```

**Full shape (`--format json --full`)** — each node is the complete issue
record, byte-for-byte the fields of the on-disk `issues/<id>.json` file
(`id`, `title`, `description`, `state`, `priority`, `assignee`, `dependencies`,
`gates_required`, `gates_status` with each gate's `status`/`updated_by`/
`updated_at`, `context`, `documents`, `labels`, `created_at`, `updated_at`, and
the lifecycle timestamps `first_ready_at`/`claimed_at`/`done_at` when present),
**plus two additive resolved-hierarchy fields**:

| Field | Meaning |
|-------|---------|
| `resolved_parent` | The node's nearest dominating container id (the [DAG-resolved](../concepts/hierarchy-resolution.md) parent), or `null` for a root. |
| `cluster` | The node's strategic root container id, or `null` for an orphan leaf. |

This lets a bulk consumer read every node's full record **and** its canonical
placement in one call instead of globbing the issue files or re-deriving
containment. The `edges` list is the same as the summary shape.

The default (no `--full`) output is unchanged from prior releases: bulk loops
that parse the summary shape are unaffected, and the two hierarchy fields appear
only in the `--full` shape. See
[storage-format § Issue JSON Schema](storage-format.md#issue-json-schema) for the
full field reference.

### `jit graph tree`

Show the DAG-resolved containment hierarchy — the parent, children, cluster, and
rank of each node — as computed by the canonical
[hierarchy resolver](../concepts/hierarchy-resolution.md). The dependency DAG is
authoritative; membership labels are advisory and are not consulted.

```
jit graph tree [<root-id>] [--json]
```

With no id the whole repository is resolved; with a root id the view is the root
plus its transitive dependency closure (the DAG subtree it contains). Each node
still carries its repository-wide resolution, so a scoped node's `parent` may
reference a container outside the listed subtree.

JSON uses the list envelope `{"count": N, "root": <id|null>, "nodes": [...]}`,
where each node is:

```json
{
  "id": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17",
  "short_id": "003f9f83",
  "title": "Issue title",
  "type": "task",
  "parent": "<container-uuid|null>",
  "children": ["<child-uuid>", "..."],
  "cluster": "<strategic-root-uuid|null>",
  "rank": 0
}
```

| Field | Meaning |
|-------|---------|
| `parent` | Nearest dominating container (deepest level, then fewest hops, then smallest id), or `null` for a root. |
| `children` | Ids whose resolved `parent` is this node, sorted ascending (the inverse of `parent`). |
| `cluster` | Strategic root of the parent chain, or `null` for an orphan leaf. |
| `rank` | Longest dependency-path length to an in-set sink (sinks are `0`). |
| `type` | The node's `type:` label value; omitted when it has none. |

## Status and Validation

<!-- jit status, jit validate -->

## Maintenance Commands

### `jit migrate lifecycle-timestamps`

One-time backfill of the issue [lifecycle
timestamps](storage-format.md#lifecycle-timestamps) (`first_ready_at`,
`claimed_at`, `done_at`) for issues created before those fields were written at
transition time.

```
jit migrate lifecycle-timestamps [--json]
```

For every issue missing one of the fields, the value is derived from
`.jit/events.jsonl`: the first `issue_state_changed` into `ready`, the first
`issue_claimed`, and the first `issue_state_changed` into `done`, respectively.
Only still-absent fields are filled — an existing timestamp is never overwritten,
preserving first-occurrence semantics. Updated issues are written atomically and
a single `lifecycle_timestamps_backfilled` event records the count.

The migration is **idempotent**: a second run over an already-migrated
repository writes nothing, appends no event, and reports `issues_updated: 0`.
Issues whose event log carries no relevant transition (predating event coverage,
or auto-promoted straight to `ready` at creation, which logs no transition) keep
their fields unset — the timestamps are unrecoverable, not defaulted.

`--json` prints `{ "issues_scanned": N, "issues_updated": M }`.

## Configuration

### `jit config get`

Resolve a dotted key against the WHOLE configuration surface — type
hierarchy, label namespaces, item kinds, documentation paths, validation
settings, project identity, schema version, and the system/user/repo-layered
worktree/coordination/lock/event settings — via a generic path walk over the
config, not a hand-maintained list of recognised keys.

**Usage:**
```bash
jit config get <DOTTED.KEY> [--json]
```

A dotted path mirrors `config.toml`'s own structure, including a section's
kebab-case keys (e.g. `item_kinds.<name>.id-pattern`, matching the TOML
`id-pattern` spelling) and a user-declared map's own entries (e.g.
`namespaces.type.unique`, `type_hierarchy.types.epic`). Giving an
INTERMEDIATE key returns the whole subtree at that point rather than erroring
— `jit config get documentation` prints the entire `documentation` table.

**Examples:**
```bash
# A leaf under a section untouched by the old hand-mapped `config get`.
jit config get type_hierarchy.strategic_types
# ["milestone", "epic"] (pretty-printed; a single scalar prints bare)

jit config get documentation.development_root
# dev

jit config get namespaces.type.unique
# true

# An intermediate key: the whole section.
jit config get documentation --json
# {"key": "documentation", "value": {"development_root": "dev", ...}}

# The pre-existing system/user/repo-layered settings still resolve exactly
# as before (env var, then repo, then user, then system, then default).
jit config get worktree.mode
jit config get coordination.default_ttl_secs
```

**Two resolution strategies**, matching how the rest of jit already reads
these sections — `config get` does not invent a third:
- `worktree`, `coordination`, `global_operations`, `locks`, `events`: the
  system/user/repo-merged, default-filled view (same as `jit config show`).
- Every other section (`version`, `project`, `type_hierarchy`, `validation`,
  `documentation`, `namespaces`, `item_kinds`, `invariant_projection`,
  `rules_gates_projection`): read from the REPO's `config.toml` only, with no
  system/user merge and no built-in defaults layered in — jit has no concept
  of a system/user override for a repo's type hierarchy or label namespaces.
  This means these sections reflect exactly what `config.toml` declares
  (an absent section resolves to `{}`), which can differ from `jit config
  show`'s built-in-default-filled view of the sections it covers (e.g.
  `namespaces`).

`templates` and `invariants` are NOT part of the dotted-path surface: both
are loaded from sibling files (`.jit/templates.toml`, `.jit/invariants.toml`)
rather than `config.toml` itself. Introspect them via `jit config
list-templates` / `jit invariant list`.

**Exit codes:**
- `0` — key resolved
- `2` — unknown key (`INVALID_ARGUMENT`). An unknown TOP-LEVEL key names the
  valid sections; an unknown NESTED key names the missing segment and its
  resolved parent path.

```bash
jit config get bogus_section
# Error: unknown config key 'bogus_section'; valid top-level sections:
# coordination, documentation, events, global_operations,
# invariant_projection, item_kinds, locks, namespaces, project,
# rules_gates_projection, type_hierarchy, validation, version, worktree

jit config get documentation.bogus_field
# Error: unknown config key 'documentation.bogus_field': no 'bogus_field'
# under 'documentation'
```

### `jit config show`

Display the merged, default-filled effective configuration from all sources
(system, user, repository).

```bash
jit config show [--json]
```

### `jit config set`

Set a `section.field` key in the repository (or, with `--global`, the
user-global) `config.toml`.

```bash
jit config set <KEY> <VALUE> [--global] [--json]

jit config set coordination.default_ttl_secs 1200
jit config set --global worktree.enforce_leases warn
```

### `jit config validate`

Validate configuration files for syntax errors, invalid values, and
deprecated options.

```bash
jit config validate [--json]
```

Exit codes:
- `0` — Valid configuration
- `1` — Errors found
- `2` — Warnings only

### `jit config show-hierarchy` / `jit config list-templates`

`show-hierarchy` prints the effective type→level map (built from the
namespace registry, with built-in defaults applied); `list-templates` lists
the built-in hierarchy templates `jit init --hierarchy-template` accepts.

```bash
jit config show-hierarchy [--json]
jit config list-templates [--json]
```

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

### List envelope

Every list- and query-family command wraps its collection in a uniform
envelope: a numeric top-level `count` and a plural, collection-typed key holding
the array. `count` always equals the length of that array, so a single parse
path works for every command — no bare-array or dual-shape fallback is needed.

```json
{
  "count": 0,
  "issues": [],
  "message": "Found 0 issue(s)"
}
```

The collection key is command-specific:

| Command | Collection key |
| --- | --- |
| `issue list`, `list`, `query all`/`available`(`ready`)/`blocked`/`strategic`/`closed`, `issue search`, `issue show <id> <id> …`, `issue status <id> <id> …`, `issue children` (plus a `container` header) | `issues` |
| `issue progress` (plus a `container` header), `query count` | `by_state` |
| `search` | `results` |
| `gate list` | `gates` |
| `gate preset list` | `presets` |
| `events tail`, `events query` | `events` |
| `doc list` | `documents` |
| `doc assets list` | `assets` |
| `claim list`, `claim status` | `leases` |
| `label namespaces` | `namespaces` |
| `label values` | `values` |
| `config list-templates` | `templates` |
| `item list`, `item search` | `items` |
| `worktree list` | `worktrees` |
| `graph roots` | `roots` |
| `graph rdeps`, `rdeps` | `dependents` |
| `graph deps` | `nodes` |
| `gate status --all`/`--limit` | `results` |
| `gate status-all` | `gates` |
| `invariant check` | `findings` |

Some envelopes carry additional metadata keys alongside `count` and the
collection (for example `query`/`namespace` context, `issue_id`, or a `warnings`
array), but `count` and the collection key are always present. Top-level
`search` uses `count` rather than `total`, and `label namespaces --json` returns
only `namespaces`, `count`, and the optional `message` field instead of internal
configuration details.

For a few commands `count` is the size of the named collection while a separate
aggregate lives elsewhere: `graph deps` counts the top-level `nodes` (whereas
`summary.total` is the unique-dependency count across the whole tree), and `gate
status-all` counts the `gates` entries (whereas `total` / `passed` are
readiness tallies over all required gates).

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
  jit gate evaluate "$ISSUE_ID" tests --quiet
  echo "✓ Tests passed for $ISSUE_ID"
else
  jit gate fail "$ISSUE_ID" tests --quiet
  echo "✗ Tests failed for $ISSUE_ID"
  exit 1
fi

# Run linter
if cargo clippy -- -D warnings; then
  jit gate evaluate "$ISSUE_ID" clippy --quiet
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

JIT uses a standardized exit-code taxonomy for scripting:

| Code | Meaning |
|------|---------|
| `0`  | Success |
| `1`  | Generic error (unclassified failure) |
| `2`  | Invalid argument / usage error |
| `3`  | Resource not found (issue, gate, repository) |
| `4`  | Validation failed (cycle detected, gate not passed, broken references) |
| `5`  | Permission denied |
| `6`  | Resource already exists |
| `10` | External dependency failed (git, filesystem, repository format too new) |

Argument-class failures that resolve an id prefix are exit `2`, each with a
distinguishing `code` under `--json`:

- **Ambiguous prefix** — a prefix matching more than one issue: `code`
  `AMBIGUOUS_ID`. Human message begins `Ambiguous ID '<prefix>' matches multiple
  issues:`.
- **Too-short prefix** — a prefix shorter than the 4-character minimum: `code`
  `INVALID_ID_PREFIX`. Human message is `Issue ID prefix must be at least 4
  characters`.

`jit dep rm <from> <target>` validates **both** id arguments identically: a
too-short or ambiguous prefix in either position is the same argument error
(exit `2`), rather than a short `<target>` being silently reported as "not
found".

**Startup failures under `--json`.** A failure that aborts before a command
handler runs still prints its human line on stderr, and with `--json` also emits
a structured error object on stdout while keeping its exit code:

- **Repository not found** (exit `3`): `code` `REPOSITORY_NOT_FOUND`.
- **Repository format too new** (exit `10`): `code` `REPOSITORY_FORMAT_TOO_NEW`
  (the binary is older than the repository's on-disk format; upgrade `jit`).

```bash
# Check exit codes
if jit issue create --title "Test" --orphan --quiet; then
  echo "Created successfully"
else
  echo "Failed with exit code: $?"
fi
```
