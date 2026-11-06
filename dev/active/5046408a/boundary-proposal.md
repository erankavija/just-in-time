# 5046408a selected-A boundary amendment, R2: proposal for bounded design re-review

Status: **proposal/read-only**. This R2 replaces `5046408a-rework-3-boundary-proposal.md` (R1-reviewed). Steward selected A: preserve the shared typed item/recover adapters, and give worktree exec a data-only entry that preserves its own under-lock pending-record refusal. The I/O-source repair is in scope. The footprint stays at the same thirteen files: the accepted ten plus commands/mod.rs, commands/worktree.rs and dispatch/worktree.rs. R2 amends only the exec root/lock handoff to resolve the single blocking finding of `5046408a-boundary-design-review-r1.md`. Every other R1 coverage row passed and is carried over unchanged. B and registry/contract cutover are not selected. No retry counter is reset. Earlier alternatives and failure history remain in `5046408a-rework-3-boundary-proposal-before-A.md` and `5046408a-rework-3-result.md`.

504 source remains frozen at HEAD `376978ed05c011717ab7c2d88fab54954f045f15`, diff SHA256 `f0c5fd8a3e18932804cd4f02b412b9d1540321d408522733e66601a7ce22b9a3` (re-verified for R2). No production source, test assertion, registry, tracker or main source was edited. Source citations below are `crates/jit/src/...` lines in that frozen worker tree.

## Proven failures and exhaustive registry scope (unchanged from R1-reviewed proposal)

The complete non-fail-fast run collected CLI 1222 pass/3 fail and contracts 548 pass/2 fail. Formatting and all-target Clippy passed. Logs and intermediate RED/compile corrections remain in the cycle3 result. The nine applicable failure-registry arms were taken directly from their setup declarations and independently reproduced in `5046408a-rework-3-full-boundary-registry.json`:

| Arm/setup | Required result | Current result |
|---|---|---|
| config show / malformed config | PARSE_ERROR | Matches |
| config validate / malformed config | VALIDATION_FAILED findings | Matches |
| config show-hierarchy / malformed config | PARSE_ERROR | Matches |
| doc conformance / malformed config | PARSE_ERROR | Matches |
| issue list / pending redo | RETRYABLE_CONFLICT | Matches |
| item list / malformed config | INVALID_ARGUMENT | PARSE_ERROR |
| item search / malformed config | INVALID_ARGUMENT | PARSE_ERROR |
| label namespaces / malformed config | PARSE_ERROR | Matches |
| recover / malformed config | recovery_failed | PARSE_ERROR |

The universal retryable-read projection passed its eleven commands in human and JSON forms. The existing worktree-exec pending test requires GenericError, the absolute record path and recovery guidance, no child invocation and unchanged record bytes. Bootstrap currently returns RetryableConflict first. All registry, projection and exec assertions remain unchanged under selected A. The two actor failures reproduce on unchanged main and stay excluded; see `5046408a-actor-task-proposal.md` and `5046408a-actor-baseline-main.json`.

## Shared typed error adapters (unchanged; R1 rows "item adapter provenance", "recovery adapter and D-27", "canonical parsing" passed)

Carried over verbatim in substance from the R1-reviewed proposal §"Shared typed error adapters", points 1–4:

1. Extract the existing `CommandExecutor::cached_config` argument-error computation (commands/mod.rs:2908–2920) into one crate-visible helper, `configuration_argument_error(error: impl Display) -> InvalidArgumentError`, with the existing `invalid .jit/config.toml: ...` context. The cached full-view consumer and the item invocation's early required-declaration failure both call it.
2. At the CLI bootstrap composition boundary, adapt only a typed `ConfigurationAssemblyError` from the required WorktreeConfig projection. The adapter is a direct `anyhow::Error::downcast::<ConfigurationAssemblyError>()` on ordinary `open_repository`'s failure, and a failed downcast returns the original error unchanged. A private dispatch adapter selects the item helper for `Commands::Item` and the existing `RecoveryFailedError` constructor for `Commands::Recover`. Every other command keeps the canonical typed error. The adapter does not search nested chains or assign exit codes.
3. Recover routes the same authored projection failure through `RecoveryFailedError` after committed recovery (commands/bootstrap.rs:94–99 precedes projection at :109–110). Root-path, unsafe-capability, unsupported-index, permission and retryable-admission errors are not wrapped.
4. Ordinary operations keep canonical parser failures. Config validate keeps its separately admitted data-only report. An env-selected root removes only the declared-root projection requirement.

After an early failure, no executor or code authority is constructed and no mutation is authorized.

## Data-only worktree exec: one command computation over one held data root

The existing operation (commands/worktree.rs:82–98) acquires the repository lock, refuses a pending record by path and launches a child. It uses no code layout. Its sole caller is dispatch/worktree.rs:59–70. Replace it with one command-owned data-only entry in commands/worktree.rs and keep no second lock/pending/child implementation or compatibility alias. A normal group handler retained for registration delegates to the same operation with its actual JsonFileStorage. The CLI data-only route reaches the operation directly.

A small command context carries the selected JsonFileStorage over the **absolute** selected data path and the cwd captured at invocation. It carries no CommandExecutor, WorktreePaths, code root, Git probe or mutation guard. The command owner builds it from RepositoryInvocation with the existing selected-data discovery helper. The narrow bootstrap result carries this context for `WorktreeCommands::Exec`, and other worktree commands keep ordinary bootstrap. The registered `worktree_exec` leaf, existing `WorktreeExecResponse` and `Outcome::Exited` stay the output owner. Both adapters share one child-status outcome computation and registered-leaf admission before the common emitter. No command or schema leaf is added.

### Storage-owned retained root evidence (new in R2)

All root identity, lock-chain, pending-presence and index-capture work moves into **one storage operation** in `storage/repository_state_store.rs`. It is built from the existing `DataReadSession` evidence and `CapabilityRoots::open_data` identity checks. The command layer receives either a held root or a pending-record path. It never compares identities, opens capabilities or orders locks.

**Factoring (no new identity protocol).** Split the root-only half out of `DataReadSession::open_after_missing` (repository_state_store.rs:455–475) into a private storage type, called `DataRootHandle` here:

- `DataRootHandle { data_root: PathBuf, evidence: RepositoryRootEvidence, roots: CapabilityRoots }`.
- `DataRootHandle::open_after_missing(data, after_missing)` holds lines 459–467 moved unchanged: `lexical_absolute`, `discover_data_read_evidence` (518–537, the single missing-ancestry owner), `CapabilityRoots::open_data(&evidence)` (capability_fs.rs:92–138, leaf identity via `ensure_capability_identity` at :491–504) and the published-ancestor guard at :463. It reads nothing beneath the root and creates nothing.
- `DataRootHandle::is_current(&self) -> Result<bool>` returns `discover_data_read_evidence(&self.data_root, || {})? == self.evidence`. This is the comparison now inlined at :494.
- `DataRootHandle::reopen(&self) -> Result<CapabilityRoots>` returns `CapabilityRoots::open_data(&self.evidence)`. This is the call now inlined at :497.
- `DataReadSession` becomes `{ root: DataRootHandle, opened: CommitObservation }`. `open_after_missing` is `DataRootHandle::open_after_missing` plus `observe_json_commits(&root.roots)` (:468). `admit` becomes `if !self.root.is_current()? { Moved }` and then `self.opened.admit(observe_json_commits(&self.root.reopen()?))`. Semantics are unchanged. `ConfigurationObservation::attempt` (:589–590) and `has_data_root`/`read`/`declarations` read through `self.root` mechanically.

**Held root.** Add to the same file:

```rust
/// A present data root held under bootstrap → repository locks, bound to the evidence its precheck took.
pub(crate) struct HeldDataRoot { root: DataRootHandle, _repository: RepoWriteGuard, _bootstrap: RepoWriteGuard }
pub(crate) enum DataRootHold { PendingRecord(PathBuf), Initialized(HeldDataRoot) }

impl JsonFileStorage {
    pub(crate) fn hold_initialized_data_root(&self) -> Result<DataRootHold, RepositoryStateStoreError> {
        self.hold_initialized_data_root_at(|_| {})
    }
    fn hold_initialized_data_root_at(&self, at: impl FnMut(HoldPoint)) -> Result<DataRootHold, RepositoryStateStoreError>;
}
enum HoldPoint { LockChainAcquired, IndexCaptured }   // private test seam, no-op in production
```

The `_at(hook)` seam follows the existing private-closure convention of `DataReadSession::open_after_missing`, `recover_after_peek` (:862) and `ConfigurationObservation::attempt(after_index)` (:562). `HeldDataRoot` and `DataRootHold` are reachable from commands through `pub mod repository_state_store` (storage/mod.rs:46), as `ConfigurationObservation` already is from commands/config.rs:84. No storage/mod.rs edit is needed.

### Exact order and error precedence (steps 2–7 amended)

1. **Invocation inputs/discovery (unchanged):** capture cwd and data override once. Run the existing global write-policy-token validation before repository work. Select the data path through the existing JIT_DATA_DIR/upward-discovery logic, without loading configuration, deriving a default code root or selecting a code checkout. A code-root setting has no role in where the child runs; it stays in the inherited environment for a child JIT invocation. The wrapper authorizes no JIT mutation through linked-write policy.
2. **Root precheck, retained:** `E := DataRootHandle::open_after_missing(storage.root())`. If no data root is present, return the existing `RepositoryNotFoundError` for the data path. An ancestor published during absence discovery returns the existing `RetryableConflict` (:463–467). Symlinked or unsafe roots return their typed `Layout`/`UnsafeTarget`, and unreadable ones `Io`. E is held for the rest of the operation and is never replaced by a fresh probe.
3. **Acquire the existing chain in the mutation-session order:** first `self.acquire_bootstrap_write_lock()` (json.rs:751; data-root-parent lock json.rs:515–519). Then `E.is_current()`, and on false release and return `RetryableConflict { path: data_root }`. This mirrors `open_mutation_session` bootstrap→`revalidate_layout` (repository_state_store.rs:1043–1044) and recovery's bootstrap→`discover_root_evidence` check (:917–918). Then `self.acquire_repo_write_lock_raw()` (json.rs:776). It reenters the held bootstrap predecessor (repo_lock.rs:217–235) and takes `<data>/.repo-write.lock`. JIT_LOCK_TIMEOUT applies, and a timeout stays `LockTimeout` with no child. A data root missing at lock time stays the lock owner's `RepositoryNotFoundError` (repo_lock.rs:278–281). No worktree/events lock, recovery session, mutation session or new ordering is introduced. Exec waits by design: its job is to run under the writer mutex.
4. **Post-acquisition revalidation and rebinding (`HoldPoint::LockChainAcquired`):** `E.is_current()` must hold again, and `roots := E.reopen()` must pass `ensure_capability_identity` against E's identity. A changed or missing root returns `RetryableConflict { path: data_root }`, and an unsafe one returns its typed `Layout`/`UnsafeTarget`. On either failure the function returns early and both guards drop before the error reaches the caller. All later reads go **only** through `roots`, the identity-verified handle on the directory whose `.repo-write.lock` is held. A path re-resolution never supplies a fresh accepted root.
5. **Pending presence first, bound to the held root:** `open_leftover_record(&roots)` (:697–712) is the existing bounded no-follow presence owner. It does not parse, read, replay, replace or remove the record. If present, check `E.is_current()` (conflict as in step 4) and return `DataRootHold::PendingRecord(data_root/RECORD_SCRATCH_DIR/RECORD_FILE_NAME)`. Guards drop on return. commands/worktree.rs maps this variant to the existing GenericError text, "Refusing to run over a leftover redo record at {path}: run `jit recover` first". This holds for the existing test's malformed `leftover` bytes. Presence or capability failures stay typed. No `RetryableConflict` from a read is ever relabeled as this refusal.
6. **Clean initialized capture across revalidation (`HoldPoint::IndexCaptured`):** `entry := inspect_capability_path(&roots, &VirtualPath::INDEX, ReadMode::Whole)` (:2154) reads through the held handle, then `E.is_current()` runs again. Only when E is still current does `validate_captured_index(&data_root, true, &entry?)` (json.rs:409–434) decide. Missing, malformed or unsupported indexes, permission errors and unsafe entries keep their existing typed failures. Success returns `DataRootHold::Initialized(HeldDataRoot { root: E with roots, guards })`. Exec needs no declaration assembly, so none runs. Because the chain is held, a cooperating writer cannot publish between steps 5 and 6. A non-cooperating replacement anywhere from step 2 through step 6 ends in the `RetryableConflict` of step 3, 4, 5 or 6.
7. **Child:** only on `Initialized(held)`, run the exact program and argv with `current_dir` set to the captured cwd. JIT_DATA_DIR selects the protected data, not the child cwd. Stdin, stdout, stderr and environment are inherited. The command keeps `held` alive across the child wait and returns the child's integer status unchanged. The generic fallback for an `ExitStatus` without a numeric code is retained. A spawn failure is the existing wrapper GenericError. The final identity check is step 6's. A non-cooperating replacement after launch is outside any pre-launch check. Child JIT commands make their own admission or locking against the root they select.
8. **Cleanup/rendering (unchanged contract):** each return path drops the guards, repository before bootstrap. That covers precheck failure (no guard taken), a step-3/4 conflict, pending refusal, index failure, spawn error and every child status. `HeldDataRoot` field order and `RepoWriteGuard::drop` (repo_lock.rs:326–333) release the repository lock first. The bootstrap flock drops at depth zero. No manual lock-file deletion, fresh lock protocol, process-global cwd/environment mutation or root publication is added. Ignored lock files may persist as before. Only wrapper errors get JIT's human/JSON error rendering, and child bytes pass through unwrapped.

Precedence: absent or unsafe data fails before any lock. An occupied lock can time out before pending inspection. A root changed while waiting fails as `RetryableConflict` before pending or index inspection. Under the held and revalidated root, a pending record outranks a missing or invalid index and malformed configuration. Only a clean store reaches initialized validation. A clean but uninitialized store may create and release the existing lock files, but it publishes no repository content and starts no child.

### Data-bootstrap predecessor under changed root ancestry (new in R2)

The predecessor is the shared lock at `<parent of data path>/.jit-bootstrap.lock`. JsonFileStorage fixes that path at construction (json.rs:515–519), and the lock opens it once at acquisition (lock.rs:200–203). `HeldDataRoot` retains its guard, unchanged and not rebound, until after the repository guard drops. It confers ordering only, not root authority. Exec creates and publishes nothing in the parent namespace.

- **Leaf replaced in the same parent (R1 case):** the retained bootstrap guard is still the live parent's lock. The data identity check fails.
- **Ancestry changed and the data directory gone or different at the path:** the evidence differs, so the result is `RetryableConflict`. A new symlink in the ancestry gives `Layout(SymlinkedRoot)` from `ensure_symlink_free_ancestry` (:2372). Both guards release.
- **Same data directory inode moved under a new symlink-free ancestry at the same path:** evidence (path, identity, symlink-free) is equal, so the root is accepted. This is the same acceptance rule as `DataReadSession::admit` and `revalidate_layout`. The held repository lock lives inside that directory and moved with it, so it still excludes that root's writers. The retained bootstrap guard stays on the lock file opened under the old parent. It is not reacquired under the new parent. This matches `open_mutation_session` and recovery, which also bind data identity and not parent identity. Binding parent identity would change the lock subsystem and is not proposed.

### D25 and the boundary of this exception (unchanged; R1 row D-25 passed)

Exec is an explicit mutex/child operation, not an ordinary read. No ordinary reader goes through it. Config validate, item and issue reads and every other lock-free capture keep `DataReadSession` admission, bounded `RetryableConflict`, no writer lock and no replay. The R2 factoring leaves `DataReadSession::admit` semantics unchanged. A child JIT read stays lock-free. A child JIT mutation waits on the same lock and fails with the bounded `LockTimeout`. The existing CLI tests assert both. Exec never recovers. D26/D27 original-root replay and audit bytes are unchanged.

### Pre-existing lock-entry concern (separate, not fixed)

`FileLocker::lock_exclusive` opens the lock pathname (lock.rs:200–203, `open_or_create` :324–331), and `LockGuard` exposes no lock-file identity. The design cannot prove that the held `.repo-write.lock` inode is the entry inside E in two cases. First, the lock-file entry itself is replaced. Second, the root goes A→B→A between the step-3 and step-4 checks while the open resolved inside B. This is R1's pre-existing shared lock concern. `open_mutation_session` has the same exposure: it acquires the repository lock after `revalidate_layout` (:1044–1051) and does not revalidate afterward. R2 neither fixes it nor depends on fixing it. Closing it needs a lock-handle identity API in `storage/lock.rs` or `repo_lock.rs`, which is a separate owner for separate escalation.

## I/O source preservation (unchanged; R1 row passed)

Change only `RepositoryStateStoreError::Io` (repository_state_store.rs:89–90) to `#[error("{0}")]` with the existing `#[from]`, so the concrete `std::io::Error` stays in the source chain that `output/failure.rs:483–492` searches. Add the owning chain assertion in repository_state_store_tests.rs and rerun the unchanged permission projection. Add no classifier special case.

## Exact thirteen-file footprint and ownership

| File | Responsibility in combined correction |
|---|---|
| crates/jit/src/declarations/mod.rs | Existing canonical syntax/full/projection computation and owning tests |
| crates/jit/src/config.rs | Canonical full-view integration |
| crates/jit/src/storage/repository_state_store.rs | Existing admitted observations; **`DataRootHandle` factored from `DataReadSession`; `HeldDataRoot`/`DataRootHold` and `JsonFileStorage::hold_initialized_data_root` (retained evidence, lock-chain revalidation, pending presence, bound index capture)**; I/O source preservation |
| crates/jit/src/storage/repository_state_store_tests.rs | Existing observation migration/composition, I/O-source owner, **one handoff/replacement owner** |
| crates/jit/src/storage/json.rs | Shared data-only initialized index checks (existing `validate_captured_index`, `acquire_bootstrap_write_lock`, `acquire_repo_write_lock_raw` reused as is); no code layout or recovery for exec |
| crates/jit/src/commands/bootstrap.rs | Shared data discovery, required projections and explicit-root validation; recovery order unchanged |
| crates/jit/src/dispatch/mod.rs | Typed operation-error composition and explicit data-only exec context routing |
| crates/jit/src/commands/config.rs | Existing single data-only diagnostic report computation |
| crates/jit/src/dispatch/config.rs | Existing canonical diagnostic outcome/registered findings admission |
| crates/jit/tests/cli_repo_workflow/project_config_tests.rs | Existing diagnostic boundary tests; all original assertions retained |
| crates/jit/src/commands/mod.rs | Shared existing item configuration-error adapter; no actor-API change in 504 |
| crates/jit/src/commands/worktree.rs | Single data-only exec computation over `hold_initialized_data_root`; pending-message mapping; composition wiring tests |
| crates/jit/src/dispatch/worktree.rs | Shared registered exec outcome/status adapter |

No edit to failure_lever_registry.toml, worktree_exec_tests.rs, output/error taxonomy, generated facts, docs, actor files, main.rs, cli.rs, storage/mod.rs, capability_fs.rs, repo_lock.rs, lock.rs or a new Cargo target. `IssueStore::acquire_repo_write_lock` (json.rs:1073) stays for its conformance and test callers, and exec stops calling it. No 14th owner is required. Every primitive used is already `pub(crate)` or `pub(in crate::storage)` and reachable from repository_state_store.rs. If implementation cannot hold these contracts inside the footprint, report the concrete gap before editing.

## Bounded validation and review conditions

- **Handoff owner (one test, repository_state_store_tests.rs, beside `test_data_read_admission_detects_root_replacement` at :3400):** `test_hold_initialized_data_root_refuses_root_replaced_while_chain_held`.
  - Setup: for each `HoldPoint` (`LockChainAcquired`, `IndexCaptured`), build a fresh TempDir with an initialized data root A (valid index).
  - Interleaving: `hold_initialized_data_root_at` runs a hook at that point which renames A to `old` and creates an initialized B (valid index) at the data path. The state this forces is the post-state of R1's interleaving: the A chain is held while the path resolves to B. It does this without threads or timing, because whether the replacement happened during the wait or just after the grant cannot be observed by the following checks.
  - Assertions: the result is `Err(RepositoryStateStoreError::RetryableConflict { path })` naming the data path, and no `DataRootHold` is returned, so no child can start. Both A and B are valid, so only the retained-identity check can produce this error. `RepoWriteLock::for_storage_root(&old, Duration::ZERO).acquire()` and a zero-timeout bootstrap acquisition both succeed afterward, which proves the guards were released. B's bytes are unchanged.
  - Scope: no threads, sleeps, fixed paths or process-global state. The existing storage race matrix (`test_data_read_admission_detects_root_replacement`, `test_data_read_rejects_ancestor_published_during_absence_discovery` :3379, the :3314 composition and the recovery identity matrix) stays as it is and is not copied. The refactor must keep it green.
- **commands/worktree.rs (composition wiring only, existing fixtures):** exec reaches storage only through `hold_initialized_data_root`. `PendingRecord` maps to the existing message with no child. Selected data is independent of child cwd. There is no code or Git requirement. A clean missing or invalid index never runs the child. Pending plus an invalid index refuses pending without replay. A return or spawn error releases the lock so a later acquisition succeeds. No identity or replacement assertion is repeated here. Existing CLI worktree-exec tests still own child status, child read/mutation behavior, lock timeout and the unchanged pending refusal.
- Owning storage test proves I/O-source preservation. The unchanged permission-denied exit projection proves the public boundary.
- Shared adapter owner tests compare early typed declaration failure with the existing item full-view error computation. Run the complete unchanged failure-envelope corpus, including all nine registry arms.
- Keep the existing config report/repair, observation/rework1, D25/D26/D27 and policy tests. After implementation, run affected owners and full CLI/contracts without fail-fast, then fmt and all-target Clippy. The separate actor fix lands on main and is refreshed into 504 first.
- Fresh bounded independent design re-review of this R2 handoff precedes any edits. The lead owns source integration, the configured full gate and implementation reviews. No new retry cycle is granted.

## Counterexample walk (R1 finding 1)

R1's interleaving under the amended sequence:

1. Precheck: step 2 retains `E = (path P, identity id(A))` and `roots` on A.
2. Exec acquires the bootstrap lock (parent of P, unchanged). Step 3's `E.is_current()` is true because P still resolves to A. Exec calls `acquire_repo_write_lock_raw`, which opens `A/.repo-write.lock` (lock.rs:201) and polls while the other holder owns it.
3. A non-cooperating actor renames A away and publishes initialized B at P. The holder releases, and exec's flock is granted on A's lock inode.
4. Step 4 (`HoldPoint::LockChainAcquired`): `discover_data_read_evidence(P)` returns `id(B) ≠ id(A)`, so `E.is_current()` is false and the result is **`RepositoryStateStoreError::RetryableConflict { path: P }`**. If the identities matched but the replacement landed inside `reopen`, `ensure_capability_identity` returns the same conflict (capability_fs.rs:500–502 via the `From` at repository_state_store.rs:95–103).
5. `hold_initialized_data_root` returns early, so the repository guard on A's lock drops and then the bootstrap guard drops. No `DataRootHold` exists, commands/worktree.rs propagates the typed error through wrapper rendering, and **no child starts**. B is never read or accepted under A's guard. A later fresh invocation takes a new precheck on B and B's own lock.

The same replacement after the pending probe (step 5) or after the index capture (step 6, `HoldPoint::IndexCaptured`) fails at that step's `E.is_current()` with the same `RetryableConflict`, before the index is validated or any child launches.

## R1 resolution

| R1 required-resolution clause | Amended text | Source basis (frozen tree) |
|---|---|---|
| Storage-owned root-only capability/evidence survives lock acquisition | "Storage-owned retained root evidence": `DataRootHandle` factored from `DataReadSession`; step 2 "E is held for the rest of the operation and is never replaced by a fresh probe"; `HeldDataRoot { root, guards }` | repository_state_store.rs:443–475, 518–537; capability_fs.rs:92–138 |
| Binds the under-lock presence check and initialized capture to the same selected root | Steps 4–6: all reads go only through `roots := E.reopen()`, verified against E; `open_leftover_record(&roots)` and `inspect_capability_path(&roots, INDEX)` | :697–712, :2154; capability_fs.rs:491–504 |
| Revalidate after acquiring the chain and across the capture, before child launch | Step 3 (after bootstrap), step 4 (after full chain), step 5 (after pending probe), step 6 (after index capture, before validation and before return); step 7 launches only on `Initialized` | Mirrors :1043–1044 and :917–923; json.rs:751, :776; repo_lock.rs:217–235 |
| Changed/missing/unsafe root releases guards, typed failure, no child | Steps 3–4: `RetryableConflict { path }` for changed or missing, typed `Layout`/`UnsafeTarget`/`Io` for unsafe or unreadable; early return drops guards; step 8 order | repository_state_store.rs:59–103; repo_lock.rs:278–281, :326–333 |
| Replacement never silently becomes a fresh accepted root while old guard held | No fresh `DataReadSession` or capture under the guard (replaces the old step 5's use of json.rs:842–860 `load_index`); only E-bound `roots`; counterexample walk | json.rs:842–860 (no longer used by exec) |
| State what is retained for the data-bootstrap predecessor when root ancestry changes | "Data-bootstrap predecessor under changed root ancestry" | json.rs:515–519; lock.rs:200–203; repository_state_store.rs:2337–2390 (`discover_root_evidence`, `ensure_symlink_free_ancestry` :2372) |
| Reuse/factor `DataReadSession` evidence and `CapabilityRoots::open_data` in canonical owner; no private command protocol or artificial layout | `DataRootHandle` holds the moved lines 459–467; `DataReadSession::admit` reuses `is_current`/`reopen`; commands/worktree.rs only matches `DataRootHold`; no `RepositoryLayout` built | :451–499 |
| No exec-only lock workaround or lock rewrite; lock-entry note separate | Step 3 uses existing `acquire_bootstrap_write_lock`/`acquire_repo_write_lock_raw` in the mutation-session order; "Pre-existing lock-entry concern (separate, not fixed)" | lock.rs:200–203, :324–331; repository_state_store.rs:1044–1051 |
| Name owning deterministic handoff assertion in repository_state_store_tests.rs; command wiring only in commands/worktree.rs; preserve race matrix | "Bounded validation", first two bullets | repository_state_store_tests.rs:3379, :3400 |
| Feasible within thirteen files | Footprint table; no storage/mod.rs, capability_fs.rs, repo_lock.rs or lock.rs edit | storage/mod.rs:46 (`pub mod repository_state_store`) |
