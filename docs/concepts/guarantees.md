# System Guarantees

> **Diátaxis Type:** Explanation  
> **Audience:** Users who need to understand JIT's reliability properties

This document explains what JIT guarantees about data integrity, consistency, and failure handling. Understanding these guarantees helps you build reliable workflows and troubleshoot issues.

## Invariants

JIT maintains four core invariants that are enforced at all times:

### DAG Property

**Guarantee:** Dependencies always form a directed acyclic graph (DAG) - cycles are strictly prevented.

Dependencies in JIT represent "FROM depends on TO" relationships. If issue A depends on B, then A cannot complete until B is done. To prevent deadlock, JIT enforces that the dependency graph is always acyclic.

**How it works:**

JIT uses depth-first search (DFS) to detect potential cycles before adding any dependency:

```rust
// Simplified algorithm from crates/jit/src/graph.rs
fn would_create_cycle(from: &str, to: &str) -> bool {
    // Adding edge from → to creates a cycle if there's already a path to → from
    // In other words: if 'from' is reachable from 'to'
    is_reachable(to, from)
}
```

**Example:**

```bash
# Create issues
jit issue create --title "Task A"  # → a1b2c3
jit issue create --title "Task B"  # → d4e5f6
jit issue create --title "Task C"  # → g7h8i9

# Build dependency chain: A ← B ← C
jit dep add a1b2c3 d4e5f6  # A depends on B ✓
jit dep add d4e5f6 g7h8i9  # B depends on C ✓

# Try to create a cycle: C ← A
jit dep add g7h8i9 a1b2c3  # ✗ ERROR: Cycle detected
```

**Why this matters:**

- **No deadlocks:** Issues can always make progress when dependencies complete
- **Clear work order:** Topological sort determines execution order
- **Predictable scheduling:** Agents can identify ready work deterministically

**Transitive reduction:**

While JIT allows transitive dependencies (A→B→C and A→C simultaneously), minimal edges are preferred for clarity. The graph always represents the *minimal* set of relationships needed.

### Atomic Operations

**Guarantee:** All file writes are atomic - either the entire write succeeds or nothing changes.

JIT uses the write-temp-rename pattern for all file operations. This leverages the POSIX guarantee that `rename()` is atomic at the filesystem level.

**How it works:**

```rust
// From crates/jit/src/storage/json.rs
fn write_json<T>(path: &Path, data: &T) -> Result<()> {
    let json = serialize(data)?;
    
    // Write to temporary file
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, json)?;
    
    // Atomic rename (POSIX guarantee)
    fs::rename(&temp_path, path)?;
    
    Ok(())
}
```

**Multi-agent safety:**

File locking prevents race conditions during concurrent updates:

- **Index updates:** Exclusive lock on `.index.lock`
- **Issue updates:** Per-issue lock on `.issues/{id}.lock`
- **Claim operations:** Exclusive lock on `claims.index.lock`
- **Event log:** Exclusive lock on `.events.lock`

**Example - Two agents claiming simultaneously:**

```bash
# Agent 1                          # Agent 2
jit claim acquire abc123           jit claim acquire abc123
  ↓ acquire claims.index.lock        ↓ (blocked waiting for lock)
  ↓ read claims index
  ↓ verify issue unassigned
  ↓ write new claim
  ↓ release lock                     ↓ acquire claims.index.lock
                                     ↓ read claims index
                                     ↓ verify issue unassigned
                                     ✗ ERROR: Already claimed
```

**Benefits:**

- **No partial writes:** Crashes never leave corrupted JSON files
- **No lost updates:** File locks serialize concurrent modifications
- **Crash safety:** Temp files cleaned up automatically on next operation

### Event Logging

**Guarantee:** All state changes are logged to `.jit/events.jsonl` as an append-only audit trail.

Every operation that modifies issue state, dependencies, or gates emits an event. The event log provides complete observability and supports future undo/replay capabilities.

**Event types:**

```
IssueCreated          - New issue created
IssueClaimed          - Agent claimed issue
IssueUnclaimed        - Issue released
IssueUpdated          - State, priority, or assignee changed
DependencyAdded       - Dependency relationship added
DependencyRemoved     - Dependency relationship removed
GatePassed            - Quality gate marked passed
GateFailed            - Quality gate marked failed
GateChecked           - Automated gate execution completed
```

**Log format:**

Events are stored as newline-delimited JSON (JSONL):

```jsonl
{"IssueCreated":{"id":"evt-001","issue_id":"abc123","timestamp":"2026-02-02T20:00:00Z","title":"Fix bug","priority":"high"}}
{"IssueClaimed":{"id":"evt-002","issue_id":"abc123","timestamp":"2026-02-02T20:01:00Z","assignee":"agent:worker-1"}}
{"IssueUpdated":{"id":"evt-003","issue_id":"abc123","timestamp":"2026-02-02T20:05:00Z","field":"state","old_value":"ready","new_value":"in_progress"}}
```

**Properties:**

- **Append-only:** Events are never modified or deleted
- **Ordered:** Timestamp establishes causal ordering
- **Complete:** Every mutation is logged
- **Durable:** Atomic append with file locking

**Benefits:**

- **Observability:** Debug workflows by examining event history
- **Audit trail:** Compliance requirements satisfied
- **Future capabilities:** Undo operations, replay history, time-travel debugging

**Query examples:**

```bash
# View recent activity
jit events tail -n 20

# Find all events for specific issue
jit events query --issue-id abc123

# Track state transitions
jit events query --event-type IssueUpdated
```

### Git Optional

**Guarantee:** Core issue tracking works without git - repository versioning is optional.

JIT is designed as a standalone issue tracker that *enhances* git workflows but doesn't require them. This supports use cases beyond software development.

**What works without git:**

✅ Issue creation, updates, queries  
✅ Dependency management  
✅ Quality gates (automated and manual)  
✅ Agent claiming and coordination  
✅ Event logging and queries  
✅ Status and visualization  

**What requires git:**

❌ `jit doc show --at <commit>` - View document at specific git revision  
❌ `jit doc archive` - Track document history across moves  
❌ `jit snapshot export --at <tag>` - Export from specific git revision  
❌ Document asset validation from git history  

**Fallback behavior:**

When git is unavailable:

- **Document operations:** Fall back to working tree only
- **Snapshot export:** Export from current working tree
- **History commands:** Return error with helpful message

**Example:**

```bash
# Without git - core functionality works
mkdir my-project && cd my-project
jit init
jit issue create --title "Task 1"
jit issue create --title "Task 2"
jit dep add <task1> <task2>
jit query available
# ✓ All basic operations work

# With git - enhanced document management
git init
jit doc add <issue> path/to/design.md
jit doc show <issue> path/to/design.md --at HEAD~3
# ✓ History and versioning available
```

**Design rationale:**

Making git optional allows JIT to be used for:
- Research projects without version control
- Knowledge work and personal task management
- Environments where git isn't available
- Rapid prototyping and experimentation

## Consistency Model

JIT provides **eventual consistency** through file-based coordination. Understanding the consistency model helps you build reliable multi-agent workflows.

### Consistency Guarantees

**1. Read-your-own-writes**

A process always sees its own updates immediately:

```bash
# Same process (shell session)
jit issue update abc123 --state in_progress
jit issue show abc123
# ✓ Shows in_progress immediately
```

**2. Atomic operations**

Individual operations are isolated and atomic:

```bash
# Agent 1 claims atomically
jit claim acquire abc123
# Agent 2's simultaneous claim will either succeed or fail cleanly
# No partial state, no corruption
```

**3. Eventually consistent across processes**

Different processes see updates after file operations complete:

```bash
# Terminal 1                     # Terminal 2
jit issue update abc --state done
                                 jit query available
                                 # ✓ Sees updated state (file reread)
```

### What JIT Does NOT Guarantee

Understanding limitations prevents incorrect assumptions:

❌ **Strong consistency across processes**  
- Updates are not instantly visible to other processes
- File operations are the synchronization point
- Use claims for coordination, not assumptions about state

❌ **Distributed coordination**  
- JIT is designed for single-machine use
- No distributed locking or consensus
- Network filesystems may violate atomicity guarantees

❌ **Snapshot isolation**  
- Long-running operations may see intermediate state
- Use claims to establish boundaries
- Event log provides ordering guarantees

❌ **Automatic conflict resolution**  
- First-writer-wins for non-conflicting fields
- Claim operations detect conflicts explicitly
- No CRDTs or operational transformation

### Implications for Multi-Agent Workflows

**✓ Safe patterns:**

```bash
# Claim-based coordination (atomic)
jit claim acquire <issue>
# Work on issue
jit issue update <issue> --state done
jit claim release <lease-id>

# Polling for ready work (eventually consistent)
while true; do
  jit query available --json | process_available_work
  sleep 5
done

# Event-driven workflows
jit events tail -n 1 --follow | react_to_events
```

**✗ Unsafe patterns:**

```bash
# Assuming state without claiming
state=$(jit issue show abc123 --json | jq -r '.state')
# ✗ State may change before next operation
jit issue update abc123 --state in_progress  # Race condition!

# Correct approach: use atomic claim
jit claim acquire abc123  # Prevents concurrent modification
```

### File-Based Synchronization

All coordination happens through filesystem operations:

```
.jit/
├── issues/{id}.json          # Issue data (per-file locks)
├── index.json                # Issue index (exclusive lock)
├── gates.json                # Gate registry (exclusive lock)
├── events.jsonl              # Event log (append-only, locked)
└── .git/jit/                 # Shared control plane
    ├── claims.jsonl          # Claim log (append-only)
    ├── claims.index.json     # Active claims (exclusive lock)
    └── heartbeats/           # Lease keep-alive
```

**Synchronization points:**

1. **File locks** - Serialize updates to shared state
2. **Atomic renames** - Publish updates atomically
3. **Claims** - Establish exclusive access boundaries
4. **Event log** - Establish causal ordering

## Failure Modes

JIT is designed to handle failures gracefully without data loss or corruption.

### Partial Write Recovery

**Scenario:** Process crashes during file write.

**Recovery:**

Atomic operations (write-temp-rename) prevent partial writes:

```bash
# During write crash:
.jit/issues/abc123.json.tmp  # Incomplete temp file
.jit/issues/abc123.json      # Previous version intact

# On next operation:
jit validate  # Cleans up .tmp files older than 5 minutes
# OR
# Temp files ignored by read operations (only .json files read)
```

**Result:** No corruption, no data loss, previous state preserved.

### Corrupted JSON Detection

**Scenario:** Manual edit creates invalid JSON, or disk corruption occurs.

**Symptom:**

```bash
jit issue show abc123
Error: Failed to deserialize data: expected value at line 15 column 3
  File: .jit/issues/abc123.json
```

**Recovery options:**

1. **Restore from git:**
   ```bash
   git checkout HEAD -- .jit/issues/abc123.json
   ```

2. **Manual repair:**
   ```bash
   # Edit file with proper JSON syntax
   vim .jit/issues/abc123.json
   # Validate
   jit issue show abc123
   ```

3. **Check event log for last known state:**
   ```bash
   jit events query --issue-id abc123 | tail -n 10
   # Reconstruct from events
   ```

**Prevention:** Use `jit` commands instead of manual editing.

### Stale Temporary Files

**Scenario:** Crash leaves `.tmp` files behind.

**Detection:**

```bash
jit validate
Warning: Found stale temporary files (will be cleaned):
  .jit/issues/abc123.json.tmp (age: 2 hours)
```

**Recovery:**

```bash
# Automatic cleanup (removes files older than 5 minutes)
jit validate --fix

# Manual cleanup
find .jit -name '*.tmp' -mmin +5 -delete
```

**Impact:** Stale temp files are harmless - ignored by read operations, cleaned automatically.

### Stale Claim Leases

**Scenario:** Agent crashes while holding a claim.

**Detection:**

```bash
jit claim list
Lease: lease-001
  Issue: abc123
  Agent: agent:worker-1
  Expires: 2026-02-02T19:00:00Z (10 minutes ago) ⚠️ STALE
```

**Recovery:**

```bash
# Automatic recovery - expired leases are ignored
jit query available  # Shows issue as available

# Manual eviction if needed
jit claim force-evict lease-001 --reason "agent crashed"
```

**Prevention:** Use heartbeats to renew leases:

```bash
# Renew lease before expiry
jit claim renew lease-001 --extension 600
```

### Missing File Handling

**Scenario:** Issue file deleted manually or corrupted beyond recovery.

**Symptom:**

```bash
jit issue show abc123
Error: Issue not found: abc123
  Expected at: .jit/issues/abc123.json
```

**Recovery:**

1. **Check git history:**
   ```bash
   git log -- .jit/issues/abc123.json
   git checkout <commit> -- .jit/issues/abc123.json
   ```

2. **Recreate from event log:**
   ```bash
   jit events query --issue-id abc123
   # Use events to reconstruct state
   ```

3. **Manual recreation:**
   ```bash
   # Create new issue with same ID (requires manual JSON editing)
   # NOT RECOMMENDED - use git recovery
   ```

**Prevention:**

- Commit `.jit/` directory regularly
- Use `jit validate` to detect inconsistencies
- Never manually delete issue files

### Network Errors (External Assets)

**Scenario:** Document references external URL, but network is unavailable.

**Impact:** Document operations fail, but core issue tracking unaffected.

```bash
jit doc check-links --scope all
Warning: External URL not validated: https://example.com/diagram.png
  Referenced in: dev/design.md

# Core functionality still works
jit query available  # ✓ Works
jit issue update abc123 --state done  # ✓ Works
```

**Mitigation:** Download critical assets locally using per-document asset pattern.

### Graceful Degradation

JIT is designed with isolation and fault tolerance:

**✓ Isolated failures:**

- Corrupted issue → Only that issue affected, others work normally
- Missing git → Core features still work, document features disabled
- Stale lease → Automatically expired, issue becomes available
- Invalid event → Logged but doesn't block operations

**✓ No cascading failures:**

- Storage errors don't crash the CLI (return error codes)
- Validation errors suggest recovery steps
- Lock timeouts prevent indefinite hangs (default 5 seconds)
- Event log corruption doesn't prevent issue operations

**✓ Recovery-oriented design:**

```bash
# Comprehensive health check
jit validate
# Issues:
#   - 3 stale temp files (will clean)
#   - 1 expired lease (will evict)
# 
# Run with --fix to repair

# Automatic repair
jit validate --fix
# ✓ Cleaned 3 temp files
# ✓ Evicted 1 stale lease
# ✓ Rebuilt index
```

## See Also

- [Core Model](core-model.md) - Understanding issues, dependencies, and gates
- [Design Philosophy](design-philosophy.md) - Why these guarantees matter
- [Troubleshooting Guide](../how-to/troubleshooting.md) - Practical recovery procedures
- Implementation: `crates/jit/src/storage/` - Atomic operations and locking
- Implementation: `crates/jit/src/graph.rs` - Cycle detection algorithm
