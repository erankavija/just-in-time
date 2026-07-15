# Rust build efficiency: pre-optimization baseline

Measurement foundation for the `rust-build-efficiency` story (jit:73482aa1),
produced by jit:4e22a20d before any build-topology, profile, or dependency
change. Captures a trustworthy pre-optimization baseline and a complete
pre-change test inventory so later work (suite consolidation, profile tuning,
dependency pruning) can be judged against real numbers instead of a single
anecdotal run.

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
  into the same inventory under a synthetic `"doctest"` kind. See "Current
  footprint" below for the full accounting.
- **Measurement**: wall time via `time.monotonic()` around each command;
  maximum resident set size via `getrusage(RUSAGE_CHILDREN)` immediately
  after the command exits — the same mechanism GNU `time -v` uses internally,
  with the same caveat: for a multi-process build tree (cargo fans out many
  rustc invocations) this is the high-water mark of the single largest reaped
  process in the tree, not a sum across concurrently running rustc processes.

## Environment

| | |
|---|---|
| Git revision (pre-optimization) | `56d737891129babf9571eb4de5659295bce8d827` |
| OS | Arch Linux, kernel Linux 7.0.14-arch1-1 |
| CPU count | 24 |
| Memory | 33,543,761,920 bytes (~31.2 GiB) |
| Cargo | cargo 1.97.0 (c980f4866 2026-06-30) |
| Rustc | rustc 1.97.0 (2d8144b78 2026-07-07) |
| Target triple | x86_64-unknown-linux-gnu |
| Linker | cargo default (no `.cargo/config.toml` linker override present): cc (GCC) 16.1.1 20260625 invoking GNU ld (GNU Binutils) 2.46.1 |
| Relevant `CARGO_*`/`RUST*` env vars | none set |

Full environment block, including the exact `cargo_rust_env` capture, is in
[`baseline.json`](baseline.json)'s `environment` object — this table is a
projection of it, not a second source of truth.

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

## Current footprint (from the test inventory, same clean sample)

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
