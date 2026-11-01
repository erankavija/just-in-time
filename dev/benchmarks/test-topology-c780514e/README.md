# Test-topology measurements

> **Diátaxis Type:** Reference

## Doctest substep — 871df4b0

The measurements below verify @/issue/871df4b0/requirement/REQ-01 under the
[suite measurement protocol](../../active/c780514e-test-topology/plan.md#suite-measurement-protocol-plan-fixed--how-suite-cost-is-measured).
Both runs used commit `f66a7d35da40e3209be50eff49889281f575dde5`, containing
implementation `c7bb5037d` and its removed-example ownership mappings. Both used
`CARGO_INCREMENTAL=0`, acquired `/tmp/jit-ci-turnstile.lock` before an exclusive
`/tmp/jit-full-ci.lock`, and started with 64 GB free. Start conditions were checked
after lock acquisition: at least 60 GB free and one-minute load below 4.08.

### Measurements

Captured on 2026-10-22; times are Europe/Helsinki. Load is the host's one-minute
load average sampled immediately before the command.

| Run | Start | Load | Result | Doctest wall time |
|---|---|---:|---|---:|
| Full `./scripts/cargo-ci.sh` pipeline | 23:29:37 | 2.31 | 4,591 workspace tests and 25 doctests passed; none skipped or ignored | 2.644 s |
| Warm `cargo test --doc --workspace` | 23:32:10 | 1.74 | 25 doctests passed, 0 failed, 0 ignored | 2.627 s |

The full pipeline rebuilt the suite in 35.420 s before running tests. Its measured
nextest-plus-doctest clock was 18.201 s; formatting, Clippy and build budgets passed.
The subsequent warm-only run reported `Finished test profile ... in 0.14s` with no
compilation step. Bash's elapsed timer measured the entire Cargo command, including
rustdoc compilation and execution, as `warm_doctest_seconds=2.627`; this is not just
the test harness's narrower execution time.

The warm command is below five seconds at load below ten, satisfying REQ-01.
These are observed per-substep measurements. The existing 30-second combined
suite budget does **not** enforce a five-second doctest limit; no CI limits were
added or changed by this work.

### Reproduction

From the workspace root at the measured commit, acquire the locks and check the
start conditions before the full pipeline. The pipeline leaves the target warm:

```bash
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock bash -c '
  df -BG .; uptime
  test "$(df -BG --output=avail . | tail -1 | tr -dc 0-9)" -ge 60 || exit 2
  awk "NR == 1 { exit ! (\$1 < 4.08) }" /proc/loadavg || exit 2
  CARGO_INCREMENTAL=0 ./scripts/cargo-ci.sh
'
```

Then measure only the warm doctest command under the same serialization:

```bash
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock bash -c '
  git rev-parse HEAD; df -BG .; uptime
  test "$(df -BG --output=avail . | tail -1 | tr -dc 0-9)" -ge 60 || exit 2
  awk "NR == 1 { exit ! (\$1 < 4.08) }" /proc/loadavg || exit 2
  TIMEFORMAT="warm_doctest_seconds=%3R"
  time CARGO_INCREMENTAL=0 cargo test --doc --workspace
'
```

## Cold profiled-fixture builds — 955aaa9a

The measurements below verify @/issue/955aaa9a/requirement/REQ-01 on an ext4 SATA
SSD host with 24 cores, captured on 2026-11-01 between 02:50 and 02:56 Europe/Helsinki.
Fixed code is commit `61a07d756`; pre-fix code is its parent's `test_utils.rs` and
`lock.rs` (`c03bbff17`).

### Where a cold build spent its time

A pre-fix cold `jit-dogfood` build issues 171 `fsync` calls. On a quiet host they
take 0.96 s of a roughly 1.1 s build (`strace -f -T`), and 24 concurrent consumers all
finish within 1.29 s, so lock queueing, polling and retirement are not the cost. With
7.5 GB of unsynced writes on the same filesystem, the same single-process build took
77.6 s and 13.9 s, because ext4's ordered-mode journal commit makes each `fsync` wait
on that backlog. The fix initializes over memory and writes the tree without a sync.

### Suite runs

Each run is the first `cargo nextest run --workspace --no-fail-fast` after a
crate-source change, so every fixture key is cold. Load is the one-minute load average
and dirty memory, sampled every 5 s during the test window.

| Code | Load mean / max | Dirty mean / max | Result | Nextest wall time |
|---|---:|---:|---|---:|
| Pre-fix | 17.7 / 26.0 | 5.8 / 7.3 GiB | 4,648 run: 4,638 passed, 8 failed on the fixture lock, 2 timed out | 38.3 s |
| Fixed, run 1 | 19.4 / 25.3 | 7.0 / 7.5 GiB | 4,651 passed | 26.9 s |
| Fixed, run 2 | 23.7 / 28.7 | 6.9 / 7.8 GiB | 4,651 passed | 26.2 s |
| Fixed, run 3 | 22.5 / 29.0 | 6.5 / 7.8 GiB | 4,651 passed | 25.9 s |

No fixed run has a "busy" line, a failure or a timeout. The per-test bound in
`.config/nextest.toml` is unchanged.

### Reproduction

Under `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`, from the
workspace root at the measured commit:

```bash
echo "// cold key $(date +%s)" >> crates/jit/src/lib.rs
CARGO_INCREMENTAL=0 cargo nextest run --workspace --no-run
# Three writers rewriting four 600 MiB files each, without syncing.
for w in 0 1 2; do
  mkdir -p target/tmp/load$w
  ( while :; do for f in 0 1 2 3; do
      head -c 600M /dev/urandom > target/tmp/load$w/$f
    done; done ) &
done
# The cargo wrapper on this host runs tests at nice 19; spinners compete at the same priority.
for c in $(seq 24); do nice -n 19 sh -c 'while :; do :; done' & done
CARGO_INCREMENTAL=0 cargo nextest run --workspace --no-fail-fast
kill $(jobs -p); rm -rf target/tmp/load?; git checkout -- crates/jit/src/lib.rs
```

## Build footprint and suite clock before and after — e2764021, 7721a2a9

All figures below come from exclusive build windows on 2026-11-01, entered with
`flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock` and
`CARGO_INCREMENTAL=0`, except the footprint before row. Each run started at a one-minute
load below 4.08. sccache was on: after runs used the `scripts/cargo-ci.sh` default, and
before runs set `RUSTC_WRAPPER=sccache`, the value that default selects at that commit.
After is commit `de34a0590`; before is commit `0896fc29b`, measured in a seeded
worktree with isolated build state.

### Build footprint

The budget checker passes on the after tree. Before is gate run `8e22dd96` at
commit `0896fc29b`, command `./scripts/cargo-ci.sh`. After is
`./scripts/rust-build-budget.sh` at load 3.11.

| | Integration targets | Active test executables | Active test-executable bytes | Budget |
|---|---:|---:|---:|---:|
| Before | 10/12 | 14 | 1,723,670,208 | 2,147,483,648 |
| After | 10/12 | 14 | 1,694,388,096 | 2,147,483,648 |

### Suite clock

After runs are `./scripts/cargo-ci.sh`, with `touch crates/jit/src/lib.rs` before each
rebuild run. The suite clock excludes the suite build, which is shown separately.

| After | Start load | Suite build | Test | Doctest | Suite clock |
|---|---:|---:|---:|---:|---:|
| Warm, first of the window | 3.31 | 0.6 s | 12.8 s | 2.9 s | 15.8 s |
| Warm 1 | 3.98 | 0.6 s | 21.1 s | 2.8 s | 24.0 s |
| Warm 2 | 3.09 | 0.5 s | 12.0 s | 2.8 s | 14.7 s |
| Warm 3 | 3.11 | 0.5 s | 11.9 s | 2.7 s | 14.7 s |
| Rebuild 1 | 2.55 | 37.8 s | 12.0 s | 2.7 s | 14.7 s |
| Rebuild 2 | 2.78 | 24.8 s | 12.4 s | 4.9 s | 17.3 s |

`./scripts/cargo-ci.sh` cannot give a clean before figure on today's host:
`test_cargo_ci_selftest_fails_seeded_broken_merges` hangs at that commit through the cargo
shim loop c882f1f8 fixed, hits the 20 s limit, and takes the suite clock to about 35.7 s.
So before runs are `cargo nextest run --workspace -E 'not test(test_cargo_ci_selftest_fails_seeded_broken_merges)'`
followed by `cargo test --doc --workspace`, with `touch crates/jit/src/lib.rs` and
`cargo nextest run --workspace --no-run` before each rebuild run. That excludes one test of
4,674; it took 3.99 s in the earlier gate run below, and ran in parallel with the rest.

| Before | Start load | Suite build | Test | Doctest | Test plus doctest |
|---|---:|---:|---:|---:|---:|
| Warm 1 | 2.93 | — | 16.9 s | 4.2 s | 21.1 s |
| Warm 2 | 3.96 | — | 16.9 s | 4.2 s | 21.1 s |
| Warm 3 | 2.63 | — | 16.9 s | 4.2 s | 21.1 s |
| Rebuild 1 | 3.38 | 24.8 s | 16.9 s | 4.3 s | 21.2 s |
| Rebuild 2 | 2.62 | 24.9 s | 16.8 s | 4.2 s | 21.1 s |

Those agree with the figures this work started from, which carry no recorded load: gate run
`8e22dd96` at `0896fc29b` (`./scripts/cargo-ci.sh`) reported 21.1 s warm, and earlier
runs 24-29.6 s after a rebuild.

Warm 1 ran while the host load climbed to 11.34 by its end, and is the only after run
above the before figure. The other after runs are 14.7-17.3 s against 21.1 s. The suite went
from 4,674 tests and 61 doctests to 4,632 tests and 25 doctests.

### Per-test profile

After: `PROFILE_TEST_SUITE_OUTPUT=<path> ./scripts/profile-test-suite.sh` at load 3.95, warm,
recorded in [`suite-profile.json`](../suite-profile.json): 4,632 tests, none over 5 s. The
slowest is `commands::document::tests::test_add_document_reference_succeeds_over_an_event_log_past_the_former_ceiling`
at 3.62 s. The doctest substep ran 2.7-2.9 s warm and at most 4.9 s across the runs above.

Before: the filtered nextest command with `--message-format libtest-json-plus` at load 3.51,
warm: 4,673 tests, the slowest being `server_integration`
`test_jit_server_shutdown_force_closes_a_stalled_connection_and_exits_zero` at 5.32 s,
with the doctest substep at 4.2-4.3 s.
