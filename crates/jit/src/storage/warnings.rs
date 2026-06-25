//! Typed non-fatal warnings produced by the storage layer.
//!
//! Storage routines (recovery, lock cleanup, worktree identity) occasionally
//! detect conditions that are worth reporting but do not warrant failing the
//! operation: a gap in the claims-log sequence, a stale lock removed from a
//! dead process, a relocated worktree, and so on.
//!
//! Historically these were emitted with `eprintln!` directly from the storage
//! layer. That leaked a rendering decision (and stray stderr) into a layer that
//! must only own persistence, so `--json` callers would see human-formatted
//! noise on stderr. Instead, storage now returns [`StorageWarning`] values and
//! the output layer decides whether and how to render them (see
//! `OutputWriter::print_warning` and the `recover` command's JSON/text output).
//!
//! [`StorageWarning`] implements [`Display`](std::fmt::Display) so it plugs
//! straight into the output layer, and [`Serialize`](serde::Serialize) so it
//! can be embedded in `--json` payloads as structured data.

use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// A non-fatal diagnostic produced by the storage layer.
///
/// Each variant corresponds to a condition that storage used to print with
/// `eprintln!`. The storage layer collects these and hands them back to the
/// caller; the output layer is solely responsible for rendering (and may
/// suppress them, e.g. under `--quiet`, or serialize them under `--json`).
///
/// The [`Display`](std::fmt::Display) form is the bare message body with no
/// `Warning:`/`Error:` prefix, so the output layer can apply its own framing.
///
/// # Examples
///
/// ```
/// use jit::storage::StorageWarning;
///
/// let warning = StorageWarning::SequenceGap { missing: 7 };
/// assert_eq!(warning.to_string(), "Sequence gap detected - missing sequence 7");
///
/// // Warnings are values, so the output layer decides how to render them.
/// let collected = vec![warning, StorageWarning::IndexRebuilt];
/// for w in &collected {
///     // e.g. output.print_warning(w)?;
///     assert!(!w.to_string().is_empty());
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageWarning {
    /// A gap was detected in the claims-log sequence numbers during rebuild.
    SequenceGap {
        /// The sequence number that was expected but missing.
        missing: u64,
    },
    /// A duplicate active lease was found for an issue while verifying the
    /// claims index (a sign the index is inconsistent and must be rebuilt).
    DuplicateLease {
        /// The issue ID that had more than one active lease.
        issue_id: String,
    },
    /// The claims index was inconsistent and has been rebuilt from the log.
    IndexRebuilt,
    /// A stale lock left behind by a finished process was removed.
    StaleLockRemoved {
        /// Path of the lock file that was removed.
        path: PathBuf,
    },
    /// A lock owned by a process that no longer exists was removed.
    LockFromDeadProcess {
        /// PID recorded in the lock metadata.
        pid: u32,
        /// Path of the lock file that was removed.
        path: PathBuf,
    },
    /// A lock is far older than its TTL but its owning process is still alive,
    /// so it was left in place for manual intervention.
    LockVeryOld {
        /// Path of the long-lived lock file.
        path: PathBuf,
        /// Age of the lock in seconds.
        age_secs: i64,
        /// PID recorded in the lock metadata.
        pid: u32,
        /// Agent identifier recorded in the lock metadata.
        agent_id: String,
    },
    /// Best-effort cleanup of orphaned temp files failed (non-fatal).
    TempCleanupFailed {
        /// Human-readable reason the cleanup failed.
        reason: String,
    },
    /// A worktree was detected at a new path and its identity was updated.
    WorktreeRelocated {
        /// Previous recorded root path.
        from: String,
        /// New root path.
        to: String,
    },
}

impl fmt::Display for StorageWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageWarning::SequenceGap { missing } => {
                write!(f, "Sequence gap detected - missing sequence {}", missing)
            }
            StorageWarning::DuplicateLease { issue_id } => {
                write!(f, "Duplicate active lease for issue: {}", issue_id)
            }
            StorageWarning::IndexRebuilt => {
                write!(f, "Claims index inconsistent, rebuilding from log")
            }
            StorageWarning::StaleLockRemoved { path } => {
                write!(f, "Removed stale lock: {}", path.display())
            }
            StorageWarning::LockFromDeadProcess { pid, path } => {
                write!(
                    f,
                    "Removing lock from dead process {}: {}",
                    pid,
                    path.display()
                )
            }
            StorageWarning::LockVeryOld {
                path,
                age_secs,
                pid,
                agent_id,
            } => write!(
                f,
                "Lock very old: {} ({}s, pid={}, agent={})",
                path.display(),
                age_secs,
                pid,
                agent_id
            ),
            StorageWarning::TempCleanupFailed { reason } => {
                write!(f, "Failed to cleanup orphaned temp files: {}", reason)
            }
            StorageWarning::WorktreeRelocated { from, to } => {
                write!(f, "Worktree relocated: {} -> {}", from, to)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_sequence_gap_matches_legacy_message() {
        let warning = StorageWarning::SequenceGap { missing: 3 };
        assert_eq!(
            warning.to_string(),
            "Sequence gap detected - missing sequence 3"
        );
    }

    #[test]
    fn test_display_omits_warning_prefix() {
        // The output layer adds the "Warning:" framing, so the Display body
        // must not include it (otherwise it would be doubled).
        let warning = StorageWarning::IndexRebuilt;
        assert!(!warning.to_string().starts_with("Warning:"));
    }

    #[test]
    fn test_serialize_tags_variant_kind() {
        let warning = StorageWarning::DuplicateLease {
            issue_id: "abc".to_string(),
        };
        let value = serde_json::to_value(&warning).unwrap();
        assert_eq!(value["kind"], "duplicate_lease");
        assert_eq!(value["issue_id"], "abc");
    }
}
