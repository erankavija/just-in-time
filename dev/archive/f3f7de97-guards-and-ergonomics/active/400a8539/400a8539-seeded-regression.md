# Seeded regression for the startup observation (400a8539)

> **Diátaxis Type:** Reference (evidence record)

`start_server` reports what the spawned child did rather than what it had not
done yet after a fixed pause. This records the regression that reporting was run
against, so the demonstration lives in the repository.

Run by the execution lead on the merged tree at `main`. The seed is one edit,
run, then removed; the file is compared against
`git show HEAD:crates/jit/src/commands/serve.rs` afterwards rather than against
`git diff --quiet`, which reports only that the tree matches `HEAD` and not that
`HEAD` is right.

## The seed

The child's exit is never observed, so a server that dies during startup is
indistinguishable from one that is merely slow to answer:

```rust
fn exit_status(child: &mut Child) -> Result<Option<std::process::ExitStatus>> {
-   child.try_wait().context("Failed to check server startup")
+   let _ = child;
+   Ok(None)
}
```

## Result

```
CARGO_INCREMENTAL=0 cargo test -p jit --lib commands::serve

test commands::serve::tests::test_start_server_errors_when_child_exits_immediately ... FAILED
test commands::serve::tests::test_start_server_errors_when_child_exits_after_being_observed_running ... FAILED
test result: FAILED. 23 passed; 2 failed; finished in 35.00s
```

Both cases that assert a dying child is reported fail, and only those two: a
child that exits at once and a child that exits after being observed running are
each guarded, and the seed reaches both. The run takes the full startup bound
because a watch that cannot see an exit runs until it gives up, which is the
behaviour the seed describes.

With the seed removed:

```
test result: ok. 25 passed; 0 failed; finished in 2.00s
```

## What the suite does not distinguish, stated plainly

A second seed was run and is recorded here because it found a coverage limit
rather than a defect. Reading the child's exit **once**, at the start, instead of
on every round:

```rust
let mut looked_for_exit = false;
loop {
    if !looked_for_exit {
        looked_for_exit = true;
        if let Some(status) = exit_status(child)? { ... }
    }
```

fails nothing. Every case still passes, because `watch_startup` reads the exit
once more when the bound expires, so a child that dies mid-observation is still
reported with the right status — just at the bound instead of at once. The suite
is complete about *what* is reported and silent about *when*.

That silence is deliberate rather than an omission to fix. An assertion that the
watch stops on the round that finds the exit has to count rounds, and the number
of rounds depends on how quickly the child exits relative to the poll interval —
a schedule dependence, which is the defect class this container removes. Adding
one would trade a real property for a flaky test. The promptness of the report is
therefore held by the shape of `watch_startup`, which reads the exit at the top
of every round, and by review of that shape, not by a test.

## The probe's own race, and the test that holds it

Review found that answering the readiness probe and exiting are not exclusive: a
child can serve the probe and die before the parent finishes reading, and a start
that concluded on the probe alone then published a PID for a process already
gone — the stale record `REQ-02` forbids. `watch_startup` now reads the child's
exit once more after a positive probe, and reports the exit when it finds one.

`test_start_server_errors_when_the_child_answers_the_probe_and_then_exits` holds
it. The interleaving that is otherwise a race is made the only one the case can
take: the injected probe returns nothing until the announced child has reached
its post-run state, so its "serving" answer is true and stale together.

Waiting for that state is an observation rather than a clock. An exited child the
parent has not reaped is a zombie, so signalling it still succeeds and liveness
cannot detect it; its state letter in `/proc` reads `Z` exactly when it has run to
completion, which is the condition worth waiting for.

Removing the recheck fails this case and only this case:

```
test commands::serve::tests::test_start_server_errors_when_the_child_answers_the_probe_and_then_exits ... FAILED
test result: FAILED. 25 passed; 1 failed
```

One note on the helper, because it cost a confusing round. Polling that state
with `yield_now` starved the other tests in the module when the suite ran in
parallel, and two unrelated cases failed — including one that reported a spawn
failure, which reads like a defect in the code under test rather than in the
test's own scheduling. It polls with the startup interval instead.
