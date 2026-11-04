# Test-run rules (steward, 2026-11-04)

- Your own test runs are advisory. Use the TMPDIR your prompt header names (per-worktree), with `CARGO_INCREMENTAL=0`. If no header names one, use `TMPDIR=/mnt/nvme/jit/tmp/df129d9e-workers`.
- Run every whole-suite or multi-target run (`cargo test -p jit --tests`, `cargo nextest run …`, `cargo test --workspace`) under the host lock: `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock <command>`. Targeted single-test runs while editing need no lock.
- On a disk-backed TMPDIR a few tests fail for disk-only reasons: `commands::gate_check::tests::test_check_gate_classifies_named_run_drift_but_not_unrelated_run` (directory order) and sync-heavy tests that time out at 20 s (the re-application proptest, `test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses`, the `test_profile_*` package tests). Report them; do not rework for them. The lead's branch CI and gates are authoritative.
