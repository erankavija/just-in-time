//! Test mode disables the git guards of a live repository: the main-history
//! check and the claims-index validation.
//!
//! In-process tests enable it through [`enable`], which never touches the
//! process environment; spawned binaries receive `JIT_TEST_MODE` per `Command`.

#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(test, feature = "test-support"))]
static ENABLED: AtomicBool = AtomicBool::new(false);

/// True when the process runs in test mode.
pub fn is_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    std::env::var_os("JIT_TEST_MODE").is_some()
}

/// Enable test mode for the rest of the process.
#[cfg(any(test, feature = "test-support"))]
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}
