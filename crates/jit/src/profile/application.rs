use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::domain::ProfileOrigin;
use crate::profile::ProfilePackageModel;
use crate::repository_state::FileMode;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;

/// Whether an application published its changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileApplicationStatus {
    /// Repository targets and audit state were already exact.
    Unchanged,
    /// A rehearsal found a publication it would make.
    WouldApply,
    /// A redo record reached its durable commit point.
    Applied,
}

/// Collection key of every profile command's list envelope.
pub enum ProfilesKey {}

impl CollectionKey for ProfilesKey {
    const KEY: &'static str = "profiles";
}

/// The list envelope every profile command returns, with entries in the
/// command's documented order.
pub type ProfileCollection<T> = ListEnvelope<ProfilesKey, T>;

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
        self.items.last()
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
    /// say: it still names its profile and its version, and it claims no change
    /// and nothing a publication carried.
    ///
    /// A repeated selector observes a publication its first occurrence already
    /// accounts for, so this is what its second and later observations report.
    fn observe_without_publishing(&mut self);
}

impl SelectionObservation for ProfileApplyResult {
    fn profile_id(&self) -> &str {
        &self.id
    }

    fn observe_without_publishing(&mut self) {
        self.status = ProfileApplicationStatus::Unchanged;
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
        self.refusal = None;
    }
}

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
}

/// Count-wrapped, occurrence-ordered package inspection response.
pub type ProfileShowResult = ProfileCollection<ProfileShowEntry>;

/// What one profile would do to one target or declaration it participates in.
///
/// The vocabulary is the whole decision a profile selection makes, so a reader
/// of a rehearsal and a reader of a difference report name the same outcomes, and a file and a declaration are named in one vocabulary rather
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
    /// Existing target is not declared and would be removed.
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
        reason: Option<String>,
    ) -> Self {
        Self {
            path,
            action,
            executable: mode == FileMode::Executable,
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
    /// Canonical semantic identity: the registry it lives in, its declaration
    /// kind and target, and its local name.
    pub identity: String,
    /// Planned operation.
    pub action: ProfileTargetAction,
    /// Why an unpublishable declaration cannot be published, absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Whether a preview found work to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePlanStatus {
    /// Package targets already match.
    Unchanged,
    /// Applying the plan would publish at least one change.
    WouldApply,
    /// The profile decided at least one target or declaration that cannot be
    /// published, or what it would publish fails final validation.
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
    /// Why final validation refuses what this profile would publish, absent
    /// otherwise. A refusal concerns the selection's proposed repository rather
    /// than one decision of this profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

impl ProfilePlanEntry {
    /// This entry under a selection final validation refuses. A profile
    /// publishing nothing takes no part in the refusal.
    pub(crate) fn refused_by(mut self, refusal: &str) -> Self {
        if self.status != ProfilePlanStatus::Unchanged {
            self.status = ProfilePlanStatus::WouldConflict;
            self.refusal = Some(refusal.to_string());
        }
        self
    }

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
        self.items
            .iter()
            .filter(|profile| profile.status == ProfilePlanStatus::WouldConflict)
    }
}

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
