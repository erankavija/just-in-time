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
    /// A rehearsal found a publication it would make.
    WouldApply,
    /// A recoverable transaction reached its durable commit point.
    Applied,
}

/// The one count-wrapped collection envelope returned by profile commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileCollection<T> {
    /// Number of entries in [`Self::profiles`].
    pub count: usize,
    /// Per-profile entries in the command's documented order.
    pub profiles: Vec<T>,
}

impl<T> ProfileCollection<T> {
    /// Wrap entries without changing their order.
    pub fn new(profiles: Vec<T>) -> Self {
        Self {
            count: profiles.len(),
            profiles,
        }
    }
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
    /// Where the package bytes were read from.
    pub origin: ProfileOrigin,
    /// Whether anything was published.
    pub status: ProfileApplicationStatus,
    /// Deterministic plan identity rebuilt under the write lock.
    pub plan_hash: String,
    /// Transaction identifier for an applied result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Non-fatal cleanup diagnostics.
    pub warnings: Vec<ProfileApplicationWarning>,
    /// Every file target this profile decided about.
    pub targets: Vec<ProfileTargetChange>,
    /// Every semantic declaration this profile decided about.
    pub contributions: Vec<ProfileContributionChange>,
}

/// Result of applying an ordered selection and its dependency closure.
///
/// Dependency-only packages appear once before the selected roots, followed by
/// the roots in selector occurrence order. A repeated root's later observations
/// carry no publication decisions. This is the same projection a rehearsal
/// returns, so both results compare entry for entry.
pub type ProfileComposedApplyResult = ProfileCollection<ProfileApplyResult>;

impl ProfileCollection<ProfileApplyResult> {
    /// The result for the caller's last selected package occurrence.
    ///
    /// `None` only for an empty composition, which application does not produce.
    pub fn requested(&self) -> Option<&ProfileApplyResult> {
        self.profiles.last()
    }
}

/// One profile's answer inside a selection's ordered observations.
///
/// A selection settles once and is then observed in the order the caller named
/// its roots, so the same profile can be observed more than once. Every field
/// of an answer is either about the profile itself — which every observation of
/// it shares — or about the publication that answer describes, which only the
/// observation carrying the publication may claim.
pub(crate) trait SelectionObservation {
    /// The profile this answer is about.
    fn profile_id(&self) -> &str;

    /// Reduce this answer to what an observation that publishes nothing may
    /// say: it still names its profile, its version, and the plan it was
    /// derived from, and it claims no change and nothing a publication carried.
    ///
    /// A repeated selector observes a transaction its first occurrence already
    /// accounts for, so this is what its second and later observations report.
    fn observe_without_publishing(&mut self);
}

impl SelectionObservation for ProfileApplyResult {
    fn profile_id(&self) -> &str {
        &self.id
    }

    fn observe_without_publishing(&mut self) {
        self.status = ProfileApplicationStatus::Unchanged;
        self.transaction_id = None;
        self.warnings.clear();
        self.targets.clear();
        self.contributions.clear();
    }
}

impl SelectionObservation for ProfilePlanEntry {
    fn profile_id(&self) -> &str {
        &self.id
    }

    fn observe_without_publishing(&mut self) {
        self.status = ProfilePlanStatus::Unchanged;
        self.targets.clear();
        self.contributions.clear();
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
pub type ProfileListResult = ProfileCollection<ProfileSummary>;

/// One complete package inspection entry.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProfileShowEntry {
    /// Stable profile identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
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
pub type ProfileShowResult = ProfileCollection<ProfileShowEntry>;

/// What one profile would do to one target or declaration it participates in.
///
/// The vocabulary is the whole three-way decision a profile selection makes, so
/// a reader of a rehearsal and a reader of a difference report name the same
/// outcomes, and a file and a declaration are named in one vocabulary rather
/// than two. A rehearsal fails on [`Self::Conflict`] rather than reporting it,
/// which is the only difference between the two surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileTargetAction {
    /// Existing bytes and mode already match.
    Unchanged,
    /// Target is absent and would be created.
    Create,
    /// Existing target would be replaced by the package projection.
    Update,
    /// The profile stopped contributing the target and its value survives.
    Retain,
    /// The profile stopped contributing unchanged, solely owned content.
    Remove,
    /// The target cannot be published, for the reason the change carries.
    Conflict,
}

/// One deterministic profile target decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileTargetChange {
    /// Repository-relative target path.
    pub path: String,
    /// Planned operation.
    pub action: ProfileTargetAction,
    /// Platform-neutral file-mode intent.
    pub executable: bool,
    /// Packages whose applied records claim this target, in package-id order.
    ///
    /// An empty list states that no package owns the target, so an adopter
    /// reading a conflict knows whether to edit its own content or resolve a
    /// package's claim; more than one entry names every owner of shared
    /// content.
    pub owners: Vec<String>,
    /// Why an unpublishable target cannot be published, absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProfileTargetChange {
    /// Construct a public target projection from canonical repository-state
    /// vocabulary.
    pub(crate) fn new(
        path: String,
        action: ProfileTargetAction,
        mode: FileMode,
        owners: Vec<String>,
        reason: Option<String>,
    ) -> Self {
        Self {
            path,
            action,
            executable: mode == FileMode::Executable,
            owners,
            reason,
        }
    }
}

/// One deterministic profile declaration decision.
///
/// A package claims semantic declarations as well as files, and composes them
/// by identity rather than by the registry file that holds them, so a reader
/// inspecting what a selection would do reads them in the same vocabulary a
/// target decision uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileContributionChange {
    /// Canonical semantic identity, spelled the way `jit profile validate`
    /// names one: the registry it lives in, its declaration kind and target,
    /// and its local name.
    pub identity: String,
    /// Planned operation.
    pub action: ProfileTargetAction,
    /// Packages whose applied records claim this declaration, in package-id
    /// order.
    ///
    /// An empty list states that no package owns the declaration, so it is the
    /// repository's own; more than one entry names every owner of a shared
    /// declaration.
    pub owners: Vec<String>,
    /// Why an unpublishable declaration cannot be published, absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Whether a preview found work to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePlanStatus {
    /// Package targets and installed provenance already match.
    Unchanged,
    /// Applying the plan would publish at least one change.
    WouldApply,
    /// The profile decided at least one target or declaration that cannot be
    /// published.
    ///
    /// A rehearsal of a publication fails before it can report this; a
    /// difference report states it, which is what lets an adopter inspect the
    /// decision before publishing it.
    WouldConflict,
}

/// One deterministic, non-mutating profile application preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePlanEntry {
    /// Stable profile identifier.
    pub id: String,
    /// Semantic package version.
    pub version: String,
    /// Where the package bytes were read from.
    pub origin: ProfileOrigin,
    /// Whether execution would publish this profile.
    pub status: ProfilePlanStatus,
    /// Identity of the complete canonical repository materialization plan this
    /// entry was derived from.
    pub plan_hash: String,
    /// The targets this profile decides, sorted by path.
    ///
    /// An observation that would publish nothing decides no target, so a
    /// repeated selector's later observations carry none.
    pub targets: Vec<ProfileTargetChange>,
    /// The declarations this profile decides, sorted by semantic identity.
    ///
    /// An observation that would publish nothing decides no declaration, so a
    /// repeated selector's later observations carry none.
    pub contributions: Vec<ProfileContributionChange>,
}

impl ProfilePlanEntry {
    /// The targets this profile decided `action` about, in report order.
    pub fn decided(
        &self,
        action: ProfileTargetAction,
    ) -> impl Iterator<Item = &ProfileTargetChange> {
        self.targets
            .iter()
            .filter(move |target| target.action == action)
    }

    /// The declarations this profile decided `action` about, in report order.
    pub fn decided_contributions(
        &self,
        action: ProfileTargetAction,
    ) -> impl Iterator<Item = &ProfileContributionChange> {
        self.contributions
            .iter()
            .filter(move |contribution| contribution.action == action)
    }
}

/// Count-wrapped profile application previews.
pub type ProfilePlanResult = ProfileCollection<ProfilePlanEntry>;

impl ProfileCollection<ProfilePlanEntry> {
    /// The profiles that decided a target or declaration that cannot be
    /// published, in report order.
    pub fn conflicted(&self) -> impl Iterator<Item = &ProfilePlanEntry> {
        self.profiles
            .iter()
            .filter(|profile| profile.status == ProfilePlanStatus::WouldConflict)
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
    fn test_installed_record_rejects_pre_release_embedded_origin_through_strict_decoding() {
        let mut stored: serde_json::Value =
            serde_json::from_slice(&record_at("profiles/example").to_bytes().unwrap()).unwrap();
        stored["origin"] = serde_json::json!({ "source": "embedded" });

        assert!(
            serde_json::from_value::<AppliedProfileRecord>(stored).is_err(),
            "the retired pre-release source tag must fail at the ordinary strict wire boundary"
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

/// One package-tree capture inside the shared profile collection.
///
/// [`Self::targets`] carries every path the capture decided about, including
/// removals, so an adopter sees what the destination stopped carrying as well
/// as what it now carries. [`Self::contributions`] reports the declarations the
/// captured manifest carries through the same decision vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileCaptureEntry {
    /// Stable identifier the captured manifest declares.
    pub id: String,
    /// Semantic version the captured manifest declares.
    pub version: String,
    /// Where the source package bytes were read from.
    pub origin: ProfileOrigin,
    /// Hash of the complete captured package.
    pub package_hash: String,
    /// Repository-relative package directory the capture read.
    pub source: String,
    /// Repository-relative directory the tree was published at.
    pub destination: String,
    /// Captured package file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total captured package byte size.
    pub byte_size: usize,
    /// Whether the capture published a transaction.
    pub status: ProfileApplicationStatus,
    /// Every path the capture decided about, in canonical path order.
    pub targets: Vec<ProfileTargetChange>,
    /// Every contribution the capture decided about, in declaration order.
    pub contributions: Vec<ProfileContributionChange>,
}

pub type ProfileCaptureResult = ProfileCollection<ProfileCaptureEntry>;

/// One packed package inside the shared profile collection.
///
/// The identity fields are the package's own, read from the directory that was
/// packed; the archive carries the same three, which is what an add holds the
/// arriving content against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePackEntry {
    /// Stable identifier the packed manifest declares.
    pub id: String,
    /// Semantic version the packed manifest declares.
    pub version: String,
    /// Where the package bytes were read from.
    pub origin: ProfileOrigin,
    /// Identity digest of the packed package.
    pub package_hash: String,
    /// Repository-relative package directory that was packed.
    pub source: String,
    /// Path the archive was written to, as the invocation named it.
    pub archive: String,
    /// Packed package file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total packed package byte size.
    pub byte_size: usize,
    /// Size of the written archive.
    pub archive_bytes: u64,
    /// The archive-file publication decision.
    pub targets: Vec<ProfileTargetChange>,
    /// Whether the archive was published or only rehearsed.
    pub status: ProfileApplicationStatus,
}

pub type ProfilePackResult = ProfileCollection<ProfilePackEntry>;

/// One added package inside the shared profile collection.
///
/// The identity fields are recomputed from the extracted content rather than
/// read from the archive, so they describe the package that was published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileAddEntry {
    /// Stable identifier the added manifest declares.
    pub id: String,
    /// Semantic version the added manifest declares.
    pub version: String,
    /// Where the package will be readable after publication.
    pub origin: ProfileOrigin,
    /// Identity digest recomputed from the added content.
    pub package_hash: String,
    /// Path the archive was read from, as the invocation named it.
    pub archive: String,
    /// Repository-relative directory the package was published at.
    pub destination: String,
    /// Added package file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total added package byte size.
    pub byte_size: usize,
    /// Every package-tree path the add decided about.
    pub targets: Vec<ProfileTargetChange>,
    /// Whether the package tree was published or only rehearsed.
    pub status: ProfileApplicationStatus,
}

pub type ProfileAddResult = ProfileCollection<ProfileAddEntry>;
