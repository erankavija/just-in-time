//! Declared repository input sets, and the content digest taken over one.
//!
//! A declaration names roots and the categories beneath them it does not
//! claim: "these paths, minus these patterns". Two consumers state their
//! inputs this way and both read them through the types here — the production
//! binary's build inputs ([`build_provenance`](crate::domain::build_provenance))
//! and a quality gate's checker inputs
//! ([`GateDefinition::inputs`](crate::declarations::GateDefinition)) — so the
//! covering rule, the pattern dialect, and the digest have one implementation
//! between them (`@/invariant/convention-convergence`).
//!
//! Both constrained values are validated where a declaration is parsed, not
//! where a consumer first matches: an unparseable root or an uncompilable
//! pattern is a declaration error, and every consumer matches through the same
//! compiled form.
//!
//! # Digesting
//!
//! [`InputsDigest`] identifies the *content* of a declared input set. The
//! digest is built by feeding one entry per covered file to
//! [`InputsDigestBuilder`] in path order; the encoding length-prefixes both the
//! path and the bytes, so a rename, an added file, a removed file, and an edit
//! each produce a different value. Modification times, ownership, and ordering
//! of the underlying walk never enter it — a checkout that rewrites timestamps
//! without changing bytes digests to the same value.

use schemars::JsonSchema;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Component, Path};

/// Domain separator opening every [`InputsDigest`] computation.
///
/// Fixes the encoding this digest is taken under, so a value can never be
/// confused with a digest some other mechanism computed over the same bytes.
const DIGEST_DOMAIN: &[u8] = b"jit-repository-inputs-v1\0";

/// A repository-relative path a declaration names as a root.
///
/// A root stands for itself when it names a file, and for everything beneath
/// it when it names a directory — [`covers`](Self::covers) answers that
/// question for a candidate path. Parsing rejects anything that is not a
/// relative path of ordinary segments, so a consumer receives a root it can
/// join to a repository worktree without re-checking it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct DeclaredRoot(String);

impl DeclaredRoot {
    /// Borrow the repository-relative root path.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `repository_path` is this root itself or lies beneath it.
    ///
    /// The separator is required, so a sibling whose name merely starts with
    /// the root's name is not covered.
    pub fn covers(&self, repository_path: &str) -> bool {
        repository_path
            .strip_prefix(self.as_str())
            .is_some_and(|remainder| remainder.is_empty() || remainder.starts_with('/'))
    }

    /// The root-relative remainder of `repository_path`, or `None` when the
    /// path does not lie beneath this root.
    ///
    /// The root itself yields `None`: this asks for membership of a directory,
    /// which the root is the domain of rather than a member of. A caller
    /// asking whether the root's own path is claimed wants
    /// [`covers`](Self::covers).
    pub fn relative_path<'a>(&self, repository_path: &'a str) -> Option<&'a str> {
        repository_path
            .strip_prefix(self.as_str())
            .and_then(|remainder| remainder.strip_prefix('/'))
            .filter(|remainder| !remainder.is_empty())
    }
}

impl TryFrom<&str> for DeclaredRoot {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_safe_relative_path(value) {
            Ok(Self(value.to_string()))
        } else {
            Err(format!(
                "invalid root '{value}'; expected a relative repository path"
            ))
        }
    }
}

impl TryFrom<String> for DeclaredRoot {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for DeclaredRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for DeclaredRoot {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for DeclaredRoot {
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

/// A shell-style pattern naming repository-relative paths a declaration
/// removes from its roots.
///
/// The pattern is compiled where the declaration is parsed, so an uncompilable
/// pattern is a declaration error rather than a surprise for whichever consumer
/// matches first, and every consumer matches through the same compiled form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionPattern(glob::Pattern);

impl ExclusionPattern {
    /// Borrow the authored pattern text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this pattern matches the repository-relative path
    /// `repository_path`.
    ///
    /// `*` stays inside one path segment and `**` spans segments, which is what
    /// lets a pattern describe a category — a directory anywhere in the tree,
    /// or a filename shape within one directory — rather than a path literal.
    /// Matching is case-sensitive, and a leading dot is an ordinary character,
    /// so a pattern reaches dot-directories without a spelling of its own.
    ///
    /// The match is whole-path: `docs/evals/**` matches every path beneath that
    /// `evals` directory, while `docs/evals` alone matches only that exact path.
    pub fn matches(&self, repository_path: &str) -> bool {
        self.0
            .matches_with(repository_path, EXCLUSION_MATCH_OPTIONS)
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

/// A set of repository files named as roots minus excluded categories.
///
/// [`roots`](Self::roots) must name at least one path: a declaration covering
/// nothing would digest every repository state to the same value, so an empty
/// root list is rejected where the declaration is parsed rather than producing
/// a set whose digest means nothing. A consumer that reads no repository file
/// declares no [`RepositoryInputs`] at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RepositoryInputs {
    /// Repository-relative paths this set is drawn from.
    roots: Vec<DeclaredRoot>,
    /// Patterns matching repository-relative paths the set does not contain.
    exclude: Vec<ExclusionPattern>,
}

impl RepositoryInputs {
    /// Build a set from authored root and pattern text.
    ///
    /// # Errors
    ///
    /// Returns a message naming the offending value when `roots` is empty, a
    /// root is not a relative repository path, or a pattern does not compile.
    pub fn parse(roots: &[&str], exclude: &[&str]) -> Result<Self, String> {
        let roots = roots
            .iter()
            .map(|root| DeclaredRoot::try_from(*root))
            .collect::<Result<Vec<_>, _>>()?;
        let exclude = exclude
            .iter()
            .map(|pattern| ExclusionPattern::try_from(*pattern))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(roots, exclude)
    }

    /// Build a set from already-parsed values.
    ///
    /// # Errors
    ///
    /// Returns a message when `roots` is empty.
    pub fn new(roots: Vec<DeclaredRoot>, exclude: Vec<ExclusionPattern>) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("a repository input set must declare at least one root".to_string());
        }
        Ok(Self { roots, exclude })
    }

    /// The declared roots.
    pub fn roots(&self) -> &[DeclaredRoot] {
        &self.roots
    }

    /// The declared exclusion patterns.
    pub fn exclude(&self) -> &[ExclusionPattern] {
        &self.exclude
    }

    /// Whether `repository_path` belongs to this set: some root covers it and
    /// no exclusion matches it.
    pub fn covers(&self, repository_path: &str) -> bool {
        self.roots.iter().any(|root| root.covers(repository_path))
            && !self
                .exclude
                .iter()
                .any(|pattern| pattern.matches(repository_path))
    }
}

impl<'de> Deserialize<'de> for RepositoryInputs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Authored {
            roots: Vec<DeclaredRoot>,
            #[serde(default)]
            exclude: Vec<ExclusionPattern>,
        }

        let authored = Authored::deserialize(deserializer)?;
        Self::new(authored.roots, authored.exclude).map_err(de::Error::custom)
    }
}

/// A content digest over one declared repository input set.
///
/// Two runs that recorded the same value read byte-identical content at
/// byte-identical paths. The value is 64 lowercase hex characters; parsing
/// rejects any other spelling, so a digest read back from a record is a digest
/// this crate could have produced.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(with = "String")]
pub struct InputsDigest(String);

impl InputsDigest {
    /// Borrow the hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for InputsDigest {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() == 64
            && value
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        {
            Ok(Self(value.to_string()))
        } else {
            Err(format!(
                "invalid input digest '{value}'; expected 64 lowercase hex characters"
            ))
        }
    }
}

impl TryFrom<String> for InputsDigest {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for InputsDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for InputsDigest {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for InputsDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// Accumulates one [`InputsDigest`] from the covered files fed to it.
///
/// The caller pushes one entry per covered file, in ascending path order, and
/// the order is part of the value: a walk that enumerates the same files must
/// sort them the same way to reproduce a digest. Streaming one file at a time
/// keeps the whole input set from having to be resident at once.
#[derive(Debug, Clone)]
pub struct InputsDigestBuilder {
    hasher: Sha256,
}

impl InputsDigestBuilder {
    /// Start a digest over an empty set.
    pub fn new() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DIGEST_DOMAIN);
        Self { hasher }
    }

    /// Add one covered file's repository-relative path and working-tree bytes.
    pub fn push_file(&mut self, repository_path: &str, contents: &[u8]) {
        self.hasher
            .update((repository_path.len() as u64).to_le_bytes());
        self.hasher.update(repository_path.as_bytes());
        self.hasher.update((contents.len() as u64).to_le_bytes());
        self.hasher.update(contents);
    }

    /// Finish the digest over everything pushed so far.
    pub fn finish(self) -> InputsDigest {
        InputsDigest(format!("{:x}", self.hasher.finalize()))
    }
}

impl Default for InputsDigestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `path` is a relative path of ordinary segments, safe to join to a
/// repository or package root on any platform.
///
/// One rule for every declared path: the sources and targets the profile
/// package model validates, the live-source roots a manifest declares, and the
/// roots a repository input set declares (`@/invariant/convention-convergence`).
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
/// A pattern names repository-relative paths, so the segment rules a path is
/// held to apply to it. Wildcards are left to the compiler: `*`, `?` and bracket
/// expressions are ordinary segment content, and `**` forms a segment of its
/// own. The path predicate is not reused, because its rejection of `:` would
/// reject a bracket expression that merely contains one.
fn is_relative_pattern_shape(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && has_ordinary_segments(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(roots: &[&str], exclude: &[&str]) -> RepositoryInputs {
        RepositoryInputs::parse(roots, exclude).expect("a declarable input set")
    }

    fn digest_of(entries: &[(&str, &[u8])]) -> InputsDigest {
        entries
            .iter()
            .fold(InputsDigestBuilder::new(), |mut builder, (path, bytes)| {
                builder.push_file(path, bytes);
                builder
            })
            .finish()
    }

    #[test]
    fn test_declared_root_covers_the_root_itself_and_its_descendants() {
        let root = DeclaredRoot::try_from("crates/jit").expect("a relative root");
        assert!(root.covers("crates/jit"));
        assert!(root.covers("crates/jit/src/main.rs"));
        assert!(!root.covers("crates/jit-server/src/main.rs"));
        assert!(!root.covers("crates"));
    }

    #[test]
    fn test_declared_root_relative_path_excludes_the_root_itself() {
        let root = DeclaredRoot::try_from("docs").expect("a relative root");
        assert_eq!(
            root.relative_path("docs/reference/cli.md"),
            Some("reference/cli.md")
        );
        assert_eq!(root.relative_path("docs"), None);
        assert_eq!(root.relative_path("docsite/index.md"), None);
    }

    #[test]
    fn test_declared_root_rejects_paths_that_cannot_be_joined_to_a_worktree() {
        for rejected in [
            "/absolute",
            "..",
            "../escape",
            "./here",
            "trailing/",
            "",
            "a//b",
        ] {
            assert!(
                DeclaredRoot::try_from(rejected).is_err(),
                "{rejected} must not parse as a root"
            );
        }
    }

    #[test]
    fn test_exclusion_pattern_matches_across_segments_only_through_the_recursive_wildcard() {
        let within = ExclusionPattern::try_from("skills/*.md").expect("a compilable pattern");
        assert!(within.matches("skills/notes.md"));
        assert!(!within.matches("skills/references/notes.md"));

        let across = ExclusionPattern::try_from("skills/**/*.md").expect("a compilable pattern");
        assert!(across.matches("skills/references/notes.md"));
        assert!(across.matches("skills/a/b/c/notes.md"));
        assert!(!across.matches("skills/references/notes.txt"));

        // A whole-path match, so a category is named by spanning what follows it.
        let category =
            ExclusionPattern::try_from("skills/*/evals/**").expect("a compilable pattern");
        assert!(category.matches("skills/jit-manage/evals/case.md"));
        assert!(category.matches("skills/jit-manage/evals/fixtures/input.json"));
        assert!(!category.matches("skills/jit-manage/evals"));
    }

    #[test]
    fn test_exclusion_pattern_matches_a_dot_prefixed_segment_without_spelling_the_dot() {
        let pattern = ExclusionPattern::try_from("**/*").expect("a compilable pattern");
        assert!(pattern.matches(".agents/notes.md"));
        assert!(pattern.matches("visible/.gitignore"));
    }

    #[test]
    fn test_repository_inputs_covers_root_members_that_no_exclusion_matches() {
        let declared = inputs(&["crates", "Cargo.toml"], &["crates/*/fixtures/**"]);
        assert!(declared.covers("Cargo.toml"));
        assert!(declared.covers("crates/jit/src/lib.rs"));
        assert!(!declared.covers("crates/jit/fixtures/sample.json"));
        assert!(!declared.covers("docs/index.md"));
    }

    #[test]
    fn test_repository_inputs_rejects_a_declaration_covering_nothing() {
        assert!(RepositoryInputs::parse(&[], &[]).is_err());
        assert!(RepositoryInputs::parse(&["crates"], &["["]).is_err());
        assert!(RepositoryInputs::parse(&["/etc"], &[]).is_err());
    }

    #[test]
    fn test_repository_inputs_deserialize_rejects_invalid_values_at_parse() {
        assert!(toml::from_str::<RepositoryInputs>("roots = [\"crates\"]\n").is_ok());
        assert!(toml::from_str::<RepositoryInputs>("roots = []\n").is_err());
        assert!(toml::from_str::<RepositoryInputs>("roots = [\"../escape\"]\n").is_err());
        assert!(
            toml::from_str::<RepositoryInputs>("roots = [\"crates\"]\nexclude = [\"[\"]\n")
                .is_err()
        );
    }

    #[test]
    fn test_inputs_digest_builder_distinguishes_content_path_and_membership() {
        let baseline = digest_of(&[("a.rs", b"one"), ("b.rs", b"two")]);

        assert_eq!(
            baseline,
            digest_of(&[("a.rs", b"one"), ("b.rs", b"two")]),
            "the same entries must digest to the same value"
        );
        assert_ne!(
            baseline,
            digest_of(&[("a.rs", b"one!"), ("b.rs", b"two")]),
            "changed content must change the digest"
        );
        assert_ne!(
            baseline,
            digest_of(&[("a.rs", b"one"), ("b.rs", b"two"), ("c.rs", b"three")]),
            "an added file must change the digest"
        );
        assert_ne!(
            baseline,
            digest_of(&[("a.rs", b"one")]),
            "a removed file must change the digest"
        );
        assert_ne!(
            baseline,
            digest_of(&[("a.rs", b"one"), ("renamed.rs", b"two")]),
            "a renamed file must change the digest"
        );
    }

    #[test]
    fn test_inputs_digest_builder_separates_path_from_content() {
        assert_ne!(
            digest_of(&[("ab", b"c")]),
            digest_of(&[("a", b"bc")]),
            "length-prefixing must keep a path boundary from sliding into content"
        );
    }

    #[test]
    fn test_inputs_digest_parse_rejects_values_this_crate_cannot_have_produced() {
        let produced = digest_of(&[("a.rs", b"one")]);
        assert!(InputsDigest::try_from(produced.as_str()).is_ok());
        assert!(InputsDigest::try_from("not-a-digest").is_err());
        assert!(InputsDigest::try_from(produced.as_str().to_uppercase().as_str()).is_err());
    }
}
