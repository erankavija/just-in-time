//! Bulk update operations for filtering and modifying multiple issues
//!
//! Provides unified update interface supporting both single-issue and batch modes.
//! Uses query filter engine to select issues and applies operations atomically per-issue.

use super::*;
use crate::domain::{Issue, Priority, State};
use crate::query::QueryFilter;
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
        // Validate state transition
        if let Some(new_state) = operations.state {
            // Check if blocked by dependencies
            if matches!(new_state, State::Ready | State::Done) {
                let all_issues = self.storage.list_issues()?;
                let context = crate::query::QueryContext::from_issues(&all_issues);

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

        // Label validation would go here
        // (hierarchy checks, etc.)

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
}
