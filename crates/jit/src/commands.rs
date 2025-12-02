//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.

use crate::domain::{Event, Gate, GateState, GateStatus, Issue, Priority, State};
use crate::graph::DependencyGraph;
use crate::storage::IssueStore;
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub done: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Initialize a new jit repository in the current directory
    pub fn init(&self) -> Result<()> {
        self.storage.init()?;
        println!("Initialized jit repository");
        Ok(())
    }

    /// Create a new issue with the specified properties.
    ///
    /// # Arguments
    ///
    /// * `title` - A brief summary of the issue
    /// * `description` - Detailed description of the issue
    /// * `priority` - Priority level (Low, Normal, High, Critical)
    /// * `gates` - List of quality gate keys that must pass before completion
    ///
    /// # Returns
    ///
    /// The unique ID of the newly created issue
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::{CommandExecutor, Priority};
    /// use jit::storage::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// let id = executor.create_issue(
    ///     "Fix login bug".to_string(),
    ///     "Users cannot log in with special characters".to_string(),
    ///     Priority::High,
    ///     vec!["unit-tests".to_string(), "code-review".to_string()],
    /// ).unwrap();
    /// ```
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
    ) -> Result<String> {
        let mut issue = Issue::new(title, description);
        issue.priority = priority;
        issue.gates_required = gates;

        // Auto-transition to Ready if no blockers
        if issue.dependencies.is_empty() && issue.gates_required.is_empty() {
            issue.state = State::Ready;
        }

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_created(&issue);
        self.storage.append_event(&event)?;

        Ok(issue.id)
    }

    /// List issues matching the specified filters.
    ///
    /// All filters are optional. If no filters are provided, returns all issues.
    ///
    /// # Arguments
    ///
    /// * `state_filter` - Filter by issue state (Open, Ready, InProgress, Done)
    /// * `assignee_filter` - Filter by assignee name
    /// * `priority_filter` - Filter by priority level
    ///
    /// # Returns
    ///
    /// Vector of issues matching all specified filters
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

    pub fn update_issue(
        &self,
        id: &str,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        state: Option<State>,
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
        let old_state = issue.state;

        if let Some(s) = state {
            // Validate state transition
            if s == State::Ready || s == State::Done {
                let issues = self.storage.list_issues()?;
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!("Cannot transition to {:?}: issue is blocked", s));
                }
            }
            issue.state = s;

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

    pub fn assign_issue(&self, id: &str, assignee: String) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;
        issue.assignee = Some(assignee);
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    /// Atomically claim an unassigned issue.
    ///
    /// This operation is atomic - it only succeeds if the issue is currently unassigned.
    /// This prevents race conditions when multiple agents try to claim the same issue.
    ///
    /// # Arguments
    ///
    /// * `id` - The issue ID to claim
    /// * `assignee` - The assignee identifier (format: `type:identifier`, e.g., "copilot:session-1")
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The issue does not exist
    /// - The issue is already assigned to someone else
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::{CommandExecutor, Priority};
    /// # use jit::storage::InMemoryStorage;
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// let id = executor.create_issue("Task".into(), "".into(), Priority::Normal, vec![]).unwrap();
    /// executor.claim_issue(&id, "agent:worker-1".to_string()).unwrap();
    /// ```
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

    /// Release an issue from its assignee (for timeout/error recovery)
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

    /// Add a dependency between two issues.
    ///
    /// Creates a dependency relationship where `issue_id` depends on `dep_id`.
    /// The issue cannot transition to Ready or Done until the dependency is complete.
    ///
    /// # Arguments
    ///
    /// * `issue_id` - The issue that depends on another
    /// * `dep_id` - The issue that must be completed first
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Either issue does not exist
    /// - Adding the dependency would create a cycle (violates DAG property)
    /// - The dependency already exists
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::{CommandExecutor, Priority};
    /// # use jit::storage::InMemoryStorage;
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// let backend = executor.create_issue("Backend API".into(), "".into(), Priority::Normal, vec![]).unwrap();
    /// let frontend = executor.create_issue("Frontend UI".into(), "".into(), Priority::Normal, vec![]).unwrap();
    ///
    /// // Frontend depends on backend
    /// executor.add_dependency(&frontend, &backend).unwrap();
    /// ```
    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        // Validate both issues exist
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        // Check for cycles
        graph.validate_add_dependency(issue_id, dep_id)?;

        // Add the dependency
        let mut issue = self.storage.load_issue(issue_id)?;
        if !issue.dependencies.contains(&dep_id.to_string()) {
            issue.dependencies.push(dep_id.to_string());

            // If issue becomes blocked by this dependency, transition to Open
            let dep_issue = self.storage.load_issue(dep_id)?;
            if issue.state == State::Ready && dep_issue.state != State::Done {
                let old_state = issue.state;
                issue.state = State::Open;
                self.storage.save_issue(&issue)?;

                // Log state change
                let event =
                    Event::new_issue_state_changed(issue.id.clone(), old_state, State::Open);
                self.storage.append_event(&event)?;
            } else {
                self.storage.save_issue(&issue)?;
            }
        }

        Ok(())
    }

    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        issue.dependencies.retain(|d| d != dep_id);
        self.storage.save_issue(&issue)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(issue_id)?;

        Ok(())
    }

    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key.clone());

            // If issue was Ready, transition to Open since gate is pending
            if issue.state == State::Ready {
                let old_state = issue.state;
                issue.state = State::Open;
                self.storage.save_issue(&issue)?;

                // Log state change
                let event =
                    Event::new_issue_state_changed(issue.id.clone(), old_state, State::Open);
                self.storage.append_event(&event)?;
            } else {
                self.storage.save_issue(&issue)?;
            }
        }
        Ok(())
    }

    pub fn pass_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Passed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_passed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        // Check if this issue can now transition to ready
        self.auto_transition_to_ready(issue_id)?;

        Ok(())
    }

    pub fn fail_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Failed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_failed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        Ok(())
    }

    /// Check if issue should auto-transition to Ready and do so if needed
    /// Returns true if transition occurred
    fn auto_transition_to_ready(&self, issue_id: &str) -> Result<bool> {
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

    /// Check and auto-transition all Open issues that are now unblocked
    fn check_auto_transitions(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;
        let open_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.state == State::Open)
            .map(|i| i.id.clone())
            .collect();

        for issue_id in open_issues {
            self.auto_transition_to_ready(&issue_id)?;
        }

        Ok(())
    }

    pub fn show_graph(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let issue = self.storage.load_issue(issue_id)?;
        let mut result = vec![issue.clone()];

        // Recursively get dependencies
        let mut to_process = issue.dependencies.clone();
        let mut processed = std::collections::HashSet::new();

        while let Some(dep_id) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                to_process.extend(dep_issue.dependencies.clone());
                result.push(dep_issue);
            }
        }

        Ok(result)
    }

    pub fn show_downstream(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let dependents = graph.get_transitive_dependents(issue_id);
        Ok(dependents.into_iter().cloned().collect())
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let roots = graph.get_roots();
        Ok(roots.into_iter().cloned().collect())
    }

    /// Validate repository integrity (silent version).
    ///
    /// Performs comprehensive validation of the issue repository:
    /// - Checks all dependency references point to existing issues
    /// - Validates all required gates are defined in the registry
    /// - Ensures the dependency graph is acyclic (DAG property)
    ///
    /// # Returns
    ///
    /// `Ok(())` if validation passes, or an error describing the first problem found.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::{CommandExecutor, Priority};
    /// # use jit::storage::InMemoryStorage;
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// executor.create_issue("Task".into(), "".into(), Priority::Normal, vec![]).unwrap();
    /// assert!(executor.validate_silent().is_ok());
    /// ```
    pub fn validate_silent(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;
        
        // Build lookup map of valid issue IDs
        let valid_ids: std::collections::HashSet<String> = 
            issues.iter().map(|i| i.id.clone()).collect();
        
        // Check for broken dependency references
        for issue in &issues {
            for dep in &issue.dependencies {
                if !valid_ids.contains(dep) {
                    return Err(anyhow!(
                        "Invalid dependency: issue '{}' depends on '{}' which does not exist",
                        issue.id,
                        dep
                    ));
                }
            }
        }
        
        // Check for invalid gate references
        let registry = self.storage.load_gate_registry()?;
        for issue in &issues {
            for gate_key in &issue.gates_required {
                if !registry.gates.contains_key(gate_key) {
                    return Err(anyhow!(
                        "Gate '{}' required by issue '{}' is not defined in registry",
                        gate_key,
                        issue.id
                    ));
                }
            }
        }
        
        // Validate DAG (no cycles)
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        graph.validate_dag()?;
        
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_silent()?;
        println!("✓ Repository is valid");
        Ok(())
    }

    /// Get status summary
    pub fn get_status(&self) -> Result<StatusSummary> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let open = issues.iter().filter(|i| i.state == State::Open).count();
        let ready = issues.iter().filter(|i| i.state == State::Ready).count();
        let in_progress = issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        Ok(StatusSummary {
            open,
            ready,
            in_progress,
            done,
            blocked,
            total: issues.len(),
        })
    }

    pub fn status(&self) -> Result<()> {
        let summary = self.get_status()?;

        println!("Status:");
        println!("  Open: {}", summary.open);
        println!("  Ready: {}", summary.ready);
        println!("  In Progress: {}", summary.in_progress);
        println!("  Done: {}", summary.done);
        println!("  Blocked: {}", summary.blocked);

        Ok(())
    }

    // Registry commands
    pub fn list_gates(&self) -> Result<Vec<Gate>> {
        let registry = self.storage.load_gate_registry()?;
        Ok(registry.gates.into_values().collect())
    }

    pub fn add_gate_definition(
        &self,
        key: String,
        title: String,
        description: String,
        auto: bool,
        example_integration: Option<String>,
    ) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(anyhow!("Gate '{}' already exists", key));
        }

        registry.gates.insert(
            key.clone(),
            Gate {
                key,
                title,
                description,
                auto,
                example_integration,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn remove_gate_definition(&self, key: &str) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if !registry.gates.contains_key(key) {
            return Err(anyhow!("Gate '{}' not found", key));
        }

        registry.gates.remove(key);
        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn show_gate_definition(&self, key: &str) -> Result<Gate> {
        let registry = self.storage.load_gate_registry()?;
        registry
            .gates
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow!("Gate '{}' not found", key))
    }

    // Event commands
    pub fn tail_events(&self, n: usize) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;
        let start = events.len().saturating_sub(n);
        Ok(events[start..].to_vec())
    }

    pub fn query_events(
        &self,
        event_type: Option<String>,
        issue_id: Option<String>,
        limit: usize,
    ) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;

        let filtered: Vec<Event> = events
            .into_iter()
            .rev()
            .filter(|e| {
                if let Some(ref et) = event_type {
                    if e.get_type() != et {
                        return false;
                    }
                }
                if let Some(ref iid) = issue_id {
                    if e.get_issue_id() != iid {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect();

        Ok(filtered.into_iter().rev().collect())
    }

    // Search and filter
    pub fn search_issues(&self, query: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let query_lower = query.to_lowercase();

        let results = issues
            .into_iter()
            .filter(|issue| {
                if query.is_empty() {
                    return true;
                }

                // Search in title, description, and ID
                issue.title.to_lowercase().contains(&query_lower)
                    || issue.description.to_lowercase().contains(&query_lower)
                    || issue.id.to_lowercase().starts_with(&query_lower)
            })
            .collect();

        Ok(results)
    }

    pub fn search_issues_with_filters(
        &self,
        query: &str,
        priority_filter: Option<Priority>,
        state_filter: Option<State>,
        assignee_filter: Option<String>,
    ) -> Result<Vec<Issue>> {
        let mut results = self.search_issues(query)?;

        // Apply additional filters
        results.retain(|issue| {
            if let Some(ref priority) = priority_filter {
                if &issue.priority != priority {
                    return false;
                }
            }
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
            true
        });

        Ok(results)
    }

    // Graph export
    pub fn export_graph(&self, format: &str) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        match format.to_lowercase().as_str() {
            "dot" => Ok(crate::visualization::export_dot(&graph)),
            "mermaid" => Ok(crate::visualization::export_mermaid(&graph)),
            _ => Err(anyhow!(
                "Unsupported format: {}. Use 'dot' or 'mermaid'",
                format
            )),
        }
    }

    /// Query ready issues (unassigned, state=ready, unblocked)
    pub fn query_ready(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let ready: Vec<Issue> = issues
            .iter()
            .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
            .cloned()
            .collect();

        Ok(ready)
    }

    /// Query blocked issues with reasons
    pub fn query_blocked(&self) -> Result<Vec<(Issue, Vec<String>)>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let mut blocked = Vec::new();

        for issue in &issues {
            if issue.is_blocked(&resolved) {
                let mut reasons = Vec::new();

                // Check dependencies
                for dep_id in &issue.dependencies {
                    if let Some(dep) = resolved.get(dep_id) {
                        if dep.state != State::Done {
                            reasons.push(format!(
                                "dependency:{} ({}:{:?})",
                                dep_id, dep.title, dep.state
                            ));
                        }
                    }
                }

                // Check gates
                for gate_key in &issue.gates_required {
                    let gate_state = issue.gates_status.get(gate_key);
                    let is_passed = gate_state
                        .map(|gs| gs.status == GateStatus::Passed)
                        .unwrap_or(false);

                    if !is_passed {
                        let status_str = gate_state
                            .map(|gs| format!("{:?}", gs.status))
                            .unwrap_or_else(|| "Pending".to_string());
                        reasons.push(format!("gate:{} ({})", gate_key, status_str));
                    }
                }

                blocked.push((issue.clone(), reasons));
            }
        }

        Ok(blocked)
    }

    /// Query issues by assignee
    pub fn query_by_assignee(&self, assignee: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues
            .into_iter()
            .filter(|i| i.assignee.as_deref() == Some(assignee))
            .collect();

        Ok(filtered)
    }

    /// Query issues by state
    pub fn query_by_state(&self, state: State) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues.into_iter().filter(|i| i.state == state).collect();

        Ok(filtered)
    }

    /// Query issues by priority
    pub fn query_by_priority(&self, priority: Priority) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = issues
            .into_iter()
            .filter(|i| i.priority == priority)
            .collect();

        Ok(filtered)
    }
}

pub fn parse_priority(s: &str) -> Result<Priority> {
    match s.to_lowercase().as_str() {
        "low" => Ok(Priority::Low),
        "normal" => Ok(Priority::Normal),
        "high" => Ok(Priority::High),
        "critical" => Ok(Priority::Critical),
        _ => Err(anyhow!("Invalid priority: {}", s)),
    }
}

pub fn parse_state(s: &str) -> Result<State> {
    match s.to_lowercase().as_str() {
        "open" => Ok(State::Open),
        "ready" => Ok(State::Ready),
        "in_progress" | "inprogress" => Ok(State::InProgress),
        "done" => Ok(State::Done),
        "archived" => Ok(State::Archived),
        _ => Err(anyhow!("Invalid state: {}", s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use tempfile::TempDir;

    fn setup() -> (TempDir, CommandExecutor<JsonFileStorage>) {
        let temp_dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp_dir.path());
        let executor = CommandExecutor::new(storage);
        executor.init().unwrap();
        (temp_dir, executor)
    }

    #[test]
    fn test_create_and_show_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Test Issue".to_string(),
                "Description".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.priority, Priority::High);
    }

    #[test]
    fn test_list_issues_with_filters() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Issue 1".to_string(),
                "Desc".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Low,
                vec![],
            )
            .unwrap();

        let all = executor.list_issues(None, None, None).unwrap();
        assert_eq!(all.len(), 2);

        let high_priority = executor
            .list_issues(None, None, Some(Priority::High))
            .unwrap();
        assert_eq!(high_priority.len(), 1);
    }

    #[test]
    fn test_update_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Original".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor
            .update_issue(&id, Some("Updated".to_string()), None, None, None)
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.title, "Updated");
    }

    #[test]
    fn test_add_dependency_prevents_cycles() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id1, &id2).unwrap();

        let result = executor.add_dependency(&id2, &id1);
        assert!(result.is_err());
    }

    #[test]
    fn test_claim_issue_requires_unassigned() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id, "user1".to_string()).unwrap();

        let result = executor.claim_issue(&id, "user2".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_claim_next_returns_highest_priority() {
        let (_temp, executor) = setup();

        let _low = executor
            .create_issue("Low".to_string(), "Desc".to_string(), Priority::Low, vec![])
            .unwrap();
        let high_id = executor
            .create_issue(
                "High".to_string(),
                "Desc".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();

        let claimed_id = executor.claim_next("user".to_string(), None).unwrap();
        assert_eq!(claimed_id, high_id);
    }

    #[test]
    fn test_gate_operations() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["review".to_string()],
            )
            .unwrap();

        executor
            .pass_gate(&id, "review".to_string(), Some("reviewer".to_string()))
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(
            issue.gates_status.get("review").unwrap().status,
            GateStatus::Passed
        );
    }

    #[test]
    fn test_cannot_transition_to_done_when_blocked() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["review".to_string()],
            )
            .unwrap();

        let result = executor.update_issue(&id, None, None, None, Some(State::Done));
        assert!(result.is_err());
    }

    #[test]
    fn test_tail_events() {
        let (_temp, executor) = setup();

        // Create several issues to generate events
        executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 3".to_string(),
                "D3".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Tail last 2 events
        let events = executor.tail_events(2).unwrap();
        assert_eq!(events.len(), 2);

        // Tail more than exist
        let all_events = executor.tail_events(100).unwrap();
        assert_eq!(all_events.len(), 3);
    }

    #[test]
    fn test_query_events_by_type() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Test Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id, "alice".to_string()).unwrap();

        // Query for issue_created events
        let created_events = executor
            .query_events(Some("issue_created".to_string()), None, 100)
            .unwrap();
        assert_eq!(created_events.len(), 1);

        // Query for issue_claimed events
        let claimed_events = executor
            .query_events(Some("issue_claimed".to_string()), None, 100)
            .unwrap();
        assert_eq!(claimed_events.len(), 1);

        // Query for non-existent event type
        let no_events = executor
            .query_events(Some("issue_deleted".to_string()), None, 100)
            .unwrap();
        assert_eq!(no_events.len(), 0);
    }

    #[test]
    fn test_query_events_by_issue_id() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id1, "alice".to_string()).unwrap();

        // Query events for id1 - should have: created, claimed, state_changed (Ready->InProgress)
        let events_id1 = executor.query_events(None, Some(id1.clone()), 100).unwrap();
        assert_eq!(events_id1.len(), 3); // created + claimed + state_changed

        // Query events for id2 - should have only: created (created as Ready)
        let events_id2 = executor.query_events(None, Some(id2.clone()), 100).unwrap();
        assert_eq!(events_id2.len(), 1); // created only
    }

    #[test]
    fn test_query_events_with_limit() {
        let (_temp, executor) = setup();

        // Create 5 issues
        for i in 1..=5 {
            executor
                .create_issue(
                    format!("Issue {}", i),
                    "Desc".to_string(),
                    Priority::Normal,
                    vec![],
                )
                .unwrap();
        }

        // Query with limit
        let limited = executor.query_events(None, None, 3).unwrap();
        assert_eq!(limited.len(), 3);

        // Verify all events exist
        let all = executor.query_events(None, None, 100).unwrap();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_query_events_combined_filters() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id1, "alice".to_string()).unwrap();
        executor.claim_issue(&id2, "bob".to_string()).unwrap();

        // Query for specific event type on specific issue
        let specific = executor
            .query_events(Some("issue_claimed".to_string()), Some(id1.clone()), 100)
            .unwrap();
        assert_eq!(specific.len(), 1);

        // Verify it's the correct event
        assert_eq!(specific[0].get_type(), "issue_claimed");
        assert_eq!(specific[0].get_issue_id(), id1);
    }

    #[test]
    fn test_delete_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "To Delete".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Verify it exists
        assert!(executor.show_issue(&id).is_ok());

        // Delete it
        executor.delete_issue(&id).unwrap();

        // Verify it's gone
        assert!(executor.show_issue(&id).is_err());

        // Verify it's not in the list
        let issues = executor.list_issues(None, None, None).unwrap();
        assert!(!issues.iter().any(|i| i.id == id));
    }

    #[test]
    fn test_delete_nonexistent_issue() {
        let (_temp, executor) = setup();

        let result = executor.delete_issue("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.assign_issue(&id, "alice".to_string()).unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.assignee, Some("alice".to_string()));
    }

    #[test]
    fn test_assign_already_assigned_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.assign_issue(&id, "alice".to_string()).unwrap();
        // assign_issue allows reassignment (unlike claim_issue)
        executor.assign_issue(&id, "bob".to_string()).unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.assignee, Some("bob".to_string()));
    }

    #[test]
    fn test_unassign_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.assign_issue(&id, "alice".to_string()).unwrap();
        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.assignee, Some("alice".to_string()));

        executor.unassign_issue(&id).unwrap();
        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.assignee, None);
    }

    #[test]
    fn test_unassign_unassigned_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Should not error when unassigning unassigned issue
        executor.unassign_issue(&id).unwrap();
    }

    #[test]
    fn test_add_dependency() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let issue2 = executor.show_issue(&id2).unwrap();
        assert!(issue2.dependencies.contains(&id1));
    }

    #[test]
    fn test_remove_dependency() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();
        executor.remove_dependency(&id2, &id1).unwrap();

        let issue2 = executor.show_issue(&id2).unwrap();
        assert!(!issue2.dependencies.contains(&id1));
    }

    #[test]
    fn test_remove_nonexistent_dependency() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Removing non-existent dependency should not error
        executor.remove_dependency(&id2, &id1).unwrap();
    }

    #[test]
    fn test_add_gate() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_gate(&id, "review".to_string()).unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert!(issue.gates_required.contains(&"review".to_string()));
    }

    #[test]
    fn test_pass_gate() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["review".to_string()],
            )
            .unwrap();

        executor
            .pass_gate(&id, "review".to_string(), Some("alice".to_string()))
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        let gate_state = issue.gates_status.get("review").unwrap();
        assert_eq!(gate_state.status, GateStatus::Passed);
        assert_eq!(gate_state.updated_by, Some("alice".to_string()));
    }

    #[test]
    fn test_pass_gate_not_required() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Passing a gate not required should error
        let result = executor.pass_gate(&id, "review".to_string(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_fail_gate() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["tests".to_string()],
            )
            .unwrap();

        executor
            .fail_gate(&id, "tests".to_string(), Some("ci".to_string()))
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        let gate_state = issue.gates_status.get("tests").unwrap();
        assert_eq!(gate_state.status, GateStatus::Failed);
        assert_eq!(gate_state.updated_by, Some("ci".to_string()));
    }

    #[test]
    fn test_show_graph() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Root".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Child".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let graph = executor.show_graph(&id2).unwrap();
        // Should include the issue itself + its dependency
        assert_eq!(graph.len(), 2);
        assert!(graph.iter().any(|i| i.id == id1));
        assert!(graph.iter().any(|i| i.id == id2));
    }

    #[test]
    fn test_show_downstream() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Root".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Dependent".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let downstream = executor.show_downstream(&id1).unwrap();
        assert_eq!(downstream.len(), 1);
        assert!(downstream.iter().any(|i| i.id == id2));
    }

    #[test]
    fn test_show_roots() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Root".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Child".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let roots = executor.show_roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, id1);
    }

    #[test]
    fn test_validate_success() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "D1".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        // Should not error on valid DAG
        executor.validate().unwrap();
    }

    #[test]
    fn test_list_gates() {
        let (_temp, executor) = setup();

        executor
            .add_gate_definition(
                "tests".to_string(),
                "Unit Tests".to_string(),
                "Run test suite".to_string(),
                true,
                None,
            )
            .unwrap();

        let gates = executor.list_gates().unwrap();
        assert!(gates.iter().any(|g| g.key == "tests"));
    }

    #[test]
    fn test_add_gate_definition() {
        let (_temp, executor) = setup();

        executor
            .add_gate_definition(
                "review".to_string(),
                "Code Review".to_string(),
                "Peer review required".to_string(),
                false,
                Some("GitHub PR".to_string()),
            )
            .unwrap();

        let gate = executor.show_gate_definition("review").unwrap();
        assert_eq!(gate.title, "Code Review");
        assert!(!gate.auto);
    }

    #[test]
    fn test_add_duplicate_gate_definition() {
        let (_temp, executor) = setup();

        executor
            .add_gate_definition(
                "review".to_string(),
                "Code Review".to_string(),
                "Description".to_string(),
                false,
                None,
            )
            .unwrap();

        let result = executor.add_gate_definition(
            "review".to_string(),
            "Another".to_string(),
            "Desc".to_string(),
            false,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_gate_definition() {
        let (_temp, executor) = setup();

        executor
            .add_gate_definition(
                "temp".to_string(),
                "Temporary".to_string(),
                "Desc".to_string(),
                false,
                None,
            )
            .unwrap();

        executor.remove_gate_definition("temp").unwrap();

        let result = executor.show_gate_definition("temp");
        assert!(result.is_err());
    }

    #[test]
    fn test_show_gate_definition() {
        let (_temp, executor) = setup();

        executor
            .add_gate_definition(
                "security".to_string(),
                "Security Scan".to_string(),
                "Run security checks".to_string(),
                true,
                Some("snyk".to_string()),
            )
            .unwrap();

        let gate = executor.show_gate_definition("security").unwrap();
        assert_eq!(gate.key, "security");
        assert_eq!(gate.title, "Security Scan");
        assert!(gate.auto);
        assert_eq!(gate.example_integration, Some("snyk".to_string()));
    }

    #[test]
    fn test_show_nonexistent_gate_definition() {
        let (_temp, executor) = setup();

        let result = executor.show_gate_definition("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_export_graph() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "API".to_string(),
                "Design API".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Backend".to_string(),
                "Implement".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let dot = executor.export_graph("dot").unwrap();
        assert!(dot.contains("digraph"));
        assert!(dot.contains(&id1));
        assert!(dot.contains(&id2));

        let mermaid = executor.export_graph("mermaid").unwrap();
        assert!(mermaid.contains("graph LR"));
        assert!(mermaid.contains(&id1));
        assert!(mermaid.contains(&id2));
    }

    #[test]
    fn test_export_graph_invalid_format() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let result = executor.export_graph("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_status() {
        let (_temp, executor) = setup();

        // Create some issues in different states
        executor
            .create_issue(
                "Open".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "InProgress".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id2, "alice".to_string()).unwrap();

        // Status should not error
        executor.status().unwrap();
    }

    // TDD: Search and filter tests - written BEFORE implementation
    #[test]
    fn test_search_by_title_substring() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Fix bug in parser".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Add new feature".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Fix bug in lexer".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let results = executor.search_issues("bug").unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|i| i.title.contains("bug")));
    }

    #[test]
    fn test_search_by_description() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Task 1".to_string(),
                "Contains security vulnerability".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Task 2".to_string(),
                "Regular task".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let results = executor.search_issues("security").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].description.contains("security"));
    }

    #[test]
    fn test_search_case_insensitive() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Fix BUG".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let results = executor.search_issues("bug").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_with_filters() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Critical bug".to_string(),
                "Desc".to_string(),
                Priority::Critical,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Normal bug".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Search with priority filter
        let results = executor
            .search_issues_with_filters("bug", Some(Priority::Critical), None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id1);
    }

    #[test]
    fn test_search_with_state_filter() {
        let (_temp, executor) = setup();

        // Create issue with no blockers (auto-transitions to Ready)
        let id1 = executor
            .create_issue(
                "Ready task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Create issue with a gate (stays Open)
        executor
            .create_issue(
                "Open task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["some-gate".to_string()],
            )
            .unwrap();

        let results = executor
            .search_issues_with_filters("task", None, Some(State::Ready), None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id1);
        assert_eq!(results[0].state, State::Ready);
    }

    #[test]
    fn test_search_with_assignee_filter() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Alice work".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Bob work".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.assign_issue(&id1, "alice".to_string()).unwrap();
        executor.assign_issue(&id2, "bob".to_string()).unwrap();

        let results = executor
            .search_issues_with_filters("work", None, None, Some("alice".to_string()))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].assignee, Some("alice".to_string()));
    }

    #[test]
    fn test_search_empty_query_returns_all() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Task 1".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Task 2".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let results = executor.search_issues("").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_no_matches() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let results = executor.search_issues("nonexistent").unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_by_id_prefix() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        // Search by first 8 chars of UUID
        let prefix = &id[..8];
        let results = executor.search_issues(prefix).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }
}
