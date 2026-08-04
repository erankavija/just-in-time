//! Shared test mechanism for deciding lock-contention properties on what the
//! lock did.
//!
//! A test about a lock is a test about ordering: who was admitted, who was
//! refused, and in what order. Elapsed time is how such a test usually reaches
//! that question, and it is the wrong thing to decide it on — a contender given
//! a fixed wait fails a loaded host rather than a broken lock, and a holder that
//! sleeps a fixed span hopes the contender arrived rather than establishing it.
//!
//! This module supplies the two pieces those tests need instead.
//!
//! [`Contenders`] is what the threads of one test record against: each writes
//! down that its wait for the lock expired, so a test can establish that
//! contention happened. [`Contenders::await_refusals`] then waits for the
//! contenders' own recorded progress rather than for a clock, and
//! [`admitted_when_reached`] puts a refused contender's question back to the
//! lock, so the answer the test reads is the lock's rather than the scheduler's.
//!
//! [`ProgressWatch`] carries the bound every one of those waits needs. It is
//! spent only while its subject is entirely still: any observed progress resets
//! it, so a run that keeps moving never reaches it however slowly the host runs
//! it, and a run that has stopped fails with what it last saw instead of
//! hanging.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// How long a contended lock may go without any progress at all before a test
/// waiting on it reports it stuck.
///
/// Generous on purpose: progress of any kind resets the watch, so a healthy run
/// never spends this however slowly the host schedules it, and a run that does
/// spend it has stopped rather than slowed. The alternative to a bound is an
/// unbounded wait, which turns a stuck lock into a continuous-integration job
/// that runs to its execution ceiling instead of failing.
pub(crate) const CONTENTION_STALL_LIMIT: Duration = Duration::from_secs(30);

/// Interval between observations of a subject a test is waiting on.
///
/// Short enough that the wait ends promptly once the subject moves, long enough
/// to leave the contenders the CPU while the waiting thread polls them.
const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// A bound a test's wait spends only while nothing it is waiting on progresses.
///
/// Each observation reports whether the subject moved. Movement restarts the
/// bound, so the watch measures the subject going quiet rather than this
/// thread's patience.
#[derive(Debug)]
pub(crate) struct ProgressWatch {
    progressed_at: Instant,
}

impl ProgressWatch {
    /// Start watching, treating the moment of construction as progress.
    pub(crate) fn new() -> Self {
        Self {
            progressed_at: Instant::now(),
        }
    }

    /// Record one observation and panic once the subject has been still for
    /// [`CONTENTION_STALL_LIMIT`].
    ///
    /// `progressed` is whether this observation found the subject moved since
    /// the last one. `describe_stall` renders the failure from how long the
    /// subject has been still, and is called only when the bound is reached.
    ///
    /// # Panics
    ///
    /// Panics with `describe_stall`'s message when no observation has found
    /// progress for [`CONTENTION_STALL_LIMIT`].
    pub(crate) fn observe(
        &mut self,
        progressed: bool,
        describe_stall: impl FnOnce(Duration) -> String,
    ) {
        if progressed {
            self.progressed_at = Instant::now();
            return;
        }
        let stalled_for = self.progressed_at.elapsed();
        assert!(
            stalled_for < CONTENTION_STALL_LIMIT,
            "{}",
            describe_stall(stalled_for)
        );
    }
}

/// The threads contending for one lock, and what they have achieved against it.
///
/// Shared between the threads of one test so each can see the others make
/// progress. A contender records a refusal when its wait for the lock expires,
/// which is the observable proof that it met the lock held — a test that needs
/// contention establishes it here rather than assuming a schedule produced it.
#[derive(Debug, Default)]
pub(crate) struct Contenders {
    refused: Mutex<HashSet<ThreadId>>,
}

impl Contenders {
    /// A record shared by the threads of one test.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record that the calling thread's wait for the lock expired.
    ///
    /// Returns whether this is the first refusal this contender has recorded,
    /// which is the progress a waiter watching the contenders is looking for.
    pub(crate) fn record_refusal(&self) -> bool {
        self.locked_refusals().insert(std::thread::current().id())
    }

    /// How many distinct contenders have had a wait for the lock expire.
    pub(crate) fn refused_count(&self) -> usize {
        self.locked_refusals().len()
    }

    /// Wait until `contender_count` distinct contenders have each been refused
    /// the lock.
    ///
    /// Returns on the contenders' own recorded progress rather than on a clock:
    /// as soon as they have all been refused, however long the host took to
    /// schedule them.
    ///
    /// # Panics
    ///
    /// Panics when no further contender is refused for
    /// [`CONTENTION_STALL_LIMIT`], which means the missing ones are not asking
    /// for the lock — a condition that will not arrive, reported rather than
    /// waited out.
    pub(crate) fn await_refusals(&self, contender_count: usize) {
        let mut watch = ProgressWatch::new();
        let mut refused = self.refused_count();
        while refused < contender_count {
            std::thread::sleep(PROGRESS_POLL_INTERVAL);
            let now_refused = self.refused_count();
            let progressed = now_refused != refused;
            refused = now_refused;
            watch.observe(progressed, |stalled_for| {
                format!(
                    "{refused} of {contender_count} contenders were refused the \
                     lock, and none of the rest asked for it in {stalled_for:?}"
                )
            });
        }
    }

    fn locked_refusals(&self) -> std::sync::MutexGuard<'_, HashSet<ThreadId>> {
        self.refused
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Repeat `attempt` until the lock decides, treating a refused wait as "this
/// contender has not reached the critical section yet".
///
/// An expired wait reports only that the caller was still queued when its wait
/// ran out, which is a fact about the host's scheduler rather than about the
/// lock. Asking again is what puts the question back to the lock, so load
/// changes how many attempts a contender makes and nothing else about the
/// answer it reads. `is_refusal` is what tells an expired wait apart from an
/// outcome the locked operation decided.
///
/// # Panics
///
/// Panics when this contender is refused for [`CONTENTION_STALL_LIMIT`] with no
/// other contender refused in the meantime: nothing is releasing the lock, and
/// the last refusal is reported rather than retried forever.
pub(crate) fn admitted_when_reached<T, E: std::fmt::Display>(
    contenders: &Contenders,
    subject: &str,
    mut attempt: impl FnMut() -> Result<T, E>,
    is_refusal: impl Fn(&E) -> bool,
) -> Result<T, E> {
    let mut watch = ProgressWatch::new();
    loop {
        match attempt() {
            Err(error) if is_refusal(&error) => {
                let progressed = contenders.record_refusal();
                watch.observe(progressed, |stalled_for| {
                    format!(
                        "no contender for {subject} was admitted or newly refused \
                         in {stalled_for:?} while this one waited for it, so \
                         nothing is releasing it; the last wait ended with: {error}"
                    )
                });
            }
            decided => return decided,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_observe_resets_the_bound_on_every_progressing_observation() {
        let mut watch = ProgressWatch::new();
        // Far more observations than the bound allows in stalled succession;
        // each reports progress, so none of them spends it.
        (0..10_000).for_each(|_| watch.observe(true, |_| unreachable!("progress was reported")));
    }

    #[test]
    fn test_await_refusals_returns_once_every_contender_has_been_refused() {
        let contenders = Contenders::new();
        let contender_count = 4;
        let threads: Vec<_> = (0..contender_count)
            .map(|_| {
                let contenders = Arc::clone(&contenders);
                std::thread::spawn(move || contenders.record_refusal())
            })
            .collect();

        contenders.await_refusals(contender_count);

        assert_eq!(contenders.refused_count(), contender_count);
        assert!(
            threads.into_iter().all(|thread| thread.join().unwrap()),
            "each contender's first refusal is reported as progress"
        );
    }

    #[test]
    fn test_record_refusal_counts_each_contender_once_however_often_it_asks() {
        let contenders = Contenders::new();
        let repeats = 5;
        (0..repeats).for_each(|_| {
            contenders.record_refusal();
        });

        assert_eq!(
            contenders.refused_count(),
            1,
            "one thread refused repeatedly is one contender, not {repeats}"
        );
    }

    #[test]
    fn test_admitted_when_reached_returns_the_first_answer_the_lock_gives() {
        let contenders = Contenders::new();
        let refusals_before_admission = 3;
        let attempts = AtomicUsize::new(0);

        let admitted = admitted_when_reached(
            &contenders,
            "a lock that admits after being asked again",
            || match attempts.fetch_add(1, Ordering::SeqCst) {
                n if n < refusals_before_admission => Err("refused"),
                _ => Ok("admitted"),
            },
            |error| *error == "refused",
        );

        assert_eq!(admitted, Ok("admitted"));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            refusals_before_admission + 1,
            "every refusal is retried and the first decision is returned"
        );
    }

    #[test]
    fn test_admitted_when_reached_returns_a_decision_that_is_not_a_refusal() {
        let contenders = Contenders::new();

        let decided = admitted_when_reached(
            &contenders,
            "a lock whose operation fails",
            || Err::<(), _>("the operation was rejected"),
            |error| *error == "refused",
        );

        assert_eq!(decided, Err("the operation was rejected"));
        assert_eq!(
            contenders.refused_count(),
            0,
            "a rejection is the lock's answer, not a wait that expired"
        );
    }
}
