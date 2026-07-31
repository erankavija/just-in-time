//! Render of this repository's generated graph-template region.
//!
//! This repository declares the plan-before-fan-out bracket twice: once as a
//! template contribution of the embedded `jit-dogfood` package, and once in its
//! own `.jit/templates.toml`. The package is the authority, so the registry
//! file's declaration is a generated region bounded by
//! [`TEMPLATE_REGION_BEGIN`] / [`TEMPLATE_REGION_END`] and everything outside
//! those delimiters is authored (`@/issue/e204e63d/decision/D-1`).
//!
//! Everything here is pure: the functions render bytes and read nothing from
//! the filesystem. The two callers supply the I/O — the
//! `render-template-region` example writes the registry file, and the dogfood
//! module's drift assertion reads it — and the module compiles only for the
//! crate's own tests and for the dev-dependency-active builds those two need,
//! so an adopter build carries none of it.

use crate::profile::{jit_dogfood_package, DogfoodProfileError};
use crate::repository_state::{
    render_managed_document, ManagedDocumentClaim, ManagedDocumentError, RegionPlacement,
};
use crate::templates::GraphTemplate;

/// Delimiters of the generated region in `.jit/templates.toml`, written in
/// TOML's own comment syntax.
pub const TEMPLATE_REGION_BEGIN: &str = "# jit:plan-template:begin";
/// End delimiter; see [`TEMPLATE_REGION_BEGIN`].
pub const TEMPLATE_REGION_END: &str = "# jit:plan-template:end";

/// Identity of the region claim, shared by every caller of the splice.
const TEMPLATE_REGION_ID: &str = "plan-template";

/// The command that brings `.jit/templates.toml` back into agreement with the
/// package, named in the registry file and in the drift assertion's message.
pub const TEMPLATE_REGION_GENERATOR: &str = "./scripts/generate-template-region.sh";

/// A render of the generated template region that could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum TemplateRegionError {
    /// The embedded package failed to load.
    #[error(transparent)]
    Package(#[from] DogfoodProfileError),
    /// A packaged template contribution does not parse as a graph template.
    #[error("a packaged template contribution is not a graph template: {0}")]
    Contribution(#[from] serde_json::Error),
    /// The packaged declarations do not serialize as TOML.
    #[error("the packaged template declarations do not serialize as TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// The registry does not carry exactly one well-formed region.
    #[error("the template registry does not carry the generated region: {0}")]
    Splice(#[from] ManagedDocumentError),
    /// The registry is not UTF-8.
    #[error("the template registry is not UTF-8")]
    NotUtf8,
    /// A delimiter is absent from the registry.
    #[error("the template registry declares no '{0}' delimiter")]
    MissingDelimiter(&'static str),
}

/// The packaged template contributions, typed as the model the repository's
/// template registry parses its declarations into.
pub fn packaged_templates() -> Result<Vec<GraphTemplate>, TemplateRegionError> {
    use crate::repository_state::{Contribution, KeyedArrayTarget};
    let package = jit_dogfood_package()?;
    package
        .manifest()
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::KeyedArray {
                target: KeyedArrayTarget::Templates,
                value,
                ..
            } => Some(value.clone()),
            _ => None,
        })
        .map(|value| serde_json::from_value(value).map_err(TemplateRegionError::Contribution))
        .collect()
}

/// The packaged declarations serialized as a registry-file template block.
pub fn render_template_block(templates: &[GraphTemplate]) -> Result<String, TemplateRegionError> {
    #[derive(serde::Serialize)]
    struct TemplateBlock<'a> {
        template: &'a [GraphTemplate],
    }
    toml::to_string(&TemplateBlock {
        template: templates,
    })
    .map_err(TemplateRegionError::Serialize)
}

/// Splice `block` into the registry's generated region, preserving every byte
/// outside the delimiters.
///
/// The region must already exist: a registry whose delimiters are absent,
/// duplicated, or out of order is reported rather than appended to.
pub fn splice_template_region(
    existing: &[u8],
    block: &str,
) -> Result<Vec<u8>, TemplateRegionError> {
    render_managed_document(
        existing,
        &[ManagedDocumentClaim::Region {
            owner: "jit-dogfood".into(),
            region_id: TEMPLATE_REGION_ID.into(),
            begin: TEMPLATE_REGION_BEGIN.as_bytes().to_vec(),
            end: TEMPLATE_REGION_END.as_bytes().to_vec(),
            content: block.as_bytes().to_vec(),
            placement: RegionPlacement::RequireExisting,
        }],
    )
    .map_err(TemplateRegionError::Splice)
}

/// The registry bytes outside the generated region: everything through the
/// begin delimiter, and everything from the end delimiter onward.
pub fn outside_template_region(registry: &[u8]) -> Result<(String, String), TemplateRegionError> {
    let text = std::str::from_utf8(registry).map_err(|_| TemplateRegionError::NotUtf8)?;
    let begin = text
        .find(TEMPLATE_REGION_BEGIN)
        .ok_or(TemplateRegionError::MissingDelimiter(TEMPLATE_REGION_BEGIN))?
        + TEMPLATE_REGION_BEGIN.len();
    let end = text
        .find(TEMPLATE_REGION_END)
        .ok_or(TemplateRegionError::MissingDelimiter(TEMPLATE_REGION_END))?;
    Ok((text[..begin].to_string(), text[end..].to_string()))
}

/// The registry bytes the packaged declarations render to, given the registry's
/// current bytes: the authored text outside the delimiters, with the packaged
/// block between them.
pub fn render_template_registry(existing: &[u8]) -> Result<Vec<u8>, TemplateRegionError> {
    splice_template_region(existing, &render_template_block(&packaged_templates()?)?)
}
