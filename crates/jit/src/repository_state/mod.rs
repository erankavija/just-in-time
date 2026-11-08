//! Pure closed repository-state capture and materialization vocabulary.
//!
//! This subsystem owns canonical paths, immutable images, bounded capture,
//! managed-document composition, and exact deltas.
//! It performs no filesystem I/O and imports no validation, storage, command, or
//! profile modules.

use crate::domain::identity::ProfileId;
mod archive;
mod default_rules;
mod export;
mod image;
mod index;
mod initialize;
mod issue_record;
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

pub use archive::{finalize_archive_execution, ArchiveExecutionError};
pub(crate) use default_rules::default_rule_membership_diff_from_identities;
pub use default_rules::{
    default_ruleset, reconcile_default_rules_with_config, type_hierarchy_known_schema,
    TYPE_HIERARCHY_SCHEMA_FILE,
};
pub(crate) use export::{
    classify_repository_export, finalize_repository_export, ExternalExportPath,
    RepositoryExportDestination, RepositoryExportError, RepositoryExportIntent,
};
pub use image::{
    CaptureBudget, CaptureError, CaptureSpec, CapturedBytes, DeltaError, DirectoryListing,
    EntryIdentity, EventLogTail, ExpectedPreimage, FileMode, PinnedDocumentEvidence,
    PinnedSourceClass, ReadMode, RepositoryAction, RepositoryActionOwner, RepositoryDelta,
    RepositoryEntry, RepositoryImage, RepositoryTargetKind, TargetClaim, EVENT_LOG_TAIL_WINDOW,
};
pub(crate) use image::{CapturedAnswers, ImageToken};
pub use index::RepositoryIndexError;
#[cfg(test)]
pub(crate) use index::SUPPORTED_INDEX_SCHEMA_VERSION;
pub(crate) use index::{captured_issue_ids, listed_issue_ids, RepositoryIndex};
pub use initialize::{
    render_repo_config, structural_minimum_config, GitattributesClaim, GitattributesStatus,
    InitializationError, InitializationScaffold, FRESH_CONFIG_SCHEMA_VERSION,
};
pub use issue_record::{
    decode_issue_record, parse_issue_records, IssueRecordsError, MalformedIssueRecordError,
};
pub(crate) use managed_document::compose_managed_documents;
pub use managed_document::{
    render_managed_document, ManagedDocumentClaim, ManagedDocumentError, RegionPlacement,
};
pub use materialize::{
    assemble_config, render_capture_closure, render_data_root_ignore, validate_capture_closure,
    ValidationCaptureClosure,
};
pub(crate) use mutation::MutationContextFactory;
pub(crate) use mutation::{
    canonical_json, captured_gate_run_ids, captured_gate_run_issue_ids, captured_gate_verdict_keys,
    captured_gate_verdict_records, created_issue, mutated_issue,
};
pub use mutation::{
    finalize, fresh_index_bytes, serialize_event, serialize_gate_run, serialize_gate_verdict,
    serialize_issue, IdAuthority, MutationClock, MutationContext, MutationContextAnnotation,
    MutationError, MutationIntent, SystemMutationClock,
};
pub use package_tree::{
    finalize_package_tree_capture, CapturedTreeFile, PackageTreeCapture, PackageTreeCaptureError,
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
        finalize(layout, base, context, intents, None).map_err(GateRegistryEditError::Mutation)?;
    let gate_path = VirtualPath::GATES;
    let gate_bytes = match record_plan
        .delta()
        .actions()
        .iter()
        .find_map(|action| match action {
            RepositoryAction::WriteFile { path, bytes, .. } if path == &gate_path => Some(bytes),
            _ => None,
        }) {
        Some(bytes) => bytes.clone(),
        None => base
            .file_bytes(&gate_path)
            .map_err(GateRegistryEditError::Capture)?
            .map(|bytes| bytes.to_vec())
            .ok_or(GateRegistryEditError::MissingDocument)?,
    };
    let overlaid = apply_overlay(base, std::iter::once((gate_path, Some(gate_bytes))))
        .map_err(GateRegistryEditError::Overlay)?;
    let mut actions = record_plan.delta().actions().to_vec();
    actions.extend(compose_complete(&overlaid, &declarations)?);
    let delta = RepositoryDelta::new(layout, actions)?;
    MaterializationPlan::new(base, delta, context)
        .map_err(|error| GateRegistryEditError::Mutation(error).into())
}

/// A typed failure raised while finalizing a gate-registry edit.
///
/// Composed into [`RepositoryStateError::GateRegistryEdit`]. The four
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
    /// The captured registry bytes could not be read.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// The typed edit's registry disagreed with the projection declarations.
    #[error("gate registry edit and projection declarations disagree")]
    DeclarationMismatch,
    /// The record finalizer failed.
    #[error(transparent)]
    Mutation(#[from] MutationError),
    /// The edited registry document could not be materialized.
    #[error(transparent)]
    GateDeclaration(#[from] crate::declarations::GateDeclarationError),
    /// The record plan and captured image both lacked the resulting document.
    #[error("gate registry edit produced no document bytes")]
    MissingDocument,
    /// The proposed-state overlay could not be closed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
}
pub(crate) use overlay::overlaid_issue_ids;
pub use overlay::{action_overrides, apply_overlay, OverlayError};
pub use path::{
    is_graph_relevant_data_path, repository_relative_path, RepositoryLayout, RepositoryLayoutError,
    RepositoryRootClass, RepositoryRootEvidence, VirtualPath, BOOTSTRAP_LOCK_FILE,
    EVENTS_LOCK_FILE, INDEX_LOCK_FILE, RECORD_SCRATCH_DIR, REPO_WRITE_LOCK_FILE,
};
pub use profile_apply::{
    compose_resolved_contributions, CompleteProjectionConfig, ComposedContribution, Contribution,
    ContributionIdentity, ContributionIdentityTarget, ContributionOutcome, ContributionRegistry,
    KeyedArrayTarget, MapEntryTarget, ProfileApplicationInput, ProfileAssetClaim, ProfileClaims,
    ProfileContributionClaim, ProfileRegionClaim, ProfileTargetConflict,
    ProfileTargetConflictEntry, ProfileTargetConflictsError, ProfileTargetSubject, ScalarTarget,
    SetStringTarget,
};
pub(crate) use profile_apply::{
    drop_overridden_claims, profile_capture_closure, profile_contribution_overrides,
    profile_contribution_target_paths,
};
pub use projection::{
    render_id_anchor_rows, render_invariants_markdown, require_target, ProjectionError,
};
pub(crate) use projection_render::{render_projection_body, ProjectionInputs};
pub use rule_serialize::{render_rule_block, rules_file_header};
pub use rule_serialize::{serialize_ruleset, SchemaFile, SerializedRuleSet};
pub use rules_document::{parse_rule_identities, splice_default_membership, RulesDocumentError};
pub use rules_gates_projection::render_rules_and_gates_markdown;

use crate::declarations::rules::RuleSet;
use crate::declarations::{ConfigurationAssemblyError, GateRegistry};

/// A typed failure raised while deriving materialized repository state.
///
/// Producer errors retain their concrete source and raw declaration identities;
/// rendering belongs to this type rather than to producer call sites.
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    /// Captured bytes were not valid UTF-8.
    #[error(transparent)]
    MalformedUtf8(#[from] std::string::FromUtf8Error),
    /// A requested projection is not declared.
    #[error("unknown projection '{0}'")]
    UnknownProjection(String),
    /// A projection references an item kind that is not declared.
    #[error("projection '{projection}' references unknown kind '{kind}'")]
    UnknownKind { projection: String, kind: String },
    /// Closed-image capture failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A producer path was invalid for the repository layout.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// Captured configuration declarations failed assembly.
    #[error(transparent)]
    Configuration(#[from] ConfigurationAssemblyError),
    /// Captured invariant declarations were invalid.
    #[error(transparent)]
    InvariantConfig(#[from] crate::declarations::invariants::InvariantConfigError),
    /// Addressable item declarations or source content were invalid.
    #[error(transparent)]
    Item(#[from] std::sync::Arc<crate::domain::item::ItemError>),
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
    /// The proposed archive layout leaves a supported edge unresolvable.
    #[error(transparent)]
    ProposedLayout(#[from] crate::domain::artifact_classifier::ProposedLayoutViolation),
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

impl From<crate::domain::item::ItemError> for ProducerError {
    fn from(error: crate::domain::item::ItemError) -> Self {
        Self::Item(std::sync::Arc::new(error))
    }
}

/// A typed failure raised while parsing captured declaration files into the
/// canonical bundle (`declarations_from_image` / `validation_declarations_with_config`).
///
/// Each variant carries its leaf parser's concrete error; rendering lives in this
/// type's `Display` impl rather than at the parse site. Also the stored type of a
/// deferred `rules.toml` load failure (`CapturedRepositoryDeclarations::rules_load_error`).
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
    /// The captured configuration could not be assembled.
    #[error(transparent)]
    Producer(#[from] ProducerError),
    /// The gate registry could not be parsed.
    #[error(".jit/gates.toml: {0}")]
    Gate(#[from] crate::declarations::GateDeclarationError),
    /// The rule set could not be parsed.
    #[error(transparent)]
    Rule(#[from] crate::declarations::rules::RuleConfigError),
}

type ResolvedItemKinds = Result<
    std::sync::Arc<[crate::domain::item::ItemKind]>,
    std::sync::Arc<crate::domain::item::ItemError>,
>;

fn resolved_item_kinds(config: &crate::config::JitConfig) -> ResolvedItemKinds {
    crate::domain::item::resolve_item_kinds(config.item_kinds.as_ref())
        .map(std::sync::Arc::from)
        .map_err(std::sync::Arc::new)
}

/// Explicit declaration bundle consumed by the closed producer call graph.
pub struct RepositoryDeclarations<'a> {
    /// Resolved kinds, retaining errors for the effective-configuration pass.
    pub item_kinds: ResolvedItemKinds,
    /// Assembled repository configuration.
    pub config: &'a crate::config::JitConfig,
    /// Authored gate registry.
    pub gates: &'a GateRegistry,
    /// Authored rule registry.
    pub rules: &'a RuleSet,
}

/// One owned parse of every captured declaration consumed by materialization,
/// validation, or a read.
pub struct CapturedRepositoryDeclarations {
    item_kinds: ResolvedItemKinds,
    namespaces: crate::domain::LabelNamespaces,
    pub(crate) config: crate::config::JitConfig,
    pub(crate) gates: GateRegistry,
    pub(crate) rules: RuleSet,
    rules_load_error: Option<DeclarationParseError>,
}

impl CapturedRepositoryDeclarations {
    pub(crate) fn from_parts(
        config: crate::config::JitConfig,
        gates: GateRegistry,
        rules: RuleSet,
    ) -> Self {
        Self {
            item_kinds: resolved_item_kinds(&config),
            namespaces: crate::config_manager::namespaces_from_config(&config),
            config,
            gates,
            rules,
            rules_load_error: None,
        }
    }

    pub(crate) fn borrowed(&self) -> RepositoryDeclarations<'_> {
        RepositoryDeclarations {
            item_kinds: self.item_kinds.clone(),
            config: &self.config,
            gates: &self.gates,
            rules: &self.rules,
        }
    }

    /// The assembled repository configuration.
    pub fn config(&self) -> &crate::config::JitConfig {
        &self.config
    }

    /// The label namespace registry the configuration declares.
    pub fn namespaces(&self) -> &crate::domain::LabelNamespaces {
        &self.namespaces
    }

    /// The type hierarchy built from [`namespaces`](Self::namespaces).
    ///
    /// # Errors
    ///
    /// A malformed declared hierarchy.
    pub fn hierarchy(
        &self,
    ) -> Result<
        crate::domain::type_taxonomy::HierarchyConfig,
        crate::domain::type_taxonomy::ConfigError,
    > {
        crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&self.namespaces)
    }

    pub(crate) fn item_kinds(&self) -> Result<&[crate::domain::item::ItemKind], ProducerError> {
        self.item_kinds
            .as_deref()
            .map_err(|error| error.clone().into())
    }

    /// The effective rule set: authored rules reconciled with the
    /// registry-derived defaults.
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    /// The authored gate registry.
    pub fn gates(&self) -> &GateRegistry {
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
    declarations_with_config(image, materialize::assemble_config(image))
}

/// [`declarations_from_image`] over `config`, the configuration assembly of
/// `image` already made.
pub(crate) fn declarations_with_config(
    image: &RepositoryImage,
    config: Result<crate::config::JitConfig, ProducerError>,
) -> Result<CapturedRepositoryDeclarations, DeclarationParseError> {
    let mut declarations = validation_declarations_with_config(image, config)?;
    match declarations.take_rules_load_error() {
        Some(error) => Err(error),
        None => Ok(declarations),
    }
}

/// Parse captured declarations for validation over `config`, the
/// configuration assembly of `image` already made, while preserving an
/// unloadable rules source as reportable state. Configuration, gates,
/// templates, and invariants remain strict; only rules use the empty-set
/// sentinel because validation must still report bindings into an unloadable
/// rule source.
pub(crate) fn validation_declarations_with_config(
    image: &RepositoryImage,
    config: Result<crate::config::JitConfig, ProducerError>,
) -> Result<CapturedRepositoryDeclarations, DeclarationParseError> {
    let config = config?;
    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let gates = match image.file_bytes(&VirtualPath::GATES)? {
        Some(bytes) => crate::declarations::parse_gate_registry(bytes)?,
        None => GateRegistry::default(),
    };
    let rules = match image.file_bytes(&VirtualPath::RULES)? {
        Some(bytes) => (|| -> Result<RuleSet, DeclarationParseError> {
            let content = std::str::from_utf8(bytes)?;
            // An absent optional schema is skipped; the parse substitutes its
            // placeholder.
            let schemas = RuleSet::schema_requests(content)?
                .into_iter()
                .filter_map(|request| -> Option<Result<_, DeclarationParseError>> {
                    let bytes = VirtualPath::schema_reference(&request.reference)
                        .map_err(DeclarationParseError::from)
                        .and_then(|path| Ok(image.file_bytes(&path)?.map(<[u8]>::to_vec)));
                    match bytes {
                        Ok(Some(bytes)) => Some(Ok((request.reference, bytes))),
                        Ok(None) if !request.required => None,
                        Ok(None) => Some(Err(DeclarationParseError::MissingCapture(
                            request.reference,
                        ))),
                        Err(error) => Some(Err(error)),
                    }
                })
                .collect::<Result<Vec<_>, DeclarationParseError>>()?;
            let parsed = RuleSet::parse(content, Some(&config), schemas)?;
            Ok(reconcile_default_rules_with_config(parsed, &namespaces)?)
        })(),
        None => Ok(default_ruleset(&namespaces)),
    };
    let (rules, rules_load_error) = match rules {
        Ok(rules) => (rules, None),
        Err(error) => (RuleSet { rules: Vec::new() }, Some(error)),
    };
    let mut declarations = CapturedRepositoryDeclarations::from_parts(config, gates, rules);
    declarations.rules_load_error = rules_load_error;
    Ok(declarations)
}

/// How a configuration assembly of an image becomes its declarations:
/// [`declarations_with_config`] or [`validation_declarations_with_config`].
pub(crate) type DeclarationParse =
    fn(
        &RepositoryImage,
        Result<crate::config::JitConfig, ProducerError>,
    ) -> Result<CapturedRepositoryDeclarations, DeclarationParseError>;

/// The declarations of one captured image, parsed on first use from the
/// configuration its capture assembled, so a holder fails on a declaration
/// only when it reads one. Every read returns that one parse, its failure
/// included.
pub(crate) struct DeferredDeclarations {
    parse: DeclarationParse,
    /// The capture's assembly, until the parse takes it; `None` assembles
    /// from the image.
    config: std::sync::Mutex<Option<Result<crate::config::JitConfig, ProducerError>>>,
    parsed: std::sync::OnceLock<
        Result<CapturedRepositoryDeclarations, std::sync::Arc<DeclarationParseError>>,
    >,
}

impl DeferredDeclarations {
    /// Declarations `parse` derives from `config`, or from the image's own
    /// assembly when `config` is `None`.
    pub(crate) fn new(
        parse: DeclarationParse,
        config: Option<Result<crate::config::JitConfig, ProducerError>>,
    ) -> Self {
        Self {
            parse,
            config: std::sync::Mutex::new(config),
            parsed: std::sync::OnceLock::new(),
        }
    }

    /// The declarations of `image`, the image this holder was captured with.
    pub(crate) fn get(
        &self,
        image: &RepositoryImage,
    ) -> Result<&CapturedRepositoryDeclarations, std::sync::Arc<DeclarationParseError>> {
        self.parsed
            .get_or_init(|| {
                let config = self
                    .config
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .unwrap_or_else(|| materialize::assemble_config(image));
                (self.parse)(image, config).map_err(std::sync::Arc::new)
            })
            .as_ref()
            .map_err(std::sync::Arc::clone)
    }
}

impl std::fmt::Debug for DeferredDeclarations {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeferredDeclarations")
            .field("parsed", &self.parsed.get().map(Result::is_ok))
            .finish_non_exhaustive()
    }
}

/// Complete deterministic pure materialization result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationPlan {
    /// Exact bounded repository image from which this plan was derived.
    image: RepositoryImage,
    /// Exact normalized delta.
    delta: RepositoryDelta,
    /// The delta storage publishes: `delta`, plus the generic event append an
    /// event-less write over a present data root carries.
    publication: RepositoryDelta,
    /// Per-projection row counts produced by a configured-projection render.
    projection_counts: std::collections::BTreeMap<String, usize>,
    profile_targets: Vec<ProfileTargetMaterialization>,
    profile_contributions: Vec<ProfileContributionMaterialization>,
    applied_profiles: std::collections::BTreeSet<ProfileId>,
}

/// What one profile decided about one target it participates in.
///
/// A value the profile publishes is created, updated, or already exact, and a
/// value packages being applied define differently is a conflict carrying its
/// own reason. A publication acts on this vocabulary and a difference report
/// states it, so neither can describe an outcome the other would not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileTargetDisposition {
    /// The repository already holds the value this profile would publish.
    Unchanged,
    /// The target is absent and would be created.
    Create,
    /// The target holds another value this profile replaces.
    Update,
    /// The target is a derived file the profile's composition no longer yields.
    Remove,
    /// The target cannot be published, for the reason it carries.
    Conflict(ProfileTargetConflict),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileTargetMaterialization {
    /// Package whose composed claims decided this target.
    ///
    /// One aggregate plan carries the targets of every member it composes, so a
    /// reader that reports per profile needs each decision to name the profile
    /// that made it. A target two packages both contribute is decided by each
    /// of them and therefore appears once per contributing owner.
    pub(crate) owner: ProfileId,
    pub(crate) path: VirtualPath,
    pub(crate) disposition: ProfileTargetDisposition,
    pub(crate) mode: FileMode,
}

/// What one profile decided about one semantic identity it participates in.
///
/// Contributions compose by identity rather than by the registry file that
/// holds them, so this is what a plan carries beside its file-target decisions:
/// the same disposition vocabulary, stated for the declaration an adopter reads
/// and resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileContributionMaterialization {
    /// Package whose composed claims decided this declaration.
    pub(crate) owner: ProfileId,
    /// Canonical semantic identity the decision is about.
    pub(crate) identity: ContributionIdentity,
    pub(crate) disposition: ProfileTargetDisposition,
}

/// Complete producer output awaiting the shared plan tail.
struct MaterializationDerivation {
    delta: RepositoryDelta,
    profile_targets: Vec<ProfileTargetMaterialization>,
    profile_contributions: Vec<ProfileContributionMaterialization>,
    applied_profiles: std::collections::BTreeSet<ProfileId>,
}

impl MaterializationDerivation {
    fn new(delta: RepositoryDelta) -> Self {
        Self {
            delta,
            profile_targets: Vec::new(),
            profile_contributions: Vec::new(),
            applied_profiles: std::collections::BTreeSet::new(),
        }
    }

    fn with_profile_targets(mut self, targets: Vec<ProfileTargetMaterialization>) -> Self {
        self.profile_targets = targets;
        self
    }

    fn with_profile_contributions(
        mut self,
        contributions: Vec<ProfileContributionMaterialization>,
    ) -> Self {
        self.profile_contributions = contributions;
        self
    }

    /// Record packages whose candidate derivations have a non-event effect.
    fn with_applied_profiles(mut self, profiles: std::collections::BTreeSet<ProfileId>) -> Self {
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

    /// The delta storage publishes; reports read [`Self::delta`].
    pub fn publication(&self) -> &RepositoryDelta {
        &self.publication
    }

    /// Return the count produced for one configured projection in this plan.
    pub(crate) fn projection_count(&self, name: &str) -> Option<usize> {
        self.projection_counts.get(name).copied()
    }

    pub(crate) fn profile_targets(&self) -> &[ProfileTargetMaterialization] {
        &self.profile_targets
    }

    pub(crate) fn profile_contributions(&self) -> &[ProfileContributionMaterialization] {
        &self.profile_contributions
    }

    /// Package identities responsible for a non-event effect in this plan.
    pub(crate) fn applied_profiles(&self) -> &std::collections::BTreeSet<ProfileId> {
        &self.applied_profiles
    }

    /// Close a delta over the image it was derived from, composing its
    /// publication with `context`'s event identity and timestamp.
    ///
    /// # Errors
    ///
    /// [`MutationError::Capture`] when the publication needs the generic event
    /// and `image` did not capture the event log's tail.
    pub(crate) fn new(
        image: &RepositoryImage,
        delta: RepositoryDelta,
        context: &MutationContext,
    ) -> Result<Self, MutationError> {
        let publication = mutation::publication_of(image, &delta, context)?;
        Ok(Self {
            image: image.clone(),
            delta,
            publication,
            projection_counts: std::collections::BTreeMap::new(),
            profile_targets: Vec::new(),
            profile_contributions: Vec::new(),
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

    fn with_profile_contributions(
        mut self,
        contributions: Vec<ProfileContributionMaterialization>,
    ) -> Self {
        self.profile_contributions = contributions;
        self
    }

    fn with_applied_profiles(mut self, profiles: std::collections::BTreeSet<ProfileId>) -> Self {
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
    },
    /// Render the selected configured projections (`None` selects all).
    RenderConfiguredProjections {
        /// Parsed declarations captured from the image.
        declarations: RepositoryDeclarations<'a>,
        /// Declaration-scoped projection names.
        selected: Option<std::collections::BTreeSet<String>>,
    },
    /// Repair all declaration-owned derived state.
    RepairDerivedState {
        /// Parsed declarations captured from the image.
        declarations: RepositoryDeclarations<'a>,
    },
    /// Derive a fresh or missing-file repository scaffold.
    Initialize {
        /// Fully rendered semantic scaffold contribution.
        scaffold: &'a InitializationScaffold,
    },
    /// Derive one complete profile-selection application over an existing
    /// repository. The collection is dependency-first and unique; selector
    /// occurrence handling remains at the command boundary.
    ApplyProfileSelection {
        /// Parsed package metadata and neutral canonical claims for the whole
        /// selected closure.
        profiles: &'a [ProfileApplicationInput],
        /// Requested lifecycle operation recorded when this selection changes
        /// repository-owned state.
        operation: crate::domain::ProfileLifecycleOperation,
    },
}

/// Invoke the constrained closed producer graph for one typed request;
/// `context` stamps the events the plan publishes.
///
/// The direct request match is deliberately closed: callers cannot register
/// callbacks or choose individual producer families. Adding a family requires
/// extending this function.
pub fn derive_materialization(
    image: &RepositoryImage,
    context: &MutationContext,
    request: MaterializationRequest<'_>,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let (derivation, projection_counts) = match request {
        MaterializationRequest::SemanticMutation { declarations } => {
            let delta = derive_semantic_mutation(image, &declarations)?;
            (MaterializationDerivation::new(delta), Default::default())
        }
        MaterializationRequest::RenderConfiguredProjections {
            declarations,
            selected,
        } => {
            let (delta, counts) = derive_project_render(image, &declarations, selected.as_ref())?;
            (MaterializationDerivation::new(delta), counts)
        }
        MaterializationRequest::RepairDerivedState { declarations } => {
            let delta = derive_repair(image, &declarations)?;
            (MaterializationDerivation::new(delta), Default::default())
        }
        MaterializationRequest::Initialize { scaffold } => (
            initialize::derive_initialization(image, scaffold, context)?,
            Default::default(),
        ),
        MaterializationRequest::ApplyProfileSelection {
            profiles,
            operation,
        } => (
            initialize::derive_profile_applications(
                image,
                profiles,
                context,
                operation,
                std::collections::BTreeSet::new(),
            )?,
            Default::default(),
        ),
    };
    let MaterializationDerivation {
        delta,
        profile_targets,
        profile_contributions,
        applied_profiles,
    } = derivation;
    Ok(MaterializationPlan::new(image, delta, context)?
        .with_projection_counts(projection_counts)
        .with_profile_targets(profile_targets)
        .with_profile_contributions(profile_contributions)
        .with_applied_profiles(applied_profiles))
}

/// Finalize an authored `config.toml` edit plus the complete coupled producer set
/// as one plan — the single-declaration config-mutation session entry.
///
/// The command supplies the edited, preservation-safe `config.toml` bytes; this
/// overlays them onto the captured `base` and assembles the edited configuration
/// first, so an edit that fails assembly is a typed planning error and never a
/// published file. It then derives the complete owned-materialization set —
/// default rules and their schemas, plus every configured projection — from that
/// edited configuration, with `declarations` supplying the gates and rules,
/// exactly as `derive_semantic_mutation` does. The
/// authored `config.toml` write, emitted only when the edit changes its bytes,
/// carries the captured base preimage, and every
/// derived action its own base preimage, so `session.apply` revalidates against
/// exactly what was captured.
///
/// This is a constrained, single-declaration entry invoking the closed complete
/// producer set — `config.toml` only, never a generic authored-bytes seam. Other
/// authored-declaration edits adopt this same pattern per caller.
pub fn finalize_config_edit(
    base: &RepositoryImage,
    context: &MutationContext,
    edited_config_bytes: &[u8],
    declarations: RepositoryDeclarations<'_>,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let config_path = VirtualPath::CONFIG;
    let overlay = std::iter::once((config_path.clone(), Some(edited_config_bytes.to_vec())))
        .collect::<std::collections::BTreeMap<_, _>>();
    let overlaid = apply_overlay(base, overlay)?;
    // The edit must assemble before any planning: an invalid config is a typed
    // planning error, not a published file.
    let edited_config = materialize::assemble_config(&overlaid)?;

    // The authored config write carries the base preimage; the complete producer
    // set derives the coupled schemas/rule-membership/projections from the edited
    // configuration over the overlaid image (their targets keep their base
    // preimages, since only `config.toml` is overlaid).
    // An unchanged edit writes nothing, so a no-op publishes no record.
    let base_entry = base.entry(&config_path).map_err(ProducerError::from)?;
    let unchanged = base
        .file_bytes(&config_path)
        .map_err(ProducerError::from)?
        .is_some_and(|bytes| bytes == edited_config_bytes);
    let mut actions = (!unchanged)
        .then(|| RepositoryAction::WriteFile {
            path: config_path.clone(),
            owner: RepositoryActionOwner::AuthoredConfig,
            expected: ExpectedPreimage::of(base_entry),
            bytes: edited_config_bytes.to_vec(),
            mode: FileMode::Regular,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let edited_declarations = RepositoryDeclarations {
        item_kinds: resolved_item_kinds(&edited_config),
        config: &edited_config,
        gates: declarations.gates,
        rules: declarations.rules,
    };
    actions.extend(compose_complete(&overlaid, &edited_declarations)?);
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    Ok(MaterializationPlan::new(base, delta, context)?)
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
    /// Managed-document composition rejected an ambiguous or malformed claim.
    #[error(transparent)]
    ManagedDocument(#[from] ManagedDocumentError),
    /// Projection declaration or source resolution failed.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// A profile selection decided targets or declarations it cannot publish.
    #[error(transparent)]
    ProfileTargetConflicts(#[from] ProfileTargetConflictsError),
    /// Layout classification rejected a producer path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// A proposed-state overlay could not be closed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
    /// A producer read an uncaptured path or malformed captured bytes.
    #[error("materialization producer failed: {0}")]
    Producer(#[from] ProducerError),
    /// Composing the plan's event append failed.
    #[error(transparent)]
    Mutation(#[from] MutationError),
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
    /// `rules.toml` declares more than one rule with the same name, so
    /// default-rule and schema ownership cannot be proven.
    #[error(
        "rules.toml declares more than one rule named '{0}'; \
         default-rule and schema ownership cannot be proven"
    )]
    DuplicateRuleName(String),
}

/// The complete owned-materialization producer set: default rules and their
/// schemas, the data root's ignore file, plus every configured projection, each
/// derived from declared authority (`@/inv/single-source-prose`). A caller cannot
/// invoke a subset — the intent selects the whole set. Shared projection targets compose through the one
/// managed-document primitive, never last-writer-wins; `rules.toml` splices only
/// the generated default-family spans, preserving authored content unconditionally.
fn compose_complete(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<Vec<RepositoryAction>, RepositoryStateError> {
    let mut actions = materialize::compose_default_ruleset(image, declarations.config)?;
    actions.extend(materialize::compose_data_root_ignore(image)?);
    actions.extend(
        // A semantic mutation is complete over EVERY declared projection.
        materialize::compose_configured_projections(image, declarations, None)?.actions,
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
    let rendered = materialize::compose_configured_projections(image, declarations, selected)?;
    Ok((
        RepositoryDelta::new(image.layout(), rendered.actions)?,
        rendered.counts,
    ))
}

/// Repair intent: the semantic-mutation expected state, whose per-target composition is
/// itself ownership-safe — `rules.toml` splices only generated default spans,
/// region projections splice only their managed region, and full-file projections
/// replace a target the declaration proves. Ambiguous ownership fails before
/// publication rather than rewriting an authored boundary.
fn derive_repair(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    derive_semantic_mutation(image, declarations)
}

/// Enumerate the drift-independent set of paths `derive_repair` may materialize.
///
/// The returned set names every target repair could write, independent of whether
/// `image` currently drifts from it — unlike `derive_repair`, which is drift-
/// filtered at every family (a projection skips when its composed bytes already
/// match the captured occupant, the default ruleset writes only on diff). Walking `derive_repair`'s own
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
/// - The data root's ignore file: a member exactly when the capture spec
///   captured it, mirroring the containment guard its producer applies.
/// - Configured-projection targets: every projection declared in `declarations`'
///   configuration contributes its own target unconditionally, since a declared
///   projection's target is always part of the operation's capture closure.
///
/// Obsolete-schema deletions are deliberately excluded: they are occupant-proven
/// repair actions (a captured schema file whose sole default-rule reference is
/// gone), not declared targets, so naming them here would require reading occupant
/// state this function does not otherwise need.
///
/// `derive_repair` is not refactored to consume this set — it needs each target's
/// composed bytes, which a path set cannot supply — so a caller needing both the
/// target set and the composed delta invokes this function and `derive_repair`
/// separately over the same `image` and `declarations`.
pub fn repair_target_paths(
    image: &RepositoryImage,
    declarations: RepositoryDeclarations<'_>,
) -> Result<std::collections::BTreeSet<VirtualPath>, RepositoryStateError> {
    let config = declarations.config;
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
        for schema in materialize::serialized_default_ruleset(config)?.schema_files {
            let vpath = VirtualPath::schema(&schema.name)?;
            if image.capture_spec().contains_path(&vpath) {
                targets.insert(vpath);
            }
        }
    }

    // Data-root ignore family: the one target, guarded by capture-spec
    // containment exactly as its producer guards it.
    if image
        .capture_spec()
        .contains_path(&VirtualPath::DATA_IGNORE)
    {
        targets.insert(VirtualPath::DATA_IGNORE);
    }

    // Configured-projection targets: every declared projection's own target,
    // computed purely from declared configuration.
    if let Some(projections) = config.projection.as_ref() {
        for (name, projection) in projections {
            let target = require_target(projection, name)?;
            targets.insert(image.layout().classify_repository_relative(target)?);
        }
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
            let path = action.path();
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
                        content: actual,
                        mode: actual_mode,
                    },
                ) if actual.bytes() == bytes.as_slice() && actual_mode == mode => None,
                (RepositoryAction::WriteFile { .. }, _) => Some(MaterializationDriftKind::Stale),
                (RepositoryAction::AppendEvents { .. }, _) => Some(MaterializationDriftKind::Stale),
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
    use crate::test_harness::image::{synthetic_layout, ImageBuilder};

    /// An image declaring `rules` and the schema `entry`, every other
    /// declaration absent.
    fn rules_image(rules: &str, schema: &str, entry: RepositoryEntry) -> RepositoryImage {
        VirtualPath::DECLARATION_FILES
            .into_iter()
            .fold(ImageBuilder::new(), |builder, path| {
                builder.entry(path, RepositoryEntry::Absent)
            })
            .file(".jit/rules.toml", rules)
            .entry(schema, entry)
            .build()
    }

    #[test]
    fn test_deferred_declarations_get_returns_one_parse_from_every_call() {
        let deferred = |gates: &str| {
            let image = ImageBuilder::new()
                .files(&[
                    (".jit/config.toml", Some("")),
                    (".jit/gates.toml", Some(gates)),
                    (".jit/rules.toml", None),
                ])
                .build();
            let config = assemble_config(&image);
            (
                DeferredDeclarations::new(declarations_with_config, Some(config)),
                image,
            )
        };

        let (parsed, image) = deferred("");
        assert!(std::ptr::eq(
            parsed.get(&image).unwrap(),
            parsed.get(&image).unwrap()
        ));

        let (unparseable, image) = deferred("[[gates]\n");
        let first = unparseable.get(&image).err().unwrap();
        let second = unparseable.get(&image).err().unwrap();
        assert!(matches!(*first, DeclarationParseError::Gate(_)));
        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn test_declarations_from_image_rejects_a_malformed_rules_file_rather_than_yielding_no_rules() {
        let image = rules_image(
            "[[rules]]\nname = \"broken\"\nassert = {}\n",
            ".jit/schemas/unused.json",
            RepositoryEntry::Absent,
        );
        assert!(matches!(
            declarations_from_image(&image),
            Err(DeclarationParseError::Rule(_))
        ));
    }

    #[test]
    fn test_declarations_from_image_rejects_an_absent_required_schema() {
        let image = rules_image(
            "[[rules]]\nname = \"body\"\nassert = { json-schema = \"schemas/body.json\" }\n",
            ".jit/schemas/body.json",
            RepositoryEntry::Absent,
        );
        assert!(matches!(
            declarations_from_image(&image),
            Err(DeclarationParseError::MissingCapture(reference)) if reference == "schemas/body.json"
        ));
    }

    #[test]
    fn test_declarations_from_image_treats_a_directory_at_an_optional_schema_path_as_absent() {
        let reference = "schemas/default-namespace-registry.json";
        let rules = format!(
            "[[rules]]\nname = \"namespace-registry\"\norigin = \"default\"\n\
             assert = {{ json-schema = \"{reference}\" }}\n"
        );
        let parse = |entry| {
            declarations_from_image(&rules_image(&rules, &format!(".jit/{reference}"), entry))
                .unwrap()
                .rules()
                .clone()
        };
        assert_eq!(
            parse(RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("dir", b"dir").unwrap(),
                mode: FileMode::Regular,
            }),
            parse(RepositoryEntry::Absent)
        );
    }

    fn plan(image: &RepositoryImage, actions: Vec<RepositoryAction>) -> MaterializationPlan {
        MaterializationPlan::new(
            image,
            RepositoryDelta::new(image.layout(), actions).unwrap(),
            &MutationContext::preview(),
        )
        .unwrap()
    }

    #[test]
    fn test_compare_materializations_propagates_undiscovered_for_every_action() {
        let image = ImageBuilder::new().build();
        let target = VirtualPath::data("unrequested.json").unwrap();
        let occupant = EntryIdentity::for_bytes("occupant", b"captured").unwrap();
        let actions = [
            RepositoryAction::CreateDirectory {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryInit,
                expected: ExpectedPreimage::Absent,
            },
            RepositoryAction::WriteFile {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
                bytes: b"expected".to_vec(),
                mode: FileMode::Regular,
            },
            RepositoryAction::SetMode {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::File {
                    identity: occupant.clone(),
                    mode: FileMode::Regular,
                },
                mode: FileMode::Executable,
            },
            RepositoryAction::DeleteFile {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
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
        let create = VirtualPath::data("create").unwrap();
        let delete = VirtualPath::data("delete").unwrap();
        let set_mode = VirtualPath::data("set-mode").unwrap();
        let write = VirtualPath::data("write").unwrap();
        let delete_content = CapturedBytes::new("delete", b"delete me".to_vec()).unwrap();
        let mode_content = CapturedBytes::new("mode", b"mode".to_vec()).unwrap();
        let delete_identity = delete_content.identity().clone();
        let mode_identity = mode_content.identity().clone();
        let image = ImageBuilder::new()
            .entry(create.clone(), RepositoryEntry::Absent)
            .entry(
                delete.clone(),
                RepositoryEntry::File {
                    content: delete_content,
                    mode: FileMode::Regular,
                },
            )
            .entry(
                set_mode.clone(),
                RepositoryEntry::File {
                    content: mode_content,
                    mode: FileMode::Regular,
                },
            )
            .file(write.clone(), b"old")
            .build();
        let expected = plan(
            &image,
            vec![
                RepositoryAction::CreateDirectory {
                    path: create.clone(),
                    owner: RepositoryActionOwner::RepositoryInit,
                    expected: ExpectedPreimage::Absent,
                },
                RepositoryAction::DeleteFile {
                    path: delete.clone(),
                    owner: RepositoryActionOwner::RepositoryStateMutation,
                    expected: ExpectedPreimage::File {
                        identity: delete_identity,
                        mode: FileMode::Regular,
                    },
                },
                RepositoryAction::SetMode {
                    path: set_mode.clone(),
                    owner: RepositoryActionOwner::RepositoryStateMutation,
                    expected: ExpectedPreimage::File {
                        identity: mode_identity,
                        mode: FileMode::Regular,
                    },
                    mode: FileMode::Executable,
                },
                RepositoryAction::WriteFile {
                    path: write.clone(),
                    owner: RepositoryActionOwner::RepositoryStateMutation,
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

    fn empty_declarations() -> (crate::config::JitConfig, GateRegistry, RuleSet) {
        (
            crate::declarations::assemble_configuration(None, None, None).unwrap(),
            GateRegistry::default(),
            RuleSet { rules: Vec::new() },
        )
    }

    fn gate_definition(key: &str) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            version: 1,
            key: key.parse().unwrap(),
            title: key.to_string(),
            description: String::new(),
            stage: crate::declarations::GateStage::Postcheck,
            mode: crate::declarations::GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            inputs: None,
        }
    }

    #[test]
    fn test_finalize_gate_registry_edit_missing_edit_intent_is_typed() {
        let image = ImageBuilder::new().build();
        let context = MutationContext::preview();
        let (config, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            item_kinds: Ok(std::sync::Arc::from([])),
            config: &config,
            gates: &gates,
            rules: &rules,
        };
        let error =
            finalize_gate_registry_edit(&synthetic_layout(), &image, &context, &[], declarations)
                .unwrap_err();
        assert!(matches!(
            error,
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::MissingEdit)
        ));
    }

    #[test]
    fn test_finalize_gate_registry_edit_multiple_edit_intents_is_typed() {
        let image = ImageBuilder::new().build();
        let context = MutationContext::preview();
        let (config, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            item_kinds: Ok(std::sync::Arc::from([])),
            config: &config,
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
        let error = finalize_gate_registry_edit(
            &synthetic_layout(),
            &image,
            &context,
            &intents,
            declarations,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::MultipleEdits)
        ));
    }

    #[test]
    fn test_finalize_gate_registry_edit_declaration_mismatch_is_typed() {
        let image = ImageBuilder::new().build();
        let context = MutationContext::preview();
        let (config, _default_gates, rules) = empty_declarations();
        let mut mismatched = GateRegistry::default();
        mismatched
            .gates
            .insert("distinct".parse().unwrap(), gate_definition("distinct"));
        let declarations = RepositoryDeclarations {
            item_kinds: Ok(std::sync::Arc::from([])),
            config: &config,
            gates: &mismatched,
            rules: &rules,
        };
        let intents = vec![MutationIntent::EditGateRegistry {
            registry: Box::new(GateRegistry::default()),
        }];
        let error = finalize_gate_registry_edit(
            &synthetic_layout(),
            &image,
            &context,
            &intents,
            declarations,
        )
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
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&crate::test_utils::issue_id(75)),
                RepositoryEntry::Absent,
            )
            .build();
        let context = MutationContext::preview();
        let (config, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            item_kinds: Ok(std::sync::Arc::from([])),
            config: &config,
            gates: &gates,
            rules: &rules,
        };
        let intents = vec![
            MutationIntent::EditGateRegistry {
                registry: Box::new(GateRegistry::default()),
            },
            MutationIntent::ClaimIssue {
                issue_id: crate::test_utils::issue_id(75),
                agent: "agent:tester".parse().unwrap(),
            },
        ];
        let error = finalize_gate_registry_edit(
            &synthetic_layout(),
            &image,
            &context,
            &intents,
            declarations,
        )
        .unwrap_err();
        match error {
            RepositoryStateError::GateRegistryEdit(GateRegistryEditError::Mutation(
                MutationError::MissingIssue(id),
            )) => assert_eq!(id, crate::test_utils::issue_id(75)),
            other => panic!("expected a typed missing-issue mutation error, got {other:?}"),
        }
    }

    /// An image whose only present declaration is `gates`, when given.
    fn gates_image(gates: Option<&str>) -> RepositoryImage {
        let builder = VirtualPath::DECLARATION_FILES
            .into_iter()
            .fold(ImageBuilder::new(), |builder, path| {
                builder.entry(path, RepositoryEntry::Absent)
            });
        match gates {
            Some(gates) => builder.file(".jit/gates.toml", gates),
            None => builder,
        }
        .build()
    }

    #[test]
    fn test_declarations_from_image_without_a_gate_registry_declares_no_gates() {
        let declarations = declarations_from_image(&gates_image(None)).unwrap();

        assert!(declarations.gates().gates.is_empty());
    }

    /// The key is the registry identity, so keeping either row would discard
    /// the other.
    #[test]
    fn test_declarations_from_image_rejects_duplicate_gate_keys_naming_the_key() {
        let row = "[[gates]]\nkey = \"review\"\ntitle = \"Review\"\n\
                   description = \"Review\"\nstage = \"postcheck\"\nmode = \"manual\"\n";

        assert!(matches!(
            declarations_from_image(&gates_image(Some(&format!("{row}\n{row}")))),
            Err(DeclarationParseError::Gate(
                crate::declarations::GateDeclarationError::DuplicateKey(key)
            )) if key.as_str() == "review"
        ));
    }

    const MALFORMED_ITEM_KINDS: &str = "[item_kinds.custom]\nsection = \"Items\"\n";

    #[test]
    fn test_validation_declarations_with_config_assembles_from_declaration_files_and_index() {
        let paths = VirtualPath::DECLARATION_FILES
            .into_iter()
            .chain([VirtualPath::INDEX])
            .collect::<Vec<_>>();
        let budget = CaptureBudget {
            max_listings: 0,
            max_bytes: u64::MAX,
            max_depth: usize::MAX,
        };
        let image = paths
            .iter()
            .fold(ImageBuilder::new(), |builder, path| {
                builder.file(path.clone(), "")
            })
            .with_spec(CaptureSpec::new(paths, budget).unwrap())
            .build();

        validation_declarations_with_config(&image, assemble_config(&image)).unwrap();
    }

    #[test]
    fn test_validation_declarations_with_config_rejects_malformed_item_kinds() {
        let image = ImageBuilder::new()
            .files(&[
                (".jit/config.toml", Some(MALFORMED_ITEM_KINDS)),
                (".jit/gates.toml", None),
                (".jit/rules.toml", None),
            ])
            .build();
        assert!(matches!(
            validation_declarations_with_config(&image, assemble_config(&image)),
            Err(DeclarationParseError::Producer(
                ProducerError::Configuration(ConfigurationAssemblyError::ItemKinds(_))
            ))
        ));
    }

    #[test]
    fn test_finalize_config_edit_refuses_edit_that_fails_assembly() {
        let base = ImageBuilder::new()
            .files(&[(".jit/config.toml", Some(""))])
            .build();
        let (config, gates, rules) = empty_declarations();
        let declarations = RepositoryDeclarations {
            item_kinds: Ok(std::sync::Arc::from([])),
            config: &config,
            gates: &gates,
            rules: &rules,
        };
        assert!(matches!(
            finalize_config_edit(
                &base,
                &MutationContext::preview(),
                MALFORMED_ITEM_KINDS.as_bytes(),
                declarations
            ),
            Err(RepositoryStateError::Producer(
                ProducerError::Configuration(ConfigurationAssemblyError::ItemKinds(_))
            ))
        ));
    }

    fn write(path: &VirtualPath, bytes: &[u8]) -> RepositoryAction {
        RepositoryAction::WriteFile {
            path: path.clone(),
            owner: RepositoryActionOwner::RepositoryStateMutation,
            expected: ExpectedPreimage::Absent,
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    /// The events a publication appends, decoded.
    fn published_events(plan: &MaterializationPlan) -> Vec<crate::domain::Event> {
        plan.publication()
            .actions()
            .iter()
            .filter_map(|action| match action {
                RepositoryAction::AppendEvents { bytes, .. } => Some(bytes),
                _ => None,
            })
            .flat_map(|bytes| {
                bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| serde_json::from_slice(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn test_materialization_plan_new_publishes_an_event_less_write_with_one_files_published_event()
    {
        let (config, note) = (
            VirtualPath::CONFIG,
            VirtualPath::worktree("note.md").unwrap(),
        );
        let image = ImageBuilder::new()
            .file(config.clone(), b"old")
            .entry(note.clone(), RepositoryEntry::Absent)
            .build();
        let delta = RepositoryDelta::new(
            image.layout(),
            vec![write(&note, b"note"), write(&config, b"new")],
        )
        .unwrap();

        let plan =
            MaterializationPlan::new(&image, delta.clone(), &MutationContext::preview()).unwrap();

        assert_eq!(
            plan.delta(),
            &delta,
            "reports keep reading the derived delta"
        );
        let events = published_events(&plan);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].body(),
            &crate::domain::EventBody::draft_files_published(vec![
                config.repository_relative(),
                note.repository_relative(),
            ])
        );
        assert_eq!(
            plan.publication().file_writes().collect::<Vec<_>>(),
            delta.file_writes().collect::<Vec<_>>(),
            "the publication adds only the append"
        );
    }

    #[test]
    fn test_materialization_plan_new_rebuilds_the_same_publication_from_a_reused_context() {
        let config = VirtualPath::CONFIG;
        let image = ImageBuilder::new().file(config.clone(), b"old").build();
        let delta = RepositoryDelta::new(image.layout(), vec![write(&config, b"new")]).unwrap();
        let context = MutationContext::deterministic(
            [7; 32],
            chrono::DateTime::from_timestamp(1, 0).unwrap(),
        );

        let first = MaterializationPlan::new(&image, delta.clone(), &context).unwrap();
        let retried = MaterializationPlan::new(&image, delta, &context).unwrap();

        assert_eq!(first.publication(), retried.publication());
    }

    #[test]
    fn test_materialization_plan_new_publishes_the_derived_delta_when_no_generic_event_is_due() {
        let config = VirtualPath::CONFIG;
        let present = ImageBuilder::new().file(config.clone(), b"old").build();
        let absent_root = ImageBuilder::new()
            .entry(config.clone(), RepositoryEntry::Absent)
            .build();
        let appending = finalize(
            present.layout(),
            &present,
            &MutationContext::preview(),
            &[MutationIntent::RecordEvent {
                phase: 0,
                event: Box::new(crate::domain::EventBody::draft_files_published(Vec::new())),
            }],
            None,
        )
        .unwrap()
        .delta()
        .clone();
        let cases = [
            (
                "empty delta",
                &present,
                RepositoryDelta::new(present.layout(), Vec::new()),
            ),
            ("domain event", &present, Ok(appending)),
            (
                "absent data root",
                &absent_root,
                RepositoryDelta::new(absent_root.layout(), vec![write(&config, b"new")]),
            ),
        ];

        for (case, image, delta) in cases {
            let delta = delta.unwrap();
            let plan = MaterializationPlan::new(image, delta.clone(), &MutationContext::preview())
                .unwrap();
            assert_eq!(plan.publication(), &delta, "{case}");
        }
    }

    #[test]
    fn test_materialization_plan_new_refuses_a_generic_event_over_an_image_without_the_event_log() {
        let config = VirtualPath::CONFIG;
        let spec = CaptureSpec::new(
            [config.clone()],
            CaptureBudget {
                max_listings: 0,
                max_bytes: u64::MAX,
                max_depth: usize::MAX,
            },
        )
        .unwrap();
        let image = ImageBuilder::new()
            .with_spec(spec)
            .file(config.clone(), b"old")
            .build();
        let delta = RepositoryDelta::new(image.layout(), vec![write(&config, b"new")]).unwrap();

        assert!(matches!(
            MaterializationPlan::new(&image, delta, &MutationContext::preview()),
            Err(MutationError::Capture(CaptureError::UndiscoveredRepositoryPath(path)))
                if path == VirtualPath::EVENTS
        ));
    }
}
