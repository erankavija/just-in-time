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
//! `(section, id-pattern, marker(s), link-namespace(s), scope, source-of-truth)`
//! that says which entries are addressable and how. The model is generic: no kind
//! NAME is interpreted by this module, only the tuple (REQ-01). Kinds are authored
//! entirely in the `[item_kinds]` config table (scaffolded by `jit init`); this
//! module bakes in no domain defaults. With no `[item_kinds]` table the kind set
//! is empty (single-consumer design, no backward-compat layer — see
//! [`resolve_item_kinds`]). A kind's tuple is chosen to align with the
//! `label-coverage` rule's own defaults, so the coverage machinery is compatible
//! with the model without rewriting any rule (REQ-05).
//!
//! Indexing is pure and substrate-specific but shares one derivation core
//! ([`derive_scope_items`], which enforces per-scope uniqueness and mints
//! qualified ids): [`index_items`] projects an issue's markdown (markdown is the
//! single source of truth, recomputed on demand), while [`index_markdown_items`]
//! projects a standalone markdown source file (used for project-scope (`@`)
//! kinds, whose source path comes from config). A list entry without a matching
//! self-id is plain prose and incurs no addressing requirement (REQ-06).

use crate::config::{
    ItemKindConfig, ItemKindSource, KindScopeConfig, SourceOfTruth, TomlSourceDescriptor,
};
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
    /// let err = KindScope::parse_for("example", Some("global")).unwrap_err();
    /// assert!(err.to_string().contains("example"));
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
    /// A `scope = "project"` kind declares no `source` for the substrate its
    /// `source-of-truth` reads from, so it cannot be indexed at project scope: a
    /// markdown-first kind needs a markdown `source` path, a registry-first kind
    /// needs a structured toml `source` descriptor. Either way an absent source
    /// declaration is a typed error rather than a silent empty result.
    #[error(
        "project-scope item kind '{kind}' declares no 'source'; \
         a project-scope kind must name a repository-local source \
         (a markdown file for markdown-first, or a toml descriptor for registry-first)"
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
    /// The `.toml` file backing a registry-first kind's source descriptor is not
    /// valid TOML, so its entries cannot be projected.
    #[error("item kind '{kind}' toml source '{path}' is not valid TOML: {source}")]
    TomlSourceParse {
        /// The kind whose descriptor names the file.
        kind: String,
        /// The repository-local path of the offending `.toml` file.
        path: String,
        /// The underlying TOML parse error (boxed to keep [`ItemError`] small).
        source: Box<toml::de::Error>,
    },
    /// An entry of a registry-first kind's source table is missing a field the
    /// descriptor maps (its `id-field` or `text-field`), so no self-id / text can
    /// be projected for that entry.
    #[error(
        "item kind '{kind}' toml source table '{table}' has an entry missing required \
         field '{field}'"
    )]
    TomlSourceMissingField {
        /// The kind whose descriptor names the table.
        kind: String,
        /// The array-of-tables key being projected.
        table: String,
        /// The descriptor-mapped field absent from the entry.
        field: String,
    },
    /// A descriptor-mapped field of a registry-first kind's source has an
    /// unexpected TOML type (e.g. an `id-field` that is not a string, or a
    /// `link-fields` value that is neither a string nor an array of strings).
    #[error(
        "item kind '{kind}' toml source table '{table}' field '{field}' has an \
         unexpected type: expected {expected}"
    )]
    TomlSourceFieldType {
        /// The kind whose descriptor names the table.
        kind: String,
        /// The array-of-tables key being projected.
        table: String,
        /// The mapped field with the wrong type.
        field: String,
        /// A short description of the type(s) the loader accepts.
        expected: String,
    },
}

/// A resolved item-kind projection: the `(section, id-pattern, markers,
/// link-namespaces)` four-tuple with all defaults already applied.
///
/// Resolved from an [`ItemKindConfig`] via [`ItemKind::from_config`]. The
/// `id_pattern` is pre-compiled so indexing is regex-error-free.
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
/// use jit::domain::item::ItemKind;
///
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         link_namespaces: Some(vec!["satisfies".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// assert_eq!(kind.name(), "example");
/// // The kind exposes the same triple the label-coverage rule consumes.
/// let (section, marker, pattern) = kind.as_triple();
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
    source_path: Option<String>,
    toml_source: Option<TomlSourceDescriptor>,
    source_of_truth: SourceOfTruth,
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
    ///     section: Some("glossary".to_string()),
    ///     id_pattern: Some("G-\\d+".to_string()),
    ///     markers: None,
    ///     link_namespaces: Some(vec!["defines".to_string()]),
    ///     ..Default::default()
    /// };
    /// let kind = ItemKind::from_config("example", &cfg).unwrap();
    /// assert_eq!(kind.section(), "glossary");
    /// assert_eq!(kind.link_namespaces(), &["defines".to_string()]);
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
        // Scope is now a typed field — invalid tokens are rejected at TOML parse
        // time, so this conversion is infallible.
        let kind_scope = match config.scope {
            None | Some(KindScopeConfig::Issue) => KindScope::Issue,
            Some(KindScopeConfig::Project) => KindScope::Project,
        };
        let source_of_truth = config.source_of_truth();
        // Split the polymorphic `source` into the markdown PATH and the structured
        // toml DESCRIPTOR; at most one is ever set (the two shapes are mutually
        // exclusive at parse time).
        let (source_path, toml_source) = match &config.source {
            Some(ItemKindSource::Path(path)) => (Some(path.clone()), None),
            Some(ItemKindSource::Toml(descriptor)) => (None, Some(descriptor.clone())),
            None => (None, None),
        };
        // A project-scope kind must declare the `source` its `source-of-truth`
        // reads from, or it cannot be indexed at project scope: a markdown-first
        // kind needs a markdown `source` PATH, a registry-first kind needs a
        // structured toml `source` DESCRIPTOR. Reject a missing declaration at
        // resolution so a misconfigured kind surfaces a typed error, not a silent
        // empty result. The check is symmetric across both directions and branches
        // only on `source-of-truth`, never on any kind NAME, so the engine
        // hardcodes no domain concept (REQ-03).
        let has_source = match source_of_truth {
            SourceOfTruth::MarkdownFirst => source_path.is_some(),
            SourceOfTruth::RegistryFirst => toml_source.is_some(),
        };
        if kind_scope.is_project() && !has_source {
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
            source_path,
            toml_source,
            source_of_truth,
        })
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
    /// use jit::config::ItemKindConfig;
    /// use jit::domain::item::{ItemKind, KindScope};
    ///
    /// let kind = ItemKind::from_config("example", &ItemKindConfig::default()).unwrap();
    /// assert_eq!(kind.kind_scope(), KindScope::Issue);
    /// ```
    pub fn kind_scope(&self) -> KindScope {
        self.kind_scope
    }

    /// The repository-local MARKDOWN source file a project-scope, markdown-first
    /// kind reads its items from, or `None` for an issue-scope kind or a
    /// registry-first kind backed by a [`toml_source`](Self::toml_source)
    /// descriptor.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::ItemKindConfig;
    /// use jit::domain::item::ItemKind;
    ///
    /// // An issue-scoped kind has no project source file.
    /// let kind = ItemKind::from_config("example", &ItemKindConfig::default()).unwrap();
    /// assert_eq!(kind.source(), None);
    /// ```
    pub fn source(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    /// The structured TOML source descriptor a registry-first project kind reads
    /// its items from, or `None` for an issue-scope or markdown-first kind. A
    /// registry-first project kind returns `Some(descriptor)` naming the `.toml`
    /// registry its items are projected from (its `source` is a
    /// `{ toml = "...", table = "...", ... }` descriptor).
    ///
    /// When present, [`commands`](crate::commands) reads the descriptor's `toml`
    /// file through the storage boundary and projects each table entry into an
    /// addressable item via [`load_toml_scope_items`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::ItemKindConfig;
    /// use jit::domain::item::ItemKind;
    ///
    /// // A markdown-first kind has no toml source descriptor.
    /// let kind = ItemKind::from_config("example", &ItemKindConfig::default()).unwrap();
    /// assert!(kind.toml_source().is_none());
    /// ```
    pub fn toml_source(&self) -> Option<&crate::config::TomlSourceDescriptor> {
        self.toml_source.as_ref()
    }

    /// The kind's authoring DIRECTION (which substrate is canonical).
    ///
    /// `markdown-first` kinds are parsed from a markdown substrate (issue
    /// descriptions, or a project-scope `source` file); `registry-first` kinds are
    /// projected from a structured registry. Callers route the sourcing path on
    /// this value.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{ItemKindConfig, SourceOfTruth};
    /// use jit::domain::item::ItemKind;
    ///
    /// // A kind defaults to markdown-first.
    /// let kind = ItemKind::from_config("example", &ItemKindConfig::default()).unwrap();
    /// assert_eq!(kind.source_of_truth(), SourceOfTruth::MarkdownFirst);
    /// ```
    pub fn source_of_truth(&self) -> SourceOfTruth {
        self.source_of_truth
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
    /// use jit::config::ItemKindConfig;
    /// use jit::domain::item::ItemKind;
    ///
    /// let kind = ItemKind::from_config(
    ///     "example",
    ///     &ItemKindConfig {
    ///         section: Some("success_criteria".into()),
    ///         id_pattern: Some("REQ-[0-9]+".into()),
    ///         markers: Some(vec!["[hard]".into()]),
    ///         ..Default::default()
    ///     },
    /// )
    /// .unwrap();
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
    /// `<namespace>:<target>` link labels the item carries, projected from a
    /// registry-first kind's `link-fields` mapping. Empty for markdown-sourced and
    /// invariant items (links there are carried by the linking NODE, not the item),
    /// in which case the field is omitted from serialized output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
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
///     kind: "policy".to_string(),
///     self_id: "INV-01".to_string(),
///     text: "all writes are atomic".to_string(),
///     links: Vec::new(),
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
    /// `<namespace>:<target>` link labels carried by the item (empty for markdown
    /// substrates; populated by a registry-first kind's `link-fields` mapping).
    pub links: Vec<String>,
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
///     kind: "policy".to_string(),
///     self_id: "INV-01".to_string(),
///     text: "atomic writes".to_string(),
///     links: Vec::new(),
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
            links: candidate.links,
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
/// use jit::config::ItemKindConfig;
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_items, ItemKind};
/// use jit::domain::Issue;
///
/// let issue = Issue::new(
///     "T".to_string(),
///     "## Success Criteria\n\n- [hard] REQ-01: a\n- prose line\n".to_string(),
/// );
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("REQ-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// let items = index_items(&issue, &[kind], &MarkdownContentParser).unwrap();
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
                    // Markdown items carry no item-side links: a markdown link lives
                    // on the linking NODE's labels, not the addressed item.
                    links: Vec::new(),
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
/// use jit::config::ItemKindConfig;
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_markdown_items, ItemKind, Scope};
///
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("[A-Z]+-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// let md = "## Success Criteria\n\n- [hard] INV-01: all writes are atomic\n";
/// let items =
///     index_markdown_items(md, &Scope::Project, &[kind], &MarkdownContentParser).unwrap();
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
/// use jit::config::ItemKindConfig;
/// use jit::domain::item::{ItemKind, ProjectSource};
///
/// let kind = ItemKind::from_config("example", &ItemKindConfig::default()).unwrap();
/// let src = ProjectSource {
///     kind,
///     markdown: "## Success Criteria\n\n- [hard] INV-01: x\n".to_string(),
/// };
/// assert_eq!(src.kind.name(), "example");
/// ```
#[derive(Debug, Clone)]
pub struct ProjectSource {
    /// The project-scope kind whose items this source holds.
    pub kind: ItemKind,
    /// The markdown text of the kind's `source` file (already read from disk).
    pub markdown: String,
}

/// Index every project-scope (`@`) substrate through ONE per-scope dedup pass.
///
/// Two substrates feed the project scope and BOTH funnel through the SAME single
/// [`derive_scope_items`] call here, so per-scope uniqueness and qualified-id
/// derivation are identical across them (REQ-03, REQ-04, REQ-05):
///
/// 1. **Markdown-first** kinds (each a [`ProjectSource`]) are parsed and scanned
///    via the same [`extract_raw_items`] path as issue descriptions.
/// 2. **Registry-first** kinds supply their candidates directly as
///    `registry_items` — already projected from a structured registry, NOT a
///    markdown section, since the registry is their authoritative source (REQ-02).
///
/// Pooling all candidates before deriving means a self-id repeated across any two
/// project kinds (markdown or registry) is reported as a duplicate (REQ-03), and
/// qualified-id derivation matches issue scope (REQ-01, REQ-05). Empty inputs yield
/// no items (graceful), never an error.
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_project_sources, ItemKind, ProjectSource, RawScopeItem};
///
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("REQ-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// let sources = vec![ProjectSource {
///     kind,
///     markdown: "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n".to_string(),
/// }];
/// // A registry-first candidate is supplied directly.
/// let registry = vec![RawScopeItem {
///     kind: "policy".to_string(),
///     self_id: "INV-01".to_string(),
///     text: "every dependency edge stays acyclic".to_string(),
///     links: Vec::new(),
/// }];
/// let items = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap();
/// let qids: Vec<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
/// assert!(qids.contains(&"@/REQ-01"));
/// assert!(qids.contains(&"@/INV-01"));
/// ```
pub fn index_project_sources(
    sources: &[ProjectSource],
    registry_items: Vec<RawScopeItem>,
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let mut raw: Vec<RawScopeItem> = sources
        .iter()
        .flat_map(|source| {
            let sections = sections_from_markdown(&source.markdown, parser);
            extract_raw_items(&sections, std::slice::from_ref(&source.kind))
        })
        .collect();
    raw.extend(registry_items);
    derive_scope_items(&Scope::Project, raw)
}

/// Project a registry-first kind's `.toml` source into raw scope candidates
/// through its declared field mapping.
///
/// This is the generic analogue of the hard-wired invariant projection: it takes
/// the already-read `toml_content` (the file I/O happens in the command layer
/// through the storage boundary, so this stays a PURE function) and maps each
/// entry of the descriptor's named array-of-tables into a [`RawScopeItem`]:
/// `id-field` -> self-id (so the derived qualified id is `@/<self-id>`),
/// `text-field` -> text, and each `link-fields` entry -> `<namespace>:<target>`
/// link labels (the mapped field may be a single string or an array of strings).
/// `kind_name` tags every candidate so the derived item reports the right kind.
///
/// Graceful vs. typed-error contract:
/// - A missing `table` key yields NO items (an empty registry, like an absent
///   file), never an error.
/// - A malformed file is [`ItemError::TomlSourceParse`].
/// - An entry missing the mapped `id-field` / `text-field` is
///   [`ItemError::TomlSourceMissingField`]; a mapped field of an unexpected TOML
///   type is [`ItemError::TomlSourceFieldType`].
/// - An entry that simply lacks a mapped LINK field contributes no labels for it
///   (graceful) — only the addressing fields are mandatory.
///
/// # Examples
///
/// ```
/// use jit::config::TomlSourceDescriptor;
/// use jit::domain::item::load_toml_scope_items;
///
/// let descriptor = TomlSourceDescriptor {
///     toml: "policies.toml".into(),
///     table: "policies".into(),
///     id_field: "id".into(),
///     text_field: "statement".into(),
///     link_fields: [("enforces".to_string(), "enforced-by".to_string())]
///         .into_iter()
///         .collect(),
/// };
/// let content = "\
/// [[policies]]\n\
/// id = \"POL-01\"\n\
/// statement = \"all writes are atomic\"\n\
/// enforced-by = [\"cargo-ci\"]\n";
/// let rows = load_toml_scope_items("policy", &descriptor, content).unwrap();
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows[0].self_id, "POL-01");
/// assert_eq!(rows[0].links, vec!["enforces:cargo-ci".to_string()]);
/// ```
pub fn load_toml_scope_items(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    toml_content: &str,
) -> Result<Vec<RawScopeItem>, ItemError> {
    let table: toml::Table =
        toml::from_str(toml_content).map_err(|source| ItemError::TomlSourceParse {
            kind: kind_name.to_string(),
            path: descriptor.toml.clone(),
            source: Box::new(source),
        })?;
    // A missing table is an empty registry (graceful), mirroring an absent file.
    let Some(entries) = table.get(&descriptor.table) else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| ItemError::TomlSourceFieldType {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: descriptor.table.clone(),
            expected: "an array of tables".to_string(),
        })?;
    entries
        .iter()
        .map(|entry| project_toml_entry(kind_name, descriptor, entry))
        .collect()
}

/// Map one TOML table entry into a [`RawScopeItem`] through `descriptor`'s field
/// mapping (the per-entry core of [`load_toml_scope_items`]).
fn project_toml_entry(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
) -> Result<RawScopeItem, ItemError> {
    let self_id = required_toml_str(kind_name, descriptor, entry, &descriptor.id_field)?;
    let text = required_toml_str(kind_name, descriptor, entry, &descriptor.text_field)?;
    // Link fields iterate in namespace order (BTreeMap) for deterministic labels.
    let links = descriptor
        .link_fields
        .iter()
        .map(|(namespace, field)| toml_link_labels(kind_name, descriptor, entry, namespace, field))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(RawScopeItem {
        kind: kind_name.to_string(),
        self_id,
        text,
        links,
    })
}

/// Read a required string `field` from a TOML entry, or a typed error naming the
/// missing field / wrong type.
fn required_toml_str(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
    field: &str,
) -> Result<String, ItemError> {
    let value = entry
        .get(field)
        .ok_or_else(|| ItemError::TomlSourceMissingField {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: field.to_string(),
        })?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ItemError::TomlSourceFieldType {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: field.to_string(),
            expected: "a string".to_string(),
        })
}

/// Project one mapped link `field` of a TOML entry into `<namespace>:<target>`
/// labels (a single string or an array of strings); an absent field is graceful.
fn toml_link_labels(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
    namespace: &str,
    field: &str,
) -> Result<Vec<String>, ItemError> {
    let field_type_err = || ItemError::TomlSourceFieldType {
        kind: kind_name.to_string(),
        table: descriptor.table.clone(),
        field: field.to_string(),
        expected: "a string or an array of strings".to_string(),
    };
    // An absent link field contributes no labels (graceful).
    let Some(value) = entry.get(field) else {
        return Ok(Vec::new());
    };
    let targets: Vec<&str> = match value {
        toml::Value::String(single) => vec![single.as_str()],
        toml::Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().ok_or_else(field_type_err))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(field_type_err()),
    };
    Ok(targets
        .into_iter()
        .map(|target| format!("{namespace}:{target}"))
        .collect())
}

/// Resolve the effective set of item kinds from an optional config registry.
///
/// The engine bakes in no domain defaults: when the registry is `None` (no
/// `[item_kinds]` table) the kind set is EMPTY (per design D4, single consumer,
/// no backward-compat layer). Kinds are authored entirely in the `[item_kinds]`
/// config table, which `jit init` scaffolds with a complete, editable set. When
/// the registry is present it is used verbatim (each entry resolved through
/// [`ItemKind::from_config`]); the caller opts in to every kind it wants.
///
/// Kinds are returned in name order for deterministic output.
///
/// # Examples
///
/// ```
/// use jit::domain::item::resolve_item_kinds;
///
/// // No `[item_kinds]` table -> no kinds (no baked built-ins).
/// assert!(resolve_item_kinds(None).unwrap().is_empty());
/// ```
pub fn resolve_item_kinds(
    registry: Option<&HashMap<String, ItemKindConfig>>,
) -> Result<Vec<ItemKind>, ItemError> {
    match registry {
        None => Ok(Vec::new()),
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
///     "example".to_string(),
///     ItemKindConfig {
///         section: Some("success_criteria".to_string()),
///         markers: Some(vec!["[hard]".to_string()]),
///         id_pattern: Some("REQ-\\d+".to_string()),
///         ..Default::default()
///     },
/// );
/// let triple = expand_kind_triple(Some(&registry), "example").unwrap();
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

    // The canonical kinds `jit init` authors into the `[item_kinds]` table, rebuilt
    // here from their exact field shape so the domain layer can pin that those
    // fields project as expected. They are no longer baked into the engine (a repo
    // with no `[item_kinds]` table has no kinds); these helpers stand in for the
    // config-authored table.
    fn req_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("success_criteria".to_string()),
            id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".to_string()),
            markers: Some(vec!["[hard]".to_string()]),
            link_namespaces: Some(vec!["satisfies".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
        }
    }

    fn decision_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("decisions".to_string()),
            id_pattern: Some("D-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["per".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
        }
    }

    fn risk_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("risks".to_string()),
            id_pattern: Some("RISK-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["mitigates".to_string(), "resolves".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
        }
    }

    fn invariant_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("success_criteria".to_string()),
            id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["enforces".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source: Some(ItemKindSource::Toml(TomlSourceDescriptor {
                toml: ".jit/invariants.toml".to_string(),
                table: "invariants".to_string(),
                id_field: "id".to_string(),
                text_field: "statement".to_string(),
                link_fields: std::collections::BTreeMap::new(),
            })),
            source_of_truth: Some(SourceOfTruth::RegistryFirst),
        }
    }

    fn req_kind() -> ItemKind {
        ItemKind::from_config("requirement", &req_cfg()).unwrap()
    }

    fn decision_kind() -> ItemKind {
        ItemKind::from_config("decision", &decision_cfg()).unwrap()
    }

    fn risk_kind() -> ItemKind {
        ItemKind::from_config("risk", &risk_cfg()).unwrap()
    }

    fn invariant_kind() -> ItemKind {
        ItemKind::from_config("invariant", &invariant_cfg()).unwrap()
    }

    /// The four canonical kinds, in name order (the set `jit init` authors).
    fn canonical_kinds() -> Vec<ItemKind> {
        vec![decision_kind(), invariant_kind(), req_kind(), risk_kind()]
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
    fn test_resolve_item_kinds_empty_when_absent() {
        // No `[item_kinds]` table -> no kinds. The engine bakes in no domain
        // defaults (D4: single consumer, no backward-compat layer); kinds are
        // authored entirely in config (scaffolded by `jit init`).
        assert!(resolve_item_kinds(None).unwrap().is_empty());
    }

    #[test]
    fn test_resolve_item_kinds_canonical_table_resolves_all_four() {
        // The complete table `jit init` authors resolves to the four canonical
        // kinds, in name order — each through the generic `from_config` path.
        let map: HashMap<String, ItemKindConfig> = [
            ("requirement", req_cfg()),
            ("decision", decision_cfg()),
            ("risk", risk_cfg()),
            ("invariant", invariant_cfg()),
        ]
        .into_iter()
        .map(|(name, cfg)| (name.to_string(), cfg))
        .collect();
        let kinds = resolve_item_kinds(Some(&map)).unwrap();
        let names: Vec<&str> = kinds.iter().map(ItemKind::name).collect();
        assert_eq!(names, vec!["decision", "invariant", "requirement", "risk"]);
    }

    #[test]
    fn test_decision_kind_tuple() {
        // The canonical decision kind: section `decisions`, D-NN ids, no marker,
        // `per` link namespace, issue-scoped, markdown-first.
        let decision = decision_kind();
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
    fn test_invariant_kind_is_project_scope_registry_first() {
        // The canonical invariant kind: project-scoped, registry-first, `enforces`
        // link namespace, and NO markdown `source` file (items come from the toml
        // registry named by its descriptor).
        let inv = invariant_kind();
        assert_eq!(inv.name(), "invariant");
        assert_eq!(inv.kind_scope(), KindScope::Project);
        assert_eq!(inv.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(inv.source(), None);
        assert_eq!(inv.link_namespaces(), &["enforces".to_string()]);
    }

    #[test]
    fn test_registry_first_project_kind_requires_toml_descriptor() {
        // Symmetric to the markdown-first source requirement (REQ-03): a
        // registry-first project kind must declare a toml `source` descriptor, or it
        // is rejected at resolution with a typed MissingProjectSource — never a
        // silent empty result.
        let base = |source: Option<ItemKindSource>| ItemKindConfig {
            section: Some(DEFAULT_ITEM_SECTION.to_string()),
            id_pattern: Some(DEFAULT_ITEM_ID_PATTERN.to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["enforces".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source,
            source_of_truth: Some(SourceOfTruth::RegistryFirst),
        };

        // No descriptor → rejected (the registry-first analogue of a missing
        // markdown source).
        let err = ItemKind::from_config("policy", &base(None)).unwrap_err();
        assert!(matches!(err, ItemError::MissingProjectSource { .. }));

        // With a toml descriptor → accepted; the descriptor is exposed by
        // `toml_source`, and no markdown `source` path is set.
        let descriptor = TomlSourceDescriptor {
            toml: "policies.toml".to_string(),
            table: "policies".to_string(),
            id_field: "id".to_string(),
            text_field: "statement".to_string(),
            link_fields: std::collections::BTreeMap::new(),
        };
        let kind =
            ItemKind::from_config("policy", &base(Some(ItemKindSource::Toml(descriptor)))).unwrap();
        assert_eq!(kind.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(kind.source(), None);
        assert_eq!(kind.toml_source().unwrap().table, "policies");
    }

    #[test]
    fn test_invariant_kind_routes_through_toml_descriptor() {
        // REQ-03: the canonical `invariant` kind carries a toml `source` descriptor
        // naming `.jit/invariants.toml`, so it routes through the GENERIC
        // registry-first path with no reserved-name branch. The descriptor maps
        // `id`/`statement` and NO link-fields (so each item's links stay empty,
        // matching the prior typed projection byte-for-byte).
        let inv = invariant_kind();
        assert_eq!(inv.name(), "invariant");
        assert_eq!(inv.kind_scope(), KindScope::Project);
        assert_eq!(inv.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(inv.source(), None);
        let descriptor = inv
            .toml_source()
            .expect("invariant carries a toml descriptor");
        assert_eq!(descriptor.toml, ".jit/invariants.toml");
        assert_eq!(descriptor.table, "invariants");
        assert_eq!(descriptor.id_field, "id");
        assert_eq!(descriptor.text_field, "statement");
        assert!(descriptor.link_fields.is_empty());
    }

    #[test]
    fn test_invariant_name_is_no_longer_reserved() {
        // REQ-03: the `invariant` NAME no longer carries any special routing. A
        // config-declared `invariant` kind resolves as ordinary config like any
        // other name — declarations the old reserved-name branch rejected are now
        // accepted because the engine hardcodes no domain concept.
        let cfg = |scope: KindScopeConfig, sot: SourceOfTruth, source: Option<ItemKindSource>| {
            ItemKindConfig {
                section: Some(DEFAULT_ITEM_SECTION.to_string()),
                id_pattern: Some("INV-[0-9]+".to_string()),
                markers: Some(vec![]),
                link_namespaces: Some(vec!["enforces".to_string()]),
                scope: Some(scope),
                source,
                source_of_truth: Some(sot),
            }
        };

        // Markdown-first project `invariant` with a source: once a reserved-name
        // rejection, now an ordinary markdown-first project kind.
        let md = ItemKind::from_config(
            "invariant",
            &cfg(
                KindScopeConfig::Project,
                SourceOfTruth::MarkdownFirst,
                Some(ItemKindSource::Path("project-items.md".to_string())),
            ),
        )
        .unwrap();
        assert_eq!(md.source_of_truth(), SourceOfTruth::MarkdownFirst);
        assert_eq!(md.source(), Some("project-items.md"));

        // Registry-first issue-scoped `invariant`: once rejected to keep invariants
        // out of the issue-description parser, now an ordinary issue-scope kind (the
        // project-source requirement does not apply to issue scope).
        let issue = ItemKind::from_config(
            "invariant",
            &cfg(KindScopeConfig::Issue, SourceOfTruth::RegistryFirst, None),
        )
        .unwrap();
        assert_eq!(issue.kind_scope(), KindScope::Issue);
    }

    #[test]
    fn test_markdown_first_project_kind_still_requires_source() {
        // The guard still fires for a markdown-first project kind with no source.
        let cfg = ItemKindConfig {
            section: Some(DEFAULT_ITEM_SECTION.to_string()),
            id_pattern: Some(DEFAULT_ITEM_ID_PATTERN.to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["upholds".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
        };
        let err = ItemKind::from_config("doc-req", &cfg).unwrap_err();
        assert!(matches!(err, ItemError::MissingProjectSource { .. }));
    }

    #[test]
    fn test_index_project_sources_pools_registry_and_markdown() {
        // Both substrates dedup through one pass: a markdown source and a
        // registry-derived candidate both surface as `@/<self-id>`.
        let sources = vec![ProjectSource {
            kind: req_kind(),
            markdown: "## Success Criteria\n\n- [hard] REQ-01: a\n".to_string(),
        }];
        let registry = vec![RawScopeItem {
            kind: "invariant".to_string(),
            self_id: "INV-01".to_string(),
            text: "atomic writes".to_string(),
            links: Vec::new(),
        }];
        let items = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap();
        let qids: Vec<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
        assert!(qids.contains(&"@/REQ-01"));
        assert!(qids.contains(&"@/INV-01"));
    }

    #[test]
    fn test_index_project_sources_cross_substrate_duplicate_is_error() {
        // A self-id shared by a markdown source and a registry candidate collides
        // in the single project-scope dedup pass (REQ-03).
        let sources = vec![ProjectSource {
            kind: req_kind(),
            markdown: "## Success Criteria\n\n- [hard] INV-01: a\n".to_string(),
        }];
        let registry = vec![RawScopeItem {
            kind: "invariant".to_string(),
            self_id: "INV-01".to_string(),
            text: "dup".to_string(),
            links: Vec::new(),
        }];
        let err = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap_err();
        assert!(matches!(err, ItemError::DuplicateSelfId { .. }));
    }

    fn policy_descriptor() -> TomlSourceDescriptor {
        TomlSourceDescriptor {
            toml: "policies.toml".to_string(),
            table: "policies".to_string(),
            id_field: "id".to_string(),
            text_field: "statement".to_string(),
            link_fields: [("enforces".to_string(), "enforced-by".to_string())]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn test_load_toml_scope_items_maps_fields_and_links() {
        // REQ-02: id-field -> self-id, text-field -> text, and a string-array link
        // field -> `<namespace>:<target>` labels; a single-string link field maps to
        // one label.
        let content = "\
[[policies]]
id = \"POL-01\"
statement = \"all writes are atomic\"
enforced-by = [\"cargo-ci\", \"jit-validate\"]

[[policies]]
id = \"POL-02\"
statement = \"single enforcer\"
enforced-by = \"tests\"
";
        let rows = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "policy");
        assert_eq!(rows[0].self_id, "POL-01");
        assert_eq!(rows[0].text, "all writes are atomic");
        assert_eq!(
            rows[0].links,
            vec!["enforces:cargo-ci", "enforces:jit-validate"]
        );
        // A single string link value yields exactly one label.
        assert_eq!(rows[1].links, vec!["enforces:tests"]);
    }

    #[test]
    fn test_load_toml_scope_items_absent_table_is_graceful() {
        // A file with no matching table is an empty registry, never an error.
        let rows =
            load_toml_scope_items("policy", &policy_descriptor(), "[other]\nx = 1\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_load_toml_scope_items_absent_link_field_is_graceful() {
        // An entry lacking the mapped link field contributes no labels (only the
        // addressing fields are mandatory).
        let content = "[[policies]]\nid = \"POL-03\"\nstatement = \"no enforcer\"\n";
        let rows = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].links.is_empty());
    }

    #[test]
    fn test_load_toml_scope_items_missing_id_field_is_error() {
        // A missing mapped id-field is a typed, descriptive error naming the field.
        let content = "[[policies]]\nstatement = \"x\"\n";
        let err = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap_err();
        assert!(matches!(
            err,
            ItemError::TomlSourceMissingField { ref field, .. } if field == "id"
        ));
    }

    #[test]
    fn test_load_toml_scope_items_wrong_link_type_is_error() {
        // A link field that is neither a string nor an array of strings is a typed
        // field-type error.
        let content = "[[policies]]\nid = \"POL-04\"\nstatement = \"x\"\nenforced-by = 7\n";
        let err = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap_err();
        assert!(matches!(
            err,
            ItemError::TomlSourceFieldType { ref field, .. } if field == "enforced-by"
        ));
    }

    #[test]
    fn test_load_toml_scope_items_malformed_toml_is_parse_error() {
        let err =
            load_toml_scope_items("policy", &policy_descriptor(), "not = = toml").unwrap_err();
        assert!(matches!(err, ItemError::TomlSourceParse { .. }));
    }

    #[test]
    fn test_index_items_projects_decisions_with_canonical_kinds() {
        // The canonical kind set (as `jit init` authors it) indexes a `## Decisions`
        // section's D-NN lines through the same generic parse path as requirements.
        let issue = Issue::new(
            "T".to_string(),
            "## Decisions\n\n- D-01: use json\n- D-02: atomic writes\n".to_string(),
        );
        let kinds = canonical_kinds();
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
            links: Vec::new(),
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
