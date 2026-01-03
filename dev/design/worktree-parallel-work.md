# Parallel Multi-Agent Work with Git Worktrees

**Issue:** ad601a15-5217-439f-9b3c-94a52c06c18b  
**Status:** Design  
**Author:** System  
**Date:** 2026-01-03

## Problem Statement

Current `.jit/` storage is repository-local but not worktree-aware. Multiple agents working on the same machine cannot operate in parallel because they share the same `.jit/` directory, leading to file conflicts and race conditions.

**Key limitations:**
- Shared `.jit/issues/` causes concurrent write conflicts
- No coordination mechanism for issue ownership
- No visibility into which agent is working on what
- Manual deconfliction required (inefficient)

## Goals

1. **Enable true parallelism**: Multiple agents work on different issues simultaneously on the same machine
2. **Maintain safety**: Prevent conflicting edits through lease-based coordination
3. **Preserve simplicity**: Keep storage plaintext JSON, git-versionable
4. **Support recovery**: Handle crashes and stale claims gracefully
5. **Audit trail**: All coordination actions logged for observability

## Solution Architecture

### Two-Tier Storage Model

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

## Identity System

### Agent Identity

**Format:** `{type}:{identifier}`

**Examples:**
- `agent:copilot-1` - GitHub Copilot session 1
- `agent:cursor-main` - Cursor editor instance
- `human:alice` - Human user Alice
- `ci:github-actions` - CI/CD pipeline

**Persistence:**
- Environment variable: `JIT_AGENT_ID`
- Config file: `~/.config/jit/agent.toml`
- Per-session override: `--agent-id` flag

**Stability:** Agent ID should persist across process restarts for the same logical agent/session.

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

### Lease Identity

**Format:** ULID (Universally Unique Lexicographically Sortable Identifier)

**Example:** `01HXJK2M3N4P5Q6R7S8T9VWXYZ`

**Properties:**
- Sortable by creation time
- Globally unique
- 26 characters (URL-safe Base32)

## Claim/Lease Semantics

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

### Single-Writer Policy

**Per-issue structural edits require an active lease:**
- Creating/updating/deleting issue
- Adding/removing dependencies
- Adding/removing gates
- Changing issue state

**Multi-writer allowed (CRDT-based, future):**
- Free-text description edits (Automerge CRDT)
- Comment additions (append-only)

### Lease Operations

**Acquire:**
1. Check if issue is unleased or lease expired
2. Create new lease with TTL
3. Append to `claims.jsonl`
4. Update `claims.index.json`
5. Return lease to caller

**Renew (Heartbeat):**
1. Find active lease by ID
2. Extend `expires_at` by TTL
3. Append renew operation to `claims.jsonl`
4. Update `claims.index.json`

**Release:**
1. Find active lease by ID
2. Append release operation to `claims.jsonl`
3. Remove from `claims.index.json`

**Force Evict:**
1. Admin/operator force-releases a lease
2. Append force-evict operation with reason
3. Remove from `claims.index.json`
4. Log warning event

### Automatic Expiration

**On every claim operation:**
1. Load `claims.index.json`
2. Filter out expired leases (now > expires_at)
3. For each expired lease, append auto-evict to `claims.jsonl`
4. Write updated index

**Eviction is lazy:** Only happens during claim operations, not background process (for simplicity).

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
```

**Immutability:** Never modify or delete entries. Append-only for audit trail.

### Claims Index JSON (Derived View)

**Location:** `.git/jit/claims.index.json`

**Schema:**
```json
{
  "schema_version": 1,
  "generated_at": "2026-01-03T12:05:00Z",
  "sequence": 42,
  "active": [
    {
      "lease_id": "01HXJK2M3N4P5Q",
      "issue_id": "ad601a15",
      "agent_id": "agent:copilot-1",
      "worktree_id": "wt:abc123ef",
      "branch": "agents/copilot-1",
      "expires_at": "2026-01-03T12:15:00Z"
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
        '.active[] | select(.issue_id==$issue and .agent_id==$agent and .worktree_id==$wt) | .lease_id' \
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

## CLI Surface

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

**`jit worktree` subcommand group:**

```bash
# Show current worktree info
jit worktree info [--json]

# List all worktrees (from git)
jit worktree list [--json]

# Initialize worktree (manual)
jit worktree init
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

## Refinements and Additional Considerations

### Policy and Invariants

**CLI-Level Enforcement (Primary):**
```rust
// CommandExecutor enforces lease requirement BEFORE operation
impl CommandExecutor {
    pub fn update_issue(&self, id: &str, params: UpdateParams) -> Result<()> {
        // Require active lease for structural edits
        self.require_active_lease(id, "update issue")?;
        
        // Proceed with update
        self.storage.update_issue(id, params)
    }
    
    fn require_active_lease(&self, issue_id: &str, operation: &str) -> Result<()> {
        let agent_id = self.get_agent_id()?;
        let worktree_id = self.get_worktree_id()?;
        
        if !self.claim_coordinator.has_active_lease(issue_id, &agent_id, &worktree_id)? {
            bail!(
                "Operation '{}' on issue {} requires active lease.\n\
                 Acquire: jit claim acquire {} --ttl 600",
                operation, issue_id, issue_id
            );
        }
        
        Ok(())
    }
}
```

**Invariants to codify:**
- **One active lease per issue:** Enforced in `acquire_claim()` and verified in `verify_index_consistency()`
- **Lease ownership:** Only the lease holder (agent + worktree) can renew or release
- **Structural edits require lease:** CLI checks before hooks (defense in depth)

**Global Operations Divergence Gate:**
```rust
pub fn validate_divergence(&self) -> Result<()> {
    let merge_base = self.git_merge_base("HEAD", "origin/main")?;
    let main_commit = self.git_rev_parse("origin/main")?;
    
    if merge_base != main_commit {
        bail!(
            "Branch diverged from origin/main.\n\
             Global operations require common history.\n\
             Run: git rebase origin/main"
        );
    }
    Ok(())
}

// Expose as CLI command
pub fn cmd_validate(args: ValidateArgs) -> Result<()> {
    if args.divergence {
        coordinator.validate_divergence()?;
        println!("✓ Branch is up-to-date with origin/main");
    }
    Ok(())
}
```

### Atomicity and Durability Enhancements

**Directory fsync after rename:**
```rust
fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
    let index_path = self.paths.shared_jit.join("claims.index.json");
    let temp_path = index_path.with_extension("tmp");
    
    // Write temp file
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&temp_path, json)?;
    
    // Fsync temp file
    let file = File::open(&temp_path)?;
    file.sync_all()?;
    drop(file);
    
    // Atomic rename
    fs::rename(&temp_path, &index_path)?;
    
    // IMPORTANT: Fsync parent directory to ensure rename is durable
    let parent_dir = File::open(index_path.parent().unwrap())?;
    parent_dir.sync_all()?;
    
    Ok(())
}
```

**Monotonic time for TTL checks:**
```rust
use std::time::Instant;

pub struct Lease {
    // Store creation instant for monotonic expiry checks
    acquired_instant: Instant,
    
    // Store UTC for audit trail and display
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub ttl_secs: u64,
}

impl Lease {
    pub fn is_expired(&self) -> bool {
        // Use monotonic clock for expiry check (immune to wall-clock jumps)
        self.acquired_instant.elapsed().as_secs() > self.ttl_secs
    }
}

// Serialize without Instant (not serializable)
// On load, reconstruct from expires_at and current time
```

**Sequence numbers for strict ordering:**
```json
{"schema_version":1,"sequence":1,"op":"acquire","lease_id":"01HX...","issue_id":"ad601a15",...}
{"schema_version":1,"sequence":2,"op":"renew","lease_id":"01HX...",...}
{"schema_version":1,"sequence":3,"op":"release","lease_id":"01HX...",...}
```

### Identity Refinements

**Agent identity provenance:**
```toml
# ~/.config/jit/agent.toml
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"

# Session override via env
# JIT_AGENT_ID=agent:session-xyz jit claim acquire ...
```

**Worktree relocation detection:**
```rust
pub fn load_worktree_identity(paths: &WorktreePaths) -> Result<WorktreeIdentity> {
    let wt_file = paths.local_jit.join("worktree.json");
    let mut wt: WorktreeIdentity = serde_json::from_str(&fs::read_to_string(&wt_file)?)?;
    
    // Check if worktree was moved
    let current_root = paths.worktree_root.to_string_lossy().to_string();
    if wt.root != current_root {
        warn!("Worktree relocated: {} -> {}", wt.root, current_root);
        
        // Update location
        wt.root = current_root;
        wt.relocated_at = Some(Utc::now());
        
        // Write updated identity
        fs::write(&wt_file, serde_json::to_string_pretty(&wt)?)?;
    }
    
    Ok(wt)
}
```

### Lease Context Stamping

**Commit message trailers (audit trail):**
```rust
pub fn commit_with_lease_context(
    &self,
    message: &str,
    lease: &Lease,
) -> Result<()> {
    let full_message = format!(
        "{}\n\n\
         JIT-Lease: {}\n\
         JIT-Agent: {}\n\
         JIT-Worktree: {}",
        message,
        lease.lease_id,
        lease.agent_id,
        lease.worktree_id
    );
    
    // Git commit with trailers
    Command::new("git")
        .args(["commit", "-m", &full_message])
        .status()?;
    
    Ok(())
}
```

**Server-side verification (optional):**
```bash
# .git/hooks/pre-receive (for local bare origin)
while read old_sha new_sha ref_name; do
  # Extract lease info from commit trailers
  lease_id=$(git log --format='%(trailers:key=JIT-Lease,valueonly)' -1 $new_sha)
  
  if [ -n "$lease_id" ]; then
    # Verify lease was valid at commit time
    # (requires claims.jsonl access on server)
  fi
done
```

### File Formats and Merge Strategies

**`.gitattributes` for safe merges:**
```gitattributes
# Union merge for append-only logs
*.jsonl merge=union
**/*-set.jsonl merge=union

# CRDT merge driver for descriptions (future)
**/description.automerge merge=automerge

# Never merge binary indices
**/*.sqlite binary
```

**Unified event schema:**
```json
{
  "schema_version": 1,
  "sequence": 42,
  "type": "claim.acquired",
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

### Configuration and Toggles

**Repository config (`.jit/config.toml`):**
```toml
[worktree]
mode = "auto"  # auto | on | off
enforce_leases = "strict"  # strict | warn | off

[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
lease_renewal_threshold_pct = 10  # Warn when <10% TTL left

[global_operations]
require_main_history = true  # Enforce divergence gate
allowed_branches = ["main", "develop"]  # Can edit global config
```

**CLI flags for overrides:**
```bash
jit claim acquire <issue> --ttl 1200
jit claim acquire <issue> --agent-id agent:override-session
jit issue update <issue> --strict  # Force lease check even if mode=warn
```

### Recovery and Housekeeping

**Enhanced stale lock cleanup:**
```rust
pub struct LockMetadata {
    pub pid: u32,
    pub created_at: DateTime<Utc>,
    pub agent_id: String,
}

pub fn cleanup_stale_locks(&self) -> Result<()> {
    let lock_dir = self.paths.shared_jit.join("locks");
    
    for entry in fs::read_dir(&lock_dir)? {
        let path = entry.path();
        let metadata_path = path.with_extension("meta");
        
        if let Ok(meta_json) = fs::read_to_string(&metadata_path) {
            let meta: LockMetadata = serde_json::from_str(&meta_json)?;
            
            // Check if process still exists
            if !process_exists(meta.pid) {
                warn!("Removing lock from dead process {}: {}", meta.pid, path.display());
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(&metadata_path);
                continue;
            }
            
            // Check TTL (1 hour max)
            let age = Utc::now().signed_duration_since(meta.created_at);
            if age.num_seconds() > 3600 {
                warn!("Removing stale lock ({}s old): {}", age.num_seconds(), path.display());
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(&metadata_path);
            }
        }
    }
    
    Ok(())
}
```

**Automatic index rebuild triggers:**
```rust
pub fn load_claims_index(&self) -> Result<ClaimsIndex> {
    let index_path = self.paths.shared_jit.join("claims.index.json");
    
    if !index_path.exists() {
        info!("Claims index not found, rebuilding from log");
        return self.rebuild_index_from_log();
    }
    
    match fs::read_to_string(&index_path)
        .and_then(|s| Ok(serde_json::from_str::<ClaimsIndex>(&s)?))
    {
        Ok(index) => {
            // Verify schema version
            if index.schema_version != CURRENT_SCHEMA_VERSION {
                warn!("Schema version mismatch, rebuilding index");
                return self.rebuild_index_from_log();
            }
            
            // Verify consistency
            if !self.verify_index_consistency_quick(&index)? {
                warn!("Index consistency check failed, rebuilding");
                return self.rebuild_index_from_log();
            }
            
            Ok(index)
        }
        Err(e) => {
            error!("Failed to load index: {}, rebuilding", e);
            self.rebuild_index_from_log()
        }
    }
}
```

### Additional Hooks and Server-Side Checks

**Pre-push hook (strict teams):**
```bash
#!/usr/bin/env bash
# .git/hooks/pre-push
set -euo pipefail

# Verify all committed leases are still valid
for commit in $(git rev-list @{u}..HEAD); do
  issue_files=$(git diff-tree --no-commit-id --name-only -r $commit | grep 'issues/.*/issue.json' || true)
  
  for file in $issue_files; do
    issue_id=$(echo "$file" | sed -E 's#.*/issues/([^/]+)/.*#\1#')
    
    # Extract lease from commit message
    lease_id=$(git log --format='%(trailers:key=JIT-Lease,valueonly)' -1 $commit)
    
    if [ -n "$lease_id" ]; then
      # Verify lease exists in current index
      exists=$(jq -r --arg lid "$lease_id" '.active[] | select(.lease_id==$lid) | .lease_id' .git/jit/claims.index.json 2>/dev/null || echo "")
      
      if [ -z "$exists" ]; then
        echo "❌ Warning: Lease $lease_id in commit $commit is no longer active"
        echo "   Consider re-acquiring lease before pushing"
      fi
    fi
  done
done
```

### Edge Cases and Long-Running Work

**Lease renewal warnings:**
```rust
pub fn check_lease_expiry_warning(&self, lease: &Lease) -> Option<String> {
    let remaining_secs = lease.ttl_secs.saturating_sub(
        lease.acquired_instant.elapsed().as_secs()
    );
    
    let threshold_secs = (lease.ttl_secs * 10) / 100;  // 10% of TTL
    
    if remaining_secs < threshold_secs {
        return Some(format!(
            "⚠️  Lease {} expires in {}s. Renew: jit claim renew {} --ttl {}",
            lease.lease_id,
            remaining_secs,
            lease.lease_id,
            lease.ttl_secs
        ));
    }
    
    None
}
```

**Automatic heartbeat thread (opt-in):**
```rust
pub fn start_heartbeat_daemon(
    agent_id: String,
    paths: WorktreePaths,
    interval_secs: u64,
) -> Result<JoinHandle<()>> {
    let handle = thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(interval_secs));
            
            // Update heartbeat file
            if let Err(e) = update_heartbeat(&agent_id, &paths) {
                error!("Heartbeat update failed: {}", e);
            }
            
            // Auto-renew active leases
            if let Err(e) = auto_renew_leases(&agent_id, &paths) {
                error!("Lease renewal failed: {}", e);
            }
        }
    });
    
    Ok(handle)
}
```

### Integration Points

**Web UI endpoints:**
```rust
// jit-server routes
#[get("/api/claims/status")]
async fn get_claim_status() -> Json<ClaimStatus> {
    let coordinator = ClaimCoordinator::new(...);
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
```

**Dispatcher integration:**
```rust
// jit-dispatch agent wrapper
impl AgentExecutor {
    pub async fn execute_with_lease(&self, issue_id: &str) -> Result<()> {
        // Acquire lease
        let lease = self.coordinator.acquire_claim(
            issue_id,
            &self.agent_id,
            self.config.default_ttl,
        )?;
        
        // Start heartbeat daemon
        let _heartbeat = start_heartbeat_daemon(
            self.agent_id.clone(),
            self.paths.clone(),
            30,
        )?;
        
        // Execute work
        let result = self.execute_issue(issue_id).await;
        
        // Release lease
        self.coordinator.release_claim(&lease.lease_id)?;
        
        result
    }
}
```

**Lease transfer for handoff:**
```rust
pub fn transfer_lease(
    &self,
    lease_id: &str,
    to_agent_id: &str,
    to_worktree_id: &str,
) -> Result<Lease> {
    let lock_path = self.paths.shared_jit.join("locks/claims.lock");
    let _guard = self.locker.lock_exclusive(&lock_path)?;
    
    let mut index = self.load_claims_index()?;
    
    // Find existing lease
    let old_lease = index.find_lease(lease_id)
        .ok_or_else(|| anyhow!("Lease {} not found", lease_id))?;
    
    // Create new lease with same issue, different owner
    let new_lease = Lease {
        lease_id: Ulid::new().to_string(),
        issue_id: old_lease.issue_id.clone(),
        agent_id: to_agent_id.to_string(),
        worktree_id: to_worktree_id.to_string(),
        branch: self.get_current_branch()?,
        ttl_secs: old_lease.ttl_secs,
        acquired_at: Utc::now(),
        expires_at: Utc::now() + Duration::seconds(old_lease.ttl_secs as i64),
    };
    
    // Log transfer
    self.append_claim_op(&ClaimOp::Transfer {
        from_lease_id: lease_id.to_string(),
        to_lease: new_lease.clone(),
        transferred_at: Utc::now(),
    })?;
    
    // Update index
    index.remove_lease(lease_id);
    index.add_lease(new_lease.clone());
    self.write_index_atomic(&index)?;
    
    Ok(new_lease)
}
```

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
3. **CRDT merge driver** for descriptions (future)
4. **Cross-machine coordination** via custom refs (future)

## Open Questions and Future Work

### Questions

1. **Claim visibility across machines?**
   - Current design: Local-only (`.git/jit/` not versioned)
   - Future: Use custom refs `refs/jit/claims/*` for replication?

2. **CRDT for multi-writer descriptions?**
   - Current: Single-writer via leases
   - Future: Automerge CRDT for concurrent free-text edits?

3. **Heartbeat daemon vs. manual renewal?**
   - Current: Both supported (manual + opt-in daemon)
   - Future: Auto-start daemon based on config?

### Future Enhancements

- **Web UI integration:** Show active claims and worktree status
- **CI/CD coordination:** Prevent claim acquisition during CI runs
- **Claim history query:** `jit claim history --issue <id>`
- **Lease transfer:** `jit claim transfer <lease-id> --to <agent-id>` ✓ (designed above)
- **Global event aggregation:** Merge control-plane and data-plane events

## References

- [Git Worktree Documentation](https://git-scm.com/docs/git-worktree)
- [ULID Specification](https://github.com/ulid/spec)
- [fs4 Rust Crate](https://docs.rs/fs4/) - File locking
- [Automerge CRDT](https://automerge.org/) - Future multi-writer support

## Acceptance Criteria

- [ ] Multiple agents can work on different issues in parallel worktrees
- [ ] No file conflicts between concurrent agents
- [ ] Lease acquisition is atomic and race-free
- [ ] Expired leases are automatically evicted
- [ ] Pre-commit hook prevents unauthorized structural edits
- [ ] Crash recovery rebuilds index from audit log
- [ ] All operations logged for observability
- [ ] Documentation covers setup and usage
- [ ] Integration tests validate concurrent scenarios
