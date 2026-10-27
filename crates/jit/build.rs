use std::env;
use std::path::PathBuf;

#[path = "src/crate_source_digest.rs"]
mod crate_source_digest;

// Compile-time build metadata for the `jit` CLI, embedded as env vars read by
// `src/build_info.rs`, plus the crate source digest that keys the
// profiled-repository test fixture (`src/test_utils.rs`).
//
// This script reads nothing ambient: no git state, no wall clock, no injected
// environment. PROFILE and TARGET are supplied by cargo itself and the digest
// covers only the crate's own source, so two builds from identical sources are
// byte-for-byte reproducible and a git metadata change never makes an
// otherwise-unchanged rebuild non-fresh.
fn main() {
    emit("JIT_BUILD_PROFILE", env::var("PROFILE").ok());
    emit("JIT_BUILD_TARGET", env::var("TARGET").ok());

    let crate_root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    crate_source_digest::CRATE_SOURCE_ROOTS
        .iter()
        .for_each(|root| println!("cargo:rerun-if-changed={root}"));
    let digest = crate_source_digest::crate_source_sha256(
        &crate_root,
        &crate_source_digest::CRATE_SOURCE_ROOTS,
    )
    .expect("hash the jit crate source");
    println!("cargo:rustc-env=JIT_CRATE_SOURCE_SHA256={digest}");
}

/// Emit `name` as a `rustc-env` value, substituting the `"unknown"` fallback
/// when cargo supplied nothing.
fn emit(name: &str, value: Option<String>) {
    println!(
        "cargo:rustc-env={name}={}",
        value.unwrap_or_else(|| "unknown".to_string())
    );
}
