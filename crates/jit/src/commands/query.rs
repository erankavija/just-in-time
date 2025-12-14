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
        use crate::labels as label_utils;

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
        let value = parts[1];

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
        let filtered: Vec<Issue> = if value == "*" {
            // Wildcard: match all labels in this namespace
            issues
                .into_iter()
                .filter(|issue| {
                    issue.labels.iter().any(|label| {
                        if let Ok((ns, _)) = label_utils::parse_label(label) {
                            ns == namespace
                        } else {
                            false
                        }
                    })
                })
                .collect()
        } else {
            // Exact match
            issues
                .into_iter()
                .filter(|issue| issue.labels.contains(&pattern.to_string()))
                .collect()
        };

        Ok(filtered)
    }

    pub fn query_strategic(&self) -> Result<Vec<Issue>> {
        use crate::labels as label_utils;

        let namespaces = self.storage.load_label_namespaces()?;
        let strategic_namespaces: Vec<String> = namespaces
            .namespaces
            .iter()
            .filter(|(_, ns)| ns.strategic)
            .map(|(name, _)| name.clone())
            .collect();

        if strategic_namespaces.is_empty() {
            return Ok(Vec::new());
        }

        let issues = self.storage.list_issues()?;
        let filtered = issues
            .into_iter()
            .filter(|issue| {
                issue.labels.iter().any(|label| {
                    if let Ok((ns, _)) = label_utils::parse_label(label) {
                        strategic_namespaces.contains(&ns)
                    } else {
                        false
                    }
                })
            })
            .collect();

        Ok(filtered)
    }
}
