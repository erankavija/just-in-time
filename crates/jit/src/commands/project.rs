//! Generic documentation-projection command (`jit project render`).
//!
//! One command renders every declared `[projection.<name>]` (or a single named
//! one) into its configured documentation target. It is a thin boundary: it loads
//! the projection registry plus the effective rules and gate registry, delegates
//! ALL body rendering to
//! [`render_projection_body`](crate::repository_state::render_projection_body)
//! and region-splicing to
//! [`splice_region`](crate::repository_state::splice_region), and owns no CLI
//! parsing or output formatting (the layer boundary in AGENTS.md "Separation of
//! Concerns"). The target path, mode, style, and delimiters come ONLY from config.
//!
//! The render is TWO-PHASE (REQ-07): phase 1 renders every projection body and
//! materializes every target's final bytes in memory — where every typed failure
//! (unknown kind, missing source, missing target, missing region file, absent
//! markers) surfaces; phase 2 writes the materialized targets atomically only once
//! every projection has validated. A failing render therefore leaves the working
//! tree byte-identical (`@/inv/atomic-writes`).

use super::*;
use crate::config::{ProjectionConfig, ProjectionMode, ProjectionStyle};
use crate::repository_state::{compose_projection, require_target, ProjectionError};
use crate::repository_state::{render_projection_body, ProjectionInputs};
use std::collections::BTreeMap;

/// The result of rendering ONE projection, serialized as an element of
/// [`ProjectRenderResult::projections`].
#[derive(Debug, Serialize)]
pub struct ProjectionRenderReport {
    /// The projection name (its `[projection.<name>]` table key).
    pub name: String,
    /// The repo-relative documentation target that was written.
    pub target: String,
    /// The projection mode used, as its config token (`separate-file`|`region`).
    pub mode: String,
    /// The render style used, as its config token (`id-anchor`|`full`).
    pub style: String,
    /// The kind name(s) rendered, in declaration order.
    pub kinds: Vec<String>,
    /// Rows (id-anchor) or registry entries (full) rendered into the target.
    pub count: usize,
}

/// The result of a `jit project render` run over one or all projections.
///
/// Serialized as the `--json` payload with the list envelope
/// `{"count": N, "projections": [...]}`.
#[derive(Debug, Serialize)]
pub struct ProjectRenderResult {
    /// The number of projections rendered (mirrors `projections.len()`).
    pub count: usize,
    /// Per-projection reports, in projection-name order.
    pub projections: Vec<ProjectionRenderReport>,
}

/// The config token for a [`ProjectionMode`].
fn mode_token(mode: ProjectionMode) -> String {
    match mode {
        ProjectionMode::SeparateFile => "separate-file",
        ProjectionMode::Region => "region",
    }
    .to_string()
}

/// The config token for a [`ProjectionStyle`].
fn style_token(style: ProjectionStyle) -> String {
    match style {
        ProjectionStyle::Full => "full",
        ProjectionStyle::IdAnchor => "id-anchor",
    }
    .to_string()
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Render every declared projection, or the single `name`d one, into its
    /// configured documentation target.
    ///
    /// Reads the `[projection.*]` registry from config; with `name = Some(n)` an
    /// unknown projection name is an error, and with `name = None` every declared
    /// projection is rendered in name order (an absent registry renders nothing).
    /// Each projection's body is rendered through the shared generic path (so a
    /// markdown-first kind projects its `- **{id}** — {text}` rows and the built-in
    /// registry views reproduce their rich blocks), then written atomically through
    /// the storage boundary; in region mode only the delimited block changes and a
    /// missing target/source/marker or unknown kind is a typed error before any
    /// write (`@/inv/atomic-writes`, REQ-07).
    pub fn project_render(&self, name: Option<&str>) -> Result<ProjectRenderResult> {
        let config = self.cached_config()?;
        let registry = config.projection.clone().unwrap_or_default();
        let selected: Vec<(String, ProjectionConfig)> = match name {
            Some(requested) => {
                let projection = registry
                    .get(requested)
                    .ok_or_else(|| anyhow!("unknown projection '{requested}'"))?
                    .clone();
                vec![(requested.to_string(), projection)]
            }
            None => registry.into_iter().collect(),
        };

        let rules = self.effective_rules()?;
        let gates = self.storage().load_gate_registry()?;
        let inputs = ProjectionInputs {
            config,
            rules,
            gates: &gates,
        };

        // Phase 1 — render every projection body and materialize every target's
        // final bytes in memory. Every REQ-07 typed failure (unknown kind, missing
        // source, missing target, missing region file, absent markers) surfaces
        // here, BEFORE any write. Region projections that share a target thread
        // their splices through the same pending content, so a later region sees an
        // earlier one's change (matching the sequential single-write result).
        let mut pending: BTreeMap<String, String> = BTreeMap::new();
        let mut projections = Vec::with_capacity(selected.len());
        for (proj_name, projection) in &selected {
            let mut read = |path: &str| {
                self.storage()
                    .read_repo_file(path)
                    .map_err(anyhow::Error::from)
            };
            // The typed `ProjectionError`s flow through `with_context` (which anyhow
            // keeps downcastable) so they map to the validation exit code.
            let (body, count) = render_projection_body(projection, &inputs, &mut read)
                .with_context(|| format!("projection '{proj_name}'"))?;
            let target = require_target(projection, proj_name)
                .with_context(|| format!("projection '{proj_name}'"))?;
            // Region projections that share a target thread their splices through
            // the shared `pending` map, so a later region composes onto an earlier
            // one's change instead of overwriting it.
            compose_projection(
                &mut pending,
                &target,
                projection.mode(),
                &body,
                &projection.region_begin(proj_name),
                &projection.region_end(proj_name),
                |path| {
                    self.storage().read_repo_file(path).map_err(|source| {
                        ProjectionError::Read {
                            path: path.to_string(),
                            source,
                        }
                        .into()
                    })
                },
            )
            .with_context(|| format!("projection '{proj_name}'"))?;
            projections.push(ProjectionRenderReport {
                name: proj_name.clone(),
                target,
                mode: mode_token(projection.mode()),
                style: style_token(projection.style()),
                kinds: projection.kinds().to_vec(),
                count,
            });
        }

        // Phase 2 — write every materialized target atomically through the storage
        // boundary, only now that all projections have validated. Each distinct
        // target is written once (`@/inv/atomic-writes`).
        for (target, content) in &pending {
            self.storage()
                .write_repo_file(target, content)
                .map_err(|source| ProjectionError::Write {
                    path: target.clone(),
                    source,
                })?;
        }

        Ok(ProjectRenderResult {
            count: projections.len(),
            projections,
        })
    }
}
