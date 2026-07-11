//! Built-in runtime coordination and recovery defaults.
//!
//! These constants are the single source of truth for the operational defaults
//! that govern multi-agent coordination and startup recovery: the auto-heartbeat
//! cadence, the file-lock acquisition timeout and poll interval, the orphaned
//! temp-file cleanup threshold, and the default claim lease TTL. Each production
//! call site references the constant here instead of an inline literal, so the
//! value lives in exactly one place.
//!
//! [`render_reference_markdown`] projects these constants into the committed
//! reference `docs/reference/runtime-defaults.md`; a conformance test asserts the
//! committed copy matches the projection, so changing a default without
//! refreshing the reference fails the test suite (`@/inv/single-source-prose`).

/// Cadence, in seconds, at which the optional auto-heartbeat daemon renews an
/// indefinite (TTL=0) lease.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Maximum time, in seconds, a writer waits for a `.jit` file lock or the
/// repository write lock before failing. The `JIT_LOCK_TIMEOUT` environment
/// variable overrides this default (whole seconds).
pub const LOCK_TIMEOUT_SECS: u64 = 5;

/// Interval, in milliseconds, between successive attempts while blocking on a
/// contended file lock.
pub const LOCK_POLL_INTERVAL_MS: u64 = 10;

/// Age, in seconds, at which orphaned `*.tmp` files are swept during startup
/// recovery (1 hour).
pub const TEMP_CLEANUP_THRESHOLD_SECS: u64 = 3600;

/// Default time-to-live, in seconds, for a lease created by `jit claim acquire`,
/// and the default extension applied by `jit claim renew` (10 minutes).
pub const CLAIM_TTL_SECS: u64 = 600;

/// Repo-relative path of the committed reference that projects these defaults.
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
            "Cadence at which the optional auto-heartbeat daemon renews an indefinite (TTL=0) lease.",
        ),
        (
            "Lock acquisition timeout",
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            "Maximum time a writer waits for a `.jit` file lock or the repository write lock before failing. Override with the `JIT_LOCK_TIMEOUT` environment variable.",
        ),
        (
            "Lock poll interval",
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            "Wait between successive attempts while blocking on a contended file lock.",
        ),
        (
            "Temp-file cleanup threshold",
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            "Age at which orphaned `*.tmp` files are swept during startup recovery.",
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
         Built-in defaults for multi-agent coordination and startup recovery. Each\n\
         value is what jit uses when nothing overrides it; the source of truth is\n\
         the `crates/jit/src/runtime_defaults.rs` module, which every production\n\
         call site reads.\n\
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
    #[test]
    #[ignore = "writes the committed reference; run explicitly to regenerate"]
    fn regenerate_reference() {
        std::fs::write(reference_path(), render_reference_markdown())
            .expect("should write the runtime-defaults reference");
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
