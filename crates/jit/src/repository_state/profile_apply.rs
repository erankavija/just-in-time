//! ApplyProfile materialization: compose a profile package's canonical claims
//! (declaration-overlay registry edits, exact assets, and managed regions) plus the
//! configured projections those declarations imply into the exact set of
//! profile-owned targets.
//!
//! `repository_state` owns this composition; the profile package produces the
//! neutral [`ProfileClaims`] and the command captures the base image and applies the
//! resulting delta. The applied record carries the profile layer's canonical
//! resolved-variable provenance, but package parsing, storage, and command code
//! remain outside this module; the typed `ApplyProfile` materialization request
//! sits between them.

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
use crate::profile::ResolvedVariables;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
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
    fn path(self) -> &'static str {
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

/// One composed semantic definition retained in installed-profile provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileContribution {
    /// Canonical semantic identity of the retained definition.
    pub identity: ContributionIdentity,
    /// Fully resolved definition rendered into the registry.
    pub definition: Contribution,
    /// Every package sharing ownership, in stable package-id order.
    pub owners: Vec<ProfilePackageId>,
}

impl From<ComposedContribution> for AppliedProfileContribution {
    fn from(contribution: ComposedContribution) -> Self {
        Self {
            identity: contribution.identity,
            definition: contribution.definition,
            owners: contribution.owners,
        }
    }
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
    pub region_id: String,
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

/// Canonical repository-local provenance for one installed profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileRecord {
    /// Stable profile identifier.
    pub id: String,
    /// Installed package version.
    pub version: String,
    /// Package discovery source.
    pub origin: ProfileOrigin,
    /// Digest of the complete package manifest and content.
    pub package_hash: String,
    /// Canonical public values and source kinds used to resolve this package.
    pub variables: ResolvedVariables,
    /// Digests of every installed package target, keyed by repository-relative path.
    pub target_hashes: BTreeMap<String, String>,
    /// Resolved semantic declarations and every package sharing each definition.
    pub contributions: Vec<AppliedProfileContribution>,
}

impl AppliedProfileRecord {
    /// Construct canonical installed-profile provenance from typed package metadata.
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        origin: ProfileOrigin,
        package_hash: impl Into<String>,
        variables: ResolvedVariables,
        target_hashes: BTreeMap<String, String>,
        contributions: Vec<AppliedProfileContribution>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            origin,
            package_hash: package_hash.into(),
            variables,
            target_hashes,
            contributions,
        }
    }

    /// Whether the immutable package provenance agrees, excluding repository-wide
    /// semantic ownership that is resolved at application time.
    pub fn matches_package_provenance(&self, expected: &Self) -> bool {
        self.id == expected.id
            && self.version == expected.version
            && self.origin == expected.origin
            && self.package_hash == expected.package_hash
            && self.variables == expected.variables
            && self.target_hashes == expected.target_hashes
    }

    /// Encode the stable installed-record image.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// Neutral profile package input consumed by the one materialization dispatcher.
#[derive(Clone)]
pub struct ProfileApplicationInput {
    pub id: String,
    pub version: String,
    pub package_hash: String,
    /// Exact public values and source kinds that produced `claims`.
    pub variables: ResolvedVariables,
    pub target_hashes: BTreeMap<String, String>,
    pub origin: ProfileOrigin,
    pub claims: ProfileClaims,
    pub record_path: VirtualPath,
}

impl ProfileApplicationInput {
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

    pub(crate) fn record(
        &self,
        contributions: Vec<AppliedProfileContribution>,
    ) -> AppliedProfileRecord {
        AppliedProfileRecord::new(
            self.id.clone(),
            self.version.clone(),
            self.origin.clone(),
            self.package_hash.clone(),
            self.variables.clone(),
            self.target_hashes.clone(),
            contributions,
        )
    }
}

/// Enumerate paths implied by a profile's proposed declarations before rendering.
pub(crate) fn profile_capture_closure(
    base: &RepositoryImage,
    claims: &ProfileClaims,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let record_paths = applied_profile_record_paths(base)?;
    if record_paths
        .iter()
        .any(|path| !base.capture_spec().contains_path(path))
    {
        return Ok(record_paths);
    }
    let composed = compose_profile_contributions(base, claims)?;
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
    pub contributions: Vec<AppliedProfileContribution>,
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
/// (the typed `ApplyProfile` materialization request) decides which targets to
/// write.
pub(super) fn compose_profile_targets(
    base: &RepositoryImage,
    claims: ProfileClaims,
) -> Result<ProfileTargetComposition, RepositoryStateError> {
    let composed = compose_profile_contributions(base, &claims)?;
    let mut targets = render_composed_contributions(base, &composed)?;
    let package_id = claims.package_id.clone();
    let projection_targets = configured_projection_targets(base, &targets)?;
    for asset in claims.assets {
        let path = asset.claim.target().clone();
        if !asset.replace_owned && !projection_targets.contains(&path) {
            if let RepositoryEntry::File { bytes, .. } =
                base.entry(&path).map_err(ProducerError::from)?
            {
                if bytes != &asset.bytes {
                    return Err(RepositoryStateError::ProfileTargetConflict(
                        ProfileTargetConflictError {
                            occupant: profile_conflict_occupant(base, &path)?,
                            candidate: package_id.clone(),
                            path,
                        },
                    ));
                }
            }
        }
        targets.insert(path, (asset.bytes, asset.mode));
    }
    // Profile-owned regions compose over the captured base; a region target keeps
    // its captured file mode (a fresh target is Regular), matching the profile's
    // region-target mode contract.
    let regions = claims.regions.into_iter().map(|region| {
        let path = region.claim.target().clone();
        let claim = ManagedDocumentClaim::Region {
            owner: region.claim.owner().to_string(),
            region_id: region.region_id.clone(),
            begin: format!("<!-- jit:{}:begin -->", region.region_id).into_bytes(),
            end: format!("<!-- jit:{}:end -->", region.region_id).into_bytes(),
            content: region.content,
            placement: RegionPlacement::AppendIfAbsent,
        };
        (path, claim)
    });
    for (path, bytes) in compose_managed_documents(base, regions)? {
        let mode = existing_file_mode(base, &path)?;
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
    Ok(ProfileTargetComposition {
        targets,
        contributions: composed.into_iter().map(Into::into).collect(),
    })
}

/// Check all selected package contributions against one captured repository image.
///
/// This is deliberately a read-only semantic preflight. Publication remains the
/// existing per-package path, while a conflict anywhere in the selected set is
/// reported before that path can publish an earlier package.
pub(crate) fn preflight_profile_contributions(
    base: &RepositoryImage,
    candidates: Vec<ProfileContributionClaim>,
) -> Result<(), RepositoryStateError> {
    let existing = existing_contribution_claims(base, &candidates)?;
    compose_resolved_contributions(existing, candidates)
        .map(|_| ())
        .map_err(Into::into)
}

/// Compose a candidate package's claims with the repository's per-identity
/// ownership evidence before any registry renderer receives a definition.
fn compose_profile_contributions(
    base: &RepositoryImage,
    claims: &ProfileClaims,
) -> Result<Vec<ComposedContribution>, RepositoryStateError> {
    let existing = existing_contribution_claims(base, &claims.contributions)?;
    compose_resolved_contributions(existing, claims.contributions.clone()).map_err(Into::into)
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

/// Resolve recorded package ownership and repository-authored declarations for
/// exactly the semantic identities a candidate contributes.
fn existing_contribution_claims(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<Vec<ExistingContributionClaim>, RepositoryStateError> {
    let candidate_identities = candidates
        .iter()
        .map(|claim| claim.contribution.semantic_identity())
        .collect::<BTreeSet<_>>();
    let package_claims = applied_profile_record_paths(base)?
        .into_iter()
        .map(|path| {
            let RepositoryEntry::File { bytes, .. } =
                base.entry(&path).map_err(ProducerError::from)?
            else {
                return Ok(Vec::new());
            };
            let record: AppliedProfileRecord = serde_json::from_slice(bytes).map_err(|source| {
                ProducerError::ProfileRecordParse {
                    path: path.repository_relative(),
                    source,
                }
            })?;
            Ok(record
                .contributions
                .into_iter()
                .filter(|contribution| {
                    candidate_identities.contains(&contribution.definition.semantic_identity())
                })
                .flat_map(|contribution| {
                    contribution.owners.into_iter().map(move |package_id| {
                        ExistingContributionClaim::Package(ProfileContributionClaim {
                            package_id,
                            contribution: contribution.definition.clone(),
                        })
                    })
                })
                .collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>, RepositoryStateError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let package_identities = package_claims
        .iter()
        .filter_map(|claim| match claim {
            ExistingContributionClaim::Package(claim) => {
                Some(claim.contribution.semantic_identity())
            }
            ExistingContributionClaim::Repository(_) => None,
        })
        .collect::<BTreeSet<_>>();
    let repository_claims = candidates
        .iter()
        .filter(|claim| !package_identities.contains(&claim.contribution.semantic_identity()))
        .map(|claim| repository_definition(base, &claim.contribution))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .map(ExistingContributionClaim::Repository)
        .collect::<Vec<_>>();
    Ok(package_claims
        .into_iter()
        .chain(repository_claims)
        .collect())
}

/// Read the repository definition matching one candidate identity, if authored.
fn repository_definition(
    base: &RepositoryImage,
    candidate: &Contribution,
) -> Result<Option<Contribution>, RepositoryStateError> {
    let registry = candidate.registry_path();
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
    match candidate {
        Contribution::Scalar { target, .. } => {
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
        Contribution::MapEntry {
            target, identity, ..
        } => Ok(
            semantic_map_entry(&semantic, target.table_path(), identity).map(|value| {
                Contribution::MapEntry {
                    target: *target,
                    identity: identity.clone(),
                    value: value.clone(),
                }
            }),
        ),
        Contribution::SetString { target, value } => {
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
                .then(|| candidate.clone()))
        }
        Contribution::KeyedArray {
            target, identity, ..
        } => {
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
                        == Some(identity)
                })
                .cloned()
                .map(|value| Contribution::KeyedArray {
                    target: *target,
                    identity: identity.clone(),
                    value,
                }))
        }
        Contribution::Projection { name, .. } => semantic
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
    let target = target.repository_relative();
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
        if record.target_hashes.contains_key(&target) {
            return Ok(ProfileConflictOccupant::Package(ProfilePackageId::new(
                record.id,
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
    };
    use std::collections::BTreeMap;

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
        let target_name = target.repository_relative();
        let target_hashes = package_owns_target
            .then(|| (target_name, "target-hash".to_string()))
            .into_iter()
            .collect();
        let record = AppliedProfileRecord::new(
            "base-package",
            "1.0.0",
            ProfileOrigin::Directory(
                crate::repository_state::RootRelativePath::parse("packages/base-package")
                    .expect("a canonical package location"),
            ),
            "package-hash",
            ResolvedVariables::default(),
            target_hashes,
            Vec::new(),
        );
        let record_bytes = record.to_bytes().unwrap();
        let record_path = VirtualPath::data("profiles/base-package.json").unwrap();
        let profiles = VirtualPath::PROFILES;
        let mut spec = CaptureSpec::phase_one(
            [
                VirtualPath::CONFIG,
                profiles.clone(),
                record_path.clone(),
                target.clone(),
            ],
            CaptureBudget {
                max_paths: 8,
                max_listings: 1,
                max_bytes: 4096,
                max_depth: 8,
            },
        )
        .unwrap();
        spec.discover_listing(profiles.clone()).unwrap();
        let record_identity = EntryIdentity::for_bytes("record", &record_bytes).unwrap();
        let entries = BTreeMap::from([
            (VirtualPath::CONFIG, RepositoryEntry::Absent),
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
}
