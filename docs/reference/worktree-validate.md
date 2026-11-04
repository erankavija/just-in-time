# jit worktree and jit validate Command Reference

> **Diátaxis Type:** Reference  
> **Last Updated:** 2026-08-16

CLI reference for worktree information and repository validation commands.

---

## jit worktree

Display and manage git worktree context for parallel work.

```bash
jit worktree <COMMAND> [OPTIONS]
```

### Global Options

| Option | Description |
|--------|-------------|
| `-q, --quiet` | Suppress non-essential output |
| `--json` | Emit machine-readable JSON |
| `-h, --help` | Print help information |

---

### jit worktree info

Show current worktree information.

#### Synopsis

```bash
jit worktree info [OPTIONS]
```

#### Description

Displays the current worktree's identity, branch, root path, and whether this is the main worktree or a secondary one. Useful for debugging worktree detection and understanding context.

#### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

#### Examples

```bash
# Show current worktree info
jit worktree info

# JSON output for scripting
jit worktree info --json
```

#### Output

```
Worktree Information:
  ID:         wt:a1b2c3d4
  Branch:     feature/my-work
  Root:       /home/user/project-wt
  Type:       secondary worktree
  Common dir: /home/user/project/.git
```

`Type:` prints `main worktree` or `secondary worktree`. `Common dir:` is the
shared git directory (the repository's `.git`) common to every worktree — not
the main worktree's path.

For JSON output (the response envelope also carries `message` and `warnings`):
```json
{
  "worktree_id": "wt:a1b2c3d4",
  "branch": "feature/my-work",
  "root_path": "/home/user/project-wt",
  "is_main_worktree": false,
  "common_dir": "/home/user/project/.git"
}
```

#### Failure modes

The command fails outside a git repository and when worktree detection cannot
complete. For the code each failure exits with, see the
[Exit Codes reference](exit-codes.md).

---

### jit worktree list

List all git worktrees with JIT status.

#### Synopsis

```bash
jit worktree list [OPTIONS]
```

#### Description

Shows all worktrees associated with the repository, including their worktree ID, current branch, path, and count of active claims. Expired leases and indefinite leases with stale heartbeats do not count as active. Useful for seeing which worktrees exist and what work is happening in each.

#### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

#### Examples

```bash
# List all worktrees
jit worktree list

# JSON output
jit worktree list --json
```

#### Output

A table with one row per worktree — columns `WORKTREE ID`, `BRANCH`, `PATH`,
`CLAIMS` (the count of active claims held from that worktree):

```
WORKTREE ID      BRANCH                    PATH                                                 CLAIMS
----------------------------------------------------------------------------------------------------
wt:main          main                      /home/user/project                                        0
wt:a1b2c3d4      feature/auth              /home/user/project-feature                                2
wt:e5f6g7h8      bugfix/login              /home/user/project-bugfix                                 1
```

For JSON output (envelope also carries `message` and `warnings`):
```json
{
  "count": 3,
  "worktrees": [
    {
      "worktree_id": "wt:main",
      "branch": "main",
      "path": "/home/user/project",
      "is_main": true,
      "active_claims": 0
    }
  ]
}
```

#### Failure modes

The command fails outside a git repository and when the worktrees cannot be
listed. For the code each failure exits with, see the
[Exit Codes reference](exit-codes.md).

---

### jit worktree exec

Run a command holding the repository write lock.

#### Synopsis

```bash
jit worktree exec [OPTIONS] -- <COMMAND> [ARGS]...
```

#### Description

The command waits for the lock up to `JIT_LOCK_TIMEOUT` seconds, refuses a
pending redo record without applying it, runs `<COMMAND>`, exits with its status,
and releases the lock. It knows nothing about what the command does.

Every mutation waits for the lock, so what the command reads and writes under
`.jit/` holds still while it runs. A `jit` mutation run by the command waits on
the same lock and fails with `LOCK_TIMEOUT`; `jit` reads run normally.

`--json` shapes only this command's own failures, and belongs before `--`.

#### Failure modes

| Failure | Code | Exit |
|---------|------|------|
| Lock wait expired; the command did not run | `LOCK_TIMEOUT` | 11 |
| Pending redo record, or the command could not start | `GENERIC_ERROR` | 1 |

Any other status is the command's own.

---

### jit worktree store-divergence

Report the records the inspected checkout's store and the primary checkout's
store disagree about.

#### Synopsis

```bash
jit worktree store-divergence [OPTIONS]
```

#### Description

Each checkout owns a store under its own `.jit/`. Issue records are read with a
fallback from the local store to the primary checkout, but event history has no such fallback, so a linked checkout's events
are visible only from inside it. This check compares the two stores as they
physically sit — no fallback, no aggregation — and names every issue record and
every event record they hold differently.

The check reads only. It writes to neither store and repairs nothing; it reports
what diverged so you can decide what to do about it.

It reports nothing in three cases: when the inspected store is the primary
checkout's, which has no other store to compare against; when it is a linked
checkout's and agrees with the primary; and outside version control, where there
is no second checkout at all. An empty report therefore means "nothing to report
here" in every environment, and the `Reference store:` line says which case you
are in.

#### Store selection

The inspected store is the selected data root, and the checkout holding that
root is the checkout under inspection — the working directory the command runs
in only supplies the default. That default is the running checkout's own
`.jit/`; `JIT_DATA_DIR` names a data root explicitly and takes precedence over
discovery. The reference store is then that same root resolved under the
repository's primary checkout.

Selecting a linked checkout's store therefore inspects that store against the
primary's own, whichever directory the command runs from — including the primary
checkout itself:

```bash
JIT_DATA_DIR=../feature-worktree/.jit jit worktree store-divergence
```

A selected root that is the primary checkout's own store, or one outside version
control, has no counterpart to resolve and reports no reference store.

#### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

#### Examples

```bash
# Report what this checkout's store and the primary's disagree about
jit worktree store-divergence

# JSON output for scripting
jit worktree store-divergence --json
```

#### Finding classes

Every finding names one record, the kind of record it is (`issue` or `event`),
and how the two stores disagree about it:

| Class | Meaning |
|--------|---------|
| `local_only` | This checkout's store holds the record; the primary checkout's store does not. |
| `reference_only` | The primary checkout's store holds the record; this checkout's store does not. |
| `conflicting` | Both stores hold the record under one identity, with values that differ. |

Records both stores hold identically are agreement and produce no finding.
Issue records are matched on issue id and event records on event id.

#### Output

The two compared stores, then one row per finding:

```
Checkout store:  /home/user/project-feature/.jit
Reference store: /home/user/project/.jit

RECORD   CLASS            ID
------------------------------------------------------------------------
issue    local_only       4a281416-0e75-4cec-aaaa-35c5b964512a
issue    reference_only   452ce1ea-8833-411d-a5c6-baed654c9379
event    conflicting      eee505cd-57e8-4ede-9606-e46cb75d5aba

3 divergence(s)
```

A checkout with nothing to report prints `No divergent records.` in place of the
table.

JSON output uses the list envelope (which also carries `message`), plus the two
compared stores; `reference_store` is `null` when the checkout has none:

```json
{
  "count": 3,
  "checkout_store": "/home/user/project-feature/.jit",
  "reference_store": "/home/user/project/.jit",
  "divergences": [
    {
      "record": "issue",
      "class": "local_only",
      "id": "4a281416-0e75-4cec-aaaa-35c5b964512a"
    }
  ]
}
```

Findings are ordered by record kind, issues before events, and by id within each
kind, so re-running against unchanged stores reports the same sequence.

#### Failure modes

Reporting a divergence is a success: the command exits `0` whether or not it
finds anything. It fails when the primary checkout cannot be resolved, or when
either store holds a record that cannot be read or parsed. For the code each
failure exits with, see the [Exit Codes reference](exit-codes.md).

---

## jit validate

Validate repository integrity and consistency.

### Synopsis

```bash
jit validate [ID] [OPTIONS]
```

### Description

`jit validate` runs the repository's declarative rules and integrity checks; its
behavior depends on the arguments.

- **Whole-repository** (`jit validate`, no id): evaluates every issue against the
  declarative ruleset in `.jit/rules.toml` (label format, namespace registry,
  uniqueness, type-hierarchy, and any project graph rules) and the built-in
  integrity checks — broken dependency references, gate references absent from
  the registry, DAG cycles, issues isolated from the dependency graph, redundant
  (transitively-reducible) edges, and record fidelity:
  the index, issue records, gate-run results, verdict records, archive evidence,
  and event log each decode and reproduce what the writer would publish, and no event id appears on
  more than one line. An event log that does not parse is one finding naming its
  line. This mode and the `repository_validation` checker read the whole event
  log and every gate-run record for fidelity; other validation modes do not
  read history. See [History Files](storage-format.md#history-files) for all
  history readers. Advisory membership-vs-DAG divergences are reported but do
  not fail the run. A built-in `repository_validation` gate runs this same
  computation and never passes where it fails; see
  [gate presets](gate-presets.md) for its `failed` and `error` runs.
- **Per-issue** (`jit validate <id>`): reports the findings concerning that
  one issue. The id accepts a full UUID, 8-char short id, or unique prefix.
- **Selected rule** (`jit validate <id> --rule <rule>`): applies exactly one
  configured graph rule with that issue as its sole firing subject. The complete
  repository remains graph and identifier-resolution context only; a missing,
  disabled, non-graph, or non-matching rule is an error rather than a vacuous
  pass.
- **Explain** (`jit validate <id> --explain`): every configured rule's outcome
  for that issue; a matched rule fails exactly when the per-issue view holds a
  finding of it.
- **Scope** (`jit validate --scope <id>`): the findings of a container's bracket
  scope, the issues in its transitive dependency closure bounded at its
  template's breakdown node, excluding the graph rules that need the whole
  repository at transition time. It is shaped as a deterministic gate checker.

#### Views

Every mode above except whole-repository validation is a view: a selection of
the one whole-repository report. Rules evaluate over the whole repository, and
the view keeps the findings its selection admits, so a coverage child or
dependency target outside the selected issue or scope still counts. Views skip
the history reads. Selections differ only in what they admit:

- **Unloadable rule set.** The per-issue, selected-rule, and scope views report
  it as the `rules-file` finding; `--explain` fails with that finding's message.
- **Enforcement drift.** `enforcement-drift` findings appear in the scope view
  and never in the per-issue, explain, or selected-rule views.
- **Graph-rule configuration errors.** The per-issue and explain views admit
  one when the rule applies to the issue and the error concerns that issue or
  no single issue. The selected-rule view admits the selected rule's error under
  the same condition. The scope view admits one that concerns a scope member, or
  concerns no single issue and comes from a rule that applies to a member.

Each built-in validation gate checker reports what its view reports; see
[gate presets](gate-presets.md#portable-checker-types).

`--fix` additionally repairs the auto-fixable findings (type-hierarchy label
fixes, transitive-reduction violations, pending state transitions) and
derived-state drift (default rules and schemas, configured projections). Derived
repairs publish through one redo record
([Publication and Recovery](storage-format.md#publication-and-recovery)) and
preserve authored or unmanaged bytes. An ambiguous target is reported without
writes. Coordination state — stale
locks and expired leases — is repaired by
[`jit recover`](#jit-recover), not by `jit validate`.
Plain `jit validate` does not check leases; `--leases` checks lease records
for expiry and references to missing worktrees or issues.

### Options

| Option | Description |
|--------|-------------|
| `[ID]` | Positional. Report this issue's [view](#views) of the whole-repository report; omit to validate the whole repository |
| `--explain` | Report which rules matched the issue and whether each passed (requires an `[ID]`) |
| `--rule <RULE>` | Apply one configured graph rule with `[ID]` as its sole firing issue; clean exits `0`, and a rule or selection failure exits `1`. Invalid combinations or a missing required `[ID]` are usage errors and exit `2` |
| `--scope <ID>` | Report the findings of a container's bracket scope as a deterministic gate checker. Mutually exclusive with `[ID]`, `--fix`, `--branch-drift`, `--leases`, `--explain`, and `--rule` |
| `--fix` | Auto-fix repairable rule/graph/state findings and derived-state drift |
| `--dry-run` | Show what `--fix` would change without applying it (requires `--fix`) |
| `--branch-drift` | Validate that git's `origin/main` is an ancestor of the current branch (requires git) |
| `--leases` | Report expired leases and leases referencing missing worktrees or issues |
| `--json` | Output as JSON |

`--fix`, `--branch-drift`, and `--leases` are repo-wide and cannot be combined
with a positional `[ID]`.

### Examples

```bash
# Whole-repository validation
jit validate

# The findings concerning one issue (its view of the whole-repository report)
jit validate a1b2c3d4

# Explain which rules apply to an issue
jit validate a1b2c3d4 --explain

# Reproduce one configured graph-rule application
jit validate a1b2c3d4 --rule coverage-preview

# Validate a container's bracket subtree as a gate check
jit validate --scope <container-id>

# Check git branch drift and expired or dangling leases
jit validate --branch-drift --leases

# Preview, then apply, the auto-fixes
jit validate --fix --dry-run
jit validate --fix

# JSON output for CI
jit validate --json
```

### Output

Whole-repository, clean:
```
Repository validation passed
```

Whole-repository, with an integrity error and a warning-severity finding:
```
error: Repository integrity error: Invalid repository: repository-integrity validation pass: Invalid dependency: issue 'a1b2c3d4' depends on 'deadbeef' which does not exist
warning [orphan-leaf] issue e5f6g7h8 (type:task) is an orphaned leaf with no parent association label

Warnings: 1
```

Error-severity rule findings print with a leading `error [<rule>]`, warnings with
`warning [<rule>]`.

If `--fix` cannot repair a target unambiguously, human output includes the complete actionable
cause chain. With `--json`, the same failure is a standard error object whose
code is `VALIDATION_FAILED` and whose message retains that chain (for example,
the target and duplicate delimiter); see
the [machine-readable failure contract](cli-commands.md#cli-json-contracts).

### Failure modes

`jit validate` passes when it finds nothing to report (and, under `--fix`, when
its repairs complete). It signals findings by exiting non-zero:

- **Declarative-rule errors** — whole-repo or per-issue.
- **Repository-integrity failures** — a broken dependency, a DAG cycle, an
  isolated issue, a redundant edge, an unknown gate reference, or a stored record
  that fails fidelity checks. A `--scope` error finding reports the same way.
- **Check failures** under `--explain`, `--branch-drift`, or `--leases`.

It fails as a usage error when `--dry-run` is given without `--fix`, and on the
hidden `--divergence` stub, which errors and redirects to `--branch-drift` or
`jit query divergence`. It fails as not-found when the positional issue id or the
`--scope` container does not resolve. A rule pass that fails, such as a rule
reading a planning document that is missing, fails every view with that error
rather than reporting a clean selection; whole-repository validation reports it
as a structural failure beside the findings collected before it.

For `--rule`, a missing, disabled, non-graph, or non-matching configured rule is
a validation finding and exits `1`, just like an uncovered selected application.
Only invalid flag combinations or a missing required positional issue id are
usage errors and exit `2`.

The code each outcome exits with — including the codes that signal findings
rather than errors — is in the [Exit Codes reference](exit-codes.md).

### When to run it

Use `jit validate` for:

- Manual health checks
- CI/CD pipelines
- Debugging coordination issues
- After crashes or unexpected termination

---

## jit recover

Run recovery: apply a leftover redo record, then the repair routines.

### Synopsis

```bash
jit recover [OPTIONS]
```

### Description

Like every mutating command, `jit recover` applies a leftover redo record when
its session opens
([Publication and Recovery](storage-format.md#publication-and-recovery)). It
then runs, in sequence:
1. Repair declaration-derived state through the `jit validate --fix` path
2. In a Git repository, the coordination routines under `.git/jit/`: clear
   claim locks left by dead processes (checked by PID), evict expired leases,
   reclaim indefinite leases whose heartbeat is [stale](claim.md#staleness), and remove
   interrupted lease operations' `*.tmp` stage files under `leases/` older than
   the [cleanup threshold](runtime-defaults.md). Without Git they are skipped
   with a warning.
3. Remove orphaned `*.tmp` files under `.jit/` older than the
   [cleanup threshold](runtime-defaults.md)

It removes only provably stale data and never the pending redo record, so it is
safe to run at any time.

### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

### Examples

```bash
# Run recovery
jit recover

# JSON output
jit recover --json
```

### Output

```
Recovery complete:
  • Redo records applied: 0
  • Stale locks cleaned: 1
  • Expired leases evicted: 2
  • Stale leases reclaimed: 1
  • Temp files removed: 0
  • Derived-state targets repaired: 0
```

### Failure modes

The command fails with `recovery_failed` when applying a leftover redo record,
or a recovery routine, fails. For the status that code exits with, see the
[Error Codes reference](error-codes.md).

---

## See Also

- [Troubleshooting Guide](../how-to/troubleshooting.md) - Common issues and solutions
- [jit claim Reference](claim.md) - Lease management commands
- [Configuration Reference](configuration.md) - Worktree settings
- [Parallel Work Tutorial](../tutorials/parallel-work-worktrees.md) - Getting started
