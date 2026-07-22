//! Label operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    /// Get issue by ID.
    ///
    /// Note: Helper method for label operations, part of public API.
    #[allow(dead_code)]
    pub fn get_issue(&self, id: &str) -> Result<Issue> {
        self.storage.load_issue(id)
    }

    /// Add a label to an issue.
    ///
    /// Returns warnings from validation if any.
    ///
    /// Note: Part of public API, may be used by external consumers.
    #[allow(dead_code)]
    pub fn add_label(&self, issue_id: &str, label: &str) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // This path is only reached internally from `jit issue reject --reason`
        // (adding a `resolution:` label); reject deliberately BYPASSES rule
        // ENFORCEMENT, so label format / uniqueness / registry checks are NOT
        // applied as blockers here — they live solely in the default rule set and
        // are surfaced below only as non-blocking WARNINGS (a0f0f342 migration: no
        // inline `validate_label` / uniqueness reject remains).
        Ok(self
            .publish_captured_field_update(CapturedFieldUpdate::label_edit(
                issue_id.to_string(),
                CapturedLabelEdit::AppendUnchecked(label.to_string()),
            ))?
            .warnings)
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

#[cfg(test)]
mod tests {
    use crate::commands::test_helpers::{memory_executor, with_open_race, OpenRaceAction};
    use crate::commands::CommandExecutor;
    use crate::domain::Priority;
    use crate::storage::{InMemoryStorage, IssueStore};

    fn create_issue(executor: &CommandExecutor<InMemoryStorage>, title: &str) -> String {
        executor
            .create_issue(
                title.to_string(),
                String::new(),
                Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )
            .unwrap()
            .0
    }

    #[test]
    fn test_add_label_recaptures_after_concurrent_issue_change() {
        let storage = InMemoryStorage::new();
        let executor = memory_executor(storage.clone());
        let id = create_issue(&executor, "Label race");

        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".to_string());
        let raced = with_open_race(storage, 2, OpenRaceAction::Save(Box::new(concurrent)));
        let executor = memory_executor(raced.clone());

        executor.add_label(&id, "resolution:done").unwrap();

        let labels = raced.load_issue(&id).unwrap().labels;
        assert!(labels.iter().any(|label| label == "owner:concurrent"));
        assert!(labels.iter().any(|label| label == "resolution:done"));
    }

    #[test]
    fn test_add_label_preserves_unchecked_duplicate_append_behavior() {
        let storage = InMemoryStorage::new();
        let executor = memory_executor(storage.clone());
        let id = create_issue(&executor, "Duplicate label");

        executor.add_label(&id, "resolution:done").unwrap();
        executor.add_label(&id, "resolution:done").unwrap();

        assert_eq!(
            storage
                .load_issue(&id)
                .unwrap()
                .labels
                .iter()
                .filter(|label| label.as_str() == "resolution:done")
                .count(),
            2
        );
    }
}
