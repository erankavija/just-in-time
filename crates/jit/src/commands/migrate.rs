//! One-time data migrations over the repository.

use super::*;
use crate::domain::queries::derive_lifecycle_timestamps;

/// Outcome of the lifecycle-timestamp backfill migration.
///
/// `issues_scanned` is every issue the run inspected; `issues_updated` is the
/// subset that gained at least one derived timestamp and was rewritten. A
/// re-run over an already-migrated repository reports `issues_updated == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleBackfillResult {
    /// Number of issues inspected.
    pub issues_scanned: usize,
    /// Number of issues rewritten with at least one newly-derived timestamp.
    pub issues_updated: usize,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Backfill lifecycle timestamps (`first_ready_at`, `claimed_at`, `done_at`)
    /// onto issues that predate the transition-time write points, deriving each
    /// value from the event log.
    ///
    /// One-time and idempotent: for every issue missing a field, it derives the
    /// value with
    /// [`derive_lifecycle_timestamps`](crate::domain::queries::derive_lifecycle_timestamps)
    /// and fills ONLY the still-absent fields (an existing stamp is never
    /// overwritten, preserving first-occurrence semantics). Issues whose fields
    /// are all set — or whose event log yields nothing to fill — are left byte
    /// unchanged. Each updated issue is saved through the storage layer (atomic
    /// temp-file + rename, INV-ATOMIC-WRITES). When at least one issue changed, a
    /// single [`Event::LifecycleTimestampsBackfilled`] is appended (INV-EVENT-LOG);
    /// a no-op run appends nothing, so re-running is safe and quiet.
    ///
    /// Issues predating event coverage (no relevant events) keep `None` for the
    /// missing fields — the timestamps are unrecoverable, not defaulted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jit::commands::CommandExecutor;
    /// # use jit::storage::JsonFileStorage;
    /// # fn run(executor: &CommandExecutor<JsonFileStorage>) -> anyhow::Result<()> {
    /// let result = executor.backfill_lifecycle_timestamps()?;
    /// println!("updated {} of {} issues", result.issues_updated, result.issues_scanned);
    /// # Ok(())
    /// # }
    /// ```
    pub fn backfill_lifecycle_timestamps(&self) -> Result<LifecycleBackfillResult> {
        let events = self.storage.read_events()?;
        let issues = self.storage.list_issues()?;
        let issues_scanned = issues.len();
        let mut issues_updated = 0;

        for mut issue in issues {
            let derived = derive_lifecycle_timestamps(&issue.id, &events);
            let mut changed = false;

            if issue.first_ready_at.is_none() {
                if let Some(t) = derived.first_ready_at {
                    issue.first_ready_at = Some(t);
                    changed = true;
                }
            }
            if issue.claimed_at.is_none() {
                if let Some(t) = derived.claimed_at {
                    issue.claimed_at = Some(t);
                    changed = true;
                }
            }
            if issue.done_at.is_none() {
                if let Some(t) = derived.done_at {
                    issue.done_at = Some(t);
                    changed = true;
                }
            }

            if changed {
                self.storage.save_issue(issue)?;
                issues_updated += 1;
            }
        }

        if issues_updated > 0 {
            let event = Event::new_lifecycle_timestamps_backfilled(issues_updated);
            self.storage.append_event(&event)?;
        }

        Ok(LifecycleBackfillResult {
            issues_scanned,
            issues_updated,
        })
    }
}
