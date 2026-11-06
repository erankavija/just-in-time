# Storage Format Reference

> **Diátaxis Type:** Reference

JIT stores its issue and configuration data in a data root, normally `.jit/`
at the repository root. Machine-local runtime state lives there too, gitignored.
Multi-agent lease coordination lives under `.git/jit/` (see
[The `.git/jit/` Control Plane](#the-gitjit-control-plane)).

Record fields and serialization are generated from the code that writes them
in [Storage Record Layout](storage-records.md).

## Directory Structure

```
.jit/
├── .gitignore        # Engine-owned ignore entries for the data root
├── index.json        # Format version and deletion tombstones
├── config.toml       # Repository configuration
├── gates.toml        # Gate registry definitions
├── templates.toml    # Graph template registry
├── rules.toml        # Validation rules
├── invariants.toml   # Invariants registry
├── events.jsonl      # Append-only event log
├── issues/           # One JSON file per issue
│   └── <uuid>.json   # Issue data
├── gate-runs/        # Runs by issue; reusable verdicts by gate and inputs
│   ├── <issue-id>/
│   │   └── <run-id>.json
│   └── verdicts/<gate-key>/<inputs-digest>.json
├── archive-evidence/
│   ├── container/<issue-id>.json
│   └── document/<sha256-of-normalized-path>.json
├── schemas/          # JSON Schema files referenced by rules.toml
└── tmp/              # Machine-local scratch directory
    └── redo.record   # The pending redo record, while a mutation leaves one
```

`templates.toml` is authored per project; `jit init` does not scaffold it.
`config.toml`, the empty `gates.toml`, the empty `invariants.toml`, and
`rules.toml` (with the rules the declared registry generates, and their
`schemas/` files) are scaffolded by `jit init`, so the `gate`, `invariant`, and
`rule` item kinds each resolve against a registry that exists before anything is
authored into it — the `rule` kind itself only once a `[item_kinds]` declaration
reaches the repository.

A live repository also carries gitignored, machine-local files directly under
`.jit/`: `server.log`, `server.pid.json`, a fixed set of lock files, `tmp/`, and,
for a data root inside a checkout, `worktree.json` (see
[Worktree Identity](#worktree-identity)). The data root's own `.gitignore` is derived state, not
authored: the engine composes its delimited engine-owned region, whose single
entry is the scratch directory `tmp/`, so a repository never commits a pending
redo record. Initialization writes it and `jit validate --fix` restores it,
each preserving every line outside that region, and the repository's root
ignore file is neither read nor written. The store's fixed lock set under
`.jit/` is
`.repo-write.lock`, `.index.lock` and `.events.lock`; it is
independent of the number of issues. The repository-sibling
`.jit-bootstrap.lock` and the `.git/jit/locks/claims.lock` control-plane lock
complete the fixed set, and each checkout over a data root outside every
checkout adds one `worktree.lock` (see [Worktree Identity](#worktree-identity)).
The `.jit/` storage locks and `.jit-bootstrap.lock` retain
their inodes for advisory-lock race safety; they are opened with
create-if-absent semantics and are not unlinked. No lock and no
`worktree.json` write creates `.jit/`; `jit init` alone creates the data root.
Against an absent data root, acquiring `.repo-write.lock` and recording
`worktree.json` fail as repository-not-found, and `.jit-bootstrap.lock`, beside
the data root, is acquired without it. A shared read never makes write
access a precondition: where it cannot create or open the lock file for writing
it opens an existing one read-only, which still excludes a writer, and where no
regular lock file can be opened at all it reads without an advisory lock and
records a storage warning. Publication is atomic, so such a read still observes
a complete version rather than a partial one. An exclusive acquisition still
fails when it cannot open its lock file. The claims lock belongs to the separate
recovery-aware control plane described below.

Issue reads do not create per-issue lock files. A pending redo record lives in
the scratch directory `.jit/tmp/` as `redo.record` (see
[Publication and Recovery](#publication-and-recovery)). All of these are
runtime state, not part of the versioned data format.

Lease records live under `.git/jit/`, not `.jit/`. See
[The `.git/jit/` control plane](#the-gitjit-control-plane) below.

## Publication and Recovery

A repository mutation publishes through one **redo record**: the directories it
creates, the new contents of every file it writes, and the paths it removes, except
that the event log is the one file the record extends rather than replaces,
with new lines positioned at its committed length (`@/charter/D-17`). Every
redo record over an existing data root appends at least one event, so the log's
length counts every commit: a write that records no domain event, such as
`jit config set` or `jit project render`, appends one `files_published` event
naming the paths it writes, and a record without an append is refused before it
commits. The mutation fails, before anything durable is written, if
any path it creates, writes, or removes changed since it read it. The record is
then written and synced under the scratch directory; that write is the
mutation's commit point. Application creates the directories, replaces each
other file through the temp-file and rename pattern (`@/inv/atomic-writes`),
removes the listed paths, extends the event log, and ends by removing the
record.

The [redo record implementation](../../crates/jit/src/storage/redo_record.rs)
defines the canonical format version as 2. A record with worktree operations
binds them to the committing session's absolute worktree-root path and
filesystem identity; a data-only record has no worktree binding. Before any
replay write, recovery verifies the original root without following symlinks.
If it is missing, moved, replaced, inaccessible, or symlinked, recovery fails
with a typed error and leaves the record intact without replaying any of its
operations. Recovery never substitutes a newly configured root. Data-only
records recover without a code root.

A command that changes the repository, and `jit recover`, opens a mutation
session, and a session that opens over a leftover record applies it before
serving the command. `jit snapshot export` publishes, so it reads under that
session's lock. Application is idempotent, so an interrupted application
converges on the same state when repeated. A leftover record that does not
parse, including one whose own write was interrupted, is refused: the session
open fails with an error naming the record. Repository discovery recognizes a
pending record as well as an initialized `index.json`, so a repository whose
index a crash left unwritten still reaches recovery. Recovery completes the
older committed record before selecting a new worktree root or applying the
linked-checkout write policy to new work. It preserves the original event and
override audit; it does not authorize a new mutation or create another override
event. With no pending record, a policy refusal precedes mutation locks and
writes; with a pending record, only completion of that record may precede it.

A read through the read driver takes no writer lock and does not apply
redo records (`@/charter/D-25`). It counts only when no redo record was present
and the event log did not grow between its open and the end of its reads;
otherwise it retries. After bounded attempts it returns `RETRYABLE_CONFLICT`:
retry the read, and run `jit recover` if a pending redo record persists. These
commands read through the driver:

- `jit validate`, except `--fix` without `--dry-run`, `--branch-drift` and
  `--leases`
- `jit gate preset list` and `jit gate preset show`
- `jit doc check-links`
- `jit invariant check`
- `jit issue batch-create --dry-run`
- `jit profile apply --dry-run` and `jit profile diff`
- `jit profile add --dry-run` and `jit profile pack --dry-run`

Issue listings also admit one snapshot over every contributing data root before
serving it. Other read-only commands may read individual files as they stand,
which until a record is applied may be the state before the mutation or partly
applied.

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
      "updated_at": "2026-01-15T10:30:00Z",
      "run_id": "<gate-run-uuid>"
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
| `gates_status` | object | Per-gate status with timestamps; `run_id` names the gate run the status was projected from, the run every latest-run view shows, and is absent until a run is recorded |
| `labels` | string[] | Format: `namespace:value` |
| `content_format` | enum? | `markdown`, `html`, or `xml`; selects the parser for `description`. Absent inherits `[validation].content_format`, falling back to `markdown`. Omitted from JSON when unset |
| `documents` | object[] | Linked document references |
| `context` | object | Arbitrary metadata |
| `created_at` | timestamp | When the issue was created (RFC 3339) |
| `updated_at` | timestamp | When the issue was last modified (RFC 3339) |
| `first_ready_at` | timestamp? | When the issue FIRST entered `ready` (see below) |
| `claimed_at` | timestamp? | When the issue was FIRST claimed/assigned |
| `done_at` | timestamp? | When the issue FIRST reached `done` |
| `archived_from` | enum? | The state the issue held before entering `archived`, making `archived` terminality-preserving. Required while `state` is `archived`, absent otherwise; cleared on revive. Loading an `archived` record without it fails |

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

- `first_ready_at` — set the first time the issue enters `ready`, including
  creation in `ready`.
- `claimed_at` — set at the first claim or assignment.
- `done_at` — set the first time the issue reaches `done`. **Re-opening and
  re-completing does not overwrite it** ("first occurrence" semantics apply to
  all three).

Each field is **optional** and omitted from the JSON when unset (an issue that
never reached the milestone, or one predating these fields).

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

A lifecycle timestamp dropped by a writer lacking these fields stays unset.

## Data and Worktree Roots

The data root holds issues and configuration. `JIT_DATA_DIR` selects it when
set; otherwise JIT discovers `.jit/` from the invocation. A separate worktree
root can name the code checkout, including when the data root is a separate Git
checkout or a `.jit/` directory with its own `.git`:

```toml
[worktree]
root = ".."
```

`[worktree].root` and `JIT_WORKTREE_ROOT` accept absolute paths or paths relative
to the selected data root. The environment variable takes precedence. With
neither set, JIT uses the top level of the Git checkout containing the data
root. If the data root is outside Git, it uses the discovered `.jit/` parent
when that is the selected data root, or otherwise the invocation directory. A
selected root must be an
existing directory outside the data root and cannot contain symlinks in its
ancestry. Invalid selections report the path and the setting that supplied it.

Initialization scaffolds worktree merge attributes only when the nested data root
and the code root belong to the same Git checkout. An independent data repository
does not cause initialization to write attributes in the outer code checkout.

Gate checkers run from the code worktree root (or their configured subdirectory),
and gate runs record that checkout's commit and branch. Explicit branch-drift
validation and worktree-relative documents also use the code checkout. The
history guard for gate-registry mutations uses the data repository's history;
an external data root outside Git does not inherit the code checkout's history.
Claims coordination uses the data repository's Git common directory, as
[the control-plane reference](#the-gitjit-control-plane) describes.

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

The log's committed length is the end of its last newline-terminated line. A
final line without a terminating newline is an uncommitted append: readers ignore
it and the next event append, positioned at the committed length, overwrites it.
Blank lines are skipped. An unknown event tag or any other malformed
newline-terminated line, including a malformed middle line, fails the read
with an error naming the line.

## Gate Run Records

A recorded gate run lives at `gate-runs/<issue-id>/<run-id>.json`. The issue's
gate state names its latest run in `run_id`; latest-run views read that record
without scanning prior runs. A gate state without `run_id` has no latest run.
The record's fields and serialization are specified in
[Storage Record Layout](storage-records.md#gate-run-records).

### Verdict records

`gate-runs/verdicts/<gate-key>/<inputs-digest>.json` names the source issue and
run for a reusable verdict over the gate's declared inputs. The latest recorded
run that executed the checker and passed or failed replaces that key's record.
Reuse reads that record and the run it names. See
[Gate Verdict Records](storage-records.md#gate-verdict-records) for the fields.

## Archive Evidence Records

`archive-evidence/<target-key>.json` holds a target's ordered archive executions.
The target key is `container/<issue-id>` or
`document/<sha256-of-normalized-path>`, with the document digest taken over its
normalized repository-relative path. Archive preview and execution read this
record for prior evidence. Each execution records the source and destination
paths it had at the time, so those paths are historical and may not exist in
the current tree. See
[Archive Evidence Records](storage-records.md#archive-evidence-records) for the
fields.

Gate runs, verdict records, and archive evidence are written in the same redo
record as the repository change that produces them. Lease records use their
own commit point under [the control plane](#the-gitjit-control-plane).

## History Files

These paths hold history; the table names their readers.

| Path | Readers |
|------|---------|
| `.jit/events.jsonl` | `jit events tail`, `jit events query`; `jit worktree store-divergence` reads the compared stores' logs; whole-repository `jit validate` and the `repository_validation` checker read the whole log for record fidelity |
| `.jit/gate-runs/` | `jit gate status <id> --all` or `--limit <N>` and the server's gate-run list (`GET /api/issues/:id/gate-runs`) read an issue's run history; whole-repository `jit validate` and `repository_validation` read every gate-run and verdict record for fidelity |
| `.jit/tmp/` | Can hold appended event bytes in `redo.record`; mutating commands and `jit recover` read a pending record to complete publication |
| `.git/jit/claims.jsonl` | None; no command reads claim history |

No other command reads history, and no file is derived from a history file.
Ordinary commands address current records by key, including named gate runs
and verdict records under `gate-runs/`, rather than scanning history. Issue
operations read only the event log's length and final line, without parsing a
record, to locate the committed append position and detect concurrent changes.
Default `jit search` excludes the data-root history paths above; explicit file
selection can include them (see [Repository Search](cli-commands.md#repository-search)).

## Gate Registry

`gates.toml` stores gate definitions as a `[[gates]]` array of tables. Each
entry carries its own lowercase-kebab `key`, rather than being indexed as an
object field; a registry holding any other spelling fails to load:

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

[gates.inputs]
roots   = ["Cargo.toml", "crates"]
exclude = ["crates/*/fixtures/**"]
```

`[gates.inputs]` is optional. It names the repository files the checker reads,
as roots minus glob exclusions; an evaluation digests them and reuses a prior
run's verdict over the same digest instead of executing the checker again. A
gate that omits the table executes its checker on every evaluation. See
[Custom Gates](../how-to/custom-gates.md#declare-the-files-a-checker-reads).

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

A template names gates rather than carrying them. Each name in a node's or
anchor's `gates` resolves, when the template is applied, to a gate preset or a
key in the [gate registry](#gate-registry). A `rule_validation` gate in turn
names a rule in the [rules registry](#validation-rules-registry), so a gate
evaluating a configured rule runs only where that rule is declared. A
`templates.toml` is therefore one file of a configuration set rather than a
distribution unit: it travels with the gates and rules it names, in a
[profile package](profiles.md) or copied together with them by hand.

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
claim the same issue. The repository is the one holding the data root; for a
data root outside every checkout, it is the one holding the invoking directory.
`jit claim` requires a git repository for this reason: with neither, it fails
with exit code 10 rather than falling back to a
per-worktree lease store.

```
.git/jit/
├── leases/
│   └── {issue-id}.json  # The issue's lease; absent when it has none
├── claims.seq           # The last claim-log sequence number
├── claims.jsonl         # Append-only history of claim operations
└── locks/               # Advisory lock files guarding lease changes
    └── claims.lock
```

A lease names its holder's `worktree_id`, which
[Worktree Identity](#worktree-identity) defines. A primary checkout's identity
files can sit in this directory over a data root outside every checkout, and are
separate from the claims state.

- **`leases/{issue-id}.json`**: the issue's lease. Each record carries
  `lease_id`, `issue_id`, `agent_id`, `worktree_id`, `branch`, `ttl_secs`,
  `acquired_at`, `expires_at`, and `last_beat`. A `ttl_secs` of `0` marks an
  indefinite lease, kept alive by `jit claim heartbeat` updating `last_beat`.
  Claim operations change a record under `locks/claims.lock` by writing a
  synced stage file, renaming it over the record and syncing the directory, or
  by unlinking it; that rename or unlink commits the change. Other names in
  `leases/`, such as an interrupted operation's `{issue-id}.json.tmp` stage
  file, are not leases; `jit recover` removes stale stage files.
  `jit claim list` and `status` read the records without the lock.
- **`claims.seq`**: the last sequence number handed to `claims.jsonl`, as a
  decimal integer, advanced after each committed change.
- **`claims.jsonl`**: an append-only, newline-delimited history of claim
  operations (`Acquire`, `Renew`, `Heartbeat`, `Release`, `AutoEvict`,
  `ForceEvict`, and `Reclaim`, recorded when `jit recover` removes a stale
  indefinite lease), each entry carrying its `seq`. An operation appends its
  lines in one write after advancing `claims.seq`, so an interruption loses at
  most that operation's history lines and never a lease. No lease read or
  change reads it.
- **`locks/claims.lock`**: an advisory file lock serializing lease changes.

`jit claim` command usage (acquire, release, renew, heartbeat, status, list,
force-evict) is documented in the [Claim reference](claim.md).

## Worktree Identity

Each checkout has one stable `worktree_id`, recorded in a machine-local
`worktree.json` with its branch and top-level path. The file's location follows
the data root:

- **A data root inside a checkout** has one owner and holds the file:
  `<data root>/worktree.json`. A copy reaching a linked checkout gets a new
  identity.
- **A data root outside every checkout**, invoked from inside one, can serve many
  checkouts, so each keeps its file in its own git directory:
  `<git dir>/jit/worktree.json`, beside the `worktree.lock` that serializes changes
  to it. `git rev-parse --absolute-git-dir` names that directory: the common
  directory for a primary checkout, `<common dir>/worktrees/<name>` for a linked
  one. Such a data root holds no identity, and ignores a `worktree.json` found in
  it.
- **A data root and an invocation both outside every checkout** have no checkout
  to identify and no claims to attribute; `jit worktree` and `jit claim` report the
  git requirement.

A checkout found away from its recorded path reports one relocation, keeps its id
and records the new path. `git worktree move` keeps the linked checkout's git
directory and so its id; `git worktree remove` discards that directory and its
identity with it.

## Versioning

The repository's on-disk **format version** is the `schema_version` field in
`index.json`:

```json
{ "schema_version": 4, "deleted_ids": [] }
```

Issue membership is the `.json` files under `issues/`. `deleted_ids` holds
tombstones, which let a linked checkout's delete hide the primary checkout's
file. The first store that mentions an id decides, and within a store a file
wins over a tombstone.

The marker changes whenever an on-disk layout or interpretation changes, and a
binary accepts exactly one format. On startup every command that opens the
repository compares the marker against that format. On any other version,
older or newer, the binary refuses to operate and exits nonzero (exit code 10,
external-dependency family) with a single-line error naming both versions, so
an older binary and a newer one each refuse the other's repository. `jit`
performs no migration; moving a repository between formats is an explicit
rewrite made alongside installing the matching binary.

An index carrying any other field, such as `all_ids`,
fails validation.

`config.toml` separately records a `[version] schema` describing the
configuration-file layout:

```toml
[version]
schema = 2
```
