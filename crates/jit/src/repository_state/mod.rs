//! Pure closed repository-state capture and materialization vocabulary.
//!
//! This subsystem owns canonical paths, immutable images, bounded capture,
//! managed-document composition, exact deltas, and deterministic plan identity.
//! It performs no filesystem I/O and imports no validation, storage, command, or
//! profile modules.

mod archive;
mod default_rules;
mod export;
mod image;
mod index;
mod initialize;
mod managed_document;
mod materialize;
mod mutation;
mod overlay;
mod package_tree;
mod path;
mod profile_apply;
mod projection;
mod projection_render;
mod rule_serialize;
mod rules_document;
mod rules_gates_projection;

pub(crate) use archive::captured_archive_events;
pub use archive::{finalize_archive_execution, ArchiveExecutionError};
pub(crate) use default_rules::default_rule_membership_diff_from_identities;
pub use default_rules::{
    default_ruleset, hierarchy_config, reconcile_default_rules_with_config,
    type_hierarchy_known_schema, TYPE_HIERARCHY_SCHEMA_FILE,
};
// Rule-membership-diff seam: `default_rule_membership_diff` and
// `DefaultRuleMembershipDiff` are `pub` only for this function's own doctest (no
// production or integration-test consumer outside `repository_state`), so their
// exposure as crate public API is gated behind `test-support`. The `pub(crate)`
// twin keeps every internal caller's resolution
// (`crate::repository_state::DefaultRuleMembershipDiff`, etc.) identical in
// both feature states.
#[cfg(feature = "test-support")]
pub use default_rules::{default_rule_membership_diff, DefaultRuleMembershipDiff};
// Measured (cargo build -p jit, feature off): neither name is referenced by
// that path anywhere in the crate — `default_rule_membership_diff` has no
// production caller at all (only this module's own unit tests and its
// doctest call it), and `DefaultRuleMembershipDiff` is otherwise reached only
// via `default_rule_membership_diff_from_identities`'s separate, unconditional
// `pub(crate)` path. Both are genuinely unused here, so the allow is scoped to
// exactly this pair, not a blanket over unrelated items.
#[cfg(not(feature = "test-support"))]
#[allow(unused_imports)]
pub(crate) use default_rules::{default_rule_membership_diff, DefaultRuleMembershipDiff};
pub(crate) use export::{
    classify_repository_export, finalize_repository_export, ExternalExportPath,
    RepositoryExportDestination, RepositoryExportError, RepositoryExportIntent,
};
pub use image::{
    plan_hash, CaptureBudget, CaptureError, CaptureSpec, DeltaError, EntryIdentity,
    ExpectedPreimage, FileMode, LinkedWorktreeEvidence, LinkedWorktreeSourceClass,
    ListingFingerprint, MaterializationIntent, PinnedDocumentEvidence, PinnedSourceClass,
    PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositorySeed, RepositorySeedKind, RepositoryTargetKind, SeedError, TargetClaim,
};
pub use index::RepositoryIndexError;
pub(crate) use index::{RepositoryIndex, SUPPORTED_INDEX_SCHEMA_VERSION};
pub use initialize::{
    render_repo_config, structural_minimum_config, GitattributesClaim, GitattributesStatus,
    InitializationError, InitializationScaffold, FRESH_CONFIG_SCHEMA_VERSION,
};
pub(crate) use managed_document::compose_managed_documents;
pub use managed_document::{
    render_managed_document, ManagedDocumentClaim, ManagedDocumentError, RegionPlacement,
};
pub use materialize::{assemble_config, render_capture_closure};
pub use package_tree::{
    finalize_package_tree_capture, CapturedTreeFile, PackageTreeCapture, PackageTreeCaptureError,
    TreeFileDisposition, TreeFileOutcome,
};
// Capture-closure seam: `validate_capture_closure` and `ValidationCaptureClosure`
// are `pub` only for the `fast_rules` integration-test crate (no production
// consumer outside `repository_state`/`commands::validate`), so their exposure
// as crate public API is gated behind `test-support`. The `pub(crate)` twin
// keeps every internal caller's resolution
// (`crate::repository_state::validate_capture_closure`, etc.) identical in both
// feature states.
#[cfg(not(feature = "test-support"))]
pub(crate) use materialize::validate_capture_closure;
#[cfg(feature = "test-support")]
pub use materialize::{validate_capture_closure, ValidationCaptureClosure};
// Measured (cargo build -p jit, feature off): `validate_capture_closure` has a
// real internal caller (`commands/validate.rs` resolves it through this same
// `crate::repository_state::` path in both feature states) so it needs no
// allow. `ValidationCaptureClosure` itself has no by-name crate-internal
// caller — it's only reached as `validate_capture_closure`'s inferred return
// type — so its feature-off twin alone is allowed unused.
#[cfg(not(feature = "test-support"))]
#[allow(unused_imports)]
pub(crate) use materialize::ValidationCaptureClosure;
pub(crate) use mutation::captured_gate_run_result_paths;
pub use mutation::{
    finalize, fresh_index_bytes, gate_run_result_relative_path, prefix_has_torn_tail,
    serialize_event, serialize_gate_run, serialize_issue, IdAuthority, MutationClock,
    MutationContext, MutationError, MutationIntent, SystemMutationClock,
};

/// Finalize one typed gate-registry edit, its audit event, and every coupled
/// declaration-derived materialization into one recoverable plan.
///
/// `declarations.gates` must be the proposed registry represented by the edit
/// intent. The base image remains the expected preimage for every action; an
/// overlay is used only while deriving projections so their contents reflect the
/// proposed registry before anything is published.
pub fn finalize_gate_registry_edit(
    layout: &RepositoryLayout,
    base: &RepositoryImage,
    context: &MutationContext,
    intents: &[MutationIntent],
    declarations: RepositoryDeclarations<'_>,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let registry = intents
        .iter()
        .find_map(|intent| match intent {
            MutationIntent::EditGateRegistry { registry } => Some(&**registry),
            _ => None,
        })
        .ok_or(GateRegistryEditError::MissingEdit)?;
    if intents
        .iter()
        .filter(|intent| matches!(intent, MutationIntent::EditGateRegistry { .. }))
        .count()
        != 1
    {
        return Err(GateRegistryEditError::MultipleEdits.into());
    }
    if declarations.gates != registry {
        return Err(GateRegistryEditError::DeclarationMismatch.into());
    }

    let record_plan =
        finalize(layout, base, context, intents).map_err(GateRegistryEditError::Mutation)?;
    let gate_path = VirtualPath::GATES;
    let gate_bytes = crate::declarations::serialize_gate_registry(registry)
        .map_err(GateRegistryEditError::GateDeclaration)?;
    let overlaid = apply_overlay(base, std::iter::once((gate_path, Some(gate_bytes))))
        .map_err(GateRegistryEditError::Overlay)?;
    let mut actions = record_plan.delta().actions().to_vec();
    actions.extend(compose_complete(&overlaid, &declarations)?);
    let delta = RepositoryDelta::new(layout, actions)?;
    let seed = context
        .repository_seed(intents)
        .map_err(GateRegistryEditError::Mutation)?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::SemanticMutation,
        delta,
    )?)
}

/// A typed failure raised while finalizing a gate-registry edit.
///
/// Composed into [`RepositoryStateError::GateRegistryEdit`]. The three
/// invariant-shaped variants carry no message text of their own beyond their
/// `Display` impl; every propagated leaf retains its concrete source so
/// rendering stays in `Display` rather than at the call site.
#[derive(Debug, thiserror::Error)]
pub enum GateRegistryEditError {
    /// No typed `EditGateRegistry` intent was present among the finalized intents.
    #[error("gate registry finalization requires one typed edit")]
    MissingEdit,
    /// More than one typed `EditGateRegistry` intent was present.
    #[error("gate registry finalization requires exactly one typed edit")]
    MultipleEdits,
    /// The typed edit's registry disagreed with the projection declarations.
    #[error("gate registry edit and projection declarations disagree")]
    DeclarationMismatch,
    /// The record finalizer or seed derivation failed.
    #[error(transparent)]
    Mutation(#[from] MutationError),
    /// The edited registry could not be serialized canonically.
    #[error(transparent)]
    GateDeclaration(#[from] crate::declarations::GateDeclarationError),
    /// The proposed-state overlay could not be closed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
    /// The closed semantic seed was invalid.
    #[error(transparent)]
    Seed(#[from] SeedError),
}
pub use overlay::{apply_overlay, OverlayError};
pub use path::{
    RepositoryLayout, RepositoryLayoutError, RepositoryRootClass, RepositoryRootEvidence,
    RootRelativePath, VirtualPath,
};
pub use profile_apply::{
    compose_resolved_contributions, AppliedClaimTarget, AppliedManagedRegionTarget,
    AppliedProfileClaim, AppliedProfileClaimIdentity, AppliedProfileRecord,
    CompleteProjectionConfig, ComposedContribution, Contribution, ContributionCompositionConflict,
    ContributionConflictOwner, ContributionIdentity, ContributionIdentityTarget,
    ContributionRegistry, ExistingContributionClaim, KeyedArrayTarget, MapEntryTarget,
    ProfileApplicationInput, ProfileAssetClaim, ProfileBaseFingerprint, ProfileClaims,
    ProfileConflictOccupant, ProfileContributionClaim, ProfilePackageId, ProfileRegionClaim,
    ProfileTargetConflictError, ProfileThreeWayConflictError, ScalarTarget, SetStringTarget,
};
pub(crate) use profile_apply::{
    preflight_profile_contributions, profile_capture_closure, profile_contribution_overrides,
    profile_contribution_target_paths,
};
pub use projection::{
    render_id_anchor_rows, render_invariants_markdown, require_target, ProjectionError,
};
pub(crate) use projection_render::{render_projection_body, ProjectionInputs};
pub use rule_serialize::{render_rule_block, rules_file_header};
// Rule-serialization seam: `serialize_ruleset`, `SchemaFile`, and
// `SerializedRuleSet` are `pub` only for integration-test crates (no
// production consumer outside `repository_state`), so their exposure as
// crate public API is gated behind `test-support`. The `pub(crate)` twin
// keeps every internal caller's resolution
// (`crate::repository_state::SerializedRuleSet`, etc.) identical in both
// feature states.
#[cfg(feature = "test-support")]
pub use rule_serialize::{serialize_ruleset, SchemaFile, SerializedRuleSet};
#[cfg(not(feature = "test-support"))]
pub(crate) use rule_serialize::{serialize_ruleset, SerializedRuleSet};
// `SchemaFile` itself has no by-name crate-internal caller (it's only
// reached as the element type of `SerializedRuleSet::schema_files`), so its
// feature-off twin alone is allowed unused.
#[cfg(not(feature = "test-support"))]
#[allow(unused_imports)]
pub(crate) use rule_serialize::SchemaFile;
pub use rules_document::{parse_rule_identities, splice_default_membership, RulesDocumentError};
pub use rules_gates_projection::render_rules_and_gates_markdown;

use crate::declarations::rules::RuleSet;
use crate::declarations::{ConfigurationDeclarationError, ConfigurationDeclarations, GateRegistry};

/// A typed failure raised while deriving materialized repository state.
///
/// Producer errors retain their concrete source and raw declaration identities;
/// rendering belongs to this type rather than to producer call sites.
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    /// A required declaration was absent from the captured repository image.
    #[error("captured image has no {0}")]
    MissingCapture(&'static str),
    /// Captured bytes were not valid UTF-8.
    #[error(transparent)]
    MalformedUtf8(#[from] std::string::FromUtf8Error),
    /// A requested projection is not declared.
    #[error("unknown projection '{0}'")]
    UnknownProjection(String),
    /// A projection references an item kind that is not declared.
    #[error("projection '{projection}' references unknown kind '{kind}'")]
    UnknownKind { projection: String, kind: String },
    /// An authored config edit could not be parsed.
    #[error("invalid config.toml edit: {0}")]
    ConfigParse(#[source] ConfigurationDeclarationError),
    /// Closed-image capture failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A producer path was invalid for the repository layout.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// Captured configuration declarations were invalid.
    #[error(transparent)]
    ConfigurationDeclaration(#[from] ConfigurationDeclarationError),
    /// Captured invariant declarations were invalid.
    #[error(transparent)]
    InvariantConfig(#[from] crate::declarations::invariants::InvariantConfigError),
    /// Addressable item declarations or source content were invalid.
    #[error(transparent)]
    Item(#[from] crate::domain::item::ItemError),
    /// Captured rule declarations were invalid.
    #[error(transparent)]
    RuleConfig(#[from] crate::declarations::rules::RuleConfigError),
    /// A configured projection could not be rendered.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// Managed-document claims could not be composed.
    #[error(transparent)]
    ManagedDocument(#[from] ManagedDocumentError),
    /// A selected content parser is not available in this build.
    #[error(transparent)]
    ContentParser(#[from] crate::document::ContentParserError),
    /// The repository-level content format is invalid.
    #[error(
        "invalid [validation].content_format in .jit/config.toml: '{0}': Invalid content format: '{0}' (expected markdown, html, or xml)"
    )]
    InvalidContentFormat(String),
    /// A rules document could not be parsed or safely spliced.
    #[error(transparent)]
    RulesDocument(#[from] RulesDocumentError),
    /// Captured declarations could not be assembled into the canonical bundle.
    #[error(transparent)]
    DeclarationAssembly(Box<DeclarationParseError>),
    /// A profile registry target is occupied by a non-file entry.
    #[error("profile registry '{target}' is not a regular file")]
    ProfileRegistryNotFile { target: String },
    /// A profile registry could not be decoded or parsed.
    #[error("invalid profile registry '{target}': {source}")]
    ProfileRegistryParse {
        target: String,
        #[source]
        source: Box<ProfileRegistryParseError>,
    },
    /// An applied-profile provenance record could not be decoded while resolving
    /// the owner of a conflicting target.
    #[error("invalid applied profile record '{path}': {source}")]
    ProfileRecordParse {
        /// Repository-relative record path.
        path: String,
        /// JSON decoding failure.
        source: serde_json::Error,
    },
    /// A provenance record was stored under a name other than its package id.
    #[error("applied profile record '{path}' names package '{id}', expected '{expected}'")]
    ProfileRecordPathMismatch {
        /// Repository-relative record path.
        path: String,
        /// Record package identity.
        id: String,
        /// Canonical record path for that package identity.
        expected: String,
    },
    /// A canonical profile-claim fingerprint could not serialize its resolved
    /// semantic value.
    #[error("could not fingerprint profile claim: {0}")]
    ProfileClaimFingerprint(#[source] serde_json::Error),
    /// A supported archive edge target is absent from the proposed plan.
    #[error("supported archive edge target is absent from plan: {target}")]
    ProposedLayoutTargetAbsent { target: String },
    /// A supported archive edge does not resolve after the proposed move.
    #[error(
        "supported edge {reference} from {parent} resolves to {resolved} in the proposed layout, not an available target location ({available})",
        available = .available.join(", ")
    )]
    ProposedLayoutEdgeUnavailable {
        reference: String,
        parent: String,
        resolved: String,
        available: Vec<String>,
    },
}

/// Typed parse failures for a profile-owned TOML registry.
#[derive(Debug, thiserror::Error)]
pub enum ProfileRegistryParseError {
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    TomlEdit(#[from] toml_edit::TomlError),
    #[error(transparent)]
    TomlDeserialize(#[from] toml_edit::de::Error),
    #[error("set target is not an array")]
    SetTargetNotArray,
    #[error("set target contains a non-string member")]
    SetTargetNonStringMember,
    #[error("scalar target is not a string")]
    ScalarTargetNotString,
    #[error("entry lacks '{field}' identity")]
    MissingIdentity { field: String },
    #[error("duplicate '{field}' identity")]
    DuplicateIdentity { field: String },
    #[error("'{key}' is not a table")]
    NotTable { key: String },
    #[error("'{key}' is not an inline table")]
    NotInlineTable { key: String },
    #[error("'{key}' is not an array")]
    NotArray { key: String },
    #[error("'{key}' is not an array of tables")]
    NotArrayOfTables { key: String },
    #[error("invalid projection definition: {0}")]
    ProjectionDefinition(String),
    #[error("contribution value is not a table")]
    ContributionNotTable,
    #[error("null is not a TOML value")]
    NullValue,
    #[error("unsupported numeric value")]
    UnsupportedNumericValue,
    #[error("value is not a table")]
    ValueNotTable,
}

/// A typed failure raised while parsing captured declaration files into the
/// canonical bundle ([`declarations_from_image`] /
/// [`validation_declarations_from_image`]).
///
/// Each variant carries its leaf parser's concrete error; rendering lives in this
/// type's `Display` impl rather than at the parse site. Also the stored type of a
/// deferred `rules.toml` load failure ([`CapturedRepositoryDeclarations::rules_load_error`]).
#[derive(Debug, thiserror::Error)]
pub enum DeclarationParseError {
    /// A required declaration file was absent from the captured image.
    #[error("captured image has no {0}")]
    MissingCapture(String),
    /// Reading a captured entry failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A declaration path was invalid for the repository layout.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// Captured bytes were not valid UTF-8.
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    /// The repository configuration could not be parsed.
    #[error(transparent)]
    Configuration(#[from] ConfigurationDeclarationError),
    /// The captured configuration could not be assembled into the effective config.
    #[error(transparent)]
    Producer(#[from] ProducerError),
    /// The template registry could not be parsed.
    #[error(transparent)]
    Template(#[from] crate::templates::TemplateConfigError),
    /// The gate registry could not be parsed.
    #[error(transparent)]
    Gate(#[from] crate::declarations::GateDeclarationError),
    /// The rule set could not be parsed.
    #[error(transparent)]
    Rule(#[from] crate::declarations::rules::RuleConfigError),
}

/// Explicit declaration bundle consumed by the closed producer call graph.
pub struct RepositoryDeclarations<'a> {
    /// Parsed repository configuration.
    pub configuration: &'a ConfigurationDeclarations,
    /// Authored gate registry.
    pub gates: &'a GateRegistry,
    /// Authored rule registry.
    pub rules: &'a RuleSet,
}

/// One owned parse of every captured declaration consumed by materialization or
/// validation.
pub(crate) struct CapturedRepositoryDeclarations {
    pub(crate) configuration: ConfigurationDeclarations,
    pub(crate) config: crate::config::JitConfig,
    pub(crate) gates: GateRegistry,
    pub(crate) rules: RuleSet,
    rules_load_error: Option<DeclarationParseError>,
}

impl CapturedRepositoryDeclarations {
    pub(crate) fn from_parts(
        configuration: ConfigurationDeclarations,
        config: crate::config::JitConfig,
        gates: GateRegistry,
        rules: RuleSet,
    ) -> Self {
        Self {
            configuration,
            config,
            gates,
            rules,
            rules_load_error: None,
        }
    }

    pub(crate) fn borrowed(&self) -> RepositoryDeclarations<'_> {
        RepositoryDeclarations {
            configuration: &self.configuration,
            gates: &self.gates,
            rules: &self.rules,
        }
    }

    pub(crate) fn config(&self) -> &crate::config::JitConfig {
        &self.config
    }

    pub(crate) fn rules(&self) -> &RuleSet {
        &self.rules
    }

    pub(crate) fn gates(&self) -> &GateRegistry {
        &self.gates
    }

    pub(crate) fn rules_loaded(&self) -> bool {
        self.rules_load_error.is_none()
    }

    pub(crate) fn rules_load_error(&self) -> Option<&DeclarationParseError> {
        self.rules_load_error.as_ref()
    }

    pub(crate) fn take_rules_load_error(&mut self) -> Option<DeclarationParseError> {
        self.rules_load_error.take()
    }
}

/// Parse one captured image into the canonical declaration bundle exactly once.
pub(crate) fn declarations_from_image(
    image: &RepositoryImage,
) -> Result<CapturedRepositoryDeclarations, DeclarationParseError> {
    let mut declarations = validation_declarations_from_image(image)?;
    match declarations.take_rules_load_error() {
        Some(error) => Err(error),
        None => Ok(declarations),
    }
}

/// Parse captured declarations for validation while preserving an unloadable
/// rules source as reportable state. Configuration, gates, templates, and
/// invariants remain strict; only rules use the empty-set sentinel because
/// validation must still report bindings into an unloadable rule source.
pub(crate) fn validation_declarations_from_image(
    image: &RepositoryImage,
) -> Result<CapturedRepositoryDeclarations, DeclarationParseError> {
    let config_bytes = image
        .file_bytes(&VirtualPath::CONFIG)?
        .ok_or_else(|| DeclarationParseError::MissingCapture(".jit/config.toml".to_string()))?;
    let configuration = crate::declarations::parse_configuration(config_bytes)?;
    let mut config = materialize::assemble_config_from_declarations(image, &configuration)?;
    let hierarchy_types = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| {
            hierarchy
                .types
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    config.templates = match image.file_bytes(&VirtualPath::TEMPLATES) {
        Ok(Some(bytes)) => crate::templates::TemplateRegistry::from_toml_str(
            std::str::from_utf8(bytes)?,
            &hierarchy_types,
        )?,
        Ok(None) | Err(CaptureError::UndiscoveredRepositoryPath(_)) => {
            crate::templates::TemplateRegistry::empty()
        }
        Err(error) => return Err(error.into()),
    };
    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let gates = match image.file_bytes(&VirtualPath::GATES)? {
        Some(bytes) => crate::declarations::parse_gate_registry(bytes)?,
        None => GateRegistry::default(),
    };
    let rules = match image.file_bytes(&VirtualPath::RULES)? {
        Some(bytes) => (|| -> Result<RuleSet, DeclarationParseError> {
            let content = std::str::from_utf8(bytes)?;
            let schemas = RuleSet::schema_requests(content)?
                .into_iter()
                .map(|request| -> Result<_, DeclarationParseError> {
                    let path = VirtualPath::data(&request.reference)?;
                    let bytes = image.file_bytes(&path)?.ok_or_else(|| {
                        DeclarationParseError::MissingCapture(request.reference.clone())
                    })?;
                    Ok((request.reference, bytes.to_vec()))
                })
                .collect::<Result<Vec<_>, DeclarationParseError>>()?;
            let parsed = RuleSet::parse(content, Some(&config), schemas)?;
            Ok(reconcile_default_rules_with_config(parsed, &namespaces))
        })(),
        None => Ok(default_ruleset(&namespaces)),
    };
    let (rules, rules_load_error) = match rules {
        Ok(rules) => (rules, None),
        Err(error) => (RuleSet { rules: Vec::new() }, Some(error)),
    };
    Ok(CapturedRepositoryDeclarations {
        configuration,
        config,
        gates,
        rules,
        rules_load_error,
    })
}

/// Complete deterministic pure materialization result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationPlan {
    /// Exact bounded repository image from which this plan was derived.
    image: RepositoryImage,
    /// Exact normalized delta.
    delta: RepositoryDelta,
    /// Semantic hash covering the complete image and inputs.
    hash: String,
    /// Per-projection row counts produced by a configured-projection render.
    projection_counts: std::collections::BTreeMap<String, usize>,
    profile_targets: Vec<ProfileTargetMaterialization>,
    applied_profiles: std::collections::BTreeSet<crate::profile::ProfileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileTargetDisposition {
    Unchanged,
    Create,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileTargetMaterialization {
    /// Package whose composed claims decided this target.
    ///
    /// One aggregate plan carries the targets of every member it composes, so a
    /// reader that reports per profile needs each decision to name the profile
    /// that made it. A target two packages both contribute is decided by each
    /// of them and therefore appears once per contributing owner.
    pub(crate) owner: crate::profile::ProfileId,
    pub(crate) path: VirtualPath,
    pub(crate) disposition: ProfileTargetDisposition,
    pub(crate) mode: FileMode,
}

/// Complete producer output awaiting the shared plan-identity tail.
struct MaterializationDerivation {
    delta: RepositoryDelta,
    seed: RepositorySeed,
    intent: MaterializationIntent,
    profile_targets: Vec<ProfileTargetMaterialization>,
    applied_profiles: std::collections::BTreeSet<crate::profile::ProfileId>,
}

impl MaterializationDerivation {
    fn new(delta: RepositoryDelta, seed: RepositorySeed, intent: MaterializationIntent) -> Self {
        Self {
            delta,
            seed,
            intent,
            profile_targets: Vec::new(),
            applied_profiles: std::collections::BTreeSet::new(),
        }
    }

    fn with_profile_targets(mut self, targets: Vec<ProfileTargetMaterialization>) -> Self {
        self.profile_targets = targets;
        self
    }

    /// Record packages whose candidate derivations have a non-event effect.
    fn with_applied_profiles(
        mut self,
        profiles: std::collections::BTreeSet<crate::profile::ProfileId>,
    ) -> Self {
        self.applied_profiles = profiles;
        self
    }
}

impl MaterializationPlan {
    /// The exact bounded repository image closed into this plan.
    pub fn image(&self) -> &RepositoryImage {
        &self.image
    }

    /// The exact normalized delta closed into this plan.
    pub fn delta(&self) -> &RepositoryDelta {
        &self.delta
    }

    /// The semantic identity computed from the captured image and complete plan inputs.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Return the count produced for one configured projection in this plan.
    pub(crate) fn projection_count(&self, name: &str) -> Option<usize> {
        self.projection_counts.get(name).copied()
    }

    pub(crate) fn profile_targets(&self) -> &[ProfileTargetMaterialization] {
        &self.profile_targets
    }

    /// Package identities responsible for a non-event effect in this plan.
    pub(crate) fn applied_profiles(
        &self,
    ) -> &std::collections::BTreeSet<crate::profile::ProfileId> {
        &self.applied_profiles
    }

    /// Close a delta into a plan whose identity is computed from all plan inputs.
    pub(crate) fn new(
        image: &RepositoryImage,
        seed: &RepositorySeed,
        intent: &MaterializationIntent,
        delta: RepositoryDelta,
    ) -> Result<Self, PlanHashError> {
        let hash = plan_hash(image, seed, intent, &delta)?;
        Ok(Self {
            image: image.clone(),
            delta,
            hash,
            projection_counts: std::collections::BTreeMap::new(),
            profile_targets: Vec::new(),
            applied_profiles: std::collections::BTreeSet::new(),
        })
    }

    fn with_projection_counts(mut self, counts: std::collections::BTreeMap<String, usize>) -> Self {
        self.projection_counts = counts;
        self
    }

    fn with_profile_targets(mut self, targets: Vec<ProfileTargetMaterialization>) -> Self {
        self.profile_targets = targets;
        self
    }

    fn with_applied_profiles(
        mut self,
        profiles: std::collections::BTreeSet<crate::profile::ProfileId>,
    ) -> Self {
        self.applied_profiles = profiles;
        self
    }
}

/// Typed payload for one invocation of the closed materialization planner.
///
/// Initialization and profile application carry semantic inputs that do not yet
/// exist as declarations in the captured image. Keeping those inputs in variants
/// of this request lets every consumer enter one planner without an untyped bag or
/// a second plan vocabulary.
pub enum MaterializationRequest<'a> {
    /// Rebuild all declaration-owned state after a semantic mutation.
    SemanticMutation {
        /// Parsed declarations captured from the image.
        declarations: RepositoryDeclarations<'a>,
        /// Closed semantic seed for this operation.
        seed: &'a RepositorySeed,
    },
    /// Render the selected configured projections (`None` selects all).
    RenderConfiguredProjections {
        /// Parsed declarations captured from the image.
        declarations: RepositoryDeclarations<'a>,
        /// Closed semantic seed for this operation.
        seed: &'a RepositorySeed,
        /// Declaration-scoped projection names.
        selected: Option<std::collections::BTreeSet<String>>,
    },
    /// Repair all declaration-owned derived state.
    RepairDerivedState {
        /// Parsed declarations captured from the image.
        declarations: RepositoryDeclarations<'a>,
        /// Neutral claims for every installed profile whose exact package
        /// identity was proven from repository provenance.
        profiles: Vec<ProfileClaims>,
        /// Closed semantic seed for this operation.
        seed: &'a RepositorySeed,
    },
    /// Derive a fresh or missing-file repository scaffold.
    Initialize {
        /// Fully rendered semantic scaffold contribution.
        scaffold: &'a InitializationScaffold,
        /// Stable mutation identity and time authority.
        context: &'a MutationContext,
    },
    /// Derive one complete profile-selection application over an existing
    /// repository. The collection is dependency-first and unique; selector
    /// occurrence handling remains at the command boundary.
    ApplyProfileSelection {
        /// Parsed package metadata and neutral canonical claims for the whole
        /// selected closure.
        profiles: Vec<ProfileApplicationInput>,
        /// Stable mutation identity and time authority.
        context: &'a MutationContext,
        /// Requested lifecycle operation recorded when this selection changes
        /// repository-owned state.
        operation: crate::domain::ProfileLifecycleOperation,
    },
}

/// Invoke the constrained closed producer graph for one typed request.
///
/// The direct request match is deliberately closed: callers cannot register
/// callbacks or choose individual producer families. Adding a family requires
/// extending this function.
pub fn derive_materialization(
    image: &RepositoryImage,
    request: MaterializationRequest<'_>,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let (derivation, projection_counts) = match request {
        MaterializationRequest::SemanticMutation { declarations, seed } => {
            let delta = derive_semantic_mutation(image, &declarations)?;
            (
                MaterializationDerivation::new(
                    delta,
                    (*seed).clone(),
                    MaterializationIntent::SemanticMutation,
                ),
                Default::default(),
            )
        }
        MaterializationRequest::RenderConfiguredProjections {
            declarations,
            seed,
            selected,
        } => {
            let (delta, counts) = derive_project_render(image, &declarations, selected.as_ref())?;
            (
                MaterializationDerivation::new(
                    delta,
                    (*seed).clone(),
                    MaterializationIntent::RenderConfiguredProjections { selected },
                ),
                counts,
            )
        }
        MaterializationRequest::RepairDerivedState {
            declarations,
            profiles,
            seed,
        } => {
            let delta = derive_repair(image, &declarations, profiles)?;
            (
                MaterializationDerivation::new(
                    delta,
                    (*seed).clone(),
                    MaterializationIntent::RepairDerivedState,
                ),
                Default::default(),
            )
        }
        MaterializationRequest::Initialize { scaffold, context } => (
            initialize::derive_initialization(image, scaffold, context)?,
            Default::default(),
        ),
        MaterializationRequest::ApplyProfileSelection {
            profiles,
            context,
            operation,
        } => (
            initialize::derive_profile_applications(
                image,
                &profiles,
                context,
                operation,
                std::collections::BTreeSet::new(),
            )?,
            Default::default(),
        ),
    };
    let MaterializationDerivation {
        delta,
        seed,
        intent,
        profile_targets,
        applied_profiles,
    } = derivation;
    MaterializationPlan::new(image, &seed, &intent, delta)
        .map(|plan| {
            plan.with_projection_counts(projection_counts)
                .with_profile_targets(profile_targets)
                .with_applied_profiles(applied_profiles)
        })
        .map_err(Into::into)
}

/// Finalize an authored `config.toml` edit plus the complete coupled producer set
/// as one plan — the single-declaration config-mutation session entry.
///
/// The command supplies the edited, preservation-safe `config.toml` bytes; this
/// parses them through the pure configuration parser first, so a malformed edit is
/// a typed planning error and never a published file. It then overlays the edited
/// bytes onto the captured `base` and derives the complete owned-materialization
/// set — default rules and their schemas, plus every configured projection — from
/// that edited configuration, exactly as [`derive_semantic_mutation`] does. The
/// authored `config.toml` write carries the captured base preimage, and every
/// derived action its own base preimage, so `session.apply` revalidates against
/// exactly what was captured; the whole set publishes under the existing
/// [`MaterializationIntent::SemanticMutation`].
///
/// This is a constrained, single-declaration entry invoking the closed complete
/// producer set — `config.toml` only, never a generic authored-bytes seam. Other
/// authored-declaration edits adopt this same pattern per caller.
pub fn finalize_config_edit(
    base: &RepositoryImage,
    edited_config_bytes: &[u8],
    declarations: RepositoryDeclarations<'_>,
    seed: &RepositorySeed,
) -> Result<MaterializationPlan, RepositoryStateError> {
    // The edit must parse before any planning: a malformed config is a typed
    // planning error, not a published file.
    let edited_configuration = crate::declarations::parse_configuration(edited_config_bytes)
        .map_err(ProducerError::ConfigParse)?;

    let config_path = VirtualPath::CONFIG;
    let overlay = std::iter::once((config_path.clone(), Some(edited_config_bytes.to_vec())))
        .collect::<std::collections::BTreeMap<_, _>>();
    let overlaid = apply_overlay(base, overlay)?;

    // The authored config write carries the base preimage; the complete producer
    // set derives the coupled schemas/rule-membership/projections from the edited
    // configuration over the overlaid image (their targets keep their base
    // preimages, since only `config.toml` is overlaid).
    let mut actions = vec![RepositoryAction::WriteFile {
        path: config_path.clone(),
        owner: "authored-config".to_string(),
        expected: ExpectedPreimage::of(base.entry(&config_path).map_err(ProducerError::from)?),
        bytes: edited_config_bytes.to_vec(),
        mode: FileMode::Regular,
    }];
    let edited_declarations = RepositoryDeclarations {
        configuration: &edited_configuration,
        gates: declarations.gates,
        rules: declarations.rules,
    };
    actions.extend(compose_complete(&overlaid, &edited_declarations)?);
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    MaterializationPlan::new(base, seed, &MaterializationIntent::SemanticMutation, delta)
        .map_err(Into::into)
}

/// Pure derivation failure.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryStateError {
    /// Initialization or profile materialization failed.
    #[error(transparent)]
    Initialization(#[from] InitializationError),
    /// Delta normalization rejected an alias or duplicate target.
    #[error(transparent)]
    Delta(#[from] DeltaError),
    /// Plan identity serialization failed.
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
    /// Managed-document composition rejected an ambiguous or malformed claim.
    #[error(transparent)]
    ManagedDocument(#[from] ManagedDocumentError),
    /// Projection declaration or source resolution failed.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// A profile asset would overwrite an unowned authored occupant.
    #[error(transparent)]
    ProfileTargetConflict(#[from] ProfileTargetConflictError),
    /// A package replacement would overwrite a target changed after its base.
    #[error(transparent)]
    ProfileThreeWayConflict(#[from] Box<ProfileThreeWayConflictError>),
    /// Resolved package definitions disagree for one semantic identity.
    #[error(transparent)]
    ContributionComposition(#[from] ContributionCompositionConflict),
    /// Layout classification rejected a producer path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// A proposed-state overlay could not be closed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
    /// A producer read an uncaptured path or malformed captured bytes.
    #[error("materialization producer failed: {0}")]
    Producer(#[from] ProducerError),
    /// A gate-registry edit finalization failed.
    #[error(transparent)]
    GateRegistryEdit(#[from] GateRegistryEditError),
    /// An archive execution finalization failed.
    #[error(transparent)]
    ArchiveExecution(#[from] ArchiveExecutionError),
    /// Ownership of a materialization boundary cannot be proven, so repair is
    /// refused before publication rather than risk rewriting or deleting authored
    /// content (ownership matrix: "never rewrites an authored boundary it cannot
    /// prove"). Carries a typed reason naming the ambiguous boundary.
    #[error("ambiguous materialization ownership, not repairable: {0}")]
    AmbiguousOwnership(#[from] AmbiguousOwnershipError),
}

/// The proven-unprovable ownership boundary that refused a materialization repair.
///
/// Each variant holds the raw identifier at fault; rendering lives in this type's
/// `Display` impl. Composed into [`RepositoryStateError::AmbiguousOwnership`].
#[derive(Debug, thiserror::Error)]
pub enum AmbiguousOwnershipError {
    /// More than one installed profile claims the same materialization target.
    #[error("multiple installed profiles claim {0:?}")]
    MultipleProfileClaims(VirtualPath),
    /// `rules.toml` declares more than one rule with the same name, so
    /// default-rule and schema ownership cannot be proven.
    #[error(
        "rules.toml declares more than one rule named '{0}'; \
         default-rule and schema ownership cannot be proven"
    )]
    DuplicateRuleName(String),
}

/// The complete owned-materialization producer set: default rules and their
/// schemas, plus every configured projection, each derived from declared authority
/// (`@/inv/single-source-prose`). A caller cannot invoke a subset — the intent
/// selects the whole set. Shared projection targets compose through the one
/// managed-document primitive, never last-writer-wins; `rules.toml` splices only
/// the generated default-family spans, preserving authored content unconditionally.
fn compose_complete(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<Vec<RepositoryAction>, RepositoryStateError> {
    let config = materialize::assemble_config_from_declarations(image, declarations.configuration)?;
    let mut actions = materialize::compose_default_ruleset(image, &config)?;
    actions.extend(
        // A semantic mutation is complete over EVERY declared projection.
        materialize::compose_configured_projections(image, &config, declarations, None)?.actions,
    );
    Ok(actions)
}

/// Semantic-mutation intent: always invokes the complete producer set.
fn derive_semantic_mutation(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    Ok(RepositoryDelta::new(
        image.layout(),
        compose_complete(image, declarations)?,
    )?)
}

/// Render-only intent: a constrained-complete operation over the configured
/// projections in scope. `selected` scopes WHICH declared projections participate
/// (declaration scope), never which producer families run: every in-scope
/// projection is composed completely from its own declared sources, and an
/// out-of-scope projection contributes no action so its target is left untouched.
fn derive_project_render(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
    selected: Option<&std::collections::BTreeSet<String>>,
) -> Result<(RepositoryDelta, std::collections::BTreeMap<String, usize>), RepositoryStateError> {
    let config = materialize::assemble_config_from_declarations(image, declarations.configuration)?;
    let rendered =
        materialize::compose_configured_projections(image, &config, declarations, selected)?;
    Ok((
        RepositoryDelta::new(image.layout(), rendered.actions)?,
        rendered.counts,
    ))
}

/// Repair intent: the same complete expected state, whose per-target composition is
/// itself ownership-safe — `rules.toml` splices only generated default spans,
/// region projections splice only their managed region, and full-file projections
/// replace a target the declaration proves. Ambiguous ownership fails before
/// publication rather than rewriting an authored boundary.
fn derive_repair(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
    profiles: Vec<ProfileClaims>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    let mut actions = compose_complete(image, declarations)?;
    let mut claimed = std::collections::BTreeSet::new();
    for claims in profiles {
        for (path, (bytes, mode)) in profile_apply::compose_profile_targets(image, claims)?.targets
        {
            if !claimed.insert(path.clone()) {
                return Err(AmbiguousOwnershipError::MultipleProfileClaims(path.clone()).into());
            }
            actions.retain(|action| action.path() != &path);
            let expected = ExpectedPreimage::of(image.entry(&path).map_err(ProducerError::from)?);
            if !matches!(
                image.entry(&path).map_err(ProducerError::from)?,
                RepositoryEntry::File {
                    bytes: existing,
                    mode: existing_mode,
                    ..
                } if existing.as_slice() == bytes && *existing_mode == mode
            ) {
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: "profile-repair".to_string(),
                    expected,
                    bytes,
                    mode,
                });
            }
        }
    }
    Ok(RepositoryDelta::new(image.layout(), actions)?)
}

/// Enumerate the drift-independent set of paths `derive_repair` may materialize.
///
/// The returned set names every target repair could write, independent of whether
/// `image` currently drifts from it — unlike `derive_repair`, which is drift-
/// filtered at every family (a projection skips when its composed bytes already
/// match the captured occupant, the default ruleset writes only on diff, a profile
/// target pushes only when its entry differs). Walking `derive_repair`'s own
/// actions therefore enumerates only the paths currently drifting, not the
/// complete target space; this function enumerates the target space directly, so a
/// coverage test can hold it independent of current repository state.
///
/// Three families compose the result:
/// - Default-ruleset targets: `rules.toml` is a member exactly when the capture
///   spec captured it AND it is present (an absent `rules.toml` is the
///   in-memory-defaults case `derive_repair` never materializes anything for); its
///   baked schema files (`schemas/default-*.json`, derived from `declarations`'
///   configuration) are members exactly when the capture spec captured that exact
///   schema path. This mirrors the presence guards `derive_repair`'s default-rule
///   producer applies before ever comparing bytes, never the byte-diff itself.
/// - Configured-projection targets: every projection declared in `declarations`'
///   configuration contributes its own target unconditionally, since a declared
///   projection's target is always part of the operation's capture closure.
/// - Profile targets: the keys of `compose_profile_targets` for each installed
///   profile's claims (its composed bytes and mode are irrelevant to a path-only
///   authority).
///
/// Obsolete-schema deletions are deliberately excluded: they are occupant-proven
/// repair actions (a captured schema file whose sole default-rule reference is
/// gone), not declared targets, so naming them here would require reading occupant
/// state this function does not otherwise need.
///
/// `derive_repair` is not refactored to consume this set — it needs each target's
/// composed bytes, which a path set cannot supply — so a caller needing both the
/// target set and the composed delta invokes this function and `derive_repair`
/// separately over the same `image`, `declarations`, and `profiles`.
pub fn repair_target_paths(
    image: &RepositoryImage,
    declarations: RepositoryDeclarations<'_>,
    profiles: Vec<ProfileClaims>,
) -> Result<std::collections::BTreeSet<VirtualPath>, RepositoryStateError> {
    let config = materialize::assemble_config_from_declarations(image, declarations.configuration)?;
    let mut targets = std::collections::BTreeSet::new();

    // Default-ruleset family: rules.toml, guarded by capture-spec containment and
    // presence; its baked schema files, each individually guarded by capture-spec
    // containment.
    let rules_path = VirtualPath::RULES;
    if image.capture_spec().contains_path(&rules_path)
        && image
            .file_bytes(&rules_path)
            .map_err(ProducerError::from)?
            .is_some()
    {
        targets.insert(rules_path);
        for schema in materialize::serialized_default_ruleset(&config).schema_files {
            let vpath = VirtualPath::data(format!("schemas/{}", schema.name))?;
            if image.capture_spec().contains_path(&vpath) {
                targets.insert(vpath);
            }
        }
    }

    // Configured-projection targets: every declared projection's own target,
    // computed purely from declared configuration.
    if let Some(projections) = config.projection.as_ref() {
        for (name, projection) in projections {
            let target = require_target(projection, name)?;
            targets.insert(image.layout().classify_repository_relative(target)?);
        }
    }

    // Profile targets: the exact keys of each installed profile's composed target
    // set.
    for claims in profiles {
        targets.extend(
            profile_apply::compose_profile_targets(image, claims)?
                .targets
                .into_keys(),
        );
    }

    Ok(targets)
}

/// Kind of mismatch between a captured image and expected materialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializationDriftKind {
    /// Expected target is absent.
    Missing,
    /// Present target bytes or mode differ.
    Stale,
    /// An explicitly owned deletion target remains present.
    Unexpected,
}

/// One deterministic derived-state mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationDrift {
    /// Canonical target.
    pub path: VirtualPath,
    /// Mismatch classification.
    pub kind: MaterializationDriftKind,
}

/// Compare expected exact actions against the same closed image.
pub fn compare_materializations(
    image: &RepositoryImage,
    expected: &MaterializationPlan,
) -> Result<Vec<MaterializationDrift>, CaptureError> {
    expected
        .delta()
        .actions()
        .iter()
        .try_fold(Vec::new(), |mut drifts, action| {
            let path = match action {
                RepositoryAction::CreateDirectory { path, .. }
                | RepositoryAction::WriteFile { path, .. }
                | RepositoryAction::SetMode { path, .. }
                | RepositoryAction::DeleteFile { path, .. } => path,
            };
            let captured = image.entry(path)?;
            let drift = match (action, captured) {
                (RepositoryAction::CreateDirectory { .. }, RepositoryEntry::Directory { .. }) => {
                    None
                }
                (RepositoryAction::CreateDirectory { .. }, _) => {
                    Some(MaterializationDriftKind::Missing)
                }
                (
                    RepositoryAction::WriteFile { bytes, mode, .. },
                    RepositoryEntry::File {
                        bytes: actual,
                        mode: actual_mode,
                        ..
                    },
                ) if actual == bytes && actual_mode == mode => None,
                (RepositoryAction::WriteFile { .. }, _) => Some(MaterializationDriftKind::Stale),
                (
                    RepositoryAction::SetMode { mode, .. },
                    RepositoryEntry::File {
                        mode: actual_mode, ..
                    },
                ) if actual_mode == mode => None,
                (RepositoryAction::SetMode { .. }, _) => Some(MaterializationDriftKind::Stale),
                (RepositoryAction::DeleteFile { .. }, RepositoryEntry::Absent) => None,
                (RepositoryAction::DeleteFile { .. }, _) => {
                    Some(MaterializationDriftKind::Unexpected)
                }
            };
            if let Some(kind) = drift {
                drifts.push(MaterializationDrift {
                    path: path.clone(),
                    kind,
                });
            }
            Ok(drifts)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn budget() -> CaptureBudget {
        CaptureBudget {
            max_paths: 8,
            max_listings: 0,
            max_bytes: 128,
            max_depth: 4,
        }
    }

    fn plan(image: &RepositoryImage, actions: Vec<RepositoryAction>) -> MaterializationPlan {
        let seed = RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "repository-state-test".into(),
            },
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        MaterializationPlan::new(
            image,
            &seed,
            &MaterializationIntent::SemanticMutation,
            RepositoryDelta::new(image.layout(), actions).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn test_compare_materializations_propagates_undiscovered_for_every_action() {
        let layout = layout();
        let image = RepositoryImage::close(
            layout.clone(),
            CaptureSpec::phase_one(Vec::<VirtualPath>::new(), budget()).unwrap(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let target = VirtualPath::data("unrequested.json").unwrap();
        let occupant = EntryIdentity::for_bytes("occupant", b"captured").unwrap();
        let actions = [
            RepositoryAction::CreateDirectory {
                path: target.clone(),
                owner: "directory".into(),
                expected: ExpectedPreimage::Absent,
            },
            RepositoryAction::WriteFile {
                path: target.clone(),
                owner: "writer".into(),
                expected: ExpectedPreimage::Absent,
                bytes: b"expected".to_vec(),
                mode: FileMode::Regular,
            },
            RepositoryAction::SetMode {
                path: target.clone(),
                owner: "mode".into(),
                expected: ExpectedPreimage::File {
                    identity: occupant.clone(),
                    mode: FileMode::Regular,
                },
                mode: FileMode::Executable,
            },
            RepositoryAction::DeleteFile {
                path: target.clone(),
                owner: "delete".into(),
                expected: ExpectedPreimage::File {
                    identity: occupant,
                    mode: FileMode::Regular,
                },
            },
        ];

        for action in actions {
            assert_eq!(
                compare_materializations(&image, &plan(&image, vec![action])),
                Err(CaptureError::UndiscoveredRepositoryPath(target.clone()))
            );
        }
    }

    #[test]
    fn test_compare_materializations_classifies_captured_mismatches() {
        let layout = layout();
        let create = VirtualPath::data("create").unwrap();
        let delete = VirtualPath::data("delete").unwrap();
        let set_mode = VirtualPath::data("set-mode").unwrap();
        let write = VirtualPath::data("write").unwrap();
        let delete_bytes = b"delete me".to_vec();
        let mode_bytes = b"mode".to_vec();
        let write_bytes = b"old".to_vec();
        let delete_identity = EntryIdentity::for_bytes("delete", &delete_bytes).unwrap();
        let mode_identity = EntryIdentity::for_bytes("mode", &mode_bytes).unwrap();
        let image = RepositoryImage::close(
            layout.clone(),
            CaptureSpec::phase_one(
                [
                    create.clone(),
                    delete.clone(),
                    set_mode.clone(),
                    write.clone(),
                ],
                budget(),
            )
            .unwrap(),
            BTreeMap::from([
                (create.clone(), RepositoryEntry::Absent),
                (
                    delete.clone(),
                    RepositoryEntry::File {
                        identity: delete_identity.clone(),
                        bytes: delete_bytes,
                        mode: FileMode::Regular,
                    },
                ),
                (
                    set_mode.clone(),
                    RepositoryEntry::File {
                        identity: mode_identity.clone(),
                        bytes: mode_bytes,
                        mode: FileMode::Regular,
                    },
                ),
                (
                    write.clone(),
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes("write", &write_bytes).unwrap(),
                        bytes: write_bytes,
                        mode: FileMode::Regular,
                    },
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let expected = plan(
            &image,
            vec![
                RepositoryAction::CreateDirectory {
                    path: create.clone(),
                    owner: "directory".into(),
                    expected: ExpectedPreimage::Absent,
                },
                RepositoryAction::DeleteFile {
                    path: delete.clone(),
                    owner: "delete".into(),
                    expected: ExpectedPreimage::File {
                        identity: delete_identity,
                        mode: FileMode::Regular,
                    },
                },
                RepositoryAction::SetMode {
                    path: set_mode.clone(),
                    owner: "mode".into(),
                    expected: ExpectedPreimage::File {
                        identity: mode_identity,
                        mode: FileMode::Regular,
                    },
                    mode: FileMode::Executable,
                },
                RepositoryAction::WriteFile {
                    path: write.clone(),
                    owner: "writer".into(),
                    expected: ExpectedPreimage::Absent,
                    bytes: b"new".to_vec(),
                    mode: FileMode::Regular,
                },
            ],
        );

        assert_eq!(
            compare_materializations(&image, &expected).unwrap(),
            vec![
                MaterializationDrift {
                    path: create,
                    kind: MaterializationDriftKind::Missing,
                },
                MaterializationDrift {
                    path: delete,
                    kind: MaterializationDriftKind::Unexpected,
                },
                MaterializationDrift {
                    path: set_mode,
                    kind: MaterializationDriftKind::Stale,
                },
                MaterializationDrift {
                    path: write,
                    kind: MaterializationDriftKind::Stale,
                },
            ]
        );
    }

    fn empty_image() -> RepositoryImage {
        RepositoryImage::close(
            layout(),
            CaptureSpec::phase_one(Vec::<VirtualPath>::new(), budget()).unwrap(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn image_with(entries: Vec<(VirtualPath, RepositoryEntry)>) -> RepositoryImage {
        let paths: Vec<VirtualPath> = entries.iter().map(|(path, _)| path.clone()).collect();
        RepositoryImage::close(
            layout(),
            CaptureSpec::phase_one(paths, budget()).unwrap(),
            entries.into_iter().collect::<BTreeMap<_, _>>(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn empty_declarations() -> (ConfigurationDeclarations, GateRegistry, RuleSet) {
        (
            crate::declarations::parse_configuration(b"").unwrap(),
            GateRegistry::default(),
            RuleSet { rules: Vec::new() },
        )
    }

    fn gate_definition(key: &str) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: String::new(),
            stage: crate::declarations::GateStage::Postcheck,
            mode: crate::declarations::GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
            inputs: None,
        }
    }

    #[test]
    fn test_finalize_gate_registry_edit_missing_edit_intent_is_typed() {
        let image = empty_image();
        let context = MutationContext::preview();
        let (configuration, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            configuration: &configuration,
            gates: &gates,
            rules: &rules,
        };
        let error = finalize_gate_registry_edit(&layout(), &image, &context, &[], declarations)
            .unwrap_err();
        assert!(matches!(
            error,
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::MissingEdit)
        ));
    }

    #[test]
    fn test_finalize_gate_registry_edit_multiple_edit_intents_is_typed() {
        let image = empty_image();
        let context = MutationContext::preview();
        let (configuration, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            configuration: &configuration,
            gates: &gates,
            rules: &rules,
        };
        let intents = vec![
            MutationIntent::EditGateRegistry {
                registry: Box::new(GateRegistry::default()),
            },
            MutationIntent::EditGateRegistry {
                registry: Box::new(GateRegistry::default()),
            },
        ];
        let error =
            finalize_gate_registry_edit(&layout(), &image, &context, &intents, declarations)
                .unwrap_err();
        assert!(matches!(
            error,
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::MultipleEdits)
        ));
    }

    #[test]
    fn test_finalize_gate_registry_edit_declaration_mismatch_is_typed() {
        let image = empty_image();
        let context = MutationContext::preview();
        let (configuration, _default_gates, rules) = empty_declarations();
        let mut mismatched = GateRegistry::default();
        mismatched
            .gates
            .insert("distinct".to_string(), gate_definition("distinct"));
        let declarations = RepositoryDeclarations {
            configuration: &configuration,
            gates: &mismatched,
            rules: &rules,
        };
        let intents = vec![MutationIntent::EditGateRegistry {
            registry: Box::new(GateRegistry::default()),
        }];
        let error =
            finalize_gate_registry_edit(&layout(), &image, &context, &intents, declarations)
                .unwrap_err();
        assert!(matches!(
            error,
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::DeclarationMismatch)
        ));
    }

    #[test]
    fn test_finalize_gate_registry_edit_propagates_typed_mutation_error() {
        // A well-formed single edit intent alongside a claim of an issue absent
        // from the captured image: the record finalizer's own typed
        // `MutationError::MissingIssue` must surface through
        // `GateRegistryEditError::Mutation`, short-circuiting before the
        // declaration-derived producer set is ever composed.
        let image = image_with(vec![
            (
                VirtualPath::data("events.jsonl").unwrap(),
                RepositoryEntry::Absent,
            ),
            (
                VirtualPath::data("issues/missing.json").unwrap(),
                RepositoryEntry::Absent,
            ),
        ]);
        let context = MutationContext::preview();
        let (configuration, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            configuration: &configuration,
            gates: &gates,
            rules: &rules,
        };
        let intents = vec![
            MutationIntent::EditGateRegistry {
                registry: Box::new(GateRegistry::default()),
            },
            MutationIntent::ClaimIssue {
                issue_id: "missing".to_string(),
                agent: "agent:tester".parse().unwrap(),
            },
        ];
        let error =
            finalize_gate_registry_edit(&layout(), &image, &context, &intents, declarations)
                .unwrap_err();
        match error {
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::Mutation(
                MutationError::MissingIssue(id),
            )) => assert_eq!(id, "missing"),
            other => panic!("expected a typed missing-issue mutation error, got {other:?}"),
        }
    }
}
