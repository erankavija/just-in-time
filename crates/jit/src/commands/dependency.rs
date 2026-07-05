//! Dependency graph operations

use super::*;
use crate::errors::RedundantDependencyError;

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
    /// Add a dependency to an issue, dropping any now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependency_with_policy`](Self::add_dependency_with_policy) with
    /// [`RedundancyPolicy::Reduce`]: the resulting graph is left acyclic and
    /// transitively reduced, which internal callers (templates, breakdown,
    /// batch-create) rely on. Returns `(result, warnings)` where `warnings`
    /// carries any lease warning.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::InMemoryStorage;
    ///
    /// let executor = CommandExecutor::new(InMemoryStorage::new());
    /// let (_result, _warnings) = executor.add_dependency("epic-1", "task-2")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn add_dependency(
        &self,
        issue_id: &str,
        dep_id: &str,
    ) -> Result<(DependencyAddResult, Vec<String>)> {
        self.add_dependency_with_policy(issue_id, dep_id, RedundancyPolicy::Reduce)
    }

    /// Add a dependency under an explicit transitive-reduction [`RedundancyPolicy`].
    ///
    /// Cycle detection runs first (the existing write-time guard,
    /// INV-DAG-ACYCLIC). The candidate graph (the current issues plus the new
    /// edge) is then checked for transitive-reduction violations with the same
    /// [`DependencyGraph::find_redundant_edges`] property `jit validate` enforces:
    ///
    /// * [`RedundancyPolicy::Reject`] — if the edge shadows an existing direct
    ///   edge or is itself already reachable, the add fails with a
    ///   [`RedundantDependencyError`](crate::errors::RedundantDependencyError)
    ///   naming the offending edge pair; nothing is written.
    /// * [`RedundancyPolicy::Reduce`] — the edge is added and every now-redundant
    ///   edge is dropped in the same operation, leaving the graph reduced.
    ///
    /// A non-redundant edge is added normally under either policy.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{CommandExecutor, RedundancyPolicy};
    /// use jit::storage::InMemoryStorage;
    ///
    /// let executor = CommandExecutor::new(InMemoryStorage::new());
    /// let result = executor.add_dependency_with_policy("b", "c", RedundancyPolicy::Reject);
    /// # let _ = result;
    /// ```
    pub fn add_dependency_with_policy(
        &self,
        issue_id: &str,
        dep_id: &str,
        policy: RedundancyPolicy,
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

        // Check for cycles (DAG validation) — the existing write-time guard.
        graph.validate_add_dependency(&full_issue_id, &full_dep_id)?;

        // Idempotent: an already-present edge is a no-op (no reduction, no event).
        let from_issue = self.storage.load_issue(&full_issue_id)?;
        if from_issue.dependencies.contains(&full_dep_id.to_string()) {
            return Ok((DependencyAddResult::AlreadyExists, warnings));
        }

        // Build the candidate graph WITH the new edge and reuse the same
        // transitive-reduction property `jit validate` enforces
        // (`find_redundant_edges`) to detect any violation the edge would
        // introduce BEFORE persisting it — the write-time analogue of cycle
        // detection above.
        let mut candidate_issues = issues.clone();
        if let Some(candidate_from) = candidate_issues.iter_mut().find(|i| i.id == full_issue_id) {
            candidate_from.dependencies.push(full_dep_id.to_string());
        }
        let candidate_refs: Vec<&Issue> = candidate_issues.iter().collect();
        let candidate_graph = DependencyGraph::new(&candidate_refs);
        let redundant = candidate_graph.find_redundant_edges();

        if !redundant.is_empty() && policy == RedundancyPolicy::Reject {
            return Err(RedundantDependencyError::new(
                (full_issue_id.clone(), full_dep_id.clone()),
                redundant,
            )
            .into());
        }

        // Non-redundant edge (redundant empty) OR Reduce policy: apply the add and
        // drop any now-redundant edge(s), leaving the graph transitively reduced.
        self.apply_reduced_dependency_add(
            &issues,
            &candidate_issues,
            &full_issue_id,
            &full_dep_id,
            warnings,
        )
    }

    /// Persist a dependency add whose candidate graph is reduced in place.
    ///
    /// Sets every changed node's dependencies to its transitive reduction over
    /// the candidate graph, demotes the source issue when the new edge blocks it,
    /// and event-logs each change. The source issue gains the new edge (a
    /// `dependency-add` update event) unless the edge was itself redundant, in
    /// which case it is a no-op [`DependencyAddResult::Skipped`]. Other nodes only
    /// ever LOSE now-redundant edges; because dropping a redundant edge preserves
    /// reachability, no readiness transition is needed for them — each such drop
    /// is recorded as a `dependency_reduced` event, mirroring `jit validate --fix`.
    fn apply_reduced_dependency_add(
        &self,
        original_issues: &[Issue],
        candidate_issues: &[Issue],
        full_issue_id: &str,
        full_dep_id: &str,
        warnings: Vec<String>,
    ) -> Result<(DependencyAddResult, Vec<String>)> {
        use std::collections::HashSet;

        let candidate_refs: Vec<&Issue> = candidate_issues.iter().collect();
        let graph = DependencyGraph::new(&candidate_refs);

        let original_deps = |id: &str| -> HashSet<String> {
            original_issues
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.dependencies.iter().cloned().collect())
                .unwrap_or_default()
        };

        // --- Source issue: gains the new edge unless the edge is itself redundant.
        let reduced_from = graph.compute_transitive_reduction(full_issue_id);
        let result = if reduced_from == original_deps(full_issue_id) {
            // The new edge was itself redundant and nothing else on the source
            // changed: the reduced graph equals the original, so the add is a no-op.
            DependencyAddResult::Skipped {
                reason: "transitive (already reachable via other dependencies)".to_string(),
            }
        } else {
            let mut from_issue = self.storage.load_issue(full_issue_id)?;
            from_issue.dependencies = reduced_from.iter().cloned().collect();
            let from_id = from_issue.id.clone();

            // If the new dependency blocks a Ready issue, demote it to Backlog.
            //
            // INTENTIONAL direct state write (does NOT route through
            // `apply_state_transition`): this is an automatic invariant-maintaining
            // demotion, not a user-initiated forward transition. Adding a not-yet-done
            // dependency to a Ready issue MUST move it to Backlog to keep the DAG
            // invariant (a Ready issue cannot have an incomplete dependency).
            // Subjecting this to graph-rule enforcement could BLOCK the demotion and
            // leave the issue Ready with an unmet dependency — a corrupt state. So it
            // bypasses the chokepoint deliberately. Only the ADDED edge can demote;
            // the reachability-preserving edges dropped below never do.
            let dep_issue = self.storage.load_issue(full_dep_id)?;
            if from_issue.state == State::Ready && dep_issue.state != State::Done {
                let old_state = from_issue.state;
                from_issue.state = State::Backlog;

                self.storage.save_issue(from_issue)?;

                let event =
                    Event::new_issue_state_changed(from_id.clone(), old_state, State::Backlog);
                self.storage.append_event(&event)?;
            } else {
                self.storage.save_issue(from_issue)?;
            }

            // Record the field edit (after the save) so the change is event-logged,
            // mirroring `update_issue` and `remove_dependency(_ies)`.
            let event = Event::new_issue_updated(
                from_id,
                "dependency-add".to_string(),
                vec!["dependencies".to_string()],
            );
            self.storage.append_event(&event)?;
            DependencyAddResult::Added
        };

        // --- Other nodes: drop the edges the new edge made redundant.
        for candidate in candidate_issues {
            if candidate.id == full_issue_id {
                continue;
            }
            let reduced = graph.compute_transitive_reduction(&candidate.id);
            if reduced == original_deps(&candidate.id) {
                continue;
            }
            let mut issue = self.storage.load_issue(&candidate.id)?;
            let old_count = issue.dependencies.len();
            let removed: Vec<String> = issue
                .dependencies
                .iter()
                .filter(|d| !reduced.contains(*d))
                .cloned()
                .collect();
            issue.dependencies = reduced.iter().cloned().collect();
            let new_count = issue.dependencies.len();
            let issue_id = issue.id.clone();
            self.storage.save_issue(issue)?;

            let event = Event::new_dependency_reduced(issue_id, old_count, new_count, removed);
            self.storage.append_event(&event)?;
        }

        Ok((result, warnings))
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

    /// Add multiple dependencies to an issue, dropping now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependencies_with_policy`](Self::add_dependencies_with_policy) with
    /// [`RedundancyPolicy::Reduce`], preserving the eager-reduction behavior
    /// internal callers rely on.
    pub fn add_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesAddResult> {
        self.add_dependencies_with_policy(issue_id, dep_ids, RedundancyPolicy::Reduce)
    }

    /// Add multiple dependencies under an explicit [`RedundancyPolicy`].
    ///
    /// Each edge is added independently via
    /// [`add_dependency_with_policy`](Self::add_dependency_with_policy); a per-edge
    /// failure (cycle, missing node, or a rejected redundant edge under
    /// [`RedundancyPolicy::Reject`]) is collected into the result's `errors`
    /// rather than aborting the batch.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{CommandExecutor, RedundancyPolicy};
    /// use jit::storage::InMemoryStorage;
    ///
    /// let executor = CommandExecutor::new(InMemoryStorage::new());
    /// let deps = vec!["task-2".to_string()];
    /// let result = executor.add_dependencies_with_policy("epic-1", &deps, RedundancyPolicy::Reduce);
    /// # let _ = result;
    /// ```
    pub fn add_dependencies_with_policy(
        &self,
        issue_id: &str,
        dep_ids: &[String],
        policy: RedundancyPolicy,
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
            match self.add_dependency_with_policy(issue_id, dep_id, policy) {
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

    /// Remove multiple dependencies from an issue.
    ///
    /// This is what `jit dep rm <from> <id>...` calls. Each `dep_id` is
    /// matched directly against `issue`'s OWN stored `dependencies` (by exact
    /// id, or by normalized prefix once at least 4 characters are given) —
    /// NOT resolved through the repo-wide index first. A dependency whose
    /// target issue was deleted no longer resolves there, which previously
    /// made a dangling edge permanently unremovable via the CLI (jit:f847df3f);
    /// matching the raw stored id sidesteps that resolution step entirely and
    /// works whether the target is alive or gone. An exact stored id always
    /// wins; a prefix that matches more than one stored dependency is rejected
    /// as ambiguous rather than removing an arbitrary one.
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
            let normalized = dep_id.to_lowercase().replace('-', "");

            // An exact stored id is always unambiguous, even when it is also a
            // prefix of another stored id.
            let exact = issue
                .dependencies
                .iter()
                .find(|stored| stored.as_str() == dep_id.as_str())
                .cloned();

            let stored_match = if let Some(exact) = exact {
                Some(exact)
            } else {
                // No exact match: validate the target id the SAME way
                // `resolve_issue_id` validates `<from>`, so both id arguments of
                // `dep rm` are checked identically (jit:a05b87ae). A sub-4-char
                // prefix is a typed argument error (exit 2), not a silent
                // "not found" (exit 0).
                if normalized.len() < 4 {
                    return Err(crate::storage::InvalidIdPrefixError::new(dep_id.clone()).into());
                }

                // Match by normalized prefix, collecting ALL matches so an
                // ambiguous prefix is rejected the way `resolve_issue_id` rejects
                // it, rather than silently removing whichever edge happens to
                // appear first (jit:f847df3f).
                let matches: Vec<String> = issue
                    .dependencies
                    .iter()
                    .filter(|stored| {
                        stored
                            .to_lowercase()
                            .replace('-', "")
                            .starts_with(&normalized)
                    })
                    .cloned()
                    .collect();

                match matches.as_slice() {
                    [] => None,
                    [only] => Some(only.clone()),
                    _ => {
                        return Err(crate::storage::AmbiguousIdError::dependency(
                            dep_id.clone(),
                            matches,
                        )
                        .into());
                    }
                }
            };

            match stored_match {
                Some(full_dep_id) => {
                    issue.dependencies.retain(|d| d != &full_dep_id);
                    removed.push(dep_id.clone());
                }
                None => {
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
