# Seeded regressions for the lock-contention properties (e3c6c767)

> **Diátaxis Type:** Reference (evidence record)

A property restated to ignore the clock is worth restating only while it can
still fail. This records the regression each restated property was run against,
so the demonstration lives in the repository rather than in a report.

Run by the execution lead on the merged tree at `main`, against
`crates/jit/src/storage/lock.rs` as it stands after this issue and `4c700c80`.
Each seed is one edit, run, then removed; the file is compared against
`git show HEAD:crates/jit/src/storage/lock.rs` afterwards rather than against
`git diff --quiet`, which reports only that the tree matches `HEAD` and not that
`HEAD` is right.

## The seed

`FileLocker::lock_exclusive` returns its guard immediately, so the lock grants
itself to every caller and excludes nobody:

```rust
pub fn lock_exclusive(&self, path: &Path) -> Result<LockGuard> {
    let file = self.open_or_create(path)?;
    if true {
        return Ok(LockGuard::new(file, path.to_path_buf()));
    }
    // the polling acquisition below is unreachable
```

`try_lock_exclusive` is untouched, so a test that takes the lock for itself still
gets it and only the contenders' path is broken. That is exclusion genuinely
breaking rather than the whole lock disappearing.

## Result

```
CARGO_INCREMENTAL=0 cargo test -p jit --lib storage::lock

test storage::lock::tests::test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time ... FAILED
test storage::lock::tests::test_lock_exclusive_refuses_every_contender_that_asks_while_the_lock_is_held ... FAILED
test result: FAILED. 18 passed; 2 failed; finished in 30.00s
```

Both restated properties fail, and no other test in the module does — the two
that assert exclusion are exactly the two the seed reaches.

With the seed removed:

```
test result: ok. 20 passed; 0 failed; finished in 0.10s
```

## How they fail, stated plainly

Both fail through `contention_probe.rs`'s stall bound rather than through their
own assertions, and the run takes the bound's full 30 seconds. The reason is
structural: a lock that admits every caller produces no refusals, so
`Contenders::await_refusals` never reaches its count and reports that nothing
progressed. The peak-holders assertion is never evaluated, because the test does
not get past the wait that establishes contention.

This satisfies the criterion — both properties fail when exclusion genuinely
breaks — while the message a reader gets names the wait rather than the defect.
A reader who seeds this and reads "no contender made progress" has to work out
that the cause is the opposite: every contender made progress, none was refused,
and the lock excluded nobody. Recorded rather than left for the next reader to
rediscover.

## Why the exclusivity property needs the lock held first

`test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time` counts the
contenders inside the critical section and asserts the peak is one. Eight threads
each holding for ten milliseconds do not establish that on their own: a host free
to run them one after another holds the peak at one whether the lock excludes or
not, so a broken lock passes.

The test therefore takes the lock before spawning them and holds it until every
contender has recorded being refused it. All eight are demonstrably queued before
any is admitted, so a lock that stops excluding admits several at once. The
overlap the assertion measures is produced rather than hoped for, which is the
same construction `57675b68` used for the claim coordinator and `4c700c80`
extracted into `contention_probe`.
