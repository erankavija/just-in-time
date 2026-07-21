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

        for issue in matched {
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

            if let Err(e) = blocked.and_then(|()| self.validate_update(issue, operations, false)) {
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

    /// Validate the field operations of an update: gate keys, assignee format, and
    /// the local rules evaluated against the post-update shape. The dependency and
    /// gate guards on a state change live in the captured transition derivation.
    ///
    /// Returns the [`WriteValidation`] outcome (non-blocking warnings plus any
    /// `--force`-bypassed enforce rules). The caller emits the bypass events only
    /// AFTER the issue write succeeds, so a failed save leaves no false bypass
    /// entry. In a read-only preview (`force = false`) blocking rules surface as
    /// an `Err` and `bypassed_rules` is always empty, so nothing is ever logged.
    fn validate_update(
        &self,
        issue: &Issue,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<WriteValidation> {
        // Label format / uniqueness / registry are enforced SOLELY by
        // `validate_for_write` against the projected post-update shape below
        // (a0f0f342 migration) — no inline `validate_label_operations` call here.

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

        // Enforce declarative local rules (and the legacy validator) against the
        // POST-update shape so the batch path (`jit issue update --filter`)
        // cannot bypass enforce rules. On `--force` the bypassed rules are
        // returned for the caller to log AFTER its save; otherwise it blocks.
        let projected = Self::projected_after_update(issue, operations);
        self.validate_for_write(&projected, force)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Issue, Priority, State};

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
        storage
            .save_issue(create_test_issue("test-1", State::Ready, vec!["type:task"]))
            .unwrap();
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
        storage.save_issue(issue).unwrap();

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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec![]))
            .unwrap();
        storage
            .save_issue(create_test_issue("2", State::Ready, vec![]))
            .unwrap();
        storage
            .save_issue(create_test_issue("3", State::InProgress, vec![]))
            .unwrap();

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
        storage
            .save_issue(create_test_issue("1", State::Done, vec![]))
            .unwrap();

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
        storage.save_issue(issue).unwrap();

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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec![]))
            .unwrap();

        let mut blocked = create_test_issue("2", State::Ready, vec![]);
        blocked.gates_required = vec!["tests".to_string()];
        storage.save_issue(blocked).unwrap();

        storage
            .save_issue(create_test_issue("3", State::Ready, vec![]))
            .unwrap();

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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

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

        // Issue already has type:task label
        storage
            .save_issue(create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

        let mut executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to add another type:* label (violates uniqueness)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["type:epic".to_string()],
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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec![]))
            .unwrap();

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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec![]))
            .unwrap();

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
        use crate::storage::{InMemoryStorage, IssueStore};

        let seed = |storage: &InMemoryStorage| {
            for id in ["aaaa1111", "bbbb2222"] {
                storage
                    .save_issue(create_test_issue(id, State::Ready, vec!["type:task"]))
                    .unwrap();
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
        use crate::storage::{InMemoryStorage, IssueStore};

        let seed = |storage: &InMemoryStorage| {
            storage
                .save_issue(create_test_issue(
                    "aaaa1111",
                    State::Backlog,
                    vec!["type:task"],
                ))
                .unwrap();
            let mut blocked = create_test_issue("bbbb2222", State::Ready, vec!["type:task"]);
            blocked.dependencies = vec!["aaaa1111".to_string()];
            storage.save_issue(blocked).unwrap();
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
        use crate::storage::{InMemoryStorage, IssueStore};

        let seed = |storage: &InMemoryStorage| {
            let mut gated = create_test_issue("aaaa1111", State::Ready, vec!["type:task"]);
            gated.gates_required = vec!["tests".to_string()];
            storage.save_issue(gated).unwrap();
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
        storage
            .save_issue(create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

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
}
