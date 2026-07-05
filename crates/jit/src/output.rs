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
use thiserror::Error;

use crate::domain::{
    GateFindings, GateMode, GateRunResult, GateRunStatus, GateStage, GateState, GateStatus, Issue,
    MinimalBlockedIssue, MinimalIssue, Priority, State,
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
    /// A claim/lease command was run outside a git repository (exit code 10).
    pub const CLAIM_REQUIRES_GIT: &'static str = "CLAIM_REQUIRES_GIT";
    /// An id prefix matched more than one candidate (exit code 2).
    pub const AMBIGUOUS_ID: &'static str = "AMBIGUOUS_ID";
    /// An id prefix was shorter than the 4-character minimum (exit code 2).
    pub const INVALID_ID_PREFIX: &'static str = "INVALID_ID_PREFIX";
    /// No `.jit` repository exists at the resolved data directory (exit code 3).
    pub const REPOSITORY_NOT_FOUND: &'static str = "REPOSITORY_NOT_FOUND";
    /// The repository's on-disk format is newer than this binary supports (exit code 10).
    pub const REPOSITORY_FORMAT_TOO_NEW: &'static str = "REPOSITORY_FORMAT_TOO_NEW";
}

impl ErrorCode {
    /// Map error code string to exit code
    pub fn to_exit_code(code: &str) -> ExitCode {
        match code {
            Self::ISSUE_NOT_FOUND | Self::GATE_NOT_FOUND => ExitCode::NotFound,
            Self::CYCLE_DETECTED | Self::VALIDATION_FAILED | Self::BLOCKED | Self::GATE_FAILED => {
                ExitCode::ValidationFailed
            }
            Self::INVALID_ARGUMENT
            | Self::INVALID_STATE
            | Self::AMBIGUOUS_ID
            | Self::INVALID_ID_PREFIX => ExitCode::InvalidArgument,
            Self::ALREADY_EXISTS => ExitCode::AlreadyExists,
            Self::REPOSITORY_NOT_FOUND => ExitCode::NotFound,
            Self::IO_ERROR | Self::CLAIM_REQUIRES_GIT | Self::REPOSITORY_FORMAT_TOO_NEW => {
                ExitCode::ExternalError
            }
            _ => ExitCode::GenericError,
        }
    }
}

/// Refine a fallback JSON error when the underlying failure is a typed
/// id-resolution error.
///
/// The generic `--json` error path at each command hands in a call-site fallback
/// (e.g. `ISSUE_NOT_FOUND`). When the actual failure is an ambiguous-prefix or
/// too-short-prefix id lookup, that fallback misreports both the code and the
/// exit class, so this replaces it with the matching argument error
/// (`AMBIGUOUS_ID` / `INVALID_ID_PREFIX`, exit code 2) carrying the offending
/// prefix in `details`. Any other error keeps its `fallback` unchanged, so this
/// is a no-op for the vast majority of call sites.
///
/// # Examples
///
/// ```
/// use jit::output::{refine_id_error, ErrorCode, JsonError};
/// use jit::storage::InvalidIdPrefixError;
///
/// let err: anyhow::Error = InvalidIdPrefixError::new("ab").into();
/// let fallback = JsonError::new(ErrorCode::ISSUE_NOT_FOUND, "Issue not found: ab", "issue show");
/// let refined = refine_id_error(&err, fallback);
/// assert_eq!(refined.exit_code(), jit::ExitCode::InvalidArgument);
///
/// // A non-prefix error keeps the fallback untouched.
/// let other = anyhow::anyhow!("something else");
/// let fallback = JsonError::new(ErrorCode::ISSUE_NOT_FOUND, "Issue not found", "issue show");
/// let kept = refine_id_error(&other, fallback);
/// assert_eq!(kept.exit_code(), jit::ExitCode::NotFound);
/// ```
pub fn refine_id_error(error: &anyhow::Error, fallback: JsonError) -> JsonError {
    if let Some(prefix_error) = error.downcast_ref::<crate::storage::InvalidIdPrefixError>() {
        return JsonError::new(
            ErrorCode::INVALID_ID_PREFIX,
            prefix_error.to_string(),
            String::new(),
        )
        .with_details(serde_json::json!({ "prefix": prefix_error.prefix() }));
    }
    if let Some(ambiguous) = error.downcast_ref::<crate::storage::AmbiguousIdError>() {
        return JsonError::new(
            ErrorCode::AMBIGUOUS_ID,
            ambiguous.to_string(),
            String::new(),
        )
        .with_details(serde_json::json!({
            "prefix": ambiguous.prefix(),
            "matches": ambiguous.matches(),
        }));
    }
    fallback
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
        .with_details(serde_json::json!({"key": gate_key}))
        .with_suggestion("Run 'jit gate list' to see available gates")
        .with_suggestion("Add the gate to the registry first with 'jit gate define'")
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
            "To complete: jit gate evaluate {} <gate_key>",
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
                "warnings": blocked.warnings(),
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
            "key": gate_key,
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

/// Response for `graph deps` with tree structure.
///
/// List envelope: `count` is the number of top-level entries in `nodes` (the
/// immediate dependency nodes; each may carry nested `children`). This differs
/// from `summary.total`, which counts every unique dependency across the whole
/// tree. The node collection was renamed from `tree` to `nodes` when the
/// envelope landed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDepsTreeResponse {
    pub issue_id: String,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Number of top-level entries in `nodes` (list envelope `count`).
    pub count: usize,
    /// Dependency nodes, each of which may carry nested `children`.
    pub nodes: Vec<DependencyTreeNode>,
    /// Summary statistics
    pub summary: DependencySummary,
}

/// Summary statistics for dependencies
///
/// # Examples
///
/// ```
/// use jit::output::DependencySummary;
/// use jit::domain::State;
/// use std::collections::HashMap;
///
/// let mut by_state = HashMap::new();
/// by_state.insert(State::Done, 2usize);
/// by_state.insert(State::Ready, 1usize);
/// let summary = DependencySummary { total: 3, by_state };
///
/// // Keys serialize as snake_case JSON strings.
/// let json = serde_json::to_string(&summary).unwrap();
/// assert!(json.contains("\"done\":2") || json.contains("\"done\": 2"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencySummary {
    /// Total number of unique dependencies
    pub total: usize,
    /// Count by state (keys are canonical snake_case state names)
    pub by_state: std::collections::HashMap<State, usize>,
}

/// Response for `graph roots` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphRootsResponse {
    pub roots: Vec<MinimalIssue>,
    pub count: usize,
}

/// One node's resolved hierarchy facts, as rendered by `graph tree`.
///
/// The `type` key is the value of the node's `type:` label (absent when it has
/// none). `parent`, `cluster`, `children`, and `rank` are the DAG-authoritative
/// resolution from [`resolve_hierarchy`](crate::graph::hierarchy::resolve_hierarchy).
#[derive(Debug, Serialize, JsonSchema)]
pub struct HierarchyNodeView {
    /// Full issue id.
    pub id: String,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The node's `type:` label value, if any.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// Nearest dominating container id, or `null` for a root node.
    pub parent: Option<String>,
    /// Ids of nodes whose resolved parent is this node, sorted ascending.
    pub children: Vec<String>,
    /// Strategic root container id, or `null` for an orphan leaf.
    pub cluster: Option<String>,
    /// Longest dependency-path length to an in-set sink.
    pub rank: u32,
}

/// Response for `graph tree` command.
///
/// List envelope: `count` is the number of entries in `nodes`. `root` echoes the
/// optional root id the view was scoped to (`null` for the whole repository).
/// Each node carries its DAG-resolved parent, children, cluster, and rank.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphTreeResponse {
    /// The root id the tree was scoped to, or `null` for the whole repository.
    pub root: Option<String>,
    /// Number of entries in `nodes`.
    pub count: usize,
    /// Resolved hierarchy per node, ordered by ascending short id.
    pub nodes: Vec<HierarchyNodeView>,
}

/// One reported membership-vs-DAG divergence, as rendered by `query divergence`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceView {
    /// Full id of the issue carrying the unsupported membership label.
    pub id: String,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The full `namespace:value` membership label.
    pub label: String,
    /// The label namespace.
    pub namespace: String,
    /// The label value.
    pub value: String,
}

/// Response for `query divergence` command.
///
/// List envelope: `count` is the number of entries in `divergences`. Each entry
/// is a membership label whose claim the dependency DAG does not back (the issue
/// carries the label but is not in the closure of the container that owns it).
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceResponse {
    /// Number of entries in `divergences`.
    pub count: usize,
    /// The reported divergences, ordered by `(issue_id, label)`.
    pub divergences: Vec<DivergenceView>,
}

// ============================================================================
// Issue Show Response
// ============================================================================

/// Per-gate view emitted in `issue show --json`.
///
/// One entry per gate in the issue's `gates_required`. `status` reflects the
/// gate's [`GateState`] (or `pending` when none exists). `last_run_at` and
/// `exit_code` come from the gate's latest [`GateRunResult`] and are both
/// `null` when no run has been recorded (required-but-never-run, or a manual
/// gate attested without a run).
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::output::IssueShowResponse;
///
/// // A required-but-never-run gate surfaces as a pending `GateView` with both
/// // run fields absent.
/// let mut issue = Issue::new("Title".into(), "Body".into());
/// issue.gates_required = vec!["tests".into()];
/// let response = IssueShowResponse::from_issue(issue, vec![], &[]);
///
/// let gate = &response.gates[0];
/// assert_eq!(gate.key, "tests");
/// assert!(gate.last_run_at.is_none());
/// assert!(gate.exit_code.is_none());
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateView {
    /// Gate key.
    pub key: String,
    /// Gate status: `pending`, `passed`, or `failed`.
    pub status: crate::domain::GateStatus,
    /// Timestamp of the gate's latest run, or `null` when never run.
    pub last_run_at: Option<String>,
    /// Exit code of the gate's latest run, or `null` when never run.
    pub exit_code: Option<i32>,
}

impl GateView {
    /// Build the per-gate view for a single required gate.
    ///
    /// `state` is the issue's recorded [`GateState`] for this gate (if any) and
    /// `runs` are all recorded runs for the issue; the latest run matching
    /// `key` supplies `last_run_at`/`exit_code`.
    fn build(key: &str, state: Option<&crate::domain::GateState>, runs: &[GateRunResult]) -> Self {
        let status = state
            .map(|s| s.status)
            .unwrap_or(crate::domain::GateStatus::Pending);

        let latest = runs
            .iter()
            .filter(|r| r.gate_key == key)
            .max_by_key(|r| r.started_at);

        GateView {
            key: key.to_string(),
            status,
            last_run_at: latest.map(|r| r.started_at.to_rfc3339()),
            exit_code: latest.and_then(|r| r.exit_code),
        }
    }
}

/// A single unmet dependency, as projected into `issue show --json` and the
/// compact `issue status` view.
///
/// A dependency is *unmet* when it is not in a terminal state (`Done` or
/// `Rejected`) — the exact same readiness test as [`Issue::is_blocked`] and
/// [`query_ready`](crate::domain::queries::query_ready): a terminal dependency
/// unblocks its dependents, so it is never listed here. The shape is a subset of
/// the enriched `dependencies` entries (`id`, `short_id`, `title`, `state`),
/// carrying only what an orchestrator needs to see what is still blocking work.
///
/// # Examples
///
/// ```
/// use jit::domain::{Issue, MinimalIssue, State};
/// use jit::output::UnmetDependency;
///
/// let mut dep = Issue::new("Upstream".into(), String::new());
/// dep.state = State::InProgress;
/// let minimal = MinimalIssue::from(&dep);
///
/// let unmet = UnmetDependency::from(&minimal);
/// assert_eq!(unmet.id, dep.id);
/// assert_eq!(unmet.state, State::InProgress);
/// assert_eq!(unmet.short_id, dep.id[..8]);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct UnmetDependency {
    pub id: String,
    /// Short ID (first 8 chars of the full UUID), for human-readable references.
    pub short_id: String,
    pub title: String,
    pub state: State,
}

impl From<&MinimalIssue> for UnmetDependency {
    fn from(dep: &MinimalIssue) -> Self {
        Self {
            id: dep.id.clone(),
            short_id: dep.short_id(),
            title: dep.title.clone(),
            state: dep.state,
        }
    }
}

/// Response for `issue show` command with enriched dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowResponse {
    pub id: String,
    /// Short ID (first 8 chars of the full UUID), for human-readable references.
    pub short_id: String,
    pub title: String,
    pub description: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    /// Enriched dependency list with full metadata
    pub dependencies: Vec<MinimalIssue>,
    /// Dependency ids from the issue's stored `dependencies` array that did
    /// NOT resolve to an existing issue (e.g. a stale reference left by raw
    /// storage mutation or legacy data predating the delete-cascade fix in
    /// `CommandExecutor::delete_issue`, jit:f847df3f). Deleting an issue
    /// through the CLI strips its own id from every dependent's
    /// `dependencies`, so this is empty in a repository touched only through
    /// the CLI; when non-empty, it means a dangling id exists and — unlike
    /// `dependencies`, which silently omits ids it cannot resolve — is not
    /// hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling_dependency_ids: Vec<String>,
    /// The subset of `dependencies` that are not yet met (state is not terminal),
    /// consistent with readiness — see [`UnmetDependency`]. Always an array,
    /// empty when every dependency is `Done`/`Rejected` or there are none.
    pub unmet_dependencies: Vec<UnmetDependency>,
    /// Per-gate view, one entry per required gate, enriched from each gate's
    /// latest run.
    pub gates: Vec<GateView>,
    pub context: std::collections::HashMap<String, String>,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub labels: Vec<String>,
    /// Per-issue content format override; absent means the repo default applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_format: Option<crate::domain::ContentFormat>,
    pub created_at: String,
    pub updated_at: String,
    /// When the issue FIRST entered `ready` (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ready_at: Option<String>,
    /// When the issue was FIRST claimed/assigned (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    /// When the issue FIRST reached `done` (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_at: Option<String>,
}

impl IssueShowResponse {
    /// Create from Issue with enriched dependencies and the issue's gate runs.
    ///
    /// `gate_runs` are all recorded [`GateRunResult`]s for the issue; the
    /// latest run per required gate enriches that gate's `last_run_at` and
    /// `exit_code`. Pass an empty slice when no runs exist.
    ///
    /// `enriched_deps` normally holds one [`MinimalIssue`] per entry in
    /// `issue.dependencies`, but an id that no longer resolves to a stored
    /// issue is left out of it by
    /// [`get_dependencies_enriched`](crate::commands::CommandExecutor::get_dependencies_enriched).
    /// This constructor recovers any such id by diffing `enriched_deps`
    /// against `issue.dependencies` and reports it via
    /// `dangling_dependency_ids` instead of dropping it silently
    /// (jit:f847df3f).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::output::IssueShowResponse;
    ///
    /// let issue = Issue::new("Refactor".into(), "Body".into());
    /// // No required gates and no runs -> an empty `gates` array.
    /// let response = IssueShowResponse::from_issue(issue.clone(), vec![], &[]);
    /// assert_eq!(response.id, issue.id);
    /// assert!(response.gates.is_empty());
    /// assert!(response.dangling_dependency_ids.is_empty());
    ///
    /// // A stored dependency id that isn't among `enriched_deps` (e.g. its
    /// // target no longer exists) surfaces in `dangling_dependency_ids`
    /// // rather than being silently dropped.
    /// let mut orphaned = Issue::new("Parent".into(), "".into());
    /// orphaned.dependencies = vec!["missing-id".to_string()];
    /// let response = IssueShowResponse::from_issue(orphaned, vec![], &[]);
    /// assert_eq!(response.dangling_dependency_ids, vec!["missing-id".to_string()]);
    /// ```
    pub fn from_issue(
        issue: crate::domain::Issue,
        enriched_deps: Vec<MinimalIssue>,
        gate_runs: &[GateRunResult],
    ) -> Self {
        let gates = issue
            .gates_required
            .iter()
            .map(|key| GateView::build(key, issue.gates_status.get(key), gate_runs))
            .collect();

        // See the doc comment above: recover ids `enriched_deps` couldn't
        // resolve so they surface instead of vanishing from the response.
        let resolved_ids: std::collections::HashSet<&str> =
            enriched_deps.iter().map(|dep| dep.id.as_str()).collect();
        let dangling_dependency_ids: Vec<String> = issue
            .dependencies
            .iter()
            .filter(|dep_id| !resolved_ids.contains(dep_id.as_str()))
            .cloned()
            .collect();

        // Unmet = resolved dependency whose state is not terminal, the same
        // met/unmet test `Issue::is_blocked` / `query_ready` apply. Dangling ids
        // are reported separately above rather than projected here.
        let unmet_dependencies: Vec<UnmetDependency> = enriched_deps
            .iter()
            .filter(|dep| !dep.state.is_terminal())
            .map(UnmetDependency::from)
            .collect();

        Self {
            short_id: issue.short_id(),
            id: issue.id,
            title: issue.title,
            description: issue.description,
            state: issue.state,
            priority: issue.priority,
            assignee: issue
                .assignee
                .as_ref()
                .map(crate::domain::Assignee::to_string),
            dependencies: enriched_deps,
            dangling_dependency_ids,
            unmet_dependencies,
            gates,
            context: issue.context,
            documents: issue.documents,
            labels: issue.labels,
            content_format: issue.content_format,
            created_at: issue.created_at.to_rfc3339(),
            updated_at: issue.updated_at.to_rfc3339(),
            first_ready_at: issue.first_ready_at.map(|t| t.to_rfc3339()),
            claimed_at: issue.claimed_at.map(|t| t.to_rfc3339()),
            done_at: issue.done_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Compact orchestration status for one issue: where it stands in one glance.
///
/// This is the small shape behind `jit issue status` (text one-liner and
/// `--json` object). It projects an [`IssueShowResponse`] down to the fields an
/// agent reconstructs by hand when it only needs "state, per-gate status, and
/// what is still blocking": `state`, a `{key, status}` entry per required gate
/// (b1586c0d convention), and the `short_id`s of the [unmet
/// dependencies](UnmetDependency). It carries no run history, description, or
/// enriched dependency metadata.
///
/// # Examples
///
/// ```
/// use jit::domain::{GateStatus, Issue, State};
/// use jit::output::{IssueShowResponse, IssueStatusResponse};
///
/// let mut issue = Issue::new("Build parser".into(), "Body".into());
/// issue.state = State::Ready;
/// issue.gates_required = vec!["tests".into()];
/// let show = IssueShowResponse::from_issue(issue, vec![], &[]);
///
/// let status = IssueStatusResponse::from_show(&show);
/// assert_eq!(status.state, State::Ready);
/// assert_eq!(status.gates.len(), 1);
/// assert_eq!(status.gates[0].key, "tests");
/// assert_eq!(status.gates[0].status, GateStatus::Pending);
/// assert!(status.unmet_dependencies.is_empty());
///
/// // The one-line render is stable and greppable; empty sections read `none`.
/// let line = status.to_line();
/// assert!(line.contains("[ready]"));
/// assert!(line.contains("gates: tests=pending"));
/// assert!(line.contains("unmet: none"));
/// assert!(line.ends_with("title: Build parser"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueStatusResponse {
    /// Short ID (first 8 chars of the full UUID).
    pub short_id: String,
    pub state: State,
    /// One entry per required gate, `{key, status}` only.
    pub gates: Vec<GateStatusEntry>,
    /// Short ids of the dependencies that are not yet met (state not terminal),
    /// consistent with readiness. Empty when nothing is blocking.
    pub unmet_dependencies: Vec<String>,
    pub title: String,
}

impl IssueStatusResponse {
    /// Project a full [`IssueShowResponse`] down to the compact status shape.
    pub fn from_show(show: &IssueShowResponse) -> Self {
        Self {
            short_id: show.short_id.clone(),
            state: show.state,
            gates: show
                .gates
                .iter()
                .map(|g| GateStatusEntry {
                    key: g.key.clone(),
                    status: g.status,
                })
                .collect(),
            unmet_dependencies: show
                .unmet_dependencies
                .iter()
                .map(|d| d.short_id.clone())
                .collect(),
            title: show.title.clone(),
        }
    }

    /// Render the stable, greppable one-line text form:
    ///
    /// ```text
    /// <short_id> [<state>] gates: <key>=<status>,... unmet: <short_id>,... title: <title>
    /// ```
    ///
    /// The `gates:` and `unmet:` sections each render `none` when empty, so the
    /// field order and separators are fixed regardless of content. `state` and
    /// each gate `status` use their canonical snake_case names.
    pub fn to_line(&self) -> String {
        let gates = if self.gates.is_empty() {
            "none".to_string()
        } else {
            self.gates
                .iter()
                .map(|g| format!("{}={}", g.key, g.status.as_str()))
                .collect::<Vec<_>>()
                .join(",")
        };
        let unmet = if self.unmet_dependencies.is_empty() {
            "none".to_string()
        } else {
            self.unmet_dependencies.join(",")
        };
        format!(
            "{} [{}] gates: {} unmet: {} title: {}",
            self.short_id,
            self.state.as_str(),
            gates,
            unmet,
            self.title
        )
    }
}

/// One `(state, count)` bucket in a [`StateRollup`].
///
/// Every [`State`] variant is represented, so `count` is `0` for a state with
/// no issues rather than the entry being omitted (see [`StateRollup`]).
///
/// # Examples
///
/// ```
/// use jit::domain::State;
/// use jit::output::StateCount;
///
/// let bucket = StateCount { state: State::Done, count: 3 };
/// let json = serde_json::to_value(&bucket).unwrap();
/// assert_eq!(json["state"], "done");
/// assert_eq!(json["count"], 3);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateCount {
    /// The lifecycle state this bucket counts.
    pub state: State,
    /// Number of issues in that state (`0` when the state is unpopulated).
    pub count: usize,
}

/// Counts-by-state aggregation with a terminal-state rollup over a set of
/// issues, shared by `jit issue progress` (over a container's direct children)
/// and `jit query count --by state` (over a label bucket or the whole repo).
///
/// `by_state` has one [`StateCount`] per [`State`] variant in [`State::all`]
/// order, zero-count states included, so the shape is stable and complete
/// (INV-DOMAIN-AGNOSTIC — the state list is enumerated, never hardcoded).
/// `count` is `by_state.len()` (the list-envelope count, one entry per state).
///
/// Terminal-state semantics, tied to [`State::is_terminal`] (`Done`/`Rejected`):
/// `done` and `rejected` are reported separately because a rejected issue is
/// terminal but not delivered; `open` is every non-terminal issue
/// (`total − done − rejected`, so `Archived` — which is not terminal — counts
/// as open). The `done`/`total` ratio and `percent` (rounded, `0` when `total`
/// is `0`) measure delivery, i.e. `done` against `total`.
///
/// # Examples
///
/// ```
/// use jit::domain::{Issue, State};
/// use jit::output::StateRollup;
///
/// let mut done = Issue::new("Shipped".into(), String::new());
/// done.state = State::Done;
/// let mut rejected = Issue::new("Dropped".into(), String::new());
/// rejected.state = State::Rejected;
/// let open = Issue::new("Todo".into(), String::new()); // Backlog: non-terminal
///
/// let rollup = StateRollup::from_issues(&[done, rejected, open]);
/// assert_eq!(rollup.total, 3);
/// assert_eq!(rollup.done, 1);
/// assert_eq!(rollup.rejected, 1);
/// assert_eq!(rollup.open, 1);
/// assert_eq!(rollup.percent, 33); // round(100 * 1 / 3)
/// assert_eq!(rollup.count, State::all().len());
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateRollup {
    /// Length of `by_state` (list-envelope count); equals the number of
    /// [`State`] variants.
    pub count: usize,
    /// One entry per state, zero-count states included, in [`State::all`] order.
    pub by_state: Vec<StateCount>,
    /// Total issues aggregated.
    pub total: usize,
    /// Issues in `Done`.
    pub done: usize,
    /// Issues in `Rejected` (terminal but not delivered).
    pub rejected: usize,
    /// Non-terminal issues (`total − done − rejected`).
    pub open: usize,
    /// `round(100 * done / total)`, or `0` when `total` is `0`.
    pub percent: u32,
}

impl StateRollup {
    /// Aggregate a slice of issues into the counts-by-state rollup.
    ///
    /// `by_state` is [`count_by_state`](crate::domain::queries::count_by_state)
    /// (every variant, zero-count states kept); `done`/`rejected`/`open` and
    /// `percent` follow the terminal-state semantics documented on the type.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Issue, State};
    /// use jit::output::StateRollup;
    ///
    /// let mut done = Issue::new("Shipped".into(), String::new());
    /// done.state = State::Done;
    /// let todo = Issue::new("Todo".into(), String::new()); // Backlog
    ///
    /// let rollup = StateRollup::from_issues(&[done, todo]);
    /// assert_eq!(rollup.total, 2);
    /// assert_eq!(rollup.done, 1);
    /// assert_eq!(rollup.open, 1); // the Backlog issue is non-terminal
    /// assert_eq!(rollup.percent, 50);
    /// // An empty slice is well-defined: total 0, percent 0, all buckets 0.
    /// let empty = StateRollup::from_issues(&[]);
    /// assert_eq!(empty.total, 0);
    /// assert_eq!(empty.percent, 0);
    /// assert!(empty.by_state.iter().all(|b| b.count == 0));
    /// ```
    pub fn from_issues(issues: &[Issue]) -> Self {
        let by_state: Vec<StateCount> = crate::domain::queries::count_by_state(issues)
            .into_iter()
            .map(|(state, count)| StateCount { state, count })
            .collect();

        let total = issues.len();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let rejected = issues.iter().filter(|i| i.state == State::Rejected).count();
        let open = total - done - rejected;
        let percent = if total == 0 {
            0
        } else {
            ((done as f64 / total as f64) * 100.0).round() as u32
        };

        Self {
            count: by_state.len(),
            by_state,
            total,
            done,
            rejected,
            open,
            percent,
        }
    }

    /// Render the two-line greppable text form:
    ///
    /// ```text
    /// by state: backlog=0 ready=1 in_progress=1 gated=0 done=2 rejected=1 archived=0
    /// done 2/5 (40%)  open 1  rejected 1
    /// ```
    ///
    /// The `by state:` line lists every state as `<state>=<count>` in canonical
    /// order (states enumerated from the domain, not hardcoded), so its columns
    /// are fixed regardless of which states are populated.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Issue, State};
    /// use jit::output::StateRollup;
    ///
    /// let mut done = Issue::new("Shipped".into(), String::new());
    /// done.state = State::Done;
    /// let lines = StateRollup::from_issues(&[done]).to_lines();
    ///
    /// assert_eq!(lines.len(), 2);
    /// assert!(lines[0].starts_with("by state: "));
    /// assert!(lines[0].contains("done=1"));
    /// assert_eq!(lines[1], "done 1/1 (100%)  open 0  rejected 0");
    /// ```
    pub fn to_lines(&self) -> Vec<String> {
        let by_state = self
            .by_state
            .iter()
            .map(|c| format!("{}={}", c.state.as_str(), c.count))
            .collect::<Vec<_>>()
            .join(" ");
        vec![
            format!("by state: {by_state}"),
            format!(
                "done {}/{} ({}%)  open {}  rejected {}",
                self.done, self.total, self.percent, self.open, self.rejected
            ),
        ]
    }
}

/// Compact container header — `{short_id, title, state}` — carried at the top of
/// `jit issue children` and `jit issue progress` JSON so a consumer sees which
/// container the child listing or rollup is for without a second lookup.
///
/// # Examples
///
/// ```
/// use jit::domain::{Issue, State};
/// use jit::output::ContainerHeader;
///
/// let mut epic = Issue::new("Auth epic".into(), String::new());
/// epic.state = State::InProgress;
/// let header = ContainerHeader::from(&epic);
///
/// assert_eq!(header.short_id, epic.id[..8]);
/// assert_eq!(header.title, "Auth epic");
/// assert_eq!(header.state, State::InProgress);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContainerHeader {
    /// Short ID (first 8 chars of the full UUID).
    pub short_id: String,
    /// The container's title.
    pub title: String,
    /// The container's own lifecycle state.
    pub state: State,
}

impl From<&Issue> for ContainerHeader {
    fn from(issue: &Issue) -> Self {
        Self {
            short_id: issue.short_id(),
            title: issue.title.clone(),
            state: issue.state,
        }
    }
}

/// Response for `jit issue children`: a container header plus one compact
/// [`IssueStatusResponse`] per resolvable direct child (its immediate
/// dependencies, depth 1), and the ids of any child edges that no longer
/// resolve.
///
/// `issues` and `count` form the standard `{count, issues}` list envelope
/// (`count == issues.len()`), with `container` added on top. `dangling` follows
/// the [`IssueShowResponse::dangling_dependency_ids`] precedent: a dependency id
/// that points at no stored issue is surfaced here rather than silently dropped,
/// and is omitted from the JSON when empty. Children are ordered by ascending
/// short id (stored dependency order is set-derived and not meaningful).
///
/// # Examples
///
/// ```
/// use jit::output::{ContainerHeader, IssueChildrenResponse};
/// use jit::domain::{Issue, State};
///
/// let container = ContainerHeader::from(&Issue::new("Epic".into(), String::new()));
/// let response = IssueChildrenResponse { container, count: 0, issues: vec![], dangling: vec![] };
/// let json = serde_json::to_value(&response).unwrap();
/// assert_eq!(json["count"], 0);
/// assert!(json["issues"].as_array().unwrap().is_empty());
/// // An empty `dangling` list is omitted from the JSON.
/// assert!(json.get("dangling").is_none());
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueChildrenResponse {
    /// `{short_id, title, state}` of the queried container.
    pub container: ContainerHeader,
    /// Number of resolvable direct children (equals `issues.len()`).
    pub count: usize,
    /// One compact status projection per resolvable direct child, ascending
    /// short id.
    pub issues: Vec<IssueStatusResponse>,
    /// Dependency ids that resolve to no stored issue (dangling edges), omitted
    /// when empty. `count`/`issues` cover resolvable children only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling: Vec<String>,
}

/// Response for `jit issue progress`: a container header, the counts-by-state
/// [`StateRollup`] over its resolvable direct children, and any dangling child
/// edges.
///
/// The rollup fields (`count`, `by_state`, `total`, `done`, `rejected`, `open`,
/// `percent`) are flattened to the top level, so the JSON is the `StateRollup`
/// shape plus `container` (and `dangling` when non-empty). `total` and every
/// count are over **resolvable** children only; a dependency id resolving to no
/// stored issue is surfaced in `dangling` (the [`IssueShowResponse`] precedent)
/// rather than counted or dropped.
///
/// # Examples
///
/// ```
/// use jit::output::{ContainerHeader, ContainerProgressResponse, StateRollup};
/// use jit::domain::{Issue, State};
///
/// let mut done = Issue::new("Child".into(), String::new());
/// done.state = State::Done;
/// let container = ContainerHeader::from(&Issue::new("Epic".into(), String::new()));
/// let response = ContainerProgressResponse {
///     container,
///     rollup: StateRollup::from_issues(&[done]),
///     dangling: vec![],
/// };
///
/// let json = serde_json::to_value(&response).unwrap();
/// // Rollup fields are flattened alongside `container`.
/// assert_eq!(json["total"], 1);
/// assert_eq!(json["done"], 1);
/// assert_eq!(json["percent"], 100);
/// assert_eq!(json["container"]["title"], "Epic");
/// assert!(json.get("dangling").is_none());
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContainerProgressResponse {
    /// `{short_id, title, state}` of the queried container.
    pub container: ContainerHeader,
    /// The counts-by-state rollup over resolvable direct children, flattened to
    /// the top level.
    #[serde(flatten)]
    pub rollup: StateRollup,
    /// Dependency ids that resolve to no stored issue (dangling edges), omitted
    /// when empty. The rollup counts resolvable children only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling: Vec<String>,
}

/// Render a single top-level field of a serialized issue value as plain text.
///
/// `value` must be the JSON object produced by serializing an
/// [`IssueShowResponse`]. The field is looked up by its serialized key (e.g.
/// `state`, `title`, `labels`). String fields render as their raw contents and
/// scalar fields (number/bool/null) as their textual value; array and object
/// fields fall back to compact JSON, since a plain-text rendering of those is
/// ambiguous. An unknown key returns `None` so callers can signal a usage error.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::output::{project_field, IssueShowResponse};
///
/// let issue = Issue::new("My Title".into(), "Body".into());
/// let value = serde_json::to_value(IssueShowResponse::from_issue(issue, vec![], &[])).unwrap();
///
/// // String field -> raw, unquoted.
/// assert_eq!(project_field(&value, "title").unwrap(), "My Title");
/// // Array field -> compact JSON.
/// assert_eq!(project_field(&value, "labels").unwrap(), "[]");
/// // Unknown field -> None.
/// assert!(project_field(&value, "bogus").is_none());
/// ```
pub fn project_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).map(|field| match field {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    })
}

/// Error returned when a requested projection field is not a known top-level
/// field of the serialized issue. Carries the offending field name.
///
/// # Examples
///
/// ```
/// use jit::output::UnknownFieldError;
///
/// let err = UnknownFieldError("bogus".into());
/// assert_eq!(err.to_string(), "unknown field 'bogus'");
/// assert_eq!(err.0, "bogus");
/// ```
#[derive(Debug, Error, PartialEq)]
#[error("unknown field '{0}'")]
pub struct UnknownFieldError(pub String);

/// Render the named top-level fields of a serialized issue value as a single
/// compact JSON object, preserving the requested order.
///
/// `value` must be the JSON object produced by serializing an
/// [`IssueShowResponse`]. Each name is looked up by its serialized key. The
/// first unknown key is returned as `Err(UnknownFieldError(name))` so callers
/// can signal a usage error; otherwise the result is a compact object
/// `{"a":...,"b":...}` whose keys keep the requested order (so the output is
/// stable regardless of the `serde_json` map-ordering feature).
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::output::{project_fields, IssueShowResponse, UnknownFieldError};
///
/// let issue = Issue::new("My Title".into(), "Body".into());
/// let value = serde_json::to_value(IssueShowResponse::from_issue(issue, vec![], &[])).unwrap();
///
/// let out = project_fields(&value, &["title".into(), "state".into()]).unwrap();
/// assert_eq!(out, r#"{"title":"My Title","state":"backlog"}"#);
///
/// // Unknown field reports its name.
/// assert_eq!(
///     project_fields(&value, &["bogus".into()]).unwrap_err(),
///     UnknownFieldError("bogus".into()),
/// );
/// ```
pub fn project_fields(value: &Value, names: &[String]) -> Result<String, UnknownFieldError> {
    let pairs = names
        .iter()
        .map(|name| {
            value
                .get(name)
                .map(|field| (name.as_str(), field))
                .ok_or_else(|| UnknownFieldError(name.clone()))
        })
        .collect::<Result<Vec<_>, UnknownFieldError>>()?;

    // Build the object by hand so the requested key order is preserved
    // regardless of whether `serde_json`'s `preserve_order` feature is enabled.
    // `Value::to_string` yields compact JSON for both the key and the value.
    let body = pairs
        .iter()
        .map(|(name, field)| format!("{}:{}", Value::String((*name).to_string()), field))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!("{{{body}}}"))
}

/// Render a resolved `jit config get` value as plain text.
///
/// A scalar (string/number/bool/null) renders bare, mirroring
/// [`project_field`]'s convention (a string's raw contents, unquoted, rather
/// than its `Display` which would include the JSON quotes). An array or
/// object — returned for an intermediate key, e.g. `jit config get
/// documentation` — has no sensible bare rendering, so it falls back to
/// PRETTY-printed JSON (unlike [`project_field`]'s compact fallback): a
/// config section is read by a human at a terminal far more often than
/// parsed by a script, and scripts should pass `--json` anyway.
///
/// # Examples
///
/// ```
/// use jit::output::render_config_get_value;
/// use serde_json::json;
///
/// assert_eq!(render_config_get_value(&json!("dev")), "dev");
/// assert_eq!(render_config_get_value(&json!(600)), "600");
/// assert_eq!(render_config_get_value(&json!(null)), "null");
/// assert_eq!(
///     render_config_get_value(&json!({"a": 1})),
///     "{\n  \"a\": 1\n}"
/// );
/// ```
pub fn render_config_get_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
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
            updated_at: issue.updated_at.to_rfc3339(),
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
            assignee: issue
                .assignee
                .as_ref()
                .map(crate::domain::Assignee::to_string),
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
///     findings: None,
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
    pub key: String,
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
    /// Structured findings parsed from the checker's machine-readable block, if
    /// one was emitted. Kept even in the lean form (which drops the raw
    /// stdout/stderr blob), so rework loops read verdict + findings as data
    /// without re-grepping a report that a passing lean summary omits entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<GateFindings>,
}

impl GateRunSummary {
    /// Build a summary that drops stdout/stderr for passing runs but keeps
    /// them for failed/error runs so diagnostics survive. Structured findings
    /// are retained regardless of status.
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
            key: r.gate_key.clone(),
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
            findings: r.findings.clone(),
        }
    }
}

/// Per-required-gate readiness entry in a [`GateCheckAllResponse`].
///
/// Reports one required gate's authoritative per-issue status (`passed`,
/// `failed`, or `pending`) so a `--json` consumer can distinguish a gate that
/// ran and failed from one that has never run/attested, both of which map to
/// the single nonzero exit code of `gate status-all`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateStatusEntry {
    pub key: String,
    pub status: GateStatus,
}

/// JSON payload of `jit gate status-all --json` (alias `gate check-all`).
///
/// `results` contains one [`GateRunSummary`] per recorded AUTOMATED run;
/// `gates` covers EVERY required gate (automated and manual) with its
/// readiness status, so manual gates are represented too. `total`, `passed`,
/// and `not_run` are computed over all required gates: `total` is their count,
/// `passed` how many are green, and `not_run` the keys still pending.
/// `all_passed` mirrors the strict exit contract (0 iff true, else 4).
///
/// List envelope: `count` is the length of the `gates` collection (one
/// entry per required gate). It equals `total` whenever every required gate has
/// a status entry; `total`/`passed` remain the readiness tallies.
///
/// # Examples
///
/// ```
/// use jit::output::GateCheckAllResponse;
///
/// let payload = GateCheckAllResponse {
///     count: 0,
///     results: vec![],
///     passed: 0,
///     total: 2,
///     not_run: vec!["tests".into(), "clippy".into()],
///     gates: vec![],
///     all_passed: false,
/// };
/// assert_eq!(payload.not_run.len(), 2);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateCheckAllResponse {
    /// Length of `gates` (list envelope `count`).
    pub count: usize,
    pub results: Vec<GateRunSummary>,
    pub passed: usize,
    pub total: usize,
    pub not_run: Vec<String>,
    pub gates: Vec<GateStatusEntry>,
    pub all_passed: bool,
}

/// JSON payload of `jit gate status --all` / `--limit` (the history view).
///
/// `results` holds one [`GateRunSummary`] per matching run, newest-first, after
/// any `--gate` / `--status` filtering and `--limit` capping. Each summary is the
/// full form (stdout/stderr included) so history inspection never loses report
/// text. `count` is `results.len()` (the number returned, after filtering).
///
/// # Examples
///
/// ```
/// use jit::output::GateRunHistoryResponse;
///
/// let payload = GateRunHistoryResponse {
///     results: vec![],
///     count: 0,
/// };
/// assert_eq!(payload.count, 0);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateRunHistoryResponse {
    pub results: Vec<GateRunSummary>,
    pub count: usize,
}

/// JSON payload of `jit gate status`'s flat report-text view
/// (`--stdout` / `--stderr` / `--tail`).
///
/// Carries the latest run's stored report text for the selected stream(s),
/// already tail-trimmed when `--tail <N>` was supplied. `stdout` / `stderr` are
/// present only for the streams the caller asked for, so the JSON mirrors the
/// plain verbatim output exactly.
///
/// # Examples
///
/// ```
/// use jit::output::GateFlatReportResponse;
///
/// let payload = GateFlatReportResponse {
///     key: "tests".into(),
///     run_id: "r1".into(),
///     stdout: Some("the report text".into()),
///     stderr: None,
/// };
/// assert_eq!(payload.stderr, None);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFlatReportResponse {
    pub key: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

/// JSON payload of `jit gate status <id> <gate> --findings`.
///
/// The structured-findings view of a single gate's latest run. `verdict`,
/// `summary`, and `findings` come straight from the checker's machine-readable
/// block ([`GateFindings`](crate::domain::GateFindings)); they are present only
/// when that run carried a block. `has_findings` is `false` for a plain-text
/// checker (no block emitted) so a consumer can distinguish "ran, no structured
/// findings" from "no findings recorded", without inspecting the optional
/// fields.
///
/// # Examples
///
/// ```
/// use jit::output::GateFindingsResponse;
///
/// let payload = GateFindingsResponse {
///     key: "code-review".into(),
///     run_id: "r1".into(),
///     has_findings: false,
///     verdict: None,
///     summary: None,
///     findings: vec![],
/// };
/// assert!(!payload.has_findings);
/// assert!(payload.findings.is_empty());
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFindingsResponse {
    pub key: String,
    pub run_id: String,
    /// Whether the latest run carried a machine-readable findings block.
    pub has_findings: bool,
    /// Checker-declared verdict; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// One-line run summary; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Individual findings; empty when no block was emitted.
    pub findings: Vec<crate::domain::GateFinding>,
}

// ============================================================================
// Gate Registry Response Types
// ============================================================================

/// Response for `gate list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateListResponse {
    pub gates: Vec<GateDefinition>,
    pub count: usize,
}

/// Gate definition structure (for registry responses)
///
/// Serialized as snake_case JSON; `stage` and `mode` are typed enums so the
/// serialized form is always well-formed and identical to the registry storage
/// format (`"precheck"` / `"postcheck"` / `"manual"` / `"auto"`).
///
/// # Examples
///
/// ```
/// use jit::output::GateDefinition;
/// use jit::domain::{GateMode, GateStage};
///
/// let def = GateDefinition {
///     key: "tests".to_string(),
///     title: "Tests".to_string(),
///     description: "Run test suite".to_string(),
///     auto: true,
///     example_integration: None,
///     stage: GateStage::Postcheck,
///     mode: GateMode::Auto,
/// };
/// let json = serde_json::to_value(&def).unwrap();
/// assert_eq!(json["stage"], "postcheck");
/// assert_eq!(json["mode"], "auto");
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateDefinition {
    pub key: String,
    pub title: String,
    pub description: String,
    pub auto: bool,
    pub example_integration: Option<String>,
    /// Execution stage; serializes as `"precheck"` or `"postcheck"`.
    pub stage: GateStage,
    /// Execution mode; serializes as `"manual"` or `"auto"`.
    pub mode: GateMode,
}

impl From<crate::domain::Gate> for GateDefinition {
    fn from(gate: crate::domain::Gate) -> Self {
        Self {
            key: gate.key,
            title: gate.title,
            description: gate.description,
            auto: gate.auto,
            example_integration: gate.example_integration,
            stage: gate.stage,
            mode: gate.mode,
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
    fn test_show_response_root_shape_short_id_and_arrays() {
        use crate::domain::Issue;
        // short_id is id[0..8]; labels and dependencies always serialize as arrays.
        let issue = Issue::new("T".to_string(), "B".to_string());
        let expected_short = issue.short_id();
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(v["short_id"], serde_json::Value::String(expected_short));
        assert!(v["labels"].is_array(), "labels must serialize as an array");
        assert!(
            v["dependencies"].is_array(),
            "dependencies must serialize as an array"
        );
        assert_eq!(v["labels"].as_array().unwrap().len(), 0);
        assert_eq!(v["dependencies"].as_array().unwrap().len(), 0);
        // A clean issue has no dangling deps, so the field is omitted from
        // the JSON entirely (`skip_serializing_if = "Vec::is_empty"`).
        assert!(v.get("dangling_dependency_ids").is_none());
        // Unset lifecycle timestamps are omitted from the show shape.
        assert!(v.get("first_ready_at").is_none());
        assert!(v.get("claimed_at").is_none());
        assert!(v.get("done_at").is_none());
    }

    #[test]
    fn test_show_response_surfaces_lifecycle_timestamps() {
        use crate::domain::Issue;
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        let at = chrono::Utc::now();
        issue.mark_first_ready(at);
        issue.mark_claimed(at);
        issue.mark_done(at);

        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(
            v["first_ready_at"],
            serde_json::Value::String(at.to_rfc3339())
        );
        assert_eq!(v["claimed_at"], serde_json::Value::String(at.to_rfc3339()));
        assert_eq!(v["done_at"], serde_json::Value::String(at.to_rfc3339()));
    }

    #[test]
    fn test_show_response_surfaces_dangling_dependency_ids_not_silently() {
        // jit:f847df3f: a dependency id that never resolved to a MinimalIssue
        // (constructed directly here via storage-level state, standing in for
        // a dangling reference left by raw storage mutation or legacy data)
        // must surface in `dangling_dependency_ids` rather than vanish when
        // `enriched_deps` only contains the resolvable ones.
        use crate::domain::Issue;

        let mut issue = Issue::new("Parent".to_string(), "Body".to_string());
        issue.dependencies = vec!["resolved-id".to_string(), "dangling-id".to_string()];

        // Simulate what `get_dependencies_enriched` would produce: only the
        // resolvable dependency's MinimalIssue, the dangling one absent.
        let mut resolved_dep = Issue::new("Dep".to_string(), "".to_string());
        resolved_dep.id = "resolved-id".to_string();
        let enriched_deps = vec![MinimalIssue::from(&resolved_dep)];

        let resp = IssueShowResponse::from_issue(issue, enriched_deps, &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(
            v["dependencies"].as_array().unwrap().len(),
            1,
            "only the resolvable dependency appears in the enriched list"
        );
        assert_eq!(
            v["dangling_dependency_ids"],
            serde_json::json!(["dangling-id"]),
            "the dangling id must be reported, not silently dropped"
        );
    }

    #[test]
    fn test_show_response_exposes_content_format() {
        use crate::domain::{ContentFormat, Issue};
        // Set -> appears in `jit issue show --json` (create/show parity).
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        issue.content_format = Some(ContentFormat::Html);
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["content_format"], "html");

        // Absent -> omitted (existing issues without the field stay clean).
        let issue2 = Issue::new("T".to_string(), "B".to_string());
        let resp2 = IssueShowResponse::from_issue(issue2, vec![], &[]);
        let v2 = serde_json::to_value(&resp2).unwrap();
        assert!(v2.get("content_format").is_none());
    }

    #[test]
    fn test_show_response_gates_array_replaces_split_fields() {
        use crate::domain::Issue;
        // `gates` replaces `gates_required`/`gates_status`; neither legacy field
        // appears in the `issue show --json` shape.
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert!(v["gates"].is_array(), "gates must serialize as an array");
        assert!(
            v.get("gates_required").is_none(),
            "gates_required must be gone"
        );
        assert!(v.get("gates_status").is_none(), "gates_status must be gone");
    }

    #[test]
    fn test_show_response_gate_never_run_is_null() {
        use crate::domain::Issue;
        // Required-but-never-run gate: status pending, last_run_at/exit_code null.
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        let gate = &v["gates"][0];
        assert_eq!(gate["key"], "tests");
        assert_eq!(gate["status"], "pending");
        assert!(gate["last_run_at"].is_null(), "last_run_at must be null");
        assert!(gate["exit_code"].is_null(), "exit_code must be null");
    }

    #[test]
    fn test_show_response_gate_with_run_is_enriched() {
        use crate::domain::{
            GateRunResult, GateRunStatus, GateStage, GateState, GateStatus, Issue,
        };
        use chrono::Utc;

        // A gate that has run: status from GateState, last_run_at/exit_code from
        // the latest matching GateRunResult.
        let mut issue = Issue::new("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
        let run_at = Utc::now();
        issue.gates_status.insert(
            "tests".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("ci:test".parse().unwrap()),
                updated_at: run_at,
            },
        );

        let run = GateRunResult {
            schema_version: 1,
            run_id: "run-1".to_string(),
            gate_key: "tests".to_string(),
            stage: GateStage::Postcheck,
            issue_id: issue.id.clone(),
            commit: None,
            branch: None,
            status: GateRunStatus::Passed,
            started_at: run_at,
            completed_at: Some(run_at),
            duration_ms: Some(42),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            command: "cargo test".to_string(),
            by: None,
            message: None,
            findings: None,
        };

        let resp = IssueShowResponse::from_issue(issue, vec![], std::slice::from_ref(&run));
        let v = serde_json::to_value(&resp).unwrap();

        let gate = &v["gates"][0];
        assert_eq!(gate["key"], "tests");
        assert_eq!(gate["status"], "passed");
        assert_eq!(gate["exit_code"], 0);
        assert_eq!(gate["last_run_at"], run_at.to_rfc3339());
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

    #[test]
    fn test_gate_definition_serializes_stage_and_mode_as_snake_case() {
        use crate::domain::{GateMode, GateStage};
        // --json gate output must remain snake_case regardless of enum Debug repr.
        let def = GateDefinition {
            key: "ci".to_string(),
            title: "CI".to_string(),
            description: "Continuous integration".to_string(),
            auto: true,
            example_integration: None,
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
        };
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck", "stage must be snake_case");
        assert_eq!(v["mode"], "auto", "mode must be snake_case");

        let def_pre = GateDefinition {
            key: "lint".to_string(),
            title: "Lint".to_string(),
            description: "Linting".to_string(),
            auto: false,
            example_integration: None,
            stage: GateStage::Precheck,
            mode: GateMode::Manual,
        };
        let v2 = serde_json::to_value(&def_pre).unwrap();
        assert_eq!(v2["stage"], "precheck");
        assert_eq!(v2["mode"], "manual");
    }

    #[test]
    fn test_gate_definition_from_gate_round_trips_stage_mode() {
        use crate::domain::{Gate, GateMode, GateStage};
        let gate = Gate {
            version: 1,
            key: "tests".to_string(),
            title: "Tests".to_string(),
            description: "Run tests".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: true,
            example_integration: None,
        };
        let def = GateDefinition::from(gate);
        assert_eq!(def.stage, GateStage::Postcheck);
        assert_eq!(def.mode, GateMode::Auto);
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck");
        assert_eq!(v["mode"], "auto");
    }
}
