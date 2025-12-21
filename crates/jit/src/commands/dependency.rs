//! Dependency graph operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<DependencyAddResult> {
        // Resolve both IDs first
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        // Load all issues and build graph for analysis
        // Note: Storage layer handles locking internally
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        // Check for cycles (DAG validation)
        graph.validate_add_dependency(&full_issue_id, &full_dep_id)?;

        // Check if this dependency is transitive (redundant)
        if graph.is_transitive(&full_issue_id, &full_dep_id) {
            let reason = "transitive (already reachable via other dependencies)".to_string();
            return Ok(DependencyAddResult::Skipped { reason });
        }

        // Load the issue and add dependency
        let mut from_issue = self.storage.load_issue(&full_issue_id)?;
        if from_issue.dependencies.contains(&full_dep_id.to_string()) {
            return Ok(DependencyAddResult::AlreadyExists);
        }

        from_issue.dependencies.push(full_dep_id.to_string());

        // Apply transitive reduction: remove any deps now reachable through others
        // Build a temporary graph with the new edge to compute reduction
        let temp_issue = from_issue.clone();
        let mut temp_issues = issues.clone();
        temp_issues.retain(|i| i.id != full_issue_id);
        temp_issues.push(temp_issue);
        let temp_refs: Vec<&Issue> = temp_issues.iter().collect();
        let new_graph = DependencyGraph::new(&temp_refs);
        let new_reduced = new_graph.compute_transitive_reduction(&full_issue_id);
        from_issue.dependencies = new_reduced.into_iter().collect();

        // If issue becomes blocked by this dependency, transition to Backlog
        let dep_issue = self.storage.load_issue(&full_dep_id)?;
        if from_issue.state == State::Ready && dep_issue.state != State::Done {
            let old_state = from_issue.state;
            from_issue.state = State::Backlog;
            self.storage.save_issue(&from_issue)?;

            // Log state change
            let event =
                Event::new_issue_state_changed(from_issue.id.clone(), old_state, State::Backlog);
            self.storage.append_event(&event)?;
        } else {
            self.storage.save_issue(&from_issue)?;
        }

        Ok(DependencyAddResult::Added)
    }

    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        let mut issue = self.storage.load_issue(&full_issue_id)?;
        issue.dependencies.retain(|d| d != &full_dep_id);
        self.storage.save_issue(&issue)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(&full_issue_id)?;

        Ok(())
    }
}
