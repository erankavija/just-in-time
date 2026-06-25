//! Addressable structured items: a pure projection over issue descriptions.
//!
//! An **addressable item** is a structured list entry in a declared section of an
//! issue description that carries a *self-id* matched by an id-pattern. Its
//! **qualified id** `<issue-id>/<self-id>` is globally unique and is *derived*
//! from existing data (the issue's short id plus the parsed self-id) — nothing is
//! persisted twice (REQ-02, REQ-03).
//!
//! An **item kind** ([`ItemKind`]) is the config-declared projection
//! `(section, id-pattern, marker(s), link-namespace(s))` that says which entries
//! are addressable and how. `requirement` is just one kind; the model is generic
//! and no kind NAME is interpreted by this module — only the four-tuple is
//! (REQ-01). The built-in default [`ItemKind::requirement_default`] reproduces
//! the exact triple the `label-coverage` rule already consumes, so the existing
//! coverage machinery is demonstrably compatible with the model (REQ-05).
//!
//! Indexing ([`index_items`]) is a pure function of the issue, the kinds, and a
//! [`ContentParser`]: markdown stays the single source of truth and the item
//! index is recomputed on demand. A list entry without a matching self-id is
//! plain prose and incurs no addressing requirement (REQ-06).

use crate::config::ItemKindConfig;
use crate::document::ContentParser;
use crate::domain::{project, Issue};
use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Default section slug scanned for items when a kind declares none.
///
/// Matches the `label-coverage` rule's `criteria-section` default so the built-in
/// `requirement` kind and the coverage rule read the SAME section.
pub const DEFAULT_ITEM_SECTION: &str = "success_criteria";

/// Default self-id pattern when a kind declares none — the repo default id shape
/// (`REQ-01`, `D-2`, `RISK-03`, ...), identical to the `label-coverage` rule's
/// `id-pattern` default.
pub const DEFAULT_ITEM_ID_PATTERN: &str = "[A-Z][A-Z0-9]*-[0-9]+";

/// Default link-label namespace referencing items of a kind when none is
/// declared, matching the `label-coverage` rule's `satisfies-namespace` default.
pub const DEFAULT_ITEM_LINK_NAMESPACE: &str = "satisfies";

/// The conventional NAME of the built-in requirement kind.
///
/// Used only to LABEL the default kind for display and `--kind` filtering; it is
/// never branched on in indexing logic (the kind is fully described by its
/// four-tuple), keeping the engine domain-agnostic (REQ-01).
pub const REQUIREMENT_KIND_NAME: &str = "requirement";

/// Errors raised while resolving item kinds or indexing items.
#[derive(Debug, Error)]
pub enum ItemError {
    /// A kind's `id-pattern` is not a valid regular expression.
    #[error("item kind '{kind}' has an invalid id-pattern '{pattern}': {source}")]
    InvalidIdPattern {
        /// The offending kind name.
        kind: String,
        /// The pattern string that failed to compile.
        pattern: String,
        /// The underlying regex compilation error.
        source: regex::Error,
    },
    /// Two addressable items in one issue share a self-id, so the qualified id
    /// `<issue-id>/<self-id>` would not be unique within the issue (REQ-02).
    #[error(
        "issue {issue} declares self-id '{self_id}' more than once for kind '{kind}'; \
         self-ids must be unique within an issue"
    )]
    DuplicateSelfId {
        /// Short id of the issue whose items collide.
        issue: String,
        /// The duplicated self-id.
        self_id: String,
        /// The kind under which the collision occurred.
        kind: String,
    },
    /// The body could not be parsed for the issue's content format.
    #[error("failed to parse description of issue {issue}: {source}")]
    Parse {
        /// Short id of the issue whose body failed to parse.
        issue: String,
        /// The underlying parser error.
        source: crate::document::ContentParserError,
    },
}

/// A resolved item-kind projection: the `(section, id-pattern, markers,
/// link-namespaces)` four-tuple with all defaults already applied.
///
/// Resolved from an [`ItemKindConfig`] via [`ItemKind::from_config`] (or built
/// directly for the requirement default). The `id_pattern` is pre-compiled so
/// indexing is regex-error-free.
///
/// # Examples
///
/// ```
/// use jit::domain::item::ItemKind;
///
/// let req = ItemKind::requirement_default().unwrap();
/// assert_eq!(req.name(), "requirement");
/// // The kind exposes the same triple the label-coverage rule consumes.
/// let (section, marker, pattern) = req.as_triple();
/// assert_eq!(section, "success_criteria");
/// assert_eq!(marker, Some("[hard]"));
/// assert_eq!(pattern, "[A-Z][A-Z0-9]*-[0-9]+");
/// ```
#[derive(Debug, Clone)]
pub struct ItemKind {
    name: String,
    section: String,
    id_pattern_src: String,
    id_pattern: regex::Regex,
    markers: Vec<String>,
    link_namespaces: Vec<String>,
}

impl ItemKind {
    /// Resolve a configured kind into its four-tuple, applying repo defaults for
    /// any field the config leaves unset.
    ///
    /// `name` labels the kind (for display and `--kind` filtering only). An
    /// invalid `id-pattern` regex is surfaced as [`ItemError::InvalidIdPattern`]
    /// rather than silently dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::ItemKindConfig;
    /// use jit::domain::item::ItemKind;
    ///
    /// let cfg = ItemKindConfig {
    ///     section: Some("decisions".to_string()),
    ///     id_pattern: Some("D-\\d+".to_string()),
    ///     markers: None,
    ///     link_namespaces: Some(vec!["per".to_string()]),
    /// };
    /// let kind = ItemKind::from_config("decision", &cfg).unwrap();
    /// assert_eq!(kind.section(), "decisions");
    /// assert_eq!(kind.link_namespaces(), &["per".to_string()]);
    /// ```
    pub fn from_config(name: &str, config: &ItemKindConfig) -> Result<Self, ItemError> {
        let section = config
            .section
            .clone()
            .unwrap_or_else(|| DEFAULT_ITEM_SECTION.to_string());
        let id_pattern_src = config
            .id_pattern
            .clone()
            .unwrap_or_else(|| DEFAULT_ITEM_ID_PATTERN.to_string());
        let id_pattern =
            regex::Regex::new(&id_pattern_src).map_err(|source| ItemError::InvalidIdPattern {
                kind: name.to_string(),
                pattern: id_pattern_src.clone(),
                source,
            })?;
        let markers = config.markers.clone().unwrap_or_default();
        let link_namespaces = config
            .link_namespaces
            .clone()
            .unwrap_or_else(|| vec![DEFAULT_ITEM_LINK_NAMESPACE.to_string()]);
        Ok(Self {
            name: name.to_string(),
            section,
            id_pattern_src,
            id_pattern,
            markers,
            link_namespaces,
        })
    }

    /// Build the built-in `requirement` kind.
    ///
    /// Its four-tuple is `(success_criteria, [hard], REQ/repo id-pattern,
    /// satisfies)` — exactly the triple the `label-coverage` rule reads with its
    /// own defaults, so the requirement model and the coverage rule are
    /// compatible by construction (REQ-05).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::ItemKind;
    ///
    /// let req = ItemKind::requirement_default().unwrap();
    /// assert_eq!(req.markers(), &["[hard]".to_string()]);
    /// assert_eq!(req.link_namespaces(), &["satisfies".to_string()]);
    /// ```
    pub fn requirement_default() -> Result<Self, ItemError> {
        Self::from_config(
            REQUIREMENT_KIND_NAME,
            &ItemKindConfig {
                section: Some(DEFAULT_ITEM_SECTION.to_string()),
                id_pattern: Some(DEFAULT_ITEM_ID_PATTERN.to_string()),
                markers: Some(vec!["[hard]".to_string()]),
                link_namespaces: Some(vec![DEFAULT_ITEM_LINK_NAMESPACE.to_string()]),
            },
        )
    }

    /// The kind's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The section slug scanned for this kind's items.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// The markers an item must begin with to qualify (empty = any item).
    pub fn markers(&self) -> &[String] {
        &self.markers
    }

    /// The link-label namespaces that reference this kind by qualified id.
    pub fn link_namespaces(&self) -> &[String] {
        &self.link_namespaces
    }

    /// The kind as the `(section, marker, id-pattern)` triple consumed by the
    /// validation engine's `criterion_ids` / `label-coverage` machinery.
    ///
    /// Only the FIRST marker is returned: the engine's coverage rule accepts a
    /// single `marker`, and this triple is what proves model/rule compatibility
    /// (REQ-05). A kind with no markers yields `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::ItemKind;
    ///
    /// let kind = ItemKind::requirement_default().unwrap();
    /// let (section, marker, pattern) = kind.as_triple();
    /// assert_eq!((section, marker), ("success_criteria", Some("[hard]")));
    /// assert!(!pattern.is_empty());
    /// ```
    pub fn as_triple(&self) -> (&str, Option<&str>, &str) {
        (
            &self.section,
            self.markers.first().map(String::as_str),
            &self.id_pattern_src,
        )
    }

    /// Whether an item's text qualifies under this kind's markers.
    ///
    /// True when the kind declares no markers, or the text (after leading
    /// whitespace) begins with ANY declared marker.
    fn marker_matches(&self, text: &str) -> bool {
        self.markers.is_empty()
            || self
                .markers
                .iter()
                .any(|m| text.trim_start().starts_with(m))
    }
}

/// Compute an addressable item's qualified id `<issue-id>/<self-id>`.
///
/// A pure projection over the issue's short id and the parsed self-id; nothing is
/// persisted (REQ-02). The first segment is whatever short-id form the caller
/// passes (use [`Issue::short_id`](crate::domain::Issue::short_id)).
///
/// # Examples
///
/// ```
/// use jit::domain::item::qualified_id;
///
/// assert_eq!(qualified_id("56ab0224", "REQ-01"), "56ab0224/REQ-01");
/// ```
pub fn qualified_id(issue_short_id: &str, self_id: &str) -> String {
    format!("{issue_short_id}/{self_id}")
}

/// Split a qualified id `<scope>/<self-id>` into its two segments.
///
/// Returns `None` when the input carries no `/` separator (it is not a qualified
/// id). Only the FIRST `/` splits, so a self-id may itself contain slashes.
///
/// # Examples
///
/// ```
/// use jit::domain::item::split_qualified_id;
///
/// assert_eq!(split_qualified_id("56ab0224/REQ-01"), Some(("56ab0224", "REQ-01")));
/// assert_eq!(split_qualified_id("REQ-01"), None);
/// ```
pub fn split_qualified_id(qualified: &str) -> Option<(&str, &str)> {
    qualified.split_once('/')
}

/// One addressable item projected from an issue description.
///
/// Carries the *derived* qualified id alongside the source self-id, the owning
/// issue's short id, the kind name, and the raw item text. Nothing here is stored
/// on the issue — it is recomputed by [`index_items`] (REQ-03).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AddressableItem {
    /// The kind this item belongs to (display name).
    pub kind: String,
    /// Globally-unique qualified id `<issue-id>/<self-id>` (derived).
    pub qualified_id: String,
    /// The human-authored self-id, unique within its issue.
    pub self_id: String,
    /// Short id of the issue this item was projected from.
    pub issue_id: String,
    /// The raw text of the source list entry.
    pub text: String,
}

/// Index the addressable items of a single issue across all given kinds.
///
/// A pure projection: parses the issue description with `parser`, and for each
/// kind scans its declared section's list entries, keeping those that match the
/// kind's markers and yield a self-id under its id-pattern. The qualified id is
/// derived as `<issue-short-id>/<self-id>`.
///
/// Self-id uniqueness is enforced WITHIN a kind for the issue: a repeated self-id
/// is an [`ItemError::DuplicateSelfId`]. A list entry with no self-id match is
/// plain prose and is skipped, never an error (REQ-06).
///
/// # Examples
///
/// ```
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_items, ItemKind};
/// use jit::domain::Issue;
///
/// let issue = Issue::new(
///     "T".to_string(),
///     "## Success Criteria\n\n- [hard] REQ-01: a\n- prose line\n".to_string(),
/// );
/// let kinds = vec![ItemKind::requirement_default().unwrap()];
/// let items = index_items(&issue, &kinds, &MarkdownContentParser).unwrap();
/// assert_eq!(items.len(), 1);
/// assert_eq!(items[0].self_id, "REQ-01");
/// assert!(items[0].qualified_id.ends_with("/REQ-01"));
/// ```
pub fn index_items(
    issue: &Issue,
    kinds: &[ItemKind],
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let short_id = issue.short_id();
    let projection = project(issue).with_sections(&issue.description, parser);
    let sections = projection.sections.unwrap_or_default();

    let mut out = Vec::new();
    for kind in kinds {
        let Some(section) = sections.get(kind.section()) else {
            continue;
        };
        let mut seen: HashMap<String, ()> = HashMap::new();
        for text in &section.items {
            if !kind.marker_matches(text) {
                continue;
            }
            // No self-id match means plain prose: skip, never error (REQ-06).
            let Some(self_id) = kind.id_pattern.find(text).map(|m| m.as_str().to_string()) else {
                continue;
            };
            if seen.insert(self_id.clone(), ()).is_some() {
                return Err(ItemError::DuplicateSelfId {
                    issue: short_id.clone(),
                    self_id,
                    kind: kind.name().to_string(),
                });
            }
            out.push(AddressableItem {
                kind: kind.name().to_string(),
                qualified_id: qualified_id(&short_id, &self_id),
                self_id,
                issue_id: short_id.clone(),
                text: text.clone(),
            });
        }
    }
    Ok(out)
}

/// Resolve the effective set of item kinds from an optional config registry.
///
/// When the registry is `None` (no `[item_kinds]` table), the single built-in
/// `requirement` kind is returned, so a repo that declares nothing still has the
/// requirement model. When the registry is present it is used verbatim (each
/// entry resolved through [`ItemKind::from_config`]); the caller opts in to every
/// kind it wants, including re-declaring `requirement` with non-default fields.
///
/// Kinds are returned in name order for deterministic output.
///
/// # Examples
///
/// ```
/// use jit::domain::item::resolve_item_kinds;
///
/// // No registry -> the built-in requirement kind only.
/// let kinds = resolve_item_kinds(None).unwrap();
/// assert_eq!(kinds.len(), 1);
/// assert_eq!(kinds[0].name(), "requirement");
/// ```
pub fn resolve_item_kinds(
    registry: Option<&HashMap<String, ItemKindConfig>>,
) -> Result<Vec<ItemKind>, ItemError> {
    match registry {
        None => Ok(vec![ItemKind::requirement_default()?]),
        Some(map) => {
            let mut names: Vec<&String> = map.keys().collect();
            names.sort();
            names
                .into_iter()
                .map(|name| ItemKind::from_config(name, &map[name]))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::MarkdownContentParser;
    use crate::domain::Issue;

    fn req_kind() -> ItemKind {
        ItemKind::requirement_default().unwrap()
    }

    #[test]
    fn test_qualified_id_is_derived() {
        // REQ-02: qualified id is <issue-id>/<self-id>, a pure projection.
        let mut issue = Issue::new("T".to_string(), String::new());
        issue.id = "56ab0224-fd6e-4929-a61e-ffb1a3104496".to_string();
        assert_eq!(qualified_id(&issue.short_id(), "REQ-01"), "56ab0224/REQ-01");
    }

    #[test]
    fn test_split_qualified_id_roundtrip() {
        let q = qualified_id("56ab0224", "REQ-01");
        assert_eq!(split_qualified_id(&q), Some(("56ab0224", "REQ-01")));
        assert_eq!(split_qualified_id("bare"), None);
    }

    #[test]
    fn test_index_items_projects_requirements() {
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: first\n- [hard] REQ-02: second\n".to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].self_id, "REQ-01");
        assert_eq!(items[0].kind, "requirement");
        assert_eq!(
            items[0].qualified_id,
            qualified_id(&issue.short_id(), "REQ-01")
        );
        assert_eq!(items[1].self_id, "REQ-02");
    }

    #[test]
    fn test_index_items_graceful_degradation() {
        // REQ-06: a list line with no self-id is plain prose, not an error.
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: real\n- [hard] just prose, no id\n"
                .to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].self_id, "REQ-01");
    }

    #[test]
    fn test_index_items_marker_filters_prose() {
        // An unmarked criterion line is ignored by a marker-gated kind.
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: hard one\n- REQ-99: soft, no marker\n"
                .to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].self_id, "REQ-01");
    }

    #[test]
    fn test_index_items_duplicate_self_id_is_error() {
        // REQ-02: self-id uniqueness within an issue is validated.
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: a\n- [hard] REQ-01: dup\n".to_string(),
        );
        let err = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap_err();
        assert!(matches!(err, ItemError::DuplicateSelfId { .. }));
    }

    #[test]
    fn test_index_items_missing_section_is_empty() {
        let issue = Issue::new("T".to_string(), "## Other\n\n- nothing here\n".to_string());
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_generic_kind_no_kind_name_branch() {
        // REQ-01: a kind defined entirely by config (a name the engine has never
        // heard of) indexes purely from its four-tuple, proving no kind name is
        // hardcoded in indexing logic.
        let cfg = ItemKindConfig {
            section: Some("decisions".to_string()),
            id_pattern: Some("D-\\d+".to_string()),
            markers: None,
            link_namespaces: Some(vec!["per".to_string()]),
        };
        let kind = ItemKind::from_config("decision", &cfg).unwrap();
        let issue = Issue::new(
            "T".to_string(),
            "## Decisions\n\n- D-1: use json\n- D-2: atomic writes\n".to_string(),
        );
        let items = index_items(&issue, &[kind], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "decision");
        assert_eq!(items[0].self_id, "D-1");
    }

    #[test]
    fn test_invalid_id_pattern_is_typed_error() {
        let cfg = ItemKindConfig {
            section: None,
            id_pattern: Some("REQ-(".to_string()),
            markers: None,
            link_namespaces: None,
        };
        let err = ItemKind::from_config("broken", &cfg).unwrap_err();
        assert!(matches!(err, ItemError::InvalidIdPattern { .. }));
    }

    #[test]
    fn test_requirement_default_triple_matches_coverage_defaults() {
        // REQ-05: the requirement kind expands to the SAME (section, marker,
        // id-pattern) triple the label-coverage rule uses by default.
        let kind = req_kind();
        let (section, marker, pattern) = kind.as_triple();
        assert_eq!(section, "success_criteria");
        assert_eq!(marker, Some("[hard]"));
        assert_eq!(pattern, "[A-Z][A-Z0-9]*-[0-9]+");
    }

    #[test]
    fn test_resolve_item_kinds_default_when_absent() {
        let kinds = resolve_item_kinds(None).unwrap();
        assert_eq!(kinds.len(), 1);
        assert_eq!(kinds[0].name(), "requirement");
    }

    #[test]
    fn test_resolve_item_kinds_uses_registry() {
        let mut map = HashMap::new();
        map.insert("requirement".to_string(), ItemKindConfig::default());
        map.insert(
            "decision".to_string(),
            ItemKindConfig {
                section: Some("decisions".to_string()),
                id_pattern: Some("D-\\d+".to_string()),
                markers: None,
                link_namespaces: None,
            },
        );
        let kinds = resolve_item_kinds(Some(&map)).unwrap();
        // Returned in name order.
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0].name(), "decision");
        assert_eq!(kinds[1].name(), "requirement");
    }
}
