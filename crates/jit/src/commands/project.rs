//! Generic documentation-projection command (`jit project render`).
//!
//! One command renders every declared `[projection.<name>]` (or a single named
//! one) into its configured documentation target. It is a thin boundary: it opens
//! a [`RepositoryStateStore`](crate::storage::RepositoryStateStore)
//! session, captures the closed image the render reads from, delegates ALL body
//! rendering and target composition to the pure
//! [`repository_state`](crate::repository_state) producers, and owns no CLI
//! parsing or output formatting (the layer boundary in AGENTS.md "Separation of
//! Concerns"). The target path, mode, style, and delimiters come ONLY from config.
//!
//! The render captures one bounded round (the engine registries, then the
//! selected projections' sources and targets and the schema files the
//! effective rules reference) and reads only that image-projected content. Every
//! typed failure (unknown kind, missing source, missing target, absent markers)
//! surfaces from the canonical derivation BEFORE the exact delta is applied, so a
//! failing render leaves every target byte-identical (`@/inv/atomic-writes`) and
//! keeps the validation exit code. The delta is scoped to the selected projections
//! (declaration scope), so a single-name render leaves sibling targets untouched.

use super::*;
use crate::config::{JitConfig, ProjectionMode, ProjectionStyle};

/// The result of rendering ONE projection, serialized as an element of
/// [`ProjectRenderResult`].
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectionRenderReport {
    /// The projection name (its `[projection.<name>]` table key).
    pub name: String,
    /// The repo-relative documentation target that was written.
    pub target: String,
    /// The projection mode used.
    pub mode: ProjectionMode,
    /// The render style used.
    pub style: ProjectionStyle,
    /// The kind name(s) rendered, in declaration order.
    pub kinds: Vec<String>,
    /// Rows (id-anchor) or registry entries (full) rendered into the target.
    pub count: usize,
}

/// Collection key of [`ProjectRenderResult`].
pub enum ProjectionsKey {}

impl CollectionKey for ProjectionsKey {
    const KEY: &'static str = "projections";
    const SCHEMA_NAME: Option<&'static str> = Some("ProjectRenderResult");
}

/// The result of a `jit project render` run over one or all projections: one
/// report per projection, in projection-name order.
pub type ProjectRenderResult = ListEnvelope<ProjectionsKey, ProjectionRenderReport>;

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
        Some(requested) => Err(crate::errors::InvalidArgumentError::new(format!(
            "unknown projection '{requested}'"
        ))
        .into()),
        None => Ok(registry
            .into_iter()
            .flat_map(|entries| entries.keys().cloned())
            .collect()),
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> ItemOps<'_, S> {
    /// Render every declared projection, or the single `name`d one, into its
    /// configured documentation target through one mutation session.
    ///
    /// With `name = Some(n)` an unknown projection name is an error, and with
    /// `name = None` every declared projection is rendered in name order (an absent
    /// registry renders nothing). The render captures the engine registries and,
    /// after parsing them, the selected projections' sources and targets plus the
    /// schema files the effective rules reference (one bounded capture round); it
    /// reads only that image-projected content. Every projection body is composed
    /// through the ONE managed-document engine and the resulting exact delta is
    /// applied through [`RepositoryStateStore`](crate::storage::RepositoryStateStore),
    /// so a region-mode projection changes only its delimited block, a shared target
    /// composes deterministically, and a failing render leaves every target
    /// byte-identical (`@/inv/atomic-writes`). A missing target/source/marker or
    /// unknown kind is a typed [`crate::repository_state::ProjectionError`] surfaced before any write, so it
    /// keeps the validation exit code.
    pub fn project_render(&self, name: Option<&str>) -> Result<ProjectRenderResult> {
        use crate::repository_state::{
            derive_materialization, render_capture_closure, require_target, CaptureBudget,
            CaptureSpec, MaterializationRequest, VirtualPath,
        };
        use std::collections::BTreeSet;

        let layout = self.exec.require_layout()?;
        let budget = CaptureBudget {
            max_listings: 64,
            max_bytes: 256 * 1024 * 1024,
            max_depth: 16,
        };
        let registries = || -> Result<[VirtualPath; 4]> {
            Ok([
                VirtualPath::CONFIG,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::GATES,
            ])
        };

        // One bounded capture round: the engine registries, then the closure
        // their declarations enumerate.
        let context = super::production_mutation_context();
        with_mutation_session(self.exec.storage(), &layout, "project render", |session| {
            let mut spec = CaptureSpec::new(registries()?, budget)?;
            spec.discover_tail(VirtualPath::EVENTS)?;
            let mut configuration = super::ConfigurationParse::captured();
            let Some(image) = capture_discovering(session, spec, |image, spec| {
                let config = configuration.parse(image)?;
                let rules_text = super::image_virtual_bytes(image, &VirtualPath::RULES)?
                    .map(String::from_utf8)
                    .transpose()?;
                let closure = render_capture_closure(
                    image.layout(),
                    config,
                    &selected_projection_names(config, name)?,
                    rules_text.as_deref(),
                )
                .map_err(project_producer_error)?;
                Ok(spec.discover_paths(closure)?)
            })?
            else {
                return Ok(SessionStep::Retry);
            };

            let declarations = crate::repository_state::declarations_with_config(
                &image,
                configuration.into_config(&image),
            )?;
            let config = declarations.config();
            let selected_names = selected_projection_names(config, name)?;

            // Derive the exact delta over the same image and apply it. The delta
            // is scoped to the selected projections (declaration scope), so a
            // single-name render leaves sibling targets untouched.
            let selected_set: BTreeSet<String> = selected_names.iter().cloned().collect();
            let plan = derive_materialization(
                &image,
                &context,
                MaterializationRequest::RenderConfiguredProjections {
                    declarations: declarations.borrowed(),
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
                        mode: projection.mode(),
                        style: projection.style(),
                        kinds: projection.kinds().to_vec(),
                        count,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SessionStep::Apply(
                plan,
                ProjectRenderResult::new(projections),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_projection_render_report_serializes_enum_tokens_to_identical_json() {
        let report = ProjectionRenderReport {
            name: "invariants".to_string(),
            target: ".jit/invariants.md".to_string(),
            mode: ProjectionMode::Region,
            style: ProjectionStyle::IdAnchor,
            kinds: vec!["invariant".to_string()],
            count: 2,
        };

        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"name":"invariants","target":".jit/invariants.md","mode":"region","style":"id-anchor","kinds":["invariant"],"count":2}"#
        );
    }

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
