//! Immutable profile-package domain model and embedded-package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod manifest;
mod package;

pub use manifest::{
    profile_manifest_schema, AssetDeclaration, CompleteProjectionConfig, Contribution,
    KeyedArrayTarget, MapEntryTarget, ProfileManifest, ProfileMetadata, RegionDeclaration,
    RegionPlacement, SetStringTarget, SingletonTableTarget, MANIFEST_FILE_NAME,
    PROFILE_MANIFEST_VERSION,
};
pub use package::{
    EmbeddedProfilePackage, PackageHash, ProfilePackageError, ProfilePackageHashes,
    MAX_EMBEDDED_PROFILE_BYTES, MAX_EMBEDDED_PROFILE_FILES,
};
