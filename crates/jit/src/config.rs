//! Configuration file loading and parsing.
//!
//! JIT supports repository-level configuration through `.jit/config.toml`.
//! If no config file exists, the system falls back to sensible defaults.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure loaded from `.jit/config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct JitConfig {
    /// Schema version for migrations (optional).
    pub version: Option<VersionConfig>,
    /// Project identity configuration (optional `[project]` table).
    ///
    /// Declares the project's canonical, human-editable name: the `@<project>`
    /// scope token used by the multi-jit addressing scheme. `jit init` seeds
    /// this with a slug of the repository directory's basename when
    /// `.jit/config.toml` does not already exist, and never touches an
    /// existing `[project]` table on a later `init`. See [`ProjectConfig`].
    pub project: Option<ProjectConfig>,
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
    /// Generic documentation-projection registry (optional).
    ///
    /// Each `[projection.<name>]` table declares one projection of an addressable
    /// item kind (or kinds) into a documentation target: its `kind`, `mode`
    /// (`separate-file`|`region`), `target` (repo-relative), render `style`
    /// (`id-anchor`|`full`), and optional region delimiters. Keyed by the
    /// projection name, which also derives the default region markers
    /// (`<!-- jit:<name>:begin -->` / `<!-- jit:<name>:end -->`). The engine drives
    /// every declared projection from the single `jit project render` command; the
    /// target path and delimiters live ONLY here in config. See
    /// [`ProjectionConfig`].
    pub projection: Option<std::collections::BTreeMap<String, ProjectionConfig>>,
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
    /// [`InvariantRegistry`](crate::declarations::invariants::InvariantRegistry).
    /// Kept here so later indexing can project each entry as a project-scoped
    /// (`@`) addressable item.
    #[serde(skip)]
    pub invariants: crate::declarations::invariants::InvariantRegistry,
}

/// Schema version configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionConfig {
    /// Schema version number (default: 1).
    pub schema: u32,
}

/// Project identity configuration from the `[project]` TOML table.
///
/// The sole home for the project's canonical, human-editable name — not
/// `index.json`, which carries only machine metadata
/// (`schema_version`/`all_ids`/`deleted_ids`) and is not meant to be hand-edited.
/// Resolve a present value with
/// [`ConfigManager::get_project_name`](crate::config_manager::ConfigManager::get_project_name).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectConfig {
    /// The project's canonical name: the `@<project>` scope token in the
    /// multi-jit addressing scheme. `None` when the `[project]` table is
    /// present but the `name` key is absent.
    pub name: Option<ProjectName>,
}

/// Parse error for [`ProjectName`].
///
/// Returned by [`ProjectName::from_str`] and (via serde) by TOML
/// deserialization when the value does not match `^[a-z][a-z0-9-]*$`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjectNameError {
    /// The value does not match `^[a-z][a-z0-9-]*$`.
    #[error("invalid project name '{value}'; expected to match ^[a-z][a-z0-9-]*$")]
    Invalid {
        /// The unrecognised value.
        value: String,
    },
}

/// The project's canonical, human-editable identity: the `@<project>` scope
/// token used by the multi-jit addressing scheme's project-qualified address
/// form.
///
/// Parsed from a string matching `^[a-z][a-z0-9-]*$` (starts with a lowercase
/// letter; digits and hyphens allowed thereafter) by both
/// [`ProjectName::from_str`] and TOML deserialization, mirroring the
/// [`WorktreeMode`] / [`EnforcementMode`] pattern: an invalid value is a
/// descriptive parse error, not a silent fallback, so a misconfigured
/// `[project] name` is rejected at [`JitConfig::load`] time and surfaces
/// through `jit config validate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectName(String);

impl ProjectName {
    /// The validated project name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for ProjectName {
    type Err = ProjectNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if first_ok && rest_ok {
            Ok(ProjectName(s.to_string()))
        } else {
            Err(ProjectNameError::Invalid {
                value: s.to_string(),
            })
        }
    }
}

impl<'de> serde::Deserialize<'de> for ProjectName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<ProjectName>().map_err(serde::de::Error::custom)
    }
}

/// Slugify an arbitrary string into a valid [`ProjectName`] token.
///
/// Lowercases the input, collapses every run of characters outside `[a-z0-9]`
/// into a single `-`, and trims leading/trailing `-`. Used by `jit init` to
/// seed `[project] name` from the repository directory's basename — distinct
/// from the underscore-style
/// [`slugify_heading`](crate::document::parser::slugify_heading), which serves
/// document section slugs, not the `ProjectName` pattern. Falls back to the
/// fixed literal `"project"` when the result is empty or does not start with a
/// lowercase letter (e.g. a basename starting with a digit), so the seeded
/// value always satisfies [`ProjectName`]'s `^[a-z][a-z0-9-]*$` pattern.
pub fn slugify_project_name(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    let starts_with_lowercase_letter = slug.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if starts_with_lowercase_letter {
        slug
    } else {
        "project".to_string()
    }
}

/// Type hierarchy configuration from TOML.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IconConfigToml {
    /// Custom type name to icon mapping (optional, partial overrides allowed).
    pub custom: Option<HashMap<String, String>>,
}

/// Deserialize and eagerly validate `[validation].strictness`.
///
/// Accepts an absent key (yielding `None`) or one of the three levels
/// (`strict`/`loose`/`permissive`, case-insensitively); any other value is
/// rejected at deserialize time via [`Strictness`](crate::validation::Strictness)
/// so a `config.toml` carrying an unrecognized strictness fails to load instead
/// of persisting a silently-inert value. The validated raw string is preserved
/// so `jit config get`/`show` round-trips the author's spelling.
fn deserialize_strictness<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(raw) = value.as_deref() {
        raw.parse::<crate::validation::Strictness>()
            .map_err(serde::de::Error::custom)?;
    }
    Ok(value)
}

/// Validation behavior configuration.
///
/// Per-rule enforcement lives in `.jit/rules.toml` (DR §8.2/§8.4), whose built-in
/// default rules derive from this repo's registry at load; serde ignores any
/// stale enforcement keys still present in an old
/// `config.toml` (no `deny_unknown_fields`), so such a file still parses. The
/// operative keys here are `strictness` (the repo-wide enforcement modulator),
/// `default_type`, and `content_format`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ValidationConfig {
    /// Repo-wide enforcement strictness: `"strict"`, `"loose"`, or
    /// `"permissive"`. It modulates which rule violations block a write or
    /// transition, layered on top of each rule's `enforce`/severity, without
    /// changing either. Absent means `"loose"` (only an enforced error blocks —
    /// the pre-strictness behavior). Resolved via
    /// [`ValidationConfig::strictness`].
    ///
    /// Validated at deserialize time (like [`ProjectName`]): loading a
    /// `config.toml` whose `strictness` is not one of the three levels fails
    /// eagerly rather than deferring the error to the next validation call, so an
    /// unrecognized value can never persist and sit silently.
    #[serde(default, deserialize_with = "deserialize_strictness")]
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

    /// Resolve the repo-wide validation
    /// [`Strictness`](crate::validation::Strictness) from `strictness`, defaulting
    /// to [`Strictness::Loose`](crate::validation::Strictness::Loose) when unset.
    ///
    /// The value is already validated at deserialize time (see
    /// [`deserialize_strictness`]), so a loaded config resolves infallibly; the
    /// `Result` guards a `ValidationConfig` constructed by hand with an invalid
    /// string, which is still surfaced as an error rather than silently
    /// defaulting.
    pub fn strictness(&self) -> Result<crate::validation::Strictness> {
        crate::validation::Strictness::from_config_value(self.strictness.as_deref())
    }
}

/// Documentation lifecycle management configuration.
///
/// Every field is optional, and an absent one declares nothing rather than
/// resolving to a classification the engine supplies, so the default value is
/// the wholly unauthored table: what a repository whose configuration omits
/// `[documentation]` declares. A repository obtains a development-area
/// classification by declaring one, directly or by applying a profile package
/// that contributes it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DocumentationConfig {
    /// Root directory for development documentation. Absent declares no root,
    /// so no boundary excludes an area from classification.
    ///
    /// Archival planning classifies by this root as well: a linked artifact it
    /// does not contain is retained where it is, so the plan schedules no
    /// destination for it and artifact discovery stops following links at it
    /// (`@/issue/8e071e18/decision/D-14`).
    pub development_root: Option<String>,
    /// Paths inside the development root whose artifacts are subject to
    /// archival. Absent declares no managed area.
    pub managed_paths: Option<Vec<String>>,
    /// Where archived docs are stored. Absent declares no archive root.
    pub archive_root: Option<String>,
    /// Paths inside the development root whose artifacts archive by copy while
    /// the source is retained. Absent declares no permanent area.
    pub permanent_paths: Option<Vec<String>>,
    /// Areas that organize their artifacts one directory per issue. Absent
    /// declares no such area, exactly as an authored empty list does.
    pub issue_scoped_areas: Option<Vec<String>>,
    /// Roots an in-content citation scan reads. Absent falls back to the
    /// development root together with the configured permanent paths.
    pub citation_scan_roots: Option<Vec<String>>,
}

impl DocumentationConfig {
    /// The declared development root, empty when unauthored.
    pub fn development_root(&self) -> String {
        self.development_root.clone().unwrap_or_default()
    }

    /// The declared managed paths, empty when unauthored.
    pub fn managed_paths(&self) -> Vec<String> {
        self.managed_paths.clone().unwrap_or_default()
    }

    /// The declared archive root, empty when unauthored.
    pub fn archive_root(&self) -> String {
        self.archive_root.clone().unwrap_or_default()
    }

    /// The declared permanent paths, empty when unauthored.
    pub fn permanent_paths(&self) -> Vec<String> {
        self.permanent_paths.clone().unwrap_or_default()
    }

    /// The areas that organize their artifacts one directory per issue, empty
    /// when unauthored.
    ///
    /// An authored list is the whole registry, so an empty list and an absent
    /// key alike opt every area out of the convention.
    pub fn issue_scoped_areas(&self) -> Vec<String> {
        self.issue_scoped_areas.clone().unwrap_or_default()
    }

    /// The repository-relative roots an in-content citation scan reads, falling
    /// back to the development root together with the configured permanent
    /// paths when unauthored (`@/issue/8e071e18/decision/D-16`).
    ///
    /// The fallback is derived from this table's own values, so reclassifying an
    /// area carries the scanned set with it. Entries are matched with
    /// [`contains_path`](crate::domain::artifact_classifier::contains_path), the
    /// matcher the classification lists use: a directory entry reaches
    /// everything beneath it and a file entry reaches exactly one file. An
    /// authored list is the whole universe — it replaces the fallback rather
    /// than extending it, and its entries need not lie under the development
    /// root, since the citations a move can break live wherever the adopter
    /// writes them.
    pub fn citation_scan_roots(&self) -> Vec<String> {
        self.citation_scan_roots.clone().unwrap_or_else(|| {
            std::iter::once(self.development_root())
                .chain(self.permanent_paths())
                .collect()
        })
    }

    /// Whether `area` is one of the declared issue-scoped areas.
    ///
    /// Membership is exact-area matching under lexical path normalization, so
    /// `dev/x`, `./dev/x` and `dev/x/` all name the same area while a path
    /// *inside* a declared area does not. This is deliberately narrower than
    /// the prefix containment the archival classifier applies to its own path
    /// lists ([`contains_path`](crate::domain::artifact_classifier::contains_path)):
    /// a caller names one area and receives the issue-scoped directory inside
    /// it, so accepting a path beneath a declared area would accept an
    /// already-resolved directory as an area and defeat the rejection of
    /// undeclared ones.
    pub fn is_issue_scoped_area(&self, area: &str) -> bool {
        let area = crate::domain::artifact_plan::normalize_artifact_path(area);
        !area.is_empty()
            && self.issue_scoped_areas().iter().any(|declared| {
                crate::domain::artifact_plan::normalize_artifact_path(declared) == area
            })
    }
}

/// Label namespace configuration from TOML.
/// Replaces the namespace definitions in labels.json.
///
/// The per-namespace constraint fields (`values`, `pattern`, `required`) were
/// removed when `.jit/rules.toml` became the operative validation ruleset (DR
/// §8.4): a repo that wants those constraints authors the corresponding rules in
/// `rules.toml`. The registry keeps only TAXONOMY (`description`/`unique`/
/// `examples`); serde ignores any stale constraint keys in an old `config.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
/// survive only so partial-declaration validation can report which fields are
/// missing and so direct struct construction in tests can exercise malformed
/// declarations — NOT for any implicit kinds (the engine bakes in none; with no
/// `[item_kinds]` table the kind set is empty). The `source` PATH (project-scope
/// source file) is NOT one of the six and stays optional.
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
/// single issue (qualified id `@/<kind>/<self-id>`). A *markdown-first* project kind sets
/// `scope = "project"` and a `source` file (a repository-local path, relative to
/// the repo root) whose markdown is scanned the SAME way an issue description is. It
/// still declares all six required fields (the optional `source` PATH is in
/// addition). The example uses `glossary`; the `invariant` kind is an
/// ordinary registry-first kind whose `source` is a toml descriptor pointing at
/// `.jit/invariants.toml` (a markdown `source` path applies only to markdown-first
/// kinds):
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
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
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
    /// descriptions (qualified id `@/issue/<short-id>/<kind>/<self-id>`), or
    /// [`KindScopeConfig::Project`] for items projected from a repository-local
    /// `source` file (qualified id `@/<kind>/<self-id>`). Required in an explicit
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
    /// Shorthand names this kind may ALSO be addressed by, beyond its registry
    /// key. An alias is accepted anywhere a kind name is (the kind segment of a
    /// project- or issue-scope address, and `--kind` filters); canonical output
    /// still uses the registry name, so aliases are input sugar only. Aliases
    /// share the kind-name namespace: an alias duplicating a kind name or another
    /// kind's alias is rejected by [`JitConfig::validate_item_kinds`]. NOT one of
    /// the six required fields — it stays optional, and an absent field means the
    /// kind has no aliases.
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
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
    ///     aliases: None,
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
    pub fn path(&self) -> Option<&str> {
        match self {
            ItemKindSource::Path(p) => Some(p),
            ItemKindSource::Toml(_) => None,
        }
    }

    /// The structured TOML descriptor, or `None` when this is a bare path.
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

impl Serialize for ItemKindSource {
    /// Mirrors the [`Deserialize`] impl above: a [`ItemKindSource::Path`]
    /// serializes as its bare path string, a [`ItemKindSource::Toml`] as its
    /// descriptor table — so a value round-trips through TOML or JSON
    /// unchanged, and `jit config get` (which serializes the loaded
    /// [`JitConfig`] to walk it generically) renders the same shape a hand
    /// authored `config.toml` declares.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ItemKindSource::Path(path) => serializer.serialize_str(path),
            ItemKindSource::Toml(descriptor) => descriptor.serialize(serializer),
        }
    }
}

/// The field mapping that projects a `.toml` registry table into addressable
/// items for a registry-first project kind.
///
/// Each entry of the named `table` (an array-of-tables) becomes one addressable
/// item: `id-field` supplies its self-id (so its qualified id is `@/<kind>/<self-id>`),
/// `text-field` supplies its display text, and each `link-fields` entry maps a
/// toml field holding link targets to the link NAMESPACE those targets are
/// labelled under. The `text-field` is the only OPTIONAL addressing field: an
/// entry lacking it falls back to its `id-field` value as display text (so a
/// description-less rule projects its `name`). A `link-fields` value may be a
/// single string or an array of strings; an absent link field on an entry
/// contributes no labels (graceful).
///
/// This is the generic analogue of the typed invariant registry: it carries only
/// the addressing mapping (id/text/links), leaving any kind-specific TYPED
/// validation (e.g. an invariant's `enforced`/`advisory` discriminant) to a
/// dedicated loader.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TomlSourceDescriptor {
    /// Repository-local path to the `.toml` registry file (read through storage).
    pub toml: String,
    /// The array-of-tables key whose entries are projected (e.g. `"policies"`).
    pub table: String,
    /// The entry field supplying each item's self-id (e.g. `"id"`).
    #[serde(rename = "id-field")]
    pub id_field: String,
    /// The entry field supplying each item's display text (e.g. `"statement"`).
    /// OPTIONAL at projection time: an entry that omits this field falls back to
    /// its [`id_field`](Self::id_field) value as display text (a present-but-
    /// non-string value is still a typed error).
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScopeConfig {
    /// Items are projected from issue descriptions
    /// (`@/issue/<short-id>/<kind>/<self-id>`).
    Issue,
    /// Items are projected from a config-declared source file
    /// (`@/<kind>/<self-id>`).
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

impl Serialize for KindScopeConfig {
    /// Serializes to the same `"issue"` / `"project"` token [`FromStr`](std::str::FromStr)
    /// and [`Deserialize`] accept, so `jit config get` renders the value a
    /// hand-authored `config.toml` would use.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let token = match self {
            KindScopeConfig::Issue => "issue",
            KindScopeConfig::Project => "project",
        };
        serializer.serialize_str(token)
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
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
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

/// The default begin marker delimiting projection `<name>`'s region in `region`
/// mode: `<!-- jit:<name>:begin -->`.
///
/// Derived from the projection name (the `[projection.<name>]` table key), so a
/// projection needs no explicit `region-begin` unless it wants a custom marker.
/// Used by [`ProjectionConfig::region_begin`].
pub fn default_region_begin(name: &str) -> String {
    format!("<!-- jit:{name}:begin -->")
}

/// The default end marker delimiting projection `<name>`'s region in `region`
/// mode: `<!-- jit:<name>:end -->`. The name-derived counterpart of
/// [`default_region_begin`], used by [`ProjectionConfig::region_end`].
pub fn default_region_end(name: &str) -> String {
    format!("<!-- jit:{name}:end -->")
}

/// The item kind(s) a projection renders: one kind name, or several.
///
/// Deserializes from either a bare string (`kind = "invariant"`) or an array
/// (`kind = ["rule", "gate"]`); serializes back to the same shape (a single-element
/// list round-trips to the bare string), so `jit config get` renders what a
/// hand-authored `config.toml` would write. The names are resolved to configured
/// [`ItemKind`](crate::domain::item::ItemKind)s at render time — an unknown name is
/// a typed error there, not here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionKinds(Vec<String>);

impl ProjectionKinds {
    /// The declared kind names, in authored order.
    pub fn names(&self) -> &[String] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProjectionKinds {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A bare string is one kind; a sequence is several. A Visitor keeps this
        // robust under the TOML deserializer (mirrors `ItemKindSource`).
        struct KindsVisitor;
        impl<'de> serde::de::Visitor<'de> for KindsVisitor {
            type Value = ProjectionKinds;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a kind name string, or an array of kind name strings")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ProjectionKinds(vec![v.to_string()]))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ProjectionKinds(vec![v]))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut names = Vec::new();
                while let Some(name) = seq.next_element::<String>()? {
                    names.push(name);
                }
                if names.is_empty() {
                    return Err(serde::de::Error::custom(
                        "a projection `kind` list must name at least one addressable kind",
                    ));
                }
                Ok(ProjectionKinds(names))
            }
        }
        deserializer.deserialize_any(KindsVisitor)
    }
}

impl Serialize for ProjectionKinds {
    /// Mirrors [`Deserialize`]: a single kind serializes as a bare string, several
    /// as an array, so a value round-trips through TOML or JSON unchanged.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_slice() {
            [one] => serializer.serialize_str(one),
            many => many.serialize(serializer),
        }
    }
}

impl schemars::JsonSchema for ProjectionKinds {
    fn schema_name() -> String {
        "ProjectionKinds".to_string()
    }

    /// Mirrors the string-or-array serialization: the schema accepts either a
    /// bare kind-name string or an array of kind-name strings, so a profile
    /// manifest carrying `kind = "invariant"` or `kind = ["rule", "gate"]`
    /// validates against the generated manifest schema.
    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                any_of: Some(vec![
                    String::json_schema(generator),
                    Vec::<String>::json_schema(generator),
                ]),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// One generic documentation projection declared as `[projection.<name>]`.
///
/// Replaces the bespoke per-projection tables: any addressable item kind renders
/// into a documentation target through this one shape, driven by the single
/// `jit project render` command. Fields:
///
/// - `kind` — the [`ProjectionKinds`] to render (one name, or an array).
/// - `mode` — [`ProjectionMode`] (`separate-file` writes a whole file; `region`
///   rewrites only a delimited block, byte-preserving everything outside).
/// - `target` — REQUIRED repo-relative documentation path (config-driven; the
///   engine hardcodes no filename and applies no default). A projection that
///   declares no target is a typed error at render and validation, never a
///   silent default.
/// - `style` — [`ProjectionStyle`] (`id-anchor` renders generic `- **{id}** —
///   {text}` rows; `full` renders the built-in rich registry views).
/// - `region-begin` / `region-end` — optional region delimiters; when unset they
///   default to `<!-- jit:<name>:begin -->` / `<!-- jit:<name>:end -->`, derived
///   from the projection name (so the accessors take that name).
///
/// # Examples
///
/// ```
/// use jit::config::{ProjectionConfig, ProjectionMode, ProjectionStyle};
///
/// let cfg: ProjectionConfig = toml::from_str(
///     r#"
/// kind = "charter"
/// mode = "region"
/// target = "AGENTS.md"
/// style = "id-anchor"
/// "#,
/// )
/// .unwrap();
/// assert_eq!(cfg.kind.names(), ["charter"]);
/// assert_eq!(cfg.mode(), ProjectionMode::Region);
/// assert_eq!(cfg.style(), ProjectionStyle::IdAnchor);
/// // Region markers default from the projection name.
/// assert_eq!(cfg.region_begin("charter"), "<!-- jit:charter:begin -->");
///
/// // `kind` also accepts an array (e.g. the rules-and-gates projection).
/// let multi: ProjectionConfig =
///     toml::from_str("kind = [\"rule\", \"gate\"]\ntarget = \"ref.md\"\n").unwrap();
/// assert_eq!(multi.kind.names(), ["rule", "gate"]);
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct ProjectionConfig {
    /// The item kind(s) whose addressable rows this projection renders.
    pub kind: ProjectionKinds,
    /// Projection mode: a whole separate file, or a delimited region within an
    /// existing file. Defaults to [`ProjectionMode::SeparateFile`] when unset.
    #[serde(default)]
    pub mode: Option<ProjectionMode>,
    /// Repo-relative path of the documentation target. REQUIRED: a projection
    /// that declares no `target` is a typed error at render and validation
    /// (`ProjectionError::MissingTarget`), never a silent default. The field is
    /// deserialized as optional so a target-less table parses and then fails with
    /// a projection-named error rather than an opaque serde message.
    #[serde(default)]
    pub target: Option<String>,
    /// Begin marker delimiting the rewritten region in `region` mode. Defaults to
    /// `<!-- jit:<name>:begin -->` when unset.
    #[serde(default, rename = "region-begin")]
    pub region_begin: Option<String>,
    /// End marker delimiting the rewritten region in `region` mode. Defaults to
    /// `<!-- jit:<name>:end -->` when unset.
    #[serde(default, rename = "region-end")]
    pub region_end: Option<String>,
    /// Render style for the projected markdown block. Defaults to
    /// [`ProjectionStyle::Full`] when unset.
    #[serde(default)]
    pub style: Option<ProjectionStyle>,
}

impl ProjectionConfig {
    /// The declared kind names, in authored order.
    pub fn kinds(&self) -> &[String] {
        self.kind.names()
    }

    /// The resolved projection mode (defaulting to [`ProjectionMode::SeparateFile`]).
    pub fn mode(&self) -> ProjectionMode {
        self.mode.unwrap_or_default()
    }

    /// The resolved begin marker for `region` mode, defaulting to the name-derived
    /// [`default_region_begin`].
    pub fn region_begin(&self, name: &str) -> String {
        self.region_begin
            .clone()
            .unwrap_or_else(|| default_region_begin(name))
    }

    /// The resolved end marker for `region` mode, defaulting to the name-derived
    /// [`default_region_end`].
    pub fn region_end(&self, name: &str) -> String {
        self.region_end
            .clone()
            .unwrap_or_else(|| default_region_end(name))
    }

    /// The resolved render style (defaulting to [`ProjectionStyle::Full`]).
    pub fn style(&self) -> ProjectionStyle {
        self.style.unwrap_or_default()
    }
}

/// How a registry is projected into its documentation target.
///
/// Deserialized from the kebab-case tokens `"separate-file"` / `"region"` via
/// serde rename; an unrecognized value is a descriptive parse error rather than a
/// silent default. The default (no `mode` field) is [`ProjectionMode::SeparateFile`].
#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq, schemars::JsonSchema,
)]
pub enum ProjectionMode {
    /// Write the rendered block to a separate whole file (the default).
    #[default]
    #[serde(rename = "separate-file")]
    SeparateFile,
    /// Replace only a delimited region within an existing file, byte-preserving
    /// everything outside the delimiters.
    #[serde(rename = "region")]
    Region,
}

/// How a projected registry is rendered into the markdown block.
///
/// Deserialized from the kebab-case tokens `"full"` / `"id-anchor"` via serde
/// rename; an unrecognized value is a descriptive parse error rather than a
/// silent default. The default (no `style` field) is [`ProjectionStyle::Full`].
///
/// - [`ProjectionStyle::IdAnchor`] renders a heading-less generic bullet list of
///   `- **{self-id}** — {text}` rows from a kind's addressable items — the
///   kind-agnostic style any item kind can use with no dedicated code.
/// - [`ProjectionStyle::Full`] renders the built-in rich registry views (the
///   invariant registry's `[kind]`/enforced-by bullets, or the rule + gate
///   registries' `## Rules` / `## Gates` sections with severity/enforce metadata),
///   which carry typed fields absent from a generic addressable row.
#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq, schemars::JsonSchema,
)]
pub enum ProjectionStyle {
    /// Render the built-in rich registry view for the projection's kind(s) (the
    /// default): the invariant registry's `[kind]`/enforced-by bullets, or the
    /// rule + gate registries' `## Rules` / `## Gates` sections with metadata.
    #[default]
    #[serde(rename = "full")]
    Full,
    /// Render a heading-less generic bullet list of `- **{self-id}** — {text}`
    /// rows from the kind's addressable items, for embedding beneath a
    /// hand-authored heading. Kind-agnostic: any item kind renders this way.
    #[serde(rename = "id-anchor")]
    IdAnchor,
}

/// An error validating an explicitly-declared `[item_kinds.X]` table.
///
/// Raised by [`JitConfig::validate_item_kinds`] (and thus [`JitConfig::load`])
/// when a declared kind omits one or more of its six required fields. A repo with
/// no `[item_kinds]` table declares no kinds (the engine bakes in none), so there
/// is nothing to validate in that case.
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
    /// A kind's declared `alias` duplicates a kind name or another kind's alias.
    /// Aliases share the kind-name namespace, so a shorthand that is already a
    /// registry key or another declared alias is ambiguous and rejected.
    #[error(
        "[item_kinds.{kind}] alias '{alias}' collides with {conflict}; \
         aliases share the kind-name namespace and must each be unique"
    )]
    AliasCollision {
        /// The kind declaring the offending alias.
        kind: String,
        /// The duplicated alias token.
        alias: String,
        /// A short description of what the alias collides with (a kind name or
        /// another declared alias).
        conflict: String,
    },
}

/// Parse error for [`WorktreeMode`].
///
/// Returned by [`WorktreeMode::from_str`] and (via serde) by TOML /
/// environment-variable deserialization when the value is not one of the
/// recognised tokens.
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
    pub fn worktree_mode(&self) -> WorktreeMode {
        self.mode.unwrap_or(WorktreeMode::Auto)
    }

    /// Get the enforcement mode, defaulting to [`EnforcementMode::Strict`] when unset.
    ///
    /// The field is typed — invalid tokens are rejected at TOML parse time, so
    /// this method is infallible.
    pub fn enforcement_mode(&self) -> EnforcementMode {
        self.enforce_leases.unwrap_or(EnforcementMode::Strict)
    }
}

/// Coordination settings for leases and multi-agent work.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CoordinationConfig {
    /// Default TTL for new leases in seconds (default:
    /// [`crate::runtime_defaults::CLAIM_TTL_SECS`]).
    pub default_ttl_secs: Option<u64>,
    /// Warn when lease has less than this percentage of TTL remaining (default: 10).
    pub lease_renewal_threshold_pct: Option<u8>,
    /// Staleness threshold for TTL=0 leases in seconds (default: 3600).
    pub stale_threshold_secs: Option<u64>,
    /// Maximum concurrent TTL=0 leases per agent (default: 2).
    pub max_indefinite_leases_per_agent: Option<u32>,
    /// Maximum concurrent TTL=0 leases per repository (default: 10).
    pub max_indefinite_leases_per_repo: Option<u32>,
}

impl CoordinationConfig {
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs
            .unwrap_or(crate::runtime_defaults::CLAIM_TTL_SECS)
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
    /// Get the default TTL, falling back to the coordination default
    /// ([`crate::runtime_defaults::CLAIM_TTL_SECS`]).
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs
            .unwrap_or(crate::runtime_defaults::CLAIM_TTL_SECS)
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
                project: None,
                type_hierarchy: None,
                validation: None,
                documentation: None,
                namespaces: None,
                item_kinds: None,
                projection: None,
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
                invariants: crate::declarations::invariants::InvariantRegistry::load(jit_root)
                    .context("invalid .jit/invariants.toml")?,
            });
        }

        let content =
            std::fs::read_to_string(&config_path).context("Failed to read config.toml")?;

        // An old `config.toml` may still carry removed enforcement keys
        // (`require_type_label`, namespace `values`/`pattern`/`required`, etc.).
        // serde ignores them (no `deny_unknown_fields`), so the file still parses;
        // the keys simply have no effect — validation is rule-driven.
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
        // rather than silently filling defaults. A repo with no `[item_kinds]`
        // table declares no kinds, so there is nothing to validate in that case.
        config
            .validate_item_kinds()
            .context("invalid [item_kinds] in .jit/config.toml")?;

        // Chain-load `.jit/invariants.toml` on the config-present path too, with
        // the same graceful-absent / typed-error contract as the early return
        // above, so both load paths populate the registry identically.
        config.invariants = crate::declarations::invariants::InvariantRegistry::load(jit_root)
            .context("invalid .jit/invariants.toml")?;

        Ok(config)
    }

    /// Validate every explicitly-declared `[item_kinds.X]` table, requiring all
    /// six fields (`section`, `id-pattern`, `markers`, `link-namespaces`, `scope`,
    /// `source-of-truth`) on each, and rejecting any `aliases` collision.
    ///
    /// Called by [`JitConfig::load`]. A `None` registry (no `[item_kinds]` table
    /// at all) declares no kinds (the engine bakes in none) and so validates
    /// trivially. Kinds are checked in name order so the first error is
    /// deterministic. The optional `source` PATH is not one of the six and is not
    /// required. The optional `aliases` list is likewise not required, but an
    /// alias that duplicates a kind name or another kind's alias is an
    /// [`ItemKindConfigError::AliasCollision`] (aliases share the kind-name
    /// namespace).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::config::JitConfig;
    ///
    /// // No `[item_kinds]` table -> no kinds declared, nothing to validate.
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
        for name in &names {
            let missing = registry[*name].missing_required_fields();
            if !missing.is_empty() {
                return Err(ItemKindConfigError::MissingFields {
                    kind: (*name).clone(),
                    missing: missing.join(", "),
                });
            }
        }
        // Aliases share the kind-name namespace: an alias that duplicates a
        // registry key or another declared alias is ambiguous. Kind names take
        // precedence, and aliases are checked in kind-name then declaration order
        // so the first reported collision is deterministic.
        let kind_names: std::collections::HashSet<&str> =
            registry.keys().map(String::as_str).collect();
        let mut seen_aliases: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for name in &names {
            let Some(aliases) = &registry[*name].aliases else {
                continue;
            };
            for alias in aliases {
                if kind_names.contains(alias.as_str()) {
                    return Err(ItemKindConfigError::AliasCollision {
                        kind: (*name).clone(),
                        alias: alias.clone(),
                        conflict: format!("kind name '{alias}'"),
                    });
                }
                if !seen_aliases.insert(alias.as_str()) {
                    return Err(ItemKindConfigError::AliasCollision {
                        kind: (*name).clone(),
                        alias: alias.clone(),
                        conflict: format!("another declared alias '{alias}'"),
                    });
                }
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
    /// Load the system/user/repo-layered effective configuration for
    /// `jit_root`, probing each source's existence and loading whichever are
    /// present.
    ///
    /// Owns ALL filesystem interaction for this assembly — the `/etc/jit` and
    /// `~/.config/jit` existence checks, home-directory resolution, and each
    /// present source's [`JitConfig::load`] — so callers outside this module
    /// (notably the `commands` layer) never touch `std::fs` / `dirs` directly
    /// (AGENTS.md "Separation of Concerns": config IO stays in the config
    /// layer). Mirrors the system (`/etc/jit`) > user (`~/.config/jit`) > repo
    /// priority `jit config show` has always used.
    pub fn load(jit_root: &Path) -> Result<Self> {
        let mut loader = ConfigLoader::new();
        let system_path = Path::new("/etc/jit");
        if system_path.exists() {
            loader = loader.with_system_config(system_path)?;
        }
        if let Some(home) = dirs::home_dir() {
            let user_path = home.join(".config/jit");
            if user_path.exists() {
                loader = loader.with_user_config(&user_path)?;
            }
        }
        loader = loader.with_repo_config(jit_root)?;
        Ok(loader.build())
    }

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

    /// Assemble a single JSON snapshot of the effective configuration across
    /// its WHOLE surface, for `jit config get`'s generic dotted-path walk
    /// (see [`resolve_dotted_key`](crate::commands::resolve_dotted_key)).
    ///
    /// Two resolution strategies, matching how the rest of the codebase
    /// already reads these sections — this method does not invent a THIRD
    /// merge policy:
    /// - `worktree`, `coordination`, `global_operations`, `locks`, `events`:
    ///   the existing system/user/repo-merged accessors above (env var, then
    ///   repo, then user, then system, then default), unchanged from `jit
    ///   config show`.
    /// - every other section (`version`, `project`, `type_hierarchy`,
    ///   `validation`, `documentation`, `namespaces`, `item_kinds`,
    ///   `projection`): read from the
    ///   REPO config only, with no system/user merge and no built-in
    ///   defaults layered in. jit has no notion of a system/user override for
    ///   a repo's type hierarchy, label namespaces, or item kinds — every
    ///   other reader of these fields loads the repo file directly (e.g.
    ///   [`ConfigManager::namespaces_from_config`](crate::config_manager::ConfigManager::namespaces_from_config)).
    ///   This is also why these sections reflect exactly what `config.toml`
    ///   declares rather than `config show`'s built-in-default-filled view: an
    ///   absent section serializes as an empty object (matching an absent
    ///   TOML table), not a struct of defaulted fields.
    ///
    /// `templates` and `invariants` are deliberately excluded: both are
    /// `#[serde(skip)]` on [`JitConfig`], populated from the SIBLING
    /// `templates.toml` / `invariants.toml` files rather than `config.toml`
    /// itself, so neither has a dotted path here. Introspect them via `jit
    /// config list-templates` / `jit invariant list`.
    pub fn full_snapshot(&self) -> Result<serde_json::Value> {
        fn section<T: Serialize>(value: Option<&T>) -> Result<serde_json::Value> {
            match value {
                Some(v) => Ok(serde_json::to_value(v)?),
                None => Ok(serde_json::json!({})),
            }
        }

        let repo = self.repo_config.as_ref();
        let mut map = serde_json::Map::new();
        map.insert(
            "version".to_string(),
            section(repo.and_then(|c| c.version.as_ref()))?,
        );
        map.insert(
            "project".to_string(),
            section(repo.and_then(|c| c.project.as_ref()))?,
        );
        map.insert(
            "type_hierarchy".to_string(),
            section(repo.and_then(|c| c.type_hierarchy.as_ref()))?,
        );
        map.insert(
            "validation".to_string(),
            section(repo.and_then(|c| c.validation.as_ref()))?,
        );
        map.insert(
            "documentation".to_string(),
            section(repo.and_then(|c| c.documentation.as_ref()))?,
        );
        map.insert(
            "namespaces".to_string(),
            section(repo.and_then(|c| c.namespaces.as_ref()))?,
        );
        map.insert(
            "item_kinds".to_string(),
            section(repo.and_then(|c| c.item_kinds.as_ref()))?,
        );
        map.insert(
            "projection".to_string(),
            section(repo.and_then(|c| c.projection.as_ref()))?,
        );

        map.insert(
            "worktree".to_string(),
            serde_json::json!({
                "mode": format!(
                    "{:?}",
                    self.worktree_mode().unwrap_or(WorktreeMode::Auto)
                )
                .to_lowercase(),
                "enforce_leases": format!(
                    "{:?}",
                    self.enforcement_mode().unwrap_or(EnforcementMode::Strict)
                )
                .to_lowercase(),
            }),
        );
        map.insert(
            "coordination".to_string(),
            serde_json::json!({
                "default_ttl_secs": self.coordination().default_ttl_secs(),
                "lease_renewal_threshold_pct": self.coordination().lease_renewal_threshold_pct(),
                "stale_threshold_secs": self.coordination().stale_threshold_secs(),
                "max_indefinite_leases_per_agent": self.coordination().max_indefinite_leases_per_agent(),
                "max_indefinite_leases_per_repo": self.coordination().max_indefinite_leases_per_repo(),
            }),
        );
        map.insert(
            "global_operations".to_string(),
            serde_json::json!({
                "require_main_history": self.global_operations().require_main_history(),
                "allowed_branches": self.global_operations().allowed_branches(),
            }),
        );
        map.insert(
            "locks".to_string(),
            serde_json::json!({
                "max_age_secs": self.locks().max_age_secs(),
                "enable_metadata": self.locks().enable_metadata(),
            }),
        );
        map.insert(
            "events".to_string(),
            serde_json::json!({
                "enable_sequences": self.events().enable_sequences(),
                "use_unified_envelope": self.events().use_unified_envelope(),
            }),
        );

        Ok(serde_json::Value::Object(map))
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
            .unwrap_or(crate::runtime_defaults::CLAIM_TTL_SECS)
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
        let roles = &config.templates.roles;
        assert_eq!(plan.planning_type(roles), Some("planning"));
        assert_eq!(plan.breakdown_type(roles), Some("breakdown"));
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
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"
"#,
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        assert_eq!(config.invariants.invariants.len(), 1);
        let inv = &config.invariants.invariants[0];
        assert_eq!(inv.id, "sample-invariant");
        assert_eq!(
            inv.kind,
            crate::declarations::invariants::InvariantKind::Enforced
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
            "[[invariants]]\nid = \"second-invariant\"\nstatement = \"s\"\nkind = \"advisory\"\n",
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.invariants.invariants.len(), 1);
        assert_eq!(config.invariants.invariants[0].id, "second-invariant");
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
    fn test_documentation_config_unauthored_table_classifies_no_area() {
        // A table whose keys are all absent declares no classification, so no
        // area name reaches a repository that never named one (REQ-01).
        let unauthored = DocumentationConfig::default();

        assert!(unauthored.development_root().is_empty());
        assert!(unauthored.archive_root().is_empty());
        assert!(unauthored.managed_paths().is_empty());
        assert!(unauthored.permanent_paths().is_empty());
        assert!(unauthored.issue_scoped_areas().is_empty());
        // The membership query answers from the same empty registry, so an area
        // an adopter might plausibly name is rejected rather than accepted by a
        // declaration they never wrote.
        assert!(!unauthored.is_issue_scoped_area("dev/active"));
    }

    #[test]
    fn test_documentation_config_accessors_answer_from_the_authored_declaration() {
        // Each accessor reports what the table declares, so a repository's
        // classification is its own declaration and nothing else.
        let authored = DocumentationConfig {
            development_root: Some("workspace".to_string()),
            managed_paths: Some(vec!["workspace/drafts".to_string()]),
            archive_root: Some("workspace/attic".to_string()),
            permanent_paths: Some(vec!["workspace/handbook".to_string()]),
            issue_scoped_areas: Some(vec!["workspace/drafts".to_string()]),
            citation_scan_roots: None,
        };

        assert_eq!(authored.development_root(), "workspace");
        assert_eq!(authored.archive_root(), "workspace/attic");
        assert_eq!(authored.managed_paths(), vec!["workspace/drafts"]);
        assert_eq!(authored.permanent_paths(), vec!["workspace/handbook"]);
        assert!(authored.is_issue_scoped_area("workspace/drafts"));
        // The unauthored scan universe derives from this table's own values.
        assert_eq!(
            authored.citation_scan_roots(),
            vec!["workspace", "workspace/handbook"]
        );
    }

    #[test]
    fn test_is_issue_scoped_area_matches_a_declared_area_exactly_rather_than_by_prefix() {
        use crate::domain::artifact_classifier::contains_path;

        let unauthored = DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            issue_scoped_areas: None,
            citation_scan_roots: None,
        };
        let declared = unauthored
            .issue_scoped_areas()
            .into_iter()
            .next()
            .expect("the shipped registry should declare at least one area");
        assert!(unauthored.is_issue_scoped_area(&declared));

        // The same area written with redundant path syntax names the same area.
        assert!(unauthored.is_issue_scoped_area(&format!("./{declared}/")));

        // Exact-area matching: a path *inside* a declared area is not itself a
        // declared area, so a caller cannot pass an issue's own directory where
        // an area is expected. Not vacuous — prefix containment, which the
        // archival classifier applies to its own path lists, does accept it.
        let inside = format!("{declared}/abcd1234-example");
        assert!(!unauthored.is_issue_scoped_area(&inside));
        assert!(contains_path(&declared, &inside));

        // A sibling whose name merely starts with a declared area's name is not
        // that area either.
        assert!(!unauthored.is_issue_scoped_area(&format!("{declared}-other")));
    }

    #[test]
    fn test_is_issue_scoped_area_follows_an_authored_registry_instead_of_the_shipped_one() {
        let shipped = DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            issue_scoped_areas: None,
            citation_scan_roots: None,
        };
        let shipped_area = shipped
            .issue_scoped_areas()
            .into_iter()
            .next()
            .expect("the shipped registry should declare at least one area");

        // An authored registry is the whole registry: it replaces the shipped
        // declaration rather than extending it, and its entries need no
        // relationship to the shipped area names (`@/invariant/domain-agnostic`).
        let authored_area = "workspace/notes".to_string();
        let authored = DocumentationConfig {
            issue_scoped_areas: Some(vec![authored_area.clone()]),
            ..shipped.clone()
        };
        assert_eq!(authored.issue_scoped_areas(), vec![authored_area.clone()]);
        assert!(authored.is_issue_scoped_area(&authored_area));
        assert!(!authored.is_issue_scoped_area(&shipped_area));

        // An authored empty registry declares that no area adopts the
        // convention, which is distinct from leaving the list unauthored.
        let opted_out = DocumentationConfig {
            issue_scoped_areas: Some(Vec::new()),
            ..shipped.clone()
        };
        assert!(opted_out.issue_scoped_areas().is_empty());
        assert!(!opted_out.is_issue_scoped_area(&shipped_area));
        assert!(shipped.is_issue_scoped_area(&shipped_area));
    }

    #[test]
    fn test_citation_scan_roots_resolve_to_the_development_root_with_the_permanent_paths_when_unauthored(
    ) {
        // The default is computed from the table it sits in rather than frozen
        // into a declaration of its own (`@/issue/8e071e18/decision/D-16`).
        let derived_universe = |config: &DocumentationConfig| {
            std::iter::once(config.development_root())
                .chain(config.permanent_paths())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let resolved_universe = |config: &DocumentationConfig| {
            config
                .citation_scan_roots()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        };

        let unauthored = DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            issue_scoped_areas: None,
            citation_scan_roots: None,
        };
        assert_eq!(
            resolved_universe(&unauthored),
            derived_universe(&unauthored)
        );

        // Reclassifying the table moves the default with it: a repository that
        // renames its development root and declares its own permanent areas
        // scans those, not the shipped ones.
        let reclassified = DocumentationConfig {
            development_root: Some("workspace".to_string()),
            permanent_paths: Some(vec![
                "workspace/guides".to_string(),
                "README.md".to_string(),
            ]),
            ..unauthored.clone()
        };
        assert_eq!(
            resolved_universe(&reclassified),
            derived_universe(&reclassified)
        );
        assert!(resolved_universe(&reclassified).is_disjoint(&resolved_universe(&unauthored)));
    }

    #[test]
    fn test_citation_scan_roots_replace_the_default_with_an_authored_universe_reaching_outside_the_development_root(
    ) {
        use crate::domain::artifact_classifier::contains_path;

        let unauthored = DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            issue_scoped_areas: None,
            citation_scan_roots: None,
        };
        let development_root = unauthored.development_root();

        // A directory entry and a file entry, both outside the development
        // root: the universe is not bounded by it.
        let outside_directory = "scripts".to_string();
        let outside_file = "CHANGELOG.md".to_string();
        assert!(!contains_path(&development_root, &outside_directory));
        assert!(!contains_path(&development_root, &outside_file));

        let authored_entries = vec![
            development_root.clone(),
            outside_directory.clone(),
            outside_file.clone(),
        ];
        let authored = DocumentationConfig {
            citation_scan_roots: Some(authored_entries.clone()),
            ..unauthored.clone()
        };
        let resolved = authored.citation_scan_roots();
        assert!(resolved.contains(&outside_directory) && resolved.contains(&outside_file));

        // The authored list is the whole universe: it replaces the default
        // instead of extending it, so a default root it omits is gone.
        let omitted = unauthored
            .citation_scan_roots()
            .into_iter()
            .find(|root| !authored_entries.contains(root))
            .expect("the default universe should reach roots this fixture omits");
        assert!(!resolved.contains(&omitted));
        assert_eq!(resolved.len(), authored_entries.len());

        // Entries are matched the way the classification lists are: a directory
        // entry reaches everything beneath it, a file entry reaches that one
        // file, and a sibling whose name merely starts the same way is outside.
        let scanned = |path: &str| resolved.iter().any(|root| contains_path(root, path));
        assert!(scanned(&format!("{outside_directory}/ci/check.sh")));
        assert!(scanned(&outside_file));
        assert!(!scanned(&format!("{outside_file}.bak")));
        assert!(!scanned(&format!("{outside_directory}-legacy/check.sh")));
        assert!(!scanned("target/debug/build.log"));
    }

    #[test]
    fn test_unauthored_documentation_table_stays_non_configured_despite_fallbacks() {
        // The fallbacks stay non-authorizing (REQ-03): a repository with no
        // `[documentation]` table, or one that authors none of the required
        // fields, must still report Unconfigured/Incomplete from
        // `PolicyStatus::from_documentation` — which judges authored fields
        // only, never what the accessors above would fall back to.
        use crate::domain::artifact_plan::PolicyStatus;

        assert_eq!(
            PolicyStatus::from_documentation(None),
            PolicyStatus::Unconfigured
        );

        let unauthored = DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            issue_scoped_areas: None,
            citation_scan_roots: None,
        };
        assert_eq!(
            PolicyStatus::from_documentation(Some(&unauthored)),
            PolicyStatus::Incomplete
        );

        // Not vacuous: the fallbacks themselves are non-empty (derived from
        // the real shipped declaration), yet still don't count as authored.
        assert!(!unauthored.managed_paths().is_empty());
        assert!(!unauthored.permanent_paths().is_empty());
    }

    #[test]
    fn test_load_invalid_invariant_fails_config_load_both_paths() {
        // A malformed invariant entry fails config load with a descriptive,
        // context-bearing error on BOTH the config-absent and config-present paths.
        let bad = "[[invariants]]\nid = \"sample-invariant\"\nkind = \"advisory\"\n"; // missing statement

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
    fn test_each_strictness_level_deserializes() {
        for level in ["strict", "loose", "permissive", "STRICT"] {
            let toml = format!("[validation]\nstrictness = \"{level}\"\n");
            let config: JitConfig =
                toml::from_str(&toml).unwrap_or_else(|e| panic!("'{level}' must deserialize: {e}"));
            // The validated raw spelling is preserved for round-tripping.
            assert_eq!(
                config.validation.unwrap().strictness,
                Some(level.to_string())
            );
        }
    }

    #[test]
    fn test_invalid_strictness_is_rejected_at_deserialize() {
        // An unrecognized level must fail config load eagerly, not defer the
        // error to a later validation call (F1). The message names the bad value.
        let toml = "[validation]\nstrictness = \"banana\"\n";
        let err = toml::from_str::<JitConfig>(toml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("banana"), "names the bad value: {msg}");
        assert!(
            msg.contains("strict") && msg.contains("loose") && msg.contains("permissive"),
            "lists the accepted values: {msg}"
        );
    }

    #[test]
    fn test_invalid_strictness_fails_config_load_from_disk() {
        // The eager rejection also fires through the on-disk load path, so a
        // committed config.toml with a bad strictness never resolves silently.
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("config.toml"),
            "[validation]\nstrictness = \"banana\"\n",
        )
        .unwrap();
        let err = JitConfig::load(temp_dir.path()).unwrap_err();
        // The load path wraps the deserialize error with context, so inspect the
        // full chain for the offending value.
        let chain = format!("{err:#}");
        assert!(chain.contains("banana"), "{chain}");
        assert!(chain.contains("strictness"), "{chain}");
    }

    #[test]
    fn test_removed_icon_preset_is_rejected_during_config_load() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("config.toml"),
            r#"
[type_hierarchy]
types = { objective = 1 }

[type_hierarchy.icons]
preset = "navigation"
"#,
        )
        .unwrap();

        let error = JitConfig::load(temp_dir.path())
            .expect_err("removed icon preset must not be silently ignored");
        let message = format!("{error:#}");
        assert!(
            message.contains("preset"),
            "error names the removed key: {message}"
        );
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
        // `.jit/rules.toml` is the operative validation ruleset, so the stale keys
        // have no effect.
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
    fn test_projection_empty_kind_list_is_rejected() {
        // A `[projection.*]` table with `kind = []` declares nothing addressable
        // to render; parsing rejects it instead of accepting a no-items
        // projection (jit:450db193 review F1, round 4).
        let config_toml = r#"
[projection.empty]
kind = []
mode = "region"
target = "AGENTS.md"
"#;
        let error = toml::from_str::<JitConfig>(config_toml)
            .expect_err("empty projection kind list must not parse");
        assert!(
            error
                .to_string()
                .contains("must name at least one addressable kind"),
            "error names the empty-kind-list defect: {error}"
        );
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
            other => panic!("expected MissingFields, got {other:?}"),
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

    /// A complete `[item_kinds.X]` table body with the six required fields, so a
    /// test can focus on the `aliases` behavior under study.
    fn complete_kind_body() -> &'static str {
        "section = \"success_criteria\"\n\
         id-pattern = \"REQ-\\\\d+\"\n\
         markers = []\n\
         link-namespaces = [\"satisfies\"]\n\
         scope = \"issue\"\n\
         source-of-truth = \"markdown-first\"\n"
    }

    #[test]
    fn test_item_kinds_aliases_parse_and_validate() {
        // REQ-01: `aliases` parses from `[item_kinds.<kind>]` and a non-colliding
        // alias validates cleanly.
        let body = complete_kind_body();
        let config_toml = format!("[item_kinds.invariant]\n{body}aliases = [\"inv\"]\n");
        let config: JitConfig = toml::from_str(&config_toml).unwrap();
        config
            .validate_item_kinds()
            .expect("non-colliding alias validates");
        let kinds = config.item_kinds.unwrap();
        assert_eq!(
            kinds["invariant"].aliases.as_deref(),
            Some(["inv".to_string()].as_slice())
        );
    }

    #[test]
    fn test_item_kinds_alias_colliding_with_kind_name_is_rejected() {
        // REQ-02: an alias equal to a declared kind NAME is rejected — aliases
        // share the kind-name namespace.
        let body = complete_kind_body();
        let config_toml = format!(
            "[item_kinds.requirement]\n{body}\n[item_kinds.decision]\n{body}aliases = [\"requirement\"]\n"
        );
        let config: JitConfig = toml::from_str(&config_toml).unwrap();
        let err = config
            .validate_item_kinds()
            .expect_err("alias duplicating a kind name must be rejected");
        match err {
            ItemKindConfigError::AliasCollision {
                kind,
                alias,
                conflict,
            } => {
                assert_eq!(kind, "decision");
                assert_eq!(alias, "requirement");
                assert!(
                    conflict.contains("kind name"),
                    "conflict names cause: {conflict}"
                );
            }
            other => panic!("expected AliasCollision, got {other:?}"),
        }
    }

    #[test]
    fn test_item_kinds_alias_colliding_with_another_alias_is_rejected() {
        // REQ-02: an alias equal to ANOTHER kind's alias is rejected.
        let body = complete_kind_body();
        let config_toml = format!(
            "[item_kinds.requirement]\n{body}aliases = [\"rq\"]\n[item_kinds.decision]\n{body}aliases = [\"rq\"]\n"
        );
        let config: JitConfig = toml::from_str(&config_toml).unwrap();
        let err = config
            .validate_item_kinds()
            .expect_err("alias duplicating another alias must be rejected");
        match err {
            ItemKindConfigError::AliasCollision {
                alias, conflict, ..
            } => {
                assert_eq!(alias, "rq");
                assert!(
                    conflict.contains("another declared alias"),
                    "conflict names cause: {conflict}"
                );
            }
            other => panic!("expected AliasCollision, got {other:?}"),
        }
    }

    #[test]
    fn test_item_kinds_alias_collision_rejected_through_load() {
        // The alias-collision guard fires through the real `JitConfig::load` path,
        // the same path that rejects a partial `[item_kinds]` declaration.
        let body = complete_kind_body();
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(
            temp_dir.path().join("config.toml"),
            format!("[item_kinds.requirement]\n{body}aliases = [\"requirement\"]\n"),
        )
        .unwrap();
        let err = JitConfig::load(temp_dir.path()).expect_err("load must reject colliding alias");
        let msg = format!("{err:#}");
        assert!(msg.contains("requirement"), "error names the token: {msg}");
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
        // Uses `policy` for the registry-first example: the direction
        // (`source-of-truth`) is independent of the `source` field. (The built-in
        // `invariant` kind is an ordinary registry-first kind too; this test only
        // exercises the typed parse.)
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
        // (relied on by direct struct construction).
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
        // domain layer then resolves an EMPTY kind set (the engine bakes in no
        // kinds) in that case.
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
lease_renewal_threshold_pct = 10
stale_threshold_secs = 3600
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs, Some(600));
        assert_eq!(coord.lease_renewal_threshold_pct, Some(10));
        assert_eq!(coord.stale_threshold_secs, Some(3600));
        assert_eq!(coord.max_indefinite_leases_per_agent, Some(2));
        assert_eq!(coord.max_indefinite_leases_per_repo, Some(10));
    }

    #[test]
    fn test_coordination_config_defaults() {
        let coord = CoordinationConfig::default();
        assert_eq!(coord.default_ttl_secs(), 600);
        assert_eq!(coord.lease_renewal_threshold_pct(), 10);
        assert_eq!(coord.stale_threshold_secs(), 3600);
        assert_eq!(coord.max_indefinite_leases_per_agent(), 2);
        assert_eq!(coord.max_indefinite_leases_per_repo(), 10);
    }

    /// A `[coordination]` block carrying the removed auto-heartbeat daemon keys
    /// (`heartbeat_interval_secs`, `auto_renew_leases`) still parses: the unknown
    /// keys are ignored, and the surviving keys resolve normally.
    #[test]
    fn test_coordination_config_ignores_legacy_heartbeat_keys() {
        let config_toml = r#"
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
auto_renew_leases = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs, Some(600));
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
    }

    /// An agent config carrying a legacy `[behavior]` section (the removed
    /// auto-heartbeat daemon settings) still parses: the unknown section is
    /// ignored rather than rejected.
    #[test]
    fn test_agent_config_ignores_legacy_behavior_section() {
        let config_toml = r#"
[agent]
id = "agent:copilot-1"

[behavior]
auto_heartbeat = false
heartbeat_interval = 30
"#;
        let config: AgentConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.agent.id, "agent:copilot-1");
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
        assert_eq!(config.coordination().stale_threshold_secs(), 3600);
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
        assert_eq!(config.coordination().stale_threshold_secs(), 3600);
    }

    #[test]
    fn test_config_loader_repo_overrides_user() {
        let user_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();

        // User config sets TTL to 900
        let user_config = r#"
[coordination]
default_ttl_secs = 900
stale_threshold_secs = 1800
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
        // User value used for stale threshold (not in repo config)
        assert_eq!(config.coordination().stale_threshold_secs(), 1800);
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
max_indefinite_leases_per_agent = 1
stale_threshold_secs = 1800
"#;
        std::fs::write(system_dir.path().join("config.toml"), system_config).unwrap();

        // User config overrides system
        let user_config = r#"
[coordination]
default_ttl_secs = 600
max_indefinite_leases_per_agent = 7
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
        // User wins for the per-agent indefinite-lease cap (not in repo)
        assert_eq!(config.coordination().max_indefinite_leases_per_agent(), 7);
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

    // ============================================================
    // [project] table tests (TDD, jit:3d9e9222)
    // ============================================================

    #[test]
    fn test_project_name_accepts_valid_tokens() {
        assert_eq!("a".parse::<ProjectName>().unwrap().as_str(), "a");
        assert_eq!(
            "just-in-time".parse::<ProjectName>().unwrap().as_str(),
            "just-in-time"
        );
        assert_eq!("a1-2b3".parse::<ProjectName>().unwrap().as_str(), "a1-2b3");
    }

    #[test]
    fn test_project_name_rejects_uppercase() {
        let err = "Bad_Name".parse::<ProjectName>().unwrap_err();
        assert!(err.to_string().contains("Bad_Name"));
    }

    #[test]
    fn test_project_name_rejects_leading_digit() {
        let err = "1abc".parse::<ProjectName>().unwrap_err();
        assert!(err.to_string().contains("1abc"));
    }

    #[test]
    fn test_project_name_rejects_underscore() {
        let err = "my_project".parse::<ProjectName>().unwrap_err();
        assert!(err.to_string().contains("my_project"));
    }

    #[test]
    fn test_project_name_rejects_empty() {
        assert!("".parse::<ProjectName>().is_err());
    }

    #[test]
    fn test_slugify_project_name_lowercases_and_dashes() {
        assert_eq!(slugify_project_name("My Cool Project!"), "my-cool-project");
        assert_eq!(slugify_project_name("Just_In_Time"), "just-in-time");
        assert_eq!(slugify_project_name("just-in-time"), "just-in-time");
    }

    #[test]
    fn test_slugify_project_name_falls_back_on_leading_digit() {
        // A basename starting with a digit can never satisfy
        // ^[a-z][a-z0-9-]*$, even after slugifying, so it falls back.
        assert_eq!(slugify_project_name("123-repo"), "project");
    }

    #[test]
    fn test_slugify_project_name_falls_back_on_empty_result() {
        assert_eq!(slugify_project_name(""), "project");
        assert_eq!(slugify_project_name("___"), "project");
        assert_eq!(slugify_project_name("!!!"), "project");
    }

    #[test]
    fn test_slugify_project_name_result_always_parses_as_project_name() {
        for input in ["My Repo", "123-repo", "", "___", "café-repo", "a"] {
            let slug = slugify_project_name(input);
            assert!(
                slug.parse::<ProjectName>().is_ok(),
                "slug '{slug}' from input '{input}' must satisfy ProjectName's pattern"
            );
        }
    }

    #[test]
    fn test_jitconfig_load_parses_valid_project_table() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[project]\nname = \"just-in-time\"\n",
        )
        .unwrap();

        let config = JitConfig::load(dir.path()).unwrap();
        assert_eq!(
            config
                .project
                .as_ref()
                .unwrap()
                .name
                .as_ref()
                .unwrap()
                .as_str(),
            "just-in-time"
        );
    }

    #[test]
    fn test_jitconfig_load_rejects_invalid_project_name() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[project]\nname = \"Bad_Name\"\n",
        )
        .unwrap();

        let result = JitConfig::load(dir.path());
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("Bad_Name"),
            "error must name the offending value: {msg}"
        );
    }

    #[test]
    fn test_jitconfig_load_project_absent_is_none() {
        let dir = TempDir::new().unwrap();
        let config = JitConfig::load(dir.path()).unwrap();
        assert!(config.project.is_none());
    }
}
