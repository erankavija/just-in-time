# Seeded-regression evidence — an unreachable wait reported rather than waited out (91cc038c)

> **Diátaxis Type:** Reference (measurement record)
> Two measurements, each in its own scratch worktree detached from `main` — the
> first at `36fc326c`, the second at `84bb42f3` after `code-review` found a
> third site — so the parent checkout carried neither seed nor reverted helper
> at any point. Both worktrees were removed after their runs.

## The seeds

### Reentrancy: the repository write lock stops excluding

The same seed `4c700c80`'s survey used for its exclusion property.
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

### File lock: the polling acquisition admits whoever asks

`FileLocker::lock_exclusive`'s polling loop was made to return a guard whether or
not the underlying try succeeded, by widening `Ok(true) =>` to `Ok(_) =>`. The
test that drives `FileLocker` directly needs this one, because the reentrancy
seed above is in a different lock and leaves it passing.

## What each run measured

Each test was run against its seed twice: once with the unreachable-target
report, and once with the code as it stood before that report existed. Only that
code differed between the halves — the worktree's `contention_probe.rs`,
`claim_coordinator.rs` and `lock.rs` were checked out from the preceding commit
for the second half, with the seed left in place. Each figure is the test
harness's own `finished in`, not a wall-clock reading taken around it.

| Test | Seed | Before | After |
| --- | --- | --- | --- |
| `storage::repo_lock::tests::test_second_thread_waits_for_the_holder` | reentrancy | FAILED in 30.00 s | FAILED in 0.00 s |
| `storage::repository_state_store::tests::test_session_disjoint_sessions_serialize_on_worktree_bootstrap` | reentrancy | FAILED in 30.00 s | FAILED in 0.00 s |
| `storage::lock::tests::test_lock_exclusive_refuses_every_contender_that_asks_while_the_lock_is_held` | file lock | FAILED in 30.00 s | FAILED in 0.00 s |
| `storage::lock::tests::test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time` | reentrancy | ok in 0.03 s | ok in 0.03 s |

The last row is the control. It contends for `FileLocker`, which the reentrancy
seed does not touch, so it passes in both halves and shows the change costs a
passing run nothing.

The third row needed a second seed and a second measurement, because that test
drives `FileLocker` directly rather than through `admitted_when_reached`. It was
found by `code-review` rather than by this document's first pass: recording only
expired waits left a contender the lock wrongly admitted recording nothing at
all, so that site alone still waited the bound out after the first fix. The seed
is `FileLocker::lock_exclusive`'s polling acquisition returning a guard whether
or not the underlying try succeeded — `Ok(true) =>` widened to `Ok(_) =>` — and
the run before the record reports:

```
0 of 4 contenders were refused the lock, and nothing the set does moved in 30.00011839s
```

against, after it:

```
the lock let 2 of 4 contenders through without ever refusing them, so the 0
refusals recorded are all there will ever be
```

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
  panicked at crates/jit/src/storage/contention_probe.rs:209:
  the lock let 1 of 1 contenders through without ever refusing them, so the 0
  refusals recorded are all there will ever be
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
