//! Issue search operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn search_issues(&self, query: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let query_lower = query.to_lowercase();

        let results = issues
            .into_iter()
            .filter(|issue| {
                if query.is_empty() {
                    return true;
                }

                // Search in title, description, and ID
                issue.title.to_lowercase().contains(&query_lower)
                    || issue.description.to_lowercase().contains(&query_lower)
                    || issue.id.to_lowercase().starts_with(&query_lower)
            })
            .collect();

        Ok(results)
    }

    /// Search issues by text `query`, then narrow by optional filters.
    ///
    /// An empty `query` matches every issue, so callers can pass `""` to search
    /// across the whole repository and rely on the filters alone. The
    /// `label_filters` are ANDed: an issue is kept only when it carries every
    /// requested label. The priority, state, and assignee filters each keep an
    /// issue only when it matches exactly.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    ///
    /// // No text query: match all, then keep only issues labelled `type:epic`.
    /// let epics = executor.search_issues_with_filters(
    ///     "",
    ///     None,
    ///     None,
    ///     None,
    ///     &["type:epic".to_string()],
    /// )?;
    /// assert!(epics.iter().all(|i| i.labels.contains(&"type:epic".to_string())));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn search_issues_with_filters(
        &self,
        query: &str,
        priority_filter: Option<Priority>,
        state_filter: Option<State>,
        assignee_filter: Option<String>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        let mut results = self.search_issues(query)?;

        // Apply additional filters
        results.retain(|issue| {
            if let Some(ref priority) = priority_filter {
                if &issue.priority != priority {
                    return false;
                }
            }
            if let Some(ref state) = state_filter {
                if &issue.state != state {
                    return false;
                }
            }
            if let Some(ref assignee) = assignee_filter {
                if issue.assignee.as_ref() != Some(assignee) {
                    return false;
                }
            }
            // Label filters are ANDed: keep only issues carrying every label.
            if !label_filters
                .iter()
                .all(|wanted| issue.labels.contains(wanted))
            {
                return false;
            }
            true
        });

        Ok(results)
    }
}
