use crate::domain::ProfileOrigin;
use crate::profile::ProfilePackageModel;
use crate::repository_state::{AppliedProfileRecord, FileMode};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;

/// Whether an application published a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileApplicationStatus {
    /// Repository targets and audit state were already exact.
    Unchanged,
    /// A recoverable transaction reached its durable commit point.
    Applied,
}

/// Non-fatal application diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileApplicationWarning {
    /// Final targets are committed, but machine-local transaction cleanup remains.
    TransactionCleanupPending {
        /// Machine-local transaction identifier recoverable on the next mutation.
        transaction_id: String,
        /// Cleanup failure retained for diagnostics.
        reason: String,
    },
}

/// Internal command-layer profile application result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileApplyResult {
    /// Stable profile identifier.
    pub id: String,
    /// Applied semantic version.
    pub version: String,
    /// Whether anything was published.
    pub status: ProfileApplicationStatus,
    /// Deterministic plan identity rebuilt under the write lock.
    pub plan_hash: String,
    /// Transaction identifier for an applied result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Non-fatal cleanup diagnostics.
    pub warnings: Vec<ProfileApplicationWarning>,
}

/// Result of applying one package together with the packages it depends on.
///
/// A package that declares a dependency is applied as a set: every package the
/// closure resolves to is applied in its own right, with its own provenance
/// record and its own audit event, so [`Self::profiles`] carries one entry per
/// applied package rather than one summary over them. The order is the order
/// they were applied — each package after everything it depends on — which puts
/// the package the caller named last ([`Self::requested`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileComposedApplyResult {
    /// Number of results in [`Self::profiles`].
    pub count: usize,
    /// One result per applied package, dependencies before their dependants.
    pub profiles: Vec<ProfileApplyResult>,
}

impl ProfileComposedApplyResult {
    /// Collect per-package results into the composed answer.
    pub(crate) fn new(profiles: Vec<ProfileApplyResult>) -> Self {
        Self {
            count: profiles.len(),
            profiles,
        }
    }

    /// The result for the package the caller named.
    ///
    /// `None` only for an empty composition, which application does not
    /// produce: the named package is always applied, after everything it
    /// depends on.
    pub fn requested(&self) -> Option<&ProfileApplyResult> {
        self.profiles.last()
    }
}

/// One profile exposed by `jit profile list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileSummary {
    /// Stable package identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Where the resolved package's bytes were read from.
    pub origin: ProfileOrigin,
    /// Compatible JIT version requirement authored by the manifest.
    pub jit: String,
    /// Whether the stored values independently reproduce the package's exact provenance.
    pub applied: bool,
}

/// Count-wrapped profile-list response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileListResult {
    /// Number of profiles in [`Self::profiles`].
    pub count: usize,
    /// Profiles sorted by stable ID.
    pub profiles: Vec<ProfileSummary>,
}

/// One complete package inspection entry.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProfileShowEntry {
    /// Canonical package model produced from the immutable manifest bytes.
    pub manifest: ProfilePackageModel,
    /// Package discovery origin.
    pub origin: ProfileOrigin,
    /// Hash of the complete canonical package.
    pub package_hash: String,
    /// Per-target package hashes.
    pub target_hashes: BTreeMap<String, String>,
    /// Package file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total package byte size.
    pub byte_size: usize,
    /// Stored provenance record when the selected repository has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<AppliedProfileRecord>,
}

/// Count-wrapped, occurrence-ordered package inspection response.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProfileShowResult {
    /// Number of entries in [`Self::profiles`].
    pub count: usize,
    /// One package entry per selector occurrence, in selector order.
    pub profiles: Vec<ProfileShowEntry>,
}

impl ProfileShowResult {
    /// Collect package inspection entries without changing their selector order.
    pub(crate) fn new(profiles: Vec<ProfileShowEntry>) -> Self {
        Self {
            count: profiles.len(),
            profiles,
        }
    }
}

/// Planned target operation exposed by profile dry-run output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileTargetAction {
    /// Existing bytes and mode already match.
    Unchanged,
    /// Target is absent and would be created.
    Create,
    /// Existing target would be replaced by the package projection.
    Update,
}

/// One deterministic profile target in a dry-run plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileTargetChange {
    /// Repository-relative target path.
    pub path: String,
    /// Planned operation.
    pub action: ProfileTargetAction,
    /// Platform-neutral file-mode intent.
    pub executable: bool,
}

impl ProfileTargetChange {
    /// Construct a public target projection from canonical repository-state
    /// vocabulary.
    pub(crate) fn new(path: String, action: ProfileTargetAction, mode: FileMode) -> Self {
        Self {
            path,
            action,
            executable: mode == FileMode::Executable,
        }
    }
}

/// Whether a dry-run found work to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePlanStatus {
    /// Package targets and installed provenance already match.
    Unchanged,
    /// Applying the plan would publish at least one change.
    WouldApply,
}

/// One deterministic, non-mutating profile application preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePlanEntry {
    /// Stable profile identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Whether execution would publish this profile.
    pub status: ProfilePlanStatus,
    /// Identity of the complete canonical repository materialization plan this
    /// entry was derived from.
    pub plan_hash: String,
    /// The targets this profile decides, sorted by path.
    pub targets: Vec<ProfileTargetChange>,
}

/// Count-wrapped profile application previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePlanResult {
    /// Number of previews in [`Self::profiles`].
    pub count: usize,
    /// One preview per profile this rehearsal reports. Which profiles those are
    /// is stated by the command that produced them.
    pub profiles: Vec<ProfilePlanEntry>,
}

impl ProfilePlanResult {
    /// Collect previews without changing their selector order.
    pub(crate) fn new(profiles: Vec<ProfilePlanEntry>) -> Self {
        Self {
            count: profiles.len(),
            profiles,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::RootRelativePath;

    /// A record naming the package at the worktree-relative `location`.
    fn record_at(location: &str) -> AppliedProfileRecord {
        record(ProfileOrigin::Directory(
            RootRelativePath::parse(location).expect("a canonical package location"),
        ))
    }

    fn record(origin: ProfileOrigin) -> AppliedProfileRecord {
        AppliedProfileRecord::new(
            "example".try_into().expect("test profile id is canonical"),
            "1.0.0",
            "*",
            origin,
            "package",
            crate::profile::ResolvedVariables::default(),
            std::collections::BTreeSet::new(),
        )
    }

    /// A record whose `origin` is replaced by `origin`, as stored bytes.
    fn stored_with_origin(origin: serde_json::Value) -> Vec<u8> {
        let mut value: serde_json::Value =
            serde_json::from_slice(&record_at("profiles/example").to_bytes().unwrap()).unwrap();
        value["origin"] = origin;
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn test_installed_record_is_canonical_stable_json() {
        let value: serde_json::Value =
            serde_json::from_slice(&record_at("profiles/example").to_bytes().unwrap()).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                "claims",
                "compatible_jit",
                "id",
                "origin",
                "package_hash",
                "record_version",
                "variables",
                "version"
            ]
        );
    }

    #[test]
    fn test_installed_record_round_trips_the_location_a_directory_origin_names() {
        // The location survives the round trip through stored bytes, which is
        // what a later run reads to find the package again, and two records
        // naming different locations are distinguishable in the stored image.
        let location = RootRelativePath::parse("profiles/example").unwrap();
        let stored = record_at("profiles/example").to_bytes().unwrap();

        let read: AppliedProfileRecord = serde_json::from_slice(&stored).unwrap();
        assert_eq!(read.origin, ProfileOrigin::Directory(location));
        assert_ne!(
            stored,
            record_at("vendor/elsewhere").to_bytes().unwrap(),
            "records naming different locations must not store the same image"
        );
    }

    #[test]
    fn test_installed_record_round_trips_shipped_embedded_provenance_without_configuration() {
        let stored = record(ProfileOrigin::Embedded).to_bytes().unwrap();

        assert_eq!(
            serde_json::from_slice::<AppliedProfileRecord>(&stored)
                .expect("embedded provenance remains a valid v2 record")
                .origin,
            ProfileOrigin::Embedded
        );
    }

    #[test]
    fn test_installed_record_rejects_an_absent_or_malformed_directory_location() {
        // Every one of these is a stored record a repository could hold, and
        // each must fail the read rather than resolve to a package the record
        // does not actually name.
        for origin in [
            serde_json::json!({ "source": "directory" }),
            serde_json::json!({ "source": "directory", "location": "../outside" }),
            serde_json::json!({ "source": "directory", "location": "/absolute" }),
            serde_json::json!({ "source": "directory", "location": "a/./b" }),
            serde_json::json!({ "source": "directory", "location": 7 }),
            // A source the origin vocabulary does not carry is refused
            // outright rather than read past to its location.
            serde_json::json!({ "source": "unsupported", "location": "profiles/example" }),
            serde_json::json!({ "source": "unknown", "location": "profiles/example" }),
        ] {
            let stored = stored_with_origin(origin.clone());
            assert!(
                serde_json::from_slice::<AppliedProfileRecord>(&stored).is_err(),
                "{origin} must not read as a valid record"
            );
        }
    }

    #[test]
    fn test_installed_record_requires_ownership_claim_evidence() {
        let mut stored: serde_json::Value =
            serde_json::from_slice(&record_at("profiles/example").to_bytes().unwrap()).unwrap();
        stored
            .as_object_mut()
            .expect("an installed record serializes as an object")
            .remove("claims");

        assert!(
            serde_json::from_value::<AppliedProfileRecord>(stored).is_err(),
            "greenfield records must not silently drop semantic ownership evidence"
        );
    }
}
