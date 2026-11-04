# Per-test durations and the per-test limits derived from them (jit:1eb949a5)

Nothing bounds an individual test except the 20 s hard stop in `.config/nextest.toml`
(`slow-timeout = { period = "10s", terminate-after = 2 }`). This note measures every
test's duration and derives one limit per layer from the measured distribution.

Raw files are under `raw/`; `distribution.py` computes every figure below.

## Environment

Measured tree: `ba6ed02e3` (main at dispatch).
2026-11-03, 24 CPUs, `sccache` on PATH. Nextest
0.9.133 (the pinned version), 4821 tests per run, 0 failed, 0 retried.

Three warm measured runs after one unmeasured `cargo nextest run --workspace --no-run`.
Each run is `./scripts/cargo-ci.sh --cargo nextest run --workspace --tool-config-file
measure:<scratch>/measure.toml`, which takes the gate's host lock. The scratch config
holds only `[profile.default.junit] path = "junit.xml"`; `.config/nextest.toml` is
unchanged. Source of every duration: the `time` attribute of each `<testcase>` in
nextest's JUnit report (`raw/junit-N.xml`).

| run | start (UTC) | load average at start (1 / 5 / 15 min) | nextest wall time |
| --- | --- | --- | --- |
| 1 | 23:15:04 | 3.77 / 5.34 / 4.68 | 14.7 s |
| 2 | 23:16:39 | 3.74 / 5.23 / 4.72 | 14.3 s |
| 3 | 23:18:44 | 3.74 / 5.18 / 4.80 | 14.7 s |

Quiet-host rule: each run waited for a 1-minute load below 4 (the steward's bound in
`scripts/cargo-ci.sh`) before starting. Samples are in `raw/load-N.txt`, outputs in
`raw/run-N.out`. A first attempt started runs back to back with 1-minute loads of
3.60, 8.18, 11.20 and 13.90 (each run's own load); it was discarded and is not in
`raw/`.

Durations are wall-clock per test inside a 24-way parallel run, so each includes
contention with its neighbours. A limit derived from them holds under the same
parallelism the gate uses.

## Layer mapping

The layer is the JUnit `<testsuite name>`, which is the nextest binary id.

| layer | binary ids | tests |
| --- | --- | --- |
| unit | `jit` (lib), `jit-server` (lib), `jit::bin/jit`, `jit-server::bin/jit-server` | 3157 |
| integration | `jit::cli_gate`, `cli_issue`, `cli_item_validate`, `cli_query_graph`, `cli_repo_workflow`, `fast_docs_templates`, `fast_issue`, `fast_rules`, `scratch_build`, `jit-server::server_integration` (the `crates/*/tests/*` targets) | 1664 |

The two `bin/` targets hold inline `#[cfg(test)]` unit tests of `main.rs`, so they join
the unit layer. Harness tests over `InMemoryStorage` live inside the lib and are unit
tests by this mapping; the integration layer is every test that lives in a
`crates/*/tests/*` target, which includes the CLI tests that spawn the binary.

## Distribution

A test's duration is its median over the three runs ("median-of-runs"); "worst-run" is
its slowest of the three. Seconds.

| layer | basis | n | median | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- | --- |
| unit | median-of-runs | 3157 | 0.015 | 0.070 | 0.335 | 3.608 |
| unit | worst-run | 3157 | 0.018 | 0.082 | 0.340 | 3.760 |
| integration | median-of-runs | 1664 | 0.055 | 0.231 | 1.044 | 6.647 |
| integration | worst-run | 1664 | 0.065 | 0.266 | 1.079 | 7.058 |
| all | median-of-runs | 4821 | 0.019 | 0.164 | 0.546 | 6.647 |

Full output with the 40 slowest tests: `raw/distribution.out`. The tail is continuous
rather than gapped: the unit layer has 10 tests at or above 1 s and 2 at or above 2 s;
the integration layer has 20 at or above 1 s and 6 at or above 2 s. There is no gap
wide enough to cut at, so each limit is a multiple of p99.

## Proposed limits

Rule: limit = 5 x the layer's worst-run p99, rounded up to a whole second. The worst
run is the basis because a limit must hold in every run, not in the median one. The
factor 5 leaves room for a test that is 5 times slower than the layer's 99th percentile
before it is flagged, which absorbs the run-to-run spread seen here (worst-run exceeds
median-of-runs by at most 3% at each layer's p99).

| layer | worst-run p99 | 5 x p99 | proposed limit | headroom under the 20 s hard stop |
| --- | --- | --- | --- | --- |
| unit | 0.340 s | 1.70 s | 2 s | 10x |
| integration | 1.079 s | 5.40 s | 6 s | 3.3x |

A 2 s unit limit is 133 times the unit median and 6 times its p99, so a unit test
cannot hide in the suite total while 3155 of 3157 stay inside it. The integration
limit of 6 s is 5.6 times its p99 and keeps 1663 of 1664 inside. Each limit flags a
short list that fits one refactoring pass (3 tests) and is far below the 20 s stop.

### Tests above the proposed limits

Duration is median-of-runs, with worst-run in parentheses.

| layer | limit | test | duration |
| --- | --- | --- | --- |
| unit | 2 s | `jit` `commands::document::tests::test_add_document_reference_succeeds_over_an_event_log_past_the_former_ceiling` | 3.608 s (3.760) |
| unit | 2 s | `jit` `commands::serve::tests::test_start_server_bounds_a_child_that_neither_serves_nor_exits` | 2.017 s (2.019) |
| integration | 6 s | `jit::cli_repo_workflow` `lease_enforcement_tests::test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses` | 6.647 s (7.058) |

The `serve` test sits at 2.017 s in every run because it waits out a fixed 2 s bound;
it exceeds the unit limit by construction, and is a refactoring candidate (inject a
shorter bound) rather than a limit-tuning one. The next-slowest tests below the limits
are `test_strict_lease_exempt_paths_write_for_their_stated_reason` (2.848 s,
integration), `test_profile_add_publishes_a_package_whose_directories_outnumber_its_files`
(2.598 s), and `storage::claim_coordinator::proptests::test_sequence_numbers_monotonic`
(1.654 s, unit); the complete tail is in `raw/distribution.out`.

## Doctests

Doctests are not nextest tests and are outside the per-test limits. `cargo test --doc
-p jit` (through `./scripts/cargo-ci.sh --cargo`): 25 passed, tests finished in 1.40 s,
64.5 s wall including the doctest compile. Load at start was 6.38 / 7.34 / 6.12 (another
lead's gate was running), so the wall figure is not a quiet-host sample; the 1.40 s
test time is the figure to compare. Output: `raw/doctest.out`, `raw/load-doctest.txt`.

## Reproducing

```sh
printf '[profile.default.junit]\npath = "junit.xml"\n' > /abs/path/measure.toml
./scripts/cargo-ci.sh --cargo nextest run --workspace --tool-config-file measure:/abs/path/measure.toml
cp target/nextest/default/junit.xml raw/junit-N.xml
python3 -I dev/benchmarks/per-test-durations-1eb949a5/distribution.py raw/junit-*.xml
```
