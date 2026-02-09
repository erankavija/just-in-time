//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::fmt::Display;
use std::io::{self, Write};

use crate::domain::{MinimalBlockedIssue, MinimalIssue, Priority, State};

/// Version of the JSON output format
const OUTPUT_VERSION: &str = "0.2.0";

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

/// Wrapper for successful command output with metadata
#[derive(Debug, Serialize)]
pub struct JsonOutput<T: Serialize> {
    pub success: bool,
    pub data: T,
    pub metadata: Metadata,
}

impl<T: Serialize> JsonOutput<T> {
    /// Create a new successful output with the given data
    pub fn success(data: T, command: impl Into<String>) -> Self {
        Self {
            success: true,
            data,
            metadata: Metadata::new(command),
        }
    }

    /// Serialize to JSON string with pretty formatting
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Wrapper for error output with suggestions
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub success: bool,
    pub error: ErrorDetail,
    pub metadata: Metadata,
}

#[allow(dead_code)]
impl JsonError {
    /// Create a new error output
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            success: false,
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
            metadata: Metadata::new(command),
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
    pub const IO_ERROR: &'static str = "IO_ERROR";
    pub const PARSE_ERROR: &'static str = "PARSE_ERROR";
}

impl ErrorCode {
    /// Map error code string to exit code
    pub fn to_exit_code(code: &str) -> ExitCode {
        match code {
            Self::ISSUE_NOT_FOUND | Self::GATE_NOT_FOUND => ExitCode::NotFound,
            Self::CYCLE_DETECTED | Self::VALIDATION_FAILED => ExitCode::ValidationFailed,
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
        .with_suggestion("Run 'jit issue list' to see available issues")
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
}

/// Metadata included in all responses
#[derive(Debug, Serialize)]
pub struct Metadata {
    /// Timestamp when the response was generated
    #[serde(serialize_with = "serialize_timestamp")]
    pub timestamp: chrono::DateTime<Utc>,
    /// Version of the output format
    pub version: String,
    /// Command that generated this response
    pub command: String,
}

impl Metadata {
    fn new(command: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            version: OUTPUT_VERSION.to_string(),
            command: command.into(),
        }
    }
}

/// Serialize timestamp in ISO 8601 format
fn serialize_timestamp<S>(dt: &chrono::DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&dt.to_rfc3339())
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

/// Response for `graph show <id>` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphShowResponse {
    pub issue_id: String,
    pub dependencies: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `graph show` (all) command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphShowAllResponse {
    pub dependencies: Vec<DependencyPair>,
    pub count: usize,
}

/// A pair of issues with a dependency relationship
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyPair {
    pub from_id: String,
    pub from_title: String,
    pub to_id: String,
    pub to_title: String,
}

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
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_output_success() {
        let data = json!({"id": "123", "title": "Test"});
        let output = JsonOutput::success(data, "issue show");

        assert!(output.success);
        assert_eq!(output.data["id"], "123");
        assert_eq!(output.metadata.version, "0.2.0");
        assert_eq!(output.metadata.command, "issue show");
    }

    #[test]
    fn test_json_output_serialization() {
        let data = json!({"id": "123"});
        let output = JsonOutput::success(data, "issue list");

        let json_str = output.to_json_string().unwrap();
        assert!(json_str.contains("\"success\": true"));
        assert!(json_str.contains("\"id\": \"123\""));
        assert!(json_str.contains("\"version\": \"0.2.0\""));
        assert!(json_str.contains("\"timestamp\":"));
        assert!(json_str.contains("\"command\": \"issue list\""));
    }

    #[test]
    fn test_json_error_basic() {
        let error = JsonError::new("TEST_ERROR", "This is a test error", "test command");

        assert!(!error.success);
        assert_eq!(error.error.code, "TEST_ERROR");
        assert_eq!(error.error.message, "This is a test error");
        assert_eq!(error.metadata.command, "test command");
        assert!(error.error.details.is_none());
        assert!(error.error.suggestions.is_empty());
    }

    #[test]
    fn test_json_error_with_details() {
        let error = JsonError::new("NOT_FOUND", "Resource not found", "show resource")
            .with_details(json!({"requested_id": "abc123"}));

        assert_eq!(error.error.details, Some(json!({"requested_id": "abc123"})));
        assert_eq!(error.metadata.command, "show resource");
    }

    #[test]
    fn test_json_error_with_suggestions() {
        let error = JsonError::new("NOT_FOUND", "Issue not found", "issue show")
            .with_suggestion("Run 'jit issue list' to see available issues")
            .with_suggestion("Check if the issue ID is correct");

        assert_eq!(error.error.suggestions.len(), 2);
        assert!(error.error.suggestions[0].contains("jit issue list"));
        assert_eq!(error.metadata.command, "issue show");
    }

    #[test]
    fn test_json_error_serialization() {
        let error = JsonError::new("TEST_ERROR", "Test", "test")
            .with_details(json!({"key": "value"}))
            .with_suggestion("Try something");

        let json_str = error.to_json_string().unwrap();
        assert!(json_str.contains("\"success\": false"));
        assert!(json_str.contains("\"code\": \"TEST_ERROR\""));
        assert!(json_str.contains("\"message\": \"Test\""));
        assert!(json_str.contains("\"details\""));
        assert!(json_str.contains("\"suggestions\""));
        assert!(json_str.contains("\"command\": \"test\""));
    }

    #[test]
    fn test_metadata_includes_timestamp() {
        let metadata = Metadata::new("test command");
        assert_eq!(metadata.version, "0.2.0");
        assert_eq!(metadata.command, "test command");
        // Timestamp should be recent (within last 5 seconds)
        let now = Utc::now();
        let diff = now.signed_duration_since(metadata.timestamp);
        assert!(diff.num_seconds() < 5);
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

        assert!(serialized.contains("\"success\": true"));
        assert!(serialized.contains("\"count\": 1"));
        assert!(serialized.contains("\"metadata\""));
        assert!(serialized.contains("\"command\": \"query ready\""));
    }

    #[test]
    fn test_blocked_query_response_serialization() {
        let blocked_issue = MinimalBlockedIssue {
            id: "test-id".to_string(),
            title: "Issue 1".to_string(),
            state: State::Ready,
            priority: Priority::Normal,
            blocked_reasons: vec!["dep:abc123".to_string()],
        };

        let response = BlockedQueryResponse {
            issues: vec![blocked_issue],
            count: 1,
        };

        let json_output = JsonOutput::success(response, "query blocked");
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"success\": true"));
        assert!(serialized.contains("\"blocked_reasons\""));
        assert!(serialized.contains("\"command\": \"query blocked\""));
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
        assert!(serialized.contains("\"command\": \"query assignee\""));
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
        assert!(serialized.contains("\"command\": \"query state\""));
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
        assert!(serialized.contains("\"command\": \"query priority\""));
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
