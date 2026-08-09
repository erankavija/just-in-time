//! Convert an immutable profile package into image-independent repository-state claims.

use super::{resolve_package, ProfilePackage, ResolvedProfileContent, VariableInputs};
use crate::repository_state::{
    FileMode, ProfileAssetClaim, ProfileClaims, ProfilePackageId, ProfileRegionClaim, TargetClaim,
};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ProfileClaimError {
    #[error("profile targets '{first}' and '{second}' overlap")]
    OverlappingTargets { first: String, second: String },
    #[error(transparent)]
    Variable(#[from] super::VariableError),
    #[error("declared package source '{0}' is unavailable")]
    MissingSource(String),
    #[error("profile target path is not canonical: {0}")]
    Layout(#[from] crate::repository_state::RepositoryLayoutError),
    #[error("invalid profile ownership claim: {0}")]
    Claim(#[from] crate::repository_state::DeltaError),
}

pub fn build_profile_claims(
    package: &ProfilePackage,
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<ProfileClaims, ProfileClaimError> {
    let resolved = resolve_package(package, &VariableInputs::default())?;
    build_profile_claims_from_resolved(&resolved, layout, false)
}

pub fn build_profile_repair_claims(
    package: &ProfilePackage,
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<ProfileClaims, ProfileClaimError> {
    let resolved = resolve_package(package, &VariableInputs::default())?;
    build_profile_claims_from_resolved(&resolved, layout, true)
}

/// Convert already-resolved package content into repository-state claims.
pub fn build_profile_claims_from_resolved(
    package: &ResolvedProfileContent,
    layout: &crate::repository_state::RepositoryLayout,
    replace_owned: bool,
) -> Result<ProfileClaims, ProfileClaimError> {
    validate_target_overlaps(package)?;
    let assets = package
        .model()
        .assets
        .iter()
        .map(|asset| {
            let bytes = package
                .source_bytes(&asset.source)
                .ok_or_else(|| ProfileClaimError::MissingSource(asset.source.clone()))?
                .to_vec();
            let target = layout.classify_repository_relative(&asset.target)?;
            Ok(ProfileAssetClaim {
                claim: TargetClaim::new(layout, target, format!("profile-asset:{}", asset.target))?,
                bytes,
                mode: if asset.executable {
                    FileMode::Executable
                } else {
                    FileMode::Regular
                },
                replace_owned,
            })
        })
        .collect::<Result<Vec<_>, ProfileClaimError>>()?;
    let regions = package
        .model()
        .regions
        .iter()
        .map(|region| {
            let content = package
                .source_bytes(&region.source)
                .ok_or_else(|| ProfileClaimError::MissingSource(region.source.clone()))?
                .to_vec();
            let target = layout.classify_repository_relative(&region.target)?;
            Ok(ProfileRegionClaim {
                claim: TargetClaim::new(
                    layout,
                    target,
                    format!("profile-region:{}", region.region_id),
                )?,
                region_id: region.region_id.clone(),
                content,
            })
        })
        .collect::<Result<Vec<_>, ProfileClaimError>>()?;
    Ok(ProfileClaims {
        package_id: ProfilePackageId::new(package.model().id.to_string()),
        contributions: if replace_owned {
            Vec::new()
        } else {
            package.model().contributions.clone()
        },
        assets,
        regions,
    })
}

fn validate_target_overlaps(package: &ResolvedProfileContent) -> Result<(), ProfileClaimError> {
    let semantic = package
        .model()
        .contributions
        .iter()
        .map(|contribution| contribution.registry_path().to_string())
        .collect::<BTreeSet<_>>();
    let content = package
        .model()
        .assets
        .iter()
        .map(|asset| asset.target.clone())
        .chain(
            package
                .model()
                .regions
                .iter()
                .map(|region| region.target.clone()),
        )
        .collect::<BTreeSet<_>>();
    if let Some(target) = semantic.intersection(&content).next() {
        return Err(ProfileClaimError::OverlappingTargets {
            first: target.clone(),
            second: target.clone(),
        });
    }
    let mut targets = semantic.iter().chain(content.iter()).collect::<Vec<_>>();
    targets.sort();
    for pair in targets.windows(2) {
        if Path::new(pair[1]).starts_with(pair[0]) && pair[1] != pair[0] {
            return Err(ProfileClaimError::OverlappingTargets {
                first: pair[0].clone(),
                second: pair[1].clone(),
            });
        }
    }
    Ok(())
}
