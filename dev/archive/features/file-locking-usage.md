# File Locking Usage Guide

**Status**: Complete (Phase 1.2)  
**Date**: 2025-12-03  
**Audience**: Developers using jit with concurrent agents

## Overview

Just-In-Time implements file-based locking to ensure safe concurrent access when multiple agents or processes access the same `.jit/` repository. This prevents race conditions, data corruption, and lost updates.

## Key Features

✅ **Multi-process safe**: Multiple `jit` CLI processes or MCP server instances can run concurrently  
✅ **Automatic**: Locking happens transparently—no API changes needed  
✅ **Deadlock-free**: Consistent lock ordering prevents deadlocks  
✅ **Cross-platform**: Works on Linux, macOS, Windows  
✅ **Panic-safe**: Locks released automatically on crash (RAII pattern)  
✅ **Configurable**: Timeout adjustable via environment variable

## How It Works

### Lock Files

Each data file has a corresponding `.lock` file:

```
.jit/
├── .index.lock          # Protects index.json
├── .gates.lock          # Protects gates.json
├── .events.lock         # Protects events.jsonl
├── index.json
├── gates.json
├── events.jsonl
└── issues/
    ├── <issue-id>.json
    └── <issue-id>.lock  # Per-issue lock file
```

**Why separate .lock files?**

The atomic write pattern (write temp file, then rename) would invalidate file descriptor locks. Using separate `.lock` files ensures locks remain valid across atomic writes.

### Lock Types

**Shared (Read) Locks:**
- Multiple processes can hold shared locks simultaneously
- Used for read operations: `load_issue`, `list_issues`, `read_events`, etc.
- Blocks writers (exclusive locks)

**Exclusive (Write) Locks:**
- Only one process can hold an exclusive lock
- Blocks all other readers and writers
- Used for write operations: `save_issue`, `delete_issue`, `append_event`, etc.

### Lock Ordering

To prevent deadlocks, locks are always acquired in this order:

1. **Index lock** (`.index.lock`)
2. **Issue lock** (`<issue-id>.lock`)
3. **Other locks** (gates, events)

Example: Creating a new issue
```
1. Acquire exclusive lock on .index.lock
2. Read index.json
3. Check if issue ID exists
4. Acquire exclusive lock on <issue-id>.lock
5. Write issue JSON file
6. Update index.json
7. Release all locks (automatic on drop)
```

## Configuration

### Lock Timeout

Control how long to wait for lock acquisition:

```bash
# Default: 5 seconds
export JIT_LOCK_TIMEOUT=10  # Wait up to 10 seconds

jit issue create --title "My Issue"
```

**When to increase timeout:**
- High-contention scenarios (many concurrent agents)
- Slow file systems (network mounts)
- Long-running operations

**When to decrease timeout:**
- Fast failure desired (fail-fast systems)
- Low-latency requirements

### Lock Behavior

The FileLocker polls every 10ms for lock availability:

```rust
// Pseudocode
while !acquired && elapsed < timeout {
    try_acquire_lock();
    if !acquired {
        sleep(10ms);
    }
}
```

If timeout expires, operation fails with clear error message.

## Performance Characteristics

### Overhead

- **Single-threaded**: Minimal overhead (~1-2% measured)
- **Low contention** (2-5 concurrent agents): <5% overhead
- **High contention** (10+ concurrent agents): Dependent on workload

Lock acquisition is fast (microseconds) when uncontested.

### Scalability

**Recommended limits:**
- **5-10 concurrent agents**: Optimal performance
- **10-20 concurrent agents**: Good performance with increased contention
- **20+ concurrent agents**: Consider alternative storage backend (SQLite)

The system has been stress-tested with 50 concurrent threads successfully.

## Concurrency Patterns

### Pattern 1: Independent Issue Updates

✅ **Works perfectly** - Different issues can be updated concurrently:

```bash
# Agent 1
jit issue update ISSUE1 --state in-progress

# Agent 2 (concurrent)
jit issue update ISSUE2 --state done
```

Each issue has its own lock, so no contention.

### Pattern 2: Same Issue Updates

⚠️ **Serialized** - Updates to the same issue are sequential:

```bash
# Agent 1
jit issue update ISSUE1 --title "New title"

# Agent 2 (waits for Agent 1)
jit issue update ISSUE1 --state in-progress
```

Agent 2 waits for Agent 1's lock to be released. Last write wins.

### Pattern 3: Many Issue Creates

✅ **High throughput** - Creating issues is well-optimized:

```bash
# 10 agents creating issues concurrently
# All succeed, no corruption
```

Index lock is held briefly only during ID registration, then issue writes happen in parallel.

### Pattern 4: List + Create

✅ **Consistent** - Readers see consistent state:

```bash
# Agent 1 (reader)
jit query all

# Agent 2 (writer, concurrent)
jit issue create --title "New"
```

Agent 1 sees either the old or new state, never corrupted/inconsistent data.

## Error Handling

### Lock Timeout Error

```
Error: Lock timeout: could not acquire lock on .jit/.index.lock after 5s

This usually means another jit process is running.
Try:
  1. Wait a few seconds and retry
  2. Check for stuck processes: ps aux | grep jit
  3. Increase timeout: JIT_LOCK_TIMEOUT=10 jit ...
```

**Troubleshooting:**

1. **Check for stuck processes:**
   ```bash
   ps aux | grep jit
   # Kill if needed: kill <pid>
   ```

2. **Increase timeout:**
   ```bash
   export JIT_LOCK_TIMEOUT=30
   jit issue create --title "Test"
   ```

3. **Check file system:**
   ```bash
   # Ensure .jit/ is on local file system
   df -h .jit/
   ```

### Lock Contention

If experiencing frequent timeouts:

1. **Reduce concurrency**: Limit number of concurrent agents
2. **Increase timeout**: Give more time for lock acquisition
3. **Batch operations**: Group multiple operations into fewer transactions
4. **Consider alternatives**: For >20 concurrent agents, use SQLite backend

## Best Practices

### ✅ DO

- Use default timeout for most scenarios
- Let operations fail and retry at application level
- Monitor lock timeout errors in production
- Test concurrent scenarios before deployment
- Use separate `.jit/` directories for independent projects

### ❌ DON'T

- Set timeout < 1 second (may cause spurious failures)
- Set timeout > 60 seconds (indicates deeper problem)
- Run jit on NFS/network mounts (not officially supported)
- Manually delete `.lock` files (OS handles cleanup)
- Share `.jit/` across machines (use one per machine/container)

## Implementation Details

### Technology

- **Library**: `fs4` crate (cross-platform file locking)
- **Lock type**: Advisory locks (flock on Unix, LockFileEx on Windows)
- **Release**: Automatic via RAII (Drop trait)
- **Timeout**: Polling with configurable timeout

### Advisory vs Mandatory

Jit uses **advisory locks** (not mandatory):

✅ **Pros:**
- Cross-platform support
- OS handles cleanup on crash
- Low overhead
- Standard Unix/Windows behavior

❌ **Cons:**
- Malicious/buggy programs can ignore locks
- Assumes all jit processes cooperate

**This is acceptable** because:
1. All jit processes use the same locking code
2. Typical use case is trusted agents/tools
3. Alternative (mandatory locks) not portable

### Windows vs Unix

**Unix (Linux, macOS):**
- Uses `flock()` system call
- Locks associated with file descriptor
- Released on process exit

**Windows:**
- Uses `LockFileEx()` API
- Similar behavior to Unix
- Tested in CI

Both platforms provide equivalent safety guarantees.

## Testing

The locking implementation includes comprehensive tests:

### Unit Tests (6 tests)

- `test_exclusive_lock_acquired`
- `test_shared_lock_acquired`
- `test_try_lock_non_blocking`
- `test_lock_released_on_drop`
- `test_exclusive_lock_prevents_concurrent_writes`
- `test_shared_locks_allow_concurrent_reads`

### Integration Tests (7 tests)

- `test_concurrent_issue_creates_no_corruption` (50 threads × 5 issues)
- `test_concurrent_updates_to_different_issues`
- `test_concurrent_updates_to_same_issue`
- `test_concurrent_read_write_issue`
- `test_concurrent_dependency_operations`
- `test_concurrent_list_and_create`

Run tests:
```bash
cargo test storage::json --lib
cargo test lock_tests --test '*'
```

## Migration Notes

### From Pre-Locking Versions

File locking was added in **v0.4.0** (Phase 1.2).

**No migration needed**—locking is transparent:
- Existing `.jit/` repositories work as-is
- `.lock` files created automatically on first access
- No performance degradation for single-agent use

### Upgrading

```bash
# Upgrade jit binary
cargo install --path . --force

# No repository changes needed
jit status  # Works immediately
```

`.lock` files are automatically created and can be safely ignored by version control:

```gitignore
# .gitignore
.jit/*.lock
.jit/issues/*.lock
```

## Future Enhancements

Planned improvements (not yet implemented):

- [ ] **Retry with exponential backoff**: Automatic retries on lock contention
- [ ] **Lock metrics**: Track lock acquisition times and contention
- [ ] **Distributed locking**: Support for networked/distributed repositories
- [ ] **Lock debugging**: `jit debug locks` command to inspect lock state

## Troubleshooting

### Q: Seeing "Lock timeout" errors frequently

**A:** Likely high contention. Solutions:
1. Increase `JIT_LOCK_TIMEOUT` (try 10-15 seconds)
2. Reduce number of concurrent agents
3. Check for slow file system (network mounts)
4. Review agent coordination patterns

### Q: Locks not released after crash

**A:** This should not happen—OS releases locks automatically. If it does:
1. Verify no zombie processes: `ps aux | grep jit`
2. Check that `.jit/` is on local file system (not NFS)
3. Report bug with OS details

### Q: Performance degraded after upgrade

**A:** File locking adds minimal overhead. If experiencing issues:
1. Measure with `time jit status` (should be <100ms)
2. Check for high contention (many concurrent agents)
3. Profile with `--release` build (debug builds are slower)

### Q: Can I disable locking?

**A:** No, locking is always enabled for safety. However:
- Single-threaded usage has negligible overhead
- Locking is necessary for concurrent safety
- Alternative: Use InMemoryStorage for testing (no locking needed)

## See Also

- [File Locking Design Document](file-locking-design.md) - Implementation details
- [Storage Abstraction](storage-abstraction.md) - Storage layer architecture
- [Testing Guide](../TESTING.md) - How to test concurrent scenarios
- [Roadmap](../ROADMAP.md) - Future locking improvements

## Support

Questions or issues with file locking?

1. Check existing tests: `cargo test storage::json::tests::test_concurrent_*`
2. Review design doc: `docs/file-locking-design.md`
3. Open GitHub issue with reproduction steps
