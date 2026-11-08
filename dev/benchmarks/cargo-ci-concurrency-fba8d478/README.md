# cargo-ci concurrency (fba8d478, 2026-11-08)

Two `scripts/cargo-ci.sh` runs started together on different worktrees under the host's cargo jobserver pool, with the lock bypassed (`CARGO_CI_NO_LOCK=1`) and the per-test limits of `.config/nextest.toml` enforced (2 s unit, 6 s integration).

## Method

- Pool: `cargo-pool-daemon` was running (pid 81426, [pool-daemon.txt](pool-daemon.txt)). Test processes take pool tokens through `~/.dotfiles/bin/cargo-pool-run`.
- Trees: worktree A (`agent-fba8d478`) and twin B (`agent-fba8twin`), both at `23d73ee42`, 24 CPUs. Each has its own target directory and `TMPDIR` (`/mnt/nvme/jit/tmp/w-fba8a`, `w-fba8b`), `CARGO_INCREMENTAL=0`.
- Warm-up, unmeasured: `cargo build --workspace --all-targets` and `cargo nextest run --workspace --no-run` in each, one at a time. Trials therefore measure test-time contention, not a cold compile.
- Every run, baseline and trial, waited for a 1-minute load below 1 first (`waitload-*.txt`, sampled every 15 s). The host was drained by the other leads; the only processes above idle during the runs were these.
- Serial baseline: `./scripts/cargo-ci.sh` with the lock, A then B, one at a time. Wall-clock from `date +%s.%N` around the script.
- Trials: `CARGO_CI_NO_LOCK=1 ./scripts/cargo-ci.sh` in A and B started together from one shell, then `wait`. `/proc/loadavg` sampled every 5 s into `load-trialN.txt`.
- Logs: `<run>.log` (step lines and any TIMEOUT/FAIL), `<run>.summary` (exit code, wall-clock), `<run>.start` (start load).

## Serial baseline (lock held, quiet host)

| run | start load1 | exit | wall (s) | test step (ms) |
| --- | --- | --- | --- | --- |
| A | 0.82 | 0 | 33.0 | 27555 |
| B | 0.80 | 0 | 33.1 | 27696 |

Serial cost of two runs: 66.1 s. No TIMEOUT or FAIL line in either run: no test ran over its layer limit at load below 1.

The slowest tests in a separate quiet-host nextest run (load1 0.83, [junit-quiet.xml](junit-quiet.xml), [slowest-run.log](slowest-run.log)), all under their limit:

| layer | limit | test | s |
| --- | --- | --- | --- |
| integration | 6 s | `cli_repo_workflow::steering_scenarios::test_all_steering_scenarios` | 4.099 |
| integration | 6 s | `profile_acceptance_tests::test_shipped_profile_archive_bootstraps_a_repository_without_jit_state` | 3.411 |
| integration | 6 s | `lease_enforcement_tests::test_strict_lease_exempt_paths_write_for_their_stated_reason` | 3.282 |

Slowest five unit tests ([slowest-unit.txt](slowest-unit.txt)): `commands::gate_check::tests::test_check_gate_executes_again_after_declared_inputs_change` 1.419, `test_utils::tests::test_profiled_repository_fixture_memory_build_writes_the_tree_a_file_build_writes` 1.369, `commands::gate_check::tests::test_check_gate_classifies_named_run_drift_but_not_unrelated_run` 1.358, `commands::gate_check::tests::test_check_gate_records_pre_run_verdict_when_gate_registry_changes_during_checker` 1.340, `profile::repository_package::tests::test_profile_applies_to_neutral_repo_and_ordinary_renderers_consume_config` 1.294 (limit 2 s).

## Trials (lock bypassed, both runs together)

| trial | start load1 | peak load1 | run | exit | verdict | wall (s) | test step (ms) | TIMEOUT/FAIL |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 0.94 | 15.86 | A | 0 | pass | 58.5 | 52814 | none |
| 1 | | | B | 0 | pass | 66.4 | 53109 | none |
| 2 | 0.78 | 17.49 | A | 0 | pass | 58.6 | 52792 | none |
| 2 | | | B | 0 | pass | 66.5 | 53127 | none |
| 3 | 0.90 | 16.16 | A | 0 | pass | 66.4 | 52959 | none |
| 3 | | | B | 0 | pass | 58.4 | 52617 | none |

Every run: 5159 passed, 0 failed, 0 skipped; 25 doctests passed. The slower wall in each trial is the run whose doctest step took 10.5 s instead of 2.9 s.

## Against the serial baseline

Both runs of a trial finish in 58 to 67 s, so the pair costs about 66 s against 66.1 s for two serial runs. Each run's test step takes about 53 s together against 27.6 s alone: the two suites share the same cores, and concurrency leaves the total unchanged. What it removes is the queue: the second gate no longer waits for the first to finish, so its wait drops from a full run to nothing and both verdicts arrive at about the same time.

## Decision

Remove the lock. All three trials ended with both runs at exit 0 and no TIMEOUT or FAIL line, although each test step ran at about twice its serial duration (D-01, REQ-02). The pool exists only on this host, so another host running concurrent gates has no such evidence. Two trials ran 1 to 2 minutes apart on the same pair of trees; a larger pair count or a third concurrent run is unmeasured.
