//! Render of this repository's generated graph-template region.
//!
//! This repository declares the plan-before-fan-out bracket twice: once as a
//! template contribution of the `jit-dogfood` package, and once in its own
//! `.jit/templates.toml`. The package is the authority, so the registry file's
//! declaration is a generated region bounded by [`TEMPLATE_REGION_BEGIN`] /
//! [`TEMPLATE_REGION_END`] and everything outside those delimiters is authored
//! (`@/issue/e204e63d/decision/D-1`).
//!
//! Everything here is pure: the functions render bytes, compare parsed
//! declarations, and read nothing from the filesystem. The package arrives as
//! an argument, and the two callers supply the I/O around it —
//! [`crate::generated_artifacts`] declares the render the `regenerate` example
//! publishes the registry file through, and the repository package module's
//! drift assertions read it. The module compiles only for the crate's own tests
//! and dev-dependency-active builds those two need, so an adopter build carries
//! none of it.

use crate::profile::ProfilePackage;
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

/// Root of the field paths the comparison reports, naming the declarations the
/// way the registry file spells them (`[[template]]`).
const TEMPLATE_ARRAY_PATH: &str = "template";

/// Repo-relative path of the registry file that carries the generated region.
pub const TEMPLATE_REGISTRY_PATH: &str = ".jit/templates.toml";

/// The command that brings [`TEMPLATE_REGISTRY_PATH`] back into agreement with
/// the package, named in the registry file and in the drift assertion's message.
pub const TEMPLATE_REGION_GENERATOR: &str = "./scripts/generate-template-region.sh";

/// A render of the generated template region that could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum TemplateRegionError {
    /// A packaged template contribution does not parse as a graph template.
    #[error("a packaged template contribution is not a graph template: {0}")]
    Contribution(#[from] serde_json::Error),
    /// A parsed declaration does not serialize into the values the field-by-field
    /// comparison walks.
    #[error("a parsed template declaration does not serialize for comparison: {0}")]
    Compare(#[source] serde_json::Error),
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

/// The template contributions `package` declares, typed as the model the
/// repository's template registry parses its declarations into.
pub fn packaged_templates(
    package: &ProfilePackage,
) -> Result<Vec<GraphTemplate>, TemplateRegionError> {
    use crate::repository_state::{Contribution, KeyedArrayTarget};
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

/// The registry text and the bounds of its generated region: where the begin
/// delimiter ends, and where the end delimiter starts.
fn template_region_bounds(registry: &[u8]) -> Result<(&str, usize, usize), TemplateRegionError> {
    let text = std::str::from_utf8(registry).map_err(|_| TemplateRegionError::NotUtf8)?;
    let begin = text
        .find(TEMPLATE_REGION_BEGIN)
        .ok_or(TemplateRegionError::MissingDelimiter(TEMPLATE_REGION_BEGIN))?
        + TEMPLATE_REGION_BEGIN.len();
    let end = text
        .find(TEMPLATE_REGION_END)
        .ok_or(TemplateRegionError::MissingDelimiter(TEMPLATE_REGION_END))?;
    Ok((text, begin, end))
}

/// The registry bytes outside the generated region: everything through the
/// begin delimiter, and everything from the end delimiter onward.
pub fn outside_template_region(registry: &[u8]) -> Result<(String, String), TemplateRegionError> {
    let (text, begin, end) = template_region_bounds(registry)?;
    Ok((text[..begin].to_string(), text[end..].to_string()))
}

/// The registry bytes between the delimiters: the generated region's own
/// declarations, which is the extent the package is the authority for
/// (`@/issue/e204e63d/decision/D-1`). A template the repository authors outside
/// the delimiters is not part of it.
pub fn inside_template_region(registry: &[u8]) -> Result<String, TemplateRegionError> {
    let (text, begin, end) = template_region_bounds(registry)?;
    Ok(text[begin..end].to_string())
}

/// The registry bytes `package`'s declarations render to, given the registry's
/// current bytes: the authored text outside the delimiters, with the packaged
/// block between them.
pub fn render_template_registry(
    existing: &[u8],
    package: &ProfilePackage,
) -> Result<Vec<u8>, TemplateRegionError> {
    splice_template_region(
        existing,
        &render_template_block(&packaged_templates(package)?)?,
    )
}

/// How the declarations of the repository's generated region disagree with the
/// packaged ones, or `None` when every field agrees.
///
/// The comparison walks the two sides as the values they parse into, so it
/// holds no expectation of its own about what either declares and covers every
/// field the model carries, description strings included. Each reported line
/// names a field by its path through the declaration and shows both values, and
/// the report closes with [`TEMPLATE_REGION_GENERATOR`], which renders the
/// repository's region from the packaged declarations.
///
/// # Errors
///
/// [`TemplateRegionError::Compare`] when a parsed declaration does not
/// serialize into the compared values.
pub fn template_drift_report(
    repository: &[GraphTemplate],
    packaged: &[GraphTemplate],
) -> Result<Option<String>, TemplateRegionError> {
    let compared = |declarations: &[GraphTemplate]| {
        serde_json::to_value(declarations).map_err(TemplateRegionError::Compare)
    };
    let differences = value_differences(
        TEMPLATE_ARRAY_PATH,
        &compared(repository)?,
        &compared(packaged)?,
    );
    Ok((!differences.is_empty()).then(|| {
        format!(
            "the repository's template declarations disagree with the packaged ones:\n  {}\n\
             the generated region of .jit/templates.toml holds the packaged declarations: \
             regenerate it with {TEMPLATE_REGION_GENERATOR}",
            differences.join("\n  ")
        )
    }))
}

/// Every field at which two compared values differ, each named by its path
/// through the declaration and shown from both sides.
///
/// Objects are compared over the union of their keys and arrays position by
/// position, so a field one side omits and a position one side does not reach
/// are both reported where they belong rather than collapsing the whole
/// declaration into one difference.
fn value_differences(
    path: &str,
    repository: &serde_json::Value,
    packaged: &serde_json::Value,
) -> Vec<String> {
    use serde_json::Value;
    match (repository, packaged) {
        (left, right) if left == right => Vec::new(),
        (Value::Object(left), Value::Object(right)) => left
            .keys()
            .chain(right.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .flat_map(|key| {
                member_differences(&format!("{path}.{key}"), left.get(key), right.get(key))
            })
            .collect(),
        (Value::Array(left), Value::Array(right)) => (0..left.len().max(right.len()))
            .flat_map(|index| {
                member_differences(
                    &format!("{path}[{index}]"),
                    left.get(index),
                    right.get(index),
                )
            })
            .collect(),
        (left, right) => vec![format!(
            "{path}: the repository declares {left}, the package declares {right}"
        )],
    }
}

/// The differences at one field path, where either side may be absent.
fn member_differences(
    path: &str,
    repository: Option<&serde_json::Value>,
    packaged: Option<&serde_json::Value>,
) -> Vec<String> {
    match (repository, packaged) {
        (Some(left), Some(right)) => value_differences(path, left, right),
        (Some(left), None) => vec![format!(
            "{path}: the repository declares {left}, the package declares nothing"
        )],
        (None, Some(right)) => vec![format!(
            "{path}: the repository declares nothing, the package declares {right}"
        )],
        (None, None) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A declaration the repository carries and the package does not is
    /// reported at its own position, naming the side that lacks it.
    #[test]
    fn test_template_drift_report_names_a_template_only_the_repository_declares() {
        let (_workspace, package) = crate::test_utils::temporary_repository_package("jit-dogfood");
        let packaged = packaged_templates(&package).unwrap();
        let repository = [packaged.clone(), packaged.clone()].concat();
        let report = template_drift_report(&repository, &packaged)
            .unwrap()
            .expect("a repository declaring a template the package does not is drift");

        assert!(
            report.contains(&format!("template[{}]", packaged.len())),
            "{report}"
        );
        assert!(report.contains("the package declares nothing"), "{report}");
    }
}
