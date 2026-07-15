//! Compile-time build provenance for the `jit` CLI.
//!
//! Every provenance field originates in [`build.rs`](../../build.rs): it is
//! populated ONLY from the four explicit release-injection variables
//! (`JIT_BUILD_GIT_HASH`, `JIT_BUILD_GIT_SHORT_HASH`, `JIT_BUILD_GIT_DIRTY`,
//! `SOURCE_DATE_EPOCH`) and reports a documented fallback otherwise. An
//! ordinary build never reads ambient git or the wall clock, so its
//! provenance is `"unknown"`/`None` rather than the surrounding repository's
//! state (jit:5d862134). `scripts/install-jit.sh` supplies the real values at
//! install time.

use schemars::JsonSchema;
use serde::Serialize;

/// Version and build provenance reported by `jit version`.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct VersionInfo {
    /// Package name.
    pub package: &'static str,
    /// Crate package version.
    pub version: &'static str,
    /// Full Git commit hash from the injected `JIT_BUILD_GIT_HASH`, or
    /// `"unknown"` when the build injected no provenance.
    pub git_commit: &'static str,
    /// Short Git commit hash from the injected `JIT_BUILD_GIT_SHORT_HASH`, or
    /// `"unknown"` when the build injected no provenance.
    pub git_short_commit: &'static str,
    /// Whether the source tree was dirty at build time, from the injected
    /// `JIT_BUILD_GIT_DIRTY`. `None` when no dirty flag was injected.
    pub git_dirty: Option<bool>,
    /// Cargo build profile, such as `debug` or `release`.
    pub build_profile: &'static str,
    /// Build timestamp as a Unix epoch seconds string from the injected
    /// `SOURCE_DATE_EPOCH`, or `"unknown"` when none was injected. Never the
    /// wall clock.
    pub build_timestamp: &'static str,
    /// Cargo target triple.
    pub target: &'static str,
}

/// Concise version text used by Clap for `jit --version`.
pub const VERSION_TEXT: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (commit ",
    env!("JIT_GIT_SHORT_HASH"),
    ", dirty=",
    env!("JIT_GIT_DIRTY"),
    ", profile ",
    env!("JIT_BUILD_PROFILE"),
    ")"
);

/// Return compile-time version and provenance metadata.
pub fn version_info() -> VersionInfo {
    VersionInfo {
        package: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        git_commit: env!("JIT_GIT_HASH"),
        git_short_commit: env!("JIT_GIT_SHORT_HASH"),
        git_dirty: parse_dirty(env!("JIT_GIT_DIRTY")),
        build_profile: env!("JIT_BUILD_PROFILE"),
        build_timestamp: env!("JIT_BUILD_TIMESTAMP"),
        target: env!("JIT_BUILD_TARGET"),
    }
}

fn parse_dirty(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_dirty;

    #[test]
    fn test_parse_dirty_known_values() {
        assert_eq!(parse_dirty("true"), Some(true));
        assert_eq!(parse_dirty("false"), Some(false));
    }

    #[test]
    fn test_parse_dirty_unknown_values() {
        assert_eq!(parse_dirty("unknown"), None);
        assert_eq!(parse_dirty(""), None);
    }
}
