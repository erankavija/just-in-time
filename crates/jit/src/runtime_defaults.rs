//! Built-in runtime defaults.
//!
//! These constants are the single source of truth for the operational defaults
//! that govern multi-agent lease coordination and recovery cleanup: the
//! file-lock acquisition timeout and poll interval, the orphaned temp-file
//! cleanup threshold, the claim lease defaults, and the indefinite-lease limits.
//! Call sites reference the constants here instead of inline literals, so each
//! default value has one definition. The configuration defaults owned by their
//! value types (each type's `DEFAULT`, such as
//! [`EnforcementMode::DEFAULT`](crate::config::EnforcementMode::DEFAULT)) and
//! the name-derived projection region markers are projected alongside them.
//!
//! The test-support `render_reference_markdown` projects these constants into
//! the committed reference `docs/reference/runtime-defaults.md`; a conformance
//! test asserts the committed copy matches the projection, so changing a default
//! without refreshing the reference fails the test suite
//! (`@/inv/single-source-prose`).

/// Maximum time, in seconds, a caller waits to acquire a file lock before
/// failing. Every production lock resolves its wait through
/// `storage::lock::configured_lock_timeout`, which reads the `JIT_LOCK_TIMEOUT`
/// environment variable (whole seconds) when set and falls back to this
/// default. The default sits well above one mutation's lock hold, so
/// concurrent callers queue rather than fail.
pub const LOCK_TIMEOUT_SECS: u64 = 60;

// Concurrent callers queue behind a mutation that holds the repository lock
// for several seconds; a shorter default turns that contention into failures.
const _: () = assert!(LOCK_TIMEOUT_SECS >= 60);

/// Interval, in milliseconds, between successive attempts while blocking on a
/// contended file lock.
pub const LOCK_POLL_INTERVAL_MS: u64 = 10;

/// Minimum age, in seconds, at which `jit recover` sweeps an orphaned `*.tmp`
/// file (1 hour). A pending redo record is not a `*.tmp` file; session open
/// applies it instead.
pub const TEMP_CLEANUP_THRESHOLD_SECS: u64 = 3600;

/// Default time-to-live, in seconds, for a lease created by `jit claim acquire`,
/// and the default extension applied by `jit claim renew` (1 hour).
pub const CLAIM_TTL_SECS: u64 = 3600;

/// Seconds without a heartbeat after which an indefinite claim lease is stale
/// (1 hour).
pub const CLAIM_STALE_THRESHOLD_SECS: u64 = 3600;

/// Default maximum concurrent indefinite (TTL=0) leases per agent; the
/// `[coordination] max_indefinite_leases_per_agent` key overrides it.
pub const MAX_INDEFINITE_LEASES_PER_AGENT: u32 = 2;

/// Default maximum concurrent indefinite leases per repository; the
/// `[coordination] max_indefinite_leases_per_repo` key overrides it.
pub const MAX_INDEFINITE_LEASES_PER_REPO: u32 = 10;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    /// Repo-relative path of the committed reference that projects these defaults.
    pub const REFERENCE_PATH: &str = "docs/reference/runtime-defaults.md";

    /// The command that renders [`REFERENCE_PATH`] from these constants, named in
    /// the conformance test's message so a stale reference carries its own repair.
    pub const REFERENCE_GENERATOR: &str = "./scripts/generate-runtime-defaults-reference.sh";
}

/// Render the runtime defaults as the committed markdown reference.
///
/// The returned string is the full contents of the committed reference: every
/// value derives from the constants in this module, so the projection cannot
/// drift from the values the code uses. The conformance test in this module
/// asserts the committed file equals this output.
#[cfg(any(test, feature = "test-support"))]
pub fn render_reference_markdown() -> String {
    use crate::config::{
        default_region_begin, default_region_end, EnforcementMode, ProjectionMode, ProjectionStyle,
    };
    use crate::domain::{ContentFormat, LinkedCheckoutWriteStance};
    use crate::validation::Strictness;

    // (label, value-with-unit, operational scope). Values format the constants
    // so the table is a pure projection of the source of truth above.
    let rows = [
        (
            "Lock acquisition timeout",
            format!("{LOCK_TIMEOUT_SECS} seconds"),
            "Default wait for acquiring a file lock before failing. Every lock jit acquires, including the repository lock chain, the shared `.jit` file locks, and claim coordination, resolves its wait from the `JIT_LOCK_TIMEOUT` environment variable (whole seconds) when set, falling back to this default. A wait that expires exits with the dedicated lock-timeout exit code.",
        ),
        (
            "Lock poll interval",
            format!("{LOCK_POLL_INTERVAL_MS} milliseconds"),
            "Wait between successive attempts while blocking on a contended file lock.",
        ),
        (
            "Temp-file cleanup threshold",
            format!("{TEMP_CLEANUP_THRESHOLD_SECS} seconds"),
            "Minimum age at which `jit recover` removes an orphaned `*.tmp` file under `.jit/` or `.git/jit/leases/`. A pending redo record is not swept: the next mutation session applies it (see [Publication and Recovery](storage-format.md#publication-and-recovery)).",
        ),
        (
            "Claim lease TTL",
            format!("{CLAIM_TTL_SECS} seconds"),
            "Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`.",
        ),
        (
            "Claim stale threshold",
            format!("{CLAIM_STALE_THRESHOLD_SECS} seconds"),
            "Time without a heartbeat after which an indefinite lease is stale. A stale lease no longer refuses other agents' writes, and `jit recover` reclaims it.",
        ),
        (
            "Indefinite leases per agent",
            MAX_INDEFINITE_LEASES_PER_AGENT.to_string(),
            "Default maximum concurrent indefinite (`--ttl 0`) leases one agent holds; `jit claim acquire` refuses beyond it. Overridden by `[coordination] max_indefinite_leases_per_agent`.",
        ),
        (
            "Indefinite leases per repository",
            MAX_INDEFINITE_LEASES_PER_REPO.to_string(),
            "Default maximum concurrent indefinite leases across the repository. Overridden by `[coordination] max_indefinite_leases_per_repo`.",
        ),
        (
            "Lease enforcement mode",
            EnforcementMode::DEFAULT.to_string(),
            "Mode applied when the repository's `[worktree]` section omits `enforce_leases`. In `strict` a write is refused only when another agent holds an active lease on the issue. A repository with no `[worktree]` section enforces no leases.",
        ),
        (
            "Linked-checkout write policy",
            LinkedCheckoutWriteStance::DEFAULT.to_string(),
            "Stance on state-mutating commands run inside a linked non-primary checkout when the repository declares no `[worktree] write_policy`, whether the key or the whole section is absent. The `JIT_WORKTREE_WRITE_POLICY` environment variable outranks it for one invocation.",
        ),
        (
            "Validation strictness",
            Strictness::DEFAULT.to_string(),
            "Level applied when the repository declares no `[validation] strictness`.",
        ),
        (
            "Content format",
            ContentFormat::DEFAULT.to_string(),
            "Body parser applied to an issue that declares no `content_format` when the repository declares no `[validation] content_format`.",
        ),
        (
            "Projection mode",
            ProjectionMode::DEFAULT.to_string(),
            "Mode applied when a `[projection.<name>]` table omits `mode`.",
        ),
        (
            "Projection style",
            ProjectionStyle::DEFAULT.to_string(),
            "Style applied when a `[projection.<name>]` table omits `style`.",
        ),
        (
            "Projection region markers",
            format!(
                "`{}` / `{}`",
                default_region_begin("<name>"),
                default_region_end("<name>")
            ),
            "Delimiters applied in `region` mode when a `[projection.<name>]` table omits `region-begin` / `region-end`; `<name>` is the projection's table key.",
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
         # Runtime Defaults\n\
         \n\
         Built-in defaults for configuration keys, multi-agent lease coordination, and\n\
         recovery cleanup. This reference is generated by\n\
         `crates/jit/src/runtime_defaults.rs` from the named constants that define\n\
         these values.\n\
         \n\
         | Default | Value | Scope |\n\
         | --- | --- | --- |\n\
         {table}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EnforcementMode;
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
            format!("{CLAIM_STALE_THRESHOLD_SECS} seconds"),
            MAX_INDEFINITE_LEASES_PER_AGENT.to_string(),
            MAX_INDEFINITE_LEASES_PER_REPO.to_string(),
            EnforcementMode::DEFAULT.to_string(),
        ] {
            assert!(doc.contains(&expected), "missing {expected} in:\n{doc}");
        }
    }
}
