//! Issue query operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn query_ready(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let ready: Vec<Issue> = issues
            .iter()
            .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
            .cloned()
            .collect();

        Ok(ready)
    }

    pub fn query_blocked(&self) -> Result<Vec<(Issue, Vec<String>)>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let mut blocked = Vec::new();

        for issue in &issues {
            if issue.is_blocked(&resolved) {
                let mut reasons = Vec::new();

                // Check dependencies
                for dep_id in &issue.dependencies {
                    if let Some(dep) = resolved.get(dep_id) {
                        if dep.state != State::Done {
                            reasons.push(format!(
                                "dependency:{} ({}:{:?})",
                                dep_id, dep.title, dep.state
                            ));
                        }
                    }
                }

                // Check gates
                for gate_key in &issue.gates_required {
                    let gate_state = issue.gates_status.get(gate_key);
                    let is_passed = gate_state
                        .map(|gs| gs.status == GateStatus::Passed)
                        .unwrap_or(false);

                    if !is_passed {
                        let status_str = gate_state
                            .map(|gs| format!("{:?}", gs.status))
                            .unwrap_or_else(|| "Pending".to_string());
                        reasons.push(format!("gate:{} ({})", gate_key, status_str));
                    }
                }

                blocked.push((issue.clone(), reasons));
            }
        }

        Ok(blocked)
    }

    pub fn query_by_assignee(&self, assignee: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues
            .into_iter()
            .filter(|i| i.assignee.as_deref() == Some(assignee))
            .collect();

        Ok(filtered)
    }

    pub fn query_by_state(&self, state: State) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues.into_iter().filter(|i| i.state == state).collect();

        Ok(filtered)
    }

    pub fn query_by_priority(&self, priority: Priority) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues
            .into_iter()
            .filter(|i| i.priority == priority)
            .collect();

        Ok(filtered)
    }

    pub fn query_by_label(&self, pattern: &str) -> Result<Vec<Issue>> {
        use crate::labels;

        // Validate pattern format
        if !pattern.contains(':') {
            return Err(anyhow!(
                "Invalid label pattern '{}': must be 'namespace:value' or 'namespace:*'",
                pattern
            ));
        }

        let parts: Vec<&str> = pattern.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "Invalid label pattern '{}': must contain exactly one colon",
                pattern
            ));
        }

        let namespace = parts[0];

        // Validate namespace format
        if !namespace
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(anyhow!(
                "Invalid label pattern '{}': namespace must be lowercase alphanumeric with hyphens",
                pattern
            ));
        }

        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues
            .into_iter()
            .filter(|issue| labels::matches_pattern(&issue.labels, pattern))
            .collect();

        Ok(filtered)
    }

    pub fn query_strategic(&self) -> Result<Vec<Issue>> {
        use crate::labels as label_utils;

        let namespaces = self.config_manager.get_namespaces()?;

        // Get strategic types from config, or fall back to hierarchy-based approach
        let strategic_types: Vec<String> = if let Some(ref types) = namespaces.strategic_types {
            types.clone()
        } else {
            // Fallback: use hierarchy levels 1-2
            let type_hierarchy = namespaces.get_type_hierarchy();
            type_hierarchy
                .iter()
                .filter(|(_, &level)| level <= 2)
                .map(|(type_name, _)| type_name.clone())
                .collect()
        };

        if strategic_types.is_empty() {
            return Ok(Vec::new());
        }

        let issues = self.storage.list_issues()?;
        let filtered = issues
            .into_iter()
            .filter(|issue| {
                // Check if issue has type:X label where X is a strategic type
                issue.labels.iter().any(|label| {
                    if let Ok((ns, value)) = label_utils::parse_label(label) {
                        ns == "type" && strategic_types.contains(&value)
                    } else {
                        false
                    }
                })
            })
            .collect();

        Ok(filtered)
    }

    pub fn query_closed(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues.into_iter().filter(|i| i.state.is_closed()).collect();

        Ok(filtered)
    }

    /// Query all issues with optional filters
    pub fn query_all(
        &self,
        state_filter: Option<State>,
        assignee_filter: Option<&str>,
        priority_filter: Option<Priority>,
        label_filter: Option<&str>,
    ) -> Result<Vec<Issue>> {
        let mut issues = self.storage.list_issues()?;

        // Apply filters
        if let Some(state) = state_filter {
            issues.retain(|i| i.state == state);
        }
        if let Some(assignee) = assignee_filter {
            issues.retain(|i| i.assignee.as_deref() == Some(assignee));
        }
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if let Some(label_pattern) = label_filter {
            let label_matches = self.query_by_label(label_pattern)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }

    /// Query available issues with optional filters (unassigned + state=ready + unblocked)
    pub fn query_available(
        &self,
        priority_filter: Option<Priority>,
        label_filter: Option<&str>,
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_ready()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if let Some(label_pattern) = label_filter {
            let label_matches = self.query_by_label(label_pattern)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        // Sort by priority (Critical > High > Normal > Low)
        issues.sort_by_key(|i| match i.priority {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        });

        Ok(issues)
    }

    /// Query blocked issues with optional filters
    pub fn query_blocked_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filter: Option<&str>,
    ) -> Result<Vec<(Issue, Vec<String>)>> {
        let mut blocked = self.query_blocked()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            blocked.retain(|(i, _)| i.priority == priority);
        }
        if let Some(label_pattern) = label_filter {
            let label_matches = self.query_by_label(label_pattern)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            blocked.retain(|(i, _)| label_ids.contains(i.id.as_str()));
        }

        Ok(blocked)
    }

    /// Query strategic issues with optional filters
    pub fn query_strategic_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filter: Option<&str>,
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_strategic()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if let Some(label_pattern) = label_filter {
            let label_matches = self.query_by_label(label_pattern)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }

    /// Query closed issues with optional filters
    pub fn query_closed_filtered(
        &self,
        priority_filter: Option<Priority>,
        label_filter: Option<&str>,
    ) -> Result<Vec<Issue>> {
        let mut issues = self.query_closed()?;

        // Apply additional filters
        if let Some(priority) = priority_filter {
            issues.retain(|i| i.priority == priority);
        }
        if let Some(label_pattern) = label_filter {
            let label_matches = self.query_by_label(label_pattern)?;
            let label_ids: std::collections::HashSet<_> =
                label_matches.iter().map(|i| i.id.as_str()).collect();
            issues.retain(|i| label_ids.contains(i.id.as_str()));
        }

        Ok(issues)
    }
}
