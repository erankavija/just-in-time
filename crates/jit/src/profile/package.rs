use super::manifest::{
    is_lowercase_kebab, ProfileManifest, MANIFEST_FILE_NAME, PROFILE_MANIFEST_VERSION,
};
use crate::repository_state::{Contribution, MapEntryTarget};
use include_dir::Dir;
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const TARGET_HASH_DOMAIN: &[u8] = b"jit-profile-target-v1\0";
const PACKAGE_HASH_DOMAIN: &[u8] = b"jit-profile-package-v1\0";

/// Maximum number of files, including the manifest, in one embedded package.
pub const MAX_EMBEDDED_PROFILE_FILES: usize = 512;

/// Maximum total bytes, including the manifest, in one embedded package.
pub const MAX_EMBEDDED_PROFILE_BYTES: usize = 4 * 1024 * 1024;

/// A lowercase hexadecimal SHA-256 digest.
pub type PackageHash = String;

/// Canonical package and per-repository-target hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePackageHashes {
    /// Hash of the canonical manifest plus every declared embedded source.
    pub package: PackageHash,
    /// Hash of ordered operations and source bytes grouped by repository target.
    pub targets: BTreeMap<String, PackageHash>,
}

/// A validated immutable package embedded recursively at compile time.
#[derive(Debug, Clone)]
pub struct EmbeddedProfilePackage<'a> {
    manifest: ProfileManifest,
    files: BTreeMap<String, &'a [u8]>,
    hashes: ProfilePackageHashes,
}

impl<'a> EmbeddedProfilePackage<'a> {
    /// Parse and validate a recursively embedded directory.
    pub fn from_dir(directory: &'a Dir<'a>) -> Result<Self, ProfilePackageError> {
        let files = embedded_files(directory)?;
        Self::from_files(files)
    }

    fn from_files(files: BTreeMap<String, &'a [u8]>) -> Result<Self, ProfilePackageError> {
        validate_package_bounds(&files)?;
        let manifest_bytes = files
            .get(MANIFEST_FILE_NAME)
            .copied()
            .ok_or(ProfilePackageError::MissingManifest)?;
        let manifest_text = std::str::from_utf8(manifest_bytes)
            .map_err(|source| ProfilePackageError::ManifestUtf8 { source })?;
        let manifest: ProfileManifest =
            toml::from_str(manifest_text).map_err(ProfilePackageError::ManifestToml)?;

        validate_manifest(&manifest, &files)?;
        let hashes = compute_hashes(&manifest, &files)?;

        Ok(Self {
            manifest,
            files,
            hashes,
        })
    }

    /// Parsed runtime manifest.
    pub fn manifest(&self) -> &ProfileManifest {
        &self.manifest
    }

    /// Embedded bytes for a declared package-relative source.
    pub fn source_bytes(&self, source: &str) -> Option<&'a [u8]> {
        self.files.get(source).copied()
    }

    /// Canonical package and target hashes.
    pub fn hashes(&self) -> &ProfilePackageHashes {
        &self.hashes
    }

    /// Embedded file count, including `manifest.toml`.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total embedded byte size, including `manifest.toml`.
    pub fn byte_size(&self) -> usize {
        self.files.values().map(|bytes| bytes.len()).sum()
    }
}

/// Validation failures for immutable embedded packages.
#[derive(Debug, thiserror::Error)]
pub enum ProfilePackageError {
    /// The root manifest is absent.
    #[error("embedded profile package is missing root manifest.toml")]
    MissingManifest,
    /// The manifest is not UTF-8 TOML.
    #[error("embedded profile manifest is not UTF-8: {source}")]
    ManifestUtf8 {
        /// UTF-8 parser error.
        source: std::str::Utf8Error,
    },
    /// TOML does not match the runtime manifest wire type.
    #[error("invalid embedded profile manifest: {0}")]
    ManifestToml(#[source] toml::de::Error),
    /// Unsupported wire version.
    #[error("unsupported profile manifest version {actual}; expected {expected}")]
    ManifestVersion {
        /// Parsed version.
        actual: u32,
        /// Supported version.
        expected: u32,
    },
    /// A package declares itself as a dependency.
    #[error("profile '{package}' cannot depend on itself ('{dependency}')")]
    SelfDependency {
        /// Package that owns the manifest.
        package: String,
        /// Offending dependency entry.
        dependency: String,
    },
    /// Invalid semantic package version.
    #[error("invalid profile version '{value}': {source}")]
    InvalidVersion {
        /// Authored value.
        value: String,
        /// Semver parser error.
        source: semver::Error,
    },
    /// Invalid compatible JIT version requirement.
    #[error("invalid compatible JIT requirement '{value}': {source}")]
    InvalidCompatibility {
        /// Authored value.
        value: String,
        /// Semver parser error.
        source: semver::Error,
    },
    /// Unsafe absolute, traversal, platform-prefix, or empty path.
    #[error("unsafe {field} path '{path}'")]
    UnsafePath {
        /// Manifest field.
        field: &'static str,
        /// Rejected value.
        path: String,
    },
    /// A package source is declared more than once.
    #[error("embedded source '{0}' is declared more than once")]
    DuplicateSource(String),
    /// Two file/region declarations target the same repository path.
    #[error("repository content target '{0}' is declared more than once")]
    DuplicateContentTarget(String),
    /// A declaration references absent embedded bytes.
    #[error("declared embedded source '{0}' is missing")]
    MissingContent(String),
    /// Embedded bytes have no declaration.
    #[error("embedded source '{0}' is not declared by the manifest")]
    ExtraContent(String),
    /// Invalid semantic contribution.
    #[error("invalid contribution at index {index}: {message}")]
    InvalidContribution {
        /// Manifest order.
        index: usize,
        /// Specific contract failure.
        message: String,
    },
    /// Duplicate semantic contribution identity.
    #[error("duplicate contribution identity '{0}'")]
    DuplicateContribution(String),
    /// Invalid region marker identity.
    #[error("invalid region id '{0}'; expected lowercase-kebab")]
    InvalidRegionId(String),
    /// Canonical serialization unexpectedly failed.
    #[error("failed to serialize canonical profile data: {0}")]
    CanonicalSerialization(#[source] serde_json::Error),
    /// Embedded count or byte-size exceeds the compile-time package budget.
    #[error(
        "embedded profile package exceeds bounds: {file_count} files/{byte_size} bytes; \
         maximum is {max_files} files/{max_bytes} bytes"
    )]
    PackageBounds {
        /// Actual file count.
        file_count: usize,
        /// Actual total bytes.
        byte_size: usize,
        /// Maximum supported file count.
        max_files: usize,
        /// Maximum supported total bytes.
        max_bytes: usize,
    },
}

fn embedded_files<'a>(
    directory: &'a Dir<'a>,
) -> Result<BTreeMap<String, &'a [u8]>, ProfilePackageError> {
    fn visit<'a>(directory: &'a Dir<'a>, files: &mut BTreeMap<String, &'a [u8]>) {
        directory.files().for_each(|file| {
            files.insert(normalize_embedded_path(file.path()), file.contents());
        });
        directory.dirs().for_each(|child| visit(child, files));
    }

    let mut files = BTreeMap::new();
    visit(directory, &mut files);
    Ok(files)
}

fn normalize_embedded_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_package_bounds(files: &BTreeMap<String, &[u8]>) -> Result<(), ProfilePackageError> {
    let file_count = files.len();
    let byte_size = files
        .values()
        .fold(0usize, |total, bytes| total.saturating_add(bytes.len()));
    if file_count <= MAX_EMBEDDED_PROFILE_FILES && byte_size <= MAX_EMBEDDED_PROFILE_BYTES {
        Ok(())
    } else {
        Err(ProfilePackageError::PackageBounds {
            file_count,
            byte_size,
            max_files: MAX_EMBEDDED_PROFILE_FILES,
            max_bytes: MAX_EMBEDDED_PROFILE_BYTES,
        })
    }
}

fn validate_manifest(
    manifest: &ProfileManifest,
    files: &BTreeMap<String, &[u8]>,
) -> Result<(), ProfilePackageError> {
    if manifest.profile.manifest_version != PROFILE_MANIFEST_VERSION {
        return Err(ProfilePackageError::ManifestVersion {
            actual: manifest.profile.manifest_version,
            expected: PROFILE_MANIFEST_VERSION,
        });
    }
    if let Some(dependency) = manifest
        .dependencies
        .iter()
        .find(|dependency| *dependency == &manifest.profile.id)
    {
        return Err(ProfilePackageError::SelfDependency {
            package: manifest.profile.id.to_string(),
            dependency: dependency.to_string(),
        });
    }
    Version::parse(&manifest.profile.version).map_err(|source| {
        ProfilePackageError::InvalidVersion {
            value: manifest.profile.version.clone(),
            source,
        }
    })?;
    VersionReq::parse(&manifest.profile.jit).map_err(|source| {
        ProfilePackageError::InvalidCompatibility {
            value: manifest.profile.jit.clone(),
            source,
        }
    })?;

    let mut contribution_ids = BTreeSet::new();
    for (index, contribution) in manifest.contributions.iter().enumerate() {
        validate_contribution(index, contribution)?;
        let identity = contribution_identity(contribution);
        if !contribution_ids.insert(identity.clone()) {
            return Err(ProfilePackageError::DuplicateContribution(identity));
        }
    }

    let mut declared_sources = BTreeSet::new();
    let mut content_targets = BTreeSet::new();
    for asset in &manifest.assets {
        validate_relative_path("asset source", &asset.source)?;
        validate_relative_path("asset target", &asset.target)?;
        insert_unique_source(&mut declared_sources, &asset.source)?;
        if !content_targets.insert(asset.target.clone()) {
            return Err(ProfilePackageError::DuplicateContentTarget(
                asset.target.clone(),
            ));
        }
    }
    for region in &manifest.regions {
        validate_relative_path("region source", &region.source)?;
        validate_relative_path("region target", &region.target)?;
        if !is_lowercase_kebab(&region.region_id) {
            return Err(ProfilePackageError::InvalidRegionId(
                region.region_id.clone(),
            ));
        }
        insert_unique_source(&mut declared_sources, &region.source)?;
        if !content_targets.insert(region.target.clone()) {
            return Err(ProfilePackageError::DuplicateContentTarget(
                region.target.clone(),
            ));
        }
    }

    for source in &declared_sources {
        if !files.contains_key(source) {
            return Err(ProfilePackageError::MissingContent(source.clone()));
        }
    }
    for path in files
        .keys()
        .filter(|path| path.as_str() != MANIFEST_FILE_NAME)
    {
        if !declared_sources.contains(path) {
            return Err(ProfilePackageError::ExtraContent(path.clone()));
        }
    }
    Ok(())
}

fn insert_unique_source(
    sources: &mut BTreeSet<String>,
    source: &str,
) -> Result<(), ProfilePackageError> {
    if source == MANIFEST_FILE_NAME {
        return Err(ProfilePackageError::DuplicateSource(source.to_string()));
    }
    if sources.insert(source.to_string()) {
        Ok(())
    } else {
        Err(ProfilePackageError::DuplicateSource(source.to_string()))
    }
}

fn validate_contribution(
    index: usize,
    contribution: &Contribution,
) -> Result<(), ProfilePackageError> {
    let invalid = |message: String| ProfilePackageError::InvalidContribution { index, message };
    match contribution {
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => {
            if identity.trim().is_empty() {
                return Err(invalid("identity must not be empty".to_string()));
            }
            match target {
                MapEntryTarget::TypeHierarchyTypes
                    if value.as_u64().is_none_or(|level| level == 0) =>
                {
                    Err(invalid(
                        "type-hierarchy-types value must be a positive integer".to_string(),
                    ))
                }
                MapEntryTarget::LabelAssociations if !value.is_string() => Err(invalid(
                    "label-associations value must be a string".to_string(),
                )),
                MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds if !value.is_object() => {
                    Err(invalid(format!(
                        "{target:?} value must be a complete table"
                    )))
                }
                _ => Ok(()),
            }
        }
        Contribution::SetString { value, .. } if value.trim().is_empty() => {
            Err(invalid("set string must not be empty".to_string()))
        }
        Contribution::SetString { .. } => Ok(()),
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => {
            let table = value
                .as_object()
                .ok_or_else(|| invalid("keyed-array value must be a complete table".to_string()))?;
            let field = target.identity_field();
            match table.get(field).and_then(Value::as_str) {
                Some(actual) if actual == identity && !identity.is_empty() => Ok(()),
                Some(actual) => Err(invalid(format!(
                    "{target:?} identity '{identity}' does not match value.{field} '{actual}'"
                ))),
                None => Err(invalid(format!(
                    "{target:?} value must contain string field '{field}'"
                ))),
            }
        }
        Contribution::Projection { value, .. } => {
            validate_relative_path("projection target", &value.target)
                .map_err(|error| invalid(error.to_string()))
        }
    }
}

fn validate_relative_path(field: &'static str, path: &str) -> Result<(), ProfilePackageError> {
    let parsed = Path::new(path);
    let has_windows_prefix = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    let has_noncanonical_segment = path
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."));
    let safe = !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && !has_windows_prefix
        && !has_noncanonical_segment
        && !parsed.is_absolute()
        && parsed.components().all(|component| match component {
            Component::Normal(value) => value != "." && value != "..",
            _ => false,
        });
    if safe {
        Ok(())
    } else {
        Err(ProfilePackageError::UnsafePath {
            field,
            path: path.to_string(),
        })
    }
}

fn contribution_identity(contribution: &Contribution) -> String {
    let semantic = match contribution {
        Contribution::MapEntry {
            target, identity, ..
        } => format!("map-entry:{target:?}:{identity}"),
        Contribution::SetString { target, value } => {
            format!("set-string:{target:?}:{value}")
        }
        Contribution::KeyedArray {
            target, identity, ..
        } => format!("keyed-array:{target:?}:{identity}"),
        Contribution::Projection { name, .. } => {
            format!("projection:{name}")
        }
    };
    format!("{}:{semantic}", contribution.registry_path())
}

fn compute_hashes(
    manifest: &ProfileManifest,
    files: &BTreeMap<String, &[u8]>,
) -> Result<ProfilePackageHashes, ProfilePackageError> {
    let mut target_frames: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();

    for contribution in &manifest.contributions {
        target_frames
            .entry(contribution.registry_path().to_string())
            .or_default()
            .push(canonical_bytes(contribution)?);
    }
    for asset in &manifest.assets {
        let mut frame = canonical_bytes(asset)?;
        append_frame(
            &mut frame,
            files
                .get(&asset.source)
                .copied()
                .ok_or_else(|| ProfilePackageError::MissingContent(asset.source.clone()))?,
        );
        target_frames
            .entry(asset.target.clone())
            .or_default()
            .push(frame);
    }
    for region in &manifest.regions {
        let mut frame = canonical_bytes(region)?;
        append_frame(
            &mut frame,
            files
                .get(&region.source)
                .copied()
                .ok_or_else(|| ProfilePackageError::MissingContent(region.source.clone()))?,
        );
        target_frames
            .entry(region.target.clone())
            .or_default()
            .push(frame);
    }

    let targets = target_frames
        .into_iter()
        .map(|(target, frames)| {
            let mut hasher = Sha256::new();
            hasher.update(TARGET_HASH_DOMAIN);
            hash_frame(&mut hasher, target.as_bytes());
            frames
                .iter()
                .for_each(|frame| hash_frame(&mut hasher, frame));
            (target, format!("{:x}", hasher.finalize()))
        })
        .collect();

    let mut package = Sha256::new();
    package.update(PACKAGE_HASH_DOMAIN);
    hash_frame(&mut package, &canonical_bytes(manifest)?);
    for (path, contents) in files
        .iter()
        .filter(|(path, _)| path.as_str() != MANIFEST_FILE_NAME)
    {
        hash_frame(&mut package, path.as_bytes());
        hash_frame(&mut package, contents);
    }

    Ok(ProfilePackageHashes {
        package: format!("{:x}", package.finalize()),
        targets,
    })
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ProfilePackageError> {
    let canonical = canonical_json(value)?;
    serde_json::to_vec(&canonical).map_err(ProfilePackageError::CanonicalSerialization)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Value, ProfilePackageError> {
    serde_json::to_value(value)
        .map(canonicalize_value)
        .map_err(ProfilePackageError::CanonicalSerialization)
}

fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_value(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect::<Map<_, _>>(),
        ),
        scalar => scalar,
    }
}

fn append_frame(destination: &mut Vec<u8>, frame: &[u8]) {
    destination.extend_from_slice(&(frame.len() as u64).to_be_bytes());
    destination.extend_from_slice(frame);
}

fn hash_frame(hasher: &mut Sha256, frame: &[u8]) {
    hasher.update((frame.len() as u64).to_be_bytes());
    hasher.update(frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{profile_manifest_schema, ProfileId};
    use crate::repository_state::{Contribution, KeyedArrayTarget, MapEntryTarget};
    use include_dir::{include_dir, Dir};

    static VALID_PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/synthetic-valid");

    fn package() -> EmbeddedProfilePackage<'static> {
        EmbeddedProfilePackage::from_dir(&VALID_PACKAGE).expect("valid synthetic package")
    }

    fn manifest_text() -> String {
        VALID_PACKAGE
            .get_file(MANIFEST_FILE_NAME)
            .expect("fixture manifest")
            .contents_utf8()
            .expect("UTF-8 manifest")
            .to_string()
    }

    fn parse_modified(old: &str, new: &str) -> Result<ProfileManifest, toml::de::Error> {
        toml::from_str(&manifest_text().replace(old, new))
    }

    fn schema_has_property(value: &Value, property: &str) -> bool {
        match value {
            Value::Array(values) => values
                .iter()
                .any(|value| schema_has_property(value, property)),
            Value::Object(values) => {
                values
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|properties| properties.contains_key(property))
                    || values
                        .values()
                        .any(|value| schema_has_property(value, property))
            }
            _ => false,
        }
    }

    #[test]
    fn test_embedded_package_recurses_and_preserves_manifest_order() {
        let package = package();
        assert_eq!(package.file_count(), 4);
        assert!(package.file_count() <= MAX_EMBEDDED_PROFILE_FILES);
        assert!(package.byte_size() <= MAX_EMBEDDED_PROFILE_BYTES);
        assert_eq!(package.manifest().profile.id.as_str(), "synthetic-workflow");
        assert_eq!(package.manifest().contributions.len(), 10);
        assert!(matches!(
            package.manifest().contributions.first(),
            Some(Contribution::MapEntry {
                target: MapEntryTarget::TypeHierarchyTypes,
                identity,
                ..
            }) if identity == "initiative"
        ));
        assert!(matches!(
            package.manifest().contributions.get(6),
            Some(Contribution::KeyedArray {
                target: KeyedArrayTarget::Gates,
                identity,
                ..
            }) if identity == "synthetic-review"
        ));
        assert!(matches!(
            package.manifest().contributions.last(),
            Some(Contribution::Projection { name, .. }) if name == "rules-and-gates"
        ));
        assert_eq!(
            package.source_bytes("nested/scripts/check.sh"),
            Some(b"#!/bin/sh\nexit 0\n".as_slice())
        );
        assert!(package.manifest().assets[1].executable);
        assert!(package.manifest().dependencies.is_empty());
    }

    #[test]
    fn test_manifest_schema_is_generated_from_runtime_wire_type() {
        let package = package();
        let schema = serde_json::to_value(profile_manifest_schema()).unwrap();
        let instance = serde_json::to_value(package.manifest()).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&instance)
            .expect("runtime manifest must satisfy generated schema");

        let schema_text = schema.to_string();
        assert!(schema_text.contains("manifest-version"));
        assert!(schema_text.contains("projection"));
        assert!(!schema_has_property(&schema, "hook"));
        assert!(schema_has_property(&schema, "dependencies"));
        assert!(!schema_has_property(&schema, "variables"));
    }

    #[test]
    fn test_manifest_dependency_is_reported_in_runtime_inspection_and_schema() {
        let manifest =
            manifest_text().replace("[profile]", "dependencies = [\"jit-default\"]\n\n[profile]");
        let mut files = package().files.clone();
        files.insert(MANIFEST_FILE_NAME.to_string(), manifest.leak().as_bytes());

        let package = EmbeddedProfilePackage::from_files(files).unwrap();
        assert_eq!(
            package.manifest().dependencies,
            vec![ProfileId::try_from("jit-default").unwrap()]
        );

        let schema = serde_json::to_value(profile_manifest_schema()).unwrap();
        let instance = serde_json::to_value(package.manifest()).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&instance)
            .expect("manifest with dependency must satisfy generated schema");
        assert_eq!(instance["dependencies"][0], "jit-default");
    }

    #[test]
    fn test_hashes_are_stable_grouped_by_target_and_cover_source_drift() {
        let first = package();
        let second = package();
        assert_eq!(first.hashes(), second.hashes());
        assert_eq!(
            first.hashes().package,
            "40a33c9798b0523436977f492bef09a4deddc9da7293fede33623c101b34a417"
        );
        assert_eq!(first.hashes().targets.len(), 7);
        assert_eq!(
            first.hashes().targets[".jit/config.toml"],
            "0b22a166dff17fbf500cd9682ba0348b3219aac7582736f9df8371f592de4694"
        );
        assert!(first.hashes().targets.contains_key(".jit/gates.toml"));
        assert!(first.hashes().targets.contains_key("AGENTS.md"));
        assert!(first.hashes().targets.contains_key("bin/check.sh"));

        let mut changed = first.files.clone();
        changed.insert(
            "nested/scripts/check.sh".to_string(),
            b"#!/bin/sh\nexit 1\n",
        );
        let changed = EmbeddedProfilePackage::from_files(changed).unwrap();
        assert_ne!(first.hashes().package, changed.hashes().package);
        assert_ne!(
            first.hashes().targets["bin/check.sh"],
            changed.hashes().targets["bin/check.sh"]
        );
        assert_eq!(
            first.hashes().targets["docs/workflow.txt"],
            changed.hashes().targets["docs/workflow.txt"]
        );

        let reordered_manifest = manifest_text().replacen(
            "[[contribution]]\nkind = \"map-entry\"\ntarget = \"type-hierarchy-types\"\nidentity = \"initiative\"\nvalue = 1\n\n",
            "",
            1,
        ) + "\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"type-hierarchy-types\"\nidentity = \"initiative\"\nvalue = 1\n";
        let mut reordered = first.files.clone();
        reordered.insert(
            MANIFEST_FILE_NAME.to_string(),
            reordered_manifest.leak().as_bytes(),
        );
        let reordered = EmbeddedProfilePackage::from_files(reordered).unwrap();
        assert_ne!(
            first.hashes().targets[".jit/config.toml"],
            reordered.hashes().targets[".jit/config.toml"]
        );
    }

    #[test]
    fn test_manifest_rejects_hooks_unknown_kinds_and_incomplete_projections() {
        let hook = parse_modified(
            "jit = \">=0.2.0, <2.0.0\"",
            "jit = \">=0.2.0, <2.0.0\"\nhook = \"install.sh\"",
        )
        .unwrap_err()
        .to_string();
        assert!(hook.contains("unknown field `hook`"), "{hook}");

        let kind = parse_modified("kind = \"map-entry\"", "kind = \"arbitrary-hook\"")
            .unwrap_err()
            .to_string();
        assert!(kind.contains("unknown variant `arbitrary-hook`"), "{kind}");

        let incomplete = parse_modified(
            "value = { kind = \"invariant\", mode = \"region\", target = \"AGENTS.md\", style = \"id-anchor\" }",
            "value = { kind = \"invariant\", mode = \"region\", target = \"AGENTS.md\" }",
        )
        .unwrap_err()
        .to_string();
        assert!(incomplete.contains("missing field `style`"), "{incomplete}");
    }

    #[test]
    fn test_manifest_rejects_malformed_dependency_id_during_parse() {
        let error = parse_modified("[profile]", "dependencies = [\"Jit Default\"]\n\n[profile]")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Jit Default"), "{error}");
        assert!(error.contains("lowercase-kebab"), "{error}");
    }

    #[test]
    fn test_validation_rejects_self_referential_dependency_by_id() {
        let manifest = manifest_text().replace(
            "[profile]",
            "dependencies = [\"synthetic-workflow\"]\n\n[profile]",
        );
        let mut files = package().files.clone();
        files.insert(MANIFEST_FILE_NAME.to_string(), manifest.leak().as_bytes());

        let error = EmbeddedProfilePackage::from_files(files)
            .unwrap_err()
            .to_string();
        assert!(error.contains("synthetic-workflow"), "{error}");
        assert!(error.contains("cannot depend on itself"), "{error}");
    }

    #[test]
    fn test_validation_rejects_unsafe_paths_and_invalid_semantics() {
        let mut files = package().files.clone();
        let unsafe_manifest = manifest_text().replace(
            "target = \"docs/workflow.txt\"",
            "target = \"../outside.txt\"",
        );
        files.insert(
            MANIFEST_FILE_NAME.to_string(),
            unsafe_manifest.leak().as_bytes(),
        );
        assert!(matches!(
            EmbeddedProfilePackage::from_files(files),
            Err(ProfilePackageError::UnsafePath { .. })
        ));

        let windows_absolute = manifest_text().replace(
            "target = \"docs/workflow.txt\"",
            "target = \"C:/outside.txt\"",
        );
        let mut files = package().files.clone();
        files.insert(
            MANIFEST_FILE_NAME.to_string(),
            windows_absolute.leak().as_bytes(),
        );
        assert!(matches!(
            EmbeddedProfilePackage::from_files(files),
            Err(ProfilePackageError::UnsafePath { .. })
        ));

        let bad_map = parse_modified("value = 1", "value = \"one\"").unwrap();
        let files = package().files.clone();
        assert!(matches!(
            validate_manifest(&bad_map, &files),
            Err(ProfilePackageError::InvalidContribution { index: 0, .. })
        ));

        let bad_key = parse_modified(
            "value = { key = \"synthetic-review\", title = \"Synthetic review\" }",
            "value = { key = \"other-review\", title = \"Synthetic review\" }",
        )
        .unwrap();
        assert!(matches!(
            validate_manifest(&bad_key, &files),
            Err(ProfilePackageError::InvalidContribution { index: 6, .. })
        ));
    }

    #[test]
    fn test_validation_rejects_invalid_identity_version_and_compatibility() {
        let package = package();
        let cases = [
            (
                "id = \"synthetic-workflow\"",
                "id = \"Synthetic Workflow\"",
                "invalid profile id",
            ),
            (
                "version = \"1.2.3\"",
                "version = \"01.2.3\"",
                "invalid profile version",
            ),
            (
                "jit = \">=0.2.0, <2.0.0\"",
                "jit = \"not a requirement\"",
                "invalid compatible JIT requirement",
            ),
            (
                "manifest-version = 1",
                "manifest-version = 2",
                "unsupported profile manifest version",
            ),
        ];

        for (old, new, expected) in cases {
            let manifest = manifest_text().replace(old, new);
            let mut files = package.files.clone();
            files.insert(MANIFEST_FILE_NAME.to_string(), manifest.leak().as_bytes());
            let error = EmbeddedProfilePackage::from_files(files)
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn test_validation_rejects_missing_extra_and_duplicate_content() {
        let package = package();

        let mut missing = package.files.clone();
        missing.remove("assets/workflow.txt");
        assert!(matches!(
            EmbeddedProfilePackage::from_files(missing),
            Err(ProfilePackageError::MissingContent(path)) if path == "assets/workflow.txt"
        ));

        let mut extra = package.files.clone();
        extra.insert("assets/undeclared.txt".to_string(), b"extra");
        assert!(matches!(
            EmbeddedProfilePackage::from_files(extra),
            Err(ProfilePackageError::ExtraContent(path)) if path == "assets/undeclared.txt"
        ));

        let duplicate_manifest = manifest_text().replace(
            "source = \"nested/scripts/check.sh\"",
            "source = \"assets/workflow.txt\"",
        );
        let mut duplicate = package.files.clone();
        duplicate.insert(
            MANIFEST_FILE_NAME.to_string(),
            duplicate_manifest.leak().as_bytes(),
        );
        assert!(matches!(
            EmbeddedProfilePackage::from_files(duplicate),
            Err(ProfilePackageError::DuplicateSource(path)) if path == "assets/workflow.txt"
        ));

        let reserved_manifest = manifest_text().replace(
            "source = \"nested/scripts/check.sh\"",
            "source = \"manifest.toml\"",
        );
        let mut reserved = package.files.clone();
        reserved.insert(
            MANIFEST_FILE_NAME.to_string(),
            reserved_manifest.leak().as_bytes(),
        );
        assert!(matches!(
            EmbeddedProfilePackage::from_files(reserved),
            Err(ProfilePackageError::DuplicateSource(path)) if path == "manifest.toml"
        ));

        let duplicate_contribution = manifest_text().replace(
            "[[asset]]\nsource = \"assets/workflow.txt\"",
            "[[contribution]]\nkind = \"projection\"\nname = \"invariants\"\nvalue = { kind = \"invariant\", mode = \"separate-file\", target = \"OTHER.md\", style = \"full\" }\n\n[[asset]]\nsource = \"assets/workflow.txt\"",
        );
        let mut duplicate = package.files.clone();
        duplicate.insert(
            MANIFEST_FILE_NAME.to_string(),
            duplicate_contribution.leak().as_bytes(),
        );
        assert!(matches!(
            EmbeddedProfilePackage::from_files(duplicate),
            Err(ProfilePackageError::DuplicateContribution(identity))
                if identity.contains("invariants")
        ));
    }

    #[test]
    fn test_validation_rejects_package_count_and_size_over_budget() {
        let package = package();

        let mut too_many = package.files.clone();
        for index in 0..MAX_EMBEDDED_PROFILE_FILES {
            too_many.insert(format!("undeclared/{index}.txt"), b"x");
        }
        assert!(matches!(
            EmbeddedProfilePackage::from_files(too_many),
            Err(ProfilePackageError::PackageBounds {
                file_count,
                ..
            }) if file_count > MAX_EMBEDDED_PROFILE_FILES
        ));

        let oversized: &'static [u8] = Vec::leak(vec![b'x'; MAX_EMBEDDED_PROFILE_BYTES]);
        let mut too_large = package.files.clone();
        too_large.insert("undeclared/oversized.bin".to_string(), oversized);
        assert!(matches!(
            EmbeddedProfilePackage::from_files(too_large),
            Err(ProfilePackageError::PackageBounds {
                byte_size,
                ..
            }) if byte_size > MAX_EMBEDDED_PROFILE_BYTES
        ));
    }

    #[test]
    fn test_embedding_dependency_contract_has_no_optional_features_and_production_tree() {
        let manifest = include_str!("../../Cargo.toml");
        assert!(
            manifest.contains("include_dir = { version = \"0.7.4\", default-features = false }"),
            "embedding dependency must remain pinned without optional features"
        );
        let production_tree =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/jit-dogfood");
        assert!(production_tree.join(MANIFEST_FILE_NAME).is_file());
        assert!(!manifest_text().contains("jit-dogfood"));
    }
}
