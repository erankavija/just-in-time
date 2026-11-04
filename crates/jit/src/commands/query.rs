//! Issue query operations
//!
//! This module provides CLI orchestration wrappers around domain query functions.
//! The wrappers handle storage access and delegate to pure domain functions.

use super::*;
use crate::domain::queries::{query_all, AllFilter};

/// Query, search, and event-log operations of a [`CommandExecutor`].
pub struct QueryOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The query family.
    pub fn queries(&self) -> QueryOps<'_, S> {
        QueryOps { exec: self }
    }
}

impl<S: IssueStore> QueryOps<'_, S> {
    pub fn query_ready(&self) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
        Ok(crate::domain::queries::query_ready(&issues))
    }

    pub fn query_blocked(
        &self,
    ) -> Result<Vec<(Issue, Vec<crate::domain::queries::BlockingReason>)>> {
        let issues = self.exec.storage.list_issues()?;
        Ok(crate::domain::queries::query_blocked(&issues))
    }

    pub fn query_by_assignee(&self, assignee: &crate::domain::Assignee) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
        Ok(crate::domain::queries::query_by_assignee(&issues, assignee))
    }

    pub fn query_strategic(&self) -> Result<Vec<Issue>> {
        let namespaces = self.exec.config_manager.get_namespaces()?;

        // Get strategic types from config, or fall back to hierarchy-based approach
        let strategic_types: Vec<String> = if let Some(ref types) = namespaces.strategic_types {
            types.clone()
        } else {
            // Fallback: use hierarchy levels 1-2
            let type_hierarchy = namespaces.declared_type_hierarchy();
            type_hierarchy
                .iter()
                .filter(|(_, &level)| level <= 2)
                .map(|(type_name, _)| type_name.clone())
                .collect()
        };

        let issues = self.exec.storage.list_issues()?;
        Ok(crate::domain::queries::query_strategic(
            &issues,
            &strategic_types,
        ))
    }

    pub fn query_closed(&self) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
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
        assignee_filter: Option<crate::domain::Assignee>,
        priority_filter: Option<Priority>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
        label_filters
            .iter()
            .try_for_each(|pattern| crate::labels::validate_label_pattern(pattern))?;
        Ok(crate::domain::queries::query_all(
            &issues,
            crate::domain::queries::AllFilter {
                state: state_filter,
                assignee: assignee_filter,
                priority: priority_filter,
                labels: label_filters,
            },
        ))
    }

    /// Query available work, narrowed by every supplied field filter.
    pub fn query_available(&self, filter: AllFilter<'_>) -> Result<Vec<Issue>> {
        filter
            .labels
            .iter()
            .try_for_each(|pattern| crate::labels::validate_label_pattern(pattern))?;
        let issues = self.exec.storage.list_issues()?;
        let available = crate::domain::queries::query_available(
            &issues,
            crate::domain::queries::AvailableFilter::default(),
        );
        Ok(query_all(&available, filter))
    }

    /// Query blocked issues, narrowed by every supplied field filter.
    pub fn query_blocked_filtered(
        &self,
        filter: AllFilter<'_>,
    ) -> Result<Vec<(Issue, Vec<crate::domain::queries::BlockingReason>)>> {
        let matches = self.query_all(
            filter.state,
            filter.assignee,
            filter.priority,
            filter.labels,
        )?;
        let ids: std::collections::HashSet<_> = matches.iter().map(|issue| &issue.id).collect();
        Ok(self
            .query_blocked()?
            .into_iter()
            .filter(|(issue, _)| ids.contains(&issue.id))
            .collect())
    }

    /// Query strategic issues, narrowed by every supplied field filter.
    pub fn query_strategic_filtered(&self, filter: AllFilter<'_>) -> Result<Vec<Issue>> {
        filter
            .labels
            .iter()
            .try_for_each(|pattern| crate::labels::validate_label_pattern(pattern))?;
        Ok(query_all(&self.query_strategic()?, filter))
    }

    /// Aggregate filtered issues into the counts-by-state
    /// [`StateRollup`](crate::output::StateRollup) behind `jit query count --by
    /// state`.
    ///
    /// Every supplied field filter narrows the bucket. Label patterns use the
    /// same validation as [`Self::query_all`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::{Priority, State};
    /// use jit::storage::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.add_data_file("config.toml", "");
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// let done = executor
    ///     .issues().create_issue("Shipped".into(), String::new(), Priority::Normal,
    ///         vec![], vec![], None, None, false)
    ///     .unwrap().issue.id;
    /// executor
    ///     .issues().create_issue("Todo".into(), String::new(), Priority::Normal,
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    /// executor
    ///     .issues().update_issue(&done, None, None, None, Some(State::Done),
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    ///
    /// // No filters aggregates the whole repository.
    /// let rollup = executor.queries().query_count_by_state(Default::default()).unwrap();
    /// assert_eq!(rollup.extras.total, 2);
    /// assert_eq!(rollup.extras.done, 1);
    /// assert_eq!(rollup.extras.percent, 50);
    /// // Every lifecycle state has a bucket, zero-count states included.
    /// assert_eq!(rollup.items.len(), State::ALL.len());
    /// ```
    pub fn query_count_by_state(
        &self,
        filter: AllFilter<'_>,
    ) -> Result<crate::output::StateRollup> {
        let issues = self.query_all(
            filter.state,
            filter.assignee,
            filter.priority,
            filter.labels,
        )?;
        Ok(crate::output::StateRollup::from_issues(&issues))
    }

    /// Query closed issues, narrowed by every supplied field filter.
    pub fn query_closed_filtered(&self, filter: AllFilter<'_>) -> Result<Vec<Issue>> {
        filter
            .labels
            .iter()
            .try_for_each(|pattern| crate::labels::validate_label_pattern(pattern))?;
        Ok(query_all(&self.query_closed()?, filter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_available_delegates_filters_to_domain() {
        let executor =
            crate::commands::test_helpers::memory_executor(crate::storage::InMemoryStorage::new());
        let mut issue = crate::test_utils::fixture_issue("Available".to_string(), String::new());
        issue.state = State::Ready;
        issue.priority = Priority::High;
        issue.labels = vec!["component:api".to_string()];
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let result = executor
            .queries()
            .query_available(AllFilter {
                priority: Some(Priority::High),
                labels: &["component:api".to_string()],
                ..AllFilter::default()
            })
            .unwrap();
        assert_eq!(result.iter().map(|i| &i.id).collect::<Vec<_>>(), [&id]);
        assert!(executor
            .queries()
            .query_available(AllFilter {
                priority: Some(Priority::Low),
                labels: &["component:api".to_string()],
                ..AllFilter::default()
            })
            .unwrap()
            .is_empty());
        assert!(executor
            .queries()
            .query_available(AllFilter {
                priority: None,
                labels: &["component:web".to_string()],
                ..AllFilter::default()
            })
            .unwrap()
            .is_empty());
        assert!(executor
            .queries()
            .query_available(AllFilter {
                priority: None,
                labels: &["invalid".to_string()],
                ..AllFilter::default()
            })
            .is_err());
    }
}
