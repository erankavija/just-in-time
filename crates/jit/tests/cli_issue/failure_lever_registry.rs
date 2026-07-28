//! Typed, canonical input for the machine-readable failure probes.

use serde::{de::Error as _, Deserialize, Deserializer};
use std::collections::BTreeSet;

const REGISTRY_TOML: &str = include_str!("failure_lever_registry.toml");
const CLAIM_FRAGMENT_TOML: &str =
    include_str!("../../../../dev/active/a2546471-json-error-contract/levers/claim.toml");

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureLeverRegistry {
    pub(crate) schema: FailureLeverSchema,
    pub(crate) arms: Vec<FailureLever>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FailureLeverFragment {
    schema: FailureLeverSchema,
    namespace: String,
    arms: Vec<FailureLever>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum FailureLeverSchema {
    #[serde(rename = "failure-lever-registry/v1")]
    V1,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FailureLever {
    Invocation(FailureLeverInvocation),
    Exemption(FailureLeverExemption),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FailureLeverInvocation {
    pub(crate) path: String,
    pub(crate) argv: Vec<String>,
    pub(crate) setup: Vec<String>,
    pub(crate) setup_notes: Option<String>,
    pub(crate) expected_failure: String,
    pub(crate) expected_exit: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FailureLeverExemption {
    pub(crate) path: String,
    pub(crate) exemption_reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFailureLever {
    path: String,
    argv: Option<Vec<String>>,
    setup: Option<Vec<String>>,
    setup_notes: Option<String>,
    expected_failure: Option<String>,
    expected_exit: Option<i32>,
    exemption_reason: Option<String>,
    confirmation: Confirmation,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Confirmation {
    Invoked,
    SourceOnly,
}

impl<'de> Deserialize<'de> for FailureLever {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawFailureLever::deserialize(deserializer)?;
        match raw {
            RawFailureLever {
                path,
                argv: Some(argv),
                setup: Some(setup),
                setup_notes,
                expected_failure: Some(expected_failure),
                expected_exit: Some(expected_exit),
                exemption_reason: None,
                confirmation: Confirmation::Invoked,
            } => Ok(Self::Invocation(FailureLeverInvocation {
                path,
                argv,
                setup,
                setup_notes,
                expected_failure,
                expected_exit,
            })),
            RawFailureLever {
                path,
                argv: None,
                setup: None,
                setup_notes: None,
                expected_failure: None,
                expected_exit: None,
                exemption_reason: Some(exemption_reason),
                confirmation: Confirmation::SourceOnly,
            } => Ok(Self::Exemption(FailureLeverExemption {
                path,
                exemption_reason,
            })),
            _ => Err(D::Error::custom(
                "a failure lever must be either an invoked arm with argv, setup, expected_failure, expected_exit, and confirmation = 'invoked', or an exemption with only exemption_reason and confirmation = 'source-only'",
            )),
        }
    }
}

impl FailureLever {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Invocation(lever) => &lever.path,
            Self::Exemption(lever) => &lever.path,
        }
    }
}

pub(crate) fn failure_lever_registry() -> FailureLeverRegistry {
    toml::from_str(REGISTRY_TOML).expect("committed failure-lever registry must be valid")
}

#[test]
fn test_failure_lever_registry_deserializes_typed_committed_input() {
    let registry = failure_lever_registry();
    assert_eq!(registry.schema, FailureLeverSchema::V1);
    assert!(!registry.arms.is_empty());

    let mut invocations = BTreeSet::new();
    registry.arms.iter().for_each(|arm| match arm {
        FailureLever::Invocation(lever) => {
            assert!(!lever.path.is_empty());
            assert!(!lever.argv.is_empty());
            assert!(lever.argv.iter().any(|arg| arg == "--json"));
            assert!(!lever.setup.is_empty());
            assert!(!lever.expected_failure.is_empty());
            assert!(invocations.insert(lever.argv.clone()));
        }
        FailureLever::Exemption(lever) => {
            assert!(!lever.path.is_empty());
            assert!(!lever.exemption_reason.is_empty());
        }
    });
}

#[test]
fn test_failure_lever_registry_claim_fragment_matches_assembled_rows() {
    let fragment: FailureLeverFragment =
        toml::from_str(CLAIM_FRAGMENT_TOML).expect("claim survey fragment must be valid");
    assert_eq!(fragment.schema, FailureLeverSchema::V1);
    assert_eq!(fragment.namespace, "claim");

    let assembled_claim_arms = failure_lever_registry()
        .arms
        .into_iter()
        .filter(|arm| arm.path().starts_with("claim "))
        .collect::<Vec<_>>();

    assert_eq!(fragment.arms, assembled_claim_arms);
}

#[test]
fn test_failure_lever_registry_rejects_entries_missing_required_fields() {
    let incomplete_invocation = r#"
        schema = "failure-lever-registry/v1"
        [[arms]]
        path = "status"
        argv = ["status", "--json"]
        expected_failure = "failure"
        expected_exit = 1
        confirmation = "invoked"
    "#;
    let incomplete_exemption = r#"
        schema = "failure-lever-registry/v1"
        [[arms]]
        path = "version"
        confirmation = "source-only"
    "#;

    assert!(toml::from_str::<FailureLeverRegistry>(incomplete_invocation).is_err());
    assert!(toml::from_str::<FailureLeverRegistry>(incomplete_exemption).is_err());
}
