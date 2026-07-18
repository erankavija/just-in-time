//! Issue CRUD operations and lifecycle management

use super::*;
use crate::errors::{DeletionNotConfirmedError, TransitionBlockedError, TransitionBlocker};
use crate::storage::StorageWarning;

/// How `update_issue` should apply a new description value.
///
/// `Replace` overwrites the existing description entirely (`--description` /
/// `--description-file`). `Append` adds text to the end of whatever
/// description already exists (`--append-description` /
/// `--append-description-file`), separated by exactly one blank line; when
/// the existing description is empty the result is just the new text, with
/// no leading blank line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptionUpdate {
    Replace(String),
    Append(String),
}

impl DescriptionUpdate {
    /// Apply this operation against an existing description, returning the
    /// resulting description text.
    pub fn apply(self, existing: &str) -> String {
        match self {
            DescriptionUpdate::Replace(text) => text,
            DescriptionUpdate::Append(text) if existing.is_empty() => text,
            DescriptionUpdate::Append(text) => format!("{existing}\n\n{text}"),
        }
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Hard-reject an explicit `--type <kind>` whose kind is not declared in the
    /// configured `[type_hierarchy]`, deciding through the SAME rule engine the
    /// write path uses — NOT a parallel `config.toml` containment check.
    ///
    /// The `type-hierarchy-known` default rule (`validation::defaults`)
    /// reports an undeclared `type:<kind>` as an `error` finding, but because it
    /// is `enforce = false` it only WARNS on the normal write path
    /// (`validate_for_write`). An explicit `--type` is a deliberate, hard
    /// contract, so this PROMOTES that one rule's finding to a hard rejection for
    /// the explicit-type create/update path ONLY. The rule's global
    /// severity/enforce is untouched, so non-`--type` writes (and every other
    /// path) keep their existing warn-only behavior.
    ///
    /// Config stays the single source of truth via the existing validation layer:
    /// the decision is the rule engine's `type-hierarchy-known` finding
    /// for THIS issue's final shape. When no `[type_hierarchy]` is configured the
    /// behavior matches that rule. `issue` MUST already carry the derived
    /// `type:<kind>` label.
    fn reject_undeclared_type(&self, issue: &Issue) -> Result<()> {
        let rules = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let evaluation = crate::validation::evaluate_local(issue, rules, repo_format)
            .map_err(|err| anyhow!("rule evaluation failed: {err}"))?;
        if let Some(finding) = evaluation
            .findings()
            .into_iter()
            .find(|finding| finding.rule == "type-hierarchy-known")
        {
            return Err(crate::errors::ValidationFailedError::new(finding.message.clone()).into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        mut labels: Vec<String>,
        content_format: Option<crate::domain::ContentFormat>,
        issue_type: Option<String>,
        force: bool,
    ) -> Result<(String, Vec<String>)> {
        // Config comes from the executor cache so it is not re-parsed per call.
        // Label format and ALL namespace constraints (canonical format, uniqueness,
        // registry, etc.) are now enforced SOLELY by the effective rule set inside
        // `validate_for_write` (a0f0f342 migration) — no inline format/uniqueness
        // check remains here.
        let config = self.cached_config()?;

        // REQ-02: an explicit `--type <kind>` derives the canonical `type:<kind>`
        // label, replacing any caller-supplied `type:*` label. The command layer
        // owns this derivation (the CLI only forwards the typed value). Applied
        // BEFORE the default-type fallback so an explicit type suppresses the
        // default. Kind validity is enforced below via `reject_undeclared_type`,
        // which routes the decision through the rule engine on the final shape.
        let explicit_type = issue_type.is_some();
        if let Some(kind) = issue_type {
            labels.retain(|label| !label_utils::is_type_label(label));
            labels.push(label_utils::type_label(&kind));
        }

        // Apply the configured default type when the issue carries no `type:*`
        // label. This is a write-time CONVENIENCE (not validation enforcement), so
        // it survives the a0f0f342 migration that removed the hard-coded
        // validator. The `[validation].default_type` config field remains its
        // input until the config->rules migration (task 0abaddc0).
        if let Some(default_type) = config
            .validation
            .as_ref()
            .and_then(|v| v.default_type.as_deref())
        {
            let has_type = label_utils::type_label_value(&labels).is_some();
            if !has_type {
                labels.push(label_utils::type_label(default_type));
            }
        }

        let mut issue = Issue::new(title, description);
        issue.priority = priority;
        issue.gates_required = gates;
        issue.labels = labels;
        issue.content_format = content_format;

        // Auto-promote a brand-new issue to Ready if it has no dependencies
        // (gates don't block Ready), BEFORE validating, so rules keyed on the
        // final state (e.g. `when = { state = "ready" }`) see the shape that
        // will be persisted.
        //
        // INTENTIONAL direct state write (does NOT route through
        // `apply_state_transition`): this is the INITIAL state of an issue being
        // constructed, not a transition of an existing persisted issue. There is
        // no prior state to transition from, no dependency neighborhood yet (the
        // issue has no dependencies by construction here), and `validate_for_write`
        // below covers create-time validation.
        if issue.dependencies.is_empty() {
            issue.state = State::Ready;
            // This direct write is the issue's INITIAL Ready state, so it does not
            // pass through the `apply_state_transition` chokepoint that stamps
            // `first_ready_at` for later transitions. Stamp it here so a
            // dependency-free issue that is born Ready still records when it
            // became workable (first-occurrence semantics via `mark_first_ready`).
            issue.mark_first_ready(chrono::Utc::now());
        }

        // REQ-02: when `--type` was explicitly provided, hard-reject an undeclared
        // kind through the SAME rule engine the write uses
        // (`type-hierarchy-known`), which only warns on the normal path.
        // Scoped to the explicit-type path so non-`--type` writes keep warn-only
        // behavior; runs before the write so a bad type changes nothing.
        if explicit_type {
            self.reject_undeclared_type(&issue)?;
        }

        // Single write-time validation entry point: runs the legacy validator
        // plus the declarative local rules against the FINAL issue shape, and
        // defers any `--force` bypass events until after the save succeeds.
        let validation = self.validate_for_write(&issue, force)?;

        // Clone fields needed for event and return value before moving issue
        let issue_id = issue.id.clone();
        let title = issue.title.clone();
        let priority = issue.priority;

        self.storage.save_issue(issue)?;

        // Log event
        let event = Event::IssueCreated {
            id: uuid::Uuid::new_v4().to_string(),
            issue_id: issue_id.clone(),
            timestamp: chrono::Utc::now(),
            title,
            priority,
        };
        self.storage.append_event(&event)?;

        // Emit bypass events only AFTER the issue write committed.
        self.log_rule_bypasses(&issue_id, &validation.bypassed_rules)?;

        Ok((issue_id, validation.warnings))
    }

    pub fn list_issues(
        &self,
        state_filter: Option<State>,
        assignee_filter: Option<String>,
        priority_filter: Option<Priority>,
    ) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;

        let filtered = issues
            .into_iter()
            .filter(|issue| {
                if let Some(ref state) = state_filter {
                    if &issue.state != state {
                        return false;
                    }
                }
                if let Some(ref assignee) = assignee_filter {
                    if !issue
                        .assignee
                        .as_ref()
                        .is_some_and(|a| a == assignee.as_str())
                    {
                        return false;
                    }
                }
                if let Some(ref priority) = priority_filter {
                    if &issue.priority != priority {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    pub fn show_issue(&self, id: &str) -> Result<Issue> {
        let full_id = self.storage.resolve_issue_id(id)?;
        self.storage.load_issue(&full_id)
    }

    /// Get enriched dependency information for an issue.
    ///
    /// Only ids that resolve to a stored issue are returned; a dependency id
    /// that no longer resolves (e.g. a dangling reference left by raw storage
    /// mutation or legacy data — normal deletion no longer produces one, see
    /// `delete_issue`) is left out here. That is not a loss of information:
    /// [`IssueShowResponse::from_issue`](crate::output::IssueShowResponse::from_issue)
    /// diffs its own `enriched_deps` argument against the issue's full
    /// `dependencies` list to recover any id missing here and surfaces it via
    /// `dangling_dependency_ids`, so `jit issue show --json` never silently
    /// hides one (jit:f847df3f).
    pub fn get_dependencies_enriched(&self, issue: &Issue) -> Vec<crate::domain::MinimalIssue> {
        issue
            .dependencies
            .iter()
            .filter_map(|dep_id| {
                self.storage
                    .load_issue(dep_id)
                    .ok()
                    .map(|dep| crate::domain::MinimalIssue::from(&dep))
            })
            .collect()
    }

    /// Build the compact [`IssueStatusResponse`](crate::output::IssueStatusResponse)
    /// for one already-loaded issue.
    ///
    /// Loads the issue's enriched dependencies and gate runs, builds the full
    /// [`IssueShowResponse`](crate::output::IssueShowResponse), and projects it
    /// down to the compact status shape — so the projection is byte-for-byte the
    /// same one `jit issue show`/`jit issue status` produce. Shared by
    /// `issue status` and `issue children` (the latter is this projection over a
    /// container's direct dependencies).
    pub fn issue_status_response(
        &self,
        issue: Issue,
    ) -> Result<crate::output::IssueStatusResponse> {
        let enriched_deps = self.get_dependencies_enriched(&issue);
        let gate_runs = self.list_gate_runs(&issue.id, None)?;
        let show = crate::output::IssueShowResponse::from_issue(issue, enriched_deps, &gate_runs);
        Ok(crate::output::IssueStatusResponse::from_show(&show))
    }

    /// Resolve a container's direct dependency edges (its depth-1 children) into
    /// the resolvable child issues and the ids of any dangling edges.
    ///
    /// A dependency id resolving to no stored issue is a *dangling* edge and is
    /// collected separately (following the `issue show`
    /// [`dangling_dependency_ids`](crate::output::IssueShowResponse::dangling_dependency_ids)
    /// precedent) rather than silently dropped. Only a not-found id is treated
    /// as dangling: any genuine storage error (I/O, deserialization) propagates
    /// via `?` instead of being swallowed. Dangling ids are returned sorted.
    fn resolve_direct_children(&self, container: &Issue) -> Result<(Vec<Issue>, Vec<String>)> {
        let mut children = Vec::new();
        let mut dangling = Vec::new();
        for dep_id in &container.dependencies {
            match self.storage.load_issue_or_not_found(dep_id) {
                Ok(child) => children.push(child),
                Err(crate::storage::PathReadError::NotFound(_)) => dangling.push(dep_id.clone()),
                // A real lookup failure is not a dangling edge — surface it.
                Err(other) => return Err(other.into()),
            }
        }
        dangling.sort();
        Ok((children, dangling))
    }

    /// Build the `jit issue children` response: the container's direct children
    /// (depth-1 dependencies), each projected to the compact `issue status`
    /// shape, plus any dangling edges.
    ///
    /// Containment follows the dependency DAG (membership labels are advisory
    /// and not consulted). The container id is resolved the same way as
    /// `issue show`, so a bad id yields the typed id-resolution error unwrapped
    /// (the caller routes it through `handle_json_error!`). Children are ordered
    /// by ascending short id, since stored dependency order is set-derived and
    /// not meaningful.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::Priority;
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    /// let executor = CommandExecutor::new(storage);
    /// let new = |title: &str| {
    ///     executor
    ///         .create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], vec![], None, None, false)
    ///         .unwrap()
    ///         .0
    /// };
    ///
    /// let epic = new("Epic");
    /// let child = new("Child");
    /// executor.add_dependency(&epic, &child).unwrap();
    ///
    /// let response = executor.issue_children(&epic).unwrap();
    /// assert_eq!(response.container.title, "Epic");
    /// assert_eq!(response.count, 1);
    /// assert_eq!(response.issues[0].title, "Child");
    /// assert!(response.dangling.is_empty());
    /// ```
    pub fn issue_children(&self, id: &str) -> Result<crate::output::IssueChildrenResponse> {
        let container = self.show_issue(id)?;
        let (children, dangling) = self.resolve_direct_children(&container)?;

        let mut issues = children
            .into_iter()
            .map(|child| self.issue_status_response(child))
            .collect::<Result<Vec<_>>>()?;
        issues.sort_by(|a, b| a.short_id.cmp(&b.short_id));

        Ok(crate::output::IssueChildrenResponse {
            container: crate::output::ContainerHeader::from(&container),
            count: issues.len(),
            issues,
            dangling,
        })
    }

    /// Build the `jit issue progress` response: the counts-by-state rollup over
    /// the container's direct children (depth 1), plus any dangling edges.
    ///
    /// Membership follows the dependency DAG, as for [`Self::issue_children`].
    /// The rollup counts resolvable children only; dangling edges are surfaced
    /// separately rather than counted or dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::{Priority, State};
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    /// let executor = CommandExecutor::new(storage);
    /// let new = |title: &str| {
    ///     executor
    ///         .create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], vec![], None, None, false)
    ///         .unwrap()
    ///         .0
    /// };
    ///
    /// let epic = new("Epic");
    /// let done = new("Done child");
    /// let todo = new("Todo child");
    /// executor.add_dependency(&epic, &done).unwrap();
    /// executor.add_dependency(&epic, &todo).unwrap();
    /// executor
    ///     .update_issue(&done, None, None, None, Some(State::Done),
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    ///
    /// let progress = executor.issue_progress(&epic).unwrap();
    /// assert_eq!(progress.rollup.total, 2);
    /// assert_eq!(progress.rollup.done, 1);
    /// assert_eq!(progress.rollup.percent, 50);
    /// assert!(progress.dangling.is_empty());
    /// ```
    pub fn issue_progress(&self, id: &str) -> Result<crate::output::ContainerProgressResponse> {
        let container = self.show_issue(id)?;
        let (children, dangling) = self.resolve_direct_children(&container)?;

        Ok(crate::output::ContainerProgressResponse {
            container: crate::output::ContainerHeader::from(&container),
            rollup: crate::output::StateRollup::from_issues(&children),
            dangling,
        })
    }

    /// The dependencies that hold a transition back: every dependency unmet by
    /// [`is_dependency_met`], plus every dependency id that resolves to no issue.
    pub(super) fn blocking_dependencies(
        &self,
        issue: &Issue,
        resolved_issues: &std::collections::HashMap<String, &Issue>,
    ) -> Vec<TransitionBlocker> {
        issue
            .dependencies
            .iter()
            .filter_map(|dep_id| match resolved_issues.get(dep_id).copied() {
                Some(dependency)
                    if is_dependency_met(dependency.state, dependency.archived_from) =>
                {
                    None
                }
                Some(dependency) => Some(TransitionBlocker::dependency(dependency.clone())),
                None => Some(TransitionBlocker::missing_dependency(dep_id.clone())),
            })
            .collect()
    }

    /// Update issue fields.
    ///
    /// Note: This function has 9 parameters (exceeds clippy's 7-parameter guideline).
    /// This is intentional because:
    /// - Each parameter corresponds to a distinct CLI flag (--title, --desc, --priority, etc.)
    /// - Grouping into a struct would obscure the 1:1 CLI mapping
    /// - All parameters are optional (except id), making a builder pattern overkill
    /// - The function is only called from CLI parsing, not used as a general API
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue(
        &self,
        id: &str,
        title: Option<String>,
        description: Option<DescriptionUpdate>,
        priority: Option<Priority>,
        state: Option<State>,
        add_labels: Vec<String>,
        remove_labels: Vec<String>,
        // Tri-state: `None` leaves the field unchanged; `Some(None)` clears the
        // override back to repo-default inheritance; `Some(Some(fmt))` sets it.
        content_format: Option<Option<crate::domain::ContentFormat>>,
        issue_type: Option<String>,
        force: bool,
    ) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;

        // Snapshot the editable fields so we can later tell whether the edits
        // actually changed anything. Idempotent flags (re-setting the same
        // title, adding an existing label, removing a missing one) must not
        // count as a change, otherwise a gate-blocked `--state done` retry
        // would still bump `updated_at` and emit a false progress signal.
        let original_title = issue.title.clone();
        let original_description = issue.description.clone();
        let original_priority = issue.priority;
        let original_labels = issue.labels.clone();
        let original_content_format = issue.content_format;

        if let Some(t) = title {
            issue.title = t;
        }
        if let Some(op) = description {
            issue.description = op.apply(&issue.description);
        }
        if let Some(p) = priority {
            issue.priority = p;
        }
        if let Some(new_cf) = content_format {
            // `Some(None)` clears to inherit; `Some(Some(fmt))` sets the override.
            issue.content_format = new_cf;
        }

        // Handle label operations. Label format / uniqueness / registry are
        // enforced solely by `validate_for_write` against the FINAL shape below
        // (a0f0f342 migration) — no inline format/uniqueness check here.
        for label_str in &add_labels {
            if !issue.labels.contains(label_str) {
                issue.labels.push(label_str.clone());
            }
        }
        for label in &remove_labels {
            issue.labels.retain(|l| l != label);
        }

        // REQ-02: an explicit `--type <kind>` rewrites the issue's `type:<kind>`
        // label in place, replacing any existing `type:*` label. The command
        // layer owns this (the CLI only forwards the typed value). Runs after the
        // generic add/remove so the typed flag is authoritative for the `type`
        // namespace. Kind validity is enforced below via `reject_undeclared_type`
        // on the final shape, through the rule engine.
        let explicit_type = issue_type.is_some();
        if let Some(kind) = issue_type {
            issue
                .labels
                .retain(|label| !label_utils::is_type_label(label));
            issue.labels.push(label_utils::type_label(&kind));
        }

        // REQ-02: when `--type` was explicitly provided, hard-reject an undeclared
        // kind through the SAME rule engine the write uses
        // (`type-hierarchy-known`), which only warns on the normal path.
        // Scoped to the explicit-type path so non-`--type` updates keep warn-only
        // behavior; runs before any persistence so a bad type changes nothing. The
        // `type` rule is state-independent, so evaluating against the pre-transition
        // shape here matches what the projected write below would report.
        if explicit_type {
            self.reject_undeclared_type(&issue)?;
        }

        // Which editable fields actually changed (not merely whether edit flags
        // were provided). Drives whether a gate-blocked `--state done` must still
        // persist the issue to keep real edits, and supplies the `issue_updated`
        // event's field list (mirroring `bulk_update`, which logs the same event
        // for content edits).
        let mut changed_fields = Vec::new();
        if issue.title != original_title {
            changed_fields.push("title".to_string());
        }
        if issue.description != original_description {
            changed_fields.push("description".to_string());
        }
        if issue.priority != original_priority {
            changed_fields.push("priority".to_string());
        }
        if issue.labels != original_labels {
            changed_fields.push("labels".to_string());
        }
        if issue.content_format != original_content_format {
            changed_fields.push("content_format".to_string());
        }
        let has_field_edits = !changed_fields.is_empty();

        let old_state = issue.state;

        // Resolve the requested state transition's TARGET WITHOUT mutating
        // `issue.state` yet, so the actual state change is performed by the
        // chokepoint (`apply_state_transition`), which owns the dependency/gate
        // guards and graph enforcement. `issue.state` is projected into the target
        // only for the `validate_for_write` shape below, then restored.
        //
        // A `--state done` request against unpassed gates resolves to `Gated`
        // instead: the diversion (`gate_blocked`) persists the gated shape and
        // returns the gate-blocking error. Resolving it here means the chokepoint
        // is entered with the target actually being landed. The dependency guard
        // runs first so a dependency-blocked issue reports its dependencies rather
        // than diverting to Gated.
        let mut gate_blocked = false;
        let mut target_state: Option<State> = None;
        if let Some(s) = state {
            if s == State::Done {
                let issues = self.storage.list_issues()?;
                let resolved = crate::domain::queries::build_issue_map(&issues);

                let blockers = self.blocking_dependencies(&issue, &resolved);
                if !blockers.is_empty() {
                    return Err(TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        State::Done,
                        issue.state,
                        blockers,
                    )
                    .into());
                }

                // If gates not passed, the final shape is Gated, not Done.
                if issue.has_unpassed_gates() {
                    target_state = Some(State::Gated);
                    gate_blocked = true;
                } else {
                    target_state = Some(State::Done);
                }
            } else {
                target_state = Some(s);
            }
        }

        // Single write-time validation entry point against the FINAL shape
        // (field edits + resolved state transition applied). The target state is
        // projected onto a temporary value only for validation; the real
        // mutation happens via the chokepoint below. Runs BEFORE any persistence
        // so a blocked write changes nothing; any `--force` bypass events are
        // deferred (emitted after the save in the persisted case, or
        // unconditionally for a forced no-op override).
        if let Some(t) = target_state {
            issue.state = t;
        }
        let validation = self.validate_for_write(&issue, force)?;
        issue.state = old_state;
        warnings.extend(validation.warnings);

        // Gate-blocked `--state done`: persist the projected Gated shape (only
        // when something changed) and return the gate-blocking error. Bypass
        // events are emitted from inside that path (after its save when it
        // persists, otherwise for the forced no-op override). The diversion
        // routes through the chokepoint against the GATED target state, with
        // the user's --force preserved for graph-rule bypass.
        if gate_blocked {
            let persist = has_field_edits || old_state != State::Gated;
            return self.handle_gate_blocking(
                &mut issue,
                old_state,
                persist,
                &changed_fields,
                &validation.bypassed_rules,
                force,
            );
        }

        // Apply the resolved state transition through the SINGLE chokepoint, which
        // runs transition-time graph-rule enforcement (CC-2) and mutates
        // `issue.state`. `persist = false`: this path batches the state change
        // with any field edits into one combined save below (and emits the
        // state-change event there), so the chokepoint only enforces + mutates.
        // A blocking enforce rule returns a `TransitionBlockedError` (exit 4) and
        // persists nothing; non-blocking findings surface as warnings.
        if let Some(t) = target_state {
            warnings.extend(self.apply_state_transition(&mut issue, t, force, false, |_| {})?);
        }

        // Persist only when something actually changed: real field edits or a
        // genuine state transition. A pure no-op `issue update` (e.g. only
        // idempotent gate/assignee flags, already handled upstream) must not
        // bump `updated_at` and emit a false progress signal.
        let persisted = has_field_edits || old_state != issue.state;
        if persisted {
            let new_state = issue.state;
            self.storage.save_issue(issue)?;

            // Log state change event (after the save).
            if old_state != new_state {
                let event = Event::new_issue_state_changed(full_id.clone(), old_state, new_state);
                self.storage.append_event(&event)?;

                // Log completion event if transitioning to Done.
                if new_state == State::Done {
                    let event = Event::new_issue_completed(full_id.clone());
                    self.storage.append_event(&event)?;
                }
            }

            // Log the field-edit event (after the save), mirroring `bulk_update`
            // so every content mutation is captured in the event log, not only
            // state transitions.
            if !changed_fields.is_empty() {
                let event = Event::new_issue_updated(
                    full_id.clone(),
                    "issue-update".to_string(),
                    changed_fields,
                );
                self.storage.append_event(&event)?;
            }
        }

        // Emit bypass events whenever the user explicitly forced an override of an
        // `enforce` rule, independent of whether other fields/state changed. A
        // forced no-op write (no field edits, no transition) against an issue that
        // violates an enforce rule still produces a non-empty `bypassed_rules`, and
        // dropping those events would lose the audit trail of the deliberate
        // override. In the persisted case this runs AFTER the save above, preserving
        // the "log only after the write commits" ordering; in the no-op case there
        // is no save to order against. Ordinary (non-forced) rejections and previews
        // yield an empty `bypassed_rules`, so this stays a no-op for them.
        self.log_rule_bypasses(&full_id, &validation.bypassed_rules)?;

        // Check if any dependent issues can now transition to ready (after save!)
        if let Some(s) = state {
            if s.is_terminal() {
                self.check_auto_transitions()?;
            }
        }

        Ok(warnings)
    }

    /// Confirm operator intent for the destructive `jit issue delete` command
    /// (jit:0daba57d).
    ///
    /// Deletion is disabled by default (Phase 3 safety): the caller must pass
    /// `allowed = true`, which the CLI dispatch derives from the process
    /// environment (`JIT_ALLOW_DELETION=1`). Reading that environment variable
    /// is left to the caller — mirroring `resolve_gate_key`'s
    /// already-resolved-input pattern — so this stays a pure decision, testable
    /// without mutating global process state. Returns
    /// [`DeletionNotConfirmedError`] (classified by `error_to_exit_code` as
    /// `ExitCode::InvalidArgument`, exit code 2) rather than performing the
    /// delete when `allowed` is `false`; `id` is echoed verbatim in the
    /// refusal's example remediation command, so it need not be a resolved
    /// full id.
    pub fn confirm_deletion_allowed(
        &self,
        id: &str,
        allowed: bool,
    ) -> std::result::Result<(), DeletionNotConfirmedError> {
        if allowed {
            Ok(())
        } else {
            Err(DeletionNotConfirmedError::new(id))
        }
    }

    /// Delete an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    ///
    /// Also strips `id` from the `dependencies` array of every issue that
    /// referenced it (jit:f847df3f). Without this cascade a deleted issue left
    /// a dangling edge behind: `jit validate` rejected it, and the normal `jit
    /// dep rm <from> <id>` path could not repair it because removing a
    /// dependency resolved the target id first, and a deleted target no longer
    /// resolves. The cascade is an automatic invariant-maintaining side effect
    /// of deletion, not a user-initiated edit to the dependent — like
    /// `add_dependency`'s Ready-to-Backlog demotion, it intentionally bypasses
    /// the active-lease check on the dependents; only the issue actually being
    /// deleted is lease-checked (above). Each dependent's rewrite is its own
    /// atomic (temp+rename) file write with its own `issue_updated` event, so a
    /// crash mid-cascade cannot corrupt an individual file, only leave later
    /// dependents stale. `jit dep rm` (which matches a raw stored dependency id
    /// without requiring target resolution) and `jit issue show --json`'s
    /// `dangling_dependency_ids` field remain as defense-in-depth for that case
    /// and for any legacy data that predates this fix.
    pub fn delete_issue(&self, id: &str) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        self.storage.delete_issue(&full_id)?;

        // Deletion is a state change; record it after the delete commits so a
        // failed delete never leaves a ghost event (event-logging invariant).
        let event = Event::new_issue_deleted(full_id.clone());
        self.storage.append_event(&event)?;

        // Cascade: strip the deleted id from every dependent so the deletion
        // never leaves a dangling edge in the graph.
        for mut dependent in self.storage.list_issues()? {
            if !dependent.dependencies.iter().any(|d| d == &full_id) {
                continue;
            }
            dependent.dependencies.retain(|d| d != &full_id);
            let dependent_id = dependent.id.clone();
            self.storage.save_issue(dependent)?;
            let event = Event::new_issue_updated(
                dependent_id,
                "dependency-cascade-delete".to_string(),
                vec!["dependencies".to_string()],
            );
            self.storage.append_event(&event)?;
        }

        // Removing an edge can unblock a dependent stuck in Backlog, mirroring
        // the readiness check `remove_dependency(_ies)` runs after an edge is
        // removed.
        self.check_auto_transitions()?;

        Ok(warnings)
    }

    /// Update issue state with precheck/postcheck hooks
    ///
    /// This method runs prechecks before transitioning to InProgress
    /// and postchecks when transitioning to Gated.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    ///
    /// Note: this state-only transition path (used by `jit claim` → InProgress and
    /// `jit issue reject` → Rejected) intentionally does NOT run `.jit/rules.toml`
    /// local-rule enforcement: `claim` carries no content edits and `reject`
    /// deliberately bypasses validation. Local rules are enforced on the
    /// content-bearing write paths (`create_issue`, `update_issue`, bulk update)
    /// via `validate_for_write`. Transition-time GRAPH-rule enforcement (CC-2) and
    /// the actual state mutation/save/event are delegated to the single
    /// `apply_state_transition` chokepoint (which skips enforcement for
    /// `Rejected`), so this path never sets `issue.state` directly.
    pub fn update_issue_state(&self, id: &str, new_state: State) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let issue = self.storage.load_issue(&full_id)?;
        let old_state = issue.state;

        // Handle prechecks for Ready -> InProgress transition
        if old_state == State::Ready && new_state == State::InProgress {
            self.run_prechecks(&full_id)?;
        }

        // Reload issue after prechecks (which may have modified it)
        let mut issue = self.storage.load_issue(&full_id)?;

        // Resolve the targets that need per-path handling before the chokepoint:
        // the gate diversion into `Gated`, and the postcheck run that follows an
        // explicit `--state gated`. Everything else — the dependency and gate
        // guards, graph-rule enforcement (CC-2), the state mutation, the save, and
        // the audit logging — belongs to the single chokepoint
        // (`apply_state_transition`), as does the rejection/no-op policy. This
        // state-only path carries no content edits and no `--force`.
        match new_state {
            State::Done => {
                // The dependency guard runs ahead of the gate check so a
                // dependency-blocked issue reports its dependencies rather than
                // diverting to Gated.
                let issues = self.storage.list_issues()?;
                let resolved = crate::domain::queries::build_issue_map(&issues);

                let blockers = self.blocking_dependencies(&issue, &resolved);
                if !blockers.is_empty() {
                    return Err(TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        State::Done,
                        issue.state,
                        blockers,
                    )
                    .into());
                }

                // If gates not passed, transition to Gated and return error.
                // This path never carries field edits, so a retry on an
                // already-gated issue is a pure no-op (no save, no event).
                if issue.has_unpassed_gates() {
                    let persist = old_state != State::Gated;
                    // This path runs no local-rule validation, so there are no
                    // bypassed rules to log; it also carries no --force flag.
                    return self.handle_gate_blocking(
                        &mut issue,
                        old_state,
                        persist,
                        &[],
                        &[],
                        false,
                    );
                }
            }
            State::Gated => {
                // Move to Gated through the chokepoint (enforces a
                // `when = { state = "gated" }` rule, saves, and logs), then run
                // postchecks which may auto-transition to Done (also enforced via
                // the chokepoint inside `auto_transition_to_done`).
                warnings.extend(self.apply_state_transition(
                    &mut issue,
                    State::Gated,
                    false,
                    true,
                    |_| {},
                )?);

                // Run postchecks (which may auto-transition to Done)
                self.run_postchecks(&full_id)?;
                return Ok(warnings);
            }
            _ => {}
        }

        // Apply the transition through the chokepoint: enforce (CC-2), mutate,
        // save, and log. `Rejected`'s validation bypass and the no-op guard are
        // handled inside it.
        warnings.extend(self.apply_state_transition(&mut issue, new_state, false, true, |_| {})?);

        Ok(warnings)
    }

    /// Assign an issue to someone.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn assign_issue(&self, id: &str, assignee: String) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        // Validate through the one `Assignee` path so `assign` cannot persist a
        // raw, malformed assignee (this command previously skipped validation).
        let assignee: crate::domain::Assignee = assignee.parse()?;
        let mut issue = self.storage.load_issue(&full_id)?;
        // No-op if already assigned to the same assignee: don't bump updated_at.
        if issue.assignee.as_ref() == Some(&assignee) {
            return Ok(warnings);
        }
        issue.assignee = Some(assignee.clone());
        // Record the first assignment time (first-occurrence only; re-assigning an
        // already-claimed issue leaves the original stamp intact).
        issue.mark_claimed(chrono::Utc::now());
        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;
        // The assignee (and `claimed_at`) mutation above appends an
        // `issue_claimed` event so the change is auditable (@/inv/event-log) and the
        // lifecycle-timestamp backfill can fold it back into `claimed_at` (see
        // `derive_lifecycle_timestamps`), matching the `claim`/lease-acquire paths.
        self.storage
            .append_event(&Event::new_issue_claimed(issue_id, assignee))?;
        Ok(warnings)
    }

    /// Claim an issue for `assignee`.
    ///
    /// Returns any non-fatal [`StorageWarning`]s (e.g. a worktree relocation
    /// observed while checking leases) so the calling command can surface them
    /// at its own output boundary; this method never writes to stderr.
    pub fn claim_issue(&self, id: &str, assignee: String) -> Result<Vec<StorageWarning>> {
        use super::claim::check_issue_lease;

        let full_id = self.storage.resolve_issue_id(id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Parse the claimant up front so a pre-existing assignee can be compared
        // against it. Claiming an issue already assigned to a DIFFERENT assignee
        // still hard-fails exactly as before (REQ-02); claiming one already
        // assigned to the SAME assignee is idempotent and falls through to the
        // normal claim flow below (REQ-01) instead of erroring, so an assignment
        // made while dependencies were still unmet can be promoted into a real
        // claim once they reach a terminal state.
        let claimant: crate::domain::Assignee = assignee.parse()?;
        if let Some(existing) = &issue.assignee {
            if existing != &claimant {
                return Err(anyhow!(
                    "Issue {full_id} is already assigned to {existing}; refusing to claim as \
                     {claimant} (re-claiming as {existing} succeeds and promotes it to in_progress)"
                ));
            }
        }

        // Check for existing lease held by another agent.
        // Use both short and full ID since leases may store either. Collect the
        // relocation warning from the identity load (the first call fixes the
        // recorded path, so the second observes none).
        let short_id = issue.short_id();
        let (lease_short, mut warnings) = check_issue_lease(&short_id, Some(&assignee))?;
        let (lease_full, warnings_full) = check_issue_lease(&full_id, Some(&assignee))?;
        warnings.extend(warnings_full);
        let conflicting_lease = lease_short.or(lease_full);

        if let Some(lease) = conflicting_lease {
            let expires_str = lease
                .expires_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "indefinitely".to_string());
            return Err(anyhow!(
                "Issue {} is currently leased by {} {}.\n\
                 Use 'jit claim acquire' to properly coordinate work.",
                id,
                lease.agent_id,
                expires_str
            ));
        }

        let old_state = issue.state;

        // If Ready, try to transition to InProgress first (this enforces prechecks).
        // Backlog issues remain blocked until their dependencies are terminal.
        if old_state == State::Ready {
            self.update_issue_state(&full_id, State::InProgress)?;
        } else if old_state == State::Backlog {
            let issues = self.storage.list_issues()?;
            let resolved = crate::domain::queries::build_issue_map(&issues);
            let blockers = self.blocking_dependencies(&issue, &resolved);
            if !blockers.is_empty() {
                return Err(TransitionBlockedError::dependencies(
                    issue.id.clone(),
                    State::InProgress,
                    issue.state,
                    blockers,
                )
                .into());
            }
        }

        // If we get here, prechecks passed (or issue wasn't Ready)
        // Now assign the issue. `claimant` was already validated through the one
        // `Assignee` path above and is reused here (and for the event below) so
        // it cannot diverge from the stored assignee.
        let actor = claimant;
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = Some(actor.clone());
        // Record the first claim time (first-occurrence only; a re-claim by the
        // same assignee leaves the original stamp intact). This stamp and the
        // `issue_claimed` event below are coupled: the mutation persists together
        // with the event (@/inv/event-log), and the event feeds the
        // lifecycle-timestamp backfill (`derive_lifecycle_timestamps`).
        issue.mark_claimed(chrono::Utc::now());

        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;

        // Log assignment event
        let event = Event::new_issue_claimed(issue_id, actor);
        self.storage.append_event(&event)?;

        Ok(warnings)
    }

    /// Unassign an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn unassign_issue(&self, id: &str) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;
        // No-op if already unassigned: don't bump updated_at.
        if issue.assignee.is_none() {
            return Ok(warnings);
        }
        issue.assignee = None;
        self.storage.save_issue(issue)?;
        Ok(warnings)
    }

    pub fn release_issue(&self, id: &str, reason: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        let mut issue = self.storage.load_issue(&full_id)?;
        let old_assignee = issue.assignee.clone();

        // If in progress, transition back to ready THROUGH the chokepoint, which
        // enforces graph rules, clears the assignee in the same save (via the
        // pre-save hook), and logs the state-change event. Releasing back to Ready
        // is a regression and ordinarily matches no done/gated-scoped enforce
        // rule, but routing it here keeps the invariant that no command sets
        // `issue.state` directly.
        if issue.state == State::InProgress {
            self.apply_state_transition(&mut issue, State::Ready, false, true, |issue| {
                issue.assignee = None;
            })?;
        } else {
            // No state change: just clear the assignee and save.
            issue.assignee = None;
            self.storage.save_issue(issue)?;
        }

        // Log release event (after any state-change event the chokepoint emitted).
        // Only a real prior assignee is a release actor; releasing an unassigned
        // issue records no actor (and an empty assignee is not a valid `Assignee`).
        if let Some(assignee) = old_assignee {
            let event = Event::new_issue_released(full_id.clone(), assignee, reason.to_string());
            self.storage.append_event(&event)?;
        }

        Ok(())
    }

    pub fn claim_next(
        &self,
        assignee: String,
        _filter: Option<String>,
    ) -> Result<(String, Vec<StorageWarning>)> {
        let issues = self.storage.list_issues()?;

        // Highest-priority issue of the domain ready set (Ready, unassigned, every
        // dependency met); `sort_by_key` is stable, so ties keep storage order.
        let mut candidates = crate::domain::queries::query_ready(&issues);

        candidates.sort_by_key(|i| match i.priority {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        });

        if let Some(issue) = candidates.first() {
            let id = issue.id.clone();
            let warnings = self.claim_issue(&id, assignee)?;
            Ok((id, warnings))
        } else {
            Err(anyhow!("No ready issues available"))
        }
    }

    pub(super) fn auto_transition_to_ready(&self, issue_id: &str) -> Result<bool> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);

        let mut issue = self.storage.load_issue(&full_id)?;

        if issue.should_auto_transition_to_ready(&resolved) {
            // Route the auto-promotion through the chokepoint (enforce + save +
            // log). Dependencies are already satisfied (the predicate checked),
            // so this is the dep guard for this path.
            self.apply_state_transition(&mut issue, State::Ready, false, true, |_| {})?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn auto_transition_to_done(&self, issue_id: &str) -> Result<bool> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        if issue.should_auto_transition_to_done() {
            // Route the gates-pass auto-done through the chokepoint. It runs
            // transition-time graph-rule enforcement (CC-2) BEFORE the Done shape
            // persists: a blocking enforce rule (e.g. an enforce-at-done coverage
            // rule) returns a `TransitionBlockedError` (exit 4) and persists
            // nothing, leaving the issue Gated with the findings reported. Without
            // this an auto gate-pass could complete an issue past an
            // enforce-at-done rule. This path carries no `--force`. The chokepoint
            // also emits the state-change AND `issue_completed` events.
            self.apply_state_transition(&mut issue, State::Done, false, true, |_| {})?;

            // Check if any dependent issues can now transition to ready
            self.check_auto_transitions()?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn check_auto_transitions(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;
        let backlog_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.state == State::Backlog)
            .map(|i| i.id.clone())
            .collect();

        for issue_id in backlog_issues {
            self.auto_transition_to_ready(&issue_id)?;
        }

        Ok(())
    }

    /// Helper to handle gate blocking when transitioning to Done.
    ///
    /// Moves the issue to `Gated` and returns a gate-blocking error with clear
    /// feedback. Persistence and audit logging are conditional:
    ///
    /// - `persist` is the caller's decision about whether anything actually
    ///   changed (a genuine transition into `Gated`, or accompanying field
    ///   edits that must not be lost). When false, the call is a pure no-op —
    ///   e.g. retrying `--state done` on an already-`Gated` issue with no other
    ///   edits — and the issue is neither saved (no `updated_at` bump) nor
    ///   logged.
    /// - The `issue_state_changed` event is only appended for a real transition
    ///   (`old_state != Gated`); a `gated -> gated` no-op must never be logged,
    ///   as it would corrupt the audit log for metrics and stalled-work
    ///   detection that read `events.jsonl`.
    /// - `bypassed_rules` lists the `enforce` rules a `--force` write overrode;
    ///   one `LocalRuleBypassed` event is emitted per entry whenever the list is
    ///   non-empty (a deliberate override always merits an audit entry, even on a
    ///   forced no-op). When `persist` is true the events are emitted AFTER the
    ///   save commits, so a failed write leaves no false bypass entry.
    fn handle_gate_blocking(
        &self,
        issue: &mut Issue,
        old_state: State,
        persist: bool,
        changed_fields: &[String],
        bypassed_rules: &[String],
        force: bool,
    ) -> Result<Vec<String>> {
        let registry = self.storage.load_gate_registry()?;
        let gate_blockers = unpassed_gate_blockers(issue, &registry);
        // The gate-diversion path lands the issue in `gated`, so it enforces
        // graph rules against THAT target state via the chokepoint, exactly
        // like an explicit `--state gated` transition. Rules keyed on the
        // originally requested state (e.g. `state = "done"`) still do not fire
        // here — the diversion's target is `gated`, not `done` (see
        // `test_gated_diversion_runs_before_graph_enforcement`). A blocking
        // `state = "gated"` enforce rule therefore blocks the diversion before
        // anything persists. Persistence and the state-changed event are
        // handled below (not by the chokepoint) because this path may carry
        // field edits in the same save and is a no-op for an already-gated
        // issue.
        let mut diversion_warnings = Vec::new();
        if old_state != State::Gated {
            issue.state = old_state;
            // Non-blocking findings ride on the gate-blocking error returned
            // below (TransitionBlockedError::with_warnings) so they surface in
            // both the rendered message and the JSON details.
            diversion_warnings =
                self.apply_state_transition(issue, State::Gated, force, false, |_| {})?;
        }
        issue.state = State::Gated;

        let issue_id = issue.id.clone();
        if persist {
            // Save the state change (and any field edits) before returning error
            self.storage.save_issue(issue.clone())?;

            // Log the state change only for a genuine transition into Gated.
            if old_state != State::Gated {
                let event =
                    Event::new_issue_state_changed(issue_id.clone(), old_state, State::Gated);
                self.storage.append_event(&event)?;
            }

            // Log the field-edit event for any persisted content changes, so a
            // gate-blocked `--state done` that still keeps real edits records them
            // (same contract as the non-blocked path and `bulk_update`).
            if !changed_fields.is_empty() {
                let event = Event::new_issue_updated(
                    issue_id.clone(),
                    "issue-update".to_string(),
                    changed_fields.to_vec(),
                );
                self.storage.append_event(&event)?;
            }
        }

        // Emit any `--force` bypass events whenever an enforce rule was overridden,
        // regardless of whether the gate-blocked write persisted other changes. A
        // forced no-op `--state done` on an already-`Gated` issue that violates an
        // enforce rule still carries a deliberate override that must be audited. In
        // the persisted case this runs AFTER the save above (preserving ordering);
        // in the no-op case there is no save to order against. An empty
        // `bypassed_rules` (ordinary writes) keeps this a no-op.
        self.log_rule_bypasses(&issue_id, bypassed_rules)?;

        Err(TransitionBlockedError::gates(
            issue.id.clone(),
            State::Done,
            State::Gated,
            gate_blockers,
        )
        .with_warnings(diversion_warnings)
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateDefinition;
    use crate::declarations::{GateMode, GateStage};
    use crate::domain::State;
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create config with enforcement off for test backward compatibility
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        CommandExecutor::new(storage)
    }

    #[test]
    fn test_claim_issue_enforces_prechecks() {
        let executor = setup();

        // Define a manual precheck gate (TDD reminder)
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "tdd-reminder".to_string(),
            GateDefinition {
                version: 1,
                key: "tdd-reminder".to_string(),
                title: "TDD Reminder".to_string(),
                description: "Write tests first".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with precheck gate
        let mut issue = crate::domain::Issue::new("Test task".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "tdd-reminder".to_string())
            .unwrap();

        // Try to claim the issue - should fail because precheck hasn't passed
        let result = executor.claim_issue(&issue_id, "agent:test".to_string());

        // Currently this test FAILS because claim_issue bypasses prechecks
        // After fix, claiming should fail with "Manual precheck 'tdd-reminder' has not been passed"
        assert!(
            result.is_err(),
            "Claiming should fail when precheck gate hasn't passed"
        );
        assert!(result.unwrap_err().to_string().contains("tdd-reminder"));

        // Verify issue is still Ready, not InProgress
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::Ready,
            "Issue should remain Ready when precheck fails"
        );
        assert!(
            issue.assignee.is_none(),
            "Issue should not be assigned when precheck fails"
        );
    }

    #[test]
    fn test_claim_issue_succeeds_when_prechecks_pass() {
        let executor = setup();

        // Define a manual precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "tdd-reminder".to_string(),
            GateDefinition {
                version: 1,
                key: "tdd-reminder".to_string(),
                title: "TDD Reminder".to_string(),
                description: "Write tests first".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with precheck gate
        let mut issue = crate::domain::Issue::new("Test task".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "tdd-reminder".to_string())
            .unwrap();

        // Pass the precheck manually
        executor
            .pass_gate(
                &issue_id,
                "tdd-reminder".to_string(),
                Some("human:dev".to_string()),
                false,
            )
            .unwrap();

        // Now claiming should succeed
        let result = executor.claim_issue(&issue_id, "agent:test".to_string());
        assert!(
            result.is_ok(),
            "Claiming should succeed when precheck passes"
        );

        // Verify issue transitioned to InProgress and is assigned
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);
        assert_eq!(issue.assignee, Some("agent:test".parse().unwrap()));
    }

    /// REQ-01: an issue assigned to X while its dependency was still open
    /// (leaving the issue Backlog and blocked) must be claimable by that SAME
    /// assignee once the dependency reaches a terminal state, instead of
    /// hard-failing with "already assigned". This is the assign-then-claim
    /// promotion sequence container stewardship produces.
    #[test]
    fn test_claim_promotes_existing_same_assignee_assignment_once_unblocked() {
        let executor = setup();

        let dependency = crate::domain::Issue::new("Dependency".to_string(), "".to_string());
        let dependency_id = dependency.id.clone();
        executor.storage.save_issue(dependency).unwrap();

        let mut dependent = crate::domain::Issue::new("Dependent".to_string(), "".to_string());
        dependent.dependencies.push(dependency_id.clone());
        let dependent_id = dependent.id.clone();
        executor.storage.save_issue(dependent).unwrap();

        // Assign while the dependency is still open. The issue remains Backlog
        // (a full claim here would exit 4 on the blocked in_progress transition).
        executor
            .assign_issue(&dependent_id, "agent:test".to_string())
            .unwrap();
        let assigned = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(assigned.state, State::Backlog);
        assert_eq!(assigned.assignee, Some("agent:test".parse().unwrap()));

        // Complete the dependency: this auto-promotes the dependent to Ready.
        executor
            .update_issue(
                &dependency_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let promoted = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(
            promoted.state,
            State::Ready,
            "dependent should auto-promote to Ready once its dependency reaches a terminal state"
        );

        // Claim as the SAME assignee: must proceed rather than erroring.
        let result = executor.claim_issue(&dependent_id, "agent:test".to_string());
        assert!(
            result.is_ok(),
            "same-assignee claim should promote the assignment, got: {:?}",
            result.err()
        );

        let claimed = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(claimed.state, State::InProgress);
        assert_eq!(
            claimed.assignee,
            Some("agent:test".parse().unwrap()),
            "assignee should remain a live claim after the promotion"
        );
    }

    /// REQ-02: claiming an issue already assigned to a DIFFERENT assignee must
    /// still hard-fail exactly as before the idempotency fix. The message must
    /// also name the current holder and state that re-claiming as that same
    /// holder succeeds (jit:30a3b5c1 error message polish).
    #[test]
    fn test_claim_rejects_different_assignee_when_already_assigned() {
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Task".to_string(), "".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        executor
            .claim_issue(&issue_id, "agent:first".to_string())
            .unwrap();

        let result = executor.claim_issue(&issue_id, "agent:second".to_string());
        assert!(
            result.is_err(),
            "claim by a different assignee must still be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already assigned"));
        assert!(
            msg.contains("agent:first"),
            "must name the current holder, got: {msg}"
        );
        assert!(
            msg.contains("re-claiming as agent:first succeeds"),
            "must state that re-claiming as the same holder succeeds, got: {msg}"
        );

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::InProgress,
            "rejected claim must not disturb the existing in-progress state"
        );
        assert_eq!(
            issue.assignee,
            Some("agent:first".parse().unwrap()),
            "rejected claim must not disturb the original assignee"
        );
    }

    #[test]
    fn test_rejected_state_bypasses_gates() {
        let executor = setup();

        // Create issue with gates
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        // Add gates that haven't been passed
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();
        executor
            .add_gate(&issue_id, "code-review".to_string())
            .unwrap();

        // Transition to Rejected should succeed without passing gates
        let result = executor.update_issue_state(&issue_id, State::Rejected);
        assert!(
            result.is_ok(),
            "Rejected state should bypass gate validation"
        );

        // Verify state is Rejected
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Rejected);
    }

    #[test]
    fn test_rejected_state_bypasses_dependencies() {
        let executor = setup();

        // Create dependency
        let mut dep = crate::domain::Issue::new("Dependency".to_string(), "Dep".to_string());
        dep.state = State::InProgress; // Not done
        let dep_id = dep.id.clone();
        executor.storage.save_issue(dep).unwrap();

        // Create issue that depends on it
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.dependencies.push(dep_id.clone());
        issue.state = State::Backlog; // Blocked
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        // Transition to Rejected should succeed even with unmet dependencies
        let result = executor.update_issue_state(&issue_id, State::Rejected);
        assert!(
            result.is_ok(),
            "Rejected state should bypass dependency checks"
        );

        // Verify state is Rejected
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Rejected);
    }

    #[test]
    fn test_done_state_still_enforces_gates() {
        let executor = setup();

        // Create issue with unpassed gates
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();

        // Transition to Done should fail (gates not passed)
        let result = executor.update_issue_state(&issue_id, State::Done);
        assert!(
            result.is_err(),
            "Done state should still enforce gate validation"
        );

        // Verify state transitioned to Gated (not Done)
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
    }

    #[test]
    fn test_retry_done_on_gated_issue_is_event_log_noop() {
        let executor = setup();

        // Issue in progress with an unpassed gate.
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();

        // First `--state done`: a genuine InProgress -> Gated transition, which
        // should persist and emit exactly one state-change event.
        let first = executor.update_issue_state(&issue_id, State::Done);
        assert!(first.is_err(), "unpassed gates should block done");
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(gated.state, State::Gated);

        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Retry `--state done` on the already-gated issue. It must still report
        // the blocking gates, but must NOT append another state-change event or
        // bump updated_at (the no-op gated -> gated audit-log corruption).
        let retry = executor.update_issue_state(&issue_id, State::Done);
        assert!(retry.is_err(), "retry should still report blocking gates");

        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "retrying done on a gated issue must not append a state-change event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "retrying done on a gated issue must not bump updated_at"
        );
    }

    /// The CLI path `jit issue update <id> --state done` goes through
    /// `update_issue`. A pure retry (no field edits) on an already-gated issue
    /// must be a no-op for the audit log and timestamp.
    #[test]
    fn test_update_issue_pure_done_retry_on_gated_is_noop() {
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();

        // First done attempt: genuine transition to Gated.
        assert!(executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(gated.state, State::Gated);
        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Pure retry via update_issue (all fields None) must not save or log.
        assert!(executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "pure done retry must not append an event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "pure done retry must not bump updated_at"
        );
    }

    /// `content_format` update is tri-state: `None` leaves it unchanged,
    /// `Some(Some(fmt))` sets the override, and `Some(None)` clears it back to
    /// repo-default inheritance. The CLI maps `--content-format inherit` to the
    /// clear case — this is the only path back to `None` after an override was set.
    #[test]
    fn test_update_issue_content_format_tristate_set_keep_clear() {
        use crate::domain::ContentFormat;
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Test".to_string(), "Body".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        // Set the override to Html.
        executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                Some(Some(ContentFormat::Html)),
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            Some(ContentFormat::Html)
        );

        // `None` leaves it unchanged (e.g. a title-only edit).
        executor
            .update_issue(
                &issue_id,
                Some("Renamed".to_string()),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            Some(ContentFormat::Html),
            "None must not touch content_format"
        );

        // `Some(None)` clears back to inherit.
        executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                Some(None),
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            None,
            "Some(None) must clear the override to repo-default inheritance"
        );
    }

    /// A gate-blocked `--state done` that *also* carries field edits must still
    /// persist those edits (they must not be silently dropped), while not
    /// logging a spurious `gated -> gated` state-change event.
    #[test]
    fn test_update_issue_field_edits_persist_when_done_blocked_on_gated() {
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Old Title".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();

        // Move to Gated via a first blocked done attempt.
        assert!(executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let events_after_first = executor.storage.read_events().unwrap().len();

        // Retry done together with a title edit on the already-gated issue.
        let result = executor.update_issue(
            &issue_id,
            Some("New Title".to_string()),
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        );
        assert!(result.is_err(), "gates should still block done");

        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            after.title, "New Title",
            "field edits must persist even when done is gate-blocked"
        );
        // The persisted title edit is event-logged (no spurious gated -> gated
        // state-change event, but the field edit itself must be recorded).
        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_after_first + 1,
            "the persisted field edit must append exactly one issue_updated event"
        );
        let logged = events
            .iter()
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == issue_id)
            .expect("a gate-blocked field edit must log an issue_updated event");
        match logged {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "title"),
                "issue_updated must record the changed field, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    /// Providing an edit flag whose value matches the current value (e.g.
    /// `--title <same>`) is a semantic no-op and must not bump `updated_at` or
    /// emit an event on a gate-blocked `--state done` retry.
    #[test]
    fn test_update_issue_idempotent_field_on_gated_retry_is_noop() {
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Same Title".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "tests".to_string()).unwrap();

        // First blocked done attempt: transition to Gated.
        assert!(executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Retry with the SAME title value: no real change, so it must be a no-op.
        assert!(executor
            .update_issue(
                &issue_id,
                Some("Same Title".to_string()),
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .is_err());
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "idempotent edit must not append an event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "idempotent edit must not bump updated_at"
        );
    }

    #[test]
    fn test_assign_same_assignee_is_noop() {
        let executor = setup();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        executor
            .assign_issue(&issue_id, "agent:a".to_string())
            .unwrap();
        let updated_after_assign = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Re-assigning the same assignee is a no-op.
        executor
            .assign_issue(&issue_id, "agent:a".to_string())
            .unwrap();
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_after_assign,
            "re-assigning the same assignee must not bump updated_at"
        );
    }

    #[test]
    fn test_unassign_already_unassigned_is_noop() {
        let executor = setup();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Unassigning an issue that has no assignee is a no-op.
        executor.unassign_issue(&issue_id).unwrap();
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_before,
            "unassigning an already-unassigned issue must not bump updated_at"
        );
    }

    #[test]
    fn test_update_issue_with_no_changes_is_noop() {
        let executor = setup();

        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;
        let events_before = executor.storage.read_events().unwrap().len();

        // No fields and no state change: pure no-op.
        executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            after.updated_at, updated_before,
            "a no-change issue update must not bump updated_at"
        );
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_before,
            "a no-change issue update must not append events"
        );
    }

    #[test]
    fn test_update_issue_description_logs_issue_updated_event() {
        let executor = setup();

        let issue = crate::domain::Issue::new("Test".to_string(), "old".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let events_before = executor.storage.read_events().unwrap().len();

        // A real description edit must be captured in the event log, not only
        // state transitions (mirrors the bulk-update event-logging contract).
        executor
            .update_issue(
                &issue_id,
                None,
                Some(DescriptionUpdate::Replace("new description".to_string())),
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();

        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_before + 1,
            "a description edit must append exactly one event"
        );
        let logged = events
            .iter()
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == issue_id)
            .expect("description edit must log an issue_updated event");
        match logged {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "description"),
                "issue_updated must record the changed field, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_confirm_deletion_allowed_refuses_when_not_allowed() {
        let executor = setup();

        let result = executor.confirm_deletion_allowed("abc12345", false);

        let err = result.expect_err("must refuse without confirmation");
        assert!(err.message().contains("JIT_ALLOW_DELETION=1"));
        assert!(err.message().contains("abc12345"));
    }

    #[test]
    fn test_confirm_deletion_allowed_ok_when_allowed() {
        let executor = setup();

        assert!(executor.confirm_deletion_allowed("abc12345", true).is_ok());
    }

    #[test]
    fn test_delete_issue_logs_issue_deleted_event() {
        let executor = setup();

        let issue = crate::domain::Issue::new("Doomed".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let events_before = executor.storage.read_events().unwrap().len();

        // Deletion is a state change and must be captured in the event log.
        executor.delete_issue(&issue_id).unwrap();

        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_before + 1,
            "a deletion must append exactly one event"
        );
        let logged = events
            .iter()
            .find(|e| e.get_type() == "issue_deleted" && e.get_issue_id() == issue_id)
            .expect("deletion must log an issue_deleted event");
        assert!(matches!(logged, Event::IssueDeleted { .. }));
    }

    #[test]
    fn test_add_dependency_logs_issue_updated_event() {
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();

        let events_before = executor.storage.read_events().unwrap().len();
        executor.add_dependency(&a_id, &b_id).unwrap();

        // Adding a dependency edits `A`; the change must be event-logged.
        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == a_id)
            .expect("dependency addition must log an issue_updated event");
        match logged {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependency_logs_issue_updated_event() {
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();

        // Snapshot AFTER the add so we isolate the removal's event.
        let events_before = executor.storage.read_events().unwrap().len();
        executor.remove_dependency(&a_id, &b_id).unwrap();

        // Removing a dependency edits `A`; the change must be event-logged.
        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == a_id)
            .expect("dependency removal must log an issue_updated event");
        match logged {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependencies_logs_issue_updated_event() {
        // The CLI `jit dep rm` path goes through the plural `remove_dependencies`,
        // which must also event-log the edit.
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();

        let events_before = executor.storage.read_events().unwrap().len();
        executor
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == a_id)
            .expect("CLI dependency removal must log an issue_updated event");
        match logged {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependency_noop_does_not_save_or_log() {
        // Removing an edge that is not present must not persist (no `updated_at`
        // bump) and must not append an event, both for the single and bulk paths.
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        // B exists but A does NOT depend on it, so removing B from A is a no-op.
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();

        let updated_before = executor.storage.load_issue(&a_id).unwrap().updated_at;
        let events_before = executor.storage.read_events().unwrap().len();

        executor.remove_dependency(&a_id, &b_id).unwrap();
        executor
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        assert_eq!(
            executor.storage.load_issue(&a_id).unwrap().updated_at,
            updated_before,
            "a no-op dependency removal must not bump updated_at"
        );
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_before,
            "a no-op dependency removal must not append an event"
        );
    }

    // --- jit:f847df3f: delete must not leave dangling dependency edges -----

    #[test]
    fn test_delete_issue_strips_id_from_dependent_dependencies() {
        // A depends on B. Deleting B must remove B's id from A's stored
        // `dependencies`, not just B's own file — otherwise A is left with a
        // dangling edge that corrupts the graph (jit:f847df3f).
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();
        assert!(executor.storage.load_issue(&a_id).unwrap().dependencies == vec![b_id.clone()]);

        let events_before = executor.storage.read_events().unwrap().len();
        executor.delete_issue(&b_id).unwrap();

        let a_after = executor.storage.load_issue(&a_id).unwrap();
        assert!(
            !a_after.dependencies.contains(&b_id),
            "B's id must be gone from A's dependencies after B is deleted, got: {:?}",
            a_after.dependencies
        );

        // The cascade edit is itself event-logged (in addition to the
        // `issue_deleted` event for B), preserving @/inv/event-log.
        let events = executor.storage.read_events().unwrap();
        assert!(
            events.len() > events_before,
            "the delete cascade must append at least the deletion event"
        );
        let cascade_event = events
            .iter()
            .skip(events_before)
            .find(|e| e.get_type() == "issue_updated" && e.get_issue_id() == a_id)
            .expect("the cascade cleanup of A's dependencies must be event-logged");
        match cascade_event {
            Event::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "cascade event must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
        let delete_event = events
            .iter()
            .skip(events_before)
            .find(|e| e.get_type() == "issue_deleted" && e.get_issue_id() == b_id)
            .expect("deleting B must still log its own issue_deleted event");
        assert!(matches!(delete_event, Event::IssueDeleted { .. }));
    }

    #[test]
    fn test_delete_issue_then_validate_reports_no_dangling_dependency() {
        // After deleting a dependency through the executor, `jit validate`
        // must not report "depends on ... which does not exist" (jit:f847df3f).
        let executor = setup();

        let (a_id, _) = executor
            .create_issue(
                "A".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let (b_id, _) = executor
            .create_issue(
                "B".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();
        assert!(
            executor.validate_silent().is_ok(),
            "validation should pass before deletion"
        );

        executor.delete_issue(&b_id).unwrap();

        let result = executor.validate_silent();
        assert!(
            result.is_ok(),
            "validate must report no dangling dependency after delete, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_remove_dependencies_removes_dangling_edge_by_raw_id() {
        // Reproduces a pre-existing dangling edge the way legacy data (or a raw
        // `IssueStore::delete_issue` bypassing the executor's cascade) could:
        // A depends on B, then B's file disappears without A's `dependencies`
        // being cleaned up. `jit dep rm` (the plural `remove_dependencies`)
        // must still remove the edge by matching A's raw stored id, even
        // though B can no longer be resolved through the repo index
        // (jit:f847df3f).
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();

        // Bypass the executor's cascading delete to reproduce a dangling edge.
        executor.storage.delete_issue(&b_id).unwrap();
        assert!(
            executor.storage.resolve_issue_id(&b_id).is_err(),
            "the deleted id must no longer resolve, matching the reported bug"
        );

        let result = executor
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        assert_eq!(result.removed, vec![b_id.clone()]);
        assert!(
            result.not_found.is_empty(),
            "a dangling edge must be reported as removed, not not_found: {:?}",
            result.not_found
        );
        let issue = executor.storage.load_issue(&a_id).unwrap();
        assert!(
            !issue.dependencies.contains(&b_id),
            "the dangling edge must actually be gone from A's dependencies"
        );
    }

    #[test]
    fn test_remove_dependencies_removes_dangling_edge_by_short_prefix() {
        // Same as above but the CLI caller supplies a short prefix of the
        // dangling id (as `jit dep rm` accepts for live ids); matching must
        // work against the raw stored id, not via global resolution.
        let executor = setup();

        let a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();
        let b = crate::domain::Issue::new("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        executor.storage.save_issue(b).unwrap();
        executor.add_dependency(&a_id, &b_id).unwrap();
        executor.storage.delete_issue(&b_id).unwrap();

        let prefix = b_id[..8].to_string();
        let result = executor
            .remove_dependencies(&a_id, std::slice::from_ref(&prefix))
            .unwrap();

        assert_eq!(result.removed, vec![prefix]);
        let issue = executor.storage.load_issue(&a_id).unwrap();
        assert!(!issue.dependencies.contains(&b_id));
    }

    #[test]
    fn test_remove_dependencies_ambiguous_prefix_errors_and_removes_nothing() {
        // Two stored dependency ids share a normalized prefix. `jit dep rm`
        // with that prefix must error (mirroring resolve_issue_id's ambiguity
        // rejection) instead of silently removing whichever appears first, and
        // neither edge is removed (jit:f847df3f review).
        let executor = setup();

        let mut a = crate::domain::Issue::new("A".to_string(), "Test".to_string());
        let dep1 = "abcd1234-0000-0000-0000-000000000001".to_string();
        let dep2 = "abcd1234-0000-0000-0000-000000000002".to_string();
        a.dependencies = vec![dep1.clone(), dep2.clone()];
        let a_id = a.id.clone();
        executor.storage.save_issue(a).unwrap();

        let prefix = "abcd1234".to_string();
        let err = executor
            .remove_dependencies(&a_id, std::slice::from_ref(&prefix))
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("ambiguous"),
            "expected an ambiguity error, got: {err}"
        );

        let issue = executor.storage.load_issue(&a_id).unwrap();
        assert!(
            issue.dependencies.contains(&dep1) && issue.dependencies.contains(&dep2),
            "an ambiguous dep rm must remove nothing"
        );
    }

    #[test]
    fn test_get_dependencies_enriched_omits_dangling_id_from_resolved_list() {
        // `get_dependencies_enriched` itself only returns resolvable
        // dependencies (dangling ids are recovered downstream by
        // `IssueShowResponse::from_issue`, see its doc comment and tests in
        // `output.rs`). This pins that contract so the two stay in sync.
        let executor = setup();

        let dep = crate::domain::Issue::new("Dep".to_string(), "Test".to_string());
        let dep_id = dep.id.clone();
        executor.storage.save_issue(dep).unwrap();

        let mut issue = crate::domain::Issue::new("Parent".to_string(), "Test".to_string());
        issue.dependencies = vec![dep_id.clone(), "missing-id".to_string()];

        let resolved = executor.get_dependencies_enriched(&issue);
        assert_eq!(resolved.len(), 1, "only the resolvable dep must appear");
        assert_eq!(resolved[0].id, dep_id);
    }

    /// REQ-02: an explicit `--type` whose kind IS declared in the configured
    /// `[type_hierarchy]` is accepted and the canonical `type:<kind>` label is
    /// written. Acceptance is decided by the rule engine: the
    /// `type-hierarchy-known` rule produces no finding for a declared
    /// kind, the SAME validation layer the write path uses.
    #[test]
    fn test_create_explicit_declared_type_accepted_via_rule_engine() {
        let executor = setup();
        let (id, _warnings) = executor
            .create_issue(
                "Declared type".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                Some("task".to_string()),
                false,
            )
            .expect("a declared --type kind must be accepted");
        let issue = executor.storage.load_issue(&id).unwrap();
        assert!(
            issue.labels.iter().any(|l| l == "type:task"),
            "declared --type must write the canonical label, got: {:?}",
            issue.labels
        );
    }

    /// REQ-02: an explicit `--type` whose kind is NOT declared is hard-rejected
    /// through the existing rule engine (the `type-hierarchy-known`
    /// finding), surfacing as a `ValidationFailedError` (exit 4) and persisting
    /// nothing — NOT via a parallel config containment check.
    #[test]
    fn test_create_explicit_undeclared_type_rejected_via_rule_engine() {
        let executor = setup();
        let before = executor.storage.list_issues().unwrap().len();
        let err = executor
            .create_issue(
                "Undeclared type".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                Some("xyzzy-unknown-type".to_string()),
                false,
            )
            .expect_err("an undeclared --type kind must be rejected");
        assert!(
            err.downcast_ref::<crate::errors::ValidationFailedError>()
                .is_some(),
            "rejection must be a ValidationFailedError, got: {err:?}"
        );
        assert_eq!(
            executor.storage.list_issues().unwrap().len(),
            before,
            "a rejected create must persist nothing"
        );
    }

    /// REQ-02: `issue update --type <declared>` is accepted via the rule engine
    /// and leaves exactly one canonical `type:` label.
    #[test]
    fn test_update_explicit_declared_type_accepted_via_rule_engine() {
        let executor = setup();
        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                Some("story".to_string()),
                false,
            )
            .expect("a declared --type update must be accepted");
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let type_labels: Vec<&str> = issue
            .labels
            .iter()
            .filter(|l| l.starts_with("type:"))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            type_labels,
            vec!["type:story"],
            "update --type must leave exactly one canonical type label"
        );
    }

    /// REQ-02: `issue update --type <undeclared>` is hard-rejected through the
    /// rule engine (mirroring create), as a `ValidationFailedError`.
    #[test]
    fn test_update_explicit_undeclared_type_rejected_via_rule_engine() {
        let executor = setup();
        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        let err = executor
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                Some("xyzzy-unknown-type".to_string()),
                false,
            )
            .expect_err("update with an undeclared --type must be rejected");
        assert!(
            err.downcast_ref::<crate::errors::ValidationFailedError>()
                .is_some(),
            "rejection must be a ValidationFailedError, got: {err:?}"
        );
    }

    #[test]
    fn test_manual_transition_to_ready_state_actually_changes_state() {
        let executor = setup();

        // Create dependency issue that is Done
        let mut dep = crate::domain::Issue::new("Dependency".to_string(), "Dep".to_string());
        dep.state = State::Done;
        let dep_id = dep.id.clone();
        executor.storage.save_issue(dep).unwrap();

        // Create issue in Backlog that depends on the Done dependency
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::Backlog;
        issue.dependencies.push(dep_id.clone());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        // Manually transition to Ready should succeed (dependency is done)
        let result = executor.update_issue(
            &issue_id,
            None,               // title
            None,               // description
            None,               // priority
            Some(State::Ready), // state
            vec![],             // add_labels
            vec![],             // remove_labels
            None,
            None,
            false, // force
        );

        assert!(
            result.is_ok(),
            "Transition to Ready should succeed when unblocked"
        );

        // BUG: Issue state is NOT updated to Ready, it remains Backlog
        // After fix, this assertion should pass
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::Ready,
            "Issue state should be Ready after manual transition"
        );
    }
}
