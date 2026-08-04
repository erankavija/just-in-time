# Survey — assertions in the cargo-ci suite whose outcome could turn on host load (4c700c80)

> **Diátaxis Type:** Reference (survey)
> Surveyed against the worktree branch for `4c700c80`, anchored to `main` at
> `44c2d358`, with this issue's commits applied. Every site below was read at
> that revision before it was recorded. Files this issue changes are cited by
> test name as well as by line, because their line numbers move with the change.

## Scope

The gate `cargo-ci` compiles and runs `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets`, `cargo test --workspace`,
`cargo test -p jit --test provenance_contract -- --ignored`, and
`scripts/rust-build-budget.sh`. The suite this survey covers is everything those
two `cargo test` invocations execute. `#[ignore]`d tests outside the provenance
suite are named where they carry a timing construct, and marked as not executed.

A site qualifies when a **busy host can fail it while the code under test is
correct**. Two adjacent shapes are recorded and distinguished:

- An assertion load can only make *more* true — a lower bound on elapsed time, a
  "this must not have happened yet" window, a wait that only lengthens — is not a
  verdict a busy host can flip, and is listed as such.
- An assertion read off a measurement the *system under test* took with its own
  clock is outside the host's schedule by construction.

## Method

Five searches, run from the worktree root, each result verified by reading the
cited file:

```bash
# 1. Every workspace file carrying a timing construct.
grep -rlnE "thread::sleep|Instant::now|elapsed\(\)|recv_timeout|wait_timeout|try_wait|Barrier::new" \
  crates/ --include="*.rs" | sort

# 2. Every such construct, with line numbers, in each of those files.
for f in $(...output of 1...); do grep -nE \
  "thread::sleep|Instant::now|\.elapsed\(\)|recv_timeout|wait_timeout|try_wait|Barrier::new" "$f"; done

# 3. Prose naming the removed thread cap, the build lock's reasons, or load sensitivity.
grep -rn "test-threads\|test_threads" scripts/ crates/ docs/ dev/ contrib/ .github/ CHANGELOG.md CLAUDE.md
grep -rn "load-sensitive\|timing-sensitive\|file-lock proptest" \
  scripts/ crates/ docs/ dev/ contrib/ .github/ CHANGELOG.md CLAUDE.md

# 4. Concurrency carrying no clock, which searches 1-2 would miss.
grep -rln "thread::spawn" crates/ --include="*.rs" | sort

# 5. Timing calls reached through an alias, which search 1's qualified names miss.
grep -rnE "^[^/]*[^:a-zA-Z_](sleep|now)\(" crates/ --include="*.rs" \
  | grep -vE "thread::sleep|Instant::now|Utc::now|clock\.now|self\.clock|SystemClock|\.now\(\)"
```

Search 1 returned 22 files; search 2 returned 115 construct sites, against 135
at the anchor `44c2d358` before this issue's commits. Each site was read in its
enclosing test and classified below.

Search 4 returns 17 files, 12 of them already in search 1. Of the five that are
not, `claim_coordinator_proptests.rs` and `repository_state_store_tests.rs`
carry the restatements `57675b68` and this issue landed and hold no clock;
`crates/jit/src/storage/json.rs:1584` (a spawned caller is refused while a
session is retained), `crates/jit/tests/cli_query_graph/check_links_tests.rs:19`
and `crates/jit/tests/cli_query_graph/remote_document_tls_tests.rs:78` (loopback
servers in background threads) assert outcomes the code enforces, with no
test-side deadline and no bound a slow host can exhaust.

**Search 5 is new to this re-run, and it found a site the earlier survey's
regex could not see.** A test that imports `sleep` and calls it unqualified is
invisible to search 1's `thread::sleep`. Two files do this:
`crates/jit/src/storage/temp_cleanup.rs` (five sites, all classified below as
unflippable) and `crates/server/src/shutdown.rs:515`, which is reported for
separate work. Search 5 also surfaces every `Clock` implementation, which is the
opposite shape — an injected clock is a supplied instant rather than a host read.

Production-code constructs
(`storage/lock.rs` poll loops, `storage/repo_lock.rs` owner waits,
`gate_execution.rs` child-process waits, `server/src/shutdown.rs` drain
sampling, `server/src/watcher.rs`, `server/src/routes.rs`, `profile/package.rs`,
`commands/serve.rs` startup probe) are the behaviour under test rather than
assertions about it, and are excluded.

## The restated properties still fail when the thing they protect breaks

A property restated to ignore the clock is only worth restating while it can
still fail, so every one was seeded and run. Each edit is one line, made to a
clean worktree at this issue's HEAD, run with `CARGO_INCREMENTAL=0`, and
reverted; `cargo test -p jit` is green before and after (4173 passed, 0 failed).

**The lock stops excluding (REQ-01, REQ-02).** `RepoWriteLock::acquire`'s
reentrancy check at `crates/jit/src/storage/repo_lock.rs:232` was changed from
`if state.owner == Some(me)` to `if state.owner.is_some()`, so a second thread
is admitted to a lock another thread holds. All three restated contention tests
failed:

```
test storage::repo_lock::tests::test_second_thread_waits_for_the_holder ... FAILED
test storage::repository_state_store::tests::test_session_disjoint_sessions_serialize_on_worktree_bootstrap ... FAILED
test commands::archive::tests::test_execute_waits_for_competing_repository_write_guard ... FAILED
test result: FAILED. 105 passed; 5 failed
```

The ordering assertions are what fired, not the bounds: the waiting threads
panicked at `repo_lock.rs:422` ("a second thread must not enter while the lock
is held") and at `repository_state_store_tests.rs:2639` ("a disjoint-data-root
session opened one worktree's bootstrap concurrently"). Their main threads then
reached the stall bound in `contention_probe.rs:85`, because a contender that is
never refused never records the refusal they wait for — the bound reporting
exactly the condition it exists to report. The other two failures are the
crossed-acquisition and shared-store contention tests, which read the same
check and place the seed.

**The archive stops converging (REQ-02).** Exclusion is not the whole of what
the archive test holds, so its second half gets its own seed:
`capture_archive_plan` at `crates/jit/src/commands/archive.rs:644` was made to
discard every captured base, so the archive retries to its limit and returns
`MutationSessionExhausted` after the write guard is released. The restated
assertion is the one that failed:

```
panicked at crates/jit/src/commands/archive.rs:1326:
assertion failed: done_receiver.recv().unwrap().is_ok()
test result: FAILED. 0 passed; 1 failed
```

The 100-millisecond "no result while the guard is held" assertion passed under
this seed, so the archive did wait for the guard and then failed to complete —
which is precisely the half the restatement reads.

**The lease misses its own boundary (REQ-03).** Both boundaries were seeded
separately, each by relaxing one comparison. `is_expired`'s
`clock.now() >= expires_at` at `crates/jit/src/storage/lease.rs:128` became `>`:

```
test storage::lease::tests::test_is_expired_turns_over_exactly_at_the_recorded_expiry ... FAILED
test result: FAILED. 10 passed; 1 failed
```

`is_stale`'s `elapsed_secs >= stale_threshold_secs` at
`crates/jit/src/storage/lease.rs:154` became `>`:

```
test storage::lease::tests::test_is_stale_turns_over_at_the_threshold_and_a_heartbeat_resets_it ... FAILED
test result: FAILED. 10 passed; 1 failed
```

Each seed failed exactly one test, and it is the one holding that boundary. A
supplied clock is what makes that possible: the tests set the instant one
millisecond before an expiry and then the expiry itself, which a sleeping test
cannot aim at.

## Result — sites changed by this issue

| Site | Assertion | Result |
| --- | --- | --- |
| `crates/jit/src/storage/repo_lock.rs:397` `test_second_thread_waits_for_the_holder` | the waiting thread enters only after the holder released | **Restated.** The contender retries a refused wait rather than unwrapping a five-second one, and reads the lock's answer. The holder releases once the contender has recorded being refused, so contention is produced rather than hoped for: the previous 150-millisecond sleep let a late contender pass the ordering assertion without ever meeting the lock held. |
| `crates/jit/src/storage/repository_state_store_tests.rs:2592` `test_session_disjoint_sessions_serialize_on_worktree_bootstrap` | the second session opens only after the first released the worktree bootstrap lock | **Restated**, the same way. The shared worktree bootstrap lock is registered with a wait shorter than any critical section, so the contending session is refused promptly and says so. |
| `crates/jit/src/commands/archive.rs:1297` `test_execute_waits_for_competing_repository_write_guard` | the archive completes once the competing write guard is released | **Restated.** The worker is joined before the result is received. A finished thread has already sent, so the receive completes without waiting, and the only remaining block is a join whose failure is a genuine deadlock. |
| `crates/jit/src/storage/lease.rs` (whole module) | expiry and staleness turn over at their recorded boundaries | **Restated.** `Lease` takes the `storage::clock::Clock` the claim coordinator already takes, so every time-dependent operation reads a supplied instant. The eleven tests set a clock instead of sleeping toward a boundary, and the module's tests run in no measurable time against the 4.8 seconds of sleeps they replace. |
| `crates/jit/src/storage/repo_lock.rs:250` `RepoWriteLock::acquire` | (production) an expired in-process owner wait | **Changed.** It returns the typed `LockTimeout` the file-lock wait already returns, so a caller distinguishes "still queued" from "refused" by type rather than by message (`@/invariant/semantic-types`). Without it, a retrying contender cannot tell a wait that expired from an operation that failed. |
| `crates/jit/src/storage/contention_probe.rs` | (mechanism) | **Added.** `Contenders`, `ProgressWatch` and `admitted_when_reached` carry the shape `57675b68` landed for the claim coordinator. `claim_coordinator::Claimants` composes them, so the coordinator, the repository lock and the worktree bootstrap lock share one mechanism rather than three copies (`@/invariant/convention-convergence`). |

The stall bound the mechanism carries is itself an elapsed-time assertion, and
it is recorded with the retained ones below rather than counted as clock-free.

## Result — remaining sites, by disposition

### Reported for separate work

| Site | Assertion | Margin | Restatement available |
| --- | --- | --- | --- |
| `crates/server/src/shutdown.rs:515` `test_run_shutdown_sequence_reports_drained_when_final_interval_empties` | the drain reports `Drained` rather than force-closing, because the last live connection completed inside the final sampling interval | 50 ms against a 400 ms deadline | **Found by search 5 in this re-run; not filed.** The test sleeps 3.5 sampling intervals and then completes the connection, so the completion must land in the fourth 100-millisecond interval before the 400-millisecond deadline. A stall of 50 ms in that window expires the deadline first and the outcome becomes `ForcedClosed`. This is the tightest margin in the suite outside `lock.rs`. Restating it means the drain loop taking its sampling schedule from outside rather than from `tokio::time`, which is a change to `drain_connections` in `crates/server`, outside this issue's four sites and touching shutdown semantics `76a4bd21` owns. |

### Filed as separate work

| Site | Assertion | Margin | Result |
| --- | --- | --- | --- |
| `crates/jit/src/storage/lock.rs:470,475` `test_exclusive_lock_prevents_concurrent_writes` | the holder acquired within 50 ms; the contender was refused while the holder slept 200 ms | 50 ms / 200 ms | Filed as `e3c6c767`, "Decide the lock tests on lock behaviour, not on scheduling", and in flight in the same wave as this issue. The tightest margins in the suite. Present at this survey's revision because that work is on its own branch; out of this issue's scope, and this issue's worker was directed not to touch the file. |

### Outside the schedule by construction

Each of these is read from a measurement the system under test took with its own
clock, so no assertion moves with what else the host is doing.

| Site | Assertion |
| --- | --- |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:529` | the force-closed survivor was given the whole drain deadline, from the server's reported `drained_for_ms` |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:605` | a server with nothing to drain stopped at once, from the same reported field |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:503,508,519` | the signal, the deadline and the open-connection counts the server logged |

### Load can only make them more true

A busy host lengthens every interval below, and every assertion here is a lower
bound, a "not yet" window, or an ordering the code enforces rather than the sleep.
None can be flipped to failure by load; where load matters at all it weakens
detection, which is a missed defect rather than a false verdict.

| Site | Why load cannot flip it |
| --- | --- |
| `crates/jit/tests/fast_docs_templates/template_apply_atomicity_tests.rs:601` | asserts the bystander waited *at least* half the injected stall |
| `crates/jit/src/storage/repository_state_store_contention_tests.rs:87,92,97` | peak concurrent holders observed through atomics; overlap is what a broken lock produces, and load only narrows it |
| `crates/jit/src/commands/init.rs:1203` | one of two racing initializations succeeds and one fails, whatever the schedule interleaves |
| `crates/jit/src/storage/repo_lock.rs:470,507` | assert that a held lock refuses a contender, and that crossed acquisition times out rather than hanging — outcomes a slow host produces more readily |
| `crates/jit/src/storage/repo_lock.rs:525` | the unwinding holder released before `join` returned |
| `crates/jit/src/commands/archive.rs:1316` | asserts *no* result arrives in 100 ms while the write guard is held; a slower host delays the result it forbids |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:565` | asserts the registration observation does *not* complete while the server is `SIGSTOP`ped |
| `crates/jit/src/storage/claim_coordinator.rs:2287` | asserts an expired lease is filtered after sleeping past its TTL |
| `crates/jit/src/storage/claim_coordinator.rs:2028,2049,2303,2623`, `crates/jit/src/commands/claim.rs:1308,1368,1546`, `crates/jit/src/commands/gate_check.rs:4704`, `crates/jit/tests/cli_repo_workflow/integration_test.rs:1077` | monotonic timestamp comparisons after a short sleep |
| `crates/jit/src/storage/temp_cleanup.rs:129,149,167,181,217` | cleanup by file age against a zero-second threshold |
| `crates/jit/src/commands/serve.rs:961` | asserts a blocking directory still exists; nothing removes it |
| `crates/jit/src/storage/contention_probe.rs:140` | the poll interval between observations of a subject; a longer one costs observations, never the verdict |

### Retained, with the argument

| Site | Assertion | Margin | Argument |
| --- | --- | --- | --- |
| `crates/jit/src/storage/contention_probe.rs:38,84` `CONTENTION_STALL_LIMIT`, `ProgressWatch::observe` | nothing the wait is watching has progressed for the limit | 30 s | An elapsed-time assertion, and the mechanism `57675b68` introduced, now shared. It is not a window a caller has to be scheduled inside: any observed progress resets it, so a run in which the lock keeps changing hands never spends it however slowly it does so. A host that starves every contender for thirty consecutive seconds does flip it, and that is the exposure this retention carries. The alternative is an unbounded wait, which returns the failure mode `@/issue/f3f7de97/requirement/REQ-08` exists to close: a stuck lock would hold a continuous-integration job to its execution ceiling instead of failing. `@/issue/57675b68/requirement/REQ-01` was amended by the owner to permit a bound of this shape — one that fires only when nothing at all is progressing, and fails loudly when it does. |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:497` | the server exits strictly inside ten seconds of the signal | product requirement | The bound is the shipped contract the test exists to hold, not a margin chosen to be long enough. Weakening it would weaken the requirement. |
| `crates/server/tests/server_integration/graceful_shutdown_tests.rs:116,178,296,318` | startup, read and close waits give up at their budgets | 10–30 s | Stated at the call sites as hang bounds rather than properties; the properties themselves are asserted from the server's own record. Removing them restores an unbounded wait, which is the failure mode `76a4bd21` closed. |
| `crates/server/src/shutdown.rs:270` `TEST_BUDGET` | every await in the drain suite reaches a verdict inside five seconds | 5 s | A hang bound on awaits that would otherwise never return, not the property under test. Distinct from `:515`, which is a schedule the test's own timing has to hit. |
| `crates/jit/tests/cli_repo_workflow/serve_cli_tests.rs` | seeding a repository, and the foreground-serve case, reach a verdict inside their budgets | 15–90 s | Bounded deliberately by `76a4bd21` after this test held four CI jobs to a six-hour execution ceiling. The trade — a budget that a saturated host can exceed, against a wait nothing bounds — is that issue's decision. |
| `crates/jit/src/profile/package.rs:1749` `read_within` | a read that would block forever becomes an assertable `None` | budget | The same shape: a hang bound on its own thread, stated as such at the call site. |
| `crates/jit/src/gate_execution.rs:804` | a gate that times out returns inside ten seconds | 10 s against a 1 s timeout | The test spawns a background grandchild that holds the pipe for 60 s; the assertion separates "the group kill released the pipe" from "we waited for the grandchild". Nothing in the test competes for the budget, so only a whole-process stall of nine seconds can flip it, and the alternative to the bound is a hang. |
| `crates/jit/src/validation/graph.rs:4022` | evaluating 400 issues completes within five seconds | ~1000× | A single-pass in-memory evaluation measured in milliseconds; the regression it catches is an accidental quadratic, three orders of magnitude away. |
| `crates/jit/src/storage/temp_cleanup.rs:118` | a freshly created file is not removed at a ten-second threshold | 10 s | Flipping it requires the process to stall ten seconds between creating one empty file and reading its age. |

## Not executed by the gate

`crates/jit/tests/fast_docs_templates/lock_tests.rs` is six `#[ignore]`d tests
whose bodies are commented-out placeholders; the sleeps at `:120,:124,:184,:187`
run in no gate step, and the assertions that remain compare a counter the test
itself increments. They are inert rather than load-sensitive.

## Observations

Two things the searches surfaced that are neither load-sensitive assertions nor
this issue's work, recorded so they are not rediscovered:

- **`storage::lease` has no caller.** Nothing in the workspace names
  `storage::lease::Lease`; the crate's canonical `storage::Lease` is
  `claim_coordinator::Lease`, re-exported at
  `crates/jit/src/storage/mod.rs:52`. The module is reached only by
  `pub mod lease;` and its own tests. REQ-03 names the injected clock, so it was
  supplied as written, and this is recorded for the owner's decision on
  `@/invariant/canonical-cutover`.
- **Two clock abstractions.** `crates/jit/src/storage/clock.rs` declares `Clock`
  and `crates/jit/src/repository_state/mutation.rs:36` declares `MutationClock`.
  Both are injected wall-clock sources returning `DateTime<Utc>`, each with a
  system implementation and a fixed test implementation. `@/invariant/convention-convergence`
  reads on one convention having one form; neither is load-sensitive, and
  converging them is outside this issue.
