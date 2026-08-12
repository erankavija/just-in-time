//! Profile-selection materialization: compose a profile package's canonical claims
//! (declaration-overlay registry edits, exact assets, and managed regions) plus the
//! configured projections those declarations imply into the exact set of
//! profile-owned targets.
//!
//! `repository_state` owns this composition; the profile package produces the
//! neutral [`ProfileClaims`] and the command captures the base image and applies the
//! resulting delta. The applied record carries the profile layer's canonical
//! resolved-variable provenance, but package parsing, storage, and command code
//! remain outside this module; the typed aggregate profile-selection
//! materialization request sits between them.

use std::collections::{BTreeMap, BTreeSet};

use super::materialize::{
    assemble_config, compose_configured_projections, serialized_default_ruleset,
};
use super::{
    apply_overlay, compose_managed_documents, declarations_from_image, FileMode,
    ManagedDocumentClaim, ProducerError, ProfileRegistryParseError, RegionPlacement,
    RepositoryAction, RepositoryEntry, RepositoryImage, RepositoryStateError, TargetClaim,
    VirtualPath,
};
use crate::config::{ProjectionKinds, ProjectionMode, ProjectionStyle};
use crate::domain::ProfileOrigin;
use crate::profile::{
    decide_three_way, ProfileId, RegionId, ResolvedVariables, ThreeWayDecision, ThreeWayInput,
    ThreeWayValue,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

/// A semantic contribution to one JIT registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Contribution {
    Scalar {
        target: ScalarTarget,
        value: String,
    },
    MapEntry {
        target: MapEntryTarget,
        identity: String,
        value: JsonValue,
    },
    SetString {
        target: SetStringTarget,
        value: String,
    },
    KeyedArray {
        target: KeyedArrayTarget,
        identity: String,
        value: JsonValue,
    },
    Projection {
        name: String,
        value: CompleteProjectionConfig,
    },
}

impl Contribution {
    pub fn registry_path(&self) -> &'static str {
        match self {
            Self::Scalar { .. }
            | Self::MapEntry { .. }
            | Self::SetString { .. }
            | Self::Projection { .. } => ".jit/config.toml",
            Self::KeyedArray { target, .. } => target.registry_path(),
        }
    }

    /// Return the canonical identity of the one semantic declaration this
    /// contribution supplies.
    ///
    /// The identity includes the registry and declaration target, so equal names
    /// in distinct registry tables remain separate contributions. It is a typed
    /// semantic protocol, never a spelling derived from a Rust `Debug` impl.
    pub fn semantic_identity(&self) -> ContributionIdentity {
        let target = match self {
            Self::Scalar { target, .. } => ContributionIdentityTarget::Scalar { target: *target },
            Self::MapEntry {
                target, identity, ..
            } => ContributionIdentityTarget::MapEntry {
                target: *target,
                name: identity.clone(),
            },
            Self::SetString { target, value } => ContributionIdentityTarget::SetString {
                target: *target,
                value: value.clone(),
            },
            Self::KeyedArray {
                target, identity, ..
            } => ContributionIdentityTarget::KeyedArray {
                target: *target,
                name: identity.clone(),
            },
            Self::Projection { name, .. } => {
                ContributionIdentityTarget::Projection { name: name.clone() }
            }
        };
        ContributionIdentity {
            registry: match self {
                Self::Scalar { .. }
                | Self::MapEntry { .. }
                | Self::SetString { .. }
                | Self::Projection { .. } => ContributionRegistry::Config,
                Self::KeyedArray { target, .. } => match target {
                    KeyedArrayTarget::Gates => ContributionRegistry::Gates,
                    KeyedArrayTarget::Invariants => ContributionRegistry::Invariants,
                    KeyedArrayTarget::Rules => ContributionRegistry::Rules,
                    KeyedArrayTarget::Templates => ContributionRegistry::Templates,
                },
            },
            target,
        }
    }
}

/// Canonical semantic identity of one contribution within a registry.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ContributionIdentity {
    /// Registry in which this declaration is defined.
    pub registry: ContributionRegistry,
    /// Declaration target and its local identity.
    pub target: ContributionIdentityTarget,
}

/// Stable vocabulary of profile-contribution registries.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ContributionRegistry {
    Config,
    Gates,
    Invariants,
    Rules,
    Templates,
}

impl ContributionRegistry {
    /// Repository-relative path of the registry file this vocabulary names.
    pub fn path(self) -> &'static str {
        match self {
            Self::Config => ".jit/config.toml",
            Self::Gates => ".jit/gates.toml",
            Self::Invariants => ".jit/invariants.toml",
            Self::Rules => ".jit/rules.toml",
            Self::Templates => ".jit/templates.toml",
        }
    }
}

/// Stable local target of a contribution semantic identity.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ContributionIdentityTarget {
    Scalar {
        target: ScalarTarget,
    },
    MapEntry {
        target: MapEntryTarget,
        name: String,
    },
    SetString {
        target: SetStringTarget,
        value: String,
    },
    KeyedArray {
        target: KeyedArrayTarget,
        name: String,
    },
    Projection {
        name: String,
    },
}

impl std::fmt::Display for ContributionIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:", self.registry.path())?;
        match &self.target {
            ContributionIdentityTarget::Scalar { target } => write!(formatter, "scalar:{target}"),
            ContributionIdentityTarget::MapEntry { target, name } => {
                write!(formatter, "map-entry:{target}:{name}")
            }
            ContributionIdentityTarget::SetString { target, value } => {
                write!(formatter, "set-string:{target}:{value}")
            }
            ContributionIdentityTarget::KeyedArray { target, name } => {
                write!(formatter, "keyed-array:{target}:{name}")
            }
            ContributionIdentityTarget::Projection { name } => {
                write!(formatter, "projection:{name}")
            }
        }
    }
}

/// One resolved contribution supplied by a profile package.
///
/// Callers must construct this only from a resolved package model; comparison is
/// intentionally over the resolved contribution rather than package source bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileContributionClaim {
    /// Package that contributes the definition.
    pub(crate) package_id: ProfilePackageId,
    /// Fully resolved semantic definition.
    pub(crate) contribution: Contribution,
}

/// A prior semantic definition at the composition boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum ExistingContributionClaim {
    /// A repository-authored definition with no package provenance.
    Repository(Contribution),
    /// A definition owned by an installed package.
    Package(ProfileContributionClaim),
}

/// A definition that survived semantic composition with all package owners.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedContribution {
    /// Canonical semantic identity shared by the contributors.
    pub identity: ContributionIdentity,
    /// The shared resolved definition.
    pub definition: Contribution,
    /// Every package owning `definition`, sorted by package identity.
    pub owners: Vec<ProfilePackageId>,
}

/// Origin of one differing definition in a semantic contribution conflict.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContributionConflictOwner {
    /// The repository itself authored one of the conflicting definitions.
    Repository,
    /// An installed or selected package authored one of the definitions.
    Package(ProfilePackageId),
}

impl std::fmt::Display for ContributionConflictOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository => formatter.write_str("the repository"),
            Self::Package(package_id) => write!(formatter, "package {package_id}"),
        }
    }
}

/// A semantic identity has more than one resolved definition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("profile contribution '{identity}' conflicts between {owners:?}")]
pub struct ContributionCompositionConflict {
    /// Shared semantic identity with incompatible definitions.
    pub identity: ContributionIdentity,
    /// Every repository or package owner of a conflicting definition, sorted
    /// independently of selector occurrence.
    pub owners: Vec<ContributionConflictOwner>,
}

enum ContributionClaimSource {
    Repository,
    Package(ProfilePackageId),
}

struct CompositionInput {
    source: ContributionClaimSource,
    contribution: Contribution,
}

/// Compose resolved package claims with existing semantic claims.
///
/// Existing claims owned by a selected package are excluded before comparison:
/// its new resolved definition replaces its own prior identity claim rather
/// than conflicting with it. Different package definitions for an identity
/// produce one order-independent error naming all owners. Repository-authored
/// definitions remain distinct from package-owned definitions in that error.
pub fn compose_resolved_contributions(
    existing: impl IntoIterator<Item = ExistingContributionClaim>,
    candidates: impl IntoIterator<Item = ProfileContributionClaim>,
) -> Result<Vec<ComposedContribution>, ContributionCompositionConflict> {
    let candidates = candidates.into_iter().collect::<Vec<_>>();
    let selected_packages = candidates
        .iter()
        .map(|claim| claim.package_id.clone())
        .collect::<BTreeSet<_>>();
    let existing = existing.into_iter().filter_map(|claim| match claim {
        ExistingContributionClaim::Repository(contribution) => Some(CompositionInput {
            source: ContributionClaimSource::Repository,
            contribution,
        }),
        ExistingContributionClaim::Package(claim)
            if selected_packages.contains(&claim.package_id) =>
        {
            None
        }
        ExistingContributionClaim::Package(claim) => Some(CompositionInput {
            source: ContributionClaimSource::Package(claim.package_id),
            contribution: claim.contribution,
        }),
    });
    let mut positions = BTreeMap::<ContributionIdentity, usize>::new();
    let mut grouped = Vec::<(ContributionIdentity, Vec<CompositionInput>)>::new();
    for claim in existing.chain(candidates.into_iter().map(|claim| CompositionInput {
        source: ContributionClaimSource::Package(claim.package_id),
        contribution: claim.contribution,
    })) {
        let identity = claim.contribution.semantic_identity();
        if let Some(index) = positions.get(&identity) {
            grouped[*index].1.push(claim);
        } else {
            positions.insert(identity.clone(), grouped.len());
            grouped.push((identity, vec![claim]));
        }
    }

    grouped
        .into_iter()
        .filter_map(|(identity, claims)| compose_contribution_identity(identity, claims))
        .collect()
}

fn compose_contribution_identity(
    identity: ContributionIdentity,
    claims: Vec<CompositionInput>,
) -> Option<Result<ComposedContribution, ContributionCompositionConflict>> {
    let first = claims.first()?;
    let definition = first.contribution.clone();
    if claims.iter().any(|claim| claim.contribution != definition) {
        let owners = claims
            .iter()
            .map(|claim| match &claim.source {
                ContributionClaimSource::Repository => ContributionConflictOwner::Repository,
                ContributionClaimSource::Package(package_id) => {
                    ContributionConflictOwner::Package(package_id.clone())
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Some(Err(ContributionCompositionConflict { identity, owners }));
    }
    let owners = claims
        .into_iter()
        .filter_map(|claim| match claim.source {
            ContributionClaimSource::Repository => None,
            ContributionClaimSource::Package(package_id) => Some(package_id),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Some(Ok(ComposedContribution {
        identity,
        definition,
        owners,
    }))
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MapEntryTarget {
    TypeHierarchyTypes,
    LabelAssociations,
    Namespaces,
    ItemKinds,
}

impl MapEntryTarget {
    /// The `.jit/config.toml` table path whose keys are this target's entry
    /// identities.
    pub(crate) fn table_path(self) -> &'static [&'static str] {
        match self {
            Self::TypeHierarchyTypes => &["type_hierarchy", "types"],
            Self::LabelAssociations => &["type_hierarchy", "label_associations"],
            Self::Namespaces => &["namespaces"],
            Self::ItemKinds => &["item_kinds"],
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::TypeHierarchyTypes => "type-hierarchy-types",
            Self::LabelAssociations => "label-associations",
            Self::Namespaces => "namespaces",
            Self::ItemKinds => "item-kinds",
        }
    }
}

impl std::fmt::Display for MapEntryTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.semantic_name())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ScalarTarget {
    DocumentationDevelopmentRoot,
    DocumentationArchiveRoot,
    ValidationStrictness,
    ValidationDefaultType,
}

impl ScalarTarget {
    /// The `.jit/config.toml` table and key this target's value is written at.
    pub(crate) fn config_path(self) -> (&'static str, &'static str) {
        match self {
            Self::DocumentationDevelopmentRoot => ("documentation", "development_root"),
            Self::DocumentationArchiveRoot => ("documentation", "archive_root"),
            Self::ValidationStrictness => ("validation", "strictness"),
            Self::ValidationDefaultType => ("validation", "default_type"),
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::DocumentationDevelopmentRoot => "documentation-development-root",
            Self::DocumentationArchiveRoot => "documentation-archive-root",
            Self::ValidationStrictness => "validation-strictness",
            Self::ValidationDefaultType => "validation-default-type",
        }
    }
}

impl std::fmt::Display for ScalarTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.semantic_name())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SetStringTarget {
    StrategicTypes,
    DocumentationManagedPaths,
    DocumentationPermanentPaths,
    DocumentationIssueScopedAreas,
}

impl SetStringTarget {
    /// The `.jit/config.toml` table and key holding this target's array.
    pub(crate) fn config_path(self) -> (&'static str, &'static str) {
        match self {
            Self::StrategicTypes => ("type_hierarchy", "strategic_types"),
            Self::DocumentationManagedPaths => ("documentation", "managed_paths"),
            Self::DocumentationPermanentPaths => ("documentation", "permanent_paths"),
            Self::DocumentationIssueScopedAreas => ("documentation", "issue_scoped_areas"),
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::StrategicTypes => "strategic-types",
            Self::DocumentationManagedPaths => "documentation-managed-paths",
            Self::DocumentationPermanentPaths => "documentation-permanent-paths",
            Self::DocumentationIssueScopedAreas => "documentation-issue-scoped-areas",
        }
    }
}

impl std::fmt::Display for SetStringTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.semantic_name())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum KeyedArrayTarget {
    Gates,
    Invariants,
    Rules,
    Templates,
}

impl KeyedArrayTarget {
    pub(crate) fn registry_path(self) -> &'static str {
        match self {
            Self::Gates => ".jit/gates.toml",
            Self::Invariants => ".jit/invariants.toml",
            Self::Rules => ".jit/rules.toml",
            Self::Templates => ".jit/templates.toml",
        }
    }

    pub(crate) fn identity_field(self) -> &'static str {
        match self {
            Self::Gates => "key",
            Self::Invariants => "id",
            Self::Rules | Self::Templates => "name",
        }
    }

    pub(crate) fn array_name(self) -> &'static str {
        match self {
            Self::Gates => "gates",
            Self::Invariants => "invariants",
            Self::Rules => "rules",
            Self::Templates => "template",
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::Gates => "gates",
            Self::Invariants => "invariants",
            Self::Rules => "rules",
            Self::Templates => "templates",
        }
    }
}

impl std::fmt::Display for KeyedArrayTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.semantic_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteProjectionConfig {
    pub kind: ProjectionKinds,
    pub mode: ProjectionMode,
    pub target: String,
    pub style: ProjectionStyle,
}

#[derive(Clone)]
pub struct ProfileAssetClaim {
    pub claim: TargetClaim,
    pub bytes: Vec<u8>,
    pub mode: FileMode,
    pub replace_owned: bool,
}

#[derive(Clone)]
pub struct ProfileRegionClaim {
    pub claim: TargetClaim,
    pub region_id: RegionId,
    pub content: Vec<u8>,
}

/// The source that already owns a conflicting repository declaration or target.
///
/// Repository-authored content is deliberately distinct from a package occupant:
/// an adopter needs to know whether it must change its own file or resolve a
/// package composition conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileConflictOccupant {
    /// The repository authored the existing declaration or target.
    Repository,
    /// An already-applied package authored the existing declaration or target.
    Package(ProfilePackageId),
}

impl std::fmt::Display for ProfileConflictOccupant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository => formatter.write_str("the repository"),
            Self::Package(id) => write!(formatter, "package {id}"),
        }
    }
}

/// Stable semantic identity of a profile package.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct ProfilePackageId(String);

impl ProfilePackageId {
    /// Construct an identity from a validated package manifest identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the package identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProfilePackageId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A package asset cannot replace an authored occupant it does not own.
#[derive(Debug, thiserror::Error)]
#[error(
    "profile package {candidate} asset target '{}' conflicts with {occupant} and contains differing bytes",
    .path.repository_relative()
)]
pub struct ProfileTargetConflictError {
    /// Conflicting canonical repository path.
    pub path: VirtualPath,
    /// Package whose asset is being applied.
    pub candidate: ProfilePackageId,
    /// Existing owner of the target.
    pub occupant: ProfileConflictOccupant,
}

/// A profile-owned target changed independently after its recorded base.
///
/// The values are domain-separated fingerprints: the durable provenance this
/// layer records, without turning provenance into configuration authority.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "profile package {owner} target '{}' diverged (base: {base:?}, current: {current:?}, candidate: {candidate:?})",
    .target.repository_relative()
)]
pub struct ProfileThreeWayConflictError {
    /// Canonical repository target whose replacement was refused.
    pub target: VirtualPath,
    /// Package whose recorded claim is being replaced.
    pub owner: ProfilePackageId,
    /// Fingerprint recorded when this package last published the target.
    pub base: ThreeWayValue<ProfileBaseFingerprint>,
    /// Fingerprint observed in the captured repository image.
    pub current: ThreeWayValue<ProfileBaseFingerprint>,
    /// Fingerprint resolved from the candidate package.
    pub candidate: ThreeWayValue<ProfileBaseFingerprint>,
}

/// A profile package's contribution to a repository, in canonical repository-state
/// vocabulary. Produced by `profile::package` from the immutable manifest and
/// consumed only by [`compose_profile_targets`].
#[derive(Clone)]
pub struct ProfileClaims {
    /// Package that authored these claims.
    pub package_id: ProfilePackageId,
    /// Fully resolved semantic contributions that retain their package owner.
    pub contributions: Vec<ProfileContributionClaim>,
    pub assets: Vec<ProfileAssetClaim>,
    pub regions: Vec<ProfileRegionClaim>,
}

impl ProfileClaims {
    pub(crate) fn target_paths(&self) -> Result<Vec<VirtualPath>, super::RepositoryLayoutError> {
        let mut paths = self
            .contributions
            .iter()
            .map(|claim| claim.contribution.registry_target())
            .collect::<Result<Vec<_>, _>>()?;
        paths.extend(self.assets.iter().map(|asset| asset.claim.target().clone()));
        paths.extend(
            self.regions
                .iter()
                .map(|region| region.claim.target().clone()),
        );
        Ok(paths)
    }
}

impl Contribution {
    fn registry_target(&self) -> Result<VirtualPath, super::RepositoryLayoutError> {
        let relative = match self {
            Self::Scalar { .. }
            | Self::MapEntry { .. }
            | Self::SetString { .. }
            | Self::Projection { .. } => "config.toml",
            Self::KeyedArray { target, .. } => match target {
                KeyedArrayTarget::Gates => "gates.toml",
                KeyedArrayTarget::Invariants => "invariants.toml",
                KeyedArrayTarget::Rules => "rules.toml",
                KeyedArrayTarget::Templates => "templates.toml",
            },
        };
        VirtualPath::data(relative)
    }
}

/// The sole installed-profile provenance wire version accepted by ordinary
/// readers.
pub const APPLIED_PROFILE_RECORD_VERSION: u8 = 2;

/// A domain-separated SHA-256 fingerprint of one profile-owned base value.
///
/// The string representation is deliberately constrained at deserialization so
/// a record can never claim a digest this implementation could not have
/// produced. The domain is part of every preimage; semantic declarations,
/// static files, and managed regions therefore cannot collide merely because
/// their raw bytes happen to be equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct ProfileBaseFingerprint(String);

impl ProfileBaseFingerprint {
    /// Borrow the canonical lowercase hexadecimal digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_digest(digest: impl Into<String>) -> Result<Self, String> {
        let digest = digest.into();
        if digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(digest))
        } else {
            Err("expected 64 lowercase hexadecimal characters".to_string())
        }
    }
}

impl std::fmt::Display for ProfileBaseFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProfileBaseFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::from_digest(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// The typed target component of an asset or managed-region ownership claim.
///
/// A virtual repository path cannot deserialize independently of a repository
/// layout. This representation preserves its root discriminator and canonical
/// root-relative path so records stay portable while avoiding an unqualified
/// target string protocol.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct AppliedClaimTarget {
    /// Selected repository root containing the target.
    pub root: super::RepositoryRootClass,
    /// Canonical path below `root`.
    pub path: super::RootRelativePath,
    /// Mode that is part of the published content identity.
    pub mode: FileMode,
}

/// Typed target location of a managed-region identity. Unlike an asset, a
/// region's mode belongs to its base fingerprint rather than its identity.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct AppliedManagedRegionTarget {
    /// Selected repository root containing the managed region.
    pub root: super::RepositoryRootClass,
    /// Canonical path below `root`.
    pub path: super::RootRelativePath,
}

impl AppliedManagedRegionTarget {
    fn from_virtual_path(path: &VirtualPath) -> Self {
        Self {
            root: path.root_class(),
            path: path.relative().clone(),
        }
    }
}

impl AppliedClaimTarget {
    fn from_virtual_path(path: &VirtualPath, mode: FileMode) -> Self {
        Self {
            root: path.root_class(),
            path: path.relative().clone(),
            mode,
        }
    }
}

/// Canonical ownership identity for exactly one profile contribution.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AppliedProfileClaimIdentity {
    /// One semantic registry declaration. Its value is fingerprinted, never
    /// embedded in provenance.
    Semantic { identity: ContributionIdentity },
    /// One exact package asset and its mode.
    Asset { target: AppliedClaimTarget },
    /// One managed region, distinguished from a whole-file asset by its region
    /// identifier and fingerprinted over its source body and mode.
    ManagedRegion {
        target: AppliedManagedRegionTarget,
        region_id: RegionId,
    },
}

impl AppliedProfileClaimIdentity {
    /// The repository path this claim's value is read from.
    ///
    /// A semantic declaration is read from the registry its identity names, and
    /// an asset or managed region from the path it occupies. A capture that
    /// intends to compare a claim must cover this path, and
    /// [`claimed_target_state`] reads exactly it, so both sides of that
    /// arrangement derive the path here.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryLayoutError`](super::RepositoryLayoutError) when the
    /// stored target is not a canonical path below its recorded root.
    pub fn claimed_path(
        &self,
        layout: &super::RepositoryLayout,
    ) -> Result<VirtualPath, super::RepositoryLayoutError> {
        match self {
            Self::Semantic { identity } => {
                layout.classify_repository_relative(identity.registry.path())
            }
            Self::Asset { target } => VirtualPath::from_root(target.root, target.path.clone()),
            Self::ManagedRegion { target, .. } => {
                VirtualPath::from_root(target.root, target.path.clone())
            }
        }
    }
}

/// The repository-relative name of what one claim owns.
///
/// A semantic declaration is named by its own canonical identity, which already
/// begins with its registry path; an asset by the repository path it occupies;
/// and a managed region by its document and region id. Every message about a
/// claim — the profile check's report and repository-wide validation's finding —
/// names it this way, so an adopter reading either finds the same subject.
impl std::fmt::Display for AppliedProfileClaimIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Semantic { identity } => identity.fmt(formatter),
            Self::Asset { target } => {
                formatter.write_str(&super::repository_relative_path(target.root, &target.path))
            }
            Self::ManagedRegion { target, region_id } => write!(
                formatter,
                "{}#{region_id}",
                super::repository_relative_path(target.root, &target.path)
            ),
        }
    }
}

/// Provenance for one contribution a profile owns at its recorded base.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileClaim {
    /// Typed identity of the owned contribution.
    pub identity: AppliedProfileClaimIdentity,
    /// Domain-separated fingerprint of the contribution when it was published.
    pub base_fingerprint: ProfileBaseFingerprint,
    /// Preserve an adopted repository baseline if this becomes the last owner.
    pub retain_if_unowned: bool,
}

impl AppliedProfileClaim {
    fn semantic(
        contribution: &Contribution,
        retain_if_unowned: bool,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            identity: AppliedProfileClaimIdentity::Semantic {
                identity: contribution.semantic_identity(),
            },
            base_fingerprint: fingerprint_semantic_contribution(contribution)?,
            retain_if_unowned,
        })
    }

    fn asset(target: &VirtualPath, bytes: &[u8], mode: FileMode, retain_if_unowned: bool) -> Self {
        Self {
            identity: AppliedProfileClaimIdentity::Asset {
                target: AppliedClaimTarget::from_virtual_path(target, mode),
            },
            base_fingerprint: fingerprint_profile_asset(bytes, mode),
            retain_if_unowned,
        }
    }

    fn managed_region(
        target: &VirtualPath,
        region_id: RegionId,
        content: &[u8],
        mode: FileMode,
        retain_if_unowned: bool,
    ) -> Self {
        Self {
            identity: AppliedProfileClaimIdentity::ManagedRegion {
                target: AppliedManagedRegionTarget::from_virtual_path(target),
                region_id: region_id.clone(),
            },
            base_fingerprint: fingerprint_managed_region(&region_id, content, mode),
            retain_if_unowned,
        }
    }
}

/// Fingerprint one resolved semantic contribution under the v2 semantic domain.
pub fn fingerprint_semantic_contribution(
    contribution: &Contribution,
) -> Result<ProfileBaseFingerprint, serde_json::Error> {
    canonical_json_bytes(contribution)
        .map(|bytes| fingerprint_bytes(b"jit-profile-record-v2:semantic\0", &[bytes.as_slice()]))
}

fn fingerprint_bytes(domain: &[u8], fields: &[&[u8]]) -> ProfileBaseFingerprint {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    fields.iter().for_each(|field| {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    });
    // SHA-256 always produces this exact canonical spelling.
    ProfileBaseFingerprint(format!("{:x}", hasher.finalize()))
}

/// Fingerprint exact profile asset bytes and mode under the sole asset domain.
fn fingerprint_profile_asset(bytes: &[u8], mode: FileMode) -> ProfileBaseFingerprint {
    fingerprint_bytes(
        b"jit-profile-record-v2:asset\0",
        &[bytes, &file_mode_bytes(mode)],
    )
}

/// Fingerprint one managed region's published body and target mode under the
/// sole managed-region domain.
///
/// Publication records this over the body it splices in; the ownership check
/// recomputes it over the body the document currently holds, so both sides of
/// that comparison are the same construction.
fn fingerprint_managed_region(
    region_id: &RegionId,
    content: &[u8],
    mode: FileMode,
) -> ProfileBaseFingerprint {
    fingerprint_bytes(
        b"jit-profile-record-v2:managed-region\0",
        &[
            region_id.as_str().as_bytes(),
            content,
            &file_mode_bytes(mode),
        ],
    )
}

/// The exact delimiters one profile-owned managed region is published between.
///
/// Composition splices a region between these bytes and the ownership check
/// reads it back from between them, so the pair is spelled once here rather
/// than at each end of that round trip.
fn region_delimiters(region_id: &RegionId) -> (Vec<u8>, Vec<u8>) {
    (
        format!("<!-- jit:{region_id}:begin -->").into_bytes(),
        format!("<!-- jit:{region_id}:end -->").into_bytes(),
    )
}

fn file_mode_bytes(mode: FileMode) -> Vec<u8> {
    match mode {
        FileMode::Regular => b"regular".to_vec(),
        FileMode::Executable => b"executable".to_vec(),
    }
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonicalize_json(serde_json::to_value(value)?))
}

fn canonicalize_json(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(values) => {
            JsonValue::Array(values.into_iter().map(canonicalize_json).collect())
        }
        JsonValue::Object(values) => JsonValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}

/// Canonical repository-local provenance for one installed profile.
///
/// This record deliberately carries no registry definitions or effective
/// configuration. Registry loaders remain the sole source of behavior; claims
/// are only identities, historical fingerprints, and retention intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileRecord {
    /// Explicit current wire discriminator. Only `2` is accepted here.
    #[serde(deserialize_with = "deserialize_current_record_version")]
    pub record_version: u8,
    /// Stable package identity.
    pub id: ProfileId,
    /// Installed package version.
    pub version: String,
    /// Compatible JIT range authored by the package manifest.
    pub compatible_jit: String,
    /// Package discovery source.
    pub origin: ProfileOrigin,
    /// Digest of the complete package manifest and content.
    pub package_hash: String,
    /// Canonical public values and source kinds used to resolve this package.
    pub variables: ResolvedVariables,
    /// One sorted ownership claim for every semantic, asset, or managed-region
    /// contribution this profile published.
    #[serde(deserialize_with = "deserialize_applied_profile_claims")]
    pub claims: BTreeSet<AppliedProfileClaim>,
}

fn deserialize_applied_profile_claims<'de, D>(
    deserializer: D,
) -> Result<BTreeSet<AppliedProfileClaim>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let claims = Vec::<AppliedProfileClaim>::deserialize(deserializer)?;
    let identities = claims
        .iter()
        .map(|claim| claim.identity.clone())
        .collect::<BTreeSet<_>>();
    (identities.len() == claims.len())
        .then(|| claims.into_iter().collect())
        .ok_or_else(|| {
            serde::de::Error::custom("applied profile record contains duplicate claim identities")
        })
}

fn deserialize_current_record_version<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let record_version = u8::deserialize(deserializer)?;
    (record_version == APPLIED_PROFILE_RECORD_VERSION)
        .then_some(record_version)
        .ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unsupported applied profile record_version {record_version}; expected {APPLIED_PROFILE_RECORD_VERSION}"
            ))
        })
}

impl AppliedProfileRecord {
    /// Construct the only current installed-profile provenance representation.
    pub fn new(
        id: ProfileId,
        version: impl Into<String>,
        compatible_jit: impl Into<String>,
        origin: ProfileOrigin,
        package_hash: impl Into<String>,
        variables: ResolvedVariables,
        claims: BTreeSet<AppliedProfileClaim>,
    ) -> Self {
        Self {
            record_version: APPLIED_PROFILE_RECORD_VERSION,
            id,
            version: version.into(),
            compatible_jit: compatible_jit.into(),
            origin,
            package_hash: package_hash.into(),
            variables,
            claims,
        }
    }

    /// Whether immutable package provenance agrees, excluding mutable
    /// contribution ownership and repository content.
    pub fn matches_package_provenance(&self, expected: &Self) -> bool {
        self.record_version == APPLIED_PROFILE_RECORD_VERSION
            && self.id == expected.id
            && self.version == expected.version
            && self.compatible_jit == expected.compatible_jit
            && self.origin == expected.origin
            && self.package_hash == expected.package_hash
            && self.variables == expected.variables
    }

    /// Encode the stable current record image. Claims remain in identity order
    /// by construction because the record owns a `BTreeSet`.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// Verify that a persisted provenance record occupies the only canonical path
/// for its package identity. Ownership readers call this before trusting claims.
pub(crate) fn validate_applied_record_path(
    path: &VirtualPath,
    record: &AppliedProfileRecord,
) -> Result<(), ProducerError> {
    let expected = VirtualPath::data(format!("profiles/{}.json", record.id))?;
    (path == &expected)
        .then_some(())
        .ok_or_else(|| ProducerError::ProfileRecordPathMismatch {
            path: path.repository_relative(),
            id: record.id.to_string(),
            expected: expected.repository_relative(),
        })
}

/// Neutral profile package input consumed by the one materialization dispatcher.
#[derive(Clone)]
pub struct ProfileApplicationInput {
    /// Stable package identity carried from the resolved manifest.
    pub id: ProfileId,
    pub version: String,
    /// Compatible JIT range authored by this package's manifest.
    pub compatible_jit: String,
    pub package_hash: String,
    /// Exact public values and source kinds that produced `claims`.
    pub variables: ResolvedVariables,
    pub target_hashes: BTreeMap<String, String>,
    pub origin: ProfileOrigin,
    pub claims: ProfileClaims,
    /// Resolved candidates sharing one of this package's semantic identities.
    ///
    /// The command supplies this scoped selection or dependency-closure context
    /// before aggregate selection publication begins. It lets every affected record
    /// retain the same complete owner set without making unrelated package
    /// declarations part of this package's materialization.
    pub(crate) contribution_context: Vec<ProfileContributionClaim>,
    pub record_path: VirtualPath,
}

impl ProfileApplicationInput {
    /// Scope a selected package set to the semantic identities this input owns.
    pub(crate) fn with_contribution_context(
        mut self,
        candidates: &[ProfileContributionClaim],
    ) -> Self {
        let identities = self
            .claims
            .contributions
            .iter()
            .map(|claim| claim.contribution.semantic_identity())
            .collect::<BTreeSet<_>>();
        self.contribution_context = candidates
            .iter()
            .filter(|claim| identities.contains(&claim.contribution.semantic_identity()))
            .cloned()
            .collect();
        self
    }

    /// Whether this package contributes either authored input of the coupled
    /// default-rule/schema materialization.
    ///
    /// Such a package re-derives the default rules and the schemas they
    /// reference, so an application of it reaches the generated schema
    /// directory whether or not the package names a target under it.
    pub(crate) fn owns_default_rule_authority(&self) -> bool {
        self.target_hashes.contains_key(".jit/config.toml")
            || self.target_hashes.contains_key(".jit/rules.toml")
    }

    pub(super) fn record(
        &self,
        composition: &ProfileTargetComposition,
    ) -> Result<AppliedProfileRecord, serde_json::Error> {
        let semantic = self
            .claims
            .contributions
            .iter()
            .map(|claim| {
                let identity = AppliedProfileClaimIdentity::Semantic {
                    identity: claim.contribution.semantic_identity(),
                };
                AppliedProfileClaim::semantic(
                    &claim.contribution,
                    composition.retained_claims.contains(&identity),
                )
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let assets = self
            .claims
            .assets
            .iter()
            .map(|asset| {
                let (bytes, mode) =
                    composition
                        .targets
                        .get(asset.claim.target())
                        .ok_or_else(|| {
                            serde_json::Error::io(std::io::Error::other(
                                "profile composition omitted a declared asset target",
                            ))
                        })?;
                let identity = AppliedProfileClaimIdentity::Asset {
                    target: AppliedClaimTarget::from_virtual_path(asset.claim.target(), *mode),
                };
                Ok(AppliedProfileClaim::asset(
                    asset.claim.target(),
                    bytes,
                    *mode,
                    composition.retained_claims.contains(&identity),
                ))
            })
            .collect::<Result<BTreeSet<_>, serde_json::Error>>()?;
        let regions = self.claims.regions.iter().map(|region| {
            let mode = composition
                .targets
                .get(region.claim.target())
                .map(|(_, mode)| *mode)
                .unwrap_or(FileMode::Regular);
            let identity = AppliedProfileClaimIdentity::ManagedRegion {
                target: AppliedManagedRegionTarget::from_virtual_path(region.claim.target()),
                region_id: region.region_id.clone(),
            };
            AppliedProfileClaim::managed_region(
                region.claim.target(),
                region.region_id.clone(),
                &region.content,
                mode,
                composition.retained_claims.contains(&identity),
            )
        });
        Ok(AppliedProfileRecord::new(
            self.id.clone(),
            self.version.clone(),
            self.compatible_jit.clone(),
            self.origin.clone(),
            self.package_hash.clone(),
            self.variables.clone(),
            semantic.into_iter().chain(assets).chain(regions).collect(),
        ))
    }
}

/// Enumerate paths implied by a profile's proposed declarations before rendering.
pub(crate) fn profile_capture_closure(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let record_paths = applied_profile_record_paths(base)?;
    let replacement_paths = replacement_asset_paths(base, &profile.claims.package_id)?;
    let mut required_paths = record_paths.clone();
    required_paths.extend(replacement_paths);
    if required_paths
        .iter()
        .any(|path| !base.capture_spec().contains_path(path))
    {
        return Ok(required_paths);
    }
    let composed = compose_profile_contributions(base, &profile.contribution_context)?;
    let composed = owned_composed_contributions(composed, &profile.claims.package_id);
    let registries = render_composed_contributions(base, &composed)?;
    let proposed = apply_overlay(
        base,
        registries
            .into_iter()
            .map(|(path, (bytes, _))| (path, Some(bytes))),
    )?;
    let config = super::materialize::assemble_config(&proposed)?;
    let rules = proposed
        .file_bytes(&VirtualPath::RULES)
        .map_err(ProducerError::from)?
        .map(|bytes| String::from_utf8(bytes.to_vec()))
        .transpose()
        .map_err(ProducerError::from)?;
    let mut closure = record_paths.into_iter().collect::<BTreeSet<_>>();
    closure.extend(super::materialize::render_capture_closure(
        proposed.layout(),
        &config,
        &[],
        rules.as_deref(),
    )?);
    Ok(closure.into_iter().collect())
}

/// Target bytes and semantic owner evidence derived by profile composition.
#[derive(Debug)]
pub(super) struct ProfileTargetComposition {
    pub targets: BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
    /// Claims whose matching captured content was repository-authored rather
    /// than already claimed by a package.
    pub retained_claims: BTreeSet<AppliedProfileClaimIdentity>,
    /// Former, safely removable exact asset targets.
    pub removals: Vec<VirtualPath>,
}

/// Derive every profile-owned target's exact final bytes and mode from a captured
/// base image.
///
/// The declaration-overlay registries and exact assets land verbatim; the managed
/// regions compose over the captured base through the one managed-document engine;
/// and every configured projection the merged declarations imply is re-rendered
/// over the resulting proposed image and folded back into the target it already
/// produced. A projection whose target is not itself a profile-owned
/// asset/region/registry leaves that target untouched (matching the
/// profile-application contract: a projection only rewrites bytes the profile
/// otherwise publishes).
///
/// The result equals the byte-for-byte final image of every profile-owned target
/// regardless of whether it changed from the captured occupant; the caller
/// (the typed aggregate profile-selection materialization request) decides
/// which targets to write.
pub(super) fn compose_profile_targets(
    base: &RepositoryImage,
    claims: ProfileClaims,
) -> Result<ProfileTargetComposition, RepositoryStateError> {
    let contribution_context = claims.contributions.clone();
    compose_profile_targets_with_context(base, claims, contribution_context)
}

/// Derive profile-owned targets with a preflighted, identity-scoped candidate
/// context. Definitions outside the package's own identities remain outside its
/// scoped contribution materialization, while equal definitions retain their full owner set
/// in every affected provenance record.
pub(super) fn compose_profile_targets_with_context(
    base: &RepositoryImage,
    claims: ProfileClaims,
    contribution_context: Vec<ProfileContributionClaim>,
) -> Result<ProfileTargetComposition, RepositoryStateError> {
    let mut retained_claims = retained_semantic_claim_identities(base, &claims.contributions)?;
    retained_claims.extend(retained_claim_identities_for_package(
        base,
        &claims.package_id,
    )?);
    let composed = compose_profile_contributions(base, &contribution_context)?;
    let composed = owned_composed_contributions(composed, &claims.package_id);
    let mut targets = render_composed_contributions(base, &composed)?;
    let package_id = claims.package_id.clone();
    let asset_claims = claims.assets.to_vec();
    let projection_targets = configured_projection_targets(base, &targets)?;
    for asset in claims.assets {
        let path = asset.claim.target().clone();
        targets.insert(path, (asset.bytes, asset.mode));
    }
    // Profile-owned regions compose over the captured base; a region target keeps
    // its captured file mode (a fresh target is Regular), matching the profile's
    // region-target mode contract.
    let regions = claims.regions.iter().map(|region| {
        let path = region.claim.target().clone();
        let (begin, end) = region_delimiters(&region.region_id);
        let claim = ManagedDocumentClaim::Region {
            owner: region.claim.owner().to_string(),
            region_id: region.region_id.to_string(),
            begin,
            end,
            content: region.content.clone(),
            placement: RegionPlacement::AppendIfAbsent,
        };
        (path, claim)
    });
    for (path, bytes) in compose_managed_documents(base, regions)? {
        let mode = existing_file_mode(base, &path)?;
        if matches!(
            base.entry(&path).map_err(ProducerError::from)?,
            RepositoryEntry::File { bytes: existing, mode: existing_mode, .. }
                if existing == &bytes && *existing_mode == mode
        ) && matches!(
            profile_conflict_occupant(base, &path)?,
            ProfileConflictOccupant::Repository
        ) {
            claims
                .regions
                .iter()
                .filter(|region| region.claim.target() == &path)
                .for_each(|region| {
                    retained_claims.insert(AppliedProfileClaimIdentity::ManagedRegion {
                        target: AppliedManagedRegionTarget::from_virtual_path(&path),
                        region_id: region.region_id.clone(),
                    });
                });
        }
        targets.insert(path, (bytes, mode));
    }

    // Build the proposed image (base overlaid with every target computed so far) and
    // re-render every configured projection from the PROPOSED declarations. The
    // occupant `compose_configured_projections` compares against IS the target
    // computed above, so it yields the rendered bytes whether or not it emits an
    // action: an emitted write updates the target to the rendered bytes, and a
    // no-op means the target already holds them. Either way the target ends at the
    // exact rendered projection bytes.
    let overlay = targets
        .iter()
        .map(|(path, (bytes, _))| (path.clone(), Some(bytes.clone())));
    let proposed = apply_overlay(base, overlay)?;
    let config = assemble_config(&proposed)?;
    let schema_overlay = serialized_default_ruleset(&config)
        .schema_files
        .into_iter()
        .map(|schema| {
            Ok((
                VirtualPath::data(format!("schemas/{}", schema.name))?,
                Some(schema.content.into_bytes()),
            ))
        })
        .collect::<Result<Vec<_>, super::RepositoryLayoutError>>()?;
    let proposed = apply_overlay(&proposed, schema_overlay)?;
    let declarations = declarations_from_image(&proposed)
        .map_err(|error| ProducerError::DeclarationAssembly(Box::new(error)))?;
    let projection_actions = compose_configured_projections(
        &proposed,
        declarations.config(),
        &declarations.borrowed(),
        None,
    )?
    .actions;
    for action in projection_actions {
        if let RepositoryAction::WriteFile { path, bytes, .. } = action {
            if let Some(target) = targets.get_mut(&path) {
                target.0 = bytes;
            }
        }
    }

    // An asset claim describes the bytes ultimately published. A projection may
    // rewrite a declared asset target, so adoption is evaluated only after that
    // final target image is known.
    for asset in &asset_claims {
        let path = asset.claim.target();
        let Some((bytes, mode)) = targets.get(path) else {
            continue;
        };
        if !asset.replace_owned && !projection_targets.contains(path) {
            decide_asset_replacement(base, &package_id, path, bytes, *mode)?;
        }
        if matches!(
            base.entry(path).map_err(ProducerError::from)?,
            RepositoryEntry::File { bytes: existing, mode: existing_mode, .. }
                if existing == bytes && *existing_mode == *mode
        ) && matches!(
            profile_conflict_occupant(base, path)?,
            ProfileConflictOccupant::Repository
        ) {
            retained_claims.insert(AppliedProfileClaimIdentity::Asset {
                target: AppliedClaimTarget::from_virtual_path(path, *mode),
            });
        }
    }
    // Any composed target, not just a directly declared asset or region, keeps
    // a former asset path alive. A declaration-derived target can legitimately
    // reuse that path during the same aggregate application.
    let candidate_paths = targets.keys().cloned().collect::<BTreeSet<_>>();
    let removals = obsolete_asset_removals(base, &package_id, &candidate_paths)?;
    Ok(ProfileTargetComposition {
        targets,
        retained_claims,
        removals,
    })
}

/// Decide whether an asset target can replace its package's recorded base.
///
/// A target with no matching prior claim remains the ordinary first-application
/// collision case. A matching claim takes the shared pure three-way path, so a
/// changed package updates its own unchanged content while a concurrent edit is
/// rejected before the aggregate transaction publishes anything.
fn decide_asset_replacement(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
    path: &VirtualPath,
    candidate_bytes: &[u8],
    candidate_mode: FileMode,
) -> Result<(), RepositoryStateError> {
    let RepositoryEntry::File {
        bytes: current_bytes,
        mode: current_mode,
        ..
    } = base.entry(path).map_err(ProducerError::from)?
    else {
        return Ok(());
    };
    if current_bytes == candidate_bytes && *current_mode == candidate_mode {
        return Ok(());
    }
    let Some(recorded) = recorded_asset_claim(base, package_id, path)? else {
        return Err(RepositoryStateError::ProfileTargetConflict(
            ProfileTargetConflictError {
                occupant: profile_conflict_occupant(base, path)?,
                candidate: package_id.clone(),
                path: path.clone(),
            },
        ));
    };
    let decision = decide_three_way(ThreeWayInput {
        target: path.clone(),
        owner: package_id.clone(),
        base: ThreeWayValue::Present(recorded.base_fingerprint),
        current: ThreeWayValue::Present(fingerprint_profile_asset(current_bytes, *current_mode)),
        candidate: ThreeWayValue::Present(fingerprint_profile_asset(
            candidate_bytes,
            candidate_mode,
        )),
        surviving_owners: 0,
        retain_if_unowned: recorded.retain_if_unowned,
    });
    match decision {
        ThreeWayDecision::Update | ThreeWayDecision::Unchanged => Ok(()),
        ThreeWayDecision::Conflict(conflict) => Err(RepositoryStateError::ProfileThreeWayConflict(
            Box::new(ProfileThreeWayConflictError {
                target: conflict.target,
                owner: conflict.owner,
                base: conflict.base,
                current: conflict.current,
                candidate: conflict.candidate,
            }),
        )),
        ThreeWayDecision::Retain | ThreeWayDecision::Remove => {
            unreachable!("a present candidate is always update, unchanged, or conflict")
        }
    }
}

fn recorded_asset_claim(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
    path: &VirtualPath,
) -> Result<Option<AppliedProfileClaim>, RepositoryStateError> {
    let record_path = VirtualPath::data(format!("profiles/{}.json", package_id.as_str()))?;
    if !base.capture_spec().contains_path(&record_path) {
        return Ok(None);
    }
    let RepositoryEntry::File { bytes, .. } =
        base.entry(&record_path).map_err(ProducerError::from)?
    else {
        return Ok(None);
    };
    let record: AppliedProfileRecord =
        serde_json::from_slice(bytes).map_err(|source| ProducerError::ProfileRecordParse {
            path: record_path.repository_relative(),
            source,
        })?;
    validate_applied_record_path(&record_path, &record)?;
    Ok(record
        .claims
        .into_iter()
        .find(|claim| match &claim.identity {
            AppliedProfileClaimIdentity::Asset { target } => {
                target.root == path.root_class() && target.path == *path.relative()
            }
            AppliedProfileClaimIdentity::Semantic { .. }
            | AppliedProfileClaimIdentity::ManagedRegion { .. } => false,
        }))
}

/// Target paths named by the package's former asset claims.
///
/// Capture planning asks for these before deciding removal so its final plan has
/// both the recorded base and the current repository value under one session.
fn replacement_asset_paths(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let record_path = VirtualPath::data(format!("profiles/{}.json", package_id.as_str()))?;
    if !base.capture_spec().contains_path(&record_path) {
        return Ok(Vec::new());
    }
    let RepositoryEntry::File { bytes, .. } =
        base.entry(&record_path).map_err(ProducerError::from)?
    else {
        return Ok(Vec::new());
    };
    let record: AppliedProfileRecord =
        serde_json::from_slice(bytes).map_err(|source| ProducerError::ProfileRecordParse {
            path: record_path.repository_relative(),
            source,
        })?;
    validate_applied_record_path(&record_path, &record)?;
    record
        .claims
        .into_iter()
        .filter_map(|claim| match claim.identity {
            AppliedProfileClaimIdentity::Asset { target } => Some(target),
            AppliedProfileClaimIdentity::Semantic { .. }
            | AppliedProfileClaimIdentity::ManagedRegion { .. } => None,
        })
        .map(|target| VirtualPath::from_root(target.root, target.path).map_err(Into::into))
        .collect()
}

/// Decide which former sole asset claims an updated package may remove.
fn obsolete_asset_removals(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
    candidate_paths: &BTreeSet<VirtualPath>,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let record_path = VirtualPath::data(format!("profiles/{}.json", package_id.as_str()))?;
    if !base.capture_spec().contains_path(&record_path) {
        return Ok(Vec::new());
    }
    let RepositoryEntry::File { bytes, .. } =
        base.entry(&record_path).map_err(ProducerError::from)?
    else {
        return Ok(Vec::new());
    };
    let record: AppliedProfileRecord =
        serde_json::from_slice(bytes).map_err(|source| ProducerError::ProfileRecordParse {
            path: record_path.repository_relative(),
            source,
        })?;
    validate_applied_record_path(&record_path, &record)?;
    record
        .claims
        .into_iter()
        .filter_map(|claim| {
            let target = match &claim.identity {
                AppliedProfileClaimIdentity::Asset { target } => Some(target.clone()),
                AppliedProfileClaimIdentity::Semantic { .. }
                | AppliedProfileClaimIdentity::ManagedRegion { .. } => None,
            }?;
            Some((claim, target))
        })
        .map(|(claim, target)| {
            let path = VirtualPath::from_root(target.root, target.path)?;
            if candidate_paths.contains(&path) {
                return Ok(None);
            }
            let current = match base.entry(&path).map_err(ProducerError::from)? {
                RepositoryEntry::Absent => ThreeWayValue::Absent,
                RepositoryEntry::File { bytes, mode, .. } => {
                    ThreeWayValue::Present(fingerprint_profile_asset(bytes, *mode))
                }
                _ => return Ok(None),
            };
            let decision = decide_three_way(ThreeWayInput {
                target: path.clone(),
                owner: package_id.clone(),
                base: ThreeWayValue::Present(claim.base_fingerprint),
                current,
                candidate: ThreeWayValue::Absent,
                surviving_owners: surviving_asset_owners(base, package_id, &path)?,
                retain_if_unowned: claim.retain_if_unowned,
            });
            Ok(matches!(decision, ThreeWayDecision::Remove).then_some(path))
        })
        .collect::<Result<Vec<_>, RepositoryStateError>>()
        .map(|paths| paths.into_iter().flatten().collect())
}

fn surviving_asset_owners(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
    path: &VirtualPath,
) -> Result<usize, RepositoryStateError> {
    applied_profile_record_paths(base)?
        .into_iter()
        .map(|record_path| {
            let RepositoryEntry::File { bytes, .. } =
                base.entry(&record_path).map_err(ProducerError::from)?
            else {
                return Ok(false);
            };
            let record: AppliedProfileRecord = serde_json::from_slice(bytes).map_err(|source| {
                ProducerError::ProfileRecordParse {
                    path: record_path.repository_relative(),
                    source,
                }
            })?;
            validate_applied_record_path(&record_path, &record)?;
            Ok(record.id.as_str() != package_id.as_str()
                && record.claims.iter().any(|claim| match &claim.identity {
                    AppliedProfileClaimIdentity::Asset { target } => {
                        target.root == path.root_class() && target.path == *path.relative()
                    }
                    AppliedProfileClaimIdentity::Semantic { .. }
                    | AppliedProfileClaimIdentity::ManagedRegion { .. } => false,
                }))
        })
        .collect::<Result<Vec<_>, RepositoryStateError>>()
        .map(|owners| owners.into_iter().filter(|owner| *owner).count())
}

/// Check all selected package contributions against one captured repository image.
///
/// This is deliberately a read-only semantic preflight. The aggregate
/// publication path reports a conflict anywhere in the selected set before its
/// one transaction can publish any member.
pub(crate) fn preflight_profile_contributions(
    base: &RepositoryImage,
    candidates: Vec<ProfileContributionClaim>,
) -> Result<(), RepositoryStateError> {
    compose_profile_contributions(base, &candidates).map(|_| ())
}

/// Render the complete selected contribution set into a proposed registry view.
///
/// Capture planning uses this view before it asks each profile to close over its
/// configured projections. A dependent profile can therefore resolve a
/// projection whose item kind is supplied by another member of the same
/// selection, without treating that proposed registry state as a separately
/// publishable materialization.
pub(crate) fn profile_contribution_overrides(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, RepositoryStateError> {
    let composed = compose_profile_contributions(base, candidates)?;
    render_composed_contributions(base, &composed).map(|registries| {
        registries
            .into_iter()
            .map(|(path, (bytes, _))| (path, Some(bytes)))
            .collect()
    })
}

/// Return every registry path whose semantic definition contributes to this
/// operation's preflight context.
pub(crate) fn profile_contribution_target_paths(
    candidates: &[ProfileContributionClaim],
) -> Result<Vec<VirtualPath>, super::RepositoryLayoutError> {
    Ok(candidates
        .iter()
        .map(|claim| claim.contribution.registry_target())
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect())
}

/// Compose a candidate context with the repository's per-identity ownership
/// evidence before any registry renderer receives a definition.
fn compose_profile_contributions(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<Vec<ComposedContribution>, RepositoryStateError> {
    let (recorded, repository) = existing_contribution_claims(base, candidates)?;
    let mut composed = compose_resolved_contributions(repository, candidates.iter().cloned())?;
    for contribution in &mut composed {
        let expected = fingerprint_semantic_contribution(&contribution.definition)
            .map_err(ProducerError::ProfileClaimFingerprint)?;
        let owners = recorded
            .iter()
            .filter(|claim| claim.identity == contribution.identity)
            .map(|claim| {
                (claim.base_fingerprint == expected)
                    .then(|| claim.package_id.clone())
                    .ok_or_else(|| ContributionCompositionConflict {
                        identity: contribution.identity.clone(),
                        owners: contribution
                            .owners
                            .iter()
                            .cloned()
                            .map(ContributionConflictOwner::Package)
                            .chain(std::iter::once(ContributionConflictOwner::Package(
                                claim.package_id.clone(),
                            )))
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        contribution.owners = contribution
            .owners
            .iter()
            .cloned()
            .chain(owners)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    Ok(composed)
}

fn owned_composed_contributions(
    contributions: Vec<ComposedContribution>,
    package_id: &ProfilePackageId,
) -> Vec<ComposedContribution> {
    contributions
        .into_iter()
        .filter(|contribution| contribution.owners.contains(package_id))
        .collect()
}

/// Render definitions that have already passed one semantic ownership
/// composition. This renderer deliberately contains no occupant convention.
fn render_composed_contributions(
    base: &RepositoryImage,
    contributions: &[ComposedContribution],
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, RepositoryStateError> {
    let mut documents = BTreeMap::<String, MergeDocument>::new();
    for contribution in contributions {
        let target = contribution.definition.registry_path().to_string();
        if !documents.contains_key(&target) {
            let path = base.layout().classify_repository_relative(&target)?;
            let existing = match base.entry(&path).map_err(ProducerError::from)? {
                RepositoryEntry::File { bytes, mode, .. } => Some((bytes.clone(), *mode)),
                RepositoryEntry::Absent => None,
                _ => return Err(ProducerError::ProfileRegistryNotFile { target }.into()),
            };
            documents.insert(target.clone(), MergeDocument::load(&target, existing)?);
        }
        let document = documents
            .get_mut(&target)
            .expect("profile registry document was inserted");
        render_composed_contribution(&target, &mut document.document, &contribution.definition)?;
    }
    documents
        .into_iter()
        .map(|(path, document)| {
            Ok((
                base.layout().classify_repository_relative(&path)?,
                (document.document.to_string().into_bytes(), document.mode),
            ))
        })
        .collect()
}

/// Every applied-profile record the profiles listing names, each beside the
/// path it is filed under.
///
/// A record is the repository's own statement about one profile, so a record
/// that does not parse, or that is filed under a name other than its own
/// profile's, fails the read rather than dropping that profile from the answer.
/// An occupant of the listing that is not a regular file is not a record and
/// contributes nothing.
///
/// Composition, adoption-intent retention, and repository-wide validation's
/// profile-ownership pass all begin here, so the record inventory is enumerated
/// and read under one rule.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when a listed record is outside the capture,
/// does not parse, or names a profile other than the one its path does.
pub fn applied_profile_records(
    base: &RepositoryImage,
) -> Result<Vec<(VirtualPath, AppliedProfileRecord)>, RepositoryStateError> {
    applied_profile_record_paths(base)?
        .into_iter()
        .map(|path| {
            let RepositoryEntry::File { bytes, .. } =
                base.entry(&path).map_err(ProducerError::from)?
            else {
                return Ok(None);
            };
            let record: AppliedProfileRecord = serde_json::from_slice(bytes).map_err(|source| {
                ProducerError::ProfileRecordParse {
                    path: path.repository_relative(),
                    source,
                }
            })?;
            validate_applied_record_path(&path, &record)?;
            Ok(Some((path, record)))
        })
        .filter_map(Result::transpose)
        .collect()
}

/// Discover every captured applied-profile record named by the profiles listing.
fn applied_profile_record_paths(
    base: &RepositoryImage,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let Some(listing) = base.listing_fingerprints().get(&VirtualPath::PROFILES) else {
        return Ok(Vec::new());
    };
    listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .map(|name| VirtualPath::data(format!("profiles/{name}")).map_err(Into::into))
        .collect()
}

/// One ownership fact read from an existing v2 record. It intentionally has no
/// definition: the registry remains the authority for every effective value.
struct RecordedSemanticClaim {
    package_id: ProfilePackageId,
    identity: ContributionIdentity,
    base_fingerprint: ProfileBaseFingerprint,
}

/// Read only ownership facts from current records and registry definitions from
/// the captured registries. No behavior is reconstructed from a record.
fn existing_contribution_claims(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<(Vec<RecordedSemanticClaim>, Vec<ExistingContributionClaim>), RepositoryStateError> {
    let candidate_identities = candidates
        .iter()
        .map(|claim| claim.contribution.semantic_identity())
        .collect::<BTreeSet<_>>();
    let recorded = applied_profile_records(base)?
        .into_iter()
        .flat_map(|(_, record)| {
            let package_id = ProfilePackageId::new(record.id.to_string());
            record
                .claims
                .into_iter()
                .filter_map(|claim| match claim.identity {
                    AppliedProfileClaimIdentity::Semantic { identity } => {
                        Some((identity, claim.base_fingerprint))
                    }
                    _ => None,
                })
                .filter(|(identity, _)| candidate_identities.contains(identity))
                .map(move |(identity, base_fingerprint)| RecordedSemanticClaim {
                    package_id: package_id.clone(),
                    identity,
                    base_fingerprint,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let owned_identities = recorded
        .iter()
        .map(|claim| claim.identity.clone())
        .collect::<BTreeSet<_>>();
    let repository = candidates
        .iter()
        .filter(|claim| !owned_identities.contains(&claim.contribution.semantic_identity()))
        .map(|claim| repository_definition(base, &claim.contribution.semantic_identity()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .map(ExistingContributionClaim::Repository)
        .collect();
    Ok((recorded, repository))
}

/// Preserve adoption intent when the same package record is re-applied or is
/// replaced by another version of the same package. The record remains
/// ownership provenance only; registry values are still read from the captured
/// image.
fn retained_claim_identities_for_package(
    base: &RepositoryImage,
    package_id: &ProfilePackageId,
) -> Result<BTreeSet<AppliedProfileClaimIdentity>, RepositoryStateError> {
    applied_profile_records(base)?
        .into_iter()
        .map(|(_, record)| {
            Ok(if record.id.as_str() == package_id.as_str() {
                record
                    .claims
                    .into_iter()
                    .filter(|claim| claim.retain_if_unowned)
                    .map(|claim| claim.identity)
                    .collect()
            } else {
                BTreeSet::new()
            })
        })
        .collect::<Result<Vec<BTreeSet<_>>, RepositoryStateError>>()
        .map(|sets| sets.into_iter().flatten().collect())
}

/// Find repository-authored semantic values a new profile adopts unchanged.
///
/// The record is only an ownership witness: definitions are always read from
/// the captured registries, then compared to the resolved package value.
fn retained_semantic_claim_identities(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<BTreeSet<AppliedProfileClaimIdentity>, RepositoryStateError> {
    let (_, repository) = existing_contribution_claims(base, candidates)?;
    Ok(repository
        .into_iter()
        .filter_map(|claim| match claim {
            ExistingContributionClaim::Repository(contribution) => Some(contribution),
            ExistingContributionClaim::Package(_) => None,
        })
        .filter(|existing| {
            candidates
                .iter()
                .any(|candidate| candidate.contribution == *existing)
        })
        .map(|contribution| AppliedProfileClaimIdentity::Semantic {
            identity: contribution.semantic_identity(),
        })
        .collect())
}

/// What a captured image says about the value one recorded ownership claim
/// published.
///
/// The comparison is against the claim's own recorded base, never against what
/// a package would publish today, so it answers "does the repository still hold
/// what this profile put there" for a repository whose package moved, changed,
/// or disappeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedTargetState {
    /// The claimed target lies outside this capture, so this image is not
    /// evidence either way.
    Uncaptured,
    /// The repository no longer holds what the claim names.
    Absent,
    /// The repository holds exactly the value the claim recorded.
    Unchanged,
    /// The repository holds a value other than the one the claim recorded.
    Changed,
}

impl ClaimedTargetState {
    fn matching(matches: bool) -> Self {
        if matches {
            Self::Unchanged
        } else {
            Self::Changed
        }
    }
}

/// Compare one recorded ownership claim against the value `base` currently
/// holds for it.
///
/// This is the sole comparison behind both the profile-scoped agreement check
/// and repository-wide validation's profile-ownership pass, so each claim kind
/// is read back through the construction that published it: an asset by its
/// exact bytes and mode, a managed region by the body between the delimiters
/// composition splices it under, and a semantic declaration by the registry
/// entry its identity names.
///
/// A region's published body is read back with the newline the splice inserts
/// after the begin delimiter removed, and a content that did not end in a
/// newline is stored with the terminator the splice supplies. Both spellings
/// render the same document, so either matching the recorded fingerprint is
/// agreement rather than a difference the document could express.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when the image cannot be read at a captured
/// path, when a registry the claim names is not a regular file, or when the
/// registry does not parse as the declaration the identity addresses.
pub fn claimed_target_state(
    base: &RepositoryImage,
    claim: &AppliedProfileClaim,
) -> Result<ClaimedTargetState, RepositoryStateError> {
    let path = claim.identity.claimed_path(base.layout())?;
    if !base.capture_spec().contains_path(&path) {
        return Ok(ClaimedTargetState::Uncaptured);
    }
    match &claim.identity {
        AppliedProfileClaimIdentity::Semantic { identity } => {
            Ok(match repository_definition(base, identity)? {
                None => ClaimedTargetState::Absent,
                Some(current) => ClaimedTargetState::matching(
                    fingerprint_semantic_contribution(&current)
                        .map_err(ProducerError::ProfileClaimFingerprint)?
                        == claim.base_fingerprint,
                ),
            })
        }
        AppliedProfileClaimIdentity::Asset { .. } => {
            Ok(match base.entry(&path).map_err(ProducerError::from)? {
                RepositoryEntry::File { bytes, mode, .. } => ClaimedTargetState::matching(
                    fingerprint_profile_asset(bytes, *mode) == claim.base_fingerprint,
                ),
                _ => ClaimedTargetState::Absent,
            })
        }
        AppliedProfileClaimIdentity::ManagedRegion { region_id, .. } => {
            let RepositoryEntry::File { bytes, mode, .. } =
                base.entry(&path).map_err(ProducerError::from)?
            else {
                return Ok(ClaimedTargetState::Absent);
            };
            let (begin, end) = region_delimiters(region_id);
            let Some(body) = super::managed_document::region_body(bytes, &begin, &end) else {
                return Ok(ClaimedTargetState::Absent);
            };
            let published = body.strip_prefix(b"\n").unwrap_or(body);
            Ok(ClaimedTargetState::matching(
                [
                    published,
                    published.strip_suffix(b"\n").unwrap_or(published),
                ]
                .into_iter()
                .any(|content| {
                    fingerprint_managed_region(region_id, content, *mode) == claim.base_fingerprint
                }),
            ))
        }
    }
}

/// Read the repository definition carrying one semantic identity, if authored.
///
/// The identity alone decides both which registry is read and which declaration
/// inside it answers, so a caller holding an identity without the contribution
/// that minted it — a recorded ownership claim — reads the current value through
/// the same function composition uses.
fn repository_definition(
    base: &RepositoryImage,
    identity: &ContributionIdentity,
) -> Result<Option<Contribution>, RepositoryStateError> {
    let registry = identity.registry.path();
    let path = base.layout().classify_repository_relative(registry)?;
    let existing = match base.entry(&path).map_err(ProducerError::from)? {
        RepositoryEntry::Absent => return Ok(None),
        RepositoryEntry::File { bytes, mode, .. } => Some((bytes.clone(), *mode)),
        _ => {
            return Err(ProducerError::ProfileRegistryNotFile {
                target: registry.to_string(),
            }
            .into())
        }
    };
    let document = MergeDocument::load(registry, existing)?;
    let semantic = semantic_document(registry, &document.document)?;
    match &identity.target {
        ContributionIdentityTarget::Scalar { target } => {
            let (table, key) = target.config_path();
            let Some(value) = semantic.get(table).and_then(|table| table.get(key)) else {
                return Ok(None);
            };
            let value = value.as_str().ok_or_else(|| {
                profile_registry_error(registry, ProfileRegistryParseError::ScalarTargetNotString)
            })?;
            Ok(Some(Contribution::Scalar {
                target: *target,
                value: value.to_string(),
            }))
        }
        ContributionIdentityTarget::MapEntry { target, name } => Ok(semantic_map_entry(
            &semantic,
            target.table_path(),
            name,
        )
        .map(|value| Contribution::MapEntry {
            target: *target,
            identity: name.clone(),
            value: value.clone(),
        })),
        ContributionIdentityTarget::SetString { target, value } => {
            let (table, key) = target.config_path();
            let Some(values) = semantic.get(table).and_then(|table| table.get(key)) else {
                return Ok(None);
            };
            let values = values.as_array().ok_or_else(|| {
                profile_registry_error(registry, ProfileRegistryParseError::SetTargetNotArray)
            })?;
            if values.iter().any(|value| !value.is_string()) {
                return Err(profile_registry_error(
                    registry,
                    ProfileRegistryParseError::SetTargetNonStringMember,
                ));
            }
            Ok(values
                .iter()
                .any(|entry| entry.as_str() == Some(value))
                .then(|| Contribution::SetString {
                    target: *target,
                    value: value.clone(),
                }))
        }
        ContributionIdentityTarget::KeyedArray { target, name } => {
            let Some(entries) = semantic.get(target.array_name()) else {
                return Ok(None);
            };
            let entries = entries.as_array().ok_or_else(|| {
                profile_registry_error(
                    registry,
                    ProfileRegistryParseError::NotArrayOfTables {
                        key: target.array_name().to_string(),
                    },
                )
            })?;
            Ok(entries
                .iter()
                .find(|entry| {
                    entry
                        .get(target.identity_field())
                        .and_then(JsonValue::as_str)
                        == Some(name.as_str())
                })
                .cloned()
                .map(|value| Contribution::KeyedArray {
                    target: *target,
                    identity: name.clone(),
                    value,
                }))
        }
        ContributionIdentityTarget::Projection { name } => semantic
            .get("projection")
            .and_then(|projections| projections.get(name))
            .map(|value| {
                serde_json::from_value(value.clone())
                    .map(|value| Contribution::Projection {
                        name: name.clone(),
                        value,
                    })
                    .map_err(|error| {
                        profile_registry_error(
                            registry,
                            ProfileRegistryParseError::ProjectionDefinition(error.to_string()),
                        )
                    })
            })
            .transpose(),
    }
}

fn configured_projection_targets(
    base: &RepositoryImage,
    registries: &BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
) -> Result<BTreeSet<VirtualPath>, RepositoryStateError> {
    let config_path = VirtualPath::CONFIG;
    let bytes = match registries.get(&config_path) {
        Some((bytes, _)) => Some(bytes.as_slice()),
        None => base.file_bytes(&config_path).map_err(ProducerError::from)?,
    };
    let Some(bytes) = bytes else {
        return Ok(BTreeSet::new());
    };
    let declarations =
        crate::declarations::parse_configuration(bytes).map_err(ProducerError::from)?;
    declarations
        .projections
        .values()
        .filter_map(|projection| projection.target.as_deref())
        .map(|target| {
            base.layout()
                .classify_repository_relative(target)
                .map_err(Into::into)
        })
        .collect()
}

struct MergeDocument {
    document: DocumentMut,
    mode: FileMode,
}

impl MergeDocument {
    fn load(
        target: &str,
        existing: Option<(Vec<u8>, FileMode)>,
    ) -> Result<Self, RepositoryStateError> {
        let (bytes, mode) = existing.unwrap_or_else(|| (Vec::new(), FileMode::Regular));
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| profile_registry_error(target, error.into()))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| profile_registry_error(target, error.into()))?;
        Ok(Self { document, mode })
    }
}

fn profile_registry_error(target: &str, source: ProfileRegistryParseError) -> RepositoryStateError {
    ProducerError::ProfileRegistryParse {
        target: target.to_string(),
        source: Box::new(source),
    }
    .into()
}

fn render_composed_contribution(
    registry: &str,
    document: &mut DocumentMut,
    contribution: &Contribution,
) -> Result<(), RepositoryStateError> {
    let semantic = semantic_document(registry, document)?;
    let context = ContributionRenderContext { registry };
    match contribution {
        Contribution::Scalar { target, value } => {
            merge_scalar(&context, document, &semantic, *target, value)
        }
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => merge_map_entry(&context, document, &semantic, *target, identity, value),
        Contribution::SetString { target, value } => {
            merge_set_string(registry, document, &semantic, *target, value)
        }
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => merge_keyed_array(&context, document, *target, identity, value),
        Contribution::Projection { name, value } => {
            merge_projection(&context, document, &semantic, name, value)
        }
    }
}

/// The registry receiving an already-composed contribution.
struct ContributionRenderContext<'a> {
    registry: &'a str,
}

fn merge_scalar(
    context: &ContributionRenderContext<'_>,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    target: ScalarTarget,
    candidate: &str,
) -> Result<(), RepositoryStateError> {
    let (table, key) = target.config_path();
    if semantic
        .get(table)
        .and_then(|table| table.get(key))
        .is_some()
    {
        return Ok(());
    }
    ensure_table(document.as_table_mut(), table, context.registry)?.insert(key, candidate.into());
    Ok(())
}

fn merge_map_entry(
    context: &ContributionRenderContext<'_>,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    target: MapEntryTarget,
    identity: &str,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    if semantic_map_entry(semantic, target.table_path(), identity).is_some() {
        return Ok(());
    }
    match target {
        MapEntryTarget::TypeHierarchyTypes => {
            ensure_inline_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", context.registry)?,
                "types",
                context.registry,
            )?
            .insert(identity, json_to_edit_value(candidate, context.registry)?);
        }
        MapEntryTarget::LabelAssociations => {
            ensure_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", context.registry)?,
                "label_associations",
                context.registry,
            )?
            .insert(
                identity,
                Item::Value(json_to_edit_value(candidate, context.registry)?),
            );
        }
        MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds => {
            let root = if target == MapEntryTarget::Namespaces {
                "namespaces"
            } else {
                "item_kinds"
            };
            ensure_table(document.as_table_mut(), root, context.registry)?.insert(
                identity,
                Item::Table(json_object_to_table(candidate, context.registry)?),
            );
        }
    }
    Ok(())
}

fn semantic_map_entry<'a>(
    semantic: &'a JsonValue,
    path: &[&str],
    identity: &str,
) -> Option<&'a JsonValue> {
    path.iter()
        .try_fold(semantic, |value, key| value.get(*key))?
        .as_object()?
        .get(identity)
}

fn merge_set_string(
    registry: &str,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    target: SetStringTarget,
    candidate: &str,
) -> Result<(), RepositoryStateError> {
    let (table, key) = target.config_path();
    let values = semantic.get(table).and_then(|table| table.get(key));
    if let Some(values) = values {
        let values = values.as_array().ok_or_else(|| {
            profile_registry_error(registry, ProfileRegistryParseError::SetTargetNotArray)
        })?;
        if values.iter().any(|value| !value.is_string()) {
            return Err(profile_registry_error(
                registry,
                ProfileRegistryParseError::SetTargetNonStringMember,
            ));
        }
        if values.iter().any(|value| value.as_str() == Some(candidate)) {
            return Ok(());
        }
    }
    ensure_array(
        ensure_table(document.as_table_mut(), table, registry)?,
        key,
        registry,
    )?
    .push(candidate);
    Ok(())
}

fn merge_keyed_array(
    context: &ContributionRenderContext<'_>,
    document: &mut DocumentMut,
    target: KeyedArrayTarget,
    identity: &str,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    let field = target.identity_field();
    let (array, preserved_comment) = ensure_array_of_tables(
        document.as_table_mut(),
        target.array_name(),
        context.registry,
    )?;
    let mut identities = BTreeSet::new();
    let mut existing = false;
    for table in array.iter() {
        let Some(actual) = table.get(field).and_then(Item::as_str) else {
            return Err(profile_registry_error(
                context.registry,
                ProfileRegistryParseError::MissingIdentity {
                    field: field.to_string(),
                },
            ));
        };
        if !identities.insert(actual.to_string()) {
            return Err(profile_registry_error(
                context.registry,
                ProfileRegistryParseError::DuplicateIdentity {
                    field: field.to_string(),
                },
            ));
        }
        if actual == identity {
            existing = true;
        }
    }
    if existing {
        return Ok(());
    }
    let mut table = json_object_to_table(candidate, context.registry)?;
    if let Some(comment) = preserved_comment {
        table.decor_mut().set_prefix(comment);
    }
    array.push(table);
    Ok(())
}

fn merge_projection(
    context: &ContributionRenderContext<'_>,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    name: &str,
    candidate: &CompleteProjectionConfig,
) -> Result<(), RepositoryStateError> {
    let candidate = serde_json::to_value(candidate).expect("projection config serializes");
    if semantic
        .get("projection")
        .and_then(|projections| projections.get(name))
        .is_some()
    {
        return Ok(());
    }
    ensure_table(document.as_table_mut(), "projection", context.registry)?.insert(
        name,
        Item::Table(json_object_to_table(&candidate, context.registry)?),
    );
    Ok(())
}

fn semantic_document(
    registry: &str,
    document: &DocumentMut,
) -> Result<JsonValue, RepositoryStateError> {
    toml_edit::de::from_str(&document.to_string())
        .map_err(|error| profile_registry_error(registry, error.into()))
}

/// Who already holds `target`: an applied package, or the repository itself.
///
/// An applied-profile record states the targets its package published, keyed by
/// the same repository-relative spelling
/// ([`VirtualPath::repository_relative`]) the package's manifest addresses them
/// with, so a target no record claims is the repository's own content. The
/// records are read from the `.jit/profiles/` listing, which every capture that
/// reaches composition discovers together with the record files beneath it.
fn profile_conflict_occupant(
    base: &RepositoryImage,
    target: &VirtualPath,
) -> Result<ProfileConflictOccupant, RepositoryStateError> {
    let Some(listing) = base.listing_fingerprints().get(&VirtualPath::PROFILES) else {
        return Ok(ProfileConflictOccupant::Repository);
    };
    for name in listing.children().keys() {
        let record_path = VirtualPath::data(format!("profiles/{name}"))?;
        let RepositoryEntry::File { bytes, .. } =
            base.entry(&record_path).map_err(ProducerError::from)?
        else {
            continue;
        };
        let record: AppliedProfileRecord =
            serde_json::from_slice(bytes).map_err(|source| ProducerError::ProfileRecordParse {
                path: record_path.repository_relative(),
                source,
            })?;
        validate_applied_record_path(&record_path, &record)?;
        if record.claims.iter().any(|claim| match &claim.identity {
            AppliedProfileClaimIdentity::Semantic { .. } => false,
            AppliedProfileClaimIdentity::Asset {
                target: claimed_target,
            } => {
                claimed_target.root == target.root_class()
                    && claimed_target.path == *target.relative()
            }
            AppliedProfileClaimIdentity::ManagedRegion {
                target: claimed_target,
                ..
            } => {
                claimed_target.root == target.root_class()
                    && claimed_target.path == *target.relative()
            }
        }) {
            return Ok(ProfileConflictOccupant::Package(ProfilePackageId::new(
                record.id.to_string(),
            )));
        }
    }
    Ok(ProfileConflictOccupant::Repository)
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Table, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotTable {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_inline_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut InlineTable, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::InlineTable(InlineTable::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_inline_table_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotInlineTable {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_array<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Array, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::Array(Array::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_array_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotArray {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_array_of_tables<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<(&'a mut ArrayOfTables, Option<String>), RepositoryStateError> {
    let preserved_comment = empty_array_comments(parent, key);
    let convert = !parent.contains_key(key)
        || parent
            .get(key)
            .and_then(Item::as_array)
            .is_some_and(Array::is_empty);
    if convert {
        parent.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let array = parent
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotArrayOfTables {
                    key: key.to_string(),
                },
            )
        })?;
    Ok((array, preserved_comment))
}

fn empty_array_comments(parent: &Table, key: &str) -> Option<String> {
    let array = parent.get(key)?.as_array()?;
    if !array.is_empty() {
        return None;
    }
    let decor = parent.key(key)?.leaf_decor();
    let fragments = [
        decor.prefix(),
        decor.suffix(),
        array.decor().prefix(),
        array.decor().suffix(),
    ]
    .into_iter()
    .filter_map(comment_fragment)
    .collect::<String>();
    (!fragments.is_empty()).then_some(fragments)
}

fn comment_fragment(raw: Option<&toml_edit::RawString>) -> Option<String> {
    let text = raw?.as_str()?;
    let comment = text.get(text.find('#')?..)?.trim_end_matches(['\r', '\n']);
    Some(format!("{comment}\n"))
}

fn json_object_to_table(value: &JsonValue, registry: &str) -> Result<Table, RepositoryStateError> {
    value
        .as_object()
        .ok_or_else(|| {
            profile_registry_error(registry, ProfileRegistryParseError::ContributionNotTable)
        })?
        .iter()
        .try_fold(Table::new(), |mut table, (key, value)| {
            table.insert(key, json_to_item(value, registry)?);
            Ok(table)
        })
}

fn json_to_item(value: &JsonValue, registry: &str) -> Result<Item, RepositoryStateError> {
    match value {
        JsonValue::Object(_) => Ok(Item::Value(Value::InlineTable(json_to_inline_table(
            value, registry,
        )?))),
        _ => Ok(Item::Value(json_to_edit_value(value, registry)?)),
    }
}

fn json_to_edit_value(value: &JsonValue, registry: &str) -> Result<Value, RepositoryStateError> {
    match value {
        JsonValue::Null => Err(profile_registry_error(
            registry,
            ProfileRegistryParseError::NullValue,
        )),
        JsonValue::Bool(value) => Ok(Value::from(*value)),
        JsonValue::Number(value) => value
            .as_i64()
            .map(Value::from)
            .or_else(|| value.as_f64().map(Value::from))
            .ok_or_else(|| {
                profile_registry_error(registry, ProfileRegistryParseError::UnsupportedNumericValue)
            }),
        JsonValue::String(value) => Ok(Value::from(value.clone())),
        JsonValue::Array(values) => values
            .iter()
            .try_fold(Array::new(), |mut array, value| {
                array.push(json_to_edit_value(value, registry)?);
                Ok(array)
            })
            .map(Value::Array),
        JsonValue::Object(_) => json_to_inline_table(value, registry).map(Value::InlineTable),
    }
}

fn json_to_inline_table(
    value: &JsonValue,
    registry: &str,
) -> Result<InlineTable, RepositoryStateError> {
    value
        .as_object()
        .ok_or_else(|| profile_registry_error(registry, ProfileRegistryParseError::ValueNotTable))?
        .iter()
        .try_fold(InlineTable::new(), |mut table, (key, value)| {
            table.insert(key, json_to_edit_value(value, registry)?);
            Ok(table)
        })
}

/// The captured file mode at `path`, or `Regular` for an absent or non-file entry.
fn existing_file_mode(
    base: &RepositoryImage,
    path: &VirtualPath,
) -> Result<FileMode, RepositoryStateError> {
    match base.entry(path).map_err(ProducerError::from)? {
        RepositoryEntry::File { mode, .. } => Ok(*mode),
        _ => Ok(FileMode::Regular),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, EntryIdentity, ListingFingerprint, RepositoryLayout,
        RepositoryRootEvidence,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn image_with_profile_owner(
        target: &VirtualPath,
        target_bytes: &[u8],
        package_owns_target: bool,
    ) -> (RepositoryImage, RepositoryLayout) {
        let layout = super::super::RepositoryLayout::new(
            super::super::RepositoryRootEvidence::new("/repo", "worktree", true),
            super::super::RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let config_bytes = b"";
        let config_identity = EntryIdentity::for_bytes("config", config_bytes).unwrap();
        let claims = package_owns_target
            .then(|| AppliedProfileClaim::asset(target, target_bytes, FileMode::Regular, false))
            .into_iter()
            .collect();
        let record = AppliedProfileRecord::new(
            "base-package"
                .try_into()
                .expect("test profile id is canonical"),
            "1.0.0",
            "*",
            ProfileOrigin::Directory(
                crate::repository_state::RootRelativePath::parse("packages/base-package")
                    .expect("a canonical package location"),
            ),
            "package-hash",
            ResolvedVariables::default(),
            claims,
        );
        let record_bytes = record.to_bytes().unwrap();
        let record_path = VirtualPath::data("profiles/base-package.json").unwrap();
        let profiles = VirtualPath::PROFILES;
        let mut spec = CaptureSpec::phase_one(
            [
                VirtualPath::CONFIG,
                VirtualPath::GATES,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::TEMPLATES,
                profiles.clone(),
                record_path.clone(),
                target.clone(),
            ],
            CaptureBudget {
                max_paths: 16,
                max_listings: 1,
                max_bytes: 4096,
                max_depth: 8,
            },
        )
        .unwrap();
        spec.discover_listing(profiles.clone()).unwrap();
        let record_identity = EntryIdentity::for_bytes("record", &record_bytes).unwrap();
        let mut entries = BTreeMap::from([
            (
                VirtualPath::CONFIG,
                RepositoryEntry::File {
                    identity: config_identity,
                    bytes: config_bytes.to_vec(),
                    mode: FileMode::Regular,
                },
            ),
            (
                profiles.clone(),
                RepositoryEntry::Directory {
                    identity: EntryIdentity::for_bytes("profiles", b"directory").unwrap(),
                    mode: FileMode::Regular,
                },
            ),
            (
                record_path,
                RepositoryEntry::File {
                    identity: record_identity.clone(),
                    bytes: record_bytes,
                    mode: FileMode::Regular,
                },
            ),
            (
                target.clone(),
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("target", target_bytes).unwrap(),
                    bytes: target_bytes.to_vec(),
                    mode: FileMode::Regular,
                },
            ),
        ]);
        entries.extend(
            [
                VirtualPath::GATES,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::TEMPLATES,
            ]
            .into_iter()
            .map(|path| (path, RepositoryEntry::Absent)),
        );
        let listings = BTreeMap::from([(
            profiles,
            ListingFingerprint::new(BTreeMap::from([(
                "base-package.json".to_string(),
                record_identity,
            )]))
            .unwrap(),
        )]);
        (
            RepositoryImage::close(
                layout.clone(),
                spec,
                entries,
                listings,
                BTreeMap::new(),
                BTreeMap::new(),
            )
            .unwrap(),
            layout,
        )
    }

    /// Render one contribution that has already passed semantic composition.
    fn merge_authored_contribution(
        document: &mut DocumentMut,
        contribution: &Contribution,
    ) -> Result<(), RepositoryStateError> {
        render_composed_contribution(".jit/config.toml", document, contribution)
    }

    fn namespace_contribution(identity: &str, description: &str) -> Contribution {
        Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: identity.to_string(),
            value: serde_json::json!({ "description": description }),
        }
    }

    fn contribution_claim(package: &str, contribution: Contribution) -> ProfileContributionClaim {
        ProfileContributionClaim {
            package_id: ProfilePackageId::new(package),
            contribution,
        }
    }

    #[test]
    fn test_applied_profile_record_v2_wire_is_strict_and_claims_are_sorted() {
        let alpha = namespace_contribution("alpha", "First namespace.");
        let beta = namespace_contribution("beta", "Second namespace.");
        let claims = [
            AppliedProfileClaim::semantic(&beta, false).unwrap(),
            AppliedProfileClaim::semantic(&alpha, false).unwrap(),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        let record = AppliedProfileRecord::new(
            "example".try_into().expect("test profile id is canonical"),
            "1.2.3",
            ">=1.0.0",
            ProfileOrigin::Directory(
                crate::repository_state::RootRelativePath::parse("profiles/example")
                    .expect("canonical package source"),
            ),
            "a".repeat(64),
            ResolvedVariables::default(),
            claims,
        );

        let wire = serde_json::to_value(&record).expect("record serializes");
        assert_eq!(wire["record_version"], serde_json::json!(2));
        assert_eq!(wire["id"], serde_json::json!("example"));
        assert_eq!(wire["version"], serde_json::json!("1.2.3"));
        assert_eq!(wire["compatible_jit"], serde_json::json!(">=1.0.0"));
        assert_eq!(wire["package_hash"], serde_json::json!("a".repeat(64)));
        assert_eq!(
            wire["claims"]
                .as_array()
                .expect("claims are an array")
                .iter()
                .map(|claim| claim["identity"]["identity"]["target"]["name"].clone())
                .collect::<Vec<_>>(),
            vec![serde_json::json!("alpha"), serde_json::json!("beta")]
        );
        assert_eq!(
            serde_json::from_value::<AppliedProfileRecord>(wire.clone())
                .expect("exact current record reads"),
            record
        );

        let mut obsolete_version = wire.clone();
        obsolete_version["record_version"] = serde_json::json!(1);
        assert!(serde_json::from_value::<AppliedProfileRecord>(obsolete_version).is_err());

        let mut missing_claims = wire.clone();
        missing_claims
            .as_object_mut()
            .expect("record is an object")
            .remove("claims");
        assert!(serde_json::from_value::<AppliedProfileRecord>(missing_claims).is_err());

        let mut unknown = wire;
        unknown
            .as_object_mut()
            .expect("record is an object")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<AppliedProfileRecord>(unknown).is_err());
    }

    #[test]
    fn test_applied_profile_record_rejects_duplicate_claim_identities() {
        let target = VirtualPath::worktree("docs/example.md").expect("canonical target");
        let claim = AppliedProfileClaim::asset(&target, b"content", FileMode::Regular, false);
        let record = AppliedProfileRecord::new(
            "example".try_into().expect("test profile id is canonical"),
            "1.0.0",
            "*",
            ProfileOrigin::Embedded,
            "a".repeat(64),
            ResolvedVariables::default(),
            BTreeSet::from([claim]),
        );
        let mut wire = serde_json::to_value(record).expect("current record serializes");
        let duplicate = wire["claims"][0].clone();
        wire["claims"]
            .as_array_mut()
            .expect("claims are an array")
            .push(duplicate);

        assert!(serde_json::from_value::<AppliedProfileRecord>(wire).is_err());
    }

    #[test]
    fn test_applied_profile_record_wire_rejects_duplicate_json_member_names() {
        let record = AppliedProfileRecord::new(
            "example".try_into().expect("test profile id is canonical"),
            "1.0.0",
            "*",
            ProfileOrigin::Embedded,
            "a".repeat(64),
            ResolvedVariables::default(),
            BTreeSet::new(),
        );
        let wire = String::from_utf8(record.to_bytes().expect("record serializes"))
            .expect("record wire is UTF-8");
        let duplicate = wire.replacen(
            "\"record_version\": 2,",
            "\"record_version\": 2,\n  \"record_version\": 2,",
            1,
        );

        assert!(serde_json::from_str::<AppliedProfileRecord>(&duplicate).is_err());
    }

    #[test]
    fn test_validate_applied_record_path_rejects_a_record_under_another_package_id() {
        let record = AppliedProfileRecord::new(
            "example".try_into().expect("test profile id is canonical"),
            "1.0.0",
            "*",
            ProfileOrigin::Embedded,
            "a".repeat(64),
            ResolvedVariables::default(),
            BTreeSet::new(),
        );
        let path = VirtualPath::data("profiles/other.json").expect("canonical path");

        assert!(matches!(
            validate_applied_record_path(&path, &record),
            Err(ProducerError::ProfileRecordPathMismatch { .. })
        ));
    }

    #[test]
    fn test_profile_application_record_fingerprints_the_final_derived_asset_target() {
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .expect("layout is valid");
        let target = VirtualPath::worktree("docs/example.md").expect("canonical target");
        let claims = ProfileClaims {
            package_id: ProfilePackageId::new("example"),
            contributions: Vec::new(),
            assets: vec![ProfileAssetClaim {
                claim: TargetClaim::new(&layout, target.clone(), "example")
                    .expect("target claim is valid"),
                bytes: b"declared bytes".to_vec(),
                mode: FileMode::Regular,
                replace_owned: false,
            }],
            regions: Vec::new(),
        };
        let input = ProfileApplicationInput {
            id: "example".try_into().expect("test profile id is canonical"),
            version: "1.0.0".into(),
            compatible_jit: "*".into(),
            package_hash: "a".repeat(64),
            variables: ResolvedVariables::default(),
            target_hashes: BTreeMap::new(),
            origin: ProfileOrigin::Embedded,
            claims,
            contribution_context: Vec::new(),
            record_path: VirtualPath::data("profiles/example.json").expect("record path"),
        };
        let composition = ProfileTargetComposition {
            targets: BTreeMap::from([(
                target.clone(),
                (b"derived projection bytes".to_vec(), FileMode::Executable),
            )]),
            retained_claims: BTreeSet::new(),
            removals: Vec::new(),
        };

        let record = input.record(&composition).expect("record is derived");
        let expected = AppliedProfileClaim::asset(
            &target,
            b"derived projection bytes",
            FileMode::Executable,
            false,
        );
        assert_eq!(record.claims, BTreeSet::from([expected]));
    }

    #[test]
    fn test_profile_claim_fingerprints_canonicalize_semantic_values_and_separate_domains() {
        let first = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "example".to_string(),
            value: serde_json::from_str(r#"{"description":"Example.","rank":1}"#)
                .expect("valid JSON"),
        };
        let reordered = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "example".to_string(),
            value: serde_json::from_str(r#"{"rank":1,"description":"Example."}"#)
                .expect("valid JSON"),
        };
        let different_identity = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "other".to_string(),
            value: first_value(&first),
        };
        let target = VirtualPath::worktree("docs/example.md").expect("canonical target");
        let asset = AppliedProfileClaim::asset(&target, b"same bytes", FileMode::Regular, false);
        let executable =
            AppliedProfileClaim::asset(&target, b"same bytes", FileMode::Executable, false);
        let region = AppliedProfileClaim::managed_region(
            &target,
            "example-region"
                .try_into()
                .expect("test region id is canonical"),
            b"same bytes",
            FileMode::Regular,
            false,
        );

        assert_eq!(
            fingerprint_semantic_contribution(&first).expect("first fingerprint"),
            fingerprint_semantic_contribution(&reordered).expect("reordered fingerprint")
        );
        assert_ne!(
            fingerprint_semantic_contribution(&first).expect("first fingerprint"),
            fingerprint_semantic_contribution(&different_identity)
                .expect("different identity fingerprint")
        );
        assert_ne!(asset.base_fingerprint, executable.base_fingerprint);
        assert_ne!(asset.base_fingerprint, region.base_fingerprint);
        assert_ne!(asset.identity, region.identity);
    }

    #[test]
    fn test_managed_region_identity_ignores_mode_while_its_fingerprint_includes_mode() {
        let target = VirtualPath::worktree("AGENTS.md").expect("canonical target");
        let regular = AppliedProfileClaim::managed_region(
            &target,
            "guidance".try_into().expect("canonical region id"),
            b"region body",
            FileMode::Regular,
            true,
        );
        let executable = AppliedProfileClaim::managed_region(
            &target,
            "guidance".try_into().expect("canonical region id"),
            b"region body",
            FileMode::Executable,
            true,
        );

        assert_eq!(regular.identity, executable.identity);
        assert_ne!(regular.base_fingerprint, executable.base_fingerprint);
    }

    fn first_value(contribution: &Contribution) -> JsonValue {
        match contribution {
            Contribution::MapEntry { value, .. } => value.clone(),
            _ => unreachable!("test fixture is a map entry"),
        }
    }

    #[test]
    fn test_contribution_identity_uses_stable_typed_registry_target_and_name_dimensions() {
        let namespace = namespace_contribution("shared", "Shared vocabulary.");
        let other_namespace = namespace_contribution("other", "Shared vocabulary.");
        let item_kind = Contribution::MapEntry {
            target: MapEntryTarget::ItemKinds,
            identity: "shared".to_string(),
            value: serde_json::json!({ "section": "shared" }),
        };
        let gate = Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: "same-name".to_string(),
            value: serde_json::json!({ "key": "same-name" }),
        };
        let rule = Contribution::KeyedArray {
            target: KeyedArrayTarget::Rules,
            identity: "same-name".to_string(),
            value: serde_json::json!({ "name": "same-name" }),
        };

        assert_ne!(
            namespace.semantic_identity(),
            other_namespace.semantic_identity()
        );
        assert_ne!(namespace.semantic_identity(), item_kind.semantic_identity());
        assert_ne!(
            gate.semantic_identity().registry,
            rule.semantic_identity().registry
        );
        let encoded = serde_json::to_string(&namespace.semantic_identity()).unwrap();
        assert!(encoded.contains("\"registry\":\"config\""));
        assert!(encoded.contains("\"kind\":\"map-entry\""));
        assert!(encoded.contains("\"target\":\"namespaces\""));
        assert!(!encoded.contains("Namespaces"));
        assert_eq!(
            namespace.semantic_identity().to_string(),
            ".jit/config.toml:map-entry:namespaces:shared"
        );
    }

    #[test]
    fn test_compose_resolved_contributions_records_sorted_shared_owners() {
        let contribution = namespace_contribution("shared", "Shared vocabulary.");

        let composed = compose_resolved_contributions(
            [],
            [
                contribution_claim("workflow", contribution.clone()),
                contribution_claim("base", contribution),
            ],
        )
        .expect("equal resolved definitions compose");

        assert_eq!(composed.len(), 1);
        assert_eq!(
            composed[0].owners,
            vec![
                ProfilePackageId::new("base"),
                ProfilePackageId::new("workflow")
            ]
        );
    }

    #[test]
    fn test_compose_resolved_contributions_conflicts_independent_of_input_order() {
        let shared_identity = "shared";
        let first = namespace_contribution(shared_identity, "First definition.");
        let second = namespace_contribution(shared_identity, "Second definition.");
        let expected_owners = vec![
            ContributionConflictOwner::Package(ProfilePackageId::new("base")),
            ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
        ];

        for claims in [
            vec![
                contribution_claim("base", first.clone()),
                contribution_claim("workflow", second.clone()),
            ],
            vec![
                contribution_claim("workflow", second.clone()),
                contribution_claim("base", first.clone()),
            ],
        ] {
            let conflict = compose_resolved_contributions([], claims)
                .expect_err("different definitions must conflict without a winner");

            assert_eq!(conflict.identity, first.semantic_identity());
            assert_eq!(conflict.owners, expected_owners);
        }
    }

    #[test]
    fn test_compose_resolved_contributions_replaces_a_package_owned_identity() {
        let existing = namespace_contribution("shared", "Previous definition.");
        let replacement = namespace_contribution("shared", "Replacement definition.");

        let composed = compose_resolved_contributions(
            [ExistingContributionClaim::Package(contribution_claim(
                "workflow", existing,
            ))],
            [contribution_claim("workflow", replacement.clone())],
        )
        .expect("a package does not conflict with its own former identity claim");

        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].definition, replacement);
        assert_eq!(composed[0].owners, vec![ProfilePackageId::new("workflow")]);
    }

    #[test]
    fn test_compose_resolved_contributions_keeps_distinct_identities_in_one_registry() {
        let composed = compose_resolved_contributions(
            [],
            [
                contribution_claim("workflow", namespace_contribution("one", "First.")),
                contribution_claim("workflow", namespace_contribution("two", "Second.")),
            ],
        )
        .expect("different semantic identities in one registry compose");

        assert_eq!(composed.len(), 2);
        assert!(composed
            .iter()
            .all(|contribution| contribution.owners == [ProfilePackageId::new("workflow")]));
    }

    #[test]
    fn test_compose_resolved_contributions_distinguishes_repository_and_package_conflicts() {
        let existing = namespace_contribution("shared", "Existing definition.");
        let replacement = namespace_contribution("shared", "Replacement definition.");
        let candidate = contribution_claim("workflow", replacement);

        let cases = [
            (
                ExistingContributionClaim::Repository(existing.clone()),
                vec![
                    ContributionConflictOwner::Repository,
                    ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
                ],
            ),
            (
                ExistingContributionClaim::Package(contribution_claim("base", existing)),
                vec![
                    ContributionConflictOwner::Package(ProfilePackageId::new("base")),
                    ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
                ],
            ),
        ];

        for (existing, expected_owners) in cases {
            let conflict = compose_resolved_contributions([existing], [candidate.clone()])
                .expect_err("an existing differently-defined identity conflicts");

            assert_eq!(
                conflict.identity,
                candidate.contribution.semantic_identity()
            );
            assert_eq!(conflict.owners, expected_owners);
        }
    }

    #[test]
    fn test_profile_target_conflict_reports_package_occupant_and_candidate() {
        let target = VirtualPath::data("custom.txt").unwrap();
        let (image, layout) = image_with_profile_owner(&target, b"authored by package", true);
        let claims = ProfileClaims {
            package_id: ProfilePackageId::new("workflow-package"),
            contributions: Vec::new(),
            assets: vec![ProfileAssetClaim {
                claim: TargetClaim::new(&layout, target.clone(), "profile-asset:custom.txt")
                    .unwrap(),
                bytes: b"different package bytes".to_vec(),
                mode: FileMode::Regular,
                replace_owned: false,
            }],
            regions: Vec::new(),
        };

        let error = compose_profile_targets(&image, claims)
            .expect_err("different package target bytes must conflict");
        assert!(matches!(
            error,
            RepositoryStateError::ProfileTargetConflict(ProfileTargetConflictError {
                path,
                candidate,
                occupant: ProfileConflictOccupant::Package(occupant),
            }) if path == target
                && candidate.as_str() == "workflow-package"
                && occupant.as_str() == "base-package"
        ));
    }

    #[test]
    fn test_profile_target_conflict_reports_repository_occupant() {
        let target = VirtualPath::data("custom.txt").unwrap();
        let (image, layout) = image_with_profile_owner(&target, b"repository bytes", false);
        let claims = ProfileClaims {
            package_id: ProfilePackageId::new("workflow-package"),
            contributions: Vec::new(),
            assets: vec![ProfileAssetClaim {
                claim: TargetClaim::new(&layout, target.clone(), "profile-asset:custom.txt")
                    .unwrap(),
                bytes: b"different package bytes".to_vec(),
                mode: FileMode::Regular,
                replace_owned: false,
            }],
            regions: Vec::new(),
        };

        let error = compose_profile_targets(&image, claims)
            .expect_err("a package must not replace repository-authored bytes");
        assert!(matches!(
            error,
            RepositoryStateError::ProfileTargetConflict(ProfileTargetConflictError {
                occupant: ProfileConflictOccupant::Repository,
                candidate,
                path,
            }) if candidate.as_str() == "workflow-package" && path == target
        ));
    }

    #[test]
    fn test_profile_record_retains_adopted_identical_repository_asset_baseline() {
        let target = VirtualPath::data("custom.txt").unwrap();
        let (image, layout) = image_with_profile_owner(&target, b"repository bytes", false);
        let claims = ProfileClaims {
            package_id: ProfilePackageId::new("workflow-package"),
            contributions: Vec::new(),
            assets: vec![ProfileAssetClaim {
                claim: TargetClaim::new(&layout, target.clone(), "profile-asset:custom.txt")
                    .unwrap(),
                bytes: b"repository bytes".to_vec(),
                mode: FileMode::Regular,
                replace_owned: false,
            }],
            regions: Vec::new(),
        };
        let composition = compose_profile_targets(&image, claims.clone())
            .expect("identical repository asset is adoptable");
        let input = ProfileApplicationInput {
            id: "workflow-package"
                .try_into()
                .expect("test profile id is canonical"),
            version: "1.0.0".to_string(),
            compatible_jit: "*".to_string(),
            package_hash: "a".repeat(64),
            variables: ResolvedVariables::default(),
            target_hashes: BTreeMap::new(),
            origin: ProfileOrigin::Directory(
                crate::repository_state::RootRelativePath::parse("profiles/workflow-package")
                    .expect("canonical package source"),
            ),
            claims,
            contribution_context: Vec::new(),
            record_path: VirtualPath::data("profiles/workflow-package.json")
                .expect("canonical record path"),
        };
        let identity = AppliedProfileClaimIdentity::Asset {
            target: AppliedClaimTarget::from_virtual_path(&target, FileMode::Regular),
        };

        assert!(composition.retained_claims.contains(&identity));
        assert!(input
            .record(&composition)
            .expect("record serializes")
            .claims
            .iter()
            .any(|claim| claim.identity == identity && claim.retain_if_unowned));
    }

    #[test]
    fn test_retained_semantic_claim_identities_marks_an_adopted_registry_definition() {
        let layout = super::super::RepositoryLayout::new(
            super::super::RepositoryRootEvidence::new("/repo", "worktree", true),
            super::super::RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let config =
            b"[namespaces.adopted]\ndescription = \"Repository definition.\"\nunique = false\n";
        let mut spec = CaptureSpec::phase_one(
            [VirtualPath::CONFIG],
            CaptureBudget {
                max_paths: 4,
                max_listings: 0,
                max_bytes: 4096,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_paths([]).unwrap();
        let image = RepositoryImage::close(
            layout,
            spec,
            BTreeMap::from([(
                VirtualPath::CONFIG,
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("config", config).unwrap(),
                    bytes: config.to_vec(),
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let contribution = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "adopted".to_string(),
            value: serde_json::json!({
                "description": "Repository definition.",
                "unique": false,
            }),
        };
        let candidate = contribution_claim("workflow", contribution.clone());

        assert_eq!(
            retained_semantic_claim_identities(&image, &[candidate]).unwrap(),
            BTreeSet::from([AppliedProfileClaimIdentity::Semantic {
                identity: contribution.semantic_identity(),
            }])
        );
    }

    #[test]
    fn test_profile_registry_error_preserves_raw_target_and_typed_source() {
        let error = profile_registry_error(
            ".jit/config.toml",
            ProfileRegistryParseError::NotArray {
                key: "strategic_types".to_string(),
            },
        );

        assert!(matches!(
            error,
            RepositoryStateError::Producer(ProducerError::ProfileRegistryParse {
                target,
                source,
            }) if target == ".jit/config.toml"
                && matches!(*source, ProfileRegistryParseError::NotArray { ref key }
                    if key == "strategic_types")
        ));
    }

    #[test]
    fn test_scalar_contributions_write_documentation_and_validation_keys_independently() {
        let mut document = "[documentation]\n[validation]\n"
            .parse::<DocumentMut>()
            .unwrap();
        for (target, value) in [
            (
                ScalarTarget::DocumentationDevelopmentRoot,
                "workspace".to_string(),
            ),
            (
                ScalarTarget::DocumentationArchiveRoot,
                "archive".to_string(),
            ),
            (ScalarTarget::ValidationStrictness, "strict".to_string()),
            (ScalarTarget::ValidationDefaultType, "work-item".to_string()),
        ] {
            merge_authored_contribution(&mut document, &Contribution::Scalar { target, value })
                .unwrap();
        }

        let json: JsonValue = toml_edit::de::from_str(&document.to_string()).unwrap();
        assert_eq!(json["documentation"]["development_root"], "workspace");
        assert_eq!(json["documentation"]["archive_root"], "archive");
        assert_eq!(json["validation"]["strictness"], "strict");
        assert_eq!(json["validation"]["default_type"], "work-item");
    }

    #[test]
    fn test_documentation_list_contributions_merge_as_sets() {
        let mut document = "[documentation]\n".parse::<DocumentMut>().unwrap();
        for (target, value) in [
            (
                SetStringTarget::DocumentationManagedPaths,
                "workspace/active",
            ),
            (
                SetStringTarget::DocumentationManagedPaths,
                "workspace/design",
            ),
            (SetStringTarget::DocumentationPermanentPaths, "README.md"),
            (
                SetStringTarget::DocumentationIssueScopedAreas,
                "workspace/active",
            ),
        ] {
            merge_authored_contribution(
                &mut document,
                &Contribution::SetString {
                    target,
                    value: value.to_string(),
                },
            )
            .unwrap();
        }

        let json: JsonValue = toml_edit::de::from_str(&document.to_string()).unwrap();
        assert_eq!(
            json["documentation"]["managed_paths"],
            serde_json::json!(["workspace/active", "workspace/design"])
        );
        assert_eq!(
            json["documentation"]["permanent_paths"],
            serde_json::json!(["README.md"])
        );
        assert_eq!(
            json["documentation"]["issue_scoped_areas"],
            serde_json::json!(["workspace/active"])
        );
    }

    #[test]
    fn test_scalar_renderer_preserves_an_existing_composed_definition() {
        let mut document = "[validation]\ndefault_type = \"task\"\n"
            .parse::<DocumentMut>()
            .unwrap();
        let contribution = Contribution::Scalar {
            target: ScalarTarget::ValidationDefaultType,
            value: "task".to_string(),
        };
        merge_authored_contribution(&mut document, &contribution).unwrap();
        assert_eq!(
            document["validation"]["default_type"].as_str(),
            Some("task")
        );
    }

    #[test]
    fn test_empty_contributions_leave_authored_configuration_without_an_overlay() {
        let authored = b"[documentation]\ndevelopment_root = \"notes\"\n";
        let path = VirtualPath::data("config.toml").unwrap();
        let layout = super::super::RepositoryLayout::new(
            super::super::RepositoryRootEvidence::new("/repo", "worktree", true),
            super::super::RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let mut spec = CaptureSpec::phase_one(
            [path.clone()],
            CaptureBudget {
                max_paths: 4,
                max_listings: 0,
                max_bytes: 1024,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_paths([]).unwrap();
        let image = RepositoryImage::close(
            layout,
            spec,
            BTreeMap::from([(
                path,
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes(".jit/config.toml", authored).unwrap(),
                    bytes: authored.to_vec(),
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        assert!(render_composed_contributions(&image, &[])
            .unwrap()
            .is_empty());
    }

    /// An image whose exact-path closure is `captured` and which holds a file at
    /// each path `held` names.
    ///
    /// A path in `captured` but not in `held` is captured absence; a path in
    /// neither is outside the closure, which is what distinguishes "the
    /// repository no longer holds this" from "this image cannot say".
    fn image_over(
        captured: &[VirtualPath],
        held: &[(VirtualPath, &[u8], FileMode)],
    ) -> RepositoryImage {
        let layout = super::super::RepositoryLayout::new(
            super::super::RepositoryRootEvidence::new("/repo", "worktree", true),
            super::super::RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .expect("layout is valid");
        // Phase one is seeded from the data root, so the registry is always in
        // the closure and every other captured path is discovered onto it.
        let mut spec = CaptureSpec::phase_one(
            [VirtualPath::CONFIG],
            CaptureBudget {
                max_paths: 16,
                max_listings: 0,
                max_bytes: 64 * 1024,
                max_depth: 8,
            },
        )
        .expect("capture spec is valid");
        spec.discover_paths(captured.to_vec())
            .expect("captured paths are discoverable");
        let captured = std::iter::once(VirtualPath::CONFIG)
            .chain(captured.iter().cloned())
            .collect::<BTreeSet<_>>();
        let entries = captured
            .iter()
            .map(|path| {
                let held = held.iter().find(|(candidate, ..)| candidate == path);
                let entry = match held {
                    None => RepositoryEntry::Absent,
                    Some((path, bytes, mode)) => RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes(path.repository_relative(), bytes)
                            .expect("entry identity"),
                        bytes: bytes.to_vec(),
                        mode: *mode,
                    },
                };
                (path.clone(), entry)
            })
            .collect();
        RepositoryImage::close(
            layout,
            spec,
            entries,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("image closes")
    }

    #[test]
    fn test_claimed_target_state_answers_for_an_asset_from_its_recorded_bytes_and_mode() {
        let target = VirtualPath::worktree("docs/profile.txt").expect("canonical target");
        let claim = AppliedProfileClaim::asset(&target, b"published\n", FileMode::Regular, false);
        let state = |held: &[(VirtualPath, &[u8], FileMode)]| {
            claimed_target_state(&image_over(std::slice::from_ref(&target), held), &claim)
                .expect("the claim is comparable")
        };

        assert_eq!(
            state(&[(target.clone(), b"published\n", FileMode::Regular)]),
            ClaimedTargetState::Unchanged
        );
        assert_eq!(
            state(&[(target.clone(), b"edited in place\n", FileMode::Regular)]),
            ClaimedTargetState::Changed
        );
        assert_eq!(
            state(&[(target.clone(), b"published\n", FileMode::Executable)]),
            ClaimedTargetState::Changed,
            "mode belongs to the published identity, so a mode-only edit is a change"
        );
        assert_eq!(state(&[]), ClaimedTargetState::Absent);
        assert_eq!(
            claimed_target_state(&image_over(&[], &[]), &claim).expect("an uncaptured claim"),
            ClaimedTargetState::Uncaptured,
            "an image that never captured the target is evidence of nothing about it"
        );
    }

    /// The region a profile published is read back out of the document the
    /// composition engine wrote it into, so an untouched region agrees and an
    /// edited body does not.
    #[test]
    fn test_claimed_target_state_reads_a_managed_region_back_out_of_its_published_document() {
        let target = VirtualPath::worktree("AGENTS.md").expect("canonical target");
        let region_id: RegionId = "guidance".try_into().expect("canonical region id");
        let content = b"managed guidance\n";
        let claim = AppliedProfileClaim::managed_region(
            &target,
            region_id.clone(),
            content,
            FileMode::Regular,
            false,
        );
        let (begin, end) = region_delimiters(&region_id);
        let published = super::super::managed_document::render_managed_document(
            b"# Doc\n\nauthored prose\n",
            &[ManagedDocumentClaim::Region {
                owner: "example".to_string(),
                region_id: region_id.to_string(),
                begin: begin.clone(),
                end: end.clone(),
                content: content.to_vec(),
                placement: RegionPlacement::AppendIfAbsent,
            }],
        )
        .expect("the region composes into the document");
        let state = |bytes: &[u8]| {
            claimed_target_state(
                &image_over(
                    std::slice::from_ref(&target),
                    &[(target.clone(), bytes, FileMode::Regular)],
                ),
                &claim,
            )
            .expect("the claim is comparable")
        };

        assert_eq!(state(&published), ClaimedTargetState::Unchanged);
        assert_eq!(
            state(
                &String::from_utf8(published.clone())
                    .expect("the document is UTF-8")
                    .replace("managed guidance", "edited guidance")
                    .into_bytes()
            ),
            ClaimedTargetState::Changed
        );
        assert_eq!(
            state(b"# Doc\n\nauthored prose\n"),
            ClaimedTargetState::Absent,
            "a document whose markers are gone no longer holds the claimed region"
        );
    }

    /// A semantic claim is compared against the registry entry its own identity
    /// addresses, so the check reads the current declaration without holding the
    /// package that contributed it.
    #[test]
    fn test_claimed_target_state_compares_a_semantic_claim_against_the_registry_it_names() {
        let contribution = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "stream".to_string(),
            value: serde_json::json!({"description": "Synthetic stream.", "unique": false}),
        };
        let claim =
            AppliedProfileClaim::semantic(&contribution, false).expect("claim fingerprints");
        let rendered = |contribution: Option<&Contribution>| {
            let mut document = "".parse::<DocumentMut>().expect("an empty document parses");
            if let Some(contribution) = contribution {
                merge_authored_contribution(&mut document, contribution)
                    .expect("the contribution renders");
            }
            document.to_string().into_bytes()
        };
        let config = VirtualPath::CONFIG;
        let state = |bytes: &[u8]| {
            claimed_target_state(
                &image_over(
                    std::slice::from_ref(&config),
                    &[(config.clone(), bytes, FileMode::Regular)],
                ),
                &claim,
            )
            .expect("the claim is comparable")
        };

        assert_eq!(
            state(&rendered(Some(&contribution))),
            ClaimedTargetState::Unchanged
        );
        assert_eq!(
            state(&rendered(Some(&Contribution::MapEntry {
                target: MapEntryTarget::Namespaces,
                identity: "stream".to_string(),
                value: serde_json::json!({"description": "Edited by hand.", "unique": false}),
            }))),
            ClaimedTargetState::Changed
        );
        assert_eq!(state(&rendered(None)), ClaimedTargetState::Absent);
    }
}
