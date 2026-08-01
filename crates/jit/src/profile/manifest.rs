use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Component, Path};

use crate::repository_state::Contribution;

/// The only manifest filename recognized at the root of an embedded package.
pub const MANIFEST_FILE_NAME: &str = "manifest.toml";

/// Version of the immutable v1 profile-manifest wire contract.
pub const PROFILE_MANIFEST_VERSION: u32 = 1;

/// Stable lowercase-kebab identifier used to name a profile package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct ProfileId(String);

impl ProfileId {
    /// Borrow the canonical package identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ProfileId {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_lowercase_kebab(value) {
            Ok(Self(value.to_string()))
        } else {
            Err(format!(
                "invalid profile id '{value}'; expected lowercase-kebab"
            ))
        }
    }
}

impl TryFrom<String> for ProfileId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for ProfileId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// A complete declarative profile manifest.
///
/// Unknown fields are rejected so hook, lifecycle, and future composition
/// syntax cannot silently enter the v1 contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileManifest {
    /// Stable package identity, version, and compatible JIT range.
    pub profile: ProfileMetadata,
    /// Ordered package identifiers whose contributions this package extends.
    #[serde(default)]
    pub dependencies: Vec<ProfileId>,
    /// Ordered semantic contributions. Order is part of package identity.
    #[serde(default, rename = "contribution")]
    pub contributions: Vec<Contribution>,
    /// Ordered one-to-one file assets.
    #[serde(default, rename = "asset")]
    pub assets: Vec<AssetDeclaration>,
    /// Ordered managed-region source declarations.
    #[serde(default, rename = "region")]
    pub regions: Vec<RegionDeclaration>,
    /// Repository roots the package's live assets are drawn from.
    #[serde(default, rename = "live-source")]
    pub live_sources: Vec<LiveSourceDeclaration>,
}

/// Stable package metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ProfileMetadata {
    /// Manifest wire-version discriminator.
    pub manifest_version: u32,
    /// Stable lowercase-kebab package identifier.
    pub id: ProfileId,
    /// Semantic package version.
    pub version: String,
    /// Semantic JIT version requirement.
    pub jit: String,
}

/// A one-to-one embedded file asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetDeclaration {
    /// Package-relative source path.
    pub source: String,
    /// Repository-relative destination path.
    pub target: String,
    /// Whether Unix application should publish the executable bit.
    #[serde(default)]
    pub executable: bool,
}

/// A managed region sourced from one embedded file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RegionDeclaration {
    /// Package-relative source path.
    pub source: String,
    /// Repository-relative destination path.
    pub target: String,
    /// Stable marker identity.
    pub region_id: String,
    /// V1 placement policy.
    pub placement: RegionPlacement,
}

/// One repository root the package's live assets are drawn from.
///
/// A live asset is a package file whose bytes come from a repository file that
/// this package also consumes at its working path. Declaring the roots states
/// the domain those files are drawn from, so a consumer asking which
/// directories a package draws from reads it rather than inferring it from the
/// asset inventory.
///
/// [`exclude`](Self::exclude) bounds the root: repository material beneath it
/// that the package deliberately does not carry. An empty list claims the whole
/// root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveSourceDeclaration {
    /// Repository-relative directory the live assets beneath it are drawn from.
    pub root: LiveSourceRoot,
    /// Patterns matching root-relative paths the package does not carry.
    #[serde(default)]
    pub exclude: Vec<ExclusionPattern>,
}

impl LiveSourceDeclaration {
    /// Whether any declared pattern matches `root_relative`.
    ///
    /// `root_relative` is a path beneath [`Self::root`], as
    /// [`LiveSourceRoot::relative_path`] returns it.
    pub fn excludes(&self, root_relative: &str) -> bool {
        self.exclude
            .iter()
            .any(|pattern| pattern.matches(root_relative))
    }
}

/// A repository-relative directory a package draws live assets from.
///
/// Parsing rejects anything that is not a relative path of ordinary segments,
/// so a consumer receives a root it can join to a repository worktree without
/// re-checking it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct LiveSourceRoot(String);

impl LiveSourceRoot {
    /// Borrow the repository-relative root path.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The root-relative remainder of `repository_path`, or `None` when the
    /// path does not lie beneath this root.
    ///
    /// The root itself yields `None`: a root is the domain, never a member of
    /// it. A sibling whose name merely starts with the root's name yields
    /// `None` too, since the separator is required.
    pub fn relative_path<'a>(&self, repository_path: &'a str) -> Option<&'a str> {
        repository_path
            .strip_prefix(self.as_str())
            .and_then(|remainder| remainder.strip_prefix('/'))
            .filter(|remainder| !remainder.is_empty())
    }
}

impl TryFrom<&str> for LiveSourceRoot {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_safe_relative_path(value) {
            Ok(Self(value.to_string()))
        } else {
            Err(format!(
                "invalid live-source root '{value}'; expected a relative repository path"
            ))
        }
    }
}

impl TryFrom<String> for LiveSourceRoot {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for LiveSourceRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for LiveSourceRoot {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for LiveSourceRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// Match options fixing what an exclusion pattern's wildcards mean.
const EXCLUSION_MATCH_OPTIONS: glob::MatchOptions = glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

/// A shell-style pattern naming root-relative paths a package does not carry.
///
/// The pattern is compiled where the manifest is parsed, so an uncompilable
/// pattern is a manifest error rather than a surprise for whichever consumer
/// matches first, and every consumer matches through the same compiled form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionPattern(glob::Pattern);

impl ExclusionPattern {
    /// Borrow the authored pattern text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this pattern matches the root-relative path `root_relative`.
    ///
    /// `*` stays inside one path segment and `**` spans segments, which is what
    /// lets a pattern describe a category — a directory anywhere beneath the
    /// root, or a filename shape within one directory — rather than a path
    /// literal. Matching is case-sensitive, and a leading dot is an ordinary
    /// character, so a pattern reaches dot-directories without a spelling of
    /// its own.
    ///
    /// The match is whole-path: `evals/**` matches every path beneath an
    /// `evals` directory, while `evals` alone matches only that exact path.
    pub fn matches(&self, root_relative: &str) -> bool {
        self.0.matches_with(root_relative, EXCLUSION_MATCH_OPTIONS)
    }
}

/// Published as the authored pattern text, the only form the wire carries.
impl JsonSchema for ExclusionPattern {
    fn is_referenceable() -> bool {
        false
    }

    fn schema_name() -> String {
        String::schema_name()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        String::json_schema(generator)
    }
}

impl TryFrom<&str> for ExclusionPattern {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if !is_relative_pattern_shape(value) {
            return Err(format!(
                "invalid exclusion pattern '{value}'; expected a relative path pattern"
            ));
        }
        glob::Pattern::new(value)
            .map(Self)
            .map_err(|error| format!("invalid exclusion pattern '{value}': {error}"))
    }
}

impl TryFrom<String> for ExclusionPattern {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for ExclusionPattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(formatter)
    }
}

impl Serialize for ExclusionPattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExclusionPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// Supported managed-region placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RegionPlacement {
    /// Append a managed region using the deterministic bootstrap rules.
    Append,
}

/// Generate the JSON Schema from the same runtime type used to parse manifests.
pub fn profile_manifest_schema() -> RootSchema {
    schema_for!(ProfileManifest)
}

/// Whether `path` is a relative path of ordinary segments, safe to join to a
/// repository or package root on any platform.
///
/// One rule for every manifest path: the sources and targets the package model
/// validates, and the live-source roots parsing validates
/// (`@/invariant/convention-convergence`).
pub(crate) fn is_safe_relative_path(path: &str) -> bool {
    let parsed = Path::new(path);
    let has_windows_prefix = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && !has_windows_prefix
        && has_ordinary_segments(path)
        && !parsed.is_absolute()
        && parsed.components().all(|component| match component {
            Component::Normal(value) => value != "." && value != "..",
            _ => false,
        })
}

/// Whether `value` splits on `/` into non-empty segments that are neither `.`
/// nor `..`, which rejects a leading or trailing separator and a doubled one.
fn has_ordinary_segments(value: &str) -> bool {
    value
        .split('/')
        .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}

/// Whether `value` has the shape of a relative path pattern, before any attempt
/// to compile it.
///
/// A pattern names root-relative paths, so the segment rules a path is held to
/// apply to it. Wildcards are left to the compiler: `*`, `?` and bracket
/// expressions are ordinary segment content, and `**` forms a segment of its
/// own. The path predicate is not reused, because its rejection of `:` would
/// reject a bracket expression that merely contains one.
fn is_relative_pattern_shape(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && has_ordinary_segments(value)
}

pub(crate) fn is_lowercase_kebab(value: &str) -> bool {
    let mut segments = value.split('-');
    segments.next().is_some_and(|first| {
        !first.is_empty()
            && first.starts_with(|character: char| character.is_ascii_lowercase())
            && first
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    }) && segments.all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One manifest whose metadata is fixed and whose body is under test.
    fn manifest_of(body: &str) -> Result<ProfileManifest, toml::de::Error> {
        toml::from_str(&format!(
            "[profile]\n\
             manifest-version = 1\n\
             id = \"example\"\n\
             version = \"1.0.0\"\n\
             jit = \">=1.0.0\"\n\
             {body}"
        ))
    }

    /// The parse failure `body` produces, or a panic naming what parsed.
    fn rejection_of(body: &str) -> String {
        match manifest_of(body) {
            Err(error) => error.to_string(),
            Ok(manifest) => panic!("expected a parse failure, got {manifest:?}"),
        }
    }

    #[test]
    fn test_profile_manifest_deserialize_declares_one_entry_per_root_with_its_own_exclusions() {
        let manifest = manifest_of(
            "[[live-source]]\n\
             root = \".agents/skills\"\n\
             exclude = [\"*/evals/**\", \"*/references/fixtures/**\"]\n\
             \n\
             [[live-source]]\n\
             root = \"contrib/gates\"\n\
             exclude = []\n",
        )
        .expect("a manifest declaring two roots parses");

        let roots: Vec<&str> = manifest
            .live_sources
            .iter()
            .map(|declaration| declaration.root.as_str())
            .collect();
        assert_eq!(roots, [".agents/skills", "contrib/gates"]);

        let skills = &manifest.live_sources[0];
        let gates = &manifest.live_sources[1];
        assert!(skills.excludes("jit-manage/evals/case.md"));
        assert!(!skills.excludes("jit-manage/SKILL.md"));
        // An empty list bounds nothing, so the root claims everything beneath it.
        assert!(gates.exclude.is_empty());
        assert!(!gates.excludes("ai-review.sh"));
    }

    #[test]
    fn test_profile_manifest_deserialize_accepts_a_manifest_declaring_no_live_source() {
        let manifest = manifest_of(
            "[[asset]]\n\
             source = \"assets/live/bin/check.sh\"\n\
             target = \"bin/check.sh\"\n\
             executable = true\n\
             \n\
             [[region]]\n\
             source = \"assets/regions/guidance.md\"\n\
             target = \"AGENTS.md\"\n\
             region-id = \"guidance\"\n\
             placement = \"append\"\n",
        )
        .expect("a manifest without a live-source declaration parses");

        assert!(manifest.live_sources.is_empty());
        // The keys that were already there still mean what they meant.
        assert_eq!(manifest.assets[0].target, "bin/check.sh");
        assert!(manifest.assets[0].executable);
        assert_eq!(manifest.regions[0].region_id, "guidance");
        assert_eq!(manifest.regions[0].placement, RegionPlacement::Append);
    }

    #[test]
    fn test_profile_manifest_deserialize_rejects_a_root_that_is_not_a_relative_repository_path() {
        for root in [
            "/absolute",
            "../escape",
            "skills/../escape",
            "double//separator",
            "trailing/",
            "/",
            "",
            "back\\slash",
            "C:/drive",
        ] {
            // A TOML literal string, so the authored bytes reach the parser
            // unescaped and a backslash is a path character rather than an
            // escape.
            let rejection = rejection_of(&format!("[[live-source]]\nroot = '{root}'\n"));
            assert!(
                rejection.contains(root) && rejection.contains("live-source root"),
                "'{root}' was rejected without naming it as a root: {rejection}"
            );
        }
    }

    #[test]
    fn test_profile_manifest_deserialize_rejects_an_exclusion_pattern_that_does_not_compile() {
        for pattern in ["***", "a**b", "**b/x", "[a-"] {
            let rejection = rejection_of(&format!(
                "[[live-source]]\nroot = 'skills'\nexclude = ['{pattern}']\n"
            ));
            assert!(
                rejection.contains(pattern) && rejection.contains("exclusion pattern"),
                "'{pattern}' was rejected without naming it as a pattern: {rejection}"
            );
        }
    }

    #[test]
    fn test_profile_manifest_deserialize_rejects_an_exclusion_pattern_that_is_not_root_relative() {
        for pattern in [
            "/absolute/**",
            "../escape/**",
            "double//separator",
            "back\\slash",
            "",
        ] {
            let rejection = rejection_of(&format!(
                "[[live-source]]\nroot = 'skills'\nexclude = ['{pattern}']\n"
            ));
            assert!(
                rejection.contains("relative path pattern"),
                "'{pattern}' was not rejected as a relative pattern: {rejection}"
            );
        }
    }

    #[test]
    fn test_exclusion_pattern_matches_across_segments_only_through_the_recursive_wildcard() {
        let within = ExclusionPattern::try_from("*.md").expect("a compilable pattern");
        assert!(within.matches("notes.md"));
        assert!(!within.matches("references/notes.md"));

        let across = ExclusionPattern::try_from("**/*.md").expect("a compilable pattern");
        assert!(across.matches("references/notes.md"));
        assert!(across.matches("a/b/c/notes.md"));
        assert!(!across.matches("references/notes.txt"));

        // A whole-path match, so a category is named by spanning what follows it.
        let category = ExclusionPattern::try_from("*/evals/**").expect("a compilable pattern");
        assert!(category.matches("jit-manage/evals/case.md"));
        assert!(category.matches("jit-manage/evals/fixtures/input.json"));
        assert!(!category.matches("jit-manage/evals"));
    }

    #[test]
    fn test_exclusion_pattern_matches_a_dot_prefixed_segment_without_spelling_the_dot() {
        let pattern = ExclusionPattern::try_from("**/*").expect("a compilable pattern");
        assert!(pattern.matches(".hidden/notes.md"));
        assert!(pattern.matches("visible/.gitignore"));
    }

    #[test]
    fn test_live_source_root_relative_path_yields_a_remainder_only_beneath_the_root() {
        let root = LiveSourceRoot::try_from(".agents/skills").expect("a relative root");

        assert_eq!(
            root.relative_path(".agents/skills/jit-manage/SKILL.md"),
            Some("jit-manage/SKILL.md")
        );
        // The root is the domain, not a member of it.
        assert_eq!(root.relative_path(".agents/skills"), None);
        // A sibling sharing the root's name prefix is outside it.
        assert_eq!(root.relative_path(".agents/skillsets/other.md"), None);
        assert_eq!(root.relative_path("contrib/gates/ai-review.sh"), None);
    }

    #[test]
    fn test_live_source_declaration_excludes_a_path_matched_by_any_declared_pattern() {
        let declaration = LiveSourceDeclaration {
            root: LiveSourceRoot::try_from("skills").expect("a relative root"),
            exclude: ["**/evals/**", "**/trigger-*.md"]
                .into_iter()
                .map(|pattern| ExclusionPattern::try_from(pattern).expect("a compilable pattern"))
                .collect(),
        };

        assert!(declaration.excludes("jit-manage/evals/case.md"));
        assert!(declaration.excludes("jit-manage/references/trigger-log.md"));
        assert!(!declaration.excludes("jit-manage/references/plan-schema.md"));
    }

    #[test]
    fn test_profile_manifest_serialize_carries_every_declared_root_and_pattern() {
        let manifest = manifest_of(
            "[[live-source]]\n\
             root = \".agents/skills\"\n\
             exclude = [\"*/evals/**\"]\n",
        )
        .expect("a manifest declaring one root parses");

        let wire = serde_json::to_value(&manifest).expect("the manifest serializes");
        assert_eq!(wire["live-source"][0]["root"], ".agents/skills");
        assert_eq!(wire["live-source"][0]["exclude"][0], "*/evals/**");

        // The wire form is the parsed value's own form: reading it back is exact.
        let restored: ProfileManifest =
            serde_json::from_value(wire).expect("the serialized manifest parses");
        assert_eq!(restored, manifest);
    }
}
