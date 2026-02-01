//! Dependency graph operations

use super::*;

/// Result of adding multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>, // (id, reason)
    pub errors: Vec<(String, String)>,  // (id, error message)
}

/// Result of removing multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<DependencyAddResult> {
        // Resolve both IDs first
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        // Require active lease for structural operations
        self.require_active_lease(&full_issue_id)?;

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

        // Require active lease for structural operations
        self.require_active_lease(&full_issue_id)?;

        let mut issue = self.storage.load_issue(&full_issue_id)?;
        issue.dependencies.retain(|d| d != &full_dep_id);
        self.storage.save_issue(&issue)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(&full_issue_id)?;

        Ok(())
    }

    /// Add multiple dependencies to an issue
    pub fn add_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesAddResult> {
        // Validate input
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }

        let mut result = DependenciesAddResult {
            added: Vec::new(),
            already_exist: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        };

        // Try to add each dependency individually
        for dep_id in dep_ids {
            match self.add_dependency(issue_id, dep_id) {
                Ok(DependencyAddResult::Added) => {
                    result.added.push(dep_id.clone());
                }
                Ok(DependencyAddResult::AlreadyExists) => {
                    result.already_exist.push(dep_id.clone());
                }
                Ok(DependencyAddResult::Skipped { reason }) => {
                    result.skipped.push((dep_id.clone(), reason));
                }
                Err(e) => {
                    result.errors.push((dep_id.clone(), e.to_string()));
                }
            }
        }

        Ok(result)
    }

    /// Remove multiple dependencies from an issue
    pub fn remove_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesRemoveResult> {
        // Validate input
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }

        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_issue_id)?;

        let mut removed = Vec::new();
        let mut not_found = Vec::new();

        for dep_id in dep_ids {
            // Resolve the dependency ID
            match self.storage.resolve_issue_id(dep_id) {
                Ok(full_dep_id) => {
                    if issue.dependencies.contains(&full_dep_id) {
                        issue.dependencies.retain(|d| d != &full_dep_id);
                        removed.push(dep_id.clone());
                    } else {
                        not_found.push(dep_id.clone());
                    }
                }
                Err(_) => {
                    not_found.push(dep_id.clone());
                }
            }
        }

        // Save issue once with all removals
        self.storage.save_issue(&issue)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(&full_issue_id)?;

        Ok(DependenciesRemoveResult { removed, not_found })
    }
}
