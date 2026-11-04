//! Issue search operations

use super::*;
use crate::search::{SearchOptions, SearchResult};
use std::collections::{HashMap, HashSet};

/// A ripgrep content match, paired with the title of the issue its file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMatch {
    pub result: SearchResult,
    /// `None` for matches outside `issues/` files and for issues that fail to load.
    pub issue_title: Option<String>,
}

impl<S: IssueStore> QueryOps<'_, S> {
    /// Search file contents under the storage root with ripgrep.
    ///
    /// Each match carries the title of its issue, loaded once per issue. Matches
    /// in documents, and in issues that cannot be loaded, carry no title.
    ///
    /// # Errors
    ///
    /// Returns the [`SearchError`](crate::search::SearchError) that ripgrep
    /// reports, unchanged, so callers can downcast it.
    pub fn search_content(&self, query: &str, options: SearchOptions) -> Result<Vec<ContentMatch>> {
        let results = crate::search::search(self.exec.storage.root(), query, options)?;
        let titles: HashMap<IssueId, String> = results
            .iter()
            .filter_map(|result| result.issue_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .filter_map(|id| {
                let title = self.exec.storage.load_issue(&id).ok()?.title;
                Some((id, title))
            })
            .collect();

        Ok(results
            .into_iter()
            .map(|result| {
                let issue_title = result
                    .issue_id
                    .as_ref()
                    .and_then(|id| titles.get(id).cloned());
                ContentMatch {
                    result,
                    issue_title,
                }
            })
            .collect())
    }

    pub fn search_issues(&self, query: &str) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
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
                    || issue.id.as_str().starts_with(&query_lower)
            })
            .collect();

        Ok(results)
    }

    /// Search issues by text `query`, then narrow by optional filters.
    ///
    /// An empty `query` matches every issue, so callers can pass `""` to search
    /// across the whole repository and rely on the filters alone. The
    /// `label_filters` are ANDed: an issue is kept only when it carries every
    /// requested label pattern (`namespace:value` or `namespace:*`, as in
    /// `query --label`). The priority, state, and assignee filters each keep an
    /// issue only when it matches exactly.
    ///
    /// # Errors
    ///
    /// Returns `InvalidLabelPatternError` for a malformed `label_filters` entry.
    pub fn search_issues_with_filters(
        &self,
        query: &str,
        priority_filter: Option<Priority>,
        state_filter: Option<State>,
        assignee_filter: Option<crate::domain::Assignee>,
        label_filters: &[String],
    ) -> Result<Vec<Issue>> {
        label_filters
            .iter()
            .try_for_each(|pattern| crate::labels::validate_label_pattern(pattern))?;
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

        Ok(crate::domain::queries::query_by_labels(
            &results,
            label_filters,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::search::SearchOptions;
    use crate::test_harness::TestHarness;

    #[test]
    fn test_search_content_attaches_issue_titles() {
        let root = tempfile::TempDir::new().unwrap();
        let harness = TestHarness::rooted_at(root.path());
        let id = harness.create_issue("Needle issue");
        let issues_dir = root.path().join("issues");
        std::fs::create_dir_all(&issues_dir).unwrap();
        std::fs::write(issues_dir.join(format!("{id}.json")), "needle\n").unwrap();

        let matches = harness
            .executor
            .queries()
            .search_content("needle", SearchOptions::default())
            .unwrap();

        let [only] = matches.as_slice() else {
            panic!("expected exactly one match, got {}", matches.len());
        };
        assert_eq!(only.result.issue_id.as_ref(), Some(&id));
        assert_eq!(only.issue_title.as_deref(), Some("Needle issue"));
    }

    #[test]
    fn test_search_content_leaves_title_absent_outside_issue_files() {
        let root = tempfile::TempDir::new().unwrap();
        let harness = TestHarness::rooted_at(root.path());
        let docs_dir = root.path().join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("design.md"), "needle\n").unwrap();

        let matches = harness
            .executor
            .queries()
            .search_content("needle", SearchOptions::default())
            .unwrap();

        let [only] = matches.as_slice() else {
            panic!("expected exactly one match, got {}", matches.len());
        };
        assert_eq!(only.result.issue_id, None);
        assert_eq!(only.issue_title, None);
    }
}
