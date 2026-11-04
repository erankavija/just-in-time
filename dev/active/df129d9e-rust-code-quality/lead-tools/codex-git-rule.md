
# Git rule for Codex workers (overrides any merge instruction above)

Do not run `git merge`, `git commit`, `git add`, `git checkout`, `git restore`, `git stash`, `git reset` or any other git command that writes. Your sandbox makes git metadata read-only, and workarounds corrupt the branch. Read-only git (`status`, `diff`, `log`, `show`, `grep`) is fine. Leave all changes uncommitted; the lead merges main into your branch and commits after review.

# Temp directory rule (steward, 2026-11-04)

Your own test runs are advisory: prefix every cargo test or nextest command with `TMPDIR=/mnt/nvme/jit/tmp/df129d9e-workers CARGO_INCREMENTAL=0`, so parallel worker suites do not fill the shared `/tmp` tmpfs during the lead's authoritative runs. On that disk-backed TMPDIR a few tests are known to fail for disk-only reasons: `commands::gate_check::tests::test_check_gate_classifies_named_run_drift_but_not_unrelated_run` (directory order) and sync-heavy tests that time out at 20 s (the re-application proptest, `test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses`, the `test_profile_*` package tests). Report them in your return; do not rework for them. The lead's branch CI and gates run on the default `/tmp`.
