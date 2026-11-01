//! Strict profile-manifest decoding.
//!
//! Wire structs live only at this boundary.  The decoder immediately maps the
//! manifest wire to [`ProfilePackageModel`], so package readers never carry a
//! versioned manifest representation.

use super::manifest::{
    AssetDeclaration, EnvironmentVariableName, LiveSourceDeclaration, ProfileDependencyRequirement,
    ProfileIncompatibility, ProfilePackageModel, ProfileVariableDeclaration, ProfileVariableName,
    RegionDeclaration,
};
use crate::domain::identity::ProfileId;
use crate::repository_state::Contribution;
use semver::{Version, VersionReq};
use serde::Deserialize;

const MANIFEST_VERSION: u32 = 2;

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
    #[error("unsupported profile manifest version {0} in field 'manifest-version'; expected {MANIFEST_VERSION}")]
    UnsupportedVersion(u32),
}

/// Decode the manifest wire directly into the canonical model.
///
/// The source is parsed once; the version discriminator is checked on the
/// parsed document before the wire is deserialized from it.
pub(crate) fn decode_manifest(bytes: &[u8]) -> Result<ProfilePackageModel, ManifestWireError> {
    let document: toml::Value = toml::from_str(std::str::from_utf8(bytes)?)?;
    let version = document
        .get("profile")
        .and_then(toml::Value::as_table)
        .ok_or(ManifestWireError::InvalidVersionField("profile"))?
        .get("manifest-version")
        .and_then(toml::Value::as_integer)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(ManifestWireError::InvalidVersionField("manifest-version"))?;
    if version != MANIFEST_VERSION {
        return Err(ManifestWireError::UnsupportedVersion(version));
    }

    let wire: Manifest = document.try_into()?;
    Ok(ProfilePackageModel {
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
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    profile: ManifestProfile,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ManifestProfile {
    /// Checked on the parsed document before the wire is deserialized.
    #[serde(rename = "manifest-version")]
    _manifest_version: u32,
    id: ProfileId,
    version: Version,
    compatible_jit: VersionReq,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDependency {
    id: ProfileId,
    version: VersionReq,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestIncompatibility {
    id: ProfileId,
    version: VersionReq,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestVariable {
    name: ProfileVariableName,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    env: Option<EnvironmentVariableName>,
}

#[cfg(test)]
mod tests {
    use super::{decode_manifest, ManifestWireError};

    #[test]
    fn test_decode_manifest_accepts_lifecycle_fields_and_normalizes_wire_names() {
        let model = decode_manifest(
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

        assert_eq!(model.id.as_str(), "example-package");
        assert_eq!(model.version.to_string(), "2.0.0");
        assert_eq!(model.compatible_jit.to_string(), ">=2.0.0, <3.0.0");
        assert_eq!(model.dependencies[0].id.as_str(), "base-package");
        assert_eq!(model.dependencies[0].version.to_string(), ">=1.4.0, <2.0.0");
        assert_eq!(model.incompatibilities[0].id.as_str(), "legacy-package");
        assert_eq!(model.incompatibilities[0].version.to_string(), ">=4.0.0");
        assert_eq!(model.variables[0].name.as_str(), "PROJECT_NAME");
        assert_eq!(model.variables[0].default.as_deref(), Some("example"));
        assert_eq!(
            model.variables[0].env.as_ref().map(|name| name.as_str()),
            Some("PROJECT_NAME")
        );
    }

    #[test]
    fn test_decode_manifest_returns_canonical_model_without_wire_discriminator() {
        let model = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
compatible-jit = ">=2.0.0"
"#,
        )
        .expect("the manifest should decode");

        let model = serde_json::to_value(model).expect("the model serializes");
        assert_eq!(model["id"], "example-package");
        assert_eq!(model["compatible-jit"], ">=2.0.0");
        assert!(model.get("manifest-version").is_none());
        assert!(model.get("profile").is_none());
    }

    #[test]
    fn test_decode_manifest_rejects_every_version_other_than_two_by_naming_it() {
        for version in [0, 1, 3, 9] {
            let error = decode_manifest(
                format!(
                    "[profile]\nmanifest-version = {version}\nid = \"example-package\"\n\
                     version = \"1.0.0\"\ncompatible-jit = \">=1.0.0\"\n"
                )
                .as_bytes(),
            )
            .expect_err("only manifest version 2 decodes")
            .to_string();
            assert!(
                error.contains(&format!("manifest version {version} ")),
                "{error}"
            );
        }
    }

    #[test]
    fn test_decode_manifest_rejects_unknown_field_and_missing_field_by_name() {
        let unknown_field = decode_manifest(
            br#"
[profile]
manifest-version = 2
id = "example-package"
version = "2.0.0"
jit = ">=2.0.0"
compatible-jit = ">=2.0.0"
"#,
        )
        .expect_err("unknown fields must not enter the wire");
        assert!(
            unknown_field.to_string().contains("unknown field `jit`"),
            "{unknown_field}"
        );

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
        .expect_err("the wire must require its compatible-jit field");
        assert!(missing_field.to_string().contains("compatible-jit"));
    }

    #[test]
    fn test_decode_manifest_rejects_each_malformed_version_field_naming_it() {
        let manifest = |version: &str,
                        compatible: &str,
                        dependency: &str,
                        incompatibility: &str| {
            format!(
                "[profile]\nmanifest-version = 2\nid = \"example\"\nversion = \"{version}\"\ncompatible-jit = \"{compatible}\"\n\n[[dependency]]\nid = \"base\"\nversion = \"{dependency}\"\n\n[[incompatibility]]\nid = \"old\"\nversion = \"{incompatibility}\"\n"
            )
        };
        assert!(decode_manifest(manifest("1.0.0", "*", "*", "*").as_bytes()).is_ok());

        for (malformed, field) in [
            (manifest("01.2.3", "*", "*", "*"), "version"),
            (manifest("1.0.0", "nope", "*", "*"), "compatible-jit"),
            (manifest("1.0.0", "*", "nope", "*"), "dependency"),
            (manifest("1.0.0", "*", "*", "nope"), "incompatibility"),
        ] {
            let error = decode_manifest(malformed.as_bytes()).unwrap_err();
            assert!(
                matches!(error, ManifestWireError::Toml(_)) && error.to_string().contains(field),
                "{error}"
            );
        }
    }
}
