//! Issue CRUD operations and lifecycle management

use super::*;
use crate::errors::{TransitionBlockedError, TransitionBlocker};

impl<S: IssueStore> CommandExecutor<S> {
    #[allow(clippy::too_many_arguments)]
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        mut labels: Vec<String>,
        content_format: Option<crate::domain::ContentFormat>,
        force: bool,
    ) -> Result<(String, Vec<String>)> {
        // Config comes from the executor cache so it is not re-parsed per call.
        // Label format and ALL namespace constraints (canonical format, uniqueness,
        // registry, etc.) are now enforced SOLELY by the effective rule set inside
        // `validate_for_write` (a0f0f342 migration) — no inline format/uniqueness
        // check remains here.
        let config = self.cached_config()?;

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
            let has_type = labels.iter().any(|l| {
                label_utils::parse_label(l)
                    .map(|(ns, _)| ns == "type")
                    .unwrap_or(false)
            });
            if !has_type {
                labels.push(format!("type:{default_type}"));
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

    /// Get enriched dependency information for an issue
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

    fn blocking_dependencies(
        &self,
        issue: &Issue,
        resolved_issues: &std::collections::HashMap<String, &Issue>,
    ) -> Vec<TransitionBlocker> {
        issue
            .dependencies
            .iter()
            .filter_map(|dep_id| match resolved_issues.get(dep_id).copied() {
                Some(dependency) if dependency.state.is_terminal() => None,
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
        description: Option<String>,
        priority: Option<Priority>,
        state: Option<State>,
        add_labels: Vec<String>,
        remove_labels: Vec<String>,
        // Tri-state: `None` leaves the field unchanged; `Some(None)` clears the
        // override back to repo-default inheritance; `Some(Some(fmt))` sets it.
        content_format: Option<Option<crate::domain::ContentFormat>>,
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
        if let Some(d) = description {
            issue.description = d;
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

        // Resolve the requested state transition's TARGET (running dependency/gate
        // guards) WITHOUT mutating `issue.state` yet, so the actual state change is
        // performed by the chokepoint (`apply_state_transition`) and cannot bypass
        // graph enforcement. `issue.state` is projected into the target only for
        // the `validate_for_write` shape below, then restored. `gate_blocked` marks
        // the gate-diversion-to-Gated case, which still persists/returns an error.
        let mut gate_blocked = false;
        let mut target_state: Option<State> = None;
        if let Some(s) = state {
            if s == State::Ready {
                // Check dependencies only (gates don't block Ready)
                let issues = self.storage.list_issues()?;
                let resolved = crate::domain::queries::build_issue_map(&issues);

                let blockers = self.blocking_dependencies(&issue, &resolved);
                if !blockers.is_empty() {
                    return Err(TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        State::Ready,
                        issue.state,
                        blockers,
                    )
                    .into());
                }

                target_state = Some(State::Ready);
            } else if s == State::Done {
                // Check both dependencies and gates
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

    /// Delete an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
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

        // Run the per-target pre-guards (dependency/gate/gate-diversion) that
        // differ per path. The ACTUAL state mutation, graph-rule enforcement
        // (CC-2), save, and audit logging are all performed by the single
        // chokepoint (`apply_state_transition`); the rejection/no-op policy lives
        // inside it, so this path never special-cases them. This state-only path
        // carries no content edits and no `--force`.
        match new_state {
            State::Ready => {
                // Check dependencies only (gates don't block Ready)
                let issues = self.storage.list_issues()?;
                let resolved = crate::domain::queries::build_issue_map(&issues);

                let blockers = self.blocking_dependencies(&issue, &resolved);
                if !blockers.is_empty() {
                    return Err(TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        State::Ready,
                        issue.state,
                        blockers,
                    )
                    .into());
                }
            }
            State::Done => {
                // Check both dependencies and gates
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
        issue.assignee = Some(assignee);
        self.storage.save_issue(issue)?;
        Ok(warnings)
    }

    pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
        use super::claim::check_issue_lease;

        let full_id = self.storage.resolve_issue_id(id)?;
        let issue = self.storage.load_issue(&full_id)?;

        if issue.assignee.is_some() {
            return Err(anyhow!("Issue is already assigned"));
        }

        // Check for existing lease held by another agent
        // Use both short and full ID since leases may store either
        let short_id = issue.short_id();
        let conflicting_lease = check_issue_lease(&short_id, Some(&assignee))?
            .or(check_issue_lease(&full_id, Some(&assignee))?);

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
        // Now assign the issue, validating through the one `Assignee` path.
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = Some(assignee.parse::<crate::domain::Assignee>()?);

        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;

        // Log assignment event
        let event = Event::new_issue_claimed(issue_id, assignee);
        self.storage.append_event(&event)?;

        Ok(())
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
        let event = Event::new_issue_released(
            full_id.clone(),
            old_assignee.map(|a| a.to_string()).unwrap_or_default(),
            reason.to_string(),
        );
        self.storage.append_event(&event)?;

        Ok(())
    }

    pub fn claim_next(&self, assignee: String, _filter: Option<String>) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);

        // Find first ready, unassigned issue with highest priority
        let mut candidates: Vec<&Issue> = issues
            .iter()
            .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
            .collect();

        candidates.sort_by_key(|i| match i.priority {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        });

        if let Some(issue) = candidates.first() {
            let id = issue.id.clone();
            self.claim_issue(&id, assignee)?;
            Ok(id)
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
        let unpassed = issue.get_unpassed_gates();
        let gate_blockers = unpassed
            .into_iter()
            .map(|gate_key| {
                let status = issue
                    .gates_status
                    .get(&gate_key)
                    .map(|gate| gate.status)
                    .unwrap_or(GateStatus::Pending);
                (gate_key, status)
            })
            .collect();
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
    use crate::domain::{Gate, GateMode, GateStage, State};
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
            Gate {
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
            Gate {
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

        // Transition to Rejected should succeed even with incomplete dependencies
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
                Some("new description".to_string()),
                None,
                None,
                vec![],
                vec![],
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
