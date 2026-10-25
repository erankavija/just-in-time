//! Profile-selection materialization: compose a profile package's canonical claims
//! (declaration-overlay registry edits, exact assets, and managed regions) plus the
//! configured projections those declarations imply into the exact set of
//! profile-owned targets.
//!
//! `repository_state` owns this composition; the profile package produces the
//! neutral [`ProfileClaims`] and the command captures the base image and applies the
//! resulting delta. Package parsing, storage, and command code remain outside
//! this module; the typed aggregate profile-selection materialization request
//! sits between them.

use std::collections::{BTreeMap, BTreeSet};

use super::materialize::{
    assemble_config, compose_configured_projections, serialized_default_ruleset,
};
use super::{
    apply_overlay, compose_managed_documents, declarations_from_image, FileMode,
    ManagedDocumentClaim, ProducerError, ProfileRegistryParseError, ProfileTargetDisposition,
    RegionPlacement, RepositoryAction, RepositoryEntry, RepositoryImage, RepositoryStateError,
    TargetClaim, VirtualPath,
};
use crate::config::{ProjectionKinds, ProjectionMode, ProjectionStyle};
use crate::domain::identity::ProfileId;
use crate::profile::{RegionId, ResolvedVariables};
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ContributionConflictOwner {
    /// A package being applied authored one of the definitions.
    Package(ProfilePackageId),
}

impl std::fmt::Display for ContributionConflictOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Package(package_id) => write!(formatter, "package {package_id}"),
        }
    }
}

/// What composition decided about one semantic identity.
///
/// Composition decides every identity it is asked about rather than refusing
/// the first contested one: a selection is refused as a whole, and an adopter
/// inspecting a report before publishing needs the whole answer rather than the
/// first disagreement a traversal reached.
#[derive(Debug, Clone, PartialEq)]
pub enum ContributionOutcome {
    /// Every contributor of the identity resolved one shared definition.
    Agreed(ComposedContribution),
    /// Contributors resolved definitions that differ, so none of them wins.
    Contested {
        /// Shared semantic identity with incompatible definitions.
        identity: ContributionIdentity,
        /// Every package defining a conflicting definition, sorted
        /// independently of selector occurrence.
        owners: Vec<ContributionConflictOwner>,
    },
}

impl ContributionOutcome {
    /// The semantic identity this outcome decided.
    pub fn identity(&self) -> &ContributionIdentity {
        match self {
            Self::Agreed(contribution) => &contribution.identity,
            Self::Contested { identity, .. } => identity,
        }
    }
}

/// Compose resolved package claims by semantic identity.
///
/// Only the packages being applied are parties: different package definitions
/// for an identity decide one order-independent
/// [`ContributionOutcome::Contested`] naming every package that declares it.
pub fn compose_resolved_contributions(
    candidates: impl IntoIterator<Item = ProfileContributionClaim>,
) -> Vec<ContributionOutcome> {
    let mut positions = BTreeMap::<ContributionIdentity, usize>::new();
    let mut grouped = Vec::<(ContributionIdentity, Vec<ProfileContributionClaim>)>::new();
    for claim in candidates {
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
    claims: Vec<ProfileContributionClaim>,
) -> Option<ContributionOutcome> {
    let definition = claims.first()?.contribution.clone();
    let contested = claims.iter().any(|claim| claim.contribution != definition);
    let owners = claims
        .into_iter()
        .map(|claim| claim.package_id)
        .collect::<BTreeSet<_>>();
    Some(if contested {
        ContributionOutcome::Contested {
            identity,
            owners: owners
                .into_iter()
                .map(ContributionConflictOwner::Package)
                .collect(),
        }
    } else {
        ContributionOutcome::Agreed(ComposedContribution {
            identity,
            definition,
            owners: owners.into_iter().collect(),
        })
    })
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
}

#[derive(Clone)]
pub struct ProfileRegionClaim {
    pub claim: TargetClaim,
    pub region_id: RegionId,
    pub content: Vec<u8>,
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

/// Why one profile-owned target or declaration cannot be published.
///
/// Only the packages being applied are parties to a conflict; the repository's
/// current value never is. [`Self::message`] is how a conflict reaches a reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileTargetConflict {
    /// Packages being applied define one target or semantic identity
    /// differently.
    ///
    /// No package holds authority over another's definitions, so composition
    /// picks no winner and the publication is refused.
    Contested {
        /// Every package declaring the target or identity, sorted independently
        /// of selector occurrence.
        owners: Vec<ContributionConflictOwner>,
    },
}

impl ProfileTargetConflict {
    /// A human clause naming why the target cannot be published.
    ///
    /// The refusal message and the difference report's reason are both this
    /// sentence, so an adopter who inspected a decision and an adopter whose
    /// publication refused it read the same words.
    pub fn message(&self) -> String {
        match self {
            Self::Contested { owners } => format!(
                "is defined differently by {}",
                owners
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }
}

/// What one profile decision is about.
///
/// A package claims two kinds of thing, and a refusal names which one it
/// refused: a repository file it publishes bytes into, or a semantic identity
/// it declares inside a registry. The registry file is not the identity's
/// subject — several packages contribute distinct identities to one registry —
/// so an identity is named by its canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileTargetSubject {
    /// A repository file the package publishes.
    File(VirtualPath),
    /// A semantic identity the package declares in a registry.
    Contribution(ContributionIdentity),
}

impl std::fmt::Display for ProfileTargetSubject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(path) => write!(formatter, "target '{}'", path.repository_relative()),
            Self::Contribution(identity) => write!(formatter, "declaration '{identity}'"),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProfileTargetSubjectWire<'a> {
    File { path: String },
    Contribution { identity: &'a ContributionIdentity },
}

impl Serialize for ProfileTargetSubject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match self {
            // Rendered through the same repository-relative spelling a message
            // names it with, not the wrapper type that carries it.
            Self::File(path) => ProfileTargetSubjectWire::File {
                path: path.repository_relative(),
            },
            Self::Contribution(identity) => ProfileTargetSubjectWire::Contribution { identity },
        };
        wire.serialize(serializer)
    }
}

/// One decision a profile selection cannot publish, named the way its reader
/// acts on it: the profile that decided it, what it decided about, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileTargetConflictEntry {
    /// Profile whose contribution was refused.
    pub owner: ProfileId,
    /// Canonical target or declaration that was refused.
    pub subject: ProfileTargetSubject,
    /// Why it cannot be published.
    pub conflict: ProfileTargetConflict,
}

impl std::fmt::Display for ProfileTargetConflictEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "profile {} {} {}",
            self.owner,
            self.subject,
            self.conflict.message()
        )
    }
}

/// Every target and declaration one profile selection cannot publish.
///
/// A selection is refused as a whole, so the refusal carries every decision
/// that refused it rather than the first one a composition reached: an adopter
/// that only ever sees one conflict per run cannot tell how much work resolving
/// the selection is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[error(
    "{} profile decision(s) cannot be published: {}",
    .conflicts.len(),
    .conflicts.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
)]
pub struct ProfileTargetConflictsError {
    /// Refused decisions, in the order the plan decided them.
    pub conflicts: Vec<ProfileTargetConflictEntry>,
}

impl ProfileTargetConflictsError {
    /// Collect refused decisions into the selection's one refusal.
    pub fn new(conflicts: Vec<ProfileTargetConflictEntry>) -> Self {
        Self { conflicts }
    }
}

/// A profile package's contribution to a repository, in canonical repository-state
/// vocabulary. Produced by `profile::package` from the immutable manifest and
/// consumed only by [`compose_profile_targets_with_context`].
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

/// The byte prefix every managed-region delimiter this engine writes begins
/// with. Text carrying it holds a region some owner manages.
const REGION_MARKER_PREFIX: &[u8] = b"<!-- jit:";

/// The exact delimiters one profile-owned managed region is published between.
///
/// Composition splices a region between these bytes.
fn region_delimiters(region_id: &RegionId) -> (Vec<u8>, Vec<u8>) {
    let delimiter = |edge: &str| {
        [
            REGION_MARKER_PREFIX,
            region_id.as_str().as_bytes(),
            format!(":{edge} -->").as_bytes(),
        ]
        .concat()
    };
    (delimiter("begin"), delimiter("end"))
}

/// Neutral profile package input consumed by the one materialization dispatcher.
#[derive(Clone)]
pub struct ProfileApplicationInput {
    /// Stable package identity carried from the resolved manifest.
    pub id: ProfileId,
    pub version: String,
    /// Exact public values and source kinds that produced `claims`.
    pub variables: ResolvedVariables,
    pub target_hashes: BTreeMap<String, String>,
    pub claims: ProfileClaims,
    /// Resolved candidates sharing one of this package's semantic identities.
    ///
    /// The command supplies this scoped selection or dependency-closure context
    /// before aggregate selection publication begins, so composition compares
    /// this package's definitions with the selection's without making unrelated
    /// package declarations part of this package's materialization.
    pub(crate) contribution_context: Vec<ProfileContributionClaim>,
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
}

/// Enumerate paths implied by a profile's proposed declarations before rendering.
pub(crate) fn profile_capture_closure(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let composition = compose_profile_contributions(base, &profile.contribution_context)?;
    let composed =
        owned_composed_contributions(composition.definitions, &profile.claims.package_id);
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
    Ok(super::materialize::render_capture_closure(
        proposed.layout(),
        &config,
        &[],
        rules.as_deref(),
    )?
    .into_iter()
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect())
}

/// Target bytes and the decisions profile composition derived for one package.
#[derive(Debug)]
pub(super) struct ProfileTargetComposition {
    pub targets: BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
    /// What this package decided about every target it composes.
    ///
    /// This is half of the whole decision, the other half being
    /// [`Self::contribution_decisions`]. A publication writes the composed
    /// values it names and refuses when it names a conflict; a difference
    /// report states the same decision without publishing any of it.
    pub decisions: BTreeMap<VirtualPath, ProfileTargetDecision>,
    /// What this package decided about every semantic identity it declares.
    ///
    /// A declaration is decided by its identity rather than by the registry
    /// file that holds it, because several packages contribute distinct
    /// identities to one registry and each of them is decided on its own.
    pub contribution_decisions: BTreeMap<ContributionIdentity, ProfileTargetDisposition>,
}

/// One target a package composes, and what it decided about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileTargetDecision {
    /// What this package decided.
    pub(crate) disposition: ProfileTargetDisposition,
    /// Platform-neutral mode of the composed value.
    pub(crate) mode: FileMode,
}

/// Asset targets that packages being applied declare with differing content,
/// each beside every package declaring it.
///
/// An asset target is one key, so it composes by the rule a semantic identity
/// composes by: identical restatements merge, and differing definitions are a
/// conflict naming every declaring package.
pub(super) fn contested_asset_targets<'a>(
    selection: impl IntoIterator<Item = &'a ProfileClaims>,
) -> BTreeMap<VirtualPath, Vec<ContributionConflictOwner>> {
    let mut declared =
        BTreeMap::<&VirtualPath, Vec<(&ProfilePackageId, &ProfileAssetClaim)>>::new();
    for claims in selection {
        for asset in &claims.assets {
            declared
                .entry(asset.claim.target())
                .or_default()
                .push((&claims.package_id, asset));
        }
    }
    declared
        .into_iter()
        .filter(|(_, declarations)| {
            declarations.iter().any(|(_, asset)| {
                (&asset.bytes, asset.mode) != (&declarations[0].1.bytes, declarations[0].1.mode)
            })
        })
        .map(|(path, declarations)| {
            let owners = declarations
                .into_iter()
                .map(|(package_id, _)| package_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(ContributionConflictOwner::Package)
                .collect();
            (path.clone(), owners)
        })
        .collect()
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
///
/// `contribution_context` holds the selected packages' definitions of this
/// package's semantic identities, and `contested_assets` the asset targets
/// selected packages declare differently ([`contested_asset_targets`]). The
/// repository's current content decides only whether a value is created,
/// updated, or already exact; it is never a conflict party.
pub(super) fn compose_profile_targets_with_context(
    base: &RepositoryImage,
    claims: ProfileClaims,
    contribution_context: Vec<ProfileContributionClaim>,
    contested_assets: &BTreeMap<VirtualPath, Vec<ContributionConflictOwner>>,
) -> Result<ProfileTargetComposition, RepositoryStateError> {
    let declared_identities = claims
        .contributions
        .iter()
        .map(|claim| claim.contribution.semantic_identity())
        .collect::<BTreeSet<_>>();
    let composition = compose_profile_contributions(base, &contribution_context)?;
    let contribution_decisions = composition
        .decisions
        .into_iter()
        .filter(|(identity, _)| declared_identities.contains(identity))
        .collect::<BTreeMap<_, _>>();
    let composed = owned_composed_contributions(composition.definitions, &claims.package_id);
    let mut targets = render_composed_contributions(base, &composed)?;
    let projection_targets = configured_projection_targets(base, &targets)?;
    let asset_paths = claims
        .assets
        .iter()
        .map(|asset| asset.claim.target().clone())
        .collect::<BTreeSet<_>>();
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
    let schema_overlay = serialized_default_ruleset(&config)?
        .schema_files
        .into_iter()
        .map(|schema| {
            Ok((
                VirtualPath::schema(&schema.name)?,
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

    // A configured projection derives the final bytes of an asset target it
    // rewrites, so the bytes packages declare there are not what they publish.
    let decisions = targets
        .iter()
        .map(|(path, (bytes, mode))| {
            let contested = contested_assets
                .get(path)
                .filter(|_| asset_paths.contains(path) && !projection_targets.contains(path));
            let disposition = match contested {
                Some(owners) => {
                    ProfileTargetDisposition::Conflict(ProfileTargetConflict::Contested {
                        owners: owners.clone(),
                    })
                }
                None => composed_target_disposition(base, path, bytes, *mode)?,
            };
            Ok((
                path.clone(),
                ProfileTargetDecision {
                    disposition,
                    mode: *mode,
                },
            ))
        })
        .collect::<Result<_, RepositoryStateError>>()?;
    Ok(ProfileTargetComposition {
        targets,
        decisions,
        contribution_decisions,
    })
}

/// Compare one composed target against the value the captured base holds for it.
fn composed_target_disposition(
    base: &RepositoryImage,
    path: &VirtualPath,
    bytes: &[u8],
    mode: FileMode,
) -> Result<ProfileTargetDisposition, RepositoryStateError> {
    Ok(match base.entry(path).map_err(ProducerError::from)? {
        RepositoryEntry::Absent => ProfileTargetDisposition::Create,
        RepositoryEntry::File {
            content: existing,
            mode: existing_mode,
        } if existing.bytes() == bytes && *existing_mode == mode => {
            ProfileTargetDisposition::Unchanged
        }
        _ => ProfileTargetDisposition::Update,
    })
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
    let composition = compose_profile_contributions(base, candidates)?;
    render_composed_contributions(base, &composition.definitions).map(|registries| {
        registries
            .into_iter()
            .map(|(path, (bytes, _))| (path, Some(bytes)))
            .collect()
    })
}

/// Return every registry path whose semantic definition contributes to this
/// operation's candidate context.
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

/// What one candidate context decided, beside the definitions its render
/// publishes.
struct ContributionComposition {
    /// Definitions a registry render publishes. A contested identity is absent:
    /// composition picks no winner among differing definitions, so the registry
    /// keeps what it holds and the publication is refused from the decision.
    definitions: Vec<ComposedContribution>,
    /// One decision per semantic identity the candidate context declares.
    decisions: BTreeMap<ContributionIdentity, ProfileTargetDisposition>,
}

/// Compose a candidate context before any registry renderer receives a
/// definition.
///
/// Every identity is decided rather than the first disagreement refusing the
/// whole composition: a contested identity is one decision an adopter reads
/// beside the rest, and the refusal is taken from that decision by
/// `ensure_publishable_targets` instead of originating here.
fn compose_profile_contributions(
    base: &RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<ContributionComposition, RepositoryStateError> {
    let mut definitions = Vec::new();
    let mut decisions = BTreeMap::new();
    for outcome in compose_resolved_contributions(candidates.iter().cloned()) {
        match outcome {
            ContributionOutcome::Contested { identity, owners } => {
                decisions.insert(
                    identity,
                    ProfileTargetDisposition::Conflict(ProfileTargetConflict::Contested { owners }),
                );
            }
            ContributionOutcome::Agreed(contribution) => {
                decisions.insert(
                    contribution.identity.clone(),
                    contribution_disposition(base, &contribution.identity)?,
                );
                definitions.push(contribution);
            }
        }
    }
    Ok(ContributionComposition {
        definitions,
        decisions,
    })
}

/// What publishing one agreed definition does to the registry.
///
/// A registry render publishes a declaration only where the registry has none
/// under its identity ([`render_composed_contribution`]), so an absent
/// declaration is created and a held one is left unchanged, whatever it says.
fn contribution_disposition(
    base: &RepositoryImage,
    identity: &ContributionIdentity,
) -> Result<ProfileTargetDisposition, RepositoryStateError> {
    Ok(match repository_definition(base, identity)? {
        None => ProfileTargetDisposition::Create,
        Some(_) => ProfileTargetDisposition::Unchanged,
    })
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

/// Render definitions that have already passed one semantic composition.
/// This renderer deliberately contains no occupant convention.
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
                RepositoryEntry::File { content, mode } => Some((content.bytes().to_vec(), *mode)),
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

/// Read the repository definition carrying one semantic identity, if authored.
///
/// The identity alone decides both which registry is read and which declaration
/// inside it answers.
fn repository_definition(
    base: &RepositoryImage,
    identity: &ContributionIdentity,
) -> Result<Option<Contribution>, RepositoryStateError> {
    let registry = identity.registry.path();
    let path = base.layout().classify_repository_relative(registry)?;
    let bytes = match base.entry(&path).map_err(ProducerError::from)? {
        RepositoryEntry::Absent => return Ok(None),
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
        _ => {
            return Err(ProducerError::ProfileRegistryNotFile {
                target: registry.to_string(),
            }
            .into())
        }
    };
    contribution_in_registry(identity, &bytes)
}

/// The declaration one semantic identity addresses inside a registry's authored
/// text, if that registry declares it.
///
/// This is where a registry's text answers an identity, and it is deliberately
/// separate from where the text comes from: composition reads it out of a
/// captured image.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when `text` is not the UTF-8 TOML document
/// its registry is read from, or when the declaration the identity addresses is
/// not the shape its target describes.
pub(crate) fn contribution_in_registry(
    identity: &ContributionIdentity,
    text: &[u8],
) -> Result<Option<Contribution>, RepositoryStateError> {
    let registry = identity.registry.path();
    let document = MergeDocument::load(registry, Some((text.to_vec(), FileMode::Regular)))?;
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

/// The value one contribution declares, as its own model states it.
///
/// Every variant carries its declaration in one `value`, so this is the single
/// answer to "what does this contribution say" for the packaged-carrier
/// comparison.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when a projection configuration does not
/// serialize.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn contributed_json_value(
    contribution: &Contribution,
) -> Result<JsonValue, RepositoryStateError> {
    match contribution {
        Contribution::KeyedArray { value, .. } | Contribution::MapEntry { value, .. } => {
            Ok(value.clone())
        }
        Contribution::Scalar { value, .. } | Contribution::SetString { value, .. } => {
            Ok(JsonValue::String(value.clone()))
        }
        Contribution::Projection { value, .. } => serde_json::to_value(value).map_err(|error| {
            profile_registry_error(
                contribution.registry_path(),
                ProfileRegistryParseError::ProjectionDefinition(error.to_string()),
            )
        }),
    }
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
        CaptureBudget, CaptureSpec, CapturedBytes, RepositoryLayout, RepositoryRootEvidence,
    };
    use std::collections::BTreeMap;

    /// A captured image whose configuration registry holds `config` and whose
    /// named targets hold their bytes, every other registry absent.
    ///
    /// Every repository shape a composition decides against is stated through
    /// this one builder (`@/inv/shared-test-contracts`).
    fn image_holding(
        config: &[u8],
        targets: &[(VirtualPath, &[u8])],
    ) -> (RepositoryImage, RepositoryLayout) {
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let registries = [
            VirtualPath::CONFIG,
            VirtualPath::GATES,
            VirtualPath::INVARIANTS,
            VirtualPath::RULES,
            VirtualPath::TEMPLATES,
        ];
        let mut spec = CaptureSpec::new(
            registries
                .iter()
                .cloned()
                .chain(targets.iter().map(|(target, _)| target.clone())),
            CaptureBudget {
                max_listings: 0,
                max_bytes: 65536,
                max_depth: 8,
            },
        )
        .unwrap();
        spec.discover_paths([]).unwrap();
        let mut entries = registries
            .into_iter()
            .map(|path| (path, RepositoryEntry::Absent))
            .collect::<BTreeMap<_, _>>();
        entries.insert(
            VirtualPath::CONFIG,
            RepositoryEntry::File {
                content: CapturedBytes::new("config", config.to_vec()).unwrap(),
                mode: FileMode::Regular,
            },
        );
        entries.extend(targets.iter().map(|(target, bytes)| {
            (
                target.clone(),
                RepositoryEntry::File {
                    content: CapturedBytes::new("target", bytes.to_vec()).unwrap(),
                    mode: FileMode::Regular,
                },
            )
        }));
        (
            RepositoryImage::close(
                layout.clone(),
                spec,
                entries,
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

    fn package_owners(packages: &[&str]) -> Vec<ContributionConflictOwner> {
        packages
            .iter()
            .map(|package| ContributionConflictOwner::Package(ProfilePackageId::new(*package)))
            .collect()
    }

    /// One package's asset claims, every target carrying `bytes`.
    fn asset_claims(
        layout: &RepositoryLayout,
        package: &str,
        targets: &[VirtualPath],
        bytes: &[u8],
    ) -> ProfileClaims {
        ProfileClaims {
            package_id: ProfilePackageId::new(package),
            contributions: Vec::new(),
            assets: targets
                .iter()
                .map(|target| ProfileAssetClaim {
                    claim: TargetClaim::new(
                        layout,
                        target.clone(),
                        format!("profile-asset:{}", target.repository_relative()),
                    )
                    .unwrap(),
                    bytes: bytes.to_vec(),
                    mode: FileMode::Regular,
                })
                .collect(),
            regions: Vec::new(),
        }
    }

    fn disposition_of<'a>(
        composition: &'a ProfileTargetComposition,
        target: &VirtualPath,
    ) -> &'a ProfileTargetDisposition {
        &composition
            .decisions
            .get(target)
            .expect("the composition decided this target")
            .disposition
    }

    /// The definitions `outcomes` agreed on, in the order they were decided.
    fn agreed_definitions(outcomes: Vec<ContributionOutcome>) -> Vec<ComposedContribution> {
        outcomes
            .into_iter()
            .map(|outcome| match outcome {
                ContributionOutcome::Agreed(contribution) => contribution,
                ContributionOutcome::Contested { identity, owners } => {
                    panic!("'{identity}' was contested between {owners:?}")
                }
            })
            .collect()
    }

    /// The one identity `outcomes` found contested, beside every owner defining
    /// it.
    fn contested_outcome(
        outcomes: Vec<ContributionOutcome>,
    ) -> (ContributionIdentity, Vec<ContributionConflictOwner>) {
        let mut contested = outcomes.into_iter().filter_map(|outcome| match outcome {
            ContributionOutcome::Contested { identity, owners } => Some((identity, owners)),
            ContributionOutcome::Agreed(_) => None,
        });
        let first = contested
            .next()
            .expect("differing definitions decide a contested identity");
        assert!(
            contested.next().is_none(),
            "this composition contests exactly one identity"
        );
        first
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

        let composed = agreed_definitions(compose_resolved_contributions([
            contribution_claim("workflow", contribution.clone()),
            contribution_claim("base", contribution),
        ]));

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
            let (identity, owners) = contested_outcome(compose_resolved_contributions(claims));

            assert_eq!(identity, first.semantic_identity());
            assert_eq!(owners, package_owners(&["base", "workflow"]));
        }
    }

    #[test]
    fn test_compose_resolved_contributions_keeps_distinct_identities_in_one_registry() {
        let composed = agreed_definitions(compose_resolved_contributions([
            contribution_claim("workflow", namespace_contribution("one", "First.")),
            contribution_claim("workflow", namespace_contribution("two", "Second.")),
        ]));

        assert_eq!(composed.len(), 2);
        assert!(composed
            .iter()
            .all(|contribution| contribution.owners == [ProfilePackageId::new("workflow")]));
    }

    /// The repository is never a conflict party: a registry already declaring
    /// the identity differently keeps its entry and the decision is no conflict.
    #[test]
    fn test_compose_profile_targets_never_contests_a_differing_registry_declaration() {
        let (image, layout) = image_holding(
            b"[namespaces.shared]\ndescription = \"Repository definition.\"\nunique = false\n",
            &[],
        );
        let contribution = namespace_contribution("shared", "Package definition.");
        let claims = ProfileClaims {
            contributions: vec![contribution_claim("workflow", contribution.clone())],
            ..asset_claims(&layout, "workflow", &[], b"")
        };

        let context = claims.contributions.clone();
        let composition =
            compose_profile_targets_with_context(&image, claims, context, &BTreeMap::new())
                .unwrap();

        assert_eq!(
            composition
                .contribution_decisions
                .get(&contribution.semantic_identity()),
            Some(&ProfileTargetDisposition::Unchanged)
        );
    }

    /// The repository is never a conflict party: a package asset over a file
    /// holding other bytes replaces it.
    #[test]
    fn test_compose_profile_targets_updates_an_asset_over_differing_repository_content() {
        let target = VirtualPath::worktree("docs/custom.txt").unwrap();
        let (image, layout) = image_holding(b"", &[(target.clone(), b"repository bytes")]);
        let claims = asset_claims(
            &layout,
            "workflow",
            std::slice::from_ref(&target),
            b"package bytes",
        );

        let context = claims.contributions.clone();
        let composition =
            compose_profile_targets_with_context(&image, claims, context, &BTreeMap::new())
                .unwrap();

        assert_eq!(
            disposition_of(&composition, &target),
            &ProfileTargetDisposition::Update
        );
    }

    /// An asset target is one key: packages being applied that declare it
    /// differently contest it, whatever order they are selected in, and an
    /// identical restatement composes.
    #[test]
    fn test_contested_asset_targets_names_every_package_declaring_a_target_differently() {
        let contested = VirtualPath::worktree("docs/contested.txt").unwrap();
        let restated = VirtualPath::worktree("docs/restated.txt").unwrap();
        let (_, layout) = image_holding(b"", &[]);
        let base = ProfileClaims {
            assets: asset_claims(
                &layout,
                "base",
                std::slice::from_ref(&contested),
                b"base bytes",
            )
            .assets
            .into_iter()
            .chain(asset_claims(&layout, "base", std::slice::from_ref(&restated), b"same").assets)
            .collect(),
            ..asset_claims(&layout, "base", &[], b"")
        };
        let workflow = ProfileClaims {
            assets: asset_claims(
                &layout,
                "workflow",
                std::slice::from_ref(&contested),
                b"workflow bytes",
            )
            .assets
            .into_iter()
            .chain(asset_claims(&layout, "workflow", &[restated], b"same").assets)
            .collect(),
            ..asset_claims(&layout, "workflow", &[], b"")
        };

        for selection in [[&base, &workflow], [&workflow, &base]] {
            assert_eq!(
                contested_asset_targets(selection),
                BTreeMap::from([(contested.clone(), package_owners(&["base", "workflow"]))])
            );
        }
    }

    /// A composition decides every target it was asked about, so a package
    /// contesting three targets answers with three refusals rather than the
    /// first one it reached.
    #[test]
    fn test_compose_profile_targets_with_context_refuses_every_contested_asset_target() {
        let targets = ["first.txt", "second.txt", "third.txt"]
            .map(|name| VirtualPath::worktree(name).unwrap())
            .to_vec();
        let (image, layout) = image_holding(b"", &[]);
        let workflow = asset_claims(&layout, "workflow", &targets, b"workflow bytes");
        let base = asset_claims(&layout, "base", &targets, b"base bytes");
        let contested = contested_asset_targets([&workflow, &base]);

        let composition =
            compose_profile_targets_with_context(&image, workflow, Vec::new(), &contested)
                .expect("a contested target is decided, not refused, by composition");

        for target in &targets {
            assert_eq!(
                disposition_of(&composition, target),
                &ProfileTargetDisposition::Conflict(ProfileTargetConflict::Contested {
                    owners: package_owners(&["base", "workflow"]),
                }),
                "every contested target is decided, not only the first one reached"
            );
        }
    }

    /// A refusal reaches a reader as rendered facts, never as the Rust types
    /// that carry them, and states no remedy.
    #[test]
    fn test_profile_target_conflict_entry_message_names_profile_subject_and_packages() {
        let identity = namespace_contribution("shared", "Shared vocabulary.").semantic_identity();
        let entries = [
            ProfileTargetSubject::File(VirtualPath::data("custom.txt").unwrap()),
            ProfileTargetSubject::Contribution(identity),
        ]
        .map(|subject| ProfileTargetConflictEntry {
            owner: "workflow".try_into().expect("a canonical profile id"),
            subject,
            conflict: ProfileTargetConflict::Contested {
                owners: package_owners(&["base-package", "workflow"]),
            },
        });

        for entry in &entries {
            let message = entry.to_string();
            for leaked in ["ContributionConflictOwner", "Package(", "{"] {
                assert!(
                    !message.contains(leaked),
                    "a refusal names facts, not Rust syntax: {message}"
                );
            }
            let subject = match &entry.subject {
                ProfileTargetSubject::File(path) => path.repository_relative(),
                ProfileTargetSubject::Contribution(identity) => identity.to_string(),
            };
            assert!(
                message.contains(&subject)
                    && message.contains(entry.owner.as_str())
                    && message.contains("base-package"),
                "a refusal names the profile, its subject, and every party: {message}"
            );
        }
    }

    #[test]
    fn test_profile_target_conflict_entry_serializes_owner_subject_and_conflict() {
        let entry = ProfileTargetConflictEntry {
            owner: ProfileId::try_from("captured").expect("a canonical test profile id"),
            subject: ProfileTargetSubject::File(
                VirtualPath::worktree("docs/guide.md").expect("a canonical worktree path"),
            ),
            conflict: ProfileTargetConflict::Contested {
                owners: package_owners(&["captured", "other"]),
            },
        };

        let serialized = serde_json::to_value(&entry).expect("a conflict entry serializes");

        assert_eq!(serialized["owner"], "captured");
        assert_eq!(serialized["subject"]["kind"], "file");
        assert_eq!(serialized["subject"]["path"], "docs/guide.md");
        assert_eq!(serialized["conflict"]["kind"], "contested");
        assert_eq!(
            serialized["conflict"]["owners"].as_array().map(Vec::len),
            Some(2)
        );
        assert!(serialized["conflict"].get("remedy").is_none());
    }

    #[test]
    fn test_profile_target_subject_serializes_a_contribution_by_its_canonical_identity() {
        let subject = ProfileTargetSubject::Contribution(ContributionIdentity {
            registry: ContributionRegistry::Config,
            target: ContributionIdentityTarget::Projection {
                name: "docs".to_string(),
            },
        });

        let serialized = serde_json::to_value(&subject).expect("a subject serializes");

        assert_eq!(serialized["kind"], "contribution");
        assert_eq!(serialized["identity"]["registry"], "config");
        assert_eq!(serialized["identity"]["target"]["kind"], "projection");
        assert_eq!(serialized["identity"]["target"]["name"], "docs");
    }

    #[test]
    fn test_profile_target_conflicts_error_serializes_as_its_conflict_list() {
        let entry = ProfileTargetConflictEntry {
            owner: ProfileId::try_from("captured").expect("a canonical test profile id"),
            subject: ProfileTargetSubject::File(
                VirtualPath::worktree("docs/guide.md").expect("a canonical worktree path"),
            ),
            conflict: ProfileTargetConflict::Contested {
                owners: package_owners(&["captured", "other"]),
            },
        };
        let error = ProfileTargetConflictsError::new(vec![entry]);

        let serialized = serde_json::to_value(&error).expect("a conflicts error serializes");

        assert_eq!(serialized["conflicts"].as_array().unwrap().len(), 1);
        assert_eq!(serialized["conflicts"][0]["owner"], "captured");
        assert_eq!(serialized["conflicts"][0]["conflict"]["kind"], "contested");
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
        let mut spec = CaptureSpec::new(
            [path.clone()],
            CaptureBudget {
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
                    content: CapturedBytes::new(".jit/config.toml", authored.to_vec()).unwrap(),
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        assert!(render_composed_contributions(&image, &[])
            .unwrap()
            .is_empty());
    }
}
