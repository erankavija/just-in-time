# jit claim Command Reference

> **Diátaxis Type:** Reference  
> **Last Updated:** 2026-02-02

Complete CLI reference for `jit claim` subcommands used in lease-based claim coordination.

## Overview

The `jit claim` command family manages exclusive leases on issues for parallel work coordination. Leases prevent conflicting edits when multiple agents work simultaneously.

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

Acquires an exclusive lease to work on an issue. Only one agent can hold a lease on an issue at a time, preventing conflicting edits. The lease automatically expires after the TTL unless renewed.

### Arguments

| Argument | Description |
|----------|-------------|
| `<ISSUE_ID>` | Issue ID to claim (short or full UUID) |

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--ttl <SECONDS>` | `600` | Time-to-live in seconds. Use `0` for indefinite lease (requires `--reason`) |
| `--agent-id <ID>` | From config | Override agent identifier |
| `--reason <TEXT>` | None | Reason for claim (required for TTL=0) |
| `--json` | false | Output as JSON |

### Examples

```bash
# Standard 10-minute lease
jit claim acquire abc123

# 1-hour lease
jit claim acquire abc123 --ttl 3600

# Indefinite lease (requires reason)
jit claim acquire abc123 --ttl 0 --reason "Manual review required"

# JSON output for scripting
jit claim acquire abc123 --json
```

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Lease acquired successfully |
| 1 | Issue already claimed by another agent |
| 1 | Issue not found |
| 1 | TTL=0 without required --reason |
| 1 | Exceeded indefinite lease limits |

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

Explicitly release a lease before expiration.

### Synopsis

```bash
jit claim release [OPTIONS] <LEASE_ID>
```

### Description

Releases a lease before it expires, making the issue immediately available for other agents to claim. Only the lease owner can release it.

### Arguments

| Argument | Description |
|----------|-------------|
| `<LEASE_ID>` | Lease ID to release (UUID from acquire) |

### Options

| Option | Description |
|--------|-------------|
| `--json` | Output as JSON |

### Examples

```bash
# Release a lease
jit claim release abc12345-6789-0123-4567-89abcdef0123

# JSON output
jit claim release abc12345-6789-... --json
```

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Lease released successfully |
| 1 | Lease not found |
| 1 | Not authorized (different owner) |

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
| `--extension <SECONDS>` | `600` | Seconds to extend the lease |
| `--json` | false | Output as JSON |

### Examples

```bash
# Extend by 10 minutes (default)
jit claim renew abc12345-6789-...

# Extend by 1 hour
jit claim renew abc12345-6789-... --extension 3600

# JSON output
jit claim renew abc12345-6789-... --json
```

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Lease renewed successfully |
| 1 | Lease not found |
| 1 | Not authorized (different owner) |

---

## jit claim heartbeat

Send heartbeat for an indefinite lease to prevent staleness.

### Synopsis

```bash
jit claim heartbeat [OPTIONS] <LEASE_ID>
```

### Description

Updates the `last_beat` timestamp for an indefinite (TTL=0) lease without changing expiration. This signals that the agent is still actively working on the issue. Leases become stale after the configured threshold (default: 1 hour) without heartbeat.

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

```
now - last_beat > stale_threshold_secs (default: 3600)
```

Stale leases are:
- Highlighted in `jit claim status`
- Rejected by pre-commit hooks (in strict mode)
- Candidates for force-eviction

Configure threshold in `.jit/config.toml`:

```toml
[coordination]
stale_threshold_secs = 3600  # 1 hour
```

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Heartbeat sent successfully |
| 1 | Lease not found |
| 1 | Not authorized (different owner) |

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

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success (even if no leases found) |
| 1 | Error querying leases |

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

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Error reading lease index |

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

### Exit Codes

| Code | Description |
|------|-------------|
| 0 | Lease evicted successfully |
| 1 | Lease not found |
| 1 | Missing required --reason |

---

## See Also

- [Configuration Reference](configuration.md) - Coordination settings
- [Multi-Agent Coordination How-To](../how-to/multi-agent-coordination.md) - Usage patterns
- [Parallel Work Tutorial](../tutorials/parallel-work-worktrees.md) - Getting started
- [Troubleshooting Guide](../how-to/troubleshooting.md) - Common issues
