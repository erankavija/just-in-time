//! Issue query operations
//!
//! This module provides CLI orchestration wrappers around domain query functions.
//! The wrappers handle storage access and delegate to pure domain functions.

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn query_ready(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_ready(&issues))
    }

    pub fn query_blocked(
        &self,
    ) -> Result<Vec<(Issue, Vec<crate::domain::queries::BlockingReason>)>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_blocked(&issues))
    }

    pub fn query_by_assignee(&self, assignee: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_assignee(&issues, assignee))
    }

    pub fn query_by_state(&self, state: State) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_state(&issues, state))
    }

    pub fn query_by_priority(&self, priority: Priority) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_priority(&issues, priority))
    }

    pub fn query_by_label(&self, pattern: &str) -> Result<Vec<Issue>> {
        validate_label_pattern(pattern)?;
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_label(&issues, pattern))
    }

    /// Query issues matching every given label pattern (AND-combined).
    ///
    /// Each pattern is validated individually with the same rules as
    /// [`Self::query_by_label`] (the first invalid pattern's error is
    /// returned). Matching is AND-combined: an issue is kept only when it
    /// matches every pattern. An empty `patterns` slice matches every issue.
    pub fn query_by_labels(&self, patterns: &[String]) -> Result<Vec<Issue>> {
        for pattern in patterns {
            validate_label_pattern(pattern)?;
        }

        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_labels(&issues, patterns))
    }

    pub fn query_strategic(&self) -> Result<Vec<Issue>> {
        let namespaces = self.config_manager.get_namespaces()?;

        // Get strategic types from config, or fall back to hierarchy-based approach
        let strategic_types: Vec<String> = if let Some(ref types) = namespaces.strategic_types {
            types.clone()
        } else {
            // Fallback: use hierarchy levels 1-2
            let type_hierarchy = namespaces.get_type_hierarchy();
            type_hierarchy
                .iter()
                .filter(|(_, &level)| level <= 2)
                .map(|(type_name, _)| type_name.clone())
                .collect()
        };

        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_strategic(
            &issues,
            &strategic_types,
        ))
    }

    pub fn query_closed(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        Ok(crate::domain::queries::query_closed(&issues))
    }

    /// Query all issues with optional filters
    ///
    /// `label_filters` is repeatable and AND-combined: an issue is kept only
    /// when it matches every pattern given. An empty slice applies no label
    /// filtering, matching the pre-repeatable single-label behavior exactly.
    pub fn query_all(
        &self,
        state_filter: Option<State>,
        assignee_filter: Option<&str>,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let mut issues = self.storage.list_issues()?;

        // Apply filters
        if let Some(state) = state_filter {
            issues.retain(|i| i.state == state);
        }
        if let Some(assignee) = assignee_filter {
            issues.retain(|i| i.assignee.as_ref().is_some_and(|a| a == assignee));
        }
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if !label_filters.is_empty() {
            let label_matches = self.query_by_labels(label_filters)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }

    /// Query available issues with optional filters (unassigned + state=ready + unblocked)
    ///
    /// `label_filters` is repeatable and AND-combined; see [`Self::query_all`].
    pub fn query_available(
        &self,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_ready()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if !label_filters.is_empty() {
            let label_matches = self.query_by_labels(label_filters)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        // Sort by priority (Critical > High > Normal > Low)
        issues.sort_by_key(|i| match i.priority {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        });

        Ok(issues)
    }

    /// Query blocked issues with optional filters
    ///
    /// `label_filters` is repeatable and AND-combined; see [`Self::query_all`].
    pub fn query_blocked_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<(Issue, Vec<crate::domain::queries::BlockingReason>)>> {
        let mut blocked = self.query_blocked()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            blocked.retain(|(i, _)| i.priority == priority);
        }
        if !label_filters.is_empty() {
            let label_matches = self.query_by_labels(label_filters)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            blocked.retain(|(i, _)| label_ids.contains(i.id.as_str()));
        }

        Ok(blocked)
    }

    /// Query strategic issues with optional filters
    ///
    /// `label_filters` is repeatable and AND-combined; see [`Self::query_all`].
    pub fn query_strategic_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_strategic()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if !label_filters.is_empty() {
            let label_matches = self.query_by_labels(label_filters)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }

    /// Aggregate a label bucket into the counts-by-state
    /// [`StateRollup`](crate::output::StateRollup) behind `jit query count --by
    /// state`.
    ///
    /// The bucket is every issue matching all `label_filters` (AND-combined; an
    /// empty slice aggregates the whole repository), the advisory-grouping
    /// counterpart to the DAG-authoritative
    /// [`issue_progress`](Self::issue_progress). Patterns are validated with the
    /// same rules as [`Self::query_by_labels`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::{Priority, State};
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// let (done, _) = executor
    ///     .create_issue("Shipped".into(), String::new(), Priority::Normal,
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    /// executor
    ///     .create_issue("Todo".into(), String::new(), Priority::Normal,
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    /// executor
    ///     .update_issue(&done, None, None, None, Some(State::Done),
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    ///
    /// // No label filter aggregates the whole repository.
    /// let rollup = executor.query_count_by_state(&[]).unwrap();
    /// assert_eq!(rollup.total, 2);
    /// assert_eq!(rollup.done, 1);
    /// assert_eq!(rollup.percent, 50);
    /// // Every lifecycle state has a bucket, zero-count states included.
    /// assert_eq!(rollup.by_state.len(), State::all().len());
    /// ```
    pub fn query_count_by_state(
        &self,
        label_filters: &[String],
    ) -> Result<crate::output::StateRollup> {
        let issues = self.query_by_labels(label_filters)?;
        Ok(crate::output::StateRollup::from_issues(&issues))
    }

    /// Query closed issues with optional filters
    ///
    /// `label_filters` is repeatable and AND-combined; see [`Self::query_all`].
    pub fn query_closed_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_closed()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if !label_filters.is_empty() {
            let label_matches = self.query_by_labels(label_filters)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }
}

/// Validate a label filter pattern's format (`namespace:value` or
/// `namespace:*`). Shared by [`CommandExecutor::query_by_label`] and
/// [`CommandExecutor::query_by_labels`] so both single- and multi-pattern
/// callers reject malformed patterns identically.
fn validate_label_pattern(pattern: &str) -> Result<()> {
    if !pattern.contains(':') {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Invalid label pattern '{}': must be 'namespace:value' or 'namespace:*'",
            pattern
        ))
        .into());
    }

    let parts: Vec<&str> = pattern.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Invalid label pattern '{}': must contain exactly one colon",
            pattern
        ))
        .into());
    }

    let namespace = parts[0];

    if !namespace
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Invalid label pattern '{}': namespace must be lowercase alphanumeric with hyphens",
            pattern
        ))
        .into());
    }

    Ok(())
}
