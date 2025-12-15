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

    pub fn search_issues_with_filters(
        &self,
        query: &str,
        priority_filter: Option<Priority>,
        state_filter: Option<State>,
        assignee_filter: Option<String>,
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
            true
        });

        Ok(results)
    }
}
