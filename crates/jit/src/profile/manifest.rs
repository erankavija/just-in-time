use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

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
