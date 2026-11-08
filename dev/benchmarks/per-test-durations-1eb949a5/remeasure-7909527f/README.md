# Re-measurement for 7909527f (2026-11-08)

The three unit tests that 0b35be66's run `after3` left within 25 % of the 2 s limit in [../README.md](../README.md), measured after their refactor.

## Method

As in [../remeasure-0b35be66/README.md](../remeasure-0b35be66/README.md): `CARGO_INCREMENTAL=0 cargo nextest run --workspace --no-fail-fast --tool-config-file measure:<scratch>/measure.toml` under `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`, disk-backed `TMPDIR`, after an unmeasured `--no-run` build. Durations are each `<testcase>`'s `time` in `junit-after.xml`; the load sample is `load-after.txt` and the summary `summary-after.txt`.

| run | tree | start (EET) | 1-min load at start | result |
| --- | --- | --- | --- | --- |
| before | 0b35be66 `after3` (`4f38ff979`) | 04:37 | 3.93 | 5175 passed |
| after | this change on `3caaf7a24` | 05:17 | 3.89 | 5165 passed |

## Durations (seconds)

| test | before (load 3.93) | after (load 3.89) |
| --- | --- | --- |
| `storage::claim_coordinator::proptests::test_sequence_numbers_monotonic` | 1.739 | 0.506 |
| `commands::gate_check::tests::test_repository_validation_records_a_repository_judged_wrong_as_a_failed_run` | 1.648 | 0.039 |
| `commands::gate_check::tests::test_capture_gate_evaluation_image_holds_only_the_evaluated_issues_named_runs` | 1.620 | 0.567 |

## What each change cut

- **Sequence numbers.** Every acquisition probed the branch label through `git rev-parse`; the proptest directory is no checkout, so each of about 660 probes forked git to learn there is none. The fixture now declares its paths outside a git repository, which yields the same absent branch without a process. No property of the file observes the branch. Cases, generator ranges and assertions are unchanged.
- **Failed-run records.** The four changes each own a repository, so they are judged on scoped threads, and `judge_repository_after` builds its repository through `setup_unsynced_test_repo_with_config`: the property is the checker's verdict, not a sync. The sibling test sharing the helper gains the same fixture.
- **Evaluated issue's named runs.** The repository is built through `setup_unsynced_test_repo_with_config`; the image contents and run history assertions are unchanged.

No test sleeps or waits on wall-clock time to observe its property; none was ignored, deleted or weakened.

## Other unit tests over 1.5 s

`commands::bootstrap::tests::test_open_repository_replays_original_audit_before_recovered_write_policy`: 1.833 s (1.797 s in 0b35be66's `after3`). It is outside this issue's list.
