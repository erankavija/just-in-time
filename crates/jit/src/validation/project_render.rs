//! Render the markdown BODY of a generic `[projection.<name>]` projection.
//!
//! The single wiring between a projection's declared `kind`/`style` and the bytes
//! it produces, shared by the `jit project render` command (over the storage
//! boundary) and the `jit validate` freshness check (over a repository view). It
//! is substrate-agnostic: the caller supplies a `read(path)` closure, so this file
//! performs no direct filesystem I/O and knows nothing about storage vs. an overlay.
//!
//! Two styles, two renderers (see [`ProjectionStyle`]):
//!
//! - **`id-anchor`** — generic and kind-agnostic. Each declared kind's addressable
//!   rows (resolved through the SAME path as `jit item list`: a markdown-first
//!   kind's `source` file is scanned, a registry-first kind's `.toml` is projected
//!   through its descriptor) render as `- **{self-id}** — {text}` bullets via
//!   [`render_id_anchor_rows`]. Any item kind projects this way with no dedicated
//!   code — this is what the charter dogfood exercises.
//! - **`full`** — the built-in rich registry views, whose typed fields (invariant
//!   kind/enforced-by; rule severity/enforcement; gate title) are absent from a
//!   generic row. Selected by the kind's declared registry SOURCE, not its name: a
//!   projection whose kinds' `[item_kinds.<name>].source` is the invariant store
//!   renders the invariant registry, and the rule + gate stores together render the
//!   rule + gate registries — so a repo may rename the equivalent kind freely
//!   (`@/inv/domain-agnostic`). Any other source set under `full` is a typed error.
//!
//! A missing source, an unknown kind, or a non-project kind is a typed
//! [`ProjectionError`] raised BEFORE any body is returned, so the caller never
//! writes a partial or empty-by-accident block (REQ-07).

use crate::config::{JitConfig, ProjectionConfig, ProjectionStyle, SourceOfTruth};
use crate::document::content_parser_for;
use crate::domain::item::{
    derive_scope_items, index_markdown_items, load_toml_scope_items, resolve_item_kinds,
    AddressableItem, ItemKind, Scope,
};
use crate::domain::ContentFormat;
use crate::storage::GateRegistry;
use crate::validation::projection::{
    render_id_anchor_rows, render_invariants_markdown, ProjectionError,
};
use crate::validation::rules::RuleSet;
use crate::validation::rules_gates_projection::render_rules_and_gates_markdown;
use anyhow::Result;
use std::collections::BTreeSet;

/// The loaded registries a projection body render reads besides its source files:
/// the config (item kinds + invariant registry), the effective rules, and the gate
/// registry. The rules/gates are only consulted by the `full` style.
pub struct ProjectionInputs<'a> {
    /// The loaded repository config (item-kind registry + invariant registry).
    pub config: &'a JitConfig,
    /// The effective rule set (for the `full` rule+gate view).
    pub rules: &'a RuleSet,
    /// The gate registry (for the `full` rule+gate view).
    pub gates: &'a GateRegistry,
}

/// Render one projection's markdown body, returning `(body, count)`.
///
/// `read(path)` reads a repo-relative file through the caller's boundary
/// (`Ok(Some(text))` present, `Ok(None)` absent). `count` is the number of rows
/// (id-anchor) or registry entries (full) rendered, for reporting only. Errors are
/// typed [`ProjectionError`]s (unknown kind, missing source, `full` unsupported for
/// the kind set) or the underlying item-index error for a malformed source.
pub fn render_projection_body(
    proj: &ProjectionConfig,
    inputs: &ProjectionInputs,
    read: &mut dyn FnMut(&str) -> Result<Option<String>>,
) -> Result<(String, usize)> {
    let all_kinds = resolve_item_kinds(inputs.config.item_kinds.as_ref())?;
    // Resolve every declared kind name (accepting aliases) to its ItemKind up
    // front, so an unknown kind fails before any source is read (REQ-07).
    let resolved: Vec<&ItemKind> = proj
        .kinds()
        .iter()
        .map(|name| {
            all_kinds
                .iter()
                .find(|kind| kind.matches_name(name))
                .ok_or_else(|| ProjectionError::UnknownKind { kind: name.clone() })
        })
        .collect::<std::result::Result<_, _>>()?;

    match proj.style() {
        ProjectionStyle::IdAnchor => {
            let rows = resolve_rows(&resolved, inputs.config, read)?;
            let count = rows.len();
            Ok((render_id_anchor_rows(&rows), count))
        }
        ProjectionStyle::Full => render_full_body(proj, &resolved, inputs),
    }
}

/// Resolve the addressable rows of every declared kind, in declaration order.
///
/// A markdown-first kind's `source` file is scanned; a registry-first kind's
/// `.toml` descriptor is projected. A declared source that does not exist is a
/// typed [`ProjectionError::SourceNotFound`] (region rendering never falls back to
/// an empty block). A non-project kind is [`ProjectionError::NotProjectScoped`].
fn resolve_rows(
    kinds: &[&ItemKind],
    config: &JitConfig,
    read: &mut dyn FnMut(&str) -> Result<Option<String>>,
) -> Result<Vec<AddressableItem>> {
    let parser = content_parser_for(None, repo_content_format(config)?)?;
    let mut rows = Vec::new();
    for kind in kinds {
        if !kind.kind_scope().is_project() {
            return Err(ProjectionError::NotProjectScoped {
                kind: kind.name().to_string(),
            }
            .into());
        }
        match kind.source_of_truth() {
            SourceOfTruth::MarkdownFirst => {
                // A project markdown-first kind always carries a source path (kind
                // resolution rejects it otherwise); treat a None defensively as a
                // missing source rather than panicking.
                let source = kind
                    .source()
                    .ok_or_else(|| ProjectionError::NotProjectScoped {
                        kind: kind.name().to_string(),
                    })?;
                let markdown = read(source)?.ok_or_else(|| ProjectionError::SourceNotFound {
                    path: source.to_string(),
                    kind: kind.name().to_string(),
                })?;
                rows.extend(index_markdown_items(
                    &markdown,
                    &Scope::Project,
                    std::slice::from_ref(*kind),
                    parser.as_ref(),
                )?);
            }
            SourceOfTruth::RegistryFirst => {
                let descriptor =
                    kind.toml_source()
                        .ok_or_else(|| ProjectionError::NotProjectScoped {
                            kind: kind.name().to_string(),
                        })?;
                let content =
                    read(&descriptor.toml)?.ok_or_else(|| ProjectionError::SourceNotFound {
                        path: descriptor.toml.clone(),
                        kind: kind.name().to_string(),
                    })?;
                let raw = load_toml_scope_items(kind.name(), descriptor, &content)?;
                rows.extend(derive_scope_items(&Scope::Project, raw)?);
            }
        }
    }
    Ok(rows)
}

/// The engine's own registry stores — fixed `.jit/` locations jit reads its
/// invariant, rule, and gate registries from (repository.rs loads `config.invariants`
/// from `.jit/invariants.toml`, and the rule/gate stores live beside it). These are
/// jit's infrastructure, NOT a user domain vocabulary.
const INVARIANT_STORE: &str = ".jit/invariants.toml";
const RULE_STORE: &str = ".jit/rules.toml";
const GATE_STORE: &str = ".jit/gates.toml";

/// Render the `full` body for the built-in registry-first kinds.
///
/// Dispatches on what CONFIGURATION declares, not on kind NAMES: a projected
/// kind's `[item_kinds.<name>].source` registry path selects the rich view, so a
/// repo may RENAME or alias the equivalent kind freely and still get the right
/// render as long as its source points at the engine's registry store
/// (`@/inv/domain-agnostic`). A projection whose kinds' declared sources are the
/// invariant store renders the invariant registry; the rule + gate stores together
/// render the rule + gate registries. Any other source set — a markdown-first kind
/// with no registry source, or a registry that is not one of jit's own stores — is
/// [`ProjectionError::FullStyleUnsupported`] (there is no built-in rich view for it).
fn render_full_body(
    proj: &ProjectionConfig,
    kinds: &[&ItemKind],
    inputs: &ProjectionInputs,
) -> Result<(String, usize)> {
    // Every kind must declare a registry source; a missing one (a markdown-first
    // kind) has no built-in rich view.
    let Some(sources) = kinds
        .iter()
        .map(|kind| {
            kind.toml_source()
                .map(|descriptor| descriptor.toml.as_str())
        })
        .collect::<Option<BTreeSet<&str>>>()
    else {
        return Err(ProjectionError::FullStyleUnsupported {
            kinds: proj.kinds().to_vec(),
        }
        .into());
    };

    if sources == BTreeSet::from([INVARIANT_STORE]) {
        let registry = &inputs.config.invariants;
        let body = render_invariants_markdown(registry, ProjectionStyle::Full);
        Ok((body, registry.invariants.len()))
    } else if sources == BTreeSet::from([RULE_STORE, GATE_STORE]) {
        let body =
            render_rules_and_gates_markdown(inputs.rules, inputs.gates, ProjectionStyle::Full);
        Ok((body, inputs.rules.rules.len() + inputs.gates.gates.len()))
    } else {
        Err(ProjectionError::FullStyleUnsupported {
            kinds: proj.kinds().to_vec(),
        }
        .into())
    }
}

/// The repo-level content format (`[validation].content_format`, defaulting to
/// Markdown), matching the selection issue descriptions and project sources use so
/// a projection scans its source identically.
fn repo_content_format(config: &JitConfig) -> Result<ContentFormat> {
    match config.validation.as_ref() {
        Some(validation) => Ok(validation.content_format()?),
        None => Ok(ContentFormat::Markdown),
    }
}
