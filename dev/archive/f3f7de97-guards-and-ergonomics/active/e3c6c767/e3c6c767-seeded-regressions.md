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

The exclusion primitive reports every attempt as acquired, so no path that
consults it excludes anybody:

```rust
pub fn try_lock_exclusive(&self, path: &Path) -> Result<Option<LockGuard>> {
    let file = self.open_or_create(path)?;

-   match Fs4FileExt::try_lock_exclusive(&file) {
+   match Ok::<bool, std::io::Error>(true) {
```

The seed goes at the primitive rather than at one caller of it. A seed confined
to `lock_exclusive` leaves `try_lock_exclusive` honest, and a test that asks the
lock directly would then still get the right answer from a lock that is broken
for everyone else.

## Result

```
CARGO_INCREMENTAL=0 cargo test -p jit --lib storage::lock

test storage::lock::tests::test_try_lock_non_blocking ... FAILED
test storage::lock::tests::test_lock_exclusive_refuses_every_contender_that_asks_while_the_lock_is_held ... FAILED
test storage::lock::tests::test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time ... FAILED

the lock this contender holds is granted to a second caller, so it excludes nobody
```

Three tests fail and all three assert exclusion; the rest of the module passes.
Each failure arrives through the assertion that owns the property, naming what
the lock did.

With the seed removed:

```
test result: ok. 20 passed; 0 failed; finished in 0.10s
```

## Why the exclusivity property is asked rather than sampled

`test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time` began by
counting contenders inside the critical section and asserting the peak was one.
That cannot establish exclusion. Eight threads each holding briefly do not
overlap by construction: a host free to run admitted contenders one after another
observes a peak of one whether the lock excludes or not, so a broken lock passes.
Holding the lock until every contender has been refused it fixes half of that —
all eight are demonstrably queued before any is admitted — but says nothing about
what happens after the release, where a serial schedule still hides the defect.

A peak that cannot be forced above one is not a test of exclusion. The holder
therefore asks the lock directly: while inside the critical section, an
independent acquisition of the same lock must fail. That answer depends on the
lock and on nothing else being scheduled, so it holds on any host, and it is what
the seed above makes fail.

The peak counters remain as a second observation, and the refusal test remains
the statement about contenders that arrive while the lock is held. Neither is
load-bearing for exclusion on its own.
