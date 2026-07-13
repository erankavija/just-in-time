//! Pure, versioned domain model for dependency-aware artifact archival plans.
//!
//! Planning code builds one [`ArtifactPlan`]. Preview surfaces serialize that
//! value directly, and execution reads the same value through
//! [`ArtifactPlan::executable_artifacts`]. Keeping ordering and eligibility in
//! this module prevents either consumer from growing a second classification
//! model.

use crate::config::DocumentationConfig;
use crate::domain::State;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fmt;

/// The first archive-plan JSON schema version.
///
/// Fields and vocabulary associated with a published version are append-only;
/// changing an existing field or code requires a new schema version.
pub const ARCHIVE_PLAN_SCHEMA_VERSION: u32 = 1;

/// A container or one arbitrary document selected for archival planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PlanTarget {
    /// A resolved-hierarchy container, identified by its full durable id.
    Container { id: String },
    /// An arbitrary repository-relative document path.
    Document { path: String },
}

/// Whether the mutation-authorizing documentation policy is explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyStatus {
    /// All required archival policy fields were explicitly supplied.
    Configured,
    /// A `[documentation]` table exists but lacks at least one required field.
    Incomplete,
    /// No `[documentation]` table exists.
    Unconfigured,
}

impl PolicyStatus {
    /// Classify policy by authored fields, without consulting accessor defaults.
    pub fn from_documentation(documentation: Option<&DocumentationConfig>) -> Self {
        match documentation {
            None => Self::Unconfigured,
            Some(config)
                if config.managed_paths.is_some()
                    && config.permanent_paths.is_some()
                    && config.archive_root.is_some() =>
            {
                Self::Configured
            }
            Some(_) => Self::Incomplete,
        }
    }
}

/// The version component of an artifact's `(normalized path, version)` identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtifactVersion {
    /// Content read from the current working tree.
    WorkingTree,
    /// Historical content at one canonical full immutable commit OID.
    Pinned(CanonicalCommitOid),
}

/// A canonical full lowercase commit OID accepted from the storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalCommitOid(String);

impl CanonicalCommitOid {
    /// The canonical full OID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ArtifactVersion {
    /// Construct a historical version from a canonical full commit OID.
    ///
    /// Full lowercase OIDs from Git's SHA-1 and SHA-256 object formats are
    /// accepted. Symbolic names, abbreviated hashes, and non-canonical
    /// uppercase hashes are rejected. Revision resolution belongs at the
    /// storage boundary; only its canonical result crosses into the plan
    /// domain.
    pub fn pinned(oid: impl Into<String>) -> Result<Self, PlanError> {
        let oid = oid.into();
        if matches!(oid.len(), 40 | 64)
            && oid
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self::Pinned(CanonicalCommitOid(oid)))
        } else {
            Err(PlanError::NonCanonicalCommitOid(oid))
        }
    }

    /// Return the exact string used in JSON and ordering.
    pub fn as_str(&self) -> &str {
        match self {
            Self::WorkingTree => "working-tree",
            Self::Pinned(oid) => oid.as_str(),
        }
    }

    /// Whether this is a non-relocating historical version.
    pub fn is_pinned(&self) -> bool {
        matches!(self, Self::Pinned(_))
    }
}

impl Ord for ArtifactVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for ArtifactVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for ArtifactVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArtifactVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "working-tree" {
            Ok(Self::WorkingTree)
        } else {
            Self::pinned(value).map_err(serde::de::Error::custom)
        }
    }
}

/// Stable identity used to compare and de-duplicate artifact versions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactIdentity {
    source: String,
    version: ArtifactVersion,
}

impl ArtifactIdentity {
    /// Normalized repository-relative source path.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Working-tree or canonical pinned version.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }
}

/// Bytes identity captured from a single read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentIdentity {
    sha256: String,
    byte_size: u64,
}

impl ContentIdentity {
    /// Hash and size one already-read byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self {
            sha256: format!("{:x}", hasher.finalize()),
            byte_size: bytes.len() as u64,
        }
    }

    /// Reconstruct a recorded identity, validating its stable wire shape.
    pub fn try_new(sha256: impl Into<String>, byte_size: u64) -> Result<Self, PlanError> {
        let sha256 = sha256.into();
        if sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self { sha256, byte_size })
        } else {
            Err(PlanError::InvalidContentHash(sha256))
        }
    }

    /// Canonical lowercase SHA-256 digest.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Exact number of bytes hashed.
    pub fn byte_size(&self) -> u64 {
        self.byte_size
    }
}

impl<'de> Deserialize<'de> for ContentIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireIdentity {
            sha256: String,
            byte_size: u64,
        }

        let wire = WireIdentity::deserialize(deserializer)?;
        Self::try_new(wire.sha256, wire.byte_size).map_err(serde::de::Error::custom)
    }
}

/// One archival action selected by the pure classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactAction {
    Move,
    Copy,
    Retain,
    Block,
}

/// How an artifact entered the selected closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactProvenance {
    Explicit,
    Embedded,
}

/// One issue document reference that owns an artifact version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOwner {
    /// Full durable issue id.
    pub issue: String,
    /// Stable index of the reference within the issue's document list.
    pub document_index: usize,
    /// Lifecycle state used for terminal-owner classification.
    pub state: State,
    /// Whether the owner belongs to the selected resolved subtree.
    pub inside_subtree: bool,
    /// Whether this particular direct reference is commit-pinned.
    pub pinned: bool,
    /// Whether execution will durably relink this direct reference.
    pub selected_for_relink: bool,
}

/// Planner classification for an embedded edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Supported,
    Unsupported,
    External,
}

/// How an embedded reference resolves from its parent artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeResolutionMode {
    Relative,
    RootRelative,
    External,
}

/// One embedded reference observed while constructing the artifact graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactEdge {
    /// Reference text as it appeared in the parent artifact.
    pub reference: String,
    /// Normalized local target when resolution produced one.
    pub target: Option<String>,
    /// Supported, unsupported, or external classification.
    pub kind: EdgeKind,
    /// Base against which the reference resolves.
    pub resolution_mode: EdgeResolutionMode,
}

/// One durable issue-document relink selected by the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceChange {
    /// Full durable issue id.
    pub issue: String,
    /// Stable index of the reference within the issue's document list.
    pub document_index: usize,
    /// Existing normalized reference path.
    pub from_path: String,
    /// New normalized mirror path.
    pub to_path: String,
}

/// A source removal guarded by a previously recorded byte identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDeletion {
    /// Normalized source path to remove after durable relinking and event append.
    pub source: String,
    /// Identity that must still match immediately before removal.
    pub content_identity: ContentIdentity,
}

/// Stable evidence flags. These constrain action selection but never block it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceCode {
    PermanentPath,
    OutsideOwner,
    ActiveOwner,
    UnmanagedPath,
    ArchivedSource,
    PinnedHistorical,
}

impl EvidenceCode {
    /// Complete schema-v1 evidence vocabulary, in binding contract order.
    pub const ALL: [Self; 6] = [
        Self::PermanentPath,
        Self::OutsideOwner,
        Self::ActiveOwner,
        Self::UnmanagedPath,
        Self::ArchivedSource,
        Self::PinnedHistorical,
    ];

    /// Stable kebab-case wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermanentPath => "permanent-path",
            Self::OutsideOwner => "outside-owner",
            Self::ActiveOwner => "active-owner",
            Self::UnmanagedPath => "unmanaged-path",
            Self::ArchivedSource => "archived-source",
            Self::PinnedHistorical => "pinned-historical",
        }
    }
}

/// Stable conditions that make a target or artifact ineligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockerCode {
    PolicyUnconfigured,
    PolicyIncomplete,
    UnmanagedSelectedRoot,
    DestinationConflict,
    DocumentNonTerminalOwner,
    PinnedReadFailed,
    RepositoryEscape,
    UnresolvableEdge,
    UnpreservableLayout,
    NonTerminalTarget,
    MissingSource,
    SymlinkArtifact,
}

impl BlockerCode {
    /// Complete schema-v1 blocker vocabulary, in binding contract order.
    pub const ALL: [Self; 12] = [
        Self::PolicyUnconfigured,
        Self::PolicyIncomplete,
        Self::UnmanagedSelectedRoot,
        Self::DestinationConflict,
        Self::DocumentNonTerminalOwner,
        Self::PinnedReadFailed,
        Self::RepositoryEscape,
        Self::UnresolvableEdge,
        Self::UnpreservableLayout,
        Self::NonTerminalTarget,
        Self::MissingSource,
        Self::SymlinkArtifact,
    ];

    /// Stable kebab-case wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PolicyUnconfigured => "policy-unconfigured",
            Self::PolicyIncomplete => "policy-incomplete",
            Self::UnmanagedSelectedRoot => "unmanaged-selected-root",
            Self::DestinationConflict => "destination-conflict",
            Self::DocumentNonTerminalOwner => "document-non-terminal-owner",
            Self::PinnedReadFailed => "pinned-read-failed",
            Self::RepositoryEscape => "repository-escape",
            Self::UnresolvableEdge => "unresolvable-edge",
            Self::UnpreservableLayout => "unpreservable-layout",
            Self::NonTerminalTarget => "non-terminal-target",
            Self::MissingSource => "missing-source",
            Self::SymlinkArtifact => "symlink-artifact",
        }
    }
}

/// Stable non-blocking diagnostics emitted by planning or execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WarningCode {
    MissingEdgeTarget,
    ExternalEdge,
    NoOwner,
    ResidueSource,
    DeletionFailed,
    NotSelectedSibling,
    DynamicLoadingSuspected,
}

impl WarningCode {
    /// Complete schema-v1 warning vocabulary, in binding contract order.
    pub const ALL: [Self; 7] = [
        Self::MissingEdgeTarget,
        Self::ExternalEdge,
        Self::NoOwner,
        Self::ResidueSource,
        Self::DeletionFailed,
        Self::NotSelectedSibling,
        Self::DynamicLoadingSuspected,
    ];

    /// Stable kebab-case wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingEdgeTarget => "missing-edge-target",
            Self::ExternalEdge => "external-edge",
            Self::NoOwner => "no-owner",
            Self::ResidueSource => "residue-source",
            Self::DeletionFailed => "deletion-failed",
            Self::NotSelectedSibling => "not-selected-sibling",
            Self::DynamicLoadingSuspected => "dynamic-loading-suspected",
        }
    }
}

/// A target- or artifact-level blocker with an optional normalized path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBlocker {
    /// Stable schema-v1 blocker code.
    pub code: BlockerCode,
    /// Normalized affected path, or `None` for a target-wide condition.
    pub path: Option<String>,
}

impl PlanBlocker {
    /// Construct a blocker and normalize its optional path.
    pub fn new(code: BlockerCode, path: Option<impl Into<String>>) -> Self {
        Self {
            code,
            path: path.map(Into::into).map(|path| normalize_path(&path)),
        }
    }
}

/// A target- or artifact-level warning with an optional normalized path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanWarning {
    /// Stable schema-v1 warning code.
    pub code: WarningCode,
    /// Normalized affected path, or `None` for a target-wide condition.
    pub path: Option<String>,
}

impl PlanWarning {
    /// Construct a warning and normalize its optional path.
    pub fn new(code: WarningCode, path: Option<impl Into<String>>) -> Self {
        Self {
            code,
            path: path.map(Into::into).map(|path| normalize_path(&path)),
        }
    }
}

/// One artifact version and all pure classification data needed by consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPlanEntry {
    source: String,
    version: ArtifactVersion,
    content_identity: Option<ContentIdentity>,
    destination: Option<String>,
    action: ArtifactAction,
    already_archived: bool,
    provenance: Vec<ArtifactProvenance>,
    format: Option<String>,
    owners: Vec<ArtifactOwner>,
    edges: Vec<ArtifactEdge>,
    reference_changes: Vec<ReferenceChange>,
    pending_deletions: Vec<PendingDeletion>,
    evidence: Vec<EvidenceCode>,
    blockers: Vec<PlanBlocker>,
    warnings: Vec<PlanWarning>,
}

impl ArtifactPlanEntry {
    /// Start an entry with all optional classification collections empty.
    pub fn new(
        source: impl Into<String>,
        version: ArtifactVersion,
        action: ArtifactAction,
    ) -> Self {
        let source = source.into();
        Self {
            source: normalize_path(&source),
            version,
            content_identity: None,
            destination: None,
            action,
            already_archived: false,
            provenance: Vec::new(),
            format: None,
            owners: Vec::new(),
            edges: Vec::new(),
            reference_changes: Vec::new(),
            pending_deletions: Vec::new(),
            evidence: Vec::new(),
            blockers: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Record the identity captured from the source read used for planning.
    pub fn with_content_identity(mut self, identity: ContentIdentity) -> Self {
        self.content_identity = Some(identity);
        self
    }

    /// Set and normalize the mirror destination.
    pub fn with_destination(mut self, destination: impl Into<String>) -> Self {
        let destination = destination.into();
        self.destination = Some(normalize_path(&destination));
        self
    }

    /// Record adoption of content-identical bytes already at the destination.
    pub fn with_already_archived(mut self, already_archived: bool) -> Self {
        self.already_archived = already_archived;
        self
    }

    /// Supply explicit/embedded provenance flags; plan construction canonicalizes them.
    pub fn with_provenance(mut self, provenance: Vec<ArtifactProvenance>) -> Self {
        self.provenance = provenance;
        self
    }

    /// Supply the resolved format name, when known.
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Supply all repository-wide direct owners.
    pub fn with_owners(mut self, owners: Vec<ArtifactOwner>) -> Self {
        self.owners = owners;
        self
    }

    /// Supply embedded edges discovered from this artifact.
    pub fn with_edges(mut self, edges: Vec<ArtifactEdge>) -> Self {
        self.edges = edges;
        self
    }

    /// Supply the exact selected direct-reference relinks.
    pub fn with_reference_changes(mut self, changes: Vec<ReferenceChange>) -> Self {
        self.reference_changes = changes;
        self
    }

    /// Supply identity-guarded source removals.
    pub fn with_pending_deletions(mut self, deletions: Vec<PendingDeletion>) -> Self {
        self.pending_deletions = deletions;
        self
    }

    /// Supply non-blocking evidence flags.
    pub fn with_evidence(mut self, evidence: Vec<EvidenceCode>) -> Self {
        self.evidence = evidence;
        self
    }

    /// Supply artifact-level blockers.
    pub fn with_blockers(mut self, blockers: Vec<PlanBlocker>) -> Self {
        self.blockers = blockers;
        self
    }

    /// Supply artifact-level warnings.
    pub fn with_warnings(mut self, warnings: Vec<PlanWarning>) -> Self {
        self.warnings = warnings;
        self
    }

    /// Return the canonical `(normalized source, version)` identity.
    pub fn identity(&self) -> ArtifactIdentity {
        ArtifactIdentity {
            source: self.source.clone(),
            version: self.version.clone(),
        }
    }

    /// Normalized repository-relative source path.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Working-tree or canonical historical version.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// Purely classified archival action.
    pub fn action(&self) -> ArtifactAction {
        self.action
    }

    /// Byte identity required by publication and deletion operations.
    pub fn content_identity(&self) -> Option<&ContentIdentity> {
        self.content_identity.as_ref()
    }

    /// Mirrored destination, absent for retained and historical entries.
    pub fn destination(&self) -> Option<&str> {
        self.destination.as_deref()
    }

    /// Whether content-identical bytes already occupy the destination.
    pub fn already_archived(&self) -> bool {
        self.already_archived
    }

    /// Canonically ordered explicit/embedded provenance flags.
    pub fn provenance(&self) -> &[ArtifactProvenance] {
        &self.provenance
    }

    /// Resolved format name, when known.
    pub fn format(&self) -> Option<&str> {
        self.format.as_deref()
    }

    /// Canonically ordered direct owners.
    pub fn owners(&self) -> &[ArtifactOwner] {
        &self.owners
    }

    /// Canonically ordered embedded edges.
    pub fn edges(&self) -> &[ArtifactEdge] {
        &self.edges
    }

    /// Canonically ordered selected direct-reference relinks.
    pub fn reference_changes(&self) -> &[ReferenceChange] {
        &self.reference_changes
    }

    /// Canonically ordered identity-guarded source removals.
    pub fn pending_deletions(&self) -> &[PendingDeletion] {
        &self.pending_deletions
    }

    /// Canonically ordered evidence flags.
    pub fn evidence(&self) -> &[EvidenceCode] {
        &self.evidence
    }

    /// Canonically ordered artifact-level blockers.
    pub fn blockers(&self) -> &[PlanBlocker] {
        &self.blockers
    }

    /// Canonically ordered artifact-level warnings.
    pub fn warnings(&self) -> &[PlanWarning] {
        &self.warnings
    }

    fn normalize(&mut self) -> Result<(), PlanError> {
        self.source = normalize_path(&self.source);
        self.destination = self.destination.take().map(|path| normalize_path(&path));
        self.provenance.sort_unstable();
        self.provenance.dedup();
        self.evidence.sort_unstable();
        self.evidence.dedup();
        if self.version.is_pinned() && !self.evidence.contains(&EvidenceCode::PinnedHistorical) {
            self.evidence.push(EvidenceCode::PinnedHistorical);
            self.evidence.sort_unstable();
        }

        self.owners.sort_by(|left, right| {
            (
                &left.issue,
                left.document_index,
                left.state,
                left.inside_subtree,
                left.pinned,
                left.selected_for_relink,
            )
                .cmp(&(
                    &right.issue,
                    right.document_index,
                    right.state,
                    right.inside_subtree,
                    right.pinned,
                    right.selected_for_relink,
                ))
        });
        self.edges.sort_by(|left, right| {
            (
                left.kind,
                left.resolution_mode,
                &left.reference,
                &left.target,
            )
                .cmp(&(
                    right.kind,
                    right.resolution_mode,
                    &right.reference,
                    &right.target,
                ))
        });
        for change in &mut self.reference_changes {
            change.from_path = normalize_path(&change.from_path);
            change.to_path = normalize_path(&change.to_path);
        }
        self.reference_changes.sort_by(|left, right| {
            (
                &left.issue,
                left.document_index,
                &left.from_path,
                &left.to_path,
            )
                .cmp(&(
                    &right.issue,
                    right.document_index,
                    &right.from_path,
                    &right.to_path,
                ))
        });
        for deletion in &mut self.pending_deletions {
            deletion.source = normalize_path(&deletion.source);
        }
        self.pending_deletions
            .sort_by(|left, right| left.source.cmp(&right.source));
        for blocker in &mut self.blockers {
            blocker.path = blocker.path.take().map(|path| normalize_path(&path));
        }
        sort_blockers(&mut self.blockers);
        for warning in &mut self.warnings {
            warning.path = warning.path.take().map(|path| normalize_path(&path));
        }
        sort_warnings(&mut self.warnings);

        if self.version.is_pinned()
            && (self.action != ArtifactAction::Retain
                || self.destination.is_some()
                || !self.reference_changes.is_empty()
                || !self.pending_deletions.is_empty())
        {
            return Err(PlanError::RelocatingPinnedArtifact(self.source.clone()));
        }

        match (self.action, self.destination.is_some()) {
            (ArtifactAction::Move | ArtifactAction::Copy, false) => {
                return Err(PlanError::MissingDestination(self.source.clone()));
            }
            (ArtifactAction::Retain, true) => {
                return Err(PlanError::UnexpectedDestination(self.source.clone()));
            }
            _ => {}
        }

        let publishes = matches!(self.action, ArtifactAction::Move | ArtifactAction::Copy)
            && !self.already_archived;
        let requires_identity =
            !self.version.is_pinned() && (publishes || !self.pending_deletions.is_empty());
        if requires_identity && self.content_identity.is_none() {
            return Err(PlanError::MissingContentIdentity(self.source.clone()));
        }
        if !requires_identity && self.content_identity.is_some() {
            return Err(PlanError::UnexpectedContentIdentity(self.source.clone()));
        }

        Ok(())
    }
}

/// Derived action totals serialized in every plan envelope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCounts {
    /// Artifacts selected for relocation with source removal.
    pub r#move: usize,
    /// Artifacts selected for publication while retaining their source.
    pub copy: usize,
    /// Artifacts that remain only at their source.
    pub retain: usize,
    /// Artifacts whose classification cannot execute.
    pub block: usize,
    /// Artifacts whose destination publication is already satisfied.
    pub already_archived: usize,
    /// Identity-guarded source removals, including residue cleanup.
    pub pending_deletions: usize,
}

impl ActionCounts {
    fn from_artifacts(artifacts: &[ArtifactPlanEntry]) -> Self {
        artifacts
            .iter()
            .fold(Self::default(), |mut counts, artifact| {
                match artifact.action {
                    ArtifactAction::Move => counts.r#move += 1,
                    ArtifactAction::Copy => counts.copy += 1,
                    ArtifactAction::Retain => counts.retain += 1,
                    ArtifactAction::Block => counts.block += 1,
                }
                counts.already_archived += usize::from(artifact.already_archived);
                counts.pending_deletions += artifact.pending_deletions.len();
                counts
            })
    }
}

/// Schema-v1 archive plan shared by preview rendering and execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPlan {
    schema_version: u32,
    target: PlanTarget,
    destination_root: String,
    eligible: bool,
    policy_status: PolicyStatus,
    action_counts: ActionCounts,
    count: usize,
    artifacts: Vec<ArtifactPlanEntry>,
    blockers: Vec<PlanBlocker>,
    warnings: Vec<PlanWarning>,
}

impl ArtifactPlan {
    /// Construct and canonicalize one plan envelope.
    pub fn new(
        target: PlanTarget,
        destination_root: impl Into<String>,
        policy_status: PolicyStatus,
        mut artifacts: Vec<ArtifactPlanEntry>,
        mut blockers: Vec<PlanBlocker>,
        mut warnings: Vec<PlanWarning>,
    ) -> Result<Self, PlanError> {
        artifacts
            .iter_mut()
            .try_for_each(ArtifactPlanEntry::normalize)?;
        artifacts.sort_by_key(ArtifactPlanEntry::identity);

        let policy_blocker = match policy_status {
            PolicyStatus::Configured => None,
            PolicyStatus::Incomplete => Some(BlockerCode::PolicyIncomplete),
            PolicyStatus::Unconfigured => Some(BlockerCode::PolicyUnconfigured),
        };
        if let Some(code) = policy_blocker {
            blockers.push(PlanBlocker::new(code, None::<String>));
        }
        for blocker in &mut blockers {
            blocker.path = blocker.path.take().map(|path| normalize_path(&path));
        }
        sort_blockers(&mut blockers);
        for warning in &mut warnings {
            warning.path = warning.path.take().map(|path| normalize_path(&path));
        }
        sort_warnings(&mut warnings);

        let action_counts = ActionCounts::from_artifacts(&artifacts);
        let eligible = policy_status == PolicyStatus::Configured
            && blockers.is_empty()
            && artifacts.iter().all(|artifact| {
                artifact.action != ArtifactAction::Block && artifact.blockers.is_empty()
            });
        let count = artifacts.len();
        let destination_root = destination_root.into();

        Ok(Self {
            schema_version: ARCHIVE_PLAN_SCHEMA_VERSION,
            target: normalize_target(target),
            destination_root: normalize_path(&destination_root),
            eligible,
            policy_status,
            action_counts,
            count,
            artifacts,
            blockers,
            warnings,
        })
    }

    /// Whether all policy and plan/artifact blockers permit mutation.
    pub fn eligible(&self) -> bool {
        self.eligible
    }

    /// Wire schema version carried by this envelope.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Selected container or document target.
    pub fn target(&self) -> &PlanTarget {
        &self.target
    }

    /// Normalized configured archive root plus any target segment.
    pub fn destination_root(&self) -> &str {
        &self.destination_root
    }

    /// Explicitness state of the mutation-authorizing policy.
    pub fn policy_status(&self) -> PolicyStatus {
        self.policy_status
    }

    /// Counts derived from the canonical artifact list.
    pub fn action_counts(&self) -> ActionCounts {
        self.action_counts
    }

    /// Canonically ordered target-level blockers.
    pub fn blockers(&self) -> &[PlanBlocker] {
        &self.blockers
    }

    /// Canonically ordered target-level warnings.
    pub fn warnings(&self) -> &[PlanWarning] {
        &self.warnings
    }

    /// Deterministically ordered artifacts used by preview rendering.
    pub fn artifacts(&self) -> &[ArtifactPlanEntry] {
        &self.artifacts
    }

    /// Artifact count mirrored in the repository list-envelope convention.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Return the exact same artifact records for execution when eligible.
    ///
    /// Policy status is checked independently from the serialized `eligible`
    /// bit so accessor defaults or a hand-authored JSON value cannot authorize
    /// mutation.
    pub fn executable_artifacts(&self) -> Result<&[ArtifactPlanEntry], PlanError> {
        if self.schema_version != ARCHIVE_PLAN_SCHEMA_VERSION {
            return Err(PlanError::UnsupportedSchemaVersion(self.schema_version));
        }
        let mut canonical_artifacts = self.artifacts.clone();
        canonical_artifacts
            .iter_mut()
            .try_for_each(ArtifactPlanEntry::normalize)?;
        canonical_artifacts.sort_by_key(ArtifactPlanEntry::identity);
        if canonical_artifacts != self.artifacts
            || self.count != self.artifacts.len()
            || self.action_counts != ActionCounts::from_artifacts(&self.artifacts)
        {
            return Err(PlanError::NonCanonicalEnvelope);
        }
        if self.policy_status != PolicyStatus::Configured
            || !self.eligible
            || !self.blockers.is_empty()
            || self.artifacts.iter().any(|artifact| {
                artifact.action == ArtifactAction::Block || !artifact.blockers.is_empty()
            })
        {
            return Err(PlanError::Ineligible);
        }
        Ok(&self.artifacts)
    }
}

/// Construction and execution-readiness failures for the plan model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// A symbolic, abbreviated, uppercase, or otherwise non-canonical pin.
    NonCanonicalCommitOid(String),
    /// A recorded digest that is not exactly 64 lowercase hexadecimal characters.
    InvalidContentHash(String),
    /// A publication or deletion lacks its required source identity.
    MissingContentIdentity(String),
    /// An operation carrying no publication or deletion supplied an identity.
    UnexpectedContentIdentity(String),
    /// A move/copy action lacks its mirror destination.
    MissingDestination(String),
    /// A retained artifact incorrectly names a relocation destination.
    UnexpectedDestination(String),
    /// A pinned historical entry attempted a relocating operation.
    RelocatingPinnedArtifact(String),
    /// Execution was asked to consume an unknown wire schema.
    UnsupportedSchemaVersion(u32),
    /// Derived fields or ordering do not match the artifact contents.
    NonCanonicalEnvelope,
    /// Policy or plan blockers prohibit mutation.
    Ineligible,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalCommitOid(oid) => write!(
                formatter,
                "pinned artifact version is not a canonical full commit OID: {oid}"
            ),
            Self::InvalidContentHash(hash) => {
                write!(
                    formatter,
                    "content identity is not a lowercase SHA-256: {hash}"
                )
            }
            Self::MissingContentIdentity(path) => write!(
                formatter,
                "published or deleted working-tree artifact lacks content identity: {path}"
            ),
            Self::UnexpectedContentIdentity(path) => write!(
                formatter,
                "non-publishing artifact unexpectedly carries content identity: {path}"
            ),
            Self::MissingDestination(path) => {
                write!(formatter, "relocating artifact lacks a destination: {path}")
            }
            Self::UnexpectedDestination(path) => {
                write!(
                    formatter,
                    "retained artifact unexpectedly has a destination: {path}"
                )
            }
            Self::RelocatingPinnedArtifact(path) => {
                write!(
                    formatter,
                    "pinned historical artifact cannot relocate: {path}"
                )
            }
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    formatter,
                    "unsupported artifact-plan schema version: {version}"
                )
            }
            Self::NonCanonicalEnvelope => formatter
                .write_str("artifact plan ordering, count, or action counts are not canonical"),
            Self::Ineligible => formatter.write_str("artifact plan is not eligible for execution"),
        }
    }
}

impl std::error::Error for PlanError {}

fn normalize_target(target: PlanTarget) -> PlanTarget {
    match target {
        PlanTarget::Container { id } => PlanTarget::Container { id },
        PlanTarget::Document { path } => PlanTarget::Document {
            path: normalize_path(&path),
        },
    }
}

fn normalize_path(path: &str) -> String {
    let slash_normalized = path.replace('\\', "/");
    let mut components = Vec::new();
    for component in slash_normalized.split('/') {
        match component {
            "" | "." => {}
            ".." if components.last().is_some_and(|last| *last != "..") => {
                components.pop();
            }
            component => components.push(component),
        }
    }
    components.join("/")
}

fn blocker_cmp(left: &PlanBlocker, right: &PlanBlocker) -> Ordering {
    (left.code.as_str(), left.path.as_deref()).cmp(&(right.code.as_str(), right.path.as_deref()))
}

fn warning_cmp(left: &PlanWarning, right: &PlanWarning) -> Ordering {
    (left.code.as_str(), left.path.as_deref()).cmp(&(right.code.as_str(), right.path.as_deref()))
}

fn sort_blockers(blockers: &mut Vec<PlanBlocker>) {
    blockers.sort_by(blocker_cmp);
    blockers.dedup();
}

fn sort_warnings(warnings: &mut Vec<PlanWarning>) {
    warnings.sort_by(warning_cmp);
    warnings.dedup();
}
