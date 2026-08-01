//! Bulk update operations for filtering and modifying multiple issues
//!
//! Provides unified update interface supporting both single-issue and batch modes.
//! Uses query filter engine to select issues and applies operations atomically per-issue.

use super::*;
use crate::domain::{Assignee, Issue, Priority, State};
use crate::query_engine::QueryFilter;
use serde::Serialize;

/// Operations to apply to issues
#[derive(Debug, Clone, Default)]
pub struct UpdateOperations {
    /// New state to set
    pub state: Option<State>,
    /// Labels to add
    pub add_labels: Vec<String>,
    /// Labels to remove
    pub remove_labels: Vec<String>,
    /// New assignee to set
    pub assignee: Option<String>,
    /// Clear assignee
    pub unassign: bool,
    /// New priority to set
    pub priority: Option<Priority>,
    /// Gates to add
    pub add_gates: Vec<String>,
    /// Gates to remove
    pub remove_gates: Vec<String>,
}

impl UpdateOperations {
    /// True when every operation's target value already holds on `issue`, so
    /// `issue` is nominated as a skip CANDIDATE — not yet a confirmed skip.
    ///
    /// Pure and Issue-local: it decides each operation kind from `issue` alone
    /// (state equality, label presence/absence, priority equality, gate
    /// presence/absence, assignee identity or absence) and never consults the
    /// repository. It never authorizes a skip by itself — a `false` result
    /// means only "not a candidate," and a `true` result means only
    /// "worth checking further": repository-dependent validity (gate registry
    /// membership, dependency/gate transition guards, rule cleanliness, label
    /// and assignee format) is decided exclusively inside a session — the
    /// shared verification session that confirms candidates
    /// (`CommandExecutor::confirm_bulk_noop_candidates`), or the normal
    /// per-issue session for anything that session does not confirm.
    pub fn is_provable_noop(&self, issue: &Issue) -> bool {
        let state_holds = self.state.is_none_or(|target| target == issue.state);
        let labels_hold = self
            .add_labels
            .iter()
            .all(|label| issue.labels.contains(label))
            && self
                .remove_labels
                .iter()
                .all(|label| !issue.labels.contains(label));
        let priority_holds = self.priority.is_none_or(|target| target == issue.priority);
        let gates_hold = self
            .add_gates
            .iter()
            .all(|gate| issue.gates_required.contains(gate))
            && self
                .remove_gates
                .iter()
                .all(|gate| !issue.gates_required.contains(gate));
        let assignee_holds = match &self.assignee {
            Some(target) => {
                issue.assignee.as_ref().map(Assignee::to_string).as_deref() == Some(target.as_str())
            }
            None => !self.unassign || issue.assignee.is_none(),
        };

        state_holds && labels_hold && priority_holds && gates_hold && assignee_holds
    }
}

/// Result of bulk update operation
#[derive(Debug, Serialize)]
pub struct BulkUpdateResult {
    /// IDs that matched the filter
    pub matched: Vec<String>,
    /// IDs successfully updated
    pub modified: Vec<String>,
    /// IDs skipped with reasons (id, reason)
    pub skipped: Vec<(String, String)>,
    /// IDs that failed with errors (id, error)
    pub errors: Vec<(String, String)>,
    /// Per-issue non-enforcing graph-rule warnings (id, message).
    /// Warnings never block the transition; they are informational findings
    /// from rules with `enforce = false`.  Empty when no rules fired.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<(String, String)>,
    /// Summary statistics
    pub summary: BulkUpdateSummary,
}

/// Summary statistics for bulk update
#[derive(Debug, Serialize)]
pub struct BulkUpdateSummary {
    pub total_matched: usize,
    pub total_modified: usize,
    pub total_skipped: usize,
    pub total_errors: usize,
}

impl BulkUpdateResult {
    /// Create a new empty result
    pub fn new() -> Self {
        BulkUpdateResult {
            matched: Vec::new(),
            modified: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            summary: BulkUpdateSummary {
                total_matched: 0,
                total_modified: 0,
                total_skipped: 0,
                total_errors: 0,
            },
        }
    }

    /// Compute summary from current data
    pub fn compute_summary(&mut self) {
        self.summary = BulkUpdateSummary {
            total_matched: self.matched.len(),
            total_modified: self.modified.len(),
            total_skipped: self.skipped.len(),
            total_errors: self.errors.len(),
        };
    }
}

impl Default for BulkUpdateResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Preview of bulk update operation (dry-run)
#[derive(Debug, Serialize)]
pub struct BulkUpdatePreview {
    /// IDs that would be matched
    pub matched: Vec<String>,
    /// Planned changes per issue (id, description)
    pub changes: Vec<(String, Vec<String>)>,
    /// Warnings (id, warning)
    pub warnings: Vec<(String, String)>,
    /// Would-be errors (id, error)
    pub would_fail: Vec<(String, String)>,
    /// Summary of what would happen
    pub summary: PreviewSummary,
}

/// Summary for preview
#[derive(Debug, Serialize)]
pub struct PreviewSummary {
    pub total_matched: usize,
    pub total_would_modify: usize,
    pub total_would_skip: usize,
    pub total_would_fail: usize,
}

impl BulkUpdatePreview {
    /// Create a new empty preview
    pub fn new() -> Self {
        BulkUpdatePreview {
            matched: Vec::new(),
            changes: Vec::new(),
            warnings: Vec::new(),
            would_fail: Vec::new(),
            summary: PreviewSummary {
                total_matched: 0,
                total_would_modify: 0,
                total_would_skip: 0,
                total_would_fail: 0,
            },
        }
    }

    /// Compute summary from current data
    pub fn compute_summary(&mut self) {
        let would_modify = self.changes.iter().filter(|(_, c)| !c.is_empty()).count();
        let would_skip = self.changes.iter().filter(|(_, c)| c.is_empty()).count();

        self.summary = PreviewSummary {
            total_matched: self.matched.len(),
            total_would_modify: would_modify,
            total_would_skip: would_skip,
            total_would_fail: self.would_fail.len(),
        };
    }
}

impl Default for BulkUpdatePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Apply bulk update to filtered issues
    ///
    /// Applies operations to all matched issues with per-issue atomicity.
    /// Best-effort: continues on errors, tracks successes and failures.
    ///
    /// Only opens per-issue preflight/publication sessions for matched issues
    /// that are not confirmed no-ops. [`UpdateOperations::is_provable_noop`]
    /// nominates skip CANDIDATES from the unlocked `matched` snapshot, but
    /// never authorizes a skip by itself; every candidate is confirmed inside
    /// one shared verification session
    /// ([`Self::confirm_bulk_noop_candidates`]) before being recorded as
    /// skipped, so repository-dependent validity is decided only inside a
    /// session — the shared one, or the normal per-issue one for anything the
    /// shared session does not confirm. The shared session's own apply
    /// carries no mutation, but still replays the same optimistic-concurrency
    /// check a real mutation gets, so a confirmed skip is atomic with the
    /// capture instant, not merely read against it (jit 412925b9, round 4).
    ///
    /// A verification-session failure (a capture or declaration error, not a
    /// candidate-level rejection) confirms NOTHING rather than aborting the
    /// whole command: every candidate falls through to its own per-issue
    /// session, so best-effort semantics survive exactly as they did before
    /// this prefilter existed -- each issue's own session failure lands in
    /// `errors` and the loop continues (jit 412925b9, round 3). This means
    /// the `C * J + 1` session-open bound holds on the non-failing path; a
    /// failed verification session instead costs one open (the failed shared
    /// session, still counted by the once-per-session-open probe) plus `C`
    /// per candidate, since every candidate demotes rather than only the
    /// ones requiring a change.
    pub fn apply_bulk_update(
        &mut self,
        filter: &QueryFilter,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<BulkUpdateResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let all_issues = self.storage.list_issues()?;
        let matched = filter.filter_issues(&all_issues)?;

        let mut result = BulkUpdateResult::new();
        result.matched = matched.iter().map(|i| i.id.clone()).collect();

        let candidates: Vec<&Issue> = matched
            .iter()
            .copied()
            .filter(|issue| operations.is_provable_noop(issue))
            .collect();
        let confirmed_skips = if candidates.is_empty() {
            std::collections::HashSet::new()
        } else {
            // Best-effort, not fail-fast: a verification error confirms no
            // candidate, so every one of them is demoted to the per-issue
            // path below instead of aborting the entire bulk command.
            self.confirm_bulk_noop_candidates(&candidates, operations, force)
                .unwrap_or_default()
        };

        for issue in matched {
            if confirmed_skips.contains(&issue.id) {
                result
                    .skipped
                    .push((issue.id.clone(), "No changes needed".to_string()));
                continue;
            }
            match self.apply_operations_to_issue(issue, operations, force) {
                Ok((modified, issue_warnings)) => {
                    for msg in issue_warnings {
                        result.warnings.push((issue.id.clone(), msg));
                    }
                    if modified {
                        result.modified.push(issue.id.clone());
                    } else {
                        result
                            .skipped
                            .push((issue.id.clone(), "No changes needed".to_string()));
                    }
                }
                Err(e) => {
                    result.errors.push((issue.id.clone(), e.to_string()));
                }
            }
        }

        result.compute_summary();
        Ok(result)
    }

    /// Confirm skip candidates nominated by
    /// [`UpdateOperations::is_provable_noop`] inside one shared verification
    /// session, returning the ids safe to record as skipped.
    ///
    /// `is_provable_noop` nominates candidates from the unlocked outer
    /// snapshot and never authorizes a skip on its own. Rather than
    /// re-implementing a hand-picked subset of the write path's checks (which
    /// only ever grows a new gap one reviewer finding at a time — see jit
    /// 412925b9), this runs the SAME authoritative derivation the per-issue
    /// path runs: it captures the repository declarations and the
    /// candidates' issue records from ONE consistent point in time, builds
    /// the identical [`CapturedFieldUpdate`] request
    /// ([`CapturedFieldUpdate::bulk`], shared with
    /// [`CommandExecutor::publish_captured_bulk_update`]) with the caller's
    /// real `force`, and calls [`derive_field_update`] against the freshly
    /// captured issue -- the exact function the per-issue session calls
    /// before applying a plan. A candidate is confirmed clean only when that
    /// derivation reports no field change, no error, and no intents at all
    /// (an empty `intents` list rules out a `--force` bypass event too,
    /// since [`derive_field_update`] appends one whenever a blocking rule is
    /// force-overridden, changed or not).
    ///
    /// Only candidates confirmed clean on every count are returned; anything
    /// else -- including a derivation error, or one that reports a change,
    /// like the gate-semantics Done-with-unpassed-gates redirect that
    /// rewrites `state` to `Gated` even when the request's target state
    /// already equalled the issue's current state -- is left for the caller
    /// to route through the normal per-issue session, exactly like a
    /// non-candidate.
    ///
    /// The confirmed set is not returned straight from this read: the
    /// closure finalizes an EMPTY-delta [`MaterializationPlan`] over the
    /// exact captured `image` and returns [`SessionStep::Apply`], so the
    /// driver's `session.apply` still replays the same optimistic-concurrency
    /// check a real mutation's apply gets -- a concurrent change to anything
    /// within the captured footprint (the same footprint the per-issue
    /// publication session captures, since both call
    /// [`Self::capture_proposed_base`] identically) raises a retryable
    /// conflict and the driver re-runs this whole closure against a fresh
    /// capture, so a confirmed candidate is never stale by more than one
    /// failed apply attempt -- the same guarantee any pre-change per-issue
    /// outcome had. A zero-action delta is a kernel-level no-op short-
    /// circuited before any journal, control-directory, or event-log write
    /// (`execute_repository_delta`'s `if delta.actions().is_empty()` guard,
    /// mirrored by `MemoryMutationSession::apply`), so a successful
    /// confirmation leaves no observable trace and still counts as exactly
    /// one session open, matching the `C * J + 1` budget (jit 412925b9,
    /// round 4).
    fn confirm_bulk_noop_candidates(
        &self,
        candidates: &[&Issue],
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<std::collections::HashSet<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{declarations_from_image, finalize, MutationContext};
        use std::collections::HashSet;

        let layout = self.require_layout()?;
        with_mutation_session(
            &self.storage,
            &layout,
            "bulk-update no-op verification",
            |session| {
                let Some(image) = self.capture_proposed_base(
                    session,
                    &std::collections::BTreeMap::new(),
                    &[],
                    None,
                )?
                else {
                    return Ok(SessionStep::Retry);
                };
                let active = captured_active_issues(&image)?;
                let declarations = declarations_from_image(&image)?;
                let config = declarations.config();
                let plan_content =
                    crate::commands::validate::plan_content_from_image(&image, &active)?;
                let context = MutationContext::production();

                let confirmed = candidates
                    .iter()
                    .filter_map(|candidate| {
                        let fresh = active.iter().find(|issue| issue.id == candidate.id)?;
                        let request =
                            CapturedFieldUpdate::bulk(fresh.id.clone(), operations, force).ok()?;
                        let derived = derive_field_update(
                            fresh.clone(),
                            &request,
                            CapturedTransitionEvidence {
                                issues: &active,
                                declarations: &declarations,
                                config,
                                plan_content: &plan_content,
                                context: &context,
                            },
                        )
                        .ok()?;
                        let confirmed_clean = !derived.changed
                            && derived.error_after_apply.is_none()
                            && derived.intents.is_empty();
                        confirmed_clean.then(|| fresh.id.clone())
                    })
                    .collect::<HashSet<_>>();

                // Confirmation is atomic with the capture instant, not merely
                // read against it: an empty-delta plan over this exact image
                // carries no mutation, but `session.apply` still replays the
                // same optimistic-concurrency check every real mutation gets
                // -- if anything within the captured footprint changed since
                // capture, apply raises a retryable conflict and the driver
                // re-runs this whole closure against a fresh capture, so a
                // confirmed candidate is never stale by more than one failed
                // apply attempt. The kernel short-circuits a zero-action
                // delta before any journal, control, or event write (see
                // `execute_repository_delta` / `MemoryMutationSession::apply`),
                // so a successful confirmation leaves no observable trace.
                let plan = finalize(&layout, &image, &context, &[])?;
                Ok(SessionStep::Apply(plan, confirmed))
            },
        )
    }

    /// Apply operations to a single issue
    ///
    /// Best-effort operation: applies all changes that pass validation,
    /// tracks which fields were modified, and logs appropriate events.
    ///
    /// Returns `Ok((modified, warnings))` where `modified` is `true` when the issue
    /// was changed and `warnings` carries non-enforcing graph-rule findings from the
    /// state transition (if any).  Returns `Err` on hard failures so the caller can
    /// record the issue in its `errors` list and continue best-effort.
    ///
    /// Note: this does NOT call `update_issue_state()` (it skips that path's
    /// prechecks/postchecks/gate-diversion — see the DESIGN DECISION below), but
    /// the literal state change is routed through the captured transition
    /// derivation, so the dependency and gate guards and
    /// transition-time graph-rule enforcement (CC-2) hold on the bulk path with
    /// exactly the semantics of a single-issue transition (jit bc86f54c,
    /// jit b6eb2585). A blocked transition or a blocking enforce rule fails THIS
    /// issue's update like any other per-issue failure; `apply_bulk_update` records
    /// it in `errors` and continues best-effort with the remaining matched issues.
    fn apply_operations_to_issue(
        &mut self,
        issue: &Issue,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<(bool, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_bulk_update(issue.id.clone(), operations, force)
    }

    /// Preview bulk update without applying changes (dry-run)
    pub fn preview_bulk_update(
        &self,
        filter: &QueryFilter,
        operations: &UpdateOperations,
    ) -> Result<BulkUpdatePreview> {
        let all_issues = self.storage.list_issues()?;
        let matched = filter.filter_issues(&all_issues)?;

        let mut preview = BulkUpdatePreview::new();
        preview.matched = matched.iter().map(|i| i.id.clone()).collect();

        for issue in matched {
            let changes = self.compute_changes(issue, operations)?;

            // Check for validation errors. Preview is read-only: pass force=false
            // so blocking rules surface as would-fail and nothing is logged. The
            // transition guards the write path reaches through the chokepoint are
            // replayed here as the pure read they are, so a dependency- or
            // gate-blocked item is reported before anyone runs the update.
            let blocked = operations
                .state
                .filter(|target| *target != issue.state)
                .map(|target| self.transition_blockers(issue, target))
                .unwrap_or(Ok(()));

            if let Err(e) = blocked.and_then(|()| self.validate_update_preview(issue, operations)) {
                preview.would_fail.push((issue.id.clone(), e.to_string()));
            } else if changes.is_empty() {
                // No changes - would be skipped
                preview.changes.push((issue.id.clone(), vec![]));
            } else {
                preview.changes.push((issue.id.clone(), changes));
            }
        }

        preview.compute_summary();
        Ok(preview)
    }

    /// Compute what changes would be made to an issue
    fn compute_changes(&self, issue: &Issue, operations: &UpdateOperations) -> Result<Vec<String>> {
        let mut changes = Vec::new();

        // State change
        if let Some(new_state) = operations.state {
            if issue.state != new_state {
                changes.push(format!("state: {:?} → {:?}", issue.state, new_state));
            }
        }

        // Label additions
        for label in &operations.add_labels {
            if !issue.labels.contains(label) {
                changes.push(format!("add label: {}", label));
            }
        }

        // Label removals
        for label in &operations.remove_labels {
            if issue.labels.contains(label) {
                changes.push(format!("remove label: {}", label));
            }
        }

        // Gate additions
        for gate_key in &operations.add_gates {
            if !issue.gates_required.contains(gate_key) {
                changes.push(format!("add gate: {}", gate_key));
            }
        }

        // Gate removals
        for gate_key in &operations.remove_gates {
            if issue.gates_required.contains(gate_key) {
                changes.push(format!("remove gate: {}", gate_key));
            }
        }

        // Assignee change
        let current_assignee = issue.assignee.as_ref().map(Assignee::to_string);
        if let Some(ref assignee) = operations.assignee {
            if current_assignee.as_deref() != Some(assignee.as_str()) {
                changes.push(format!(
                    "assignee: {} → {}",
                    current_assignee.as_deref().unwrap_or("none"),
                    assignee
                ));
            }
        } else if operations.unassign && issue.assignee.is_some() {
            changes.push(format!(
                "assignee: {} → none",
                current_assignee.as_deref().unwrap_or("none")
            ));
        }

        // Priority change
        if let Some(new_priority) = operations.priority {
            if issue.priority != new_priority {
                changes.push(format!(
                    "priority: {:?} → {:?}",
                    issue.priority, new_priority
                ));
            }
        }

        Ok(changes)
    }

    /// Apply the label/state operations to a clone of `issue`, returning the
    /// shape the issue would have AFTER the bulk update. Used to evaluate local
    /// validation rules against the result of the write (not the pre-update
    /// state), so `--filter` updates cannot slip past `enforce` rules.
    ///
    /// Only the fields that local rules read (labels, state) are projected;
    /// gate/assignee/priority changes are applied too for completeness but do not
    /// affect the validation projection.
    fn projected_after_update(issue: &Issue, operations: &UpdateOperations) -> Issue {
        let mut updated = issue.clone();
        if let Some(new_state) = operations.state {
            updated.state = new_state;
        }
        for label in &operations.add_labels {
            if !updated.labels.contains(label) {
                updated.labels.push(label.clone());
            }
        }
        for label in &operations.remove_labels {
            updated.labels.retain(|l| l != label);
        }
        if let Some(ref assignee) = operations.assignee {
            // Projection only feeds label/state rules; a malformed assignee here
            // (rejected later by `validate_update`) is simply dropped.
            updated.assignee = assignee.parse().ok();
        } else if operations.unassign {
            updated.assignee = None;
        }
        if let Some(new_priority) = operations.priority {
            updated.priority = new_priority;
        }
        updated
    }

    /// Validate a read-only preview against the current repository view.
    /// Publication re-derives the same checks from its captured mutation image.
    fn validate_update_preview(&self, issue: &Issue, operations: &UpdateOperations) -> Result<()> {
        // Label format, uniqueness, and registry rules are evaluated against the
        // projected post-update shape below.

        // Validate gate operations - check that gates exist in registry
        if !operations.add_gates.is_empty() {
            let registry = self.storage.load_gate_registry()?;
            for gate_key in &operations.add_gates {
                if !registry.gates.contains_key(gate_key) {
                    return Err(crate::storage::GateNotFoundError::single(gate_key).into());
                }
            }
        }

        // Validate assignee format
        if let Some(ref assignee) = operations.assignee {
            crate::labels::validate_assignee_format(assignee)?;
        }

        // The captured mutation coordinator owns dependency and gate guards.

        let projected = Self::projected_after_update(issue, operations);
        let repo_format = self.repo_content_format()?;
        let strictness = self
            .cached_config()?
            .validation
            .as_ref()
            .map(crate::config::ValidationConfig::strictness)
            .transpose()?
            .unwrap_or_default();
        let evaluation =
            crate::validation::evaluate_local(&projected, self.effective_rules()?, repo_format)?
                .with_strictness(strictness);
        if let Some(message) = evaluation.rejection_message() {
            return Err(crate::errors::ValidationFailedError::new(message).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Issue, Priority, State};

    /// Fails the FIRST session open at the once-per-session-open failure
    /// point `SessionOpenCounter` also observes
    /// (`TransactionFailurePoint::RepositoryRecoveryExternal`), then lets
    /// every later open through. `apply_bulk_update` always attempts the
    /// shared verification session before any per-issue session, so this
    /// injects exactly one failure into that shared session, isolating its
    /// effect from the per-issue sessions that must still succeed.
    struct FailFirstSessionOpen {
        opens: std::sync::atomic::AtomicUsize,
        failed_once: std::sync::atomic::AtomicBool,
    }

    impl crate::storage::TransactionFailureInjector for FailFirstSessionOpen {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
            use std::sync::atomic::Ordering;
            if point == &crate::storage::TransactionFailurePoint::RepositoryRecoveryExternal {
                self.opens.fetch_add(1, Ordering::SeqCst);
                if !self.failed_once.swap(true, Ordering::SeqCst) {
                    return Err(std::io::Error::other(
                        "injected verification-session capture failure",
                    ));
                }
            }
            Ok(())
        }
    }

    fn create_test_issue(id: &str, state: State, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            title: format!("Test {}", id),
            description: String::new(),
            state,
            priority: Priority::Normal,
            assignee: None,
            dependencies: vec![],
            gates_required: vec![],
            gates_status: Default::default(),
            context: Default::default(),
            documents: vec![],
            labels: labels.iter().map(|s| s.to_string()).collect(),
            content_format: None,
            created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            first_ready_at: None,
            claimed_at: None,
            done_at: None,
            archived_from: None,
        }
    }

    #[test]
    fn test_update_operations_default() {
        let ops = UpdateOperations::default();
        assert!(ops.state.is_none());
        assert!(ops.add_labels.is_empty());
        assert!(ops.remove_labels.is_empty());
        assert!(ops.assignee.is_none());
        assert!(!ops.unassign);
        assert!(ops.priority.is_none());
    }

    #[test]
    fn test_bulk_update_result_new() {
        let result = BulkUpdateResult::new();
        assert!(result.matched.is_empty());
        assert!(result.modified.is_empty());
        assert!(result.skipped.is_empty());
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert_eq!(result.summary.total_matched, 0);
    }

    #[test]
    fn test_bulk_update_result_compute_summary() {
        let mut result = BulkUpdateResult::new();
        result.matched = vec!["1".to_string(), "2".to_string()];
        result.modified = vec!["1".to_string()];
        result.skipped = vec![("2".to_string(), "no changes".to_string())];

        result.compute_summary();

        assert_eq!(result.summary.total_matched, 2);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_skipped, 1);
        assert_eq!(result.summary.total_errors, 0);
    }

    #[test]
    fn test_preview_compute_summary() {
        let mut preview = BulkUpdatePreview::new();
        preview.matched = vec!["1".to_string(), "2".to_string()];
        preview.changes = vec![
            ("1".to_string(), vec!["state change".to_string()]),
            ("2".to_string(), vec![]), // No changes
        ];

        preview.compute_summary();

        assert_eq!(preview.summary.total_matched, 2);
        assert_eq!(preview.summary.total_would_modify, 1);
        assert_eq!(preview.summary.total_would_skip, 1);
    }

    #[test]
    fn test_compute_changes_state() {
        let issue = create_test_issue("1", State::Ready, vec![]);
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let executor =
            crate::commands::test_helpers::memory_executor(crate::storage::InMemoryStorage::new());

        let changes = executor.compute_changes(&issue, &ops).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("Ready"));
        assert!(changes[0].contains("Done"));
    }

    #[test]
    fn test_compute_changes_labels() {
        let issue = create_test_issue("1", State::Ready, vec!["type:task"]);
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string()],
            remove_labels: vec!["type:task".to_string()],
            ..Default::default()
        };

        let executor =
            crate::commands::test_helpers::memory_executor(crate::storage::InMemoryStorage::new());

        let changes = executor.compute_changes(&issue, &ops).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes
            .iter()
            .any(|c| c.contains("add label: milestone:v1.0")));
        assert!(changes
            .iter()
            .any(|c| c.contains("remove label: type:task")));
    }

    #[test]
    fn test_compute_changes_no_changes() {
        let issue = create_test_issue("1", State::Ready, vec![]);
        let ops = UpdateOperations {
            state: Some(State::Ready), // Same state
            ..Default::default()
        };

        let executor =
            crate::commands::test_helpers::memory_executor(crate::storage::InMemoryStorage::new());

        let changes = executor.compute_changes(&issue, &ops).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_apply_bulk_update_assignee_stamps_claimed_at_and_logs_event() {
        use crate::query_engine::QueryFilter;
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("test-1", State::Ready, vec!["type:task"]),
        );
        // A clone shares the in-memory state, so we can read events after the
        // executor takes ownership of `storage`.
        let reader = storage.clone();

        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("agent:worker-1".to_string()),
            ..Default::default()
        };
        executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // claimed_at stamped by the bulk assignment.
        let updated = executor.get_issue("test-1").unwrap();
        assert!(updated.claimed_at.is_some());

        // @/inv/event-log: the assignee mutation appended an issue_claimed event.
        let claimed = reader
            .read_events()
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e, Event::IssueClaimed { .. }))
            .count();
        assert_eq!(claimed, 1);

        // First-occurrence: a second assignee change does not move claimed_at.
        let first = updated.claimed_at;
        let ops2 = UpdateOperations {
            assignee: Some("agent:worker-2".to_string()),
            ..Default::default()
        };
        executor.apply_bulk_update(&filter, &ops2, false).unwrap();
        assert_eq!(executor.get_issue("test-1").unwrap().claimed_at, first);
    }

    #[test]
    fn test_apply_bulk_update_single_issue() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create test issue
        let issue = create_test_issue("test-1", State::Ready, vec!["type:task"]);
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Apply bulk update
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify issue was updated
        let updated = executor.get_issue("test-1").unwrap();
        assert_eq!(updated.state, State::Done);
    }

    #[test]
    fn test_apply_bulk_update_multiple_issues() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create test issues
        for issue in [
            create_test_issue("1", State::Ready, vec![]),
            create_test_issue("2", State::Ready, vec![]),
            create_test_issue("3", State::InProgress, vec![]),
        ] {
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Update all ready issues
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 2);
        assert_eq!(result.summary.total_modified, 2);

        // Verify labels added
        let issue1 = executor.get_issue("1").unwrap();
        assert!(issue1.labels.contains(&"milestone:v1.0".to_string()));

        let issue3 = executor.get_issue("3").unwrap();
        assert!(!issue3.labels.contains(&"milestone:v1.0".to_string()));
    }

    #[test]
    fn test_apply_bulk_update_no_changes() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Done, vec![]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to set same state
        let filter = QueryFilter::parse("state:done").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_skipped, 1);
    }

    #[test]
    fn test_apply_bulk_update_with_errors() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create issue with unpassed gates
        let mut issue = create_test_issue("1", State::Gated, vec![]);
        issue.gates_required = vec!["tests".to_string()];
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to transition to Done without passing gates
        let filter = QueryFilter::parse("state:gated").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        // The typed gate blocker from the transition chokepoint, rendered.
        assert!(result.errors[0].1.contains("1 gate(s) not passed"));
        assert!(result.errors[0].1.contains("tests [pending]"));
    }

    #[test]
    fn test_apply_bulk_update_best_effort() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create mix of valid and invalid issues
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec![]),
        );

        let mut blocked = create_test_issue("2", State::Ready, vec![]);
        blocked.gates_required = vec!["tests".to_string()];
        crate::commands::test_helpers::seed_issue(&storage, blocked);

        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("3", State::Ready, vec![]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to transition all to Done
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should succeed for 2, fail for 1
        assert_eq!(result.summary.total_matched, 3);
        assert_eq!(result.summary.total_modified, 2);
        assert_eq!(result.summary.total_errors, 1);

        // Verify partial success
        assert!(result.modified.contains(&"1".to_string()));
        assert!(result.modified.contains(&"3".to_string()));
        assert_eq!(result.errors[0].0, "2");
    }

    #[test]
    fn test_bulk_update_rejects_invalid_label_format() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec!["type:task"]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to add label without colon (invalid format)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["bad_label_no_colon".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should reject with error. After the a0f0f342 consolidation the rejection
        // comes from the always-enforced `label-format` rule (canonical format,
        // origin = "default"), not the removed inline `validate_label_operations`
        // check.
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("label-format"));
    }

    #[test]
    fn test_bulk_update_rejects_duplicate_unique_namespace() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        // `type` is unique only because this repository declares it so.
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        storage.add_data_file(
            "config.toml",
            &format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            ),
        );

        // Issue already carries the leaf kind that configuration declares.
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                "1",
                State::Ready,
                vec![&format!("type:{}", taxonomy.type_at_level(4))],
            ),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to add another type:* label (violates uniqueness)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec![format!("type:{}", taxonomy.type_at_level(2))],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should reject with error. After the a0f0f342 consolidation the rejection
        // comes from the always-enforced `namespace-unique-type` rule (origin =
        // "default"), not the removed inline uniqueness check.
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("namespace-unique-type"));
    }

    #[test]
    fn test_bulk_update_rejects_invalid_assignee_format() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec![]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to set assignee without colon (invalid format)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("invalid_no_colon".to_string()),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should reject with error
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("format"));
    }

    #[test]
    fn test_bulk_update_accepts_valid_assignee_format() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec![]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Valid assignee format should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("agent:copilot".to_string()),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should succeed
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify assignee set correctly
        let updated = executor.get_issue("1").unwrap();
        assert_eq!(updated.assignee, Some("agent:copilot".parse().unwrap()));
    }

    /// Strip the volatile fields (event id, timestamp) so two event logs
    /// recorded by different executors compare on their semantic payload.
    fn event_payload(event: &Event) -> serde_json::Value {
        let mut value = serde_json::to_value(event).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("id");
        object.remove("timestamp");
        value
    }

    /// Every event the log recorded for `issue_id`, in order, payload-normalized.
    fn event_payloads_for(
        storage: &crate::storage::InMemoryStorage,
        issue_id: &str,
    ) -> Vec<String> {
        use crate::storage::IssueStore;
        storage
            .read_events()
            .unwrap()
            .iter()
            .map(event_payload)
            .filter(|value| value["issue_id"] == issue_id)
            .map(|value| value.to_string())
            .collect()
    }

    /// REQ-3: a bulk state transition emits the same event sequence per issue as
    /// the equivalent single-issue transition. Both runs start from an identical
    /// repository, so any divergence is the transition path's own doing.
    #[test]
    fn test_bulk_transition_emits_same_events_as_single_transitions() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let seed = |storage: &InMemoryStorage| {
            for id in ["aaaa1111", "bbbb2222"] {
                crate::commands::test_helpers::seed_issue(
                    storage,
                    create_test_issue(id, State::Ready, vec!["type:task"]),
                );
            }
        };

        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let bulk_reader = bulk_storage.clone();
        let mut bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };
        let result = bulk.apply_bulk_update(&filter, &ops, false).unwrap();
        assert_eq!(result.summary.total_modified, 2);

        let single_storage = InMemoryStorage::new();
        seed(&single_storage);
        let single_reader = single_storage.clone();
        let single = crate::commands::test_helpers::memory_executor(single_storage);
        for id in ["aaaa1111", "bbbb2222"] {
            single
                .update_issue(
                    id,
                    None,
                    None,
                    None,
                    Some(State::Done),
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap();
        }

        for id in ["aaaa1111", "bbbb2222"] {
            assert_eq!(
                event_payloads_for(&bulk_reader, id),
                event_payloads_for(&single_reader, id),
                "event sequence for issue {id} diverges between bulk and single transition"
            );
        }
    }

    /// REQ-2: a dependency-blocked bulk item carries the same typed blockers as
    /// the equivalent single-issue transition.
    #[test]
    fn test_apply_operations_dependency_blocked_reports_same_typed_blockers() {
        use crate::errors::TransitionBlockedError;
        use crate::storage::InMemoryStorage;

        let seed = |storage: &InMemoryStorage| {
            crate::commands::test_helpers::seed_issue(
                storage,
                create_test_issue("aaaa1111", State::Backlog, vec!["type:task"]),
            );
            let mut blocked = create_test_issue("bbbb2222", State::Ready, vec!["type:task"]);
            blocked.dependencies = vec!["aaaa1111".to_string()];
            crate::commands::test_helpers::seed_issue(storage, blocked);
        };

        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let mut bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
        let issue = bulk.get_issue("bbbb2222").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };
        let bulk_error = bulk
            .apply_operations_to_issue(&issue, &ops, false)
            .unwrap_err()
            .downcast::<TransitionBlockedError>()
            .expect("bulk reports a typed transition blocker");

        let single_storage = InMemoryStorage::new();
        seed(&single_storage);
        let single = crate::commands::test_helpers::memory_executor(single_storage);
        let single_error = single
            .update_issue(
                "bbbb2222",
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap_err()
            .downcast::<TransitionBlockedError>()
            .expect("single-issue update reports a typed transition blocker");

        assert_eq!(bulk_error.blockers(), single_error.blockers());
        assert_eq!(bulk_error.requested_state(), single_error.requested_state());
    }

    /// REQ-2, gate arm: an unpassed gate blocks the bulk item with the same typed
    /// blockers the single-issue transition reports.
    #[test]
    fn test_apply_operations_gate_blocked_reports_same_typed_blockers() {
        use crate::errors::TransitionBlockedError;
        use crate::storage::InMemoryStorage;

        let seed = |storage: &InMemoryStorage| {
            let mut gated = create_test_issue("aaaa1111", State::Ready, vec!["type:task"]);
            gated.gates_required = vec!["tests".to_string()];
            crate::commands::test_helpers::seed_issue(storage, gated);
        };

        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let mut bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
        let issue = bulk.get_issue("aaaa1111").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };
        let bulk_error = bulk
            .apply_operations_to_issue(&issue, &ops, false)
            .unwrap_err()
            .downcast::<TransitionBlockedError>()
            .expect("bulk reports a typed transition blocker");

        let single_storage = InMemoryStorage::new();
        seed(&single_storage);
        let single = crate::commands::test_helpers::memory_executor(single_storage);
        let single_error = single
            .update_issue(
                "aaaa1111",
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap_err()
            .downcast::<TransitionBlockedError>()
            .expect("single-issue update reports a typed transition blocker");

        assert_eq!(bulk_error.blockers(), single_error.blockers());
        assert_eq!(bulk_error.requested_state(), single_error.requested_state());
    }

    #[test]
    fn test_bulk_update_accepts_valid_labels() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec!["type:task"]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Valid labels should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string(), "epic:auth".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        // Should succeed
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify labels added
        let updated = executor.get_issue("1").unwrap();
        assert!(updated.labels.contains(&"milestone:v1.0".to_string()));
        assert!(updated.labels.contains(&"epic:auth".to_string()));
    }

    // -- UpdateOperations::is_provable_noop -----------------------------

    #[test]
    fn test_is_provable_noop_state_holds_only_at_target_state() {
        let issue = create_test_issue("1", State::Ready, vec![]);

        assert!(UpdateOperations {
            state: Some(State::Ready),
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        }
        .is_provable_noop(&issue));
        // No state operation requested: vacuously holds.
        assert!(UpdateOperations::default().is_provable_noop(&issue));
    }

    #[test]
    fn test_is_provable_noop_labels_require_adds_present_and_removes_absent() {
        let issue = create_test_issue("1", State::Ready, vec!["type:task"]);

        assert!(UpdateOperations {
            add_labels: vec!["type:task".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(UpdateOperations {
            remove_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            remove_labels: vec!["type:task".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
    }

    #[test]
    fn test_is_provable_noop_priority_holds_only_at_target_priority() {
        let issue = create_test_issue("1", State::Ready, vec![]);

        assert!(UpdateOperations {
            priority: Some(Priority::Normal),
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        }
        .is_provable_noop(&issue));
    }

    #[test]
    fn test_is_provable_noop_gates_require_adds_present_and_removes_absent() {
        let mut issue = create_test_issue("1", State::Ready, vec![]);
        issue.gates_required = vec!["tests".to_string()];

        assert!(UpdateOperations {
            add_gates: vec!["tests".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            add_gates: vec!["code-review".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(UpdateOperations {
            remove_gates: vec!["code-review".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            remove_gates: vec!["tests".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
    }

    #[test]
    fn test_is_provable_noop_assignee_holds_at_target_identity_or_absence() {
        let mut issue = create_test_issue("1", State::Ready, vec![]);

        // Already unassigned: `unassign` is a no-op.
        assert!(UpdateOperations {
            unassign: true,
            ..Default::default()
        }
        .is_provable_noop(&issue));

        issue.assignee = Some("agent:worker-1".parse().unwrap());
        assert!(!UpdateOperations {
            unassign: true,
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(UpdateOperations {
            assignee: Some("agent:worker-1".to_string()),
            ..Default::default()
        }
        .is_provable_noop(&issue));
        assert!(!UpdateOperations {
            assignee: Some("agent:worker-2".to_string()),
            ..Default::default()
        }
        .is_provable_noop(&issue));
    }

    #[test]
    fn test_is_provable_noop_mixed_operations_requires_every_target_to_already_hold() {
        let issue = create_test_issue("1", State::Ready, vec!["type:task"]);

        // Priority already holds, but the label add does not: NOT a no-op, since
        // every operation's target must already hold, not just some.
        assert!(!UpdateOperations {
            priority: Some(Priority::Normal),
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));

        // Every operation's target already holds: a provable no-op.
        assert!(UpdateOperations {
            priority: Some(Priority::Normal),
            add_labels: vec!["type:task".to_string()],
            remove_labels: vec!["milestone:v0.9".to_string()],
            ..Default::default()
        }
        .is_provable_noop(&issue));
    }

    // -- Session budget (REQ-01, REQ-04) ---------------------------------

    /// REQ-01, REQ-04 (amended, D-1): sessions open only for matched issues
    /// that are not CONFIRMED no-ops, plus exactly one shared verification
    /// session whenever at least one skip candidate exists. `C` (the
    /// per-mutation session constant) is derived from a single-issue baseline
    /// run rather than hardcoded, session opens are counted via the
    /// once-per-session-open failure-point probe shared with the
    /// automatic-transitions budget test
    /// (`crate::commands::issue::tests::test_check_auto_transitions_opens_sessions_only_for_eligible_backlog_issues`),
    /// and the assertion is the semantic relationship `C * J + 1`, not a
    /// copied constant.
    #[test]
    fn test_apply_bulk_update_session_budget_scales_with_issues_needing_change() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        };

        // Baseline: a single issue that DOES require the change derives C.
        // No skip candidates exist in this run, so no shared verification
        // session is funded and the baseline measures C alone.
        let baseline_counter = SessionOpenCounter::new();
        let baseline_storage =
            InMemoryStorage::new().with_repository_state_failure_view(baseline_counter.clone());
        crate::commands::test_helpers::seed_issue(
            &baseline_storage,
            create_test_issue("baseline", State::Ready, vec![]),
        );
        let mut baseline_executor =
            crate::commands::test_helpers::memory_executor(baseline_storage);
        let baseline_result = baseline_executor
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();
        assert_eq!(baseline_result.summary.total_modified, 1);
        let sessions_per_change = baseline_counter.count();
        assert!(
            sessions_per_change > 0,
            "a real change must open at least one session"
        );

        // K = 5 matched issues, J = 2 require the change; the remaining K-J
        // already hold the target priority and must be recognized as skip
        // candidates confirmed clean by the shared verification session.
        let k = 5;
        let j = 2;
        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_repository_state_failure_view(counter.clone());
        for index in 0..k {
            let mut issue = create_test_issue(&format!("issue-{index}"), State::Ready, vec![]);
            if index >= j {
                issue.priority = Priority::High;
            }
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }
        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, k);
        assert_eq!(result.summary.total_modified, j);
        assert_eq!(result.summary.total_skipped, k - j);
        assert_eq!(
            counter.count(),
            sessions_per_change * j + 1,
            "session opens must scale with issues that are not confirmed no-ops, plus the one shared verification session that confirmed the rest"
        );
    }

    /// Amended REQ-04's zero-candidate case: when no matched issue is a skip
    /// candidate, no shared verification session is opened at all, so the
    /// budget is exactly `C * K`, with no `+ 1` term.
    #[test]
    fn test_apply_bulk_update_session_budget_zero_candidates_opens_exactly_c_times_k() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        };

        let baseline_counter = SessionOpenCounter::new();
        let baseline_storage =
            InMemoryStorage::new().with_repository_state_failure_view(baseline_counter.clone());
        crate::commands::test_helpers::seed_issue(
            &baseline_storage,
            create_test_issue("baseline", State::Ready, vec![]),
        );
        let mut baseline_executor =
            crate::commands::test_helpers::memory_executor(baseline_storage);
        baseline_executor
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();
        let sessions_per_change = baseline_counter.count();
        assert!(sessions_per_change > 0);

        // K = 3 matched issues, none already at the target priority: zero
        // skip candidates.
        let k = 3;
        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_repository_state_failure_view(counter.clone());
        for index in 0..k {
            crate::commands::test_helpers::seed_issue(
                &storage,
                create_test_issue(&format!("issue-{index}"), State::Ready, vec![]),
            );
        }
        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, k);
        assert_eq!(result.summary.total_modified, k);
        assert_eq!(result.summary.total_skipped, 0);
        assert_eq!(
            counter.count(),
            sessions_per_change * k,
            "no skip candidates must mean no shared verification session, so the budget carries no +1 term"
        );
    }

    /// Edge case of amended REQ-01: every matched issue is a skip candidate
    /// and every candidate is confirmed clean, so no PER-ISSUE session opens
    /// at all -- but the one shared verification session that confirmed them
    /// still does.
    #[test]
    fn test_apply_bulk_update_all_confirmed_noops_opens_only_shared_verification_session() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_repository_state_failure_view(counter.clone());
        for index in 0..3 {
            let mut issue = create_test_issue(&format!("noop-{index}"), State::Ready, vec![]);
            issue.priority = Priority::High;
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }
        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 3);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_skipped, 3);
        assert_eq!(
            counter.count(),
            1,
            "every candidate confirmed clean means zero per-issue sessions, but the shared verification session that confirmed them still opens exactly once"
        );
    }

    /// REQ-03: an issue whose update is potentially invalid is not even a
    /// skip candidate (its state differs from the requested target), so it
    /// still opens a per-issue session, and the authoritative in-session
    /// check still produces the same typed error as before this change
    /// (matching `test_apply_bulk_update_with_errors`).
    #[test]
    fn test_apply_bulk_update_potentially_invalid_transition_still_opens_session_and_errors() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_repository_state_failure_view(counter.clone());

        let mut issue = create_test_issue("blocked", State::Gated, vec![]);
        issue.gates_required = vec!["tests".to_string()];
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:gated").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("1 gate(s) not passed"));
        assert!(
            counter.count() > 0,
            "a potentially invalid update must still open a session rather than being skipped as a provable no-op"
        );
    }

    /// REQ-03 regression: the in-session write path evaluates local rules
    /// against the projected issue unconditionally, even when no field
    /// actually changes (see `test_bulk_update_force_noop_logs_bypass` in
    /// `fast_rules::local_rule_enforcement_tests`, which covers the `--force`
    /// arm of this same scenario). A field-level no-op is therefore only a
    /// CONFIRMED no-op when the shared verification session also finds the
    /// issue clean; a pre-existing violation must demote the candidate to a
    /// real per-issue session so the error surfaces exactly as pre-change,
    /// not be swallowed by the prefilter as "No changes needed". The session
    /// count proves the demotion actually happened: one shared verification
    /// session (which nominated then rejected the candidate) plus a real
    /// per-issue session for the demoted issue's authoritative attempt.
    #[test]
    fn test_apply_bulk_update_field_level_noop_with_preexisting_violation_still_errors() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_repository_state_failure_view(counter.clone());
        // Seeded directly, bypassing create-time validation, so the issue
        // already violates the always-enforced default label-format rule.
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("1", State::Ready, vec!["bad_label_no_colon"]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        // Field-level no-op: the requested priority already holds, so this
        // issue IS nominated as a skip candidate.
        let ops = UpdateOperations {
            priority: Some(Priority::Normal),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(
            result.summary.total_modified, 0,
            "no field actually changed"
        );
        assert_eq!(
            result.summary.total_errors, 1,
            "a pre-existing rule violation must still surface as an error, not a skip"
        );
        assert_eq!(result.summary.total_skipped, 0);
        assert!(result.errors[0].1.contains("label-format"));
        assert!(
            counter.count() > 1,
            "the candidate must be demoted to a real per-issue session, not just the shared verification session"
        );
    }

    /// REQ-03 (round 2 regression, `@/inv/gate-semantics`): the gate-semantics
    /// Done redirect in `derive_field_update` fires from "target == Done AND
    /// unpassed gates" alone, independent of whether the issue's CURRENT
    /// state already equals Done. A bulk `state: Some(Done)` request on an
    /// already-Done issue with an unpassed required gate therefore looks
    /// like a field-level no-op to `is_provable_noop` (state already equals
    /// the target), but the authoritative derivation still rewrites state to
    /// `Gated` and reports a gate error -- so the shared verification
    /// session must demote it, never confirm it as a skip.
    ///
    /// Proves this by comparing the full `apply_bulk_update` path against
    /// `apply_operations_to_issue` called directly on an identically seeded
    /// issue -- the per-issue path every matched issue went through before
    /// this prefilter existed -- and asserting byte-identical results
    /// (error text and final stored state), not a hardcoded message.
    #[test]
    fn test_apply_bulk_update_done_state_noop_with_unpassed_gate_still_diverts_and_errors() {
        use crate::errors::TransitionBlockedError;
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let seed = |storage: &InMemoryStorage| {
            let mut issue = create_test_issue("aaaa1111", State::Done, vec!["type:task"]);
            issue.gates_required = vec!["tests".to_string()];
            crate::commands::test_helpers::seed_issue(storage, issue);
        };
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        // Ground truth: the per-issue path called directly, bypassing the
        // prefilter entirely (what every matched issue went through before
        // 412925b9).
        let direct_storage = InMemoryStorage::new();
        seed(&direct_storage);
        let mut direct = crate::commands::test_helpers::memory_executor(direct_storage);
        let issue = direct.get_issue("aaaa1111").unwrap();
        let direct_error = direct
            .apply_operations_to_issue(&issue, &ops, false)
            .unwrap_err()
            .downcast::<TransitionBlockedError>()
            .expect("direct per-issue path reports a typed transition blocker");
        let direct_final_state = direct.get_issue("aaaa1111").unwrap().state;
        assert_eq!(
            direct_final_state,
            State::Gated,
            "gate-semantics diverts an unpassed-gate Done request to Gated"
        );

        // Full bulk path: this issue IS a skip candidate (state == target),
        // so the shared verification session must demote it.
        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let mut bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
        let filter = QueryFilter::parse("state:done").unwrap();
        let result = bulk.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(
            result.summary.total_skipped, 0,
            "must be demoted to a per-issue session, not confirmed as a skip"
        );
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert_eq!(result.errors[0].1, direct_error.to_string());
        assert_eq!(
            bulk.get_issue("aaaa1111").unwrap().state,
            direct_final_state,
            "the divert-to-Gated write must happen identically through the bulk path"
        );
    }

    /// REQ-03 (round 3 regression): a shared-verification-session failure
    /// (a capture or declaration error, distinct from a per-candidate
    /// rejection) must demote every candidate to the normal per-issue path,
    /// not abort the whole command -- the pre-change contract was per-issue
    /// best-effort, and this prefilter must not regress it.
    ///
    /// Seeds one provable no-op candidate (priority already holds) and one
    /// issue that genuinely needs the change, both matched. Injects a
    /// failure into the FIRST session open -- always the shared verification
    /// session, since `apply_bulk_update` attempts it before any per-issue
    /// session -- and asserts the command still succeeds, with both issues
    /// reaching their own per-issue session and landing in the exact same
    /// result bucket (`skipped` for the no-op, `modified` for the real
    /// change) that the per-issue path alone would have produced.
    #[test]
    fn test_apply_bulk_update_verification_session_failure_demotes_all_candidates() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        let injector = Arc::new(FailFirstSessionOpen {
            opens: AtomicUsize::new(0),
            failed_once: AtomicBool::new(false),
        });
        let storage = InMemoryStorage::new().with_repository_state_failure_view(injector.clone());

        let mut noop_issue = create_test_issue("noop", State::Ready, vec![]);
        noop_issue.priority = Priority::High;
        crate::commands::test_helpers::seed_issue(&storage, noop_issue);
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue("changer", State::Ready, vec![]),
        );

        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).expect(
            "a verification-session failure must degrade to per-issue best-effort, not abort the command",
        );

        assert_eq!(result.summary.total_matched, 2);
        assert_eq!(result.summary.total_errors, 0);
        assert_eq!(
            result.summary.total_skipped, 1,
            "the no-op candidate is demoted (not confirmed), but the per-issue path still reports it as unchanged"
        );
        assert_eq!(
            result.summary.total_modified, 1,
            "the genuinely changing issue still reaches its per-issue session and succeeds there"
        );
        assert!(
            injector.opens.load(Ordering::SeqCst) > 1,
            "per-issue sessions must still have opened after the failed shared verification session"
        );
        assert_eq!(executor.get_issue("noop").unwrap().priority, Priority::High);
        assert_eq!(
            executor.get_issue("changer").unwrap().priority,
            Priority::High
        );
    }

    /// REQ-03 (round 4 regression): a confirmed skip is atomic with the
    /// verification session's capture instant, not merely read against it --
    /// a change landing after the capture but before the (empty-delta) apply
    /// must not be silently missed.
    ///
    /// Forces exactly one retry by injecting a single apply-time conflict via
    /// `InMemoryStorage::inject_repository_state_apply_conflicts` (the
    /// existing test seam `MemoryMutationSession::apply` consumes before its
    /// own natural optimistic-concurrency check) -- this fires on the shared
    /// verification session's FIRST apply, the very first apply anywhere in
    /// this call. Combines it with the `OpenRace` probe infrastructure,
    /// triggered on the SECOND session open (the retry that follows), to
    /// mutate the sole candidate's priority away from the requested target
    /// right as that retry begins -- strictly before its own capture reads
    /// storage. The retry's capture therefore observes the changed value, so
    /// the candidate must fail re-confirmation and demote to its own
    /// per-issue session instead of being confirmed as a stale skip.
    #[test]
    fn test_apply_bulk_update_verification_apply_conflict_forces_fresh_recapture_not_stale_skip() {
        use crate::commands::test_helpers::{with_open_race, OpenRaceAction};
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        let mut noop_issue = create_test_issue("noop", State::Ready, vec![]);
        noop_issue.priority = Priority::High; // matches the requested target
        crate::commands::test_helpers::seed_issue(&storage, noop_issue.clone());

        // Fires when the SECOND session opens -- the retry after the
        // injected apply conflict below -- mutating "noop" away from the
        // requested target before that retry's own capture reads it.
        let mut changed = noop_issue;
        changed.priority = Priority::Low;
        let storage = with_open_race(storage, 2, OpenRaceAction::Save(Box::new(changed)));
        // Force exactly one retry on the shared verification session's own
        // (empty-delta) apply, as if its natural optimistic check had itself
        // found the concurrent mutation.
        storage.inject_repository_state_apply_conflicts(1);

        let mut executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            priority: Some(Priority::High),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(
            result.summary.total_skipped, 0,
            "the candidate changed before the retry's capture, so it must not be confirmed as a stale skip"
        );
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);
        assert_eq!(executor.get_issue("noop").unwrap().priority, Priority::High);
    }
}
