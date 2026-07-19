//! Thread-scoped guard-order enforcement for claim/repository serialization.
//!
//! Claim flows must acquire in the fixed order coordinator → bootstrap →
//! repository → events. The store never acquires the claim coordinator, and a
//! holder of a repository mutation session must never then enter coordination
//! (reverse acquisition). This module tracks, per thread, whether a repository
//! mutation session is held so [`CoordinationOrderGuard::enter`] can reject the
//! reverse order with a typed error before any lock is taken.
//!
//! The order is a same-thread invariant: a claim holds the coordinator and then
//! opens the session on one thread; a violation is that same thread trying to
//! coordinate while already holding the session. Thread scoping keeps parallel
//! tests and unrelated worktrees independent.

use std::cell::Cell;

thread_local! {
    /// Depth of repository mutation sessions held on the current thread.
    static REPOSITORY_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A repository mutation session was held while entering claim coordination.
#[derive(Debug, thiserror::Error)]
#[error(
    "claim coordination cannot be entered while a repository mutation session is held on this thread \
     (required order: coordinator -> bootstrap -> repository -> events)"
)]
pub struct ReverseAcquisitionError;

/// Marks a repository mutation session as held on the current thread for its
/// lifetime. Dropped when the session closes.
#[must_use = "the guard must be held for the session lifetime"]
pub struct RepositoryOrderGuard(());

impl RepositoryOrderGuard {
    /// Enter a repository mutation session on this thread.
    pub fn enter() -> Self {
        REPOSITORY_DEPTH.with(|depth| depth.set(depth.get() + 1));
        Self(())
    }
}

impl Drop for RepositoryOrderGuard {
    fn drop(&mut self) {
        REPOSITORY_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Marks claim coordination as in progress on the current thread.
///
/// [`CoordinationOrderGuard::enter`] fails when a [`RepositoryOrderGuard`] is
/// held, enforcing that coordination is always the outermost guard.
#[must_use = "the guard must be held for the coordinated operation"]
pub struct CoordinationOrderGuard(());

impl CoordinationOrderGuard {
    /// Enter claim coordination, rejecting reverse acquisition.
    pub fn enter() -> Result<Self, ReverseAcquisitionError> {
        if REPOSITORY_DEPTH.with(Cell::get) > 0 {
            return Err(ReverseAcquisitionError);
        }
        Ok(Self(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordination_allowed_without_repository_session() {
        assert!(CoordinationOrderGuard::enter().is_ok());
    }

    #[test]
    fn test_coordination_rejected_while_repository_session_held() {
        let _session = RepositoryOrderGuard::enter();
        assert!(CoordinationOrderGuard::enter().is_err());
    }

    #[test]
    fn test_coordination_allowed_again_after_session_drops() {
        {
            let _session = RepositoryOrderGuard::enter();
            assert!(CoordinationOrderGuard::enter().is_err());
        }
        assert!(CoordinationOrderGuard::enter().is_ok());
    }

    #[test]
    fn test_coordination_then_session_is_allowed_order() {
        // coordinator -> repository is the correct order and must be permitted.
        let _coord = CoordinationOrderGuard::enter().unwrap();
        let _session = RepositoryOrderGuard::enter();
    }
}
