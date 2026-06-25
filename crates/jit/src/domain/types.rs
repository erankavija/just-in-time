//! Core domain types for the issue tracker.
//!
//! This module defines the fundamental data structures used throughout the system:
//! issues, gates, events, and their associated states and priorities.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

/// Length of short issue ID (git-style short hash)
pub const SHORT_ID_LENGTH: usize = 8;

/// Issue lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
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
    /// Completed successfully
    Done,
    /// Won't implement (bypasses gates)
    Rejected,
    /// No longer relevant
    Archived,
}

impl State {
    /// Check if this state is terminal (Done or Rejected)
    ///
    /// Terminal states represent closure - either successful completion (Done)
    /// or decision not to implement (Rejected). Both unblock dependent issues.
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Done | State::Rejected)
    }

    /// Check if this state is closed (Done or Rejected)
    ///
    /// This is an alias for is_terminal() for query semantics.
    pub fn is_closed(self) -> bool {
        self.is_terminal()
    }
}

impl FromStr for State {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "backlog" => Ok(State::Backlog),
            "open" => Ok(State::Backlog), // Backward compatibility alias
            "ready" => Ok(State::Ready),
            "in_progress" | "inprogress" => Ok(State::InProgress),
            "gated" => Ok(State::Gated),
            "done" => Ok(State::Done),
            "rejected" => Ok(State::Rejected),
            "archived" => Ok(State::Archived),
            _ => Err(anyhow!("Invalid state: {}", s)),
        }
    }
}

/// Issue priority level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

impl FromStr for Priority {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Priority::Low),
            "normal" => Ok(Priority::Normal),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            _ => Err(anyhow!("Invalid priority: {}", s)),
        }
    }
}

/// The content format of an issue body, selecting which [`ContentParser`] the
/// validation projection uses to extract `sections`.
///
/// Serialized lowercase (`"markdown"`, `"html"`, `"xml"`) so issue JSON stays
/// human-readable. When absent on an issue the repo default
/// (`[validation].content_format`) applies; the final fallback is `Markdown`,
/// which is always compiled in. HTML/XML are only usable when the `html`/`xml`
/// cargo features are built (see
/// [`content_parser_for`](crate::document::content_parser_for)).
///
/// # Examples
///
/// ```
/// use jit::domain::ContentFormat;
/// use std::str::FromStr;
///
/// assert_eq!(ContentFormat::from_str("html").unwrap(), ContentFormat::Html);
/// assert_eq!(serde_json::to_string(&ContentFormat::Xml).unwrap(), "\"xml\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    /// Markdown body (the default; always available).
    #[default]
    Markdown,
    /// HTML body (requires the `html` cargo feature to parse).
    Html,
    /// XML body (requires the `xml` cargo feature to parse).
    Xml,
}

impl FromStr for ContentFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(ContentFormat::Markdown),
            "html" => Ok(ContentFormat::Html),
            "xml" => Ok(ContentFormat::Xml),
            _ => Err(anyhow!(
                "Invalid content format: '{}' (expected markdown, html, or xml)",
                s
            )),
        }
    }
}

/// Quality gate status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Gate not yet evaluated
    Pending,
    /// Gate passed successfully
    Passed,
    /// Gate failed
    Failed,
}

/// A parsed assignee in the documented `{kind}:{identifier}` form.
///
/// Issues and gate states identify the responsible agent or person with a
/// `kind:identifier` string (e.g. `agent:copilot`, `human:alice`,
/// `ci:github-actions`). This newtype is the single place that format is parsed
/// and validated: every value stored in [`Issue::assignee`] or
/// [`GateState::updated_by`] is built through [`Assignee::from_str`] (directly or
/// via deserialization), so a raw, unvalidated string can never reach storage.
///
/// The fields are private; construct an `Assignee` by parsing (`str::parse` /
/// [`FromStr`]) and read the parts via [`Assignee::kind`] /
/// [`Assignee::identifier`]. It serializes transparently as the
/// `kind:identifier` string (through [`Display`] / [`FromStr`]), so on-disk
/// issue JSON is unchanged.
///
/// # Examples
///
/// ```
/// use jit::domain::Assignee;
/// use std::str::FromStr;
///
/// let a = Assignee::from_str("agent:copilot").unwrap();
/// assert_eq!(a.kind(), "agent");
/// assert_eq!(a.identifier(), "copilot");
/// assert_eq!(a.to_string(), "agent:copilot");
///
/// // Only the first colon splits, so identifiers may contain colons.
/// let a: Assignee = "ci:job:42".parse().unwrap();
/// assert_eq!(a.identifier(), "job:42");
///
/// // Malformed values are rejected.
/// assert!(Assignee::from_str("nocolon").is_err());
/// assert!(Assignee::from_str(":missing-kind").is_err());
/// assert!(Assignee::from_str("missing-id:").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Assignee {
    kind: String,
    identifier: String,
}

impl Assignee {
    /// The kind segment before the first colon (e.g. `agent` in `agent:copilot`).
    ///
    /// # Examples
    ///
    /// ```
    /// let a: jit::domain::Assignee = "agent:copilot".parse().unwrap();
    /// assert_eq!(a.kind(), "agent");
    /// ```
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The identifier segment after the first colon (e.g. `copilot`).
    ///
    /// # Examples
    ///
    /// ```
    /// let a: jit::domain::Assignee = "agent:copilot".parse().unwrap();
    /// assert_eq!(a.identifier(), "copilot");
    /// ```
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

/// Error returned when a string cannot be parsed as an [`Assignee`].
///
/// Distinguishes the failure modes so callers (e.g. agent-identity validation)
/// can map them to their own user-facing messages while reusing the one parse
/// path.
///
/// # Examples
///
/// ```
/// use jit::domain::{Assignee, AssigneeParseError};
/// use std::str::FromStr;
///
/// assert_eq!(Assignee::from_str(""), Err(AssigneeParseError::Empty));
/// assert!(matches!(
///     Assignee::from_str("nocolon"),
///     Err(AssigneeParseError::MissingSeparator(_))
/// ));
/// assert!(matches!(
///     Assignee::from_str(":id"),
///     Err(AssigneeParseError::EmptyKind(_))
/// ));
/// assert!(matches!(
///     Assignee::from_str("kind:"),
///     Err(AssigneeParseError::EmptyIdentifier(_))
/// ));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssigneeParseError {
    /// The input was empty.
    #[error("Assignee cannot be empty")]
    Empty,
    /// The input had no `:` separating kind from identifier.
    #[error(
        "Assignee must be in format 'type:identifier' (e.g. 'agent:copilot', 'human:alice'); got '{0}'"
    )]
    MissingSeparator(String),
    /// The kind segment (before the colon) was empty.
    #[error(
        "Assignee kind cannot be empty; expected 'type:identifier' (e.g. 'agent:copilot'); got '{0}'"
    )]
    EmptyKind(String),
    /// The identifier segment (after the colon) was empty.
    #[error(
        "Assignee identifier cannot be empty; expected 'type:identifier' (e.g. 'agent:copilot'); got '{0}'"
    )]
    EmptyIdentifier(String),
}

impl FromStr for Assignee {
    type Err = AssigneeParseError;

    /// Parse a `kind:identifier` assignee, splitting on the first colon only.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Assignee;
    /// use std::str::FromStr;
    ///
    /// let a = Assignee::from_str("human:alice").unwrap();
    /// assert_eq!((a.kind(), a.identifier()), ("human", "alice"));
    /// assert!(Assignee::from_str("alice").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(AssigneeParseError::Empty);
        }
        let (kind, identifier) = s
            .split_once(':')
            .ok_or_else(|| AssigneeParseError::MissingSeparator(s.to_string()))?;
        if kind.is_empty() {
            return Err(AssigneeParseError::EmptyKind(s.to_string()));
        }
        if identifier.is_empty() {
            return Err(AssigneeParseError::EmptyIdentifier(s.to_string()));
        }
        Ok(Assignee {
            kind: kind.to_string(),
            identifier: identifier.to_string(),
        })
    }
}

impl std::fmt::Display for Assignee {
    /// Render as the canonical `kind:identifier` string.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Assignee;
    ///
    /// let a: Assignee = "ci:github-actions".parse().unwrap();
    /// assert_eq!(a.to_string(), "ci:github-actions");
    /// ```
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.identifier)
    }
}

/// Compare an [`Assignee`] against a raw `kind:identifier` string without
/// allocating, so filter predicates can match a parsed assignee to user input.
impl PartialEq<str> for Assignee {
    fn eq(&self, other: &str) -> bool {
        match other.split_once(':') {
            Some((kind, identifier)) => self.kind == kind && self.identifier == identifier,
            None => false,
        }
    }
}

impl Serialize for Assignee {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Assignee {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// State of a quality gate for a specific issue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateState {
    /// Current status of the gate
    pub status: GateStatus,
    /// Who updated the gate status (e.g., "human:alice", "ci:github-actions")
    #[schemars(with = "Option<String>")]
    pub updated_by: Option<Assignee>,
    /// When the gate was last updated
    pub updated_at: DateTime<Utc>,
}

/// An issue representing a unit of work
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    #[schemars(with = "Option<String>")]
    pub assignee: Option<Assignee>,
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
    /// Content format of the `description` body, selecting the parser used to
    /// extract `sections` during validation. Absent (`None`) means inherit the
    /// repo default (`[validation].content_format`), with a Markdown fallback.
    /// Existing issue files without this field deserialize as `None` and are NOT
    /// rewritten (skipped on serialize when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_format: Option<ContentFormat>,
    /// When the issue was created (serialized as an RFC 3339 timestamp)
    pub created_at: DateTime<Utc>,
    /// When the issue was last updated (serialized as an RFC 3339 timestamp)
    pub updated_at: DateTime<Utc>,
}

impl Issue {
    /// Create a new issue with default values
    pub fn new(title: String, description: String) -> Self {
        let now = Utc::now();
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
            content_format: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Get short ID (first 8 characters of UUID)
    ///
    /// Returns a git-style short hash for human-readable output.
    /// Minimum length is 8 characters for reasonable collision resistance.
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }

    /// Create a new issue with labels
    #[cfg(test)]
    pub fn new_with_labels(title: String, description: String, labels: Vec<String>) -> Self {
        let now = Utc::now();
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
            labels,
            content_format: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check if this issue is blocked by incomplete dependencies
    ///
    /// Returns true if any dependency is not in a terminal state (Done or Rejected).
    /// Note: Gates do not block work from starting, only from completing.
    pub fn is_blocked(&self, resolved_issues: &HashMap<String, &Issue>) -> bool {
        // Check if any dependency is not in a terminal state
        self.dependencies
            .iter()
            .any(|dep_id| !matches!(resolved_issues.get(dep_id), Some(issue) if issue.state.is_terminal()))
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

    /// Get list of unpassed gates
    ///
    /// Returns a vector of gate keys that have not yet passed.
    pub fn get_unpassed_gates(&self) -> Vec<String> {
        self.gates_required
            .iter()
            .filter(|gate_key| !matches!(self.gates_status.get(*gate_key), Some(gate_state) if gate_state.status == GateStatus::Passed))
            .cloned()
            .collect()
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

/// Minimal issue representation for efficient list queries
///
/// Returns only essential fields to reduce token usage. Use `jit issue show`
/// for full details.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MinimalIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    /// Assigned agent or person (optional for context)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Labels for categorization (optional for context)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

// Custom serialization to add computed short_id
impl Serialize for MinimalIssue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("MinimalIssue", 7)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("short_id", &self.short_id())?;
        state.serialize_field("title", &self.title)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("priority", &self.priority)?;
        if let Some(ref assignee) = self.assignee {
            state.serialize_field("assignee", assignee)?;
        } else {
            state.serialize_field("assignee", &None::<String>)?;
        }
        if !self.labels.is_empty() {
            state.serialize_field("labels", &self.labels)?;
        }
        state.end()
    }
}

impl From<&Issue> for MinimalIssue {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.as_ref().map(Assignee::to_string),
            labels: issue.labels.clone(),
        }
    }
}

impl MinimalIssue {
    /// Get short ID (first 8 characters of UUID)
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }

    /// Get state symbol for human-readable output
    /// - ✓ for terminal states (done/rejected)
    /// - ○ for active states
    pub fn state_symbol(&self) -> &str {
        if self.state.is_terminal() {
            "✓"
        } else {
            "○"
        }
    }
}

/// Minimal blocked issue for queries - includes blocking reasons
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MinimalBlockedIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

// Custom serialization to add computed short_id (mirrors MinimalIssue)
impl Serialize for MinimalBlockedIssue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("MinimalBlockedIssue", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("short_id", &self.short_id())?;
        state.serialize_field("title", &self.title)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("priority", &self.priority)?;
        if let Some(ref assignee) = self.assignee {
            state.serialize_field("assignee", assignee)?;
        } else {
            state.serialize_field("assignee", &None::<String>)?;
        }
        if !self.labels.is_empty() {
            state.serialize_field("labels", &self.labels)?;
        }
        state.serialize_field("blocked_reasons", &self.blocked_reasons)?;
        state.end()
    }
}

impl MinimalBlockedIssue {
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }
}

impl From<(&Issue, Vec<String>)> for MinimalBlockedIssue {
    fn from((issue, blocked_reasons): (&Issue, Vec<String>)) -> Self {
        Self {
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.as_ref().map(Assignee::to_string),
            labels: issue.labels.clone(),
            blocked_reasons,
        }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DocumentReference {
    /// Path relative to repository root (e.g., "docs/api-design.md")
    pub path: String,
    /// Optional git commit hash (None = HEAD, Some("a1b2c3d") = specific commit)
    pub commit: Option<String>,
    /// Human-readable label (e.g., "API Design Document")
    pub label: Option<String>,
    /// Document type hint (e.g., "design", "implementation", "notes")
    pub doc_type: Option<String>,
    /// Document format (e.g., "markdown", "asciidoc", "rst")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Assets referenced by this document (images, diagrams, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<crate::document::Asset>,
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
            format: None,
            assets: Vec::new(),
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
            format: None,
            assets: Vec::new(),
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

    /// Builder method to add format
    #[allow(dead_code)]
    pub fn with_format(mut self, format: String) -> Self {
        self.format = Some(format);
        self
    }

    /// Builder method to set assets
    #[allow(dead_code)]
    pub fn with_assets(mut self, assets: Vec<crate::document::Asset>) -> Self {
        self.assets = assets;
        self
    }
}

/// A quality gate definition in the registry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// Schema version for future evolution
    #[serde(default = "default_gate_version")]
    pub version: u32,
    /// Unique identifier for this gate type
    pub key: String,
    /// Human-readable name
    pub title: String,
    /// Explanation of what this gate checks
    pub description: String,
    /// Gate execution stage
    pub stage: GateStage,
    /// Gate mode (manual or automated)
    pub mode: GateMode,
    /// Checker configuration for automated gates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<GateChecker>,
    /// Execution priority (lower runs first, default 100)
    #[serde(default = "default_gate_priority")]
    pub priority: u32,
    /// Reserved for future extensions
    #[serde(default)]
    pub reserved: HashMap<String, serde_json::Value>,
    /// Deprecated: kept for backwards compatibility
    #[serde(default)]
    pub auto: bool,
    /// Deprecated: kept for backwards compatibility
    pub example_integration: Option<String>,
}

fn default_gate_version() -> u32 {
    1
}

fn default_gate_priority() -> u32 {
    100
}

/// Gate execution stage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStage {
    /// Runs before work starts (ready → in_progress)
    Precheck,
    /// Runs after work completes (in_progress → gated)
    Postcheck,
}

/// Gate execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Requires manual pass/fail
    Manual,
    /// Can be automatically checked
    Auto,
}

/// Gate checker configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GateChecker {
    /// Execute a shell command
    Exec {
        /// Command to execute
        command: String,
        /// Timeout in seconds
        timeout_seconds: u64,
        /// Optional working directory (relative to repo root)
        working_dir: Option<String>,
        /// Environment variables
        #[serde(default)]
        env: HashMap<String, String>,
        /// Whether to pass structured context to the checker process
        #[serde(default)]
        pass_context: bool,
        /// Inline prompt/instructions for the checker
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// Path to a prompt file (relative to repo root), read at check time
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_file: Option<String>,
    },
}

/// Structured context passed to gate checker processes
///
/// When `pass_context` is enabled on a gate checker, this context is serialized
/// to a JSON file and made available via the `JIT_CONTEXT_FILE` env var.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateContext {
    /// Schema version for forward compatibility
    pub schema_version: u32,
    /// Resolved prompt (from inline `prompt` or `prompt_file`)
    pub prompt: Option<String>,
    /// Full issue data as JSON value
    pub issue: serde_json::Value,
    /// Gate definition as JSON value
    pub gate: serde_json::Value,
    /// Chronologically-sorted run history for this gate+issue pair
    pub run_history: Vec<GateRunResult>,
}

/// Result of a gate execution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRunResult {
    /// Schema version for future evolution
    pub schema_version: u32,
    /// Unique run identifier
    pub run_id: String,
    /// Gate key that was executed
    pub gate_key: String,
    /// Stage at which gate was executed
    pub stage: GateStage,
    /// Issue ID
    pub issue_id: String,
    /// Git commit (if available)
    pub commit: Option<String>,
    /// Git branch (if available)
    pub branch: Option<String>,
    /// Result status
    pub status: GateRunStatus,
    /// When execution started
    pub started_at: DateTime<Utc>,
    /// When execution completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Exit code (for command execution)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Command that was executed
    pub command: String,
    /// Who triggered this execution
    pub by: Option<String>,
    /// Optional message
    pub message: Option<String>,
}

/// Gate run status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateRunStatus {
    /// Check succeeded
    Passed,
    /// Check failed (expected failure, e.g., tests failed)
    Failed,
    /// Unexpected error (timeout, command not found, crash)
    Error,
    /// Not yet run (for manual gates)
    Pending,
    /// Not applicable (future: for conditional gates)
    Skipped,
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
        /// Who claimed it (format: "type:identifier")
        assignee: Assignee,
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
        /// Who marked it as passed (format: "type:identifier")
        updated_by: Option<Assignee>,
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
        /// Who marked it as failed (format: "type:identifier")
        updated_by: Option<Assignee>,
    },
    /// A quality gate was added to an issue
    GateAdded {
        /// Event ID
        id: String,
        /// Issue to which gate was added
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that was added
        gate_key: String,
    },
    /// A quality gate was removed from an issue
    GateRemoved {
        /// Event ID
        id: String,
        /// Issue from which gate was removed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that was removed
        gate_key: String,
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
    /// Issue was permanently deleted
    IssueDeleted {
        /// Event ID
        id: String,
        /// Issue that was deleted
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
        /// Previous assignee (format: "type:identifier")
        assignee: Assignee,
        /// Reason for release
        reason: String,
    },
    /// Document was archived
    DocumentArchived {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Source path
        source: String,
        /// Destination path
        destination: String,
        /// Archive category
        category: String,
        /// Number of issues updated
        issues_updated: usize,
    },
    /// Issue was updated (labels, priority, assignee, etc.)
    IssueUpdated {
        /// Event ID
        id: String,
        /// Issue that was updated
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Who updated it (e.g., "bulk-update", "human:alice", "agent:copilot")
        updated_by: String,
        /// Fields that changed
        fields: Vec<String>,
    },
    /// Redundant dependencies were removed by transitive reduction during validate --fix
    DependencyReduced {
        /// Event ID
        id: String,
        /// Issue whose dependencies were reduced
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Number of dependencies before reduction
        old_count: usize,
        /// Number of dependencies after reduction
        new_count: usize,
        /// IDs of the removed (redundant) dependencies
        removed_deps: Vec<String>,
    },
    /// A blocking validation rule was bypassed via `--force` on a write.
    ///
    /// Emitted once per enforce-rule whose `error` finding was overridden by
    /// `--force` during an issue create/update/bulk write. This is the
    /// audit-sensitive override (DR §7.6): ordinary rule rejections and
    /// read-only `jit validate` runs are NOT logged, only the deliberate bypass.
    LocalRuleBypassed {
        /// Event ID
        id: String,
        /// Issue whose write bypassed the rule
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Name of the enforce rule that was bypassed
        rule: String,
    },
    /// A state transition was blocked by an enforcing graph rule (CC-2).
    ///
    /// Emitted once per blocking rule, BEFORE the transition error is returned —
    /// the attempted transition is the auditable act. Distinct from a gate or
    /// dependency block: this records that a `Scope::Graph` rule with `enforce =
    /// true` produced an `error` finding attributed to the issue in its target
    /// state, so the transition into `target` was refused.
    TransitionBlocked {
        /// Event ID
        id: String,
        /// Issue whose transition was blocked
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// State the issue was attempting to enter
        target: State,
        /// Name of the enforcing graph rule that blocked the transition
        rule: String,
    },
    /// An enforcing graph rule was bypassed via `--force` at a state transition
    /// (CC-2).
    ///
    /// The transition-path counterpart of [`Event::LocalRuleBypassed`]: emitted
    /// once per enforce graph rule whose `error` finding was overridden by
    /// `--force` during a `--state` transition. Unlike `LocalRuleBypassed` (which
    /// the write path emits AFTER its save commits), this event is appended inside
    /// `enforce_transition_graph_rules`, i.e. just BEFORE the caller persists the
    /// issue save — the enforcement and its audit entry are produced together,
    /// then the caller commits the transition. Kept distinct from the write-path
    /// bypass so the audit log can tell apart a forced write from a forced
    /// transition.
    GraphRuleBypassed {
        /// Event ID
        id: String,
        /// Issue whose transition bypassed the rule
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// State the issue transitioned into
        target: State,
        /// Name of the enforce graph rule that was bypassed
        rule: String,
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

    /// Create an issue claimed event.
    ///
    /// Takes a typed [`Assignee`] so a malformed actor can never be logged; the
    /// caller parses (and thereby validates) the actor before constructing the
    /// event.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Assignee, Event};
    ///
    /// let actor: Assignee = "agent:copilot".parse().unwrap();
    /// let event = Event::new_issue_claimed("issue-123".to_string(), actor);
    /// assert_eq!(event.get_type(), "issue_claimed");
    /// ```
    pub fn new_issue_claimed(issue_id: String, assignee: Assignee) -> Self {
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

    /// Create a gate passed event.
    ///
    /// `updated_by` is a typed [`Assignee`] (the actor who passed the gate), so
    /// no malformed actor can be logged.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Assignee, Event};
    ///
    /// let by: Assignee = "ci:runner".parse().unwrap();
    /// let event = Event::new_gate_passed("issue-123".to_string(), "tests".to_string(), Some(by));
    /// assert_eq!(event.get_type(), "gate_passed");
    /// ```
    pub fn new_gate_passed(
        issue_id: String,
        gate_key: String,
        updated_by: Option<Assignee>,
    ) -> Self {
        Event::GatePassed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    /// Create a gate failed event.
    ///
    /// `updated_by` is a typed [`Assignee`] (the actor who failed the gate), so
    /// no malformed actor can be logged.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Assignee, Event};
    ///
    /// let by: Assignee = "ci:runner".parse().unwrap();
    /// let event = Event::new_gate_failed("issue-123".to_string(), "tests".to_string(), Some(by));
    /// assert_eq!(event.get_type(), "gate_failed");
    /// ```
    pub fn new_gate_failed(
        issue_id: String,
        gate_key: String,
        updated_by: Option<Assignee>,
    ) -> Self {
        Event::GateFailed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    /// Create a gate added event
    pub fn new_gate_added(issue_id: String, gate_key: String) -> Self {
        Event::GateAdded {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
        }
    }

    /// Create a gate removed event
    pub fn new_gate_removed(issue_id: String, gate_key: String) -> Self {
        Event::GateRemoved {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
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

    /// Create an issue released event.
    ///
    /// `assignee` is the typed [`Assignee`] being released; callers emit this
    /// event only when a prior assignee existed, so the actor is always a valid
    /// `kind:identifier`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Assignee, Event};
    ///
    /// let prev: Assignee = "agent:copilot".parse().unwrap();
    /// let event = Event::new_issue_released("issue-123".to_string(), prev, "timeout".to_string());
    /// assert_eq!(event.get_type(), "issue_released");
    /// ```
    pub fn new_issue_released(issue_id: String, assignee: Assignee, reason: String) -> Self {
        Event::IssueReleased {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            assignee,
            reason,
        }
    }

    /// Create a document archived event
    pub fn new_document_archived(
        source: String,
        destination: String,
        category: String,
        issues_updated: usize,
    ) -> Self {
        Event::DocumentArchived {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            source,
            destination,
            category,
            issues_updated,
        }
    }

    /// Create a dependency reduced event
    pub fn new_dependency_reduced(
        issue_id: String,
        old_count: usize,
        new_count: usize,
        removed_deps: Vec<String>,
    ) -> Self {
        Event::DependencyReduced {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            old_count,
            new_count,
            removed_deps,
        }
    }

    /// Create an issue updated event
    pub fn new_issue_updated(issue_id: String, updated_by: String, fields: Vec<String>) -> Self {
        Event::IssueUpdated {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            updated_by,
            fields,
        }
    }

    /// Create an issue deleted event.
    ///
    /// Records that an issue was permanently deleted, preserving an audit trail
    /// of the removal in the event log (the issue file itself is gone).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Event;
    ///
    /// let event = Event::new_issue_deleted("issue-123".to_string());
    /// assert_eq!(event.get_issue_id(), "issue-123");
    /// assert_eq!(event.get_type(), "issue_deleted");
    /// ```
    pub fn new_issue_deleted(issue_id: String) -> Self {
        Event::IssueDeleted {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
        }
    }

    /// Create a local-rule-bypassed event.
    ///
    /// Records that a `--force` write deliberately bypassed an `enforce` rule
    /// whose `error` finding would otherwise have blocked the write (DR §7.6).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Event;
    ///
    /// let event = Event::new_local_rule_bypassed(
    ///     "issue-123".to_string(),
    ///     "epic-needs-requirements".to_string(),
    /// );
    /// assert_eq!(event.get_issue_id(), "issue-123");
    /// assert_eq!(event.get_type(), "local_rule_bypassed");
    /// ```
    pub fn new_local_rule_bypassed(issue_id: String, rule: String) -> Self {
        Event::LocalRuleBypassed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            rule,
        }
    }

    /// Create a transition-blocked event.
    ///
    /// Records that an enforcing graph rule blocked the issue's transition into
    /// `target` (CC-2). Appended before the blocking error is returned, one per
    /// blocking rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Event, State};
    ///
    /// let event = Event::new_transition_blocked(
    ///     "issue-123".to_string(),
    ///     State::Done,
    ///     "sdd-hard-criteria-covered".to_string(),
    /// );
    /// assert_eq!(event.get_issue_id(), "issue-123");
    /// assert_eq!(event.get_type(), "transition_blocked");
    /// ```
    pub fn new_transition_blocked(issue_id: String, target: State, rule: String) -> Self {
        Event::TransitionBlocked {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            target,
            rule,
        }
    }

    /// Create a graph-rule-bypassed event.
    ///
    /// Records that a `--force` transition deliberately bypassed an enforcing
    /// graph rule whose `error` finding would otherwise have blocked the
    /// transition into `target` (CC-2).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Event, State};
    ///
    /// let event = Event::new_graph_rule_bypassed(
    ///     "issue-123".to_string(),
    ///     State::Done,
    ///     "sdd-hard-criteria-covered".to_string(),
    /// );
    /// assert_eq!(event.get_issue_id(), "issue-123");
    /// assert_eq!(event.get_type(), "graph_rule_bypassed");
    /// ```
    pub fn new_graph_rule_bypassed(issue_id: String, target: State, rule: String) -> Self {
        Event::GraphRuleBypassed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            target,
            rule,
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
            Event::GateAdded { issue_id, .. } => issue_id,
            Event::GateRemoved { issue_id, .. } => issue_id,
            Event::IssueCompleted { issue_id, .. } => issue_id,
            Event::IssueDeleted { issue_id, .. } => issue_id,
            Event::IssueReleased { issue_id, .. } => issue_id,
            Event::IssueUpdated { issue_id, .. } => issue_id,
            Event::DocumentArchived { .. } => "", // No associated issue
            Event::DependencyReduced { issue_id, .. } => issue_id,
            Event::LocalRuleBypassed { issue_id, .. } => issue_id,
            Event::TransitionBlocked { issue_id, .. } => issue_id,
            Event::GraphRuleBypassed { issue_id, .. } => issue_id,
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
            Event::GateAdded { .. } => "gate_added",
            Event::GateRemoved { .. } => "gate_removed",
            Event::IssueCompleted { .. } => "issue_completed",
            Event::IssueDeleted { .. } => "issue_deleted",
            Event::IssueReleased { .. } => "issue_released",
            Event::IssueUpdated { .. } => "issue_updated",
            Event::DocumentArchived { .. } => "document_archived",
            Event::DependencyReduced { .. } => "dependency_reduced",
            Event::LocalRuleBypassed { .. } => "local_rule_bypassed",
            Event::TransitionBlocked { .. } => "transition_blocked",
            Event::GraphRuleBypassed { .. } => "graph_rule_bypassed",
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
                updated_by: Some("human:reviewer".parse().unwrap()),
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
                updated_by: Some("human:reviewer".parse().unwrap()),
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
                updated_by: Some("human:reviewer".parse().unwrap()),
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
                updated_by: Some("ci:tests".parse().unwrap()),
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

    // Tests for Rejected state
    #[test]
    fn test_rejected_state_serialization() {
        let json = serde_json::to_string(&State::Rejected).unwrap();
        assert_eq!(json, "\"rejected\"");
    }

    #[test]
    fn test_rejected_state_deserialization() {
        let state: State = serde_json::from_str("\"rejected\"").unwrap();
        assert_eq!(state, State::Rejected);
    }

    #[test]
    fn test_is_terminal_returns_true_for_done() {
        assert!(State::Done.is_terminal());
    }

    #[test]
    fn test_is_terminal_returns_true_for_rejected() {
        assert!(State::Rejected.is_terminal());
    }

    #[test]
    fn test_is_terminal_returns_false_for_non_terminal_states() {
        assert!(!State::Backlog.is_terminal());
        assert!(!State::Ready.is_terminal());
        assert!(!State::InProgress.is_terminal());
        assert!(!State::Gated.is_terminal());
        assert!(!State::Archived.is_terminal());
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_rejected() {
        let mut issue = Issue::new("Dependent".to_string(), "Desc".to_string());
        let mut dependency = Issue::new("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Rejected;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        // Rejected dependencies should unblock, like Done
        assert!(!issue.is_blocked(&resolved));
    }

    // Tests for extended DocumentReference schema with format and assets

    #[test]
    fn test_document_reference_with_format_and_assets() {
        use crate::document::Asset;
        use std::path::PathBuf;

        let doc = DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design Doc".to_string()),
            doc_type: Some("design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
        };

        assert_eq!(doc.format, Some("markdown".to_string()));
        assert_eq!(doc.assets.len(), 1);
        assert_eq!(doc.assets[0].original_path, "./logo.png");
    }

    #[test]
    fn test_document_reference_serialization_with_new_fields() {
        use crate::document::Asset;
        use std::path::PathBuf;

        let doc = DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design Doc".to_string()),
            doc_type: Some("design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
        assert_eq!(deserialized.format, Some("markdown".to_string()));
        assert_eq!(deserialized.assets.len(), 1);
    }

    #[test]
    fn test_document_reference_backward_compatibility() {
        // Old JSON without format and assets fields
        let old_json = r#"{
            "path": "docs/design.md",
            "commit": null,
            "label": "Design Doc",
            "doc_type": "design"
        }"#;

        let doc: DocumentReference = serde_json::from_str(old_json).unwrap();

        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.label, Some("Design Doc".to_string()));
        assert_eq!(doc.doc_type, Some("design".to_string()));
        // New fields should have default values
        assert_eq!(doc.format, None);
        assert_eq!(doc.assets.len(), 0);
    }

    #[test]
    fn test_document_reference_forward_compatibility() {
        use crate::document::Asset;
        use std::path::PathBuf;

        // New JSON with format and assets
        let doc = DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design".to_string()),
            doc_type: Some("design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![
                Asset {
                    original_path: "./arch.png".to_string(),
                    resolved_path: Some(PathBuf::from("docs/arch.png")),
                    asset_type: crate::document::AssetType::Local,
                    mime_type: Some("image/png".to_string()),
                    content_hash: Some("sha256:def456".to_string()),
                    is_shared: false,
                },
                Asset {
                    original_path: "https://example.com/logo.svg".to_string(),
                    resolved_path: None,
                    asset_type: crate::document::AssetType::External,
                    mime_type: Some("image/svg+xml".to_string()),
                    content_hash: None,
                    is_shared: false,
                },
            ],
        };

        let json = serde_json::to_string_pretty(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.format, Some("markdown".to_string()));
        assert_eq!(deserialized.assets.len(), 2);
        assert_eq!(
            deserialized.assets[0].mime_type,
            Some("image/png".to_string())
        );
        assert_eq!(
            deserialized.assets[1].original_path,
            "https://example.com/logo.svg"
        );
    }

    #[test]
    fn test_document_reference_empty_assets() {
        let doc = DocumentReference {
            path: "docs/notes.md".to_string(),
            commit: None,
            label: None,
            doc_type: Some("notes".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![],
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
        assert_eq!(deserialized.assets.len(), 0);
    }

    // FromStr trait tests
    mod fromstr_tests {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_state_from_str_valid_lowercase() {
            assert_eq!(State::from_str("backlog").unwrap(), State::Backlog);
            assert_eq!(State::from_str("ready").unwrap(), State::Ready);
            assert_eq!(State::from_str("in_progress").unwrap(), State::InProgress);
            assert_eq!(State::from_str("gated").unwrap(), State::Gated);
            assert_eq!(State::from_str("done").unwrap(), State::Done);
            assert_eq!(State::from_str("rejected").unwrap(), State::Rejected);
            assert_eq!(State::from_str("archived").unwrap(), State::Archived);
        }

        #[test]
        fn test_state_from_str_valid_uppercase() {
            assert_eq!(State::from_str("BACKLOG").unwrap(), State::Backlog);
            assert_eq!(State::from_str("READY").unwrap(), State::Ready);
            assert_eq!(State::from_str("IN_PROGRESS").unwrap(), State::InProgress);
        }

        #[test]
        fn test_state_from_str_valid_mixedcase() {
            assert_eq!(State::from_str("Backlog").unwrap(), State::Backlog);
            assert_eq!(State::from_str("Ready").unwrap(), State::Ready);
        }

        #[test]
        fn test_state_from_str_aliases() {
            // Backward compatibility alias
            assert_eq!(State::from_str("open").unwrap(), State::Backlog);
            // Alternative in_progress format
            assert_eq!(State::from_str("inprogress").unwrap(), State::InProgress);
        }

        #[test]
        fn test_state_from_str_invalid() {
            assert!(State::from_str("invalid").is_err());
            assert!(State::from_str("").is_err());
            assert!(State::from_str("pending").is_err());
        }

        #[test]
        fn test_priority_from_str_valid_lowercase() {
            assert_eq!(Priority::from_str("low").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("normal").unwrap(), Priority::Normal);
            assert_eq!(Priority::from_str("high").unwrap(), Priority::High);
            assert_eq!(Priority::from_str("critical").unwrap(), Priority::Critical);
        }

        #[test]
        fn test_priority_from_str_valid_uppercase() {
            assert_eq!(Priority::from_str("LOW").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("NORMAL").unwrap(), Priority::Normal);
            assert_eq!(Priority::from_str("HIGH").unwrap(), Priority::High);
            assert_eq!(Priority::from_str("CRITICAL").unwrap(), Priority::Critical);
        }

        #[test]
        fn test_priority_from_str_valid_mixedcase() {
            assert_eq!(Priority::from_str("Low").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("Normal").unwrap(), Priority::Normal);
        }

        #[test]
        fn test_priority_from_str_invalid() {
            assert!(Priority::from_str("invalid").is_err());
            assert!(Priority::from_str("").is_err());
            assert!(Priority::from_str("medium").is_err());
        }

        #[test]
        fn test_state_parse_method() {
            // Test using str::parse() method
            let state: State = "ready".parse().unwrap();
            assert_eq!(state, State::Ready);
        }

        #[test]
        fn test_priority_parse_method() {
            // Test using str::parse() method
            let priority: Priority = "high".parse().unwrap();
            assert_eq!(priority, Priority::High);
        }
    }

    mod assignee_tests {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_assignee_from_str_accepts_valid() {
            let a = Assignee::from_str("agent:copilot").unwrap();
            assert_eq!(a.kind(), "agent");
            assert_eq!(a.identifier(), "copilot");
        }

        #[test]
        fn test_assignee_from_str_splits_on_first_colon_only() {
            let a = Assignee::from_str("ci:job:42").unwrap();
            assert_eq!(a.kind(), "ci");
            assert_eq!(a.identifier(), "job:42");
        }

        #[test]
        fn test_assignee_from_str_rejects_empty() {
            assert_eq!(Assignee::from_str(""), Err(AssigneeParseError::Empty));
        }

        #[test]
        fn test_assignee_from_str_rejects_missing_separator() {
            assert!(matches!(
                Assignee::from_str("nocolon"),
                Err(AssigneeParseError::MissingSeparator(_))
            ));
        }

        #[test]
        fn test_assignee_from_str_rejects_empty_parts() {
            assert!(matches!(
                Assignee::from_str(":identifier"),
                Err(AssigneeParseError::EmptyKind(_))
            ));
            assert!(matches!(
                Assignee::from_str("kind:"),
                Err(AssigneeParseError::EmptyIdentifier(_))
            ));
        }

        #[test]
        fn test_assignee_display_round_trips_through_from_str() {
            for raw in [
                "agent:copilot",
                "human:alice",
                "ci:github-actions",
                "ci:job:42",
            ] {
                let parsed = Assignee::from_str(raw).unwrap();
                assert_eq!(parsed.to_string(), raw);
                assert_eq!(Assignee::from_str(&parsed.to_string()).unwrap(), parsed);
            }
        }

        #[test]
        fn test_assignee_partial_eq_str() {
            let a = Assignee::from_str("agent:copilot").unwrap();
            assert!(a == *"agent:copilot");
            assert!(a != *"agent:other");
            assert!(a != *"nocolon");
        }

        #[test]
        fn test_assignee_serializes_as_plain_string() {
            let a = Assignee::from_str("human:alice").unwrap();
            assert_eq!(serde_json::to_string(&a).unwrap(), "\"human:alice\"");
        }

        #[test]
        fn test_assignee_deserializes_from_string_round_trip() {
            let a: Assignee = serde_json::from_str("\"ci:github-actions\"").unwrap();
            assert_eq!(a, Assignee::from_str("ci:github-actions").unwrap());
        }

        #[test]
        fn test_assignee_deserialize_rejects_malformed_string() {
            assert!(serde_json::from_str::<Assignee>("\"nocolon\"").is_err());
        }

        #[test]
        fn test_issue_assignee_serializes_unchanged() {
            let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
            issue.assignee = Some(Assignee::from_str("agent:copilot").unwrap());

            let json = serde_json::to_string(&issue).unwrap();
            assert!(json.contains("\"assignee\":\"agent:copilot\""));

            let deserialized: Issue = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.assignee, issue.assignee);
        }
    }

    #[test]
    fn test_issue_timestamps_serialize_as_rfc3339() {
        let issue = Issue::new("Test".to_string(), "Desc".to_string());

        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&issue).unwrap()).unwrap();
        let created = value["created_at"].as_str().unwrap();

        // chrono's default serde emits RFC 3339, parseable back to the same instant.
        let reparsed = DateTime::parse_from_rfc3339(created).unwrap();
        assert_eq!(reparsed.with_timezone(&Utc), issue.created_at);
    }

    #[test]
    fn test_issue_loads_legacy_offset_timestamp() {
        // Issue files predating the DateTime migration store `+00:00` offsets
        // rather than chrono's `Z`; both are RFC 3339 and must still load.
        let json = r#"{
            "id": "11111111-2222-3333-4444-555555555555",
            "title": "Legacy",
            "description": "Body",
            "state": "backlog",
            "priority": "normal",
            "assignee": "agent:copilot",
            "dependencies": [],
            "gates_required": [],
            "gates_status": {},
            "context": {},
            "documents": [],
            "labels": [],
            "created_at": "2026-06-22T21:59:04.464226946+00:00",
            "updated_at": "2026-06-22T21:59:04.464226946+00:00"
        }"#;

        let issue: Issue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.assignee.unwrap().to_string(), "agent:copilot");
        assert_eq!(issue.created_at.timezone(), Utc);
    }

    mod event_assignee_tests {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_issue_claimed_assignee_serde_round_trips_as_string() {
            let actor = Assignee::from_str("agent:copilot").unwrap();
            let event = Event::new_issue_claimed("issue-1".to_string(), actor);

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"assignee\":\"agent:copilot\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_issue_released_assignee_serde_round_trips_as_string() {
            let prev = Assignee::from_str("copilot:session-1").unwrap();
            let event =
                Event::new_issue_released("issue-1".to_string(), prev, "timeout".to_string());

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"assignee\":\"copilot:session-1\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_gate_event_updated_by_serde_round_trips_as_string() {
            let by = Assignee::from_str("ci:runner").unwrap();
            let event =
                Event::new_gate_passed("issue-1".to_string(), "tests".to_string(), Some(by));

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"updated_by\":\"ci:runner\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_event_actor_rejected_before_construction() {
            // The constructors only accept an already-parsed `Assignee`, so a
            // malformed actor is rejected at parse time and can never reach an
            // event. (`"bulk-update"` is the kind of non-assignee actor that the
            // string-typed `IssueUpdated.updated_by` still carries.)
            assert!(Assignee::from_str("bulk-update").is_err());
            assert!("nocolon".parse::<Assignee>().is_err());
        }

        #[test]
        fn test_event_deserialize_rejects_malformed_assignee() {
            let json = r#"{"type":"issue_claimed","id":"e1","issue_id":"i1","timestamp":"2026-01-01T00:00:00Z","assignee":"nocolon"}"#;
            assert!(serde_json::from_str::<Event>(json).is_err());
        }
    }
}

/// Label namespace configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelNamespace {
    /// Human-readable description
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue
    pub unique: bool,
}

impl LabelNamespace {
    /// Create a new namespace with given properties
    pub fn new(description: impl Into<String>, unique: bool) -> Self {
        Self {
            description: description.into(),
            unique,
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
    /// Type hierarchy configuration (optional, defaults to standard hierarchy)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hierarchy: Option<HashMap<String, u8>>,
    /// Label associations for membership namespaces (type_name -> namespace)
    /// e.g., "epic" -> "epic", "release" -> "milestone"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_associations: Option<HashMap<String, String>>,
    /// List of type names that are considered strategic (optional)
    /// e.g., ["milestone", "epic"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategic_types: Option<Vec<String>>,
}

impl LabelNamespaces {
    /// Create empty namespace registry
    pub fn new() -> Self {
        Self {
            schema_version: 1,
            namespaces: HashMap::new(),
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Ensure namespaces exist for all membership labels in label_associations.
    /// Dynamically creates namespace entries for custom type names.
    pub fn sync_membership_namespaces(&mut self) {
        if let Some(ref associations) = self.label_associations {
            for (type_name, namespace) in associations {
                // Only create namespace if it doesn't already exist
                if !self.namespaces.contains_key(namespace) {
                    self.namespaces.insert(
                        namespace.clone(),
                        LabelNamespace::new(
                            format!("{} organizational grouping", type_name),
                            false,
                        ),
                    );
                }
            }
        }
    }

    /// Create registry with standard namespaces and default type hierarchy
    pub fn with_defaults() -> Self {
        let mut namespaces = HashMap::new();

        // Core system namespaces (not derived from hierarchy)
        namespaces.insert(
            "component".to_string(),
            LabelNamespace::new("Technical component or subsystem", false),
        );

        namespaces.insert(
            "type".to_string(),
            LabelNamespace::new("Issue type (bug, feature, task, etc.)", true),
        );

        namespaces.insert("team".to_string(), LabelNamespace::new("Owning team", true));

        // Default type hierarchy
        let mut type_hierarchy = HashMap::new();
        type_hierarchy.insert("milestone".to_string(), 1);
        type_hierarchy.insert("epic".to_string(), 2);
        type_hierarchy.insert("story".to_string(), 3);
        type_hierarchy.insert("task".to_string(), 4);

        // Default label associations
        let mut label_associations = HashMap::new();
        label_associations.insert("milestone".to_string(), "milestone".to_string());
        label_associations.insert("epic".to_string(), "epic".to_string());
        label_associations.insert("story".to_string(), "story".to_string());

        // Default strategic types (levels 1-2: milestone, epic)
        let strategic_types = vec!["milestone".to_string(), "epic".to_string()];

        let mut config = Self {
            schema_version: 2,
            namespaces,
            type_hierarchy: Some(type_hierarchy),
            label_associations: Some(label_associations),
            strategic_types: Some(strategic_types),
        };

        // Dynamically create membership namespaces from label_associations
        config.sync_membership_namespaces();

        config
    }

    /// Get the type hierarchy, or default if not specified
    pub fn get_type_hierarchy(&self) -> HashMap<String, u8> {
        if let Some(ref hierarchy) = self.type_hierarchy {
            hierarchy.clone()
        } else {
            // Fallback to default hierarchy
            let mut hierarchy = HashMap::new();
            hierarchy.insert("milestone".to_string(), 1);
            hierarchy.insert("epic".to_string(), 2);
            hierarchy.insert("story".to_string(), 3);
            hierarchy.insert("task".to_string(), 4);
            hierarchy
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
