# Seeded regression for the drain-outcome property (15afa1f8)

> **Diátaxis Type:** Reference (evidence record)

The restated drain-outcome case decides on which branch the shutdown sequence
took rather than on the host completing a connection inside a 50-millisecond
window. This records the regression it was run against, on the merged tree, so
the demonstration lives in the repository.

Run by the execution lead against `main` after the branch merged. The seed is one
edit, run, then removed; the file is compared against
`git show HEAD:crates/server/src/shutdown.rs` afterwards rather than against
`git diff --quiet`.

## The seed

The boundary arm no longer reports a zero connection count as a voluntary drain,
so a connection that finished inside the last sampling interval is retired as a
forced close:

```rust
() = &mut boundary => {
    let connections = handle.connection_count();
-   if connections == 0 {
-       return DrainOutcome::Drained;
-   }
    handle.shutdown();
    return DrainOutcome::ForcedClosed { connections };
}
```

## Result

```
CARGO_INCREMENTAL=0 cargo test -p jit-server --lib shutdown::

test shutdown::tests::test_drain_connections_reports_drained_when_the_boundary_finds_the_survivor_retired ... FAILED
  left: ForcedClosed { connections: 0 }
 right: Drained
test result: FAILED. 5 passed; 1 failed; finished in 0.31s
```

Exactly one case fails, and it is the restated one, so it is the sole guard for
the boundary's zero-count branch. The other five shutdown cases pass under the
seed, including the force-close case — which is the point of keeping both: they
distinguish a connection that finished within the deadline from one retired at
it, and only one of the two is sensitive to this defect.

With the seed removed:

```
test result: ok. 6 passed; 0 failed; finished in 0.31s
```

The failure arrives in 0.31 seconds through the assertion itself, naming the
outcome the sequence produced against the one it should have.

## Why a paused clock was rejected

`#[tokio::test(start_paused = true)]` is the cheaper restatement and it does not
hold here. Under a paused runtime tokio advances virtual time whenever no task is
runnable, and the time driver's wake flag does not account for I/O readiness, so
the completion this case drives over a real loopback socket gets one zero-timeout
poll and no real time at all. The 50-millisecond budget becomes a requirement
that the kernel has *already* delivered readiness — a tighter dependence on the
host than the one being removed, not a looser one.

Established rather than assumed: with a paused clock and a ten-millisecond
real-time delay before the completion — well inside the real clock's original
margin — the case fails; with the real clock and the same delay, it passes.

One trap is worth recording because it produces a false clearance. Delivering
that delay through `spawn_blocking` makes the paused case pass, because a
blocking task inhibits auto-advance for its lifetime and so suppresses the very
mechanism under test. The delay has to arrive from a thread the runtime does not
own.

Widening the margin was not evaluated: a margin chosen to be long enough is the
defect class this container removes.
