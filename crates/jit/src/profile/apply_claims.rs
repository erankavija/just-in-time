//! Convert an immutable profile package into image-independent repository-state claims.

use super::EmbeddedProfilePackage;
use crate::repository_state::{
    FileMode, ProfileAssetClaim, ProfileClaims, ProfileRegionClaim, TargetClaim,
};
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::path::Path;

const RESERVED_INTERPOLATION_PREFIX: &[u8] = b"{{jit:";

#[derive(Debug, thiserror::Error)]
pub enum ProfileClaimError {
    #[error("profile targets '{first}' and '{second}' overlap")]
    OverlappingTargets { first: String, second: String },
    #[error("profile source '{location}' contains unsupported interpolation token")]
    InvalidInterpolation { location: String },
    #[error("declared embedded source '{0}' is unavailable")]
    MissingSource(String),
    #[error("profile target path is not canonical: {0}")]
    Layout(#[from] crate::repository_state::RepositoryLayoutError),
    #[error("invalid profile ownership claim: {0}")]
    Claim(#[from] crate::repository_state::DeltaError),
}

pub fn build_profile_claims(
    package: &EmbeddedProfilePackage<'_>,
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<ProfileClaims, ProfileClaimError> {
    build_claims(package, layout, false)
}

pub fn build_profile_repair_claims(
    package: &EmbeddedProfilePackage<'_>,
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<ProfileClaims, ProfileClaimError> {
    build_claims(package, layout, true)
}

fn build_claims(
    package: &EmbeddedProfilePackage<'_>,
    layout: &crate::repository_state::RepositoryLayout,
    replace_owned: bool,
) -> Result<ProfileClaims, ProfileClaimError> {
    validate_interpolation(package)?;
    validate_target_overlaps(package)?;
    let assets = package
        .manifest()
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
        .manifest()
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
        contributions: if replace_owned {
            Vec::new()
        } else {
            package.manifest().contributions.clone()
        },
        assets,
        regions,
    })
}

fn validate_interpolation(package: &EmbeddedProfilePackage<'_>) -> Result<(), ProfileClaimError> {
    for source in package
        .manifest()
        .assets
        .iter()
        .map(|asset| asset.source.as_str())
        .chain(
            package
                .manifest()
                .regions
                .iter()
                .map(|region| region.source.as_str()),
        )
    {
        if package
            .source_bytes(source)
            .is_some_and(|bytes| contains_bytes(bytes, RESERVED_INTERPOLATION_PREFIX))
        {
            return Err(ProfileClaimError::InvalidInterpolation {
                location: source.into(),
            });
        }
    }
    for (index, contribution) in package.manifest().contributions.iter().enumerate() {
        if json_contains_interpolation(&serde_json::to_value(contribution).expect("serializes")) {
            return Err(ProfileClaimError::InvalidInterpolation {
                location: format!("contribution[{index}]"),
            });
        }
    }
    Ok(())
}

fn validate_target_overlaps(package: &EmbeddedProfilePackage<'_>) -> Result<(), ProfileClaimError> {
    let semantic = package
        .manifest()
        .contributions
        .iter()
        .map(|contribution| contribution.registry_path().to_string())
        .collect::<BTreeSet<_>>();
    let content = package
        .manifest()
        .assets
        .iter()
        .map(|asset| asset.target.clone())
        .chain(
            package
                .manifest()
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

fn json_contains_interpolation(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(value) => contains_bytes(value.as_bytes(), RESERVED_INTERPOLATION_PREFIX),
        JsonValue::Array(values) => values.iter().any(json_contains_interpolation),
        JsonValue::Object(values) => values.values().any(json_contains_interpolation),
        _ => false,
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
