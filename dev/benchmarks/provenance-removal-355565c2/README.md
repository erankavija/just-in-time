# Warm `cargo-ci` after removing build provenance (jit:355565c2)

REQ-06 evidence: warm `scripts/cargo-ci.sh` runs from an already-built target
directory, on the host that produced the issue's "before" figures. Raw per-step
output is under `raw/`; each `raw/*.total` records the wall clock of the whole
script, measured around the invocation.

Environment: `TMPDIR` unset, so the gate inherits the host default (`/tmp`,
tmpfs). `CARGO_INCREMENTAL=0`. Branch `worktree-agent-355565c2`.

Runs 1 and 2 were taken at `7fe0eae6`; runs 3 and 4 at `e35a36b6`, whose only
difference is a doc comment. `raw/rebuild-after-edit.*` is the run in between,
which paid for the rebuild that comment forced and is kept only so the warm runs
are not mistaken for a continuous series — it is not a REQ-06 measurement.

## Result

| Step | run 1 | run 2 | run 3 | run 4 |
| --- | --- | --- | --- | --- |
| incremental-preflight | 33 | 33 | 34 | 33 |
| fmt | 1540 | 1560 | 1541 | 1536 |
| clippy | 202 | 200 | 205 | 202 |
| test | 17473 | 17517 | 17491 | 17531 |
| doctest | 4603 | 4600 | 4664 | 4649 |
| budget | 230 | 232 | 231 | 229 |
| incremental-state | 33 | 33 | 32 | 33 |
| **suite-clock** | **22093** | **22136** | **22173** | **22199** |
| **total gate** | **24175** | **24240** | **24259** | **24276** |

All values in milliseconds. Every run exits 0 with 4479 tests run, 0 failed,
7 skipped, and 63 doctests.

## Against the before figures

The issue's measurement at `1bd734a2`, same host: a warm gate totalled
192,994 ms, of which the `provenance` step alone was 116,971 ms, and the disk-
backed `TMPDIR` override cost the rest of the suite 104,285 ms against
18,341 ms on tmpfs.

- Total gate: 192,994 ms → 24,276 ms worst case (7.95x faster), budget 45,000 ms.
- Suite clock: 69,000-104,000 ms inside `cargo-ci` → 22,199 ms worst case,
  budget 25,000 ms.
- The `provenance` step no longer exists, and `TMPDIR` is the host default.
