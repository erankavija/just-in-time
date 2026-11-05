//! Pure parsing and path resolution for recursive artifact dependency discovery.
//!
//! This module never reads the filesystem. It turns already-read bytes into a
//! deterministic set of textual references and resolves each reference with
//! repository-component semantics. The storage layer owns the recursive read
//! loop and feeds bytes through this pure core.
//!
//! The plan's walk is bounded by the classification policy: an artifact the
//! policy retains outside the development root contributes no descendants. It
//! enters the plan when a scanned artifact links it, but nothing reached only
//! through its links joins it there, so one repository-root hub no longer draws
//! the set it cites into the plan (`@/issue/8e071e18/decision/D-14`).

use crate::domain::artifact_classifier::{
    ArtifactClassificationInventory, ArtifactClassificationPolicy, EmbeddedArtifactOwner,
};
use crate::domain::artifact_inventory::ExplicitRootInventory;
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactEdge, ArtifactPlanEntry, ArtifactProvenance, ArtifactVersion,
    BlockerCode, EdgeKind, EdgeResolutionMode, PlanBlocker, PlanError, PlanWarning, WarningCode,
};
use crate::domain::identity::IssueId;
use crate::domain::vocabulary::wire_vocabulary;
use crate::domain::Issue;
use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

wire_vocabulary! {
    /// Artifact format recognized by extension; its token is the format
    /// identifier archive-plan entries carry.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
    pub enum ArtifactFormat {
        /// Markdown source.
        Markdown = "markdown",
        /// HTML markup.
        Html = "html",
        /// CSS stylesheet.
        Css = "css",
        /// JavaScript module or script.
        JavaScript = "javascript",
    }
}

/// Parsed, side-effect-free facts about one already-read artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    format: Option<ArtifactFormat>,
    references: Vec<ParsedReference>,
    dynamic_loading_suspected: bool,
}

impl ParsedArtifact {
    /// Stable format identifier used in archive-plan entries.
    pub fn format(&self) -> Option<&'static str> {
        self.format.map(ArtifactFormat::as_str)
    }

    /// Canonically ordered textual references extracted from supported syntax.
    pub fn references(&self) -> &[ParsedReference] {
        &self.references
    }

    /// Whether a binding dynamic-loading textual pattern was observed.
    pub fn dynamic_loading_suspected(&self) -> bool {
        self.dynamic_loading_suspected
    }
}

/// An extracted reference and its resolution relative to the parsed artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReference {
    pub reference: String,
    pub resolution: ReferenceResolution,
}

/// Pure outcome of resolving one textual reference from its parent path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceResolution {
    /// An anchor-only or empty reference contributes no dependency edge.
    Ignored,
    /// A remote, protocol-relative, or URI-scheme reference.
    External(ArtifactEdge),
    /// A supported local reference with its normalized repository target.
    Local { edge: ArtifactEdge, target: String },
    /// Component normalization would climb above the repository root.
    RepositoryEscape {
        edge: ArtifactEdge,
        blocker: PlanBlocker,
    },
}

/// Deterministic, cycle-safe pending set for recursive discovery.
///
/// This state machine is pure: the storage boundary supplies roots and
/// resolved targets, while ordering and visited-state decisions remain here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryGraph {
    pending: BTreeSet<String>,
    visited: BTreeSet<String>,
}

/// Incremental pure discovery state retained while callers acquire missing evidence.
/// Evidence for paths already listed by [`Self::parsed_paths`] must remain immutable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClosureState {
    graph: DiscoveryGraph,
    parsed: BTreeMap<String, ParsedArtifact>,
    archived_copies: BTreeMap<String, String>,
}

impl ArtifactClosureState {
    /// Seed a closure expansion from normalized working-tree roots.
    pub fn new(roots: impl IntoIterator<Item = String>) -> Self {
        Self {
            graph: DiscoveryGraph::new(roots),
            parsed: BTreeMap::new(),
            archived_copies: BTreeMap::new(),
        }
    }

    /// Name the copy an earlier archival of this target published for each
    /// source, keyed by source.
    ///
    /// A source that archival then deleted is read through its copy, whose
    /// bytes are the source's own, so its edges constrain a rerun exactly as
    /// they did the run that archived it.
    pub fn with_archived_copies(mut self, copies: BTreeMap<String, String>) -> Self {
        self.archived_copies = copies;
        self
    }

    /// Paths parsed so far, exposed to verify incremental reuse without instrumentation.
    pub fn parsed_paths(&self) -> impl Iterator<Item = &str> {
        self.parsed.keys().map(String::as_str)
    }
}

/// No-follow worktree evidence used by both selected-artifact and owner closure expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactEvidence {
    /// Captured ordinary file bytes.
    File(Vec<u8>),
    /// Captured absence.
    Missing,
    /// A symlink occurs at or above the path.
    Symlink,
    /// A non-file, non-directory filesystem object.
    Unsupported,
    /// A directory captured with the stated listing completeness.
    Directory {
        /// Which of [`ArtifactListingScope`]'s completeness levels the entries cover.
        scope: ArtifactListingScope,
        /// Canonical repository-relative paths covered by `scope`.
        entries: Vec<String>,
    },
    /// The requested path escaped or violated repository-relative syntax.
    InvalidPath,
}

/// Completeness of entries captured for directory evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactListingScope {
    /// Only the directory's own kind was inspected.
    MetadataOnly,
    /// Every immediate child path was captured.
    ImmediateChildren,
    /// Every recursive non-directory entry was captured.
    ///
    /// A directory is descended through without being named, so a subtree
    /// holding no file contributes nothing. This is what a reader wanting file
    /// bytes asks for.
    RecursiveFiles,
    /// Every recursive entry was captured, the directories descended through
    /// included.
    ///
    /// This is what a reader asks for when a directory is evidence in its own
    /// right rather than a container to look inside: an empty one, and one
    /// holding nothing but further directories, are both named.
    RecursiveEntries,
}

/// Archive-specific captured evidence keyed by normalized worktree path.
pub type ArtifactEvidenceMap = BTreeMap<String, ArtifactEvidence>;

/// The copy each source was published to by recorded archivals of `target`,
/// keyed by normalized source; a later record supersedes an earlier one.
pub fn recorded_archived_copies(
    executions: &[crate::domain::artifact_execution::ArchiveExecution],
    target: &crate::domain::artifact_plan::PlanTarget,
) -> BTreeMap<String, String> {
    executions
        .iter()
        .filter(|execution| &execution.target == target)
        .flat_map(|execution| &execution.publications)
        .filter_map(|publication| {
            publication.source.as_deref().map(|source| {
                (
                    crate::domain::artifact_plan::normalize_artifact_path(source),
                    crate::domain::artifact_plan::normalize_artifact_path(&publication.destination),
                )
            })
        })
        .collect()
}

/// Pure closure result: callers acquire only the paths still needed and retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactClosure {
    /// More exact path evidence is required; resume with the returned state.
    Needs {
        paths: BTreeSet<String>,
        state: ArtifactClosureState,
    },
    /// Every reachable ordinary file was parsed exactly once into this map.
    Complete(BTreeMap<String, ParsedArtifact>),
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactDiscoveryError {
    /// Discovery produced an invalid artifact-plan entry.
    #[error(transparent)]
    InvalidPlanEntry(#[from] PlanError),
    /// Closed evidence or its parsed closure omitted a graph entry.
    #[error("artifact discovery graph has no entry for {0}")]
    MissingGraphEntry(String),
}

/// Expand all supported local edges from roots using one explicit evidence map.
///
/// This closure is deliberately unbounded, and the development root must not
/// narrow it. Stopping the read at that boundary looks like free work to skip,
/// because [`discover_archive_artifacts`] discards the descendants anyway, but
/// the two walks read this map for different questions.
///
/// The plan asks what an archive takes with it, and bounds itself. The
/// repository-wide ownership relation asks what still reaches an artifact, and
/// derives [`crate::domain::artifact_plan::EvidenceCode::OutsideOwner`] from
/// chains that leave the development root and re-enter it — an issue's document
/// citing a repository-root hub that cites a development file. That evidence
/// is what keeps such an artifact a copy. Bounding the read here deletes those
/// chains, and with them the only reason those artifacts were not relocated:
/// measured over this repository, eight of them silently turn from copy into
/// relocation and their live sources are deleted while another issue's document
/// still reaches them (`@/issue/8e071e18/decision/D-14`).
/// `test_document_preview_copies_an_artifact_whose_only_outside_owner_arrives_through_an_out_of_root_hub`
/// in the archive command fails if this walk is bounded.
pub fn expand_artifact_closure(
    mut state: ArtifactClosureState,
    evidence: &ArtifactEvidenceMap,
) -> ArtifactClosure {
    let mut needs = BTreeSet::new();
    let mut revisit = BTreeSet::new();
    while let Some(path) = state.graph.next_path() {
        let fact = match evidence.get(&path) {
            Some(ArtifactEvidence::Missing) => match state.archived_copies.get(&path) {
                Some(copy) => evidence.get(copy).or_else(|| {
                    needs.insert(copy.clone());
                    revisit.insert(path.clone());
                    None
                }),
                None => Some(&ArtifactEvidence::Missing),
            },
            None => {
                needs.insert(path.clone());
                revisit.insert(path.clone());
                None
            }
            fact => fact,
        };
        match fact {
            None => {}
            Some(ArtifactEvidence::File(bytes)) => {
                let artifact = parse_artifact(&path, bytes);
                artifact.references().iter().for_each(|reference| {
                    if let ReferenceResolution::Local { target, .. } = &reference.resolution {
                        state.graph.enqueue(target.clone());
                    }
                });
                state.parsed.insert(path, artifact);
            }
            Some(
                ArtifactEvidence::Missing
                | ArtifactEvidence::Symlink
                | ArtifactEvidence::Unsupported
                | ArtifactEvidence::Directory { .. }
                | ArtifactEvidence::InvalidPath,
            ) => {}
        }
    }
    if needs.is_empty() {
        ArtifactClosure::Complete(state.parsed)
    } else {
        revisit.into_iter().for_each(|path| state.graph.defer(path));
        ArtifactClosure::Needs {
            paths: needs,
            state,
        }
    }
}

/// The references of one parsed artifact that the plan's walk resolves.
///
/// An artifact the policy retains in place contributes
/// none: the walk stops at it, so it names no edge and admits no descendant of
/// it to the plan (`@/issue/8e071e18/decision/D-14`).
fn followed_references<'a>(
    path: &str,
    artifact: &'a ParsedArtifact,
    policy: &ArtifactClassificationPolicy,
) -> std::slice::Iter<'a, ParsedReference> {
    const BOUNDED: &[ParsedReference] = &[];

    if policy.retains_source_in_place(path) {
        BOUNDED.iter()
    } else {
        artifact.references().iter()
    }
}

/// Derive selected inventory and repository-wide embedded owners from the same closed evidence.
///
/// Link targets that `is_navigation_target` identifies are skipped: they
/// contribute no edge, no artifact entry, and no traversal. An explicitly
/// selected root is inventoried whatever its filesystem kind, so a directory
/// named as a root still reaches classification and blocks there. `policy`
/// bounds this walk through `followed_references`.
pub fn discover_archive_artifacts(
    inventory: ExplicitRootInventory,
    issues: &[Issue],
    evidence: &ArtifactEvidenceMap,
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
    policy: &ArtifactClassificationPolicy,
) -> Result<(ArtifactClassificationInventory, Vec<EmbeddedArtifactOwner>), ArtifactDiscoveryError> {
    let (target, member_ids, roots, blockers) = inventory.into_discovery_parts();
    let working_roots = roots
        .iter()
        .filter(|entry| !entry.version().is_pinned())
        .map(|entry| entry.source().to_string())
        .collect::<BTreeSet<_>>();
    let explicit_paths = working_roots;
    let mut historical = roots
        .iter()
        .filter(|entry| entry.version().is_pinned())
        .cloned()
        .collect::<Vec<_>>();
    let mut working = roots
        .into_iter()
        .filter(|entry| !entry.version().is_pinned())
        .map(|entry| (entry.source().to_string(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut graph = DiscoveryGraph::new(explicit_paths.iter().cloned());
    let mut referencing = BTreeMap::<String, BTreeSet<String>>::new();
    let mut missing = BTreeSet::new();
    while let Some(path) = graph.next_path() {
        // A source the closure parsed from its archived copy contributes its
        // edges as the file it was, whatever bytes the branch below ignores.
        const PARSED_FROM_COPY: ArtifactEvidence = ArtifactEvidence::File(Vec::new());
        let fact = match evidence.get(&path) {
            Some(ArtifactEvidence::Missing) if parsed_by_path.contains_key(&path) => {
                Some(&PARSED_FROM_COPY)
            }
            fact => fact,
        };
        match fact {
            Some(ArtifactEvidence::Missing) => {
                if explicit_paths.contains(&path) {
                    append_blocker(
                        working.get_mut(&path).ok_or_else(|| {
                            ArtifactDiscoveryError::MissingGraphEntry(path.clone())
                        })?,
                        PlanBlocker::new(BlockerCode::MissingSource, Some(&path)),
                    )?;
                } else {
                    missing.insert(path.clone());
                    for parent in referencing.get(&path).into_iter().flatten() {
                        append_warning(
                            working.get_mut(parent).ok_or_else(|| {
                                ArtifactDiscoveryError::MissingGraphEntry(parent.clone())
                            })?,
                            PlanWarning::new(WarningCode::MissingEdgeTarget, Some(&path)),
                        )?;
                    }
                }
                continue;
            }
            Some(ArtifactEvidence::Unsupported) => {
                for parent in referencing.get(&path).into_iter().flatten() {
                    append_warning(
                        working.get_mut(parent).ok_or_else(|| {
                            ArtifactDiscoveryError::MissingGraphEntry(parent.clone())
                        })?,
                        PlanWarning::new(WarningCode::UnsupportedEdgeTarget, Some(&path)),
                    )?;
                }
                continue;
            }
            // A directory enters the graph only as an explicit root, because
            // navigation targets are skipped before they are enqueued. Its
            // entry stays in the inventory and classification blocks it from
            // its own location evidence.
            Some(ArtifactEvidence::Directory { .. }) => continue,
            Some(ArtifactEvidence::InvalidPath) => {
                append_blocker(
                    working
                        .get_mut(&path)
                        .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?,
                    PlanBlocker::new(BlockerCode::RepositoryEscape, Some(&path)),
                )?;
                continue;
            }
            Some(ArtifactEvidence::Symlink) => continue,
            Some(ArtifactEvidence::File(_)) => {
                let parsed = parsed_by_path
                    .get(&path)
                    .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?;
                let mut edges = Vec::new();
                let mut entry_blockers = Vec::new();
                let mut entry_warnings = Vec::new();
                if parsed.dynamic_loading_suspected() {
                    entry_warnings.push(PlanWarning::new(
                        WarningCode::DynamicLoadingSuspected,
                        Some(&path),
                    ));
                }
                for reference in followed_references(&path, parsed, policy) {
                    match &reference.resolution {
                        ReferenceResolution::Ignored => {}
                        ReferenceResolution::External(edge) => {
                            edges.push(edge.clone());
                            entry_warnings
                                .push(PlanWarning::new(WarningCode::ExternalEdge, Some(&path)));
                        }
                        ReferenceResolution::RepositoryEscape { edge, blocker } => {
                            edges.push(edge.clone());
                            entry_blockers.push(blocker.clone());
                        }
                        ReferenceResolution::Local { edge, target } => {
                            if is_navigation_target(target, evidence) {
                                continue;
                            }
                            edges.push(edge.clone());
                            referencing
                                .entry(target.clone())
                                .or_default()
                                .insert(path.clone());
                            if missing.contains(target) {
                                entry_warnings.push(PlanWarning::new(
                                    WarningCode::MissingEdgeTarget,
                                    Some(target),
                                ));
                            }
                            working.entry(target.clone()).or_insert_with(|| {
                                ArtifactPlanEntry::new(
                                    target,
                                    ArtifactVersion::WorkingTree,
                                    ArtifactAction::Retain,
                                )
                                .with_provenance(vec![ArtifactProvenance::Embedded])
                            });
                            graph.enqueue(target.clone());
                        }
                    }
                }
                let entry = working
                    .get_mut(&path)
                    .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?;
                let mut updated = entry
                    .clone()
                    .with_edges(merge(entry.edges(), edges))
                    .with_blockers(merge(entry.blockers(), entry_blockers))
                    .with_plan_warnings(merge(entry.plan_warnings(), entry_warnings));
                if let Some(format) = parsed.format() {
                    updated = updated.with_format(format);
                }
                updated.normalize()?;
                *entry = updated;
            }
            None => return Err(ArtifactDiscoveryError::MissingGraphEntry(path)),
        }
    }
    historical.extend(working.into_values());
    historical.sort_by_key(ArtifactPlanEntry::identity);
    let members = member_ids.iter().cloned().collect::<BTreeSet<_>>();
    let owners = discover_embedded_owners(issues, &members, parsed_by_path);
    Ok((
        ArtifactClassificationInventory::new(target, historical, blockers),
        owners,
    ))
}

/// Whether a resolved local target is navigation rather than a document.
///
/// A link whose target is a directory addresses a place to browse, not an
/// artifact the parent depends on (`@/issue/8e071e18/decision/D-18`). A
/// symbolic link keeps its own treatment even when it points at a directory,
/// because captured evidence reports the link itself.
///
/// Absent evidence is never navigation: the closure contract supplies a fact
/// for every reachable local target, so an uncaptured path stays a graph error
/// rather than becoming a silent skip.
fn is_navigation_target(target: &str, evidence: &ArtifactEvidenceMap) -> bool {
    matches!(
        evidence.get(target),
        Some(ArtifactEvidence::Directory { .. })
    )
}

/// Derive repository-wide embedded ownership from the same closed artifact evidence.
///
/// Ownership is not bounded by the development root. A claim it reports only
/// ever retains a source, so reaching an artifact through one the plan retains
/// outside that root is the safe direction to err in: a missed claim would let
/// an archive delete a file another issue's document still reaches.
fn discover_embedded_owners(
    issues: &[Issue],
    selected_member_ids: &BTreeSet<IssueId>,
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
) -> Vec<EmbeddedArtifactOwner> {
    let reachable_by_root = reachable_document_paths(issues, parsed_by_path);
    let mut owners = Vec::new();
    for issue in issues {
        for document in issue
            .documents
            .iter()
            .filter(|document| document.commit.is_none())
        {
            let root = crate::domain::artifact_plan::normalize_artifact_path(&document.path);
            owners.extend(
                reachable_by_root
                    .get(&root)
                    .into_iter()
                    .flatten()
                    .filter(|path| *path != &root)
                    .map(|path| EmbeddedArtifactOwner {
                        artifact: path.clone(),
                        root: root.clone(),
                        issue: issue.id.clone(),
                        state: issue.state,
                        archived_from: issue.archived_from,
                        inside_subtree: selected_member_ids.contains(&issue.id),
                    }),
            );
        }
    }
    owners.sort_by(|left, right| {
        (&left.artifact, &left.root, &left.issue).cmp(&(&right.artifact, &right.root, &right.issue))
    });
    owners.dedup();
    owners
}

fn reachable_document_paths(
    issues: &[Issue],
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
) -> BTreeMap<String, BTreeSet<String>> {
    issues
        .iter()
        .flat_map(|issue| &issue.documents)
        .filter(|document| document.commit.is_none())
        .map(|document| crate::domain::artifact_plan::normalize_artifact_path(&document.path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|root| {
            let reachable = reachable_parsed_paths(&root, parsed_by_path);
            (root, reachable)
        })
        .collect()
}

fn reachable_parsed_paths(
    root: &str,
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
) -> BTreeSet<String> {
    let mut graph = DiscoveryGraph::new([root.to_string()]);
    let mut reachable = BTreeSet::new();
    while let Some(path) = graph.next_path() {
        let Some(parsed) = parsed_by_path.get(&path) else {
            continue;
        };
        reachable.insert(path.clone());
        parsed.references().iter().for_each(|reference| {
            if let ReferenceResolution::Local { target, .. } = &reference.resolution {
                graph.enqueue(target.clone());
            }
        });
    }
    reachable
}

fn append_blocker(entry: &mut ArtifactPlanEntry, blocker: PlanBlocker) -> Result<(), PlanError> {
    let mut updated = entry
        .clone()
        .with_blockers(merge(entry.blockers(), [blocker]));
    updated.normalize()?;
    *entry = updated;
    Ok(())
}
fn append_warning(entry: &mut ArtifactPlanEntry, warning: PlanWarning) -> Result<(), PlanError> {
    let mut updated = entry
        .clone()
        .with_plan_warnings(merge(entry.plan_warnings(), [warning]));
    updated.normalize()?;
    *entry = updated;
    Ok(())
}
fn merge<T: Clone>(existing: &[T], additional: impl IntoIterator<Item = T>) -> Vec<T> {
    existing.iter().cloned().chain(additional).collect()
}

impl DiscoveryGraph {
    /// Seed discovery with normalized working-tree roots.
    pub fn new(roots: impl IntoIterator<Item = String>) -> Self {
        Self {
            pending: roots.into_iter().collect(),
            visited: BTreeSet::new(),
        }
    }

    /// Enqueue one normalized target unless it has already been visited.
    pub fn enqueue(&mut self, target: String) {
        if !self.visited.contains(&target) {
            self.pending.insert(target);
        }
    }

    fn defer(&mut self, path: String) {
        self.visited.remove(&path);
        self.pending.insert(path);
    }

    /// Take and mark the lexicographically next unvisited path.
    pub fn next_path(&mut self) -> Option<String> {
        while let Some(path) = self.pending.pop_first() {
            if self.visited.insert(path.clone()) {
                return Some(path);
            }
        }
        None
    }
}

/// Parse supported embedded references from one already-read artifact.
///
/// Format selection is extension-based deliberately: opaque roots remain valid
/// inventory entries, but random binary bytes are never guessed to be markup.
pub fn parse_artifact(path: &str, bytes: &[u8]) -> ParsedArtifact {
    let text = String::from_utf8_lossy(bytes);
    let format = format_for_path(path);
    let references = match format {
        Some(ArtifactFormat::Markdown) => markdown_references(&text),
        Some(ArtifactFormat::Html) => html_references(&text),
        Some(ArtifactFormat::Css) => css_references(&text),
        Some(ArtifactFormat::JavaScript) | None => Vec::new(),
    };
    let dynamic_loading_suspected = format.is_some() && suspects_dynamic_loading(&text);

    ParsedArtifact {
        format,
        references: references
            .into_iter()
            .map(|reference| ParsedReference {
                resolution: resolve_reference(path, &reference),
                reference,
            })
            .collect(),
        dynamic_loading_suspected,
    }
}

/// Resolve a supported textual reference using repository-component semantics.
pub fn resolve_reference(parent: &str, reference: &str) -> ReferenceResolution {
    let trimmed = reference.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return ReferenceResolution::Ignored;
    }

    let resolution_mode = if trimmed.starts_with('/') && !trimmed.starts_with("//") {
        EdgeResolutionMode::RootRelative
    } else if is_external(trimmed) {
        return ReferenceResolution::External(ArtifactEdge {
            reference: trimmed.to_string(),
            target: None,
            kind: EdgeKind::External,
            resolution_mode: EdgeResolutionMode::External,
        });
    } else {
        EdgeResolutionMode::Relative
    };

    let path_only = trimmed
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .replace('\\', "/");
    if path_only.is_empty() {
        return ReferenceResolution::Ignored;
    }

    let mut components = if resolution_mode == EdgeResolutionMode::Relative {
        parent
            .rsplit_once('/')
            .map(|(directory, _)| {
                directory
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let candidate = path_only.trim_start_matches('/');
    for component in candidate.split('/') {
        match component {
            "" | "." => {}
            ".." if components.pop().is_none() => {
                let edge = ArtifactEdge {
                    reference: trimmed.to_string(),
                    target: None,
                    kind: EdgeKind::Supported,
                    resolution_mode,
                };
                return ReferenceResolution::RepositoryEscape {
                    edge,
                    blocker: PlanBlocker::new(BlockerCode::RepositoryEscape, Some(parent)),
                };
            }
            ".." => {}
            value => components.push(value.to_string()),
        }
    }

    if components.is_empty() {
        return ReferenceResolution::Ignored;
    }
    let target = components.join("/");
    ReferenceResolution::Local {
        edge: ArtifactEdge {
            reference: trimmed.to_string(),
            target: Some(target.clone()),
            kind: EdgeKind::Supported,
            resolution_mode,
        },
        target,
    }
}

fn format_for_path(path: &str) -> Option<ArtifactFormat> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => Some(ArtifactFormat::Markdown),
        Some("html" | "htm") => Some(ArtifactFormat::Html),
        Some("css") => Some(ArtifactFormat::Css),
        Some("js" | "mjs" | "cjs") => Some(ArtifactFormat::JavaScript),
        _ => None,
    }
}

fn markdown_references(content: &str) -> Vec<String> {
    let mut references = BTreeSet::new();
    for event in Parser::new(content) {
        if let Event::Start(Tag::Image { dest_url, .. } | Tag::Link { dest_url, .. }) = event {
            references.insert(dest_url.trim().to_string());
        }
    }
    references.into_iter().collect()
}

fn html_references(content: &str) -> Vec<String> {
    static TAG: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?is)<[a-z][^>]*>").ok());
    static ATTRIBUTE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?i)([a-z_:][a-z0-9_:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))"#)
            .ok()
    });
    let (Some(tag_regex), Some(attribute_regex)) = (TAG.as_ref(), ATTRIBUTE.as_ref()) else {
        return Vec::new();
    };

    tag_regex
        .find_iter(content)
        .flat_map(|tag| attribute_regex.captures_iter(tag.as_str()))
        .filter(|captures| matches!(&captures[1].to_ascii_lowercase()[..], "src" | "href"))
        .filter_map(|captures| attribute_value(&captures).map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn css_references(content: &str) -> Vec<String> {
    static IMPORT: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?is)@import\s+(?:url\(\s*)?(?:"([^"]+)"|'([^']+)'|([^\s)'";]+))"#).ok()
    });
    static URL: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?is)url\(\s*(?:"([^"]+)"|'([^']+)'|([^\s)'";]+))\s*\)"#).ok()
    });
    let (Some(import_regex), Some(url_regex)) = (IMPORT.as_ref(), URL.as_ref()) else {
        return Vec::new();
    };

    import_regex
        .captures_iter(content)
        .chain(url_regex.captures_iter(content))
        .filter_map(|captures| capture_alternative(&captures, &[1, 2, 3]).map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn suspects_dynamic_loading(content: &str) -> bool {
    static CALL: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(fetch|import|importScripts|require)\s*\(\s*(?:"([^"]*)"|'([^']*)')"#)
            .ok()
    });
    static WORKER: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r#"(?i)\bnew\s+Worker\s*\(\s*(?:"([^"]*)"|'([^']*)')"#).ok());
    static STATIC_MODULE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r#"(?im)\b(?:import\s*(?:["']([^"']+)["']|[^;\n]*\bfrom\s*["']([^"']+)["'])|export\b[^;\n]*\bfrom\s*["']([^"']+)["'])"#,
        )
        .ok()
    });
    static DATA_ATTRIBUTE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?i)\bdata-[a-z0-9_.:-]+\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))"#).ok()
    });

    content.contains("XMLHttpRequest")
        || CALL.as_ref().is_some_and(|regex| {
            regex.captures_iter(content).any(|captures| {
                let function = &captures[1];
                capture_alternative(&captures, &[2, 3]).is_some_and(|reference| {
                    if function.eq_ignore_ascii_case("fetch")
                        || function.eq_ignore_ascii_case("importScripts")
                    {
                        is_local_url(reference)
                    } else {
                        is_explicit_local_path(reference)
                    }
                })
            })
        })
        || WORKER.as_ref().is_some_and(|regex| {
            regex
                .captures_iter(content)
                .any(|captures| capture_alternative(&captures, &[1, 2]).is_some_and(is_local_url))
        })
        || STATIC_MODULE.as_ref().is_some_and(|regex| {
            regex.captures_iter(content).any(|captures| {
                capture_alternative(&captures, &[1, 2, 3]).is_some_and(is_explicit_local_path)
            })
        })
        || DATA_ATTRIBUTE.as_ref().is_some_and(|regex| {
            regex.captures_iter(content).any(|captures| {
                capture_alternative(&captures, &[1, 2, 3]).is_some_and(is_local_path_like_value)
            })
        })
}

fn is_local_url(reference: &str) -> bool {
    local_reference_path(reference).is_some()
}

fn is_explicit_local_path(reference: &str) -> bool {
    local_reference_path(reference).is_some_and(|path| {
        path.starts_with("./")
            || path.starts_with("../")
            || (path.starts_with('/') && !path.starts_with("//"))
    })
}

fn is_local_path_like_value(reference: &str) -> bool {
    local_reference_path(reference).is_some_and(|path| {
        is_explicit_local_path(path)
            || path.contains('/')
            || path
                .rsplit_once('.')
                .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
    })
}

fn local_reference_path(reference: &str) -> Option<&str> {
    let trimmed = reference.trim();
    if trimmed.is_empty()
        || trimmed.starts_with(['#', '?'])
        || trimmed.starts_with("//")
        || is_external(trimmed)
        || trimmed.chars().any(char::is_whitespace)
    {
        return None;
    }

    let path = trimmed.split(['?', '#']).next().unwrap_or_default();
    (!path.is_empty() && !matches!(path, "." | "..")).then_some(path)
}

fn is_external(reference: &str) -> bool {
    if reference.starts_with("//") {
        return true;
    }
    let before_path = reference.split(['/', '?', '#']).next().unwrap_or_default();
    before_path
        .split_once(':')
        .map(|(scheme, _)| {
            !scheme.is_empty()
                && scheme.chars().enumerate().all(|(index, character)| {
                    if index == 0 {
                        character.is_ascii_alphabetic()
                    } else {
                        character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
                    }
                })
        })
        .unwrap_or(false)
}

fn attribute_value<'a>(captures: &'a regex::Captures<'a>) -> Option<&'a str> {
    capture_alternative(captures, &[2, 3, 4])
}

fn capture_alternative<'a>(
    captures: &'a regex::Captures<'a>,
    indices: &[usize],
) -> Option<&'a str> {
    indices
        .iter()
        .find_map(|index| captures.get(*index).map(|value| value.as_str()))
}

#[cfg(test)]
mod archived_copy_tests {
    use super::recorded_archived_copies;
    use crate::domain::artifact_plan::PlanTarget;
    use crate::test_utils::archive_execution;

    fn document(path: &str) -> PlanTarget {
        PlanTarget::Document { path: path.into() }
    }

    #[test]
    fn test_recorded_archived_copies_later_execution_supersedes_and_other_targets_are_ignored() {
        let executions = [
            archive_execution(document("a.md"), "arc", &[("src/x.md", "arc/old/x.md")]),
            archive_execution(document("b.md"), "arc", &[("src/x.md", "arc/other/x.md")]),
            archive_execution(document("a.md"), "arc", &[("src/x.md", "arc/new/x.md")]),
        ];
        let copies = recorded_archived_copies(&executions, &document("a.md"));
        assert_eq!(copies.len(), 1);
        assert_eq!(copies["src/x.md"], "arc/new/x.md");
    }
}

#[cfg(test)]
mod resolution_cache_tests {
    use super::*;
    use crate::test_utils::{document_reference, fixture_issue};

    #[test]
    fn test_parse_artifact_stores_each_reference_with_its_parent_resolution() {
        let parent = "docs/root.md";
        let source = "[local](child.md) [root](/asset.png) [external](https://example.com) [anchor](#here) [escape](../../outside.md)";
        let parsed = parse_artifact(parent, source.as_bytes());
        let extracted = markdown_references(source);
        assert_eq!(parsed.references().len(), extracted.len());
        for (stored, reference) in parsed.references().iter().zip(extracted) {
            assert_eq!(stored.reference, reference);
            assert_eq!(stored.resolution, resolve_reference(parent, &reference));
        }
    }

    #[test]
    fn test_discover_embedded_owners_shared_normalized_roots_reuse_reachability() {
        let mut first = fixture_issue("first".into(), String::new());
        first.documents = vec![document_reference("docs/root.md")];
        let mut second = fixture_issue("second".into(), String::new());
        second.documents = vec![
            document_reference("./docs/root.md"),
            document_reference("docs/other.md"),
        ];
        let mut pinned = document_reference("docs/pinned.md");
        pinned.commit = Some("abc123".into());
        second.documents.push(pinned);
        let issues = [first, second];
        let parsed = [
            ("docs/root.md", "[child](child.md)"),
            ("docs/other.md", "[child](child.md)"),
            ("docs/child.md", "[root](root.md) ![asset](asset.png)"),
            ("docs/asset.png", ""),
        ]
        .into_iter()
        .map(|(path, text)| (path.to_string(), parse_artifact(path, text.as_bytes())))
        .collect();
        let members = BTreeSet::from([issues[0].id.clone()]);
        let by_root = reachable_document_paths(&issues, &parsed);
        assert_eq!(
            by_root.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["docs/other.md", "docs/root.md"]
        );
        for (root, paths) in &by_root {
            assert_eq!(paths, &reachable_parsed_paths(root, &parsed));
        }
        let mut separately = issues
            .iter()
            .flat_map(|issue| {
                discover_embedded_owners(std::slice::from_ref(issue), &members, &parsed)
            })
            .collect::<Vec<_>>();
        separately.sort_by(|left, right| {
            (&left.artifact, &left.root, &left.issue).cmp(&(
                &right.artifact,
                &right.root,
                &right.issue,
            ))
        });
        let shared = discover_embedded_owners(&issues, &members, &parsed);
        assert!(!shared.is_empty());
        assert_eq!(shared, separately);
    }
}
