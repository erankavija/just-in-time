# Rust build efficiency: baseline and optimized benchmark results

Benchmark evidence for the `rust-build-efficiency` story (jit:73482aa1): a
pre-optimization baseline (jit:4e22a20d, before any build-topology, profile,
or dependency change) and a post-optimization comparison run (jit:26f97dc2,
after the story's landed optimization work) collected with the identical
committed harness and acceptance protocol, so the improvement claims rest on
matched-methodology measurements rather than a single anecdotal run. See
"What changed" below for the work the optimized run measures, and
"Comparison and acceptance thresholds" for the accept/reject verdict.

## Protocol

Harness: [`scripts/benchmark-rust-build.sh`](../../../scripts/benchmark-rust-build.sh).

- **Clean sample** (3 collected): a freshly created, isolated
  `CARGO_TARGET_DIR` runs `cargo clippy --workspace --all-targets -- -D
  warnings` followed by `cargo test --workspace --no-run`, each timed
  separately. The target directory is removed immediately after its
  measurements are recorded, before the next sample starts.
- **Rebuild sample** (3 collected): the same clippy + `test --no-run` sequence
  runs first as untimed-for-the-headline-metric setup, to reach "an
  equivalent completed clean build" in a fresh isolated target directory. One
  fixed, reversible, comment-only probe line
  (`// jit-benchmark-rebuild-probe: reversible comment-only source touch
  (jit:4e22a20d)`) is then appended to `crates/jit/src/lib.rs`, a second
  `cargo test --workspace --no-run` is timed (the rebuild measurement), and
  the file is restored byte-for-byte: captured via `cp` before the probe,
  restored via `mv` after, verified via `cmp` against `git show HEAD:...`
  rather than against the harness's own backup, so the check catches
  corruption in the restore mechanism itself, not just a tautological match.
  All three rebuild samples verified `probe_restored_verified: true`, and the
  branch shows zero diff against HEAD outside `dev/benchmarks/` after the run.
- **Non-overlap**: both sample kinds acquire a blocking `flock` on the same
  `CARGO_CI_BUILD_LOCK` path used by `scripts/cargo-ci.sh` and
  `scripts/verify-commit-builds.sh` before their cargo invocations, so a
  benchmark sample and a concurrent gate build never run at the same time.
  This run observed zero lock-wait events (`grep -c "waiting for exclusive
  build lock"` on the run log returns 0): no concurrent gate build competed
  for the host during sampling.
- **Test inventory**: derived once, from clean sample 1, via `cargo test
  --workspace --no-run --message-format=json` (a nearly free re-check since
  nothing changed after the timed build) to enumerate every compiled test
  binary and its Cargo target kind, then `<binary> --list` and `<binary>
  --list --ignored` per binary to enumerate test cases and classify ignored
  status. Doctests cannot go through `--no-run` (cargo does not support
  compiling them without running), so they are enumerated separately via
  `cargo test --workspace --doc -- --list` / `-- --list --ignored` and folded
  into the same inventory under a synthetic `"doctest"` kind. See "Baseline
  footprint" and "Optimized footprint" below for the full accounting.
- **Measurement**: wall time via `time.monotonic()` around each command;
  maximum resident set size via `getrusage(RUSAGE_CHILDREN)` immediately
  after the command exits — the same mechanism GNU `time -v` uses internally,
  with the same caveat: for a multi-process build tree (cargo fans out many
  rustc invocations) this is the high-water mark of the single largest reaped
  process in the tree, not a sum across concurrently running rustc processes.

## Environment

Both runs executed on the same host with the identical toolchain, CPU,
memory, and linker; only the Git revision differs, and only by the story's
own landed optimization commits (see "What changed").

| | Baseline | Optimized |
|---|---|---|
| Git revision | `56d737891129babf9571eb4de5659295bce8d827` | `e305f9655f565c6c16937138ed4ea1035c859bc3` |
| OS | Arch Linux, kernel Linux 7.0.14-arch1-1 | Arch Linux, kernel Linux 7.0.14-arch1-1 |
| CPU count | 24 | 24 |
| Memory | 33,543,761,920 bytes (~31.2 GiB) | 33,543,761,920 bytes (~31.2 GiB) |
| Cargo | cargo 1.97.0 (c980f4866 2026-06-30) | cargo 1.97.0 (c980f4866 2026-06-30) |
| Rustc | rustc 1.97.0 (2d8144b78 2026-07-07) | rustc 1.97.0 (2d8144b78 2026-07-07) |
| Target triple | x86_64-unknown-linux-gnu | x86_64-unknown-linux-gnu |
| Linker | cargo default (no `.cargo/config.toml` linker override present): cc (GCC) 16.1.1 20260625 invoking GNU ld (GNU Binutils) 2.46.1 | identical |
| Relevant `CARGO_*`/`RUST*` env vars | none set | none set |

Full environment blocks, including the exact `cargo_rust_env` capture, are in
[`baseline.json`](baseline.json)'s and [`optimized.json`](optimized.json)'s
`environment` objects — this table is a projection of them, not a second
source of truth.

## Baseline medians and variance

All wall times in seconds. Raw per-sample data:
[`baseline.json`](baseline.json) (`clean_samples`, `rebuild_samples`); command
logs under [`raw/`](raw/) (`raw/clean-<n>/`, `raw/rebuild-<n>/`).

| Metric | Sample 1 | Sample 2 | Sample 3 | Median | Min-Max spread |
|---|---|---|---|---|---|
| Clean: clippy | 26.227 | 27.050 | 26.885 | **26.885** | 26.227-27.050 (3.1%) |
| Clean: `test --no-run` | 208.317 | 195.019 | 185.582 | **195.019** | 185.582-208.317 (11.6%) |
| Clean: total (clippy + test-compile) | 234.544 | 222.069 | 212.467 | **222.069** | 212.467-234.544 (9.9%) |
| Rebuild: setup clippy | 27.124 | 27.047 | 26.760 | 27.047 | 26.760-27.124 (1.4%) |
| Rebuild: setup `test --no-run` | 188.894 | 187.053 | 191.270 | 188.894 | 187.053-191.270 (2.3%) |
| Rebuild: incremental `test --no-run` (headline) | 178.865 | 173.428 | 188.408 | **178.865** | 173.428-188.408 (8.4%) |

No single run is presented as representative on its own; use the median
column as the baseline for the story's REQ-06 improvement targets (>=40%
below the clean median, >=60% below the rebuild median), and the full
per-sample data in `baseline.json` for any deeper comparison.

**Observed variance / host noise:**

- Clean `test --no-run` wall time trended down monotonically across the three
  samples (208.3s -> 195.0s -> 185.6s), a ~12% spread. The most likely cause
  is not build-topology variance: `~/.cargo`'s registry index and crate
  source cache sit outside the isolated `CARGO_TARGET_DIR` and are shared
  across samples, so only sample 1 pays any residual registry/source
  cold-cache cost. The rebuild samples (which all start from their own fresh
  setup build, same shared registry cache) show no such monotonic trend
  (188.9s -> 187.1s -> 191.3s for the setup step), consistent with this
  explanation rather than host contention.
- Maximum RSS was stable across samples for every step (clippy ~1.098-1.100
  GiB, `test --no-run` ~1.513-1.515 GiB, rebuild ~1.245-1.250 GiB; <0.2%
  spread in every case), suggesting one specific dependency's build is
  consistently the single largest process in the tree.
- Zero lock-wait events were recorded: no concurrent `cargo-ci` or
  `verify-commit-builds` gate build overlapped this run's sampling window
  (22:56-23:31 local time on 2026-07-14).
- Host swap was fully utilized (4.0 GiB/4.0 GiB) from unrelated prior activity
  at the time sampling started, though load average was low (0.34) and no
  OOM events are visible in `journalctl -k` for the sampling window; no
  sample failed or showed anomalous RSS as a result.
- **Root cause of the original assembly failure (fixed):** all 6 samples
  completed and were durably recorded in `raw/clean-samples.jsonl` /
  `raw/rebuild-samples.jsonl`, but the harness's environment-capture step
  computed `CARGO_RUST_ENV` with `env | grep -E '^(CARGO|RUST)[A-Z_]*=' | jq
  ...`. With zero `CARGO_*`/`RUST*` variables set (this run's actual
  environment), `grep` exits 1 on a no-match search; under `set -o pipefail`
  that nonzero status becomes the whole pipeline's exit status even though
  `jq` downstream succeeds, and `set -e` then terminates the script at that
  line with no error output. The pipeline now treats an empty match set as
  success (`grep ... || true`, alongside jq's existing `// {}` fallback for
  empty input), so assembly completes in this case.
  [`scripts/benchmark-rust-build.sh`](../../../scripts/benchmark-rust-build.sh)
  documents and handles this explicitly. Reproducibility was verified
  directly: running the script with `BENCH_SKIP_SAMPLING=1` (assemble only,
  no new sampling) and `BENCH_GIT_REVISION_OVERRIDE` set to the sampled
  commit, in a shell with no `CARGO_*`/`RUST*` variables set, against the
  already-recorded `raw/*-samples.jsonl`, completes successfully and
  reproduces this `baseline.json` byte-for-byte apart from `generated_at`.
  `baseline.json`'s `clean_samples`/`rebuild_samples` are a direct slurp of
  `raw/{clean,rebuild}-samples.jsonl` (`jq -s '.' ...`), so this is also
  verified end to end: the test-target and doctest corrections below are
  applied to the raw record itself, not layered onto `baseline.json`
  separately, and reassembly was re-run after each correction to confirm the
  two stay in lockstep.

## Optimized medians and variance

All wall times in seconds. Raw per-sample data:
[`optimized.json`](optimized.json) (`clean_samples`, `rebuild_samples`);
command logs under [`raw/`](raw/) (`raw/optimized-clean-<n>/`,
`raw/optimized-rebuild-<n>/`). Same harness, same protocol, same host, same
`crates/jit/src/lib.rs` probe as the baseline run above; only the Git
revision differs.

| Metric | Sample 1 | Sample 2 | Sample 3 | Median | Min-Max spread |
|---|---|---|---|---|---|
| Clean: clippy | 20.248 | 20.312 | 20.421 | **20.312** | 20.248-20.421 (0.9%) |
| Clean: `test --no-run` | 22.127 | 22.274 | 22.464 | **22.274** | 22.127-22.464 (1.5%) |
| Clean: total (clippy + test-compile) | 42.375 | 42.586 | 42.885 | **42.586** | 42.375-42.885 (1.2%) |
| Rebuild: setup clippy | 20.177 | 20.474 | 20.316 | 20.316 | 20.177-20.474 (1.5%) |
| Rebuild: setup `test --no-run` | 22.353 | 22.741 | 22.281 | 22.353 | 22.281-22.741 (2.1%) |
| Rebuild: incremental `test --no-run` (headline) | 8.514 | 7.056 | 7.580 | **7.580** | 7.056-8.514 (20.7%) |

As with the baseline, no single run is representative on its own; the median
column is what REQ-04/REQ-05 acceptance is computed against (see
"Comparison and acceptance thresholds" below).

**Observed variance / host noise:**

- Clean-sample and rebuild-setup steps show the same low single-digit percent
  spread as the baseline (0.9-2.1%), consistent with a quiet, uncontended
  sampling window: `grep -c "waiting for exclusive build lock"` against this
  run's harness log returns 0 — no concurrent `cargo-ci`/`verify-commit-builds`
  gate build overlapped any of the six timed samples.
- The rebuild headline step shows a larger proportional spread (20.7%,
  7.056-8.514s) than the baseline's rebuild spread (8.4%), but the *absolute*
  spread is comparable (~1.5s here vs. baseline's ~15s) — at a ~7-8s median
  instead of baseline's ~179s, the same order of absolute scheduling/filesystem
  jitter that was a rounding error at baseline scale is now a visible
  percentage. This is consistent with jitter, not a regression: max-RSS for
  this step is stable across all three samples (1,004,200-1,007,012 KB, <0.3%
  spread), and the setup steps immediately before it (same target dir, same
  build) show ordinary low spread.
- Maximum RSS was stable across samples for every step and is uniformly lower
  than baseline: clippy ~1.022-1.024 GiB (baseline ~1.098-1.100 GiB), `test
  --no-run` ~1.403-1.423 GiB (baseline ~1.513-1.515 GiB), rebuild
  ~0.958-0.960 GiB (baseline ~1.245-1.250 GiB) — the largest relative RSS drop
  is on the incremental-rebuild path, consistent with the removed dependency
  subtree (`aws-lc-rs`/`aws-lc-sys`, `reqwest`, `native-tls`, `tokio-rustls`,
  `hyper-rustls`, and related crates; see "What changed") no longer being
  linked into that step.
- This run executed 2026-07-15, one day after the baseline's 2026-07-14
  sampling window; both windows were independently confirmed lock-contention
  free via the same `grep -c` check against each run's own harness log.

## Baseline footprint (from the test inventory, same clean sample)

| | |
|---|---|
| Complete target-directory bytes | 21,089,820,236 (~19.6 GiB) |
| Unique active test-executable bytes | 14,816,926,968 (~13.8 GiB) |
| Integration-test targets (Cargo `target.kind == ["test"]`) | 144 |
| Total test targets (lib + bin + integration) | 148 |
| Doctest-owning crates (kind: `"doctest"`) | 2 (`jit`, `jit_server`) |
| Total discoverable test cases (incl. doctests) | 3,306 |
| — of which doctests | 66 |
| Ignored test cases | 14 |

**Test-target filtering correction:** targets are derived from Cargo
`compiler-artifact` messages filtered to `profile.test == true`. An earlier
pass filtered only on `reason == "compiler-artifact"` and a non-null
`executable`, which also matched two ordinary (non-test) `[[bin]]` build
outputs that Cargo emits alongside their test-harness builds: the plain `jit`
and `jit-server` executables (`target.kind == ["bin"]`, `profile.test ==
false`). Both contributed zero test cases (`jit --list` and `jit-server
--list` simply fail as unrecognized CLI invocations, `list_exit_code: 2`),
so `test_case_count` and `ignored_test_case_count` are unaffected; only
`test_target_count` (150 → 148) and `unique_active_test_executable_bytes`
needed correction. The corrected byte total is a MEASUREMENT, not an
estimate: the original isolated sample's target directory was removed
immediately after the sample per the harness's own isolation design (only
the aggregate sum was persisted), so the metric was re-measured from a
fresh, fully isolated build of the identical commit — `git archive
56d73789` extracted to a scratch directory, `cargo test --workspace
--no-run --message-format=json` with the same toolchain into a fresh
`CARGO_TARGET_DIR`, under the shared build lock. The rebuilt artifact set
matches the recorded inventory exactly (148 `profile.test` executables;
the two excluded non-test bins measured `jit` 259,803,096 bytes and
`jit-server` 237,935,048 bytes), summing to 14,816,926,968 bytes. The
per-executable measurements are committed as raw evidence at
[`raw/clean-1/executable-remeasure.json`](raw/clean-1/executable-remeasure.json)
(148 entries plus the two excluded non-test binaries, with method
metadata), and the provenance is recorded alongside the sample itself in
`raw/clean-samples.jsonl` (sample 1, `correction_note`); timing and RSS
fields everywhere remain the original in-sample measurements.

**Doctests are included in the inventory**, not excluded: `cargo test
--no-run` cannot compile doctest binaries ("can't skip running doc tests with
--no-run"), so they are enumerated separately via `cargo test --workspace
--doc -- --list` and `-- --list --ignored` and appended to `targets` with a
synthetic `kind: ["doctest"]` (one entry per crate, matching Cargo's own
"Doc-tests `<crate>`" grouping — not a real Cargo `target.kind` value). They
count toward `test_case_count` and `ignored_test_case_count` (`totals.
doctest_case_count` gives the subset) but not toward `test_target_count`,
`integration_test_target_count`, or `unique_active_test_executable_bytes`:
Cargo compiles each doctest as an ephemeral per-case binary with no stable
path to size, unlike the lib/bin/integration-test binaries that persist under
`target/debug/deps` for the run's duration.

Cross-check against `cargo-ci.sh`'s full `cargo test --workspace` run: 3,306
inventoried cases (3,240 regular + 66 doctests) minus 14 ignored = 3,292
expected passes, matching `cargo-ci.sh`'s reported `3292 passed, 0 failed, 14
ignored` exactly. The inventory is complete: every case `cargo-ci.sh` counts
is accounted for in `pre-change-test-inventory.json`.

Full per-target, per-test breakdown (name, kind, executable path, every test
case with its ignored status) is in
[`pre-change-test-inventory.json`](pre-change-test-inventory.json).

## Optimized footprint (from the test inventory, same clean sample)

Derived the same way as the baseline footprint above, from optimized clean
sample 1 (`raw/optimized-clean-1/`), and published in full at
[`post-change-test-inventory.json`](post-change-test-inventory.json) (same
schema as `pre-change-test-inventory.json`).

| | Baseline | Optimized |
|---|---|---|
| Complete target-directory bytes | 21,089,820,236 (~19.6 GiB) | 3,822,359,533 (~3.56 GiB) |
| Unique active test-executable bytes | 14,816,926,968 (~13.8 GiB) | 978,319,560 (~0.91 GiB) |
| Integration-test targets (Cargo `target.kind == ["test"]`) | 144 | 11 |
| Total discoverable test cases (incl. doctests) | 3,306 | 3,330 |
| — of which doctests | 66 | 66 |
| Ignored test cases | 14 | 14 |

The 24-case increase in total test cases (3,306 -> 3,330, doctests and
ignored-count both unchanged) comes from new tests the optimization work
itself added — dependency-feature, build-profile, and budget-checker
regression fixtures — not from any change to pre-existing tests; see "Test
inventory verification" below for the full accounting of every pre-change
case.

Both target-directory bytes and unique active test-executable bytes are
reproducible across all three optimized clean samples to within rounding
(3,822,359,533 / 3,822,360,395 / 3,822,359,893 bytes; the inventory step
itself only runs on sample 1 per the harness's `BENCH_INVENTORY_SAMPLE`
default, so the executable-byte figure is sample 1's alone, while
target-directory bytes are recorded for every sample — see
[`optimized.json`](optimized.json)'s `clean_samples`).

## JSON shapes

`baseline.json`: `{schema_version, generated_at, environment, measurement_methodology,
clean_samples: [{sample, clippy: {wall_seconds, max_rss_kb, exit_code},
test_no_run: {...}, target_dir_bytes, inventory, success}], rebuild_samples:
[{sample, setup_clippy, setup_test_no_run, rebuild_test_no_run,
target_dir_bytes, probe_restored_verified, success}], medians}`.

`pre-change-test-inventory.json`: `{schema_version, generated_at_git_revision,
notes, targets: [{name, kind, executable, tests: [{name, ignored}],
test_count, ignored_count, list_exit_code, list_ignored_exit_code}], totals:
{test_target_count, integration_test_target_count, doctest_target_count,
test_case_count, doctest_case_count, ignored_test_case_count},
target_dir_bytes, unique_active_test_executable_bytes}`. `kind` is `["lib"]`,
`["bin"]`, or `["test"]` (Cargo's own `target.kind`) for compiled test
binaries, or the synthetic `["doctest"]` (this harness's own label, one entry
per crate) for doctests, whose `executable` is `null`.

`optimized.json` and `post-change-test-inventory.json` follow the identical
schemas above: the harness itself always writes `baseline.json` and
`pre-change-test-inventory.json`, and this issue's run relabels its output to
these repository-facing filenames when publishing the optimized comparison,
so the two revisions' evidence can sit side by side under `dev/benchmarks/`
without one overwriting the other.

## What changed

The optimized run measures the tip of the story's landed optimization work,
each already merged and gated independently:

- jit:5d862134 — Stabilize build provenance inputs (Git-metadata-only changes
  no longer relink every test target).
- jit:8d4f7084 — Consolidate Rust integration tests into cohesive suites
  (144 integration targets -> 11).
- jit:57d0eb79 — Bound debug information and gate incremental state
  (`line-tables-only` debuginfo, `CARGO_INCREMENTAL=0` in the gate).
- jit:3398bc19 — Remove unused resolver and duplicate TLS features (drops
  `aws-lc-rs`/`aws-lc-sys`, `reqwest`, `native-tls`, `tokio-rustls`,
  `hyper-rustls`, and related crates from the default build; see the
  compiled-crate diff below).
- jit:83efbcb4 — Bound provenance test source snapshots.
- jit:3f73423b — Enforce Rust build-footprint budgets in `cargo-ci` (the
  `bounded-rust-build-footprint` invariant's automated checker).

Directly comparing the two clean samples' `cargo test --workspace --no-run`
compiler-artifact streams (`raw/clean-1/test-no-run.log` vs.
`raw/optimized-clean-1/test-no-run.log`) confirms the dependency-pruning work
took effect: baseline compiles 277 crates, optimized compiles 260. The 17
removed crates are exactly the remote-schema-resolution and duplicate-TLS
dependency subtree named in the design doc (`aws-lc-rs`, `aws-lc-sys`,
`reqwest`, `native-tls`, `tokio-rustls`, `hyper-rustls`, `h2`, `openssl`,
`rustls-native-certs`, `rustls-platform-verifier`, `webpki-root-certs`,
`der`, `base64ct`, `foreign-types`, `foreign-types-shared`, `pem-rfc7468`,
`ipnet`), offset by 2 small additions (`rcgen`, `yasna`). `git2`'s own
platform-OpenSSL requirement (`openssl-sys`, `openssl-probe` at the versions
`git2` pins) is unaffected — only the newer `openssl`/`openssl-probe`
versions pulled in by the removed `native-tls`/`reqwest` chain are gone.
`aws-lc-sys` in particular compiles a full C cryptography library from
source and is well known to dominate wall-clock time in workspaces that pull
it transitively; its removal, together with the rest of the pruned subtree,
is the primary driver of the clean and rebuild wall-time reductions below —
not a measurement artifact, and not explained by the modest 6% reduction in
total compiled-crate count alone.

## Comparison and acceptance thresholds

Percentage reduction computed as `100 × (baseline − optimized) / baseline`
from the medians above, per REQ-04/REQ-05 (a positive value is a reduction;
this is the same quantity REQ-04/REQ-05 phrase as "X% below baseline").

| Metric | Baseline median | Optimized median | Reduction | Threshold | Result |
|---|---:|---:|---:|---|---|
| Clean: clippy | 26.885s | 20.312s | 24.4% | (no threshold) | — |
| Clean: `test --no-run` (REQ-04) | 195.019s | 22.274s | **88.6%** | >=40% lower | **PASS** |
| Clean: total (clippy + test-compile) | 222.069s | 42.586s | 80.8% | (no threshold) | — |
| Rebuild: incremental `test --no-run` (REQ-05) | 178.865s | 7.580s | **95.8%** | >=60% lower | **PASS** |
| Clean target-directory bytes, median (REQ-03) | 19.6 GiB | 3.56 GiB | 81.9% | <=10 GiB | **PASS** |
| Unique active test-executable bytes (REQ-03) | 13.8 GiB | 0.91 GiB | 93.4% | <=2 GiB | **PASS** |

Every hard budget and improvement threshold in the story's acceptance
criteria (REQ-03/REQ-04/REQ-05) is met with wide margin: the clean
test-compilation improvement (88.6%) is more than double the 40% floor, the
rebuild improvement (95.8%) is well past the 60% floor, and both disk budgets
land at roughly a third to a twentieth of their ceilings.

## Test inventory verification (REQ-07)

Four validation variants, run at the optimized revision:

| Variant | Result |
|---|---|
| `cargo test --workspace` | 3,316 passed, 0 failed, 14 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | zero warnings |
| `cargo clippy -p jit --features html,xml --all-targets -- -D warnings` | zero warnings |
| `cargo test -p jit --features html,xml` | all non-doctest suites passed (unit + all 10 of the `jit` crate's integration suites — `document_api_tests` belongs to `jit-server`, out of `-p jit` scope — 0 failed); 66/66 doctests passed |

The feature-gated doctest step initially failed under this host's default
`/tmp` (a quota-bound tmpfs already near capacity) with `ld terminated with
signal 7 [Bus error]` on every doctest binary — the same class of resource
exhaustion `scripts/cargo-ci.sh` already works around by pointing `TMPDIR` at
a disk-backed cache directory. Re-running only the doctest step
(`cargo test -p jit --doc --features html,xml`) with
`TMPDIR=${XDG_CACHE_HOME:-$HOME/.cache}/jit-cargo-ci-tmp` passed cleanly (66
passed, 0 failed); every non-doctest suite in the same feature-gated run had
already passed under the default `TMPDIR` in the same invocation, confirming
the failure was host resource pressure, not a code defect.

**Pre-change vs. post-change test inventory.** Every one of the 3,306 cases
in [`pre-change-test-inventory.json`](pre-change-test-inventory.json) is
accounted for in [`post-change-test-inventory.json`](post-change-test-inventory.json):
3,303 match exactly by name (through the consolidation qualification rule
recorded in `consolidation-inventory-diff.json`'s `case_name_map`) and
ignored-status, and the remaining 3 are exactly the cases documented as
removed by jit:5d862134 in that same artifact's `pre_existing_drift`
(`version_cli_tests`, superseded by provenance-injection tests covering the
same behavior under new names). Zero unjustified missing cases, zero
ignored-status mismatches. The committed `verify-consolidation-inventory.py`
corroborates this independently against the current tree: 0 missing, 21
"extra" cases — new tests added by the subsequent optimization stories
(`build_profile_policy_tests`, `dependency_feature_policy_tests`,
`remote_document_tls_tests`, `rust_build_budget_checker_tests`,
`repository_inventory_tests`, `scope_validation_tests`), not a loss.
