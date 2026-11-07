//! Configuration declarations and effective values.
//!
//! JIT supports repository-level configuration through `.jit/config.toml`.
//! If no config file exists, the system falls back to sensible defaults.

use crate::domain::item::KindScope;
use crate::domain::vocabulary::wire_vocabulary;
#[cfg(test)]
use crate::repository_state::VirtualPath;
use crate::runtime_defaults::{MAX_INDEFINITE_LEASES_PER_AGENT, MAX_INDEFINITE_LEASES_PER_REPO};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration structure loaded from `.jit/config.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JitConfig {
    /// Schema version for migrations (optional).
    pub version: Option<VersionConfig>,
    /// Project identity configuration (optional `[project]` table).
    ///
    /// Declares the project's canonical, human-editable name: the `@<project>`
    /// scope token used by the multi-jit addressing scheme. `jit init` derives
    /// this from a slug of the repository directory's basename when
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
    /// ([`ProjectionMode`]), `target` (repo-relative), render `style`
    /// ([`ProjectionStyle`]), and optional region delimiters. Keyed by the
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
    /// Graph templates loaded and validated from `.jit/templates.toml`.
    ///
    /// Not read from `config.toml`: populated by
    /// [`assemble_configuration`](crate::declarations::assemble_configuration) from the sibling
    /// `templates.toml` (absent file → empty registry), so it carries
    /// `#[serde(skip)]`.
    #[serde(skip)]
    pub templates: crate::templates::TemplateRegistry,
    /// Project invariants loaded from `.jit/invariants.toml`.
    ///
    /// Not read from `config.toml`: populated by
    /// [`assemble_configuration`](crate::declarations::assemble_configuration) from the sibling
    /// `invariants.toml` (absent file → empty registry), so it carries
    /// `#[serde(skip)]`. Kept here so later indexing can project each entry as a project-scoped
    /// (`@`) addressable item.
    #[serde(skip)]
    pub invariants: crate::declarations::invariants::InvariantRegistry,
}

/// Schema version configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionConfig {
    /// Schema version number.
    pub schema: u32,
}

impl VersionConfig {
    /// Schema version applied when the repository declares no `[version]`.
    pub const DEFAULT_SCHEMA: u32 = 2;
}

/// Project identity configuration from the `[project]` TOML table.
///
/// The sole home for the project's canonical, human-editable name — not
/// `index.json`, which carries only machine metadata
/// (`schema_version`/`deleted_ids`) and is not meant to be hand-edited.
/// Resolve a present value with [`JitConfig::project_name`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// The project's canonical name: the `@<project>` scope token in the
    /// multi-jit addressing scheme. `None` when the `[project]` table is
    /// present but the `name` key is absent.
    pub name: Option<ProjectName>,
}

/// Parse error for [`ProjectName`].
///
/// Returned by `ProjectName::from_str` and (via serde) by TOML
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
/// `ProjectName::from_str` and TOML deserialization, mirroring the
/// [`EnforcementMode`] pattern: an invalid value is a
/// descriptive parse error, not a silent fallback, so a misconfigured
/// `[project] name` is rejected at configuration assembly and surfaces
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
#[serde(deny_unknown_fields)]
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

/// Validation behavior configuration.
///
/// Per-rule enforcement lives in `.jit/rules.toml` (DR §8.2/§8.4), whose built-in
/// default rules derive from this repo's registry at load.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationConfig {
    /// Repo-wide enforcement strictness. It modulates which rule violations block
    /// a write or transition, layered on top of each rule's `enforce`/severity, without
    /// changing either. Decoded as [`Strictness`](crate::validation::Strictness)
    /// so configuration access and serialization use its canonical token.
    #[serde(default)]
    pub strictness: Option<crate::validation::Strictness>,
    /// Default type when none specified (optional).
    pub default_type: Option<String>,
    /// Repo-level default content format for issue bodies. Selects the
    /// [`ContentParser`](crate::document::ContentParser) used to extract
    /// `sections` for issues that carry no per-issue `content_format`.
    #[serde(default)]
    pub content_format: Option<crate::domain::ContentFormat>,
}

impl ValidationConfig {
    /// Return the repo-level default content format, defaulting to
    /// [`ContentFormat::DEFAULT`](crate::domain::ContentFormat::DEFAULT) when
    /// unset. Serde rejects values outside the exact wire vocabulary while
    /// loading configuration.
    pub fn content_format(&self) -> crate::domain::ContentFormat {
        self.content_format.unwrap_or_default()
    }

    /// Resolve the repo-wide validation strictness, defaulting to
    /// [`Strictness::DEFAULT`](crate::validation::Strictness::DEFAULT) when unset.
    pub fn strictness(&self) -> crate::validation::Strictness {
        self.strictness.unwrap_or_default()
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
#[serde(deny_unknown_fields)]
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

/// Label namespace configuration from TOML: taxonomy only. Value constraints
/// are authored as rules in `.jit/rules.toml` (DR §8.4).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
/// all six: configuration assembly validates this via
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
/// A kind may be **project-scoped**, addressing items not tied to any
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
/// use jit::config::JitConfig;
/// use jit::domain::item::KindScope;
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
/// assert_eq!(gloss.scope, Some(KindScope::Project));
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
#[serde(deny_unknown_fields)]
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
    /// Addressing scope: [`KindScope::Issue`] for items projected from issue
    /// descriptions (qualified id `@/issue/<short-id>/<kind>/<self-id>`), or
    /// [`KindScope::Project`] for items projected from a repository-local
    /// `source` file (qualified id `@/<kind>/<self-id>`). Required in an explicit
    /// declaration; an unrecognised token is a TOML parse error, not a silent
    /// fallback.
    pub scope: Option<KindScope>,
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
    /// reports which (if any) are missing so configuration assembly can name them in
    /// a descriptive error. The optional `source` PATH is not checked. An empty
    /// result means the declaration is complete.
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
    /// [`SourceOfTruth::DEFAULT`] when the `source-of-truth` field is unset.
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
#[serde(deny_unknown_fields)]
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

/// The authoring DIRECTION of an item kind: which substrate is the canonical
/// source for its items.
///
/// This is the sixth field of the item-kind six-tuple `(section, id-pattern,
/// marker(s), link-namespace(s), scope, source-of-truth)`. It is DISTINCT from
/// the [`ItemKindConfig::source`] file PATH: `source-of-truth` is a direction,
/// `source` is a location. A `markdown-first` kind is authored in prose and indexed from it; a `registry-first` kind is authored in
/// a structured registry file and projected from it.
///
/// Deserialized from the kebab-case tokens `"markdown-first"` / `"registry-first"`
/// via serde rename; an unrecognized value is a descriptive parse error rather
/// than a silent default.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum SourceOfTruth {
    /// Items are authored in markdown prose (issue descriptions or a markdown
    /// source file) and indexed from it.
    #[serde(rename = "markdown-first")]
    MarkdownFirst,
    /// Items are authored in a structured registry file and projected from it.
    #[serde(rename = "registry-first")]
    RegistryFirst,
}

impl SourceOfTruth {
    /// Direction applied when a kind omits `source-of-truth`: the requirement
    /// kind's, which is authored in issue descriptions and indexed from them.
    pub const DEFAULT: Self = Self::MarkdownFirst;
}

impl Default for SourceOfTruth {
    fn default() -> Self {
        Self::DEFAULT
    }
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
/// - `mode` — [`ProjectionMode`]: a whole separate file, or a delimited region
///   rewritten byte-preserving everything outside it.
/// - `target` — REQUIRED repo-relative documentation path (config-driven; the
///   engine hardcodes no filename and applies no default). A projection that
///   declares no target is a typed error at render and validation, never a
///   silent default.
/// - `style` — [`ProjectionStyle`] (`id-anchor` renders generic `- **{id}** —
///   {text}` rows; `full` renders the built-in rich registry views).
/// - `region-begin` / `region-end` — optional region delimiters; when unset they
///   default to [`default_region_begin`] / [`default_region_end`], derived from
///   the projection name (so the accessors take that name).
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
#[serde(deny_unknown_fields)]
pub struct ProjectionConfig {
    /// The item kind(s) whose addressable rows this projection renders.
    pub kind: ProjectionKinds,
    /// Projection mode: a whole separate file, or a delimited region within an
    /// existing file. Defaults to [`ProjectionMode::DEFAULT`] when unset.
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
    /// [`default_region_begin`] when unset.
    #[serde(default, rename = "region-begin")]
    pub region_begin: Option<String>,
    /// End marker delimiting the rewritten region in `region` mode. Defaults to
    /// [`default_region_end`] when unset.
    #[serde(default, rename = "region-end")]
    pub region_end: Option<String>,
    /// Render style for the projected markdown block. Defaults to
    /// [`ProjectionStyle::DEFAULT`] when unset.
    #[serde(default)]
    pub style: Option<ProjectionStyle>,
}

impl ProjectionConfig {
    /// The declared kind names, in authored order.
    pub fn kinds(&self) -> &[String] {
        self.kind.names()
    }

    /// The resolved projection mode (defaulting to [`ProjectionMode::DEFAULT`]).
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

    /// The resolved render style (defaulting to [`ProjectionStyle::DEFAULT`]).
    pub fn style(&self) -> ProjectionStyle {
        self.style.unwrap_or_default()
    }
}

wire_vocabulary! {
    /// How a registry is projected into its documentation target.
    ///
    /// Deserialized from each variant's kebab-case token; an unrecognized value
    /// is a descriptive parse error rather than a silent default. The default (no `mode` field) is [`ProjectionMode::DEFAULT`].
    #[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
    pub enum ProjectionMode {
        /// Write the rendered block to a separate whole file.
        SeparateFile = "separate-file",
        /// Replace only a delimited region within an existing file, byte-preserving
        /// everything outside the delimiters.
        Region = "region",
    }
}

impl ProjectionMode {
    /// Mode applied when a projection omits `mode`.
    pub const DEFAULT: Self = Self::SeparateFile;
}

impl Default for ProjectionMode {
    fn default() -> Self {
        Self::DEFAULT
    }
}

wire_vocabulary! {
    /// How a projected registry is rendered into the markdown block.
    ///
    /// Deserialized from each variant's token; an unrecognized value is a
    /// descriptive parse error rather than a silent default. The default (no
    /// `style` field) is [`ProjectionStyle::DEFAULT`].
    ///
    /// - [`ProjectionStyle::IdAnchor`] renders a heading-less generic bullet
    ///   list of `- **{self-id}** — {text}` rows from any addressable item kind.
    /// - [`ProjectionStyle::Full`] renders the built-in rich registry views,
    ///   with typed metadata absent from a generic addressable row.
    #[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
    pub enum ProjectionStyle {
        /// Render the built-in rich registry view for the projection's kind(s): the
        /// invariant registry's `[kind]`/enforced-by bullets, or the rule + gate
        /// registries' `## Rules` / `## Gates` sections with metadata.
        Full = "full",
        /// Render a heading-less generic bullet list of `- **{self-id}** — {text}`
        /// rows from the kind's addressable items, for embedding beneath a
        /// hand-authored heading. Kind-agnostic: any item kind renders this way.
        IdAnchor = "id-anchor",
    }
}

impl ProjectionStyle {
    /// Style applied when a projection omits `style`.
    pub const DEFAULT: Self = Self::Full;
}

impl Default for ProjectionStyle {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// An error validating an explicitly-declared `[item_kinds.X]` table.
///
/// Raised by [`JitConfig::validate_item_kinds`] (and thus configuration assembly)
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

/// Parse error for [`EnforcementMode`].
///
/// Returned by `EnforcementMode::from_str` and (via serde) by TOML /
/// environment-variable deserialization when the value is not one of the
/// recognised tokens.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnforcementModeError {
    /// The value is not the token of any [`EnforcementMode`].
    #[error("invalid enforcement mode '{value}'; expected {expected}")]
    Invalid {
        /// The unrecognised value.
        value: String,
        /// The accepted tokens, quoted and joined for the message.
        expected: String,
    },
}

/// Worktree and parallel work configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    /// Code root, absolute or relative to the selected data root.
    pub root: Option<std::path::PathBuf>,
    /// Lease enforcement mode; resolved by [`JitConfig::enforcement_mode`].
    pub enforce_leases: Option<EnforcementMode>,
    /// Write-policy stance for state-mutating commands run inside a linked
    /// non-primary checkout (default:
    /// [`LinkedCheckoutWriteStance::DEFAULT`](crate::domain::LinkedCheckoutWriteStance::DEFAULT)).
    pub write_policy: Option<crate::domain::LinkedCheckoutWriteStance>,
}

/// What another agent's active lease on an issue does to a write.
///
/// Parsed case-insensitively from [`EnforcementMode::as_str`] tokens in TOML
/// configuration (`[worktree] enforce_leases = ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Refuse the write.
    Strict,
    /// Warn and allow the write.
    Warn,
    /// Allow the write.
    Off,
}

impl EnforcementMode {
    /// Mode a `[worktree]` section applies when it omits `enforce_leases`.
    pub const DEFAULT: Self = Self::Strict;

    /// Every mode, in declaration order.
    pub const ALL: [Self; 3] = [Self::Strict, Self::Warn, Self::Off];

    /// The configuration token for this mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Warn => "warn",
            Self::Off => "off",
        }
    }
}

impl std::fmt::Display for EnforcementMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for EnforcementMode {
    type Err = EnforcementModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| {
                let [strict, warn, off] = Self::ALL;
                EnforcementModeError::Invalid {
                    value: s.to_string(),
                    expected: format!("'{strict}', '{warn}', or '{off}'"),
                }
            })
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
    /// Get the write-policy stance, defaulting to
    /// [`LinkedCheckoutWriteStance::DEFAULT`](crate::domain::LinkedCheckoutWriteStance::DEFAULT)
    /// when unset.
    ///
    /// The field is typed — invalid tokens are rejected at TOML parse time
    /// (via the domain type's derived `Deserialize`), so this method is
    /// infallible.
    pub fn write_policy(&self) -> crate::domain::LinkedCheckoutWriteStance {
        self.write_policy
            .unwrap_or(crate::domain::LinkedCheckoutWriteStance::DEFAULT)
    }
}

/// Coordination settings for leases and multi-agent work.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CoordinationConfig {
    /// Maximum concurrent TTL=0 leases per agent (default:
    /// [`MAX_INDEFINITE_LEASES_PER_AGENT`]).
    pub max_indefinite_leases_per_agent: Option<u32>,
    /// Maximum concurrent TTL=0 leases per repository (default:
    /// [`MAX_INDEFINITE_LEASES_PER_REPO`]).
    pub max_indefinite_leases_per_repo: Option<u32>,
}

/// The coordination limits in force: repository values, else built-in defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EffectiveCoordination {
    pub max_indefinite_leases_per_agent: u32,
    pub max_indefinite_leases_per_repo: u32,
}

/// Resolve the lease enforcement mode from a repository's `[worktree]` section:
/// `off` without the section, [`EnforcementMode::DEFAULT`] when the section
/// omits the key.
fn resolve_enforcement_mode(worktree: Option<&WorktreeConfig>) -> EnforcementMode {
    worktree.map_or(EnforcementMode::Off, |wt| {
        wt.enforce_leases.unwrap_or(EnforcementMode::DEFAULT)
    })
}

/// Resolve the lease limits from a repository's `[coordination]` section: each
/// key's value, else its built-in default.
fn resolve_coordination(coordination: Option<&CoordinationConfig>) -> EffectiveCoordination {
    let limit = |read: fn(&CoordinationConfig) -> Option<u32>, default| {
        coordination.and_then(read).unwrap_or(default)
    };
    EffectiveCoordination {
        max_indefinite_leases_per_agent: limit(
            |c| c.max_indefinite_leases_per_agent,
            MAX_INDEFINITE_LEASES_PER_AGENT,
        ),
        max_indefinite_leases_per_repo: limit(
            |c| c.max_indefinite_leases_per_repo,
            MAX_INDEFINITE_LEASES_PER_REPO,
        ),
    }
}

impl JitConfig {
    /// The lease enforcement mode write commands apply and `jit config` reports.
    ///
    /// Lease enforcement is repository policy, so only this repository config
    /// decides it; the environment does not.
    pub fn enforcement_mode(&self) -> EnforcementMode {
        resolve_enforcement_mode(self.worktree.as_ref())
    }

    /// The lease limits `claim acquire` enforces and `jit config` reports.
    ///
    /// Repository policy like [`Self::enforcement_mode`]: only this repository
    /// config decides them.
    pub fn coordination(&self) -> EffectiveCoordination {
        resolve_coordination(self.coordination.as_ref())
    }

    /// The repo-level content format, resolved by [`ValidationConfig::content_format`].
    pub fn content_format(&self) -> crate::domain::ContentFormat {
        self.validation
            .as_ref()
            .unwrap_or(&ValidationConfig::default())
            .content_format()
    }

    /// The repo-wide validation strictness, resolved by [`ValidationConfig::strictness`].
    pub fn strictness(&self) -> crate::validation::Strictness {
        self.validation
            .as_ref()
            .unwrap_or(&ValidationConfig::default())
            .strictness()
    }

    /// Validate every explicitly-declared `[item_kinds.X]` table, requiring all
    /// six fields (`section`, `id-pattern`, `markers`, `link-namespaces`, `scope`,
    /// `source-of-truth`) on each, and rejecting any `aliases` collision.
    ///
    /// Called by configuration assembly. A `None` registry (no `[item_kinds]` table
    /// at all) declares no kinds (the engine bakes in none) and so validates
    /// trivially. Kinds are checked in name order so the first error is
    /// deterministic. The optional `source` PATH is not one of the six and is not
    /// required. The optional `aliases` list is likewise not required, but an
    /// alias that duplicates a kind name or another kind's alias is an
    /// [`ItemKindConfigError::AliasCollision`] (aliases share the kind-name
    /// namespace).
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

/// The repository configuration as `jit config` reports it.
///
/// Lease policy (enforcement mode and limits) is repository-scoped, so every
/// value here comes from `.jit/config.toml` alone, else its built-in default.
#[derive(Debug, Default)]
pub struct EffectiveConfig {
    repo_config: Option<JitConfig>,
}

impl EffectiveConfig {
    /// Resolve effective values from an already-loaded repository configuration.
    pub fn new(repo_config: JitConfig) -> Self {
        Self {
            repo_config: Some(repo_config),
        }
    }

    /// Get the effective enforcement mode: the repository config's
    /// [`JitConfig::enforcement_mode`].
    pub fn enforcement_mode(&self) -> EnforcementMode {
        resolve_enforcement_mode(self.repo_config.as_ref().and_then(|c| c.worktree.as_ref()))
    }

    /// Get the effective lease limits: the repository config's
    /// [`JitConfig::coordination`].
    pub fn coordination(&self) -> EffectiveCoordination {
        resolve_coordination(
            self.repo_config
                .as_ref()
                .and_then(|c| c.coordination.as_ref()),
        )
    }

    /// Assemble a single JSON snapshot of the effective configuration across
    /// its WHOLE surface, for `jit config get`'s generic dotted-path walk
    /// (see [`resolve_dotted_key`](crate::commands::resolve_dotted_key)).
    ///
    /// `jit config show` renders this same snapshot. Two resolution
    /// strategies, matching how the rest of the codebase already reads these
    /// sections — this method does not invent a THIRD merge policy:
    /// - `worktree`, `coordination`: [`Self::enforcement_mode`] and
    ///   [`Self::coordination`] (both repo-only, default-filled).
    /// - every other section (`version`, `project`, `type_hierarchy`,
    ///   `validation`, `documentation`, `namespaces`, `item_kinds`,
    ///   `projection`): read from the
    ///   REPO config only, with no built-in defaults filled in. Every
    ///   other reader of these fields loads the repo file directly (e.g.
    ///   [`namespaces_from_config`](crate::config_manager::namespaces_from_config)).
    ///   This is also why these sections reflect exactly what `config.toml`
    ///   declares rather than `config show`'s built-in-default-filled view: an
    ///   absent section serializes as an empty object (matching an absent
    ///   TOML table), not a struct of defaulted fields.
    ///
    /// `templates` and `invariants` are deliberately excluded: both are
    /// `#[serde(skip)]` on [`JitConfig`], populated from the SIBLING
    /// `templates.toml` / `invariants.toml` files rather than `config.toml`
    /// itself, so neither has a dotted path here. Read the graph templates in
    /// `.jit/templates.toml`, and introspect the invariants via `jit item list
    /// --kind invariant`.
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
                "enforce_leases": self.enforcement_mode().as_str(),
            }),
        );
        map.insert(
            "coordination".to_string(),
            serde_json::to_value(self.coordination())?,
        );

        Ok(serde_json::Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble(
        config: Option<&str>,
        templates: Option<&str>,
        invariants: Option<&str>,
    ) -> std::result::Result<JitConfig, crate::declarations::ConfigurationAssemblyError> {
        crate::declarations::assemble_configuration(
            config.map(str::as_bytes),
            templates.map(str::as_bytes),
            invariants.map(str::as_bytes),
        )
    }

    /// The assembly error of `config.toml` alone, with its source chain.
    fn config_error(config: &str) -> String {
        format!(
            "{:#}",
            anyhow::Error::from(assemble(Some(config), None, None).unwrap_err())
        )
    }

    #[test]
    fn test_load_templates_when_config_absent() {
        // A repo with `.jit/templates.toml` but NO `.jit/config.toml` must still
        // load the registry (the absent-config path), not silently return empty.
        let config = assemble(
            None,
            Some(
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
            ),
            None,
        )
        .unwrap();
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
        let config = assemble(
            None,
            None,
            Some(
                r#"
[[invariants]]
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"
"#,
            ),
        )
        .unwrap();
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
        // With BOTH `config.toml` and `invariants.toml`, assembly keeps each.
        let config = assemble(
            Some("[type_hierarchy]\ntypes = { task = 1 }\n"),
            None,
            Some("[[invariants]]\nid = \"second-invariant\"\nstatement = \"s\"\nkind = \"advisory\"\n"),
        )
        .unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.invariants.invariants.len(), 1);
        assert_eq!(config.invariants.invariants[0].id, "second-invariant");
    }

    #[test]
    fn test_load_invariants_absent_file_is_empty_on_both_paths() {
        // No `invariants.toml` → empty registry, whether or not config.toml exists.
        [None, Some("[type_hierarchy]\ntypes = { task = 1 }\n")]
            .into_iter()
            .for_each(|config| {
                assert!(assemble(config, None, None)
                    .unwrap()
                    .invariants
                    .invariants
                    .is_empty());
            });
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

        let declared = "workspace/drafts".to_string();
        let authored = DocumentationConfig {
            issue_scoped_areas: Some(vec![declared.clone()]),
            ..DocumentationConfig::default()
        };
        assert!(authored.is_issue_scoped_area(&declared));

        // The same area written with redundant path syntax names the same area.
        assert!(authored.is_issue_scoped_area(&format!("./{declared}/")));

        // Exact-area matching: a path *inside* a declared area is not itself a
        // declared area, so a caller cannot pass an issue's own directory where
        // an area is expected. Not vacuous — prefix containment, which the
        // archival classifier applies to its own path lists, does accept it.
        let inside = format!("{declared}/abcd1234-example");
        assert!(!authored.is_issue_scoped_area(&inside));
        assert!(contains_path(&declared, &inside));

        // A sibling whose name merely starts with a declared area's name is not
        // that area either.
        assert!(!authored.is_issue_scoped_area(&format!("{declared}-other")));
    }

    #[test]
    fn test_is_issue_scoped_area_follows_the_registry_the_repository_authored() {
        // A registry is exactly what a repository wrote: the engine supplies no
        // area of its own to extend or be replaced
        // (`@/invariant/domain-agnostic`).
        let first_area = "workspace/notes".to_string();
        let authored = DocumentationConfig {
            issue_scoped_areas: Some(vec![first_area.clone()]),
            ..DocumentationConfig::default()
        };
        assert_eq!(authored.issue_scoped_areas(), vec![first_area.clone()]);
        assert!(authored.is_issue_scoped_area(&first_area));

        // Reauthoring the registry replaces it rather than extending it, so an
        // area the new declaration omits stops being one.
        let second_area = "workspace/drafts".to_string();
        let reauthored = DocumentationConfig {
            issue_scoped_areas: Some(vec![second_area.clone()]),
            ..DocumentationConfig::default()
        };
        assert!(reauthored.is_issue_scoped_area(&second_area));
        assert!(!reauthored.is_issue_scoped_area(&first_area));

        // An authored empty registry and an absent key both declare that no
        // area adopts the convention.
        let opted_out = DocumentationConfig {
            issue_scoped_areas: Some(Vec::new()),
            ..DocumentationConfig::default()
        };
        assert!(opted_out.issue_scoped_areas().is_empty());
        assert!(!opted_out.is_issue_scoped_area(&first_area));
        assert!(!DocumentationConfig::default().is_issue_scoped_area(&first_area));
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

        let classified = DocumentationConfig {
            development_root: Some("dev".to_string()),
            permanent_paths: Some(vec!["dev/architecture".to_string()]),
            ..DocumentationConfig::default()
        };
        assert_eq!(
            resolved_universe(&classified),
            derived_universe(&classified)
        );
        assert!(!resolved_universe(&classified).is_empty());

        // Reclassifying the table moves the default with it: a repository that
        // renames its development root and declares its own permanent areas
        // scans those, not the ones it replaced.
        let reclassified = DocumentationConfig {
            development_root: Some("workspace".to_string()),
            permanent_paths: Some(vec![
                "workspace/guides".to_string(),
                "README.md".to_string(),
            ]),
            ..DocumentationConfig::default()
        };
        assert_eq!(
            resolved_universe(&reclassified),
            derived_universe(&reclassified)
        );
        assert!(resolved_universe(&reclassified).is_disjoint(&resolved_universe(&classified)));
    }

    #[test]
    fn test_citation_scan_roots_replace_the_default_with_an_authored_universe_reaching_outside_the_development_root(
    ) {
        use crate::domain::artifact_classifier::contains_path;

        // A classified table, so the derived default universe this one replaces
        // reaches something and the replacement is not vacuous.
        let classified = DocumentationConfig {
            development_root: Some("dev".to_string()),
            permanent_paths: Some(vec!["dev/architecture".to_string()]),
            ..DocumentationConfig::default()
        };
        let development_root = classified.development_root();

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
            ..classified.clone()
        };
        let resolved = authored.citation_scan_roots();
        assert!(resolved.contains(&outside_directory) && resolved.contains(&outside_file));

        // The authored list is the whole universe: it replaces the default
        // instead of extending it, so a default root it omits is gone.
        let omitted = classified
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

        assert_eq!(
            PolicyStatus::from_documentation(Some(&DocumentationConfig::default())),
            PolicyStatus::Incomplete
        );

        // Not vacuous: a table authoring the keys outside the required three is
        // still Incomplete, so the status follows which fields were authored
        // rather than how much the table says.
        let partly_authored = DocumentationConfig {
            development_root: Some("dev".to_string()),
            issue_scoped_areas: Some(vec!["dev/active".to_string()]),
            citation_scan_roots: Some(vec!["dev".to_string()]),
            ..DocumentationConfig::default()
        };
        assert_eq!(
            PolicyStatus::from_documentation(Some(&partly_authored)),
            PolicyStatus::Incomplete
        );
        assert!(!partly_authored.development_root().is_empty());
    }

    #[test]
    fn test_load_invalid_invariant_fails_config_load_both_paths() {
        // A malformed invariant entry fails assembly with an error naming its
        // file, whether or not config.toml exists.
        let bad = "[[invariants]]\nid = \"sample-invariant\"\nkind = \"advisory\"\n"; // missing statement
        [None, Some("[type_hierarchy]\ntypes = { task = 1 }\n")]
            .into_iter()
            .for_each(|config| {
                let err = assemble(config, None, Some(bad)).unwrap_err();
                assert!(
                    err.to_string()
                        .contains(VirtualPath::INVARIANTS.relative().as_str()),
                    "{err}"
                );
            });
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
        assert_eq!(
            validation.strictness,
            Some(crate::validation::Strictness::Loose)
        );
        assert_eq!(validation.default_type, Some("task".to_string()));
    }

    #[test]
    fn test_content_format_resolves_absent_table_absent_key_and_declared_value() {
        use crate::domain::ContentFormat::{Html, Markdown};

        [
            ("", Markdown),
            ("[validation]", Markdown),
            ("[validation]\ncontent_format = \"html\"", Html),
        ]
        .into_iter()
        .for_each(|(source, expected)| {
            let config: JitConfig = toml::from_str(source).unwrap();
            assert_eq!(config.content_format(), expected, "{source:?}");
        });
    }

    #[test]
    fn test_strictness_resolves_absent_table_absent_key_and_declared_value() {
        use crate::validation::Strictness::{Loose, Strict};

        [
            ("", Loose),
            ("[validation]", Loose),
            ("[validation]\nstrictness = \"strict\"", Strict),
        ]
        .into_iter()
        .for_each(|(source, expected)| {
            let config: JitConfig = toml::from_str(source).unwrap();
            assert_eq!(config.strictness(), expected, "{source:?}");
        });
    }

    #[test]
    fn test_each_strictness_level_deserializes() {
        for expected in crate::validation::Strictness::ALL {
            let level = expected.as_str();
            let toml = format!("[validation]\nstrictness = \"{level}\"\n");
            let config: JitConfig =
                toml::from_str(&toml).unwrap_or_else(|e| panic!("'{level}' must deserialize: {e}"));
            assert_eq!(config.validation.unwrap().strictness, Some(expected));
        }
    }

    #[test]
    fn test_invalid_strictness_is_rejected_at_deserialize() {
        for token in ["banana", "STRICT"] {
            let config_toml = format!("[validation]\nstrictness = \"{token}\"\n");
            let err = toml::from_str::<JitConfig>(&config_toml).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains(token), "names the bad value: {msg}");
            assert!(msg.contains("strictness"), "names the key: {msg}");
            assert!(
                crate::validation::Strictness::ALL
                    .iter()
                    .all(|level| msg.contains(level.as_str())),
                "lists the accepted values: {msg}"
            );
        }
    }

    #[test]
    fn test_invalid_strictness_fails_configuration_assembly() {
        // The eager rejection also fires through assembly, so a committed
        // config.toml with a bad strictness never resolves silently.
        let chain = config_error("[validation]\nstrictness = \"banana\"\n");
        assert!(chain.contains("banana"), "{chain}");
        assert!(chain.contains("strictness"), "{chain}");
    }

    /// Every table of `config.toml` rejects an undeclared key, named in the
    /// assembly error's top-level message.
    #[test]
    fn test_load_rejects_unknown_key_by_name_in_every_table() {
        let key = "undeclared_key";
        for fragment in [
            format!("{key} = 1\n"),
            format!("[version]\n{key} = 1\n"),
            format!("[project]\n{key} = 1\n"),
            format!("[type_hierarchy]\n{key} = 1\n"),
            format!("[type_hierarchy.icons]\n{key} = 1\n"),
            format!("[validation]\n{key} = 1\n"),
            format!("[documentation]\n{key} = 1\n"),
            format!("[namespaces.ns]\n{key} = 1\n"),
            // `kind` is a custom kind; the default package does not ship it.
            format!("[item_kinds.kind]\n{key} = 1\n"),
            format!("[item_kinds.kind]\nsource = {{ {key} = 1 }}\n"),
            format!("[projection.view]\n{key} = 1\n"),
            format!("[worktree]\n{key} = 1\n"),
            format!("[coordination]\n{key} = 1\n"),
        ] {
            let message = assemble(Some(&fragment), None, None)
                .expect_err("an unknown key must fail to assemble")
                .to_string();
            assert!(
                message.contains(&format!("unknown field `{key}`")),
                "{fragment}: {message}"
            );
        }
    }

    #[test]
    fn test_load_existing_config() {
        let config = assemble(
            Some("[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.type_hierarchy.unwrap().types.len(), 2);
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
        assert_eq!(
            validation.strictness,
            Some(crate::validation::Strictness::Loose)
        );
        assert_eq!(
            validation.content_format,
            Some(crate::domain::ContentFormat::Html)
        );
    }

    #[test]
    fn test_parse_validation_rejects_noncanonical_content_format_tokens() {
        ["md", "HTML", "unknown-format"]
            .into_iter()
            .for_each(|value| {
                let config_toml = format!("[validation]\ncontent_format = \"{value}\"\n");
                let error = toml::from_str::<JitConfig>(&config_toml).unwrap_err();

                assert!(
                    error.to_string().contains(value),
                    "the error must name rejected token {value}: {error}"
                );
            });
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
[item_kinds.custom-requirement]
section = "success_criteria"
id-pattern = "REQ-\\d+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"

[item_kinds.custom-decision]
section = "decisions"
id-pattern = "DEC-\\d+"
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

        let req = &kinds["custom-requirement"];
        assert_eq!(req.section.as_deref(), Some("success_criteria"));
        assert_eq!(req.id_pattern.as_deref(), Some("REQ-\\d+"));
        assert_eq!(req.markers, Some(vec!["[hard]".to_string()]));
        assert_eq!(req.link_namespaces, Some(vec!["satisfies".to_string()]));
        assert_eq!(req.scope, Some(KindScope::Issue));
        assert_eq!(req.source_of_truth, Some(SourceOfTruth::MarkdownFirst));
        assert!(req.missing_required_fields().is_empty());

        // An empty `markers = []` is a PRESENT value (not missing).
        let decision = &kinds["custom-decision"];
        assert_eq!(decision.markers, Some(vec![]));
        assert!(decision.missing_required_fields().is_empty());
    }

    #[test]
    fn test_item_kinds_explicit_partial_declaration_is_rejected() {
        assert_eq!(
            ItemKindConfig::default().missing_required_fields(),
            REQUIRED_ITEM_KIND_FIELDS
        );
        // REQ-01: an explicitly-declared kind missing any of the six required
        // fields is rejected by validation, naming the kind and the missing keys.
        let config_toml = r#"
[item_kinds.custom-requirement]
section = "success_criteria"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let err = config
            .validate_item_kinds()
            .expect_err("partial declaration must be rejected");
        match err {
            ItemKindConfigError::MissingFields { kind, missing } => {
                assert_eq!(kind, "custom-requirement");
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
        // The required-six rule fires through configuration assembly.
        let msg = config_error(
            "[item_kinds.custom-decision]\nsection = \"decisions\"\nid-pattern = \"DEC-\\\\d+\"\n",
        );
        assert!(
            msg.contains("custom-decision"),
            "error names the kind: {msg}"
        );
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
        let config_toml = format!("[item_kinds.custom-invariant]\n{body}aliases = [\"cinv\"]\n");
        let config: JitConfig = toml::from_str(&config_toml).unwrap();
        config
            .validate_item_kinds()
            .expect("non-colliding alias validates");
        let kinds = config.item_kinds.unwrap();
        assert_eq!(
            kinds["custom-invariant"].aliases.as_deref(),
            Some(["cinv".to_string()].as_slice())
        );
    }

    #[test]
    fn test_item_kinds_alias_colliding_with_kind_name_is_rejected() {
        // REQ-02: an alias equal to a declared kind NAME is rejected — aliases
        // share the kind-name namespace.
        let body = complete_kind_body();
        let config_toml = format!(
            "[item_kinds.custom-requirement]\n{body}\n[item_kinds.custom-decision]\n{body}aliases = [\"custom-requirement\"]\n"
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
                assert_eq!(kind, "custom-decision");
                assert_eq!(alias, "custom-requirement");
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
            "[item_kinds.custom-requirement]\n{body}aliases = [\"rq\"]\n[item_kinds.custom-decision]\n{body}aliases = [\"rq\"]\n"
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
        // The alias-collision guard fires through configuration assembly, the
        // same path that rejects a partial `[item_kinds]` declaration.
        let body = complete_kind_body();
        let msg = config_error(&format!(
            "[item_kinds.custom-requirement]\n{body}aliases = [\"custom-requirement\"]\n"
        ));
        assert!(
            msg.contains("custom-requirement"),
            "error names the token: {msg}"
        );
    }

    #[test]
    fn test_item_kinds_source_of_truth_resolves_both_directions() {
        // REQ-01: both typed directions parse from their kebab-case tokens and are
        // DISTINCT from the `source` file path field. Each kind declares all six.
        let config_toml = format!(
            r#"
[item_kinds.custom-requirement]
section = "success_criteria"
id-pattern = "REQ-\\d+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "{issue}"
source-of-truth = "markdown-first"

# `policy` is a custom kind; the default package does not ship it.
[item_kinds.policy]
section = "policies"
id-pattern = "POL-\\d+"
markers = []
link-namespaces = ["upholds"]
scope = "{project}"
source = "policies.toml"
source-of-truth = "registry-first"
"#,
            issue = KindScope::Issue,
            project = KindScope::Project,
        );
        // Uses a custom `policy` kind for the registry-first example: the
        // direction (`source-of-truth`) is independent of the `source` field.
        let config: JitConfig = toml::from_str(&config_toml).unwrap();
        config
            .validate_item_kinds()
            .expect("complete kinds validate");
        let kinds = config.item_kinds.unwrap();

        let req = &kinds["custom-requirement"];
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
[item_kinds.custom-requirement]
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
        assert!(config.validate_item_kinds().is_ok());
    }

    #[test]
    fn test_item_kind_scope_parses_both_tokens() {
        // The typed scope field deserializes both valid tokens.
        let issue: ItemKindConfig = toml::from_str("scope = \"issue\"").unwrap();
        assert_eq!(issue.scope, Some(KindScope::Issue));

        let project: ItemKindConfig = toml::from_str("scope = \"project\"").unwrap();
        assert_eq!(project.scope, Some(KindScope::Project));
    }

    #[test]
    fn test_item_kind_scope_invalid_token_is_parse_error() {
        // REQ: an unrecognised scope token is a TOML parse/deserialize error,
        // not a silent fallback to issue scope.
        // `thing` is a custom kind; the default package does not ship it.
        let err =
            toml::from_str::<JitConfig>("[item_kinds.thing]\nscope = \"global\"\n").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("global"),
            "error must mention the bad token: {msg}"
        );
    }

    #[test]
    fn test_enforcement_mode_from_str_case_insensitive() {
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
        assert_eq!(
            validation.strictness,
            Some(crate::validation::Strictness::Loose)
        );

        // Namespaces
        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 2);
        assert!(namespaces["type"].unique);
        assert!(!namespaces["epic"].unique);
    }

    #[test]
    fn test_enforcement_mode_off_without_worktree_section() {
        let config: JitConfig = toml::from_str("").unwrap();
        assert_eq!(config.enforcement_mode(), EnforcementMode::Off);
    }

    #[test]
    fn test_enforcement_mode_default_to_strict() {
        let config_toml = r#"
[worktree]
# No enforce_leases specified
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.enforcement_mode(), EnforcementMode::Strict);
    }

    #[test]
    fn test_enforcement_mode_explicit_strict() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.enforcement_mode(), EnforcementMode::Strict);
    }

    #[test]
    fn test_enforcement_mode_warn() {
        let config_toml = r#"
[worktree]
enforce_leases = "warn"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.enforcement_mode(), EnforcementMode::Warn);
    }

    #[test]
    fn test_enforcement_mode_off() {
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.enforcement_mode(), EnforcementMode::Off);
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
    fn test_worktree_write_policy_default_to_refuse() {
        let config_toml = r#"
[worktree]
# No write_policy specified
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.worktree.unwrap().write_policy(),
            crate::domain::LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_worktree_write_policy_explicit_refuse() {
        let config_toml = r#"
[worktree]
write_policy = "refuse"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.worktree.unwrap().write_policy(),
            crate::domain::LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_worktree_write_policy_allow() {
        let config_toml = r#"
[worktree]
write_policy = "allow"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.worktree.unwrap().write_policy(),
            crate::domain::LinkedCheckoutWriteStance::Allow
        );
    }

    #[test]
    fn test_worktree_write_policy_invalid_token_names_accepted_values() {
        let config_toml = r#"
[worktree]
write_policy = "sometimes"
"#;
        let err = toml::from_str::<JitConfig>(config_toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("sometimes"),
            "error must mention bad value: {msg}"
        );
        assert!(
            msg.contains("refuse"),
            "error must name accepted value: {msg}"
        );
        assert!(
            msg.contains("allow"),
            "error must name accepted value: {msg}"
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
    fn test_coordination_config_full() {
        let config_toml = r#"
[coordination]
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let coord = config.coordination.unwrap();
        assert_eq!(coord.max_indefinite_leases_per_agent, Some(2));
        assert_eq!(coord.max_indefinite_leases_per_repo, Some(10));
    }

    #[test]
    fn test_full_parallel_work_config() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"

[coordination]
max_indefinite_leases_per_agent = 3
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // All sections present
        assert!(config.worktree.is_some());
        assert!(config.coordination.is_some());

        assert_eq!(config.enforcement_mode(), EnforcementMode::Strict);

        // Verify coordination
        let coord = config.coordination.unwrap();
        assert_eq!(coord.max_indefinite_leases_per_agent, Some(3));
    }

    #[test]
    fn test_enforcement_mode_reported_matches_enforced_in_every_configuration_state() {
        const NO_SECTION: &str = "";
        const SECTION_WITHOUT_KEY: &str = "[worktree]\n";
        let explicit = |mode: &str| format!("[worktree]\nenforce_leases = \"{mode}\"\n");
        let states = [NO_SECTION.to_string(), SECTION_WITHOUT_KEY.to_string()]
            .into_iter()
            .chain(EnforcementMode::ALL.map(|mode| explicit(mode.as_str())))
            .collect::<Vec<_>>();
        for repo in &states {
            let config = assemble(Some(repo), None, None).unwrap();
            let enforced = config.enforcement_mode();
            let reported = EffectiveConfig::new(config).enforcement_mode();
            assert_eq!(reported, enforced, "repo {repo:?}");
        }
    }

    #[test]
    fn test_coordination_reported_matches_enforced_in_every_configuration_state() {
        let both = |agent: Option<u32>, repo: Option<u32>| {
            let line = |key: &str, value: Option<u32>| {
                value.map_or(String::new(), |v| format!("{key} = {v}\n"))
            };
            format!(
                "[coordination]\n{}{}",
                line("max_indefinite_leases_per_agent", agent),
                line("max_indefinite_leases_per_repo", repo)
            )
        };
        let states = [
            (String::new(), None, None),
            ("[coordination]\n".to_string(), None, None),
            (both(Some(5), None), Some(5), None),
            (both(None, Some(40)), None, Some(40)),
            (both(Some(3), Some(12)), Some(3), Some(12)),
        ];
        for (repo, agent, repo_limit) in states {
            let config = assemble(Some(&repo), None, None).unwrap();
            let enforced = config.coordination();
            let reported = EffectiveConfig::new(config).coordination();
            assert_eq!(reported, enforced, "repo {repo:?}");
            assert_eq!(
                enforced,
                EffectiveCoordination {
                    max_indefinite_leases_per_agent: agent
                        .unwrap_or(MAX_INDEFINITE_LEASES_PER_AGENT),
                    max_indefinite_leases_per_repo: repo_limit
                        .unwrap_or(MAX_INDEFINITE_LEASES_PER_REPO),
                },
                "repo {repo:?}"
            );
        }
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
    fn test_assemble_configuration_parses_valid_project_table() {
        let config = assemble(Some("[project]\nname = \"just-in-time\"\n"), None, None).unwrap();
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
    fn test_assemble_configuration_rejects_invalid_project_name() {
        let msg = config_error("[project]\nname = \"Bad_Name\"\n");
        assert!(
            msg.contains("Bad_Name"),
            "error must name the offending value: {msg}"
        );
    }
}
