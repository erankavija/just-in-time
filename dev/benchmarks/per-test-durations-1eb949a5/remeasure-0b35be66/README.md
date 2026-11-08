# Re-measurement for 0b35be66 (2026-11-08)

The ten tests f487aeef's enforced limits flagged, measured before and after their refactor against the per-layer limits in [../README.md](../README.md): 2 s unit, 6 s integration.

## Method

Each run is `CARGO_INCREMENTAL=0 cargo nextest run --workspace --no-fail-fast --tool-config-file measure:<scratch>/measure.toml` under `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`, disk-backed `TMPDIR`, after an unmeasured `--no-run` build. The scratch config holds only `[profile.default.junit] path = "junit.xml"`. Durations are the `time` attribute of each `<testcase>` in the JUnit report. Raw reports, load samples and run summaries are beside this note.

| run | tree | start (EET) | 1-min load at start | result |
| --- | --- | --- | --- | --- |
| before | `bf72f7b1f` (base) | 01:08 | 13.72 | 5180 passed, 2 timed out at 20 s |
| after1 | `bcd69793d` | 01:39 | 3.98 | 5182 passed |
| after2 | `7945753a8` (after1, main merged, records-commit cut) | 02:07 | 2.13 | 5175 passed |
| after3 | `4f38ff979` (after2, unsynced event-tail and composition fixtures) | 04:37 | 3.93 | 5175 passed |

The before run was not quiet; no quiet run of the base tree exists with true durations. f487aeef's quiet runs (loads 2.81 and 4.79, 2026-11-08) killed all ten at their limit, so they bound the before figures from below: every unit test at 2.003 to 2.011 s, both integration tests at 6.003 s. after3 is the authoritative after figure: it measures the final tree.

## Durations (seconds)

| layer | limit | test | before (load 13.72) | after1 (load 3.98) | after2 (load 2.13) | after3 (load 3.93) |
| --- | --- | --- | --- | --- | --- | --- |
| unit | 2 s | `storage::claim_coordinator::proptests::test_concurrent_claims_exclusive` | 2.352 | 0.412 | 0.387 | 0.448 |
| unit | 2 s | `storage::claim_coordinator::proptests::test_concurrent_different_issues_succeed` | 1.921 | 0.436 | 0.478 | 0.473 |
| unit | 2 s | `storage::redo_conformance::prop_an_interruption_during_the_records_commit_leaves_the_complete_old_state` | 1.958 | 1.844 | 0.551 | 0.721 |
| unit | 2 s | `storage::redo_conformance::prop_re_application_after_an_interruption_at_any_point_reaches_and_holds_the_new_state` | 20.004 (timeout) | 0.475 | 0.463 | 0.442 |
| unit | 2 s | `storage::redo_conformance::prop_replay_converges_from_any_subset_of_persisted_operations` | 20.003 (timeout) | 0.379 | 0.463 | 0.267 |
| unit | 2 s | `commands::event_tail_tests::test_mutations_append_over_unparseable_history_without_capturing_the_whole_log` | 5.603 | 0.583 | 1.694 | 0.135 |
| unit | 2 s | `commands::gate_check::tests::test_repository_validation_checker_passes_exactly_where_validate_passes` | 3.122 | 0.424 | 0.537 | 0.443 |
| unit | 2 s | `commands::profile::tests::test_apply_profiles_jit_dogfood_composes_what_applying_both_packages_composes` | 2.411 | 1.280 | 1.715 | 0.136 |
| integration | 6 s | `profile_cli_tests::test_profile_add_publishes_a_package_whose_directories_outnumber_its_files` | 11.362 | 2.696 | 2.274 | 2.524 |
| integration | 6 s | `profile_cli_tests::test_profile_pack_and_add_carry_a_package_at_the_model_limits` | 10.417 | 2.500 | 2.158 | 2.327 |

All ten are under their limit in every after run. In after3 none is within 25 % of its limit; the slowest unit test of the ten is the records-commit property at 0.721 s, and the slowest integration test is 2.524 s against 6 s.

## What each change cut

- **Claim proptests.** The contended claimants waited in the lock's 10 ms poll sleep. They now hold a zero lock wait, so a refused lock returns at once and the existing progress-watched retry asks again. The refusal assertions are unchanged.
- **Redo conformance.** The filesystem backend synced every staged file, directory and the record; on a disk-backed `TMPDIR` those syncs were the whole cost. The suite's filesystem backend now commits and applies with an explicit unsynced durability, for every property of the suite. No property observes a sync: every outcome is read back in-process, and the crash model's surviving subset is constructed rather than produced by a crash. Which syncs a pass issues stays owned by `redo_apply`'s own tests, which run synced. The records-commit property also reaches each proper prefix by truncating the committed file in place rather than rewriting it.
- **Event tail, validation checker, dogfood composition.** Their cases or routes each own a separate repository, so they now run on scoped threads; every case and assertion is kept. The event-tail and composition fixtures also build their repositories through `setup_unsynced_test_repo_with_config` and `unsynced_file_executor`, the shared fixtures over unsynced `JsonFileStorage`: neither property observes a sync, and those syncs held them at about 1.7 s in after2.
- **Profile model-limit exchange.** The add of a 512-file package with 4088 unshared directories issued about 4600 fsyncs. Both tests stay in the `cli_repo_workflow` integration target and drive pack and add through the library API, with the add over unsynced `JsonFileStorage`. The archive bound, hash, file-count, byte-budget and directory-count assertions are unchanged. The CLI wiring of pack and add stays owned by `test_profile_pack_and_add_carry_one_package_between_repositories`.

## Case counts

No property test runs fewer cases. Generator ranges are unchanged.

## Other tests over a limit

after3: none. Outside this issue's list, four unit tests ran within 25 % of the 2 s limit: `commands::bootstrap::tests::test_open_repository_replays_original_audit_before_recovered_write_policy` (1.797 s; 3.443 s in after2), `storage::claim_coordinator::proptests::test_sequence_numbers_monotonic` (1.739 s), `commands::gate_check::tests::test_repository_validation_records_a_repository_judged_wrong_as_a_failed_run` (1.648 s) and `commands::gate_check::tests::test_capture_gate_evaluation_image_holds_only_the_evaluated_issues_named_runs` (1.620 s).
