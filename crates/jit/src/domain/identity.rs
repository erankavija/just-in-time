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

/// Whether `input` is the canonical lowercase hyphenated spelling of a UUID.
fn is_canonical_uuid(input: &str) -> bool {
    Uuid::parse_str(input).is_ok_and(|uuid| uuid.to_string() == input)
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
pub struct IssueId(String);

impl IssueId {
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first [`SHORT_ID_LENGTH`] characters of the id.
    pub fn short_id(&self) -> &str {
        self.0.get(..SHORT_ID_LENGTH).unwrap_or(&self.0)
    }

    /// The id of a generated UUID; its hyphenated form is canonical, so this
    /// cannot fail.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid.hyphenated().to_string())
    }
}

impl AsRef<str> for IssueId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<&str> for IssueId {
    type Error = InvalidIssueIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        is_canonical_uuid(input)
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidIssueIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for IssueId {
    type Error = InvalidIssueIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for IssueId {
    type Err = InvalidIssueIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<IssueId> for String {
    fn from(id: IssueId) -> Self {
        id.0
    }
}

impl fmt::Display for IssueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Error raised when a gate-run id is not a canonical lowercase hyphenated UUID.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid gate run id '{input}'; expected a lowercase hyphenated UUID")]
pub struct InvalidGateRunIdError {
    input: String,
}

/// A gate-run id in the canonical lowercase hyphenated UUID form.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
// The empty description keeps this rustdoc out of the published schema.
#[schemars(with = "String", description = "")]
pub struct GateRunId(String);

impl GateRunId {
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id of a generated UUID; its hyphenated form is canonical, so this
    /// cannot fail.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid.hyphenated().to_string())
    }
}

impl TryFrom<&str> for GateRunId {
    type Error = InvalidGateRunIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        is_canonical_uuid(input)
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidGateRunIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for GateRunId {
    type Error = InvalidGateRunIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for GateRunId {
    type Err = InvalidGateRunIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<GateRunId> for String {
    fn from(id: GateRunId) -> Self {
        id.0
    }
}

impl fmt::Display for GateRunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Invalid lease identity, including the rejected input.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid lease id '{input}'; expected a lowercase hyphenated UUID")]
pub struct InvalidLeaseIdError {
    input: String,
}

/// A lease id in the canonical lowercase hyphenated UUID form.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", description = "")]
pub struct LeaseId(String);

impl LeaseId {
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for LeaseId {
    type Error = InvalidLeaseIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        is_canonical_uuid(input)
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidLeaseIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for LeaseId {
    type Error = InvalidLeaseIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for LeaseId {
    type Err = InvalidLeaseIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<LeaseId> for String {
    fn from(id: LeaseId) -> Self {
        id.0
    }
}

impl fmt::Display for LeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Invalid worktree identity, including the rejected input.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid worktree id '{input}'; expected a non-empty token without whitespace")]
pub struct InvalidWorktreeIdError {
    input: String,
}

/// A non-empty worktree token without whitespace.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", description = "")]
pub struct WorktreeId(String);

impl WorktreeId {
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for WorktreeId {
    type Error = InvalidWorktreeIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        (!input.is_empty() && !input.chars().any(char::is_whitespace))
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidWorktreeIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for WorktreeId {
    type Error = InvalidWorktreeIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for WorktreeId {
    type Err = InvalidWorktreeIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<WorktreeId> for String {
    fn from(id: WorktreeId) -> Self {
        id.0
    }
}

impl fmt::Display for WorktreeId {
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

    pub(crate) fn normalised(&self) -> &str {
        &self.normalised
    }

    /// Whether this prefix names `id`.
    pub fn matches(&self, id: &IssueId) -> bool {
        id.as_str()
            .bytes()
            .filter(|byte| *byte != b'-')
            .take(self.normalised.len())
            .eq(self.normalised.bytes())
    }
}

/// A stored id names itself; the minimum applies only to requested input.
impl From<&IssueId> for IssueIdPrefix {
    fn from(id: &IssueId) -> Self {
        Self {
            requested: id.to_string(),
            normalised: normalise(id.as_str()),
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

/// Error raised when a profile id is not a lowercase-kebab token.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid profile id '{input}'; expected lowercase-kebab")]
pub struct InvalidProfileIdError {
    input: String,
}

/// Stable lowercase-kebab identifier used to name a profile package.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct ProfileId(String);

impl ProfileId {
    /// Borrow the canonical package identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ProfileId {
    type Error = InvalidProfileIdError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        is_lowercase_kebab(input)
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidProfileIdError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for ProfileId {
    type Error = InvalidProfileIdError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for ProfileId {
    type Err = InvalidProfileIdError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<ProfileId> for String {
    fn from(id: ProfileId) -> Self {
        id.0
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Error raised when a gate key is not a lowercase-kebab token.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("invalid gate key '{input}'; expected lowercase-kebab")]
pub struct InvalidGateKeyError {
    input: String,
}

/// A lowercase-kebab gate key.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", description = "")]
pub struct GateKey(String);

impl GateKey {
    /// Borrow the key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for GateKey {
    type Error = InvalidGateKeyError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        is_lowercase_kebab(input)
            .then(|| Self(input.to_string()))
            .ok_or_else(|| InvalidGateKeyError {
                input: input.to_string(),
            })
    }
}

impl TryFrom<String> for GateKey {
    type Error = InvalidGateKeyError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::try_from(input.as_str())
    }
}

impl FromStr for GateKey {
    type Err = InvalidGateKeyError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

impl From<GateKey> for String {
    fn from(key: GateKey) -> Self {
        key.0
    }
}

impl fmt::Display for GateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

pub(crate) fn is_lowercase_kebab(value: &str) -> bool {
    let mut segments = value.split('-');
    segments.next().is_some_and(|first| {
        !first.is_empty()
            && first.starts_with(|character: char| character.is_ascii_lowercase())
            && first
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    }) && segments.all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{json, Value};

    #[test]
    fn test_profile_id_round_trips_canonical_tokens() {
        for input in ["a", "jit-default", "profile2", "profile-2-extra"] {
            let id = ProfileId::try_from(input).unwrap();
            assert_eq!(id.as_str(), input);
            assert_eq!(id.to_string(), input);
            assert_eq!(input.parse::<ProfileId>().unwrap(), id);
            let encoded = serde_json::to_value(&id).unwrap();
            assert_eq!(encoded, json!(input));
            assert_eq!(serde_json::from_value::<ProfileId>(encoded).unwrap(), id);
        }
    }

    #[test]
    fn test_profile_id_rejects_non_canonical_tokens_at_parse_boundaries() {
        for input in [
            "",
            "2profile",
            "-profile",
            "profile-",
            "profile--id",
            "Profile",
            "profile_id",
            "profile.id",
            "profile/id",
            "profile id",
            " profile",
            "profile\n",
            "pröfile",
        ] {
            let error = ProfileId::try_from(input).unwrap_err();
            assert!(error.to_string().contains(input));
            assert!(input.parse::<ProfileId>().is_err());
            assert!(serde_json::from_value::<ProfileId>(json!(input)).is_err());
        }
    }

    #[test]
    fn test_profile_id_json_schema_preserves_the_published_string_contract() {
        let schema = ProfileId::json_schema(&mut schemars::gen::SchemaGenerator::default());
        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            json!({
                "description": "Stable lowercase-kebab identifier used to name a profile package.",
                "type": "string"
            })
        );
    }

    #[test]
    fn test_gate_key_round_trips_canonical_tokens_at_parse_boundaries() {
        for input in ["a", "cargo-ci", "gate2", "code-review-2-extra"] {
            let key = GateKey::try_from(input).unwrap();
            assert_eq!(key.as_str(), input);
            assert_eq!(key.to_string(), input);
            assert_eq!(input.parse::<GateKey>().unwrap(), key);
            assert_eq!(GateKey::try_from(input.to_string()).unwrap(), key);
            let encoded = serde_json::to_value(&key).unwrap();
            assert_eq!(encoded, json!(input));
            assert_eq!(serde_json::from_value::<GateKey>(encoded).unwrap(), key);
            assert_eq!(String::from(key), input);
        }
    }

    #[test]
    fn test_gate_key_rejects_non_kebab_tokens_naming_the_input() {
        for input in [
            "",
            "2gate",
            "-gate",
            "gate-",
            "gate--key",
            "Gate",
            "gate_key",
            "gate.key",
            "gate key",
            " gate",
            "gate\n",
            "verify:REQ-01",
        ] {
            let error = GateKey::try_from(input).unwrap_err();
            assert!(error.to_string().contains(&format!("'{input}'")));
            assert!(input.parse::<GateKey>().is_err());
            assert!(serde_json::from_value::<GateKey>(json!(input)).is_err());
        }
    }

    #[test]
    fn test_gate_key_json_schema_is_a_plain_string() {
        let schema = GateKey::json_schema(&mut schemars::gen::SchemaGenerator::default());
        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            json!({"type": "string"})
        );
    }

    #[test]
    fn test_lease_id_round_trips_canonical_uuid_at_parse_boundaries() {
        let input = "b53beeba-32f2-4671-a407-2d94e4d13d41";
        let id = LeaseId::try_from(input).unwrap();
        assert_eq!(id.as_str(), input);
        assert_eq!(id.to_string(), input);
        assert_eq!(input.parse::<LeaseId>().unwrap(), id);
        assert_eq!(LeaseId::try_from(input.to_string()).unwrap(), id);
        let encoded = serde_json::to_value(&id).unwrap();
        assert_eq!(encoded, json!(input));
        assert_eq!(serde_json::from_value::<LeaseId>(encoded).unwrap(), id);
        assert_eq!(String::from(id), input);
    }

    #[test]
    fn test_lease_id_rejects_non_canonical_uuid_at_parse_boundaries() {
        for input in [
            "",
            "lease-1",
            "b53beeba32f24671a4072d94e4d13d41",
            "B53BEEBA-32F2-4671-A407-2D94E4D13D41",
            "{b53beeba-32f2-4671-a407-2d94e4d13d41}",
            "urn:uuid:b53beeba-32f2-4671-a407-2d94e4d13d41",
            " b53beeba-32f2-4671-a407-2d94e4d13d41",
        ] {
            assert!(LeaseId::try_from(input)
                .unwrap_err()
                .to_string()
                .contains(input));
            assert!(input.parse::<LeaseId>().is_err());
            assert!(serde_json::from_value::<LeaseId>(json!(input)).is_err());
        }
    }

    #[test]
    fn test_worktree_id_round_trips_non_empty_tokens_at_parse_boundaries() {
        for input in [
            "wt:abc123ef",
            "wt:feature/branch",
            "/tmp/repo",
            "token",
            "树",
        ] {
            let id = WorktreeId::try_from(input).unwrap();
            assert_eq!(id.as_str(), input);
            assert_eq!(id.to_string(), input);
            assert_eq!(input.parse::<WorktreeId>().unwrap(), id);
            assert_eq!(WorktreeId::try_from(input.to_string()).unwrap(), id);
            let encoded = serde_json::to_value(&id).unwrap();
            assert_eq!(encoded, json!(input));
            assert_eq!(serde_json::from_value::<WorktreeId>(encoded).unwrap(), id);
            assert_eq!(String::from(id), input);
        }
    }

    #[test]
    fn test_worktree_id_rejects_empty_or_whitespace_tokens_at_parse_boundaries() {
        for input in [
            "",
            " ",
            "wt:two words",
            "wt:tab\t",
            "\nwt:newline",
            "wt:unicode\u{2003}space",
        ] {
            assert!(WorktreeId::try_from(input)
                .unwrap_err()
                .to_string()
                .contains(input));
            assert!(input.parse::<WorktreeId>().is_err());
            assert!(serde_json::from_value::<WorktreeId>(json!(input)).is_err());
        }
    }

    #[test]
    fn test_lease_and_worktree_id_json_schemas_preserve_plain_strings() {
        let mut generator = schemars::gen::SchemaGenerator::default();
        for schema in [
            LeaseId::json_schema(&mut generator),
            WorktreeId::json_schema(&mut generator),
        ] {
            assert_eq!(
                serde_json::to_value(schema).unwrap(),
                json!({"type": "string"})
            );
        }
    }

    #[test]
    fn test_gate_run_id_json_schema_is_a_plain_string() {
        let schema = GateRunId::json_schema(&mut schemars::gen::SchemaGenerator::default());
        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            json!({"type": "string"})
        );
    }

    #[test]
    fn test_gate_run_id_rejects_inputs_that_are_not_uuids() {
        for input in ["", "run-1", "../run", "nested/run", "."] {
            let error = GateRunId::try_from(input).unwrap_err();
            assert!(error.to_string().contains(input));
            assert!(serde_json::from_value::<GateRunId>(json!(input)).is_err());
        }
    }

    fn canonical_id() -> impl Strategy<Value = String> {
        any::<u128>().prop_map(|bits| Uuid::from_u128(bits).to_string())
    }

    #[test]
    fn test_try_from_rejects_empty_input() {
        assert!(IssueId::try_from("").is_err());
    }

    #[test]
    fn test_json_schema_is_a_plain_string() {
        let schema = IssueId::json_schema(&mut schemars::gen::SchemaGenerator::default());
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
            let parsed = IssueId::try_from(id.as_str()).unwrap();
            prop_assert_eq!(parsed.as_str(), id.as_str());
            prop_assert_eq!(parsed.to_string(), id.clone());
            let encoded = serde_json::to_value(&parsed).unwrap();
            prop_assert_eq!(&encoded, &Value::String(id.clone()));
            prop_assert_eq!(serde_json::from_value::<IssueId>(encoded).unwrap(), parsed);
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
                let error = IssueId::try_from(spelling.as_str()).unwrap_err();
                prop_assert!(error.to_string().contains(spelling.as_str()));
                prop_assert!(
                    serde_json::from_value::<IssueId>(json!(spelling)).is_err()
                );
            }
        }

        /// A canonical gate-run id survives parsing, serde and `from_uuid` as
        /// the same text.
        #[test]
        fn test_gate_run_id_round_trips_a_canonical_id(bits in any::<u128>()) {
            let uuid = Uuid::from_u128(bits);
            let id = uuid.to_string();
            let parsed = GateRunId::try_from(id.as_str()).unwrap();
            prop_assert_eq!(parsed.as_str(), id.as_str());
            prop_assert_eq!(parsed.to_string(), id.clone());
            prop_assert_eq!(&id.parse::<GateRunId>().unwrap(), &parsed);
            prop_assert_eq!(&GateRunId::from_uuid(uuid), &parsed);
            let encoded = serde_json::to_value(&parsed).unwrap();
            prop_assert_eq!(&encoded, &Value::String(id.clone()));
            prop_assert_eq!(&serde_json::from_value::<GateRunId>(encoded).unwrap(), &parsed);
            prop_assert_eq!(String::from(parsed), id);
        }

        /// Every other spelling of a UUID is refused as a gate-run id.
        #[test]
        fn test_gate_run_id_rejects_each_non_canonical_spelling(bits in any::<u128>()) {
            let uuid = Uuid::from_u128(bits);
            let canonical = uuid.to_string();
            let spellings = [
                uuid.simple().to_string(),
                uuid.braced().to_string(),
                uuid.urn().to_string(),
                canonical.to_uppercase(),
            ];
            for spelling in spellings.iter().filter(|spelling| **spelling != canonical) {
                prop_assert!(GateRunId::try_from(spelling.as_str()).is_err());
                prop_assert!(serde_json::from_value::<GateRunId>(json!(spelling)).is_err());
            }
        }

        /// A generated UUID becomes the id its canonical text parses to, on
        /// both sides of the bridge.
        #[test]
        fn test_from_uuid_yields_the_canonical_id_on_both_sides_of_the_bridge(
            bits in any::<u128>(),
        ) {
            let uuid = Uuid::from_u128(bits);
            let parsed = IssueId::try_from(uuid.to_string()).unwrap();
            prop_assert_eq!(&IssueId::from_uuid(uuid), &parsed);
        }

        /// The short id is the id's leading `SHORT_ID_LENGTH` characters.
        #[test]
        fn test_short_id_is_the_leading_characters(id in canonical_id()) {
            let parsed = IssueId::try_from(id.as_str()).unwrap();
            prop_assert_eq!(parsed.short_id(), &id[..SHORT_ID_LENGTH]);
        }

        /// A stored id converts to a prefix that names it and equals its
        /// parsed form.
        #[test]
        fn test_from_issue_id_names_the_id_as_its_parse_does(id in canonical_id()) {
            let parsed = IssueId::try_from(id.as_str()).unwrap();
            let prefix = IssueIdPrefix::from(&parsed);
            prop_assert!(prefix.matches(&parsed));
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
            prop_assert!(IssueIdPrefix::try_from(requested.as_str()).unwrap().matches(&IssueId::try_from(id).unwrap()));
        }

        #[test]
        fn test_matches_agrees_with_normalised_candidate(
            id in canonical_id(),
            other in canonical_id(),
            len in IssueIdPrefix::MIN_ID_PREFIX_LENGTH..=33usize,
            spelling in proptest::collection::vec((any::<bool>(), 0usize..3), 33),
            trailing_hyphens in 0usize..3,
        ) {
            let candidate = IssueId::try_from(id.as_str()).unwrap();
            for source in [&id, &other] {
                let mut requested: String = normalise(source)
                    .chars()
                    .chain(std::iter::once('0'))
                    .take(len)
                    .zip(&spelling)
                    .map(|(c, (upper, hyphens))| {
                        let c = if *upper { c.to_ascii_uppercase() } else { c };
                        format!("{}{c}", "-".repeat(*hyphens))
                    })
                    .collect();
                requested.push_str(&"-".repeat(trailing_hyphens));
                let prefix = IssueIdPrefix::try_from(requested.as_str()).unwrap();
                prop_assert_eq!(
                    prefix.matches(&candidate),
                    normalise(&id).starts_with(&normalise(&requested)),
                );
            }
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
        assert!(!prefix.matches(&IssueId::try_from(ID).unwrap()));
        assert!(!IssueIdPrefix::try_from(format!("{ID}0"))
            .unwrap()
            .matches(&IssueId::try_from(ID).unwrap()));
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
