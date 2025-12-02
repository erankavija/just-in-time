//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use chrono::Utc;
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::domain::{Issue, Priority, State};

/// Version of the JSON output format
const OUTPUT_VERSION: &str = "0.2.0";

/// Wrapper for successful command output with metadata
#[derive(Debug, Serialize)]
pub struct JsonOutput<T: Serialize> {
    pub success: bool,
    pub data: T,
    pub metadata: Metadata,
}

impl<T: Serialize> JsonOutput<T> {
    /// Create a new successful output with the given data
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data,
            metadata: Metadata::new(),
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
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
            metadata: Metadata::new(),
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
            ExitCode::ValidationFailed => "Validation failed (cycle detected, broken references, etc.)",
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
            ExitCode::Success.code(), ExitCode::Success.description(),
            ExitCode::GenericError.code(), ExitCode::GenericError.description(),
            ExitCode::InvalidArgument.code(), ExitCode::InvalidArgument.description(),
            ExitCode::NotFound.code(), ExitCode::NotFound.description(),
            ExitCode::ValidationFailed.code(), ExitCode::ValidationFailed.description(),
            ExitCode::PermissionDenied.code(), ExitCode::PermissionDenied.description(),
            ExitCode::AlreadyExists.code(), ExitCode::AlreadyExists.description(),
            ExitCode::ExternalError.code(), ExitCode::ExternalError.description(),
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
    pub fn issue_not_found(issue_id: &str) -> Self {
        Self::new(
            ErrorCode::ISSUE_NOT_FOUND,
            format!("Issue not found: {}", issue_id),
        )
        .with_details(serde_json::json!({"issue_id": issue_id}))
        .with_suggestion("Run 'jit issue list' to see available issues")
        .with_suggestion("Check if the issue ID is correct")
    }

    pub fn gate_not_found(gate_key: &str) -> Self {
        Self::new(
            ErrorCode::GATE_NOT_FOUND,
            format!("Gate not found: {}", gate_key),
        )
        .with_details(serde_json::json!({"gate_key": gate_key}))
        .with_suggestion("Run 'jit registry list' to see available gates")
        .with_suggestion("Add the gate to the registry first with 'jit registry add'")
    }

    pub fn cycle_detected(from: &str, to: &str) -> Self {
        Self::new(
            ErrorCode::CYCLE_DETECTED,
            format!("Adding dependency would create a cycle: {} -> {}", from, to),
        )
        .with_details(serde_json::json!({"from": from, "to": to}))
        .with_suggestion("Remove existing dependencies that create the cycle")
        .with_suggestion("Use 'jit graph show' to visualize the dependency graph")
    }

    pub fn invalid_state(state: &str) -> Self {
        Self::new(
            ErrorCode::INVALID_STATE,
            format!("Invalid state: {}", state),
        )
        .with_details(serde_json::json!({"invalid_state": state}))
        .with_suggestion("Valid states are: open, ready, in_progress, done")
    }

    pub fn invalid_priority(priority: &str) -> Self {
        Self::new(
            ErrorCode::INVALID_ARGUMENT,
            format!("Invalid priority: {}", priority),
        )
        .with_details(serde_json::json!({"invalid_priority": priority}))
        .with_suggestion("Valid priorities are: low, normal, high, critical")
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
}

impl Metadata {
    fn new() -> Self {
        Self {
            timestamp: Utc::now(),
            version: OUTPUT_VERSION.to_string(),
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

/// Response for `query ready` command
#[derive(Debug, Serialize)]
pub struct ReadyQueryResponse {
    pub issues: Vec<Issue>,
    pub count: usize,
}

/// Response for `query blocked` command
#[derive(Debug, Serialize)]
pub struct BlockedQueryResponse {
    pub issues: Vec<BlockedIssue>,
    pub count: usize,
}

/// Issue with blocking reasons
#[derive(Debug, Serialize)]
pub struct BlockedIssue {
    #[serde(flatten)]
    pub issue: Issue,
    pub blocked_reasons: Vec<BlockedReason>,
}

/// Reason why an issue is blocked
#[derive(Debug, Serialize)]
pub struct BlockedReason {
    #[serde(rename = "type")]
    pub reason_type: BlockedReasonType,
    pub detail: String,
}

/// Type of blocking reason
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockedReasonType {
    Dependency,
    Gate,
}

/// Response for `query assignee` command
#[derive(Debug, Serialize)]
pub struct AssigneeQueryResponse {
    pub assignee: String,
    pub issues: Vec<Issue>,
    pub count: usize,
}

/// Response for `query state` command
#[derive(Debug, Serialize)]
pub struct StateQueryResponse {
    pub state: State,
    pub issues: Vec<Issue>,
    pub count: usize,
}

/// Response for `query priority` command
#[derive(Debug, Serialize)]
pub struct PriorityQueryResponse {
    pub priority: Priority,
    pub issues: Vec<Issue>,
    pub count: usize,
}

// ============================================================================
// Graph Response Types
// ============================================================================

/// Response for `graph show <id>` command
#[derive(Debug, Serialize)]
pub struct GraphShowResponse {
    pub issue_id: String,
    pub dependencies: Vec<Issue>,
    pub count: usize,
}

/// Response for `graph show` (all) command
#[derive(Debug, Serialize)]
pub struct GraphShowAllResponse {
    pub dependencies: Vec<DependencyPair>,
    pub count: usize,
}

/// A pair of issues with a dependency relationship
#[derive(Debug, Serialize)]
pub struct DependencyPair {
    pub from_id: String,
    pub from_title: String,
    pub to_id: String,
    pub to_title: String,
}

/// Response for `graph downstream` command
#[derive(Debug, Serialize)]
pub struct GraphDownstreamResponse {
    pub issue_id: String,
    pub dependents: Vec<Issue>,
    pub count: usize,
}

/// Response for `graph roots` command
#[derive(Debug, Serialize)]
pub struct GraphRootsResponse {
    pub roots: Vec<Issue>,
    pub count: usize,
}

// ============================================================================
// Registry Response Types
// ============================================================================

/// Response for `registry list` command
#[derive(Debug, Serialize)]
pub struct RegistryListResponse {
    pub gates: Vec<GateDefinition>,
    pub count: usize,
}

/// Gate definition structure (for registry responses)
#[derive(Debug, Serialize)]
pub struct GateDefinition {
    pub key: String,
    pub title: String,
    pub description: String,
    pub auto: bool,
    pub example_integration: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_output_success() {
        let data = json!({"id": "123", "title": "Test"});
        let output = JsonOutput::success(data);

        assert!(output.success);
        assert_eq!(output.data["id"], "123");
        assert_eq!(output.metadata.version, "0.2.0");
    }

    #[test]
    fn test_json_output_serialization() {
        let data = json!({"id": "123"});
        let output = JsonOutput::success(data);

        let json_str = output.to_json_string().unwrap();
        assert!(json_str.contains("\"success\": true"));
        assert!(json_str.contains("\"id\": \"123\""));
        assert!(json_str.contains("\"version\": \"0.2.0\""));
        assert!(json_str.contains("\"timestamp\":"));
    }

    #[test]
    fn test_json_error_basic() {
        let error = JsonError::new("TEST_ERROR", "This is a test error");

        assert!(!error.success);
        assert_eq!(error.error.code, "TEST_ERROR");
        assert_eq!(error.error.message, "This is a test error");
        assert!(error.error.details.is_none());
        assert!(error.error.suggestions.is_empty());
    }

    #[test]
    fn test_json_error_with_details() {
        let error = JsonError::new("NOT_FOUND", "Resource not found")
            .with_details(json!({"requested_id": "abc123"}));

        assert_eq!(error.error.details, Some(json!({"requested_id": "abc123"})));
    }

    #[test]
    fn test_json_error_with_suggestions() {
        let error = JsonError::new("NOT_FOUND", "Issue not found")
            .with_suggestion("Run 'jit issue list' to see available issues")
            .with_suggestion("Check if the issue ID is correct");

        assert_eq!(error.error.suggestions.len(), 2);
        assert!(error.error.suggestions[0].contains("jit issue list"));
    }

    #[test]
    fn test_json_error_serialization() {
        let error = JsonError::new("TEST_ERROR", "Test")
            .with_details(json!({"key": "value"}))
            .with_suggestion("Try something");

        let json_str = error.to_json_string().unwrap();
        assert!(json_str.contains("\"success\": false"));
        assert!(json_str.contains("\"code\": \"TEST_ERROR\""));
        assert!(json_str.contains("\"message\": \"Test\""));
        assert!(json_str.contains("\"details\""));
        assert!(json_str.contains("\"suggestions\""));
    }

    #[test]
    fn test_metadata_includes_timestamp() {
        let metadata = Metadata::new();
        assert_eq!(metadata.version, "0.2.0");
        // Timestamp should be recent (within last 5 seconds)
        let now = Utc::now();
        let diff = now.signed_duration_since(metadata.timestamp);
        assert!(diff.num_seconds() < 5);
    }

    // ========================================================================
    // Query Response Tests
    // ========================================================================

    #[test]
    fn test_ready_query_response_serialization() {
        let issues = vec![Issue::new("Issue 1".to_string(), "".to_string())];
        let response = ReadyQueryResponse {
            issues: issues.clone(),
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"success\": true"));
        assert!(serialized.contains("\"count\": 1"));
        assert!(serialized.contains("\"metadata\""));
    }

    #[test]
    fn test_blocked_query_response_serialization() {
        let issue = Issue::new("Issue 1".to_string(), "".to_string());
        let reasons = vec![BlockedReason {
            reason_type: BlockedReasonType::Dependency,
            detail: "Waiting on ABC123".to_string(),
        }];

        let blocked_issue = BlockedIssue {
            issue,
            blocked_reasons: reasons,
        };

        let response = BlockedQueryResponse {
            issues: vec![blocked_issue],
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"success\": true"));
        assert!(serialized.contains("\"blocked_reasons\""));
        assert!(serialized.contains("\"dependency\""));
    }

    #[test]
    fn test_assignee_query_response() {
        let issues = vec![Issue::new("Issue 1".to_string(), "".to_string())];

        let response = AssigneeQueryResponse {
            assignee: "copilot:session-1".to_string(),
            issues: issues.clone(),
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"assignee\""));
        assert!(serialized.contains("copilot:session-1"));
    }

    #[test]
    fn test_state_query_response() {
        let issues = vec![Issue::new("Issue 1".to_string(), "".to_string())];

        let response = StateQueryResponse {
            state: State::Ready,
            issues: issues.clone(),
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"state\""));
        assert!(serialized.contains("\"ready\""));
    }

    #[test]
    fn test_priority_query_response() {
        let issues = vec![Issue::new("Issue 1".to_string(), "".to_string())];

        let response = PriorityQueryResponse {
            priority: Priority::High,
            issues: issues.clone(),
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"priority\""));
        assert!(serialized.contains("\"high\""));
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
