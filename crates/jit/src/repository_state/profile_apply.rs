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
    /// The registry this contribution declares into.
    pub fn registry(&self) -> ContributionRegistry {
        match self {
            Self::Scalar { .. }
            | Self::MapEntry { .. }
            | Self::SetString { .. }
            | Self::Projection { .. } => ContributionRegistry::Config,
            Self::KeyedArray { target, .. } => target.registry(),
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
            registry: self.registry(),
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
    /// Virtual path of the registry file this vocabulary names.
    pub fn path(self) -> VirtualPath {
        match self {
            Self::Config => VirtualPath::CONFIG,
            Self::Gates => VirtualPath::GATES,
            Self::Invariants => VirtualPath::INVARIANTS,
            Self::Rules => VirtualPath::RULES,
            Self::Templates => VirtualPath::TEMPLATES,
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
        write!(formatter, "{}:", self.registry.path().repository_relative())?;
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
    pub(crate) package_id: ProfileId,
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
    pub owners: Vec<ProfileId>,
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
    /// Contributors with no dependency path between them resolved definitions
    /// that differ, so none of them wins.
    Contested {
        /// Shared semantic identity with incompatible definitions.
        identity: ContributionIdentity,
        /// Every package defining a conflicting definition, sorted
        /// independently of selector occurrence.
        owners: Vec<ProfileId>,
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
/// Only the packages being applied are parties, and
/// `drop_overridden_claims` has already removed a definition a dependent
/// overrides. Definitions that still differ decide one order-independent
/// [`ContributionOutcome::Contested`] naming every package that declares one.
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
            owners: owners.into_iter().collect(),
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
    fn registry(self) -> ContributionRegistry {
        match self {
            Self::Gates => ContributionRegistry::Gates,
            Self::Invariants => ContributionRegistry::Invariants,
            Self::Rules => ContributionRegistry::Rules,
            Self::Templates => ContributionRegistry::Templates,
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

/// Why one profile-owned target or declaration cannot be published.
///
/// Only the packages being applied are parties to a conflict; the repository's
/// current value never is. [`Self::message`] is how a conflict reaches a reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileTargetConflict {
    /// Packages with no dependency path between them define one target or
    /// semantic identity differently.
    ///
    /// Neither holds authority over the other's definitions, so composition
    /// picks no winner and the publication is refused.
    Contested {
        /// Every package whose differing definition remains, sorted
        /// independently of selector occurrence.
        owners: Vec<ProfileId>,
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
                    .map(|owner| format!("package {owner}"))
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileTargetSubject {
    /// A repository file the package publishes.
    File {
        #[serde(serialize_with = "VirtualPath::serialize_repository_relative")]
        #[schemars(with = "String")]
        path: VirtualPath,
    },
    /// A semantic identity the package declares in a registry.
    Contribution { identity: ContributionIdentity },
}

impl std::fmt::Display for ProfileTargetSubject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(formatter, "target '{}'", path.repository_relative()),
            Self::Contribution { identity } => write!(formatter, "declaration '{identity}'"),
        }
    }
}

/// One decision a profile selection cannot publish, named the way its reader
/// acts on it: the profile that decided it, what it decided about, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, JsonSchema)]
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
/// consumed only by `compose_profile_targets_with_context`.
#[derive(Clone)]
pub struct ProfileClaims {
    /// Package that authored these claims.
    pub package_id: ProfileId,
    /// Fully resolved semantic contributions that retain their package owner.
    pub contributions: Vec<ProfileContributionClaim>,
    pub assets: Vec<ProfileAssetClaim>,
    pub regions: Vec<ProfileRegionClaim>,
}

/// One key a package declares inside a file: a whole asset target, or a managed
/// region of a target.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FileKey {
    path: VirtualPath,
    region: Option<RegionId>,
}

impl FileKey {
    fn of_asset(asset: &ProfileAssetClaim) -> Self {
        Self {
            path: asset.claim.target().clone(),
            region: None,
        }
    }

    fn of_region(region: &ProfileRegionClaim) -> Self {
        Self {
            path: region.claim.target().clone(),
            region: Some(region.region_id.clone()),
        }
    }
}

/// The value two declarations of one [`FileKey`] compare by.
type FileValue<'a> = (&'a [u8], Option<FileMode>);

impl ProfileClaims {
    /// This package's file keys beside their values.
    fn file_declarations(&self) -> impl Iterator<Item = (FileKey, &ProfileId, FileValue<'_>)> {
        let assets = self.assets.iter().map(|asset| {
            (
                FileKey::of_asset(asset),
                &self.package_id,
                (asset.bytes.as_slice(), Some(asset.mode)),
            )
        });
        let regions = self.regions.iter().map(|region| {
            (
                FileKey::of_region(region),
                &self.package_id,
                (region.content.as_slice(), None),
            )
        });
        assets.chain(regions)
    }

    pub(crate) fn target_paths(&self) -> Vec<VirtualPath> {
        self.contributions
            .iter()
            .map(|claim| claim.contribution.registry().path())
            .chain(self.assets.iter().map(|asset| asset.claim.target().clone()))
            .chain(
                self.regions
                    .iter()
                    .map(|region| region.claim.target().clone()),
            )
            .collect()
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
    pub version: semver::Version,
    /// Exact public values and source kinds that produced `claims`.
    pub variables: ResolvedVariables,
    /// Repository paths the package writes.
    pub targets: BTreeSet<String>,
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
        self.targets
            .contains(VirtualPath::CONFIG.repository_relative().as_str())
            || self
                .targets
                .contains(VirtualPath::RULES.repository_relative().as_str())
    }
}

/// Enumerate paths implied by a profile's proposed declarations before rendering.
pub(crate) fn profile_capture_closure(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let mut composition = compose_profile_contributions(base, &profile.contribution_context)?;
    let composed =
        owned_composed_contributions(composition.definitions, &profile.claims.package_id);
    let registries = render_composed_contributions(&mut composition.registries, &composed)?;
    let proposed = apply_overlay(
        base,
        registries
            .into_iter()
            .map(|(path, (bytes, _))| (path, Some(bytes))),
    )?;
    let config = super::materialize::assemble_config(&proposed)?;
    let rules = proposed
        .file_bytes(&VirtualPath::RULES)
        .map_err(ProducerError::from)?;
    Ok(super::materialize::render_capture_closure(
        proposed.layout(),
        &config,
        &[],
        super::materialize::schema_requests_of(&proposed, rules)?.as_deref(),
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

/// Remove each definition a dependent package overrides from `selection`.
///
/// `dependencies` maps each package to its transitive dependencies. A
/// definition is overridden when a package depending on its declarer, at any
/// depth, defines the key differently, so a common dependent's value settles
/// what its dependencies disagree on. Differing definitions no dependent
/// overrides remain for composition to refuse.
pub(crate) fn drop_overridden_claims(
    selection: Vec<ProfileClaims>,
    dependencies: &BTreeMap<ProfileId, BTreeSet<ProfileId>>,
) -> Vec<ProfileClaims> {
    let contributions = overridden_declarations(
        selection.iter().flat_map(|claims| {
            claims.contributions.iter().map(|claim| {
                (
                    claim.contribution.semantic_identity(),
                    &claims.package_id,
                    &claim.contribution,
                )
            })
        }),
        dependencies,
    );
    let files = overridden_declarations(
        selection.iter().flat_map(ProfileClaims::file_declarations),
        dependencies,
    );
    selection
        .into_iter()
        .map(|mut claims| {
            let package = claims.package_id.clone();
            claims.contributions.retain(|claim| {
                !contributions.contains(&(claim.contribution.semantic_identity(), package.clone()))
            });
            claims
                .assets
                .retain(|asset| !files.contains(&(FileKey::of_asset(asset), package.clone())));
            claims
                .regions
                .retain(|region| !files.contains(&(FileKey::of_region(region), package.clone())));
            claims
        })
        .collect()
}

/// Every `(key, package)` declaration a dependent overrides.
fn overridden_declarations<'a, K: Ord + Clone, V: PartialEq + 'a>(
    declarations: impl Iterator<Item = (K, &'a ProfileId, V)>,
    dependencies: &BTreeMap<ProfileId, BTreeSet<ProfileId>>,
) -> BTreeSet<(K, ProfileId)> {
    let depends_on = |package: &ProfileId, other: &ProfileId| {
        dependencies
            .get(package)
            .is_some_and(|required| required.contains(other))
    };
    let mut keys = BTreeMap::<K, Vec<(&ProfileId, V)>>::new();
    for (key, package, value) in declarations {
        keys.entry(key).or_default().push((package, value));
    }
    keys.into_iter()
        .flat_map(|(key, declared)| {
            declared
                .iter()
                .filter(|(package, value)| {
                    declared
                        .iter()
                        .any(|(other, stated)| stated != value && depends_on(other, package))
                })
                .map(|(package, _)| (key.clone(), (*package).clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Paths holding a file key that packages being applied declare differently,
/// each beside every package declaring it.
///
/// Asset targets and managed regions compose by the rule a semantic identity
/// composes by: identical restatements merge, and differing definitions are a
/// conflict naming every declaring package.
pub(super) fn contested_file_targets<'a>(
    selection: impl IntoIterator<Item = &'a ProfileClaims>,
) -> BTreeMap<VirtualPath, Vec<ProfileId>> {
    let mut declared = BTreeMap::<FileKey, Vec<(&ProfileId, FileValue<'_>)>>::new();
    for (key, package, value) in selection
        .into_iter()
        .flat_map(ProfileClaims::file_declarations)
    {
        declared.entry(key).or_default().push((package, value));
    }
    let mut contested = BTreeMap::<VirtualPath, BTreeSet<ProfileId>>::new();
    for (key, declarations) in declared {
        if declarations
            .iter()
            .any(|(_, value)| *value != declarations[0].1)
        {
            contested
                .entry(key.path)
                .or_default()
                .extend(declarations.into_iter().map(|(package, _)| package.clone()));
        }
    }
    contested
        .into_iter()
        .map(|(path, owners)| (path, owners.into_iter().collect()))
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
/// package's semantic identities, and `contested_files` the file targets
/// selected packages declare differently ([`contested_file_targets`]). The
/// repository's current content decides only whether a value is created,
/// updated, or already exact; it is never a conflict party.
pub(super) fn compose_profile_targets_with_context(
    base: &RepositoryImage,
    claims: ProfileClaims,
    contribution_context: Vec<ProfileContributionClaim>,
    contested_files: &BTreeMap<VirtualPath, Vec<ProfileId>>,
) -> Result<ProfileTargetComposition, RepositoryStateError> {
    let declared_identities = claims
        .contributions
        .iter()
        .map(|claim| claim.contribution.semantic_identity())
        .collect::<BTreeSet<_>>();
    let mut composition = compose_profile_contributions(base, &contribution_context)?;
    let contribution_decisions = composition
        .decisions
        .into_iter()
        .filter(|(identity, _)| declared_identities.contains(identity))
        .collect::<BTreeMap<_, _>>();
    let composed = owned_composed_contributions(composition.definitions, &claims.package_id);
    let mut targets = render_composed_contributions(&mut composition.registries, &composed)?;
    let projection_targets = configured_projection_targets(base, &targets)?;
    let file_paths = claims
        .assets
        .iter()
        .map(|asset| asset.claim.target().clone())
        .chain(
            claims
                .regions
                .iter()
                .map(|region| region.claim.target().clone()),
        )
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
    let projection_actions =
        compose_configured_projections(&proposed, &declarations.borrowed(), None)?.actions;
    for action in projection_actions {
        if let RepositoryAction::WriteFile { path, bytes, .. } = action {
            if let Some(target) = targets.get_mut(&path) {
                target.0 = bytes;
            }
        }
    }

    // A configured projection derives the final bytes of a file target it
    // rewrites, so the bytes packages declare there are not what they publish.
    let decisions = targets
        .iter()
        .map(|(path, (bytes, mode))| {
            let contested = contested_files
                .get(path)
                .filter(|_| file_paths.contains(path) && !projection_targets.contains(path));
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
    let mut composition = compose_profile_contributions(base, candidates)?;
    render_composed_contributions(&mut composition.registries, &composition.definitions).map(
        |registries| {
            registries
                .into_iter()
                .map(|(path, (bytes, _))| (path, Some(bytes)))
                .collect()
        },
    )
}

/// Return every registry path whose semantic definition contributes to this
/// operation's candidate context.
pub(crate) fn profile_contribution_target_paths(
    candidates: &[ProfileContributionClaim],
) -> Vec<VirtualPath> {
    candidates
        .iter()
        .map(|claim| claim.contribution.registry().path())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What one candidate context decided, beside the definitions its render
/// publishes.
struct ContributionComposition<'a> {
    /// Definitions a registry render publishes. A contested identity is absent:
    /// composition picks no winner among differing definitions, so the registry
    /// keeps what it holds and the publication is refused from the decision.
    definitions: Vec<ComposedContribution>,
    /// One decision per semantic identity the candidate context declares.
    decisions: BTreeMap<ContributionIdentity, ProfileTargetDisposition>,
    /// The registries the decisions read, for the render to reuse.
    registries: RegistryDocuments<'a>,
}

/// Compose a candidate context before any registry renderer receives a
/// definition.
///
/// Every identity is decided rather than the first disagreement refusing the
/// whole composition: a contested identity is one decision an adopter reads
/// beside the rest, and the refusal is taken from that decision by
/// `ensure_publishable_targets` instead of originating here.
fn compose_profile_contributions<'a>(
    base: &'a RepositoryImage,
    candidates: &[ProfileContributionClaim],
) -> Result<ContributionComposition<'a>, RepositoryStateError> {
    let mut registries = RegistryDocuments::new(base);
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
                    contribution_disposition(&mut registries, &contribution)?,
                );
                definitions.push(contribution);
            }
        }
    }
    Ok(ContributionComposition {
        definitions,
        decisions,
        registries,
    })
}

/// What publishing one agreed definition does to the registry
/// ([`render_composed_contribution`]).
fn contribution_disposition(
    registries: &mut RegistryDocuments<'_>,
    contribution: &ComposedContribution,
) -> Result<ProfileTargetDisposition, RepositoryStateError> {
    let registry = registries.parsed(contribution.identity.registry)?;
    Ok(
        match contribution_in_registry(&contribution.identity, registry)? {
            None => ProfileTargetDisposition::Create,
            Some(held) if held.contribution == contribution.definition => {
                ProfileTargetDisposition::Unchanged
            }
            Some(_) => ProfileTargetDisposition::Update,
        },
    )
}

fn owned_composed_contributions(
    contributions: Vec<ComposedContribution>,
    package_id: &ProfileId,
) -> Vec<ComposedContribution> {
    contributions
        .into_iter()
        .filter(|contribution| contribution.owners.contains(package_id))
        .collect()
}

/// Render definitions that have already passed one semantic composition.
/// This renderer deliberately contains no occupant convention.
fn render_composed_contributions(
    registries: &mut RegistryDocuments<'_>,
    contributions: &[ComposedContribution],
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, RepositoryStateError> {
    let rendered = contributions
        .iter()
        .map(|contribution| {
            let registry = contribution.definition.registry();
            render_composed_contribution(registries.parsed(registry)?, &contribution.definition)?;
            Ok(registry)
        })
        .collect::<Result<BTreeSet<_>, RepositoryStateError>>()?;
    rendered
        .into_iter()
        .map(|registry| {
            let parsed = registries.parsed(registry)?;
            Ok((
                registry.path(),
                (parsed.document.to_string().into_bytes(), parsed.mode),
            ))
        })
        .collect()
}

/// A declaration a registry holds under one semantic identity.
struct HeldEntry {
    contribution: Contribution,
    /// Its position, for a keyed array's entry.
    array_index: Option<usize>,
}

/// The one presence lookup: the declaration one semantic identity addresses
/// inside a parsed registry, if that registry declares it.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when the declaration the identity addresses
/// is not the shape its target describes.
fn contribution_in_registry(
    identity: &ContributionIdentity,
    parsed: &ParsedRegistry,
) -> Result<Option<HeldEntry>, RepositoryStateError> {
    let ParsedRegistry {
        name: registry,
        semantic,
        ..
    } = parsed;
    let registry = registry.as_str();
    let held = |contribution| HeldEntry {
        contribution,
        array_index: None,
    };
    match &identity.target {
        ContributionIdentityTarget::Scalar { target } => {
            let (table, key) = target.config_path();
            let Some(value) = semantic.get(table).and_then(|table| table.get(key)) else {
                return Ok(None);
            };
            let value = value.as_str().ok_or_else(|| {
                profile_registry_error(registry, ProfileRegistryParseError::ScalarTargetNotString)
            })?;
            Ok(Some(held(Contribution::Scalar {
                target: *target,
                value: value.to_string(),
            })))
        }
        ContributionIdentityTarget::MapEntry { target, name } => {
            Ok(
                semantic_map_entry(semantic, target.table_path(), name).map(|value| {
                    held(Contribution::MapEntry {
                        target: *target,
                        identity: name.clone(),
                        value: value.clone(),
                    })
                }),
            )
        }
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
                .then(|| {
                    held(Contribution::SetString {
                        target: *target,
                        value: value.clone(),
                    })
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
            let field = target.identity_field();
            let identities = entries
                .iter()
                .map(|entry| {
                    entry.get(field).and_then(JsonValue::as_str).ok_or_else(|| {
                        profile_registry_error(
                            registry,
                            ProfileRegistryParseError::MissingIdentity {
                                field: field.to_string(),
                            },
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if identities.iter().collect::<BTreeSet<_>>().len() != identities.len() {
                return Err(profile_registry_error(
                    registry,
                    ProfileRegistryParseError::DuplicateIdentity {
                        field: field.to_string(),
                    },
                ));
            }
            Ok(identities
                .iter()
                .position(|actual| *actual == name)
                .map(|index| HeldEntry {
                    contribution: Contribution::KeyedArray {
                        target: *target,
                        identity: name.clone(),
                        value: entries[index].clone(),
                    },
                    array_index: Some(index),
                }))
        }
        ContributionIdentityTarget::Projection { name } => semantic
            .get("projection")
            .and_then(|projections| projections.get(name))
            .map(|value| {
                serde_json::from_value(value.clone())
                    .map(|value| {
                        held(Contribution::Projection {
                            name: name.clone(),
                            value,
                        })
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
    let parsed = super::configuration_overlaid(base, |path| {
        registries
            .get(path)
            .map(|(bytes, _)| Some(bytes.as_slice()))
    })?;
    let config = parsed
        .assembled()
        .map_err(|error| ProducerError::from(error.clone()))?;
    config
        .projection
        .iter()
        .flat_map(BTreeMap::values)
        .filter_map(|projection| projection.target.as_deref())
        .map(|target| {
            base.layout()
                .classify_repository_relative(target)
                .map_err(Into::into)
        })
        .collect()
}

/// One registry document, parsed once and rendered in place.
struct ParsedRegistry {
    /// Repository-relative spelling errors name it by.
    name: String,
    document: DocumentMut,
    /// The declarations as authored. A composition renders one definition per
    /// identity, each write stays under its own identity, and a keyed array grows
    /// by appending, so this stays correct for every identity not yet rendered.
    semantic: JsonValue,
    mode: FileMode,
}

impl ParsedRegistry {
    fn parse(
        name: &str,
        existing: Option<(&[u8], FileMode)>,
    ) -> Result<Self, RepositoryStateError> {
        let (bytes, mode) = existing.unwrap_or((&[], FileMode::Regular));
        let text = std::str::from_utf8(bytes)
            .map_err(|error| profile_registry_error(name, error.into()))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| profile_registry_error(name, error.into()))?;
        let semantic = toml_edit::de::from_document(document.clone())
            .map_err(|error| profile_registry_error(name, error.into()))?;
        Ok(Self {
            name: name.to_string(),
            document,
            semantic,
            mode,
        })
    }
}

/// The registries one composition reads and renders, each parsed on first use.
struct RegistryDocuments<'a> {
    base: &'a RepositoryImage,
    parsed: BTreeMap<ContributionRegistry, ParsedRegistry>,
    #[cfg(test)]
    parses: usize,
}

impl<'a> RegistryDocuments<'a> {
    fn new(base: &'a RepositoryImage) -> Self {
        Self {
            base,
            parsed: BTreeMap::new(),
            #[cfg(test)]
            parses: 0,
        }
    }

    fn parsed(
        &mut self,
        registry: ContributionRegistry,
    ) -> Result<&mut ParsedRegistry, RepositoryStateError> {
        use std::collections::btree_map::Entry;
        match self.parsed.entry(registry) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let path = registry.path();
                let name = path.repository_relative();
                let existing = match self.base.entry(&path).map_err(ProducerError::from)? {
                    RepositoryEntry::File { content, mode } => Some((content.bytes(), *mode)),
                    RepositoryEntry::Absent => None,
                    _ => return Err(ProducerError::ProfileRegistryNotFile { target: name }.into()),
                };
                #[cfg(test)]
                {
                    self.parses += 1;
                }
                Ok(entry.insert(ParsedRegistry::parse(&name, existing)?))
            }
        }
    }
}

fn profile_registry_error(target: &str, source: ProfileRegistryParseError) -> RepositoryStateError {
    ProducerError::ProfileRegistryParse {
        target: target.to_string(),
        source: Box::new(source),
    }
    .into()
}

/// Write `contribution` as the whole entry under its identity.
///
/// A registry already holding exactly this value keeps its bytes, so reapplying
/// a package does not reformat authored text.
fn render_composed_contribution(
    parsed: &mut ParsedRegistry,
    contribution: &Contribution,
) -> Result<(), RepositoryStateError> {
    let held = contribution_in_registry(&contribution.semantic_identity(), parsed)?;
    if held
        .as_ref()
        .is_some_and(|held| held.contribution == *contribution)
    {
        return Ok(());
    }
    let is_held = held.is_some();
    let registry = parsed.name.as_str();
    let document = &mut parsed.document;
    match contribution {
        Contribution::Scalar { target, value } => render_scalar(registry, document, *target, value),
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => render_map_entry(registry, document, *target, identity, is_held, value),
        Contribution::SetString { target, value } => {
            render_set_string(registry, document, *target, value)
        }
        Contribution::KeyedArray { target, value, .. } => render_keyed_array(
            registry,
            document,
            *target,
            held.and_then(|held| held.array_index),
            value,
        ),
        Contribution::Projection { name, value } => {
            render_projection(registry, document, name, is_held, value)
        }
    }
}

fn render_scalar(
    registry: &str,
    document: &mut DocumentMut,
    target: ScalarTarget,
    candidate: &str,
) -> Result<(), RepositoryStateError> {
    let (table, key) = target.config_path();
    ensure_table(document.as_table_mut(), table, registry)?.insert(key, candidate.into());
    Ok(())
}

fn render_map_entry(
    registry: &str,
    document: &mut DocumentMut,
    target: MapEntryTarget,
    identity: &str,
    is_held: bool,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    match target {
        MapEntryTarget::TypeHierarchyTypes => {
            ensure_inline_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", registry)?,
                "types",
                registry,
            )?
            .insert(identity, json_to_edit_value(candidate, registry)?);
        }
        MapEntryTarget::LabelAssociations => {
            ensure_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", registry)?,
                "label_associations",
                registry,
            )?
            .insert(
                identity,
                Item::Value(json_to_edit_value(candidate, registry)?),
            );
        }
        MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds => {
            let root = if target == MapEntryTarget::Namespaces {
                "namespaces"
            } else {
                "item_kinds"
            };
            replace_table_entry(
                ensure_table(document.as_table_mut(), root, registry)?,
                identity,
                is_held,
                json_object_to_table(candidate, registry)?,
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

/// A set member's identity is its value, so only an absent member reaches here.
fn render_set_string(
    registry: &str,
    document: &mut DocumentMut,
    target: SetStringTarget,
    candidate: &str,
) -> Result<(), RepositoryStateError> {
    let (table, key) = target.config_path();
    ensure_array(
        ensure_table(document.as_table_mut(), table, registry)?,
        key,
        registry,
    )?
    .push(candidate);
    Ok(())
}

fn render_keyed_array(
    registry: &str,
    document: &mut DocumentMut,
    target: KeyedArrayTarget,
    held_index: Option<usize>,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    let (array, preserved_comment) =
        ensure_array_of_tables(document.as_table_mut(), target.array_name(), registry)?;
    let mut table = json_object_to_table(candidate, registry)?;
    if let Some(prior) = held_index.and_then(|index| array.get_mut(index)) {
        *prior = in_place_of(prior, table);
        return Ok(());
    }
    if let Some(comment) = preserved_comment {
        table.decor_mut().set_prefix(comment);
    }
    array.push(table);
    Ok(())
}

fn render_projection(
    registry: &str,
    document: &mut DocumentMut,
    name: &str,
    is_held: bool,
    candidate: &CompleteProjectionConfig,
) -> Result<(), RepositoryStateError> {
    let candidate = serde_json::to_value(candidate).expect("projection config serializes");
    replace_table_entry(
        ensure_table(document.as_table_mut(), "projection", registry)?,
        name,
        is_held,
        json_object_to_table(&candidate, registry)?,
    );
    Ok(())
}

fn replace_table_entry(parent: &mut Table, key: &str, is_held: bool, replacement: Table) {
    let prior = parent.get(key).and_then(Item::as_table).filter(|_| is_held);
    let replacement = match prior {
        Some(prior) => in_place_of(prior, replacement),
        None => replacement,
    };
    parent.insert(key, Item::Table(replacement));
}

/// `replacement` at the document position and with the surrounding comments of
/// `prior`, so replacing an entry moves no other entry's text.
fn in_place_of(prior: &Table, mut replacement: Table) -> Table {
    *replacement.decor_mut() = prior.decor().clone();
    if let Some(position) = prior.position() {
        replacement.set_position(position);
    }
    replacement
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
    use crate::repository_state::RepositoryLayout;
    use crate::test_harness::image::{synthetic_layout, ImageBuilder};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn parsed_registry(
        identity: &ContributionIdentity,
        text: &[u8],
    ) -> Result<ParsedRegistry, RepositoryStateError> {
        ParsedRegistry::parse(
            &identity.registry.path().repository_relative(),
            Some((text, FileMode::Regular)),
        )
    }

    /// Render one contribution that has already passed semantic composition.
    fn render_contribution(
        document: &mut DocumentMut,
        contribution: &Contribution,
    ) -> Result<(), RepositoryStateError> {
        let text = document.to_string();
        let mut parsed = parsed_registry(&contribution.semantic_identity(), text.as_bytes())?;
        render_composed_contribution(&mut parsed, contribution)?;
        *document = parsed.document;
        Ok(())
    }

    /// The declaration `identity` addresses in a registry's text.
    fn declared_in(identity: &ContributionIdentity, text: &[u8]) -> Option<Contribution> {
        let parsed = parsed_registry(identity, text).unwrap();
        contribution_in_registry(identity, &parsed)
            .unwrap()
            .map(|held| held.contribution)
    }

    fn composed(definition: Contribution) -> ComposedContribution {
        ComposedContribution {
            identity: definition.semantic_identity(),
            definition,
            owners: Vec::new(),
        }
    }

    fn gate_contribution(key: &str, description: &str) -> Contribution {
        Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: key.to_string(),
            value: serde_json::json!({ "key": key, "description": description }),
        }
    }

    fn namespace_contribution(identity: &str, description: &str) -> Contribution {
        Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: identity.to_string(),
            value: serde_json::json!({ "description": description, "unique": false }),
        }
    }

    fn contribution_claim(package: &str, contribution: Contribution) -> ProfileContributionClaim {
        ProfileContributionClaim {
            package_id: ProfileId::try_from(package).unwrap(),
            contribution,
        }
    }

    fn package_owners(packages: &[&str]) -> Vec<ProfileId> {
        packages
            .iter()
            .map(|package| ProfileId::try_from(*package).unwrap())
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
            package_id: ProfileId::try_from(package).unwrap(),
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
    ) -> (ContributionIdentity, Vec<ProfileId>) {
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
                ProfileId::try_from("base").unwrap(),
                ProfileId::try_from("workflow").unwrap()
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
            .all(|contribution| contribution.owners == [ProfileId::try_from("workflow").unwrap()]));
    }

    /// Each `(dependent, dependency)` edge closed over transitively, the way a
    /// resolved package graph states it.
    fn closed_dependencies(edges: &[(&str, &str)]) -> BTreeMap<ProfileId, BTreeSet<ProfileId>> {
        let id = |package: &str| ProfileId::try_from(package).unwrap();
        let direct = edges.iter().fold(
            BTreeMap::<ProfileId, BTreeSet<ProfileId>>::new(),
            |mut map, (from, to)| {
                map.entry(id(from)).or_default().insert(id(to));
                map
            },
        );
        fn reach(
            package: &ProfileId,
            direct: &BTreeMap<ProfileId, BTreeSet<ProfileId>>,
        ) -> BTreeSet<ProfileId> {
            direct
                .get(package)
                .into_iter()
                .flatten()
                .flat_map(|next| {
                    let mut reached = reach(next, direct);
                    reached.insert(next.clone());
                    reached
                })
                .collect()
        }
        direct
            .keys()
            .map(|package| (package.clone(), reach(package, &direct)))
            .collect()
    }

    /// A region claim at `target` holding `content`.
    fn region_claim(
        layout: &RepositoryLayout,
        target: &VirtualPath,
        content: &[u8],
    ) -> ProfileRegionClaim {
        ProfileRegionClaim {
            claim: TargetClaim::new(layout, target.clone(), "profile-region:guidance").unwrap(),
            region_id: serde_json::from_str("\"guidance\"").unwrap(),
            content: content.to_vec(),
        }
    }

    /// One package's claims defining the namespace `shared` as `description`.
    fn claims_defining_shared(package: &str, description: &str) -> ProfileClaims {
        let layout = synthetic_layout();
        ProfileClaims {
            contributions: vec![contribution_claim(
                package,
                namespace_contribution("shared", description),
            )],
            ..asset_claims(&layout, package, &[], b"")
        }
    }

    /// What composition decides about `shared` over `selection`, after each
    /// dependent's overrides.
    fn compose_shared(
        selection: Vec<ProfileClaims>,
        edges: &[(&str, &str)],
    ) -> Vec<ContributionOutcome> {
        compose_resolved_contributions(
            drop_overridden_claims(selection, &closed_dependencies(edges))
                .into_iter()
                .flat_map(|claims| claims.contributions),
        )
    }

    #[test]
    fn test_drop_overridden_claims_composes_a_direct_or_transitive_dependents_value() {
        for (selection, edges) in [
            (
                vec![
                    claims_defining_shared("base", "Base."),
                    claims_defining_shared("workflow", "Workflow."),
                ],
                vec![("workflow", "base")],
            ),
            (
                vec![
                    claims_defining_shared("base", "Base."),
                    claims_defining_shared("workflow", "Workflow."),
                ],
                vec![("workflow", "middle"), ("middle", "base")],
            ),
        ] {
            let composed = agreed_definitions(compose_shared(selection, &edges));

            assert_eq!(composed.len(), 1);
            assert_eq!(
                composed[0].definition,
                namespace_contribution("shared", "Workflow.")
            );
        }
    }

    #[test]
    fn test_drop_overridden_claims_keeps_differing_packages_with_no_dependency_path() {
        let (_, owners) = contested_outcome(compose_shared(
            vec![
                claims_defining_shared("left", "Left."),
                claims_defining_shared("right", "Right."),
            ],
            &[],
        ));

        assert_eq!(owners, package_owners(&["left", "right"]));
    }

    /// A dependent at any depth settles what its dependencies disagree on.
    #[test]
    fn test_drop_overridden_claims_composes_a_common_dependents_value_over_a_disagreement() {
        for edges in [
            vec![("top", "left"), ("top", "right")],
            vec![("top", "middle"), ("middle", "left"), ("middle", "right")],
        ] {
            let composed = agreed_definitions(compose_shared(
                vec![
                    claims_defining_shared("left", "Left."),
                    claims_defining_shared("right", "Right."),
                    claims_defining_shared("top", "Top."),
                ],
                &edges,
            ));

            assert_eq!(composed.len(), 1);
            assert_eq!(
                composed[0].definition,
                namespace_contribution("shared", "Top.")
            );
        }
    }

    /// A common dependent stating no value settles nothing.
    #[test]
    fn test_drop_overridden_claims_refuses_a_disagreement_a_silent_dependent_leaves() {
        let (_, owners) = contested_outcome(compose_shared(
            vec![
                claims_defining_shared("left", "Left."),
                claims_defining_shared("right", "Right."),
            ],
            &[("top", "left"), ("top", "right")],
        ));

        assert_eq!(owners, package_owners(&["left", "right"]));
    }

    /// The dependent's value replaces the dependency's asset and region, and
    /// neither remains a contested file.
    #[test]
    fn test_drop_overridden_claims_composes_the_dependents_asset_and_region() {
        let asset = VirtualPath::worktree("docs/shared.txt").unwrap();
        let region_target = VirtualPath::worktree("docs/guide.md").unwrap();
        let layout = synthetic_layout();
        let claims = |package: &str, bytes: &[u8]| ProfileClaims {
            regions: vec![region_claim(&layout, &region_target, bytes)],
            ..asset_claims(&layout, package, std::slice::from_ref(&asset), bytes)
        };

        let composed = drop_overridden_claims(
            vec![claims("base", b"base"), claims("workflow", b"workflow")],
            &closed_dependencies(&[("workflow", "base")]),
        );

        assert!(contested_file_targets(&composed).is_empty());
        let base = &composed[0];
        assert!(base.assets.is_empty() && base.regions.is_empty());
        let workflow = &composed[1];
        assert_eq!(workflow.assets[0].bytes, b"workflow");
        assert_eq!(workflow.regions[0].content, b"workflow");
    }

    /// One managed region is one key: unrelated packages stating it differently
    /// contest its file, and an identical restatement composes.
    #[test]
    fn test_contested_file_targets_names_every_package_declaring_a_region_differently() {
        let target = VirtualPath::worktree("docs/guide.md").unwrap();
        let layout = synthetic_layout();
        let claims = |package: &str, content: &[u8]| ProfileClaims {
            regions: vec![region_claim(&layout, &target, content)],
            ..asset_claims(&layout, package, &[], b"")
        };

        assert_eq!(
            contested_file_targets(&[claims("base", b"one"), claims("workflow", b"two")]),
            BTreeMap::from([(target.clone(), package_owners(&["base", "workflow"]))])
        );
        assert!(
            contested_file_targets(&[claims("base", b"one"), claims("workflow", b"one")])
                .is_empty()
        );
    }

    type GeneratedSelection = (Vec<(String, u8)>, Vec<(String, String)>);

    /// A selection of up to five packages over one key: each package's optional
    /// value, and which higher-numbered packages each depends on.
    fn generated_selection() -> impl Strategy<Value = GeneratedSelection> {
        (
            proptest::collection::vec(proptest::option::of(0u8..3), 5),
            proptest::collection::vec(any::<bool>(), 10),
        )
            .prop_map(|(values, edges)| {
                let name = |index: usize| format!("package-{index}");
                let declared = values
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| value.map(|value| (name(index), value)))
                    .collect();
                let pairs = (0..5).flat_map(|from| ((from + 1)..5).map(move |to| (from, to)));
                let edges = pairs
                    .zip(edges)
                    .filter(|(_, present)| *present)
                    .map(|((from, to), _)| (name(from), name(to)))
                    .collect();
                (declared, edges)
            })
    }

    /// What composition decides about the selection's one key.
    fn composed_shared_definition(
        declared: &[(String, u8)],
        edges: &[(String, String)],
    ) -> Vec<ContributionOutcome> {
        let edges = edges
            .iter()
            .map(|(from, to)| (from.as_str(), to.as_str()))
            .collect::<Vec<_>>();
        compose_shared(
            declared
                .iter()
                .map(|(package, value)| claims_defining_shared(package, &value.to_string()))
                .collect(),
            &edges,
        )
    }

    proptest! {
        #[test]
        fn prop_drop_overridden_claims_composes_independently_of_selection_order(
            (declared, edges, selected) in generated_selection().prop_flat_map(
                |(declared, edges)| {
                    let selected = Just(declared.clone()).prop_shuffle();
                    (Just(declared), Just(edges), selected)
                },
            ),
        ) {
            prop_assert_eq!(
                composed_shared_definition(&selected, &edges),
                composed_shared_definition(&declared, &edges)
            );
        }

        #[test]
        fn prop_drop_overridden_claims_contests_exactly_when_undepended_declarers_differ(
            (declared, edges) in generated_selection(),
        ) {
            let dependencies = closed_dependencies(
                &edges
                    .iter()
                    .map(|(from, to)| (from.as_str(), to.as_str()))
                    .collect::<Vec<_>>(),
            );
            let depends = |from: &String, to: &String| {
                dependencies
                    .get(&ProfileId::try_from(from.as_str()).unwrap())
                    .is_some_and(|required| {
                        required.contains(&ProfileId::try_from(to.as_str()).unwrap())
                    })
            };
            let undepended_values = declared
                .iter()
                .filter(|(package, _)| !declared.iter().any(|(other, _)| depends(other, package)))
                .map(|(_, value)| *value)
                .collect::<BTreeSet<_>>();
            let contested = composed_shared_definition(&declared, &edges)
                .iter()
                .any(|outcome| matches!(outcome, ContributionOutcome::Contested { .. }));

            prop_assert_eq!(contested, undepended_values.len() > 1);
        }
    }

    /// The repository is never a conflict party: a registry already declaring
    /// the identity differently is updated to the package's value.
    #[test]
    fn test_compose_profile_targets_never_contests_a_differing_registry_declaration() {
        let image = ImageBuilder::new()
            .file(
                VirtualPath::CONFIG,
                b"[namespaces.shared]\ndescription = \"Repository definition.\"\nunique = false\n",
            )
            .entry(VirtualPath::GATES, RepositoryEntry::Absent)
            .entry(VirtualPath::INVARIANTS, RepositoryEntry::Absent)
            .entry(VirtualPath::RULES, RepositoryEntry::Absent)
            .entry(VirtualPath::TEMPLATES, RepositoryEntry::Absent)
            .build();
        let contribution = namespace_contribution("shared", "Package definition.");
        let claims = ProfileClaims {
            contributions: vec![contribution_claim("workflow", contribution.clone())],
            ..asset_claims(image.layout(), "workflow", &[], b"")
        };

        let context = claims.contributions.clone();
        let composition =
            compose_profile_targets_with_context(&image, claims, context, &BTreeMap::new())
                .unwrap();

        assert_eq!(
            composition
                .contribution_decisions
                .get(&contribution.semantic_identity()),
            Some(&ProfileTargetDisposition::Update)
        );
        assert_eq!(
            declared_in(
                &contribution.semantic_identity(),
                &composition.targets[&VirtualPath::CONFIG].0,
            ),
            Some(contribution)
        );
    }

    /// The repository is never a conflict party: a package asset over a file
    /// holding other bytes replaces it.
    #[test]
    fn test_compose_profile_targets_updates_an_asset_over_differing_repository_content() {
        let target = VirtualPath::worktree("docs/custom.txt").unwrap();
        let image = ImageBuilder::new()
            .file(VirtualPath::CONFIG, b"")
            .entry(VirtualPath::GATES, RepositoryEntry::Absent)
            .entry(VirtualPath::INVARIANTS, RepositoryEntry::Absent)
            .entry(VirtualPath::RULES, RepositoryEntry::Absent)
            .entry(VirtualPath::TEMPLATES, RepositoryEntry::Absent)
            .file(target.clone(), b"repository bytes")
            .build();
        let claims = asset_claims(
            image.layout(),
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
    fn test_contested_file_targets_names_every_package_declaring_a_target_differently() {
        let contested = VirtualPath::worktree("docs/contested.txt").unwrap();
        let restated = VirtualPath::worktree("docs/restated.txt").unwrap();
        let layout = synthetic_layout();
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
                contested_file_targets(selection),
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
        let image = ImageBuilder::new()
            .file(VirtualPath::CONFIG, b"")
            .entry(VirtualPath::GATES, RepositoryEntry::Absent)
            .entry(VirtualPath::INVARIANTS, RepositoryEntry::Absent)
            .entry(VirtualPath::RULES, RepositoryEntry::Absent)
            .entry(VirtualPath::TEMPLATES, RepositoryEntry::Absent)
            .build();
        let workflow = asset_claims(image.layout(), "workflow", &targets, b"workflow bytes");
        let base = asset_claims(image.layout(), "base", &targets, b"base bytes");
        let contested = contested_file_targets([&workflow, &base]);

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
            ProfileTargetSubject::File {
                path: VirtualPath::data("custom.txt").unwrap(),
            },
            ProfileTargetSubject::Contribution { identity },
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
            for leaked in ["ProfileId", "{"] {
                assert!(
                    !message.contains(leaked),
                    "a refusal names facts, not Rust syntax: {message}"
                );
            }
            let subject = match &entry.subject {
                ProfileTargetSubject::File { path } => path.repository_relative(),
                ProfileTargetSubject::Contribution { identity } => identity.to_string(),
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
            subject: ProfileTargetSubject::File {
                path: VirtualPath::worktree("docs/guide.md").expect("a canonical worktree path"),
            },
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
    fn test_profile_target_conflict_output_and_schema_spell_parties_as_package_ids() {
        let conflict = ProfileTargetConflict::Contested {
            owners: package_owners(&["base", "workflow"]),
        };
        let spelled =
            |owners: JsonValue| serde_json::json!({ "kind": "contested", "owners": owners });
        let schema = serde_json::to_value(schemars::schema_for!(ProfileTargetConflict)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();

        assert_eq!(
            serde_json::to_value(&conflict).unwrap(),
            spelled(serde_json::json!(["base", "workflow"]))
        );
        assert!(validator.is_valid(&spelled(serde_json::json!(["base"]))));
        assert!(!validator.is_valid(&spelled(serde_json::json!([{ "Package": "base" }]))));
    }

    #[test]
    fn test_profile_target_subject_serializes_a_contribution_by_its_canonical_identity() {
        let subject = ProfileTargetSubject::Contribution {
            identity: ContributionIdentity {
                registry: ContributionRegistry::Config,
                target: ContributionIdentityTarget::Projection {
                    name: "docs".to_string(),
                },
            },
        };

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
            subject: ProfileTargetSubject::File {
                path: VirtualPath::worktree("docs/guide.md").expect("a canonical worktree path"),
            },
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
            render_contribution(&mut document, &Contribution::Scalar { target, value }).unwrap();
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
            render_contribution(
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

    /// One package value per contribution kind beside the registry text it
    /// lands on: a differing prior entry under its identity carrying a field the
    /// package does not declare, and the text of an entry it does not contribute.
    fn differing_registry_entries() -> Vec<(&'static str, &'static str, Contribution)> {
        let map_entry = |target, value| Contribution::MapEntry {
            target,
            identity: "shared".to_string(),
            value,
        };
        let keyed = |target, field: &str| Contribution::KeyedArray {
            target,
            identity: "shared".to_string(),
            value: serde_json::json!({ field: "shared", "description": "Package." }),
        };
        vec![
            (
                "[validation]\nstrictness = \"loose\" # sibling\ndefault_type = \"task\"\n",
                "strictness = \"loose\" # sibling\n",
                Contribution::Scalar {
                    target: ScalarTarget::ValidationDefaultType,
                    value: "story".to_string(),
                },
            ),
            (
                "[type_hierarchy]\ntypes = { sibling = 1, shared = 2 }\n",
                "sibling = 1",
                map_entry(MapEntryTarget::TypeHierarchyTypes, serde_json::json!(3)),
            ),
            (
                "[type_hierarchy.label_associations]\nsibling   = \"s\"\nshared = \"held\"\n",
                "sibling   = \"s\"\n",
                map_entry(MapEntryTarget::LabelAssociations, serde_json::json!("new")),
            ),
            (
                "[namespaces.sibling] # sibling\ndescription   = \"S\"\n\n\
                 [namespaces.shared]\ndescription = \"Held.\"\nunique = false\n",
                "[namespaces.sibling] # sibling\ndescription   = \"S\"\n",
                map_entry(
                    MapEntryTarget::Namespaces,
                    serde_json::json!({ "description": "Package." }),
                ),
            ),
            (
                "[item_kinds.shared]\nsource = \"held\"\nextra = true\n\n\
                 [item_kinds.sibling] # sibling\nsource   = \"s\"\n",
                "[item_kinds.sibling] # sibling\nsource   = \"s\"\n",
                map_entry(
                    MapEntryTarget::ItemKinds,
                    serde_json::json!({ "source": "package" }),
                ),
            ),
            (
                "[[gates]]\nkey = \"sibling\" # sibling\n\n\
                 [[gates]]\nkey = \"shared\"\ndescription = \"Held.\"\nextra = true\n",
                "[[gates]]\nkey = \"sibling\" # sibling\n",
                keyed(KeyedArrayTarget::Gates, "key"),
            ),
            (
                "[[invariants]]\nid = \"shared\"\nextra = true\n\n\
                 [[invariants]]\nid = \"sibling\" # sibling\n",
                "[[invariants]]\nid = \"sibling\" # sibling\n",
                keyed(KeyedArrayTarget::Invariants, "id"),
            ),
            (
                "[[rules]]\nname = \"shared\"\nextra = true\n\n\
                 [[rules]]\nname = \"sibling\" # sibling\n",
                "[[rules]]\nname = \"sibling\" # sibling\n",
                keyed(KeyedArrayTarget::Rules, "name"),
            ),
            (
                "[[template]]\nname = \"shared\"\nextra = true\n\n\
                 [[template]]\nname = \"sibling\" # sibling\n",
                "[[template]]\nname = \"sibling\" # sibling\n",
                keyed(KeyedArrayTarget::Templates, "name"),
            ),
            (
                "[projection.sibling] # sibling\ntarget   = \"S.md\"\n\n\
                 [projection.shared]\nkind = \"invariant\"\nmode = \"region\"\n\
                 target = \"HELD.md\"\nstyle = \"id-anchor\"\n",
                "[projection.sibling] # sibling\ntarget   = \"S.md\"\n",
                Contribution::Projection {
                    name: "shared".to_string(),
                    value: serde_json::from_value(serde_json::json!({
                        "kind": "rule",
                        "mode": "separate-file",
                        "target": "SHARED.md",
                        "style": "full",
                    }))
                    .unwrap(),
                },
            ),
        ]
    }

    #[test]
    fn test_render_composed_contribution_replaces_the_whole_entry_for_each_kind() {
        for (authored, _, contribution) in differing_registry_entries() {
            let mut document = authored.parse::<DocumentMut>().unwrap();
            render_contribution(&mut document, &contribution).unwrap();
            assert_eq!(
                declared_in(
                    &contribution.semantic_identity(),
                    document.to_string().as_bytes(),
                ),
                Some(contribution),
                "rendered over {authored}"
            );
        }
    }

    #[test]
    fn test_render_composed_contribution_keeps_the_bytes_of_entries_it_does_not_contribute() {
        for (authored, sibling, contribution) in differing_registry_entries() {
            let mut document = authored.parse::<DocumentMut>().unwrap();
            render_contribution(&mut document, &contribution).unwrap();
            assert!(
                document.to_string().contains(sibling),
                "{sibling:?} lost from {document}"
            );
        }
    }

    #[test]
    fn test_render_composed_contribution_appends_an_absent_set_member_beside_held_members() {
        let mut document = "[type_hierarchy]\nstrategic_types = [ \"epic\" ] # held\n"
            .parse::<DocumentMut>()
            .unwrap();
        render_contribution(
            &mut document,
            &Contribution::SetString {
                target: SetStringTarget::StrategicTypes,
                value: "milestone".to_string(),
            },
        )
        .unwrap();
        let json: JsonValue = toml_edit::de::from_str(&document.to_string()).unwrap();
        assert_eq!(
            json["type_hierarchy"]["strategic_types"],
            serde_json::json!(["epic", "milestone"])
        );
    }

    /// Reapplication must not reformat authored text that already says the same.
    #[test]
    fn test_render_composed_contribution_keeps_the_bytes_of_an_exact_entry() {
        let authored =
            "[namespaces.shared] # authored\ndescription   =   \"Same.\"\nunique = false\n";
        let mut document = authored.parse::<DocumentMut>().unwrap();
        render_contribution(&mut document, &namespace_contribution("shared", "Same.")).unwrap();
        assert_eq!(document.to_string(), authored);
    }

    #[test]
    fn test_contribution_disposition_compares_the_package_value_with_the_held_entry() {
        let image = ImageBuilder::new()
            .file(
                VirtualPath::CONFIG,
                b"[namespaces.shared]\ndescription = \"Held.\"\nunique = false\n",
            )
            .build();
        let disposition = |identity: &str, description: &str| {
            contribution_disposition(
                &mut RegistryDocuments::new(&image),
                &composed(namespace_contribution(identity, description)),
            )
            .unwrap()
        };
        assert_eq!(
            disposition("absent", "New."),
            ProfileTargetDisposition::Create
        );
        assert_eq!(
            disposition("shared", "Held."),
            ProfileTargetDisposition::Unchanged
        );
        assert_eq!(
            disposition("shared", "New."),
            ProfileTargetDisposition::Update
        );
    }

    #[test]
    fn test_empty_contributions_leave_authored_configuration_without_an_overlay() {
        let authored = b"[documentation]\ndevelopment_root = \"notes\"\n";
        let image = ImageBuilder::new()
            .file(VirtualPath::CONFIG, authored)
            .build();

        assert!(
            render_composed_contributions(&mut RegistryDocuments::new(&image), &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_render_composed_contributions_writes_each_registry_at_its_data_root_constant() {
        let image = ImageBuilder::new()
            .file(VirtualPath::CONFIG, b"")
            .entry(VirtualPath::GATES, RepositoryEntry::Absent)
            .entry(VirtualPath::INVARIANTS, RepositoryEntry::Absent)
            .entry(VirtualPath::RULES, RepositoryEntry::Absent)
            .entry(VirtualPath::TEMPLATES, RepositoryEntry::Absent)
            .build();
        let keyed = |target, field: &str| Contribution::KeyedArray {
            target,
            identity: "shared".to_string(),
            value: serde_json::json!({ field: "shared" }),
        };
        let contributions = [
            namespace_contribution("shared", "Shared."),
            keyed(KeyedArrayTarget::Gates, "key"),
            keyed(KeyedArrayTarget::Invariants, "id"),
            keyed(KeyedArrayTarget::Rules, "name"),
            keyed(KeyedArrayTarget::Templates, "name"),
        ]
        .map(composed);

        let rendered =
            render_composed_contributions(&mut RegistryDocuments::new(&image), &contributions)
                .unwrap();

        assert_eq!(
            rendered.into_keys().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                VirtualPath::CONFIG,
                VirtualPath::GATES,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::TEMPLATES,
            ])
        );
    }

    /// Replacing through the presence lookup keeps a held entry where it stands.
    #[test]
    fn test_render_composed_contribution_keeps_a_held_entry_at_its_position_for_each_kind() {
        let shared_before_sibling = |text: &str| {
            text.find("shared")
                .zip(text.find("sibling"))
                .map(|(shared, sibling)| shared < sibling)
        };
        for (authored, _, contribution) in differing_registry_entries() {
            let mut document = authored.parse::<DocumentMut>().unwrap();
            render_contribution(&mut document, &contribution).unwrap();
            assert_eq!(
                shared_before_sibling(&document.to_string()),
                shared_before_sibling(authored),
                "rendered over {authored}"
            );
        }
    }

    /// Replacing and appending in one parse leaves each entry at its own identity.
    #[test]
    fn test_render_composed_contributions_lands_every_contribution_over_one_parse() {
        let image = ImageBuilder::new()
            .file(
                VirtualPath::CONFIG,
                b"[type_hierarchy]\nstrategic_types = [\"epic\"]\n",
            )
            .file(
                VirtualPath::GATES,
                b"[[gates]]\nkey = \"sibling\"\n\n[[gates]]\nkey = \"held\"\n",
            )
            .build();
        let definitions = [
            gate_contribution("appended", "Appended."),
            gate_contribution("held", "Replaced."),
            Contribution::SetString {
                target: SetStringTarget::StrategicTypes,
                value: "milestone".to_string(),
            },
            Contribution::SetString {
                target: SetStringTarget::StrategicTypes,
                value: "initiative".to_string(),
            },
        ];
        let rendered = render_composed_contributions(
            &mut RegistryDocuments::new(&image),
            &definitions.clone().map(composed),
        )
        .unwrap();

        assert!(definitions.iter().all(|definition| {
            declared_in(
                &definition.semantic_identity(),
                &rendered[&definition.registry().path()].0,
            )
            .as_ref()
                == Some(definition)
        }));
    }

    #[test]
    fn test_compose_profile_contributions_parses_each_registry_once_however_many_it_renders() {
        let image = ImageBuilder::new()
            .file(
                VirtualPath::CONFIG,
                b"[namespaces.held]\ndescription = \"Held.\"\n",
            )
            .entry(VirtualPath::GATES, RepositoryEntry::Absent)
            .build();
        let claims = ["held", "second", "third"]
            .map(|name| namespace_contribution(name, "Package."))
            .into_iter()
            .chain(["one", "two", "three"].map(|key| gate_contribution(key, "Package.")))
            .map(|contribution| contribution_claim("package", contribution))
            .collect::<Vec<_>>();

        let mut composition = compose_profile_contributions(&image, &claims).unwrap();
        let rendered =
            render_composed_contributions(&mut composition.registries, &composition.definitions)
                .unwrap();

        assert_eq!(rendered.len(), 2);
        assert_eq!(composition.registries.parses, rendered.len());
    }
}
