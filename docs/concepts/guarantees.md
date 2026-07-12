# System Guarantees

> **Diátaxis Type:** Explanation  
> **Audience:** Users who need to understand JIT's reliability properties

This document explains what JIT guarantees about data integrity, consistency, and failure handling. Understanding these guarantees helps you build reliable workflows and troubleshoot issues.

## Invariants

JIT's project invariants are cited below by their address (`@/invariant/<id>`);
run `jit item show <address>` for the registry's canonical statement. The
invariant registry (`.jit/invariants.toml`) renders into the invariant region of
the project `AGENTS.md` via `jit invariant render`.

### DAG Property

**Guarantee:** Dependencies always form a directed acyclic graph (DAG) - cycles are strictly prevented. (`@/invariant/dag-acyclic`)

Dependencies in JIT represent "FROM depends on TO" relationships. If issue A depends on B, then A cannot proceed until B reaches a terminal state (done or rejected). To prevent deadlock, JIT enforces that the dependency graph is always acyclic.

**How it works:**

JIT uses depth-first search (DFS) to detect potential cycles before adding any dependency:

```rust
// Simplified algorithm from crates/jit/src/graph/mod.rs
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

- **No deadlocks:** Issues can always make progress once dependencies reach a terminal state
- **Clear work order:** Topological sort determines execution order
- **Predictable scheduling:** Agents can identify ready work deterministically

**Transitive reduction:**

JIT keeps the dependency graph transitively reduced. When `A→B→C` already holds, adding the redundant `A→C` is rejected by default, and `jit validate` enforces the reduced form. `jit dep add --reduce` instead accepts the new edge and drops whatever it makes redundant, in the same operation. Either way the graph represents the *minimal* set of relationships needed.

### Atomic Operations

**Guarantee:** Replacement writes are published atomically through the storage
layer's temp-file-and-rename path. (`@/invariant/atomic-writes`)

The storage layer writes a unique temporary file in the target's directory and
renames it onto the target. Keeping both paths in one directory makes the rename
a same-filesystem operation, so readers do not observe a partially written
replacement (`crates/jit/src/storage/atomic_write.rs`).

**How it works:**

The atomic-write primitive gives each temporary file a process-and-call-specific
name, avoiding collisions between writers before rename. It is an atomic file
replacement mechanism, not a multi-file transaction.

`events.jsonl` follows a different persistence path: JIT opens it in append mode
and serializes each append with the repository write lock and `.events.lock`
(`crates/jit/src/storage/json.rs`). It is a locked append, not a replacement-file
rename.

**Multi-agent safety:**

JIT also uses advisory file locks to coordinate operations that share mutable
state. The locks are released with their guards and work only among cooperating
processes; they complement, rather than extend, an atomic rename into a global
transaction. Advisory work-lease coordination uses `.git/jit/locks/claims.lock`
(`crates/jit/src/storage/lock.rs`).

**Example - Two agents claiming simultaneously:**

```mermaid
sequenceDiagram
    participant A1 as Agent 1
    participant L as locks/claims.lock
    participant A2 as Agent 2
    A1->>L: jit claim acquire abc123
    A2->>L: jit claim acquire abc123
    L-->>A1: lock acquired
    Note over A2: blocked waiting for lock
    A1->>A1: read claims index
    A1->>A1: verify no active lease
    A1->>A1: write new claim
    A1->>L: release lock
    L-->>A2: lock acquired
    A2->>A2: read claims index
    A2->>A2: verify no active lease
    Note over A2: ERROR: Already claimed
```

**Benefits:**

- **No partial replacement:** Readers do not observe a partially written target file.
- **Coordinated operations:** Cooperating JIT processes use advisory locks where shared access must be serialized.

### Event Logging

**Guarantee:** Every issue state change appends an event to `.jit/events.jsonl`.
(`@/invariant/event-log`)

This guarantee is about lifecycle state changes. Other operations may emit their
own event variants, but the event log is not a promise that every repository
mutation is recorded or that it reconstructs complete repository history.

**Event types:**

Each event is tagged by a snake-case `type` field. The authoritative and
evolving set of event variants is the `Event` enum in
`crates/jit/src/domain/types.rs`, including the `issue_state_changed` variant
used for lifecycle transitions. Consult that enum rather than relying on a
hand-maintained list in prose.

**Log format:**

Events are stored as newline-delimited JSON (JSONL):

```jsonl
{"type":"issue_created","id":"evt-001","issue_id":"abc123","timestamp":"2026-02-02T20:00:00Z","title":"Fix bug","priority":"high"}
{"type":"issue_claimed","id":"evt-002","issue_id":"abc123","timestamp":"2026-02-02T20:01:00Z","assignee":"agent:worker-1"}
{"type":"issue_state_changed","id":"evt-003","issue_id":"abc123","timestamp":"2026-02-02T20:05:00Z","from":"ready","to":"in_progress"}
```

**Properties:**

- **Append-only:** Events are appended to the log.
- **State-transition evidence:** The state-change invariant is recorded as
  `issue_state_changed` events.

**Benefits:**

- **Observability:** Inspect recorded lifecycle transitions for an issue.
- **Lifecycle timestamps:** `jit migrate lifecycle-timestamps` can backfill
  selected timestamps from recorded `issue_state_changed` events.

**Query examples:**

```bash
# View recent activity
jit events tail -n 20

# Find all events for specific issue
jit events query --issue-id abc123

# Track state transitions
jit events query --event-type issue_state_changed
```

### Git Optional

**Guarantee:** Core issue tracking works without git - repository versioning is optional.

JIT is designed as a standalone issue tracker that *enhances* git workflows but doesn't require them. This supports use cases beyond software development.

Issue assignment (`jit issue assign` / `jit issue claim` / `jit issue release` / `jit issue unassign`) is bookkeeping on the issue record and works fully without git. Advisory work leases (`jit claim acquire` and its sibling `jit claim` subcommands) coordinate exclusive, time-boxed access across worktrees; they need a git repository with a resolvable `HEAD` for worktree identity and branch tracking. A missing repository and a repository with zero commits both fail with a typed `ClaimRequiresGitError` (exit code 10): `git init` alone is not enough, since `HEAD` does not resolve to a branch until the first commit exists.

**What works without git:**

✅ Issue creation, updates, queries  
✅ Dependency management  
✅ Quality gates (automated and manual)  
✅ Issue assignment (`jit issue assign` / `jit issue claim` / `jit issue release` / `jit issue unassign`)  
✅ `jit doc archive` (subject to the repository's documentation configuration and archive checks)<br>
✅ Event logging and queries  
✅ Status and visualization  

**What requires git:**

❌ `jit claim acquire` / `release` / `renew` / `heartbeat` / `status` / `list` / `force-evict` - Advisory work leases (`ClaimRequiresGitError`, exit code 10)  
❌ `jit doc show --at <commit>` - View document at specific git revision  
❌ `jit snapshot export --at <tag>` - Export from specific git revision  
❌ Document asset validation from git history  

**Fallback behavior:**

When git is unavailable:

- **Document operations:** Fall back to working tree only
- **Filesystem-backed documents:** `jit doc add`, `jit doc list`, `jit doc archive`,
  and `jit doc show` without a commit reference remain available; history, diff,
  and commit-specific reads require git
- **Snapshot export:** Export from current working tree
- **History commands:** Return error with helpful message
- **Advisory leases:** Fail outright with `ClaimRequiresGitError` (exit code 10) instead of falling back; use issue assignment (`jit issue claim`) as the git-free alternative

**Example:**

```bash
# Without git - core functionality and issue assignment work
mkdir my-project && cd my-project
jit init
jit issue create --title "Task 1"
jit issue create --title "Task 2"
jit dep add <task1> <task2>
jit query available
jit issue claim <task2> agent:worker-1
# ✓ All basic operations and issue assignment work (task2 has no unmet dependencies)

jit claim acquire <task2> --agent-id agent:worker-1
# ✗ Error: Claims and leases require a git repository (exit code 10)

# git init alone is not enough - HEAD must resolve to a commit
git init
git config user.email "you@example.com"
git config user.name "Your Name"
jit claim acquire <task2> --agent-id agent:worker-1
# ✗ Error: Claims and leases require a git repository (exit code 10) - no commits yet

git commit --allow-empty -m "Initial commit"
jit claim acquire <task2> --agent-id agent:worker-1
# ✓ Advisory lease acquired

mkdir -p path/to
echo "Initial design notes" > path/to/design.md
jit doc add <task2> path/to/design.md
git add -A && git commit -m "Add design doc"
echo "Revised notes" >> path/to/design.md
git add -A && git commit -m "Revise design doc"
jit doc show <task2> path/to/design.md --at HEAD~1
# ✓ Shows the version from before the revision
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

**2. Scoped atomic operations**

Replacement-file writes, locked event appends, and lease acquisition each have
their own scope; none makes unrelated repository updates one transaction:

```bash
# Agent 1 acquires an advisory lease atomically (in a Git worktree).
jit claim acquire abc123
# Agent 2's simultaneous claim will either succeed or fail cleanly
# The lease operation does not make ordinary issue claims atomic.
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
- The event log records state-transition events; it is not a cross-operation ordering guarantee

❌ **Automatic conflict resolution**  
- Concurrent writers race on the whole issue file: the last atomic rename wins, with no field-level merge
- Ordinary `jit issue claim` reads the existing assignee and any lease only to reject a conflict it can see, not to serialize; `jit claim acquire` holds an exclusive lock for that
- No CRDTs or operational transformation

### Implications for Multi-Agent Workflows

**✓ Safe patterns:**

```bash
# Issue assignment works without git, but it is not atomic coordination.
jit issue claim <issue> agent:worker-1
# Work on issue
jit issue update <issue> --state done

# Polling for ready work (eventually consistent)
while true; do
  jit query available --json | process_available_work
  sleep 5
done

# Event-driven workflows (poll the event tail for new events)
while true; do
  jit events tail -n 10 --json | react_to_events
  sleep 5
done
```

**✗ Unsafe patterns:**

```bash
# Assuming state without claiming
state=$(jit issue show abc123 --json | jq -r '.state')
# ✗ State may change before next operation
jit issue update abc123 --state in_progress  # Race condition!

# Better: claim to record ownership; for guaranteed exclusivity acquire a lease
jit issue claim abc123 agent:worker-1  # Rejects a claim it can see is already assigned, but does not lock
jit claim acquire abc123 --agent-id agent:worker-1  # Exclusive, serialized (Git worktrees)
```

### File-Based Synchronization

All coordination happens through filesystem operations:

Two directories at the repository root carry coordination state. `.jit/` holds
per-worktree issue data; `.git/jit/` is the control plane shared across every
worktree of the repository.

```
.jit/
├── issues/{id}.json          # Issue data
├── index.json                # Issue index
├── gates.toml                # Gate registry
└── events.jsonl              # Append-only event log

.git/jit/
├── claims.jsonl              # Claim log (append-only)
├── claims.index.json         # Active claims (exclusive lock)
├── heartbeat/                # Initialized but empty; heartbeats append to claims.jsonl and update the index
└── locks/claims.lock         # Advisory lock guarding claim-log operations
```

**Synchronization points:**

1. **Advisory file locks** - Coordinate operations that acquire the same lock
2. **Atomic renames** - Publish updates atomically
3. **Claims** - Establish exclusive access boundaries
4. **Event log** - Record state-transition events

## Failure Modes

JIT is designed to handle failures gracefully without data loss or corruption.

### Partial Write Recovery

**Scenario:** Process crashes during file write.

**Recovery:**

Atomic replacement writes use a same-directory temporary file and rename:

```bash
# During an interrupted replacement:
.jit/issues/.abc123.json.<pid>.<sequence>.tmp  # Incomplete temporary file
.jit/issues/abc123.json                        # Previous version intact

# Run recovery to clean provably stale temporary files and locks.
jit recover
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

**Impact:** Harmless. Read operations only consider `.json` files, so an orphaned
`.tmp` sibling is never served and never corrupts state.

**Cleanup:**

```bash
# jit recover removes orphaned .tmp files older than one hour (ordinary claim
# commands do not run temp-file cleanup).

# Manual cleanup at any time
find .jit -name '*.tmp' -mmin +60 -delete
```

The one-hour age is the default temp-file cleanup threshold — see
[Runtime Coordination Defaults](../reference/runtime-defaults.md), the
generated source of that value.

### Stale Claim Leases

**Scenario:** Agent crashes while holding a claim.

**Detection:**

```bash
jit claim list
Lease: lease-001
  Issue: abc123
  Agent: agent:worker-1
  Expires: <timestamp in the past> ⚠️ STALE
```

**Recovery:**

```bash
# An expired lease is evicted during a later lease acquisition, so another agent
# can acquire a new lease. The issue assignee is stored separately: a Ready issue
# with any assignee is still excluded from `jit query available`.
jit claim acquire abc123 --agent-id agent:worker-2

# Clear the current assignee before looking for unassigned ready work.
jit issue release abc123 "return to the available pool"
# Or: jit issue unassign abc123
jit query available

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

- Corrupted issue → Only that issue affected, others work normally (`list_issues` skips an unreadable file)
- Missing git → Core issue operations and filesystem-backed document operations
  still work; leases, document history/diff, and commit-specific document reads
  are unavailable
- Expired finite lease → evicted during a later lease acquisition; a stale
  indefinite lease is not auto-evicted and remains until a heartbeat,
  `jit claim release`, or `jit claim force-evict`. Either way the issue stays
  assigned until `jit issue release` or `jit issue unassign` clears its assignee
- Malformed `events.jsonl` line → event *reads* fail fast with a parse error (never silently skipped); issue operations, which append rather than re-read the log, keep working

**✓ No cascading failures:**

- Storage errors don't crash the CLI (return error codes)
- Validation errors suggest recovery steps
- Lock timeouts prevent indefinite hangs (see the [default lock acquisition timeout](../reference/runtime-defaults.md))
- Event log corruption doesn't prevent issue operations

**✓ Recovery-oriented design:**

```bash
# Comprehensive health check
jit validate
# Reports validation findings (hierarchy, dependency, and format issues).

# Include lease consistency
jit validate --leases
# Lists expired or dangling leases, each with the exact fix command
# (jit claim release / jit claim force-evict).

# Auto-fix what is mechanically fixable
jit validate --fix
# ✓ Applies type-hierarchy, transitive-reduction, and pending-state-transition fixes
```

## See Also

- [Core Model](core-model.md) - Understanding issues, dependencies, and gates
- [Design Philosophy](design-philosophy.md) - Why these guarantees matter
- [Troubleshooting Guide](../how-to/troubleshooting.md) - Practical recovery procedures
- Implementation: `crates/jit/src/storage/` - Atomic operations and locking
- Implementation: `crates/jit/src/graph/mod.rs` - Cycle detection algorithm
