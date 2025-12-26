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
    /// Note: Part of public API, may be used by external consumers.
    #[allow(dead_code)]
    pub fn add_label(&self, issue_id: &str, label: &str) -> Result<()> {
        use crate::labels as label_utils;

        // Validate label format
        label_utils::validate_label(label)?;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        // Check uniqueness constraint
        let (namespace, _) = label_utils::parse_label(label)?;
        let namespaces = self.storage.load_label_namespaces()?;

        if let Some(ns_config) = namespaces.get(&namespace) {
            if ns_config.unique {
                // Check if issue already has a label in this namespace
                for existing_label in &issue.labels {
                    if let Ok((existing_ns, _)) = label_utils::parse_label(existing_label) {
                        if existing_ns == namespace {
                            return Err(anyhow!(
                                "Issue already has label '{}' from unique namespace '{}'",
                                existing_label,
                                namespace
                            ));
                        }
                    }
                }
            }
        }

        issue.labels.push(label.to_string());
        self.storage.save_issue(&issue)?;
        Ok(())
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

    pub fn add_label_namespace(&self, name: &str, description: &str, unique: bool) -> Result<()> {
        let mut namespaces = self.storage.load_label_namespaces()?;
        namespaces.add(
            name.to_string(),
            crate::domain::LabelNamespace::new(description, unique),
        );
        self.storage.save_label_namespaces(&namespaces)?;
        Ok(())
    }
}
