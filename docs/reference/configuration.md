# Configuration Reference

> **Diátaxis Type:** Reference  
> **Last Updated:** 2026-02-02

Complete reference for jit configuration options.

## Configuration Sources (Priority Order)

Configuration is loaded from multiple sources, with later sources overriding earlier ones:

1. **Hardcoded defaults** (lowest priority)
2. **System config** — `/etc/jit/config.toml`
3. **User config** — `~/.config/jit/config.toml`
4. **Repository config** — `.jit/config.toml`
5. **Environment variables** (highest priority)

## Repository Config (`.jit/config.toml`)

### `[worktree]` Section

Controls worktree-aware features and lease enforcement.

```toml
[worktree]
mode = "auto"              # Worktree mode
enforce_leases = "strict"  # Lease enforcement level
```

#### `mode`

| Value | Description |
|-------|-------------|
| `"auto"` | Enable worktree features when git worktrees detected (default) |
| `"on"` | Always enable worktree features |
| `"off"` | Disable worktree features entirely |

**Environment override:** `JIT_WORKTREE_MODE`

#### `enforce_leases`

| Value | Description |
|-------|-------------|
| `"strict"` | Require lease for write operations, fail without one (default) |
| `"warn"` | Warn if no lease but allow operation |
| `"off"` | No lease enforcement |

**Environment override:** `JIT_ENFORCE_LEASES`

### `[coordination]` Section

Settings for claim coordination between agents.

```toml
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
lease_renewal_threshold_pct = 10
stale_threshold_secs = 3600
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
auto_renew_leases = false
```

#### `default_ttl_secs`

Default lease time-to-live in seconds. Default: `600` (10 minutes).

#### `heartbeat_interval_secs`

How often to send heartbeats. Default: `30` seconds.

#### `lease_renewal_threshold_pct`

Renew lease when this percentage of TTL remains. Default: `10`.

#### `stale_threshold_secs`

Consider a lease stale after this many seconds without heartbeat. Default: `3600` (1 hour).

#### `max_indefinite_leases_per_agent`

Maximum indefinite (TTL=0) leases per agent. Default: `2`.

#### `max_indefinite_leases_per_repo`

Maximum indefinite leases across entire repository. Default: `10`.

#### `auto_renew_leases`

Automatically renew leases before expiration. Default: `false`.

### `[global_operations]` Section

Controls repository-wide operations and safety checks.

```toml
[global_operations]
require_main_history = true
allowed_branches = ["main"]
```

#### `require_main_history`

Require that worktree branches include main branch history. Default: `true`.

#### `allowed_branches`

Branches allowed for global operations. Default: `["main"]`.

### `[locks]` Section

File lock behavior for atomic operations.

```toml
[locks]
max_age_secs = 3600
enable_metadata = true
```

#### `max_age_secs`

Maximum age of a lock file before considered stale. Default: `3600` (1 hour).

#### `enable_metadata`

Store lock metadata (PID, agent, timestamp) for diagnostics. Default: `true`.

### `[events]` Section

Event logging configuration.

```toml
[events]
enable_sequences = true
use_unified_envelope = true
```

#### `enable_sequences`

Include sequence numbers in events for ordering. Default: `true`.

#### `use_unified_envelope`

Use unified event envelope format. Default: `true`.

## Agent Config (`~/.config/jit/agent.toml`)

Agent-specific configuration for persistent identity.

```toml
[agent]
id = "agent:my-agent"
created_at = "2026-01-01T00:00:00Z"
description = "My development agent"
default_ttl_secs = 900

[behavior]
auto_heartbeat = true
heartbeat_interval = 30
```

### `[agent]` Section

#### `id`

Agent identity in `type:identifier` format. Examples:
- `agent:copilot-1`
- `human:alice`
- `ci:github-actions`

**Environment override:** `JIT_AGENT_ID`

#### `created_at`

ISO 8601 timestamp when this agent config was created.

#### `description`

Human-readable description of this agent.

#### `default_ttl_secs`

Default TTL for claims made by this agent. Overrides repository default.

### `[behavior]` Section

#### `auto_heartbeat`

Automatically send heartbeats while working. Default: `true`.

#### `heartbeat_interval`

Seconds between heartbeats. Default: `30`.

## Environment Variables

| Variable | Description | Valid Values |
|----------|-------------|--------------|
| `JIT_AGENT_ID` | Agent identity | `type:identifier` |
| `JIT_WORKTREE_MODE` | Override worktree mode | `auto`, `on`, `off` |
| `JIT_ENFORCE_LEASES` | Override lease enforcement | `strict`, `warn`, `off` |

## Config Commands

### Show Effective Config

```bash
jit config show
```

Displays the merged configuration from all sources.

### Get Single Value

```bash
jit config get worktree.mode
jit config get coordination.default_ttl_secs
```

### Set Value

```bash
# Set in repository config
jit config set worktree.mode on

# Set in user config
jit config set worktree.mode on --global
```

### Validate Config

```bash
jit config validate
```

Exit codes:
- `0` — Valid configuration
- `1` — Errors found
- `2` — Warnings only

## Example Configurations

### Single Agent (Default)

No configuration needed — defaults work for single-agent workflows.

### Multi-Agent Team

`.jit/config.toml`:
```toml
[worktree]
mode = "on"
enforce_leases = "strict"

[coordination]
default_ttl_secs = 1800  # 30 minutes for longer tasks
stale_threshold_secs = 7200
```

### CI/CD Environment

`.jit/config.toml`:
```toml
[worktree]
mode = "off"  # CI doesn't use worktrees
enforce_leases = "off"

[coordination]
auto_renew_leases = true  # Keep leases alive during long builds
```

Agent config in CI:
```bash
export JIT_AGENT_ID=ci:github-actions-${{ github.run_id }}
```

### Relaxed Development

For solo development with optional coordination:

```toml
[worktree]
mode = "auto"
enforce_leases = "warn"  # Warn but don't block
```

## See Also

- [Example config.toml](./example-config.toml)
- [Tutorial: Parallel Work](../tutorials/parallel-work-worktrees.md)
- [How-to: Multi-Agent Coordination](../how-to/multi-agent-coordination.md)
