//! Addressable structured items: a pure projection over issue descriptions.
//!
//! An **addressable item** is a structured list entry in a declared section of an
//! issue description that carries a *self-id* matched by an id-pattern. Its
//! **qualified id** `<scope>/<self-id>` is globally unique and is *derived* from
//! existing data (the resolved scope plus the parsed self-id) — nothing is
//! persisted twice (REQ-02, REQ-03).
//!
//! A **scope** ([`Scope`]) is the first segment of a qualified id. It is either an
//! issue short-id ([`Scope::Issue`]) or the project sentinel `@`
//! ([`Scope::Project`], for items not tied to any single issue). Self-id
//! uniqueness is enforced *per scope*: the same self-id may exist under two
//! different scopes without conflict, but a self-id repeated within one scope is a
//! [`ItemError::DuplicateSelfId`].
//!
//! An **item kind** ([`ItemKind`]) is the config-declared projection
//! `(section, id-pattern, marker(s), link-namespace(s))` that says which entries
//! are addressable and how. `requirement` is just one kind; the model is generic
//! and no kind NAME is interpreted by this module — only the four-tuple is
//! (REQ-01). The built-in defaults ([`builtin_default_kinds`]) ship `requirement`,
//! `decision`, and `risk` markdown-first; [`ItemKind::requirement_default`] reproduces the
//! exact triple the `label-coverage` rule already consumes, so the existing
//! coverage machinery is demonstrably compatible with the model (REQ-05).
//!
//! Indexing is pure and substrate-specific but shares one derivation core
//! ([`derive_scope_items`], which enforces per-scope uniqueness and mints
//! qualified ids): [`index_items`] projects an issue's markdown (markdown is the
//! single source of truth, recomputed on demand), while [`index_markdown_items`]
//! projects a standalone markdown source file (used for project-scope (`@`)
//! kinds, whose source path comes from config). A list entry without a matching
//! self-id is plain prose and incurs no addressing requirement (REQ-06).

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

/// The conventional NAME of the built-in `decision` kind.
///
/// Like [`REQUIREMENT_KIND_NAME`], this only LABELS the default kind for display
/// and `--kind` filtering; indexing never branches on it (the kind is fully
/// described by its tuple), keeping the engine domain-agnostic (REQ-01).
pub const DECISION_KIND_NAME: &str = "decision";

/// Default section slug the built-in `decision` kind authors its items under
/// (`## Decisions`). Decisions live in issue descriptions, markdown-first.
pub const DECISION_KIND_SECTION: &str = "decisions";

/// Default self-id pattern for the built-in `decision` kind (`D-1`, `D-02`, ...).
///
/// Narrower than [`DEFAULT_ITEM_ID_PATTERN`] so an unmarked decisions list does
/// not accidentally claim arbitrary `XXX-NN` ids; decision self-ids are the `D-NN`
/// shape.
pub const DECISION_KIND_ID_PATTERN: &str = "D-[0-9]+";

/// Default link-label namespace referencing the built-in `decision` kind: a
/// `per:<issue>/D-01` label on a node points at the addressed decision.
pub const DECISION_KIND_LINK_NAMESPACE: &str = "per";

/// The conventional NAME of the built-in `risk` kind.
///
/// Like [`REQUIREMENT_KIND_NAME`], this only LABELS the default kind for display
/// and `--kind` filtering; indexing never branches on it (the kind is fully
/// described by its tuple), keeping the engine domain-agnostic (REQ-01).
pub const RISK_KIND_NAME: &str = "risk";

/// Default section slug the built-in `risk` kind authors its items under
/// (`## Risks`). Risks live in issue descriptions, markdown-first.
pub const RISK_KIND_SECTION: &str = "risks";

/// Default self-id pattern for the built-in `risk` kind (`RISK-1`, `RISK-02`, ...).
///
/// Uses the `RISK-` prefix so the unmarked risks section does not accidentally
/// claim generic `XX-NN` ids from other sections; risk self-ids are the
/// `RISK-NN` shape.
pub const RISK_KIND_ID_PATTERN: &str = "RISK-[0-9]+";

/// Link-label namespaces that reference the built-in `risk` kind.
///
/// Both `mitigates:<issue>/RISK-01` (partial mitigation) and
/// `resolves:<issue>/RISK-01` (full resolution) point at the addressed risk item.
pub const RISK_KIND_LINK_NAMESPACES: [&str; 2] = ["mitigates", "resolves"];

/// The scope sentinel that addresses project-level items not tied to any single
/// issue. The first segment of a qualified id equal to this string denotes
/// [`Scope::Project`] (REQ-01).
pub const PROJECT_SCOPE_SENTINEL: &str = "@";

/// The scope half of a qualified id `<scope>/<self-id>`: the substrate an
/// addressable item belongs to (REQ-01).
///
/// A scope is EITHER one issue (addressed by its short-id) or the whole project
/// (the `@` sentinel, for items such as invariants that no single issue owns).
/// Self-id uniqueness is enforced *per scope* (REQ-04), so the same self-id may
/// appear under two distinct scopes without collision.
///
/// The qualified id is a pure projection: [`Scope::prefix`] renders the first
/// segment and nothing about the scope is persisted separately (REQ-05).
///
/// # Examples
///
/// ```
/// use jit::domain::item::Scope;
///
/// // The project sentinel parses to the project scope.
/// assert_eq!(Scope::parse("@"), Scope::Project);
/// assert_eq!(Scope::Project.prefix(), "@");
///
/// // Anything else is an issue scope carrying the (unresolved) short-id form.
/// let issue = Scope::parse("56ab0224");
/// assert_eq!(issue, Scope::Issue("56ab0224".to_string()));
/// assert_eq!(issue.prefix(), "56ab0224");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// An issue scope, carrying the issue's short-id (the qualified-id prefix).
    Issue(String),
    /// The project scope, rendered with the [`PROJECT_SCOPE_SENTINEL`] (`@`).
    Project,
}

impl Scope {
    /// Parse a qualified id's scope segment into a [`Scope`].
    ///
    /// The [`PROJECT_SCOPE_SENTINEL`] (`@`) yields [`Scope::Project`]; every other
    /// segment is taken as an issue scope verbatim (resolution of a short-id /
    /// unique prefix to a full id is a storage concern handled by the caller, not
    /// this pure parser).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::Scope;
    ///
    /// assert_eq!(Scope::parse("@"), Scope::Project);
    /// assert_eq!(Scope::parse("56ab0224"), Scope::Issue("56ab0224".to_string()));
    /// ```
    pub fn parse(segment: &str) -> Self {
        if segment == PROJECT_SCOPE_SENTINEL {
            Scope::Project
        } else {
            Scope::Issue(segment.to_string())
        }
    }

    /// The qualified-id prefix this scope renders to (`@` for the project, the
    /// issue short-id otherwise).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::Scope;
    ///
    /// assert_eq!(Scope::Project.prefix(), "@");
    /// assert_eq!(Scope::Issue("56ab0224".to_string()).prefix(), "56ab0224");
    /// ```
    pub fn prefix(&self) -> &str {
        match self {
            Scope::Project => PROJECT_SCOPE_SENTINEL,
            Scope::Issue(short_id) => short_id,
        }
    }

    /// Whether this is the project scope (`@`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::Scope;
    ///
    /// assert!(Scope::Project.is_project());
    /// assert!(!Scope::Issue("56ab0224".to_string()).is_project());
    /// ```
    pub fn is_project(&self) -> bool {
        matches!(self, Scope::Project)
    }
}

/// The declared addressing scope of an item *kind* (as opposed to a resolved
/// [`Scope`], which carries a concrete issue short-id).
///
/// A kind is either issue-scoped (its items come from issue descriptions) or
/// project-scoped (its items come from a config-declared `source` file and address
/// as `@/<self-id>`). This is the `scope` half of the kind registry's six-tuple
/// the epic builds toward; sibling work adds the remaining `source-of-truth` field
/// without disturbing this one.
///
/// # Examples
///
/// ```
/// use jit::domain::item::KindScope;
///
/// assert_eq!(KindScope::parse(None).unwrap(), KindScope::Issue);
/// assert_eq!(KindScope::parse(Some("project")).unwrap(), KindScope::Project);
/// assert!(KindScope::parse(Some("bogus")).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScope {
    /// Items are projected from issue descriptions (`<issue>/<self-id>`).
    Issue,
    /// Items are projected from a config-declared source file (`@/<self-id>`).
    Project,
}

impl KindScope {
    /// The token a kind declares its issue scope with.
    pub const ISSUE_TOKEN: &'static str = "issue";
    /// The token a kind declares its project scope with.
    pub const PROJECT_TOKEN: &'static str = "project";

    /// Parse a kind's optional `scope` config string.
    ///
    /// `None` defaults to [`KindScope::Issue`] (the prior, issue-scoped behavior).
    /// An unrecognized value is rejected with the kind name supplied by the caller
    /// via [`KindScope::parse_for`]; this `parse` variant uses a placeholder name
    /// in its error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::KindScope;
    ///
    /// assert_eq!(KindScope::parse(None).unwrap(), KindScope::Issue);
    /// assert_eq!(KindScope::parse(Some("issue")).unwrap(), KindScope::Issue);
    /// assert_eq!(KindScope::parse(Some("project")).unwrap(), KindScope::Project);
    /// ```
    pub fn parse(scope: Option<&str>) -> Result<Self, ItemError> {
        Self::parse_for("<kind>", scope)
    }

    /// Parse a kind's optional `scope` config string, naming `kind` in any error.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::KindScope;
    ///
    /// let err = KindScope::parse_for("invariant", Some("global")).unwrap_err();
    /// assert!(err.to_string().contains("invariant"));
    /// ```
    pub fn parse_for(kind: &str, scope: Option<&str>) -> Result<Self, ItemError> {
        match scope {
            None => Ok(KindScope::Issue),
            Some(Self::ISSUE_TOKEN) => Ok(KindScope::Issue),
            Some(Self::PROJECT_TOKEN) => Ok(KindScope::Project),
            Some(other) => Err(ItemError::InvalidScope {
                kind: kind.to_string(),
                scope: other.to_string(),
            }),
        }
    }

    /// Whether this kind is project-scoped (`@`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::KindScope;
    ///
    /// assert!(KindScope::Project.is_project());
    /// assert!(!KindScope::Issue.is_project());
    /// ```
    pub fn is_project(&self) -> bool {
        matches!(self, KindScope::Project)
    }
}

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
    /// Two addressable items in one scope share a self-id, so the qualified id
    /// `<scope>/<self-id>` would not be unique within that scope (REQ-03). The
    /// same self-id under a *different* scope is fine (REQ-04).
    #[error(
        "scope {scope} declares self-id '{self_id}' more than once for kind '{kind}'; \
         self-ids must be unique within a scope"
    )]
    DuplicateSelfId {
        /// The scope whose items collide (issue short-id or `@` for project).
        scope: String,
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
    /// A kind declares an unrecognized `scope` (not `issue` or `project`).
    #[error("item kind '{kind}' has an invalid scope '{scope}'; expected 'issue' or 'project'")]
    InvalidScope {
        /// The offending kind name.
        kind: String,
        /// The unrecognized scope string.
        scope: String,
    },
    /// A `scope = "project"` kind declares no `source` file to read its items
    /// from, so it cannot be indexed at project scope.
    #[error(
        "project-scope item kind '{kind}' declares no 'source' file; \
         a project-scope kind must name a repository-local markdown source"
    )]
    MissingProjectSource {
        /// The offending kind name.
        kind: String,
    },
    /// A reference (e.g. a rule's `kind =` key) names a kind not declared in the
    /// `[item_kinds]` registry, so it cannot be expanded to a triple. Surfaced as
    /// a typed error rather than a silent pass.
    #[error(
        "item kind '{kind}' is not declared in [item_kinds]; \
         declare it or reference an existing kind"
    )]
    UnknownKind {
        /// The undeclared kind name that was referenced.
        kind: String,
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
    kind_scope: KindScope,
    source: Option<String>,
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
    ///     ..Default::default()
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
        let kind_scope = KindScope::parse_for(name, config.scope.as_deref())?;
        // A project-scope kind without a source cannot be indexed; reject at
        // resolution so a misconfigured kind surfaces a typed error, not a silent
        // empty result.
        if kind_scope.is_project() && config.source.is_none() {
            return Err(ItemError::MissingProjectSource {
                kind: name.to_string(),
            });
        }
        Ok(Self {
            name: name.to_string(),
            section,
            id_pattern_src,
            id_pattern,
            markers,
            link_namespaces,
            kind_scope,
            source: config.source.clone(),
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
                ..Default::default()
            },
        )
    }

    /// Build the built-in `decision` kind.
    ///
    /// A markdown-first, issue-scoped kind: decisions are authored as list entries
    /// under a `## Decisions` section of an issue description and indexed through
    /// the SAME generic parse path as `requirement`, with NO marker (every matching
    /// line qualifies). Its tuple is `(decisions, D-[0-9]+, <no marker>, per)`. A
    /// `per:<issue>/D-01` label references a decision by its qualified id. Shipped
    /// as a built-in default (alongside `requirement`) via
    /// [`builtin_default_kinds`], so `jit item list --kind decision` works in a
    /// default-initialized repo with no `[item_kinds]` config.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::ItemKind;
    ///
    /// let decision = ItemKind::decision_default().unwrap();
    /// assert_eq!(decision.name(), "decision");
    /// assert_eq!(decision.section(), "decisions");
    /// // No marker: every D-NN line in the section qualifies.
    /// assert!(decision.markers().is_empty());
    /// assert_eq!(decision.link_namespaces(), &["per".to_string()]);
    /// ```
    pub fn decision_default() -> Result<Self, ItemError> {
        Self::from_config(
            DECISION_KIND_NAME,
            &ItemKindConfig {
                section: Some(DECISION_KIND_SECTION.to_string()),
                id_pattern: Some(DECISION_KIND_ID_PATTERN.to_string()),
                // No marker: a decisions list needs no `[hard]`-style gate.
                markers: Some(vec![]),
                link_namespaces: Some(vec![DECISION_KIND_LINK_NAMESPACE.to_string()]),
                ..Default::default()
            },
        )
    }

    /// Build the built-in `risk` kind.
    ///
    /// A markdown-first, issue-scoped kind: risks are authored as list entries
    /// under a `## Risks` section of an issue description and indexed through the
    /// SAME generic parse path as `requirement` and `decision`, with NO marker
    /// (every matching line qualifies). Its tuple is
    /// `(risks, RISK-[0-9]+, <no marker>, [mitigates, resolves])`.
    ///
    /// Two link namespaces are declared:
    /// - `mitigates:<issue>/RISK-01` — the labelled item partially mitigates the risk.
    /// - `resolves:<issue>/RISK-01` — the labelled item fully resolves the risk.
    ///
    /// Shipped as a built-in default (alongside `requirement` and `decision`) via
    /// [`builtin_default_kinds`], so `jit item list --kind risk` works in a
    /// default-initialized repo with no `[item_kinds]` config.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::ItemKind;
    ///
    /// let risk = ItemKind::risk_default().unwrap();
    /// assert_eq!(risk.name(), "risk");
    /// assert_eq!(risk.section(), "risks");
    /// // No marker: every RISK-NN line in the section qualifies.
    /// assert!(risk.markers().is_empty());
    /// // Both `mitigates:` and `resolves:` namespaces reference risks.
    /// assert_eq!(
    ///     risk.link_namespaces(),
    ///     &["mitigates".to_string(), "resolves".to_string()]
    /// );
    /// ```
    pub fn risk_default() -> Result<Self, ItemError> {
        Self::from_config(
            RISK_KIND_NAME,
            &ItemKindConfig {
                section: Some(RISK_KIND_SECTION.to_string()),
                id_pattern: Some(RISK_KIND_ID_PATTERN.to_string()),
                // No marker: a risks list needs no `[hard]`-style gate.
                markers: Some(vec![]),
                link_namespaces: Some(
                    RISK_KIND_LINK_NAMESPACES
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
                ..Default::default()
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

    /// The kind's declared addressing scope (issue or project).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::{ItemKind, KindScope};
    ///
    /// let req = ItemKind::requirement_default().unwrap();
    /// assert_eq!(req.kind_scope(), KindScope::Issue);
    /// ```
    pub fn kind_scope(&self) -> KindScope {
        self.kind_scope
    }

    /// The repository-local source file a project-scope kind reads its items from,
    /// or `None` for an issue-scope kind.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::item::ItemKind;
    ///
    /// // The built-in requirement kind is issue-scoped and has no source file.
    /// assert_eq!(ItemKind::requirement_default().unwrap().source(), None);
    /// ```
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
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

/// Compute an addressable item's qualified id `<scope>/<self-id>`.
///
/// A pure projection over the scope prefix and the parsed self-id; nothing is
/// persisted (REQ-05). The first segment is whatever scope prefix the caller
/// passes — an issue short-id (use
/// [`Issue::short_id`](crate::domain::Issue::short_id)) or [`Scope::prefix`] for
/// the project sentinel `@`.
///
/// # Examples
///
/// ```
/// use jit::domain::item::{qualified_id, Scope};
///
/// assert_eq!(qualified_id("56ab0224", "REQ-01"), "56ab0224/REQ-01");
/// // Project scope renders with the `@` sentinel.
/// assert_eq!(qualified_id(Scope::Project.prefix(), "INV-01"), "@/INV-01");
/// ```
pub fn qualified_id(scope_prefix: &str, self_id: &str) -> String {
    format!("{scope_prefix}/{self_id}")
}

/// Split a qualified id `<scope>/<self-id>` into its two segments.
///
/// The scope is everything before the FIRST `/`; the self-id is the rest (so a
/// self-id may itself contain slashes). The scope may be an issue short-id or the
/// project sentinel `@` — parse it into a [`Scope`] with [`Scope::parse`].
/// Returns `None` when the input carries no `/` separator (it is not a qualified
/// id).
///
/// # Examples
///
/// ```
/// use jit::domain::item::split_qualified_id;
///
/// assert_eq!(split_qualified_id("56ab0224/REQ-01"), Some(("56ab0224", "REQ-01")));
/// // The project scope splits the same way.
/// assert_eq!(split_qualified_id("@/INV-01"), Some(("@", "INV-01")));
/// assert_eq!(split_qualified_id("REQ-01"), None);
/// ```
pub fn split_qualified_id(qualified: &str) -> Option<(&str, &str)> {
    qualified.split_once('/')
}

/// One addressable item projected from a scope's source.
///
/// Carries the *derived* qualified id alongside the source self-id, the owning
/// scope prefix (an issue short-id or `@`), the kind name, and the raw item text.
/// Nothing here is stored — it is recomputed on demand by the indexers (REQ-05).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AddressableItem {
    /// The kind this item belongs to (display name).
    pub kind: String,
    /// Globally-unique qualified id `<scope>/<self-id>` (derived).
    pub qualified_id: String,
    /// The human-authored self-id, unique within its scope.
    pub self_id: String,
    /// Scope prefix this item was projected from: an issue short-id, or `@` for
    /// the project scope.
    pub scope: String,
    /// The raw text of the source list entry.
    pub text: String,
}

/// A candidate addressable item extracted from a scope's source, before per-scope
/// uniqueness has been enforced and the qualified id derived.
///
/// This is the single shape every substrate (issue-scope markdown, project-scope
/// registry) funnels into [`derive_scope_items`], so the dedup + qualified-id
/// derivation lives in exactly one place (REQ-03, REQ-04, REQ-05).
///
/// # Examples
///
/// ```
/// use jit::domain::item::RawScopeItem;
///
/// let raw = RawScopeItem {
///     kind: "invariant".to_string(),
///     self_id: "INV-01".to_string(),
///     text: "all writes are atomic".to_string(),
/// };
/// assert_eq!(raw.self_id, "INV-01");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawScopeItem {
    /// The kind this candidate belongs to (display name).
    pub kind: String,
    /// The human-authored self-id.
    pub self_id: String,
    /// The raw source text of the item.
    pub text: String,
}

/// Enforce per-scope self-id uniqueness over raw candidates and derive their
/// qualified ids `<scope>/<self-id>` (REQ-03, REQ-04, REQ-05).
///
/// This is the one code path that turns extracted candidates into addressable
/// items, shared by [`index_items`] (issue scope) and [`index_markdown_items`]
/// (any scope, including project). Uniqueness is keyed on the self-id alone (the kind is not part
/// of the qualified id), so two kinds minting the same self-id in one scope is a
/// [`ItemError::DuplicateSelfId`] naming the kind that first claimed it. The same
/// self-id under a *different* scope is fine because each call is scoped to one
/// [`Scope`] (REQ-04).
///
/// # Examples
///
/// ```
/// use jit::domain::item::{derive_scope_items, RawScopeItem, Scope};
///
/// let raw = vec![RawScopeItem {
///     kind: "invariant".to_string(),
///     self_id: "INV-01".to_string(),
///     text: "atomic writes".to_string(),
/// }];
/// let items = derive_scope_items(&Scope::Project, raw).unwrap();
/// assert_eq!(items[0].qualified_id, "@/INV-01");
/// assert_eq!(items[0].scope, "@");
/// ```
pub fn derive_scope_items(
    scope: &Scope,
    raw: Vec<RawScopeItem>,
) -> Result<Vec<AddressableItem>, ItemError> {
    let prefix = scope.prefix();
    let mut out = Vec::with_capacity(raw.len());
    // Maps each claimed self-id to the kind that first claimed it, for a precise
    // collision message. Scoped to this single scope, so the same self-id under a
    // different scope never collides here (REQ-04).
    let mut seen: HashMap<String, String> = HashMap::new();
    for candidate in raw {
        if let Some(prior_kind) = seen.insert(candidate.self_id.clone(), candidate.kind.clone()) {
            return Err(ItemError::DuplicateSelfId {
                scope: prefix.to_string(),
                self_id: candidate.self_id,
                // Name the kind that FIRST claimed the self-id so a cross-kind
                // collision points at the original owner.
                kind: prior_kind,
            });
        }
        out.push(AddressableItem {
            qualified_id: qualified_id(prefix, &candidate.self_id),
            scope: prefix.to_string(),
            kind: candidate.kind,
            self_id: candidate.self_id,
            text: candidate.text,
        });
    }
    Ok(out)
}

/// Index the addressable items of a single issue across all given kinds.
///
/// A pure projection: parses the issue description with `parser`, and for each
/// kind scans its declared section's list entries, keeping those that match the
/// kind's markers and yield a self-id under its id-pattern. The qualified id is
/// derived as `<issue-short-id>/<self-id>` via the shared [`derive_scope_items`].
///
/// Self-id uniqueness is enforced PER SCOPE (here, the issue): a repeated self-id
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
    let projection = project(issue).with_sections(&issue.description, parser);
    let sections = projection.sections.unwrap_or_default();
    let raw = extract_raw_items(&sections, kinds);
    derive_scope_items(&Scope::Issue(issue.short_id()), raw)
}

/// Extract raw item candidates from already-parsed sections across all kinds.
///
/// The single section-scanning path shared by every markdown substrate
/// ([`index_items`] for issue descriptions, [`index_markdown_items`] for a
/// project-scope source file): for each kind it scans its declared section's list
/// entries, keeps those that match the kind's markers, and extracts the self-id
/// under its id-pattern. A line with no self-id match is plain prose and is
/// skipped, never an error (REQ-06). Per-scope uniqueness is NOT enforced here —
/// that is [`derive_scope_items`]' job — so this stays a pure, reusable scanner.
fn extract_raw_items(
    sections: &std::collections::BTreeMap<String, crate::domain::ProjectedSection>,
    kinds: &[ItemKind],
) -> Vec<RawScopeItem> {
    kinds
        .iter()
        .filter_map(|kind| sections.get(kind.section()).map(|section| (kind, section)))
        .flat_map(|(kind, section)| {
            section.items.iter().filter_map(move |text| {
                if !kind.marker_matches(text) {
                    return None;
                }
                // No self-id match means plain prose: skip, never error (REQ-06).
                let self_id = kind.id_pattern.find(text).map(|m| m.as_str().to_string())?;
                Some(RawScopeItem {
                    kind: kind.name().to_string(),
                    self_id,
                    text: text.clone(),
                })
            })
        })
        .collect()
}

/// Index the addressable items of a scope from a standalone markdown source.
///
/// Project-scoped kinds are markdown-first, sourced from a repository-local file
/// declared in config (the path comes only from config — no filename is
/// hardcoded). This parses `markdown` with the SAME [`ContentParser`] and
/// section-scanning path ([`extract_raw_items`]) as issue descriptions, then runs
/// the candidates through the SAME [`derive_scope_items`] derivation, so
/// qualified-id derivation and per-scope uniqueness (REQ-03, REQ-04, REQ-05) are
/// identical across substrates. With `scope = Scope::Project` each item's
/// qualified id is `@/<self-id>` and resolution of `@/<self-id>` finds it
/// (REQ-01).
///
/// # Examples
///
/// ```
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_markdown_items, ItemKind, Scope};
///
/// let kinds = vec![ItemKind::requirement_default().unwrap()];
/// let md = "## Success Criteria\n\n- [hard] INV-01: all writes are atomic\n";
/// let items =
///     index_markdown_items(md, &Scope::Project, &kinds, &MarkdownContentParser).unwrap();
/// assert_eq!(items[0].qualified_id, "@/INV-01");
/// ```
pub fn index_markdown_items(
    markdown: &str,
    scope: &Scope,
    kinds: &[ItemKind],
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let sections = sections_from_markdown(markdown, parser);
    let raw = extract_raw_items(&sections, kinds);
    derive_scope_items(scope, raw)
}

/// Parse standalone markdown into the section map [`extract_raw_items`] consumes.
///
/// A thin wrapper over the [`ContentParser`] reusing the exact section model the
/// issue projection uses, so a project-scope source file is scanned identically to
/// an issue description.
fn sections_from_markdown(
    markdown: &str,
    parser: &dyn ContentParser,
) -> std::collections::BTreeMap<String, crate::domain::ProjectedSection> {
    parser
        .parse(markdown)
        .sections
        .into_iter()
        .map(|(name, section)| (name, section.into()))
        .collect()
}

/// One project-scope source: a kind paired with the markdown text of its
/// config-declared `source` file.
///
/// [`index_project_sources`] consumes these so several project-scope kinds (each
/// reading its own file) are deduped together under the single `@` scope.
///
/// # Examples
///
/// ```
/// use jit::domain::item::{ItemKind, ProjectSource};
///
/// let src = ProjectSource {
///     kind: ItemKind::requirement_default().unwrap(),
///     markdown: "## Success Criteria\n\n- [hard] INV-01: x\n".to_string(),
/// };
/// assert_eq!(src.kind.name(), "requirement");
/// ```
#[derive(Debug, Clone)]
pub struct ProjectSource {
    /// The project-scope kind whose items this source holds.
    pub kind: ItemKind,
    /// The markdown text of the kind's `source` file (already read from disk).
    pub markdown: String,
}

/// Index every project-scope (`@`) source through ONE per-scope dedup pass.
///
/// Each [`ProjectSource`] is parsed and scanned (via [`extract_raw_items`]) with
/// the SAME path as issue descriptions, then ALL candidates across all sources are
/// run through a single [`derive_scope_items`] at [`Scope::Project`]. Pooling the
/// candidates before deriving means a self-id repeated across two project kinds is
/// reported as a duplicate (REQ-03), and qualified-id derivation matches issue
/// scope (REQ-01, REQ-05). An empty input yields no items (graceful), never an
/// error.
///
/// # Examples
///
/// ```
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_project_sources, ItemKind, ProjectSource};
///
/// let sources = vec![ProjectSource {
///     kind: ItemKind::requirement_default().unwrap(),
///     markdown: "## Success Criteria\n\n- [hard] INV-01: atomic writes\n".to_string(),
/// }];
/// let items = index_project_sources(&sources, &MarkdownContentParser).unwrap();
/// assert_eq!(items[0].qualified_id, "@/INV-01");
/// ```
pub fn index_project_sources(
    sources: &[ProjectSource],
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let raw: Vec<RawScopeItem> = sources
        .iter()
        .flat_map(|source| {
            let sections = sections_from_markdown(&source.markdown, parser);
            extract_raw_items(&sections, std::slice::from_ref(&source.kind))
        })
        .collect();
    derive_scope_items(&Scope::Project, raw)
}

/// The built-in item kinds a repo has when it declares no `[item_kinds]` table.
///
/// These are the kinds JIT ships out of the box, each authored markdown-first in
/// issue descriptions: `requirement` (`## Success Criteria`), `decision`
/// (`## Decisions`), and `risk` (`## Risks`). Returned in name order for
/// deterministic output. A project's explicit `[item_kinds]` table REPLACES this
/// set (see [`resolve_item_kinds`]).
///
/// This is the single extension point for shipping further built-in kinds:
/// appending a `*_default()` constructor here makes the new kind available in
/// every default-initialized repo with no further wiring.
///
/// # Examples
///
/// ```
/// use jit::domain::item::builtin_default_kinds;
///
/// let kinds = builtin_default_kinds().unwrap();
/// let names: Vec<&str> = kinds.iter().map(|k| k.name()).collect();
/// // Shipped built-ins, in name order.
/// assert_eq!(names, vec!["decision", "requirement", "risk"]);
/// ```
pub fn builtin_default_kinds() -> Result<Vec<ItemKind>, ItemError> {
    // Keep in name order so the default set matches the registry path's ordering.
    let mut kinds = vec![
        ItemKind::requirement_default()?,
        ItemKind::decision_default()?,
        ItemKind::risk_default()?,
    ];
    kinds.sort_by(|a, b| a.name().cmp(b.name()));
    Ok(kinds)
}

/// Resolve the effective set of item kinds from an optional config registry.
///
/// When the registry is `None` (no `[item_kinds]` table), the built-in default
/// kinds ([`builtin_default_kinds`] — `requirement`, `decision`, and `risk`) are
/// returned, so a repo that declares nothing still ships those models. When the
/// registry is present it is used verbatim (each entry resolved through
/// [`ItemKind::from_config`]); the caller opts in to every kind it wants,
/// including re-declaring `requirement` with non-default fields. An explicit table
/// REPLACES the defaults.
///
/// Kinds are returned in name order for deterministic output.
///
/// # Examples
///
/// ```
/// use jit::domain::item::resolve_item_kinds;
///
/// // No registry -> the built-in default kinds, in name order.
/// let kinds = resolve_item_kinds(None).unwrap();
/// let names: Vec<&str> = kinds.iter().map(|k| k.name()).collect();
/// assert_eq!(names, vec!["decision", "requirement", "risk"]);
/// ```
pub fn resolve_item_kinds(
    registry: Option<&HashMap<String, ItemKindConfig>>,
) -> Result<Vec<ItemKind>, ItemError> {
    match registry {
        None => builtin_default_kinds(),
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

/// The `(section, marker, id-pattern)` triple a named kind expands to, as owned
/// strings ready to splice into a free-form rule config.
///
/// This is the SAME triple [`ItemKind::as_triple`] exposes (the engine's
/// `criterion_ids` / `label-coverage` machinery consumes exactly these three
/// keys), captured here as owned values so a config layer can rewrite a rule's
/// assert table without holding a borrow. `marker` is `None` when the kind
/// declares no marker.
///
/// # Examples
///
/// ```
/// use jit::domain::item::KindTriple;
///
/// let triple = KindTriple {
///     section: "success_criteria".to_string(),
///     marker: Some("[hard]".to_string()),
///     id_pattern: "REQ-\\d+".to_string(),
/// };
/// assert_eq!(triple.marker.as_deref(), Some("[hard]"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindTriple {
    /// Section slug whose list items hold the kind's items.
    pub section: String,
    /// The kind's first marker, or `None` when it declares none.
    pub marker: Option<String>,
    /// Regex extracting a self-id from an item's text.
    pub id_pattern: String,
}

/// Expand a named kind from the config registry into its `(section, marker,
/// id-pattern)` [`KindTriple`].
///
/// This is the ONE kind→triple resolver shared by every config layer that offers
/// `kind =` sugar: it resolves the kind through [`ItemKind::from_config`] (so
/// repo defaults and id-pattern validation are applied identically to indexing)
/// and returns the same triple [`ItemKind::as_triple`] exposes. The engine then
/// consumes the triple and never sees the kind NAME, keeping it domain-agnostic
/// (REQ-05). A name absent from `registry` is an [`ItemError::UnknownKind`]; a
/// `None` registry has no declared kinds, so any name is unknown.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use jit::config::ItemKindConfig;
/// use jit::domain::item::{expand_kind_triple, ItemError};
///
/// let mut registry = HashMap::new();
/// registry.insert(
///     "requirement".to_string(),
///     ItemKindConfig {
///         section: Some("success_criteria".to_string()),
///         markers: Some(vec!["[hard]".to_string()]),
///         id_pattern: Some("REQ-\\d+".to_string()),
///         ..Default::default()
///     },
/// );
/// let triple = expand_kind_triple(Some(&registry), "requirement").unwrap();
/// assert_eq!(triple.section, "success_criteria");
/// assert_eq!(triple.marker.as_deref(), Some("[hard]"));
///
/// // An undeclared name is a typed error, not a silent pass.
/// let err = expand_kind_triple(Some(&registry), "bogus").unwrap_err();
/// assert!(matches!(err, ItemError::UnknownKind { .. }));
/// ```
pub fn expand_kind_triple(
    registry: Option<&HashMap<String, ItemKindConfig>>,
    name: &str,
) -> Result<KindTriple, ItemError> {
    let config = registry
        .and_then(|map| map.get(name))
        .ok_or_else(|| ItemError::UnknownKind {
            kind: name.to_string(),
        })?;
    let kind = ItemKind::from_config(name, config)?;
    let (section, marker, id_pattern) = kind.as_triple();
    Ok(KindTriple {
        section: section.to_string(),
        marker: marker.map(str::to_string),
        id_pattern: id_pattern.to_string(),
    })
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
    fn test_index_items_cross_kind_self_id_collision_is_error() {
        // REQ-02: two DIFFERENT kinds minting the same self-id would yield the
        // same qualified id <issue>/REQ-01 (kind is not part of the qualified id),
        // so uniqueness is enforced across ALL kinds within the issue.
        let other = ItemKind::from_config(
            "decision",
            &ItemKindConfig {
                // Read the SAME section so both kinds see REQ-01.
                section: Some(DEFAULT_ITEM_SECTION.to_string()),
                id_pattern: Some("REQ-\\d+".to_string()),
                markers: None,
                link_namespaces: None,
                ..Default::default()
            },
        )
        .unwrap();
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: a\n".to_string(),
        );
        // The marker-gated requirement kind claims REQ-01 first; the unmarked
        // `decision` kind then re-claims it from the same line → collision.
        let err = index_items(&issue, &[req_kind(), other], &MarkdownContentParser).unwrap_err();
        match err {
            ItemError::DuplicateSelfId { self_id, kind, .. } => {
                assert_eq!(self_id, "REQ-01");
                // The error names the kind that FIRST claimed the self-id.
                assert_eq!(kind, "requirement");
            }
            other => panic!("expected DuplicateSelfId, got {other:?}"),
        }
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
            ..Default::default()
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
            ..Default::default()
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
        // With no registry, the built-in default set ships requirement, decision,
        // and risk, in name order.
        let kinds = resolve_item_kinds(None).unwrap();
        let names: Vec<&str> = kinds.iter().map(ItemKind::name).collect();
        assert_eq!(names, vec!["decision", "requirement", "risk"]);
    }

    #[test]
    fn test_decision_default_tuple() {
        // The built-in decision kind: section `decisions`, D-NN ids, no marker,
        // `per` link namespace, issue-scoped, markdown-first.
        let decision = ItemKind::decision_default().unwrap();
        assert_eq!(decision.name(), "decision");
        assert_eq!(decision.section(), "decisions");
        assert!(decision.markers().is_empty());
        assert_eq!(decision.link_namespaces(), &["per".to_string()]);
        assert_eq!(decision.kind_scope(), KindScope::Issue);
        let (section, marker, pattern) = decision.as_triple();
        assert_eq!(section, "decisions");
        assert_eq!(marker, None);
        assert_eq!(pattern, "D-[0-9]+");
    }

    #[test]
    fn test_builtin_default_kinds_includes_decision() {
        let names: Vec<String> = builtin_default_kinds()
            .unwrap()
            .iter()
            .map(|k| k.name().to_string())
            .collect();
        assert!(names.contains(&"requirement".to_string()));
        assert!(names.contains(&"decision".to_string()));
    }

    #[test]
    fn test_index_items_projects_decisions_by_default() {
        // The default kind set indexes a `## Decisions` section's D-NN lines with
        // no extra config (the shipped product behavior).
        let issue = Issue::new(
            "T".to_string(),
            "## Decisions\n\n- D-01: use json\n- D-02: atomic writes\n".to_string(),
        );
        let kinds = builtin_default_kinds().unwrap();
        let items = index_items(&issue, &kinds, &MarkdownContentParser).unwrap();
        let decisions: Vec<&AddressableItem> =
            items.iter().filter(|i| i.kind == "decision").collect();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].self_id, "D-01");
        assert!(decisions[0].qualified_id.ends_with("/D-01"));
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
                ..Default::default()
            },
        );
        let kinds = resolve_item_kinds(Some(&map)).unwrap();
        // Returned in name order.
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0].name(), "decision");
        assert_eq!(kinds[1].name(), "requirement");
    }

    #[test]
    fn test_expand_kind_triple_matches_as_triple() {
        // REQ-02: expanding a named kind yields the SAME triple as_triple exposes,
        // so the `kind=` sugar and the inline form resolve identically.
        let mut registry = HashMap::new();
        registry.insert(
            "requirement".to_string(),
            ItemKindConfig {
                section: Some("success_criteria".to_string()),
                markers: Some(vec!["[hard]".to_string()]),
                id_pattern: Some("REQ-\\d+".to_string()),
                ..Default::default()
            },
        );
        let triple = expand_kind_triple(Some(&registry), "requirement").unwrap();
        let kind = ItemKind::from_config("requirement", &registry["requirement"]).unwrap();
        let (section, marker, pattern) = kind.as_triple();
        assert_eq!(triple.section, section);
        assert_eq!(triple.marker.as_deref(), marker);
        assert_eq!(triple.id_pattern, pattern);
    }

    #[test]
    fn test_expand_kind_triple_applies_defaults() {
        // A minimally-declared kind expands with the repo defaults applied.
        let mut registry = HashMap::new();
        registry.insert("requirement".to_string(), ItemKindConfig::default());
        let triple = expand_kind_triple(Some(&registry), "requirement").unwrap();
        assert_eq!(triple.section, DEFAULT_ITEM_SECTION);
        assert_eq!(triple.id_pattern, DEFAULT_ITEM_ID_PATTERN);
        // No markers declared -> no marker in the triple.
        assert_eq!(triple.marker, None);
    }

    #[test]
    fn test_expand_kind_triple_unknown_is_error() {
        // The constraint: an undeclared kind reference is a typed error.
        let registry: HashMap<String, ItemKindConfig> = HashMap::new();
        let err = expand_kind_triple(Some(&registry), "requirement").unwrap_err();
        assert!(matches!(err, ItemError::UnknownKind { ref kind } if kind == "requirement"));

        // A None registry likewise has no declared kinds.
        let err = expand_kind_triple(None, "requirement").unwrap_err();
        assert!(matches!(err, ItemError::UnknownKind { .. }));
    }

    #[test]
    fn test_expand_kind_triple_propagates_invalid_pattern() {
        let mut registry = HashMap::new();
        registry.insert(
            "broken".to_string(),
            ItemKindConfig {
                id_pattern: Some("REQ-(".to_string()),
                ..Default::default()
            },
        );
        let err = expand_kind_triple(Some(&registry), "broken").unwrap_err();
        assert!(matches!(err, ItemError::InvalidIdPattern { .. }));
    }

    fn raw(kind: &str, self_id: &str) -> RawScopeItem {
        RawScopeItem {
            kind: kind.to_string(),
            self_id: self_id.to_string(),
            text: format!("{self_id} text"),
        }
    }

    #[test]
    fn test_scope_parse_and_prefix() {
        // REQ-01: `@` is the project scope sentinel; anything else is an issue scope.
        assert_eq!(Scope::parse("@"), Scope::Project);
        assert!(Scope::parse("@").is_project());
        assert_eq!(Scope::Project.prefix(), "@");

        let issue = Scope::parse("56ab0224");
        assert_eq!(issue, Scope::Issue("56ab0224".to_string()));
        assert!(!issue.is_project());
        assert_eq!(issue.prefix(), "56ab0224");
    }

    #[test]
    fn test_derive_scope_items_derives_at_project_scope() {
        // REQ-01: project-scoped candidates resolve under the `@` prefix.
        let items = derive_scope_items(&Scope::Project, vec![raw("invariant", "INV-01")]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].qualified_id, "@/INV-01");
        assert_eq!(items[0].scope, "@");
        assert_eq!(items[0].self_id, "INV-01");
    }

    #[test]
    fn test_index_markdown_items_at_project_scope() {
        // REQ-01: a markdown source scanned at project scope mints `@/<self-id>`,
        // through the SAME parse + extract + derive path as issue scope.
        let kinds = vec![req_kind()];
        let md = "## Success Criteria\n\n- [hard] INV-01: all writes are atomic\n- prose line\n";
        let items =
            index_markdown_items(md, &Scope::Project, &kinds, &MarkdownContentParser).unwrap();
        // The prose line without a self-id is skipped (REQ-06).
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].qualified_id, "@/INV-01");
        assert_eq!(items[0].scope, "@");
    }

    #[test]
    fn test_derive_scope_items_duplicate_within_scope_is_error() {
        // REQ-03: a self-id repeated within ONE scope is a duplicate, not silently
        // resolved to one.
        let err = derive_scope_items(
            &Scope::Project,
            vec![raw("invariant", "INV-01"), raw("invariant", "INV-01")],
        )
        .unwrap_err();
        match err {
            ItemError::DuplicateSelfId { scope, self_id, .. } => {
                assert_eq!(scope, "@");
                assert_eq!(self_id, "INV-01");
            }
            other => panic!("expected DuplicateSelfId, got {other:?}"),
        }
    }

    #[test]
    fn test_derive_scope_items_cross_kind_duplicate_within_scope_is_error() {
        // REQ-03: the qualified id omits the kind, so two kinds minting the same
        // self-id in one scope still collide; the error names the FIRST claimer.
        let err = derive_scope_items(
            &Scope::Project,
            vec![raw("invariant", "X-1"), raw("decision", "X-1")],
        )
        .unwrap_err();
        match err {
            ItemError::DuplicateSelfId { kind, self_id, .. } => {
                assert_eq!(self_id, "X-1");
                assert_eq!(kind, "invariant");
            }
            other => panic!("expected DuplicateSelfId, got {other:?}"),
        }
    }

    #[test]
    fn test_same_self_id_distinct_across_scopes() {
        // REQ-04: the SAME self-id under two different scopes does not conflict and
        // yields two distinct qualified ids.
        let issue = Issue::new(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: issue one\n".to_string(),
        );
        let issue_items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        let project_items =
            derive_scope_items(&Scope::Project, vec![raw("requirement", "REQ-01")]).unwrap();

        assert_eq!(issue_items.len(), 1);
        assert_eq!(project_items.len(), 1);
        // Distinct qualified ids: issue-scope prefix vs `@`.
        assert_ne!(issue_items[0].qualified_id, project_items[0].qualified_id);
        assert_eq!(issue_items[0].self_id, project_items[0].self_id);
        assert_eq!(project_items[0].qualified_id, "@/REQ-01");
        assert!(issue_items[0].qualified_id.ends_with("/REQ-01"));
        assert!(!issue_items[0].qualified_id.starts_with('@'));
    }
}
