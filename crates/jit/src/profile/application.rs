use crate::domain::identity::ProfileId;
use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::domain::relative_path::RootRelativePath;
use crate::profile::ProfilePackageModel;
use crate::repository_state::FileMode;
use schemars::JsonSchema;
use semver::Version;
use serde::Serialize;
use std::collections::BTreeSet;

/// What one profile command's run found or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileApplicationStatus {
    /// Repository targets and audit state were already exact.
    Unchanged,
    /// A read-only run found a publication it would make.
    WouldApply,
    /// A difference report found a target or declaration that cannot be
    /// published, or a proposed repository that fails final validation. A
    /// rehearsal fails instead of reporting this.
    WouldConflict,
    /// A redo record reached its durable commit point.
    Applied,
}

/// What one run over a profile selection does with its decision.
///
/// All three run the same computation. A rehearsal fails exactly where its
/// publication would, and only a report carries an unpublishable decision back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileRunMode {
    /// Refuse a selection holding an unpublishable decision, or publish it whole.
    Publish,
    /// Refuse an unpublishable decision and publish nothing.
    Rehearse,
    /// Return the decision, conflicts included, and publish nothing.
    Report,
}

/// Collection key of every profile command's list envelope.
pub enum ProfilesKey {}

impl CollectionKey for ProfilesKey {
    const KEY: &'static str = "profiles";
}

/// The list envelope every profile command returns, with entries in the
/// command's documented order.
pub type ProfileCollection<T> = ListEnvelope<ProfilesKey, T>;

/// One profile's answer to a run over a selection, whichever mode ran it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileApplyResult {
    /// Stable profile identifier.
    pub id: ProfileId,
    /// Semantic package version.
    #[schemars(with = "String")]
    pub version: Version,
    /// Worktree-relative directory the package bytes were read from.
    pub origin: RootRelativePath,
    /// What this run found or did for this profile.
    pub status: ProfileApplicationStatus,
    /// The targets this profile decides, sorted by path. An observation that
    /// publishes nothing decides none.
    pub targets: Vec<ProfileTargetChange>,
    /// The declarations this profile decides, sorted by semantic identity. An
    /// observation that publishes nothing decides none.
    pub contributions: Vec<ProfileContributionChange>,
    /// Why final validation refuses what this profile would publish, absent
    /// otherwise. A refusal concerns the selection's proposed repository rather
    /// than one decision of this profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

impl ProfileApplyResult {
    /// This entry under a selection final validation refuses. A profile
    /// publishing nothing takes no part in the refusal.
    pub(crate) fn refused_by(mut self, refusal: &str) -> Self {
        if self.status != ProfileApplicationStatus::Unchanged {
            self.status = ProfileApplicationStatus::WouldConflict;
            self.refusal = Some(refusal.to_string());
        }
        self
    }

    /// This entry as a repeated selector observes it: the publication its first
    /// occurrence accounts for is not claimed again.
    pub(crate) fn observed_again(mut self) -> Self {
        self.status = ProfileApplicationStatus::Unchanged;
        self.targets.clear();
        self.contributions.clear();
        self.refusal = None;
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

/// Result of one run over an ordered selection and its dependency closure.
///
/// Dependency-only packages appear once before the selected roots, followed by
/// the roots in selector occurrence order. A repeated root's later observations
/// report `unchanged` and carry no decisions.
pub type ProfileComposedApplyResult = ProfileCollection<ProfileApplyResult>;

impl ProfileCollection<ProfileApplyResult> {
    /// The result for the caller's last selected package occurrence.
    ///
    /// `None` only for an empty composition, which application does not produce.
    pub fn requested(&self) -> Option<&ProfileApplyResult> {
        self.items.last()
    }

    /// The profiles that decided a target or declaration that cannot be
    /// published, in report order.
    pub fn conflicted(&self) -> impl Iterator<Item = &ProfileApplyResult> {
        self.items
            .iter()
            .filter(|profile| profile.status == ProfileApplicationStatus::WouldConflict)
    }
}

/// One complete package inspection entry.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProfileShowEntry {
    /// Stable profile identifier.
    pub id: ProfileId,
    /// Semantic package version.
    #[schemars(with = "String")]
    pub version: Version,
    /// Canonical package model produced from the immutable manifest bytes.
    pub manifest: ProfilePackageModel,
    /// Worktree-relative directory the package bytes were read from.
    pub origin: RootRelativePath,
    /// Hash of the complete canonical package.
    pub package_hash: String,
    /// Repository paths the package writes.
    pub targets: BTreeSet<String>,
    /// Package file count, including `manifest.toml`.
    pub file_count: usize,
    /// Total package byte size.
    pub byte_size: usize,
}

/// Count-wrapped, occurrence-ordered package inspection response.
pub type ProfileShowResult = ProfileCollection<ProfileShowEntry>;

/// What one profile would do to one target or declaration it participates in.
///
/// The vocabulary is the whole decision a profile selection makes, so every run
/// names the same outcomes, and a file and a declaration are named in one
/// vocabulary rather than two. A rehearsal fails on [`Self::Conflict`] rather
/// than reporting it.
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

/// One packed package inside the shared profile collection.
///
/// The identity fields are the package's own, read from the directory that was
/// packed; the archive carries the same three, which is what an add holds the
/// arriving content against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfilePackEntry {
    /// Stable identifier the packed manifest declares.
    pub id: ProfileId,
    /// Semantic version the packed manifest declares.
    #[schemars(with = "String")]
    pub version: Version,
    /// Worktree-relative directory the package bytes were read from.
    pub origin: RootRelativePath,
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
    pub id: ProfileId,
    /// Semantic version the added manifest declares.
    #[schemars(with = "String")]
    pub version: Version,
    /// Worktree-relative directory the package will be readable at after
    /// publication.
    pub origin: RootRelativePath,
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
