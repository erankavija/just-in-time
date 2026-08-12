# Live suite-duration enforcement (jit:4b7c06d0 REQ-01, jit:94d85bf1)

Evidence that the enforced suite clock sits inside its budget, and what the
margin actually is on this host. `scripts/cargo-ci.sh` now measures the named
suite clock and passes it to `scripts/rust-build-budget.sh` as
`--test-suite-ms`, where `MAX_TEST_SUITE_SECONDS=30` fails the gate at or above
30,000 ms. Before `94d85bf1` the argument was never supplied and the check
reported `suite-duration=skipped`.

Raw per-step output is under `raw/`; each `raw/*.total` records the wall clock
of the whole script, measured around the invocation.

Environment: `TMPDIR` unset, so the gate inherits the host default (`/tmp`,
tmpfs). `CARGO_INCREMENTAL=0`. Already-built warm target, as the criterion
requires. Runs taken at `0e71077d9`, holding both the checkout and host-CPU
advisory locks so nothing else on the machine was compiling.

## Idle host

| Step | run 1 | run 2 | run 3 |
| --- | --- | --- | --- |
| incremental-preflight | 124 | 131 | 129 |
| fmt | 1525 | 1564 | 1602 |
| clippy | 205 | 201 | 205 |
| suite-build | 377 | 358 | 356 |
| test | 17418 | 18328 | 17397 |
| doctest | 4739 | 5281 | 4681 |
| budget | 233 | 248 | 231 |
| incremental-state | 129 | 133 | 128 |
| **suite-clock** | **22175** | **23628** | **22095** |
| **total gate** | **24821** | **26307** | **24794** |

All values in milliseconds. Every run exits 0 with 4481 tests run, 0 failed,
7 skipped, and 63 doctests. Run 2 is the outlier because it began while run 1's
own load was still decaying; consecutive gate runs are not independent samples.

`suite-build` is the step `94d85bf1` added so compilation is paid outside the
measured span. At 356-377 ms it is a no-op on a warm target, which is the
property that lets a cold target pass the budget at all: on a cold one the same
step absorbs the whole build, and the clock still measures only execution.

## Contended host

Gate run `88661720` at `13657b9e9`, taken during a concurrent cold workspace
build in another worktree, 1-min loadavg 14.07:

| Step | contended | idle (run 3) |
| --- | --- | --- |
| suite-build | 32389 | 356 |
| test | 21753 | 17397 |
| doctest | 7706 | 4681 |
| incremental-state | 11497 | 128 |
| **suite-clock** | **29477** | **22095** |

It passed, by 523 ms.

## What the margin is

The budget is 30,000 ms. This tree measures ~22,100 ms idle, so the headroom is
about 7,900 ms — and one concurrent cold build consumes 93% of it. The suite
itself is not the variable: `incremental-state` is a directory walk with nothing
to compute and it went 128 ms to 11,497 ms, so most of the loss is I/O
contention rather than CPU.

Two consequences for anyone reading a red gate:

- A `test suite duration` failure is provisional until the host is known to have
  been quiet. Check the load first, then re-measure.
- Re-measuring needs `--force`. `jit gate evaluate` reuses a prior verdict over
  identical declared inputs, and `target/` is not a declared input, so nothing
  done to the build tree invalidates a recorded failure.
