# Re-measurement for afd2d573 (2026-11-08)

`commands::bootstrap::tests::test_open_repository_replays_original_audit_before_recovered_write_policy` against the 2 s unit limit in [../README.md](../README.md). Method as in the 0b35be66 re-measurement: `CARGO_INCREMENTAL=0 cargo nextest run --workspace --no-fail-fast` under the host locks, disk-backed `TMPDIR`, duration from the JUnit `time` attribute.

| run | tree | 1-min load at start | duration |
| --- | --- | --- | --- |
| before (0b35be66 after2) | `7945753a8` | 2.13 | 3.443 s |
| before (0b35be66 after1) | `bcd69793d` | 3.98 | 1.835 s |
| before, isolated focused run | `0ef4cdd9f` | 0.20 | 0.833 s |
| after, isolated focused run | afd2d573 | 0.20 | 0.187 s |
| after, whole workspace | afd2d573 | 3.72 | 0.268 s |

The whole-workspace run also timed out `prop_re_application_after_an_interruption_at_any_point_reaches_and_holds_the_new_state` at 20 s: this tree is based on main without 0b35be66's unsynced redo-conformance backend. Main now has 0b35be66 (`ae01461e7`), so those timeouts are fixed there; not re-measured after merging main and joining the case threads through `join_reraising`.

## Cause and change

Each of the 8 cases built its own git repository (init, config, add, commit) and linked worktree, one after another. The test now builds one repository with one commit, adds one linked worktree per case, and runs the cases on scoped threads. Every case keeps its own worktree, redo record and assertions; the case matrix and all assertions are unchanged. No sleeps or waits.
