# jit claim Command Reference

> **Diátaxis Type:** Reference  
> **Last Updated:** 2026-02-02

Complete CLI reference for `jit claim` subcommands used in lease-based claim coordination.

## Overview

The `jit claim` command family manages exclusive advisory leases on issues for
parallel work coordination. Lease acquisition is serialized by the claims
coordinator, but a lease does not itself prevent writes: whether selected write
commands require one is controlled by `[worktree].enforce_leases` (and `off`
does not block writes).

```bash
jit claim <COMMAND> [OPTIONS]
```

### Global Options

| Option | Description |
|--------|-------------|
| `-q, --quiet` | Suppress non-essential output (for scripting) |
| `-h, --help` | Print help information |

---

## jit claim acquire

Acquire an exclusive lease on an issue.

### Synopsis

```bash
jit claim acquire [OPTIONS] <ISSUE_ID>
```

### Description

Acquires an exclusive advisory lease to work on an issue. Only one agent can
hold that lease at a time; a finite lease expires unless renewed. On success,
the command also records the resolved agent as the issue assignee, but it does
not change the issue lifecycle state.

To begin lifecycle work on a ready issue, run `jit issue claim <issue-id>
<same-agent>` after acquisition. That same-assignee command runs prechecks and
promotes `ready` to `in_progress`; it is not a second lease acquisition. Ordinary
`jit issue claim` is assignee bookkeeping and lifecycle promotion, not atomic
multi-agent coordination.

### Arguments

| Argument | Description |
|----------|-------------|
| `<ISSUE_ID>` | Issue ID to claim (short or full UUID) |

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--ttl <SECONDS>` | See [runtime defaults](runtime-defaults.md) | Time-to-live in seconds. Use `0` for indefinite lease (requires `--reason`) |
| `--agent-id <ID>` | From config | Override agent identifier |
| `--reason <TEXT>` | None | Reason for claim (required for TTL=0) |
| `--json` | false | Output as JSON |

### Examples

```bash
# Lease with the default TTL
jit claim acquire abc123

# 1-hour lease
jit claim acquire abc123 --ttl 3600

# Indefinite lease (requires reason)
jit claim acquire abc123 --ttl 0 --reason "Manual review required"

# JSON output for scripting
jit claim acquire abc123 --json
```

### Expiry, assignment, and availability

When a later `jit claim acquire` finds an expired finite lease, the coordinator
evicts it before deciding whether a new lease can be acquired. Lease expiry does
not clear the issue's assignee, however. `jit query available` selects only
ready, unassigned issues, so release or unassign the issue separately before
expecting it in that query:

```bash
jit claim acquire abc123 --agent-id agent:worker-2
jit issue release abc123 "return to the available pool"
# Or: jit issue unassign abc123
jit query available
```

### Git requirement

Lease commands need a Git repository with a resolvable `HEAD` for worktree
identity and branch tracking. This does not disable filesystem-backed document
operations: `jit doc add`, `jit doc list`, `jit doc archive`, and working-tree
document reads work without Git. Document history, diffs, and commit-specific
reads require Git.

### Failure modes

Acquisition fails when another agent already holds the lease, when `--ttl 0` is
given without `--reason`, when the agent is at its indefinite-lease limit, and
when the issue does not exist. For the code each failure exits with, see the
[Exit Codes reference](exit-codes.md).

### Policy Limits (TTL=0)

Indefinite leases have policy limits to prevent deadlocks:

- **Per-agent limit:** Max 2 indefinite leases per agent (configurable)
- **Per-repo limit:** Max 10 indefinite leases per repository (configurable)

Configure in `.jit/config.toml`:

```toml
[coordination]
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
```

---

## jit claim release

Release the active lease on an issue, by issue id.

### Synopsis

```bash
jit claim release [OPTIONS] <ISSUE_ID>
```

### Description

Resolves the issue's active lease and releases it **without** requiring the lease
UUID, making that lease immediately acquirable by another agent. It does not
clear the issue assignee; use `jit issue release` or `jit issue unassign` when
the issue should again qualify as unassigned work.

Release succeeds **regardless of which agent owns the lease** (it reuses the
force-evict path). The release requires an acting identity: it is resolved from
`JIT_AGENT_ID` / `~/.config/jit/agent.toml`, falling back to the git `user.name`
(sanitized to `human:<name>` with whitespace collapsed to `-`). This identity is
recorded in the eviction audit trail (`claims.jsonl`) so every release is
attributable. If neither an agent id nor a git `user.name` is available, the
command **errors** rather than fabricating an identity.

If the issue has no active lease, the command fails with an actionable
"no active lease ... (not found)" error.

### Arguments

| Argument | Description |
|----------|-------------|
| `<ISSUE_ID>` | Issue ID whose active lease should be released (short ids accepted) |

### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON (`lease_id`, `issue_id`, `previous_owner`, `actor`, `message`) |

### Examples

```bash
# Release whatever lease is active on issue abc123 (any owner)
jit claim release abc123

# JSON output
jit claim release abc123 --json
```

### Failure modes

Release fails when no acting identity is available, when the issue does not
exist, and when the issue carries no active lease. For the code each failure
exits with, see the [Exit Codes reference](exit-codes.md).

---

## jit claim renew

Extend the expiry time of an existing lease.

### Synopsis

```bash
jit claim renew [OPTIONS] <LEASE_ID>
```

### Description

Extends the expiry time of an existing lease. For finite leases, adds time to the expiration. For indefinite leases (TTL=0), updates the `last_beat` timestamp.

### Arguments

| Argument | Description |
|----------|-------------|
| `<LEASE_ID>` | Lease ID to renew |

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--extension <SECONDS>` | See [runtime defaults](runtime-defaults.md) | Seconds to extend the lease |
| `--json` | false | Output as JSON |

### Examples

```bash
# Extend by the default TTL
jit claim renew abc12345-6789-...

# Extend by 1 hour
jit claim renew abc12345-6789-... --extension 3600

# JSON output
jit claim renew abc12345-6789-... --json
```

### Failure modes

Renewal fails when the lease belongs to a different owner and when the lease
does not exist. For the code each failure exits with, see the
[Exit Codes reference](exit-codes.md).

---

## jit claim heartbeat

Send heartbeat for an indefinite lease to prevent staleness.

### Synopsis

```bash
jit claim heartbeat [OPTIONS] <LEASE_ID>
```

### Description

Updates the `last_beat` timestamp for an indefinite (TTL=0) lease without changing expiration. This signals that the agent is still actively working on the issue. Indefinite leases become stale after a hardcoded one-hour threshold without a heartbeat.

### Arguments

| Argument | Description |
|----------|-------------|
| `<LEASE_ID>` | Lease ID to heartbeat |

### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

### Examples

```bash
# Send heartbeat
jit claim heartbeat abc12345-6789-...

# JSON output
jit claim heartbeat abc12345-6789-... --json
```

### Staleness

Indefinite leases become **stale** when:

$$\mathrm{now} - \mathrm{last\_beat} > 3600\ \mathrm{seconds}$$

The one-hour threshold is hardcoded. The `[coordination].stale_threshold_secs`
field is accepted and shown by config commands but does not currently change it.

A stale lease is:
- Marked stale in `jit claim status`
- Not counted as active: with `enforce_leases = "strict"` a structural
  operation on the issue is blocked as if no lease were held
- A candidate for `jit claim force-evict`

Staleness never auto-evicts the lease. It persists until a heartbeat,
`jit claim release`, or force-eviction.

### Failure modes

A heartbeat fails when the lease belongs to a different owner and when the lease
does not exist. For the code each failure exits with, see the
[Exit Codes reference](exit-codes.md).

---

## jit claim status

Show active lease status with optional filtering.

### Synopsis

```bash
jit claim status [OPTIONS]
```

### Description

Shows active leases. By default shows leases for the current agent. Use filters to query specific issues or agents.

### Options

| Option | Description |
|--------|-------------|
| `--issue <ID>` | Filter by issue ID |
| `--agent <ID>` | Filter by agent ID (format: `type:identifier`) |
| `--json` | Output as JSON |

### Examples

```bash
# Show my leases
jit claim status

# Check who has a specific issue
jit claim status --issue abc123

# Show all leases for an agent
jit claim status --agent agent:copilot-1

# JSON output
jit claim status --json
```

### Output

For finite leases:
```
Lease: abc12345-6789-...
  Issue:    01ABC123
  Agent:    agent:copilot-1
  Worktree: wt:a1b2c3
  Branch:   feature/work
  Acquired: 2026-02-02T17:00:00Z
  Expires:  2026-02-02T17:10:00Z (300 seconds remaining)
```

For indefinite leases:
```
Lease: abc12345-6789-...
  Issue:    01ABC123
  Agent:    agent:copilot-1
  Worktree: wt:a1b2c3
  Branch:   feature/work
  Acquired: 2026-02-02T17:00:00Z
  TTL:      indefinite
  Last beat: 2026-02-02T17:05:00Z (300 seconds ago)
```

For stale indefinite leases:
```
  ⚠️  STALE: Lease marked stale (no heartbeat for 75 minutes)
     Use 'jit claim heartbeat abc12345-6789-...' to refresh
```

### Failure modes

Finding no leases is a success, not a failure. The command fails only when the
lease query itself cannot run. For the code that failure exits with, see the
[Exit Codes reference](exit-codes.md).

---

## jit claim list

List all active leases across all agents.

### Synopsis

```bash
jit claim list [OPTIONS]
```

### Description

Shows all active leases across all agents and worktrees. Useful for seeing global state of who is working on what.

### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

### Examples

```bash
# List all leases
jit claim list

# JSON output
jit claim list --json
```

### Failure modes

The command fails only when the lease index cannot be read. For the code that
failure exits with, see the [Exit Codes reference](exit-codes.md).

---

## jit claim force-evict

Force-evict a lease (administrative operation).

### Synopsis

```bash
jit claim force-evict [OPTIONS] --reason <REASON> <LEASE_ID>
```

### Description

Removes a lease immediately regardless of ownership. This is an administrative operation for handling crashed agents or emergency situations. The eviction is logged with the provided reason for audit trail.

### Arguments

| Argument | Description |
|----------|-------------|
| `<LEASE_ID>` | Lease ID to evict |

### Options

| Option | Required | Description |
|--------|----------|-------------|
| `--reason <TEXT>` | Yes | Reason for eviction (for audit trail) |
| `--json` | No | Output as JSON |

### Examples

```bash
# Evict a stale lease from crashed agent
jit claim force-evict abc12345-6789-... --reason "Agent crashed, no heartbeat for 2 hours"

# Emergency override
jit claim force-evict abc12345-6789-... --reason "Emergency: blocking deployment"

# JSON output
jit claim force-evict abc12345-6789-... --reason "Stale" --json
```

### Audit Trail

Force-evictions are logged to the claims audit log with:
- Evicted lease ID
- Reason provided
- Timestamp
- Who performed the eviction

### Failure modes

Eviction fails when `--reason` is omitted and when the lease does not exist. For
the code each failure exits with, see the [Exit Codes reference](exit-codes.md).

---

## See Also

- [Configuration Reference](configuration.md) - Coordination settings
- [Multi-Agent Coordination How-To](../how-to/multi-agent-coordination.md) - Usage patterns
- [Parallel Work Tutorial](../tutorials/parallel-work-worktrees.md) - Getting started
- [Troubleshooting Guide](../how-to/troubleshooting.md) - Common issues
