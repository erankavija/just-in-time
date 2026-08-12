use std::env;

// Compile-time build metadata for the `jit` CLI, embedded as env vars read by
// `src/build_info.rs`.
//
// This script reads NOTHING ambient: no git state, no wall clock, no injected
// environment. PROFILE and TARGET are supplied by cargo itself, so two builds
// from identical sources are byte-for-byte reproducible and a git metadata
// change never makes an otherwise-unchanged rebuild non-fresh.
fn main() {
    emit("JIT_BUILD_PROFILE", env::var("PROFILE").ok());
    emit("JIT_BUILD_TARGET", env::var("TARGET").ok());
}

/// Emit `name` as a `rustc-env` value, substituting the `"unknown"` fallback
/// when cargo supplied nothing.
fn emit(name: &str, value: Option<String>) {
    println!(
        "cargo:rustc-env={name}={}",
        value.unwrap_or_else(|| "unknown".to_string())
    );
}
