# Re-measurement for 4a01ccba (2026-11-04)

## Quiet host (authoritative)

Three full `cargo-ci.sh --cargo nextest run --workspace --no-fail-fast` runs under the exclusive lock during a host-wide quiet window (every lead paused builds and tests), disk-backed `TMPDIR`. Each run started only when the 1-minute load was below 4; the load at each start was 1.82, 3.32 and 3.94. Raw junit, run logs, load averages and `distribution.py` output are in [quiet/](quiet/).

| layer | limit | test | median | worst |
| --- | --- | --- | --- | --- |
| unit | 2 s | `commands::document::tests::test_add_document_reference_succeeds_over_an_event_log_past_the_former_ceiling` | 0.629 s | 0.653 s |
| unit | 2 s | `commands::serve::tests::test_start_server_bounds_a_child_that_neither_serves_nor_exits` | 0.062 s | 0.062 s |
| integration | 6 s | `lease_enforcement_tests::test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses` | 2.131 s | 2.144 s |

All three are under their limits on a quiet host.

## Loaded host (first remeasurement)

Method: the "Reproducing" section of `../README.md`, three full `cargo-ci.sh --cargo nextest run --workspace --no-fail-fast` runs under the exclusive lock (`flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock`), disk-backed `TMPDIR`, then `distribution.py`. Raw junit, run logs, load averages and distribution output are beside this note. `--no-fail-fast` lets a run finish when an unrelated test hits nextest's 20 s limit (see below).

Load average (1 min) at the start of runs 1 to 3: 9.1, 12.3, 14.2. Other agents on this host do not take the lock, so the host was not quiet; the figures are an upper bound for a quiet host.

| layer | limit | test | median | worst | before (same host, alone) |
| --- | --- | --- | --- | --- | --- |
| unit | 2 s | `commands::document::tests::test_add_document_reference_succeeds_over_an_event_log_past_the_former_ceiling` | 0.639 s | 0.650 s | 2.42 s |
| unit | 2 s | `commands::serve::tests::test_start_server_bounds_a_child_that_neither_serves_nor_exits` | 0.062 s | 0.062 s | 2.015 s |
| integration | 6 s | `lease_enforcement_tests::test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses` | 2.170 s | 2.193 s | over 20 s |

All three are under their limits. The first lease implementation (sequential leaves) measured 10.3 s median, 18.4 s worst in the suite; leaves now run on concurrent threads over copies of one seeded repository.

Not in the benchmark list: `test_strict_lease_exempt_paths_write_for_their_stated_reason` now takes 3.391 s median, 4.015 s worst (it timed out at 20 s in the interim suite runs before it shared the seeded copy). Unrelated tests that hit nextest's 20 s limit on this loaded host in every run: `storage::redo_conformance::prop_re_application_after_an_interruption_at_any_point_reaches_and_holds_the_new_state`, `profile_cli_tests::test_profile_add_publishes_a_package_whose_directories_outnumber_its_files`, and `profile_cli_tests::test_profile_pack_and_add_carry_a_package_at_the_model_limits`.
