use crate::domain::ProfileOrigin;
use crate::profile::ProfileManifest;
use crate::repository_state::FileMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Minimal repository-local provenance for one installed profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileRecord {
    /// Stable profile package identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Package discovery origin.
    pub origin: ProfileOrigin,
    /// Hash of the complete canonical package.
    pub package_hash: String,
    /// Package contribution hashes keyed by repository target.
    pub target_hashes: BTreeMap<String, String>,
}

impl AppliedProfileRecord {
    /// Encode the stable installed-record image.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

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

/// One embedded profile exposed by `jit profile list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileSummary {
    /// Stable package identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Package discovery origin.
    pub origin: ProfileOrigin,
    /// Compatible JIT version requirement authored by the manifest.
    pub jit: String,
    /// Whether stored provenance exactly names this embedded package version and hash.
    pub applied: bool,
}

/// Count-wrapped profile-list response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileListResult {
    /// Number of profiles in [`Self::profiles`].
    pub count: usize,
    /// Embedded profiles sorted by stable ID.
    pub profiles: Vec<ProfileSummary>,
}

/// Complete embedded package inspection response.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProfileShowResult {
    /// Parsed immutable manifest.
    pub manifest: ProfileManifest,
    /// Package discovery origin.
    pub origin: ProfileOrigin,
    /// Hash of the complete canonical package.
    pub package_hash: String,
    /// Per-target package hashes.
    pub target_hashes: BTreeMap<String, String>,
    /// Embedded file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total embedded byte size.
    pub byte_size: usize,
    /// Stored provenance record when the selected repository has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<AppliedProfileRecord>,
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

/// Deterministic, non-mutating profile application preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePlanResult {
    /// Stable profile identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Whether execution would publish.
    pub status: ProfilePlanStatus,
    /// Stable identity of the package-target plan.
    pub plan_hash: String,
    /// Every package target, sorted by path.
    pub targets: Vec<ProfileTargetChange>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installed_record_is_minimal_stable_json() {
        let record = AppliedProfileRecord {
            id: "example".to_string(),
            version: "1.0.0".to_string(),
            origin: ProfileOrigin::Embedded,
            package_hash: "package".to_string(),
            target_hashes: BTreeMap::from([("docs/example.md".to_string(), "target".to_string())]),
        };
        let value: serde_json::Value = serde_json::from_slice(&record.to_bytes().unwrap()).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["id", "origin", "package_hash", "target_hashes", "version"]
        );
    }
}
