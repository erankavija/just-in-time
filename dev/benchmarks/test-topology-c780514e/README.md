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
