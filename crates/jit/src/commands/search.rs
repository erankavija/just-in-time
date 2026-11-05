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

/// Options for matching semantic issue fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct SemanticSearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
}

/// An issue and the semantic field which matched the query.
pub struct SemanticIssueMatch {
    pub issue: Issue,
    pub text: String,
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

    /// Match the ID, title and description of each issue.
    pub fn search_issue_matches(
        &self,
        query: &str,
        options: SemanticSearchOptions,
    ) -> Result<Vec<SemanticIssueMatch>> {
        let regex = if options.regex {
            Some(
                regex::RegexBuilder::new(query)
                    .case_insensitive(!options.case_sensitive)
                    .build()
                    .map_err(|error| {
                        crate::errors::InvalidArgumentError::new(format!(
                            "invalid search regex: {error}"
                        ))
                    })?,
            )
        } else {
            None
        };
        let needle = if options.case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        let matches = |value: &str, id: bool| {
            if let Some(regex) = &regex {
                return regex.is_match(value);
            }
            let value = if options.case_sensitive {
                value.to_string()
            } else {
                value.to_lowercase()
            };
            if id {
                value.starts_with(&needle)
            } else {
                value.contains(&needle)
            }
        };
        Ok(self
            .exec
            .storage
            .list_issues()?
            .into_iter()
            .filter_map(|issue| {
                let text = [
                    issue.title.as_str(),
                    issue.description.as_str(),
                    issue.id.as_str(),
                ]
                .into_iter()
                .enumerate()
                .find(|(index, field)| matches(field, *index == 2))
                .map(|(_, field)| field.to_string());
                text.map(|text| SemanticIssueMatch { issue, text })
            })
            .collect())
    }

    pub fn search_issues_with_options(
        &self,
        query: &str,
        options: SemanticSearchOptions,
    ) -> Result<Vec<Issue>> {
        Ok(self
            .search_issue_matches(query, options)?
            .into_iter()
            .map(|item| item.issue)
            .collect())
    }

    pub fn search_issues(&self, query: &str) -> Result<Vec<Issue>> {
        self.search_issues_with_options(query, SemanticSearchOptions::default())
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
    fn test_search_issues_with_options_matches_semantic_fields_and_id_prefix() {
        let harness = TestHarness::new();
        let id = harness.create_issue("Alpha Beacon");
        let options = super::SemanticSearchOptions {
            case_sensitive: true,
            regex: true,
        };
        let matches = harness
            .executor
            .queries()
            .search_issues_with_options("^Alpha B.*", options)
            .unwrap();
        assert!(matches.iter().any(|issue| issue.id == id));
        assert!(harness
            .executor
            .queries()
            .search_issues_with_options("^alpha", options)
            .unwrap()
            .is_empty());
        assert!(harness
            .executor
            .queries()
            .search_issues(&id.to_string()[..8])
            .unwrap()
            .iter()
            .any(|issue| issue.id == id));
        assert!(harness
            .executor
            .queries()
            .search_issues("lpha bea")
            .unwrap()
            .iter()
            .any(|issue| issue.id == id));
        assert!(harness
            .executor
            .queries()
            .search_issues_with_options("[", options)
            .is_err());
    }

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
