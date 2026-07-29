# CLI Commands Reference

> **Diátaxis Type:** Reference

## CLI JSON contracts

CLI commands that accept `--json` print machine-readable JSON to stdout. Success
responses are the command payload itself, not a `{ "success": true, "data": ... }`
envelope. Commands that return objects may include a top-level `message` field
for human-readable context.

Example successful issue update — a lightweight confirmation, not the full
issue (fetch the body with `jit issue show`):

```json
{
  "id": "5c581575-bef8-4ee6-be83-7598fd22b557",
  "short_id": "5c581575",
  "state": "done",
  "updated_at": "2026-04-28T18:25:16.699033997Z",
  "message": "Updated issue 5c581575 to Done"
}
```

For failures reached after successful argument parsing and command dispatch, a
`--json` invocation emits exactly one error envelope on stdout, the command's
payload stream. Its reported `error.code` is a registered
[machine-readable error code](error-codes.md), and that code determines the
process exit status in the [Exit Codes reference](exit-codes.md). Those generated
references are the canonical vocabulary and status taxonomy; this section is the
canonical statement of the machine-readable failure contract.

Clap parser diagnostics raised before dispatch remain outside this envelope
guarantee.

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
    "message": "Cannot transition to 'ready': issue blocked by 1 unmet dependencies",
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
          "status": "pending",
          "mode": "auto"
        }
      ],
      "remediation": [
        "jit gate status-all work-id",
        "jit gate evaluate work-id code-review",
        "jit gate status work-id code-review --all  # run history"
      ]
    },
    "suggestions": [
      "jit gate status-all work-id",
      "jit gate evaluate work-id code-review",
      "jit gate status work-id code-review --all  # run history"
    ]
  }
}
```

Each gate blocker carries the gate's registry `mode`. For a `"mode": "manual"`
blocker the remediation hint names the attested form —
`jit gate evaluate <id> <gate> --by <attestor>` — because a manual gate's
bare `evaluate` exits 2 without an attestor.

## Command and flag aliases

A few convenience aliases exist for the names agents reach for most often. They
behave identically to their canonical forms:

| Alias | Canonical | Notes |
|-------|-----------|-------|
| `jit dependency ...` | `jit dep ...` | Dependency management commands |
| `jit document ...` | `jit doc ...` | Document reference commands |
| `jit issue list` | `jit query all` | Same filters/flags (`-s`/`-a`/`-p`/`-l`, `--full`, `--json`); identical output. `-l`/`--label` is repeatable and ANDed |
| `jit list` | `jit issue list` | Top-level spelling of the same listing, with the same filters |
| `jit rdeps <id>` | `jit graph rdeps <id>` | Top-level spelling; takes the same `--depth`/`--json` |
| `jit graph dependencies` | `jit graph deps` | Long spelling of the upstream view |
| `jit graph downstream` | `jit graph rdeps` | Long spelling of the reverse view |
| `jit query ready` | `jit query available` | Visible alias |
| `jit gate eval` | `jit gate evaluate` | Visible alias |
| `jit item resolve` | `jit item show` | Distinct verb, identical behavior |
| `jit issue update <id> --add-label <label>` | `... --label <label>` | `--add-label` is an accepted alias for `--label` |
| `jit doc add <id> <path> --title <t>` | `... --label <t>` | `--title` is an accepted alias for `--label` |

```bash
# These pairs are equivalent
jit dependency add a b      # == jit dep add a b
jit document list <id>      # == jit doc list <id>
jit issue list --json       # == jit query all --json
jit list --json             # == jit issue list --json
jit rdeps abc123            # == jit graph rdeps abc123
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

One flag hints the same way. `jit validate --divergence` (exit code 2) names
both commands that word could mean:

```
$ jit validate --divergence
Error: `--divergence` is not a `jit validate` flag. Use `jit validate --branch-drift` for git branch drift, or `jit query divergence` for membership labels the DAG does not back.
```

`jit label` itself only inspects the namespace registry (`jit label
namespaces`, `jit label values`) — it never touches an issue's labels. See
`jit label --help`.

Under `--json`, the same hint is in `error.message` with code
`INVALID_ARGUMENT`, exit code 2 — identical to any other usage error.

## Archive planning and execution

The `archive` command group previews dependency-aware plans by default and
executes only when `--execute` is explicit:

```bash
jit archive document dev/active/design.md
jit archive document dev/active/design.md --json
jit archive container 2f84c930
jit archive container 2f84c930 --json
jit archive candidates
jit archive candidates --json
jit archive document dev/active/design.md --execute
jit archive container 2f84c930 --execute --json
```

Both target forms use the same planner. Without `--execute`, they are read-only.
A document target
includes its recursively supported local bundle. A container target includes
documents attached to the container and its resolved-hierarchy descendants;
ordinary sequencing dependencies do not define membership.

Human output reports the same target, policy status, eligibility, action counts,
artifacts, owners, embedded edges, reference changes, pending deletions,
evidence, warnings, and blockers as JSON. User-facing issue identifiers are
short IDs. JSON retains full durable IDs in owner and target records.

For a preview, `--json` prints the schema-version-1 artifact-plan object directly. Its top-level
fields are `schema_version`, `target`, `destination_root`, `eligible`,
`policy_status`, `action_counts`, `count`, `artifacts`, `blockers`, and
`warnings`; it does not add a `message` field. Artifact order is deterministic
by normalized source path and version. Each blocker carries `code` and `path`;
a blocker caused by lifecycle state (`non-terminal-target`,
`document-non-terminal-owner`) additionally carries a `guidance` string naming
the permitted next action.

A preview also reports the in-content citations a relocation would break as
`moving-path-citation` warnings, read from the declared `citation_scan_roots`
universe — the repository-relative directories and files whose text the scan
reads, which need not lie under the development root ([Citation scan
roots](configuration.md#citation-scan-roots)). Only a relocating artifact earns
them, and every occurrence is its own warning whose `path` names the citing file
with the occurrence's 1-based line and column, spelled
`<citing path>:<line>:<column>`. The warnings are advisory: they carry no action
and no blocker, so eligibility, `--execute`, and issue transitions all behave as
they would with no citation present. Execution relocates bytes and relinks
document records while rewriting no document content, so a citing file survives
byte for byte and keeps a stale citation until an adopter edits it. An execution
plan is built without citation evidence, so a preview is where these warnings
are read.

`jit archive candidates` is the read-only container report. It lists every
effectively terminal issue — `Done`, `Rejected`, or `Archived` from one of those
— whose `type:*` is configured at a non-leaf level of the live
`[type_hierarchy]`; it does not hardcode type names. This is the same predicate
the direct archive path gates coupled retirement on, so an Archived-from-terminal
container it would reconcile also appears here. Active, untyped, unknown-type,
leaf, and Archived issues whose pre-archive state was non-terminal (or a legacy
archived record with no recorded origin) are excluded. Each selected container is
fully evaluated through the same resolved-hierarchy planner as `archive
container`, including zero-document containers and ineligible plans. Ordinary
sequencing edges do not enlarge a candidate's resolved subtree.

The JSON shape is exactly
`{"schema_version":1,"count":N,"candidates":[...]}`. Every entry in
`candidates` is the complete schema-version-1 target-plan object described
above, not a summary: policy status, ownership, action counts, artifacts,
evidence, warnings, and all blockers (including destination conflicts) remain
available. Human output renders the identical evaluated list and retains the
reasons an entry is ineligible. Candidate order is deterministic by full target
ID, while the human view identifies targets by short ID.

An issue-linked path that exists but is not a regular file or symbolic link
(for example, a directory) is represented in its target plan with action
`block` and blocker `unsupported-artifact-type`; it does not abort the report
or omit other candidates. Direct document and container previews use the same
diagnostic, and `--execute` refuses the ineligible plan without mutation.
A supported local reference whose target is a directory is navigation: the
target contributes no artifact entry, no edge, and no warning, leaving the
referencing plan's eligibility untouched. A reference target that is a symbolic
link is evidenced as the link itself, so one pointing at a directory is
inventoried and blocked as `symlink-artifact`. Unexpected metadata or storage
failures still fail planning instead of being converted into this blocker.

The configured development root bounds every plan, and which areas inside it
are managed or permanent is repository policy (see [Development-area
classification](configuration.md#development-area-classification)). A selected
root the development root does not contain is retained: the plan schedules no
destination for it, leaves its source in place, and carries
`outside-development-root` evidence. The `unmanaged-selected-root` blocker
names a selected root the development root contains but no configured area
matches, so a retained out-of-root artifact reports none. Discovery also stops
at such an artifact instead of following the references inside it, so a linked
source file, script, agent asset, or repository-root document keeps its single
copy and draws nothing further into the plan.

The candidates command has no age, retention, category, suggestion, or default
target behavior, and it has no `--execute` form. It never writes artifacts,
issue records, or events. Missing and partial documentation policy therefore
remain `unconfigured` and `incomplete`; they are reported rather than filled by
mutation-authorizing defaults. The shared planner also preserves its detailed
semantics here: the development-root boundary above holds unchanged, an
unmanaged embedded dependency carries `unmanaged-path` evidence and can only
copy or retain, and sources already beneath the configured archive root are
evaluated as already existing — direct roots retain, relative dependencies of
relocated parents copy to the current mirror, and root-relative or
staying-parent dependencies retain.

A blocked preview is still a successful read-only command and exits zero. Check
`eligible`, then inspect target-level and per-artifact `blockers`. In particular,
an absent `[documentation]` table reports `policy_status = "unconfigured"`; a
partial table reports `"incomplete"`. Neither state silently receives defaults
that would make the plan eligible, and the human view explicitly says archival
execution is disabled.

`--execute` never accepts a saved preview as input. It acquires the repository
write guard, recomputes the plan from current issue and filesystem state, and
refuses an ineligible result. A container must be effectively terminal
(`Done`/`Rejected`, or already `Archived` retired from one of those, which keeps
a reconciling rerun eligible); a non-terminal container is blocked with
`non-terminal-target`. A document target is refused while any direct or supported
embedded-closure owner is non-terminal (an owner archived from a terminal state
counts as terminal); a managed document with no owner remains eligible and
reports `no-owner` as informational evidence. When a blocker is caused by
lifecycle state, the refusal — in both the human message and the JSON blocker's
`guidance` field — names the permitted next action: complete or reject the
container, then re-run archival; a target already `Archived` from a
non-terminal state first revives to its recorded pre-archive state
(`jit issue update <id> --state <origin>`), then completes or rejects.

A successful container execution retires the container into the `Archived`
lifecycle state as its final durable step, recording the terminal state it came
from so it stays effectively terminal (see
[States](../concepts/core-model.md#states)). This is the only place the archive
command touches lifecycle state; the document relocation, `.jit-container`
marker, and `artifact_archive_executed` event are otherwise independent of it. A
rerun of an already-archived container reconciles to a no-op and does not
re-emit the transition.

For a container target, the preferred destination root is
`<archive_root>/<container-short-id>-<slug>/`, where `<archive_root>` is the
repository-authored policy value rather than a built-in path. A membership
label supplies `<slug>`, normalized into the same suffix an issue's artifact
directory carries; [Issue artifact
directories](configuration.md#issue-artifact-directories) specifies how a
container's `type:*` label, that type's membership namespace, and that
namespace's value on the container resolve to one slug. Every other shape,
including a membership value that normalizes to nothing, gives the bare
`<archive_root>/<container-short-id>/`. Labels and the short id are the whole
input to that name, so a container's title has no part in it and retitling one
leaves its destination where it is.

Execution creates a `.jit-container` marker containing the resolved full
container ID followed by a newline. The short ID and marker-recorded full ID
remain authoritative; the suffix is only a human-readable aid. Before choosing
a preferred root, planning scans the archive root's immediate non-symlink
directories for that exact full-ID marker. One match freezes and reuses the
existing directory even after the container's membership label changes. Multiple
matches block with deterministic `destination-conflict` findings. If no marker
matches, planning resolves to the unsuffixed
`<archive_root>/<container-short-id>/` when that path already exists, and to the
preferred root otherwise. Adopting an occupied unsuffixed directory
moves no data, and its markerless accounting and conflict checks still apply, so
a container's artifacts stay in the one directory that already holds them. A
marker naming another container or a markerless resolved directory with
unaccounted entries also blocks. The marker's `ArchivePublication.source` is
`null` because it is executor-generated ownership metadata, not a copied or
moved repository artifact.

Execution stages and verifies bytes, validates supported local links in the
proposed mirror layout, and publishes with atomic no-replace semantics. It then
applies only the plan's unpinned reference changes and invalidates cached asset
metadata for those moved references. One `artifact_archive_executed` event is
made durable after publications and reference changes but before any source is
deleted. A source is removed only when every selected durable reference points
to its destination and its SHA-256 and size still match the recorded identity.
If a later publication or reference update fails after an earlier publication
succeeded, the failing invocation first records the exact successful durable
subset; failed publications and uncommitted deletions are not claimed.

Deletion failures and files edited after planning are non-fatal
`deletion-failed` warnings; the source remains. Rerun the same command after an
interruption. Execution adopts identical occupied mirror content, repairs an
unrecorded adopted state with one reconciling event, discovers residual sources
through the inverse mirror layout, and retries only safe remaining work. A
stable no-op rerun appends no event. Destinations are never overwritten.

For execution, `--execute --json` prints a distinct schema-version-1 execution
result. Its top-level fields are `schema_version`, `target`,
`destination_root`, `publications`, `reference_changes`, `planned_deletions`,
`deleted_sources`, `warnings`, `event_appended`, and `reconciling`.
`publications` reports each newly published or newly adopted destination with
its source, SHA-256/byte-size identity, and `adopted` flag. The two deletion
arrays distinguish removals recorded before
attempt from sources actually removed. `event_appended` says whether this run
created an archive commit record; `reconciling` identifies a record that covers
durable adopted state from an interruption or externally replaced identical
content. Human execution output prints publication, reference-change, and
deletion counts followed by one `warning:` line per warning.

An ineligible `--execute` is an error: it exits non-zero and performs no
publication, reference update, archive event, or source deletion. This differs
from a blocked preview, which exits zero because it only reports the plan.

## MCP Tools Reference

jit ships an MCP (Model Context Protocol) server under
[`mcp-server/`](../../mcp-server/README.md) so MCP clients (such as VS
Code, and other agents) can drive jit through structured tool calls instead of
shelling out to the CLI.

The tool surface is **generated from the CLI schema**, so it stays in lockstep
with the commands documented above rather than being maintained by hand.
[`mcp-server/README.md`](../../mcp-server/README.md) is the authoritative
reference for setup, client configuration, and the live tool list; the
generation model in brief:

- **Tools mirror commands.** Every leaf command in `jit --schema` becomes one
  tool named `jit_<command_path>` — `jit doc assets list` is
  `jit_doc_assets_list`. A tool's parameters are its command's flags (hyphens
  become underscores, so `--add-gate` is `add_gate`); repeatable flags take
  arrays. Each tool's response carries the same JSON payload the command prints
  with `--json`.
- **Curated default listing.** `tools/list` advertises an agent-facing subset —
  the commands for finding, claiming, inspecting, and transitioning work,
  checking gates, and reading repository structure. The include/exclude decisions and their
  rationale live in
  [`mcp-server/curated-tools.json`](../../mcp-server/curated-tools.json).
- **Full set on demand.** Setting `JIT_MCP_ALL_TOOLS=1` exposes every generated
  tool, not just the curated subset.

```bash
cd mcp-server && npm install
# List the curated tools (add JIT_MCP_ALL_TOOLS=1 for the full generated set)
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | node index.js
```

### See Also

- [MCP Server README](../../mcp-server/README.md) - Setup, client configuration, and the tool list
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

**Applies to:** `--label`, `--gate`, `--add-gate`, `--remove-label`, `--remove-gate`, `--except`, `--fields`

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

## Repository Commands

### `jit init`

Initialize (or re-initialize) the `.jit/` repository in the current directory.
Idempotent: re-running over an existing repository never overwrites
`config.toml`, and leaves `index.json`/`events.jsonl` intact. `rules.toml`
keeps every custom rule and hand-edited policy field intact; the one
synchronization re-init performs (when default-origin rules remain enabled) is
the default `namespace-unique-*` row set, appended or dropped to match the
current `[namespaces]`/`[type_hierarchy]` registry so each row's `@/rule/<name>`
address stays resolvable. An append-only synchronization preserves the rest of
the file byte-exact; one that drops a row re-serializes the document and may
canonicalize unusual-but-valid TOML syntax spellings elsewhere in the file —
semantically lossless, with every rule, comment, and unrelated table preserved.

```bash
jit init [--hierarchy-template <name>] [--profile <profile-id>] [--json]
```

`--hierarchy-template` selects the type hierarchy seeded into `config.toml`
(`default`, `extended`, `agile`, `minimal`); an unknown name is a usage error
(exit `2`).

`--profile <profile-id>` applies an embedded repository profile as part of
initialization. `jit init --profile jit-dogfood` is the preferred setup for
JIT's portable workflow; plain init remains methodology-neutral. For a fresh
repository, the neutral scaffold and profile projection are planned, validated,
and published together. If the data root is absent, JIT stages the complete root
beside its destination and publishes it with an atomic no-replace rename; an
occupied destination is never overwritten. The same flag can complete and apply
the profile to an existing partial repository. See
[Repository Profiles](profiles.md) for the canonical package, conflict,
transaction, recovery, and lifecycle contract.

Inside a git repository, init also creates a worktree identity
(`repository_id`, format `wt:<8-hex>`) used for lease/claim coordination, and
sets up a `.gitattributes` union-merge driver for `events.jsonl` under the
selected data directory (default `.jit/events.jsonl`, so concurrent worktrees'
event appends don't conflict) —
creating the file if absent, or appending the jit block to an existing one that
doesn't already carry it. Lease/claim coordination state lives under `.git/jit/`
(an untracked per-worktree control plane), not in the versioned `.jit/` tree.

`--json` reports what this run actually did rather than the full idempotent
set init always ensures — on a re-init `created_paths` is empty and
`modified_paths` lists only a `.gitattributes` the run had to amend (the
in-place refreshes init performs, such as the `namespace-unique-*` row sync
and projection republishing, are not path-listed):

```json
{
  "repository_root": "/path/to/repo",
  "data_dir": "/path/to/repo/.jit",
  "repository_id": "wt:d5f301ab",
  "hierarchy_template": "default",
  "gitattributes_status": "created",
  "created_paths": [
    ".jit/index.json",
    ".jit/gates.toml",
    ".jit/events.jsonl",
    ".jit/config.toml",
    ".jit/rules.toml",
    ".gitattributes"
  ],
  "modified_paths": [],
  "profile": null,
  "message": "Initialized jit repository (worktree: wt:d5f301ab)"
}
```

`gitattributes_status` is `not_applicable` outside a Git worktree (or when the
data directory is outside it), `unchanged` when the required block already
exists, and otherwise `created` or `modified` in step with the path lists.
`created_paths` lists files that did not exist before this run; `.gitattributes`
appears there only when it didn't exist and was created fresh. If it already
existed without the jit merge-driver block, this run instead appends to it and
lists it under `modified_paths`; if it already carried the block, neither list
mentions it.

`repository_id` is `null` outside a git repository. The unknown-template
failure and the repository-format-too-new startup failure (see **Scripting
and Automation § Exit Codes** below) both emit the standard `--json` error
envelope (`INVALID_ARGUMENT` / exit `2`, `REPOSITORY_FORMAT_TOO_NEW` / exit
`10`).

When `--profile` is present, `profile` contains the same
`ProfileApplyResult` returned by `jit profile apply`; otherwise it is `null`.

## Profile Commands

Profile inspection works without an initialized repository. Application targets
the current JIT repository and runs mandatory transaction recovery before
planning or writing.

### `jit profile list`

List the immutable profiles embedded in the running binary:

```bash
jit profile list [--json]
```

Human output shows each profile's ID, version, compatible JIT range, embedded
origin, and whether a matching stored provenance record exists. This record
check does not read every installed target. JSON uses the standard list envelope
`{"count": N, "profiles": [...]}`. Each profile entry carries `id`, `version`,
`origin`, `jit`, and `applied`.

The running binary is authoritative for the live values; scripts should inspect
the returned fields rather than copy package identity or compatibility values
from prose.

### `jit profile show`

Inspect one embedded package:

```bash
jit profile show <PROFILE_ID> [--json]
```

Human output summarizes package identity, compatibility, hashes, contribution
and asset counts, and installed state. JSON returns `ProfileShowResult`: the
complete parsed manifest, `origin`, `package_hash`, `target_hashes`,
`file_count`, `byte_size`, and the parseable stored `applied` provenance record
when one is present. `show` does not compare that record with current target
bytes; use `jit profile apply <PROFILE_ID> --dry-run` for exact current-state
verification.

### `jit profile apply`

Preview or apply an embedded profile to the current repository:

```bash
jit profile apply <PROFILE_ID> [--dry-run] [--json]
```

`--dry-run` builds and validates the exact plan without writing. JSON returns
`ProfilePlanResult`, including `status` (`would_apply` or `unchanged`),
`plan_hash`, and the sorted target list with each action (`create`, `update`, or
`unchanged`) and executable intent.

Without `--dry-run`, JSON returns `ProfileApplyResult`: profile identity,
`status` (`applied` or `unchanged`), `plan_hash`, an optional
`transaction_id`, and non-fatal cleanup warnings. Exact reapplication is a
successful no-op. Application and all coupled derived targets use the canonical
recoverable multi-target transaction described in
[Repository Profiles](profiles.md), including strict managed-region composition.

Unknown IDs are not-found errors (exit `3`). Conflicts, invalid package state,
final-state validation failures, filesystem failures, and recovery-required
conditions use the shared typed error envelope and exit-code taxonomy. The
[Repository Profiles reference](profiles.md) defines what application may
change and the v1.0 features that do not exist.

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
- `Commit` — short and full Git commit hash injected at build time, or `unknown`
- `Dirty` — whether the build tree was dirty, as injected at build time, or `unknown`
- `Profile` — Cargo build profile such as `debug` or `release`
- `Built` — build timestamp as Unix epoch seconds injected at build time, or `unknown`
- `Target` — Cargo target triple

The commit, dirty flag, and timestamp are populated only from the provenance
the build injects (`JIT_BUILD_GIT_HASH`, `JIT_BUILD_GIT_SHORT_HASH`,
`JIT_BUILD_GIT_DIRTY`, `SOURCE_DATE_EPOCH`); `scripts/install-jit.sh` supplies
them from the current commit. An ordinary `cargo build`/`cargo test` reads no
ambient Git state or wall clock, so it reports `unknown` for these fields. This
keeps unchanged rebuilds reproducible and insensitive to Git-metadata-only
changes.

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

`git_dirty` is `true` or `false` when a dirty flag was injected at build time,
and `null` when none was (an ordinary build injecting no provenance).

## Issue Commands

### Creating Issues (`jit issue create`)

Create one issue. The title is the subject of the verb: give it positionally or
through `-t`/`--title`. Exactly one of the two forms is required, and supplying
both is a usage error (exit code `2`).

```bash
jit issue create <TITLE> [OPTIONS]
jit issue create --title <TITLE> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-t`, `--title <TITLE>` | Title, as a flag instead of the positional argument. |
| `-d`, `--description <DESCRIPTION>` | Issue body. Defaults to the empty string. |
| `-p`, `--priority <PRIORITY>` | `low`, `normal` (default), `high`, or `critical`. |
| `--type <KIND>` | Issue type, written as a `type:<kind>` label. Must be declared in `[type_hierarchy]` in `config.toml`. Long-only, because `-t` is `--title`. |
| `-g`, `--gate <GATE>` | Gate keys the issue requires. Repeatable and comma-separated. Keys are persisted as given without a registry lookup; `jit validate` later flags any that no gate defines. |
| `-l`, `--label <LABEL>` | Labels in `namespace:value` form. Repeatable and comma-separated. |
| `--content-format <FORMAT>` | Parser for the description body during validation: `markdown`, `html`, or `xml`. Omitted, the repository default (`[validation].content_format`) applies, falling back to Markdown. `html`/`xml` require the matching cargo feature. |
| `--force` | Bypass blocking (`enforce = true`) rule failures and record each bypass as an event. Warnings never block, so they are unaffected. |
| `--orphan` | Suppress the `orphan-leaf` hint for an issue deliberately created without a container. |
| `--json` | Emit the created issue as the `issue show` object, plus a `message` field. |

```bash
jit issue create "Fix login bug"
jit issue create "Fix login bug" --type task --priority high
jit issue create --title "Fix login bug" --gate tests --label epic:auth
jit issue create "Wire up parser" --json
```

**Initial state.** A new issue has no dependencies, so it is born `Ready` and its
`first_ready_at` timestamp is stamped at creation. Adding a dependency
afterwards (`jit dep add`) is what moves it back to `Backlog` until the
dependency becomes effectively terminal (done, rejected, or archived from one
of those).

Under `--quiet` the command prints only the new issue's id, which is what
scripts capture. Validation warnings go to stderr, so they never pollute that
capture.

### Updating Issues (`jit issue update`)

Update one issue by id, or every issue matching a `--filter` expression. The two
modes are mutually exclusive; giving neither is a usage error (exit code `2`).

```bash
jit issue update <ID> [OPTIONS]
jit issue update --filter <QUERY> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--filter <FILTER>` | Boolean query selecting the issues to update (batch mode). Mutually exclusive with `<ID>`. |
| `-t`, `--title <TITLE>` | Replace the title. |
| `-d`, `--description <TEXT>` | Replace the whole description with `TEXT`. |
| `--description-file <PATH>` | Replace the whole description with the contents of `PATH` (`-` reads stdin), used verbatim. |
| `--append-description <TEXT>` | Append `TEXT` to the description. |
| `--append-description-file <PATH>` | Append the contents of `PATH` (`-` reads stdin) to the description. |
| `-p`, `--priority <PRIORITY>` | `low`, `normal`, `high`, or `critical`. |
| `-s`, `--state <STATE>` | Requested lifecycle state. |
| `--type <KIND>` | Replace the issue's `type:*` label with `type:<kind>`. Long-only. |
| `-l`, `--label <LABEL>` | Add labels (alias `--add-label`). Repeatable and comma-separated; adds to the existing set. |
| `--remove-label <LABEL>` | Remove labels. Repeatable and comma-separated. |
| `--add-gate <GATE>` | Require additional registered gates. Repeatable and comma-separated. |
| `--remove-gate <GATE>` | Drop gates from the issue's requirements. |
| `--assignee <ASSIGNEE>` | Set the assignee, in `type:identifier` form. |
| `--unassign` | Clear the assignee. |
| `--content-format <FORMAT>` | Set the description parser: `markdown`, `html`, `xml`, or `inherit`/`default` to fall back to the repository setting. |
| `--force` | Bypass blocking (`enforce`) validation rules. The bypass is recorded as an event. |
| `--json` | Emit the update confirmation object. |

**Description flags are mutually exclusive**: exactly one of `--description`,
`--description-file`, `--append-description`, `--append-description-file` may
appear in a call. The `-file` forms read the file (or stdin) verbatim, including
a trailing newline, which keeps large bodies out of `argv` and away from shell
quoting. An append separates the old and new text with exactly one blank line;
appending to an empty description yields just the new text.

```bash
jit issue update abc123 --state in_progress --assignee agent:worker-1
jit issue update abc123 --description "New text"
jit issue update abc123 --append-description "Follow-up note"
cat notes.txt | jit issue update abc123 --append-description-file -
jit issue update abc123 --label area:auth --remove-label area:core
```

**Single-issue JSON** is a lightweight confirmation rather than the full issue;
fetch the body with `jit issue show`:

```json
{
  "id": "5c581575-bef8-4ee6-be83-7598fd22b557",
  "short_id": "5c581575",
  "state": "in_progress",
  "updated_at": "2026-04-28T18:25:16.699033997Z",
  "message": "Updated issue 5c581575 to InProgress"
}
```

**State transitions are guarded.** `--state ready` runs the dependency check and
fails with the `BLOCKED` envelope when any dependency is unmet. `--state done`
runs both checks: unmet dependencies fail the same way, and unpassed gates divert
the issue to `Gated` (the diverted state is persisted) with the
`VALIDATION_FAILED` envelope. Both envelopes carry `error.details.blockers` and
`error.details.remediation`, as shown under **CLI JSON contracts** above.

**Batch mode is literal.** With `--filter`, the flags below are rejected as usage
errors rather than silently ignored, because each is a per-issue edit:
`--type`, `--content-format`, and every description flag. Batch results report
`matched`, `modified`, `skipped`, and `errors` id lists.

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
- **Single-issue:** Enforces prechecks on `ready → in_progress` and checks gate statuses on an explicit `done` request
- **Bulk:** Explicit, predictable batch changes across many issues

### Batch-Create with Dependency Wiring (`jit issue batch-create`)

`jit issue batch-create --from-json <file>` creates a whole set of issues and
their dependency edges from one declarative JSON file, replacing hand-written
`create` + `dependency add` loops. Entries reference each other by a symbolic
`key`, so you describe the dependency graph directly instead of threading
generated IDs through follow-up commands.

```bash
jit issue batch-create --from-json plan.json
jit issue batch-create --from-json plan.json --json
jit issue batch-create --from-json plan.json --dry-run --json
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
| `planning`    | no       | —                    | Opaque authoring metadata; accepted but not stored or exported |

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

On success, issues, index membership, events, and dependency edges publish as one
recoverable repository mutation.

**Dry-run.** `--dry-run` performs the same native validation and preserves exit
code `2` for validation errors, but allocates no ids and writes no issues, index,
events, or edges. Human output reports counts. JSON returns:

```json
{"valid":true,"dry_run":true,"issue_count":3,"dependency_count":2,"keys":["spec","impl","test"]}
```

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

**Summary shape (`--summary`):** drops the description and the enriched
dependency list, keeping the `MinimalIssue` fields plus the gate list. It
exposes that list under the same `gates` field name the full view uses,
projected to `{key, status}` per required gate (the full view additionally
carries each gate's `last_run_at` / `exit_code`). It never emits the on-disk
record's `gates_required` / `gates_status`; those appear only where a command
hands back the raw stored record (any `--full` record dump, the single-issue
lifecycle mutation confirmations, and `jit apply`'s `created_issues`). See the
projection rules and their current members in
[Storage Format](storage-format.md#gate-fields-in-command-output).

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
array: the ids in the issue's stored `dependencies` whose target issue is
absent from the repository. It is omitted when empty. Deleting an issue strips
its id from every dependent, so a populated array marks stored dependencies
that point at a missing target, keeping those ids visible alongside the
`dependencies` view.

**Unmet dependencies:** the `issue show --json` object also carries an
`unmet_dependencies` array: the subset of `dependencies` that are not yet **met**.
A dependency is met exactly when it is in an effective terminal state (`done`,
`rejected`, or `archived` from one of those) — the same readiness test `jit query
ready` uses to decide whether an issue is blocked — so a `rejected` dependency
counts as met and is **not** listed.
Each entry is a subset of the matching `dependencies` entry: `{id, short_id,
title, state}`. The array is always present (empty `[]` when every dependency is
met or there are none), so a caller reads the filter straight from the response.

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
- **Effective-terminal semantics** (`done`/`rejected`): `done` and `rejected`
  are reported separately because a rejected child is terminal but **not
  delivered**. `archived` is terminality-preserving, so a child archived from
  `done` folds into `done` and one archived from `rejected` folds into
  `rejected`; a child archived from a non-terminal state (or a legacy archived
  record) counts as `open`. `open` is every child that is not effectively
  terminal (`total − done − rejected`). The exact `by state` counts still show
  `archived` as its own bucket. The `done/total` ratio and `percent` (rounded;
  `0` when `total` is `0`) measure delivery — `done` against `total`.
- A bad id under `--json` returns the refined error envelope and matching exit
  code, like `issue show`.

### Assigning and Claiming Issues

There are two ways to put an assignee on an issue:

- **`jit issue assign <id> <assignee>`** sets the assignee and makes no state
  change. The issue stays in whatever state it is in (`backlog`, `ready`, ...).
- **`jit issue claim <id> <assignee>`** assigns the issue *and* transitions a
  `ready` issue to `in_progress` (the "start work" path). Claiming an
  unassigned issue assigns and promotes it; re-claiming as the current
  assignee is idempotent and succeeds, promoting it the same way. Claiming an
  issue assigned to a *different* assignee fails, naming the current holder.

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
still `backlog` behind unmet dependencies, because it cannot transition to
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
- **Closure outcome:** Records rejection without dependency or gate checks; do not
  treat it as an enforced no-return source state for later `issue update` calls
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

# Rejection bypasses dependency and gate checks; choose the closure action that
# matches the project's process rather than relying on a source-state restriction.
jit issue reject $ISSUE --reason "out-of-scope"
```

### Deleting Issues (`jit issue delete`)

Deletion permanently removes an issue record. It is a destructive, discouraged
operation — prefer `jit issue reject` — and requires explicit operator
confirmation via the environment:

```bash
JIT_ALLOW_DELETION=1 jit issue delete <ID>
```

**Key behaviors:**

- **Refusal exits nonzero.** Without `JIT_ALLOW_DELETION=1` the command refuses
  before writing anything and exits `2` (invalid-argument family) in both text
  and JSON modes, so a script can distinguish "refused" from "deleted".
- **JSON refusal envelope.** Under `--json` the refusal uses the standard
  top-level `error` object with code `DELETION_NOT_CONFIRMED`, `details.id`
  naming the issue, and a `suggestions` entry carrying the exact
  `JIT_ALLOW_DELETION=1 jit issue delete <ID>` remediation command.
- **Confirmed deletion is unchanged**: it removes the issue, logs the deletion
  event, and reports the removal (exit `0`).
- **Main worktree only.** Deletion is refused from secondary git worktrees to
  keep worktree state consistent.

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
- `--mode <MODE>` - How gate is checked: `manual` or `auto`. When omitted, the
  mode is inferred: `auto` if `--checker-command` is given, `manual`
  otherwise. An explicit `--mode manual` combined with `--checker-command` is
  a usage error (exit 2) — a manual gate cannot carry a checker, so the
  conflict is rejected rather than silently dropping the checker.
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

# Automated test gate — --checker-command with no --mode infers auto
jit gate define tests \
  --title "All Tests Pass" \
  --description "Full test suite must succeed" \
  --stage postcheck \
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
  --env REVIEWER_AGENT="your-reviewer-command"

# Usage error: explicit manual mode conflicts with a checker command
jit gate define bad --title "Bad" --description "Bad" \
  --mode manual --checker-command "cargo test"
# error: --mode manual conflicts with --checker-command for gate 'bad': ...
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

Every mutable `exec`-checker field is reachable from the CLI: each clearable
field has both a set flag and a `--clear-*` flag. Native checker types are
selected in `.jit/gates.toml`; the CLI has no checker-type option. Passing a set
flag together with its `--clear-*` twin is an `INVALID_ARGUMENT` error.

Switching a gate to `auto` (via `--mode auto` or `--auto`) requires a configured
checker. A gate that already has a native checker keeps it; otherwise supply
`--checker-command` in the same call to configure an `exec` checker. Supplying
`--checker-command` for a native checker replaces it with an `exec` checker.
Switching to `manual` drops the checker. At least one field must be provided; an
update with no fields is an `INVALID_ARGUMENT` error. Updating a key that is not
in the registry is a `GATE_NOT_FOUND` error.

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
      "example_integration": null,
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

The unified gate-run inspection surface (inspection only, non-mutating). It
offers four views over the stored run records: the latest
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
non-mutating).

**Usage:**
```bash
jit gate status-all <ISSUE_ID> [--json] [--full]
```

**Behavior:**
- Considers EVERY required gate on the issue — automated AND manual. A required
  manual gate that has not been attested counts as pending.
- Does not execute any checker commands; it only reports recorded state.
- Exits `0` only when every required gate has passed; otherwise exits `4` (see
  the [exit-code reference](exit-codes.md#command-specific-mappings)). A pending
  (auto never run, manual never attested) or failed gate is not green, and both
  map to the single nonzero code. This readiness contract is a single behaviour
  with no flag.
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
override. Short alias: `eval`.

**Usage:**
```bash
jit gate evaluate <ISSUE_ID> <GATE_KEY> [--by <WHO>] [--force]
jit gate evaluate <ISSUE_ID> --gate <GATE_KEY> [--by <WHO>] [--force]
```

The gate key may be supplied as a positional argument or via `--gate <key>`. Exactly one form must be used; supplying both or neither is an error.

**Options:**
- `--gate <KEY>` - Gate key (flag form, alternative to the positional argument)
- `--by <WHO>` - Who is passing the gate (e.g., `human:alice`, `ci:github-actions`). Required for a manual gate; ignored for an automated gate, whose verdict comes from the checker.
- `--force` - Re-run an automated gate's checker even if it already passed at the current HEAD commit

**Examples:**
```bash
# Evaluate a manual gate — record attestation (positional form)
jit gate evaluate abc123 code-review --by "human:alice"

# Same command using the flag form
jit gate evaluate abc123 --gate code-review --by "human:alice"

# Evaluate an automated gate — runs its checker (no --by needed)
jit gate evaluate abc123 tests

# Force a re-run even if it already passed at HEAD
jit gate evaluate abc123 tests --force
jit gate evaluate abc123 --gate tests --force
```

**Behavior:**
- For a manual gate: `--by` is required. Bare `jit gate evaluate <id> <gate>` on a manual gate is a usage error (exit 2) — a manual gate has no checker to run, so evaluating it without an attestor would silently record an unattributed pass. With `--by`, every invocation records fresh evidence with a new event and timestamp, including when the same attestor already passed the gate at the current `HEAD`. If that clears the final blocker on a `gated` issue, the manual-pass path may transition it to `done`.
- For an automated (auto) gate: runs the checker and records `passed` only when the checker passes; `--by` is not required. This evaluation records a run; it does not itself complete the issue.
- After required statuses are passed, use `jit issue update <id> --state done` to complete a gated issue through the explicit completion path.

**Automated skip when already passed at HEAD:**
- If an automated gate's latest run already passed at the current `HEAD` commit,
  `jit gate evaluate` skips the (often expensive) checker, exits `0`, and reports
  `already_passed: true` in `--json`. The non-`--json` path prints a concise
  "already passed at HEAD, skipping (use --force to re-run)" line.
- The skip compares the current `HEAD` against the commit stamped on the latest
  recorded run; both must be present and equal. When there is no git repository
  or no commit (`HEAD` unresolvable), the run is never skipped — the prior pass
  cannot be proven current.
- Manual attestations are never skipped; each invocation with `--by` records
  fresh evidence even at the same `HEAD`.
- `--force` bypasses the automated check and re-runs the checker unconditionally.
- On a normal run (manual attestation, or a freshly executed checker), `--json`
  reports `already_passed: false`.

**Exit-code taxonomy** (auto and manual gates): the
[exit-code reference](exit-codes.md#command-specific-mappings) is the authority
for every code `jit gate evaluate` returns. The command-specific split it
records: a checker that ran and returned verdict `fail` exits `4`; a checker that
ran but could not produce a verdict (timeout, command-not-found, or crash) exits
`10`. Pre-verdict argument errors (e.g. the gate is not required for the issue,
or a manual gate evaluated without `--by`) and lookup errors (issue not found)
are classified before the run path and are never reported as a runner error.

**Stale-binary refusal for `exec` checkers (jit:7446af34):** a `jit` binary that
predates the repository it is validating must not produce — or let an `exec`
checker's own child process produce — a trusted gate verdict. Native in-process
checkers do not use this subprocess guard. The refusal condition is ALL of:

1. the repository under validation can resolve the running binary's build
   commit in its own history (the repository the binary was built from, or a
   clone or fork that shares that history), AND
2. either the repository's current `HEAD` differs from that build commit, or
   the binary was built from a dirty working tree.

Otherwise — an unrelated repository, no git at all, or a binary built without
a resolvable commit — the check stays silent: it never fires for an ordinary
installed release validating a repository it was not built from.

This is checked in two places, which surface differently on `jit gate
evaluate`/`evaluate-all`:

- **The evaluator itself is stale:** refused BEFORE the checker ever spawns.
  `jit gate evaluate` exits `10` directly, no gate run is recorded at all, and
  under `--json` the error `code` is `STALE_BINARY`. This is PRE-verdict, so
  it carries no `verdict` field — unlike the post-verdict runner-crash case
  above (which also exits `10` but DOES carry `verdict: "error"`). Text and
  `--json` modes agree on exit `10`.
- **A checker's own child `jit` is stale** — e.g. a checker script that itself
  shells out to `jit` (like `scripts/jit-validate.sh`'s `exec jit validate
  "$@"`), which resolves `jit` from `PATH` independently of the evaluator: the
  child refuses and exits `10`, but the EVALUATOR sees an ordinary checker
  failure — `jit gate evaluate` exits `4` (`GATE_FAILED`, verdict `fail`), a
  gate run IS recorded, and the refusal is visible in that run's
  `stdout`/`stderr` (`jit gate status <id> <gate> --stderr`, or
  `error.details.checker_result.stderr` under `--json`) rather than as a
  distinct top-level error code — see the verdict-field section below.

Rebuild and reinstall with `scripts/install-jit.sh` (it injects build
provenance around `cargo install --path crates/jit`, so the reinstalled binary
reports its commit and the guard can judge it) to clear either case.

**`--json` verdict field:**

`jit gate evaluate --json` carries a `verdict` field describing the run-path outcome:

- `pass` — top-level field on the success response.
- `fail` — under `error.details` when the checker evaluated to failure (code `4`).
  This is also what a stale checker-child refusal looks like from the
  evaluator's side for an `exec` checker (see above): its subprocess exited
  nonzero, so the outer command still gets a normal `fail` verdict — the
  checker's own `checker_result.stderr` is what shows it was a
  stale-binary refusal.
- `error` — under `error.details` when checker evaluation failed unexpectedly
  (for example, an `exec` runner failed; code `10`).

Pre-verdict conditions carry no `verdict` field at all: argument/lookup errors
(codes `2` and `3`), and an `exec` evaluator's own stale-binary refusal above —
the one case where exit `10` does not carry a checker verdict.

```bash
# Success response
jit gate evaluate abc123 tests --json
# {
#   "issue_id": "abc123",
#   "key": "tests",
#   "status": "passed",
#   "verdict": "pass",
#   "already_passed": false,
#   "message": "Passed gate 'tests' for issue abc123"
# }

# Checker ran, failed          (exit 4):  error.details.verdict == "fail"
# Checker ran, runner crashed  (exit 10): error.details.verdict == "error"
# Evaluator itself stale, checker never spawned (exit 10):
#   error.code == "STALE_BINARY", no verdict field
# Checker's own child jit stale (exit 4, same as "Checker ran, failed"):
#   error.details.verdict == "fail"; refusal visible in
#   error.details.checker_result.stderr, not in error.code
```

### `jit gate evaluate-all`

Evaluate all of an issue's required gates in one command, **fail-fast**.

**Usage:**
```bash
jit gate evaluate-all <ISSUE_ID> [--by <WHO>] [--force]
```

**Options:**
- `--by <WHO>` - Record who passed the gates (e.g., `human:alice`, `ci:github-actions`)
- `--force` - Re-run every gate's checker even if it already passed at the current HEAD commit

**Behavior:**
- Runs each required gate in declaration order, delegating to `jit gate evaluate`, so
  every gate inherits the same exit-code taxonomy and `verdict` semantics.
  Automated gates also inherit **skip-if-passed-at-HEAD** behaviour (an
  already-passed gate is not re-run; its entry reports `already_passed: true`),
  while manual gates record fresh evidence on every invocation.
- **Manual gates require attestation:** every manual gate in the required set
  needs `--by <attestor>` (applied uniformly; ignored by automated gates).
  Without it, evaluation fails fast at the first manual gate reached in
  declaration order with exit `2` and **no verdict recorded** for that gate —
  automated gates evaluated before it keep their recorded verdicts.
- **Fail-fast:** on the FIRST gate that does not pass, the command stops
  immediately and exits with that gate's code from the
  [`jit gate evaluate`](#jit-gate-evaluate) taxonomy (see the
  [exit-code reference](exit-codes.md#command-specific-mappings)). Later gates are
  never attempted.
- An issue with **no required gates** succeeds with exit `0` and an empty
  `gates` array.
- `--json` emits a top-level `verdict: "pass"` plus a `gates` array, one entry
  per gate (`key`, `status`, `verdict`, `already_passed`, `warnings`). On the first
  failure it emits the same JSON-error shape as `jit gate evaluate` (with
  `error.details.key` naming the offending gate, and `error.details.verdict`
  `fail` or `error` — or no `verdict` field at all when the EVALUATOR itself
  is refused as stale, exit `10`; see the [stale-binary
  refusal](#jit-gate-evaluate) section for the two ways this can surface).

```bash
# All gates pass (one already passed at HEAD, one freshly run)
jit gate evaluate-all abc123 --json
# {
#   "issue_id": "abc123",
#   "status": "passed",
#   "verdict": "pass",
#   "gates": [
#     { "key": "tests",  "status": "passed", "verdict": "pass", "already_passed": true,  "warnings": [] },
#     { "key": "clippy", "status": "passed", "verdict": "pass", "already_passed": false, "warnings": [] }
#   ],
#   "message": "Passed 2 required gate(s) for issue abc123"
# }

# Fail-fast: first failing gate sets the exit code; later gates do not run.
jit gate evaluate-all abc123
# exit 4:  a checker's verdict was fail, OR a checker's own child jit
#          refused as stale (the refusal is in that gate's checker_result)
# exit 10: a checker's runner crashed, OR the evaluator itself refused as
#          stale (no gate ran at all for that entry)
```

### `jit gate fail`

Record a failed verdict for a **manual** gate. An automated gate is rejected
(exit `2`): its verdict comes only from running its checker via
[`jit gate evaluate`](#jit-gate-evaluate), never from a hand-recorded fail.
Every manual failure records fresh evidence with a new event and timestamp,
including repeated failures at the same `HEAD`.

**Usage:**
```bash
jit gate fail <ISSUE_ID> <GATE_KEY> [--by <WHO>]
```

**Example:**
```bash
# Record a manual gate's failing verdict (e.g. a reviewer rejected the change)
jit gate fail abc123 code-review --by "human:alice"
```

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

# Filter by a specific gate's status. query all returns a minimal per-issue
# shape (no gate detail); the full issue record with the gates_status map comes
# from graph export --full, whose nodes are the on-disk issue fields.
jit graph export --format json --full | jq '.nodes[] | select(.gates_status.tests.status == "failed") | .id'
```

### Exit Codes

Gate commands follow the standard [exit-code reference](exit-codes.md); its
[command-specific mappings](exit-codes.md#command-specific-mappings) list the
`jit gate evaluate` and `jit gate status-all` rows, including the checker-verdict
exceptions. See [`jit gate evaluate`](#jit-gate-evaluate) above for the
pass-specific `verdict` field.

## Gate Preset Commands

Gate presets are pre-configured bundles of quality gates that can be quickly applied to issues. Presets encode best practices and reduce setup time from minutes to seconds.

### `jit gate preset list`

List all available gate presets (builtin and custom).

**Usage:**
```bash
jit gate preset list [--json]
```

**Output** (line format only — run `jit gate preset list` for the live set, and
`jit gate preset show <name>` for a preset's actual gate list and count; the
builtin registry is the source of truth, so the totals below are placeholders):
```
[builtin] plan-review - External-review placeholder for the linked plan before implementation work fans out. (<N> gates)
[builtin] breakdown-review - External-review placeholder for decomposition quality, issue content, and dependency ordering before implementation. (<N> gates)
[builtin] coverage-preview - Validate the container named by the breakdown issue's brackets label. (<N> gates)
[custom] my-workflow - Custom preset created from issue abc123 (<N> gates)
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
- `NAME` - Preset name (e.g., the builtin `plan-review`, or a project preset like `rust-ci`)

**Output** (illustrative layout — run the command for a preset's actual gates,
commands, and timeouts):
```
Preset: rust-ci
Description: Custom preset created from issue abc123

Gates:
  tests - All tests pass (postcheck:auto)
    Command: <command>
    Timeout: <N>s
  ...
  code-review - Code review completed (postcheck:manual)
```

**Examples:**
```bash
# Show a project preset's details
jit gate preset show rust-ci

# Show a builtin preset
jit gate preset show plan-review

# JSON output
jit gate preset show rust-ci --json
```

### `jit gate preset apply`

Apply preset gates to one or more issues. Gates from the preset are added to the issue's required gates list. If a gate doesn't exist in the registry, it is automatically defined.

**Usage:**
```bash
jit gate preset apply <NAME> [ISSUE_ID]... [OPTIONS]
```

**Arguments:**
- `NAME` - Preset name to apply
- `ISSUE_ID...` - Issue IDs (repeatable for batch operations). Optional: passing
  none succeeds and applies the preset to nothing.

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
jit gate preset apply rust-ci abc123

# Apply to multiple issues (batch mode)
jit gate preset apply rust-ci abc123 def456 ghi789

# Apply from query results (JSON is the xargs-safe source of ids)
jit query all --json | jq -r '.issues[].id' | xargs jit gate preset apply rust-ci

# Apply with filtering - skip precheck gates
jit gate preset apply rust-ci abc123 --no-precheck

# Skip specific gates
jit gate preset apply rust-ci abc123 --except clippy --except fmt

# Override timeout for all automated gates
jit gate preset apply rust-ci abc123 --timeout 600

# Combine filters
jit gate preset apply rust-ci abc123 --no-precheck --except clippy --timeout 120
```

**Batch Output:**
```
Applied preset 'rust-ci' to 2 issue(s):
  abc123 - gates added: tests, clippy, fmt, code-review
  def456 - gates added: tests, clippy, fmt, code-review

Errors (1):
  xyz999 - Issue not found: xyz999
```

**Notes:**
- Gates are automatically added to registry if they don't exist
- Timeout override applies to all automated gates in the preset
- A missing preset target is a not-found failure (exit `3`); see the
  [exit-code reference](exit-codes.md#command-specific-mappings)
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
- Preset name must be non-empty and must not collide with a builtin preset's name

**Storage:**
Custom presets are stored in `.jit/config/gate-presets/<name>.json` and are automatically loaded alongside builtin presets. Custom presets with the same name as a builtin preset override the builtin.

### Builtin Presets

The binary embeds exactly the three planning-bracket presets — `plan-review`,
`coverage-preview`, and `breakdown-review` — which attach to the planning (`P`)
and breakdown (`B`) nodes when a breakable container is
[bracketed](../concepts/planning-bracket.md), reviewing the plan and the
decomposition before fan-out. Their definitions are the source of truth for what
each one bundles; [Built-in Gate Presets](gate-presets.md) is generated from those
definitions and lists each gate (key, title, stage, mode, description, checker).
The live commands introspect the same set: `jit gate preset list` prints every
preset with a one-line summary, and `jit gate preset show <name>` prints one
preset's gate list.

Language- and workflow-specific bundles are declared per project, not built in;
see [Declaring a project preset](../how-to/custom-gates.md#declaring-a-project-preset).

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

**Quick Start with a Project Preset:**
```bash
# Apply your project's CI workflow to a new issue
jit issue create --title "Add user login"
jit gate preset apply rust-ci abc123
# Issue now carries the rust-ci preset's gates (run `jit gate preset show rust-ci` for the current set)
```

**Create Team Standard:**
```bash
# Set up one issue with desired gates
jit gate add abc123 tests clippy code-review docs

# Save as team standard
jit gate preset create abc123 team-standard

# Apply to all issues in epic
jit query all --label epic:v2.0 --json | jq -r '.issues[].id' | xargs jit gate preset apply team-standard
```

**Customize for Special Cases:**
```bash
# Apply without precheck for hotfix
jit gate preset apply rust-ci hotfix-123 --no-precheck

# Apply with faster timeout for CI
jit gate preset apply rust-ci abc123 --timeout 60

# Apply subset of gates
jit gate preset apply rust-ci abc123 --except fmt --except clippy
```

### Exit Codes

`jit gate preset apply` follows the [exit-code reference](exit-codes.md); its
[command-specific mappings](exit-codes.md#command-specific-mappings) record the
missing-target case as the ordinary not-found status (exit `3`). A missing preset
or issue is likewise a not-found error per the global taxonomy.

## Dependency Commands

### `jit dep add`

Add one or more dependencies to an issue. `FROM` is blocked until every listed
`TO` is effectively terminal (done, rejected, or archived from one of those).
Dependencies are orthogonal to labels: issues don't need matching labels to
depend on each other.

**Usage:**
```bash
jit dep add <FROM_ID> <TO_ID>... [--reduce] [--json]
```

**Arguments:**
- `FROM_ID` — the issue that becomes blocked
- `TO_ID...` — one or more issues that must become effectively terminal (done,
  rejected, or archived from one of those) first

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
unmet dependency auto-transitions a `backlog` issue to `ready`.

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
| `jit query closed` | — | Effectively terminal issues (Done, Rejected, or Archived from one of those) |
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
  the `done`/`rejected`/`open`/`percent` effective-terminal semantics are
  identical to `issue progress` (done and rejected distinct, folding `archived`
  by its recorded origin; open = not effectively terminal; `done/total` measures
  delivery).

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
[membership-reference validation](../how-to/validation-rules.md).

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

## Event Log Commands

`events.jsonl` is the repository's append-only history: every issue creation,
state transition, claim, gate result, and registry edit appends one line.
`jit events` reads that log — it never writes to it. Both subcommands emit events
in the log's stored (append) order. The event object shape is documented under
[Event Log Records](storage-records.md#event-log-records); the full set of event
`type` tags, with each tag's scope and `issue_id` presence, is the generated
[Event Log Tags reference](events.md).

Human output prints one JSON event object per line (JSONL, the same encoding the
log stores); `--json` wraps the same events in the list envelope
`{"count": N, "events": [...]}` with a `message` field.

### `jit events tail`

Print the most recent events.

```bash
jit events tail [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-n <N>` | Number of most recent events to print (default `10`). |
| `--json` | Emit the list envelope `{"count": N, "events": [...]}`. |

```bash
jit events tail
jit events tail -n 50
jit events tail --json
```

### `jit events query`

Filter the log by event type and/or issue, capped at `--limit` results.

```bash
jit events query [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-e`, `--event-type <TYPE>` | Keep only events whose `type` tag equals `<TYPE>` (e.g. `issue_state_changed`). |
| `-i`, `--issue-id <ID>` | Keep only events carrying this `issue_id`. |
| `-l`, `--limit <N>` | Return at most `N` events (default `50`). |
| `--json` | Emit the list envelope `{"count": N, "events": [...]}`. |

```bash
# Every state change recorded for one issue
jit events query --issue-id abc123 --event-type issue_state_changed

# The 20 most recent gate passes across the repository
jit events query --event-type gate_passed --limit 20 --json
```

Type filters match the snake-case `type` tag exactly; repository- and
registry-scoped events carry no `issue_id`, so an `--issue-id` filter never
returns them.

## Document Commands

A document reference links a repository file to an issue, so an agent reading the
issue can find the design note, spec, or report that belongs with it. The file
itself stays in the repository; jit stores the reference. `jit document` is a
visible alias of `jit doc`.

Reading a document at a commit needs a git repository: `history` and `diff`
always, and `show` whenever `--at` is given or the reference itself is pinned to
a commit. The rest of the family works without git.

### `jit doc add`

Attach a document reference to an issue. Identity is (issue, path): re-running
`doc add` for a path already linked to the issue updates that reference in
place rather than appending a duplicate — `jit doc list` still shows one entry
for the path. The commit pin always reflects the invocation, exactly as on a
fresh add: a supplied `--commit` pins the reference, an omitted one records it
unpinned (the current version), so a re-run re-points a stale pin. An omitted
`--label`/`--doc-type` on the re-add leaves the existing value alone; a
supplied one overwrites it. The scanned
`format`/assets are always the freshly computed result unless `--skip-scan` is
given. The JSON result's `updated` field is `true` for a refresh and `false`
for a genuinely new reference; the appended `issue_updated` event records the
refresh under the `doc-update` tag rather than `doc-add`.

```bash
jit doc add <ID> <PATH> [--commit <COMMIT>] [--label <LABEL>] [--doc-type <DOC_TYPE>] [--skip-scan] [--json]
```

| Argument / flag | Description |
|-----------------|-------------|
| `<ID>` | Issue id (full, short, or a unique prefix). |
| `<PATH>` | Document path relative to the repository root. |
| `-c`, `--commit <COMMIT>` | Git commit to pin the reference to. Omitted, the reference is stored unpinned and reads as the current version. |
| `-l`, `--label <LABEL>` | Human-readable label for the reference (alias `--title`). |
| `--doc-type <DOC_TYPE>` | Free-form document type, e.g. `design`, `implementation`, `notes`. |
| `--skip-scan` | Skip scanning the document for asset references. |

```bash
jit doc add abc123 docs/design/auth.md --label "Auth design" --doc-type design
jit doc add abc123 docs/design/auth.md --commit 44ee4610 --json
# Re-running with the same path updates that reference instead of duplicating it
jit doc add abc123 docs/design/auth.md --commit 9c2d1a7 --json
```

### `jit doc list`

List an issue's document references.

```bash
jit doc list <ID> [--json]
```

JSON returns the list envelope with the issue id alongside it:
`{"issue_id": <id>, "count": N, "documents": [...]}`.

### `jit doc show`

Print a document's content as recorded for an issue.

```bash
jit doc show <ID> <PATH> [--at <COMMIT>] [--json]
```

`--at <COMMIT>` reads the document as it stood at that commit instead of at
`HEAD`.

### `jit doc history`

List the commits that touched a document.

```bash
jit doc history <ID> <PATH> [--json]
```

### `jit doc diff`

Diff a document between two commits.

```bash
jit doc diff <ID> <PATH> --from <COMMIT> [--to <COMMIT>] [--json]
```

`--from` is required; `--to` defaults to `HEAD`.

### `jit doc remove`

Detach a document reference from an issue. The file on disk is untouched.

```bash
jit doc remove <ID> <PATH> [--json]
```

`remove` is this group's canonical spelling: `doc rm` and `doc delete` exit `2`
with a hint (see **Wrong-verb hints** above).

### `jit doc assets list`

List the assets (images, diagrams) a document references.

```bash
jit doc assets list <ID> <PATH> [--rescan] [--json]
```

`--rescan` re-reads the document to refresh the stored asset metadata instead of
reporting what was recorded when the reference was added.

### `jit doc check-links`

Validate the links and asset references of the documents in scope.

```bash
jit doc check-links [--scope all|issue:<ID>] [--json]
```

`--scope` defaults to `all`. The exit code carries the verdict per the
[exit-code reference](exit-codes.md#command-specific-mappings): `0` when every
document is valid, otherwise `1` (broken links) or `2` (only warnings). JSON
reports `valid`, `errors`, `warnings`, and a `summary`.

### `jit doc dir`

Print the repository-relative directory an issue owns in a declared
issue-scoped area.

```bash
jit doc dir <ID> <AREA> [--json]
```

`<AREA>` is one of the issue-scoped areas the repository declares under
`[documentation]`; naming it is the caller's whole contribution, because the
directory name inside it is derived from the issue ([Issue artifact
directories](configuration.md#issue-artifact-directories)). The answer is a
name rather than a reading of the tree: it resolves the same whether or not
anything has been written there, and the command creates nothing.

Human output is the bare directory, so it composes straight into a shell
substitution:

```bash
mkdir -p "$(jit doc dir abc12345 <AREA>)"
```

JSON is a flat object naming the issue, the area, and the directory —
`{"issue_id": <id>, "short_id": <short id>, "area": <area>, "directory": <path>}`.

An area the repository does not declare is rejected with code
`INVALID_ARGUMENT` and exit code `2`, and the message lists the declared areas.
An id naming no issue is `ISSUE_NOT_FOUND`, exit code `3`.

### `jit doc conformance`

Report the artifacts sitting outside the directory their owning issue owns.

```bash
jit doc conformance [--json]
```

The report walks every declared issue-scoped area and resolves each artifact's
owner in one of two ways. A name opening with a short id is owned by the issue
that short id answers to. A name carrying none is owned by the issue whose
document reference (`jit doc add`) names that path, which is how an artifact
filed inside an issue directory states its owner — the directory already names
the issue, so the file need not repeat it.

An artifact outside the directory its owner owns (`jit doc dir`) is
`nonconforming` and carries both the owner and that directory. An ownership
neither rule settles is `unattributed` and carries neither: a short-id prefix
no single issue answers to, and a prefix-less name several issues reference. A
prefix-less name no issue references is passed over, since nothing states who
owns it. The topmost offending path component is the one named, so a misplaced
directory is a single entry rather than one per file inside it, and an artifact
anywhere beneath its owner's directory conforms.

Advice rather than enforcement: the command writes nothing, blocks no state
transition, and exits `0` whatever it finds. A listed artifact is left exactly
where it is, and acting on the report is the adopter's call.

Human output names the areas walked, then each reported artifact with its
verdict:

```
Scanned areas: <area>, <area>
Artifacts (3):
  <area>/abc12345-plan.md
    nonconforming -> <area>/abc12345-auth (issue abc12345-49b1-4b0f-9a1e-6c2f0d3a7e55)
  <area>/abc12345-auth/review.md
    nonconforming -> <area>/def67890-search (issue def67890-7c3d-4a11-b58e-2f9a1c40d6b3)
  <area>/deadbeef-notes.md
    unattributed (no issue answers to deadbeef)
```

The second entry is a name carrying no short id: `review.md` sits in one
issue's directory while another issue's document reference names it.

JSON is the list envelope over `artifacts` alongside the `areas` walked:
`{"areas": [...], "count": N, "artifacts": [...]}`. Each entry carries `path`,
`area`, `short_id`, and `status`; a `nonconforming` entry adds `issue_id` and
`canonical_directory`. `short_id` is the short id the artifact's own name opens
with, and is empty for an artifact a document reference attributed.

## Graph Commands

The graph family reads the dependency DAG. `deps` walks upstream (what an issue
needs); `rdeps` walks downstream (what needs it). `jit rdeps <id>` is a top-level
spelling of `jit graph rdeps <id>`.

### `jit graph deps`

Show what an issue depends on: the issues that must become effectively terminal
(done, rejected, or archived from one of those) before it can proceed. In
`--json` output each dependency node carries its `state`; an `archived` node
whose pre-archive state was recorded also carries it as `archived_from`. The
origin determines whether the archived dependency still satisfies: a terminal
origin satisfies, a non-terminal origin blocks, and a legacy archived node that
omits the field blocks.

```bash
jit graph deps <ID> [--depth <N>] [--json]
```

`--depth` defaults to `1` (immediate dependencies). `--depth 0` is the opt-in
unlimited transitive walk; any other `N` bounds the walk to `N` levels.

JSON uses the list envelope over `nodes`:

```json
{
  "issue_id": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17",
  "depth": 1,
  "count": 2,
  "nodes": [
    {
      "id": "<dep-uuid>",
      "short_id": "aa11bb22",
      "title": "Build parser",
      "state": "done",
      "priority": "normal",
      "level": 1,
      "children": []
    }
  ],
  "summary": { "total": 2, "by_state": { "done": 1, "ready": 1 } }
}
```

`count` is the number of top-level `nodes`; `summary.total` counts every unique
dependency across the whole tree, so the two differ whenever the walk goes deeper
than one level. Each node's `level` is its depth (`1` = immediate), `children`
holds its own dependencies, and `shared` marks a node reachable through more than
one path.

```bash
jit graph deps epic-123              # immediate dependencies
jit graph deps epic-123 --depth 2    # two levels deep
jit graph deps epic-123 --depth 0    # all transitive
```

### `jit graph rdeps`

Show the issues that depend on this one. Symmetric to `deps`, with the same
`--depth` semantics.

```bash
jit graph rdeps <ID> [--depth <N>] [--json]
jit rdeps <ID> [--depth <N>] [--json]        # top-level spelling
```

JSON uses the list envelope `{"count": N, "dependents": [...]}` over compact
issue summaries. This reads dependency edges; membership labels play no part in
it.

### `jit graph roots`

List the issues that have no dependencies of their own: the entry points of the
DAG.

```bash
jit graph roots [--json]
```

JSON uses the list envelope `{"count": N, "roots": [...]}`.

### `jit graph export`

Export the dependency graph, optionally scoped to one container's subtree.

```
jit graph export [--format dot|mermaid|json|batch] [--json] [--full] [--scope <container>] [--output <file>]
```

| Flag | Description |
|------|-------------|
| `--format` | Output format: `dot` (default), `mermaid`, `json`, or `batch`. |
| `--json` | Sugar for `--format json`. Combining it with an explicit `--format dot`/`--format mermaid`/`--format batch` is a usage error (exit 2); combining it with `--format json` is redundant but not an error. |
| `--full` | Emit complete issue records per node. **JSON only** (`--format json` or `--json`) — combining it with `dot`/`mermaid`/`batch` is a usage error (exit 2). |
| `--scope` | Restrict the export to the container's DAG-authoritative containment membership (the container and its subtree). Composes with every format. |
| `--output` | Write to a file instead of stdout. |

`dot` and `mermaid` render the graph for Graphviz / Mermaid. `json` (or
`--json`) emits a `{ "nodes": [...], "edges": [...] }` document for
programmatic consumers, in one of two node shapes; the `edges` list
(`{ "from": <id>, "to": <dep-id> }`, one per dependency edge) is identical in
both. `batch` emits the [batch-create seed shape](#batch-format) described below.

```bash
jit graph export --json                # == --format json
jit graph export --json --full         # composes with --full
jit graph export --json --format dot   # usage error (exit 2): conflicting formats
jit graph export --scope <epic> --json # only the epic's subtree
```

`--scope` restricts the **listed** nodes to the container's containment
membership; `dot`/`mermaid`/`json` render only those nodes and the edges among
them. As with [`jit graph tree`](#jit-graph-tree), resolution still runs over the
whole repository, so a scoped node's `parent`/`cluster` may name an issue outside
the subtree.

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
**plus the four additive [resolved-hierarchy](../concepts/hierarchy-resolution.md)
fields** — the same field set, with the same meanings, that
[`jit graph tree`](#jit-graph-tree) emits per node: `parent`, `children`,
`cluster`, and `rank`.

This lets a bulk consumer read every node's full record **and** its canonical
placement in one call instead of globbing the issue files or re-deriving
containment. The `edges` list is the same as the summary shape.

Resolution is always computed over the **whole repository**, never over a subset
of nodes. An unscoped export emits every issue, so its resolution fields always
name emitted nodes. A `--scope <container>` export lists only the container's
members — scoping filters which nodes are listed, not how they resolve — so a
kept node's `parent`, `cluster`, or `children` can reference ids outside the
export, exactly as when you filter unscoped output downstream. Dependency edges
that cross the scope boundary are excluded from the scoped output; the `batch`
format additionally reports each excluded crossing edge on stderr so none is
dropped silently. The same caveat applies to
`jit graph tree <root-id>`, which scopes the listed nodes.

The default (no `--full`) output stays in the lean summary shape; the four
hierarchy fields appear only in the `--full` shape. See
[storage-format § Issue JSON Schema](storage-format.md#issue-json-schema) for the
full field reference.

#### Batch format

`--format batch` emits the exact JSON array
[`jit issue batch-create --from-json`](#batch-create-with-dependency-wiring-jit-issue-batch-create) consumes — the
structural inverse of batch creation. Each in-scope node becomes one definition
keyed by its short id:

```json
[
  {
    "key": "003f9f83",
    "title": "Login",
    "type": "task",
    "priority": "normal",
    "labels": ["component:core"],
    "gates": ["code-review"],
    "depends_on": ["a1b2c3d4"]
  }
]
```

The output is a **structural seed**, not a snapshot: it carries no lifecycle
fields (state, assignee, timestamps), and every in-scope node exports regardless
of its state. Three projections make a captured subtree replayable in a fresh
repository:

- **Identity-bound labels are stripped.** The `type:*` label is lifted into the
  `type` field; membership-label namespaces (`[type_hierarchy.label_associations]`),
  the coverage rule's `satisfies-namespace`, and its `container-from-label`
  namespace (`brackets:`) are dropped. Generic labels survive.
- **Template bracket nodes are excluded** together with every edge touching them
  — planning- and breakdown-role node types — so the importing container
  scaffolds its own bracket via [`jit apply`](#jit-apply).
- **Boundary edges are reported.** A dependency on an issue outside the `--scope`
  membership is excluded from `depends_on` and reported on stderr (count plus
  `from -> to` short-id pairs), never dropped silently. `stdout` therefore stays
  a clean batch-create payload: write it to a file (`--output` or shell
  redirection) and feed that file to `jit issue batch-create --from-json`.

Without `--scope` the whole graph is exported in batch shape (still minus bracket
nodes). Round-tripping the same-scope export through batch creation in a fresh
repository with compatible configuration recreates an isomorphic subgraph (same
titles, types, priorities, gates, and in-scope edges).

```bash
jit graph export --scope <epic> --format batch --output epic-seed.json
jit issue batch-create --from-json epic-seed.json   # replay it elsewhere
```

### `jit graph tree`

Show the DAG-resolved containment hierarchy — the parent, children, cluster, and
rank of each node — as computed by the canonical
[hierarchy resolver](../concepts/hierarchy-resolution.md). The dependency DAG is
authoritative; membership labels are advisory and are not consulted.

```
jit graph tree [<root-id>] [--json]
```

With no id the whole repository is resolved; with a root id the view is the root
plus its transitive dependency closure (the DAG subtree it contains).

Scoping filters which nodes are **listed**, not how they **resolve**: resolution
always runs over the whole repository. A scoped node therefore keeps its
repository-wide `parent`, `cluster`, and `children`, any of which may name an
issue outside the listed subtree — the root's own `parent` is the usual case.
The same caveat applies to any consumer that filters the nodes of
[`jit graph export --full`](#jit-graph-export).

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

### `jit status`

Print the repository-wide state rollup.

```bash
jit status [--json]
```

```text
Status:
  Open: 3
  Ready: 4
  In Progress: 2
  Done: 8
  Rejected: 1
  Blocked: 5
```

JSON carries the same counts plus `gated` and `total`:

```json
{
  "open": 3,
  "ready": 4,
  "in_progress": 2,
  "gated": 0,
  "done": 8,
  "rejected": 1,
  "blocked": 5,
  "total": 18,
  "message": "3 open, 4 ready, 2 in progress, 8 done"
}
```

`open` counts `backlog` issues; `blocked` counts issues held by an unmet
dependency, so an issue can appear in both `open` and `blocked`. For a rollup
scoped to a container or a label bucket, use
[`jit issue progress`](#container-progress-jit-issue-progress) or
[`jit query count`](#state-aggregation-jit-query-count).

### `jit validate`

Check repository integrity and run the declarative rule set from `.jit/rules.toml`.

```bash
jit validate [<ID>] [--json]
jit validate <ID> --explain [--json]
jit validate --scope <ID> [--json]
jit validate --fix [--dry-run] [--json]
jit validate --branch-drift [--leases] [--json]
```

| Mode | What it does |
|------|--------------|
| (no arguments) | Whole repository: integrity checks (broken dependencies, unknown gates, label format, acyclicity, transitive reduction, readiness coherence, claims index) plus every local and graph rule. |
| `<ID>` | The local and graph rules for that issue only. |
| `--explain` | Per-rule outcome for one issue: which selectors matched, and `PASS`/`FAIL`/`SKIP` for each rule with the reason a skipped selector did not apply. Requires an issue id. |
| `--scope <ID>` | Evaluates the rules matching each issue in a container's transitive dependency closure, excluding whole-repository rules. Shaped as a deterministic gate checker: exit `4` on any error-severity finding, `0` when clean ([exit-code reference](exit-codes.md#command-specific-mappings)). |
| `--fix` | Apply automatic rule/graph/state fixes and provenance-proven derived-state repairs. `--dry-run` reports what would be fixed and writes nothing. |
| `--branch-drift` | Check that `origin/main` is an ancestor of the current branch. Requires git. |
| `--leases` | Check that active leases are consistent and not stale. |

**Mode exclusivity.** `--scope` may not be combined with a positional id or with
`--fix`/`--branch-drift`/`--leases`/`--explain`. `--fix`, `--branch-drift`, and
`--leases` are repository-wide, so combining any of them with a positional id is
a usage error rather than a silently ignored argument. `--dry-run` requires
`--fix`.

**Two distinct concepts.** `--branch-drift` is about git: it asks whether the
current branch still sits on top of `origin/main`. Membership divergence is
about the work graph: a membership label the DAG does not back, reported by
[`jit query divergence`](#membership-divergence-jit-query-divergence) and
mirrored in this command's advisory `divergence_count`.

Every successful JSON validation mode emits its normal mode-specific report. A
failing invocation emits the canonical error envelope and retains that same
report under `error.details`. This applies to whole-repository, per-issue,
`--explain`, `--scope`, `--branch-drift`, and `--leases` validation. For example,
a whole-repository rule failure reports:

```json
{
  "error": {
    "code": "GENERIC_ERROR",
    "message": "Repository validation failed with 1 rule error(s)",
    "details": {
      "valid": false,
      "integrity_error": null,
      "warnings": [
        { "type": "rule_warning", "issue_id": "...", "rule": "orphan-leaf", "message": "..." }
      ],
      "warning_count": 1,
      "membership_divergences": [],
      "divergence_count": 0,
      "rule_findings": [ { "issue_id": "...", "rule": "...", "message": "...", "severity": "error" } ],
      "error_count": 1,
      "message": "Repository validation failed with 1 rule error(s)"
    }
  }
}
```

Whole-repository, per-issue, and `--explain` rule failures keep their established
exit `1` through the registered `GENERIC_ERROR` mapping, as do failed
`--branch-drift` and `--leases` checks. A repository-integrity failure or failed
`--scope` gate check uses `VALIDATION_FAILED` and exits `4`. In every case the
code's registered mapping, not a separate literal, determines the process
status.

`divergence_count` and `membership_divergences` mirror
[`jit query divergence`](#membership-divergence-jit-query-divergence). They are
advisory and never change the exit status; resolve them with `jit query
divergence` when a membership label claims what the DAG does not back.

`validate --fix --json` returns the standard error envelope and retains the
failure's typed classification: an unsafe repair uses `VALIDATION_FAILED`, a
permission failure uses `PERMISSION_DENIED`, and other failures keep the
registered code selected by the shared classifier. That code determines the
process status. Its `error.message` retains the complete actionable cause chain,
including ambiguous managed-region delimiters or mismatched profile provenance.
The human error reports the same cause, and no repair target is written.

### `jit recover`

Run the recovery routines: clear locks left by dead processes (checked by PID),
rebuild the claims index from the append-only log, evict expired leases, and
remove temp files older than an hour.

```bash
jit recover [--json]
```

It removes only provably stale data, so it is safe to run at any time.

## Template Commands

### `jit apply`

Instantiate a graph template from `.jit/templates.toml` onto a container: create
the template's typed nodes with their gate presets, documents, and interpolated
descriptions, wire the declared dependency edges, and run its transforms (such as
moving the container's upstream dependencies onto a planning node).

```bash
jit apply <TEMPLATE> <CONTAINER> [--anchor <ROLE=ID>] [--force] [--json]
```

| Argument / flag | Description |
|-----------------|-------------|
| `<TEMPLATE>` | Template name. Must be declared in `.jit/templates.toml`. |
| `<CONTAINER>` | Container issue the template is applied to. |
| `--anchor <ROLE=ID>` | Bind a template anchor. Repeatable. The `container` anchor is auto-bound to `<CONTAINER>`; an explicit `--anchor container=<id>` overrides that binding. A value without an `=`, or with an empty role, is a usage error. |
| `--force` | Bypass validation warnings and refresh an already-applied template, rewriting the existing nodes' prose in place instead of creating new ones. |

The container's `type:` label must be one of the template's `applies_to` types.

```bash
jit apply plan epic-123
jit apply plan epic-123 --json
jit apply plan epic-123 --anchor container=epic-123 --force
```

JSON returns the role-to-id map plus the created issues:

```json
{
  "template": "plan",
  "container": "epic-123",
  "anchor_bindings": { "container": "epic-123" },
  "created_node_ids_by_role": { "planning": "<uuid>", "breakdown": "<uuid>" },
  "anchor_dependency_snapshots": {},
  "created_issues": { "planning": { "id": "<uuid>", "title": "..." } },
  "message": "Applied template 'plan' to epic-123"
}
```

Under `--quiet` the command prints one created id per line. Templates and their
node types are repository configuration: the `plan` template used in the examples
above is this repository's own declaration, not a shipped default.

## Item Commands

Items are addressable structured lines: entries in an issue's description (a
requirement, a decision, a risk) and entries in the project registries
(invariants, rules, gates). Each carries a self-id and resolves through a
kind-segmented qualified id:

- `@/<kind>/<self-id>` for a project item, e.g. `@/invariant/dag-acyclic`.
- `@/issue/<short-id>/<kind>/<self-id>` for an issue item, e.g.
  `@/issue/<short-id>/requirement/REQ-01`, where `<short-id>` stands for a real
  issue's short id.
- `<short-id>/<self-id>` as input sugar, where the kind is inferred from the
  self-id's shape.

Kinds and their aliases are declared in `[item_kinds]` in `.jit/config.toml`. An
alias is accepted anywhere a kind name is, so `@/inv/dag-acyclic` resolves the
same item as `@/invariant/dag-acyclic`; output always prints the registry name.
`jit init` scaffolds the table, and a repository with no `[item_kinds]` table
declares no kinds.

### `jit item list`

```bash
jit item list [--kind <KIND>] [--json]
```

`--kind` filters to one kind, named by its registry name or a declared alias.
Text output is one line per item: `<qualified_id>  [<kind>]  <text>`. JSON uses
the list envelope `{"count": N, "items": [...]}`, where each item is
`{kind, qualified_id, self_id, scope, text, ...}` and `scope` is the owning
issue's short id, or `@` for a project item.

### `jit item show` / `jit item resolve`

```bash
jit item show <QUALIFIED_ID> [--json]
jit item resolve <QUALIFIED_ID> [--json]
```

`resolve` is a distinct verb with identical behavior, for orchestrators that
think in terms of resolving an address. JSON returns
`{item, issue_full_id, issue_title}`; the two issue fields are `null` for a
project item, which no single issue owns.

```bash
jit item show @/issue/<short-id>/requirement/REQ-01   # <short-id> is a placeholder
jit item show @/invariant/dag-acyclic
jit item show @/inv/dag-acyclic          # alias of the same address
jit item show <short-id>/REQ-01 --json
```

### `jit item search`

```bash
jit item search [<QUERY>] [--kind <KIND>] [--json]
```

The query matches self-id, qualified id, and item text. It defaults to the empty
string, which matches everything, so `--kind` filters on its own.

```bash
jit item search atomic
jit item search "" --kind requirement
```

A failing `jit item` command under `--json` returns the error envelope with code
`ITEM_COMMAND_FAILED`.

## Documentation Projection Commands

An addressable item kind is a source of truth: invariants in `.jit/invariants.toml`,
rules in `.jit/rules.toml`, gates in `.jit/gates.toml`, and any markdown-first kind
in its own source file. A `[projection.<name>]` config table projects a kind (or
kinds) into a documentation target. Each target path, projection mode, render
style, and region delimiter comes from configuration alone; delimiters default to
`<!-- jit:<name>:begin -->` / `<!-- jit:<name>:end -->`. In `region` mode only the
delimited region of the target is rewritten and every byte outside it is
preserved; in `separate-file` mode the whole file is written.

### `jit project render`

```bash
jit project render [--name <name>] [--json]
```

Renders every declared `[projection.*]` into its configured target, or the single
`--name`d one. Two render styles: `id-anchor` writes generic `- **{self-id}** —
{text}` bullets from a **project-scoped** addressable kind's rows (issue-scoped
kinds have no project source and are not renderable); `full` writes the built-in
rich views (the invariant registry, or the rule + gate registries with their
metadata) and is likewise limited to **project-scoped** registry kinds. Both
styles apply the same pre-write guards: a projection that declares no `target`, a
declared source that does not exist (a markdown-first kind's source file or a
registry kind's store, under either style), an issue-scoped kind, an unknown kind, or an absent
region marker is a typed error (exit `4`) raised before any file is written — a
missing registry store is never rendered as an empty block. Rendering is
two-phase: every projection is rendered and its target's final bytes materialized
in memory before publication, so any such failure — even in a later projection
of a whole-`[projection.*]` run — leaves every target byte-identical. All changed
targets then publish through one recoverable repository transaction. JSON uses
the list envelope `{"count": N, "projections": [...]}`, each entry `{name, target,
mode, style, kinds, count}`. Under `--json` the pre-write guards above surface the
`VALIDATION_FAILED` envelope; any other command failure (for example, an
unrecognized `--name`) returns the error envelope with code
`PROJECT_COMMAND_FAILED`.

### `jit invariant check`

```bash
jit invariant check [--json]
```

Reports enforcement drift in the declared-but-unenforced direction: an invariant
whose `enforced-by` names a rule or gate that does not load. Bindings are
declarations, and this check never executes them. Exits `4` when any drift is
present (see the [exit-code reference](exit-codes.md#command-specific-mappings)).
With no drift, JSON uses the list envelope `{"count": 0, "findings": []}`. Drift
under `--json` returns the `VALIDATION_FAILED` error envelope and preserves the
report as `error.details`, including its `count` and `findings` fields.

## Repository Search

### `jit search`

Grep the `.jit/` data directory: the stored issue records, the event log, and the
other files jit keeps there. Matching is delegated to
[ripgrep](https://github.com/BurntSushi/ripgrep), which must be on `PATH`.

```bash
jit search <QUERY> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-r`, `--regex` | Treat `<QUERY>` as a regular expression. Without it the query is a fixed string. |
| `-C`, `--case-sensitive` | Match case. The default is case-insensitive. |
| `-c`, `--context <CONTEXT>` | Context lines requested from the matcher. Defaults to `0`. Results list the matching lines. |
| `-n`, `--limit <LIMIT>` | Stop after `LIMIT` results. |
| `-g`, `--glob <GLOB>` | Restrict the search to files matching a glob, e.g. `"*.json"` or `"*.md"`. |

```bash
jit search "rate limit"
jit search '^REQ-\d+' --regex --limit 20
jit search auth --glob "*.json" --json
```

JSON uses the list envelope, with the query echoed back:

```json
{
  "query": "auth",
  "count": 1,
  "results": [
    {
      "issue_id": "003f9f83-4e8a-4a5f-8e48-44f6f48a7c17",
      "path": "/path/to/repo/.jit/issues/003f9f83-4e8a-4a5f-8e48-44f6f48a7c17.json",
      "line_number": 12,
      "line_text": "  \"title\": \"Harden auth\",",
      "matches": [{ "text": "auth", "start": 22, "end": 26 }]
    }
  ],
  "message": "Found 1 result(s)"
}
```

`path` is the file's path as the matcher reports it, and `issue_id` is the issue
the file belongs to, or `null` for a file that is not an issue record. `matches`
gives each match's byte offsets within `line_text`. A missing ripgrep
returns the error envelope with code `RIPGREP_NOT_FOUND`; any other matcher
failure returns `SEARCH_FAILED`. Both exit `10` (external dependency failed; see
the [exit-code reference](exit-codes.md)).

To search issue fields rather than raw storage lines, use
[`jit issue search`](#searching-issues-jit-issue-search); to search addressable
items, use `jit item search`.

## Snapshot Commands

### `jit snapshot export`

Export a self-contained snapshot of issues, the documents they reference, the
assets those documents use, a manifest of SHA256 hashes, and a README describing
the contents. A snapshot preserves its provenance: git commit, source mode, and
timestamps.

```bash
jit snapshot export [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--out <OUT>` | Output path. Defaults to `snapshot-YYYYMMDD-HHMMSS`. |
| `--format <FORMAT>` | `dir` (default) or `tar`. |
| `--scope <SCOPE>` | `all` (default), `issue:<ID>`, or `label:<namespace>:<value>`. |
| `--at <AT>` | Export the documents as of a git commit or tag. Requires a git repository. |
| `--working-tree` | Export the documents from the working tree instead of git. |
| `--committed-only` | Fail when uncommitted documents or assets exist. Requires git, and implies `--at HEAD`. |
| `--force` | Skip repository validation before exporting. |

```bash
jit snapshot export
jit snapshot export --scope label:epic:auth --format tar --out auth-snapshot.tar
jit snapshot export --at abc123 --out release-v1.0
jit snapshot export --working-tree
```

JSON returns `{path, issue_count, document_count, format, size_bytes, message}`;
`size_bytes` is `null` for a directory export.

## Server Commands

### `jit serve`

Start the JIT API and web UI server for the current repository as a background
daemon, or inspect or stop a running one. jit runs one server process per
repository, tracked by a PID file (`server.pid.json` under `.jit/`); a second
`jit serve` reports the already-running server rather than starting a duplicate. When the
preferred port is taken, jit scans upward from the requested port through
requested+99 for the first free one (so 3000–3099 for the default `--port 3000`,
or 5000–5099 for `--port 5000`). The
server exposes the HTTP API under `/api` and the web UI at `/` — from built
static files when a web directory is found, otherwise from assets embedded in the
binary.

```bash
jit serve [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--port <PORT>` | Preferred port to listen on (default `3000`); when it is taken, jit scans the 100 ports from the requested port upward for the first free one (3000–3099 for the default). |
| `--stop` | Stop the running server for this repository. |
| `--status` | Report whether a server is running and exit. |
| `--fg` | Run in the foreground instead of daemonizing (useful for debugging; Ctrl+C stops it). |
| `--log <FILE>` | Where the daemonized server writes its output. A relative path resolves under `.jit/` (default `server.log` there); an absolute path is used as given. It governs daemon mode only — under `--fg` the server's output stays attached to the terminal and `--log` is not used. |
| `--web-dir <DIR>` | Directory of built web UI static files (auto-detected when omitted). |
| `--json` | Emit machine-readable output. |

`--stop`, `--status`, and `--fg` are mutually exclusive.

`--stop` and Ctrl+C under `--fg` shut the server down gracefully. The server
stops accepting connections at once and releases its port, live event streams
(`/api/events/stream`) end so subscribers see the stream close, requests still
in flight get up to five seconds to finish, and anything still open when that
deadline expires is closed by the server. The process then exits `0`, so a
successor can be started immediately.

```bash
jit serve                 # Start (or report an already-running server)
jit serve --port 3010     # Prefer a specific port
jit serve --status        # Check whether a server is running
jit serve --stop          # Stop the running server
jit serve --fg            # Run in the foreground for debugging
jit serve --json          # Machine-readable output
```

`--json` prints a bespoke status object rather than an issue envelope. A start
reports the launched process:

```json
{
  "status": "started",
  "pid": 12345,
  "port": 3000,
  "url": "http://localhost:3000",
  "log_file": "/repo/.jit/server.log",
  "web_ui": true,
  "web_ui_source": "embedded"
}
```

`status` is `started` (a new background daemon launched), `running` (already up,
or the `--status` view), `stopped`, `not_running`, `exited` (a foreground `--fg`
run that ended, carrying its `exit_code`), or `error` (a start, stop, or status
operation that failed, carrying an accompanying `error` message field).
`web_ui_source` is `embedded` or `filesystem`, per where the UI assets were
served from.

## Git Hook Commands

### `jit hooks install`

Copy the hook templates into `.git/hooks/` and make them executable.

```bash
jit hooks install [--json]
```

Two hooks are installed: `pre-commit` validates leases and branch divergence
before a commit, and `pre-push` validates leases before a push. An existing hook
of the same name is left in place and reported as skipped.

JSON returns `{hooks_dir, installed, skipped, message}`. A failure returns the
error envelope with code `HOOKS_INSTALL_ERROR`.

Enforcement strictness is configuration, set in `.jit/config.toml`:

```toml
[worktree]
enforce_leases = "strict"
```

## Maintenance Commands

### `jit migrate lifecycle-timestamps`

Backfills the issue [lifecycle
timestamps](storage-format.md#lifecycle-timestamps) (`first_ready_at`,
`claimed_at`, `done_at`) for issues whose stored records are missing one or more
of these fields.

```
jit migrate lifecycle-timestamps [--json]
```

For every issue missing one of the fields, the value is derived from
`.jit/events.jsonl`: the first `issue_state_changed` into `ready`, the first
`issue_claimed`, and the first `issue_state_changed` into `done`, respectively.
Only still-absent fields are filled — an existing timestamp is never overwritten,
preserving first-occurrence semantics. Updated issues are written atomically and
a single `lifecycle_timestamps_backfilled` event records the count.

The command is **idempotent**: when no issue is missing a timestamp it writes
nothing, appends no event, and reports `issues_updated: 0`. An issue whose event
log carries no matching transition — for example one auto-promoted straight to
`ready` at creation, which logs no transition — keeps those fields unset; such a
timestamp is unrecoverable, not defaulted.

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
# A leaf under any section of the config.
jit config get type_hierarchy.strategic_types
# ["milestone", "epic"] (pretty-printed; a single scalar prints bare)

jit config get documentation.development_root
# dev

jit config get namespaces.type.unique
# true

# An intermediate key: the whole section.
jit config get documentation --json
# {"key": "documentation", "value": {"development_root": "dev", ...}}

# The layered settings resolve through their precedence chain
# (env var, then repo, then user, then system, then default).
jit config get worktree.mode
jit config get coordination.default_ttl_secs
```

**Two resolution strategies**, matching how the rest of jit already reads
these sections — `config get` does not invent a third:
- `worktree`, `coordination`, `global_operations`, `locks`, `events`: the
  system/user/repo-merged, default-filled view (same as `jit config show`).
- Every other section (`version`, `project`, `type_hierarchy`, `validation`,
  `documentation`, `namespaces`, `item_kinds`, `projection`): read from the
  REPO's `config.toml` only, with no
  system/user merge and no built-in defaults layered in — jit has no concept
  of a system/user override for a repo's type hierarchy or label namespaces.
  This means these sections reflect exactly what `config.toml` declares
  (an absent section resolves to `{}`), which can differ from `jit config
  show`'s built-in-default-filled view of the sections it covers (e.g.
  `namespaces`).

`templates` and `invariants` are NOT part of the dotted-path surface: both
are loaded from sibling files (`.jit/templates.toml`, `.jit/invariants.toml`)
rather than `config.toml` itself. Introspect them via `jit config
list-templates` / `jit item list --kind invariant`.

**Exit codes** (per the [exit-code reference](exit-codes.md#command-specific-mappings)):
`0` when the key resolves, `2` (`INVALID_ARGUMENT`) for an unknown key. An unknown
TOP-LEVEL key names the valid sections; an unknown NESTED key names the missing
segment and its resolved parent path.

```bash
jit config get bogus_section
# Error: unknown config key 'bogus_section'; valid top-level sections:
# coordination, documentation, events, global_operations,
# item_kinds, locks, namespaces, project, projection,
# type_hierarchy, validation, version, worktree

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
user-global) `config.toml`. A repository-level set that changes the namespace
registry also synchronizes the default `namespace-unique-*` rows in
`rules.toml` (same write-through as re-init; custom rules and policy edits are
untouched).

```bash
jit config set <KEY> <VALUE> [--global] [--json]

jit config set coordination.default_ttl_secs 1200
jit config set --global worktree.enforce_leases warn
```

### `jit config validate`

Validate the repo, user, and environment-variable configuration for syntax
errors and invalid values (both surface when a config source fails to load).

```bash
jit config validate [--json]
```

See the [command-specific `config validate` mapping in the generated exit-code
reference](exit-codes.md#command-specific-mappings) for its outcomes.

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
ISSUE_ID=$(jit issue create --title "Add feature" --orphan --quiet --json | jq -r '.id')

# Extract specific fields
jit issue show $ISSUE_ID --json --quiet | jq -r '.title'

# Process lists
jit query all --json --quiet | jq -r '.issues[] | select(.priority == "high") | .id'

# Query and filter
jit query available --json --quiet | jq -r '.issues[0].id'

# Get status counts (per-state counts are top-level fields of the response)
jit status --json --quiet | jq '{ready, in_progress, blocked, gated, done, rejected}'
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
| `query divergence` | `divergences` |
| `invariant check` (when no drift is found) | `findings` |

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
# Evaluate automated gates from CI. For an automated gate, `jit gate evaluate`
# runs its checker and records the verdict; its exit code is nonzero when the
# gate fails. (`jit gate fail` is for manual gates only.)

ISSUE_ID=$1

# tests and clippy are automated gates: evaluate runs their checkers.
if jit gate evaluate "$ISSUE_ID" tests --quiet; then
  echo "✓ Tests passed for $ISSUE_ID"
else
  echo "✗ Tests failed for $ISSUE_ID"
  exit 1
fi

if jit gate evaluate "$ISSUE_ID" clippy --quiet; then
  echo "✓ Clippy passed for $ISSUE_ID"
else
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
READY=$(jit query available --json --quiet | jq -r '.count')
IN_PROGRESS=$(jit query all --state in_progress --json --quiet | jq -r '.count')
BLOCKED=$(jit query blocked --json --quiet | jq -r '.count')
# jit events query has no date filter (--event-type/--issue-id/--limit only),
# so this counts done-transitions within the most recent 100 events, not a
# calendar-day total. Raise --limit or filter on .timestamp in jq for a wider
# or date-scoped window.
RECENT_DONE=$(jit events query --event-type issue_state_changed --limit 100 --json | \
  jq -r '[.events[] | select(.to == "done")] | length')

echo "Ready: $READY"
echo "In Progress: $IN_PROGRESS"
echo "Blocked: $BLOCKED"
echo "Recently completed (last 100 events): $RECENT_DONE"
```
