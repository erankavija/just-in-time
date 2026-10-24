# Read concurrency for `read-only-session` (b697a3f7)

Scope: the read path of archive preview and candidates, snapshot export, declaration reads (REQ-10), gate-preset reads and previews. Citations are against HEAD 8cf19175a.

## 1. Options

The facts that decide the choice:

- A mutation session takes the worktree bootstrap lock, the bootstrap lock and then the exclusive repository lock (`repository_state_store.rs:453`, `:458`, `:466`; `repo_lock.rs:217`, `:247`). Running reads through it serializes every reader against every writer and against every other reader. `snapshot export` already does this today, because it is recovery-dispatched (`cli.rs:2897`) and so retains the startup session for the whole command (`main.rs:1893-1899`).
- Previews and candidates read lock-free today (`archive.rs:158-200` reads storage directly). The plan's contract would therefore add serialization where none exists.
- A put stages a new file and renames it into place (`redo_apply.rs:233`). Each file read is therefore one whole version. A multi-file image can still be torn across a writer's puts.
- The event log changes only by a positioned append (`image.rs:1299`). That append runs after every put and delete and before the record is removed (`redo_apply.rs:117-125`, `:308-319`).

| Option | Verdict | Reason |
|---|---|---|
| **A. Optimistic capture + seqlock admission** (candidate 1, refined) | **Chosen** | Readers take no lock, so they never wait on writers or on each other. Correctness comes from two facts the store already maintains: the redo record's presence and the event log's length. The only new code is one admission check and one driver; capture, `EventLogTail`, the record probe and the locked session are reused (D-18). It reads a bounded set: the record's name plus the log's length and tail (D-19). |
| B. Shared/exclusive flock (candidate 2) | Rejected | Readers still block on an active writer, which fails the required outcome. Linux `flock` does not queue a pending `LOCK_EX` ahead of new `LOCK_SH` requests, so a steady stream of readers starves writers. It also adds a lock mode across the reentrant `RepoWriteLock` (`repo_lock.rs:13-23`) and its bootstrap predecessors. Leftover-record handling would still need an escalation from shared to exclusive. |
| C. Capture without a session and without admission (candidate 3) | Rejected | It can return a torn image, mixing files from before and after a writer's puts. It can also observe a crashed writer's half-applied record. Previews would then disagree with execute, the drift D-20 exists to prevent. |
| D. Double-collect: re-read every captured entry and listing, plus check record absence | Rejected as the base | It is unsound under ABA. `EntryIdentity` is `dev:ino` plus SHA-256 plus size (`:1868`), and ext4 reuses freed inode numbers. A path toggled back to its old bytes by writes that append no event can then pass both collects. It also costs a second read of the whole read set. |
| E. Overlay a leftover record on the image in memory (read-only filesystems) | Deferred | This would be a third redo applier, next to the filesystem one and the memory backend's. The case it serves (a leftover record on a read-only mount) gets an explicit error instead (§3). |

D-20 requires one computation per capability, not one lock discipline. A preview calls the same capture-and-derive function as its operation (§4). Only the driver that hands that function a session differs.

## 2. Protocol and soundness

### Reader (one optimistic attempt)

1. **Open.** `CapabilityRoots::open` and `revalidate_layout`, with no locks. Probe the record name exactly as `read_leftover_record` does (`:255`), but only for presence. If a record is present, end the attempt with `Busy`. Otherwise read `L0 = live_event_log_tail` (`:1065`), the log's length plus its tail window.
2. **Capture.** Run the operation's capture/extend/derive closure unchanged (`capture_capability_image` `:1098`, `capture_memory_image` `:1213`).
3. **Admit,** in this order: `revalidate_layout_capabilities`, then the record probe (it must be absent), then read `L1` and require `L1 == L0`. For an attempt opened over an absent data root, require the root to still be absent and the worktree record path to still be empty.
4. On a pass, return the closure's result, whether `Ok` or `Err`. On a failure, discard the result, including any error. A torn image can make derivation fail, for example on a dangling reference, so an error is surfaced only after admission. Then retry (§3).

### Writer invariant this relies on (new, see the decision in §7)

**Every redo record committed over an existing data root carries a non-empty event append.** The log only grows (`image.rs:1299`). Replay rewrites the same bytes at the same offset, and replay happens only while the record is present.

At HEAD this invariant does not hold. `compose_event_action` returns no append when there are no pending events (`mutation.rs:1182`), and `redo_record_for` takes the append as optional (`:1021`). Observed: `jit config set` rewrites `config.toml` and leaves `events.jsonl` unchanged. This is the hole in the obvious protocol: two event-less writers can tear an image with `L0 == L1`.

The fix with the least mechanism is to make `RepositoryDelta` validation reject a non-empty delta over a present data root that has no `AppendEvents`. Event-less operations then emit one generic event (working name `files_published { paths }`), added by the finalizer only when the plan has no other events. The cost is one event line per declaration, projection or profile-tree write, plus an event-catalog entry. In exchange, `@/inv/event-log` ("every state change appends an event") becomes literally true. The alternative is an 8-byte generation counter in `.repo-write.lock`, which adds a second version source and its own torn-read concern.

### Proof sketch

Let W be any writer with a live write inside the capture window (s, e). W committed its record before its first live write (`redo_record.rs:208-215`). W's append comes after its last live write, and its record removal comes after the append.

- **Record committed before the open-time probe.** Either the record is still present at the probe, which gives `Busy`, or it was removed before the probe. Removal before the probe puts all of W's live writes before s, which contradicts the assumption.
- **Record committed after the probe.** W's append follows a live write later than s, so it lands after `L0` was read. At admission the probe runs before `L1`. If the record is present, the attempt is refused. If it is gone, W appended before the probe and therefore before `L1`, so `L1 > L0` and the attempt is refused.

An admitted image is therefore free of any writer's live write in (s, e). Every file read sees one whole version, so the image equals the repository state at s. The cases the assignment names:

| Case | Outcome |
|---|---|
| Writer starts and finishes inside the window | `L1 > L0` (append), so refused. |
| Writer mid-apply at open | Record present, so `Busy`. |
| Crashed writer (process or machine) | Its record persists, so every attempt reports `Busy` until the locked fallback applies the record (§3). Non-durable state does not matter, because a machine crash kills the reader too. |
| ABA on bytes with a reused inode | Irrelevant. Admission compares no entry identities; any write in the window grows the log. |
| Directory listings | Covered the same way. Listings change only through records (make-directory, put, delete), and each record now appends. |
| Event-log tail | `L` is length plus tail window (`EventLogTail`). A whole-mode read of `events.jsonl` during an append can see a partial line, and the admission check refuses that attempt. |
| Absent root published by `jit init` | The root appears by a no-replace rename (`:629-686`). The admission re-check of root absence refuses the attempt. A worktree target is applied from the record carried into the root, which the probe sees. |
| In-process server | Writer threads serialize on `RepoWriteLock`'s in-process owner state (`repo_lock.rs:226-260`). Readers never touch it, nor the session's layout-reentry or order guards, so a reader thread runs beside a writer thread. The argument above is about filesystem observations only, so it holds unchanged across threads. |
| Reader inside a held mutation session (same thread) | No concurrent writer can be mid-apply, so the attempt is admitted. |

### What is outside the guarantee (same as D-17)

External edits such as `git checkout` or an editor append no event. A reader can observe one mid-way, as the locked session can today, because the lock does not stop git. Claims and leases keep their own coordinator locks.

## 3. Liveness

- **Budget.** `READ_OPTIMISTIC_ATTEMPTS = 4` attempts, with jittered exponential backoff (about 1, 2, 4 and 8 ms) after a `Busy` result or a refused admission. A closure-level `Retry`, such as a capture-budget rediscovery, counts against the same budget.
- **Fallback.** After the budget, run exactly one more pass through the existing `with_mutation_session`, which holds the exclusive lock and returns `SessionStep::Done`. This is today's discipline, used only on contention, so it guarantees progress. Under sustained writes, readers that fall back serialize with writers and with each other. That is the documented degradation.
- **Model.** For a commit rate λ and a read duration d, P(fallback) ≈ (1 − e^(−λd))^4. With 20 agents each writing every 10 s (λ = 2/s) and d = 50 ms, the result is about 10⁻⁴.
- **Leftover record.** A crashed writer's record makes each attempt `Busy`, so the fallback is reached after roughly 15 ms. The fallback's `open_mutation_session` applies the record (`:466-469`), as `jit validate` does today. No try-lock or separate escalation path is added.
- **Read-only filesystem or no write permission.** No writer can run there, so the optimistic path succeeds unless a leftover record exists. The fallback then fails while opening or creating the lock file, or while applying. Map that failure to a typed error, `LeftoverRedoRecord` ("run `jit recover` with write access"). The reader never serves the torn pre-record state.

## 4. Interface shape

`crates/jit/src/storage/repository_state_store.rs`:

- Split `RepositoryMutationSession` (`:199`) into `RepositoryReadSession { layout, capture, extend }` and `RepositoryMutationSession: RepositoryReadSession { recovery_report, apply }`. MSRV is 1.97, so a `&mut dyn RepositoryMutationSession` upcasts to `&mut dyn RepositoryReadSession`.
- `RepositoryStateStore` (`:221`) gains `open_read_session(layout) -> Result<Box<dyn OptimisticReadSession>>`. `OptimisticReadSession: RepositoryReadSession` adds `admit(&self) -> Result<(), ReadAdmission>`, where `ReadAdmission` is `Busy` or `Moved`.
- One `admit_against(probe_record, live_log_tail)` closure-style helper serves both backends. It mirrors `ensure_write_set_unchanged_against` (`:916`).
- **JSON backend.** `JsonReadSession { layout, roots, storage, l0 }` holds no guards. Its capture reuses `capture_over`'s body (`:591`).
- **Memory backend.** `MemoryReadSession` reads `state.record.is_some()` and the `EVENTS` entry's tail under the state mutex, at open and at admission. It captures through `capture_memory_image`. The memory writer commits in one mutex section (`memory.rs:499`) and applies the operations, the append and the record removal in another (`memory.rs:539`), so the same proof holds.
- **Conformance.** One shared contract suite runs on both backends (`@/inv/shared-test-contracts`), in `repository_state_store_tests.rs` (or the contention suite):
  - A full mutation run between `capture` and `extend` makes `admit` fail.
  - An event-less delta is rejected.
  - A record left at each `RedoPoint` (existing seam, `redo_apply.rs:31-48`) gives `Busy`.
  - The fallback applies a leftover record.
- **Event-append invariant.** `crates/jit/src/repository_state/image.rs` adds the delta validation. `repository_state/mutation.rs` (`compose_event_action`, `:1177`) adds the generic event.

`crates/jit/src/commands/mod.rs`:

- Add `with_read_session(store, layout, op, |&mut dyn RepositoryReadSession| -> Result<Option<T>>)` next to `with_mutation_session` (`:452`). It runs the optimistic attempts and then the locked fallback through `with_mutation_session`.
- Retype `capture_discovering` and `grow_discovering` (`:336`) to take `&mut dyn RepositoryReadSession`.
- **D-20.** The operation's capture-and-derive function takes `&mut dyn RepositoryReadSession`. Execute calls it with its mutation session; the preview calls it through `with_read_session`. For example, `capture_archive_plan` (`archive.rs:511`) serves execute (`:850`) and preview and candidates alike.

Consumers covered:

| Consumer | Files |
|---|---|
| Archive preview and candidates | `commands/archive.rs:158-200` |
| Snapshot export | `commands/snapshot.rs:669`, `commands/mod.rs:2262` (`plan_repository_export`) |
| Declaration capture helper and its CLI and server reads | `main.rs`, `crates/server/src/` routes |
| Gate-preset list and show | — |
| Bulk dry-run derivation | `bulk_update.rs` |

**Plan contract text.** Replace `read-only-session`'s body (`plan.md:35-37`) and DEC-09 (`plan.md:488`). The rejected alternative there, "a lock-free read-session type … would need its own leftover-record rule", is answered by §3: its leftover rule is the existing locked open.

## 5. Other locked read paths

| Path | Scope | Reason |
|---|---|---|
| `snapshot export` retained startup session (`cli.rs:2897` → `main.rs:1893`) | **In scope (flag)** | The command holds the write lock for its whole run, which makes moving its session pointless. Classify it as recovery-dispatched only when the destination lies inside a repository root, or let its publish session do recovery. |
| `jit validate` without `--fix`, and `--fix --dry-run` (`validate.rs:131`, `:156`, `:510`) | Out of scope; recommended addition | It is a pure read, the heaviest one, run often by agents. The move is mechanical once the driver exists. |
| `doc check-links` (`document.rs:1380`) | Out of scope; recommended | Pure read. It is touched anyway by `check-links-through-issue-scope`. |
| Profile planning and dry-run (`profile.rs:825`, `:1286`, `:606`) | Out of scope; recommended | Previews (D-20). |
| Gate-checker validation capture (`gate_check.rs` via `capture_validation_image_with`) | Out of scope | It runs inside gate evaluation, a mutation flow. Move it later with validate. |
| Pre-executor write-policy read (`plan.md:507`) | Unchanged | Already lock-free. |
| Server startup recovery (`crates/server/src/lib.rs:65-80`) | Unchanged | Recovery, dropped before serving (`server/src/main.rs:86`). |

Recommendation: add one task, `locked-reads-optimistic`, after `read-session-driver`, covering validate, check-links and profile planning.

## 6. Measurable criterion

**Deterministic owning test (gate-worthy, insensitive to host noise).** In the contention suite, run both backends:

- One thread opens a mutation session and holds the write lock for the duration (an active writer between capture and commit).
- N = 8 threads each run `with_read_session`. Inside the closure, each waits on a `Barrier(8)` with a 10 s timeout, then returns.
- Pass when all 8 readers pass the barrier and admit while the writer still holds the lock. Serialized readers can never fill the barrier.
- A second test, which releases the writer only after the readers pass, asserts that a writer still commits with readers active.

**Benchmark (evidence of scaling).** Put it in `dev/benchmarks/read-concurrency-<issue>/`, following `nextest-concurrency-934985ca`: `benchmark.py`, `validate.py`, `raw/`, `summary.json`, `README.md`, `SHA256SUMS`. The fixture comes from `scripts/benchmark-session-cost.sh`'s generator (665 issues), using the release binary.

- **Arms.** One continuous writer loop (`jit issue update <id> --priority …`, alternating values) plus N ∈ {1, 2, 4, 8} reader loops of `jit archive container <id> --json`, which previews by default, for a fixed 15 s window per arm.
- **Contrast.** The same arms on the pre-change binary, recorded in the artifact.
- **Measured.**
  - Aggregate reader completions per second, R(N).
  - Writer completions per second, W(N).
  - Reader p95 latency.
- **Pass, both conditions.** Both are ratios within one run, so they hold on a noisy host.
  - S(8) = R(8)/R(1) ≥ 4 when the host has at least 16 logical CPUs; on smaller hosts N is capped at CPUs/2 and the threshold is N/2. The serialized baseline gives S ≈ 1.
  - W(8)/W(0) ≥ 0.8, so readers do not starve the writer.
- **Stability.**
  - Arms interleave as ABAB over 5 rounds, and the run reports the median of per-round ratios.
  - A round is rejected when the 1-minute load average at its start exceeds CPUs/4.
  - The artifact records the CPU count, kernel and `measurement_fs`.
  - `validate.py` rederives the verdict from `raw/`.

## 7. Charter

The design conflicts with the letter of D-20 ("runs the same mutation session as its operation"). It also adds a writer obligation beyond D-17. Recommended:

- **New D-22:** "Reads never wait on writers. A read captures without locks and is admitted only when no redo record was present and the event log did not grow across its capture; it retries a bounded number of times and then reads under the repository lock, which also completes a leftover record. Every redo record appends at least one event, so the log's length is the repository's commit counter."
  - **Rejected:** a shared/exclusive repository lock, which still blocks readers on writers and lets readers starve writers; re-reading the read set, which is unsound under inode reuse.
- **Amend D-20:** "A preview runs the same capture and derivation as its operation, read-only."
- **Amend D-17:** append "and every record appends an event" to the record's contents.
- **`@/inv/event-log`:** no amendment is needed, since it already states the rule. Its enforcement moves into delta validation.
