//! Dependency graph operations

use super::*;

/// Result of adding multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>, // (id, reason)
    pub errors: Vec<(String, String)>,  // (id, error message)
    /// The same per-dependency failures as their original typed errors, in the
    /// same order as [`errors`](Self::errors). Preserved (not serialized) so the
    /// CLI can classify a failure (cycle vs not-found) by downcasting the typed
    /// error instead of scanning its message text. The human-readable string in
    /// `errors` remains the serialized / displayed form.
    #[serde(skip)]
    pub typed_errors: Vec<(String, anyhow::Error)>,
}

/// Result of removing multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Add a dependency to an issue.
    ///
    /// Returns (result, warnings) tuple where warnings contains lease warnings if any.
    pub fn add_dependency(
        &self,
        issue_id: &str,
        dep_id: &str,
    ) -> Result<(DependencyAddResult, Vec<String>)> {
        // Resolve both IDs first
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_issue_id)? {
            warnings.push(warning);
        }

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
            return Ok((DependencyAddResult::Skipped { reason }, warnings));
        }

        // Load the issue and add dependency
        let mut from_issue = self.storage.load_issue(&full_issue_id)?;
        if from_issue.dependencies.contains(&full_dep_id.to_string()) {
            return Ok((DependencyAddResult::AlreadyExists, warnings));
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

        // If issue becomes blocked by this dependency, demote it to Backlog.
        //
        // INTENTIONAL direct state write (does NOT route through
        // `apply_state_transition`): this is an automatic invariant-maintaining
        // demotion, not a user-initiated forward transition. Adding a not-yet-done
        // dependency to a Ready issue MUST move it to Backlog to keep the DAG
        // invariant (a Ready issue cannot have an incomplete dependency). Subjecting
        // this to graph-rule enforcement could BLOCK the demotion and leave the
        // issue Ready with an unmet dependency — a corrupt state. So it bypasses the
        // chokepoint deliberately.
        let dep_issue = self.storage.load_issue(&full_dep_id)?;
        let from_id = from_issue.id.clone();
        if from_issue.state == State::Ready && dep_issue.state != State::Done {
            let old_state = from_issue.state;
            from_issue.state = State::Backlog;

            self.storage.save_issue(from_issue)?;

            // Log state change
            let event = Event::new_issue_state_changed(from_id.clone(), old_state, State::Backlog);
            self.storage.append_event(&event)?;
        } else {
            self.storage.save_issue(from_issue)?;
        }

        // Adding a dependency edits the issue; record the field edit (after the
        // save) so the change is event-logged. Reached only when a NEW edge is
        // actually added (AlreadyExists / transitive-redundant returned earlier),
        // so this mirrors `update_issue` and `remove_dependency(_ies)`.
        let event = Event::new_issue_updated(
            from_id,
            "dependency-add".to_string(),
            vec!["dependencies".to_string()],
        );
        self.storage.append_event(&event)?;

        Ok((DependencyAddResult::Added, warnings))
    }

    /// Remove a dependency from an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<Vec<String>> {
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_issue_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_issue_id)?;
        let before = issue.dependencies.len();
        issue.dependencies.retain(|d| d != &full_dep_id);
        let removed = issue.dependencies.len() != before;

        // Only persist (and event-log) when an edge was actually removed: a no-op
        // removal (edge absent) must not bump `updated_at` or emit an event, the
        // same no-change contract as `update_issue`. Removing an edge can unblock
        // the issue, so the readiness check runs only on the real-change path.
        if removed {
            self.storage.save_issue(issue)?;
            let event = Event::new_issue_updated(
                full_issue_id.clone(),
                "dependency-remove".to_string(),
                vec!["dependencies".to_string()],
            );
            self.storage.append_event(&event)?;
            self.auto_transition_to_ready(&full_issue_id)?;
        }

        Ok(warnings)
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
            typed_errors: Vec::new(),
        };

        // Try to add each dependency individually
        for dep_id in dep_ids {
            match self.add_dependency(issue_id, dep_id) {
                Ok((DependencyAddResult::Added, _warnings)) => {
                    result.added.push(dep_id.clone());
                }
                Ok((DependencyAddResult::AlreadyExists, _warnings)) => {
                    result.already_exist.push(dep_id.clone());
                }
                Ok((DependencyAddResult::Skipped { reason }, _warnings)) => {
                    result.skipped.push((dep_id.clone(), reason));
                }
                Err(e) => {
                    // Keep both representations in lockstep: the message for
                    // serialization / display, and the typed error for the CLI
                    // to classify by downcast.
                    result.errors.push((dep_id.clone(), e.to_string()));
                    result.typed_errors.push((dep_id.clone(), e));
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

        // Only persist (and event-log) when at least one edge was actually
        // removed: a no-op `jit dep rm` (every target absent) must not bump
        // `updated_at` or emit an event, the same no-change contract as
        // `update_issue`. Removing edges can unblock the issue, so the readiness
        // check runs only on the real-change path.
        if !removed.is_empty() {
            self.storage.save_issue(issue)?;
            let event = Event::new_issue_updated(
                full_issue_id.clone(),
                "dependency-remove".to_string(),
                vec!["dependencies".to_string()],
            );
            self.storage.append_event(&event)?;
            self.auto_transition_to_ready(&full_issue_id)?;
        }

        Ok(DependenciesRemoveResult { removed, not_found })
    }
}
