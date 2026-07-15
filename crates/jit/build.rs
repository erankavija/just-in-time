use std::env;

// Build provenance for the `jit` CLI, embedded as compile-time env vars read by
// `src/build_info.rs`.
//
// Ordinary builds (plain `cargo build`/`cargo test`) MUST NOT depend on Git
// metadata or the wall clock: watching `.git/index`, `HEAD`, or refs, or
// stamping the current time, makes an otherwise-unchanged rebuild non-fresh
// whenever git state moves — staging or committing unchanged sources then
// relinks every test target (jit:5d862134). So this script reads NOTHING
// ambient. Provenance enters ONLY through four explicit environment variables;
// absent them, each field is a stable documented fallback. This keeps two
// builds from identical sources and identical explicit environment byte-for-byte
// reproducible regardless of the surrounding git repository (REQ-01), and lets
// a metadata-only change leave the build untouched (REQ-02).
//
// Releases inject real provenance through the same four variables (see
// `scripts/install-jit.sh`); each is declared `rerun-if-env-changed`, so
// repeating a build with the same values reproduces the same provenance and
// changing any value invalidates the build output (REQ-03/REQ-04). A build with
// no injected metadata still succeeds and reports the fallbacks rather than a
// wall-clock timestamp (REQ-05).
fn main() {
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_HASH");
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_SHORT_HASH");
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_DIRTY");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    // Documented fallbacks for an ordinary build with no injected provenance:
    // the commit fields and the dirty flag report "unknown", and the build
    // timestamp reports "unknown" rather than the wall clock (REQ-05).
    emit("JIT_GIT_HASH", env_override("JIT_BUILD_GIT_HASH"));
    emit(
        "JIT_GIT_SHORT_HASH",
        env_override("JIT_BUILD_GIT_SHORT_HASH"),
    );
    emit("JIT_GIT_DIRTY", env_override("JIT_BUILD_GIT_DIRTY"));
    emit("JIT_BUILD_TIMESTAMP", env_override("SOURCE_DATE_EPOCH"));

    // PROFILE and TARGET are supplied by cargo, not git or the clock, so they
    // are stable inputs and safe to embed directly.
    emit("JIT_BUILD_PROFILE", env::var("PROFILE").ok());
    emit("JIT_BUILD_TARGET", env::var("TARGET").ok());
}

/// Read an injected provenance variable, treating blank values as absent so an
/// empty `SOURCE_DATE_EPOCH=` behaves like no injection at all.
fn env_override(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Emit `name` as a `rustc-env` value, substituting the `"unknown"` fallback
/// when no provenance was injected.
fn emit(name: &str, value: Option<String>) {
    println!(
        "cargo:rustc-env={name}={}",
        value.unwrap_or_else(|| "unknown".to_string())
    );
}
