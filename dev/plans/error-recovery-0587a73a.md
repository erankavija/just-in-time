# Implementation Plan: Comprehensive Error Recovery (`0587a73a`)

## 1. Current State Audit

### What Error Handling Exists

**Storage layer (`crates/jit/src/storage/json.rs`)**
- Atomic writes via temp-file + rename pattern in `write_json()` (lines 103-117). No `fsync` on the temp file before rename — only rename atomicity is guaranteed, not disk durability.
- `save_gate_run_result()` (lines 603-619) does NOT use atomic writes — it writes directly with `fs::write()`. A crash mid-write would leave corrupt JSON.
- `read_events()` (lines 577-601): Fails completely on any corrupt line — `serde_json::from_str().context(...)` with `?` propagates the error, aborting the entire read.
- `load_issue()` (lines 425-449): Falls through three sources (local, git, main worktree) with graceful fallback.
- `list_issues()` (lines 527-545): Uses `filter_map(|id| load_issue(id).ok())` — silently drops corrupt issues without surfacing the fact that some were skipped.

**Claims log (`crates/jit/src/storage/claims_log.rs`)**
- `append()` (lines 149-187): Uses `fsync` (`file.sync_all()`). Good.
- `read_all()` (lines 198-216): Fails on ANY corrupt line with `?`. No skip-bad-line behavior.

**Lock system (`crates/jit/src/storage/lock.rs`, `lock_cleanup.rs`)**
- `lock_exclusive()` (lines 129-156): Returns a timeout error after configurable duration.
- `cleanup_stale_locks()` (`lock_cleanup.rs` lines 51-126): PID-based detection. If process is alive but lock is old (>1 hour), only logs `eprintln!` but takes no action (lines 106-117). Not integrated with `jit validate --fix`.

**Validate command (`crates/jit/src/commands/validate.rs`)**
- `validate_silent()` (lines 194-277): Checks broken dependency references, invalid gate references, label validity, document references, type hierarchy, DAG validity, isolated nodes, transitive reduction, and claims index.
- `validate_with_fix()` (lines 40-89): Fixes hierarchy issues, transitive reduction violations, pending state transitions. Does NOT fix: corrupt JSON files, orphaned `.tmp` files, broken gate run dirs, stale locks, missing `index.json` entries.
- `validate_leases()` (lines 866-928): Reports fix commands but does NOT auto-apply them.

**Recovery command (`crates/jit/src/commands/claim.rs`)**
- `execute_recover()` (lines 400-471): Handles stale locks, index rebuild, expired lease eviction, orphaned temp files. Does NOT handle: corrupt JSON detection, schema migration, events.jsonl corruption, broken gate-runs directories.

**Error types**
- `errors.rs`: `ActionableError` with causes and remediation. Used for claim operations.
- Storage operations: All use raw `anyhow::Result` with context strings — no typed storage errors, making programmatic recovery impossible.

### What Is Missing

1. **No typed `StorageError` enum** — all storage failures are string-context anyhow errors
2. **Corrupt JSON detection and recovery**: `read_events()` and claims log `read_all()` hard-fail on corrupt lines
3. **`save_gate_run_result()` is not atomic** — direct `fs::write()` without temp-rename
4. **No `fsync` on temp files** in `write_json()` — rename atomicity doesn't guarantee durability on power failure
5. **`list_issues()` silently drops corrupt issues** without surfacing the fact
6. **Stale lock auto-fix**: `validate_leases()` reports but does not fix
7. **No `jit validate --fix` coverage for**: orphaned temp files, corrupt JSON, gate run directory corruption, schema version mismatches
8. **No failure injection test infrastructure** — all tests use happy-path storage
9. **Lock timeout errors** are plain anyhow strings — no `ActionableError` with context for operators
10. **No operator runbook** in docs

---

## 2. New Error Types to Add

Create `crates/jit/src/storage/error.rs` with a `StorageError` enum using `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("corrupt JSON in {path}: {source}")]
    CorruptJson { path: PathBuf, source: serde_json::Error },
    #[error("partial write to {path}: {source}")]
    PartialWrite { path: PathBuf, source: std::io::Error },
    #[error("lock timeout after {elapsed:?} waiting for {path}")]
    LockTimeout { path: PathBuf, elapsed: std::time::Duration },
    #[error("lock I/O error for {path}: {source}")]
    LockIoError { path: PathBuf, source: std::io::Error },
    #[error("missing required file: {path}")]
    MissingFile { path: PathBuf },
    #[error("schema mismatch in {path}: expected v{expected}, found v{found}")]
    SchemaMismatch { path: PathBuf, expected: u32, found: u32 },
    #[error("index inconsistency: {description}")]
    IndexInconsistency { description: String },
    #[error("orphaned lock at {path} (PID: {pid:?})")]
    OrphanedLock { path: PathBuf, pid: Option<u32> },
    #[error("orphaned temp file at {path} (age: {age:?})")]
    OrphanedTempFile { path: PathBuf, age: std::time::Duration },
    #[error("broken reference: issue {issue_id} references missing {ref_id}")]
    BrokenReference { issue_id: String, ref_id: String },
}
```

Add `ActionableError` constructors in `errors.rs` for:
- Lock timeout (which agent holds it, suggested commands)
- Corrupt JSON (which file, how to recover)
- Schema mismatch (current vs expected, migration path)
- Orphaned temp file

---

## 3. Specific Files and Functions to Modify

### Phase 1: Typed Errors and Storage Hardening

**`crates/jit/src/storage/error.rs`** (new file)
- Define `StorageError` enum as above
- Implement `From<StorageError>` for `anyhow::Error`

**`crates/jit/src/storage/json.rs`**
- `write_json()` (line 103): Add `fsync` — open temp file as `File`, write via `BufWriter`, call `file.sync_all()` before `fs::rename()`. Follow pattern from `claims_log.rs` line 184.
- `save_gate_run_result()` (line 603): Replace `fs::write()` with temp-file + rename pattern (mirror `write_json()`).
- `read_events()` (line 577): Add `read_events_resilient()` variant that skips corrupt lines and returns `(Vec<Event>, Vec<SkippedLine>)` where `SkippedLine = { line_number: usize, raw: String, error: String }`.
- `list_issues()` (line 527): Log a warning via `eprintln!` when an issue fails to load, rather than silently dropping.
- `load_index()` (line 125): Validate `schema_version` field; return `StorageError::SchemaMismatch` if wrong.
- `lock_exclusive()` / `lock_shared()` in `lock.rs` (lines 129, 168): Return `StorageError::LockTimeout` with path and elapsed time, convert to `ActionableError` at call site.

**`crates/jit/src/storage/claims_log.rs`**
- Add `read_all_resilient(&self) -> Result<(Vec<ClaimEntry>, Vec<SkippedLine>)>` that skips corrupt lines. Keep existing `read_all()` for callers needing integrity guarantees. Use `read_all_resilient()` from `execute_recover()`.

### Phase 2: Validate --fix Extensions

**`crates/jit/src/commands/validate.rs`**

Extend `validate_with_fix()` (line 40) to call new fix functions:

- `fix_orphaned_temp_files(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>` — calls `temp_cleanup::cleanup_orphaned_temp_files()`, reports count and paths
- `fix_stale_locks(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>` — calls `lock_cleanup::cleanup_stale_locks()`, removes orphaned locks in non-dry-run mode
- `fix_corrupt_gate_runs(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>` — scans `.jit/gate-runs/` for directories missing or invalid `result.json`; moves them to `gate-runs-corrupt/`
- `fix_index_consistency(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>` — cross-references `index.json` `all_ids` against files in `.jit/issues/`, adds missing IDs and removes orphaned entries
- `fix_expired_leases(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>` — invokes `ClaimCoordinator::evict_expired()` and writes updated index

Add `validate_storage_integrity(&self) -> Result<Vec<StorageIssue>>` (new public function) that returns typed problems each with `fixable: bool`. Foundation for `--fix` mode and JSON output.

**`crates/jit/src/errors.rs`**
- Add `lock_timeout(path: &Path, elapsed: Duration) -> ActionableError`
- Add `corrupt_json(path: &Path, error: &serde_json::Error) -> ActionableError`
- Add `schema_mismatch(path: &Path, expected: u32, found: u32) -> ActionableError`
- Add `orphaned_temp_file(path: &Path) -> ActionableError`

### Phase 3: Failure Injection Infrastructure

**`crates/jit/src/storage/json.rs`** (test utilities, `#[cfg(test)]`)

Add `FaultInjector` helpers:
- `inject_corrupt_json(path: &Path)` — overwrites with invalid JSON
- `inject_partial_write(path: &Path)` — writes truncated JSON
- `inject_lock_hold(path: &Path, duration: Duration)` — acquires lock in background thread

---

## 4. Auto-Fix Capabilities in `jit validate --fix`

| Fix Type | Detection | Repair Action |
|---|---|---|
| Orphaned `.json.tmp` files | Age > threshold, `.tmp` extension | Delete via `temp_cleanup` |
| Stale lock files (dead PID) | PID check fails | Delete lock + meta files |
| Broken gate-run directories | Missing or invalid `result.json` | Move to `gate-runs-corrupt/` |
| Index vs filesystem inconsistency | `all_ids` not matching `.json` files | Rebuild index from filesystem scan |
| Expired leases | `expires_at < now` | Evict via `ClaimCoordinator::evict_expired()` |
| Corrupt events.jsonl lines | JSON parse error on line | Move to `events.jsonl.bak`, create fresh file with recoverable entries |

The `--dry-run` flag reports each without making changes, consistent with `fix_all_transitive_reductions()` pattern.

---

## 5. Failure Injection Test Approach

**Layer 1: Unit tests in `storage/json.rs`** (`#[cfg(test)]`)
- Test `write_json()` fsync behavior using `TempDir`
- Test `read_events_resilient()` with manually injected corrupt lines
- Test `save_gate_run_result()` atomic write (no `.tmp` files remain)

**Layer 2: Harness tests** (`tests/harness_demo.rs` or new section)
- Use `JsonFileStorage` directly with `TempDir`
- Inject corrupt JSON via direct filesystem writes before calling storage methods

**Layer 3: Integration tests** (`tests/error_recovery_tests.rs`, new file)

Tests to write:
- `test_corrupt_issue_json_detected_by_validate` — write invalid JSON to issue file, assert `validate_silent()` returns error
- `test_corrupt_events_jsonl_skipped_in_resilient_read` — write corrupt line into `events.jsonl`, assert good events returned and skipped line reported
- `test_orphaned_temp_file_detected_and_fixed` — create `.json.tmp`, run `validate --fix`, assert removed
- `test_stale_lock_detected_and_fixed` — create lock file with dead-PID metadata, run `validate --fix`, assert removed
- `test_lock_timeout_returns_actionable_error` — hold lock in thread, attempt to acquire with 50ms timeout, assert error contains lock path and remediation
- `test_partial_write_recovery` — leave `.json.tmp` in place (simulating failed rename), assert `validate --fix` cleans it up
- `test_schema_mismatch_detected` — write `index.json` with wrong `schema_version`, assert descriptive error

**Proptest** (in `claim_coordinator_proptests.rs` or new file):
- Generate arbitrary sequences of claim operations with injected corruption at random positions
- Assert `read_all_resilient()` returns non-empty result when at least one valid line exists

---

## 6. Step-by-Step Implementation Order

**Step 1: Create `StorageError` enum and `ActionableError` helpers**
- Create `crates/jit/src/storage/error.rs`
- Add new helpers to `crates/jit/src/errors.rs`
- Expose `StorageError` from `crates/jit/src/storage/mod.rs`
- Zero breaking changes — purely additive

**Step 2: Harden `write_json()` with fsync**
- Modify `write_json()` in `json.rs` to open temp as `File`, write via `BufWriter`, call `file.sync_all()`, then `fs::rename()`
- Add unit test verifying no `.tmp` remains after successful write

**Step 3: Make `save_gate_run_result()` atomic**
- Modify `save_gate_run_result()` (line 603) to use same temp-rename pattern as `write_json()`
- Add test: write gate run result, verify atomic rename

**Step 4: Add `read_events_resilient()` to `JsonFileStorage`**
- Add new method returning `Result<(Vec<Event>, Vec<SkippedLine>)>`
- Update `list_issues()` to log (not fail) on individual issue load failures
- Add unit tests injecting corrupt lines into `events.jsonl`

**Step 5: Add `read_all_resilient()` to `ClaimsLog`**
- Mirror `read_events_resilient()` pattern
- Call from `ClaimCoordinator::rebuild_index_from_log()` for recovery resilience

**Step 6: Add `validate_storage_integrity()` to `CommandExecutor`**
- Implement in `crates/jit/src/commands/validate.rs`
- Detects: orphaned `.tmp` files, stale locks (PID check), corrupt gate-run dirs, index inconsistencies
- Returns `Vec<StorageIssue>` with `kind`, `path`, `fixable: bool`, `description`

**Step 7: Extend `validate_with_fix()` with new fix categories**
- Add five new fix private methods on `CommandExecutor`
- Call from `validate_with_fix()` (line 40) after existing three fix steps
- Each returns `(usize, Vec<String>)` with dry-run support

**Step 8: Add `ActionableError` to lock timeout paths**
- In `lock.rs` `lock_exclusive()` (line 143), convert timeout error to `errors::lock_timeout(path, elapsed)`
- In `claim_coordinator.rs`, convert `LockTimeout` to `ActionableError` before returning to CLI layer

**Step 9: Write failure injection tests**
- Create `crates/jit/tests/error_recovery_tests.rs` with the seven integration test cases
- Add proptest cases to `claim_coordinator_proptests.rs` for resilient log reads

**Step 10: Write operator runbook**
- Create `docs/how-to/error-recovery.md` with sections for each error category
- Contents: corrupt JSON symptoms + manual recovery, lock timeout diagnosis, schema migration, when to use `jit recover` vs `jit validate --fix`

**Step 11: Update CLI validate JSON output**
- In `crates/jit/src/main.rs` (~line 3233), extend `fixes_applied` JSON field to count all new fix types

**Step 12: Clippy and fmt pass**
```bash
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test
```

---

## Critical Files

| File | Changes |
|------|---------|
| `crates/jit/src/storage/json.rs` | Atomic `save_gate_run_result()`, fsync in `write_json()`, `read_events_resilient()`, corrupt-issue logging in `list_issues()` |
| `crates/jit/src/commands/validate.rs` | Five new fix functions, `validate_storage_integrity()` |
| `crates/jit/src/storage/claims_log.rs` | `read_all_resilient()` |
| `crates/jit/src/errors.rs` | New `ActionableError` constructors |
| `crates/jit/src/storage/lock.rs` | `StorageError::LockTimeout` integration |
| `crates/jit/src/storage/error.rs` | New file: `StorageError` enum |
| `crates/jit/tests/error_recovery_tests.rs` | New file: failure injection integration tests |
| `docs/how-to/error-recovery.md` | New file: operator runbook |
