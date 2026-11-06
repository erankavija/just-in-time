//! Immutable profile-package model and package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod application;
mod apply_claims;
mod difference;
mod graph;
mod manifest;
mod package;
pub mod package_archive;
pub mod package_capture;
mod repository_package;
#[cfg(test)]
pub(crate) mod selection_probe;
mod variables;
mod wire;
// Repository-local generator seam: the render of this repository's generated
// registry regions has two consumers, the artifact registry the `regenerate`
// example publishes through and the `package-drift` example's check, and no
// production caller. Both build with dev-dependencies active, which turns on
// `test-support` through the crate's own self-edge, so gating the module on that
// pair keeps it out of an adopter build entirely.
#[cfg(any(test, feature = "test-support"))]
pub mod registry_region;

pub(crate) use application::ProfileRunMode;
pub use application::{
    ProfileAddEntry, ProfileAddResult, ProfileApplicationStatus, ProfileApplyResult,
    ProfileCollection, ProfileComposedApplyResult, ProfileContributionChange, ProfilePackEntry,
    ProfilePackResult, ProfileShowEntry, ProfileShowResult, ProfileTargetAction,
    ProfileTargetChange, ProfilesKey,
};
pub use apply_claims::{
    build_profile_claims, build_profile_claims_from_resolved, build_resolved_contribution_claims,
    ProfileClaimError,
};
pub(crate) use difference::{ensure_publishable_targets, profile_run_entry};
pub use graph::{EngineVersion, ProfileGraphError, ResolvedProfileGraph};
pub use manifest::{
    profile_package_model_schema, AssetDeclaration, EnvironmentVariableName, LiveSourceDeclaration,
    ProfileDependencyRequirement, ProfileIncompatibility, ProfilePackageModel,
    ProfileVariableDeclaration, ProfileVariableName, RegionDeclaration, RegionId, RegionPlacement,
    LIVE_ASSET_SOURCE_PREFIX, MANIFEST_FILE_NAME,
};
pub use package::{
    PackageHash, ProfilePackage, ProfilePackageError, MAX_PROFILE_PACKAGE_BYTES,
    MAX_PROFILE_PACKAGE_FILES,
};
pub use package_archive::{
    pack_package_archive, read_package_archive, PackageArchiveError,
    MAX_PROFILE_PACKAGE_ARCHIVE_BYTES, MAX_PROFILE_PACKAGE_ARCHIVE_ENTRIES,
};
pub use package_capture::{capture_package_tree, PackageCaptureError};
pub use variables::{
    resolve_package, resolve_variables, ProfileVariableAssignment, ResolvedProfileContent,
    ResolvedVariable, ResolvedVariables, VariableError, VariableInputs, VariableSource,
};
