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
    pub fn add_label(&self, issue_id: &str, label: &str) -> Result<Vec<String>> {
        use crate::labels as label_utils;

        // Validate label format
        label_utils::validate_label(label)?;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        // Check uniqueness constraint. This path is only reached internally from
        // `jit issue reject --reason` (adding a `resolution:` label); reject
        // deliberately bypasses rule enforcement, so only the legacy validator
        // runs here, not `.jit/rules.toml` local rules. Config/namespaces come
        // from the executor cache so they are not re-parsed from disk per call.
        let (namespace, _) = label_utils::parse_label(label)?;
        let namespaces = self.cached_namespaces()?;

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

        // Validate the updated issue with the legacy validator (config from cache).
        let mut warnings = Vec::new();
        let config = self.cached_config()?;
        if let Some(ref validation_config) = config.validation {
            let validator = crate::validation::IssueValidator::new(
                validation_config.clone(),
                namespaces.clone(),
            );
            warnings = validator.validate(&issue)?;
        }

        self.storage.save_issue(issue)?;
        Ok(warnings)
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
