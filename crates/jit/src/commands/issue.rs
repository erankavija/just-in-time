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
        let namespaces = self.storage.load_label_namespaces()?;
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
        self.storage.load_issue(id)
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
        let mut issue = self.storage.load_issue(id)?;

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

                // If gates not passed, transition to Gated instead
                if issue.has_unpassed_gates() {
                    issue.state = State::Gated;
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
            if s == State::Done {
                self.check_auto_transitions()?;
            }
        }

        Ok(())
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        self.storage.delete_issue(id)
    }

    /// Update issue state with precheck/postcheck hooks
    ///
    /// This method runs prechecks before transitioning to InProgress
    /// and postchecks when transitioning to Gated.
    pub fn update_issue_state(&self, id: &str, new_state: State) -> Result<()> {
        let issue = self.storage.load_issue(id)?;
        let old_state = issue.state;

        // Handle prechecks for Ready -> InProgress transition
        if old_state == State::Ready && new_state == State::InProgress {
            self.run_prechecks(id)?;
        }

        // Reload issue after prechecks (which may have modified it)
        let mut issue = self.storage.load_issue(id)?;

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

                // If gates not passed, transition to Gated instead
                if issue.has_unpassed_gates() {
                    issue.state = State::Gated;
                } else {
                    issue.state = State::Done;
                }
            }
            State::Gated => {
                // Run postchecks when moving to Gated
                issue.state = State::Gated;
                self.storage.save_issue(&issue)?;
                
                // Log state change event
                let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Gated);
                self.storage.append_event(&event)?;
                
                // Run postchecks (which may auto-transition to Done)
                self.run_postchecks(id)?;
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
        let mut issue = self.storage.load_issue(id)?;
        issue.assignee = Some(assignee);
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;

        if issue.assignee.is_some() {
            return Err(anyhow!("Issue is already assigned"));
        }

        let old_state = issue.state;
        issue.assignee = Some(assignee.clone());

        // Transition to InProgress if Ready
        if issue.state == State::Ready {
            issue.state = State::InProgress;
        }

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_claimed(issue.id.clone(), assignee);
        self.storage.append_event(&event)?;

        // Log state change if needed
        if old_state != issue.state {
            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, issue.state);
            self.storage.append_event(&event)?;
        }

        Ok(())
    }

    pub fn unassign_issue(&self, id: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;
        issue.assignee = None;
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn release_issue(&self, id: &str, reason: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;
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
            id.to_string(),
            old_assignee.unwrap_or_default(),
            reason.to_string(),
        );
        self.storage.append_event(&event)?;

        // Log state change if it occurred
        if old_state != issue.state {
            let event = Event::new_issue_state_changed(id.to_string(), old_state, issue.state);
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
        let issues = self.storage.list_issues()?;
        let resolved: HashMap<String, &Issue> = issues.iter().map(|i| (i.id.clone(), i)).collect();

        let mut issue = self.storage.load_issue(issue_id)?;

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
        let mut issue = self.storage.load_issue(issue_id)?;

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
}
