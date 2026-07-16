use crate::domain::{Event, ProfileOrigin};
use crate::profile::{ProfileManifest, ProjectedFileMode};
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
    /// Whether an exact installed record exists in the selected repository.
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
    /// Exact installed record when the selected repository has one.
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
    /// Construct a public target projection from internal planner vocabulary.
    pub(crate) fn new(path: String, action: ProfileTargetAction, mode: ProjectedFileMode) -> Self {
        Self {
            path,
            action,
            executable: mode == ProjectedFileMode::Executable,
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

/// Construct the exact next append-only event-log image.
///
/// Every prefix byte is retained. A non-empty prefix lacking a trailing
/// newline receives exactly one separator before the serialized event.
pub fn append_profile_event_image(
    prefix: &[u8],
    event: &Event,
) -> Result<Vec<u8>, serde_json::Error> {
    let event_bytes = serde_json::to_vec(event)?;
    let separator = usize::from(!prefix.is_empty() && !prefix.ends_with(b"\n"));
    let mut image = Vec::with_capacity(prefix.len() + separator + event_bytes.len() + 1);
    image.extend_from_slice(prefix);
    if separator == 1 {
        image.push(b'\n');
    }
    image.extend_from_slice(&event_bytes);
    image.push(b'\n');
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Event;

    fn event() -> Event {
        Event::new_profile_applied(
            "example".to_string(),
            "1.0.0".to_string(),
            ProfileOrigin::Embedded,
            "package".to_string(),
            BTreeMap::from([("docs/example.md".to_string(), "target".to_string())]),
            false,
        )
    }

    #[test]
    fn test_event_image_preserves_empty_newline_torn_and_multiple_prefixes() {
        let event = event();
        let serialized = serde_json::to_vec(&event).unwrap();
        for (prefix, separator) in [
            (b"".as_slice(), b"".as_slice()),
            (b"{\"old\":1}\n".as_slice(), b"".as_slice()),
            (b"{\"torn\":".as_slice(), b"\n".as_slice()),
            (
                b"{\"first\":1}\n{\"second\":2}\n".as_slice(),
                b"".as_slice(),
            ),
        ] {
            let image = append_profile_event_image(prefix, &event).unwrap();
            let mut expected = prefix.to_vec();
            expected.extend_from_slice(separator);
            expected.extend_from_slice(&serialized);
            expected.push(b'\n');
            assert_eq!(image, expected);
        }
    }

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
