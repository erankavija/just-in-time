//! Configuration file loading and parsing.
//!
//! JIT supports repository-level configuration through `.jit/config.toml`.
//! If no config file exists, the system falls back to sensible defaults.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure loaded from `.jit/config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct JitConfig {
    /// Schema version for migrations (optional).
    pub version: Option<VersionConfig>,
    /// Type hierarchy configuration (optional).
    pub type_hierarchy: Option<HierarchyConfigToml>,
    /// Validation behavior configuration (optional).
    pub validation: Option<ValidationConfig>,
    /// Documentation lifecycle configuration (optional).
    pub documentation: Option<DocumentationConfig>,
    /// Label namespace registry (optional - replaces labels.json).
    pub namespaces: Option<HashMap<String, NamespaceConfig>>,
    /// Addressable item-kind registry (optional).
    ///
    /// Each entry declares one kind as the six-tuple `(section, id-pattern,
    /// marker(s), link-namespace(s), scope, source-of-truth)` projection over a
    /// substrate (issue descriptions or a registry/markdown source), mirroring the
    /// `[namespaces.*]` registry precedent. The engine never hardcodes a kind
    /// NAME: a kind is purely the tuple it expands to, so `requirement`,
    /// `decision`, `risk`, etc. are all just configuration. See
    /// [`ItemKindConfig`].
    pub item_kinds: Option<HashMap<String, ItemKindConfig>>,
    /// Documentation target the invariant registry projects into (optional).
    ///
    /// When the `[invariant_projection]` table is ABSENT, the projection engine
    /// falls back to [`InvariantProjectionConfig::default`], which targets a
    /// separate jit-owned file ([`DEFAULT_INVARIANT_PROJECTION_TARGET`]) in
    /// separate-file mode, so the default never touches existing docs. The target
    /// path lives ONLY here in the config layer; the projection engine reads it
    /// from config and never hardcodes a documentation filename. See
    /// [`InvariantProjectionConfig`].
    pub invariant_projection: Option<InvariantProjectionConfig>,
    /// Worktree and parallel work configuration (optional).
    pub worktree: Option<WorktreeConfig>,
    /// Coordination settings for leases and agents (optional).
    pub coordination: Option<CoordinationConfig>,
    /// Global operations configuration (optional).
    pub global_operations: Option<GlobalOperationsConfig>,
    /// Lock file configuration (optional).
    pub locks: Option<LocksConfig>,
    /// Event logging configuration (optional).
    pub events: Option<EventsConfig>,
    /// Graph templates loaded and validated from `.jit/templates.toml`.
    ///
    /// Not read from `config.toml`: populated by [`JitConfig::load`] from the
    /// sibling `templates.toml` (absent file → empty registry), so it carries
    /// `#[serde(skip)]` and defaults to an empty [`TemplateRegistry`].
    #[serde(skip)]
    pub templates: crate::templates::TemplateRegistry,
    /// Project invariants loaded from `.jit/invariants.toml`.
    ///
    /// Not read from `config.toml`: populated by [`JitConfig::load`] from the
    /// sibling `invariants.toml` (absent file → empty registry) on BOTH load
    /// paths, so it carries `#[serde(skip)]` and defaults to an empty
    /// [`InvariantRegistry`](crate::validation::invariants::InvariantRegistry).
    /// Kept here so later indexing can project each entry as a project-scoped
    /// (`@`) addressable item.
    #[serde(skip)]
    pub invariants: crate::validation::invariants::InvariantRegistry,
}

/// Schema version configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionConfig {
    /// Schema version number (default: 1).
    pub schema: u32,
}

/// Type hierarchy configuration from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct HierarchyConfigToml {
    /// Type name to hierarchy level mapping (lower = more strategic).
    pub types: HashMap<String, u8>,
    /// Type name to membership label namespace mapping (optional).
    pub label_associations: Option<HashMap<String, String>>,
    /// List of type names considered strategic (optional).
    pub strategic_types: Option<Vec<String>>,
    /// Icon configuration (optional).
    pub icons: Option<IconConfigToml>,
}

/// Icon configuration from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct IconConfigToml {
    /// Icon preset name: "simple", "navigation", "minimal", "construction" (optional).
    pub preset: Option<String>,
    /// Custom type name to icon mapping (optional, partial overrides allowed).
    pub custom: Option<HashMap<String, String>>,
}

/// Validation behavior configuration.
///
/// The former enforcement keys (`require_type_label`, `label_regex`,
/// `reject_malformed_labels`, `enforce_namespace_registry`, `warn_orphaned_leaves`,
/// `warn_strategic_consistency`) were removed when `.jit/rules.toml` became the
/// sole validation source (DR §8.2/§8.4). Only the BEHAVIORAL keys survive:
/// `default_type`, `content_format`, and the inert `strictness`. serde ignores
/// any stale enforcement keys still present in an old `config.toml` (no
/// `deny_unknown_fields`), so such a file still parses; the keys simply have no
/// effect — the operative rules live in `rules.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidationConfig {
    /// Strictness level: "strict", "loose", or "permissive". Retained as an inert
    /// forward-compat key; it no longer drives validation behavior.
    pub strictness: Option<String>,
    /// Default type when none specified (optional).
    pub default_type: Option<String>,
    /// Repo-level default content format for issue bodies ("markdown", "html",
    /// "xml"). Selects the [`ContentParser`](crate::document::ContentParser) used
    /// to extract `sections` for issues that carry no per-issue `content_format`.
    /// Absent means "markdown".
    pub content_format: Option<String>,
}

impl ValidationConfig {
    /// Resolve the repo-level default content format, defaulting to
    /// [`ContentFormat::Markdown`](crate::domain::ContentFormat::Markdown) when
    /// unset. An invalid value is surfaced as an error rather than silently
    /// defaulting, so a misconfigured `config.toml` cannot quietly pick the wrong
    /// parser.
    pub fn content_format(&self) -> Result<crate::domain::ContentFormat> {
        use std::str::FromStr;
        match self.content_format.as_deref() {
            None => Ok(crate::domain::ContentFormat::Markdown),
            Some(value) => crate::domain::ContentFormat::from_str(value).with_context(|| {
                format!("invalid [validation].content_format in .jit/config.toml: '{value}'")
            }),
        }
    }
}

/// Documentation lifecycle management configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentationConfig {
    /// Root directory for development documentation (default: "dev").
    pub development_root: Option<String>,
    /// Paths subject to archival (default: ["dev/active", "dev/studies", "dev/sessions"]).
    pub managed_paths: Option<Vec<String>>,
    /// Where archived docs are stored (default: "dev/archive").
    pub archive_root: Option<String>,
    /// Paths that never archive (default: ["docs/"]).
    pub permanent_paths: Option<Vec<String>>,
    /// Archive category mappings (e.g., design -> features).
    pub categories: Option<HashMap<String, String>>,
}

impl DocumentationConfig {
    /// Get development root with default fallback.
    pub fn development_root(&self) -> String {
        self.development_root
            .clone()
            .unwrap_or_else(|| "dev".to_string())
    }

    /// Get managed paths with default fallback.
    pub fn managed_paths(&self) -> Vec<String> {
        self.managed_paths.clone().unwrap_or_else(|| {
            vec![
                "dev/active".to_string(),
                "dev/studies".to_string(),
                "dev/sessions".to_string(),
            ]
        })
    }

    /// Get archive root with default fallback.
    pub fn archive_root(&self) -> String {
        self.archive_root
            .clone()
            .unwrap_or_else(|| "dev/archive".to_string())
    }

    /// Get permanent paths with default fallback.
    pub fn permanent_paths(&self) -> Vec<String> {
        self.permanent_paths
            .clone()
            .unwrap_or_else(|| vec!["docs/".to_string()])
    }

    /// Get category mapping (key: category ID, value: archive subdirectory).
    pub fn categories(&self) -> HashMap<String, String> {
        self.categories.clone().unwrap_or_default()
    }
}

/// Label namespace configuration from TOML.
/// Replaces the namespace definitions in labels.json.
///
/// The per-namespace constraint fields (`values`, `pattern`, `required`) were
/// removed when `.jit/rules.toml` became the sole validation source (DR §8.4): a
/// repo that wants those constraints authors the corresponding rules in
/// `rules.toml`. The registry keeps only TAXONOMY (`description`/`unique`/
/// `examples`); serde ignores any stale constraint keys in an old `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceConfig {
    /// Human-readable description.
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue.
    pub unique: bool,
    /// Example labels (optional, for documentation).
    pub examples: Option<Vec<String>>,
}

/// One addressable item-kind declaration from `[item_kinds.<name>]`.
///
/// A kind is a config-declared projection over issue descriptions, parallel to
/// [`NamespaceConfig`]: it names the `section` whose list items are scanned, the
/// `id_pattern` regex that extracts a self-id from an item, the `markers` an item
/// must begin with to qualify, and the `link_namespaces` that reference items of
/// this kind by qualified id (e.g. `satisfies:`). No kind NAME is interpreted by
/// engine logic — `requirement`, `decision`, `risk`, and any later kind are
/// distinguished solely by these fields, keeping the engine domain-agnostic.
///
/// Each member of the six-tuple `(section, id-pattern, marker(s),
/// link-namespace(s), scope, source-of-truth)` is modeled as `Option` at the
/// serde layer, but an **explicitly-declared** `[item_kinds.X]` table MUST set
/// all six: [`JitConfig::load`] validates this via
/// [`ItemKindConfig::missing_required_fields`] and rejects a partial declaration
/// with a descriptive [`ItemKindConfigError::MissingFields`]. The `Option`s
/// survive only so the IMPLICIT built-in default kinds (`requirement`,
/// `decision`, ...; used when no `[item_kinds]` table is declared at all) and
/// direct struct construction in tests can still rely on per-field defaults
/// applied by
/// [`ItemKind::from_config`](crate::domain::item::ItemKind::from_config). The
/// `source` PATH (project-scope source file) is NOT one of the six and stays
/// optional.
///
/// # Examples
///
/// A complete six-field issue-scope declaration:
///
/// ```
/// use jit::config::JitConfig;
///
/// let config: JitConfig = toml::from_str(
///     r#"
/// [item_kinds.requirement]
/// section = "success_criteria"
/// id-pattern = "REQ-\\d+"
/// markers = ["[hard]"]
/// link-namespaces = ["satisfies"]
/// scope = "issue"
/// source-of-truth = "markdown-first"
/// "#,
/// )
/// .unwrap();
/// let kinds = config.item_kinds.unwrap();
/// let req = &kinds["requirement"];
/// assert_eq!(req.section.as_deref(), Some("success_criteria"));
/// assert_eq!(req.markers, Some(vec!["[hard]".to_string()]));
/// // All six fields are set, so the declaration is complete.
/// assert!(req.missing_required_fields().is_empty());
/// ```
///
/// A kind may instead be **project-scoped**, addressing items not tied to any
/// single issue (qualified id `@/<self-id>`). A *markdown-first* project kind sets
/// `scope = "project"` and a `source` file (a repository-local path, relative to
/// the repo root) whose markdown is scanned the SAME way an issue description is. It
/// still declares all six required fields (the optional `source` PATH is in
/// addition). The example uses `glossary` (a non-reserved name); the built-in
/// `invariant` kind is reserved as project-scoped AND registry-first, so it has no
/// `source` path and its items come only from `.jit/invariants.toml`:
///
/// ```
/// use jit::config::{JitConfig, KindScopeConfig};
///
/// let config: JitConfig = toml::from_str(
///     r#"
/// [item_kinds.glossary]
/// section = "glossary"
/// id-pattern = "GLOSS-\\d+"
/// markers = []
/// link-namespaces = ["defines"]
/// scope = "project"
/// source = "project-items.md"
/// source-of-truth = "markdown-first"
/// "#,
/// )
/// .unwrap();
/// let gloss = &config.item_kinds.unwrap()["glossary"];
/// assert_eq!(gloss.scope, Some(KindScopeConfig::Project));
/// assert_eq!(gloss.source.as_ref().and_then(|s| s.path()), Some("project-items.md"));
/// assert!(gloss.missing_required_fields().is_empty());
/// ```
///
/// The sixth field, `source-of-truth`, records the authoring DIRECTION for the
/// kind (which substrate is canonical). It is a typed [`SourceOfTruth`] distinct
/// from the `source` PATH above: a `markdown-first` kind is authored in prose and
/// indexed from it; a `registry-first` kind is authored in a structured registry
/// file. Resolve a present value with [`ItemKindConfig::source_of_truth`].
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct ItemKindConfig {
    /// Section slug whose list items hold this kind's addressable items
    /// (e.g. `"success_criteria"`). Required in an explicit declaration.
    pub section: Option<String>,
    /// Regex extracting an item's self-id from its text (e.g. `"REQ-\\d+"`).
    /// Required in an explicit declaration.
    #[serde(rename = "id-pattern")]
    pub id_pattern: Option<String>,
    /// Markers an item's text must begin with to qualify (e.g. `["[hard]"]`).
    /// Required in an explicit declaration; an EMPTY list (`markers = []`) is a
    /// valid, present value meaning "every matching item qualifies".
    pub markers: Option<Vec<String>>,
    /// Link-label namespaces that reference items of this kind by qualified id
    /// (e.g. `["satisfies"]`). Required in an explicit declaration.
    #[serde(rename = "link-namespaces")]
    pub link_namespaces: Option<Vec<String>>,
    /// Addressing scope: [`KindScopeConfig::Issue`] for items projected from issue
    /// descriptions (qualified id `<issue>/<self-id>`), or
    /// [`KindScopeConfig::Project`] for items projected from a repository-local
    /// `source` file (qualified id `@/<self-id>`). Required in an explicit
    /// declaration; an unrecognised token is a TOML parse error, not a silent
    /// fallback.
    pub scope: Option<KindScopeConfig>,
    /// For a `scope = "project"` kind, where its items are sourced from. Two
    /// shapes are accepted (resolved by [`ItemKindSource`]):
    /// - a bare path STRING (`source = "glossary.md"`) for a markdown-first kind,
    ///   naming the repository-local file whose declared `section` is scanned; or
    /// - a structured TOML descriptor table
    ///   (`source = { toml = "...", table = "...", id-field = "...", text-field =
    ///   "...", link-fields = { ns = "field" } }`) for a registry-first kind,
    ///   naming a `.toml` registry and the field mapping that projects each entry
    ///   into an addressable item.
    ///
    /// Either way the location comes ONLY from config — no filename is hardcoded in
    /// engine logic — and all I/O goes through the storage boundary. Ignored for
    /// issue-scoped kinds; an absent or missing file yields no project items
    /// (graceful), not an error. NOT one of the six required fields — it stays
    /// optional.
    pub source: Option<ItemKindSource>,
    /// The kind's authoring DIRECTION (which substrate is canonical), distinct
    /// from the `source` PATH above. Required in an explicit declaration; resolve
    /// a present value with [`ItemKindConfig::source_of_truth`].
    #[serde(rename = "source-of-truth")]
    pub source_of_truth: Option<SourceOfTruth>,
}

/// The six required fields of an explicitly-declared item kind, in declaration
/// order, paired with the TOML key authors write. Used by
/// [`ItemKindConfig::missing_required_fields`] to report any absentees by their
/// authored key (`source` is intentionally absent — it is not one of the six).
const REQUIRED_ITEM_KIND_FIELDS: [&str; 6] = [
    "section",
    "id-pattern",
    "markers",
    "link-namespaces",
    "scope",
    "source-of-truth",
];

impl ItemKindConfig {
    /// The authored TOML keys of the six required fields this declaration leaves
    /// unset, in declaration order.
    ///
    /// An explicitly-declared `[item_kinds.X]` table must set all six; this
    /// reports which (if any) are missing so [`JitConfig::load`] can name them in
    /// a descriptive error. The optional `source` PATH is not checked. An empty
    /// result means the declaration is complete.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{ItemKindConfig, KindScopeConfig, SourceOfTruth};
    ///
    /// // A bare default is missing every required field.
    /// assert_eq!(
    ///     ItemKindConfig::default().missing_required_fields(),
    ///     vec![
    ///         "section",
    ///         "id-pattern",
    ///         "markers",
    ///         "link-namespaces",
    ///         "scope",
    ///         "source-of-truth",
    ///     ]
    /// );
    ///
    /// // A complete declaration reports nothing missing.
    /// let complete = ItemKindConfig {
    ///     section: Some("success_criteria".to_string()),
    ///     id_pattern: Some("REQ-\\d+".to_string()),
    ///     markers: Some(vec!["[hard]".to_string()]),
    ///     link_namespaces: Some(vec!["satisfies".to_string()]),
    ///     scope: Some(KindScopeConfig::Issue),
    ///     source_of_truth: Some(SourceOfTruth::MarkdownFirst),
    ///     source: None,
    /// };
    /// assert!(complete.missing_required_fields().is_empty());
    /// ```
    pub fn missing_required_fields(&self) -> Vec<&'static str> {
        let present = [
            self.section.is_some(),
            self.id_pattern.is_some(),
            self.markers.is_some(),
            self.link_namespaces.is_some(),
            self.scope.is_some(),
            self.source_of_truth.is_some(),
        ];
        REQUIRED_ITEM_KIND_FIELDS
            .iter()
            .zip(present)
            .filter_map(|(key, present)| (!present).then_some(*key))
            .collect()
    }
    /// Resolve the kind's [`SourceOfTruth`], defaulting to
    /// [`SourceOfTruth::MarkdownFirst`] when the `source-of-truth` field is unset.
    ///
    /// The default matches the requirement kind, which is authored in issue
    /// descriptions (markdown) and indexed from them.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{ItemKindConfig, SourceOfTruth};
    ///
    /// // An unset field resolves to markdown-first.
    /// assert_eq!(
    ///     ItemKindConfig::default().source_of_truth(),
    ///     SourceOfTruth::MarkdownFirst
    /// );
    /// let registry = ItemKindConfig {
    ///     source_of_truth: Some(SourceOfTruth::RegistryFirst),
    ///     ..Default::default()
    /// };
    /// assert_eq!(registry.source_of_truth(), SourceOfTruth::RegistryFirst);
    /// ```
    pub fn source_of_truth(&self) -> SourceOfTruth {
        self.source_of_truth.unwrap_or_default()
    }
}

/// Where a `scope = "project"` item kind reads its items from: either a bare
/// markdown file PATH or a structured TOML [`TomlSourceDescriptor`].
///
/// This is the polymorphic value of [`ItemKindConfig::source`]. A bare string
/// deserializes to [`ItemKindSource::Path`] (the markdown-first form: the named
/// file's `section` is scanned), while a table deserializes to
/// [`ItemKindSource::Toml`] (the registry-first form: each entry of the named
/// `.toml` table is projected through a field mapping). The two shapes are
/// distinguished at parse time — a string is never a descriptor and vice versa —
/// so a malformed descriptor surfaces a descriptive TOML error rather than a
/// silent fallback.
///
/// # Examples
///
/// ```
/// use jit::config::{ItemKindConfig, ItemKindSource};
///
/// // A bare path string is the markdown-first form.
/// let md: ItemKindConfig = toml::from_str(r#"source = "glossary.md""#).unwrap();
/// assert_eq!(md.source, Some(ItemKindSource::Path("glossary.md".into())));
/// assert_eq!(md.source.unwrap().path(), Some("glossary.md"));
///
/// // A table is the registry-first toml descriptor.
/// let reg: ItemKindConfig = toml::from_str(
///     r#"source = { toml = "policies.toml", table = "policies", id-field = "id", text-field = "statement" }"#,
/// )
/// .unwrap();
/// let descriptor = reg.source.unwrap();
/// assert_eq!(descriptor.path(), None);
/// assert_eq!(descriptor.toml_descriptor().unwrap().table, "policies");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKindSource {
    /// A bare repository-local file path (markdown-first): the file's `section` is
    /// scanned the same way an issue description is.
    Path(String),
    /// A structured TOML registry descriptor (registry-first): the named table is
    /// projected through a field mapping.
    Toml(TomlSourceDescriptor),
}

impl ItemKindSource {
    /// The bare markdown file path, or `None` when this is a TOML descriptor.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::ItemKindSource;
    ///
    /// assert_eq!(ItemKindSource::Path("x.md".into()).path(), Some("x.md"));
    /// ```
    pub fn path(&self) -> Option<&str> {
        match self {
            ItemKindSource::Path(p) => Some(p),
            ItemKindSource::Toml(_) => None,
        }
    }

    /// The structured TOML descriptor, or `None` when this is a bare path.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::ItemKindSource;
    ///
    /// assert!(ItemKindSource::Path("x.md".into()).toml_descriptor().is_none());
    /// ```
    pub fn toml_descriptor(&self) -> Option<&TomlSourceDescriptor> {
        match self {
            ItemKindSource::Toml(d) => Some(d),
            ItemKindSource::Path(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for ItemKindSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A bare string is the markdown path form; a table is the toml descriptor.
        // A Visitor (rather than `#[serde(untagged)]`) keeps this robust under the
        // TOML deserializer, which buffers untagged enums poorly.
        struct SourceVisitor;
        impl<'de> serde::de::Visitor<'de> for SourceVisitor {
            type Value = ItemKindSource;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(
                    "a markdown file path string, or a toml source descriptor table \
                     { toml, table, id-field, text-field, link-fields }",
                )
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ItemKindSource::Path(v.to_string()))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ItemKindSource::Path(v))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                let descriptor = TomlSourceDescriptor::deserialize(
                    serde::de::value::MapAccessDeserializer::new(map),
                )?;
                Ok(ItemKindSource::Toml(descriptor))
            }
        }
        deserializer.deserialize_any(SourceVisitor)
    }
}

/// The field mapping that projects a `.toml` registry table into addressable
/// items for a registry-first project kind.
///
/// Each entry of the named `table` (an array-of-tables) becomes one addressable
/// item: `id-field` supplies its self-id (so its qualified id is `@/<self-id>`),
/// `text-field` supplies its display text, and each `link-fields` entry maps a
/// toml field holding link targets to the link NAMESPACE those targets are
/// labelled under. A `link-fields` value may be a single string or an array of
/// strings; an absent link field on an entry contributes no labels (graceful).
///
/// This is the generic analogue of the typed invariant registry: it carries only
/// the addressing mapping (id/text/links), leaving any kind-specific TYPED
/// validation (e.g. an invariant's `enforced`/`advisory` discriminant) to a
/// dedicated loader.
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
///
/// let cfg: ItemKindConfig = toml::from_str(
///     r#"source = { toml = "policies.toml", table = "policies", id-field = "id", text-field = "statement", link-fields = { enforces = "enforced-by" } }"#,
/// )
/// .unwrap();
/// let descriptor = cfg.source.unwrap().toml_descriptor().cloned().unwrap();
/// assert_eq!(descriptor.toml, "policies.toml");
/// assert_eq!(descriptor.table, "policies");
/// assert_eq!(descriptor.id_field, "id");
/// assert_eq!(descriptor.text_field, "statement");
/// assert_eq!(descriptor.link_fields.get("enforces").map(String::as_str), Some("enforced-by"));
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TomlSourceDescriptor {
    /// Repository-local path to the `.toml` registry file (read through storage).
    pub toml: String,
    /// The array-of-tables key whose entries are projected (e.g. `"policies"`).
    pub table: String,
    /// The entry field supplying each item's self-id (e.g. `"id"`).
    #[serde(rename = "id-field")]
    pub id_field: String,
    /// The entry field supplying each item's display text (e.g. `"statement"`).
    #[serde(rename = "text-field")]
    pub text_field: String,
    /// Map of link NAMESPACE to the entry field holding its targets. Each mapped
    /// field's string (or string-array) values become `<namespace>:<target>`
    /// labels. Optional; defaults to no link fields. A [`BTreeMap`] keeps label
    /// ordering deterministic across runs.
    ///
    /// [`BTreeMap`]: std::collections::BTreeMap
    #[serde(rename = "link-fields", default)]
    pub link_fields: std::collections::BTreeMap<String, String>,
}

/// Parse error for [`KindScopeConfig`].
///
/// Returned by [`KindScopeConfig::from_str`] and (via serde) by TOML
/// deserialization when the `scope` value is not one of the recognised tokens.
///
/// # Examples
///
/// ```
/// use jit::config::KindScopeConfig;
/// let err = "global".parse::<KindScopeConfig>().unwrap_err();
/// assert!(err.to_string().contains("global"));
/// ```
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KindScopeConfigError {
    /// The value is not one of `"issue"` or `"project"`.
    #[error("invalid scope '{value}'; expected 'issue' or 'project'")]
    Invalid {
        /// The unrecognised value.
        value: String,
    },
}

/// Addressing scope of an item kind: `"issue"` or `"project"`.
///
/// This is the fifth field of the item-kind six-tuple `(section, id-pattern,
/// markers, link-namespaces, scope, source-of-truth)`. It mirrors
/// [`KindScope`](crate::domain::item::KindScope) from the domain layer but
/// lives in the config layer so unrecognised tokens fail at TOML parse time
/// rather than at projection time.
///
/// Parsed (case-insensitively) from the tokens `"issue"` / `"project"` by both
/// TOML deserialization and [`KindScopeConfig::from_str`]; an unknown token is a
/// descriptive error, not a silent fallback.
///
/// # Examples
///
/// ```
/// use jit::config::{ItemKindConfig, KindScopeConfig};
///
/// // Parsed from its token inside an item-kind declaration.
/// let cfg: ItemKindConfig = toml::from_str("scope = \"project\"").unwrap();
/// assert_eq!(cfg.scope, Some(KindScopeConfig::Project));
///
/// // FromStr also works (case-insensitive).
/// assert_eq!("ISSUE".parse::<KindScopeConfig>().unwrap(), KindScopeConfig::Issue);
///
/// // An invalid token is a descriptive error, not a silent default.
/// let err = toml::from_str::<ItemKindConfig>("scope = \"global\"").unwrap_err();
/// assert!(err.to_string().contains("global"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScopeConfig {
    /// Items are projected from issue descriptions (`<issue>/<self-id>`).
    Issue,
    /// Items are projected from a config-declared source file (`@/<self-id>`).
    Project,
}

impl std::str::FromStr for KindScopeConfig {
    type Err = KindScopeConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "issue" => Ok(KindScopeConfig::Issue),
            "project" => Ok(KindScopeConfig::Project),
            _ => Err(KindScopeConfigError::Invalid {
                value: s.to_string(),
            }),
        }
    }
}

impl<'de> serde::Deserialize<'de> for KindScopeConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<KindScopeConfig>()
            .map_err(serde::de::Error::custom)
    }
}

/// The authoring DIRECTION of an item kind: which substrate is the canonical
/// source for its items.
///
/// This is the sixth field of the item-kind six-tuple `(section, id-pattern,
/// marker(s), link-namespace(s), scope, source-of-truth)`. It is DISTINCT from
/// the [`ItemKindConfig::source`] file PATH: `source-of-truth` is a direction,
/// `source` is a location. A `markdown-first` kind (the requirement default) is
/// authored in prose and indexed from it; a `registry-first` kind is authored in
/// a structured registry file and projected from it.
///
/// Deserialized from the kebab-case tokens `"markdown-first"` / `"registry-first"`
/// via serde rename; an unrecognized value is a descriptive parse error rather
/// than a silent default.
///
/// # Examples
///
/// ```
/// use jit::config::{ItemKindConfig, SourceOfTruth};
///
/// // The default direction is markdown-first.
/// assert_eq!(SourceOfTruth::default(), SourceOfTruth::MarkdownFirst);
///
/// // Parsed from its kebab-case token inside an item-kind declaration.
/// let cfg: ItemKindConfig =
///     toml::from_str("source-of-truth = \"registry-first\"").unwrap();
/// assert_eq!(cfg.source_of_truth, Some(SourceOfTruth::RegistryFirst));
///
/// // An invalid token is a descriptive error, not a silent default.
/// let err = toml::from_str::<ItemKindConfig>("source-of-truth = \"both\"")
///     .unwrap_err();
/// assert!(err.to_string().contains("markdown-first"));
/// ```
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
pub enum SourceOfTruth {
    /// Items are authored in markdown prose (issue descriptions or a markdown
    /// source file) and indexed from it. The requirement-kind default.
    #[default]
    #[serde(rename = "markdown-first")]
    MarkdownFirst,
    /// Items are authored in a structured registry file and projected from it.
    #[serde(rename = "registry-first")]
    RegistryFirst,
}

/// The shipped DEFAULT documentation target for invariant projection.
///
/// A separate jit-owned file under `.jit/` so the default behavior never touches
/// existing project docs (decision D3). This is the SOLE place the default
/// filename lives: the projection engine reads the resolved target from config
/// and contains no documentation-filename literal. It is the value
/// [`InvariantProjectionConfig::target`] resolves to when no `target` is set.
///
/// # Examples
///
/// ```
/// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_PROJECTION_TARGET};
///
/// assert_eq!(DEFAULT_INVARIANT_PROJECTION_TARGET, ".jit/invariants.md");
/// // The default config resolves its target to this const.
/// assert_eq!(
///     InvariantProjectionConfig::default().target(),
///     DEFAULT_INVARIANT_PROJECTION_TARGET
/// );
/// ```
pub const DEFAULT_INVARIANT_PROJECTION_TARGET: &str = ".jit/invariants.md";

/// The default begin marker delimiting the invariant region in `region` mode.
///
/// Used by [`InvariantProjectionConfig::region_begin`] when no `region-begin` is
/// configured.
///
/// # Examples
///
/// ```
/// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_REGION_BEGIN};
///
/// assert_eq!(DEFAULT_INVARIANT_REGION_BEGIN, "<!-- jit:invariants:begin -->");
/// assert_eq!(
///     InvariantProjectionConfig::default().region_begin(),
///     DEFAULT_INVARIANT_REGION_BEGIN
/// );
/// ```
pub const DEFAULT_INVARIANT_REGION_BEGIN: &str = "<!-- jit:invariants:begin -->";

/// The default end marker delimiting the invariant region in `region` mode.
///
/// Used by [`InvariantProjectionConfig::region_end`] when no `region-end` is
/// configured.
///
/// # Examples
///
/// ```
/// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_REGION_END};
///
/// assert_eq!(DEFAULT_INVARIANT_REGION_END, "<!-- jit:invariants:end -->");
/// assert_eq!(
///     InvariantProjectionConfig::default().region_end(),
///     DEFAULT_INVARIANT_REGION_END
/// );
/// ```
pub const DEFAULT_INVARIANT_REGION_END: &str = "<!-- jit:invariants:end -->";

/// Where and how the invariant registry projects into human-readable docs.
///
/// Mirrors the `[item_kinds]` / `[namespaces.*]` registry precedent: an optional
/// `[invariant_projection]` table on [`JitConfig`]. When the table is ABSENT, the
/// engine uses [`InvariantProjectionConfig::default`] — separate-file mode
/// targeting [`DEFAULT_INVARIANT_PROJECTION_TARGET`] — so the default never
/// touches existing docs (decision D3). The `target` path and region delimiters
/// are config-driven; the projection engine hardcodes no documentation filename.
///
/// # Examples
///
/// ```
/// use jit::config::{
///     InvariantProjectionConfig, ProjectionMode, DEFAULT_INVARIANT_PROJECTION_TARGET,
/// };
///
/// // The default targets a separate jit-owned file.
/// let default = InvariantProjectionConfig::default();
/// assert_eq!(default.mode(), ProjectionMode::SeparateFile);
/// assert_eq!(default.target(), DEFAULT_INVARIANT_PROJECTION_TARGET);
///
/// // Region mode is opt-in via the `[invariant_projection]` table.
/// let cfg: InvariantProjectionConfig = toml::from_str(
///     r#"
/// mode = "region"
/// target = "docs/invariants.md"
/// region-begin = "<!-- INV START -->"
/// region-end = "<!-- INV END -->"
/// "#,
/// )
/// .unwrap();
/// assert_eq!(cfg.mode(), ProjectionMode::Region);
/// assert_eq!(cfg.target(), "docs/invariants.md");
/// assert_eq!(cfg.region_begin(), "<!-- INV START -->");
/// ```
///
/// All fields are `Option`s defaulting to `None`: a `None` accessor resolves to
/// its config-layer const default, so [`InvariantProjectionConfig::default`] (an
/// absent `[invariant_projection]` table) is separate-file mode targeting the
/// jit-owned file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct InvariantProjectionConfig {
    /// Projection mode: a separate jit-owned file or a delimited region within an
    /// existing file. Defaults to [`ProjectionMode::SeparateFile`] when unset.
    #[serde(default)]
    pub mode: Option<ProjectionMode>,
    /// Repo-relative path of the documentation target. Defaults to
    /// [`DEFAULT_INVARIANT_PROJECTION_TARGET`] when unset.
    #[serde(default)]
    pub target: Option<String>,
    /// Begin marker delimiting the rewritten region in `region` mode. Defaults to
    /// [`DEFAULT_INVARIANT_REGION_BEGIN`] when unset.
    #[serde(default, rename = "region-begin")]
    pub region_begin: Option<String>,
    /// End marker delimiting the rewritten region in `region` mode. Defaults to
    /// [`DEFAULT_INVARIANT_REGION_END`] when unset.
    #[serde(default, rename = "region-end")]
    pub region_end: Option<String>,
}

impl InvariantProjectionConfig {
    /// The resolved projection mode (defaulting to [`ProjectionMode::SeparateFile`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{InvariantProjectionConfig, ProjectionMode};
    ///
    /// assert_eq!(
    ///     InvariantProjectionConfig::default().mode(),
    ///     ProjectionMode::SeparateFile
    /// );
    /// ```
    pub fn mode(&self) -> ProjectionMode {
        self.mode.unwrap_or_default()
    }

    /// The resolved repo-relative target path (defaulting to the jit-owned file).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_PROJECTION_TARGET};
    ///
    /// assert_eq!(
    ///     InvariantProjectionConfig::default().target(),
    ///     DEFAULT_INVARIANT_PROJECTION_TARGET
    /// );
    /// ```
    pub fn target(&self) -> &str {
        self.target
            .as_deref()
            .unwrap_or(DEFAULT_INVARIANT_PROJECTION_TARGET)
    }

    /// The resolved begin marker for `region` mode (defaulting to the const).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_REGION_BEGIN};
    ///
    /// assert_eq!(
    ///     InvariantProjectionConfig::default().region_begin(),
    ///     DEFAULT_INVARIANT_REGION_BEGIN
    /// );
    /// ```
    pub fn region_begin(&self) -> &str {
        self.region_begin
            .as_deref()
            .unwrap_or(DEFAULT_INVARIANT_REGION_BEGIN)
    }

    /// The resolved end marker for `region` mode (defaulting to the const).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{InvariantProjectionConfig, DEFAULT_INVARIANT_REGION_END};
    ///
    /// assert_eq!(
    ///     InvariantProjectionConfig::default().region_end(),
    ///     DEFAULT_INVARIANT_REGION_END
    /// );
    /// ```
    pub fn region_end(&self) -> &str {
        self.region_end
            .as_deref()
            .unwrap_or(DEFAULT_INVARIANT_REGION_END)
    }
}

/// How the invariant registry is projected into its documentation target.
///
/// Deserialized from the kebab-case tokens `"separate-file"` / `"region"` via
/// serde rename; an unrecognized value is a descriptive parse error rather than a
/// silent default. The shipped default (no `[invariant_projection]` table) is
/// [`ProjectionMode::SeparateFile`].
///
/// # Examples
///
/// ```
/// use jit::config::{InvariantProjectionConfig, ProjectionMode};
///
/// // The default mode is separate-file.
/// assert_eq!(ProjectionMode::default(), ProjectionMode::SeparateFile);
///
/// // Parsed from its kebab-case token inside the projection table.
/// let cfg: InvariantProjectionConfig = toml::from_str("mode = \"region\"").unwrap();
/// assert_eq!(cfg.mode, Some(ProjectionMode::Region));
///
/// // An invalid token is a descriptive error, not a silent default.
/// let err = toml::from_str::<InvariantProjectionConfig>("mode = \"inline\"").unwrap_err();
/// assert!(err.to_string().contains("separate-file"));
/// ```
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
pub enum ProjectionMode {
    /// Write the rendered invariants to a separate jit-owned file (the default).
    #[default]
    #[serde(rename = "separate-file")]
    SeparateFile,
    /// Replace only a delimited region within an existing file, byte-preserving
    /// everything outside the delimiters.
    #[serde(rename = "region")]
    Region,
}

/// An error validating an explicitly-declared `[item_kinds.X]` table.
///
/// Raised by [`JitConfig::validate_item_kinds`] (and thus [`JitConfig::load`])
/// when a declared kind omits one or more of its six required fields. The
/// implicit built-in default kinds (`requirement`, `decision`, ...; no
/// `[item_kinds]` table at all) are never validated, so graceful degradation is
/// preserved.
///
/// # Examples
///
/// ```
/// use jit::config::JitConfig;
///
/// // A partial explicit declaration is rejected, naming the kind and the
/// // missing fields.
/// let err = toml::from_str::<JitConfig>("[item_kinds.requirement]\nsection = \"sc\"\n")
///     .unwrap()
///     .validate_item_kinds()
///     .unwrap_err();
/// assert!(err.to_string().contains("requirement"));
/// assert!(err.to_string().contains("source-of-truth"));
/// ```
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ItemKindConfigError {
    /// An explicitly-declared kind omits one or more required six-tuple fields.
    #[error(
        "[item_kinds.{kind}] is missing required field(s): {missing}; \
         an explicitly-declared item kind must set all six of \
         section, id-pattern, markers, link-namespaces, scope, source-of-truth"
    )]
    MissingFields {
        /// The offending kind name.
        kind: String,
        /// Comma-separated authored keys of the missing fields.
        missing: String,
    },
}

/// Parse error for [`WorktreeMode`].
///
/// Returned by [`WorktreeMode::from_str`] and (via serde) by TOML /
/// environment-variable deserialization when the value is not one of the
/// recognised tokens.
///
/// # Examples
///
/// ```
/// use jit::config::WorktreeMode;
/// let err = "bogus".parse::<WorktreeMode>().unwrap_err();
/// assert!(err.to_string().contains("bogus"));
/// ```
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorktreeModeError {
    /// The value is not one of `"auto"`, `"on"`, or `"off"`.
    #[error("invalid worktree mode '{value}'; expected 'auto', 'on', or 'off'")]
    Invalid {
        /// The unrecognised value.
        value: String,
    },
}

/// Parse error for [`EnforcementMode`].
///
/// Returned by [`EnforcementMode::from_str`] and (via serde) by TOML /
/// environment-variable deserialization when the value is not one of the
/// recognised tokens.
///
/// # Examples
///
/// ```
/// use jit::config::EnforcementMode;
/// let err = "bogus".parse::<EnforcementMode>().unwrap_err();
/// assert!(err.to_string().contains("bogus"));
/// ```
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnforcementModeError {
    /// The value is not one of `"strict"`, `"warn"`, or `"off"`.
    #[error("invalid enforcement mode '{value}'; expected 'strict', 'warn', or 'off'")]
    Invalid {
        /// The unrecognised value.
        value: String,
    },
}

/// Worktree and parallel work configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeConfig {
    /// Worktree mode (default: [`WorktreeMode::Auto`]).
    pub mode: Option<WorktreeMode>,
    /// Lease enforcement mode (default: [`EnforcementMode::Strict`]).
    pub enforce_leases: Option<EnforcementMode>,
}

/// Worktree detection mode.
///
/// Parsed (case-insensitively) from the tokens `"auto"` / `"on"` / `"off"` by
/// both TOML configuration (`[worktree] mode = ...`) and the
/// `JIT_WORKTREE_MODE` environment variable, so the two sources share one
/// case-handling rule.
///
/// # Examples
///
/// ```
/// use jit::config::WorktreeMode;
///
/// assert_eq!("auto".parse::<WorktreeMode>().unwrap(), WorktreeMode::Auto);
/// assert_eq!("ON".parse::<WorktreeMode>().unwrap(), WorktreeMode::On);
/// assert!("bogus".parse::<WorktreeMode>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeMode {
    /// Detect git worktree and enable automatically (default).
    Auto,
    /// Force worktree mode (fail if not in worktree).
    On,
    /// Disable worktree features (use legacy .jit/ only).
    Off,
}

impl std::str::FromStr for WorktreeMode {
    type Err = WorktreeModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(WorktreeMode::Auto),
            "on" => Ok(WorktreeMode::On),
            "off" => Ok(WorktreeMode::Off),
            _ => Err(WorktreeModeError::Invalid {
                value: s.to_string(),
            }),
        }
    }
}

impl<'de> serde::Deserialize<'de> for WorktreeMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<WorktreeMode>().map_err(serde::de::Error::custom)
    }
}

/// Enforcement mode for lease requirements.
///
/// Parsed (case-insensitively) from the tokens `"strict"` / `"warn"` / `"off"` by
/// both TOML configuration (`[worktree] enforce_leases = ...`) and the
/// `JIT_ENFORCE_LEASES` environment variable, so the two sources share one
/// case-handling rule.
///
/// # Examples
///
/// ```
/// use jit::config::EnforcementMode;
///
/// assert_eq!("strict".parse::<EnforcementMode>().unwrap(), EnforcementMode::Strict);
/// assert_eq!("WARN".parse::<EnforcementMode>().unwrap(), EnforcementMode::Warn);
/// assert!("bogus".parse::<EnforcementMode>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Block operations without active lease (production-safe default).
    Strict,
    /// Warn but allow operations without lease (development-friendly).
    Warn,
    /// No enforcement - bypass lease checks (backward compatible).
    Off,
}

impl std::str::FromStr for EnforcementMode {
    type Err = EnforcementModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(EnforcementMode::Strict),
            "warn" => Ok(EnforcementMode::Warn),
            "off" => Ok(EnforcementMode::Off),
            _ => Err(EnforcementModeError::Invalid {
                value: s.to_string(),
            }),
        }
    }
}

impl<'de> serde::Deserialize<'de> for EnforcementMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<EnforcementMode>()
            .map_err(serde::de::Error::custom)
    }
}

impl WorktreeConfig {
    /// Get the worktree mode, defaulting to [`WorktreeMode::Auto`] when unset.
    ///
    /// The field is typed — invalid tokens are rejected at TOML parse time, so
    /// this method is infallible.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{WorktreeConfig, WorktreeMode};
    ///
    /// // An absent field defaults to Auto.
    /// let wt: WorktreeConfig = toml::from_str("").unwrap();
    /// assert_eq!(wt.worktree_mode(), WorktreeMode::Auto);
    ///
    /// // An explicit value is returned as-is.
    /// let wt: WorktreeConfig = toml::from_str("mode = \"on\"").unwrap();
    /// assert_eq!(wt.worktree_mode(), WorktreeMode::On);
    /// ```
    pub fn worktree_mode(&self) -> WorktreeMode {
        self.mode.unwrap_or(WorktreeMode::Auto)
    }

    /// Get the enforcement mode, defaulting to [`EnforcementMode::Strict`] when unset.
    ///
    /// The field is typed — invalid tokens are rejected at TOML parse time, so
    /// this method is infallible.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::{WorktreeConfig, EnforcementMode};
    ///
    /// // An absent field defaults to Strict.
    /// let wt: WorktreeConfig = toml::from_str("").unwrap();
    /// assert_eq!(wt.enforcement_mode(), EnforcementMode::Strict);
    ///
    /// // An explicit value is returned as-is.
    /// let wt: WorktreeConfig = toml::from_str("enforce_leases = \"warn\"").unwrap();
    /// assert_eq!(wt.enforcement_mode(), EnforcementMode::Warn);
    /// ```
    pub fn enforcement_mode(&self) -> EnforcementMode {
        self.enforce_leases.unwrap_or(EnforcementMode::Strict)
    }
}

/// Coordination settings for leases and multi-agent work.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CoordinationConfig {
    /// Default TTL for new leases in seconds (default: 600).
    pub default_ttl_secs: Option<u64>,
    /// Heartbeat interval for automatic lease renewal in seconds (default: 30).
    pub heartbeat_interval_secs: Option<u64>,
    /// Warn when lease has less than this percentage of TTL remaining (default: 10).
    pub lease_renewal_threshold_pct: Option<u8>,
    /// Staleness threshold for TTL=0 leases in seconds (default: 3600).
    pub stale_threshold_secs: Option<u64>,
    /// Maximum concurrent TTL=0 leases per agent (default: 2).
    pub max_indefinite_leases_per_agent: Option<u32>,
    /// Maximum concurrent TTL=0 leases per repository (default: 10).
    pub max_indefinite_leases_per_repo: Option<u32>,
    /// Automatic lease renewal by heartbeat daemon (default: false).
    pub auto_renew_leases: Option<bool>,
}

impl CoordinationConfig {
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs.unwrap_or(600)
    }

    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.heartbeat_interval_secs.unwrap_or(30)
    }

    pub fn lease_renewal_threshold_pct(&self) -> u8 {
        self.lease_renewal_threshold_pct.unwrap_or(10)
    }

    pub fn stale_threshold_secs(&self) -> u64 {
        self.stale_threshold_secs.unwrap_or(3600)
    }

    pub fn max_indefinite_leases_per_agent(&self) -> u32 {
        self.max_indefinite_leases_per_agent.unwrap_or(2)
    }

    pub fn max_indefinite_leases_per_repo(&self) -> u32 {
        self.max_indefinite_leases_per_repo.unwrap_or(10)
    }

    pub fn auto_renew_leases(&self) -> bool {
        self.auto_renew_leases.unwrap_or(false)
    }
}

/// Global operations configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GlobalOperationsConfig {
    /// Require common history with main for global operations (default: true).
    pub require_main_history: Option<bool>,
    /// Branches allowed to modify global config (default: ["main"]).
    pub allowed_branches: Option<Vec<String>>,
}

impl GlobalOperationsConfig {
    pub fn require_main_history(&self) -> bool {
        self.require_main_history.unwrap_or(true)
    }

    pub fn allowed_branches(&self) -> Vec<String> {
        self.allowed_branches
            .clone()
            .unwrap_or_else(|| vec!["main".to_string()])
    }
}

/// Lock file configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LocksConfig {
    /// Maximum age for lock files before considered stale in seconds (default: 3600).
    pub max_age_secs: Option<u64>,
    /// Enable lock metadata for diagnostics (default: true).
    pub enable_metadata: Option<bool>,
}

impl LocksConfig {
    pub fn max_age_secs(&self) -> u64 {
        self.max_age_secs.unwrap_or(3600)
    }

    pub fn enable_metadata(&self) -> bool {
        self.enable_metadata.unwrap_or(true)
    }
}

/// Event logging configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EventsConfig {
    /// Enable sequence numbers in event logs (default: true).
    pub enable_sequences: Option<bool>,
    /// Standardize event envelopes across control and data plane (default: true).
    pub use_unified_envelope: Option<bool>,
}

impl EventsConfig {
    pub fn enable_sequences(&self) -> bool {
        self.enable_sequences.unwrap_or(true)
    }

    pub fn use_unified_envelope(&self) -> bool {
        self.use_unified_envelope.unwrap_or(true)
    }
}

// ============================================================
// Agent Configuration (separate from repository config)
// ============================================================

/// Agent configuration loaded from `~/.config/jit/agent.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Agent identity section.
    pub agent: AgentIdentity,
    /// Agent behavior section (optional).
    pub behavior: Option<AgentBehavior>,
}

/// Agent identity configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentIdentity {
    /// Persistent agent identity (format: type:identifier, e.g., "agent:copilot-1").
    pub id: String,
    /// When this identity was created (ISO 8601 timestamp).
    pub created_at: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Default TTL preference in seconds.
    pub default_ttl_secs: Option<u64>,
}

impl AgentIdentity {
    /// Get the default TTL, falling back to coordination default (600s).
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs.unwrap_or(600)
    }
}

/// Agent behavior configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentBehavior {
    /// Auto-start heartbeat daemon for lease renewal (default: false).
    pub auto_heartbeat: Option<bool>,
    /// Heartbeat interval in seconds (default: 30).
    pub heartbeat_interval: Option<u64>,
}

impl AgentBehavior {
    pub fn auto_heartbeat(&self) -> bool {
        self.auto_heartbeat.unwrap_or(false)
    }

    pub fn heartbeat_interval(&self) -> u64 {
        self.heartbeat_interval.unwrap_or(30)
    }
}

impl AgentConfig {
    /// Load agent configuration from `agent.toml` in the given directory.
    ///
    /// Returns `Ok(None)` if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(config_dir: &Path) -> Result<Option<Self>> {
        let config_path = config_dir.join("agent.toml");

        if !config_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&config_path).context("Failed to read agent.toml")?;

        let config: AgentConfig = toml::from_str(&content).context("Failed to parse agent.toml")?;

        Ok(Some(config))
    }
}

impl JitConfig {
    /// Load configuration from `.jit/config.toml` if it exists.
    ///
    /// Returns an empty config (all fields None) if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(jit_root: &Path) -> Result<Self> {
        let config_path = jit_root.join("config.toml");

        if !config_path.exists() {
            // No config file - return empty config (will use defaults)
            return Ok(JitConfig {
                version: None,
                type_hierarchy: None,
                validation: None,
                documentation: None,
                namespaces: None,
                item_kinds: None,
                invariant_projection: None,
                worktree: None,
                coordination: None,
                global_operations: None,
                locks: None,
                events: None,
                // No config.toml means no type hierarchy, so node-`type` checks
                // are skipped (empty slice); a sibling `templates.toml` is still
                // loaded and validated, and a load/validation error propagates
                // exactly as on the config-present path below.
                templates: crate::templates::TemplateRegistry::load(jit_root, &[] as &[&str])
                    .context("invalid .jit/templates.toml")?,
                // The invariant registry is independent of `config.toml`, so it
                // loads on this config-absent path too (absent file → empty
                // registry; a malformed/invalid entry fails config load with a
                // typed, descriptive error).
                invariants: crate::validation::invariants::InvariantRegistry::load(jit_root)
                    .context("invalid .jit/invariants.toml")?,
            });
        }

        let content =
            std::fs::read_to_string(&config_path).context("Failed to read config.toml")?;

        // An old `config.toml` may still carry removed enforcement keys
        // (`require_type_label`, namespace `values`/`pattern`/`required`, etc.).
        // serde ignores them (no `deny_unknown_fields`), so the file still parses;
        // the keys simply have no effect — `.jit/rules.toml` is the sole source.
        let mut config: JitConfig =
            toml::from_str(&content).context("Failed to parse config.toml")?;

        // Load and validate `.jit/templates.toml` at config time: an absent file
        // is fine (empty registry), an invalid template fails config load with a
        // descriptive error. Node
        // `type`s are checked against `[type_hierarchy].types` when a hierarchy
        // is configured (an empty slice skips that check).
        let hierarchy_types: Vec<&str> = config
            .type_hierarchy
            .as_ref()
            .map(|h| h.types.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        config.templates = crate::templates::TemplateRegistry::load(jit_root, &hierarchy_types)
            .context("invalid .jit/templates.toml")?;

        // Every EXPLICITLY-declared `[item_kinds.X]` table must set all six
        // required fields; reject a partial declaration with a descriptive error
        // rather than silently filling defaults (the implicit built-in default,
        // which has no table, is never reached here).
        config
            .validate_item_kinds()
            .context("invalid [item_kinds] in .jit/config.toml")?;

        // Chain-load `.jit/invariants.toml` on the config-present path too, with
        // the same graceful-absent / typed-error contract as the early return
        // above, so both load paths populate the registry identically.
        config.invariants = crate::validation::invariants::InvariantRegistry::load(jit_root)
            .context("invalid .jit/invariants.toml")?;

        Ok(config)
    }

    /// Validate every explicitly-declared `[item_kinds.X]` table, requiring all
    /// six fields (`section`, `id-pattern`, `markers`, `link-namespaces`, `scope`,
    /// `source-of-truth`) on each.
    ///
    /// Called by [`JitConfig::load`]. A `None` registry (no `[item_kinds]` table
    /// at all) is the IMPLICIT built-in path and validates trivially, so graceful
    /// degradation via the built-in default kinds (`requirement`, `decision`, ...)
    /// is preserved. Kinds are checked in
    /// name order so the first error is deterministic. The optional `source` PATH
    /// is not one of the six and is not required.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::JitConfig;
    ///
    /// // No `[item_kinds]` table -> nothing to validate (implicit default).
    /// let empty: JitConfig = toml::from_str("").unwrap();
    /// assert!(empty.validate_item_kinds().is_ok());
    ///
    /// // A complete declaration passes.
    /// let ok: JitConfig = toml::from_str(
    ///     r#"
    /// [item_kinds.requirement]
    /// section = "success_criteria"
    /// id-pattern = "REQ-\\d+"
    /// markers = ["[hard]"]
    /// link-namespaces = ["satisfies"]
    /// scope = "issue"
    /// source-of-truth = "markdown-first"
    /// "#,
    /// )
    /// .unwrap();
    /// assert!(ok.validate_item_kinds().is_ok());
    ///
    /// // A partial declaration is rejected.
    /// let bad: JitConfig =
    ///     toml::from_str("[item_kinds.decision]\nsection = \"decisions\"\n").unwrap();
    /// assert!(bad.validate_item_kinds().is_err());
    /// ```
    pub fn validate_item_kinds(&self) -> std::result::Result<(), ItemKindConfigError> {
        let Some(registry) = &self.item_kinds else {
            return Ok(());
        };
        let mut names: Vec<&String> = registry.keys().collect();
        names.sort();
        for name in names {
            let missing = registry[name].missing_required_fields();
            if !missing.is_empty() {
                return Err(ItemKindConfigError::MissingFields {
                    kind: name.clone(),
                    missing: missing.join(", "),
                });
            }
        }
        Ok(())
    }
}

// ============================================================
// Config Loader with Priority and Merging
// ============================================================

/// Builder for loading configuration from multiple sources with priority.
///
/// Priority order (highest to lowest):
/// 1. Repository config (`.jit/config.toml`)
/// 2. User config (`~/.config/jit/config.toml`)
/// 3. System config (`/etc/jit/config.toml`)
/// 4. Defaults (hardcoded)
#[derive(Debug, Default)]
pub struct ConfigLoader {
    system_config: Option<JitConfig>,
    user_config: Option<JitConfig>,
    repo_config: Option<JitConfig>,
}

impl ConfigLoader {
    /// Create a new config loader with only defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load and add system-level config (`/etc/jit/config.toml`).
    pub fn with_system_config(mut self, config_dir: &Path) -> Result<Self> {
        self.system_config = Some(JitConfig::load(config_dir)?);
        Ok(self)
    }

    /// Load and add user-level config (`~/.config/jit/config.toml`).
    pub fn with_user_config(mut self, config_dir: &Path) -> Result<Self> {
        self.user_config = Some(JitConfig::load(config_dir)?);
        Ok(self)
    }

    /// Load and add repository-level config (`.jit/config.toml`).
    pub fn with_repo_config(mut self, jit_root: &Path) -> Result<Self> {
        self.repo_config = Some(JitConfig::load(jit_root)?);
        Ok(self)
    }

    /// Build the effective configuration by merging all sources.
    pub fn build(self) -> EffectiveConfig {
        EffectiveConfig {
            system_config: self.system_config,
            user_config: self.user_config,
            repo_config: self.repo_config,
        }
    }
}

/// Merged configuration from all sources with priority resolution.
///
/// When accessing a config value, checks sources in order:
/// repo > user > system > default
#[derive(Debug, Default)]
pub struct EffectiveConfig {
    system_config: Option<JitConfig>,
    user_config: Option<JitConfig>,
    repo_config: Option<JitConfig>,
}

impl EffectiveConfig {
    /// Get the effective worktree mode.
    /// Priority: env var > repo > user > system > default
    ///
    /// Both the env-var and TOML paths share the same [`WorktreeMode::from_str`]
    /// implementation (case-insensitive), so the two sources handle case
    /// identically.
    pub fn worktree_mode(&self) -> Result<WorktreeMode> {
        // Check env var first (highest priority). Uses the same FromStr as TOML.
        if let Ok(val) = std::env::var("JIT_WORKTREE_MODE") {
            return val.parse::<WorktreeMode>().map_err(|e| {
                crate::errors::InvalidArgumentError::new(format!("invalid JIT_WORKTREE_MODE: {e}"))
                    .into()
            });
        }

        // Check repo first, then user, then system
        if let Some(ref cfg) = self.repo_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return Ok(wt.worktree_mode());
                }
            }
        }
        if let Some(ref cfg) = self.user_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return Ok(wt.worktree_mode());
                }
            }
        }
        if let Some(ref cfg) = self.system_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return Ok(wt.worktree_mode());
                }
            }
        }
        // Default
        Ok(WorktreeMode::Auto)
    }

    /// Get the effective enforcement mode.
    /// Priority: env var > repo > user > system > default
    ///
    /// Both the env-var and TOML paths share the same
    /// [`EnforcementMode::from_str`] implementation (case-insensitive), so the
    /// two sources handle case identically.
    pub fn enforcement_mode(&self) -> Result<EnforcementMode> {
        // Check env var first (highest priority). Uses the same FromStr as TOML.
        if let Ok(val) = std::env::var("JIT_ENFORCE_LEASES") {
            return val.parse::<EnforcementMode>().map_err(|e| {
                crate::errors::InvalidArgumentError::new(format!("invalid JIT_ENFORCE_LEASES: {e}"))
                    .into()
            });
        }

        if let Some(ref cfg) = self.repo_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return Ok(wt.enforcement_mode());
                }
            }
        }
        if let Some(ref cfg) = self.user_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return Ok(wt.enforcement_mode());
                }
            }
        }
        if let Some(ref cfg) = self.system_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return Ok(wt.enforcement_mode());
                }
            }
        }
        Ok(EnforcementMode::Strict)
    }

    /// Get the effective agent ID from environment variable.
    /// Returns None if JIT_AGENT_ID is not set.
    pub fn agent_id(&self) -> Option<String> {
        std::env::var("JIT_AGENT_ID").ok()
    }

    /// Get effective coordination config with merged values.
    pub fn coordination(&self) -> MergedCoordinationConfig {
        MergedCoordinationConfig {
            repo: self
                .repo_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
            user: self
                .user_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
            system: self
                .system_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
        }
    }

    /// Get effective global operations config with merged values.
    pub fn global_operations(&self) -> MergedGlobalOperationsConfig {
        MergedGlobalOperationsConfig {
            repo: self
                .repo_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
            user: self
                .user_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
            system: self
                .system_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
        }
    }

    /// Get effective locks config with merged values.
    pub fn locks(&self) -> MergedLocksConfig {
        MergedLocksConfig {
            repo: self.repo_config.as_ref().and_then(|c| c.locks.clone()),
            user: self.user_config.as_ref().and_then(|c| c.locks.clone()),
            system: self.system_config.as_ref().and_then(|c| c.locks.clone()),
        }
    }

    /// Get effective events config with merged values.
    pub fn events(&self) -> MergedEventsConfig {
        MergedEventsConfig {
            repo: self.repo_config.as_ref().and_then(|c| c.events.clone()),
            user: self.user_config.as_ref().and_then(|c| c.events.clone()),
            system: self.system_config.as_ref().and_then(|c| c.events.clone()),
        }
    }
}

/// Merged coordination config with priority resolution per field.
#[derive(Debug)]
pub struct MergedCoordinationConfig {
    repo: Option<CoordinationConfig>,
    user: Option<CoordinationConfig>,
    system: Option<CoordinationConfig>,
}

impl MergedCoordinationConfig {
    pub fn default_ttl_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.default_ttl_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.default_ttl_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.default_ttl_secs))
            .unwrap_or(600)
    }

    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.heartbeat_interval_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.heartbeat_interval_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.heartbeat_interval_secs))
            .unwrap_or(30)
    }

    pub fn lease_renewal_threshold_pct(&self) -> u8 {
        self.repo
            .as_ref()
            .and_then(|c| c.lease_renewal_threshold_pct)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.lease_renewal_threshold_pct)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.lease_renewal_threshold_pct)
            })
            .unwrap_or(10)
    }

    pub fn stale_threshold_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.stale_threshold_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.stale_threshold_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.stale_threshold_secs))
            .unwrap_or(3600)
    }

    pub fn max_indefinite_leases_per_agent(&self) -> u32 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_indefinite_leases_per_agent)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_agent)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_agent)
            })
            .unwrap_or(2)
    }

    pub fn max_indefinite_leases_per_repo(&self) -> u32 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_indefinite_leases_per_repo)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_repo)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_repo)
            })
            .unwrap_or(10)
    }

    pub fn auto_renew_leases(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.auto_renew_leases)
            .or_else(|| self.user.as_ref().and_then(|c| c.auto_renew_leases))
            .or_else(|| self.system.as_ref().and_then(|c| c.auto_renew_leases))
            .unwrap_or(false)
    }
}

/// Merged global operations config with priority resolution per field.
#[derive(Debug)]
pub struct MergedGlobalOperationsConfig {
    repo: Option<GlobalOperationsConfig>,
    user: Option<GlobalOperationsConfig>,
    system: Option<GlobalOperationsConfig>,
}

impl MergedGlobalOperationsConfig {
    pub fn require_main_history(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.require_main_history)
            .or_else(|| self.user.as_ref().and_then(|c| c.require_main_history))
            .or_else(|| self.system.as_ref().and_then(|c| c.require_main_history))
            .unwrap_or(true)
    }

    pub fn allowed_branches(&self) -> Vec<String> {
        self.repo
            .as_ref()
            .and_then(|c| c.allowed_branches.clone())
            .or_else(|| self.user.as_ref().and_then(|c| c.allowed_branches.clone()))
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.allowed_branches.clone())
            })
            .unwrap_or_else(|| vec!["main".to_string()])
    }
}

/// Merged locks config with priority resolution per field.
#[derive(Debug)]
pub struct MergedLocksConfig {
    repo: Option<LocksConfig>,
    user: Option<LocksConfig>,
    system: Option<LocksConfig>,
}

impl MergedLocksConfig {
    pub fn max_age_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_age_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.max_age_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.max_age_secs))
            .unwrap_or(3600)
    }

    pub fn enable_metadata(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.enable_metadata)
            .or_else(|| self.user.as_ref().and_then(|c| c.enable_metadata))
            .or_else(|| self.system.as_ref().and_then(|c| c.enable_metadata))
            .unwrap_or(true)
    }
}

/// Merged events config with priority resolution per field.
#[derive(Debug)]
pub struct MergedEventsConfig {
    repo: Option<EventsConfig>,
    user: Option<EventsConfig>,
    system: Option<EventsConfig>,
}

impl MergedEventsConfig {
    pub fn enable_sequences(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.enable_sequences)
            .or_else(|| self.user.as_ref().and_then(|c| c.enable_sequences))
            .or_else(|| self.system.as_ref().and_then(|c| c.enable_sequences))
            .unwrap_or(true)
    }

    pub fn use_unified_envelope(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.use_unified_envelope)
            .or_else(|| self.user.as_ref().and_then(|c| c.use_unified_envelope))
            .or_else(|| self.system.as_ref().and_then(|c| c.use_unified_envelope))
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_MUTEX.lock().expect("environment test mutex poisoned")
    }

    #[test]
    fn test_load_templates_when_config_absent() {
        // A repo with `.jit/templates.toml` but NO `.jit/config.toml` must still
        // load the registry (the absent-config path), not silently return empty.
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("templates.toml"),
            r#"
[[template]]
name = "plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
doc = "dev/active/{container.id}-plan.md"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
depends_on = ["planning"]
"#,
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        let plan = config
            .templates
            .get("plan")
            .expect("templates.toml is loaded even without config.toml");
        assert_eq!(plan.planning_type(), Some("planning"));
        assert_eq!(plan.breakdown_type(), Some("breakdown"));
        assert_eq!(config.templates.breakable_types(), vec!["epic".to_string()]);
    }

    #[test]
    fn test_load_invariants_when_config_absent() {
        // A repo with `.jit/invariants.toml` but NO `.jit/config.toml` must still
        // load the registry (the config-absent early-return path).
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("invariants.toml"),
            r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"
"#,
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        assert_eq!(config.invariants.invariants.len(), 1);
        let inv = &config.invariants.invariants[0];
        assert_eq!(inv.id, "INV-01");
        assert_eq!(
            inv.kind,
            crate::validation::invariants::InvariantKind::Enforced
        );
        assert_eq!(inv.enforced_by.as_deref(), Some("dag-no-cycles"));
    }

    #[test]
    fn test_load_invariants_when_config_present() {
        // With BOTH `.jit/config.toml` and `.jit/invariants.toml`, the
        // config-present path also chain-loads the invariant registry.
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[type_hierarchy]\ntypes = { task = 1 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("invariants.toml"),
            "[[invariants]]\nid = \"INV-02\"\nstatement = \"s\"\nkind = \"advisory\"\n",
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.invariants.invariants.len(), 1);
        assert_eq!(config.invariants.invariants[0].id, "INV-02");
    }

    #[test]
    fn test_load_invariants_absent_file_is_empty_on_both_paths() {
        // No `invariants.toml` → empty registry, whether or not config.toml exists.
        let no_config = TempDir::new().unwrap();
        assert!(JitConfig::load(no_config.path())
            .unwrap()
            .invariants
            .invariants
            .is_empty());

        let with_config = TempDir::new().unwrap();
        std::fs::write(
            with_config.path().join("config.toml"),
            "[type_hierarchy]\ntypes = { task = 1 }\n",
        )
        .unwrap();
        assert!(JitConfig::load(with_config.path())
            .unwrap()
            .invariants
            .invariants
            .is_empty());
    }

    #[test]
    fn test_load_invalid_invariant_fails_config_load_both_paths() {
        // A malformed invariant entry fails config load with a descriptive,
        // context-bearing error on BOTH the config-absent and config-present paths.
        let bad = "[[invariants]]\nid = \"INV-01\"\nkind = \"advisory\"\n"; // missing statement

        let no_config = TempDir::new().unwrap();
        std::fs::write(no_config.path().join("invariants.toml"), bad).unwrap();
        let err = JitConfig::load(no_config.path()).unwrap_err();
        assert!(err.to_string().contains("invariants.toml"), "{err:#}");

        let with_config = TempDir::new().unwrap();
        std::fs::write(
            with_config.path().join("config.toml"),
            "[type_hierarchy]\ntypes = { task = 1 }\n",
        )
        .unwrap();
        std::fs::write(with_config.path().join("invariants.toml"), bad).unwrap();
        let err = JitConfig::load(with_config.path()).unwrap_err();
        assert!(err.to_string().contains("invariants.toml"), "{err:#}");
    }

    #[test]
    fn test_parse_minimal_config() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
strictness = "loose"
default_type = "task"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 3);
        assert_eq!(hierarchy.label_associations.as_ref().unwrap().len(), 2);

        let validation = config.validation.unwrap();
        assert_eq!(validation.strictness, Some("loose".to_string()));
        assert_eq!(validation.default_type, Some("task".to_string()));
    }

    #[test]
    fn test_load_missing_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = JitConfig::load(temp_dir.path()).unwrap();

        // Empty config when file doesn't exist
        assert!(config.type_hierarchy.is_none());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_load_existing_config() {
        let temp_dir = TempDir::new().unwrap();

        let config_toml = r#"
[type_hierarchy]
types = { epic = 1, task = 2 }
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let config = JitConfig::load(temp_dir.path()).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.type_hierarchy.unwrap().types.len(), 2);
    }

    #[test]
    fn test_malformed_toml_returns_error() {
        let temp_dir = TempDir::new().unwrap();

        let bad_toml = "[broken syntax";
        std::fs::write(temp_dir.path().join("config.toml"), bad_toml).unwrap();

        let result = JitConfig::load(temp_dir.path());
        assert!(result.is_err());
    }

    // ============================================================
    // Deprecated-key scan (DR §8.4, decision D7) — warn, never hard-error
    // ============================================================
    // Stale-key tolerance: removed enforcement keys are ignored, not errors
    // ============================================================

    #[test]
    fn test_load_config_with_removed_keys_still_parses() {
        // An OLD config carrying the removed enforcement / namespace-constraint
        // keys still loads (no `deny_unknown_fields`): serde ignores them, the
        // surviving behavioral keys parse, and the registry taxonomy is intact.
        // `.jit/rules.toml` is the sole validation source, so the stale keys have
        // no effect.
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[validation]
default_type = "task"
strictness = "loose"
content_format = "markdown"
require_type_label = true
label_regex = '^[a-z]+:'
reject_malformed_labels = true
enforce_namespace_registry = true
warn_orphaned_leaves = false
warn_strategic_consistency = false

[namespaces.type]
description = "Issue type"
unique = true
examples = ["type:task"]
values = ["task", "bug"]
required = true
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let config = JitConfig::load(temp_dir.path()).expect("stale config must still load");
        // It parsed; the surviving keys are intact.
        let validation = config.validation.expect("validation section present");
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.strictness, Some("loose".to_string()));
        assert_eq!(validation.content_format, Some("markdown".to_string()));
        // The namespace registry survives with only its taxonomy keys.
        let namespaces = config.namespaces.expect("namespaces present");
        let type_ns = &namespaces["type"];
        assert_eq!(type_ns.description, "Issue type");
        assert!(type_ns.unique);
    }

    #[test]
    fn test_parse_schema_v2_with_version() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        assert!(config.version.is_some());
        assert_eq!(config.version.unwrap().schema, 2);

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );
    }

    #[test]
    fn test_parse_validation_behavioral_fields() {
        // Only the behavioral keys survive in [validation].
        let config_toml = r#"
[validation]
default_type = "task"
strictness = "loose"
content_format = "html"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.strictness, Some("loose".to_string()));
        assert_eq!(validation.content_format, Some("html".to_string()));
    }

    #[test]
    fn test_parse_namespaces_from_toml() {
        let config_toml = r#"
[namespaces.type]
description = "Issue type (hierarchical)"
unique = true
examples = ["type:task", "type:epic"]

[namespaces.epic]
description = "Feature or initiative membership"
unique = false
examples = ["epic:auth", "epic:billing"]

[namespaces.component]
description = "Technical area"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 3);

        let type_ns = &namespaces["type"];
        assert_eq!(type_ns.description, "Issue type (hierarchical)");
        assert!(type_ns.unique);
        assert_eq!(
            type_ns.examples,
            Some(vec!["type:task".to_string(), "type:epic".to_string()])
        );

        let epic_ns = &namespaces["epic"];
        assert!(!epic_ns.unique);

        let component_ns = &namespaces["component"];
        assert!(component_ns.examples.is_none());
    }

    #[test]
    fn test_parse_item_kinds_from_toml() {
        // The `[item_kinds.*]` registry parses as a name -> six-tuple map,
        // mirroring `[namespaces.*]`. An explicit declaration sets all six fields.
        let config_toml = r#"
[item_kinds.requirement]
section = "success_criteria"
id-pattern = "REQ-\\d+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"

[item_kinds.decision]
section = "decisions"
id-pattern = "D-\\d+"
markers = []
link-namespaces = ["per"]
scope = "issue"
source-of-truth = "markdown-first"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        // A complete registry validates.
        config
            .validate_item_kinds()
            .expect("complete kinds validate");
        let kinds = config.item_kinds.as_ref().expect("item_kinds present");
        assert_eq!(kinds.len(), 2);

        let req = &kinds["requirement"];
        assert_eq!(req.section.as_deref(), Some("success_criteria"));
        assert_eq!(req.id_pattern.as_deref(), Some("REQ-\\d+"));
        assert_eq!(req.markers, Some(vec!["[hard]".to_string()]));
        assert_eq!(req.link_namespaces, Some(vec!["satisfies".to_string()]));
        assert_eq!(req.scope, Some(KindScopeConfig::Issue));
        assert_eq!(req.source_of_truth, Some(SourceOfTruth::MarkdownFirst));
        assert!(req.missing_required_fields().is_empty());

        // An empty `markers = []` is a PRESENT value (not missing).
        let decision = &kinds["decision"];
        assert_eq!(decision.markers, Some(vec![]));
        assert!(decision.missing_required_fields().is_empty());
    }

    #[test]
    fn test_item_kinds_explicit_partial_declaration_is_rejected() {
        // REQ-01: an explicitly-declared kind missing any of the six required
        // fields is rejected by validation, naming the kind and the missing keys.
        let config_toml = r#"
[item_kinds.requirement]
section = "success_criteria"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let err = config
            .validate_item_kinds()
            .expect_err("partial declaration must be rejected");
        match err {
            ItemKindConfigError::MissingFields { kind, missing } => {
                assert_eq!(kind, "requirement");
                // Every absent required field is named; `section` (present) is not.
                for field in [
                    "id-pattern",
                    "markers",
                    "link-namespaces",
                    "scope",
                    "source-of-truth",
                ] {
                    assert!(
                        missing.contains(field),
                        "missing must name '{field}': {missing}"
                    );
                }
                assert!(
                    !missing.contains("section"),
                    "present field not reported: {missing}"
                );
            }
        }
    }

    #[test]
    fn test_item_kinds_load_rejects_partial_declaration() {
        // The required-six rule fires through the real `JitConfig::load` path.
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("config.toml"),
            "[item_kinds.decision]\nsection = \"decisions\"\nid-pattern = \"D-\\\\d+\"\n",
        )
        .unwrap();
        let err = JitConfig::load(temp_dir.path()).expect_err("load must reject partial kind");
        let msg = format!("{err:#}");
        assert!(msg.contains("decision"), "error names the kind: {msg}");
        assert!(
            msg.contains("markers"),
            "error names a missing field: {msg}"
        );
    }

    #[test]
    fn test_item_kinds_source_of_truth_resolves_both_directions() {
        // REQ-01: both typed directions parse from their kebab-case tokens and are
        // DISTINCT from the `source` file path field. Each kind declares all six.
        let config_toml = r#"
[item_kinds.requirement]
section = "success_criteria"
id-pattern = "REQ-\\d+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"

[item_kinds.policy]
section = "policies"
id-pattern = "POL-\\d+"
markers = []
link-namespaces = ["upholds"]
scope = "project"
source = "policies.toml"
source-of-truth = "registry-first"
"#;
        // Uses a non-reserved name `policy` for the registry-first example: the
        // direction (`source-of-truth`) is independent of the `source` PATH field.
        // (The built-in `invariant` kind is itself reserved as project + registry-
        // first with NO source path; this test only exercises the typed parse.)
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        config
            .validate_item_kinds()
            .expect("complete kinds validate");
        let kinds = config.item_kinds.unwrap();

        let req = &kinds["requirement"];
        assert_eq!(req.source_of_truth, Some(SourceOfTruth::MarkdownFirst));
        assert_eq!(req.source_of_truth(), SourceOfTruth::MarkdownFirst);

        let pol = &kinds["policy"];
        assert_eq!(pol.source_of_truth, Some(SourceOfTruth::RegistryFirst));
        assert_eq!(pol.source_of_truth(), SourceOfTruth::RegistryFirst);
        // The direction is independent of the `source` PATH.
        assert_eq!(
            pol.source.as_ref().and_then(|s| s.path()),
            Some("policies.toml")
        );
    }

    #[test]
    fn test_item_kinds_source_of_truth_defaults_when_unset() {
        // `source_of_truth()` resolves to markdown-first when the field is unset
        // (used by the implicit default and direct struct construction).
        assert_eq!(
            ItemKindConfig::default().source_of_truth(),
            SourceOfTruth::MarkdownFirst
        );
    }

    #[test]
    fn test_item_kinds_source_of_truth_invalid_value_is_error() {
        // REQ-01: an unrecognized direction is a descriptive parse error, not a
        // silent default.
        let config_toml = r#"
[item_kinds.requirement]
source-of-truth = "both"
"#;
        let err = toml::from_str::<JitConfig>(config_toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("markdown-first"),
            "error mentions valid token: {msg}"
        );
        assert!(
            msg.contains("registry-first"),
            "error mentions valid token: {msg}"
        );
    }

    #[test]
    fn test_item_kinds_absent_is_none() {
        // A config with no `[item_kinds]` table leaves the registry None; the
        // domain layer supplies the built-in default kinds (`requirement`,
        // `decision`, ...) in that case.
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.item_kinds.is_none());
    }

    #[test]
    fn test_kind_scope_config_parses_both_tokens() {
        // The typed KindScopeConfig field deserializes both valid tokens.
        let issue: ItemKindConfig = toml::from_str("scope = \"issue\"").unwrap();
        assert_eq!(issue.scope, Some(KindScopeConfig::Issue));

        let project: ItemKindConfig = toml::from_str("scope = \"project\"").unwrap();
        assert_eq!(project.scope, Some(KindScopeConfig::Project));
    }

    #[test]
    fn test_kind_scope_config_invalid_token_is_parse_error() {
        // REQ: an unrecognised scope token is a TOML parse/deserialize error,
        // not a silent fallback to issue scope.
        let err =
            toml::from_str::<JitConfig>("[item_kinds.thing]\nscope = \"global\"\n").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("global"),
            "error must mention the bad token: {msg}"
        );
    }

    #[test]
    fn test_kind_scope_config_from_str_case_insensitive() {
        // FromStr normalises case so "ISSUE" and "PROJECT" are accepted.
        assert_eq!(
            "ISSUE".parse::<KindScopeConfig>().unwrap(),
            KindScopeConfig::Issue
        );
        assert_eq!(
            "Project".parse::<KindScopeConfig>().unwrap(),
            KindScopeConfig::Project
        );
        assert!("unknown".parse::<KindScopeConfig>().is_err());
    }

    #[test]
    fn test_worktree_mode_and_enforcement_mode_from_str_case_insensitive() {
        // Both WorktreeMode and EnforcementMode accept mixed-case input via
        // FromStr, so TOML and env-var paths share one case-handling rule.
        assert_eq!("AUTO".parse::<WorktreeMode>().unwrap(), WorktreeMode::Auto);
        assert_eq!("On".parse::<WorktreeMode>().unwrap(), WorktreeMode::On);
        assert_eq!("OFF".parse::<WorktreeMode>().unwrap(), WorktreeMode::Off);

        assert_eq!(
            "STRICT".parse::<EnforcementMode>().unwrap(),
            EnforcementMode::Strict
        );
        assert_eq!(
            "Warn".parse::<EnforcementMode>().unwrap(),
            EnforcementMode::Warn
        );
        assert_eq!(
            "OFF".parse::<EnforcementMode>().unwrap(),
            EnforcementMode::Off
        );
    }

    #[test]
    fn test_worktree_mode_case_sensitivity_divergence_is_gone() {
        // Previously the TOML path (WorktreeConfig::worktree_mode) only
        // accepted exact lowercase, while the env-var path normalised with
        // to_lowercase() first.  Now both go through the same FromStr, so a
        // mixed-case env-var produces the same value as the lowercase TOML form.
        let _guard = env_lock();

        // Uppercase env var parsed via EffectiveConfig (env-var path).
        std::env::set_var("JIT_WORKTREE_MODE", "AUTO");
        let via_env = ConfigLoader::new().build().worktree_mode().unwrap();
        std::env::remove_var("JIT_WORKTREE_MODE");

        // Lowercase TOML form parsed directly (TOML path).
        let toml_cfg: JitConfig = toml::from_str("[worktree]\nmode = \"auto\"\n").unwrap();
        let via_toml = toml_cfg.worktree.unwrap().worktree_mode();

        assert_eq!(
            via_env, via_toml,
            "TOML and env-var must resolve identically"
        );
        assert_eq!(via_env, WorktreeMode::Auto);
    }

    #[test]
    fn test_enforcement_mode_case_sensitivity_divergence_is_gone() {
        // Same divergence test for EnforcementMode / JIT_ENFORCE_LEASES.
        let _guard = env_lock();

        std::env::set_var("JIT_ENFORCE_LEASES", "WARN");
        let via_env = ConfigLoader::new().build().enforcement_mode().unwrap();
        std::env::remove_var("JIT_ENFORCE_LEASES");

        let toml_cfg: JitConfig =
            toml::from_str("[worktree]\nenforce_leases = \"warn\"\n").unwrap();
        let via_toml = toml_cfg.worktree.unwrap().enforcement_mode();

        assert_eq!(
            via_env, via_toml,
            "TOML and env-var must resolve identically"
        );
        assert_eq!(via_env, EnforcementMode::Warn);
    }

    #[test]
    fn test_parse_full_schema_v2_config() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
story = "story"

[validation]
default_type = "task"
strictness = "loose"

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // Version
        assert_eq!(config.version.unwrap().schema, 2);

        // Hierarchy
        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 4);
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );

        // Validation
        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.strictness, Some("loose".to_string()));

        // Namespaces
        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 2);
        assert!(namespaces["type"].unique);
        assert!(!namespaces["epic"].unique);
    }

    #[test]
    fn test_enforcement_mode_default_to_strict() {
        let config_toml = r#"
[worktree]
# No enforce_leases specified
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        // enforcement_mode() is now infallible — the field is typed.
        assert_eq!(worktree.enforcement_mode(), EnforcementMode::Strict);
    }

    #[test]
    fn test_enforcement_mode_explicit_strict() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode(), EnforcementMode::Strict);
    }

    #[test]
    fn test_enforcement_mode_warn() {
        let config_toml = r#"
[worktree]
enforce_leases = "warn"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode(), EnforcementMode::Warn);
    }

    #[test]
    fn test_enforcement_mode_off() {
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode(), EnforcementMode::Off);
    }

    #[test]
    fn test_enforcement_mode_invalid() {
        // Invalid tokens are now caught at TOML parse time, not at method call time.
        let config_toml = r#"
[worktree]
enforce_leases = "maybe"
"#;
        let err = toml::from_str::<JitConfig>(config_toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("maybe"),
            "error must mention the bad value: {msg}"
        );
    }

    #[test]
    fn test_config_without_worktree_section() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.worktree.is_none());
    }

    // ============================================================
    // Tests for new config sections (TDD - written before implementation)
    // ============================================================

    #[test]
    fn test_worktree_mode_auto() {
        let config_toml = r#"
[worktree]
mode = "auto"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        // worktree_mode() is now infallible — the field is typed.
        assert_eq!(worktree.worktree_mode(), WorktreeMode::Auto);
    }

    #[test]
    fn test_worktree_mode_on() {
        let config_toml = r#"
[worktree]
mode = "on"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode(), WorktreeMode::On);
    }

    #[test]
    fn test_worktree_mode_off() {
        let config_toml = r#"
[worktree]
mode = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode(), WorktreeMode::Off);
    }

    #[test]
    fn test_worktree_mode_default_to_auto() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode(), WorktreeMode::Auto);
    }

    #[test]
    fn test_worktree_mode_invalid() {
        // Invalid tokens are now caught at TOML parse time, not at method call time.
        let config_toml = r#"
[worktree]
mode = "maybe"
"#;
        let err = toml::from_str::<JitConfig>(config_toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("maybe"),
            "error must mention the bad value: {msg}"
        );
    }

    #[test]
    fn test_coordination_config_full() {
        let config_toml = r#"
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
lease_renewal_threshold_pct = 10
stale_threshold_secs = 3600
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
auto_renew_leases = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs, Some(600));
        assert_eq!(coord.heartbeat_interval_secs, Some(30));
        assert_eq!(coord.lease_renewal_threshold_pct, Some(10));
        assert_eq!(coord.stale_threshold_secs, Some(3600));
        assert_eq!(coord.max_indefinite_leases_per_agent, Some(2));
        assert_eq!(coord.max_indefinite_leases_per_repo, Some(10));
        assert_eq!(coord.auto_renew_leases, Some(false));
    }

    #[test]
    fn test_coordination_config_defaults() {
        let coord = CoordinationConfig::default();
        assert_eq!(coord.default_ttl_secs(), 600);
        assert_eq!(coord.heartbeat_interval_secs(), 30);
        assert_eq!(coord.lease_renewal_threshold_pct(), 10);
        assert_eq!(coord.stale_threshold_secs(), 3600);
        assert_eq!(coord.max_indefinite_leases_per_agent(), 2);
        assert_eq!(coord.max_indefinite_leases_per_repo(), 10);
        assert!(!coord.auto_renew_leases());
    }

    #[test]
    fn test_global_operations_config() {
        let config_toml = r#"
[global_operations]
require_main_history = true
allowed_branches = ["main", "develop"]
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let global_ops = config.global_operations.unwrap();
        assert_eq!(global_ops.require_main_history, Some(true));
        assert_eq!(
            global_ops.allowed_branches,
            Some(vec!["main".to_string(), "develop".to_string()])
        );
    }

    #[test]
    fn test_global_operations_defaults() {
        let global_ops = GlobalOperationsConfig::default();
        assert!(global_ops.require_main_history());
        assert_eq!(global_ops.allowed_branches(), vec!["main".to_string()]);
    }

    #[test]
    fn test_locks_config() {
        let config_toml = r#"
[locks]
max_age_secs = 7200
enable_metadata = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let locks = config.locks.unwrap();
        assert_eq!(locks.max_age_secs, Some(7200));
        assert_eq!(locks.enable_metadata, Some(false));
    }

    #[test]
    fn test_locks_config_defaults() {
        let locks = LocksConfig::default();
        assert_eq!(locks.max_age_secs(), 3600);
        assert!(locks.enable_metadata());
    }

    #[test]
    fn test_events_config() {
        let config_toml = r#"
[events]
enable_sequences = true
use_unified_envelope = true
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let events = config.events.unwrap();
        assert_eq!(events.enable_sequences, Some(true));
        assert_eq!(events.use_unified_envelope, Some(true));
    }

    #[test]
    fn test_events_config_defaults() {
        let events = EventsConfig::default();
        assert!(events.enable_sequences());
        assert!(events.use_unified_envelope());
    }

    #[test]
    fn test_full_parallel_work_config() {
        let config_toml = r#"
[worktree]
mode = "auto"
enforce_leases = "strict"

[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30

[global_operations]
require_main_history = true
allowed_branches = ["main", "develop"]

[locks]
max_age_secs = 3600
enable_metadata = true

[events]
enable_sequences = true
use_unified_envelope = true
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // All sections present
        assert!(config.worktree.is_some());
        assert!(config.coordination.is_some());
        assert!(config.global_operations.is_some());
        assert!(config.locks.is_some());
        assert!(config.events.is_some());

        // Verify worktree — both methods are infallible since fields are typed.
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode(), WorktreeMode::Auto);
        assert_eq!(worktree.enforcement_mode(), EnforcementMode::Strict);

        // Verify coordination
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs(), 600);
    }

    // ============================================================
    // Agent configuration tests (TDD - written before implementation)
    // ============================================================

    #[test]
    fn test_agent_config_full() {
        let config_toml = r#"
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"
description = "GitHub Copilot Workspace Session 1"
default_ttl_secs = 900

[behavior]
auto_heartbeat = false
heartbeat_interval = 30
"#;
        let config: AgentConfig = toml::from_str(config_toml).unwrap();

        assert_eq!(config.agent.id, "agent:copilot-1");
        assert_eq!(
            config.agent.created_at,
            Some("2026-01-03T12:00:00Z".to_string())
        );
        assert_eq!(
            config.agent.description,
            Some("GitHub Copilot Workspace Session 1".to_string())
        );
        assert_eq!(config.agent.default_ttl_secs, Some(900));

        let behavior = config.behavior.unwrap();
        assert_eq!(behavior.auto_heartbeat, Some(false));
        assert_eq!(behavior.heartbeat_interval, Some(30));
    }

    #[test]
    fn test_agent_config_minimal() {
        let config_toml = r#"
[agent]
id = "agent:worker-1"
"#;
        let config: AgentConfig = toml::from_str(config_toml).unwrap();

        assert_eq!(config.agent.id, "agent:worker-1");
        assert!(config.agent.created_at.is_none());
        assert!(config.agent.description.is_none());
        assert!(config.agent.default_ttl_secs.is_none());
        assert!(config.behavior.is_none());
    }

    #[test]
    fn test_agent_identity_defaults() {
        let identity = AgentIdentity {
            id: "agent:test".to_string(),
            created_at: None,
            description: None,
            default_ttl_secs: None,
        };
        assert_eq!(identity.default_ttl_secs(), 600); // Default from coordination
    }

    #[test]
    fn test_agent_behavior_defaults() {
        let behavior = AgentBehavior::default();
        assert!(!behavior.auto_heartbeat());
        assert_eq!(behavior.heartbeat_interval(), 30);
    }

    #[test]
    fn test_agent_config_load_missing() {
        let temp_dir = TempDir::new().unwrap();
        let result = AgentConfig::load(temp_dir.path());
        // Should return None when file doesn't exist
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_agent_config_load_existing() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[agent]
id = "agent:test-agent"
description = "Test agent"
"#;
        std::fs::write(temp_dir.path().join("agent.toml"), config_toml).unwrap();

        let config = AgentConfig::load(temp_dir.path()).unwrap().unwrap();
        assert_eq!(config.agent.id, "agent:test-agent");
        assert_eq!(config.agent.description, Some("Test agent".to_string()));
    }

    // ============================================================
    // Config loading with priority and merging tests (TDD)
    // ============================================================

    #[test]
    fn test_config_loader_defaults_only() {
        let loader = ConfigLoader::new();
        let config = loader.build();

        // Should have all defaults
        assert_eq!(config.coordination().default_ttl_secs(), 600);
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
        assert!(config.global_operations().require_main_history());
        assert_eq!(config.locks().max_age_secs(), 3600);
        assert!(config.events().enable_sequences());
    }

    #[test]
    fn test_config_loader_repo_overrides_defaults() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let loader = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo value overrides default
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // Other defaults preserved
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
    }

    #[test]
    fn test_config_loader_repo_overrides_user() {
        let user_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();

        // User config sets TTL to 900
        let user_config = r#"
[coordination]
default_ttl_secs = 900
heartbeat_interval_secs = 60
"#;
        std::fs::write(user_dir.path().join("config.toml"), user_config).unwrap();

        // Repo config sets TTL to 1200 (overrides user)
        let repo_config = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(repo_dir.path().join("config.toml"), repo_config).unwrap();

        let loader = ConfigLoader::new()
            .with_user_config(user_dir.path())
            .unwrap()
            .with_repo_config(repo_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo overrides user for TTL
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // User value used for heartbeat (not in repo config)
        assert_eq!(config.coordination().heartbeat_interval_secs(), 60);
    }

    #[test]
    fn test_config_loader_full_priority_chain() {
        let system_dir = TempDir::new().unwrap();
        let user_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();

        // System config (lowest priority after defaults)
        let system_config = r#"
[coordination]
default_ttl_secs = 300
heartbeat_interval_secs = 15
stale_threshold_secs = 1800
"#;
        std::fs::write(system_dir.path().join("config.toml"), system_config).unwrap();

        // User config overrides system
        let user_config = r#"
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
"#;
        std::fs::write(user_dir.path().join("config.toml"), user_config).unwrap();

        // Repo config overrides user
        let repo_config = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(repo_dir.path().join("config.toml"), repo_config).unwrap();

        let loader = ConfigLoader::new()
            .with_system_config(system_dir.path())
            .unwrap()
            .with_user_config(user_dir.path())
            .unwrap()
            .with_repo_config(repo_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo wins for TTL
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // User wins for heartbeat (not in repo)
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
        // System wins for stale_threshold (not in user or repo)
        assert_eq!(config.coordination().stale_threshold_secs(), 1800);
    }

    #[test]
    fn test_config_loader_missing_files_ok() {
        let temp_dir = TempDir::new().unwrap();

        // Loading from non-existent paths should succeed (use defaults)
        let loader = ConfigLoader::new()
            .with_system_config(temp_dir.path())
            .unwrap()
            .with_user_config(temp_dir.path())
            .unwrap()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        // All defaults
        assert_eq!(config.coordination().default_ttl_secs(), 600);
    }

    #[test]
    fn test_effective_config_worktree_mode() {
        let _guard = env_lock();
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[worktree]
mode = "on"
enforce_leases = "warn"
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let loader = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::On);
        assert_eq!(config.enforcement_mode().unwrap(), EnforcementMode::Warn);
    }

    // ============================================================
    // Environment variable override tests (TDD)
    // ============================================================

    #[test]
    fn test_env_override_worktree_mode() {
        let _guard = env_lock();
        std::env::set_var("JIT_WORKTREE_MODE", "off");
        let config = ConfigLoader::new().build();
        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::Off);
        std::env::remove_var("JIT_WORKTREE_MODE");
    }

    #[test]
    fn test_env_override_enforce_leases() {
        let _guard = env_lock();
        std::env::set_var("JIT_ENFORCE_LEASES", "warn");
        let config = ConfigLoader::new().build();
        assert_eq!(config.enforcement_mode().unwrap(), EnforcementMode::Warn);
        std::env::remove_var("JIT_ENFORCE_LEASES");
    }

    #[test]
    fn test_env_override_agent_id() {
        let _guard = env_lock();
        std::env::set_var("JIT_AGENT_ID", "agent:env-test");
        let config = ConfigLoader::new().build();
        assert_eq!(config.agent_id(), Some("agent:env-test".to_string()));
        std::env::remove_var("JIT_AGENT_ID");
    }

    #[test]
    fn test_env_overrides_config_file() {
        let _guard = env_lock();
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[worktree]
mode = "on"
enforce_leases = "strict"
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        // Env var should override config file
        std::env::set_var("JIT_WORKTREE_MODE", "off");
        let config = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap()
            .build();
        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::Off);
        std::env::remove_var("JIT_WORKTREE_MODE");
    }

    #[test]
    fn test_env_invalid_value_returns_error() {
        let _guard = env_lock();
        std::env::set_var("JIT_WORKTREE_MODE", "invalid");
        let config = ConfigLoader::new().build();
        assert!(config.worktree_mode().is_err());
        std::env::remove_var("JIT_WORKTREE_MODE");
    }
}
