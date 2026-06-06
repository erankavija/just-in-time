//! Issue CRUD operations and lifecycle management

use super::*;
use crate::errors::{TransitionBlockedError, TransitionBlocker};

impl<S: IssueStore> CommandExecutor<S> {
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        mut labels: Vec<String>,
        force: bool,
    ) -> Result<(String, Vec<String>)> {
        // Apply default type if configured and missing (uses the cached
        // config/namespaces consumed by the unified write-validation path).
        let config = self.cached_config()?;
        let namespaces = self.cached_namespaces()?;
        if let Some(ref validation_config) = config.validation {
            let validator = crate::validation::IssueValidator::new(
                validation_config.clone(),
                namespaces.clone(),
            );
            validator.apply_default_type(&mut labels);
        }

        // Validate all labels
        for label_str in &labels {
            label_utils::validate_label(label_str)?;
        }

        // Check uniqueness constraints
        let mut unique_namespaces_seen = std::collections::HashSet::new();

        for label_str in &labels {
            if let Ok((namespace, _)) = label_utils::parse_label(label_str) {
                if let Some(ns_config) = namespaces.get(&namespace) {
                    if ns_config.unique && !unique_namespaces_seen.insert(namespace.clone()) {
                        return Err(anyhow!(
                            "Cannot add multiple labels from unique namespace '{}' to the same issue",
                            namespace
                        ));
                    }
                }
            }
        }

        let mut issue = Issue::new(title, description);
        issue.priority = priority;
        issue.gates_required = gates;
        issue.labels = labels;

        // Auto-transition to Ready if no dependencies (gates don't block Ready)
        // BEFORE validating, so rules keyed on the final state (e.g.
        // `when = { state = "ready" }`) see the shape that will be persisted.
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
                    if issue.assignee.as_ref() != Some(assignee) {
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

        if let Some(t) = title {
            issue.title = t;
        }
        if let Some(d) = description {
            issue.description = d;
        }
        if let Some(p) = priority {
            issue.priority = p;
        }

        // Handle label operations
        for label_str in &add_labels {
            label_utils::validate_label(label_str)?;
            if !issue.labels.contains(label_str) {
                issue.labels.push(label_str.clone());
            }
        }
        for label in &remove_labels {
            issue.labels.retain(|l| l != label);
        }

        // Whether the edits actually changed the issue (not merely whether
        // edit flags were provided). Drives whether a gate-blocked `--state
        // done` must still persist the issue to keep real edits.
        let has_field_edits = issue.title != original_title
            || issue.description != original_description
            || issue.priority != original_priority
            || issue.labels != original_labels;

        let old_state = issue.state;

        // Resolve the requested state transition into the in-memory issue FIRST
        // (running dependency/gate guards) so validation sees the FINAL shape.
        // `gate_blocked` marks the gate-diversion-to-Gated case, which still
        // persists/returns an error but must validate the projected Gated shape.
        let mut gate_blocked = false;
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

                issue.state = State::Ready;
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
                    issue.state = State::Gated;
                    gate_blocked = true;
                } else {
                    issue.state = State::Done;
                }
            } else {
                issue.state = s;
            }
        }

        // Single write-time validation entry point against the FINAL shape
        // (field edits + resolved state transition applied). Runs BEFORE any
        // persistence so a blocked write changes nothing; defers `--force`
        // bypass events until after the save succeeds.
        let validation = self.validate_for_write(&issue, force)?;
        warnings.extend(validation.warnings);

        // Gate-blocked `--state done`: persist the projected Gated shape (only
        // when something changed) and return the gate-blocking error. Bypass
        // events are emitted from inside this path after its save succeeds.
        if gate_blocked {
            let persist = has_field_edits || old_state != State::Gated;
            return self.handle_gate_blocking(
                &mut issue,
                old_state,
                persist,
                &validation.bypassed_rules,
            );
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
        }

        // Emit bypass events only AFTER the write committed. (If nothing was
        // persisted, there were no blocking rules to bypass either, so this is a
        // no-op — but guarding keeps the "log only after a real write" invariant.)
        if persisted {
            self.log_rule_bypasses(&full_id, &validation.bypassed_rules)?;
        }

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
        Ok(warnings)
    }

    /// Update issue state with precheck/postcheck hooks
    ///
    /// This method runs prechecks before transitioning to InProgress
    /// and postchecks when transitioning to Gated.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
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

        // Validate state transition
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

                issue.state = State::Ready;
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
                    // bypassed rules to log.
                    return self.handle_gate_blocking(&mut issue, old_state, persist, &[]);
                } else {
                    issue.state = State::Done;
                }
            }
            State::Rejected => {
                // Rejected bypasses all validation - no gates or dependencies required
                // This allows closing issues that won't be implemented without
                // needing to pass quality gates or wait for dependencies
                issue.state = State::Rejected;
            }
            State::Gated => {
                // Run postchecks when moving to Gated
                issue.state = State::Gated;

                let issue_id = issue.id.clone();
                self.storage.save_issue(issue)?;

                // Log state change event
                let event =
                    Event::new_issue_state_changed(issue_id.clone(), old_state, State::Gated);
                self.storage.append_event(&event)?;

                // Run postchecks (which may auto-transition to Done)
                self.run_postchecks(&full_id)?;
                return Ok(warnings);
            }
            _ => {
                issue.state = new_state;
            }
        }

        // Save and log
        if old_state != issue.state {
            let issue_id = issue.id.clone();
            let new_state = issue.state;

            self.storage.save_issue(issue)?;

            let event = Event::new_issue_state_changed(issue_id.clone(), old_state, new_state);
            self.storage.append_event(&event)?;

            // Log completion event if transitioning to Done
            if new_state == State::Done {
                let event = Event::new_issue_completed(issue_id);
                self.storage.append_event(&event)?;
            }
        } else {
            self.storage.save_issue(issue)?;
        }

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

        let mut issue = self.storage.load_issue(&full_id)?;
        // No-op if already assigned to the same assignee: don't bump updated_at.
        if issue.assignee.as_deref() == Some(assignee.as_str()) {
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
        // Now assign the issue
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = Some(assignee.clone());

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
        let old_state = issue.state;

        issue.assignee = None;

        // If in progress, transition back to ready
        if issue.state == State::InProgress {
            issue.state = State::Ready;
        }

        let new_state = issue.state;
        self.storage.save_issue(issue)?;

        // Log event
        let event = Event::new_issue_released(
            full_id.clone(),
            old_assignee.unwrap_or_default(),
            reason.to_string(),
        );
        self.storage.append_event(&event)?;

        // Log state change if it occurred
        if old_state != new_state {
            let event = Event::new_issue_state_changed(full_id, old_state, new_state);
            self.storage.append_event(&event)?;
        }

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
            let old_state = issue.state;
            issue.state = State::Ready;

            let issue_id = issue.id.clone();
            self.storage.save_issue(issue)?;

            // Log state change event
            let event = Event::new_issue_state_changed(issue_id, old_state, State::Ready);
            self.storage.append_event(&event)?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn auto_transition_to_done(&self, issue_id: &str) -> Result<bool> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        if issue.should_auto_transition_to_done() {
            let old_state = issue.state;
            issue.state = State::Done;

            let issue_id = issue.id.clone();
            self.storage.save_issue(issue)?;

            // Log state change event
            let event = Event::new_issue_state_changed(issue_id.clone(), old_state, State::Done);
            self.storage.append_event(&event)?;

            // Log completion event
            let event = Event::new_issue_completed(issue_id);
            self.storage.append_event(&event)?;

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
    ///   one `LocalRuleBypassed` event is emitted per entry, but only when
    ///   `persist` is true and AFTER the save commits, so a failed write leaves
    ///   no false bypass entry.
    fn handle_gate_blocking(
        &self,
        issue: &mut Issue,
        old_state: State,
        persist: bool,
        bypassed_rules: &[String],
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
        issue.state = State::Gated;

        if persist {
            let issue_id = issue.id.clone();
            // Save the state change (and any field edits) before returning error
            self.storage.save_issue(issue.clone())?;

            // Log the state change only for a genuine transition into Gated.
            if old_state != State::Gated {
                let event =
                    Event::new_issue_state_changed(issue_id.clone(), old_state, State::Gated);
                self.storage.append_event(&event)?;
            }

            // Emit any `--force` bypass events only AFTER the save committed.
            self.log_rule_bypasses(&issue_id, bypassed_rules)?;
        }

        Err(TransitionBlockedError::gates(
            issue.id.clone(),
            State::Done,
            State::Gated,
            gate_blockers,
        )
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
        assert_eq!(issue.assignee, Some("agent:test".to_string()));
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
            false,
        );
        assert!(result.is_err(), "gates should still block done");

        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            after.title, "New Title",
            "field edits must persist even when done is gate-blocked"
        );
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "no spurious gated -> gated state-change event should be logged"
        );
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
            .update_issue(&issue_id, None, None, None, None, vec![], vec![], false)
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
            false,              // force
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
