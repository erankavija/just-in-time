use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

pub use crate::repository_state::{
    CompleteProjectionConfig, Contribution, KeyedArrayTarget, MapEntryTarget, SetStringTarget,
};

/// The only manifest filename recognized at the root of an embedded package.
pub const MANIFEST_FILE_NAME: &str = "manifest.toml";

/// Version of the immutable v1 profile-manifest wire contract.
pub const PROFILE_MANIFEST_VERSION: u32 = 1;

/// A complete declarative profile manifest.
///
/// Unknown fields are rejected so hook, lifecycle, and future composition
/// syntax cannot silently enter the v1 contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileManifest {
    /// Stable package identity, version, and compatible JIT range.
    pub profile: ProfileMetadata,
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
    pub id: String,
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
