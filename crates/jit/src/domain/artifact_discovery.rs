//! Pure parsing and path resolution for recursive artifact dependency discovery.
//!
//! This module never reads the filesystem. It turns already-read bytes into a
//! deterministic set of textual references and resolves each reference with
//! repository-component semantics. The storage layer owns the recursive read
//! loop and feeds bytes through this pure core.

use crate::domain::artifact_classifier::{ArtifactClassificationInventory, EmbeddedArtifactOwner};
use crate::domain::artifact_inventory::ExplicitRootInventory;
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactEdge, ArtifactPlanEntry, ArtifactProvenance, ArtifactVersion,
    BlockerCode, EdgeKind, EdgeResolutionMode, PlanBlocker, PlanError, PlanWarning, WarningCode,
};
use crate::domain::Issue;
use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

/// Parsed, side-effect-free facts about one already-read artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    format: Option<&'static str>,
    references: Vec<String>,
    dynamic_loading_suspected: bool,
}

impl ParsedArtifact {
    /// Stable format identifier used in archive-plan entries.
    pub fn format(&self) -> Option<&'static str> {
        self.format
    }

    /// Canonically ordered textual references extracted from supported syntax.
    pub fn references(&self) -> &[String] {
        &self.references
    }

    /// Whether a binding dynamic-loading textual pattern was observed.
    pub fn dynamic_loading_suspected(&self) -> bool {
        self.dynamic_loading_suspected
    }
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
}

impl ArtifactClosureState {
    /// Seed a closure expansion from normalized working-tree roots.
    pub fn new(roots: impl IntoIterator<Item = String>) -> Self {
        Self {
            graph: DiscoveryGraph::new(roots),
            parsed: BTreeMap::new(),
        }
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
    /// The requested path escaped or violated repository-relative syntax.
    InvalidPath,
}

/// Archive-specific captured evidence keyed by normalized worktree path.
pub type ArtifactEvidenceMap = BTreeMap<String, ArtifactEvidence>;

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
pub fn expand_artifact_closure(
    mut state: ArtifactClosureState,
    evidence: &ArtifactEvidenceMap,
) -> ArtifactClosure {
    let mut needs = BTreeSet::new();
    while let Some(path) = state.graph.next_path() {
        match evidence.get(&path) {
            None => {
                needs.insert(path);
            }
            Some(ArtifactEvidence::File(bytes)) => {
                let artifact = parse_artifact(&path, bytes);
                artifact.references().iter().for_each(|reference| {
                    if let ReferenceResolution::Local { target, .. } =
                        resolve_reference(&path, reference)
                    {
                        state.graph.enqueue(target);
                    }
                });
                state.parsed.insert(path, artifact);
            }
            Some(
                ArtifactEvidence::Missing
                | ArtifactEvidence::Symlink
                | ArtifactEvidence::Unsupported
                | ArtifactEvidence::InvalidPath,
            ) => {}
        }
    }
    if needs.is_empty() {
        ArtifactClosure::Complete(state.parsed)
    } else {
        needs
            .iter()
            .cloned()
            .for_each(|path| state.graph.defer(path));
        ArtifactClosure::Needs {
            paths: needs,
            state,
        }
    }
}

/// Derive selected inventory and repository-wide embedded owners from the same closed evidence.
pub fn discover_archive_artifacts(
    inventory: ExplicitRootInventory,
    issues: &[Issue],
    evidence: &ArtifactEvidenceMap,
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
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
        match evidence.get(&path) {
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
                for reference in parsed.references() {
                    match resolve_reference(&path, reference) {
                        ReferenceResolution::Ignored => {}
                        ReferenceResolution::External(edge) => {
                            edges.push(edge);
                            entry_warnings
                                .push(PlanWarning::new(WarningCode::ExternalEdge, Some(&path)));
                        }
                        ReferenceResolution::RepositoryEscape { edge, blocker } => {
                            edges.push(edge);
                            entry_blockers.push(blocker);
                        }
                        ReferenceResolution::Local { edge, target } => {
                            edges.push(edge);
                            referencing
                                .entry(target.clone())
                                .or_default()
                                .insert(path.clone());
                            if missing.contains(&target) {
                                entry_warnings.push(PlanWarning::new(
                                    WarningCode::MissingEdgeTarget,
                                    Some(&target),
                                ));
                            }
                            working.entry(target.clone()).or_insert_with(|| {
                                ArtifactPlanEntry::new(
                                    &target,
                                    ArtifactVersion::WorkingTree,
                                    ArtifactAction::Retain,
                                )
                                .with_provenance(vec![ArtifactProvenance::Embedded])
                            });
                            graph.enqueue(target);
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
                    .with_warnings(merge(entry.warnings(), entry_warnings));
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

/// Derive repository-wide embedded ownership from the same closed artifact evidence.
fn discover_embedded_owners(
    issues: &[Issue],
    selected_member_ids: &BTreeSet<String>,
    parsed_by_path: &BTreeMap<String, ParsedArtifact>,
) -> Vec<EmbeddedArtifactOwner> {
    let mut owners = Vec::new();
    for issue in issues {
        for document in issue
            .documents
            .iter()
            .filter(|document| document.commit.is_none())
        {
            let root = crate::domain::artifact_plan::normalize_artifact_path(&document.path);
            owners.extend(
                reachable_parsed_paths(&root, parsed_by_path)
                    .into_iter()
                    .filter(|path| path != &root)
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
            if let ReferenceResolution::Local { target, .. } = resolve_reference(&path, reference) {
                graph.enqueue(target);
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
        .with_warnings(merge(entry.warnings(), [warning]));
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
        Some("markdown") => markdown_references(&text),
        Some("html") => html_references(&text),
        Some("css") => css_references(&text),
        _ => Vec::new(),
    };
    let dynamic_loading_suspected =
        matches!(format, Some("markdown" | "html" | "css" | "javascript"))
            && suspects_dynamic_loading(&text);

    ParsedArtifact {
        format,
        references,
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

fn format_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => Some("markdown"),
        Some("html" | "htm") => Some("html"),
        Some("css") => Some("css"),
        Some("js" | "mjs" | "cjs") => Some("javascript"),
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
