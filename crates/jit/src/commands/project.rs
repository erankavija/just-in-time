//! Generic documentation-projection command (`jit project render`).
//!
//! One command renders every declared `[projection.<name>]` (or a single named
//! one) into its configured documentation target. It is a thin boundary: it opens
//! a recovered [`RepositoryStateStore`](crate::storage::RepositoryStateStore)
//! session, captures the closed image the render reads from, delegates ALL body
//! rendering and target composition to the pure
//! [`repository_state`](crate::repository_state) producers, and owns no CLI
//! parsing or output formatting (the layer boundary in AGENTS.md "Separation of
//! Concerns"). The target path, mode, style, and delimiters come ONLY from config.
//!
//! The render captures a bounded two-phase closure (the engine registries, then
//! the selected projections' sources and targets and the schema files the
//! effective rules reference) and reads only that image-projected content. Every
//! typed failure (unknown kind, missing source, missing target, absent markers)
//! surfaces from the canonical derivation BEFORE the exact delta is applied, so a
//! failing render leaves every target byte-identical (`@/inv/atomic-writes`) and
//! keeps the validation exit code. The delta is scoped to the selected projections
//! (declaration scope), so a single-name render leaves sibling targets untouched.

use super::*;
use crate::config::{JitConfig, ProjectionMode, ProjectionStyle};
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

/// Preserve validation-class producer sources at the command's anyhow transport
/// boundary so callers can downcast the concrete typed error.
fn project_producer_error(error: crate::repository_state::ProducerError) -> anyhow::Error {
    match error {
        crate::repository_state::ProducerError::Projection(source) => anyhow::Error::new(source),
        crate::repository_state::ProducerError::ManagedDocument(source) => {
            anyhow::Error::new(source)
        }
        other => anyhow::Error::new(other),
    }
}

fn project_repository_state_error(
    error: crate::repository_state::RepositoryStateError,
) -> anyhow::Error {
    match error {
        crate::repository_state::RepositoryStateError::Producer(source) => {
            project_producer_error(source)
        }
        crate::repository_state::RepositoryStateError::Projection(source) => {
            anyhow::Error::new(source)
        }
        crate::repository_state::RepositoryStateError::ManagedDocument(source) => {
            anyhow::Error::new(source)
        }
        other => anyhow::Error::new(other),
    }
}

/// Select projection names from the configuration captured for this attempt.
fn selected_projection_names(config: &JitConfig, name: Option<&str>) -> Result<Vec<String>> {
    let registry = config.projection.as_ref();
    match name {
        Some(requested) if registry.is_some_and(|entries| entries.contains_key(requested)) => {
            Ok(vec![requested.to_string()])
        }
        Some(requested) => Err(anyhow!("unknown projection '{requested}'")),
        None => Ok(registry
            .into_iter()
            .flat_map(|entries| entries.keys().cloned())
            .collect()),
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> CommandExecutor<S> {
    /// Render every declared projection, or the single `name`d one, into its
    /// configured documentation target through one recovered mutation session.
    ///
    /// With `name = Some(n)` an unknown projection name is an error, and with
    /// `name = None` every declared projection is rendered in name order (an absent
    /// registry renders nothing). The render captures the engine registries and,
    /// after parsing them, the selected projections' sources and targets plus the
    /// schema files the effective rules reference (a bounded two-phase capture); it
    /// reads only that image-projected content. Every projection body is composed
    /// through the ONE managed-document engine and the resulting exact delta is
    /// applied through [`RepositoryStateStore`](crate::storage::RepositoryStateStore),
    /// so a region-mode projection changes only its delimited block, a shared target
    /// composes deterministically, and a failing render leaves every target
    /// byte-identical (`@/inv/atomic-writes`). A missing target/source/marker or
    /// unknown kind is a typed [`ProjectionError`] surfaced before any write, so it
    /// keeps the validation exit code.
    pub fn project_render(&self, name: Option<&str>) -> Result<ProjectRenderResult> {
        use crate::repository_state::{
            assemble_config, derive_materialization, render_capture_closure, require_target,
            CaptureBudget, CaptureSpec, MaterializationRequest, RepositorySeed, RepositorySeedKind,
            VirtualPath,
        };
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        let mut session = self.storage().open_mutation_session(layout)?;
        let budget = CaptureBudget {
            max_paths: 4096,
            max_listings: 64,
            max_bytes: 256 * 1024 * 1024,
            max_depth: 16,
        };
        let seed = RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "project render".to_string(),
            },
            BTreeMap::new(),
            BTreeMap::new(),
        )?;
        let registries = || -> Result<[VirtualPath; 4]> {
            Ok([
                VirtualPath::data("config.toml")?,
                VirtualPath::data("invariants.toml")?,
                VirtualPath::data("rules.toml")?,
                VirtualPath::data("gates.toml")?,
            ])
        };

        // Bounded two-phase capture with conflict retry: phase one reads the engine
        // registries, then the parsed declarations enumerate the phase-two closure.
        for _ in 0..8 {
            let phase_one = CaptureSpec::phase_one(registries()?, budget)?;
            let image_one = match session.capture(phase_one) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let config_one = assemble_config(&image_one)?;
            let selected_names = selected_projection_names(&config_one, name)?;
            let rules_text = image_repo_bytes(&image_one, ".jit/rules.toml")?
                .map(String::from_utf8)
                .transpose()?;
            let closure = render_capture_closure(
                image_one.layout(),
                &config_one,
                &selected_names,
                rules_text.as_deref(),
            )
            .map_err(project_producer_error)?;
            let mut phase_two = CaptureSpec::phase_one(registries()?, budget)?;
            phase_two.discover_paths(closure)?;
            let image = match session.capture(phase_two) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };

            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let config = declarations.config();
            let selected_names = selected_projection_names(config, name)?;
            let final_rules = image_repo_bytes(&image, ".jit/rules.toml")?
                .map(String::from_utf8)
                .transpose()?;
            let final_closure = render_capture_closure(
                image.layout(),
                config,
                &selected_names,
                final_rules.as_deref(),
            )
            .map_err(project_producer_error)?;
            let captured = image
                .capture_spec()
                .paths()
                .cloned()
                .collect::<BTreeSet<_>>();
            if final_closure.into_iter().collect::<BTreeSet<_>>() != captured {
                continue;
            }

            // Derive the exact delta over the same image and apply it. The delta
            // is scoped to the selected projections (declaration scope), so a
            // single-name render leaves sibling targets untouched.
            let selected_set: BTreeSet<String> = selected_names.iter().cloned().collect();
            let plan = derive_materialization(
                &image,
                MaterializationRequest::RenderConfiguredProjections {
                    declarations: declarations.borrowed(),
                    seed: &seed,
                    selected: Some(selected_set),
                },
            )
            // Keep projection and managed-region failures typed so the command
            // preserves their validation exit-code mapping.
            .map_err(project_repository_state_error)?;

            let registry = config.projection.as_ref();
            let projections = selected_names
                .iter()
                .map(|proj_name| {
                    let projection = registry
                        .and_then(|entries| entries.get(proj_name))
                        .ok_or_else(|| anyhow!("unknown projection '{proj_name}'"))?;
                    let target = require_target(projection, proj_name)
                        .with_context(|| format!("projection '{proj_name}'"))?;
                    let count = plan.projection_count(proj_name).ok_or_else(|| {
                        anyhow!("projection '{proj_name}' produced no render report")
                    })?;
                    Ok(ProjectionRenderReport {
                        name: proj_name.clone(),
                        target,
                        mode: mode_token(projection.mode()),
                        style: style_token(projection.style()),
                        kinds: projection.kinds().to_vec(),
                        count,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            match session.apply(&plan) {
                Ok(_) => {
                    return Ok(ProjectRenderResult {
                        count: projections.len(),
                        projections,
                    })
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "project render did not converge after repeated capture conflicts"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_producer_error_preserves_projection_source_for_downcast() {
        let error = project_producer_error(crate::repository_state::ProducerError::Projection(
            crate::repository_state::ProjectionError::MissingTarget {
                projection: "invariants".to_string(),
            },
        ));

        assert!(matches!(
            error.downcast_ref::<crate::repository_state::ProjectionError>(),
            Some(crate::repository_state::ProjectionError::MissingTarget { projection })
                if projection == "invariants"
        ));
    }
}
