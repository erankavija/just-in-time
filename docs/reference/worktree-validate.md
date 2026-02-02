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
  ID:       wt:a1b2c3d4
  Branch:   feature/my-work
  Root:     /home/user/project-wt
  Type:     secondary
  Main:     /home/user/project
```

For JSON output:
```json
{
  "worktree_id": "wt:a1b2c3d4",
  "branch": "feature/my-work",
  "worktree_root": "/home/user/project-wt",
  "is_main": false,
  "main_worktree": "/home/user/project"
}
```

#### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Not in a git repository |
| 1 | Worktree detection failed |

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

```
Git Worktrees (3):

  /home/user/project (main)
    ID:      wt:main
    Branch:  main
    Claims:  0

  /home/user/project-feature
    ID:      wt:a1b2c3d4
    Branch:  feature/auth
    Claims:  2

  /home/user/project-bugfix
    ID:      wt:e5f6g7h8
    Branch:  bugfix/login
    Claims:  1
```

#### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Not in a git repository |
| 1 | Failed to list worktrees |

---

## jit validate

Validate repository integrity and consistency.

### Synopsis

```bash
jit validate [OPTIONS]
```

### Description

Checks the JIT repository for consistency issues including:
- Orphaned lock files
- Corrupted or inconsistent claims index
- Sequence gaps in audit logs
- Stale leases (with `--leases`)
- Branch divergence from main (with `--divergence`)

Can optionally fix detected issues with `--fix`.

### Options

| Option | Description |
|--------|-------------|
| `--fix` | Attempt to automatically fix validation issues |
| `--dry-run` | Show what would be fixed without applying (requires `--fix`) |
| `--divergence` | Validate branch hasn't diverged from main |
| `--leases` | Validate active leases are consistent and not stale |
| `--json` | Output as JSON |

### Examples

```bash
# Basic validation
jit validate

# Check everything including leases and divergence
jit validate --divergence --leases

# See what would be fixed
jit validate --fix --dry-run

# Actually fix issues
jit validate --fix

# JSON output for CI
jit validate --json
```

### Output

Success:
```
✓ Repository validation passed
  - Claims log: OK (42 entries)
  - Claims index: OK (5 active leases)
  - Locks: OK (no stale locks)
```

With issues:
```
✗ Validation failed with 2 issues:
  - Orphaned lock file: .git/jit/locks/claims.lock
  - Index sequence gap at position 42

Run 'jit validate --fix' to attempt automatic repair.
```

With `--fix`:
```
✓ Fixed 2 issues:
  - Removed orphaned lock: .git/jit/locks/claims.lock
  - Rebuilt claims index from log
```

### Validation Checks

#### Default Checks

| Check | Description | Auto-Fix |
|-------|-------------|----------|
| Lock files | Detects orphaned locks from crashed processes | ✓ Removes stale locks |
| Claims index | Verifies index matches audit log | ✓ Rebuilds from log |
| Sequence gaps | Detects missing entries in logs | Reports only |
| Schema version | Verifies compatible data format | Reports only |

#### With `--leases`

| Check | Description | Auto-Fix |
|-------|-------------|----------|
| Expired leases | Finds leases past expiration | ✓ Evicts expired |
| Stale indefinite | Finds TTL=0 leases without recent heartbeat | Reports only |
| Ownership | Verifies lease metadata consistency | Reports only |

#### With `--divergence`

| Check | Description | Auto-Fix |
|-------|-------------|----------|
| Main history | Verifies branch includes origin/main | Reports only |
| Global config | Warns if editing global config while diverged | Reports only |

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Validation passed (or all issues fixed) |
| 1 | Validation failed with issues |
| 1 | --dry-run showed issues that would be fixed |

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

Runs all automatic recovery routines in sequence:
1. Clean up stale lock files
2. Rebuild corrupted indexes
3. Evict expired leases

This is equivalent to `jit validate --fix` but with less verbose output.

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
✓ Recovery complete
  - Cleaned 1 stale lock
  - Rebuilt claims index
  - Evicted 2 expired leases
```

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Recovery successful |
| 1 | Recovery failed |

---

## See Also

- [Troubleshooting Guide](../how-to/troubleshooting.md) - Common issues and solutions
- [jit claim Reference](claim.md) - Lease management commands
- [Configuration Reference](configuration.md) - Worktree settings
- [Parallel Work Tutorial](../tutorials/parallel-work-worktrees.md) - Getting started
