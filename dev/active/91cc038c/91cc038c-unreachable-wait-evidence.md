# Seeded-regression evidence — an unreachable wait reported rather than waited out (91cc038c)

> **Diátaxis Type:** Reference (measurement record)
> Measured in a scratch worktree at `36fc326c`, detached from `main`, so the
> parent checkout carried neither the seed nor the reverted helper at any point.
> The worktree was removed after the runs.

## The seed

The same one `4c700c80`'s survey used for its exclusion property.
`RepoWriteLock::acquire`'s reentrancy check at
`crates/jit/src/storage/repo_lock.rs:232` was changed from

```rust
if state.owner == Some(me) {
```

to

```rust
if state.owner.is_some() {
```

so a second thread is admitted to a lock another thread holds. The lock stops
excluding, which is the defect the tests in question exist to catch.

## What each run measured

Three tests were run against the seed, once with the unreachable-target report
and once with the helper as it stood before it. Only the helper differed between
the two halves: the worktree's `contention_probe.rs` and `claim_coordinator.rs`
were checked out from the preceding commit for the second half, with the seed
left in place. Each figure is the test harness's own `finished in`, not a
wall-clock reading taken around it.

| Test | Before | After |
| --- | --- | --- |
| `storage::repo_lock::tests::test_second_thread_waits_for_the_holder` | FAILED in 30.00 s | FAILED in 0.00 s |
| `storage::repository_state_store::tests::test_session_disjoint_sessions_serialize_on_worktree_bootstrap` | FAILED in 30.00 s | FAILED in 0.00 s |
| `storage::lock::tests::test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time` | ok in 0.03 s | ok in 0.03 s |

The third row is the control. It contends for `FileLocker`, which this seed does
not touch, so it passes in both halves and shows the change costs a passing run
nothing.

## What the failure says

Before, the waiting thread reported the bound it had spent:

```
thread '<unnamed>' panicked at crates/jit/src/storage/repo_lock.rs:422:
  a second thread must not enter while the lock is held
thread 'storage::repo_lock::tests::test_second_thread_waits_for_the_holder'
  panicked at crates/jit/src/storage/contention_probe.rs:90:
  0 of 1 contenders were refused the lock, and none of the rest asked for it in 30.00039409s
```

The first line is the real finding, from the contender the lock wrongly admitted.
The second is the main thread thirty seconds later, reporting a condition — "none
of the rest asked for it" — that was neither true nor the cause.

After, the main thread reports the cause at the moment it becomes knowable:

```
thread '<unnamed>' panicked at crates/jit/src/storage/repo_lock.rs:422:
  a second thread must not enter while the lock is held
thread 'storage::repo_lock::tests::test_second_thread_waits_for_the_holder'
  panicked at crates/jit/src/storage/contention_probe.rs:203:
  the lock admitted 1 contender it had to refuse, so the 0 of 1 refusals recorded
  are all there will ever be
```

## Why the condition is decidable at that moment

A contender records its refusal before it asks again, so one that reaches the
critical section with no refusal to its name never met the lock held. While a
test holds the lock — which every caller of `await_refusals` in this workspace
does — that can only be the lock admitting a contender it had to refuse. The
contender is now past the lock, so the refusal the waiter is counting will never
arrive from it. Nothing further needs to be waited for to know the answer.

## What still reaches the bound

The bound is not removed, and three conditions still reach it. Two are the
defects it exists to report:

- A contender that never asks for the lock at all, so no refusal and no
  admission is ever recorded.
- A holder that never releases, with the contender refused over and over and no
  other contender moving.

The third is the exposure this container carries knowingly: a host that starves
every thread of the contended set for thirty consecutive seconds fails the test
although the lock is correct. `@/issue/f3f7de97/requirement/REQ-03`,
`@/issue/4c700c80/requirement/REQ-01` and `@/issue/57675b68/requirement/REQ-01`
each state the terms on which that is accepted, and the alternative — an
unbounded wait — is the failure mode `@/issue/f3f7de97/requirement/REQ-08`
closes.

What this change removes from the bound's reach is the fourth condition, which
was neither of those: an outcome the lock had already decided, waited out for
thirty seconds before being reported under the wrong name.
