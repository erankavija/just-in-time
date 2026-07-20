//! Immutable profile-package domain model and embedded-package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod application;
mod apply_claims;
mod dogfood;
mod manifest;
mod package;

pub use crate::domain::ProfileOrigin;
pub use application::{
    append_profile_event_image, AppliedProfileRecord, ProfileApplicationStatus,
    ProfileApplicationWarning, ProfileApplyResult, ProfileListResult, ProfilePlanResult,
    ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction, ProfileTargetChange,
};
pub use apply_claims::{build_profile_claims, ProfileClaimError};
pub use dogfood::{
    jit_dogfood_gate, jit_dogfood_package, jit_dogfood_planning_gate_keys, DogfoodProfileError,
    JIT_DOGFOOD_LIVE_SOURCE_PREFIX,
};
pub use manifest::{
    profile_manifest_schema, AssetDeclaration, CompleteProjectionConfig, Contribution,
    KeyedArrayTarget, MapEntryTarget, ProfileManifest, ProfileMetadata, RegionDeclaration,
    RegionPlacement, SetStringTarget, MANIFEST_FILE_NAME, PROFILE_MANIFEST_VERSION,
};
pub use package::{
    EmbeddedProfilePackage, PackageHash, ProfilePackageError, ProfilePackageHashes,
    MAX_EMBEDDED_PROFILE_BYTES, MAX_EMBEDDED_PROFILE_FILES,
};
