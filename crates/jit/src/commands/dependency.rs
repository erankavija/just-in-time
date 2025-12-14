//! Dependency graph operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<DependencyAddResult> {
        // Load all issues and build graph for analysis
        // Note: Storage layer handles locking internally
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        // Validate type hierarchy BEFORE cycle check
        // This ensures organizational structure is enforced orthogonally to DAG
        let from_issue = self.storage.load_issue(issue_id)?;
        let to_issue = self.storage.load_issue(dep_id)?;

        // Extract type labels from both issues
        let from_type = from_issue
            .labels
            .iter()
            .find(|l| l.starts_with("type:"))
            .map(String::as_str);
        let to_type = to_issue
            .labels
            .iter()
            .find(|l| l.starts_with("type:"))
            .map(String::as_str);

        // Only validate hierarchy if both issues have type labels
        if let (Some(from), Some(to)) = (from_type, to_type) {
            let config = HierarchyConfig::default();
            type_hierarchy::validate_hierarchy(&config, from, to)
                .map_err(|e| anyhow!("Type hierarchy violation: {}", e))?;
        }

        // Check for cycles (DAG validation)
        graph.validate_add_dependency(issue_id, dep_id)?;

        // Check if this dependency is transitive (redundant)
        if graph.is_transitive(issue_id, dep_id) {
            let reason = "transitive (already reachable via other dependencies)".to_string();
            return Ok(DependencyAddResult::Skipped { reason });
        }

        // Load the issue and add dependency
        let mut issue = from_issue;
        if issue.dependencies.contains(&dep_id.to_string()) {
            return Ok(DependencyAddResult::AlreadyExists);
        }

        issue.dependencies.push(dep_id.to_string());

        // Apply transitive reduction: remove any deps now reachable through others
        // Build a temporary graph with the new edge to compute reduction
        let temp_issue = issue.clone();
        let mut temp_issues = issues.clone();
        temp_issues.retain(|i| i.id != issue_id);
        temp_issues.push(temp_issue);
        let temp_refs: Vec<&Issue> = temp_issues.iter().collect();
        let new_graph = DependencyGraph::new(&temp_refs);
        let new_reduced = new_graph.compute_transitive_reduction(issue_id);
        issue.dependencies = new_reduced.into_iter().collect();

        // If issue becomes blocked by this dependency, transition to Backlog
        let dep_issue = to_issue;
        if issue.state == State::Ready && dep_issue.state != State::Done {
            let old_state = issue.state;
            issue.state = State::Backlog;
            self.storage.save_issue(&issue)?;

            // Log state change
            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Backlog);
            self.storage.append_event(&event)?;
        } else {
            self.storage.save_issue(&issue)?;
        }

        Ok(DependencyAddResult::Added)
    }

    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        issue.dependencies.retain(|d| d != dep_id);
        self.storage.save_issue(&issue)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(issue_id)?;

        Ok(())
    }
}
