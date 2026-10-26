//! Label operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    /// Get issue by ID.
    ///
    /// Note: Helper method for label operations, part of public API.
    #[allow(dead_code)]
    pub fn get_issue(&self, id: &IssueId) -> Result<Issue> {
        self.storage.load_issue(id)
    }

    pub fn list_label_values(&self, namespace: &str) -> Result<Vec<String>> {
        let issues = self.storage.list_issues()?;
        let mut values = std::collections::HashSet::new();

        for issue in issues {
            for label in &issue.labels {
                if let Ok((ns, value)) = label_utils::parse_label(label) {
                    if ns == namespace {
                        values.insert(value.to_string());
                    }
                }
            }
        }

        let mut result: Vec<String> = values.into_iter().collect();
        result.sort();
        Ok(result)
    }
}
