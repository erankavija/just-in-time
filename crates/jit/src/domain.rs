//! Core domain types for the issue tracker.
//!
//! This module defines the fundamental data structures used throughout the system:
//! issues, gates, events, and their associated states and priorities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Issue lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Created but not actionable yet (blocked by dependencies or gates)
    Backlog,
    /// All dependencies done and gates passed, ready for work
    Ready,
    /// Currently being worked on
    InProgress,
    /// Work complete, awaiting quality gate approval
    Gated,
    /// Completed
    Done,
    /// No longer relevant
    Archived,
}

/// Issue priority level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Low priority
    Low,
    /// Normal priority (default)
    Normal,
    /// High priority
    High,
    /// Critical priority
    Critical,
}

/// Quality gate status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Gate not yet evaluated
    Pending,
    /// Gate passed successfully
    Passed,
    /// Gate failed
    Failed,
}

/// State of a quality gate for a specific issue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    /// Current status of the gate
    pub status: GateStatus,
    /// Who updated the gate status (e.g., "human:alice", "ci:github-actions")
    pub updated_by: Option<String>,
    /// When the gate was last updated
    pub updated_at: DateTime<Utc>,
}

/// An issue representing a unit of work
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// Unique identifier (UUID)
    pub id: String,
    /// Short summary of the issue
    pub title: String,
    /// Detailed description and acceptance criteria
    pub description: String,
    /// Current lifecycle state
    pub state: State,
    /// Priority level
    pub priority: Priority,
    /// Assigned agent or person (format: "type:identifier")
    pub assignee: Option<String>,
    /// IDs of issues that must be done first
    pub dependencies: Vec<String>,
    /// Gate keys that must pass before ready/done
    pub gates_required: Vec<String>,
    /// Current status of each required gate
    pub gates_status: HashMap<String, GateState>,
    /// Flexible key-value storage for agent-specific data
    pub context: HashMap<String, String>,
    /// References to design documents, notes, and artifacts
    pub documents: Vec<DocumentReference>,
    /// Labels for categorization and hierarchy (format: "namespace:value")
    pub labels: Vec<String>,
}

impl Issue {
    /// Create a new issue with default values
    pub fn new(title: String, description: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            description,
            state: State::Backlog,
            priority: Priority::Normal,
            assignee: None,
            dependencies: Vec::new(),
            gates_required: Vec::new(),
            gates_status: HashMap::new(),
            context: HashMap::new(),
            documents: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// Check if this issue is blocked by incomplete dependencies
    ///
    /// Returns true if any dependency is not done.
    /// Note: Gates do not block work from starting, only from completing.
    pub fn is_blocked(&self, resolved_issues: &HashMap<String, &Issue>) -> bool {
        // Check if any dependency is not done
        self.dependencies
            .iter()
            .any(|dep_id| !matches!(resolved_issues.get(dep_id), Some(issue) if issue.state == State::Done))
    }

    /// Check if this issue has unpassed gates
    ///
    /// Returns true if any required gate hasn't passed.
    /// Used to determine if issue can transition to Done.
    pub fn has_unpassed_gates(&self) -> bool {
        self.gates_required
            .iter()
            .any(|gate_key| !matches!(self.gates_status.get(gate_key), Some(gate_state) if gate_state.status == GateStatus::Passed))
    }

    /// Check if this issue should auto-transition to Ready state
    /// A Backlog issue transitions to Ready when it becomes unblocked
    pub fn should_auto_transition_to_ready(
        &self,
        resolved_issues: &HashMap<String, &Issue>,
    ) -> bool {
        self.state == State::Backlog && !self.is_blocked(resolved_issues)
    }

    /// Check if this issue should auto-transition to Done state
    /// A Gated issue transitions to Done when all required gates pass
    pub fn should_auto_transition_to_done(&self) -> bool {
        self.state == State::Gated && !self.has_unpassed_gates()
    }
}

/// Implement GraphNode for Issue to enable dependency graph operations
impl crate::graph::GraphNode for Issue {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
}

/// A reference to a document (design doc, notes, artifact) in the repository
///
/// Documents can reference files at HEAD or specific git commits for
/// version-aware knowledge management.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentReference {
    /// Path relative to repository root (e.g., "docs/api-design.md")
    pub path: String,
    /// Optional git commit hash (None = HEAD, Some("a1b2c3d") = specific commit)
    pub commit: Option<String>,
    /// Human-readable label (e.g., "API Design Document")
    pub label: Option<String>,
    /// Document type hint (e.g., "design", "implementation", "notes")
    pub doc_type: Option<String>,
}

impl DocumentReference {
    /// Create a new document reference pointing to HEAD
    #[allow(dead_code)]
    pub fn new(path: String) -> Self {
        Self {
            path,
            commit: None,
            label: None,
            doc_type: None,
        }
    }

    /// Create a reference to a document at a specific commit
    #[allow(dead_code)]
    pub fn at_commit(path: String, commit: String) -> Self {
        Self {
            path,
            commit: Some(commit),
            label: None,
            doc_type: None,
        }
    }

    /// Builder method to add a label
    #[allow(dead_code)]
    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    /// Builder method to add a document type
    #[allow(dead_code)]
    pub fn with_type(mut self, doc_type: String) -> Self {
        self.doc_type = Some(doc_type);
        self
    }
}

/// A quality gate definition in the registry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// Unique identifier for this gate type
    pub key: String,
    /// Human-readable name
    pub title: String,
    /// Explanation of what this gate checks
    pub description: String,
    /// Whether automation can pass this gate
    pub auto: bool,
    /// Example of how to integrate with this gate
    pub example_integration: Option<String>,
}

/// System event types for audit log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A new issue was created
    IssueCreated {
        /// Event ID
        id: String,
        /// Issue that was created
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Issue title
        title: String,
        /// Issue priority
        priority: Priority,
    },
    /// An issue was claimed by an agent
    IssueClaimed {
        /// Event ID
        id: String,
        /// Issue that was claimed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Who claimed it
        assignee: String,
    },
    /// Issue state transitioned
    IssueStateChanged {
        /// Event ID
        id: String,
        /// Issue that changed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Previous state
        from: State,
        /// New state
        to: State,
    },
    /// A quality gate passed
    GatePassed {
        /// Event ID
        id: String,
        /// Issue with the gate
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that passed
        gate_key: String,
        /// Who marked it as passed
        updated_by: Option<String>,
    },
    /// A quality gate failed
    GateFailed {
        /// Event ID
        id: String,
        /// Issue with the gate
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that failed
        gate_key: String,
        /// Who marked it as failed
        updated_by: Option<String>,
    },
    /// Issue was completed
    IssueCompleted {
        /// Event ID
        id: String,
        /// Issue that completed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
    },
    /// Issue was released from assignee
    IssueReleased {
        /// Event ID
        id: String,
        /// Issue that was released
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Previous assignee
        assignee: String,
        /// Reason for release
        reason: String,
    },
}

impl Event {
    /// Create an issue created event
    pub fn new_issue_created(issue: &Issue) -> Self {
        Event::IssueCreated {
            id: Uuid::new_v4().to_string(),
            issue_id: issue.id.clone(),
            timestamp: Utc::now(),
            title: issue.title.clone(),
            priority: issue.priority,
        }
    }

    /// Create an issue claimed event
    pub fn new_issue_claimed(issue_id: String, assignee: String) -> Self {
        Event::IssueClaimed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            assignee,
        }
    }

    /// Create an issue state changed event
    pub fn new_issue_state_changed(issue_id: String, from: State, to: State) -> Self {
        Event::IssueStateChanged {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            from,
            to,
        }
    }

    /// Create a gate passed event
    pub fn new_gate_passed(issue_id: String, gate_key: String, updated_by: Option<String>) -> Self {
        Event::GatePassed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    /// Create a gate failed event
    pub fn new_gate_failed(issue_id: String, gate_key: String, updated_by: Option<String>) -> Self {
        Event::GateFailed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    /// Create an issue completed event
    pub fn new_issue_completed(issue_id: String) -> Self {
        Event::IssueCompleted {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
        }
    }

    /// Create an issue released event
    pub fn new_issue_released(issue_id: String, assignee: String, reason: String) -> Self {
        Event::IssueReleased {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            assignee,
            reason,
        }
    }

    /// Get the issue ID associated with this event
    pub fn get_issue_id(&self) -> &str {
        match self {
            Event::IssueCreated { issue_id, .. } => issue_id,
            Event::IssueClaimed { issue_id, .. } => issue_id,
            Event::IssueStateChanged { issue_id, .. } => issue_id,
            Event::GatePassed { issue_id, .. } => issue_id,
            Event::GateFailed { issue_id, .. } => issue_id,
            Event::IssueCompleted { issue_id, .. } => issue_id,
            Event::IssueReleased { issue_id, .. } => issue_id,
        }
    }

    /// Get the event type as a string
    pub fn get_type(&self) -> &str {
        match self {
            Event::IssueCreated { .. } => "issue_created",
            Event::IssueClaimed { .. } => "issue_claimed",
            Event::IssueStateChanged { .. } => "issue_state_changed",
            Event::GatePassed { .. } => "gate_passed",
            Event::GateFailed { .. } => "gate_failed",
            Event::IssueCompleted { .. } => "issue_completed",
            Event::IssueReleased { .. } => "issue_released",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_issue_has_correct_defaults() {
        let issue = Issue::new("Test Issue".to_string(), "Description".to_string());

        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.description, "Description");
        assert_eq!(issue.state, State::Backlog);
        assert_eq!(issue.priority, Priority::Normal);
        assert_eq!(issue.assignee, None);
        assert!(issue.dependencies.is_empty());
        assert!(issue.gates_required.is_empty());
        assert!(issue.gates_status.is_empty());
        assert!(issue.context.is_empty());
        assert!(!issue.id.is_empty());
    }

    #[test]
    fn test_issue_not_blocked_with_no_dependencies_or_gates() {
        let issue = Issue::new("Test".to_string(), "Desc".to_string());
        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_incomplete_dependency() {
        let mut issue = Issue::new("Dependent".to_string(), "Desc".to_string());
        let dependency = Issue::new("Dependency".to_string(), "Desc".to_string());

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_done() {
        let mut issue = Issue::new("Dependent".to_string(), "Desc".to_string());
        let mut dependency = Issue::new("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Done;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_by_unpassed_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_by_pending_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_by_failed_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("human:reviewer".to_string()),
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_when_gate_passed() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".to_string()),
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
        assert!(!issue.has_unpassed_gates());
    }

    #[test]
    fn test_document_reference_new() {
        let doc = DocumentReference::new("docs/design.md".to_string());
        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.commit, None);
        assert_eq!(doc.label, None);
        assert_eq!(doc.doc_type, None);
    }

    #[test]
    fn test_document_reference_at_commit() {
        let doc = DocumentReference::at_commit("docs/design.md".to_string(), "a1b2c3d".to_string());
        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.commit, Some("a1b2c3d".to_string()));
    }

    #[test]
    fn test_document_reference_builder() {
        let doc = DocumentReference::new("docs/design.md".to_string())
            .with_label("API Design".to_string())
            .with_type("design".to_string());

        assert_eq!(doc.label, Some("API Design".to_string()));
        assert_eq!(doc.doc_type, Some("design".to_string()));
    }

    #[test]
    fn test_document_reference_serialization() {
        let doc = DocumentReference::new("docs/design.md".to_string())
            .with_label("Design Doc".to_string());

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
    }

    #[test]
    fn test_issue_with_documents() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        assert_eq!(issue.documents.len(), 0);

        issue
            .documents
            .push(DocumentReference::new("docs/design.md".to_string()));
        assert_eq!(issue.documents.len(), 1);
        assert_eq!(issue.documents[0].path, "docs/design.md");
    }

    #[test]
    fn test_issue_serialization_with_documents() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        issue.documents.push(
            DocumentReference::at_commit("docs/design.md".to_string(), "abc123".to_string())
                .with_label("Design".to_string()),
        );

        let json = serde_json::to_string(&issue).unwrap();
        let deserialized: Issue = serde_json::from_str(&json).unwrap();

        assert_eq!(issue.documents.len(), deserialized.documents.len());
        assert_eq!(issue.documents[0], deserialized.documents[0]);
    }

    // State model tests for Backlog and Gated states

    #[test]
    fn test_new_issue_starts_in_backlog() {
        let issue = Issue::new("Test".to_string(), "Description".to_string());
        assert_eq!(issue.state, State::Backlog);
    }

    #[test]
    fn test_backlog_issue_should_auto_transition_to_ready_when_unblocked() {
        let issue = Issue::new("Test".to_string(), "Description".to_string());
        let resolved = HashMap::new();

        assert_eq!(issue.state, State::Backlog);
        assert!(issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_backlog_issue_should_not_transition_to_ready_when_blocked() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        let dependency = Issue::new("Dependency".to_string(), "Desc".to_string());
        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert_eq!(issue.state, State::Backlog);
        assert!(!issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_gated_issue_should_auto_transition_to_done_when_gates_pass() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".to_string()),
                updated_at: Utc::now(),
            },
        );

        assert!(issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_pending() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_failed() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("ci:tests".to_string()),
                updated_at: Utc::now(),
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_in_progress_issue_should_not_auto_transition() {
        let mut issue = Issue::new("Test".to_string(), "Description".to_string());
        issue.state = State::InProgress;

        let resolved = HashMap::new();
        assert!(!issue.should_auto_transition_to_ready(&resolved));
        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_state_serialization_backlog() {
        let state = State::Backlog;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"backlog\"");

        let deserialized: State = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn test_state_serialization_gated() {
        let state = State::Gated;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"gated\"");

        let deserialized: State = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn test_new_issue_has_empty_labels() {
        let issue = Issue::new("Test".to_string(), "Description".to_string());
        assert!(issue.labels.is_empty());
    }

    #[test]
    fn test_issue_serialization_with_labels() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.labels.push("milestone:v1.0".to_string());
        issue.labels.push("epic:auth".to_string());
        issue.labels.push("type:task".to_string());

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"labels\""));
        assert!(json.contains("milestone:v1.0"));

        let deserialized: Issue = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.labels.len(), 3);
        assert!(deserialized.labels.contains(&"milestone:v1.0".to_string()));
        assert!(deserialized.labels.contains(&"epic:auth".to_string()));
        assert!(deserialized.labels.contains(&"type:task".to_string()));
    }

    #[test]
    fn test_issue_labels_can_be_modified() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        assert!(issue.labels.is_empty());

        issue.labels.push("component:backend".to_string());
        assert_eq!(issue.labels.len(), 1);

        issue.labels.push("priority:high".to_string());
        assert_eq!(issue.labels.len(), 2);

        issue.labels.retain(|l| l != "component:backend");
        assert_eq!(issue.labels.len(), 1);
        assert_eq!(issue.labels[0], "priority:high");
    }
}

/// Label namespace configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelNamespace {
    /// Human-readable description
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue
    pub unique: bool,
    /// Whether this namespace is used for strategic planning (milestones, epics)
    pub strategic: bool,
}

impl LabelNamespace {
    /// Create a new namespace with given properties
    pub fn new(description: impl Into<String>, unique: bool, strategic: bool) -> Self {
        Self {
            description: description.into(),
            unique,
            strategic,
        }
    }
}

/// Container for all label namespaces
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelNamespaces {
    /// Schema version for future migrations
    pub schema_version: u32,
    /// Map of namespace name to configuration
    pub namespaces: HashMap<String, LabelNamespace>,
}

impl LabelNamespaces {
    /// Create empty namespace registry
    pub fn new() -> Self {
        Self {
            schema_version: 1,
            namespaces: HashMap::new(),
        }
    }

    /// Create registry with standard namespaces
    pub fn with_defaults() -> Self {
        let mut namespaces = HashMap::new();

        namespaces.insert(
            "milestone".to_string(),
            LabelNamespace::new("Release milestones and version targets", false, true),
        );

        namespaces.insert(
            "epic".to_string(),
            LabelNamespace::new("Large features or initiatives", false, true),
        );

        namespaces.insert(
            "component".to_string(),
            LabelNamespace::new("Technical component or subsystem", false, false),
        );

        namespaces.insert(
            "type".to_string(),
            LabelNamespace::new("Issue type (bug, feature, task, etc.)", true, false),
        );

        namespaces.insert(
            "team".to_string(),
            LabelNamespace::new("Owning team", true, false),
        );

        Self {
            schema_version: 1,
            namespaces,
        }
    }

    /// Add or update a namespace
    pub fn add(&mut self, name: String, namespace: LabelNamespace) {
        self.namespaces.insert(name, namespace);
    }

    /// Get a namespace by name
    pub fn get(&self, name: &str) -> Option<&LabelNamespace> {
        self.namespaces.get(name)
    }

    /// Check if a namespace exists
    #[allow(dead_code)] // May be used in future
    pub fn contains(&self, name: &str) -> bool {
        self.namespaces.contains_key(name)
    }
}

impl Default for LabelNamespaces {
    fn default() -> Self {
        Self::with_defaults()
    }
}
