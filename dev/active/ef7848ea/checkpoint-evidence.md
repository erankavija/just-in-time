# Checkpoint ef7848ea — mutation cost evidence

Story checkpoint of epic f1dc843a; children 722878d4, 8b6a9e34, dd1b984c. Artifact:
[publication-cost-722878d4](../../benchmarks/publication-cost-722878d4/README.md) (`A` below),
raw outputs under its `raw/` (`R/`). Commands run from the repository root at `7619ae061`
unless stated.

## REQ-01 — every artifact figure traces to the harness

The harness is [`scripts/benchmark-mutation-cost.sh`](../../../scripts/benchmark-mutation-cost.sh),
moved from `dev/active/f1dc843a-redo-mutations/bench.sh` by `0e20d3f02`. The move changes
only where fixtures are built (`BENCH_WORK_ROOT`, default `/dev/shm/jit-benchmark-mutation-cost`,
replacing "beside the script") and adds `BENCH_GF2_REPO`/`BENCH_JIT_REPO` overrides with the
old defaults; scenarios, runs, and stdout format are unchanged. The recorded runs used
`/dev/shm/bench-722878d4` as the script directory, so `BENCH_WORK_ROOT=/dev/shm/bench-722878d4`
reproduces the recorded `work-root` line.

```console
$ git diff -M --stat 0d9297f0e 0e20d3f02 -- scripts dev/active/f1dc843a-redo-mutations/bench.sh
 .../bench.sh => scripts/benchmark-mutation-cost.sh | 28 +++++++++++++---------
```

| `A` figure | source line |
|---|---|
| base `34d37f8be…`, sha256 `ca035666…` | `R/base.txt` `jit-sha256`; base commit is the build input |
| tip `c0b1b4e2e…`, sha256 `65892a8f…` | `R/tip.txt` `jit-sha256`, `jit-head c0b1b4e2e679…` |
| gf2 at `6bf634098a7f`, jit at `c0b1b4e2e679` | `R/*.txt` `gf2-head`, `jit-head` |
| Ryzen 9 5900X, 24 cores, 7.2.6-arch2-1, tmpfs | `R/*.txt` `machine` block |
| load below 2.0, three runs, median | harness `BENCH_MAX_LOAD` default, `scenario()` loop, `median()` |
| loadavg `0.87 6.70 24.21` / `0.88 6.60 24.08` | `R/base.txt` `loadavg-start` / `loadavg-end` |
| loadavg `0.90 6.42 23.84` (both) | `R/tip.txt` `loadavg-start` / `loadavg-end` |
| issues 1002 / 1136 / 1136 | `R/*.txt` `fixture sizes`, `issues` column |
| log bytes 3,300,903 / 3,627,204 / 36,272,154, 10.00x | `R/*.txt` `fixture sizes`, `fixture-log`, `multiple` |
| leases: strict appended / source section | `R/*.txt` `fixture sizes`, `leases` column |
| changing 118 / 49, 122 / 49, 441 / 50 | `R/base.txt` / `R/tip.txt` `changing-label` rows |
| idempotent 73 / 45, 75 / 48, 134 / 49 | `idempotent-label` rows |
| lease refused 73 / 43, 73 / 46, 131 / 47 | `lease-refused` rows |
| `1.02x` each scenario after; `3.61x` before | `ratio` column of each file's median table |
| +2% (50 against 49 ms) | 50 / 49 = 1.020 from the tip `changing-label` rows |
| claim exit 1, `36274974` vs `16777216` | `R/base.txt` `jit-10x claim-acquire exit 1` |
| lease-refused exits 1, others exit 0 | `exits` fields; `R/*-stderr.log` hold 9 `No active lease` lines each (3 fixtures × 3 runs) |
| Rust balance figures | REQ-04 below |

No figure required a re-run.

## REQ-02 — evidence citations resolve to shipped harnesses

```console
$ rg -n "dev/benchmarks/|benchmark" dev/TESTING.md   # citations, by target
```

| cited artifact (`dev/TESTING.md` line) | exists | producing harness |
|---|---|---|
| `benchmarks/rust-build-budgets/README.md` (273, 323) | yes | `scripts/rust-build-budget.sh`, `scripts/benchmark-rust-build.sh` |
| `benchmarks/suite-enforcement-4b7c06d0/README.md` (274) | yes | `scripts/cargo-ci.sh`, `scripts/rust-build-budget.sh` |
| `benchmarks/cold-warm-verdict-0708d692/README.md` (285) | yes | `scripts/cargo-ci.sh`, `scripts/setup-recorded-failure-corpus.sh` |
| `benchmarks/rust-build-efficiency/report.md` (294, 354) | yes | `scripts/benchmark-rust-build.sh` |
| `benchmarks/dependency-profile-6d10e5d4/` (314) | yes | `scripts/benchmark-rust-build.sh` |
| `benchmarks/suite-profile.json`, contract `suite-timing-evidence` (380) | yes | `scripts/profile-test-suite.sh` (`DEFAULT_OUTPUT`) |
| `benchmarks/publication-cost-722878d4/README.md` (460) | yes | `scripts/benchmark-mutation-cost.sh` (459), `rust-balance.py` beside it |

Contract and policy files naming evidence: `scripts/profile-test-suite.sh` (writes
`suite-profile.json`), `scripts/rust-build-budget.sh:39` and `scripts/cargo-ci.sh:156`
(`rust-build-budgets/README.md`), `scripts/cargo-ci.sh:380,386`
(`dev/archive/4b7c06d0-test-suite-performance/benchmarks/provenance-removal-355565c2/README.md`, `dev/benchmarks/cold-warm-verdict-0708d692/README.md`); both exist.
`scripts/docs-*.toml` and `.config/nextest.toml` name no evidence artifact. The retired
`transaction-benchmark-evidence` contract and `scripts/benchmark-transaction.sh` appear only in
`dev/active/` planning records. `rust-build-efficiency/report.md:36` mentions the removed
`scripts/verify-commit-builds.sh` as a lock holder at measurement time; it is not the
artifact's harness.

```console
$ rg -n "bench\.sh" dev docs scripts crates -g '!dev/archive/**' -g '!dev/active/**'
dev/benchmarks/publication-cost-722878d4/README.md:11:  The recorded runs used it at commit `b85bdf018`, as `bench.sh` run from a copy in
$ scripts/docs-check-links.sh dev/TESTING.md dev/benchmarks/publication-cost-722878d4/README.md \
    dev/active/f1dc843a-redo-mutations/intake.md dev/active/f1dc843a-redo-mutations/plan.md
OK: all links and anchors resolve
```

## REQ-03 — no throughput assertion, nothing held out

```console
$ rg -n "#\[ignore" crates; echo $?
1
$ rg -n "default-filter|ignored" .config/nextest.toml; echo $?
1
```

`.config/nextest.toml` sets setup scripts, `retries = 0`, a 10 s × 2 slow-timeout, and a 200 ms
leak-timeout; it holds no test out of the default run.

`rg -n "elapsed|Instant::now|Duration::from_(milli)?secs|as_millis|wall" crates --type rust`
returned 176 lines. Assertions on elapsed time, classified:

| site | property | class |
|---|---|---|
| `validation/graph.rs` `test_label_uniqueness_large_fixture_correctness` | 400-issue evaluation under 5 s | **throughput; removed in `1f8d2bff7`** |
| `gate_execution.rs:786` | timed-out checker returns before its 30/60 s grandchild | timeout guard |
| `lock_contention_tests.rs:115` | lock wait honours `JIT_LOCK_TIMEOUT` | timeout guard |
| `template_apply_atomicity_tests.rs:610` | writer blocked at least half the stall (lower bound) | lock exclusion |
| `serve_cli_tests.rs:796` | `serve --fg` serves within `FG_BUDGET` (90 s) | liveness budget |
| `graceful_shutdown_tests.rs:133,235,353,375` | startup, exchange, exit within budget | liveness budget |

Every other hit is a production timeout or clock, a lock timeout argument, a
`recv_timeout`, or prose. `1f8d2bff7` also drops the comment in
`example_rulesets_tests.rs` that pointed at the removed bound.

## REQ-04 — Rust line balance

```console
$ git diff --shortstat 34d37f8be -- '*.rs'
 118 files changed, 12053 insertions(+), 14279 deletions(-)
```

Net −2,226, measured at `1f8d2bff7`, the last Rust change; later commits touch no Rust. Since
722878d4 recorded −2,507 at `b85bdf018`, `dd1b984c` added +308 net and `1f8d2bff7` removed 27.
`R/rust-balance.txt`, regenerated by the artifact's `rust-balance.py` with its stated
exclusions, records shortstat net −2,226 and redo-record remainder −2,026; `A` § "Rust line
balance from `34d37f8be`" states both.
