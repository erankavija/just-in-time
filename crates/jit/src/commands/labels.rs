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
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        // This path is only reached internally from `jit issue reject --reason`
        // (adding a `resolution:` label); reject deliberately BYPASSES rule
        // ENFORCEMENT, so label format / uniqueness / registry checks are NOT
        // applied as blockers here — they live solely in the default rule set and
        // are surfaced below only as non-blocking WARNINGS (a0f0f342 migration: no
        // inline `validate_label` / uniqueness reject remains).
        issue.labels.push(label.to_string());

        // Surface non-blocking warnings from the effective rule set (built-in
        // defaults + user rules). This path never blocks, so enforce findings are
        // reported as warnings too.
        let rules = self.effective_rules()?;
        let evaluation = crate::validation::evaluate_local(&issue, rules)
            .map_err(|err| anyhow!("rule evaluation failed: {err}"))?;
        let warnings: Vec<String> = evaluation
            .findings()
            .into_iter()
            .filter(|f| f.severity != crate::validation::rules::Severity::Off)
            .map(|f| format!("[{}] {}", f.rule, f.message))
            .collect();

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
