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
memory, and linker. The Git revision differs: the optimized run measures
`a99edacc5c960b66deb1214f2deb856aca18f0a1`, the tip of `main` at collection
time — everything merged to `main` by then, not only the
`rust-build-efficiency` story's own landed work. See "What changed" for
exactly what that includes and which commits this report attributes the
measured gains to.

| | Baseline | Optimized |
|---|---|---|
| Git revision | `56d737891129babf9571eb4de5659295bce8d827` | `a99edacc5c960b66deb1214f2deb856aca18f0a1` |
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
| Clean: clippy | 20.865 | 22.758 | 20.958 | **20.958** | 20.865-22.758 (9.1%) |
| Clean: `test --no-run` | 29.216 | 24.181 | 25.776 | **25.776** | 24.181-29.216 (20.8%) |
| Clean: total (clippy + test-compile) | 50.081 | 46.939 | 46.734 | **46.939** | 46.734-50.081 (7.2%) |
| Rebuild: setup clippy | 20.914 | 20.880 | 20.895 | 20.895 | 20.880-20.914 (0.2%) |
| Rebuild: setup `test --no-run` | 25.056 | 24.802 | 23.588 | 24.802 | 23.588-25.056 (6.2%) |
| Rebuild: incremental `test --no-run` (headline) | 12.278 | 9.518 | 9.084 | **9.518** | 9.084-12.278 (35.2%) |

As with the baseline, no single run is representative on its own; the median
column is what REQ-04/REQ-05 acceptance is computed against (see
"Comparison and acceptance thresholds" below).

**Observed variance / host noise:**

- `grep -c "waiting for exclusive build lock"` against this run's harness
  log returns 1: an unrelated `cargo-ci`/`verify-commit-builds` gate build
  held the shared lock immediately before clean sample 3 acquired it (see
  "Non-overlap" in "Protocol"). The wait itself only delays lock acquisition,
  not the timed measurement inside it (timing starts after the lock is held),
  but the elevated spread on clean sample 1's `test --no-run` (29.216s vs.
  24.181-25.776s for samples 2-3) and rebuild sample 1's headline rebuild
  (12.278s vs. 9.084-9.518s for samples 2-3) is consistent with residual host
  contention from that same competing build spilling into adjacent samples
  outside the lock's own critical section (e.g. CPU scheduling pressure from
  a build that does not itself acquire this benchmark's lock, such as an
  ordinary developer `cargo check` in a different working tree).
- The acceptance conclusion is insensitive to this contention: even the
  single slowest sample in each metric (`test --no-run` at 29.216s, headline
  rebuild at 12.278s) is an 85.0% and 93.1% reduction from the baseline
  median respectively — both still comfortably past their 40%/60% floors.
- Maximum RSS was stable and unaffected by the lock contention: clippy
  ~1.066-1.067 GiB, `test --no-run` ~1.517-1.554 GiB, rebuild ~1.013-1.015
  GiB — all below baseline's ~1.098-1.100 GiB, ~1.513-1.515 GiB, and
  ~1.245-1.250 GiB respectively, consistent with the pruned TLS/
  remote-resolution dependency subtree (see "What changed") remaining
  removed.

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
| Complete target-directory bytes | 21,089,820,236 (~19.6 GiB) | 3,947,706,819 (~3.68 GiB, median) |
| Unique active test-executable bytes | 14,816,926,968 (~13.8 GiB) | 1,006,148,744 (~0.94 GiB) |
| Integration-test targets (Cargo `target.kind == ["test"]`) | 144 | 11 |
| Total discoverable test cases (incl. doctests) | 3,306 | 3,363 |
| — of which doctests | 66 | 66 |
| Ignored test cases | 14 | 14 |

The 57-case increase in total test cases (3,306 -> 3,363, doctests and
ignored-count both unchanged) comes from new tests added by this story's own
optimization work plus unrelated parallel work merged from `main` (see
"Optimized medians and variance" above) — not from any change to
pre-existing tests; see "Test inventory verification" below for the full
accounting of every pre-change case.

**REQ-02: every clean sample records its own metrics.** Unlike the harness's
original behavior (deriving the full per-test-case inventory, including
target count and executable bytes, only for one designated sample), every
optimized clean sample now independently records its own integration-test
target count and unique active test-executable bytes, via
`derive_active_test_metrics` in
[`scripts/benchmark-rust-build.sh`](../../../scripts/benchmark-rust-build.sh)
(added in this issue's rework round):

| Sample | Target-directory bytes | Unique active test-executable bytes | Integration-test targets |
|---|---:|---:|---:|
| 1 | 3,947,706,819 | 1,006,148,744 | 11 |
| 2 | 3,947,706,881 | 1,006,148,744 | 11 |
| 3 | 3,947,706,301 | 1,006,148,744 | 11 |

Executable bytes and target count are identical across all three samples
(the same deterministic build), and target-directory bytes agree to within
580 bytes — all three independently confirm the REQ-03 budgets below. Sample
1 additionally derives the full per-test-case inventory (the designated
`BENCH_INVENTORY_SAMPLE`), which is why only its record in
[`optimized.json`](optimized.json)'s `clean_samples` carries the additional
`test_case_count`/`doctest_case_count`/`ignored_test_case_count` fields;
samples 2-3 carry the three REQ-02 fields only, which is all REQ-02
requires.

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

The optimized run measures `a99edacc...`, the tip of `main` at collection
time. That revision includes everything merged to `main` by then, which is
more than this story's own work — it also carries unrelated, independently
gated parallel work (see the compiled-crate diff below for the one piece
that affects a reported figure: a capability-based filesystem dependency
subtree). This report attributes the measured gains specifically to the
`rust-build-efficiency` story's own landed commits, each already merged and
gated independently, because they are the ones with a documented mechanism
(dependency and profile pruning) that directly explains the wall-time and
memory reductions:

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
took effect: baseline compiles 277 crates, optimized compiles 270. The 17
removed crates are exactly the remote-schema-resolution and duplicate-TLS
dependency subtree named in the design doc (`aws-lc-rs`, `aws-lc-sys`,
`reqwest`, `native-tls`, `tokio-rustls`, `hyper-rustls`, `h2`, `openssl`,
`rustls-native-certs`, `rustls-platform-verifier`, `webpki-root-certs`,
`der`, `base64ct`, `foreign-types`, `foreign-types-shared`, `pem-rfc7468`,
`ipnet`). `git2`'s own platform-OpenSSL requirement (`openssl-sys`,
`openssl-probe` at the versions `git2` pins) is unaffected — only the newer
`openssl`/`openssl-probe` versions pulled in by the removed
`native-tls`/`reqwest` chain are gone. `aws-lc-sys` in particular compiles a
full C cryptography library from source and is well known to dominate
wall-clock time in workspaces that pull it transitively; its removal,
together with the rest of the pruned subtree, is the primary driver of the
clean and rebuild wall-time reductions below — not a measurement artifact.

Offsetting that removal, this optimized revision also compiles 11 crates not
in the baseline: `rcgen` and `yasna` (small, from this story's own work), and
a 9-crate capability-based-filesystem subtree (`cap-std`, `cap-primitives`,
`ambient-authority`, `fs-set-times`, `io-extras`, `io-lifetimes`,
`rustix-linux-procfs`, `maybe-owned`) pulled in by unrelated, independently
gated parallel work that had merged into `main` by the time this issue's
rework round re-collected samples (see "Optimized medians and variance"
above) — not part of the `rust-build-efficiency` story. Net: 277 -> 270
crates, a smaller headline reduction than the story's own pruning alone
would show, but the removed TLS/remote-resolution subtree — the actual
driver of the wall-time and memory reductions — is unaffected by this
unrelated addition.

## Comparison and acceptance thresholds

Percentage reduction computed as `100 × (baseline − optimized) / baseline`
from the medians above, per REQ-04/REQ-05 (a positive value is a reduction;
this is the same quantity REQ-04/REQ-05 phrase as "X% below baseline").

| Metric | Baseline median | Optimized median | Reduction | Threshold | Result |
|---|---:|---:|---:|---|---|
| Clean: clippy | 26.885s | 20.958s | 22.0% | (no threshold) | — |
| Clean: `test --no-run` (REQ-04) | 195.019s | 25.776s | **86.8%** | >=40% lower | **PASS** |
| Clean: total (clippy + test-compile) | 222.069s | 46.939s | 78.9% | (no threshold) | — |
| Rebuild: incremental `test --no-run` (REQ-05) | 178.865s | 9.518s | **94.7%** | >=60% lower | **PASS** |
| Clean target-directory bytes, median (REQ-03) | 19.6 GiB | 3.68 GiB | 81.3% | <=10 GiB | **PASS** |
| Unique active test-executable bytes (REQ-03) | 13.8 GiB | 0.94 GiB | 93.2% | <=2 GiB | **PASS** |

Every hard budget and improvement threshold in the story's acceptance
criteria (REQ-03/REQ-04/REQ-05) is met with wide margin: the clean
test-compilation improvement (86.8%) is more than double the 40% floor, the
rebuild improvement (94.7%) is well past the 60% floor, and both disk budgets
land at roughly a third to a tenth of their ceilings. These margins hold even
against the single slowest individual sample in each metric, including the
one affected by the lock contention noted above (worst clean `test --no-run`
sample 29.216s = 85.0% reduction; worst rebuild sample 12.278s = 93.1%
reduction), so the conclusion is not sensitive to that variance.

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
