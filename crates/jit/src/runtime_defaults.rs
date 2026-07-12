//! Built-in runtime coordination and recovery defaults.
//!
//! These constants are the single source of truth for the operational defaults
//! that govern multi-agent lease coordination and recovery cleanup: the lease
//! heartbeat cadence, the file-lock acquisition timeout and poll interval, the
//! orphaned temp-file cleanup threshold, and the default claim lease TTL. Call
//! sites reference the constants here instead of inline literals, so each
//! default value has one definition.
//!
//! [`render_reference_markdown`] projects these constants into the committed
//! reference `docs/reference/runtime-defaults.md`; a conformance test asserts the
//! committed copy matches the projection, so changing a default without
//! refreshing the reference fails the test suite (`@/inv/single-source-prose`).

/// Default value, in seconds, of the agent `heartbeat_interval` configuration
/// setting — the recommended cadence at which an agent sends `jit claim
/// heartbeat` to keep an indefinite (TTL=0) lease alive. jit does not itself run
/// a heartbeat loop: `jit claim heartbeat` records a single beat on demand.
/// Lease staleness is governed separately — an indefinite lease is marked stale
/// only after the claim staleness threshold (1 hour) without a beat.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use jit::runtime_defaults::HEARTBEAT_INTERVAL_SECS;
///
/// let cadence = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
/// assert_eq!(cadence.as_secs(), HEARTBEAT_INTERVAL_SECS);
/// ```
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Maximum time, in seconds, a writer waits for a `.jit` file lock or the
/// repository write lock before failing. The `JIT_LOCK_TIMEOUT` environment
/// variable overrides this default (whole seconds).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use jit::runtime_defaults::LOCK_TIMEOUT_SECS;
///
/// let timeout = Duration::from_secs(LOCK_TIMEOUT_SECS);
/// assert_eq!(timeout.as_secs(), LOCK_TIMEOUT_SECS);
/// ```
pub const LOCK_TIMEOUT_SECS: u64 = 5;

/// Interval, in milliseconds, between successive attempts while blocking on a
/// contended file lock.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use jit::runtime_defaults::LOCK_POLL_INTERVAL_MS;
///
/// let poll = Duration::from_millis(LOCK_POLL_INTERVAL_MS);
/// assert_eq!(poll.as_millis() as u64, LOCK_POLL_INTERVAL_MS);
/// ```
pub const LOCK_POLL_INTERVAL_MS: u64 = 10;

/// Minimum age, in seconds, at which `cleanup_orphaned_temp_files` sweeps an
/// orphaned `*.tmp` file (1 hour). Passed by both callers: the `jit recover`
/// command and `ClaimCoordinator::startup_recovery`.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use jit::runtime_defaults::TEMP_CLEANUP_THRESHOLD_SECS;
///
/// let threshold = Duration::from_secs(TEMP_CLEANUP_THRESHOLD_SECS);
/// assert_eq!(threshold.as_secs(), TEMP_CLEANUP_THRESHOLD_SECS);
/// ```
pub const TEMP_CLEANUP_THRESHOLD_SECS: u64 = 3600;

/// Default time-to-live, in seconds, for a lease created by `jit claim acquire`,
/// and the default extension applied by `jit claim renew` (10 minutes).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use jit::runtime_defaults::CLAIM_TTL_SECS;
///
/// let ttl = Duration::from_secs(CLAIM_TTL_SECS);
/// assert_eq!(ttl.as_secs(), CLAIM_TTL_SECS);
/// ```
pub const CLAIM_TTL_SECS: u64 = 600;

/// Repo-relative path of the committed reference that projects these defaults.
///
/// # Examples
///
/// ```
/// use jit::runtime_defaults::REFERENCE_PATH;
///
/// assert!(REFERENCE_PATH.ends_with("runtime-defaults.md"));
/// ```
pub const REFERENCE_PATH: &str = "docs/reference/runtime-defaults.md";

/// Render the runtime coordination defaults as the committed markdown reference.
///
/// The returned string is the full contents of
/// [`REFERENCE_PATH`]: every value derives from the constants in this module, so
/// the projection cannot drift from the values the code uses. The conformance
/// test in this module asserts the committed file equals this output.
///
/// # Examples
///
/// ```
/// use jit::runtime_defaults::{render_reference_markdown, CLAIM_TTL_SECS};
///
/// let doc = render_reference_markdown();
/// assert!(doc.starts_with("<!--"));
/// assert!(doc.contains("# Runtime Coordination Defaults"));
/// // The rendered table carries each default's value with its unit.
/// assert!(doc.contains(&format!("{CLAIM_TTL_SECS} seconds")));
/// ```
pub fn render_reference_markdown() -> String {
    // (label, value-with-unit, operational scope). Values format the constants
    // so the table is a pure projection of the source of truth above.
    let rows = [
        (
            "Heartbeat interval",
            format!("{HEARTBEAT_INTERVAL_SECS} seconds"),
            "Default value of the agent `heartbeat_interval` setting — the recommended cadence at which an agent sends `jit claim heartbeat` to keep an indefinite (TTL=0) lease alive. jit does not run a heartbeat loop itself; the command records a single beat on demand. Lease staleness is governed separately: an indefinite lease is marked stale only after the claim staleness threshold (1 hour) without a beat, not after this interval. Finite leases expire on their own TTL instead (see Claim lease TTL).",
        ),
        (
            "Lock acquisition timeout",
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            "Default timeout to acquire a file lock before failing. The `JsonFileStorage` write lock resolves its timeout from the `JIT_LOCK_TIMEOUT` environment variable when set, falling back to this default; other file locks, including the claim-coordination locks, use this default and ignore the environment variable.",
        ),
        (
            "Lock poll interval",
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            "Wait between successive attempts while blocking on a contended file lock.",
        ),
        (
            "Temp-file cleanup threshold",
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            "Minimum age at which `cleanup_orphaned_temp_files` sweeps an orphaned `*.tmp` file. Both callers pass this threshold: the `jit recover` command and `ClaimCoordinator::startup_recovery`.",
        ),
        (
            "Claim lease TTL",
            format!("{CLAIM_TTL_SECS} seconds"),
            "Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`.",
        ),
    ];

    let table = rows
        .iter()
        .map(|(name, value, scope)| format!("| {name} | {value} | {scope} |"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->\n\
         \n\
         # Runtime Coordination Defaults\n\
         \n\
         Built-in defaults for multi-agent lease coordination and recovery cleanup. This\n\
         reference is generated from the `crates/jit/src/runtime_defaults.rs`\n\
         module, which defines these values.\n\
         \n\
         | Default | Value | Scope |\n\
         | --- | --- | --- |\n\
         {table}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REFERENCE_PATH)
    }

    /// REQ-03 conformance: the committed reference must equal the projection of
    /// the current constants. Editing a default without refreshing the reference
    /// (or vice versa) fails here.
    #[test]
    fn test_committed_reference_matches_projection() {
        let committed = std::fs::read_to_string(reference_path())
            .expect("committed runtime-defaults reference should exist");
        assert_eq!(
            committed,
            render_reference_markdown(),
            "{REFERENCE_PATH} is stale — regenerate it from `crate::runtime_defaults` \
             (run: cargo test -p jit runtime_defaults -- --ignored regenerate)"
        );
    }

    /// Regenerate the committed reference from the constants. Ignored by default;
    /// run explicitly after changing a default:
    ///   cargo test -p jit runtime_defaults -- --ignored regenerate
    ///
    /// Writes via the temp-file + atomic-rename pattern (`@/inv/atomic-writes`).
    #[test]
    #[ignore = "writes the committed reference; run explicitly to regenerate"]
    fn test_regenerate_reference_writes_committed_doc() {
        let path = reference_path();
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, render_reference_markdown())
            .expect("should write the runtime-defaults temp file");
        std::fs::rename(&tmp, &path)
            .expect("should atomically replace the runtime-defaults reference");
    }

    #[test]
    fn test_render_lists_every_default_value() {
        let doc = render_reference_markdown();
        for expected in [
            format!("{HEARTBEAT_INTERVAL_SECS} seconds"),
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            format!("{CLAIM_TTL_SECS} seconds"),
        ] {
            assert!(doc.contains(&expected), "missing {expected} in:\n{doc}");
        }
    }
}
