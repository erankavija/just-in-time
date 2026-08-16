//! Built-in runtime coordination and recovery defaults.
//!
//! These constants are the single source of truth for the operational defaults
//! that govern multi-agent lease coordination and recovery cleanup: the
//! file-lock acquisition timeout and poll interval, the orphaned temp-file
//! cleanup threshold, and the default claim lease TTL. Call sites reference the
//! constants here instead of inline literals, so each default value has one
//! definition.
//!
//! [`render_reference_markdown`] projects these constants into the committed
//! reference `docs/reference/runtime-defaults.md`; a conformance test asserts the
//! committed copy matches the projection, so changing a default without
//! refreshing the reference fails the test suite (`@/inv/single-source-prose`).

/// Maximum time, in seconds, a lock holder waits to acquire a file lock before
/// failing. `JsonFileStorage` resolves one timeout from the `JIT_LOCK_TIMEOUT`
/// environment variable (whole seconds) when set, falling back to this default,
/// and applies it to every lock it acquires — both its repository write lock and
/// the shared `FileLocker` it uses for all other `.jit` file locks. The
/// claim-coordination and worktree locks build their own `FileLocker` from this
/// constant and ignore the variable.
pub const LOCK_TIMEOUT_SECS: u64 = 5;

/// Interval, in milliseconds, between successive attempts while blocking on a
/// contended file lock.
pub const LOCK_POLL_INTERVAL_MS: u64 = 10;

/// Minimum age, in seconds, at which `cleanup_orphaned_temp_files` sweeps an
/// orphaned `*.tmp` file (1 hour). Passed by the `jit recover` command, the
/// single recovery entry point.
pub const TEMP_CLEANUP_THRESHOLD_SECS: u64 = 3600;

/// Default time-to-live, in seconds, for a lease created by `jit claim acquire`,
/// and the default extension applied by `jit claim renew` (1 hour).
pub const CLAIM_TTL_SECS: u64 = 3600;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    /// Repo-relative path of the committed reference that projects these defaults.
    pub const REFERENCE_PATH: &str = "docs/reference/runtime-defaults.md";

    /// The command that renders [`REFERENCE_PATH`] from these constants, named in
    /// the conformance test's message so a stale reference carries its own repair.
    pub const REFERENCE_GENERATOR: &str = "./scripts/generate-runtime-defaults-reference.sh";
}

/// Render the runtime coordination defaults as the committed markdown reference.
///
/// The returned string is the full contents of the committed reference: every
/// value derives from the constants in this module, so the projection cannot
/// drift from the values the code uses. The conformance test in this module
/// asserts the committed file equals this output.
pub fn render_reference_markdown() -> String {
    // (label, value-with-unit, operational scope). Values format the constants
    // so the table is a pure projection of the source of truth above.
    let rows = [
        (
            "Lock acquisition timeout",
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            "Default timeout for acquiring a file lock before failing. `JsonFileStorage` resolves one timeout from the `JIT_LOCK_TIMEOUT` environment variable when set, falling back to this default, and applies it to every lock it acquires — both its repository write lock and the shared `FileLocker` it uses for all other `.jit` file locks. The claim-coordination and worktree locks build their own `FileLocker` from this constant and ignore the environment variable.",
        ),
        (
            "Lock poll interval",
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            "Wait between successive attempts while blocking on a contended file lock.",
        ),
        (
            "Temp-file cleanup threshold",
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            "Minimum age at which `cleanup_orphaned_temp_files` sweeps an orphaned `*.tmp` file. Passed by the `jit recover` command, the single recovery entry point.",
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
            .join(test_support::REFERENCE_PATH)
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
            "{} is stale — regenerate it from `crate::runtime_defaults` (run: {})",
            test_support::REFERENCE_PATH,
            test_support::REFERENCE_GENERATOR,
        );
    }

    #[test]
    fn test_render_lists_every_default_value() {
        let doc = render_reference_markdown();
        for expected in [
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            format!("{CLAIM_TTL_SECS} seconds"),
        ] {
            assert!(doc.contains(&expected), "missing {expected} in:\n{doc}");
        }
    }
}
