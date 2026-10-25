//! Validated identity types. Each holds a private canonical value, parses
//! through `TryFrom<&str>`/`FromStr`, and serializes as a plain string.
//! [`IssueIdPrefix`] is the parsed form of user-supplied issue-id input.

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

/// Error raised when an issue-id prefix normalises to fewer than
/// [`IssueIdPrefix::MIN_ID_PREFIX_LENGTH`] characters.
///
/// The CLI downcasts to this type to classify the failure as an argument error
/// (exit code `2`) and to render an `INVALID_ID_PREFIX` JSON error; the
/// offending prefix is carried for the JSON `details` field.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error(
    "issue id prefix '{prefix}' must be at least {} characters",
    IssueIdPrefix::MIN_ID_PREFIX_LENGTH
)]
pub struct InvalidIdPrefixError {
    prefix: String,
}

impl InvalidIdPrefixError {
    /// Build an [`InvalidIdPrefixError`] carrying the too-short prefix.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    /// The offending (too-short) prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// A requested issue id or id prefix, normalised and length-checked.
///
/// Normalisation lowercases the input and strips its hyphens; the prefix names
/// each id whose normalised form it starts. A full id is the longest prefix of
/// itself, so it matches in any case or hyphenation. Parsing consults no
/// candidate, so a too-short input is refused without reading an issue record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
// The empty description keeps this rustdoc out of the published schema.
#[schemars(with = "String", description = "")]
pub struct IssueIdPrefix {
    requested: String,
    normalised: String,
}

impl IssueIdPrefix {
    /// Shortest prefix parsing accepts, counted after normalisation.
    pub const MIN_ID_PREFIX_LENGTH: usize = 4;

    /// The input as given.
    pub fn as_str(&self) -> &str {
        &self.requested
    }

    /// Whether this prefix names `id`.
    pub fn matches(&self, id: &str) -> bool {
        normalise(id).starts_with(&self.normalised)
    }
}

/// A stored id names itself; the minimum applies only to requested input.
impl From<&IssueId> for IssueIdPrefix {
    fn from(id: &IssueId) -> Self {
        Self {
            requested: id.to_string(),
            normalised: normalise(id),
        }
    }
}

impl TryFrom<&str> for IssueIdPrefix {
    type Error = InvalidIdPrefixError;

    fn try_from(requested: &str) -> Result<Self, Self::Error> {
        let normalised = normalise(requested);
        (normalised.chars().count() >= Self::MIN_ID_PREFIX_LENGTH)
            .then(|| Self {
                requested: requested.to_string(),
                normalised,
            })
            .ok_or_else(|| InvalidIdPrefixError::new(requested))
    }
}

impl TryFrom<String> for IssueIdPrefix {
    type Error = InvalidIdPrefixError;

    fn try_from(requested: String) -> Result<Self, Self::Error> {
        Self::try_from(requested.as_str())
    }
}

impl FromStr for IssueIdPrefix {
    type Err = InvalidIdPrefixError;

    fn from_str(requested: &str) -> Result<Self, Self::Err> {
        Self::try_from(requested)
    }
}

impl From<IssueIdPrefix> for String {
    fn from(prefix: IssueIdPrefix) -> Self {
        prefix.requested
    }
}

impl fmt::Display for IssueIdPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.requested.fmt(formatter)
    }
}

fn normalise(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
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

        /// A stored id converts to a prefix that names it and equals its
        /// parsed form.
        #[test]
        fn test_from_issue_id_names_the_id_as_its_parse_does(id in canonical_id()) {
            let prefix = IssueIdPrefix::from(&id);
            prop_assert!(prefix.matches(&id));
            prop_assert_eq!(Ok(prefix), IssueIdPrefix::try_from(id.as_str()));
        }

        /// Any case and hyphenation of any prefix of at least the minimum
        /// length, the full id included, names the id.
        #[test]
        fn test_try_from_prefix_matches_any_spelling_of_a_long_enough_prefix(
            id in canonical_id(),
            len in IssueIdPrefix::MIN_ID_PREFIX_LENGTH..=32usize,
            upper in proptest::collection::vec(any::<bool>(), 32),
            hyphens in proptest::collection::vec(0usize..3, 32),
        ) {
            let requested: String = id
                .replace('-', "")
                .chars()
                .take(len)
                .zip(upper.iter().zip(&hyphens))
                .map(|(c, (upper, hyphens))| {
                    let c = if *upper { c.to_ascii_uppercase() } else { c };
                    format!("{c}{}", "-".repeat(*hyphens))
                })
                .collect();
            prop_assert!(IssueIdPrefix::try_from(requested.as_str()).unwrap().matches(&id));
        }
    }

    const ID: &str = "b53beeba-32f2-4671-a407-2d94e4d13d41";

    #[test]
    fn test_try_from_prefix_rejects_an_input_normalising_below_the_minimum() {
        let short = &ID.replace('-', "")[..IssueIdPrefix::MIN_ID_PREFIX_LENGTH - 1];
        for requested in ["", "----", short, &format!("{short}-")] {
            assert_eq!(
                IssueIdPrefix::try_from(requested),
                Err(InvalidIdPrefixError::new(requested))
            );
            assert!(serde_json::from_value::<IssueIdPrefix>(json!(requested)).is_err());
        }
        assert!(IssueIdPrefix::try_from(&ID[..IssueIdPrefix::MIN_ID_PREFIX_LENGTH]).is_ok());
    }

    #[test]
    fn test_matches_refuses_an_id_the_prefix_diverges_from() {
        let prefix = IssueIdPrefix::try_from("b53beebb").unwrap();
        assert!(!prefix.matches(ID));
        assert!(!IssueIdPrefix::try_from(format!("{ID}0"))
            .unwrap()
            .matches(ID));
    }

    #[test]
    fn test_invalid_id_prefix_error_names_the_input_and_the_minimum() {
        let error = InvalidIdPrefixError::new("ab");
        let message = error.to_string();
        assert!(message.contains("'ab'"));
        assert!(message.contains(&IssueIdPrefix::MIN_ID_PREFIX_LENGTH.to_string()));
        assert_eq!(error.prefix(), "ab");
    }
}
