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
    /// unchanged. All repairs and their single
    /// [`Event::LifecycleTimestampsBackfilled`] audit record publish in one
    /// recoverable delta; a no-op run publishes nothing.
    ///
    /// Issues predating event coverage (no relevant events) keep `None` for the
    /// missing fields — the timestamps are unrecoverable, not defaulted.
    pub fn backfill_lifecycle_timestamps(&self) -> Result<LifecycleBackfillResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{
            finalize, CaptureBudget, CaptureSpec, MutationContext, MutationIntent, VirtualPath,
        };
        use crate::storage::RepositoryStateStoreError;
        let layout = self.require_layout()?;
        let index_path = VirtualPath::data("index.json")?;
        let events_path = VirtualPath::data("events.jsonl")?;
        let budget = CaptureBudget {
            max_paths: 1 << 16,
            max_listings: 1,
            max_bytes: 512 * 1024 * 1024,
            max_depth: 8,
        };
        // Operation-scoped so the repair and audit event keep one identity/time
        // authority across fresh-session conflict retries.
        let context = MutationContext::production();
        for _ in 0..8 {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let first = match session.capture(CaptureSpec::phase_one(
                [index_path.clone(), events_path.clone()],
                budget,
            )?) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let discovery_index = crate::storage::json::parse_repository_index(
                first
                    .file_bytes(&index_path)?
                    .ok_or_else(|| anyhow!("index.json is absent during lifecycle migration"))?,
            )?;
            let discovered_ids = discovery_index.all_ids;
            let mut spec =
                CaptureSpec::phase_one([index_path.clone(), events_path.clone()], budget)?;
            spec.discover_paths(
                discovered_ids
                    .iter()
                    .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                    .collect::<Result<Vec<_>, _>>()?,
            )?;
            let image = match session.capture(spec) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let index = crate::storage::json::parse_repository_index(
                image
                    .file_bytes(&index_path)?
                    .ok_or_else(|| anyhow!("index.json is absent during lifecycle migration"))?,
            )?;
            let ids = index.all_ids;
            if ids != discovered_ids {
                continue;
            }
            let event_text = std::str::from_utf8(image.file_bytes(&events_path)?.unwrap_or(&[]))?;
            let events = crate::domain::parse_known_events(event_text)?;
            let issues = ids
                .iter()
                .map(|id| {
                    let path = VirtualPath::data(format!("issues/{id}.json"))?;
                    let bytes = image
                        .file_bytes(&path)?
                        .ok_or_else(|| anyhow!("indexed issue {id} is absent"))?;
                    let issue = serde_json::from_slice::<Issue>(bytes)?;
                    if issue.id != *id {
                        return Err(anyhow!(
                            "indexed issue {id} contains mismatched embedded id {}",
                            issue.id
                        ));
                    }
                    Ok(issue)
                })
                .collect::<Result<Vec<_>>>()?;
            let issues_scanned = issues.len();
            let mut updates = Vec::new();
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
                    updates.push(issue);
                }
            }
            let issues_updated = updates.len();
            let intents = updates
                .into_iter()
                .map(|issue| MutationIntent::RepairIssueLifecycle {
                    issue: Box::new(issue),
                })
                .chain((issues_updated > 0).then(|| MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(Event::draft_lifecycle_timestamps_backfilled(issues_updated)),
                }))
                .collect::<Vec<_>>();
            let plan = finalize(&layout, &image, &context, &intents)?;
            match session.apply(&plan) {
                Ok(_) => {
                    return Ok(LifecycleBackfillResult {
                        issues_scanned,
                        issues_updated,
                    })
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "lifecycle migration did not converge after repeated capture conflicts"
        ))
    }
}
