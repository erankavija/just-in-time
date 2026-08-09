//! Strict versioned profile-manifest decoding.
//!
//! Wire structs live only at this boundary.  The decoder immediately maps both
//! supported manifest versions to [`ProfilePackageModel`], so package readers
//! never carry a versioned manifest representation.

use super::manifest::{
    AssetDeclaration, LiveSourceDeclaration, ProfileDependencyRequirement, ProfileId,
    ProfileIncompatibility, ProfilePackageModel, ProfileVariableDeclaration, RegionDeclaration,
};
use crate::repository_state::Contribution;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const V1: u32 = 1;
const V2: u32 = 2;

/// The decoded package model and the version-specific input used by the
/// package identity hash.
#[derive(Debug)]
pub(crate) struct DecodedManifest {
    pub(crate) model: ProfilePackageModel,
    /// V1 retains its released canonical JSON hash input; v2 hashes the exact
    /// unresolved manifest source bytes.
    pub(crate) identity_manifest: Vec<u8>,
}

/// Errors at the private manifest-wire boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ManifestWireError {
    /// The manifest bytes are not UTF-8.
    #[error("profile manifest is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    /// The manifest is not valid TOML.
    #[error("invalid profile manifest: {0}")]
    Toml(#[from] toml::de::Error),
    /// The discriminator or its enclosing field is absent or malformed.
    #[error("profile manifest field '{0}' is missing or invalid")]
    InvalidVersionField(&'static str),
    /// The wire version has no decoder.
    #[error("unsupported profile manifest version {0} in field 'manifest-version'; expected {V1} or {V2}")]
    UnsupportedVersion(u32),
    /// Canonicalizing a recognized wire failed unexpectedly.
    #[error("failed to serialize profile manifest wire: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Decode either supported manifest wire directly into the canonical model.
pub(crate) fn decode_manifest(bytes: &[u8]) -> Result<DecodedManifest, ManifestWireError> {
    let text = std::str::from_utf8(bytes)?;
    let document: toml::Value = toml::from_str(text)?;
    let profile = document
        .get("profile")
        .and_then(toml::Value::as_table)
        .ok_or(ManifestWireError::InvalidVersionField("profile"))?;
    let version = profile
        .get("manifest-version")
        .and_then(toml::Value::as_integer)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(ManifestWireError::InvalidVersionField("manifest-version"))?;

    match version {
        V1 => decode_v1(text),
        V2 => decode_v2(text),
        other => Err(ManifestWireError::UnsupportedVersion(other)),
    }
}

fn decode_v1(text: &str) -> Result<DecodedManifest, ManifestWireError> {
    let wire: ManifestV1 = toml::from_str(text)?;
    if v1_declares_template_field(text, "asset")? {
        return Err(ManifestWireError::InvalidVersionField("asset.template"));
    }
    if v1_declares_template_field(text, "region")? {
        return Err(ManifestWireError::InvalidVersionField("region.template"));
    }
    let identity_manifest = serde_json::to_vec(&canonicalize(serde_json::to_value(&wire)?))?;
    let model = ProfilePackageModel {
        id: wire.profile.id,
        version: wire.profile.version,
        compatible_jit: wire.profile.jit,
        dependencies: wire
            .dependencies
            .into_iter()
            .map(|id| ProfileDependencyRequirement {
                id,
                version: "*".to_string(),
            })
            .collect(),
        incompatibilities: Vec::new(),
        variables: Vec::new(),
        contributions: wire.contributions,
        assets: wire.assets,
        regions: wire.regions,
        live_sources: wire.live_sources,
    };
    Ok(DecodedManifest {
        identity_manifest,
        model,
    })
}

fn v1_declares_template_field(text: &str, collection: &str) -> Result<bool, ManifestWireError> {
    let document: toml::Value = toml::from_str(text)?;
    Ok(document
        .get(collection)
        .and_then(toml::Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .as_table()
                    .is_some_and(|table| table.contains_key("template"))
            })
        }))
}

fn decode_v2(text: &str) -> Result<DecodedManifest, ManifestWireError> {
    let wire: ManifestV2 = toml::from_str(text)?;
    let model = ProfilePackageModel {
        id: wire.profile.id,
        version: wire.profile.version,
        compatible_jit: wire.profile.compatible_jit,
        dependencies: wire
            .dependencies
            .into_iter()
            .map(|dependency| ProfileDependencyRequirement {
                id: dependency.id,
                version: dependency.version,
            })
            .collect(),
        incompatibilities: wire
            .incompatibilities
            .into_iter()
            .map(|incompatibility| ProfileIncompatibility {
                id: incompatibility.id,
                version: incompatibility.version,
            })
            .collect(),
        variables: wire
            .variables
            .into_iter()
            .map(|variable| ProfileVariableDeclaration {
                name: variable.name,
                default: variable.default,
                env: variable.env,
            })
            .collect(),
        contributions: wire.contributions,
        assets: wire.assets,
        regions: wire.regions,
        live_sources: wire.live_sources,
    };
    Ok(DecodedManifest {
        identity_manifest: text.as_bytes().to_vec(),
        model,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestV1 {
    profile: ManifestV1Profile,
    #[serde(default)]
    dependencies: Vec<ProfileId>,
    #[serde(default, rename = "contribution")]
    contributions: Vec<Contribution>,
    #[serde(default, rename = "asset")]
    assets: Vec<AssetDeclaration>,
    #[serde(default, rename = "region")]
    regions: Vec<RegionDeclaration>,
    #[serde(default, rename = "live-source")]
    live_sources: Vec<LiveSourceDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ManifestV1Profile {
    manifest_version: u32,
    id: ProfileId,
    version: String,
    jit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestV2 {
    profile: ManifestV2Profile,
    #[serde(default, rename = "dependency")]
    dependencies: Vec<ManifestDependency>,
    #[serde(default, rename = "incompatibility")]
    incompatibilities: Vec<ManifestIncompatibility>,
    #[serde(default, rename = "variable")]
    variables: Vec<ManifestVariable>,
    #[serde(default, rename = "contribution")]
    contributions: Vec<Contribution>,
    #[serde(default, rename = "asset")]
    assets: Vec<AssetDeclaration>,
    #[serde(default, rename = "region")]
    regions: Vec<RegionDeclaration>,
    #[serde(default, rename = "live-source")]
    live_sources: Vec<LiveSourceDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ManifestV2Profile {
    manifest_version: u32,
    id: ProfileId,
    version: String,
    compatible_jit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDependency {
    id: ProfileId,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestIncompatibility {
    id: ProfileId,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestVariable {
    name: String,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    env: Option<String>,
}

/// Canonicalize a JSON object recursively without changing array order.
fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::decode_manifest;

    #[test]
    fn test_decode_manifest_v1_accepts_shipped_wire_and_normalizes_dependencies() {
        let decoded = decode_manifest(
            br#"
dependencies = ["base-package"]

[profile]
manifest-version = 1
id = "example-package"
version = "1.2.3"
jit = ">=1.0.0, <2.0.0"
"#,
        )
        .expect("the shipped manifest wire should decode");

        assert_eq!(decoded.model.id.as_str(), "example-package");
        assert_eq!(decoded.model.version, "1.2.3");
        assert_eq!(decoded.model.compatible_jit, ">=1.0.0, <2.0.0");
        assert_eq!(decoded.model.dependencies.len(), 1);
        assert_eq!(decoded.model.dependencies[0].id.as_str(), "base-package");
        assert_eq!(decoded.model.dependencies[0].version, "*");
        assert!(decoded.model.incompatibilities.is_empty());
        assert!(decoded.model.variables.is_empty());
    }

    #[test]
    fn test_decode_manifest_v2_accepts_lifecycle_fields_and_normalizes_wire_names() {
        let decoded = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
compatible-jit = ">=2.0.0, <3.0.0"

[[dependency]]
id = "base-package"
version = ">=1.4.0, <2.0.0"

[[incompatibility]]
id = "legacy-package"
version = ">=4.0.0"

[[variable]]
name = "PROJECT_NAME"
default = "example"
env = "PROJECT_NAME"
"#,
        )
        .expect("the lifecycle manifest wire should decode");

        assert_eq!(decoded.model.id.as_str(), "example-package");
        assert_eq!(decoded.model.version, "2.0.0");
        assert_eq!(decoded.model.compatible_jit, ">=2.0.0, <3.0.0");
        assert_eq!(decoded.model.dependencies[0].id.as_str(), "base-package");
        assert_eq!(decoded.model.dependencies[0].version, ">=1.4.0, <2.0.0");
        assert_eq!(
            decoded.model.incompatibilities[0].id.as_str(),
            "legacy-package"
        );
        assert_eq!(decoded.model.incompatibilities[0].version, ">=4.0.0");
        assert_eq!(decoded.model.variables[0].name, "PROJECT_NAME");
        assert_eq!(
            decoded.model.variables[0].default.as_deref(),
            Some("example")
        );
        assert_eq!(
            decoded.model.variables[0].env.as_deref(),
            Some("PROJECT_NAME")
        );
    }

    #[test]
    fn test_decode_manifest_returns_canonical_model_without_wire_discriminator() {
        let decoded = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
compatible-jit = ">=2.0.0"
"#,
        )
        .expect("the v2 manifest should decode");

        let model = serde_json::to_value(decoded.model).expect("the model serializes");
        assert_eq!(model["id"], "example-package");
        assert_eq!(model["compatible-jit"], ">=2.0.0");
        assert!(model.get("manifest-version").is_none());
        assert!(model.get("profile").is_none());
    }

    #[test]
    fn test_decode_manifest_v2_identity_preserves_unresolved_source_bytes() {
        let source = br#"# authored formatting is part of the unresolved v2 source
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
compatible-jit = ">=2.0.0"
"#;

        let decoded = decode_manifest(source).expect("the v2 manifest should decode");

        assert_eq!(decoded.identity_manifest, source);
    }

    #[test]
    fn test_decode_manifest_rejects_wire_specific_fields_with_their_field_names() {
        let v1_error = decode_manifest(
            br#"
[profile]
manifest-version = 1
id = "example-package"
version = "1.0.0"
jit = ">=1.0.0"
compatible-jit = ">=1.0.0"
"#,
        )
        .expect_err("v2 compatibility spelling is not part of the v1 wire");
        assert!(v1_error.to_string().contains("compatible-jit"));

        let v2_error = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
jit = ">=2.0.0"
compatible-jit = ">=2.0.0"
"#,
        )
        .expect_err("v1 compatibility spelling is not part of the v2 wire");
        assert!(v2_error.to_string().contains("jit"));

        let v1_template_error = decode_manifest(
            br#"
[profile]
manifest-version = 1
id = "example-package"
version = "1.0.0"
jit = ">=1.0.0"

[[asset]]
source = "asset.txt"
target = "docs/asset.txt"
template = true
"#,
        )
        .expect_err("body substitution opt-in is a v2 wire field");
        assert!(v1_template_error.to_string().contains("asset.template"));
    }

    #[test]
    fn test_decode_manifest_rejects_unknown_field_unknown_version_and_missing_field_by_name() {
        let unknown_field = decode_manifest(
            br#"
[profile]
manifest-version = 1
id = "example-package"
version = "1.0.0"
jit = ">=1.0.0"

unexpected = true
"#,
        )
        .expect_err("unknown fields must not enter the v1 wire");
        assert!(unknown_field.to_string().contains("unexpected"));

        let unknown_version = decode_manifest(
            br#"
[profile]
manifest-version = 9
id = "example-package"
version = "1.0.0"
jit = ">=1.0.0"
"#,
        )
        .expect_err("unknown manifest versions must be rejected");
        assert!(unknown_version.to_string().contains("9"));
        assert!(unknown_version.to_string().contains("manifest version"));

        let missing_field = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"

[[dependency]]
id = "base-package"
version = "*"
"#,
        )
        .expect_err("the v2 wire must require its compatible-jit field");
        assert!(missing_field.to_string().contains("compatible-jit"));
    }
}
