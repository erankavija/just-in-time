# Storage Format Reference

> **Diátaxis Type:** Reference

JIT stores all data in the `.jit/` directory at the repository root.

## Directory Structure

```
.jit/
├── config.toml        # Repository configuration
├── index.json         # Issue index for fast queries
├── events.jsonl       # Append-only event log
├── gates.toml         # Gate registry definitions
├── worktree.json      # Worktree metadata (if using git worktrees)
├── claims.jsonl       # Active lease records
├── issues/            # One JSON file per issue
│   ├── <uuid>.json    # Issue data
│   └── <uuid>.lock    # File lock for atomic operations
└── gate-runs/         # Gate execution logs
    └── <issue-id>/    # Per-issue gate run history
```

## Issue JSON Schema

Each issue is stored as `issues/<uuid>.json`:

```json
{
  "id": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17",
  "title": "Issue title",
  "description": "Detailed description",
  "state": "ready",
  "priority": "normal",
  "assignee": "agent:copilot-1",
  "dependencies": ["<other-issue-uuid>"],
  "gates_required": ["tests", "code-review"],
  "gates_status": {
    "tests": {
      "status": "passed",
      "updated_by": "auto:executor",
      "updated_at": "2026-01-15T10:30:00Z"
    }
  },
  "labels": ["type:task", "epic:auth", "component:backend"],
  "documents": [
    {
      "path": "docs/design.md",
      "doc_type": "design",
      "label": "Design Doc"
    }
  ],
  "context": {},
  "created_at": "2026-01-15T09:00:00Z",
  "updated_at": "2026-01-15T10:30:00Z",
  "first_ready_at": "2026-01-15T09:05:00Z",
  "claimed_at": "2026-01-15T09:30:00Z",
  "done_at": "2026-01-15T10:30:00Z"
}
```

### Field Reference

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Unique identifier (ULID-based) |
| `title` | string | Short issue title |
| `description` | string | Full description (markdown) |
| `state` | enum | `backlog`, `ready`, `in_progress`, `done`, `rejected` |
| `priority` | enum | `critical`, `high`, `normal`, `low` |
| `assignee` | string? | Format: `type:identifier` (e.g., `agent:copilot-1`) |
| `dependencies` | UUID[] | Issues that must complete before this one |
| `gates_required` | string[] | Gate keys from registry |
| `gates_status` | object | Per-gate status with timestamps |
| `labels` | string[] | Format: `namespace:value` |
| `documents` | object[] | Linked document references |
| `context` | object | Arbitrary metadata |
| `created_at` | timestamp | When the issue was created (RFC 3339) |
| `updated_at` | timestamp | When the issue was last modified (RFC 3339) |
| `first_ready_at` | timestamp? | When the issue FIRST entered `ready` (see below) |
| `claimed_at` | timestamp? | When the issue was FIRST claimed/assigned |
| `done_at` | timestamp? | When the issue FIRST reached `done` |

#### Lifecycle timestamps

`first_ready_at`, `claimed_at`, and `done_at` record when an issue first passed
each lifecycle milestone. They are written **once**, at the transition:

- `first_ready_at` — set the first time the issue enters `ready`, including the
  auto-promotion of a dependency-free issue at creation.
- `claimed_at` — set at the first claim or assignment.
- `done_at` — set the first time the issue reaches `done`. **Re-opening and
  re-completing does not overwrite it** ("first occurrence" semantics apply to
  all three).

Each field is **optional** and omitted from the JSON when unset (an issue that
never reached the milestone, or one predating these fields). Backfill the fields
for pre-existing issues from the event log with
[`jit migrate lifecycle-timestamps`](cli-commands.md#jit-migrate-lifecycle-timestamps);
issues whose event log carries no relevant transition stay unset.

**Compatibility.** These three fields (like `content_format` before them) are
**additive and optional**: the issue record does not use serde
`deny_unknown_fields`, so an older `jit` binary reading a newer repository
ignores keys it does not know, and a newer binary reads an older issue file by
defaulting the missing fields to unset. Because a stale binary neither drops nor
misreads them, adding these fields does **not** bump the repository
`schema_version` (still `2`); the [format-version guard](#versioning) is
reserved for changes that would make a stale binary misinterpret existing data.

## Configuration File

`config.toml` controls repository behavior:

```toml
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4, bug = 4 }
strategic_types = ["milestone", "epic"]

[validation]
strictness = "loose"  # "strict", "loose", or "permissive"
default_type = "task"
```

See [Configuration Reference](configuration.md) for full options.

## Event Log Format

`events.jsonl` is an append-only log (one JSON object per line):

```json
{"event_type":"IssueCreated","issue_id":"abc123","timestamp":"2026-01-15T10:00:00Z","data":{}}
{"event_type":"StateChanged","issue_id":"abc123","timestamp":"2026-01-15T10:05:00Z","data":{"from":"backlog","to":"ready"}}
{"event_type":"GatePassed","issue_id":"abc123","timestamp":"2026-01-15T10:10:00Z","data":{"gate":"tests","by":"auto:executor"}}
```

### Event Types

- `IssueCreated`, `IssueUpdated`, `IssueDeleted`
- `StateChanged` - State transitions
- `DependencyAdded`, `DependencyRemoved`
- `GatePassed`, `GateFailed`
- `LeaseAcquired`, `LeaseReleased`, `LeaseExpired`
- `AssigneeChanged`

## Gate Registry

`gates.toml` stores gate definitions as a `[[gates]]` array of tables. Each
entry carries its own `key`, rather than being indexed as an object field:

```toml
[[gates]]
key         = "tests"
title       = "All Tests Pass"
description = "Run test suite"
stage       = "postcheck"
mode        = "auto"

[gates.checker]
type            = "exec"
command         = "cargo test"
timeout_seconds = 300
```

## Versioning

The repository's on-disk **format version** is the `schema_version` field in
`index.json`:

```json
{ "schema_version": 2, "all_ids": [], "deleted_ids": [] }
```

This single marker is the authoritative compatibility check, bumped whenever an
on-disk layout or interpretation changes. On startup every command that opens
the repository compares it against the format version the running `jit` binary
supports:

- Binary support **≥** repository version: operates normally. Writing with a
  newer binary may migrate the data and bump the marker.
- Binary support **<** repository version: the binary refuses to operate and
  exits nonzero (exit code 10, external-dependency family) with a single-line
  error naming both the repository's format version and the version the binary
  supports. The fix is to upgrade `jit` (e.g. `cargo install --path crates/jit`)
  rather than treating it as repository corruption.

`config.toml` separately records a `[version] schema` describing the
configuration-file layout:

```toml
[version]
schema = 2
```
