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
mod path;
mod profile_apply;
mod projection;
mod projection_render;
mod rule_serialize;
mod rules_document;
mod rules_gates_projection;

pub(crate) use archive::captured_archive_events;
pub use archive::finalize_archive_execution;
pub use default_rules::{
    default_rule_membership_diff, default_rule_membership_diff_from_identities, default_ruleset,
    hierarchy_config, reconcile_default_rules_with_config, type_hierarchy_known_schema,
    DefaultRuleMembershipDiff, TYPE_HIERARCHY_SCHEMA_FILE,
};
pub(crate) use export::{
    classify_repository_export, finalize_repository_export, ExternalExportPath,
    RepositoryExportDestination, RepositoryExportError, RepositoryExportIntent,
};
pub use image::{
    plan_hash, CaptureBudget, CaptureError, CaptureSpec, DeltaError, EntryIdentity,
    ExpectedPreimage, FileMode, LinkedWorktreeEvidence, LinkedWorktreeSourceClass,
    ListingFingerprint, MaterializationIntent, PinnedDocumentEvidence, PinnedSourceClass,
    PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositorySeed, RepositorySeedKind, SeedError, TargetClaim,
};
pub(crate) use index::{RepositoryIndex, RepositoryIndexError, SUPPORTED_INDEX_SCHEMA_VERSION};
pub use initialize::{
    render_repo_config, GitattributesClaim, GitattributesStatus, InitializationError,
    InitializationScaffold,
};
pub use managed_document::{
    compose_managed_documents, render_managed_document, ManagedDocumentClaim, ManagedDocumentError,
    RegionPlacement,
};
pub use materialize::{
    assemble_config, render_capture_closure, validate_capture_closure, ValidationCaptureClosure,
};
pub(crate) use mutation::captured_gate_run_result_paths;
pub use mutation::{
    finalize, finalize_audit_append, fresh_index_bytes, gate_run_result_relative_path, issue_draft,
    prefix_has_torn_tail, profile_applied_event, serialize_event, serialize_gate_run,
    serialize_issue, FixedMutationClock, IdAuthority, MutationClock, MutationContext,
    MutationError, MutationIntent, SystemMutationClock,
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
) -> anyhow::Result<MaterializationPlan> {
    let registry = intents
        .iter()
        .find_map(|intent| match intent {
            MutationIntent::EditGateRegistry { registry } => Some(&**registry),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("gate registry finalization requires one typed edit"))?;
    if intents
        .iter()
        .filter(|intent| matches!(intent, MutationIntent::EditGateRegistry { .. }))
        .count()
        != 1
    {
        anyhow::bail!("gate registry finalization requires exactly one typed edit");
    }
    if declarations.gates != registry {
        anyhow::bail!("gate registry edit and projection declarations disagree");
    }

    let record_plan = finalize(layout, base, context, intents)?;
    let gate_path = VirtualPath::data("gates.toml")?;
    let gate_bytes = crate::declarations::serialize_gate_registry(registry)?;
    let overlaid = apply_overlay(base, std::iter::once((gate_path, Some(gate_bytes))))?;
    let mut actions = record_plan.delta().actions().to_vec();
    actions.extend(compose_complete(&overlaid, &declarations)?);
    let delta = RepositoryDelta::new(layout, actions)?;
    let seed = context.repository_seed(intents)?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::SemanticMutation,
        delta,
    )?)
}
pub use overlay::{apply_overlay, OverlayError};
pub use path::{
    InjectivityProof, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryRootEvidence, RootRelativePath, VirtualPath,
};
pub(crate) use profile_apply::profile_capture_closure;
pub use profile_apply::{
    AppliedProfileRecord, CompleteProjectionConfig, Contribution, KeyedArrayTarget, MapEntryTarget,
    ProfileApplicationInput, ProfileAssetClaim, ProfileClaims, ProfileRegionClaim,
    ProfileTargetConflictError, SetStringTarget,
};
pub use projection::{
    render_id_anchor_rows, render_invariants_markdown, require_target, ProjectionError,
};
pub(crate) use projection_render::{render_projection_body, ProjectionInputs};
pub use rule_serialize::{
    render_rule_block, rules_file_header, serialize_ruleset, SchemaFile, SerializedRuleSet,
};
pub use rules_document::{parse_rule_identities, splice_default_membership};
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
    #[error("{0:#}")]
    RulesDocument(anyhow::Error),
    /// Captured declarations could not be assembled into the canonical bundle.
    #[error("{0:#}")]
    DeclarationAssembly(anyhow::Error),
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
    /// A profile contribution conflicts with an existing registry identity.
    #[error("profile contribution '{identity}' conflicts in '{registry}'")]
    ProfileContributionConflict { identity: String, registry: String },
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
    #[error("contribution value is not a table")]
    ContributionNotTable,
    #[error("null is not a TOML value")]
    NullValue,
    #[error("unsupported numeric value")]
    UnsupportedNumericValue,
    #[error("value is not a table")]
    ValueNotTable,
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
    rules_load_error: Option<anyhow::Error>,
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

    pub(crate) fn rules_load_error(&self) -> Option<&anyhow::Error> {
        self.rules_load_error.as_ref()
    }

    pub(crate) fn take_rules_load_error(&mut self) -> Option<anyhow::Error> {
        self.rules_load_error.take()
    }
}

/// Parse one captured image into the canonical declaration bundle exactly once.
pub(crate) fn declarations_from_image(
    image: &RepositoryImage,
) -> anyhow::Result<CapturedRepositoryDeclarations> {
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
) -> anyhow::Result<CapturedRepositoryDeclarations> {
    let config_bytes = image
        .file_bytes(&VirtualPath::data("config.toml")?)?
        .ok_or_else(|| anyhow::anyhow!("captured image has no .jit/config.toml"))?;
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
    config.templates = match image.file_bytes(&VirtualPath::data("templates.toml")?) {
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
    let gates = match image.file_bytes(&VirtualPath::data("gates.toml")?)? {
        Some(bytes) => crate::declarations::parse_gate_registry(bytes)?,
        None => GateRegistry::default(),
    };
    let rules = match image.file_bytes(&VirtualPath::data("rules.toml")?)? {
        Some(bytes) => (|| -> anyhow::Result<RuleSet> {
            let content = std::str::from_utf8(bytes)?;
            let schemas = RuleSet::schema_requests(content)?
                .into_iter()
                .map(|request| -> anyhow::Result<_> {
                    let path = VirtualPath::data(&request.reference)?;
                    let bytes = image.file_bytes(&path)?.ok_or_else(|| {
                        anyhow::anyhow!("captured image has no {}", request.reference)
                    })?;
                    Ok((request.reference, bytes.to_vec()))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileTargetDisposition {
    Unchanged,
    Create,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileTargetMaterialization {
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
}

impl MaterializationDerivation {
    fn new(delta: RepositoryDelta, seed: RepositorySeed, intent: MaterializationIntent) -> Self {
        Self {
            delta,
            seed,
            intent,
            profile_targets: Vec::new(),
        }
    }

    fn with_profile_targets(mut self, targets: Vec<ProfileTargetMaterialization>) -> Self {
        self.profile_targets = targets;
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
        /// Neutral claims for every installed profile whose exact embedded package
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
    /// Derive one embedded profile application over an existing repository.
    ApplyProfile {
        /// Parsed package metadata and neutral canonical claims.
        profile: ProfileApplicationInput,
        /// Stable mutation identity and time authority.
        context: &'a MutationContext,
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
        MaterializationRequest::ApplyProfile { profile, context } => (
            initialize::derive_profile_application(image, &profile, context)?,
            Default::default(),
        ),
    };
    let MaterializationDerivation {
        delta,
        seed,
        intent,
        profile_targets,
    } = derivation;
    MaterializationPlan::new(image, &seed, &intent, delta)
        .map(|plan| {
            plan.with_projection_counts(projection_counts)
                .with_profile_targets(profile_targets)
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

    let config_path = VirtualPath::data("config.toml")?;
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
    /// Layout classification rejected a producer path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// A proposed-state overlay could not be closed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
    /// A producer read an uncaptured path or malformed captured bytes.
    #[error("materialization producer failed: {0}")]
    Producer(#[from] ProducerError),
    /// Ownership of a materialization boundary cannot be proven, so repair is
    /// refused before publication rather than risk rewriting or deleting authored
    /// content (ownership matrix: "never rewrites an authored boundary it cannot
    /// prove"). Carries a human description of the ambiguous boundary.
    #[error("ambiguous materialization ownership, not repairable: {0}")]
    AmbiguousOwnership(String),
}

impl RepositoryStateError {
    /// Whether this derivation failed on an unowned profile target occupant.
    pub fn is_profile_target_conflict(&self) -> bool {
        matches!(
            self,
            Self::ProfileTargetConflict(_)
                | Self::Initialization(InitializationError::ProfileTargetConflict(_))
        )
    }
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
        for (path, (bytes, mode)) in profile_apply::compose_profile_targets(image, claims)? {
            if !claimed.insert(path.clone()) {
                return Err(RepositoryStateError::AmbiguousOwnership(format!(
                    "multiple installed profiles claim {path:?}"
                )));
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
}
