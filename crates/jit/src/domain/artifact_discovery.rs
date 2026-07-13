//! Pure parsing and path resolution for recursive artifact dependency discovery.
//!
//! This module never reads the filesystem. It turns already-read bytes into a
//! deterministic set of textual references and resolves each reference with
//! repository-component semantics. The storage layer owns the recursive read
//! loop and feeds bytes through this pure core.

use crate::domain::artifact_plan::{
    ArtifactEdge, BlockerCode, EdgeKind, EdgeResolutionMode, PlanBlocker,
};
use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use std::collections::BTreeSet;
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
    let dynamic_loading_suspected = matches!(format, Some("markdown" | "html" | "javascript"))
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
    static CALL_WITH_LOCAL_PATH: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b(?:fetch|import|importScripts|require)\s*\(\s*["'](?:\.{1,2}/|/)[^"']+"#,
        )
        .ok()
    });
    static WORKER_WITH_LOCAL_PATH: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r#"(?i)\bnew\s+Worker\s*\(\s*["'](?:\.{1,2}/|/)[^"']+"#).ok());
    static STATIC_MODULE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r#"(?im)\b(?:import\s*(?:["'](?:\.{1,2}/|/)[^"']+["']|[^;\n]*\bfrom\s*["'](?:\.{1,2}/|/)[^"']+["'])|export\b[^;\n]*\bfrom\s*["'](?:\.{1,2}/|/)[^"']+["'])"#,
        )
        .ok()
    });
    static DATA_ATTRIBUTE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\bdata-[a-z0-9_.:-]+\s*=\s*(?:"(?:\.{1,2}/|/)[^"]+"|'(?:\.{1,2}/|/)[^']+')"#,
        )
        .ok()
    });

    content.contains("XMLHttpRequest")
        || CALL_WITH_LOCAL_PATH
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
        || WORKER_WITH_LOCAL_PATH
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
        || STATIC_MODULE
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
        || DATA_ATTRIBUTE
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
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
