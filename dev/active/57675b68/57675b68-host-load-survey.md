# Survey — assertions in the cargo-ci suite whose outcome could turn on host load (57675b68)

> **Diátaxis Type:** Reference (survey)
> Surveyed against the worktree branch for `57675b68`, anchored to `main` at
> `b4859c87`. Every site below was read at that revision before it was recorded.
> The two files this issue changes are cited by test name as well as by line,
> because their line numbers move with the change.

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

Three searches, run from the worktree root, each result verified by reading the
cited file:

```bash
# 1. Every workspace file carrying a timing construct.
grep -rlnE "thread::sleep|Instant::now|elapsed\(\)|recv_timeout|wait_timeout|try_wait|Barrier::new" \
  crates/ --include="*.rs" | sort

# 2. Every such construct, with line numbers, in each of those files.
for f in $(...output of 1...); do grep -nE \
  "thread::sleep|Instant::now|\.elapsed\(\)|recv_timeout|wait_timeout|try_wait|Barrier::new" "$f"; done

# 3. Prose naming the proptests, the removed thread cap, or the build lock's reasons.
grep -rn "test-threads\|test_threads" scripts/ crates/ docs/ dev/ contrib/ .github/ CHANGELOG.md CLAUDE.md
grep -rn "prop_concurrent_different_issues_succeed\|prop_concurrent_claims_exclusive\|load-sensitive\|timing-sensitive\|file-lock proptest" \
  scripts/ crates/ docs/ dev/ contrib/ .github/ CHANGELOG.md CLAUDE.md
```

Search 1 returned 23 files; search 2 returned 158 construct sites. Each site was
read in its enclosing test and classified below.

A fourth search covers concurrency that carries no clock, which the first three
would miss: `grep -rln "thread::spawn" crates/ --include="*.rs"` returns 16
files, 12 of them already in search 1. Of the four that are not,
`claim_coordinator_proptests.rs` is the subject of this issue and is recorded
below; the other three —
`crates/jit/src/storage/json.rs:1584` (a spawned caller is refused while a
session is retained), `crates/jit/tests/cli_query_graph/check_links_tests.rs:19`
and `crates/jit/tests/cli_query_graph/remote_document_tls_tests.rs:78` (loopback
servers in background threads) — assert outcomes the code enforces, with no
test-side deadline and no bound a slow host can exhaust.

Production-code constructs
(`storage/lock.rs` poll loops, `storage/repo_lock.rs` owner waits,
`gate_execution.rs` child-process waits, `server/src/shutdown.rs`,
`server/src/watcher.rs`, `server/src/routes.rs`, `profile/package.rs`,
`commands/serve.rs` startup probe) are the behaviour under test rather than
assertions about it, and are excluded.

## Result — sites changed by this issue

| Site | Assertion | Result |
| --- | --- | --- |
| `crates/jit/src/storage/claim_coordinator_proptests.rs:293` `prop_concurrent_different_issues_succeed` | every concurrent claim on a distinct issue is granted | **Restated.** The property is decided on what the coordinator answered. An expired lock wait is `storage::lock::LockTimeout`, which reports only that the caller was still queued, and the claimant asks again. The granted set is compared against the requested set. |
| `crates/jit/src/storage/claim_coordinator_proptests.rs:203` `prop_concurrent_claims_exclusive` | exactly one of up to twenty claimants is granted one issue | **Restated**, the same way, plus an assertion that every refusal is the coordinator's answer rather than an expired wait. |
| `crates/jit/src/storage/claim_coordinator.rs:1838` `test_concurrent_claim_attempts_serialize` | exactly one of twenty coordinators is granted one issue | **Restated.** Reaches the coordinator through the retrying path. |
| `crates/jit/src/storage/claim_coordinator.rs:1933` `..._grants_every_distinct_issue_when_every_lock_wait_expires` | as above, with every wait forced to expire | **Restated**, and the forcing made deterministic: the test holds the claims lock until every claimant has been refused it, so the condition is produced rather than hoped for. |
| `crates/jit/src/storage/claim_coordinator.rs:1973` `..._grants_exactly_one_claimant_when_every_lock_wait_expires` | as above | **Restated**, same mechanism. |
| `scripts/cargo-ci.sh` | (not an assertion) the build lock's stated reason named this proptest as load-sensitive | **Removed.** CPU oversubscription and peak RAM remain; they survive the change. The `ionice` class choice is restated on the gate's own completion instead of on the proptests. |

The retry that carries the restatement is bounded on the claimants' progress, not
on a clock the caller races: it fails when no claimant at all has been answered
for `CLAIMANT_STALL_LIMIT`
(`crates/jit/src/storage/claim_coordinator.rs:1432,1472`). Load slows claimants
without stopping them, so a stall of that length with claimants still queued is a
stuck lock rather than a lost race.

## Result — remaining sites, by disposition

### Filed as separate work

| Site | Assertion | Margin | Result |
| --- | --- | --- | --- |
| `crates/jit/src/storage/lock.rs:476,494,498` `test_exclusive_lock_prevents_concurrent_writes` | the holder acquired within 50 ms; the contender was refused while the holder slept 200 ms | 50 ms / 200 ms | Filed as `e3c6c767`, "Decide the lock tests on lock behaviour, not on scheduling". The tightest margins in the suite. Out of this issue's scope. |

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
| `crates/jit/src/commands/init.rs:1232` | one of two racing initializations succeeds and one fails, whatever the schedule interleaves |
| `crates/jit/src/storage/repo_lock.rs:449,486` | assert that a held lock refuses a contender, and that crossed acquisition times out rather than hanging — outcomes a slow host produces more readily |
| `crates/jit/src/storage/repo_lock.rs:504` | the unwinding holder released before `join` returned |
| `crates/jit/src/commands/archive.rs:1315` | asserts *no* result arrives in 100 ms while the write guard is held; a slower host delays the result it forbids |
| `crates/server/.../graceful_shutdown_tests.rs:565` | asserts the registration observation does *not* complete while the server is `SIGSTOP`ped |
| `crates/jit/src/storage/lease.rs:291,326,462` | assert expiry or staleness *after* sleeping past a one-second bound |
| `crates/jit/src/storage/claim_coordinator.rs:2339` | asserts an expired lease is filtered after sleeping past its TTL |
| `crates/jit/src/storage/claim_coordinator.rs:2080,2101,2355,2675`, `crates/jit/src/commands/claim.rs:1316`, `crates/jit/src/commands/gate_check.rs:4708`, `crates/jit/tests/cli_repo_workflow/integration_test.rs:1111` | monotonic timestamp comparisons after a short sleep |
| `crates/jit/src/storage/temp_cleanup.rs:133,152,170,188` | cleanup by file age against a zero-second threshold |
| `crates/jit/src/commands/serve.rs:965` | asserts a blocking directory still exists; nothing removes it |

### Retained, with the argument

| Site | Assertion | Margin | Argument |
| --- | --- | --- | --- |
| `crates/server/.../graceful_shutdown_tests.rs:497` | the server exits strictly inside ten seconds of the signal | product requirement | The bound is the shipped contract the test exists to hold, not a margin chosen to be long enough. Weakening it would weaken the requirement. |
| `crates/server/.../graceful_shutdown_tests.rs:116,178,296,318` | startup, read and close waits give up at their budgets | 10–30 s | Stated at the call sites as hang bounds rather than properties; the properties themselves are asserted from the server's own record. Removing them restores an unbounded wait, which is the failure mode `76a4bd21` closed. |
| `crates/jit/tests/cli_repo_workflow/serve_cli_tests.rs:151,644,669,673` | seeding a repository, and the foreground-serve case, reach a verdict inside their budgets | 15–90 s | Bounded deliberately by `76a4bd21` after this test held four CI jobs to a six-hour execution ceiling. The trade — a budget that a saturated host can exceed, against a wait nothing bounds — is that issue's decision. |
| `crates/jit/src/gate_execution.rs:803` | a gate that times out returns inside ten seconds | 10 s against a 1 s timeout | The test spawns a background grandchild that holds the pipe for 60 s; the assertion separates "the group kill released the pipe" from "we waited for the grandchild". Nothing in the test competes for the budget, so only a whole-process stall of nine seconds can flip it, and the alternative to the bound is a hang. |
| `crates/jit/src/validation/graph.rs:4029` | evaluating 400 issues completes within five seconds | ~1000× | A single-pass in-memory evaluation measured in milliseconds; the regression it catches is an accidental quadratic, three orders of magnitude away. |
| `crates/jit/src/storage/temp_cleanup.rs:118` | a freshly created file is not removed at a ten-second threshold | 10 s | Flipping it requires the process to stall ten seconds between creating one empty file and reading its age. |

### Reported for separate work

None of these is filed. Each is the shape this issue closes — a wall-clock wait
deciding a test that is about ordering — at a wider margin, and each has a
restatement available.

| Site | Assertion | Margin | Restatement available |
| --- | --- | --- | --- |
| `crates/jit/src/storage/repo_lock.rs:402` `test_second_thread_waits_for_the_holder` | the waiting thread's `acquire()` succeeds, and it entered only after the holder released | 5 s wait against a 150 ms hold | The ordering assertion at `:404` is decided by the lock and is sound. The `unwrap()` at `:402` is not: a five-second stall turns the wait into a failure about scheduling. |
| `crates/jit/src/storage/repository_state_store_tests.rs:2620` `test_session_disjoint_sessions_serialize_on_worktree_bootstrap` | the waiting session opens, and only after the holder released | 5 s wait against a 150 ms hold | Same shape, same reading. |
| `crates/jit/src/commands/archive.rs:1320` `test_execute_waits_for_competing_repository_write_guard` | the archive completes within two seconds of the guard being released | 2 s | Join the worker first and then receive: a finished thread has already sent, so the result is read without any wait, and the only remaining block is a join whose failure is a genuine deadlock. |

One further cluster is worth an owner's decision rather than a silent retention:
`crates/jit/src/storage/lease.rs` builds leases with a one-second TTL and asserts
staleness against a one-second threshold, then reasons across sleeps of 100–900 ms
(`:286,:322,:330,:398,:439,:456`). Each of those assertions holds only while the
process stays inside the remaining fraction of a second. `Lease` reads
`Instant::now()` and `Utc::now()` directly (`crates/jit/src/storage/lease.rs:138,193`),
so the restatement is an injected clock of the kind `ClaimCoordinator` already
takes (`crates/jit/src/storage/clock.rs`), which is a change to the type rather
than to its tests. Recorded here; not filed.

## Not executed by the gate

`crates/jit/tests/fast_docs_templates/lock_tests.rs` is six `#[ignore]`d tests
whose bodies are commented-out placeholders; the sleeps at `:120,:124,:184,:187`
run in no gate step, and the assertions that remain compare a counter the test
itself increments. They are inert rather than load-sensitive.
