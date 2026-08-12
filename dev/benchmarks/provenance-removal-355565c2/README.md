# Warm `cargo-ci` after removing build provenance (jit:355565c2)

REQ-06 evidence: two consecutive warm `scripts/cargo-ci.sh` runs from an
already-built target directory, on the host that produced the issue's "before"
figures. Raw per-step output is under `raw/`; `raw/*.total` records the wall
clock of the whole script, measured around the invocation.

Environment: `TMPDIR` unset, so the gate inherits the host default (`/tmp`,
tmpfs). `CARGO_INCREMENTAL=0`. Measured at `7fe0eae6` on branch
`worktree-agent-355565c2`.

## Result

| Step | run 1 (ms) | run 2 (ms) |
| --- | --- | --- |
| incremental-preflight | 33 | 33 |
| fmt | 1540 | 1560 |
| clippy | 202 | 200 |
| test | 17473 | 17517 |
| doctest | 4603 | 4600 |
| budget | 230 | 232 |
| incremental-state | 33 | 33 |
| **suite-clock** | **22093** | **22136** |
| **total gate** | **24175** | **24240** |

Both runs pass exit 0 with 4479 tests run, 0 failed, 7 skipped, and 63
doctests.

## Against the before figures

The issue's measurement at `1bd734a2`, same host: a warm gate totalled
192,994 ms, of which the `provenance` step alone was 116,971 ms, and the disk-
backed `TMPDIR` override cost the rest of the suite 104,285 ms against
18,341 ms on tmpfs.

- Total gate: 192,994 ms → 24,175 ms (7.98x faster), budget 45,000 ms.
- Suite clock: 69,000-104,000 ms inside `cargo-ci` → 22,093 ms, budget
  25,000 ms.
- The `provenance` step no longer exists, and `TMPDIR` is the host default.
