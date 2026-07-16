use crate::config::{ProjectionMode, ProjectionStyle};
use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// A semantic contribution to one JIT registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Contribution {
    /// One key/value entry in a map-like configuration table.
    MapEntry {
        /// Supported map target.
        target: MapEntryTarget,
        /// Entry key.
        identity: String,
        /// JSON-compatible TOML value.
        value: Value,
    },
    /// One member of a set-like string array.
    SetString {
        /// Supported set target.
        target: SetStringTarget,
        /// String member.
        value: String,
    },
    /// One complete table in an array-of-tables registry.
    KeyedArray {
        /// Supported keyed-array target.
        target: KeyedArrayTarget,
        /// Registry key (`key` for gates, `name` for rules/templates).
        identity: String,
        /// Complete JSON-compatible TOML table.
        value: Value,
    },
    /// One complete root singleton table.
    SingletonTable {
        /// Supported singleton table.
        target: SingletonTableTarget,
        /// Complete projection configuration. No field defaults are applied.
        value: CompleteProjectionConfig,
    },
}

impl Contribution {
    /// Repository-relative registry file receiving this contribution.
    pub fn registry_path(&self) -> &'static str {
        match self {
            Self::MapEntry { .. } | Self::SetString { .. } | Self::SingletonTable { .. } => {
                ".jit/config.toml"
            }
            Self::KeyedArray { target, .. } => target.registry_path(),
        }
    }
}

/// Supported map-like registry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MapEntryTarget {
    /// `[type_hierarchy.types]`
    TypeHierarchyTypes,
    /// `[type_hierarchy.label_associations]`
    LabelAssociations,
    /// `[namespaces]`
    Namespaces,
    /// `[item_kinds]`
    ItemKinds,
}

/// Supported set-like registry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SetStringTarget {
    /// `type_hierarchy.strategic_types`
    StrategicTypes,
}

/// Supported keyed array-of-tables registry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyedArrayTarget {
    /// `[[gates]]`, keyed by `key`.
    Gates,
    /// `[[rules]]`, keyed by `name`.
    Rules,
    /// `[[templates]]`, keyed by `name`.
    Templates,
}

impl KeyedArrayTarget {
    pub(crate) fn registry_path(self) -> &'static str {
        match self {
            Self::Gates => ".jit/gates.toml",
            Self::Rules => ".jit/rules.toml",
            Self::Templates => ".jit/templates.toml",
        }
    }

    pub(crate) fn identity_field(self) -> &'static str {
        match self {
            Self::Gates => "key",
            Self::Rules | Self::Templates => "name",
        }
    }
}

/// Supported complete root singleton tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SingletonTableTarget {
    /// `[invariant_projection]`
    InvariantProjection,
    /// `[rules_gates_projection]`
    RulesGatesProjection,
}

/// Required fields for a profile-provided projection table.
///
/// The repository configuration types apply defaults because hand-authored
/// configuration may omit fields. A profile contribution must instead state
/// the complete value so equality and hashing never depend on ambient defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteProjectionConfig {
    /// Projection placement mode.
    pub mode: ProjectionMode,
    /// Repository-relative projection target.
    pub target: String,
    /// Projection render style.
    pub style: ProjectionStyle,
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
