//! Immutable profile-package model and package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod application;
mod apply_claims;
mod manifest;
mod package;
mod repository_package;
mod wire;
// Repository-local generator seam: the package tree this repository assembles
// is produced by one entry point and read back by nothing, so the render has no
// production caller. It builds with dev-dependencies active, which turns on
// `test-support` through the crate's own self-edge, so gating the module on
// that pair keeps it out of an adopter build entirely.
#[cfg(any(test, feature = "test-support"))]
pub mod package_assembly;
// Repository-local check seam: this checkout's packaged contributions bound to
// the registry entries they restate, plus the report shape that and the
// template-region drift assertion share. Neither has a production caller, and
// both build with dev-dependencies active, so gating them on the same pair
// keeps them out of an adopter build entirely.
#[cfg(any(test, feature = "test-support"))]
pub mod contribution_drift;
#[cfg(any(test, feature = "test-support"))]
pub mod drift_report;
// Repository-local generator seam: the render of this repository's generated
// template region has two consumers, the artifact registry the `regenerate`
// example publishes through and the repository package module's drift
// assertion, and no production caller. Both build
// with dev-dependencies active, which turns on `test-support` through the
// crate's own self-edge, so gating the module on that pair keeps it out of an
// adopter build entirely rather than shipping it as unreachable surface.
#[cfg(any(test, feature = "test-support"))]
pub mod template_region;

pub use crate::domain::repository_inputs::{DeclaredRoot, ExclusionPattern};
pub use crate::domain::ProfileOrigin;
pub use application::{
    ProfileApplicationStatus, ProfileApplicationWarning, ProfileApplyResult,
    ProfileComposedApplyResult, ProfileListResult, ProfilePlanResult, ProfilePlanStatus,
    ProfileShowEntry, ProfileShowResult, ProfileSummary, ProfileTargetAction, ProfileTargetChange,
};
pub use apply_claims::{build_profile_claims, build_profile_repair_claims, ProfileClaimError};
pub use manifest::{
    profile_package_model_schema, AssetDeclaration, LiveSourceDeclaration,
    ProfileDependencyRequirement, ProfileId, ProfileIncompatibility, ProfilePackageModel,
    ProfileVariableDeclaration, RegionDeclaration, RegionPlacement, MANIFEST_FILE_NAME,
};
pub use package::{
    PackageHash, ProfilePackage, ProfilePackageError, ProfilePackageHashes, ProfilePackageSource,
    MAX_PROFILE_PACKAGE_BYTES, MAX_PROFILE_PACKAGE_FILES,
};
pub use repository_package::JIT_DOGFOOD_LIVE_SOURCE_PREFIX;
