//! A regex that extracts an id from text, compiled once from its authored source.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Error raised when an id-pattern source is not a valid regex.
#[derive(Debug, Error, Clone, PartialEq)]
#[error("invalid id pattern '{pattern}': {source}")]
pub struct InvalidIdPatternError {
    pattern: String,
    source: regex::Error,
}

/// A compiled id pattern. Equality and serialization use the authored source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IdPattern {
    source: String,
    regex: Regex,
}

impl IdPattern {
    /// Compile `source`, failing on an invalid regex.
    pub fn new(source: impl Into<String>) -> Result<Self, InvalidIdPatternError> {
        let source = source.into();
        match Regex::new(&source) {
            Ok(regex) => Ok(Self { source, regex }),
            Err(error) => Err(InvalidIdPatternError {
                pattern: source,
                source: error,
            }),
        }
    }

    /// The authored source.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// The compiled regex.
    pub fn regex(&self) -> &Regex {
        &self.regex
    }
}

impl PartialEq for IdPattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for IdPattern {}

impl TryFrom<String> for IdPattern {
    type Error = InvalidIdPatternError;

    fn try_from(source: String) -> Result<Self, Self::Error> {
        Self::new(source)
    }
}

impl From<IdPattern> for String {
    fn from(pattern: IdPattern) -> Self {
        pattern.source
    }
}

impl fmt::Display for IdPattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_pattern_new_rejects_invalid_source_naming_it() {
        let error = IdPattern::new("REQ-(").unwrap_err();
        assert!(error.to_string().contains("REQ-("), "error was: {error}");
    }

    #[test]
    fn test_id_pattern_eq_compares_sources_not_matching_behaviour() {
        let a = IdPattern::new("[0-9]").unwrap();
        assert_eq!(a, IdPattern::new("[0-9]").unwrap());
        // `\d` matches what `[0-9]` matches on ASCII, but the sources differ.
        assert_ne!(a, IdPattern::new(r"\d").unwrap());
    }

    #[test]
    fn test_id_pattern_serde_round_trips_through_source_string() {
        let pattern = IdPattern::new("REQ-[0-9]+").unwrap();
        let json = serde_json::to_value(&pattern).unwrap();
        assert_eq!(
            json,
            serde_json::Value::String(pattern.as_str().to_string())
        );
        assert_eq!(serde_json::from_value::<IdPattern>(json).unwrap(), pattern);
        assert!(serde_json::from_value::<IdPattern>(serde_json::json!("REQ-(")).is_err());
    }

    #[test]
    fn test_id_pattern_regex_matches_compiled_source() {
        let pattern = IdPattern::new("REQ-[0-9]+").unwrap();
        assert_eq!(
            pattern.regex().find("see REQ-12 here").map(|m| m.as_str()),
            Some("REQ-12")
        );
    }
}
