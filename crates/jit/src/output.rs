//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use chrono::Utc;
use serde::{Serialize, Serializer};
use serde_json::Value;

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
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub success: bool,
    pub error: ErrorDetail,
    pub metadata: Metadata,
}

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
}

/// Error details including code, message, and suggestions
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
}
