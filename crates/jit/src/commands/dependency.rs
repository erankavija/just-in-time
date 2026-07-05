//! Dependency graph operations

use super::*;
use crate::errors::{DependencyBatchRejectedError, RedundantDependencyError};
use std::collections::HashSet;

/// Result of adding multiple dependencies.
///
/// All-or-nothing (jit:c8518f2a): [`add_dependencies_with_policy`](CommandExecutor::add_dependencies_with_policy)
/// returns this only when every requested edge validated. A batch with any
/// rejected edge instead returns `Err(`[`DependencyBatchRejectedError`]`)` and
/// leaves the dependency set — and the event log — completely unchanged, so
/// there is no per-batch `errors` field to populate here.
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>, // (id, reason)
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

    /// Add multiple dependencies under an explicit [`RedundancyPolicy`], atomically.
    ///
    /// All-or-nothing (jit:c8518f2a): every requested edge is validated —
    /// id resolution, then cycle detection and (under
    /// [`RedundancyPolicy::Reject`]) the transitive-reduction check, all against
    /// the WOULD-BE final graph with every edge of this call applied at once —
    /// before anything is written. If any edge fails, `Err(`[`DependencyBatchRejectedError`]`)`
    /// names every rejected edge (REQ-02) and the dependency set, and the event
    /// log, are left completely unchanged (REQ-01, REQ-03). Only when every
    /// edge validates are the surviving edges applied and event-logged in one
    /// pass.
    ///
    /// Validating the full candidate graph up front (rather than one edge at a
    /// time against the graph as it stood before this call) also catches a
    /// violation that only emerges from the COMBINATION of two edges in the
    /// same call — e.g. two sibling edges that are each fine alone but jointly
    /// make one of them transitively redundant.
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
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }

        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;

        // `DependenciesAddResult` carries no warnings field (matching the
        // batch's pre-existing contract); the call still matters for its
        // `Strict`-mode error, so its `Option<String>` warning is discarded.
        self.require_active_lease(&full_issue_id)?;

        let issues = self.storage.list_issues()?;
        let original_deps_of = |id: &str| -> HashSet<String> {
            issues
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.dependencies.iter().cloned().collect())
                .unwrap_or_default()
        };
        let from_deps = original_deps_of(&full_issue_id);

        // ---- Phase 1: resolve every target id. -----------------------------
        // A dep_id that fails to resolve (too-short prefix, ambiguous, or not
        // found) is rejected now: there is no full id to place in the
        // candidate graph phase 2 validates, so it can never reach phase 2.
        // This runs for every dep_id regardless of whether an earlier one
        // already failed, so a batch mixing a resolution failure with a
        // graph-validation failure names both (REQ-02).
        let mut already_exist: Vec<String> = Vec::new();
        let mut new_edges: Vec<(String, String)> = Vec::new(); // (as-supplied text, full id)
        let mut rejected: Vec<(String, anyhow::Error)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for dep_id in dep_ids {
            match self.storage.resolve_issue_id(dep_id) {
                Ok(full_dep_id) => {
                    if from_deps.contains(&full_dep_id) || !seen.insert(full_dep_id.clone()) {
                        already_exist.push(dep_id.clone());
                    } else {
                        new_edges.push((dep_id.clone(), full_dep_id));
                    }
                }
                Err(e) => rejected.push((dep_id.clone(), e)),
            }
        }

        // ---- Phase 2: validate every resolved edge against the WOULD-BE ----
        // final graph: every edge of this call applied to `full_issue_id` at
        // once, not one at a time.
        let mut candidate_issues = issues.clone();
        if let Some(from) = candidate_issues.iter_mut().find(|i| i.id == full_issue_id) {
            for (_, full_dep_id) in &new_edges {
                from.dependencies.push(full_dep_id.clone());
            }
        }
        let candidate_refs: Vec<&Issue> = candidate_issues.iter().collect();
        let candidate_graph = DependencyGraph::new(&candidate_refs);

        // Cycle check. The underlying reachability check starts at the
        // TARGET and stops the instant it visits `full_issue_id` — it never
        // inspects `full_issue_id`'s own outgoing edges — so validating
        // against the graph that already carries every sibling pending edge
        // gives the identical answer as validating one at a time. This is the
        // literal would-be-final graph the doc comment above promises.
        for (dep_id_text, full_dep_id) in &new_edges {
            if let Err(e) = candidate_graph.validate_add_dependency(&full_issue_id, full_dep_id) {
                rejected.push((dep_id_text.clone(), e.into()));
            }
        }
        let cycle_failed: HashSet<&str> = rejected
            .iter()
            .filter(|(text, _)| new_edges.iter().any(|(t, _)| t == text))
            .map(|(text, _)| text.as_str())
            .collect();

        // Redundancy check, only meaningful once the candidate graph is
        // acyclic: a cyclic candidate graph makes "redundant" undefined, and
        // any cycle failure above already dooms the whole batch, so skip it
        // when one occurred.
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut reduced_from = from_deps.clone();

        if cycle_failed.is_empty() && !new_edges.is_empty() {
            let redundant = candidate_graph.find_redundant_edges();

            // Self-redundant: one of OUR new edges is itself already reachable
            // via `full_issue_id`'s other dependencies. Precisely attributable
            // to that one edge; under `Reduce` it is silently dropped
            // (Skipped), under `Reject` it names that edge.
            let self_redundant: HashSet<&str> = redundant
                .iter()
                .filter(|(from, to)| {
                    from == &full_issue_id && new_edges.iter().any(|(_, full_id)| full_id == to)
                })
                .map(|(_, to)| to.as_str())
                .collect();

            let mut rejected_texts: HashSet<&str> = HashSet::new();
            for (dep_id_text, full_dep_id) in &new_edges {
                if self_redundant.contains(full_dep_id.as_str()) {
                    match policy {
                        RedundancyPolicy::Reject => {
                            rejected.push((
                                dep_id_text.clone(),
                                RedundantDependencyError::new(
                                    (full_issue_id.clone(), full_dep_id.clone()),
                                    redundant.clone(),
                                )
                                .into(),
                            ));
                            rejected_texts.insert(dep_id_text.as_str());
                        }
                        RedundancyPolicy::Reduce => skipped.push((
                            dep_id_text.clone(),
                            "transitive (already reachable via other dependencies)".to_string(),
                        )),
                    }
                }
            }

            // Any OTHER redundant pair — this batch's edge(s) shadow a
            // pre-existing edge on a DIFFERENT node (jit:7a50e021's original
            // "shadows an existing edge" case, e.g. a new Y -> Z edge making a
            // pre-existing X -> Z edge redundant via X -> Y -> Z). Under
            // `Reject` this rejects the whole batch too: attribute it to
            // every new edge not already accounted for above, since (unlike
            // the self-redundant case) the specific edge responsible cannot
            // always be isolated from a shared alternate path. Under
            // `Reduce`, nothing needs doing here: `apply_batch_dependency_add`
            // drops the shadowed edge from the OTHER node automatically.
            let shadows_existing_edge = redundant.iter().any(|(from, to)| {
                !(from == &full_issue_id && self_redundant.contains(to.as_str()))
            });
            if policy == RedundancyPolicy::Reject && shadows_existing_edge {
                for (dep_id_text, full_dep_id) in &new_edges {
                    if !rejected_texts.contains(dep_id_text.as_str()) {
                        rejected.push((
                            dep_id_text.clone(),
                            RedundantDependencyError::new(
                                (full_issue_id.clone(), full_dep_id.clone()),
                                redundant.clone(),
                            )
                            .into(),
                        ));
                    }
                }
            }

            reduced_from = candidate_graph.compute_transitive_reduction(&full_issue_id);
        }

        if !rejected.is_empty() {
            return Err(DependencyBatchRejectedError::new(full_issue_id, rejected).into());
        }

        // ---- Nothing rejected: apply. ---------------------------------------
        if reduced_from == from_deps {
            // Every requested edge reduced away (all skipped, or there was
            // nothing new to add): the dependency set is unchanged, so there is
            // nothing to persist or event-log.
            return Ok(DependenciesAddResult {
                added: Vec::new(),
                already_exist,
                skipped,
            });
        }

        let added_full_ids: HashSet<String> =
            reduced_from.difference(&from_deps).cloned().collect();
        let added: Vec<String> = new_edges
            .iter()
            .filter(|(_, full_id)| added_full_ids.contains(full_id))
            .map(|(text, _)| text.clone())
            .collect();

        self.apply_batch_dependency_add(&issues, &candidate_issues, &full_issue_id, &reduced_from)?;

        Ok(DependenciesAddResult {
            added,
            already_exist,
            skipped,
        })
    }

    /// Persist a validated batch dependency add whose candidate graph is
    /// reduced in place.
    ///
    /// Mirrors [`apply_reduced_dependency_add`](Self::apply_reduced_dependency_add)
    /// but for a whole batch of new edges applied to `full_issue_id` at once:
    /// sets `full_issue_id`'s dependencies to `reduced_from` (its transitive
    /// reduction over `candidate_issues`), demotes it when any newly-added
    /// dependency isn't `Done`, and drops now-redundant edges from every other
    /// node exactly like the single-edge path. Only called once every edge in
    /// the batch has already validated, so this never partially applies.
    fn apply_batch_dependency_add(
        &self,
        original_issues: &[Issue],
        candidate_issues: &[Issue],
        full_issue_id: &str,
        reduced_from: &HashSet<String>,
    ) -> Result<()> {
        let candidate_refs: Vec<&Issue> = candidate_issues.iter().collect();
        let graph = DependencyGraph::new(&candidate_refs);

        let original_deps = |id: &str| -> HashSet<String> {
            original_issues
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.dependencies.iter().cloned().collect())
                .unwrap_or_default()
        };

        // --- Source issue: gains the reduced set of new edges.
        let old_from_deps = original_deps(full_issue_id);
        let mut from_issue = self.storage.load_issue(full_issue_id)?;
        from_issue.dependencies = reduced_from.iter().cloned().collect();
        let from_id = from_issue.id.clone();

        // If any newly-added dependency isn't Done, a Ready issue must demote
        // to Backlog (INV: a Ready issue cannot have an incomplete
        // dependency). See the single-edge path for why this bypasses
        // `apply_state_transition` deliberately.
        let mut any_incomplete = false;
        for full_dep_id in reduced_from.difference(&old_from_deps) {
            let dep_issue = self.storage.load_issue(full_dep_id)?;
            if dep_issue.state != State::Done {
                any_incomplete = true;
                break;
            }
        }

        if from_issue.state == State::Ready && any_incomplete {
            let old_state = from_issue.state;
            from_issue.state = State::Backlog;
            self.storage.save_issue(from_issue)?;
            let event = Event::new_issue_state_changed(from_id.clone(), old_state, State::Backlog);
            self.storage.append_event(&event)?;
        } else {
            self.storage.save_issue(from_issue)?;
        }

        let event = Event::new_issue_updated(
            from_id,
            "dependency-add".to_string(),
            vec!["dependencies".to_string()],
        );
        self.storage.append_event(&event)?;

        // --- Other nodes: drop the edges the new edges made redundant.
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

        Ok(())
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
