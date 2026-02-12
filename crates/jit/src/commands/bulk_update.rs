//! Bulk update operations for filtering and modifying multiple issues
//!
//! Provides unified update interface supporting both single-issue and batch modes.
//! Uses query filter engine to select issues and applies operations atomically per-issue.

use super::*;
use crate::domain::{Issue, Priority, State};
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
    ) -> Result<BulkUpdateResult> {
        let all_issues = self.storage.list_issues()?;
        let matched = filter.filter_issues(&all_issues)?;

        let mut result = BulkUpdateResult::new();
        result.matched = matched.iter().map(|i| i.id.clone()).collect();

        for issue in matched {
            match self.apply_operations_to_issue(issue, operations) {
                Ok(modified) => {
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
    /// Returns Ok(true) if modifications were made, Ok(false) if no changes, Err on failure.
    ///
    /// Note: State transitions bypass update_issue_state() to avoid duplicate validation,
    /// but we still log IssueStateChanged events for consistency.
    fn apply_operations_to_issue(
        &mut self,
        issue: &Issue,
        operations: &UpdateOperations,
    ) -> Result<bool> {
        // Validate first
        self.validate_update(issue, operations)?;

        // Check if any changes needed
        let changes = self.compute_changes(issue, operations)?;
        if changes.is_empty() {
            return Ok(false);
        }

        // Load fresh copy of issue
        let mut updated = self.storage.load_issue(&issue.id)?;

        let mut modified_fields = Vec::new();

        // Apply state change
        //
        // DESIGN DECISION: Bulk operations use literal state transitions.
        //
        // Unlike single-issue updates (`update_issue_state()`), bulk updates do NOT:
        // - Run prechecks automatically (Ready → InProgress)
        // - Run postchecks automatically (Gated state)
        // - Auto-transition to Gated when attempting Done with unpassed gates
        //
        // Rationale:
        // 1. Predictability: Users get exactly the state they specify (no surprises)
        // 2. Performance: Avoiding gate execution for many issues
        // 3. Safety: Explicit control for large-scale changes
        // 4. Composability: Users can layer operations (bulk update → bulk gate check)
        // 5. Precedent: Bulk tools (SQL UPDATE, jq, sed) use literal semantics
        //
        // Validation still occurs (dependencies, gate requirements) but no
        // automatic gate execution or state orchestration.
        //
        // See issue 40f594a7 for full decision rationale.
        if let Some(new_state) = operations.state {
            if updated.state != new_state {
                let old_state = updated.state;
                updated.state = new_state;
                modified_fields.push("state".to_string());

                // Log state change event (in addition to update event)
                // This maintains consistency with single-issue state transitions
                self.storage.append_event(&Event::new_issue_state_changed(
                    issue.id.clone(),
                    old_state,
                    new_state,
                ))?;
            }
        }

        // Apply label changes
        for label in &operations.add_labels {
            if !updated.labels.contains(label) {
                updated.labels.push(label.clone());
                modified_fields.push(format!("label:+{}", label));
            }
        }

        for label in &operations.remove_labels {
            if let Some(pos) = updated.labels.iter().position(|l| l == label) {
                updated.labels.remove(pos);
                modified_fields.push(format!("label:-{}", label));
            }
        }

        // Apply gate changes
        for gate_key in &operations.add_gates {
            if !updated.gates_required.contains(gate_key) {
                updated.gates_required.push(gate_key.clone());
                modified_fields.push(format!("gate:+{}", gate_key));
            }
        }

        for gate_key in &operations.remove_gates {
            if let Some(pos) = updated.gates_required.iter().position(|g| g == gate_key) {
                updated.gates_required.remove(pos);
                updated.gates_status.remove(gate_key);
                modified_fields.push(format!("gate:-{}", gate_key));
            }
        }

        // Apply assignee change
        if let Some(ref assignee) = operations.assignee {
            if updated.assignee.as_ref() != Some(assignee) {
                updated.assignee = Some(assignee.clone());
                modified_fields.push("assignee".to_string());
            }
        } else if operations.unassign && updated.assignee.is_some() {
            updated.assignee = None;
            modified_fields.push("assignee".to_string());
        }

        // Apply priority change
        if let Some(new_priority) = operations.priority {
            if updated.priority != new_priority {
                updated.priority = new_priority;
                modified_fields.push("priority".to_string());
            }
        }

        // Save if modified
        if !modified_fields.is_empty() {
            self.storage.save_issue(&updated)?;

            // Log update event
            self.storage.append_event(&Event::new_issue_updated(
                issue.id.clone(),
                "bulk-update".to_string(),
                modified_fields.clone(),
            ))?;
        }

        Ok(!modified_fields.is_empty())
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

            // Check for validation errors
            if let Err(e) = self.validate_update(issue, operations) {
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
        if let Some(ref assignee) = operations.assignee {
            if issue.assignee.as_ref() != Some(assignee) {
                changes.push(format!(
                    "assignee: {} → {}",
                    issue.assignee.as_deref().unwrap_or("none"),
                    assignee
                ));
            }
        } else if operations.unassign && issue.assignee.is_some() {
            changes.push(format!(
                "assignee: {} → none",
                issue.assignee.as_deref().unwrap_or("none")
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

    /// Validate that update can be applied to issue
    fn validate_update(&self, issue: &Issue, operations: &UpdateOperations) -> Result<()> {
        // Validate label operations
        if !operations.add_labels.is_empty() || !operations.remove_labels.is_empty() {
            let label_namespaces = self.config_manager.get_namespaces()?;
            crate::labels::validate_label_operations(
                &issue.labels,
                &operations.add_labels,
                &operations.remove_labels,
                &label_namespaces.namespaces,
            )?;
        }

        // Validate gate operations - check that gates exist in registry
        if !operations.add_gates.is_empty() {
            let registry = self.storage.load_gate_registry()?;
            for gate_key in &operations.add_gates {
                if !registry.gates.contains_key(gate_key) {
                    return Err(anyhow::anyhow!("Gate '{}' not found in registry", gate_key));
                }
            }
        }

        // Validate assignee format
        if let Some(ref assignee) = operations.assignee {
            crate::labels::validate_assignee_format(assignee)?;
        }

        // Validate state transition
        if let Some(new_state) = operations.state {
            // Check if blocked by dependencies
            if matches!(new_state, State::Ready | State::Done) {
                let all_issues = self.storage.list_issues()?;
                let context = crate::query_engine::QueryContext::from_issues(&all_issues);

                if issue.is_blocked(&context.all_issues) {
                    return Err(anyhow::anyhow!(
                        "Cannot transition to {:?}: blocked by dependencies",
                        new_state
                    ));
                }
            }

            // Check gates for Done state
            if new_state == State::Done && issue.has_unpassed_gates() {
                return Err(anyhow::anyhow!(
                    "Cannot transition to Done: {} gates pending",
                    issue.get_unpassed_gates().len()
                ));
            }
        }

        Ok(())
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
            crate::commands::CommandExecutor::new(crate::storage::InMemoryStorage::new());

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
            crate::commands::CommandExecutor::new(crate::storage::InMemoryStorage::new());

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
            crate::commands::CommandExecutor::new(crate::storage::InMemoryStorage::new());

        let changes = executor.compute_changes(&issue, &ops).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_apply_bulk_update_single_issue() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create test issue
        let issue = create_test_issue("test-1", State::Ready, vec!["type:task"]);
        storage.save_issue(&issue).unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Apply bulk update
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
            .save_issue(&create_test_issue("1", State::Ready, vec![]))
            .unwrap();
        storage
            .save_issue(&create_test_issue("2", State::Ready, vec![]))
            .unwrap();
        storage
            .save_issue(&create_test_issue("3", State::InProgress, vec![]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Update all ready issues
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
            .save_issue(&create_test_issue("1", State::Done, vec![]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to set same state
        let filter = QueryFilter::parse("state:done").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
        storage.save_issue(&issue).unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to transition to Done without passing gates
        let filter = QueryFilter::parse("state:gated").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("gates pending"));
    }

    #[test]
    fn test_apply_bulk_update_best_effort() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create mix of valid and invalid issues
        storage
            .save_issue(&create_test_issue("1", State::Ready, vec![]))
            .unwrap();

        let mut blocked = create_test_issue("2", State::Ready, vec![]);
        blocked.gates_required = vec!["tests".to_string()];
        storage.save_issue(&blocked).unwrap();

        storage
            .save_issue(&create_test_issue("3", State::Ready, vec![]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to transition all to Done
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
            .save_issue(&create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to add label without colon (invalid format)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["bad_label_no_colon".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

        // Should reject with error
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("format"));
    }

    #[test]
    fn test_bulk_update_rejects_duplicate_unique_namespace() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Issue already has type:task label
        storage
            .save_issue(&create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to add another type:* label (violates uniqueness)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["type:epic".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

        // Should reject with error
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("unique namespace"));
    }

    #[test]
    fn test_bulk_update_rejects_invalid_assignee_format() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage
            .save_issue(&create_test_issue("1", State::Ready, vec![]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Try to set assignee without colon (invalid format)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("invalid_no_colon".to_string()),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
            .save_issue(&create_test_issue("1", State::Ready, vec![]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Valid assignee format should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("agent:copilot".to_string()),
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

        // Should succeed
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify assignee set correctly
        let updated = executor.get_issue("1").unwrap();
        assert_eq!(updated.assignee, Some("agent:copilot".to_string()));
    }

    #[test]
    fn test_bulk_update_accepts_valid_labels() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage
            .save_issue(&create_test_issue("1", State::Ready, vec!["type:task"]))
            .unwrap();

        let mut executor = crate::commands::CommandExecutor::new(storage);

        // Valid labels should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string(), "epic:auth".to_string()],
            ..Default::default()
        };

        let result = executor.apply_bulk_update(&filter, &ops).unwrap();

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
