//! Typed, canonical input for the machine-readable failure probes.

use jit::output::ErrorCode;
use serde::{de::Error as _, Deserialize, Deserializer};
use std::collections::BTreeSet;
use std::str::FromStr;

const REGISTRY_TOML: &str = include_str!("failure_lever_registry.toml");

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FailureLeverRegistry {
    pub(crate) schema: FailureLeverSchema,
    pub(crate) arms: Vec<FailureLever>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFailureLeverRegistry {
    schema: FailureLeverSchema,
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
    pub(crate) expected_code: ErrorCode,
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
    expected_code: Option<String>,
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
                expected_code: Some(expected_code),
                expected_exit: Some(expected_exit),
                exemption_reason: None,
                confirmation: Confirmation::Invoked,
            } => Ok(Self::Invocation(FailureLeverInvocation {
                path,
                argv,
                setup,
                setup_notes,
                expected_failure,
                expected_code: ErrorCode::from_str(&expected_code).map_err(D::Error::custom)?,
                expected_exit,
            })),
            RawFailureLever {
                path,
                argv: None,
                setup: None,
                setup_notes: None,
                expected_failure: None,
                expected_code: None,
                expected_exit: None,
                exemption_reason: Some(exemption_reason),
                confirmation: Confirmation::SourceOnly,
            } => Ok(Self::Exemption(FailureLeverExemption {
                path,
                exemption_reason,
            })),
            _ => Err(D::Error::custom(
                "a failure lever must be either an invoked arm with argv, setup, expected_failure, expected_code, expected_exit, and confirmation = 'invoked', or an exemption with only exemption_reason and confirmation = 'source-only'",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for FailureLeverRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let RawFailureLeverRegistry { schema, arms } =
            RawFailureLeverRegistry::deserialize(deserializer)?;
        let mut paths = BTreeSet::new();

        arms.iter().try_for_each(|arm| {
            paths.insert(arm.path()).then_some(()).ok_or_else(|| {
                D::Error::custom(format!("duplicate failure-lever path `{}`", arm.path()))
            })
        })?;

        Ok(Self { schema, arms })
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
            assert_eq!(lever.expected_exit, lever.expected_code.exit_code().code());
            assert!(invocations.insert(lever.argv.clone()));
        }
        FailureLever::Exemption(lever) => {
            assert!(!lever.path.is_empty());
            assert!(!lever.exemption_reason.is_empty());
        }
    });
}

#[test]
fn test_failure_lever_registry_test_target_uses_only_its_committed_fixture() {
    let target_source = include_str!(concat!("failure_lever_", "registry.rs"));
    let managed_planning_root = ["dev", "active"].join("/");

    assert!(
        target_source.contains("include_str!(\"failure_lever_registry.toml\")"),
        "the committed registry fixture remains the test target's input"
    );
    assert!(
        !target_source.contains(&managed_planning_root),
        "the test target must not compile or run against managed planning artifacts"
    );
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

#[test]
fn test_failure_lever_registry_rejects_duplicate_paths_across_registration_kinds() {
    let duplicate_path = r#"
        schema = "failure-lever-registry/v1"
        [[arms]]
        path = "status"
        argv = ["status", "--json"]
        setup = ["init"]
        expected_failure = "failure"
        expected_code = "PARSE_ERROR"
        expected_exit = 1
        confirmation = "invoked"

        [[arms]]
        path = "status"
        exemption_reason = "duplicate path must be rejected regardless of registration kind"
        confirmation = "source-only"
    "#;

    let error = toml::from_str::<FailureLeverRegistry>(duplicate_path)
        .expect_err("duplicate paths must be rejected by typed registry deserialization");
    assert!(error
        .to_string()
        .contains("duplicate failure-lever path `status`"));
}
