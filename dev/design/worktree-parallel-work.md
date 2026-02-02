# Parallel Multi-Agent Work with Git Worktrees

**Issue:** ad601a15-5217-439f-9b3c-94a52c06c18b  
**Status:** Design  
**Author:** System  
**Date:** 2026-01-03  
**Last Updated:** 2026-01-04

**Recent refinements:**
- **2026-01-04:** Added critical design sections from session review:
  - "Read vs Write Operations" - explicit lease requirements and data access patterns
  - "Issue Creation Workflow" - resolves chicken-and-egg problem (create doesn't require lease)
  - "Query Behavior in Worktrees" - local vs global queries, coordination-awareness
- **2026-01-03:** Reorganized for clarity: goals → architecture → identities → lease semantics → atomicity/durability → enforcement → recovery → configuration → testing → future work
- Emphasized deterministic, race-free behavior with monotonic time semantics and sequence numbers
- Enhanced atomicity with parent directory fsync after atomic renames
- Strengthened enforcement with CLI-level lease checks, divergence gating, and comprehensive hook examples
- Added configuration toggles for enforcement modes and worktree behavior
- Expanded guidance on long-running tasks, lease transfers, and integration points

## Problem Statement

Current `.jit/` storage is repository-local but not worktree-aware. Multiple agents working on the same machine cannot operate in parallel because they share the same `.jit/` directory, leading to file conflicts and race conditions.

**Key limitations:**
- Shared `.jit/issues/` causes concurrent write conflicts
- No coordination mechanism for issue ownership
- No visibility into which agent is working on what
- Manual deconfliction required (inefficient)
- Risk of race conditions and non-deterministic behavior

## Goals

1. **Enable true parallelism**: Multiple agents work on different issues simultaneously on the same machine
2. **Maintain safety**: Prevent conflicting edits through lease-based coordination
3. **Preserve simplicity**: Keep storage plaintext JSON, git-versionable
4. **Support recovery**: Handle crashes and stale claims gracefully
5. **Audit trail**: All coordination actions logged for observability
6. **Deterministic behavior**: Race-free, monotonic, and predictable operations
7. **Correctness-first**: Strong invariants with defense-in-depth enforcement

## Architecture

### Two-Tier Storage Model

This design separates data-plane (versioned issue data) from control-plane (ephemeral coordination state).

**Per-Worktree Data Plane** (`<worktree>/.jit/`):
- Issue data: `issues/<ID>/issue.json`
- Local gates: `gates/results/<issue-id>.json`
- Local events: `events/local.jsonl`
- Local cache: `cache/index.sqlite` (optional)

**Shared Control Plane** (`.git/jit/`):
- Claims registry: `claims.jsonl` (append-only audit log)
- Claims index: `claims.index.json` (derived current view)
- Locks: `locks/claims.lock` (atomic coordination)
- Heartbeats: `heartbeat/<agent-id>.json`
- Control events: `events/control.jsonl` (claim/release/evict)

### Directory Layout

```
project-root/
├── .git/
│   └── jit/                      # Shared control plane (local only)
│       ├── locks/
│       │   ├── claims.lock       # Global claim coordination lock
│       │   └── scope.lock        # Global operation lock (config, gates)
│       ├── claims.jsonl          # Append-only claim audit log
│       ├── claims.index.json     # Derived active leases view
│       ├── heartbeat/
│       │   ├── agent:copilot-1.json
│       │   └── agent:copilot-2.json
│       └── events/
│           └── control.jsonl     # Control-plane events (claims)
│
├── .jit/                         # Main worktree data plane
│   ├── worktree.json             # Worktree identity
│   ├── issues/
│   ├── gates/
│   └── events/
│
└── worktrees/
    ├── feature-a/
    │   └── .jit/                 # Isolated worktree data plane
    │       ├── worktree.json     # wt:abc123
    │       ├── issues/           # Only issues claimed by this worktree
    │       ├── gates/
    │       └── events/
    │
    └── feature-b/
        └── .jit/                 # Isolated worktree data plane
            ├── worktree.json     # wt:def456
            ├── issues/
            ├── gates/
            └── events/
```

**Design rationale:**
- Per-worktree issue data enables git-based merging and versioning
- Shared control plane provides global coordination without network dependency
- Append-only logs ensure audit trail and enable index rebuild
- Atomic operations via file locks guarantee consistency

### Key Principles

1. **Correctness**: Strong invariants enforced at multiple layers (CLI, hooks, locks)
2. **Determinism**: Monotonic clocks, sequence numbers, predictable ordering
3. **Durability**: fsync after critical writes, atomic rename pattern
4. **Recovery**: Self-healing from corrupted indices, expired leases, stale locks
5. **Defense-in-depth**: CLI-level checks before hooks before commit

## Identity System

### Agent Identity

**Format:** `{type}:{identifier}`

**Examples:**
- `agent:copilot-1` - GitHub Copilot session 1
- `agent:cursor-main` - Cursor editor instance
- `human:alice` - Human user Alice
- `ci:github-actions` - CI/CD pipeline

**Persistence:**
- Environment variable: `JIT_AGENT_ID` (highest priority, session-specific)
- Config file: `~/.config/jit/agent.toml` (persistent identity)
- Per-session override: `--agent-id` flag (explicit override)

**Provenance (agent.toml):**
```toml
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"
description = "GitHub Copilot Workspace Session 1"

# Session override via env or CLI flag
# JIT_AGENT_ID=agent:session-xyz jit claim acquire ...
# jit claim acquire <issue> --agent-id agent:override
```

**Stability:** Agent ID should persist across process restarts for the same logical agent/session to maintain lease continuity.

### Worktree Identity

**Format:** `wt:{short-hash}`

**Generation:**
- Hash of worktree absolute path + creation timestamp
- Truncate to 8 hex characters for readability
- Example: `wt:abc123ef`

**Persistence:**
- Stored in `<worktree>/.jit/worktree.json`
- Created on first `jit` command in worktree
- Immutable once created

**Schema:**
```json
{
  "schema_version": 1,
  "worktree_id": "wt:abc123ef",
  "branch": "agents/copilot-1",
  "root": "/absolute/path/to/worktree",
  "created_at": "2026-01-03T12:00:00Z"
}
```

**Relocation detection:**
```rust
pub fn load_worktree_identity(paths: &WorktreePaths) -> Result<WorktreeIdentity> {
    let wt_file = paths.local_jit.join("worktree.json");
    let mut wt: WorktreeIdentity = serde_json::from_str(&fs::read_to_string(&wt_file)?)?;
    
    // Check if worktree was moved
    let current_root = paths.worktree_root.to_string_lossy().to_string();
    if wt.root != current_root {
        warn!("Worktree relocated: {} -> {}", wt.root, current_root);
        
        // Update location (worktree_id remains stable)
        wt.root = current_root;
        wt.relocated_at = Some(Utc::now());
        
        // Write updated identity atomically
        let temp_path = wt_file.with_extension("tmp");
        fs::write(&temp_path, serde_json::to_string_pretty(&wt)?)?;
        fs::rename(temp_path, &wt_file)?;
    }
    
    Ok(wt)
}
```

**Procedure on relocation:**
1. Detect path change on any `jit` command
2. Update `worktree.json` with new path and `relocated_at` timestamp
3. Preserve `worktree_id` to maintain lease identity
4. Log warning for audit trail

### Lease Identity

**Format:** ULID (Universally Unique Lexicographically Sortable Identifier)

**Example:** `01HXJK2M3N4P5Q6R7S8T9VWXYZ`

**Properties:**
- Sortable by creation time
- Globally unique
- 26 characters (URL-safe Base32)

## Lease Semantics

### Overview

Leases are **time-bounded, exclusive claims** on issues for structural editing. This section defines the lease model, invariants, and operations.

### Lease Model

Claims are **time-bounded leases** with automatic expiration:

**Lease fields:**
```rust
struct Lease {
    lease_id: String,      // ULID
    issue_id: String,      // Issue being claimed
    agent_id: String,      // Who acquired it
    worktree_id: String,   // Where it was acquired
    branch: String,        // Branch being worked on
    ttl_secs: u64,         // Time-to-live
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}
```

**Default TTL:** 600 seconds (10 minutes)

**Monotonic expiry check:** Use `Instant` internally for TTL expiry checks to avoid wall-clock jumps affecting lease validity. Store RFC3339 timestamps for audit trail.

```rust
use std::time::Instant;

pub struct Lease {
    // Monotonic clock for expiry checks (not serialized)
    #[serde(skip)]
    acquired_instant: Instant,
    
    // UTC timestamps for audit trail
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub ttl_secs: u64,
    
    // ... other fields
}

impl Lease {
    pub fn is_expired(&self) -> bool {
        // Use monotonic clock (immune to NTP adjustments, system time changes)
        self.acquired_instant.elapsed().as_secs() > self.ttl_secs
    }
    
    pub fn remaining_secs(&self) -> u64 {
        self.ttl_secs.saturating_sub(self.acquired_instant.elapsed().as_secs())
    }
}
```

### Invariants

**Critical invariants enforced throughout the system:**

1. **One active lease per issue**: At most one lease can be active for any issue at any time
   - Enforced in: `acquire_claim()` under global lock
   - Verified in: `verify_index_consistency()` on startup

2. **Lease ownership**: Only the exact (agent_id, worktree_id) pair can renew or release a lease
   - Prevents agent impersonation
   - Ensures work stays within intended worktree

3. **Expiry determinism**: Expired leases are automatically evicted lazily during any claim operation
   - No background daemon required for expiry
   - Predictable, reproducible behavior

4. **Atomic transitions**: All lease state changes (acquire, renew, release, evict) are atomic
   - Append to log + update index under global lock
   - No partial states observable

5. **Audit completeness**: Every lease operation is logged in append-only `claims.jsonl`
   - Immutable audit trail
   - Enables index rebuild and forensics

### Single-Writer Policy

**Per-issue structural edits require an active lease:**
- Creating/updating/deleting issue
- Adding/removing dependencies
- Adding/removing gates  
- Changing issue state
- Editing labels, assignees, metadata

**CLI-level enforcement (defense-in-depth layer 1):**
```rust
impl CommandExecutor {
    pub fn update_issue(&self, id: &str, params: UpdateParams) -> Result<()> {
        // MUST check lease before operation
        self.require_active_lease(id, "update issue")?;
        
        // Proceed with update
        self.storage.update_issue(id, params)
    }
    
    fn require_active_lease(&self, issue_id: &str, operation: &str) -> Result<()> {
        let config = self.load_config()?;
        
        // Check enforcement mode
        match config.coordination.enforce_leases {
            EnforceMode::Off => return Ok(()), // Bypass
            EnforceMode::Warn => {
                // Check but only warn
                if !self.has_active_lease(issue_id)? {
                    eprintln!("⚠️  Warning: {} on {} without active lease", operation, issue_id);
                }
                return Ok(());
            }
            EnforceMode::Strict => {
                // Strict enforcement (default)
            }
        }
        
        let agent_id = self.get_agent_id()?;
        let worktree_id = self.get_worktree_id()?;
        
        if !self.claim_coordinator.has_active_lease(issue_id, &agent_id, &worktree_id)? {
            bail!(
                "Operation '{}' on issue {} requires active lease.\n\
                 Agent: {}, Worktree: {}\n\
                 Acquire: jit claim acquire {} --ttl 600",
                operation, issue_id, agent_id, worktree_id, issue_id
            );
        }
        
        Ok(())
    }
}
```

**Multi-writer allowed (future):**
- Free-text description edits (Automerge CRDT)
- Comment additions (append-only)

### Read vs Write Operations

**CRITICAL DISTINCTION:** Leases control write access, not read access. All issue data remains readable from any worktree regardless of claims.

#### Operations That Require Active Lease (Writes)

These operations modify issue state and require an active, non-stale lease:

**Issue modifications:**
- `jit issue update <id> --state <state>` - Change issue state
- `jit issue update <id> --priority <priority>` - Change priority
- `jit issue update <id> --label <label>` - Add/remove labels
- `jit issue update <id> --assignee <assignee>` - Change assignee
- `jit issue delete <id>` - Delete issue

**Dependency modifications:**
- `jit dep add <from> <to>` - Add dependency
- `jit dep rm <from> <to>` - Remove dependency

**Gate modifications:**
- `jit gate add <id> <gate-key>` - Add gate requirement
- `jit gate pass <id> <gate-key>` - Mark gate as passed
- `jit gate fail <id> <gate-key>` - Mark gate as failed

**Document modifications (when linked to issue):**
- Editing documents attached to claimed issue

#### Operations That DO NOT Require Lease (Reads)

These operations access data without modifying issue state and work from any worktree:

**Issue inspection:**
- `jit issue show <id>` - View any issue details
- `jit query all` - List issues (see "Query Behavior" below)
- `jit query available` - Query ready issues (coordination-aware)
- `jit query blocked` - Query blocked issues
- `jit query all --state <state>` - Query by state
- `jit query all --assignee <assignee>` - Query by assignee
- `jit search <query>` - Search across all issues

**Graph operations:**
- `jit graph show <id>` - View dependency tree for any issue
- `jit graph downstream <id>` - View dependents
- `jit graph roots` - View root issues
- `jit graph export` - Export full graph

**Configuration and registry:**
- `jit gate list` - List gate definitions
- `jit gate show <key>` - View gate details
- `jit config show-hierarchy` - View type hierarchy
- `jit status` - Repository status

**Coordination state:**
- `jit claim status` - View all active leases
- `jit claim status --issue <id>` - Check who has an issue
- `jit claim status --agent <agent-id>` - Check agent's leases

**Document operations (read-only):**
- `jit doc show <issue-id> <path>` - View document content
- `jit doc list <issue-id>` - List documents for issue
- `jit doc history <issue-id> <path>` - View document history

#### Issue Creation Workflow

**Issue creation is exempted from lease requirement** because it allocates a new ID that cannot conflict:

```bash
# Create new issue (no lease required, allocates fresh ID)
jit issue create --title "New feature" --priority high
# => Issue 01ABC created with state: ready

# Optionally claim to work on it
jit claim acquire 01ABC --ttl 600
# => Lease acquired: lease-xyz

# Work on issue
jit issue update 01ABC --state in-progress
```

**Rationale:** 
- New issue creation allocates a unique ID (ULID)
- No conflict possible with concurrent creates (different ULIDs)
- Issue becomes claimable immediately after creation
- Works from any worktree (main or secondary)

**Alternative workflow (create-and-claim):**
```bash
# Create and immediately claim in single operation (future)
jit claim create --title "Fix bug" --ttl 600
# => Issue 01DEF created, lease acquired
```

#### Data Access Patterns

Reads access issue data through a layered resolution strategy:

**Resolution order:**
1. **Git HEAD** - Committed issue state (canonical for merged work)
2. **Main `.jit/`** - Uncommitted issues in main worktree (fallback)
3. **Local `.jit/`** - Write copies of claimed issues in secondary worktree
4. **Claims index** - Real-time coordination state (`.git/jit/claims.index.json`)

**Example - Cross-worktree dependency checking:**
```bash
# Agent-1 in worktree-1 (working on issue-A)
# issue-A depends on issue-B (claimed by agent-2 in worktree-2)

jit graph show issue-A           # ✅ Shows full tree including issue-B
jit issue show issue-B           # ✅ Read issue-B details (via git)
jit claim status --issue issue-B # ✅ See agent-2 has lease on issue-B
jit query blocked                # ✅ Check if issue-A blocked by issue-B
```

**Key insight:** Worktrees contain **write copies** of claimed issues, but all issues remain **readable** from any worktree via git.

### Query Behavior in Worktrees

Different query commands behave differently based on context and coordination-awareness.

#### Local Queries (Worktree-Scoped)

`jit query all` shows issues in the local `.jit/issues/` directory:

**Main worktree:**
```bash
jit query all
# Shows: All issues in .jit/issues/ (both claimed and unclaimed)
```

**Secondary worktree:**
```bash
jit query all
# Shows: Only issues claimed by this worktree (write copies)
```

**Rationale:** Reflects the actual file structure in each worktree's data plane.

#### Global Queries (Coordination-Aware)

These queries aggregate across all worktrees and respect coordination state:

**Ready issues (unclaimed and unblocked):**
```bash
jit query available
# Filters out:
# - Issues with active leases (from claims index)
# - Issues blocked by dependencies
# - Issues in non-ready states
# Works from any worktree
```

**Blocked issues:**
```bash
jit query blocked
# Shows: All blocked issues with blocking reasons
# Reads: Dependencies from git (committed state)
# Works from any worktree
```

**State/priority/assignee queries:**
```bash
jit query all --state in-progress
jit query all --priority high
jit query all --assignee agent:copilot-1
# Aggregates across: git + main .jit/ + claims index
# Works from any worktree
```

**Search (full-text):**
```bash
jit search "authentication"
# Searches: Issue titles, descriptions, metadata
# Scope: git + main .jit/ (uncommitted)
# Works from any worktree
```

#### Direct Issue Access

`jit issue show <id>` works for ANY issue from ANY worktree:

**Access pattern:**
1. Check local `.jit/issues/<id>/` (if claimed by this worktree)
2. Fallback to git HEAD (committed state)
3. Fallback to main `.jit/issues/<id>/` (uncommitted in main)
4. Error if not found anywhere

**Example:**
```bash
# From secondary worktree working on issue-A
jit issue show issue-A  # ✅ Local write copy
jit issue show issue-B  # ✅ Via git (committed)
jit issue show issue-C  # ✅ Via main .jit/ (uncommitted, unclaimed)
jit issue show issue-D  # ❌ Not found (doesn't exist)
```

#### Graph Operations

Dependency graphs aggregate across all sources:

```bash
jit graph show <id>
# Traverses: Full dependency tree
# Reads: Dependencies from git + main .jit/
# Ignores: Worktree boundaries
# Shows: Complete graph regardless of claims
```

**Coordination-aware labels:**
```bash
jit graph show issue-A --show-claims
# Output:
# issue-A (claimed by agent:copilot-1)
# ├─ issue-B (claimed by agent:copilot-2)
# └─ issue-C (unclaimed)
```

#### Summary Table

| Command | Scope | Coordination-Aware | Works From |
|---------|-------|-------------------|------------|
| `jit query all` | Local `.jit/` only | ❌ No | Any worktree |
| `jit query available` | Global (git + main .jit/) | ✅ Yes (filters claimed) | Any worktree |
| `jit query blocked` | Global | ✅ Yes (checks deps) | Any worktree |
| `jit issue show <id>` | Global (layered resolution) | ❌ No | Any worktree |
| `jit graph show <id>` | Global (full graph) | ✅ Optional (--show-claims) | Any worktree |
| `jit search <query>` | Global (git + main .jit/) | ❌ No | Any worktree |
| `jit claim status` | Control plane (.git/jit/) | ✅ Yes | Any worktree |

### Lease Operations

**Acquire:**
1. Check if issue is unleased or lease expired
2. Create new lease with TTL:
   - `ttl_secs > 0`: finite lease, `expires_at = acquired_at + ttl_secs`
   - `ttl_secs = 0`: indefinite lease, no time-based expiry (see below)
3. Append to `claims.jsonl`
4. Update `claims.index.json`
5. Return lease to caller

**Renew (Heartbeat):**
1. Find active lease by ID
2. For finite leases: extend `expires_at` by TTL
3. For indefinite leases: update `last_beat` timestamp
4. Append renew or heartbeat operation to `claims.jsonl`
5. Update `claims.index.json`

**Release:**
1. Find active lease by ID
2. Append release operation to `claims.jsonl`
3. Remove from `claims.index.json`

**Force Evict:**
1. Admin/operator force-releases a lease
2. Append force-evict operation with reason
3. Remove from `claims.index.json`
4. Log warning event

### Automatic Expiration and Staleness

**On every claim operation:**
1. Load `claims.index.json`
2. For finite leases (`ttl_secs > 0`): filter out expired leases (`now > expires_at`) and, for each, append `auto-evict` to `claims.jsonl`
3. For indefinite leases (`ttl_secs = 0`): mark leases as `stale: true` if `now - last_beat > stale_threshold_secs`
4. Write updated index

**Eviction is lazy:** Only happens during claim operations, not background process (for simplicity).

### Indefinite (TTL=0) Leases

**Semantics:**
- `ttl_secs = 0` means no automatic time-based expiry; `expires_at` is `null` or omitted in the index.
- Indefinite leases still require heartbeats and staleness checks; a stale indefinite lease blocks structural edits until renewed or force-evicted.

**Use cases:**
- Manual oversight of high-risk changes (complex refactors, schema migrations)
- Global operations requiring uninterrupted exclusive control (registry/config/type-hierarchy edits)
- Long-running tasks with human supervision

**Policy and guardrails:**
- TTL=0 requires `--reason` flag when acquiring (mandatory audit trail)
- Repository config must explicitly permit indefinite leases for the requested scope
- Per-agent limit: `max_indefinite_leases_per_agent` (default: 2)
- Per-repository limit: `max_indefinite_leases_per_repo` (default: 10)
- Staleness threshold: configurable via `stale_threshold_secs` (default: 3600s / 1 hour)
- Pre-commit/pre-push hooks reject structural edits when `stale: true`
- Not recommended for routine automated agents; use finite TTL with auto-renew instead

## Atomicity and Durability

### Write-Temp-Rename Pattern

All critical file updates use atomic write-temp-rename to prevent partial writes and ensure crash safety.

**Enhanced atomic write with directory fsync:**
```rust
fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
    let index_path = self.paths.shared_jit.join("claims.index.json");
    let temp_path = index_path.with_extension("tmp");
    
    // Write to temp file
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&temp_path, json)?;
    
    // Fsync temp file content to disk
    let file = File::open(&temp_path)?;
    file.sync_all()?;
    drop(file);
    
    // Atomic rename (replaces target atomically)
    fs::rename(&temp_path, &index_path)?;
    
    // CRITICAL: Fsync parent directory to ensure rename is durable
    // Without this, a crash could lose the directory entry update
    let parent_dir = File::open(index_path.parent().unwrap())?;
    parent_dir.sync_all()?;
    
    Ok(())
}
```

**Apply everywhere:**
- Claims index updates: `claims.index.json`
- Worktree identity: `worktree.json`
- Issue updates: `issues/<id>/issue.json`
- Gate results: `gates/results/<issue-id>.json`

### Append-Only Log Durability

**Claims log append with fsync:**
```rust
fn append_claim_op(&self, op: &ClaimOp) -> Result<()> {
    let log_path = self.paths.shared_jit.join("claims.jsonl");
    
    // Ensure directory exists
    fs::create_dir_all(log_path.parent().unwrap())?;
    
    // Append operation
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    
    let json = serde_json::to_string(op)?;
    writeln!(file, "{}", json)?;
    
    // Fsync to ensure append is durable before index update
    file.sync_all()?;
    
    Ok(())
}
```

**Ordering guarantee:** Log append MUST complete (and fsync) before index update. This ensures the index can always be rebuilt from the log.

### Sequence Numbers for Total Ordering

**Optional monotonic sequence in claims log:**
```json
{"schema_version":1,"sequence":1,"op":"acquire","lease_id":"01HX...","issue_id":"ad601a15",...}
{"schema_version":1,"sequence":2,"op":"renew","lease_id":"01HX...",...}
{"schema_version":1,"sequence":3,"op":"release","lease_id":"01HX...",...}
```

**Benefits:**
- Detects missing log entries (gap in sequence)
- Provides total ordering under global lock
- Enables log compaction and archival

**Implementation:**
```rust
struct ClaimCoordinator {
    next_sequence: AtomicU64, // In-memory counter
}

impl ClaimCoordinator {
    fn append_claim_op(&self, op: &ClaimOp) -> Result<()> {
        let seq = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        
        // Wrap operation with sequence
        let envelope = ClaimOpEnvelope {
            schema_version: 1,
            sequence: seq,
            op: op.clone(),
        };
        
        // Append to log
        // ...
    }
    
    fn rebuild_index_from_log(&self) -> Result<ClaimsIndex> {
        // Read log, verify sequence continuity
        let mut expected_seq = 1;
        for line in BufReader::new(File::open(&log_path)?).lines() {
            let envelope: ClaimOpEnvelope = serde_json::from_str(&line?)?;
            
            if envelope.sequence != expected_seq {
                warn!("Sequence gap: expected {}, got {}", expected_seq, envelope.sequence);
            }
            
            expected_seq = envelope.sequence + 1;
            // Apply operation...
        }
        
        // Update next_sequence
        self.next_sequence.store(expected_seq, Ordering::SeqCst);
        
        Ok(index)
    }
}
```

### Monotonic Time Semantics

**Avoid wall-clock dependencies for expiry:**

**Problem:** System time changes (NTP, manual adjustments) can cause:
- Leases expiring prematurely
- Leases living longer than intended
- Non-deterministic behavior in tests

**Solution:** Dual-clock approach:
- **Store**: RFC3339 timestamps (`acquired_at`, `expires_at`) for human-readable audit trail
- **Check**: `Instant` (monotonic clock) for TTL expiry logic

**Serialization handling:**
```rust
#[derive(Serialize, Deserialize)]
pub struct Lease {
    pub lease_id: String,
    pub issue_id: String,
    pub agent_id: String,
    pub worktree_id: String,
    pub branch: String,
    pub ttl_secs: u64,
    
    // Serialized for audit trail
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    
    // NOT serialized, reconstructed on load
    #[serde(skip)]
    acquired_instant: Option<Instant>,
}

impl Lease {
    pub fn new(...) -> Self {
        let now_utc = Utc::now();
        let now_instant = Instant::now();
        
        Self {
            // ...
            acquired_at: now_utc,
            expires_at: now_utc + Duration::seconds(ttl_secs as i64),
            acquired_instant: Some(now_instant),
        }
    }
    
    pub fn from_index(lease_data: LeaseSerde) -> Self {
        // Reconstruct Instant from UTC timestamp
        // Use conservative approach: assume lease started at load time minus elapsed
        let elapsed_secs = Utc::now()
            .signed_duration_since(lease_data.acquired_at)
            .num_seconds()
            .max(0) as u64;
        
        let acquired_instant = Instant::now()
            .checked_sub(Duration::from_secs(elapsed_secs))
            .unwrap_or_else(Instant::now);
        
        Self {
            acquired_instant: Some(acquired_instant),
            // ... copy other fields
        }
    }
    
    pub fn is_expired(&self) -> bool {
        match self.acquired_instant {
            Some(instant) => instant.elapsed().as_secs() > self.ttl_secs,
            None => {
                // Fallback to wall-clock (less reliable)
                Utc::now() > self.expires_at
            }
        }
    }
}
```

### Lock Hygiene and Metadata

**Lock file metadata for better diagnostics:**

```rust
#[derive(Serialize, Deserialize)]
pub struct LockMetadata {
    pub pid: u32,
    pub agent_id: String,
    pub created_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

impl FileLocker {
    pub fn lock_exclusive_with_metadata(
        &self,
        path: &Path,
        agent_id: &str,
    ) -> Result<LockGuard> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        
        // Write metadata for diagnostics
        let metadata = LockMetadata {
            pid: std::process::id(),
            agent_id: agent_id.to_string(),
            created_at: Utc::now(),
            last_updated: Utc::now(),
        };
        
        let meta_path = path.with_extension("meta");
        fs::write(&meta_path, serde_json::to_string_pretty(&metadata)?)?;
        
        Ok(LockGuard {
            file,
            meta_path: Some(meta_path),
        })
    }
}

pub struct LockGuard {
    file: File,
    meta_path: Option<PathBuf>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        if let Some(ref path) = self.meta_path {
            let _ = fs::remove_file(path);
        }
    }
}
```

**Stale lock cleanup with enhanced checks:**
```rust
pub fn cleanup_stale_locks(&self) -> Result<()> {
    let lock_dir = self.paths.shared_jit.join("locks");
    if !lock_dir.exists() {
        return Ok(());
    }
    
    for entry in fs::read_dir(&lock_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        // Skip metadata files
        if path.extension().map_or(false, |e| e == "meta") {
            continue;
        }
        
        let metadata_path = path.with_extension("meta");
        
        // Try to acquire lock non-blocking
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            match file.try_lock_exclusive() {
                Ok(_guard) => {
                    // Lock acquired => it was stale
                    warn!("Removed stale lock: {}", path.display());
                    drop(_guard);
                    let _ = fs::remove_file(&path);
                    let _ = fs::remove_file(&metadata_path);
                }
                Err(_) => {
                    // Lock held, check metadata
                    if let Ok(meta_json) = fs::read_to_string(&metadata_path) {
                        let meta: LockMetadata = serde_json::from_str(&meta_json)?;
                        
                        // Check if process still exists
                        if !process_exists(meta.pid) {
                            warn!("Removing lock from dead process {}: {}", meta.pid, path.display());
                            // Force remove (process dead, lock should be stale)
                            let _ = fs::remove_file(&path);
                            let _ = fs::remove_file(&metadata_path);
                            continue;
                        }
                        
                        // Check age (1 hour TTL for locks)
                        let age = Utc::now().signed_duration_since(meta.created_at);
                        if age.num_seconds() > 3600 {
                            error!(
                                "Lock very old: {} ({}s, pid={}, agent={})",
                                path.display(),
                                age.num_seconds(),
                                meta.pid,
                                meta.agent_id
                            );
                            // Don't auto-remove if process exists, require manual intervention
                        }
                    }
                }
            }
        }
    }
    
    Ok(())
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    
    // Signal 0 doesn't send a signal but checks if process exists
    kill(Pid::from_raw(pid as i32), Signal::from_c_int(0).ok()).is_ok()
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::PROCESS_QUERY_INFORMATION;
    
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if handle.is_null() {
            false
        } else {
            winapi::um::handleapi::CloseHandle(handle);
            true
        }
    }
}
```

**Lock TTL policy:**
- Maximum lock hold time: 1 hour (detect stuck processes)
- Metadata updated periodically for long operations (optional)
- Auto-cleanup only if process dead
- Manual intervention required for old locks with live process

## Atomic Operations and Locking

### File-Based Locking

Use advisory file locks via `fs4` crate:

```rust
use fs4::FileExt;
use std::fs::OpenOptions;

pub struct FileLocker;

impl FileLocker {
    pub fn lock_exclusive(&self, path: &Path) -> Result<LockGuard> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(LockGuard { file })
    }
}

pub struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
```

### Claim Acquisition Algorithm

**Acquire claim (atomic):**

```rust
pub fn acquire_claim(
    &self,
    issue_id: &str,
    agent_id: &str,
    ttl_secs: u64
) -> Result<Lease> {
    // 1. Acquire exclusive lock
    let lock_path = self.paths.shared_jit.join("locks/claims.lock");
    fs::create_dir_all(lock_path.parent().unwrap())?;
    let _guard = self.locker.lock_exclusive(&lock_path)?;
    
    // 2. Load and evict expired
    let mut index = self.load_claims_index()?;
    self.evict_expired(&mut index)?;
    
    // 3. Check availability
    if let Some(existing) = index.find_active_lease(issue_id) {
        bail!("Issue {} already claimed by {} until {}",
            issue_id, existing.agent_id, existing.expires_at);
    }
    
    // 4. Create new lease
    let worktree_id = self.get_worktree_id()?;
    let branch = self.get_current_branch()?;
    let lease = Lease {
        lease_id: Ulid::new().to_string(),
        issue_id: issue_id.to_string(),
        agent_id: agent_id.to_string(),
        worktree_id,
        branch,
        ttl_secs,
        acquired_at: Utc::now(),
        expires_at: Utc::now() + Duration::seconds(ttl_secs as i64),
    };
    
    // 5. Append to audit log
    self.append_claim_op(&ClaimOp::Acquire(lease.clone()))?;
    
    // 6. Update index atomically
    index.add_lease(lease.clone());
    self.write_index_atomic(&index)?;
    
    // 7. Release lock (via RAII)
    Ok(lease)
}
```

### Atomic Index Updates

**Write-temp-rename pattern:**

```rust
fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
    let index_path = self.paths.shared_jit.join("claims.index.json");
    let temp_path = index_path.with_extension("tmp");
    
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&temp_path, json)?;
    
    // Fsync temp file
    let file = File::open(&temp_path)?;
    file.sync_all()?;
    drop(file);
    
    // Atomic rename
    fs::rename(temp_path, index_path)?;
    
    Ok(())
}
```

## Path Resolution

### Worktree Detection

```rust
use std::process::Command;
use std::path::PathBuf;
use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct WorktreePaths {
    pub common_dir: PathBuf,      // .git (shared)
    pub worktree_root: PathBuf,   // root of current worktree
    pub local_jit: PathBuf,       // <worktree_root>/.jit
    pub shared_jit: PathBuf,      // <common_dir>/jit
}

impl WorktreePaths {
    pub fn detect() -> Result<Self> {
        // Check if in git repo
        let is_repo = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        
        if !is_repo {
            // Not in git repo, use current dir
            let current = env::current_dir()?;
            return Ok(Self {
                common_dir: current.clone(),
                worktree_root: current.clone(),
                local_jit: current.join(".jit"),
                shared_jit: current.join(".jit"),
            });
        }
        
        // Get git common dir (shared .git)
        let common_dir = PathBuf::from(
            String::from_utf8(
                Command::new("git")
                    .args(["rev-parse", "--git-common-dir"])
                    .output()
                    .context("Failed to get git common dir")?
                    .stdout
            )?.trim()
        );
        
        // Get worktree root
        let worktree_root = PathBuf::from(
            String::from_utf8(
                Command::new("git")
                    .args(["rev-parse", "--show-toplevel"])
                    .output()
                    .context("Failed to get worktree root")?
                    .stdout
            )?.trim()
        );
        
        let local_jit = worktree_root.join(".jit");
        let shared_jit = common_dir.join("jit");
        
        Ok(Self { common_dir, worktree_root, local_jit, shared_jit })
    }
    
    pub fn is_worktree(&self) -> bool {
        // If common_dir != <worktree_root>/.git, we're in a worktree
        self.common_dir != self.worktree_root.join(".git")
    }
}
```

### Worktree ID Generation

```rust
use sha2::{Sha256, Digest};

pub fn generate_worktree_id(root: &Path, created_at: DateTime<Utc>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hasher.update(created_at.to_rfc3339().as_bytes());
    let hash = hasher.finalize();
    format!("wt:{}", hex::encode(&hash[..4]))
}

pub fn load_or_create_worktree_id(paths: &WorktreePaths) -> Result<String> {
    let wt_file = paths.local_jit.join("worktree.json");
    
    if wt_file.exists() {
        let content = fs::read_to_string(&wt_file)?;
        let wt: WorktreeIdentity = serde_json::from_str(&content)?;
        return Ok(wt.worktree_id);
    }
    
    // Generate new ID
    let wt_id = generate_worktree_id(&paths.worktree_root, Utc::now());
    let branch = get_current_branch()?;
    
    let wt = WorktreeIdentity {
        schema_version: 1,
        worktree_id: wt_id.clone(),
        branch,
        root: paths.worktree_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
    };
    
    fs::create_dir_all(&paths.local_jit)?;
    fs::write(&wt_file, serde_json::to_string_pretty(&wt)?)?;
    
    Ok(wt_id)
}
```

## Schema Definitions

### Claims JSONL (Append-Only Audit Log)

**Location:** `.git/jit/claims.jsonl`

**Operations:**

```json
{"schema_version":1,"op":"acquire","lease_id":"01HXJK2M3N4P5Q","issue_id":"ad601a15","agent_id":"agent:copilot-1","worktree_id":"wt:abc123ef","branch":"agents/copilot-1","ttl_secs":600,"acquired_at":"2026-01-03T12:00:00Z","expires_at":"2026-01-03T12:10:00Z"}

{"schema_version":1,"op":"renew","lease_id":"01HXJK2M3N4P5Q","ttl_secs":600,"renewed_at":"2026-01-03T12:05:00Z","expires_at":"2026-01-03T12:15:00Z"}

{"schema_version":1,"op":"release","lease_id":"01HXJK2M3N4P5Q","released_at":"2026-01-03T12:08:00Z","released_by":"agent:copilot-1"}

{"schema_version":1,"op":"auto-evict","lease_id":"01HXJK2M3N4P5Q","evicted_at":"2026-01-03T12:10:00Z","reason":"expired"}

{"schema_version":1,"op":"force-evict","lease_id":"01HXJK2M3N4P5Q","evicted_at":"2026-01-03T12:11:00Z","by":"human:alice","reason":"stale after crash"}

{"schema_version":1,"op":"acquire","lease_id":"01HX...","issue_id":"ISSUE-42","agent_id":"agent:human-1","worktree_id":"wt:abc123","branch":"staging/refactor-42","ttl_secs":0,"reason":"Manual oversight of complex refactor","acquired_at":"2026-01-03T12:00:00Z"}

{"schema_version":1,"op":"heartbeat","lease_id":"01HX...","at":"2026-01-03T12:15:00Z"}

{"schema_version":1,"op":"force-evict","lease_id":"01HX...","by":"admin:erankavija","reason":"holder unresponsive > 2h","at":"2026-01-03T14:30:00Z"}
```

**Immutability:** Never modify or delete entries. Append-only for audit trail.

### Claims Index JSON (Derived View)

**Location:** `.git/jit/claims.index.json`

**Schema:**
```json
{
  "schema_version": 1,
  "generated_at": "2026-01-03T12:16:00Z",
  "sequence": 42,
  "stale_threshold_secs": 3600,
  "active": [
    {
      "lease_id": "01HX...",
      "issue_id": "ISSUE-42",
      "agent_id": "agent:human-1",
      "worktree_id": "wt:abc123",
      "branch": "staging/refactor-42",
      "ttl_secs": 0,
      "expires_at": null,
      "last_beat": "2026-01-03T12:15:00Z",
      "stale": false
    }
  ]
}
```

**Rebuilding:**
```rust
pub fn rebuild_index_from_log(&self) -> Result<ClaimsIndex> {
    let log_path = self.paths.shared_jit.join("claims.jsonl");
    let mut active = HashMap::new();
    
    if log_path.exists() {
        for line in BufReader::new(File::open(&log_path)?).lines() {
            let op: ClaimOp = serde_json::from_str(&line?)?;
            match op {
                ClaimOp::Acquire(lease) => {
                    active.insert(lease.lease_id.clone(), lease);
                }
                ClaimOp::Renew { lease_id, expires_at, .. } => {
                    if let Some(lease) = active.get_mut(&lease_id) {
                        lease.expires_at = expires_at;
                    }
                }
                ClaimOp::Release { lease_id, .. } |
                ClaimOp::AutoEvict { lease_id, .. } |
                ClaimOp::ForceEvict { lease_id, .. } => {
                    active.remove(&lease_id);
                }
            }
        }
    }
    
    // Filter expired
    let now = Utc::now();
    active.retain(|_, lease| lease.expires_at > now);
    
    Ok(ClaimsIndex {
        schema_version: 1,
        generated_at: Utc::now(),
        sequence: active.len() as u64,
        active: active.into_values().collect(),
    })
}
```

### Heartbeat Schema

**Location:** `.git/jit/heartbeat/<agent-id>.json`

**Schema:**
```json
{
  "agent_id": "agent:copilot-1",
  "worktree_id": "wt:abc123ef",
  "branch": "agents/copilot-1",
  "pid": 12345,
  "last_beat": "2026-01-03T12:06:30Z",
  "interval_secs": 30
}
```

**Background heartbeat process (optional):**
```rust
// Spawn background thread that updates heartbeat every 30s
pub fn start_heartbeat_thread(
    agent_id: String,
    paths: WorktreePaths,
) -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(30));
            let _ = update_heartbeat(&agent_id, &paths);
        }
    })
}
```

## Enforcement Mechanisms

### Defense-in-Depth Strategy

Enforcement happens at multiple layers to ensure correctness:

1. **CLI-level** (layer 1): `require_active_lease()` before any structural operation
2. **Pre-commit hook** (layer 2): Validates leases and divergence before commit
3. **Pre-push hook** (layer 3, optional): Validates leases still active before push
4. **Pre-receive hook** (layer 4, optional): Server-side validation for shared bare origins

### CLI-Level Divergence Gate

**Global operations require common history with main:**

```rust
pub fn validate_divergence(&self) -> Result<()> {
    let merge_base = Command::new("git")
        .args(["merge-base", "HEAD", "origin/main"])
        .output()
        .context("Failed to get merge-base")?
        .stdout;
    
    let main_commit = Command::new("git")
        .args(["rev-parse", "origin/main"])
        .output()
        .context("Failed to get main commit")?
        .stdout;
    
    if merge_base != main_commit {
        bail!(
            "Branch diverged from origin/main.\n\
             Global operations require common history.\n\
             Run: git rebase origin/main"
        );
    }
    Ok(())
}

// Apply before global operations
impl CommandExecutor {
    pub fn update_config(&self, params: ConfigUpdate) -> Result<()> {
        // Check config setting
        if self.config.global_operations.require_main_history {
            self.validate_divergence()?;
        }
        
        // Proceed with update
        self.storage.update_config(params)
    }
    
    pub fn update_type_hierarchy(&self, params: TypeUpdate) -> Result<()> {
        if self.config.global_operations.require_main_history {
            self.validate_divergence()?;
        }
        
        self.storage.update_type_hierarchy(params)
    }
    
    pub fn update_gates_registry(&self, params: GateUpdate) -> Result<()> {
        if self.config.global_operations.require_main_history {
            self.validate_divergence()?;
        }
        
        self.storage.update_gates_registry(params)
    }
}

// Expose as CLI command for explicit validation
pub fn cmd_validate(args: ValidateArgs) -> Result<()> {
    let executor = CommandExecutor::new()?;
    
    if args.divergence {
        executor.validate_divergence()?;
        println!("✓ Branch is up-to-date with origin/main");
    }
    
    if args.leases {
        executor.validate_all_leases()?;
        println!("✓ All active leases are valid");
    }
    
    Ok(())
}
```

### Pre-Commit Hook

**Location:** `.git/hooks/pre-commit`

**Template:**

```bash
#!/usr/bin/env bash
set -euo pipefail

# Get changed files
changed=$(git diff --cached --name-only || true)

# 1. Enforce global operations on main-only
if echo "$changed" | grep -E '^\..?jit/(config|type-hierarchy|gates/registry)\b' >/dev/null; then
  base=$(git merge-base HEAD origin/main 2>/dev/null || git merge-base HEAD main 2>/dev/null || echo "")
  main=$(git rev-parse origin/main 2>/dev/null || git rev-parse main 2>/dev/null || echo "")
  
  if [ -n "$base" ] && [ -n "$main" ] && [ "$base" != "$main" ]; then
    echo "❌ Error: Global .jit changes require common history with main."
    echo "   Run: git rebase origin/main"
    exit 1
  fi
fi

# 2. Enforce active lease for structural per-issue edits
for f in $changed; do
  if echo "$f" | grep -E '^\.?jit/issues/.+/(issue\.json|deps\.or-set\.jsonl|labels\.or-set\.jsonl)$' >/dev/null; then
    issue_id=$(echo "$f" | sed -E 's#^\.?jit/issues/([^/]+)/.*#\1#')
    
    # Get current agent and worktree
    agent_id="${JIT_AGENT_ID:-}"
    if [ -z "$agent_id" ]; then
      echo "❌ Error: JIT_AGENT_ID not set. Required for structural edits."
      echo "   Export: export JIT_AGENT_ID='agent:your-name'"
      exit 1
    fi
    
    worktree_id=$(jq -r '.worktree_id' .jit/worktree.json 2>/dev/null || echo "")
    if [ -z "$worktree_id" ]; then
      echo "❌ Error: No worktree identity found."
      exit 1
    fi
    
    # Check for active lease
    claims_index=".git/jit/claims.index.json"
    if [ -f "$claims_index" ]; then
      has_claim=$(jq -r \
        --arg issue "$issue_id" \
        --arg agent "$agent_id" \
        --arg wt "$worktree_id" \
        '.active[] | select(.issue_id==$issue and .agent_id==$agent and .worktree_id==$wt and (.stale != true)) | .lease_id' \
        "$claims_index" 2>/dev/null || echo "")
      
      if [ -z "$has_claim" ]; then
        echo "❌ Error: Structural edit to issue $issue_id without active lease."
        echo "   Agent: $agent_id, Worktree: $worktree_id"
        echo "   Acquire: jit claim acquire $issue_id --ttl 600"
        exit 1
      fi
    fi
  fi
done

# All checks passed
exit 0
```

**Installation:**

```bash
# Make executable
chmod +x .git/hooks/pre-commit

# Copy to new worktrees automatically
git config core.hooksPath .git/hooks
```

### Pre-Push Hook (Strict Teams)

**Validates that leases mentioned in commits are still active before push.**

**Location:** `.git/hooks/pre-push`

**Template:**
```bash
#!/usr/bin/env bash
# Pre-push hook: Verify active leases before push
set -euo pipefail

remote="$1"
url="$2"

# Only check for non-local pushes
if [[ "$url" =~ ^(file://|/) ]]; then
  exit 0
fi

# Read push details from stdin
while read local_ref local_sha remote_ref remote_sha; do
  # Skip deletes
  if [ "$local_sha" = "0000000000000000000000000000000000000000" ]; then
    continue
  fi
  
  # Get commits being pushed
  if [ "$remote_sha" = "0000000000000000000000000000000000000000" ]; then
    # New branch, check all commits
    commits=$(git rev-list "$local_sha")
  else
    # Existing branch, check new commits
    commits=$(git rev-list "$remote_sha..$local_sha")
  fi
  
  # Verify leases for changed issue files (file-based approach)
  # Note: We use file-based detection rather than commit trailers to minimize
  # manual decoration requirements. This automatically detects issue modifications.
  changed=$(git diff --name-only "$range" | grep '^\.jit/issues/[^/]*/issue\.json$' || true)
  
  if [ -n "$changed" ]; then
    claims_index=".git/jit/claims.index.json"
    if [ ! -f "$claims_index" ]; then
      echo "⚠️  Warning: No claims index found at $claims_index"
      echo "   Skipping lease validation"
    else
      while IFS= read -r file; do
        issue_id=$(basename "$(dirname "$file")")
        
        # Check if issue has active, non-stale lease
        lease_status=$(jq -r \
          --arg iid "$issue_id" \
          '.active[] | select(.issue_id==$iid and (.stale != true)) | .lease_id' \
          "$claims_index" 2>/dev/null | tr -d '[:space:]')
        
        if [ -z "$lease_status" ]; then
          echo "❌ Error: Issue $issue_id was modified but has no active lease"
          echo "   File: $file"
          echo "   Acquire lease: jit claim acquire $issue_id --ttl 600"
          echo ""
          echo "   Emergency override: git push --no-verify"
          exit 1
        fi
      done <<< "$changed"
    fi
  fi
done

echo "✓ All leases valid and branch up-to-date"
exit 0
```

**Installation:**
```bash
chmod +x .git/hooks/pre-push

# Enable for repository
git config core.hooksPath .git/hooks

# Optional: Skip on force push (not recommended)
# git push --no-verify
```

### Pre-Receive Hook (Server-Side, Optional)

**For local bare origin merge queue setup.**

**Location:** `.git/hooks/pre-receive` (on bare repository)

**Template:**
```bash
#!/usr/bin/env bash
# Pre-receive hook: Server-side lease and divergence validation
set -euo pipefail

# Path to shared claims data (if accessible on server)
CLAIMS_LOG="${GIT_DIR}/jit/claims.jsonl"

while read old_sha new_sha ref_name; do
  # Skip deletes
  if [ "$new_sha" = "0000000000000000000000000000000000000000" ]; then
    continue
  fi
  
  # Get commits being pushed
  if [ "$old_sha" = "0000000000000000000000000000000000000000" ]; then
    commits=$(git rev-list "$new_sha")
  else
    commits=$(git rev-list "$old_sha..$new_sha")
  fi
  
  # Verify each commit
  for commit in $commits; do
    # Extract lease info
    lease_id=$(git log --format='%(trailers:key=JIT-Lease,valueonly)' -1 "$commit" | tr -d '[:space:]')
    
    if [ -n "$lease_id" ]; then
      # Verify lease existed at commit time (check claims.jsonl)
      if [ -f "$CLAIMS_LOG" ]; then
        commit_time=$(git log --format='%aI' -1 "$commit")
        
        # Check if lease was acquired before commit and not released before commit
        # (requires parsing claims.jsonl - complex, example simplified)
        lease_valid=$(grep -c "\"lease_id\":\"$lease_id\"" "$CLAIMS_LOG" || echo "0")
        
        if [ "$lease_valid" -eq "0" ]; then
          echo "❌ Rejected: Commit $commit references unknown lease $lease_id" >&2
          exit 1
        fi
      fi
    fi
    
    # Verify global operations on main branch only
    changed_files=$(git diff-tree --no-commit-id --name-only -r "$commit")
    if echo "$changed_files" | grep -E '^\..?jit/(config|type-hierarchy|gates/registry)\b' >/dev/null; then
      # Only allow on main branch
      if [ "$ref_name" != "refs/heads/main" ]; then
        echo "❌ Rejected: Global operations only allowed on main branch" >&2
        echo "   Commit: $commit" >&2
        echo "   Branch: $ref_name" >&2
        exit 1
      fi
    fi
  done
done

echo "✓ Server-side validation passed" >&2
exit 0
```

**Note:** Pre-receive hooks require access to `.git/jit/claims.jsonl` on the server, which may not be available for remote origins. This is primarily useful for local bare repository merge queue setups.

### Branch Divergence Detection

```rust
pub fn check_branch_divergence(&self) -> Result<bool> {
    let merge_base = Command::new("git")
        .args(["merge-base", "HEAD", "origin/main"])
        .output()
        .context("Failed to get merge-base")?
        .stdout;
    
    let main_commit = Command::new("git")
        .args(["rev-parse", "origin/main"])
        .output()
        .context("Failed to get main commit")?
        .stdout;
    
    Ok(merge_base != main_commit)
}

pub fn enforce_main_only_operations(&self) -> Result<()> {
    if self.check_branch_divergence()? {
        bail!("Global operations require common history with main. Run: git rebase origin/main");
    }
    Ok(())
}
```

## File Formats and Merge Strategies

### Gitattributes for Safe Merges

**Create `.gitattributes` in repository root:**

```gitattributes
# Union merge for append-only logs
# Multiple agents can append independently, git merges all lines
*.jsonl merge=union
**/*-set.jsonl merge=union
**/*-or-set.jsonl merge=union

# CRDT merge driver for descriptions (future)
# Requires custom merge driver configured in .git/config
**/description.automerge merge=automerge

# Never merge binary indices - always take one side
**/*.sqlite binary
**/cache/*.db binary

# Plaintext files use default merge
*.json merge=text
*.md merge=text
```

**Configure CRDT merge driver (future):**
```bash
# In .git/config or ~/.gitconfig
[merge "automerge"]
    name = Automerge CRDT merge driver
    driver = jit-automerge-merge %O %A %B %P
    
# jit-automerge-merge script:
# - Loads Automerge documents from %A (ours) and %B (theirs)
# - Merges using Automerge CRDT semantics
# - Writes result to %A
```

### Standardized Event Envelopes

**Unified schema for both control-plane and data-plane events:**

**Control-plane events (claims.jsonl):**
```json
{
  "schema_version": 1,
  "sequence": 42,
  "event_type": "claim.acquired",
  "timestamp": "2026-01-03T12:00:00Z",
  "actor": {
    "agent_id": "agent:copilot-1",
    "worktree_id": "wt:abc123ef"
  },
  "payload": {
    "lease_id": "01HXJK2M3N4P5Q",
    "issue_id": "ad601a15",
    "ttl_secs": 600,
    "expires_at": "2026-01-03T12:10:00Z"
  }
}
```

**Data-plane events (events/local.jsonl):**
```json
{
  "schema_version": 1,
  "sequence": 17,
  "event_type": "issue.updated",
  "timestamp": "2026-01-03T12:05:00Z",
  "actor": {
    "agent_id": "agent:copilot-1",
    "worktree_id": "wt:abc123ef",
    "lease_id": "01HXJK2M3N4P5Q"
  },
  "payload": {
    "issue_id": "ad601a15",
    "changes": {
      "state": {"from": "ready", "to": "in_progress"}
    }
  }
}
```

**Unified event types:**
- Control-plane: `claim.acquired`, `claim.renewed`, `claim.released`, `claim.evicted`
- Data-plane: `issue.created`, `issue.updated`, `issue.deleted`, `dep.added`, `dep.removed`, `gate.added`, `gate.completed`

**Benefits:**
- Consistent tooling for event parsing and analysis
- Easier correlation between control and data plane actions
- Standardized schema evolution path

### Storage Format References

**Worktree-aware layout summary:**

```
.git/jit/                   # Control plane (local only, not versioned)
├── claims.jsonl            # Append-only audit log
├── claims.index.json       # Derived view (can be rebuilt)
├── locks/
│   ├── claims.lock         # Global coordination lock
│   ├── claims.lock.meta    # Lock metadata
│   └── scope.lock          # Global operations lock
└── heartbeat/
    └── <agent-id>.json     # Optional heartbeat files

<worktree>/.jit/            # Data plane (versioned)
├── worktree.json           # Worktree identity
├── config.toml             # Repository configuration
├── issues/
│   └── <issue-id>/
│       ├── issue.json      # Issue data
│       ├── deps.or-set.jsonl    # Dependencies (OR-set CRDT)
│       └── labels.or-set.jsonl  # Labels (OR-set CRDT)
├── gates/
│   ├── registry.json       # Global gate definitions
│   └── results/
│       └── <issue-id>.json # Gate execution results
└── events/
    └── local.jsonl         # Data-plane events
```

**Key properties:**
- Control plane is machine-local, never committed to git
- Data plane is git-versioned, uses union merge for append-only logs
- All structural edits protected by leases
- Global config (registry, type-hierarchy) requires main-branch history

## Recovery and Robustness

### Crash Recovery

**On startup (any jit command):**

```rust
pub fn startup_recovery(&self) -> Result<()> {
    // 1. Check for stale lock files
    self.cleanup_stale_locks()?;
    
    // 2. Rebuild index if corrupted
    if !self.verify_index_consistency()? {
        warn!("Claims index inconsistent, rebuilding from log...");
        let index = self.rebuild_index_from_log()?;
        self.write_index_atomic(&index)?;
    }
    
    // 3. Evict expired leases
    let mut index = self.load_claims_index()?;
    self.evict_expired(&mut index)?;
    self.write_index_atomic(&index)?;
    
    Ok(())
}
```

### Stale Lock Cleanup

```rust
pub fn cleanup_stale_locks(&self) -> Result<()> {
    let lock_dir = self.paths.shared_jit.join("locks");
    if !lock_dir.exists() {
        return Ok(());
    }
    
    for entry in fs::read_dir(&lock_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        // Try to acquire lock
        match self.try_lock_exclusive(&path) {
            Ok(_guard) => {
                // Lock acquired, it was stale
                warn!("Removed stale lock: {}", path.display());
            }
            Err(_) => {
                // Lock held, check age
                let metadata = fs::metadata(&path)?;
                let age = metadata.modified()?.elapsed()?;
                
                if age > Duration::from_secs(3600) {
                    warn!("Lock file very old: {} ({}s)", path.display(), age.as_secs());
                }
            }
        }
    }
    
    Ok(())
}
```

### Index Consistency Validation

```rust
pub fn verify_index_consistency(&self) -> Result<bool> {
    let index_path = self.paths.shared_jit.join("claims.index.json");
    if !index_path.exists() {
        return Ok(false);
    }
    
    let index: ClaimsIndex = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    
    // Check for duplicates
    let mut seen_issues = HashSet::new();
    for lease in &index.active {
        if !seen_issues.insert(&lease.issue_id) {
            error!("Duplicate active lease for issue: {}", lease.issue_id);
            return Ok(false);
        }
    }
    
    // Check all leases are not expired
    let now = Utc::now();
    for lease in &index.active {
        if lease.expires_at <= now {
            error!("Expired lease in index: {}", lease.lease_id);
            return Ok(false);
        }
    }
    
    Ok(true)
}
```

## Configuration

### Repository Configuration

**Location:** `<worktree>/.jit/config.toml`

**Schema:**
```toml
[worktree]
# auto: detect git worktree and enable automatically
# on: force worktree mode (fail if not in worktree)
# off: disable worktree features (use legacy .jit/ only)
mode = "auto"

# strict: block operations without lease
# warn: warn but allow operations without lease
# off: no lease enforcement
enforce_leases = "strict"

[coordination]
# Default TTL for new leases (seconds)
default_ttl_secs = 600

# Heartbeat interval for automatic lease renewal (seconds)
heartbeat_interval_secs = 30

# Warn when lease has less than this % of TTL remaining
lease_renewal_threshold_pct = 10

# Staleness threshold for TTL=0 leases (seconds)
stale_threshold_secs = 3600

# Maximum concurrent TTL=0 leases
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10

# Automatic lease renewal by heartbeat daemon
auto_renew_leases = false

[global_operations]
# Require common history with main for global operations
require_main_history = true

# Branches allowed to modify global config
allowed_branches = ["main", "develop"]

[locks]
# Maximum age for lock files before considered stale (seconds)
max_age_secs = 3600

# Enable lock metadata for diagnostics
enable_metadata = true

[events]
# Enable sequence numbers in event logs
enable_sequences = true

# Standardize event envelopes across control and data plane
use_unified_envelope = true
```

**Loading order:**
1. Repository config: `<worktree>/.jit/config.toml`
2. User config: `~/.config/jit/config.toml`
3. System config: `/etc/jit/config.toml`
4. Defaults (hardcoded)

**CLI overrides:**
```bash
# Override config for single command
jit claim acquire <issue> --ttl 1200
jit issue update <issue> --strict  # Force lease check even if mode=warn
jit claim acquire <issue> --agent-id agent:override
```

### Agent Configuration

**Location:** `~/.config/jit/agent.toml`

**Schema:**
```toml
[agent]
# Persistent agent identity
id = "agent:copilot-1"

# When this identity was created
created_at = "2026-01-03T12:00:00Z"

# Human-readable description
description = "GitHub Copilot Workspace Session 1"

# Optional: Default TTL preference
default_ttl_secs = 900

[behavior]
# Auto-start heartbeat daemon for lease renewal
auto_heartbeat = false

# Heartbeat interval (seconds)
heartbeat_interval = 30
```

**Environment variable overrides:**
```bash
# Session-specific override (highest priority)
export JIT_AGENT_ID="agent:session-xyz"

# Run with override
JIT_AGENT_ID=agent:override jit claim acquire <issue>
```

### Configuration Commands

```bash
# Show effective configuration
jit config show [--json]

# Get specific config value
jit config get worktree.mode
jit config get coordination.default_ttl_secs

# Set config value (repository-level)
jit config set worktree.enforce_leases strict
jit config set coordination.default_ttl_secs 1200

# Set config value (user-level)
jit config set --global agent.id agent:alice
jit config set --global agent.default_ttl_secs 900

# Validate configuration
jit config validate

# Reset to defaults
jit config reset coordination.default_ttl_secs
```

### New Commands

**`jit claim` subcommand group:**

```bash
# Acquire a claim
jit claim acquire <issue-id> [--ttl <seconds>] [--agent-id <id>]

# Renew an existing claim
jit claim renew <lease-id> [--ttl <seconds>]

# Release a claim
jit claim release <lease-id>

# Force evict (admin)
jit claim force-evict <lease-id> --reason <reason>

# Show claim status
jit claim status [--json]
jit claim status --issue <issue-id> [--json]

# List all active claims
jit claim list [--json]
```

**TTL=0 constraints:**
- `jit claim acquire <issue-id> --ttl 0 --reason "..."` is only allowed when repository policy permits indefinite leases
- Requires `--reason` flag with non-empty explanation for audit trail
- CLI enforces per-agent and per-repository limits (`max_indefinite_leases_per_agent`, `max_indefinite_leases_per_repo`)
- Routine automated agents should use finite TTL with auto-renew instead

**`jit worktree` subcommand group:**

```bash
# Show current worktree info
jit worktree info [--json]

# List all worktrees (from git)
jit worktree list [--json]

# Initialize worktree (manual)
jit worktree init
```

**`jit validate` subcommand group:**

```bash
# Validate branch divergence
jit validate divergence

# Validate all active leases
jit validate leases

# Validate configuration
jit validate config

# Run all validations
jit validate all [--json]
```

### Long-Running Tasks

**Lease renewal warnings:**

```rust
pub fn check_lease_expiry_warning(&self, lease: &Lease) -> Option<String> {
    let remaining_secs = lease.remaining_secs();
    let threshold = self.config.coordination.lease_renewal_threshold_pct;
    let threshold_secs = (lease.ttl_secs * threshold as u64) / 100;
    
    if remaining_secs < threshold_secs {
        let minutes = remaining_secs / 60;
        let seconds = remaining_secs % 60;
        
        return Some(format!(
            "⚠️  Lease {} expires in {}m{}s ({}% remaining)\n\
             Renew: jit claim renew {} --ttl {}",
            lease.lease_id,
            minutes,
            seconds,
            (remaining_secs * 100) / lease.ttl_secs,
            lease.lease_id,
            lease.ttl_secs
        ));
    }
    
    None
}

// Check before each CLI operation
impl CommandExecutor {
    pub fn execute(&self) -> Result<()> {
        // Check all active leases for this agent
        let leases = self.claim_coordinator.get_agent_leases(&self.agent_id)?;
        
        for lease in leases {
            if let Some(warning) = self.check_lease_expiry_warning(&lease) {
                eprintln!("{}", warning);
            }
        }
        
        // Continue with operation...
    }
}
```

**Heartbeat daemon (optional):**

```rust
pub fn start_heartbeat_daemon(
    agent_id: String,
    paths: WorktreePaths,
    config: HeartbeatConfig,
) -> Result<JoinHandle<()>> {
    let handle = thread::spawn(move || {
        let coordinator = ClaimCoordinator::new(paths.clone(), FileLocker);
        
        loop {
            thread::sleep(Duration::from_secs(config.interval_secs));
            
            // Update heartbeat file
            if let Err(e) = update_heartbeat(&agent_id, &paths) {
                error!("Heartbeat update failed: {}", e);
            }
            
            // Auto-renew active leases if enabled
            if config.auto_renew_leases {
                if let Err(e) = auto_renew_agent_leases(&coordinator, &agent_id) {
                    error!("Lease auto-renewal failed: {}", e);
                }
            }
        }
    });
    
    Ok(handle)
}

fn auto_renew_agent_leases(
    coordinator: &ClaimCoordinator,
    agent_id: &str,
) -> Result<()> {
    let leases = coordinator.get_agent_leases(agent_id)?;
    
    for lease in leases {
        let remaining_secs = lease.remaining_secs();
        let threshold_secs = (lease.ttl_secs * 20) / 100; // Renew at 20% remaining
        
        if remaining_secs < threshold_secs {
            info!("Auto-renewing lease {} ({}s remaining)", lease.lease_id, remaining_secs);
            coordinator.renew_claim(&lease.lease_id, lease.ttl_secs)?;
        }
    }
    
    Ok(())
}
```

**Lease transfer for handoff:**

```rust
pub fn transfer_lease(
    &self,
    lease_id: &str,
    to_agent_id: &str,
    to_worktree_id: &str,
    reason: &str,
) -> Result<Lease> {
    let lock_path = self.paths.shared_jit.join("locks/claims.lock");
    let _guard = self.locker.lock_exclusive(&lock_path)?;
    
    let mut index = self.load_claims_index()?;
    
    // Find existing lease
    let old_lease = index.find_lease(lease_id)
        .ok_or_else(|| anyhow!("Lease {} not found", lease_id))?
        .clone();
    
    // Verify ownership (only owner can transfer)
    let current_agent = self.get_agent_id()?;
    if old_lease.agent_id != current_agent {
        bail!("Cannot transfer lease owned by {}", old_lease.agent_id);
    }
    
    // Create new lease with same issue, different owner
    let new_lease = Lease::new(
        old_lease.issue_id.clone(),
        to_agent_id.to_string(),
        to_worktree_id.to_string(),
        self.get_branch_for_worktree(to_worktree_id)?,
        old_lease.ttl_secs,
    );
    
    // Log transfer
    self.append_claim_op(&ClaimOp::Transfer {
        from_lease_id: lease_id.to_string(),
        to_lease: new_lease.clone(),
        transferred_at: Utc::now(),
        transferred_by: current_agent,
        reason: reason.to_string(),
    })?;
    
    // Update index
    index.remove_lease(lease_id);
    index.add_lease(new_lease.clone());
    self.write_index_atomic(&index)?;
    
    Ok(new_lease)
}
```

**CLI command:**
```bash
# Transfer lease to another agent (e.g., for debugging, handoff)
jit claim transfer <lease-id> \
  --to-agent agent:alice \
  --to-worktree wt:abc123ef \
  --reason "Handing off for review"
```

### Example Usage

```bash
# Setup
export JIT_AGENT_ID="agent:copilot-1"

# Create worktree for parallel work
git worktree add ../work-auth agents/copilot-1
cd ../work-auth

# Claim an issue (600s default TTL)
jit claim acquire ad601a15 --ttl 600
# => Lease ID: 01HXJK2M3N4P5Q6R7S8T9VWXYZ

# Work on it...
jit issue update ad601a15 --state in_progress

# Renew claim if needed
jit claim renew 01HXJK2M3N4P5Q --ttl 600

# Complete and release
jit issue update ad601a15 --state done
jit claim release 01HXJK2M3N4P5Q
```

## Testing Strategy

### Unit Tests

**Claim coordination:**
- Acquire claim for unleased issue
- Reject claim for already-claimed issue
- Automatic expiration of leases
- Lease renewal extends expiry
- Force eviction works correctly

**Path resolution:**
- Detect main worktree vs. secondary worktree
- Generate stable worktree IDs
- Load or create worktree identity

**Index operations:**
- Rebuild index from JSONL log
- Detect and fix inconsistencies
- Evict expired leases

### Integration Tests

**Concurrent claims:**
```rust
#[test]
fn test_concurrent_claim_attempts() {
    let paths = setup_test_paths();
    let coordinator = ClaimCoordinator::new(paths, FileLocker);
    
    let issue_id = "test-issue-123";
    
    // Spawn multiple threads trying to claim same issue
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let coord = coordinator.clone();
            let issue = issue_id.to_string();
            thread::spawn(move || {
                coord.acquire_claim(&issue, &format!("agent:{}", i), 60)
            })
        })
        .collect();
    
    // Collect results
    let results: Vec<_> = handles.into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    
    // Exactly one should succeed
    let successful = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(successful, 1);
}
```

**Recovery scenarios:**
```rust
#[test]
fn test_crash_recovery_rebuilds_index() {
    let paths = setup_test_paths();
    let coordinator = ClaimCoordinator::new(paths.clone(), FileLocker);
    
    // Create some claims
    coordinator.acquire_claim("issue-1", "agent:1", 60)?;
    coordinator.acquire_claim("issue-2", "agent:2", 60)?;
    
    // Corrupt the index
    fs::write(paths.shared_jit.join("claims.index.json"), "invalid json")?;
    
    // Startup recovery should rebuild
    coordinator.startup_recovery()?;
    
    // Index should be valid again
    let index = coordinator.load_claims_index()?;
    assert_eq!(index.active.len(), 2);
}
```

### Property-Based Tests

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_index_always_consistent_with_log(
        ops in prop::collection::vec(claim_op_strategy(), 1..100)
    ) {
        let paths = setup_test_paths();
        let coordinator = ClaimCoordinator::new(paths, FileLocker);
        
        // Apply operations
        for op in ops {
            coordinator.append_claim_op(&op)?;
        }
        
        // Rebuild index
        let index = coordinator.rebuild_index_from_log()?;
        
        // Verify consistency
        assert!(coordinator.verify_index_consistency()?);
    }
}
```

## Migration Path

### Phase 1: Worktree Support (Non-Breaking)

**Changes:**
- Add worktree path detection
- Create worktree identity on first use
- Keep existing storage working for non-worktree users

**Backwards compatibility:** Existing `.jit/` continues to work in main worktree.

### Phase 2: Claim Coordination (Opt-In)

**Changes:**
- Add `jit claim` commands
- Add `.git/jit/` control plane
- Make lease checks optional (warn, don't block)

**Opt-in:** Users must explicitly use `jit claim` to participate.

### Phase 3: Enforcement (Gradual)

**Changes:**
- Add pre-commit hook template
- Document installation
- Make lease enforcement strict (optional via config)

**Gradual adoption:** Teams enable enforcement when ready.

## Security and Permissions

### File Permissions

**Shared control plane:**
```bash
chmod 0700 .git/jit/               # Owner-only
chmod 0600 .git/jit/claims.jsonl   # Owner-only
chmod 0600 .git/jit/claims.index.json
```

**Per-worktree data:**
```bash
chmod 0755 .jit/                   # Readable by others (git-versioned)
chmod 0644 .jit/issues/*/issue.json
```

### Multi-User Scenarios

**Single-user machine:** Default permissions work.

**Multi-user machine (shared repo):**
- Use group permissions: `chmod 0770 .git/jit/`
- Set group ownership: `chgrp developers .git/jit/`
- Enable setgid bit: `chmod g+s .git/jit/`

**Security consideration:** `.git/jit/` is local-only and should not be committed to git.

## Integration and Future Work

### Web UI Integration

**Claim status visualization:**
```rust
// jit-server API endpoints
#[get("/api/claims/status")]
async fn get_claim_status() -> Json<ClaimStatus> {
    let coordinator = ClaimCoordinator::new(...)?;
    let index = coordinator.load_claims_index()?;
    
    Json(ClaimStatus {
        active_leases: index.active,
        total_count: index.active.len(),
        generated_at: index.generated_at,
    })
}

#[get("/api/worktrees")]
async fn list_worktrees() -> Json<Vec<WorktreeInfo>> {
    // Parse git worktree list --porcelain
    let worktrees = parse_git_worktrees()?;
    Json(worktrees)
}

#[get("/api/claims/history")]
async fn get_claim_history(issue_id: Query<String>) -> Json<Vec<ClaimOp>> {
    let history = parse_claims_log_for_issue(&issue_id)?;
    Json(history)
}
```

**UI features:**
- Active leases dashboard with agent, worktree, expiry time
- Lease history timeline for each issue
- Worktree status overview (which issues claimed in each worktree)
- Force-evict capability for administrators
- Lease renewal/extension interface

### Dispatcher Integration

**Automatic lease management for agents:**
```rust
// jit-dispatch agent wrapper
impl AgentExecutor {
    pub async fn execute_with_lease(&self, issue_id: &str) -> Result<()> {
        // Acquire lease with appropriate TTL
        let lease = self.coordinator.acquire_claim(
            issue_id,
            &self.agent_id,
            self.config.default_ttl,
        )?;
        
        // Start heartbeat daemon for auto-renewal
        let heartbeat = if self.config.auto_heartbeat {
            Some(start_heartbeat_daemon(
                self.agent_id.clone(),
                self.paths.clone(),
                HeartbeatConfig {
                    interval_secs: self.config.heartbeat_interval,
                    auto_renew_leases: true,
                },
            )?)
        } else {
            None
        };
        
        // Execute work with lease context
        let result = self.execute_issue_work(issue_id, &lease).await;
        
        // Stop heartbeat daemon
        drop(heartbeat);
        
        // Release lease
        self.coordinator.release_claim(&lease.lease_id)?;
        
        result
    }
    
    async fn execute_issue_work(&self, issue_id: &str, lease: &Lease) -> Result<()> {
        // Work implementation...
        // Can periodically check lease.remaining_secs() and warn if low
        
        Ok(())
    }
}
```

### Commit Message Trailers

**Automatic lease context in commits:**
```rust
pub fn commit_with_lease_context(
    &self,
    message: &str,
    lease: &Lease,
    issue_id: &str,
) -> Result<()> {
    let full_message = format!(
        "{}\n\n\
         JIT-Lease: {}\n\
         JIT-Issue: {}\n\
         JIT-Agent: {}\n\
         JIT-Worktree: {}",
        message,
        lease.lease_id,
        issue_id,
        lease.agent_id,
        lease.worktree_id
    );
    
    Command::new("git")
        .args(["commit", "-m", &full_message])
        .status()?;
    
    Ok(())
}
```

**Benefits:**
- Audit trail of which lease authorized each commit
- Server-side verification possible (pre-receive hooks)
- Post-hoc analysis of work attribution
- Forensics for debugging coordination issues

### Future Enhancements

**Cross-machine coordination (future):**
- Use custom refs `refs/jit/claims/*` for replication
- Push/fetch claims to enable distributed coordination
- Conflict resolution when claims diverge

**CRDT for multi-writer descriptions:**
- Integrate Automerge CRDT for concurrent free-text edits
- Custom merge driver: `jit-automerge-merge`
- Allow multiple agents to edit descriptions simultaneously

**Advanced lease operations:**
- Lease splitting: divide issue work across multiple agents
- Lease sub-claims: hierarchical ownership for sub-tasks
- Conditional leases: acquire only if dependencies ready

**Enhanced monitoring:**
- Prometheus metrics for lease acquisition rate, duration, evictions
- Alerting for frequent lease conflicts or expirations
- Dashboard for team-wide coordination visibility

**Policy enforcement:**
- Required approvers for force-evict operations
- Maximum TTL limits per agent type
- Automatic escalation for long-held leases

## Next Implementation Steps

### Phase 1: Core Infrastructure (Week 1)
1. **Worktree path detection** (`crates/jit/src/storage/worktree_paths.rs`)
2. **Claim coordinator** (`crates/jit/src/storage/claims.rs`)
3. **File locking utilities** (`crates/jit/src/storage/lock.rs`)
4. **Index rebuild and verification**

### Phase 2: CLI Integration (Week 1-2)
1. **`jit claim` commands** (acquire, renew, release, status, list)
2. **`jit worktree` commands** (info, list, init)
3. **`jit validate --divergence`** command
4. **CLI-level lease enforcement in CommandExecutor**

### Phase 3: Enforcement and Recovery (Week 2)
1. **Pre-commit hook template** (`scripts/hooks/pre-commit`)
2. **Pre-push hook template** (`scripts/hooks/pre-push`)
3. **Startup recovery routine**
4. **Stale lock cleanup**
5. **`.gitattributes` for safe merges**

### Phase 4: Configuration and Polish (Week 2-3)
1. **Repository config schema** (`.jit/config.toml`)
2. **Agent config** (`~/.config/jit/agent.toml`)
3. **Lease renewal warnings and heartbeat daemon**
4. **Lease transfer operation**

### Phase 5: Testing and Documentation (Week 3)
1. **Unit tests** (claim coordination, path resolution, index ops)
2. **Integration tests** (concurrent claims, recovery scenarios)
3. **Property-based tests** (index consistency)
4. **Tutorial**: "Parallel Work with Git Worktrees"
5. **How-to guide**: "Setting Up Multi-Agent Coordination"

### Phase 6: Advanced Features (Week 4+)
1. **Web UI integration** (claim status, worktree list endpoints)
2. **Dispatcher integration** (automatic lease management)
3. **Commit message trailers** for audit trail
4. **CRDT merge driver** for descriptions (future)
5. **Cross-machine coordination** via custom refs (future)

## Open Questions

1. **Claim visibility across machines?**
   - Current design: Local-only (`.git/jit/` not versioned)
   - Future: Use custom refs `refs/jit/claims/*` for replication?
   - Trade-off: Simplicity vs. distributed coordination

2. **CRDT for multi-writer descriptions?**
   - Current: Single-writer via leases for all fields
   - Future: Automerge CRDT for concurrent free-text edits?
   - Complexity: Custom merge driver, CRDT library dependency

3. **Heartbeat daemon lifecycle?**
   - Current: Both manual renewal and opt-in daemon supported
   - Future: Auto-start daemon based on config? Systemd integration?
   - Consider: Battery impact on laptops, resource usage

4. **Lock timeout policy?**
   - Current: 1-hour max age for locks
   - Should this be configurable per-repository?
   - How to handle truly long-running operations (multi-hour builds)?

5. **Lease transfer authorization?**
   - Current: Only lease owner can transfer
   - Future: Admin override? Automated handoff policies?
   - Security implications of privilege escalation

## References

- [Git Worktree Documentation](https://git-scm.com/docs/git-worktree)
- [ULID Specification](https://github.com/ulid/spec)
- [fs4 Rust Crate](https://docs.rs/fs4/) - File locking
- [Automerge CRDT](https://automerge.org/) - Future multi-writer support

## Acceptance Criteria

- [ ] Multiple agents can work on different issues in parallel worktrees
- [ ] No file conflicts between concurrent agents
- [ ] Lease acquisition is atomic and race-free
- [ ] Expired finite leases are automatically evicted and stale TTL=0 leases can be detected and force-evicted
- [ ] Pre-commit hook prevents unauthorized structural edits
- [ ] Crash recovery rebuilds index from audit log
- [ ] All operations logged for observability
- [ ] Documentation covers setup and usage
- [ ] Integration tests validate concurrent scenarios
