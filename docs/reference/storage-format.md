# Storage Format Reference

> **Diátaxis Type:** Reference

JIT stores its issue and configuration data in the `.jit/` directory at the
repository root. Machine-local runtime state lives there too, gitignored, and
multi-agent lease coordination lives under `.git/jit/` (see
[The `.git/jit/` Control Plane](#the-gitjit-control-plane)).

Three record shapes are defined by the code that writes them and are generated
from it: the issue identifier, the event-log line, and the gate-run record. They
live in [Storage Record Layout](storage-records.md), which this page links to
where each comes up.

## Directory Structure

```
.jit/
├── index.json        # Issue index for fast queries; carries the format version
├── config.toml       # Repository configuration
├── gates.toml        # Gate registry definitions
├── templates.toml    # Graph template registry
├── rules.toml        # Validation rules
├── invariants.toml   # Invariants registry
├── events.jsonl      # Append-only event log
├── profiles/         # Minimal applied-profile provenance records
│   └── <id>.json
├── issues/           # One JSON file per issue
│   └── <uuid>.json   # Issue data
├── gate-runs/        # Recorded gate runs, one directory per run
│   └── <run-id>/
│       └── result.json  # Run record (see Storage Record Layout)
└── schemas/          # JSON Schema files referenced by rules.toml
```

`templates.toml` and `invariants.toml` are authored per project; `jit init`
does not scaffold either one. `config.toml`, the empty `gates.toml`, and
`rules.toml` (with a default ruleset and its `schemas/` files) are scaffolded
by `jit init`.

A live repository also carries gitignored, machine-local files directly under
`.jit/`: `worktree.json`, `server.log`, `server.pid.json`, a fixed set of lock
files, and `tmp/`. The store's fixed lock set under `.jit/` is
`.repo-write.lock`, `.index.lock`, `.gates.lock`, and `.events.lock`; it is
independent of the number of issues. The repository-sibling
`.jit-bootstrap.lock` and the `.git/jit/locks/claims.lock` control-plane lock
complete the fixed set. Their paths come from the
[`JsonFileStorage`](../../crates/jit/src/storage/json.rs) and
[`ClaimCoordinator`](../../crates/jit/src/storage/claim_coordinator.rs)
implementations. The `.jit/` storage locks and `.jit-bootstrap.lock` retain
their inodes for advisory-lock race safety; they are opened with
create-if-absent semantics and are not unlinked. The claims lock belongs to the
separate recovery-aware control plane described below.

Issue reads do not create per-issue lock files. On the first read-all through a
storage instance, JIT removes empty UUID-shaped `.lock` sidecars left in
`issues/` by versions that predate this format. That sweep is legacy cleanup,
not part of read correctness or the current storage layout. Pending recoverable
multi-file transactions use `.jit/tmp/transactions/`. All of these are runtime
state, not part of the versioned data format.

When an embedded repository profile is applied, its minimal provenance record
lives under `.jit/profiles/`. Fresh profiled initialization may temporarily use
the repository-sibling `.jit-bootstrap/` control directory before `.jit/`
exists. The [Repository Profiles reference](profiles.md) defines the record,
transaction, rollback, and mandatory-recovery contract.

Lease records live under `.git/jit/`, not `.jit/`. See
[The `.git/jit/` control plane](#the-gitjit-control-plane) below.

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
| `id` | UUID | Unique identifier; its shape and prefix resolution are specified in [Storage Record Layout](storage-records.md#issue-identifiers) |
| `title` | string | Short issue title |
| `description` | string | Full description body; parsed per `content_format` |
| `state` | enum | `backlog`, `ready`, `in_progress`, `gated`, `done`, `rejected`, `archived` |
| `priority` | enum | `critical`, `high`, `normal`, `low` |
| `assignee` | string? | Format: `type:identifier` (e.g., `agent:copilot-1`) |
| `dependencies` | UUID[] | Issues that must reach an effective terminal state (done, rejected, or archived from one — see `archived_from`) before this one |
| `gates_required` | string[] | Gate keys from registry |
| `gates_status` | object | Per-gate status with timestamps |
| `labels` | string[] | Format: `namespace:value` |
| `content_format` | enum? | `markdown`, `html`, or `xml`; selects the parser for `description`. Absent inherits `[validation].content_format`, falling back to `markdown`. Omitted from JSON when unset |
| `documents` | object[] | Linked document references |
| `context` | object | Arbitrary metadata |
| `created_at` | timestamp | When the issue was created (RFC 3339) |
| `updated_at` | timestamp | When the issue was last modified (RFC 3339) |
| `first_ready_at` | timestamp? | When the issue FIRST entered `ready` (see below) |
| `claimed_at` | timestamp? | When the issue was FIRST claimed/assigned |
| `done_at` | timestamp? | When the issue FIRST reached `done` |
| `archived_from` | enum? | The state the issue held before entering `archived`, making `archived` terminality-preserving. Present only while `state` is `archived` and the origin was recorded; cleared on revive. Omitted from JSON when absent, so records predating the field round-trip unchanged and read as a legacy archive (treated as non-terminal) |

#### Gate fields in command output

The `gates_required` / `gates_status` split above is the **on-disk record
shape**. Three rules govern how it reaches a command's `--json` output:

- **Every command that hands back a stored issue record verbatim emits it
  under `gates_required` / `gates_status`.** Current members: the `--full`
  record dumps (`jit graph export --format json --full`; `jit query all`,
  `available`, `strategic`, and `closed` with `--full`, including the
  bare `jit query --full` spelling that defaults to `all`; `jit issue
  list --full` and its top-level `jit list --full` alias; `jit issue
  search --full`); the single-issue lifecycle mutation confirmations (`jit issue
  assign`, `unassign`, `reject`, `release`, `claim`, `claim-next`); and `jit
  apply`'s `created_issues` map.
- **Every projected issue view exposes the gate list as a single `gates`
  array** (`{key, status, …}` per required gate), never `gates_required` /
  `gates_status`. Current members: `jit issue create`, `jit issue show`, `jit
  issue show --summary`, `jit issue status`, and `jit issue children`.

- **Every lean list summary omits the gate list entirely, under every
  spelling.** The default (non-`--full`) output of `jit query` and its
  subcommands, `jit issue list` / `jit list`, and `jit issue search` carries
  no `gates`, `gates_required`, or `gates_status` field at all; consult
  `jit --schema` for the exact per-command projection, so an absent field is
  attributable to this rule rather than to an ungated issue.

`jit query blocked` belongs to none of the record-shape rules: both its shapes (default, and the
reason-enriched `--full`) build on the lean summary projection and carry no
gate-list fields at all — a blocking gate appears only as a `blocked_reasons`
entry.

`jit graph export --format batch` is not an issue-view projection but a
[batch-create seed](cli-commands.md#batch-format): its `gates` field is the
plain array of gate **keys** each node requires (the shape batch creation
consumes), carrying no per-gate status, so none of the three rules above apply.

So reading `gates_required` from `jit issue show --json` finds nothing. `jit
--schema` declares, per command, which of the two shapes it emits; see
[`jit issue show`](cli-commands.md#inspecting-issues-jit-issue-show).

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
**additive and optional**, and adding them keeps the repository at
`schema_version` 2. Two properties of the storage contract govern this:

- **Unknown keys are ignored on read.** The issue record does not use serde
  `deny_unknown_fields`, so a reader that lacks these fields parses the record
  without error, and a reader that has them treats a record omitting them as
  unset. No reader misinterprets the data it parses, so additive optional fields
  do not bump `schema_version`; the [format-version guard](#versioning) is
  reserved for changes that would make a reader misread existing data.
- **Unknown keys are not preserved on write.** The record captures no unknown
  keys — the struct declares an explicit field per key with no catch-all map — so
  a writer that lacks these fields does not round-trip them: any mutation triggers
  a full save that drops the keys it never parsed. A writer that has the fields
  always round-trips them.

If a writer lacking these fields drops a lifecycle timestamp,
[`jit migrate lifecycle-timestamps`](cli-commands.md#jit-migrate-lifecycle-timestamps)
can reconstruct it **only when the event log records the corresponding
transition**. A timestamp with no logged transition — notably `first_ready_at`
for a dependency-free issue auto-promoted to `ready` at creation, which emits no
state-change event — cannot be recovered and stays unset (as above).

## Configuration File

`config.toml` controls repository behavior:

```toml
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[validation]
default_type = "task"
```

The hierarchy's type names are repository-defined; `bug` and `enhancement` in
this repository's dogfood configuration are not `jit init` defaults. Individual
rules live in `rules.toml`; the `[validation].strictness` key globally modulates
which of their violations block operations. See
[Configuration Reference](configuration.md) for active options.

## Event Log Format

`events.jsonl` is the append-only event log. Its serialization — JSON Lines, one
tagged record per line, with sample records — is specified in
[Storage Record Layout](storage-records.md#event-log-records); the tag
vocabulary, each tag's scope, and which tags carry an `issue_id` are the
generated [Event Log Tags](events.md) catalog.

## Gate Run Records

A gate execution records its result under `gate-runs/`. The run's path, the
record's fields, and how an unset field is written are specified in
[Storage Record Layout](storage-records.md#gate-run-records).

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

## Validation Rules Registry

`rules.toml` stores the ruleset `jit validate` enforces, as a `[[rules]]` array
of tables. A rule pairs a selector (which issues it applies to) with an
assertion (what must hold); `severity` controls whether a violation blocks a
write or is advisory. Assertions that reference a raw JSON Schema point at a
file under `schemas/`.

The rule anatomy, selector predicates, assertion kinds, and worked examples are
documented in full in the
[Validation Rules how-to](../how-to/validation-rules.md).

## Graph Template Registry

`templates.toml` declares named, parameterized subgraphs that
`jit apply <template> <container>` instantiates onto a container issue: nodes
with anchors, roles, dependency edges, and optional document/label transforms.
Nothing in the mechanism is hardcoded; templates are entirely repository
configuration.

The template anatomy and a worked example are documented in
[Adopt the Planning Bracket](../how-to/adopt-planning-bracket.md); the
plan-before-fan-out concept it typically encodes is introduced in
[The Planning Bracket](../concepts/planning-bracket.md).

## Invariants Registry

`invariants.toml` declares project invariants as a `[[invariants]]` array of
tables. Each entry is a project-scoped addressable item at `@/invariant/<id>`
(see [Item Addresses](item-addresses.md)):

```toml
[[invariants]]
id          = "dag-acyclic"
statement   = "Cycle detection runs before every dependency operation; the graph stays acyclic."
kind        = "enforced"
enforced-by = "@/gate/cargo-ci"
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Self-id; the invariant's address is `@/invariant/<id>` |
| `statement` | string | The invariant's canonical statement |
| `kind` | enum | `enforced` (a named rule or gate mechanically guards it) or `advisory` (documented intent, not yet mechanically asserted) |
| `enforced-by` | string? | Address of the rule or gate that enforces it, when `kind = "enforced"` |

`jit invariant check` surfaces enforcement drift between this registry and the
rule/gate registries; `jit project render` projects it into a target document
via a `[projection.*]` table (see [Guarantees](../concepts/guarantees.md) for the
concept and an example projection target).

## The `.git/jit/` Control Plane

Multi-agent coordination state (claim leases and locks) lives under `.git/jit/`,
the **shared control plane**, not `.jit/`. `.jit/` is per-worktree data;
`.git/jit/` is shared across every worktree of the same repository (via git's
common directory), because a lease must be visible to every worktree racing to
claim the same issue. `jit claim` requires a git repository for this reason:
outside one, it fails with a typed `ClaimRequiresGitError` (exit code 10) rather
than falling back to a per-worktree lease store.

```
.git/jit/
├── claims.jsonl         # Append-only audit log of claim operations
├── claims.index.json    # Derived cache of currently active leases
└── locks/               # Advisory lock files guarding claim-log operations
    └── claims.lock
```

- **`claims.jsonl`**: an append-only, newline-delimited log of claim
  operations (`Acquire`, `Renew`, `Heartbeat`, `Release`, `AutoEvict`,
  `ForceEvict`), each entry carrying a monotonic `seq` for total ordering.
- **`claims.index.json`**: a cache of active leases derived from
  `claims.jsonl`, atomically updated from claim operations rather than
  hand-edited. Each lease record carries `lease_id`, `issue_id`, `agent_id`,
  `worktree_id`, `branch`, `ttl_secs`, `acquired_at`, `expires_at`,
  `last_beat`, and `stale`. A `ttl_secs` of `0` marks an indefinite lease,
  kept alive by a `Heartbeat` log operation that updates `last_beat` in this
  index; `jit claim heartbeat` does not write a per-agent heartbeat file.
- **`locks/claims.lock`**: an advisory file lock guarding atomic reads and
  appends against `claims.jsonl` and `claims.index.json`.

`jit claim` command usage (acquire, release, renew, heartbeat, status, list,
force-evict) is documented in the [Claim reference](claim.md).

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

- Binary support **≥** repository version: operates normally. An older
  repository is read as-is and keeps its marker; `jit` performs no implicit
  migration. Data migrations are explicit and idempotent — see `jit migrate`.
- Binary support **<** repository version: the binary refuses to operate and
  exits nonzero (exit code 10, external-dependency family) with a single-line
  error naming both the repository's format version and the version the binary
  supports. The fix is to upgrade `jit` (from a source checkout,
  `./scripts/install-jit.sh`)
  rather than treating it as repository corruption.

`config.toml` separately records a `[version] schema` describing the
configuration-file layout:

```toml
[version]
schema = 2
```
