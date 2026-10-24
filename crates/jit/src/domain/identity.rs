//! Validated identity types. Each holds a private canonical value, parses
//! through `TryFrom<&str>`/`FromStr`, and serializes as a plain string.

use super::SHORT_ID_LENGTH;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// Temporary bridge: issue ids stay `String` until the flip gives
/// [`ValidatedIssueId`] this name (`@/inv/canonical-cutover`).
pub type IssueId = String;

/// Temporary bridge giving the [`IssueId`] alias the newtype's `short_id()`
/// (`@/inv/canonical-cutover`).
pub trait IssueIdStaging {
    /// The first [`SHORT_ID_LENGTH`] characters of the id.
    fn short_id(&self) -> &str;
}

impl IssueIdStaging for String {
    fn short_id(&self) -> &str {
        short_id_of(self)
    }
}

fn short_id_of(id: &str) -> &str {
    id.get(..SHORT_ID_LENGTH).unwrap_or(id)
}

/// Error raised when an issue id is not a canonical lowercase hyphenated UUID.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid issue id '{input}'; expected a lowercase hyphenated UUID")]
pub struct InvalidIssueIdError {
    input: String,
}

/// An issue id in the canonical lowercase hyphenated UUID form.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
// The empty description keeps this rustdoc out of the published schema.
#[schemars(with = "String", description = "")]
pub struct ValidatedIssueId(String);

impl ValidatedIssueId {
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first [`SHORT_ID_LENGTH`] characters of the id.
    pub fn short_id(&self) -> &str {
        short_id_of(&self.0)
    }
}

impl TryFrom<&str> for ValidatedIssueId {
    type Error = InvalidIssueIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        Uuid::parse_str(input)
            .ok()
            .filter(|uuid| uuid.to_string() == input)
            .map(|_| Self(input.to_string()))
            .ok_or_else(|| InvalidIssueIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for ValidatedIssueId {
    type Error = InvalidIssueIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for ValidatedIssueId {
    type Err = InvalidIssueIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<ValidatedIssueId> for String {
    fn from(id: ValidatedIssueId) -> Self {
        id.0
    }
}

impl fmt::Display for ValidatedIssueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{json, Value};

    fn canonical_id() -> impl Strategy<Value = String> {
        any::<u128>().prop_map(|bits| Uuid::from_u128(bits).to_string())
    }

    #[test]
    fn test_try_from_rejects_empty_input() {
        assert!(ValidatedIssueId::try_from("").is_err());
    }

    #[test]
    fn test_json_schema_is_a_plain_string() {
        let schema = ValidatedIssueId::json_schema(&mut schemars::gen::SchemaGenerator::default());
        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            json!({"type": "string"})
        );
    }

    proptest! {
        /// A canonical id survives parsing and serde as the same text, and
        /// both string views print it.
        #[test]
        fn test_try_from_round_trips_a_canonical_id(id in canonical_id()) {
            let parsed = ValidatedIssueId::try_from(id.as_str()).unwrap();
            prop_assert_eq!(parsed.as_str(), id.as_str());
            prop_assert_eq!(parsed.to_string(), id.clone());
            let encoded = serde_json::to_value(&parsed).unwrap();
            prop_assert_eq!(&encoded, &Value::String(id.clone()));
            prop_assert_eq!(serde_json::from_value::<ValidatedIssueId>(encoded).unwrap(), parsed);
        }

        /// Every other spelling of the same UUID is refused by parsing and
        /// by deserialization, and the error names the input.
        #[test]
        fn test_try_from_rejects_each_non_canonical_spelling(bits in any::<u128>()) {
            let uuid = Uuid::from_u128(bits);
            let canonical = uuid.to_string();
            let spellings = [
                uuid.simple().to_string(),
                uuid.braced().to_string(),
                uuid.urn().to_string(),
                canonical.to_uppercase(),
            ];
            for spelling in spellings.iter().filter(|spelling| **spelling != canonical) {
                let error = ValidatedIssueId::try_from(spelling.as_str()).unwrap_err();
                prop_assert!(error.to_string().contains(spelling.as_str()));
                prop_assert!(
                    serde_json::from_value::<ValidatedIssueId>(json!(spelling)).is_err()
                );
            }
        }

        /// The short id is the id's leading `SHORT_ID_LENGTH` characters,
        /// and the staging bridge yields the same text.
        #[test]
        fn test_short_id_is_the_leading_characters_on_both_sides_of_the_bridge(
            id in canonical_id(),
        ) {
            let parsed = ValidatedIssueId::try_from(id.as_str()).unwrap();
            prop_assert_eq!(parsed.short_id(), &id[..SHORT_ID_LENGTH]);
            prop_assert_eq!(IssueIdStaging::short_id(&id), parsed.short_id());
        }
    }
}
