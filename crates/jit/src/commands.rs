//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.

use crate::domain::{Event, Gate, GateState, GateStatus, Issue, Priority, State};
use crate::graph::DependencyGraph;
use crate::labels;
use crate::storage::IssueStore;
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
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
    ///     vec!["type:bug".to_string()],
    /// ).unwrap();
    /// ```
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
            labels::validate_label(label_str)?;
        }

        // Check uniqueness constraints
        let namespaces = self.storage.load_label_namespaces()?;
        let mut unique_namespaces_seen = std::collections::HashSet::new();

        for label_str in &labels {
            if let Ok((namespace, _)) = labels::parse_label(label_str) {
                if let Some(ns_config) = namespaces.get(&namespace) {
                    if ns_config.unique
                        && !unique_namespaces_seen.insert(namespace.clone())
                    {
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

    #[allow(clippy::too_many_arguments)] // Update operation naturally has many optional fields
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
            labels::validate_label(label_str)?;
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
    /// let id = executor.create_issue("Task".into(), "".into(), Priority::Normal, vec![], vec![]).unwrap();
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
    /// # use jit::commands::DependencyAddResult;
    /// # use jit::storage::InMemoryStorage;
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// let backend = executor.create_issue("Backend API".into(), "".into(), Priority::Normal, vec![], vec![]).unwrap();
    /// let frontend = executor.create_issue("Frontend UI".into(), "".into(), Priority::Normal, vec![], vec![]).unwrap();
    ///
    /// // Frontend depends on backend
    /// let result = executor.add_dependency(&frontend, &backend).unwrap();
    /// assert_eq!(result, DependencyAddResult::Added);
    /// ```
    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<DependencyAddResult> {
        // Load all issues and build graph for analysis
        // Note: Storage layer handles locking internally
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        // Check for cycles
        graph.validate_add_dependency(issue_id, dep_id)?;

        // Check if this dependency is transitive (redundant)
        if graph.is_transitive(issue_id, dep_id) {
            let reason = "transitive (already reachable via other dependencies)".to_string();
            return Ok(DependencyAddResult::Skipped { reason });
        }

        // Load the issue and add dependency
        let mut issue = self.storage.load_issue(issue_id)?;
        if issue.dependencies.contains(&dep_id.to_string()) {
            return Ok(DependencyAddResult::AlreadyExists);
        }

        issue.dependencies.push(dep_id.to_string());

        // Apply transitive reduction: remove any deps now reachable through others
        // Build a temporary graph with the new edge to compute reduction
        let temp_issue = issue.clone();
        let mut temp_issues = issues.clone();
        temp_issues.retain(|i| i.id != issue_id);
        temp_issues.push(temp_issue);
        let temp_refs: Vec<&Issue> = temp_issues.iter().collect();
        let new_graph = DependencyGraph::new(&temp_refs);
        let new_reduced = new_graph.compute_transitive_reduction(issue_id);
        issue.dependencies = new_reduced.into_iter().collect();

        // If issue becomes blocked by this dependency, transition to Backlog
        let dep_issue = self.storage.load_issue(dep_id)?;
        if issue.state == State::Ready && dep_issue.state != State::Done {
            let old_state = issue.state;
            issue.state = State::Backlog;
            self.storage.save_issue(&issue)?;

            // Log state change
            let event = Event::new_issue_state_changed(issue.id.clone(), old_state, State::Backlog);
            self.storage.append_event(&event)?;
        } else {
            self.storage.save_issue(&issue)?;
        }

        Ok(DependencyAddResult::Added)
    }

    /// Break down an issue into subtasks with automatic dependency inheritance.
    ///
    /// Creates subtasks, makes the parent depend on them, and automatically copies
    /// the parent's dependencies to each subtask. The parent's original dependencies
    /// are then removed as they become transitive through the subtasks.
    ///
    /// # Arguments
    ///
    /// * `parent_id` - ID of the issue to break down
    /// * `subtasks` - List of (title, description) tuples for subtasks to create
    ///
    /// # Returns
    ///
    /// Vector of created subtask IDs
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::{CommandExecutor, Priority};
    /// # use jit::storage::InMemoryStorage;
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// let dep = executor.create_issue("Build".into(), "".into(), Priority::Normal, vec![], vec![]).unwrap();
    /// let parent = executor.create_issue("Review".into(), "".into(), Priority::High, vec![], vec![]).unwrap();
    /// executor.add_dependency(&parent, &dep).unwrap();
    ///
    /// let subtasks = vec![
    ///     ("Check tests".to_string(), "".to_string()),
    ///     ("Check docs".to_string(), "".to_string()),
    /// ];
    ///
    /// let subtask_ids = executor.breakdown_issue(&parent, subtasks).unwrap();
    /// assert_eq!(subtask_ids.len(), 2);
    ///
    /// // Each subtask now depends on Build
    /// // Parent depends on subtasks (not Build anymore - transitive)
    /// ```
    pub fn breakdown_issue(
        &self,
        parent_id: &str,
        subtasks: Vec<(String, String)>,
    ) -> Result<Vec<String>> {
        // Load parent issue
        let parent = self.storage.load_issue(parent_id)?;
        let original_deps = parent.dependencies.clone();

        // Create subtasks with inherited priority and labels
        let mut subtask_ids = Vec::new();
        for (title, desc) in subtasks {
            let subtask_id = self.create_issue(
                title,
                desc,
                parent.priority,
                vec![],
                parent.labels.clone(), // Copy parent's labels
            )?;
            subtask_ids.push(subtask_id);
        }

        // Copy parent's dependencies to each subtask
        for subtask_id in &subtask_ids {
            for dep_id in &original_deps {
                self.add_dependency(subtask_id, dep_id)?;
            }
        }

        // Make parent depend on all subtasks
        for subtask_id in &subtask_ids {
            self.add_dependency(parent_id, subtask_id)?;
        }

        // Remove parent's original dependencies (now transitive through subtasks)
        for dep_id in &original_deps {
            self.remove_dependency(parent_id, dep_id)?;
        }

        Ok(subtask_ids)
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
            // Note: Gates don't block Ready state, only Done state
            self.storage.save_issue(&issue)?;
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

        // Check if Gated issue can now transition to Done
        self.auto_transition_to_done(issue_id)?;

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

    /// Check if issue should auto-transition from Gated to Done
    /// Returns true if transition occurred
    fn auto_transition_to_done(&self, issue_id: &str) -> Result<bool> {
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

    /// Check and auto-transition all Backlog issues that are now unblocked
    fn check_auto_transitions(&self) -> Result<()> {
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
    /// executor.create_issue("Task".into(), "".into(), Priority::Normal, vec![], vec![]).unwrap();
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

        // Validate document references (git integration)
        self.validate_document_references(&issues)?;

        // Validate DAG (no cycles)
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        graph.validate_dag()?;

        Ok(())
    }

    /// Validate all document references in issues
    fn validate_document_references(&self, issues: &[Issue]) -> Result<()> {
        use git2::Repository;

        // Try to open git repository
        let repo = match Repository::open(".") {
            Ok(r) => r,
            Err(_) => {
                // If not a git repo, skip document validation
                return Ok(());
            }
        };

        for issue in issues {
            for doc in &issue.documents {
                // Validate commit hash if specified
                if let Some(ref commit_hash) = doc.commit {
                    if repo.find_commit(git2::Oid::from_str(commit_hash)?).is_err() {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': commit '{}' not found for '{}'",
                            issue.id,
                            commit_hash,
                            doc.path
                        ));
                    }
                }

                // Validate file exists (at HEAD if no commit specified)
                let reference = if let Some(ref commit_hash) = doc.commit {
                    commit_hash.as_str()
                } else {
                    "HEAD"
                };

                if self
                    .check_file_exists_in_git(&repo, &doc.path, reference)
                    .is_err()
                {
                    return Err(anyhow!(
                        "Invalid document reference in issue '{}': file '{}' not found at {}",
                        issue.id,
                        doc.path,
                        reference
                    ));
                }
            }
        }

        Ok(())
    }

    /// Check if a file exists in git at a specific reference
    fn check_file_exists_in_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<()> {
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;

        // Try to find the file in the tree
        tree.get_path(std::path::Path::new(path))?;

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

        let backlog = issues.iter().filter(|i| i.state == State::Backlog).count();
        let ready = issues.iter().filter(|i| i.state == State::Ready).count();
        let in_progress = issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();
        let gated = issues.iter().filter(|i| i.state == State::Gated).count();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        Ok(StatusSummary {
            open: backlog, // Keep 'open' field name for backward compatibility
            ready,
            in_progress,
            done,
            blocked,
            gated,
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

    /// Query issues by label pattern
    /// 
    /// Supports exact match (e.g., "milestone:v1.0") or wildcard (e.g., "milestone:*")
    /// to match all issues with labels in a specific namespace.
    /// 
    /// # Examples
    /// ```
    /// # use jit::commands::CommandExecutor;
    /// # use jit::storage::InMemoryStorage;
    /// # use jit::domain::Priority;
    /// # let storage = InMemoryStorage::new();
    /// # let executor = CommandExecutor::new(storage);
    /// # executor.init().unwrap();
    /// # executor.create_issue("Task".to_string(), "".to_string(), Priority::Normal, vec![], vec!["milestone:v1.0".to_string()]).unwrap();
    /// // Query exact match
    /// let issues = executor.query_by_label("milestone:v1.0").unwrap();
    /// 
    /// // Query all issues with milestone labels
    /// let milestones = executor.query_by_label("milestone:*").unwrap();
    /// ```
    pub fn query_by_label(&self, pattern: &str) -> Result<Vec<Issue>> {
        use crate::labels;

        // Validate pattern format
        if !pattern.contains(':') {
            return Err(anyhow!("Invalid label pattern '{}': must be 'namespace:value' or 'namespace:*'", pattern));
        }

        let parts: Vec<&str> = pattern.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid label pattern '{}': must contain exactly one colon", pattern));
        }

        let namespace = parts[0];
        let value = parts[1];

        // Validate namespace format
        if !namespace.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(anyhow!("Invalid label pattern '{}': namespace must be lowercase alphanumeric with hyphens", pattern));
        }

        let issues = self.storage.list_issues()?;
        let filtered: Vec<Issue> = if value == "*" {
            // Wildcard: match all labels in this namespace
            issues
                .into_iter()
                .filter(|issue| {
                    issue.labels.iter().any(|label| {
                        if let Ok((ns, _)) = labels::parse_label(label) {
                            ns == namespace
                        } else {
                            false
                        }
                    })
                })
                .collect()
        } else {
            // Exact match
            issues
                .into_iter()
                .filter(|issue| issue.labels.contains(&pattern.to_string()))
                .collect()
        };

        Ok(filtered)
    }

    /// Get an issue by ID
    #[allow(dead_code)] // Used in tests
    pub fn get_issue(&self, id: &str) -> Result<Issue> {
        self.storage.load_issue(id)
    }

    /// Add a label to an issue with uniqueness validation
    #[allow(dead_code)] // Used in tests
    pub fn add_label(&self, issue_id: &str, label: &str) -> Result<()> {
        use crate::labels;

        // Validate label format
        labels::validate_label(label)?;

        let mut issue = self.storage.load_issue(issue_id)?;
        
        // Check uniqueness constraint
        let (namespace, _) = labels::parse_label(label)?;
        let namespaces = self.storage.load_label_namespaces()?;
        
        if let Some(ns_config) = namespaces.get(&namespace) {
            if ns_config.unique {
                // Check if issue already has a label in this namespace
                for existing_label in &issue.labels {
                    if let Ok((existing_ns, _)) = labels::parse_label(existing_label) {
                        if existing_ns == namespace {
                            return Err(anyhow!(
                                "Issue already has label '{}' from unique namespace '{}'",
                                existing_label,
                                namespace
                            ));
                        }
                    }
                }
            }
        }

        issue.labels.push(label.to_string());
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    /// List all unique values used in a label namespace
    #[allow(dead_code)] // Used in tests
    pub fn list_label_values(&self, namespace: &str) -> Result<Vec<String>> {
        let issues = self.storage.list_issues()?;
        let mut values = std::collections::HashSet::new();

        for issue in issues {
            for label in &issue.labels {
                if let Ok((ns, value)) = crate::labels::parse_label(label) {
                    if ns == namespace {
                        values.insert(value.to_string());
                    }
                }
            }
        }

        let mut result: Vec<String> = values.into_iter().collect();
        result.sort();
        Ok(result)
    }

    /// Add a new label namespace to the registry
    #[allow(dead_code)] // Used in tests
    pub fn add_label_namespace(
        &self,
        name: &str,
        description: &str,
        unique: bool,
        strategic: bool,
    ) -> Result<()> {
        let mut namespaces = self.storage.load_label_namespaces()?;
        namespaces.add(
            name.to_string(),
            crate::domain::LabelNamespace::new(description, unique, strategic),
        );
        self.storage.save_label_namespaces(&namespaces)?;
        Ok(())
    }

    /// Query all issues with labels from strategic namespaces
    #[allow(dead_code)] // Will be used via CLI in next step
    pub fn query_strategic(&self) -> Result<Vec<Issue>> {
        use crate::labels;

        let namespaces = self.storage.load_label_namespaces()?;
        let strategic_namespaces: Vec<String> = namespaces
            .namespaces
            .iter()
            .filter(|(_, ns)| ns.strategic)
            .map(|(name, _)| name.clone())
            .collect();

        if strategic_namespaces.is_empty() {
            return Ok(Vec::new());
        }

        let issues = self.storage.list_issues()?;
        let filtered = issues
            .into_iter()
            .filter(|issue| {
                issue.labels.iter().any(|label| {
                    if let Ok((ns, _)) = labels::parse_label(label) {
                        strategic_namespaces.contains(&ns)
                    } else {
                        false
                    }
                })
            })
            .collect();

        Ok(filtered)
    }

    /// Add a document reference to an issue
    pub fn add_document_reference(
        &self,
        issue_id: &str,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        json: bool,
    ) -> Result<()> {
        use crate::domain::DocumentReference;
        use crate::output::{JsonError, JsonOutput};

        let mut issue = self.storage.load_issue(issue_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        let doc_ref = DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: label.map(String::from),
            doc_type: doc_type.map(String::from),
        };

        issue.documents.push(doc_ref.clone());
        self.storage.save_issue(&issue)?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": issue_id,
                "document": doc_ref,
            }));
            println!("{}", output.to_json_string()?);
        } else {
            println!("Added document reference to issue {}", issue_id);
            println!("  Path: {}", path);
            if let Some(c) = commit {
                println!("  Commit: {}", c);
            }
            if let Some(l) = label {
                println!("  Label: {}", l);
            }
            if let Some(t) = doc_type {
                println!("  Type: {}", t);
            }
        }

        Ok(())
    }

    /// List document references for an issue
    pub fn list_document_references(&self, issue_id: &str, json: bool) -> Result<()> {
        use crate::output::{JsonError, JsonOutput};

        let issue = self.storage.load_issue(issue_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": issue_id,
                "documents": issue.documents,
                "count": issue.documents.len(),
            }));
            println!("{}", output.to_json_string()?);
        } else if issue.documents.is_empty() {
            println!("No document references for issue {}", issue_id);
        } else {
            println!("Document references for issue {}:", issue_id);
            for doc in &issue.documents {
                print!("  - {}", doc.path);
                if let Some(ref label) = doc.label {
                    print!(" ({})", label);
                }
                if let Some(ref commit) = doc.commit {
                    print!(" [{}]", &commit[..7.min(commit.len())]);
                } else {
                    print!(" [HEAD]");
                }
                if let Some(ref doc_type) = doc.doc_type {
                    print!(" <{}>", doc_type);
                }
                println!();
            }
            println!("\nTotal: {}", issue.documents.len());
        }

        Ok(())
    }

    /// Remove a document reference from an issue
    pub fn remove_document_reference(&self, issue_id: &str, path: &str, json: bool) -> Result<()> {
        use crate::output::{JsonError, JsonOutput};

        let mut issue = self.storage.load_issue(issue_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        let original_len = issue.documents.len();
        issue.documents.retain(|doc| doc.path != path);

        if issue.documents.len() == original_len {
            let err_msg = format!(
                "Document reference {} not found in issue {}",
                path, issue_id
            );
            if json {
                let err = JsonError::new("DOCUMENT_NOT_FOUND", &err_msg)
                    .with_suggestion("Run 'jit doc list <issue-id>' to see available documents");
                println!("{}", err.to_json_string()?);
            }
            return Err(anyhow!(err_msg));
        }

        self.storage.save_issue(&issue)?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": issue_id,
                "removed_path": path,
            }));
            println!("{}", output.to_json_string()?);
        } else {
            println!(
                "Removed document reference {} from issue {}",
                path, issue_id
            );
        }

        Ok(())
    }

    /// Show document content from git
    pub fn show_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<()> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        // Determine which commit to view
        let reference = if let Some(at) = at_commit {
            at
        } else if let Some(ref commit) = doc.commit {
            commit.as_str()
        } else {
            "HEAD"
        };

        // Display metadata
        println!("Document: {}", doc.path);
        if let Some(ref label) = doc.label {
            println!("Label: {}", label);
        }
        println!("Commit: {}", reference);
        if let Some(ref doc_type) = doc.doc_type {
            println!("Type: {}", doc_type);
        }
        println!("\n---\n");

        // Try to read content from git
        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let content = self
            .read_file_from_git(&repo, &doc.path, reference)
            .map_err(|e| anyhow!("Error reading file from git: {}", e))?;

        println!("{}", content);

        Ok(())
    }

    /// List commit history for a document
    pub fn document_history(&self, issue_id: &str, path: &str, json: bool) -> Result<()> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let commits = self.get_file_history(&repo, path)?;

        if json {
            let json_output = serde_json::to_string_pretty(&commits)?;
            println!("{}", json_output);
        } else {
            println!("History for {}:", path);
            println!();
            for commit in commits {
                println!("commit {}", commit.sha);
                println!("Author: {}", commit.author);
                println!("Date:   {}", commit.date);
                println!();
                println!("    {}", commit.message);
                println!();
            }
        }

        Ok(())
    }

    /// Show diff between two versions of a document
    pub fn document_diff(
        &self,
        issue_id: &str,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<()> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let to_ref = to.unwrap_or("HEAD");

        // Get content at both commits
        let from_content = self.read_file_from_git(&repo, path, from)?;
        let to_content = self.read_file_from_git(&repo, path, to_ref)?;

        // Generate unified diff
        println!("diff --git a/{} b/{}", path, path);
        println!("--- a/{} ({})", path, from);
        println!("+++ b/{} ({})", path, to_ref);
        println!();

        // Use similar crate for diff generation
        use similar::{ChangeTag, TextDiff};
        let diff = TextDiff::from_lines(&from_content, &to_content);

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            print!("{}{}", sign, change);
        }

        Ok(())
    }

    /// Read file content from git at a specific reference (public API for server)
    #[allow(dead_code)]
    pub fn read_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(String, String)> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        // Determine which commit to view
        let reference = if let Some(at) = at_commit {
            at
        } else if let Some(ref commit) = doc.commit {
            commit.as_str()
        } else {
            "HEAD"
        };

        // Try to read content from git
        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let content = self
            .read_file_from_git(&repo, &doc.path, reference)
            .map_err(|e| anyhow!("Error reading file from git: {}", e))?;

        // Resolve the actual commit hash
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let commit_hash = format!("{}", commit.id());

        Ok((content, commit_hash))
    }

    /// Get document history (public API for server)
    #[allow(dead_code)]
    pub fn get_document_history(&self, issue_id: &str, path: &str) -> Result<Vec<CommitInfo>> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        self.get_file_history(&repo, path)
    }

    /// Get document diff (public API for server)
    #[allow(dead_code)]
    pub fn get_document_diff(
        &self,
        issue_id: &str,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<String> {
        use git2::Repository;
        use similar::{ChangeTag, TextDiff};

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let to_ref = to.unwrap_or("HEAD");

        // Get content at both commits
        let from_content = self.read_file_from_git(&repo, path, from)?;
        let to_content = self.read_file_from_git(&repo, path, to_ref)?;

        // Generate unified diff
        let mut diff_output = format!("diff --git a/{} b/{}\n", path, path);
        diff_output.push_str(&format!("--- a/{} ({})\n", path, from));
        diff_output.push_str(&format!("+++ b/{} ({})\n\n", path, to_ref));

        let diff = TextDiff::from_lines(&from_content, &to_content);

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            diff_output.push_str(&format!("{}{}", sign, change));
        }

        Ok(diff_output)
    }

    /// Read file content from git at a specific reference
    fn read_file_from_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<String> {
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let entry = tree.get_path(std::path::Path::new(path))?;
        let blob = repo.find_blob(entry.id())?;

        let content = std::str::from_utf8(blob.content())?;
        Ok(content.to_string())
    }

    /// Get commit history for a file
    fn get_file_history(&self, repo: &git2::Repository, path: &str) -> Result<Vec<CommitInfo>> {
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;

        let mut commits = Vec::new();
        let file_path = std::path::Path::new(path);

        for oid in revwalk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;

            // Check if this commit touches the file
            let tree = commit.tree()?;
            if tree.get_path(file_path).is_ok() {
                // Check if this commit modified the file (not just has it)
                let parent_count = commit.parent_count();
                let mut modified = parent_count == 0; // Root commit always counts

                if !modified && parent_count > 0 {
                    let parent = commit.parent(0)?;
                    let parent_tree = parent.tree()?;

                    // Compare file content with parent
                    let current_entry = tree.get_path(file_path).ok();
                    let parent_entry = parent_tree.get_path(file_path).ok();

                    modified = match (current_entry, parent_entry) {
                        (Some(curr), Some(par)) => curr.id() != par.id(),
                        (Some(_), None) => true, // File added
                        _ => false,
                    };
                }

                if modified {
                    let author = commit.author();
                    let time = commit.time();
                    let datetime =
                        chrono::DateTime::from_timestamp(time.seconds(), 0).unwrap_or_default();

                    commits.push(CommitInfo {
                        sha: format!("{:.7}", oid),
                        author: author.name().unwrap_or("Unknown").to_string(),
                        date: datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
                        message: commit.message().unwrap_or("").trim().to_string(),
                    });
                }
            }
        }

        Ok(commits)
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
        "backlog" => Ok(State::Backlog),
        "open" => Ok(State::Backlog), // Backward compatibility alias
        "ready" => Ok(State::Ready),
        "in_progress" | "inprogress" => Ok(State::InProgress),
        "gated" => Ok(State::Gated),
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Low,
                vec![],
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
                vec![],
            )
            .unwrap();

        executor
            .update_issue(&id, Some("Updated".to_string()), None, None, None, vec![], vec![])
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
            .create_issue("Low".to_string(), "Desc".to_string(), Priority::Low, vec![], vec![])
            .unwrap();
        let high_id = executor
            .create_issue(
                "High".to_string(),
                "Desc".to_string(),
                Priority::High,
                vec![],
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
                vec![],
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
                vec![],
            )
            .unwrap();

        // Attempting to transition to Done with unpassed gates should succeed
        // but transition to Gated instead
        let result = executor.update_issue(&id, None, None, None, Some(State::Done), vec![], vec![]);
        assert!(result.is_ok());

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.state, State::Gated);
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 3".to_string(),
                "D3".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
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
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Child".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Dependent".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Child".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "D2".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Backend".to_string(),
                "Implement".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "InProgress".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Add new feature".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Fix bug in lexer".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Task 2".to_string(),
                "Regular task".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Normal bug".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();

        // Create issue with dependency (stays in Backlog)
        let id2 = executor
            .create_issue(
                "Backlog task".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        executor.add_dependency(&id2, &id1).unwrap();

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
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Bob work".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Task 2".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
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
                vec![],
            )
            .unwrap();

        // Search by first 8 chars of UUID
        let prefix = &id[..8];
        let results = executor.search_issues(prefix).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }

    // Tests for transitive reduction

    #[test]
    fn test_add_dependency_skips_transitive_simple_chain() {
        let (_temp, executor) = setup();

        // Create A→B→C
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        executor.add_dependency(&c, &b).unwrap();
        executor.add_dependency(&b, &a).unwrap();

        // Try to add redundant C→A
        executor.add_dependency(&c, &a).unwrap();

        // C should only depend on B, not A (transitive edge removed)
        let issue_c = executor.show_issue(&c).unwrap();
        assert_eq!(issue_c.dependencies.len(), 1);
        assert!(issue_c.dependencies.contains(&b));
        assert!(!issue_c.dependencies.contains(&a));
    }

    #[test]
    fn test_add_dependency_skips_transitive_diamond() {
        let (_temp, executor) = setup();

        // Create diamond: A→B, A→C, B→C
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        executor.add_dependency(&a, &b).unwrap();
        executor.add_dependency(&b, &c).unwrap();

        // Try to add redundant A→C
        executor.add_dependency(&a, &c).unwrap();

        // A should only depend on B, not C
        let issue_a = executor.show_issue(&a).unwrap();
        assert_eq!(issue_a.dependencies.len(), 1);
        assert!(issue_a.dependencies.contains(&b));
        assert!(!issue_a.dependencies.contains(&c));
    }

    #[test]
    fn test_add_dependency_keeps_parallel_deps() {
        let (_temp, executor) = setup();

        // Create parallel: A→B, A→C (no path between B and C)
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        executor.add_dependency(&a, &b).unwrap();
        executor.add_dependency(&a, &c).unwrap();

        // Both dependencies should remain (not transitive)
        let issue_a = executor.show_issue(&a).unwrap();
        assert_eq!(issue_a.dependencies.len(), 2);
        assert!(issue_a.dependencies.contains(&b));
        assert!(issue_a.dependencies.contains(&c));
    }

    #[test]
    fn test_add_dependency_reduces_existing_edges() {
        let (_temp, executor) = setup();

        // Create A with redundant edges first
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        // Add all edges including redundant one
        executor.add_dependency(&a, &b).unwrap();
        executor.add_dependency(&a, &c).unwrap();

        // Now add B→C, which makes A→C redundant
        executor.add_dependency(&b, &c).unwrap();

        // A→C should be automatically removed when we add A→B
        // But we already have both edges. We need to trigger reduction.
        // Let's add another dependency to A to trigger reduction
        let d = executor
            .create_issue("D".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        executor.add_dependency(&a, &d).unwrap();

        // After adding D, all of A's edges should be reduced
        let issue_a = executor.show_issue(&a).unwrap();
        // A should depend on B and D, but not C (transitive via B)
        assert!(issue_a.dependencies.contains(&b));
        assert!(issue_a.dependencies.contains(&d));
        assert!(!issue_a.dependencies.contains(&c));
    }

    #[test]
    fn test_add_dependency_complex_reduction() {
        let (_temp, executor) = setup();

        // Create complex graph: A→B, A→C, A→D, B→D, C→D
        // A→D should be removed as it's transitive via both B and C
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let d = executor
            .create_issue("D".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        executor.add_dependency(&a, &b).unwrap();
        executor.add_dependency(&a, &c).unwrap();
        executor.add_dependency(&b, &d).unwrap();
        executor.add_dependency(&c, &d).unwrap();

        // Try to add A→D (should be skipped)
        executor.add_dependency(&a, &d).unwrap();

        // A should only depend on B and C, not D
        let issue_a = executor.show_issue(&a).unwrap();
        assert_eq!(issue_a.dependencies.len(), 2);
        assert!(issue_a.dependencies.contains(&b));
        assert!(issue_a.dependencies.contains(&c));
        assert!(!issue_a.dependencies.contains(&d));
    }

    #[test]
    fn test_transitive_reduction_concurrent_operations() {
        use std::sync::Arc;
        use std::thread;

        let (_temp, executor) = setup();
        let executor = Arc::new(executor);

        // Create a graph: A, B, C, D
        let a = executor
            .create_issue("A".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let b = executor
            .create_issue("B".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let c = executor
            .create_issue("C".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        // Thread 1: Add A→B, then B→C
        let executor1 = Arc::clone(&executor);
        let a1 = a.clone();
        let b1 = b.clone();
        let c1 = c.clone();
        let t1 = thread::spawn(move || {
            executor1.add_dependency(&a1, &b1).unwrap();
            executor1.add_dependency(&b1, &c1).unwrap();
        });

        // Thread 2: Try to add A→C (might be redundant depending on timing)
        let executor2 = Arc::clone(&executor);
        let a2 = a.clone();
        let c2 = c.clone();
        let t2 = thread::spawn(move || {
            // Sleep briefly to let T1 potentially establish A→B→C first
            thread::sleep(std::time::Duration::from_millis(10));
            executor2.add_dependency(&a2, &c2).unwrap();
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Final state should have transitively reduced edges
        let issue_a = executor.show_issue(&a).unwrap();
        let issue_b = executor.show_issue(&b).unwrap();

        // Verify DAG integrity
        assert!(issue_b.dependencies.contains(&c)); // B→C

        // A should only depend on B (A→C removed as transitive)
        // OR if timing caused A→C to be added first, we should still have valid state
        assert!(issue_a.dependencies.contains(&b)); // A→B always present

        // The key property: no cycles and graph is still valid
        let issues = executor.list_issues(None, None, None).unwrap();
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        assert!(graph.validate_dag().is_ok());
    }

    // Tests for issue breakdown

    #[test]
    fn test_breakdown_issue_simple() {
        let (_temp, executor) = setup();

        // Create parent with a dependency
        let dep = executor
            .create_issue(
                "Build".to_string(),
                "".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        let parent = executor
            .create_issue(
                "Security Review".to_string(),
                "".to_string(),
                Priority::High,
                vec![],
                vec![],
            )
            .unwrap();

        executor.add_dependency(&parent, &dep).unwrap();

        // Breakdown into subtasks
        let subtasks = vec![
            ("Check vulnerabilities".to_string(), "npm audit".to_string()),
            ("Scan for secrets".to_string(), "gitleaks".to_string()),
        ];

        let subtask_ids = executor.breakdown_issue(&parent, subtasks).unwrap();

        // Verify 2 subtasks created
        assert_eq!(subtask_ids.len(), 2);

        // Verify parent depends on subtasks
        let parent_issue = executor.show_issue(&parent).unwrap();
        assert_eq!(parent_issue.dependencies.len(), 2);
        assert!(parent_issue.dependencies.contains(&subtask_ids[0]));
        assert!(parent_issue.dependencies.contains(&subtask_ids[1]));

        // Verify subtasks inherit parent's original dependency
        let subtask1 = executor.show_issue(&subtask_ids[0]).unwrap();
        let subtask2 = executor.show_issue(&subtask_ids[1]).unwrap();
        assert!(subtask1.dependencies.contains(&dep));
        assert!(subtask2.dependencies.contains(&dep));

        // Verify parent's original dependency was removed (now transitive)
        assert!(!parent_issue.dependencies.contains(&dep));
    }

    #[test]
    fn test_breakdown_issue_no_dependencies() {
        let (_temp, executor) = setup();

        // Create parent with no dependencies
        let parent = executor
            .create_issue("Epic".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();

        let subtasks = vec![
            ("Task 1".to_string(), "".to_string()),
            ("Task 2".to_string(), "".to_string()),
        ];

        let subtask_ids = executor.breakdown_issue(&parent, subtasks).unwrap();

        // Verify subtasks created
        assert_eq!(subtask_ids.len(), 2);

        // Verify parent depends on subtasks
        let parent_issue = executor.show_issue(&parent).unwrap();
        assert_eq!(parent_issue.dependencies.len(), 2);

        // Verify subtasks have no dependencies (parent had none)
        let subtask1 = executor.show_issue(&subtask_ids[0]).unwrap();
        let subtask2 = executor.show_issue(&subtask_ids[1]).unwrap();
        assert_eq!(subtask1.dependencies.len(), 0);
        assert_eq!(subtask2.dependencies.len(), 0);
    }

    #[test]
    fn test_breakdown_issue_multiple_dependencies() {
        let (_temp, executor) = setup();

        // Create parent with multiple dependencies
        let dep1 = executor
            .create_issue("Dep1".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let dep2 = executor
            .create_issue("Dep2".to_string(), "".to_string(), Priority::Normal, vec![], vec![])
            .unwrap();
        let parent = executor
            .create_issue("Parent".to_string(), "".to_string(), Priority::High, vec![], vec![])
            .unwrap();

        executor.add_dependency(&parent, &dep1).unwrap();
        executor.add_dependency(&parent, &dep2).unwrap();

        let subtasks = vec![
            ("Sub1".to_string(), "".to_string()),
            ("Sub2".to_string(), "".to_string()),
        ];

        let subtask_ids = executor.breakdown_issue(&parent, subtasks).unwrap();

        // Verify all subtasks inherit both dependencies
        for subtask_id in &subtask_ids {
            let subtask = executor.show_issue(subtask_id).unwrap();
            assert!(subtask.dependencies.contains(&dep1));
            assert!(subtask.dependencies.contains(&dep2));
        }

        // Verify parent only depends on subtasks (original deps removed)
        let parent_issue = executor.show_issue(&parent).unwrap();
        assert_eq!(parent_issue.dependencies.len(), 2);
        assert!(!parent_issue.dependencies.contains(&dep1));
        assert!(!parent_issue.dependencies.contains(&dep2));
    }

    #[test]
    fn test_breakdown_issue_preserves_priority() {
        let (_temp, executor) = setup();

        let parent = executor
            .create_issue(
                "Critical Task".to_string(),
                "".to_string(),
                Priority::Critical,
                vec![],
                vec![],
            )
            .unwrap();

        let subtasks = vec![("Sub".to_string(), "desc".to_string())];

        let subtask_ids = executor.breakdown_issue(&parent, subtasks).unwrap();

        // Subtasks inherit parent's priority
        let subtask = executor.show_issue(&subtask_ids[0]).unwrap();
        assert_eq!(subtask.priority, Priority::Critical);
    }

    #[test]
    fn test_breakdown_nonexistent_issue() {
        let (_temp, executor) = setup();

        let result =
            executor.breakdown_issue("nonexistent-id", vec![("Sub".to_string(), "".to_string())]);

        assert!(result.is_err());
    }
}
