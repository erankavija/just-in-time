# Session: Claim Coordination System - Parallel Implementation

**Story:** b74af86f - Story: Claim Coordination System  
**Date:** 2026-01-06  
**Status:** In Progress  
**Approach:** Parallel development with 3 agents using git worktrees

## Context

Implementing the lease-based claim coordination system following the successful Phase 1 (Foundation) experiment. We validated that git worktrees enable parallel work with minimal conflicts.

**Foundation Complete:**
- ✅ WorktreePaths detection (b69fa9a9)
- ✅ Worktree identity generation (5186a98d)
- ✅ Agent identity configuration (61783d2f)
- ✅ Control plane directory structure (565cda12)

## Parallel Work Plan

### Agent Assignments

**Agent 1 (Main Worktree):**
- **Task:** 5bc0eff5 - Implement claims JSONL append-only log
- **Location:** `/home/vkaskivuo/Projects/just-in-time` (main worktree)
- **Branch:** `feature/worktree-parallel-work`

**Agent 2 (Worktree 1):**
- **Task:** b31a81be - Implement atomic claim acquisition with file locking
- **Location:** `../just-in-time-worktrees/task-claim-acquire`
- **Branch:** `agents/claim-acquire`

**Agent 3 (Worktree 2):**
- **Task:** e66da7c0 - Implement automatic lease expiration with monotonic time
- **Location:** `../just-in-time-worktrees/task-lease-expiry`
- **Branch:** `agents/lease-expiry`

## Setup Instructions

### Initial Setup (Run Once)

```bash
# Ensure you're on the feature branch in main worktree
cd /home/vkaskivuo/Projects/just-in-time
git checkout feature/worktree-parallel-work

# Create worktree directory
mkdir -p ../just-in-time-worktrees

# Create worktrees for Agents 2 and 3
git worktree add -b agents/claim-acquire ../just-in-time-worktrees/task-claim-acquire
git worktree add -b agents/lease-expiry ../just-in-time-worktrees/task-lease-expiry

# Verify worktrees created
git worktree list
```

Expected output:
```
/home/vkaskivuo/Projects/just-in-time                       <commit> [feature/worktree-parallel-work]
../just-in-time-worktrees/task-claim-acquire                <commit> [agents/claim-acquire]
../just-in-time-worktrees/task-lease-expiry                 <commit> [agents/lease-expiry]
```

## Agent 1 Workflow

**Task:** 5bc0eff5 - Implement claims JSONL append-only log

### Task Details

Implement append-only JSONL log for claim operations with schema versioning and fsync durability.

**Files to create:**
- `crates/jit/src/storage/claims_log.rs` - Append-only log operations
- Tests in same file

**Key requirements:**
- Schema: `{"schema_version":1,"op":"acquire"|"renew"|"release"|"auto-evict"|"force-evict",...}`
- Operations: append_claim_op(), read_claim_log()
- Fsync after every append for durability
- Support for sequence numbers (optional, for total ordering)

**Design reference:** `dev/design/worktree-parallel-work.md` - Section "Schema Definitions" > "Claims JSONL"

### Steps

```bash
# Stay in main worktree
cd /home/vkaskivuo/Projects/just-in-time

# Claim task
jit issue update 5bc0eff5 --assignee agent:agent-1

# Read task details
jit issue show 5bc0eff5

# Follow TDD:
# 1. Write tests first (should fail)
# 2. Implement minimal code to pass
# 3. Run full test suite
# 4. Run clippy and fmt

# When done, commit
git add -A
git commit -m "feat: implement claims JSONL append-only log

<description>

Resolves task 5bc0eff5."

# Pass gates
jit gate pass 5bc0eff5 tdd-reminder
jit gate pass 5bc0eff5 tests
jit gate pass 5bc0eff5 clippy
jit gate pass 5bc0eff5 fmt
jit gate pass 5bc0eff5 code-review

# Mark done
jit issue update 5bc0eff5 --state done

# Commit gate passes
git add -A
git commit -m "chore: pass all gates for task 5bc0eff5"
```

## Agent 2 Workflow

**Task:** b31a81be - Implement atomic claim acquisition with file locking

### Task Details

Implement atomic claim acquisition using file locks (`fs4` crate) to ensure race-free lease operations.

**Files to create:**
- `crates/jit/src/storage/claim_coordinator.rs` - Claim coordination logic
- Tests for concurrent claim attempts

**Key requirements:**
- Use `fs4::FileExt` for advisory file locks
- Lock file: `.git/jit/locks/claims.lock`
- Acquire → Check → Create → Append to log → Update index → Release lock
- Exactly one concurrent claim succeeds (critical invariant)

**Design reference:** `dev/design/worktree-parallel-work.md` - Section "Atomic Operations and Locking"

### Steps

```bash
# Switch to worktree
cd ../just-in-time-worktrees/task-claim-acquire

# Claim task
jit issue update b31a81be --assignee agent:agent-2

# Read task details
jit issue show b31a81be

# Follow TDD (same process as Agent 1)

# When done, commit
git add -A
git commit -m "feat: implement atomic claim acquisition with file locking

<description>

Resolves task b31a81be."

# Pass gates and mark done (same as Agent 1)
```

## Agent 3 Workflow

**Task:** e66da7c0 - Implement automatic lease expiration with monotonic time

### Task Details

Implement lease expiration using monotonic clocks (immune to NTP adjustments) with lazy eviction.

**Files to create/modify:**
- `crates/jit/src/storage/lease.rs` - Lease struct with monotonic time
- Tests for expiry logic

**Key requirements:**
- Use `std::time::Instant` for TTL checks (monotonic)
- Store `DateTime<Utc>` for audit trail (wall clock)
- Lazy eviction: check and evict expired leases during claim operations
- No background daemon required

**Design reference:** `dev/design/worktree-parallel-work.md` - Section "Lease Semantics" > "Automatic Expiration"

### Steps

```bash
# Switch to worktree
cd ../just-in-time-worktrees/task-lease-expiry

# Claim task
jit issue update e66da7c0 --assignee agent:agent-3

# Read task details
jit issue show e66da7c0

# Follow TDD (same process as Agent 1)

# When done, commit
git add -A
git commit -m "feat: implement automatic lease expiration with monotonic time

<description>

Resolves task e66da7c0."

# Pass gates and mark done (same as Agent 1)
```

## Integration (After All Agents Complete)

When all 3 agents have committed their work, integrate from the main worktree:

```bash
# Return to main worktree
cd /home/vkaskivuo/Projects/just-in-time

# Check current state
git status
git worktree list

# Merge Agent 2's work
git merge agents/claim-acquire --no-ff -m "Merge: Atomic claim acquisition (Agent 2)"

# Resolve conflicts if any (likely in .jit/events.jsonl and crates/jit/src/storage/mod.rs)
# For events.jsonl: use union merge
git show :2:.jit/events.jsonl > /tmp/ours.jsonl
git show :3:.jit/events.jsonl > /tmp/theirs.jsonl
cat /tmp/ours.jsonl /tmp/theirs.jsonl | sort -u > .jit/events.jsonl
git add .jit/events.jsonl

# For mod.rs: include all new modules
git add crates/jit/src/storage/mod.rs
git commit --no-edit

# Merge Agent 3's work
git merge agents/lease-expiry --no-ff -m "Merge: Lease expiration (Agent 3)"

# Resolve conflicts (same pattern)
git add -A
git commit --no-edit

# Run full test suite
cargo test --workspace --quiet

# Run clippy
cargo clippy --workspace --all-targets

# Run CI script
./scripts/test-ci-manual.sh

# If all pass, clean up worktrees
git worktree remove ../just-in-time-worktrees/task-claim-acquire
git worktree remove ../just-in-time-worktrees/task-lease-expiry

# Delete agent branches
git branch -d agents/claim-acquire agents/lease-expiry

# Verify cleanup
git worktree list
```

## Known Pain Points (From Phase 1 Experiment)

Based on the successful Phase 1 experiment, expect:

1. **Conflicts in `.jit/events.jsonl`** - Both agents will append events
   - **Solution:** Union merge (combine all events)

2. **Conflicts in `crates/jit/src/storage/mod.rs`** - Both agents add modules
   - **Solution:** Include all module declarations

3. **No conflicts in source code** - Git worktrees isolate file changes perfectly

4. **Manual coordination** - We don't have automation yet (that's what we're building!)
   - Agents must communicate to avoid working on same task

## Success Criteria

- ✅ All 3 tasks completed in parallel
- ✅ Merges complete with < 10 lines of conflicts
- ✅ `cargo test --workspace` passes after merge
- ✅ `cargo clippy` passes (0 warnings)
- ✅ All gates passed for each task
- ✅ TDD followed (tests written first)

## Next Round

After these 3 tasks, we'll have:
- Claims JSONL log ✓
- Atomic claim acquisition ✓
- Lease expiration ✓

Remaining tasks:
- `1a5e737d` - Implement claims index with rebuild capability
- `581f8345` - Implement heartbeat mechanism for lease renewal
- `1a4c1f79` - Implement lease renewal, release, and force-evict operations

These can be done in a second parallel round or sequentially.

## References

- Design doc: `dev/design/worktree-parallel-work.md`
- Experiment results: `dev/experiments/worktree-manual-coordination-experiment.md`
- Phase 1 commits: 01267cb, 4918854, dc6212e (merged in 88ea6c9)
- Story: `jit issue show b74af86f`
