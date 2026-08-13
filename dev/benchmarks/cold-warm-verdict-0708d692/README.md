# One commit, one verdict, cold or warm (jit:0708d692)

Evidence that a fresh target directory and a warm one reach the same
suite-duration verdict, and the measured attribution of the gap that used to
separate them. Raw gate output is under `raw/`.

Environment: 24 cores, 31 GiB RAM, SATA disk, `sccache` on PATH, `TMPDIR` at the
host default (`/tmp`, tmpfs), idle host, nothing else compiling. Every run is
`./scripts/cargo-ci.sh` in one worktree at one commit, on the branch for this
issue. All values in milliseconds.

## The verdict, before and after

| Run | tree | suite-build | test | doctest | suite-clock | budget |
| --- | --- | --- | --- | --- | --- | --- |
| before, cold (`raw/before-cold.out`) | fresh `target/` | 45195 | 52446 | 4555 | 57019 | **FAILED** |
| before, warm (`raw/before-warm.out`) | warm | 360 | 17972 | 4552 | 22541 | passed |
| after, cold (`raw/after-cold.out`) | fresh `target/` | 26938 | 22277 | 4538 | 26832 | passed |
| after, warm (`raw/after-warm.out`) | warm | 570 | 18112 | 4546 | 22677 | passed |

4512 tests passed in the two "before" runs and 4514 in the two "after" ones; the
difference is this issue's own added policy tests. The budget threshold is
`MAX_TEST_SUITE_SECONDS` in `scripts/rust-build-budget.sh`.

## Where the 34,474 ms cold penalty was

The `test` substep was the whole of it: `doctest` measured 4,555 ms cold against
4,552 ms warm. Per-test durations were the same in both conditions — 311.7
test-seconds warm against 316.9 cold — while wall time tripled, which is what a
serialized stretch inside the run looks like, not a slower suite.

**24,677 ms — a second dependency-graph variant, compiled inside the clock.**
cargo-nextest runs setup scripts as part of the suite run, and
`scripts/setup-recorded-failure-corpus.sh` invoked
`cargo test -p jit --test cli_issue`. Cargo unifies features over the packages
one invocation selects, so that selection resolved a different unit graph than
the gate's `cargo test --workspace --no-run`. Measured on a target freshly built
by the gate's own build:

| invocation | elapsed | crates compiled |
| --- | --- | --- |
| `cargo test -p jit --test cli_issue --no-run` | 24677 | 60 |
| `cargo test --workspace --test cli_issue --no-run` | 131 | 0 |

**4,336 ms — the suite's own per-target fixture construction.** Deleting
`target/jit-profiled-repository-fixtures`, `target/debug/jit-recorded-failure-corpus`
and `target/debug/jit-conformance` on an otherwise warm tree moved the clock from
22,571 to 26,911 ms (`raw/fixtures-deleted-warm-tree.out`). This is the residual
that keeps a first run in a target directory slower than the next one.

**The remainder — first-touch reads of the linked executables.** With the page
cache for the 17 executables dropped (`dd if=… iflag=nocache count=0`), reading
them back sequentially costs 3,461 ms against 22 ms when resident. Paid inside
the clock it is worth less than that, because it overlaps with test execution:

| run | tree | suite-build | test | suite-clock |
| --- | --- | --- | --- | --- |
| K (`raw/evicted-with-warm.out`) | executables evicted, gate warms them | 4062 (3430 warming) | 17884 | 22457 |
| L (`raw/evicted-without-warm.out`) | executables evicted, no warming | 639 | 18854 | 23405 |

So the warm-up moves 3,430 ms out of the measured span and takes about 950 ms
off it. It is kept for that, and because the figure it reports says whether the
run found a resident target or a stale one. A `sync` between the build and the
warm-up was measured in the same position and dropped: it cost 1,898-5,512 ms on
a fresh target and changed the clock by nothing outside noise (26,808 ms without
it against 27,723 ms with it, both cold, both passing). Raw runs recorded while
that variant was in place report a `flushed in N ms` figure in their
`suite-build` line; the shipped step reports the build and the warm-up only.

## What is not a cause

- **Compile aftermath.** A forced full recompile on a warm target directory —
  `touch crates/jit/src/lib.rs`, 27,441 ms of clippy and 24,608 ms of build
  immediately before the suite — measured `test` at 18,054 ms, matching the warm
  run's 18,040 ms (`raw/recompiled-warm-tree.out`). Neither CPU heat nor the
  scheduler explains the fresh-target gap.
- **Writeback.** See the `sync` measurement above.

## Margin

On this host the fresh-target clock is 27,723 ms against a 30,000 ms threshold,
and the warm one is 22,571 ms. The remaining fresh-target cost is the ~4,300 ms
of fixture construction above, which is work the suite does rather than state the
machine happens to hold.
