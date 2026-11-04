//! The interruption vocabulary a consumer injects a mutation session through.
//!
//! A point names one moment of a session: it opens, it checks its capture
//! against the live repository, and it applies its redo record. The record's own
//! steps stay named by [`RedoPoint`], so a publication interruption means here
//! what it means in the applier.

use crate::storage::redo_apply::{RedoInterruptionInjector, RedoPoint};
use std::sync::Arc;

/// One interruptible moment of a mutation session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MutationPoint {
    /// A session is opening, before it recovers or captures anything.
    SessionOpen,
    /// A lock-free read session is opening, before it observes commits.
    ReadSessionOpen,
    /// A held session's apply, before it checks its capture against the live
    /// repository.
    BeforeCommitCheck,
    /// An absent data root's staged scaffold, before the no-replace rename
    /// publishes it. The record its stage carries is already durable here, so
    /// this is the moment the publication becomes irreversible.
    BeforeRootPublication,
    /// One step of that session's redo-record application.
    Redo(RedoPoint),
}

/// Test seam for interrupting one named moment of a mutation session.
pub trait MutationInterruptionInjector: Send + Sync {
    /// Interrupt the session at one point, or let it proceed.
    fn check(&self, point: &MutationPoint) -> std::io::Result<()>;
}

/// Production injector: every moment proceeds.
#[derive(Debug, Default)]
pub struct NoMutationInterruptions;

impl MutationInterruptionInjector for NoMutationInterruptions {
    fn check(&self, _point: &MutationPoint) -> std::io::Result<()> {
        Ok(())
    }
}

/// Puts a record application's own points to the session injector that owns
/// them, so a session's applications and the moments around them are injected
/// through one vocabulary.
pub(in crate::storage) struct SessionRedoInterruptions(Arc<dyn MutationInterruptionInjector>);

impl SessionRedoInterruptions {
    /// Hold `injector` as the owner of every point an application reaches
    /// through this adapter.
    pub(in crate::storage) fn new(injector: Arc<dyn MutationInterruptionInjector>) -> Arc<Self> {
        Arc::new(Self(injector))
    }
}

impl RedoInterruptionInjector for SessionRedoInterruptions {
    fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
        self.0.check(&MutationPoint::Redo(point.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<MutationPoint>>);

    impl MutationInterruptionInjector for Recorded {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            self.0.lock().unwrap().push(point.clone());
            Err(std::io::Error::other("injected"))
        }
    }

    #[test]
    fn test_session_redo_interruptions_names_the_point_the_application_reached() {
        let injector = Arc::new(Recorded::default());
        let adapter = SessionRedoInterruptions::new(injector.clone());

        assert!(adapter.check(&RedoPoint::BeforePut { index: 2 }).is_err());

        assert_eq!(
            injector.0.lock().unwrap().clone(),
            vec![MutationPoint::Redo(RedoPoint::BeforePut { index: 2 })],
            "the adapter must put the application's own point to the session injector"
        );
    }
}
