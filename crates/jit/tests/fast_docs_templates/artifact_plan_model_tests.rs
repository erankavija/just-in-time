use jit::config::DocumentationConfig;
use jit::domain::artifact_plan::{
    ArtifactAction, ArtifactPlan, ArtifactPlanEntry, ArtifactVersion, BlockerCode, ContentIdentity,
    EvidenceCode, PlanBlocker, PlanTarget, PlanWarning, PolicyStatus, WarningCode,
    ARCHIVE_PLAN_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const FULL_SHA1_OID: &str = "0123456789abcdef0123456789abcdef01234567";
const FULL_SHA256_OID: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn artifact(source: &str, version: ArtifactVersion) -> ArtifactPlanEntry {
    ArtifactPlanEntry::new(source, version, ArtifactAction::Retain)
}

fn configured_plan(
    artifacts: Vec<ArtifactPlanEntry>,
    blockers: Vec<PlanBlocker>,
    warnings: Vec<PlanWarning>,
) -> ArtifactPlan {
    ArtifactPlan::new(
        PlanTarget::Container {
            id: "7d3a3a47-1e1c-466b-992c-b2df7ccae978".to_string(),
        },
        "dev/archive/7d3a3a47",
        PolicyStatus::Configured,
        artifacts,
        blockers,
        warnings,
    )
    .unwrap()
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn test_artifact_plan_serialization_is_stable_for_permuted_inputs() {
    let pinned = ArtifactVersion::pinned(FULL_SHA1_OID).unwrap();
    let first = configured_plan(
        vec![
            artifact("./dev//z.md", ArtifactVersion::WorkingTree),
            artifact("dev/a.md", ArtifactVersion::WorkingTree),
            artifact("dev/a.md", pinned.clone()),
        ],
        vec![
            PlanBlocker::new(BlockerCode::MissingSource, Some("z")),
            PlanBlocker::new(BlockerCode::DestinationConflict, Some("b")),
            PlanBlocker::new(BlockerCode::DestinationConflict, Some("a")),
        ],
        vec![
            PlanWarning::new(WarningCode::NoOwner, Some("z")),
            PlanWarning::new(WarningCode::ExternalEdge, Some("b")),
        ],
    );
    let second = configured_plan(
        vec![
            artifact("dev/a.md", pinned),
            artifact("dev/a.md", ArtifactVersion::WorkingTree),
            artifact("dev/z.md", ArtifactVersion::WorkingTree),
        ],
        vec![
            PlanBlocker::new(BlockerCode::DestinationConflict, Some("a")),
            PlanBlocker::new(BlockerCode::MissingSource, Some("z")),
            PlanBlocker::new(BlockerCode::DestinationConflict, Some("b")),
        ],
        vec![
            PlanWarning::new(WarningCode::ExternalEdge, Some("b")),
            PlanWarning::new(WarningCode::NoOwner, Some("z")),
        ],
    );

    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
}

#[test]
fn test_artifact_version_accepts_only_canonical_full_commit_oids() {
    let sha1 = ArtifactVersion::pinned(FULL_SHA1_OID).unwrap();
    let sha256 = ArtifactVersion::pinned(FULL_SHA256_OID).unwrap();

    assert_eq!(sha256.as_str(), FULL_SHA256_OID);
    assert_eq!(
        serde_json::to_value(&sha256).unwrap(),
        json!(FULL_SHA256_OID)
    );
    assert_eq!(
        serde_json::from_value::<ArtifactVersion>(json!(FULL_SHA256_OID)).unwrap(),
        sha256
    );

    assert!(sha1 < sha256);
    let mut versions = vec![sha256.clone(), sha1.clone()];
    versions.sort();
    assert_eq!(versions, vec![sha1, sha256]);

    assert!(ArtifactVersion::pinned("HEAD").is_err());
    assert!(ArtifactVersion::pinned("01234567").is_err());
    for invalid_length in [39, 41, 63, 65] {
        assert!(ArtifactVersion::pinned("0".repeat(invalid_length)).is_err());
    }
    assert!(ArtifactVersion::pinned(FULL_SHA1_OID.to_uppercase()).is_err());
    assert!(ArtifactVersion::pinned(FULL_SHA256_OID.to_uppercase()).is_err());
    assert!(ArtifactVersion::pinned(format!("{}g", "0".repeat(63))).is_err());
    assert!(serde_json::from_value::<ArtifactVersion>(json!("main")).is_err());
}

#[test]
fn test_policy_status_preserves_three_explicitness_states() {
    assert_eq!(
        PolicyStatus::from_documentation(None),
        PolicyStatus::Unconfigured
    );

    let partial = DocumentationConfig {
        development_root: None,
        managed_paths: Some(vec!["dev/active".to_string()]),
        archive_root: None,
        permanent_paths: Some(vec!["docs".to_string()]),
    };
    assert_eq!(
        PolicyStatus::from_documentation(Some(&partial)),
        PolicyStatus::Incomplete
    );

    let configured = DocumentationConfig {
        archive_root: Some("dev/archive".to_string()),
        ..partial
    };
    assert_eq!(
        PolicyStatus::from_documentation(Some(&configured)),
        PolicyStatus::Configured
    );

    for status in [PolicyStatus::Unconfigured, PolicyStatus::Incomplete] {
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "dev/active/a.md".to_string(),
            },
            "dev/archive",
            status,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(!plan.eligible());
        let expected = match status {
            PolicyStatus::Unconfigured => "policy-unconfigured",
            PolicyStatus::Incomplete => "policy-incomplete",
            PolicyStatus::Configured => unreachable!(),
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap()["blockers"][0]["code"],
            expected
        );
        assert!(plan.executable_artifacts().is_err());
    }
}

#[test]
fn test_content_identity_is_derived_from_one_byte_slice() {
    let identity = ContentIdentity::from_bytes(b"hello world");
    assert_eq!(
        serde_json::to_value(identity).unwrap(),
        json!({
            "sha256": "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
            "byte_size": 11
        })
    );

    let published_without_identity = ArtifactPlanEntry::new(
        "dev/active/publish.md",
        ArtifactVersion::WorkingTree,
        ArtifactAction::Copy,
    )
    .with_destination("dev/archive/publish.md");
    assert!(ArtifactPlan::new(
        PlanTarget::Document {
            path: "dev/active/publish.md".to_string(),
        },
        "dev/archive",
        PolicyStatus::Configured,
        vec![published_without_identity],
        Vec::new(),
        Vec::new(),
    )
    .is_err());
}

#[test]
fn test_pinned_artifact_is_non_relocating_historical_entry() {
    let value = serde_json::to_value(configured_plan(
        vec![artifact(
            "dev/active/history.md",
            ArtifactVersion::pinned(FULL_SHA1_OID).unwrap(),
        )],
        Vec::new(),
        Vec::new(),
    ))
    .unwrap();

    assert_eq!(value["artifacts"][0]["version"], FULL_SHA1_OID);
    assert_eq!(value["artifacts"][0]["action"], "retain");
    assert_eq!(value["artifacts"][0]["destination"], Value::Null);
    assert_eq!(
        value["artifacts"][0]["evidence"],
        json!(["pinned-historical"])
    );
}

#[test]
fn test_preview_and_execution_share_the_artifact_plan_type() {
    fn preview_input(plan: &ArtifactPlan) -> usize {
        plan.artifacts().len()
    }
    fn execution_input(
        plan: &ArtifactPlan,
    ) -> Result<usize, jit::domain::artifact_plan::PlanError> {
        Ok(plan.executable_artifacts()?.len())
    }

    let plan = configured_plan(
        vec![artifact("dev/a.md", ArtifactVersion::WorkingTree)],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(preview_input(&plan), 1);
    assert_eq!(execution_input(&plan).unwrap(), 1);

    let wire = serde_json::to_vec(&plan).unwrap();
    let decoded: ArtifactPlan = serde_json::from_slice(&wire).unwrap();
    assert_eq!(preview_input(&decoded), 1);
    assert_eq!(execution_input(&decoded).unwrap(), 1);
}

#[test]
fn test_archive_plan_schema_v1_golden_fields_and_codes() {
    let value = serde_json::to_value(configured_plan(
        vec![artifact("dev/a.md", ArtifactVersion::WorkingTree)],
        Vec::new(),
        Vec::new(),
    ))
    .unwrap();

    assert_eq!(ARCHIVE_PLAN_SCHEMA_VERSION, 1);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(
        object_keys(&value),
        BTreeSet::from([
            "schema_version",
            "target",
            "destination_root",
            "eligible",
            "policy_status",
            "action_counts",
            "count",
            "artifacts",
            "blockers",
            "warnings",
        ])
    );
    assert_eq!(
        object_keys(&value["artifacts"][0]),
        BTreeSet::from([
            "source",
            "version",
            "content_identity",
            "destination",
            "action",
            "already_archived",
            "provenance",
            "format",
            "owners",
            "edges",
            "reference_changes",
            "pending_deletions",
            "evidence",
            "blockers",
            "warnings",
        ])
    );
    assert_eq!(
        object_keys(&value["action_counts"]),
        BTreeSet::from([
            "move",
            "copy",
            "retain",
            "block",
            "already_archived",
            "pending_deletions",
        ])
    );
    assert_eq!(
        EvidenceCode::ALL.map(EvidenceCode::as_str),
        [
            "permanent-path",
            "outside-owner",
            "active-owner",
            "unmanaged-path",
            "archived-source",
            "pinned-historical",
        ]
    );
    assert_eq!(
        BlockerCode::ALL.map(BlockerCode::as_str),
        [
            "policy-unconfigured",
            "policy-incomplete",
            "unmanaged-selected-root",
            "destination-conflict",
            "document-non-terminal-owner",
            "pinned-read-failed",
            "repository-escape",
            "unresolvable-edge",
            "unpreservable-layout",
            "non-terminal-target",
            "missing-source",
            "symlink-artifact",
            "unsupported-artifact-type",
        ]
    );
    assert_eq!(
        WarningCode::ALL.map(WarningCode::as_str),
        [
            "missing-edge-target",
            "external-edge",
            "no-owner",
            "residue-source",
            "deletion-failed",
            "not-selected-sibling",
            "dynamic-loading-suspected",
            "unsupported-edge-target",
        ]
    );
}
