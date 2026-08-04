# Load independence of the graceful-shutdown suite — 06f1fa95

> **Diátaxis Type:** Explanation (verification record)
> Recorded 2026-08-04 UTC on branch `worktree-agent-06f1fa95-r1`, anchored to
> `main` at `b4859c87`. Every run below names its command and its verdict.

## Scope

Evidence for `@/issue/06f1fa95/requirement/REQ-02` and
`@/issue/06f1fa95/requirement/REQ-03`, and the argument that closes the
assertion audit `@/issue/06f1fa95/requirement/REQ-01` asks for. The suite is
`crates/server/tests/server_integration/graceful_shutdown_tests.rs`.

Three things are recorded: the seeded regression and what runs it, every
remaining assertion in the module and why load cannot flip it, and the verdicts
of the affected test target run repeatedly with and without a saturating
concurrent build. The whole-gate comparison the execution lead owns has its own
section, left for the lead to fill.

## The seeded regression (REQ-02)

### Mechanism

`crates/server/src/shutdown.rs` declares `configured_drain_deadline()` in two
arms. Without the `test-support` feature it is a `const fn` returning
`GRACEFUL_DRAIN_TIMEOUT`. With the feature it reads `JIT_SERVER_DRAIN_DEADLINE_MS`
and falls back to the same constant. `crates/server/src/main.rs` passes its
result to `run_shutdown_sequence`, so the deadline a running server enforces is
the one thing the seam moves.

`crates/server/Cargo.toml` enables the feature the way `crates/jit` already
enables its own: a dev-dependency on the package itself
(`jit-server = { path = ".", features = ["test-support"] }`). Feature resolution
for a shipped build does not consider dev-dependencies, so the arm that reads
the environment is compiled only for this crate's test builds. Verified by
inspection of the built artifacts:

| Build | `strings target/debug/jit-server \| grep -c JIT_SERVER_DRAIN_DEADLINE_MS` |
| --- | --- |
| `cargo build -p jit-server` | `0` |
| `cargo build -p jit-server --features test-support` | `1` |

`test_the_drain_assertion_rejects_a_server_that_force_closes_early` spawns the
same binary through the same fixture as the healthy case, with the deadline set
to `SEEDED_EARLY_CLOSE_DEADLINE` (zero). That server stops accepting and retires
the stalled connection immediately, which is the regression the drain assertion
protects against: a connection stuck mid message closed short of the entitlement
`GRACEFUL_DRAIN_TIMEOUT` grants it. The test then runs the healthy case's own
`assert_survivor_drained_to_the_deadline` against that server's log and requires
it to panic.

### Why this shape rather than a `scripts/*-selftest.sh`

`scripts/cargo-ci-selftest.sh` and its siblings seed a defect into a throwaway
copy and run the shipped checker over it. Their fixtures are dependency-free
crates that compile in seconds. The defect here is in a compiled Rust server, so
the same shape means patching `crates/server/src/shutdown.rs` in a copy of this
repository and rebuilding. Cargo fingerprints path dependencies by path, so a
copied tree rebuilds the whole workspace rather than one crate, and that cost
would land inside `cargo-ci` on every evaluation — against the goal of the
container this issue belongs to. The dispatch permits an in-tree fault-injection
seam where no existing pattern fits; this is that case, and the seam converges on
the `test-support` feature convention `crates/jit` already establishes rather
than introducing a second mechanism (`@/invariant/convention-convergence`).

The demonstration is run by `cargo test`, in the existing
`server_integration` target — no new Cargo target, so the integration-target
budget of `@/invariant/bounded-rust-build-footprint` is unchanged.

### Controls

Each control edits one thing, runs the seeded case, and is reverted. Both
failures are the seeded case reporting that the demonstration went vacuous.

| Control | Edit | Result |
| --- | --- | --- |
| The seed does nothing | `SEEDED_EARLY_CLOSE_DEADLINE = GRACEFUL_DRAIN_TIMEOUT` | **FAILED** — `the drain assertion accepted a server that force-closed the survivor after draining for 5.001s` |
| The assertion is weakened | `drained_for >= Duration::ZERO` in `assert_survivor_drained_to_the_deadline` | **FAILED** — `the drain assertion accepted a server that force-closed the survivor after draining for 1ms` |

The first control also settles what the healthy case cannot: `drained_for_ms`
tracks the drain in both directions. It read `1ms` against a zero deadline and
`5.001s` against the five-second one, so the field is measured rather than
restated from a constant.

## Assertion audit (REQ-01, REQ-03)

Every assertion the module reaches, and what its outcome turns on. "Server-timed"
means both terms come from the observed process; "observer-timed" means the
fixture's own clock is in the comparison.

| Assertion | Timing | Why load cannot flip it |
| --- | --- | --- |
| `assert_survivor_drained_to_the_deadline` — `drained_for >= GRACEFUL_DRAIN_TIMEOUT` | Server-timed | `drained_for_ms` is measured across the server's own drain, on the clock its own deadline is scheduled against, stamped before that deadline exists. `sleep` never completes early, so load can only raise the number. The comparison has no scheduling margin to consume. |
| `drained_for < GRACEFUL_DRAIN_TIMEOUT` for a server with nothing to drain | Server-timed | Same field, opposite bound. Flipping it needs the server's own drain task starved for the whole five seconds between one sample and the next — a stopped host, not a busy one — and the failure is loud. |
| `open_connections` at the signal equals the number the fixture opened | Untimed | Counts, compared against the fixture's own connection list. |
| `open_connections` at the force close equals one | Untimed | A count. The fixture opens exactly one connection that cannot finish; the other three are retired by cancellation or the graceful drain, inside the deadline, and an event stream left waiting out its keepalive would appear here instead. |
| `drain_deadline_secs` equals `GRACEFUL_DRAIN_TIMEOUT.as_secs()` | Untimed | The server's report against the crate constant. |
| exit code is `0`, no signal death | Untimed | Process status. |
| `Shutdown complete` present | Untimed | Log content. |
| `wait_for_exit`'s budget — the process is gone within `EXIT_BUDGET` of the signal | Observer-timed | Kept: it is `@/issue/f04f7888/requirement/REQ-03`'s external bound, and an adopter's supervisor times it from outside too. The margin is five seconds — the server spends the other five draining by design — covering signal delivery, the forced close, teardown, and one `READ_POLL` slice. It is also the bound that keeps a stuck server from holding a job open (`@/issue/f3f7de97/requirement/REQ-08`'s concern), so removing it is not available. |
| `wait_for_close`'s budget, four connections | Observer-timed | A hang bound, not a property: when the server closed each peer is asserted separately from the server's own record. Each wait gets its own `EXIT_BUDGET` against a server that closes everything by the deadline. |
| `read_until`'s and `probe_health`'s `EXCHANGE_BUDGET` | Observer-timed | Hang bounds on loopback exchanges whose work is milliseconds. |
| `wait_for_listening_port`'s `STARTUP_BUDGET` | Observer-timed | Hang bound on an already-built binary starting against a fresh temporary repository. |
| `STOPPED_OBSERVATION_WINDOW` — no registration is reported while the server is `SIGSTOP`ped | Observer-timed, safe direction | Asserts that nothing happened inside a window. Load makes "nothing happened" more likely, so it can cost sensitivity and cannot produce a false failure. |

Two structural changes came out of this audit:

- The force-close case stated the exit bound twice — once inside `wait_for_exit`
  and again as `assert!(exited_after < EXIT_BUDGET)`. `wait_for_exit` now holds
  it alone, checked on the iteration that finds the process running and on the
  one that finds it gone, so the bound has one home and one argument.
- The connection count at the signal was the literal `"4"`. It is derived from
  the fixture's own list of open connections (`@/invariant/semantic-test-assertions`).

Both log-reading assertions rest on `drained_for_ms` being a live measurement.
The committed cases bracket it: the force-close case fails if the field is ever
pinned to zero, and the seeded case fails if it is ever pinned to the deadline
constant.

## Load experiment (REQ-03, target scope)

Ten repetitions of the affected target on an unloaded host, then ten with a
saturating concurrent build. Host: 24 cores. Command, each repetition:

```
CARGO_INCREMENTAL=0 cargo test -p jit-server --test server_integration graceful_shutdown
```

The load was two concurrent jobs: `openssl speed -multi 24 -seconds 25 sha256
aes-256-cbc`, and a from-scratch build of this workspace into its own target
directory, `cargo build --workspace --all-targets -j 24 --target-dir <scratch>`.
That build takes 42.93 s with the host otherwise idle and took 1 m 21 s beside
the repetitions, so the contention was real rather than nominal.

| Phase | 1-minute load average across the ten runs | Verdicts |
| --- | --- | --- |
| Unloaded | 0.39 – 0.78 | 10 × PASS (4 passed, 0 failed) |
| Saturated | 8.01 – 24.49 | 10 × PASS (4 passed, 0 failed) |

The loaded band spans and exceeds the 6–10 at which this suite failed a gate
before the fix, and reaches full saturation of the 24 cores. Per-run reported
suite durations stayed at 5.32–5.38 s in both phases: the drain the case waits
out is the server's five seconds, and the host's state did not move it.

## Whole-gate comparison (REQ-03, gate scope)

`cargo-ci` takes a host-wide build lock, so the execution lead runs it over the
merged tree — once with the host otherwise idle, once with a concurrent build
beside it — and records both here.

| Run | Condition | Gate-run identifier | Verdict |
| --- | --- | --- | --- |
| 1 | Idle host | _(lead records)_ | _(lead records)_ |
| 2 | Concurrent build | _(lead records)_ | _(lead records)_ |

## What remains bounded, and why

Every wait in the module still ends. Four of them are timed by the fixture, and
all four are bounds on a hang rather than statements of a property: `EXIT_BUDGET`
in `wait_for_exit` and `wait_for_close`, `EXCHANGE_BUDGET`, and `STARTUP_BUDGET`.
Removing one would leave an unbounded wait, which is the failure mode
`@/issue/f3f7de97/requirement/REQ-08` exists to prevent — a stuck case holding a
continuous-integration job to its own execution ceiling. Each is set orders of
magnitude above the work it bounds, and each fails loudly.

The seeded case carries the one comparison whose margin is worth naming: its
verdict would change if the seeded server's drain task were starved for the full
five seconds inside a window in which it is scheduled to do nothing. That is a
stopped host rather than a loaded one, and the outcome is a loud failure rather
than a silent pass.
