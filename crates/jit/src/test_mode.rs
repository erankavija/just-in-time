//! Test mode disables the claims-index validation of a live repository. Spawned
//! binaries receive `JIT_TEST_MODE` per `Command`.

/// True when the process runs in test mode.
pub fn is_enabled() -> bool {
    std::env::var_os("JIT_TEST_MODE").is_some()
}
