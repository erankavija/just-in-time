//! Render of this repository's generated registry regions.
//!
//! This repository declares its workflow package's templates and invariants
//! twice: as contributions of the `jit-dogfood` package, and in its own
//! registry files. The package is the authority, so each registry file's copy
//! is a generated region between delimiters and every byte outside them is
//! authored (`@/issue/e204e63d/decision/D-1`).
//!
//! Everything here is pure. The module compiles only for the crate's own tests
//! and dev-dependency-active builds, so an adopter build carries none of it.

use crate::declarations::invariants::{
    serialize_invariant_registry, Invariant, InvariantConfigError, InvariantRegistry,
};
use crate::profile::ProfilePackage;
use crate::repository_state::{
    render_managed_document, Contribution, KeyedArrayTarget, ManagedDocumentClaim,
    ManagedDocumentError, RegionPlacement, VirtualPath,
};
use crate::templates::GraphTemplate;
use serde::de::DeserializeOwned;

/// The package whose contributions the generated regions hold, and which owns
/// each region claim.
pub const REGION_PACKAGE_ID: &str = "jit-dogfood";

/// The command that brings `.jit/templates.toml` back into agreement.
pub const TEMPLATE_REGION_GENERATOR: &str = "./scripts/generate-template-region.sh";

/// The command that brings `.jit/invariants.toml` back into agreement.
pub const INVARIANT_REGION_GENERATOR: &str = "./scripts/generate-invariant-region.sh";

/// One registry file's generated region.
#[derive(Debug, Clone)]
pub struct RegistryRegion {
    /// Identity of the region claim.
    id: &'static str,
    /// Begin delimiter, in TOML's own comment syntax.
    begin: &'static str,
    /// End delimiter, in TOML's own comment syntax.
    end: &'static str,
    /// The registry file that carries the region.
    pub registry: VirtualPath,
    /// The command that brings the registry back into agreement with the package.
    pub generator: &'static str,
    /// The packaged declarations serialized as a registry-file block.
    block: fn(&ProfilePackage) -> Result<String, RegistryRegionError>,
}

/// The region of `.jit/templates.toml`.
pub const TEMPLATE_REGION: RegistryRegion = RegistryRegion {
    id: "plan-template",
    begin: "# jit:plan-template:begin",
    end: "# jit:plan-template:end",
    registry: VirtualPath::TEMPLATES,
    generator: TEMPLATE_REGION_GENERATOR,
    block: template_block,
};

/// The region of `.jit/invariants.toml`.
pub const INVARIANT_REGION: RegistryRegion = RegistryRegion {
    id: "packaged-invariants",
    begin: "# jit:packaged-invariants:begin",
    end: "# jit:packaged-invariants:end",
    registry: VirtualPath::INVARIANTS,
    generator: INVARIANT_REGION_GENERATOR,
    block: invariant_block,
};

/// A render of a generated region that could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum RegistryRegionError {
    /// A packaged contribution does not parse as its registry's declaration.
    #[error("a packaged contribution is not a registry declaration: {0}")]
    Contribution(#[from] serde_json::Error),
    /// The packaged templates do not serialize as TOML.
    #[error("the packaged declarations do not serialize as TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// The packaged invariants do not serialize.
    #[error("the packaged invariants do not serialize: {0}")]
    Invariants(#[from] InvariantConfigError),
    /// The registry does not carry exactly one well-formed region.
    #[error("the registry does not carry the generated region: {0}")]
    Splice(#[from] ManagedDocumentError),
    /// The registry is not UTF-8.
    #[error("the registry is not UTF-8")]
    NotUtf8,
    /// A delimiter is absent from the registry.
    #[error("the registry declares no '{0}' delimiter")]
    MissingDelimiter(&'static str),
}

fn contributed<T: DeserializeOwned>(
    package: &ProfilePackage,
    registry: KeyedArrayTarget,
) -> Result<Vec<T>, RegistryRegionError> {
    package
        .model()
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::KeyedArray { target, value, .. } if *target == registry => {
                Some(value.clone())
            }
            _ => None,
        })
        .map(|value| Ok(serde_json::from_value(value)?))
        .collect()
}

fn template_block(package: &ProfilePackage) -> Result<String, RegistryRegionError> {
    #[derive(serde::Serialize)]
    struct TemplateBlock {
        template: Vec<GraphTemplate>,
    }
    Ok(toml::to_string(&TemplateBlock {
        template: contributed(package, KeyedArrayTarget::Templates)?,
    })?)
}

fn invariant_block(package: &ProfilePackage) -> Result<String, RegistryRegionError> {
    let invariants: Vec<Invariant> = contributed(package, KeyedArrayTarget::Invariants)?;
    let bytes = serialize_invariant_registry(&InvariantRegistry { invariants })?;
    String::from_utf8(bytes).map_err(|_| RegistryRegionError::NotUtf8)
}

impl RegistryRegion {
    /// The registry bytes `package`'s declarations render to, given the
    /// registry's current bytes: the authored text outside the delimiters, with
    /// the packaged block between them.
    ///
    /// The region must already exist: a registry whose delimiters are absent,
    /// duplicated, or out of order is reported rather than appended to.
    pub fn render(
        &self,
        existing: &[u8],
        package: &ProfilePackage,
    ) -> Result<Vec<u8>, RegistryRegionError> {
        Ok(render_managed_document(
            existing,
            &[ManagedDocumentClaim::Region {
                owner: REGION_PACKAGE_ID.into(),
                region_id: self.id.into(),
                begin: self.begin.as_bytes().to_vec(),
                end: self.end.as_bytes().to_vec(),
                content: (self.block)(package)?.into_bytes(),
                placement: RegionPlacement::RequireExisting,
            }],
        )?)
    }

    /// The registry bytes outside the region: everything through the begin
    /// delimiter, and everything from the end delimiter onward.
    pub fn outside(&self, registry: &[u8]) -> Result<(String, String), RegistryRegionError> {
        let text = std::str::from_utf8(registry).map_err(|_| RegistryRegionError::NotUtf8)?;
        let begin = text
            .find(self.begin)
            .ok_or(RegistryRegionError::MissingDelimiter(self.begin))?
            + self.begin.len();
        let end = text
            .find(self.end)
            .ok_or(RegistryRegionError::MissingDelimiter(self.end))?;
        Ok((text[..begin].to_string(), text[end..].to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever a region holds, the packaged block replaces it whole, the
    /// authored bytes around it survive, and rendering again changes nothing.
    #[test]
    fn test_registry_region_render_replaces_only_the_region_and_reaches_a_fixed_point() {
        let (_workspace, package) =
            crate::test_utils::temporary_repository_package(REGION_PACKAGE_ID);
        for region in [TEMPLATE_REGION, INVARIANT_REGION] {
            let authored = format!(
                "# authored before\n{}\n# a declaration the package does not make\n{}\n# after\n",
                region.begin, region.end
            );

            let rendered = region.render(authored.as_bytes(), &package).unwrap();

            assert_eq!(
                region.outside(&rendered).unwrap(),
                region.outside(authored.as_bytes()).unwrap()
            );
            assert!(!String::from_utf8_lossy(&rendered).contains("does not make"));
            assert_eq!(region.render(&rendered, &package).unwrap(), rendered);
        }
    }

    #[test]
    fn test_registry_region_render_refuses_a_registry_without_the_region() {
        let (_workspace, package) =
            crate::test_utils::temporary_repository_package(REGION_PACKAGE_ID);
        for region in [TEMPLATE_REGION, INVARIANT_REGION] {
            assert!(matches!(
                region.render(b"# no delimiters\n", &package),
                Err(RegistryRegionError::Splice(_))
            ));
        }
    }

    #[test]
    fn test_invariant_region_block_parses_back_to_the_packaged_invariants() {
        let (_workspace, package) =
            crate::test_utils::temporary_repository_package(REGION_PACKAGE_ID);
        let packaged: Vec<Invariant> = contributed(&package, KeyedArrayTarget::Invariants).unwrap();

        let reloaded =
            InvariantRegistry::from_toml_str(&invariant_block(&package).unwrap()).unwrap();

        assert_eq!(reloaded.invariants, packaged);
        assert!(!packaged.is_empty());
    }
}
