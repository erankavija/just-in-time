//! Immutable profile-package model and package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod application;
mod apply_claims;
mod dogfood;
mod manifest;
mod package;
// Repository-local generator seam: the render of this repository's generated
// template region has two consumers, the artifact registry the `regenerate`
// example publishes through and the dogfood module's drift assertion, and no
// production caller. Both build
// with dev-dependencies active, which turns on `test-support` through the
// crate's own self-edge, so gating the module on that pair keeps it out of an
// adopter build entirely rather than shipping it as unreachable surface.
#[cfg(any(test, feature = "test-support"))]
pub mod template_region;

pub use crate::domain::ProfileOrigin;
pub use application::{
    ProfileApplicationStatus, ProfileApplicationWarning, ProfileApplyResult, ProfileListResult,
    ProfilePlanResult, ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction,
    ProfileTargetChange,
};
pub use apply_claims::{build_profile_claims, build_profile_repair_claims, ProfileClaimError};
pub use dogfood::{
    jit_dogfood_gate, jit_dogfood_package, jit_dogfood_planning_gate_keys, DogfoodProfileError,
    JIT_DOGFOOD_LIVE_SOURCE_PREFIX,
};
pub use manifest::{
    profile_manifest_schema, AssetDeclaration, ProfileId, ProfileManifest, ProfileMetadata,
    RegionDeclaration, RegionPlacement, MANIFEST_FILE_NAME, PROFILE_MANIFEST_VERSION,
};
pub use package::{
    PackageHash, ProfilePackage, ProfilePackageError, ProfilePackageHashes, ProfilePackageSource,
    MAX_PROFILE_PACKAGE_BYTES, MAX_PROFILE_PACKAGE_FILES,
};
