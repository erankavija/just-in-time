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
  status. Doctests are out of scope: `cargo test --no-run` does not compile
  doctest binaries.
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
- The harness's own final JSON-assembly step (after all 6 samples had already
  completed and been recorded successfully) did not print a completion
  message or leave a trace of failure in the run log; the process was gone
  when checked roughly an hour later. All 6 samples' measurements were
  already durably recorded in `raw/clean-samples.jsonl` /
  `raw/rebuild-samples.jsonl` at that point, so `baseline.json` was
  regenerated directly from that recorded data (the same `jq` assembly the
  script performs, reproduced manually and verified to succeed standalone);
  no re-sampling was needed and no sample data was discarded or rerun.

## Current footprint (from the test inventory, same clean sample)

| | |
|---|---|
| Complete target-directory bytes | 21,089,820,236 (~19.6 GiB) |
| Unique active test-executable bytes | 15,314,841,704 (~14.3 GiB) |
| Integration-test targets (Cargo `target.kind == ["test"]`) | 144 |
| Total test targets (lib + bin + integration) | 150 |
| Total discoverable test cases | 3,240 |
| Ignored test cases | 14 |

Cross-check against `cargo-ci.sh`'s full `cargo test --workspace` run (which,
unlike `--no-run`, also executes doctests): 3,226 non-ignored inventoried
cases + 66 doctests (`cargo test --workspace --doc`: 66 passed for `jit`, 0
for `jit_server`) = 3,292, plus the same 14 ignored = 3,306 total — matching
`cargo-ci.sh`'s reported `3292 passed, 0 failed, 14 ignored` exactly. The
inventory is complete modulo the documented doctest exclusion.

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
test_count, ignored_count, list_exit_code, list_ignored_exit_code}], totals,
target_dir_bytes, unique_active_test_executable_bytes}`.
