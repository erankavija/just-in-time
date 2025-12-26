//! Issue CRUD operations and lifecycle management

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        labels: Vec<String>,
    ) -> Result<String> {
        // Validate all labels
        for label_str in &labels {
            label_utils::validate_label(label_str)?;
        }

        // Check uniqueness constraints
        let namespaces = self.config_manager.get_namespaces()?;
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
        if issue.dependencies.is_empty() {
            issue.state = State::Ready;
        }

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_created(&issue);
        self.storage.append_event(&event)?;

        Ok(issue.id)
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

    /// Update issue fields.
    ///
    /// Note: This function has 8 parameters (exceeds clippy's 7-parameter guideline).
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
    ) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

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

        let old_state = issue.state;

        if let Some(s) = state {
            // Validate state transition
            if s == State::Ready {
                // Check dependencies only (gates don't block Ready)
                let issues = self.storage.list_issues()?;
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!(
                        "Cannot transition to Ready: issue blocked by incomplete dependencies"
                    ));
                }

                issue.state = State::Ready;
            } else if s == State::Done {
                // Check both dependencies and gates
                let issues = self.storage.list_issues()?;
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!(
                        "Cannot transition to Done: issue blocked by incomplete dependencies"
                    ));
                }

                // If gates not passed, transition to Gated and return error
                if issue.has_unpassed_gates() {
                    return self.handle_gate_blocking(&mut issue, old_state);
                } else {
                    issue.state = State::Done;
                }
            } else {
                issue.state = s;
            }

            // Log state change event
            if old_state != issue.state {
                let event =
                    Event::new_issue_state_changed(issue.id.clone(), old_state, issue.state);
                self.storage.append_event(&event)?;

                // Log completion event if transitioning to Done
                if issue.state == State::Done {
                    let event = Event::new_issue_completed(issue.id.clone());
                    self.storage.append_event(&event)?;
                }
            }
        }

        self.storage.save_issue(&issue)?;

        // Check if any dependent issues can now transition to ready (after save!)
        if let Some(s) = state {
            if s.is_terminal() {
                self.check_auto_transitions()?;
            }
        }

        Ok(())
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        self.storage.delete_issue(&full_id)
    }

    /// Update issue state with precheck/postcheck hooks
    ///
    /// This method runs prechecks before transitioning to InProgress
    /// and postchecks when transitioning to Gated.
    pub fn update_issue_state(&self, id: &str, new_state: State) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
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
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!(
                        "Cannot transition to Ready: issue blocked by incomplete dependencies"
                    ));
                }

                issue.state = State::Ready;
            }
            State::Done => {
                // Check both dependencies and gates
                let issues = self.storage.list_issues()?;
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!(
                        "Cannot transition to Done: issue blocked by incomplete dependencies"
                    ));
                }

                // If gates not passed, transition to Gated and return error
                if issue.has_unpassed_gates() {
                    return self.handle_gate_blocking(&mut issue, old_state);
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
                self.storage.save_issue(&issue)?;

                // Log state change event
                let event =
                    Event::new_issue_state_changed(issue.id.clone(), old_state, State::Gated);
                self.storage.append_event(&event)?;

                // Run postchecks (which may auto-transition to Done)
                self.run_postchecks(&full_id)?;
                return Ok(());
            }
            _ => {
                issue.state = new_state;
            }
        }

        // Save and log
        if old_state != issue.state {
            self.storage.save_issue(&issue)?;

            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, issue.state);
            self.storage.append_event(&event)?;

            // Log completion event if transitioning to Done
            if issue.state == State::Done {
                let event = Event::new_issue_completed(issue.id.clone());
                self.storage.append_event(&event)?;
            }
        } else {
            self.storage.save_issue(&issue)?;
        }

        Ok(())
    }

    pub fn assign_issue(&self, id: &str, assignee: String) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = Some(assignee);
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        let issue = self.storage.load_issue(&full_id)?;

        if issue.assignee.is_some() {
            return Err(anyhow!("Issue is already assigned"));
        }

        let old_state = issue.state;

        // If Ready, try to transition to InProgress first (this enforces prechecks)
        if old_state == State::Ready {
            self.update_issue_state(&full_id, State::InProgress)?;
        }

        // If we get here, prechecks passed (or issue wasn't Ready)
        // Now assign the issue
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = Some(assignee.clone());
        self.storage.save_issue(&issue)?;

        // Log assignment event
        let event = Event::new_issue_claimed(issue.id.clone(), assignee);
        self.storage.append_event(&event)?;

        Ok(())
    }

    pub fn unassign_issue(&self, id: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(id)?;
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.assignee = None;
        self.storage.save_issue(&issue)?;
        Ok(())
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

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_released(
            full_id.clone(),
            old_assignee.unwrap_or_default(),
            reason.to_string(),
        );
        self.storage.append_event(&event)?;

        // Log state change if it occurred
        if old_state != issue.state {
            let event = Event::new_issue_state_changed(full_id, old_state, issue.state);
            self.storage.append_event(&event)?;
        }

        Ok(())
    }

    pub fn claim_next(&self, assignee: String, _filter: Option<String>) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        // Find first ready, unassigned issue with highest priority
        let mut candidates: Vec<&Issue> = issues
            .iter()
            .filter(|i| i.assignee.is_none() && !i.is_blocked(&resolved))
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
        let resolved: HashMap<String, &Issue> = issues.iter().map(|i| (i.id.clone(), i)).collect();

        let mut issue = self.storage.load_issue(&full_id)?;

        if issue.should_auto_transition_to_ready(&resolved) {
            let old_state = issue.state;
            issue.state = State::Ready;
            self.storage.save_issue(&issue)?;

            // Log state change event
            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Ready);
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
            self.storage.save_issue(&issue)?;

            // Log state change event
            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Done);
            self.storage.append_event(&event)?;

            // Log completion event
            let event = Event::new_issue_completed(issue.id.clone());
            self.storage.append_event(&event)?;

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

    /// Helper to handle gate blocking when transitioning to Done
    ///
    /// Transitions issue to Gated, saves, logs, and returns error with clear feedback
    fn handle_gate_blocking(&self, issue: &mut Issue, old_state: State) -> Result<()> {
        let unpassed = issue.get_unpassed_gates();
        issue.state = State::Gated;

        // Save the state change before returning error
        self.storage.save_issue(issue)?;

        // Log state change event
        let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Gated);
        self.storage.append_event(&event)?;

        Err(anyhow!(
            "Gate validation failed: Cannot transition to 'done' - {} gate(s) not passed: {}\n\
             → Issue automatically transitioned to 'gated' (awaiting gate approval)\n\
             The issue will auto-transition to 'done' when all gates pass.",
            unpassed.len(),
            unpassed.join(", ")
        ))
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();

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
        executor.storage.save_issue(&dep).unwrap();

        // Create issue that depends on it
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.dependencies.push(dep_id.clone());
        issue.state = State::Backlog; // Blocked
        let issue_id = issue.id.clone();
        executor.storage.save_issue(&issue).unwrap();

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
        executor.storage.save_issue(&issue).unwrap();
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
    fn test_manual_transition_to_ready_state_actually_changes_state() {
        let executor = setup();

        // Create dependency issue that is Done
        let mut dep = crate::domain::Issue::new("Dependency".to_string(), "Dep".to_string());
        dep.state = State::Done;
        let dep_id = dep.id.clone();
        executor.storage.save_issue(&dep).unwrap();

        // Create issue in Backlog that depends on the Done dependency
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::Backlog;
        issue.dependencies.push(dep_id.clone());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(&issue).unwrap();

        // Manually transition to Ready should succeed (dependency is done)
        let result = executor.update_issue(
            &issue_id,
            None,               // title
            None,               // description
            None,               // priority
            Some(State::Ready), // state
            vec![],             // add_labels
            vec![],             // remove_labels
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
