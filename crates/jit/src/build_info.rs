//! Compile-time build metadata for the `jit` CLI.
//!
//! The reported fields originate in [`build.rs`](../../build.rs), which reads
//! only what cargo supplies. Nothing here depends on git state or the wall
//! clock, so an ordinary build reports the same metadata as any other build of
//! the same sources.

use schemars::JsonSchema;
use serde::Serialize;

/// Version and build metadata reported by `jit version`.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct VersionInfo {
    /// Package name.
    pub package: &'static str,
    /// Crate package version.
    pub version: &'static str,
    /// Cargo build profile, such as `debug` or `release`.
    pub build_profile: &'static str,
    /// Cargo target triple.
    pub target: &'static str,
}

/// Concise version text used by Clap for `jit --version`.
pub const VERSION_TEXT: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (profile ",
    env!("JIT_BUILD_PROFILE"),
    ")"
);

/// Return compile-time version and build metadata.
pub fn version_info() -> VersionInfo {
    VersionInfo {
        package: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        build_profile: env!("JIT_BUILD_PROFILE"),
        target: env!("JIT_BUILD_TARGET"),
    }
}
