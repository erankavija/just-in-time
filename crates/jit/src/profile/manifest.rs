use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

use crate::domain::repository_inputs::{DeclaredRoot, ExclusionPattern};
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
    pub root: DeclaredRoot,
    /// Patterns matching repository-relative paths the package does not carry.
    #[serde(default)]
    pub exclude: Vec<ExclusionPattern>,
}

impl LiveSourceDeclaration {
    /// Whether any declared pattern matches `repository_path`.
    ///
    /// `repository_path` is a repository-relative path, the same form
    /// [`Self::root`] and every manifest target take, so a pattern reads as the
    /// repository location it names. Membership of the root is a separate
    /// question, answered by [`DeclaredRoot::relative_path`]: this asks only
    /// whether the declaration's patterns cover the path.
    pub fn excludes(&self, repository_path: &str) -> bool {
        self.exclude
            .iter()
            .any(|pattern| pattern.matches(repository_path))
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
             exclude = [\".agents/skills/*/evals/**\", \".agents/skills/*/references/fixtures/**\"]\n\
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
        assert!(skills.excludes(".agents/skills/jit-manage/evals/case.md"));
        assert!(!skills.excludes(".agents/skills/jit-manage/SKILL.md"));
        // An empty list bounds nothing, so the root claims everything beneath it.
        assert!(gates.exclude.is_empty());
        assert!(!gates.excludes("contrib/gates/ai-review.sh"));
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
                rejection.contains(root) && rejection.contains("invalid root"),
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
    fn test_profile_manifest_deserialize_rejects_an_exclusion_pattern_that_is_not_repository_relative(
    ) {
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
    fn test_live_source_declaration_excludes_a_path_matched_by_any_declared_pattern() {
        let declaration = LiveSourceDeclaration {
            root: DeclaredRoot::try_from("skills").expect("a relative root"),
            exclude: ["skills/**/evals/**", "skills/**/trigger-*.md"]
                .into_iter()
                .map(|pattern| ExclusionPattern::try_from(pattern).expect("a compilable pattern"))
                .collect(),
        };

        assert!(declaration.excludes("skills/jit-manage/evals/case.md"));
        assert!(declaration.excludes("skills/jit-manage/references/trigger-log.md"));
        assert!(!declaration.excludes("skills/jit-manage/references/plan-schema.md"));
    }

    #[test]
    fn test_profile_manifest_serialize_carries_every_declared_root_and_pattern() {
        let manifest = manifest_of(
            "[[live-source]]\n\
             root = \".agents/skills\"\n\
             exclude = [\".agents/skills/*/evals/**\"]\n",
        )
        .expect("a manifest declaring one root parses");

        let wire = serde_json::to_value(&manifest).expect("the manifest serializes");
        assert_eq!(wire["live-source"][0]["root"], ".agents/skills");
        assert_eq!(
            wire["live-source"][0]["exclude"][0],
            ".agents/skills/*/evals/**"
        );

        // The wire form is the parsed value's own form: reading it back is exact.
        let restored: ProfileManifest =
            serde_json::from_value(wire).expect("the serialized manifest parses");
        assert_eq!(restored, manifest);
    }
}
