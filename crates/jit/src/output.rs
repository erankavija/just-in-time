//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::fmt::Display;
use std::io::{self, Write};

use crate::domain::{
    GateRunResult, GateRunStatus, GateStage, GateState, Issue, MinimalBlockedIssue, MinimalIssue,
    Priority, State,
};
use crate::errors::{
    gate_status_name, short_id, state_name, TransitionBlockedError, TransitionBlocker,
};

// ============================================================================
// Output Context for Quiet Mode
// ============================================================================

/// Context for controlling output verbosity
pub struct OutputContext {
    quiet: bool,
    json: bool,
}

impl OutputContext {
    /// Create a new output context
    pub fn new(quiet: bool, json: bool) -> Self {
        Self { quiet, json }
    }

    /// Print essential output (always shown unless --json)
    pub fn print_data(&self, msg: impl Display) -> io::Result<()> {
        if !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print informational message (suppressed by --quiet or --json)
    pub fn print_info(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print success message (suppressed by --quiet or --json)
    pub fn print_success(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print warning (suppressed by --quiet or --json)
    pub fn print_warning(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe_stderr(&format!("Warning: {}", msg))
        } else {
            Ok(())
        }
    }

    /// Print error (always shown to stderr)
    pub fn print_error(&self, msg: impl Display) -> io::Result<()> {
        writeln_safe_stderr(&format!("Error: {}", msg))
    }

    /// Check if quiet mode is enabled
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Check if JSON mode is enabled
    pub fn is_json(&self) -> bool {
        self.json
    }
}

/// Safe println that handles broken pipes gracefully
fn writeln_safe(msg: &str) -> io::Result<()> {
    match writeln!(io::stdout(), "{}", msg) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            // Silently exit on broken pipe (expected when piping to head, etc.)
            std::process::exit(0);
        }
        Err(e) => Err(e),
    }
}

/// Safe eprintln that handles broken pipes gracefully
fn writeln_safe_stderr(msg: &str) -> io::Result<()> {
    match writeln!(io::stderr(), "{}", msg) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            // Silently exit on broken pipe
            std::process::exit(0);
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// JSON Output Types
// ============================================================================

/// Wrapper for successful command output (now returns raw data without envelope)
#[derive(Debug)]
pub struct JsonOutput<T: Serialize> {
    pub data: T,
    pub message: Option<String>,
}

impl<T: Serialize> JsonOutput<T> {
    /// Create a new successful output with the given data
    /// Note: command parameter is kept for API compatibility but no longer used
    pub fn success(data: T, _command: impl Into<String>) -> Self {
        Self {
            data,
            message: None,
        }
    }

    /// Add a human-readable message to the JSON output.
    ///
    /// The message is injected as a top-level `"message"` field in the
    /// serialized JSON object. If the data serializes to a non-object
    /// (e.g. an array), the message is silently dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::output::JsonOutput;
    /// use serde_json::json;
    ///
    /// let output = JsonOutput::success(json!({"id": "abc"}), "issue create")
    ///     .with_message("Created issue abc");
    /// let json_str = output.to_json_string().unwrap();
    /// assert!(json_str.contains("\"message\": \"Created issue abc\""));
    /// ```
    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Serialize to JSON string with pretty formatting (returns raw data, no envelope)
    /// If a message is set, it is injected into the top-level object.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        let mut value = serde_json::to_value(&self.data)?;
        if let Some(ref msg) = self.message {
            if let Value::Object(ref mut map) = value {
                map.insert("message".to_string(), Value::String(msg.clone()));
            }
        }
        serde_json::to_string_pretty(&value)
    }
}

/// Wrapper for error output with suggestions (simplified, no envelope)
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub error: ErrorDetail,
}

#[allow(dead_code)]
impl JsonError {
    /// Create a new error output
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        _command: impl Into<String>, // Kept for API compatibility
    ) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
        }
    }

    /// Add details to the error
    pub fn with_details(mut self, details: Value) -> Self {
        self.error.details = Some(details);
        self
    }

    /// Add a suggestion to the error
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.error.suggestions.push(suggestion.into());
        self
    }

    /// Add multiple suggestions to the error
    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.error.suggestions.extend(suggestions);
        self
    }

    /// Serialize to JSON string with pretty formatting
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Get the appropriate exit code for this error
    pub fn exit_code(&self) -> ExitCode {
        ErrorCode::to_exit_code(&self.error.code)
    }
}

/// Error details including code, message, and suggestions
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Error code (e.g., "ISSUE_NOT_FOUND", "CYCLE_DETECTED")
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Optional additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Suggested actions to resolve the error
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

// ============================================================================
// Exit Codes
// ============================================================================

/// Standardized exit codes for the JIT CLI
///
/// These codes follow Unix conventions and provide consistent error reporting
/// for automation and scripting.
///
/// # Examples
///
/// ```rust
/// use jit::ExitCode;
///
/// // Success case
/// std::process::exit(ExitCode::Success.code());
///
/// // Error case
/// std::process::exit(ExitCode::NotFound.code());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
#[allow(dead_code)] // Part of public API
pub enum ExitCode {
    /// Command succeeded (0)
    Success = 0,

    /// Generic error (1)
    GenericError = 1,

    /// Invalid arguments or usage error (2)
    InvalidArgument = 2,

    /// Resource not found - issue, gate, etc. (3)
    NotFound = 3,

    /// Validation failed - cycle detected, broken refs, etc. (4)
    ValidationFailed = 4,

    /// Permission denied (5)
    PermissionDenied = 5,

    /// Resource already exists (6)
    AlreadyExists = 6,

    /// External dependency failed - git, file system, etc. (10)
    ExternalError = 10,
}

#[allow(dead_code)] // Part of public API
impl ExitCode {
    /// Convert exit code to i32 for `std::process::exit`
    pub fn code(self) -> i32 {
        self as i32
    }

    /// Get a description of what this exit code means
    pub fn description(self) -> &'static str {
        match self {
            ExitCode::Success => "Command succeeded",
            ExitCode::GenericError => "Generic error occurred",
            ExitCode::InvalidArgument => "Invalid arguments or usage error",
            ExitCode::NotFound => "Resource not found (issue, gate, etc.)",
            ExitCode::ValidationFailed => {
                "Validation failed (cycle detected, broken references, etc.)"
            }
            ExitCode::PermissionDenied => "Permission denied",
            ExitCode::AlreadyExists => "Resource already exists",
            ExitCode::ExternalError => "External dependency failed (git, file system, etc.)",
        }
    }

    /// Get all exit codes as a formatted string for documentation
    pub fn all_codes_documentation() -> String {
        format!(
            "Exit Codes:\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {} - {}",
            ExitCode::Success.code(),
            ExitCode::Success.description(),
            ExitCode::GenericError.code(),
            ExitCode::GenericError.description(),
            ExitCode::InvalidArgument.code(),
            ExitCode::InvalidArgument.description(),
            ExitCode::NotFound.code(),
            ExitCode::NotFound.description(),
            ExitCode::ValidationFailed.code(),
            ExitCode::ValidationFailed.description(),
            ExitCode::PermissionDenied.code(),
            ExitCode::PermissionDenied.description(),
            ExitCode::AlreadyExists.code(),
            ExitCode::AlreadyExists.description(),
            ExitCode::ExternalError.code(),
            ExitCode::ExternalError.description(),
        )
    }
}

// ============================================================================
// Error Codes (String constants for JSON responses)
// ============================================================================

/// Standard error codes for JIT operations (JSON format)
pub struct ErrorCode;

#[allow(dead_code)]
impl ErrorCode {
    pub const ISSUE_NOT_FOUND: &'static str = "ISSUE_NOT_FOUND";
    pub const GATE_NOT_FOUND: &'static str = "GATE_NOT_FOUND";
    pub const CYCLE_DETECTED: &'static str = "CYCLE_DETECTED";
    pub const INVALID_ARGUMENT: &'static str = "INVALID_ARGUMENT";
    pub const VALIDATION_FAILED: &'static str = "VALIDATION_FAILED";
    pub const ALREADY_EXISTS: &'static str = "ALREADY_EXISTS";
    pub const INVALID_STATE: &'static str = "INVALID_STATE";
    pub const BLOCKED: &'static str = "BLOCKED";
    pub const GATE_FAILED: &'static str = "GATE_FAILED";
    pub const IO_ERROR: &'static str = "IO_ERROR";
    pub const PARSE_ERROR: &'static str = "PARSE_ERROR";
}

impl ErrorCode {
    /// Map error code string to exit code
    pub fn to_exit_code(code: &str) -> ExitCode {
        match code {
            Self::ISSUE_NOT_FOUND | Self::GATE_NOT_FOUND => ExitCode::NotFound,
            Self::CYCLE_DETECTED | Self::VALIDATION_FAILED | Self::BLOCKED | Self::GATE_FAILED => {
                ExitCode::ValidationFailed
            }
            Self::INVALID_ARGUMENT | Self::INVALID_STATE => ExitCode::InvalidArgument,
            Self::ALREADY_EXISTS => ExitCode::AlreadyExists,
            Self::IO_ERROR => ExitCode::ExternalError,
            _ => ExitCode::GenericError,
        }
    }
}

/// Helper to create common error responses
#[allow(dead_code)]
impl JsonError {
    pub fn issue_not_found(issue_id: &str, command: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::ISSUE_NOT_FOUND,
            format!("Issue not found: {}", issue_id),
            command,
        )
        .with_details(serde_json::json!({"issue_id": issue_id}))
        .with_suggestion("Run 'jit query all' to see available issues")
        .with_suggestion("Check if the issue ID is correct")
    }

    pub fn gate_not_found(gate_key: &str, command: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::GATE_NOT_FOUND,
            format!("Gate not found: {}", gate_key),
            command,
        )
        .with_details(serde_json::json!({"gate_key": gate_key}))
        .with_suggestion("Run 'jit registry list' to see available gates")
        .with_suggestion("Add the gate to the registry first with 'jit registry add'")
    }

    pub fn cycle_detected(from: &str, to: &str, command: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::CYCLE_DETECTED,
            format!("Adding dependency would create a cycle: {} -> {}", from, to),
            command,
        )
        .with_details(serde_json::json!({"from": from, "to": to}))
        .with_suggestion("Remove existing dependencies that create the cycle")
        .with_suggestion("Use 'jit graph show' to visualize the dependency graph")
    }

    pub fn invalid_state(state: &str, command: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::INVALID_STATE,
            format!("Invalid state: {}", state),
            command,
        )
        .with_details(serde_json::json!({"invalid_state": state}))
        .with_suggestion("Valid states are: open, ready, in_progress, done")
    }

    pub fn invalid_priority(priority: &str, command: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::INVALID_ARGUMENT,
            format!("Invalid priority: {}", priority),
            command,
        )
        .with_details(serde_json::json!({"invalid_priority": priority}))
        .with_suggestion("Valid priorities are: low, normal, high, critical")
    }

    pub fn gate_validation_failed(
        unpassed_gates: &[String],
        issue_id: &str,
        command: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::VALIDATION_FAILED,
            format!(
                "Cannot transition to 'done' - {} gate(s) not passed: {}",
                unpassed_gates.len(),
                unpassed_gates.join(", ")
            ),
            command,
        )
        .with_details(serde_json::json!({
            "issue_id": issue_id,
            "requested_state": "done",
            "actual_state": "gated",
            "unpassed_gates": unpassed_gates
        }))
        .with_suggestion("Issue automatically transitioned to 'gated' (awaiting gate approval)")
        .with_suggestion("The issue will auto-transition to 'done' when all gates pass")
        .with_suggestion(format!(
            "To complete: jit gate pass {} <gate_key>",
            issue_id
        ))
    }

    /// Build a JSON error for a blocked state transition.
    ///
    /// This keeps machine-readable error shaping in the output layer while the
    /// command layer returns typed blocker data.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let error = JsonError::transition_blocked(&blocked, "issue update");
    /// assert_eq!(error.exit_code().code(), 4);
    /// ```
    pub fn transition_blocked(
        blocked: &TransitionBlockedError,
        command: impl Into<String>,
    ) -> Self {
        Self::new(blocked.error_code(), blocked.summary(), command)
            .with_details(serde_json::json!({
                "issue_id": blocked.issue_id(),
                "requested_state": state_name(blocked.requested_state()),
                "actual_state": state_name(blocked.actual_state()),
                "blockers": blocked.blockers().iter().map(transition_blocker_json).collect::<Vec<_>>(),
                "remediation": blocked.remediation_commands(),
            }))
            .with_suggestions(blocked.remediation_commands())
    }
}

fn transition_blocker_json(blocker: &TransitionBlocker) -> serde_json::Value {
    match blocker {
        TransitionBlocker::Dependency {
            issue_id,
            title,
            state,
        } => serde_json::json!({
            "type": "dependency",
            "issue_id": issue_id,
            "short_id": short_id(issue_id),
            "title": title,
            "state": state_name(*state),
        }),
        TransitionBlocker::MissingDependency { issue_id } => serde_json::json!({
            "type": "dependency",
            "issue_id": issue_id,
            "short_id": short_id(issue_id),
            "title": "(missing issue)",
            "state": "missing",
        }),
        TransitionBlocker::Gate { gate_key, status } => serde_json::json!({
            "type": "gate",
            "gate_key": gate_key,
            "status": gate_status_name(*status),
        }),
        TransitionBlocker::GraphRule { rule, message } => serde_json::json!({
            "type": "graph_rule",
            "rule": rule,
            "message": message,
        }),
    }
}

// ============================================================================
// Query Response Types
// ============================================================================

/// Response for `status` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub open: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Generic response for issue list queries (available, all, ready, etc.)
/// Uses MinimalIssue for efficiency - contains only id, title, state, priority
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueListResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for blocked query with reasons (minimal issue + reasons)
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedListResponse {
    pub issues: Vec<MinimalBlockedIssue>,
    pub count: usize,
}

/// Response for `query ready` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadyQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query blocked` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedQueryResponse {
    pub issues: Vec<MinimalBlockedIssue>,
    pub count: usize,
}

/// Issue with blocking reasons (minimal version for lists)
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedIssue {
    #[serde(flatten)]
    pub issue: MinimalIssue,
    pub blocked_reasons: Vec<BlockedReason>,
}

/// Reason why an issue is blocked
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedReason {
    #[serde(rename = "type")]
    pub reason_type: BlockedReasonType,
    pub detail: String,
}

/// Type of blocking reason
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BlockedReasonType {
    Dependency,
    Gate,
}

/// Response for `query assignee` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssigneeQueryResponse {
    pub assignee: String,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query state` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateQueryResponse {
    pub state: State,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query priority` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct PriorityQueryResponse {
    pub priority: Priority,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query label` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelQueryResponse {
    pub pattern: String,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query strategic` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StrategicQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query closed` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClosedQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

// ============================================================================
// Graph Response Types
// ============================================================================

/// Response for `graph downstream` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDownstreamResponse {
    pub issue_id: String,
    pub dependents: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `graph deps` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDepsResponse {
    pub issue_id: String,
    pub dependencies: Vec<MinimalIssue>,
    pub count: usize,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Whether the list was truncated due to size limits
    #[serde(default)]
    pub truncated: bool,
}

/// Tree node for hierarchical dependency display
#[derive(Debug, Serialize, JsonSchema, Clone)]
pub struct DependencyTreeNode {
    /// Issue ID
    pub id: String,
    /// Short ID (first 8 chars)
    pub short_id: String,
    /// Issue title
    pub title: String,
    /// Current state
    pub state: State,
    /// Priority
    pub priority: Priority,
    /// Depth level in tree (1 = immediate child, 2 = grandchild, etc.)
    pub level: u32,
    /// Whether this node appears multiple times in the tree (shared dependency)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    /// Child dependencies
    pub children: Vec<DependencyTreeNode>,
}

impl DependencyTreeNode {
    /// Create from MinimalIssue
    pub fn from_minimal(issue: &MinimalIssue, level: u32) -> Self {
        Self {
            short_id: issue.short_id(),
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            level,
            shared: None,
            children: Vec::new(),
        }
    }

    /// Get state symbol for display
    pub fn state_symbol(&self) -> &str {
        match self.state {
            State::Done | State::Rejected => "✓",
            _ => "○",
        }
    }
}

/// Response for `graph deps` with tree structure
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDepsTreeResponse {
    pub issue_id: String,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Tree of dependencies
    pub tree: Vec<DependencyTreeNode>,
    /// Summary statistics
    pub summary: DependencySummary,
}

/// Summary statistics for dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencySummary {
    /// Total number of unique dependencies
    pub total: usize,
    /// Count by state
    pub by_state: std::collections::HashMap<String, usize>,
}

/// Response for `graph roots` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphRootsResponse {
    pub roots: Vec<MinimalIssue>,
    pub count: usize,
}

// ============================================================================
// Issue Show Response
// ============================================================================

/// Response for `issue show` command with enriched dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowResponse {
    pub id: String,
    pub title: String,
    pub description: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    /// Enriched dependency list with full metadata
    pub dependencies: Vec<MinimalIssue>,
    pub gates_required: Vec<String>,
    pub gates_status: std::collections::HashMap<String, crate::domain::GateState>,
    pub context: std::collections::HashMap<String, String>,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub labels: Vec<String>,
    /// Per-issue content format override; absent means the repo default applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_format: Option<crate::domain::ContentFormat>,
    pub created_at: String,
    pub updated_at: String,
}

impl IssueShowResponse {
    /// Create from Issue with enriched dependencies
    pub fn from_issue(issue: crate::domain::Issue, enriched_deps: Vec<MinimalIssue>) -> Self {
        Self {
            id: issue.id,
            title: issue.title,
            description: issue.description,
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee,
            dependencies: enriched_deps,
            gates_required: issue.gates_required,
            gates_status: issue.gates_status,
            context: issue.context,
            documents: issue.documents,
            labels: issue.labels,
            content_format: issue.content_format,
            created_at: issue.created_at,
            updated_at: issue.updated_at,
        }
    }
}

// ============================================================================
// Lean Issue Update / Show Summary Responses
// ============================================================================

/// Lightweight confirmation returned by `jit issue update --json`.
///
/// Mutating an issue does not need to echo the full body back; agents that
/// need it can call `jit issue show`.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::output::IssueUpdateResponse;
///
/// let issue = Issue::new("Refactor".into(), "Body".into());
/// let response = IssueUpdateResponse::from(&issue);
/// assert_eq!(response.id, issue.id);
/// assert_eq!(response.short_id.len(), 8);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueUpdateResponse {
    pub id: String,
    pub short_id: String,
    pub state: State,
    pub updated_at: String,
}

impl From<&Issue> for IssueUpdateResponse {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            short_id: issue.short_id(),
            state: issue.state,
            updated_at: issue.updated_at.clone(),
        }
    }
}

/// Compact response returned by `jit issue show --summary --json`.
///
/// Carries the `MinimalIssue` fields plus `gates_status`, but omits the
/// description and enriched dependency list.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::output::IssueShowSummaryResponse;
///
/// let issue = Issue::new("Title".into(), "Long description body".into());
/// let summary = IssueShowSummaryResponse::from(&issue);
/// // Description is intentionally absent from the summary shape.
/// let json = serde_json::to_string(&summary).unwrap();
/// assert!(!json.contains("Long description body"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowSummaryResponse {
    pub id: String,
    pub short_id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    pub gates_required: Vec<String>,
    pub gates_status: std::collections::HashMap<String, GateState>,
}

impl From<&Issue> for IssueShowSummaryResponse {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            short_id: issue.short_id(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.clone(),
            labels: issue.labels.clone(),
            gates_required: issue.gates_required.clone(),
            gates_status: issue.gates_status.clone(),
        }
    }
}

// ============================================================================
// Lean Gate Run Summary
// ============================================================================

/// Summary of a single gate run.
///
/// `stdout` and `stderr` are optional so the same shape can carry either a
/// lean (passing) record or a full (failing or `--full`) record. Build via
/// [`GateRunSummary::lean`] or [`GateRunSummary::full`].
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use jit::domain::{GateRunResult, GateRunStatus, GateStage};
/// use jit::output::GateRunSummary;
///
/// let run = GateRunResult {
///     schema_version: 1,
///     run_id: "r1".into(),
///     gate_key: "tests".into(),
///     stage: GateStage::Postcheck,
///     issue_id: "i1".into(),
///     commit: None,
///     branch: None,
///     status: GateRunStatus::Passed,
///     started_at: Utc::now(),
///     completed_at: None,
///     duration_ms: None,
///     exit_code: Some(0),
///     stdout: "lots of output".into(),
///     stderr: String::new(),
///     command: "cargo test".into(),
///     by: None,
///     message: None,
/// };
/// // Lean form drops stdout/stderr for passing runs.
/// let lean = GateRunSummary::lean(&run);
/// assert!(lean.stdout.is_none());
/// // Full form keeps everything.
/// let full = GateRunSummary::full(&run);
/// assert_eq!(full.stdout.as_deref(), Some("lots of output"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateRunSummary {
    pub run_id: String,
    pub gate_key: String,
    pub stage: GateStage,
    pub status: GateRunStatus,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

impl GateRunSummary {
    /// Build a summary that drops stdout/stderr for passing runs but keeps
    /// them for failed/error runs so diagnostics survive.
    pub fn lean(r: &GateRunResult) -> Self {
        let include_output = !matches!(r.status, GateRunStatus::Passed);
        Self::build(r, include_output)
    }

    /// Build a summary that always includes stdout/stderr.
    pub fn full(r: &GateRunResult) -> Self {
        Self::build(r, true)
    }

    fn build(r: &GateRunResult, include_output: bool) -> Self {
        Self {
            run_id: r.run_id.clone(),
            gate_key: r.gate_key.clone(),
            stage: r.stage,
            status: r.status,
            started_at: r.started_at.to_rfc3339(),
            completed_at: r.completed_at.map(|t| t.to_rfc3339()),
            duration_ms: r.duration_ms,
            exit_code: r.exit_code,
            command: r.command.clone(),
            commit: r.commit.clone(),
            branch: r.branch.clone(),
            by: r.by.clone(),
            message: r.message.clone(),
            stdout: include_output.then(|| r.stdout.clone()),
            stderr: include_output.then(|| r.stderr.clone()),
        }
    }
}

/// JSON payload of `jit gate check-all --json`.
///
/// `results` contains one [`GateRunSummary`] per recorded run; `not_run`
/// lists gate keys configured on the issue that have not executed yet.
///
/// # Examples
///
/// ```
/// use jit::output::GateCheckAllResponse;
///
/// let payload = GateCheckAllResponse {
///     results: vec![],
///     passed: 0,
///     total: 2,
///     not_run: vec!["tests".into(), "clippy".into()],
/// };
/// assert_eq!(payload.not_run.len(), 2);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateCheckAllResponse {
    pub results: Vec<GateRunSummary>,
    pub passed: usize,
    pub total: usize,
    pub not_run: Vec<String>,
}

// ============================================================================
// Registry Response Types
// ============================================================================

/// Response for `registry list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegistryListResponse {
    pub gates: Vec<GateDefinition>,
    pub count: usize,
}

/// Response for `gate list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateListResponse {
    pub gates: Vec<GateDefinition>,
    pub count: usize,
}

/// Gate definition structure (for registry responses)
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateDefinition {
    pub key: String,
    pub title: String,
    pub description: String,
    pub auto: bool,
    pub example_integration: Option<String>,
    pub stage: String,
    pub mode: String,
}

impl From<crate::domain::Gate> for GateDefinition {
    fn from(gate: crate::domain::Gate) -> Self {
        Self {
            key: gate.key,
            title: gate.title,
            description: gate.description,
            auto: gate.auto,
            example_integration: gate.example_integration,
            stage: format!("{:?}", gate.stage).to_lowercase(),
            mode: format!("{:?}", gate.mode).to_lowercase(),
        }
    }
}

// ============================================================================
// Label Response Types
// ============================================================================

/// Response for `label namespaces` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct NamespacesResponse {
    pub namespaces: Vec<String>,
    pub count: usize,
}

/// Response for top-level `search` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<crate::search::SearchResult>,
    pub count: usize,
}

/// Response for `worktree list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeListResponse {
    pub worktrees: Vec<crate::commands::worktree::WorktreeListEntry>,
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_show_response_exposes_content_format() {
        use crate::domain::{ContentFormat, Issue};
        // Set -> appears in `jit issue show --json` (create/show parity).
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        issue.content_format = Some(ContentFormat::Html);
        let resp = IssueShowResponse::from_issue(issue, vec![]);
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["content_format"], "html");

        // Absent -> omitted (existing issues without the field stay clean).
        let issue2 = Issue::new("T".to_string(), "B".to_string());
        let resp2 = IssueShowResponse::from_issue(issue2, vec![]);
        let v2 = serde_json::to_value(&resp2).unwrap();
        assert!(v2.get("content_format").is_none());
    }

    #[test]
    fn test_json_output_success() {
        let data = json!({"id": "123", "title": "Test"});
        let output = JsonOutput::success(data, "issue show");

        // success field removed
        assert_eq!(output.data["id"], "123");
        // metadata removed
    }

    #[test]
    fn test_json_output_serialization() {
        let data = json!({"id": "123", "title": "test"});
        let output = JsonOutput::success(data, "issue list");

        let json_str = output.to_json_string().unwrap();
        // Should contain raw data without envelope
        assert!(json_str.contains("\"id\": \"123\""));
        assert!(json_str.contains("\"title\": \"test\""));
        // Should NOT contain envelope fields
        assert!(!json_str.contains("\"success\""));
        assert!(!json_str.contains("\"data\":"));
    }

    #[test]
    fn test_json_output_with_message() {
        let data = json!({"id": "abc12345", "title": "Test issue"});
        let output = JsonOutput::success(data, "issue create")
            .with_message("Created issue abc12345 - Test issue");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["message"], "Created issue abc12345 - Test issue");
        assert_eq!(parsed["id"], "abc12345");
        assert_eq!(parsed["title"], "Test issue");
    }

    #[test]
    fn test_json_output_without_message() {
        let data = json!({"id": "123"});
        let output = JsonOutput::success(data, "test");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("message").is_none());
    }

    #[test]
    fn test_json_output_with_message_array_payload() {
        // When data serializes to a JSON array (not object), message cannot be injected.
        // Verify the output is still valid JSON (the array), just without message.
        let data = json!([{"id": "a"}, {"id": "b"}]);
        let output = JsonOutput::success(data, "some list").with_message("Should not appear");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.is_array(), "Array payload should serialize as array");
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        // message is silently dropped for non-object payloads
    }

    #[test]
    fn test_json_error_basic() {
        let error = JsonError::new("TEST_ERROR", "This is a test error", "test command");

        assert_eq!(error.error.code, "TEST_ERROR");
        assert_eq!(error.error.message, "This is a test error");
        assert!(error.error.details.is_none());
        assert!(error.error.suggestions.is_empty());
    }

    #[test]
    fn test_json_error_with_details() {
        let error = JsonError::new("NOT_FOUND", "Resource not found", "show resource")
            .with_details(json!({"requested_id": "abc123"}));

        assert_eq!(error.error.details, Some(json!({"requested_id": "abc123"})));
    }

    #[test]
    fn test_json_error_with_suggestions() {
        let error = JsonError::new("NOT_FOUND", "Issue not found", "issue show")
            .with_suggestion("Run 'jit issue list' to see available issues")
            .with_suggestion("Check if the issue ID is correct");

        assert_eq!(error.error.suggestions.len(), 2);
        assert!(error.error.suggestions[0].contains("jit issue list"));
    }

    #[test]
    fn test_json_error_serialization() {
        let error = JsonError::new("TEST_ERROR", "Test", "test")
            .with_details(json!({"key": "value"}))
            .with_suggestion("Try something");

        let json_str = error.to_json_string().unwrap();
        // Should have error object without envelope
        assert!(json_str.contains("\"code\": \"TEST_ERROR\""));
        assert!(json_str.contains("\"message\": \"Test\""));
        assert!(json_str.contains("\"details\""));
        assert!(json_str.contains("\"suggestions\""));
        // Should NOT have envelope fields
        assert!(!json_str.contains("\"success\""));
        assert!(!json_str.contains("\"metadata\""));
    }

    #[test]
    fn test_gate_failed_error_code_is_validation_failure() {
        assert_eq!(
            ErrorCode::to_exit_code(ErrorCode::GATE_FAILED),
            ExitCode::ValidationFailed
        );
    }

    // ========================================================================
    // Query Response Tests
    // ========================================================================

    /// Create a test MinimalIssue
    fn test_minimal_issue() -> MinimalIssue {
        MinimalIssue {
            id: "test-id".to_string(),
            title: "Issue 1".to_string(),
            state: State::Ready,
            priority: Priority::Normal,
            assignee: None,
            labels: Vec::new(),
        }
    }

    #[test]
    fn test_ready_query_response_serialization() {
        let issues = vec![test_minimal_issue()];
        let response = ReadyQueryResponse { issues, count: 1 };

        let json_output = JsonOutput::success(response, "query ready");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"count\": 1"));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"success\""));
        assert!(!serialized.contains("\"metadata\""));
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_blocked_query_response_serialization() {
        let blocked_issue = MinimalBlockedIssue {
            id: "test-id".to_string(),
            title: "Issue 1".to_string(),
            state: State::Ready,
            priority: Priority::Normal,
            assignee: None,
            labels: vec![],
            blocked_reasons: vec!["dep:abc123".to_string()],
        };

        let response = BlockedQueryResponse {
            issues: vec![blocked_issue],
            count: 1,
        };

        let json_output = JsonOutput::success(response, "query blocked");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"blocked_reasons\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"success\""));
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_assignee_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = AssigneeQueryResponse {
            assignee: "copilot:session-1".to_string(),
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response, "query assignee");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"assignee\""));
        assert!(serialized.contains("copilot:session-1"));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_state_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = StateQueryResponse {
            state: State::Ready,
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response, "query state");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"state\""));
        assert!(serialized.contains("\"ready\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_priority_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = PriorityQueryResponse {
            priority: Priority::High,
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response, "query priority");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"priority\""));
        assert!(serialized.contains("\"high\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_blocked_reason_types() {
        let dep = BlockedReason {
            reason_type: BlockedReasonType::Dependency,
            detail: "ABC".to_string(),
        };
        let gate = BlockedReason {
            reason_type: BlockedReasonType::Gate,
            detail: "test-gate".to_string(),
        };

        let dep_json = serde_json::to_value(&dep).unwrap();
        let gate_json = serde_json::to_value(&gate).unwrap();

        assert_eq!(dep_json["type"], "dependency");
        assert_eq!(gate_json["type"], "gate");
    }
}
