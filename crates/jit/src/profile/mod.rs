//! Immutable profile-package domain model and embedded-package validation.
//!
//! This module owns the generic, declarative package contract. It deliberately
//! contains no profile discovery, repository loading, application lifecycle, or
//! production package inventory.

mod application;
mod dogfood;
mod drift;
mod manifest;
mod package;
mod planner;
mod preset;
mod render;
mod snapshot;

pub use crate::domain::ProfileOrigin;
pub use application::{
    append_profile_event_image, AppliedProfileRecord, ProfileApplicationStatus,
    ProfileApplicationWarning, ProfileApplyResult, ProfileListResult, ProfilePlanResult,
    ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction, ProfileTargetChange,
};
pub use dogfood::{
    jit_dogfood_gate, jit_dogfood_live_projection, jit_dogfood_package,
    jit_dogfood_planning_gate_keys, DogfoodProfileError, JIT_DOGFOOD_LIVE_SOURCE_PREFIX,
};
pub use drift::{compare_projection_tree, DriftFinding, DriftKind, ProjectionDriftError};
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
pub use planner::{
    plan_profile_application, plan_profile_application_against, PlanIdentity, PlannedTarget,
    PlannedTargetAction, ProfileApplicationPlan, ProfilePlanError,
};
pub use preset::{
    compare_preset_inventory, derive_preset_projection, PresetInventory, PresetInventoryFinding,
    PresetInventoryKind, PresetProjection,
};
pub use render::{
    project_package, render_managed_region, write_projection_tree, PackageProjection,
    ProjectedFile, ProjectedFileMode, ProjectionError,
};
pub use snapshot::{RepositorySnapshot, SnapshotEntry, SnapshotError, SnapshotFile};
