# Configuration Reference

> **Diátaxis Type:** Reference

Complete reference for JIT configuration options.

**Quick links:**
- [Example config.toml](example-config.toml) - Full annotated example with all options
- [Schema Configuration](#schema-configuration) - Issue types, validation, namespaces
- [Runtime Configuration](#runtime-configuration) - Worktrees, coordination, locks

## Configuration Files

| File | Purpose |
|------|---------|
| `.jit/config.toml` | Repository config (schema + runtime) |
| `~/.config/jit/config.toml` | User defaults |
| `~/.config/jit/agent.toml` | Agent identity |
| `/etc/jit/config.toml` | System defaults |

Priority: environment variables > repository > user > system > hardcoded defaults.

---

## Schema Configuration

These settings define how issues are organized and validated. See [example-config.toml](example-config.toml) for full annotated examples.

### `[version]`

```toml
[version]
schema = 2
```

Schema version. Required for newer features like namespace registry and documentation lifecycle.

### `[documentation]`

```toml
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
session = "sessions"
study = "studies"
```

Controls document lifecycle management. Documents in `managed_paths` can be archived; documents in `permanent_paths` never archive.

### `[type_hierarchy]`

```toml
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4, bug = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
story = "story"
```

| Field | Description |
|-------|-------------|
| `types` | Type name → hierarchy level (lower = more strategic) |
| `strategic_types` | Types shown in `jit query strategic` |
| `label_associations` | Type → membership label namespace mapping |

### `[validation]`

```toml
[validation]
strictness = "loose"
default_type = "task"
require_type_label = false
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'
reject_malformed_labels = false
enforce_namespace_registry = false
warn_orphaned_leaves = true
warn_strategic_consistency = true
```

| Field | Description |
|-------|-------------|
| `strictness` | `"strict"`, `"loose"`, or `"permissive"` |
| `default_type` | Auto-assign when no type:* label |
| `warn_orphaned_leaves` | Warn when tasks lack parent labels |

### `[namespaces.*]`

```toml
[namespaces.epic]
description = "Epic membership"
unique = false
examples = ["epic:auth", "epic:docs"]
```

Define label namespaces for documentation and optional enforcement. Set `enforce_namespace_registry = true` to require defined namespaces only.

---

## Runtime Configuration

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

See [example-config.toml](example-config.toml) for a complete annotated template.

**Common patterns:**

| Use Case | Key Settings |
|----------|--------------|
| Single agent | Defaults work, no config needed |
| Multi-agent team | `enforce_leases = "strict"`, longer TTL |
| CI/CD | `worktree.mode = "off"`, `JIT_AGENT_ID` env var |
| Solo dev | `enforce_leases = "warn"` for flexibility |

## See Also

- [Example config.toml](example-config.toml) - Full annotated configuration
- [Tutorial: Parallel Work](../tutorials/parallel-work-worktrees.md)
- [How-to: Multi-Agent Coordination](../how-to/multi-agent-coordination.md)
