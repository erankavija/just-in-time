//! The interruption vocabulary a consumer injects a mutation session through.
//!
//! A point names one moment of a session: it opens, it checks its capture
//! against the live repository, and it applies its redo record. The record's own
//! steps stay named by [`RedoPoint`], so a publication interruption means here
//! what it means in the applier.

use crate::storage::redo_apply::RedoPoint;
use crate::storage::{TransactionFailureInjector, TransactionFailurePoint};
use std::sync::Arc;

/// One interruptible moment of a mutation session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MutationPoint {
    /// A session is opening, before it recovers or captures anything.
    SessionOpen,
    /// A held session's apply, before it checks its capture against the live
    /// repository.
    BeforeCommitCheck,
    /// One step of that session's redo-record application.
    Redo(RedoPoint),
}

/// Test seam for interrupting one named moment of a mutation session.
pub trait MutationInterruptionInjector: Send + Sync {
    /// Interrupt the session at one point, or let it proceed.
    fn check(&self, point: &MutationPoint) -> std::io::Result<()>;
}

/// Puts the transaction kernel's boundaries to a session injector as the
/// record-era moments they stand for, so a repointed injection fires while the
/// kernel is still the publisher.
///
/// Removed when the session commits through the redo record (jit:4319256e):
/// the session checks [`MutationPoint::SessionOpen`] and
/// [`MutationPoint::BeforeCommitCheck`] itself from there and the applier checks
/// its own [`RedoPoint`]s, leaving nothing to translate.
pub(in crate::storage) struct KernelInterruptions(Arc<dyn MutationInterruptionInjector>);

impl KernelInterruptions {
    /// Wrap `injector` as the kernel-vocabulary injector the session threads.
    pub(in crate::storage) fn new(injector: Arc<dyn MutationInterruptionInjector>) -> Arc<Self> {
        Arc::new(Self(injector))
    }
}

impl TransactionFailureInjector for KernelInterruptions {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        record_era_moment(point).map_or(Ok(()), |moment| self.0.check(&moment))
    }
}

/// The session moment a kernel boundary stands for, or `None` for a kernel step
/// the record protocol has no moment of its own for.
///
/// Each pairing is the one boundary a session crosses exactly once per moment:
/// external recovery precedes everything else a session open does;
/// read-set revalidation is the apply's comparison against the live repository;
/// control creation opens one publication with nothing durable yet written,
/// which is what [`RedoPoint::BeforeApply`] names once the record is the commit
/// point; and the post-commit boundary leaves the mutation published with only
/// cleanup outstanding, which is [`RedoPoint::AfterApply`].
fn record_era_moment(point: &TransactionFailurePoint) -> Option<MutationPoint> {
    match point {
        TransactionFailurePoint::RepositoryRecoveryExternal => Some(MutationPoint::SessionOpen),
        TransactionFailurePoint::RepositoryBeforeReadSetRevalidation => {
            Some(MutationPoint::BeforeCommitCheck)
        }
        TransactionFailurePoint::RepositoryBeforeControlCreation => {
            Some(MutationPoint::Redo(RedoPoint::BeforeApply))
        }
        TransactionFailurePoint::RepositoryAfterCommit => {
            Some(MutationPoint::Redo(RedoPoint::AfterApply))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records every moment put to it, failing at one selected moment.
    #[derive(Default)]
    struct Recorded {
        seen: Mutex<Vec<MutationPoint>>,
        fail_at: Option<MutationPoint>,
    }

    impl Recorded {
        fn failing_at(point: MutationPoint) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                fail_at: Some(point),
            })
        }

        fn seen(&self) -> Vec<MutationPoint> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl MutationInterruptionInjector for Recorded {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            self.seen.lock().unwrap().push(point.clone());
            if self.fail_at.as_ref() == Some(point) {
                return Err(std::io::Error::other("injected"));
            }
            Ok(())
        }
    }

    /// The kernel boundaries a session crosses, one sample per phase of a
    /// publication, including every boundary that stands for a session moment.
    fn kernel_boundaries() -> Vec<TransactionFailurePoint> {
        vec![
            TransactionFailurePoint::RepositoryRecoveryExternal,
            TransactionFailurePoint::RepositoryRecoveryInternal,
            TransactionFailurePoint::RepositoryBeforeReadSetRevalidation,
            TransactionFailurePoint::RepositoryBeforeControlCreation,
            TransactionFailurePoint::RepositoryCreateControl,
            TransactionFailurePoint::RepositoryPrepareAction { action: 0 },
            TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
            TransactionFailurePoint::RepositoryAfterTargetMutation { action: 0 },
            TransactionFailurePoint::RepositoryBeforeCommitDecision,
            TransactionFailurePoint::RepositoryAfterCommit,
            TransactionFailurePoint::RepositoryBeforeControlCleanup,
            TransactionFailurePoint::RepositoryCleanup,
        ]
    }

    #[test]
    fn test_record_era_moment_gives_each_session_moment_exactly_one_kernel_boundary() {
        let moments: Vec<MutationPoint> = kernel_boundaries()
            .iter()
            .filter_map(record_era_moment)
            .collect();

        assert_eq!(
            moments
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            moments.len(),
            "two kernel boundaries standing for one moment would fire a repointed injection twice"
        );
        assert_eq!(
            moments,
            vec![
                MutationPoint::SessionOpen,
                MutationPoint::BeforeCommitCheck,
                MutationPoint::Redo(RedoPoint::BeforeApply),
                MutationPoint::Redo(RedoPoint::AfterApply),
            ],
            "the moments must arrive in the order a publishing session crosses them"
        );
    }

    #[test]
    fn test_kernel_interruptions_puts_only_the_boundaries_that_stand_for_a_moment() {
        let injector = Arc::new(Recorded::default());
        let bridge = KernelInterruptions::new(injector.clone());

        for point in kernel_boundaries() {
            bridge.check(&point).expect("no moment is injected");
        }

        assert_eq!(
            injector.seen(),
            kernel_boundaries()
                .iter()
                .filter_map(record_era_moment)
                .collect::<Vec<_>>(),
            "a kernel step the record protocol has no moment for must reach no injector"
        );
    }

    #[test]
    fn test_kernel_interruptions_raises_the_injected_failure_at_its_moment() {
        let injector = Recorded::failing_at(MutationPoint::Redo(RedoPoint::BeforeApply));
        let bridge = KernelInterruptions::new(injector);

        assert!(bridge
            .check(&TransactionFailurePoint::RepositoryCreateControl)
            .is_ok());
        assert!(bridge
            .check(&TransactionFailurePoint::RepositoryBeforeControlCreation)
            .is_err());
    }
}
