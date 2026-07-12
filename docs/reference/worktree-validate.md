# jit worktree and jit validate Command Reference

> **Diátaxis Type:** Reference  
> **Last Updated:** 2026-02-02

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

Shows all worktrees associated with the repository, including their worktree ID, current branch, path, and count of active claims. Useful for seeing which worktrees exist and what work is happening in each.

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
  (transitively-reducible) edges, and claims-index consistency. Advisory
  membership-vs-DAG divergences are reported but do not fail the run.
- **Per-issue** (`jit validate <id>`): runs the declarative rules for that one
  issue. The id accepts a full UUID, 8-char short id, or unique prefix.

`--fix` additionally repairs the auto-fixable findings (type-hierarchy label
fixes, transitive-reduction violations, pending state transitions). Coordination
state — stale locks, the claims index, expired leases — is repaired by
[`jit recover`](#jit-recover), not by `jit validate`.

### Options

| Option | Description |
|--------|-------------|
| `[ID]` | Positional. Validate this one issue's rules; omit to validate the whole repository |
| `--explain` | Report which rules matched the issue and whether each passed (requires an `[ID]`) |
| `--scope <ID>` | Evaluate a container's bracket subtree as a deterministic gate checker (the rules whose selector matches each issue in the container's dependency closure). Mutually exclusive with `[ID]`, `--fix`, `--branch-drift`, `--leases`, and `--explain` |
| `--fix` | Auto-fix the repairable findings (type-hierarchy, transitive reduction, pending transitions) |
| `--dry-run` | Show what `--fix` would change without applying it (requires `--fix`) |
| `--branch-drift` | Validate that git's `origin/main` is an ancestor of the current branch (requires git) |
| `--leases` | Report active leases that are inconsistent or stale |
| `--json` | Output as JSON |

`--fix`, `--branch-drift`, and `--leases` are repo-wide and cannot be combined
with a positional `[ID]`.

### Examples

```bash
# Whole-repository validation
jit validate

# Validate a single issue's rules
jit validate a1b2c3d4

# Explain which rules apply to an issue
jit validate a1b2c3d4 --explain

# Validate a container's bracket subtree as a gate check
jit validate --scope <container-id>

# Check git branch drift and lease health
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
✓ Repository validation passed
```

Whole-repository, with an integrity error and a warning-severity finding:
```
❌ Repository integrity error: Invalid dependency: issue 'a1b2c3d4' depends on 'deadbeef' which does not exist
⚠ [orphan-leaf] issue e5f6g7h8 (type:task) is an orphaned leaf with no parent association label

Warnings: 1
```

Error-severity rule findings print with a leading `❌ [<rule>]`, warnings with
`⚠ [<rule>]`.

### Failure modes

`jit validate` passes when it finds nothing to report (and, under `--fix`, when
its repairs complete). It signals findings by exiting non-zero:

- **Declarative-rule errors** — whole-repo or per-issue.
- **Repository-integrity failures** — a broken dependency, a DAG cycle, an
  isolated issue, a redundant edge, an unknown gate reference, or a bad claims
  index. A `--scope` error finding reports the same way.
- **Check failures** under `--explain`, `--branch-drift`, or `--leases`.

It fails as a usage error when `--dry-run` is given without `--fix`, and on the
hidden `--divergence` stub, which errors and redirects to `--branch-drift` or
`jit query divergence`. It fails as not-found when the positional issue id or the
`--scope` container does not resolve.

The code each outcome exits with — including the codes that signal findings
rather than errors — is in the [Exit Codes reference](exit-codes.md).

### Integration with Hooks

The pre-commit hook runs similar validation automatically. Use `jit validate` for:

- Manual health checks
- CI/CD pipelines
- Debugging coordination issues
- After crashes or unexpected termination

---

## jit recover

Run automatic recovery routines.

### Synopsis

```bash
jit recover [OPTIONS]
```

### Description

Runs the coordination-recovery routines under `.git/jit/` in sequence:
1. Clean up stale lock files (those owned by dead processes)
2. Rebuild the claims index from the append-only claims log
3. Evict expired leases
4. Remove leftover temporary files

This repairs multi-agent coordination state. It is a different repair set from
`jit validate --fix`, which fixes rule/graph findings (type-hierarchy labels,
transitive-reduction violations, pending state transitions).

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
  • Stale locks cleaned: 1
  • Index rebuilt: true
  • Expired leases evicted: 2
  • Temp files removed: 0
```

### Failure modes

The command fails when recovery itself fails. For the code that failure exits
with, see the [Exit Codes reference](exit-codes.md).

---

## See Also

- [Troubleshooting Guide](../how-to/troubleshooting.md) - Common issues and solutions
- [jit claim Reference](claim.md) - Lease management commands
- [Configuration Reference](configuration.md) - Worktree settings
- [Parallel Work Tutorial](../tutorials/parallel-work-worktrees.md) - Getting started
