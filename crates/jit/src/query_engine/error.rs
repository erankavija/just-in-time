//! Typed parse errors for the query filter language.
//!
//! Both the lexer and parser surface failures through [`QueryParseError`] so a
//! malformed query is reported with a precise, typed reason instead of silently
//! matching nothing. In particular, an unknown filter field or an invalid
//! `state:`/`priority:` value is a hard parse error, not a fail-open no-match.

/// The canonical lifecycle state tokens accepted in a `state:` filter, listed
/// in an [`QueryParseError::InvalidState`] message.
pub(super) fn state_values() -> String {
    crate::domain::State::ALL.map(|s| s.as_str()).join(", ")
}

/// The set of priority tokens accepted in a `priority:` filter, listed in an
/// [`QueryParseError::InvalidPriority`] message.
pub(super) fn priority_values() -> String {
    crate::domain::Priority::ALL.map(|p| p.as_str()).join(", ")
}

/// A typed failure produced while lexing or parsing a query string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryParseError {
    /// A bare word did not contain the `field:value` separator.
    #[error("Invalid filter '{0}': expected format 'field:value'")]
    MissingColon(String),

    /// The field portion of a `field:value` filter was empty.
    #[error("Filter field cannot be empty: '{0}'")]
    EmptyField(String),

    /// The value portion of a `field:value` filter was empty.
    #[error("Filter value cannot be empty: '{0}'")]
    EmptyValue(String),

    /// A non-tokenizable character was encountered at the given byte position.
    #[error("Unexpected character at position {0}")]
    UnexpectedChar(usize),

    /// The filter field name is not one of the known fields.
    #[error("Unknown filter field: '{0}' (expected state, label, priority, or assignee)")]
    UnknownField(String),

    /// A `state:` filter named a value that is not a valid lifecycle state.
    #[error("Invalid state value '{value}' in filter (expected one of: {valid})")]
    InvalidState {
        /// The offending value as authored.
        value: String,
        /// Comma-separated list of accepted state tokens.
        valid: String,
    },

    /// A `priority:` filter named a value that is not a valid priority.
    #[error("Invalid priority value '{value}' in filter (expected one of: {valid})")]
    InvalidPriority {
        /// The offending value as authored.
        value: String,
        /// Comma-separated list of accepted priority tokens.
        valid: String,
    },

    /// An `assignee:` filter named a value that is not a `type:identifier` assignee.
    #[error("Invalid assignee value '{value}' in filter: {reason}")]
    InvalidAssignee {
        /// The offending value as authored.
        value: String,
        /// Why the value is not a valid assignee.
        reason: crate::domain::AssigneeParseError,
    },

    /// A parenthesized group was opened but never closed.
    #[error("Expected closing parenthesis")]
    UnclosedParen,

    /// The token stream ended where a condition was expected.
    #[error("Unexpected end of query")]
    UnexpectedEnd,

    /// A token appeared where a condition was expected.
    #[error("Expected condition, found {0}")]
    ExpectedCondition(String),
}
