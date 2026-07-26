//! Plan-document location resolver (boundary, design doc D9 / Wave-1 task T4).
//!
//! A container's plan lives in one of two places, selected by the plan-doc
//! location template (the container's graph template's planning-node `doc`, via
//! [`GraphTemplate::plan_doc_location`](crate::templates::GraphTemplate::plan_doc_location)):
//!
//! - the absence of a `doc` (modeled by the caller as the literal sentinel
//!   `"inline"`) — the plan is the issue's own body
//!   ([`Issue::description`](crate::domain::Issue)); or
//! - an external path template — the container substitutions of
//!   [`PlanDocContainer`] are applied and the resulting file is read from disk.
//!
//! This module is the **boundary**: the only place filesystem I/O happens. It
//! resolves the location, loads the content, and feeds the resulting string to
//! the PURE projection/validation engine
//! ([`project`](crate::domain::project) + [`Projection::with_sections`]). The
//! engine itself never touches the filesystem — an inline body and an external
//! file carrying identical content therefore project to the *same*
//! [`Projection`], so a planning bracket validates a plan the same way no
//! matter where it is stored (D9 success criterion).
//!
//! # Domain-agnostic
//!
//! No bracket type name (`epic` / `planning` / `breakdown`) is hardcoded here.
//! The only literal compared against is the `"inline"` sentinel, a resolver-value
//! convention meaning "use the issue body", not a domain type name.

use std::path::{Path, PathBuf};

use crate::config::DocumentationConfig;
use crate::document::{content_parser_for, ContentParserError};
use crate::domain::artifact_directory::{resolve_artifact_directory, ArtifactDirectoryError};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{project, ContentFormat, Issue, Projection};

/// The plan-doc location value that means "the plan is the issue body".
///
/// A resolver-value convention: a graph template whose planning node declares no
/// `doc` has no external plan, which the caller models with this sentinel. The
/// resolver compares the template against it to choose the inline path. It is NOT
/// a domain type name, so comparing against it keeps the engine domain-agnostic.
pub const INLINE_LOCATION: &str = "inline";

/// The document-reference label that marks a planning node's plan document.
///
/// A planning node records WHERE its container's plan lives as a
/// [`DocumentReference`](crate::domain::DocumentReference) carrying this label.
/// That reference is the validation-time source of truth for the plan-doc
/// location: `jit validate` reads the plan from this reference's `path`, so a
/// plan that is moved/archived and re-linked keeps validating from its new
/// location. The graph template's `plan_doc_location` resolves into the planning
/// node's description as an instruction naming where to author the plan;
/// applying a template attaches no document reference, and this one is created
/// when the plan is authored and linked.
pub const PLAN_DOC_LABEL: &str = "plan";

/// The `{id}` placeholder substituted with the container id in an external
/// plan-doc location template.
const ID_PLACEHOLDER: &str = "{id}";

/// The `{container.id}` placeholder, an alias for [`ID_PLACEHOLDER`] used by
/// graph-template `doc` strings (e.g. `dev/active/{container.id}-plan.md`). The
/// apply engine interpolates the full `container.*` token family at apply time;
/// this resolver substitutes this token with the container id exactly like
/// `{id}`.
const CONTAINER_ID_PLACEHOLDER: &str = "{container.id}";

/// The `{container.dir}` placeholder used by graph-template `doc` strings that
/// name an issue-scoped area (e.g. `{container.dir}/plan.md`): the canonical
/// artifact directory the container owns inside that area
/// ([`resolve_artifact_directory`]).
const CONTAINER_DIR_PLACEHOLDER: &str = "{container.dir}";

/// The container substitutions a plan-doc location template resolves against.
///
/// Carries the identifier `{id}` / `{container.id}` resolve to and, for a
/// declaration that names an issue-scoped area, the directory
/// `{container.dir}` resolves to. Resolving the area is what can fail, so it
/// happens once here and [`resolve_plan_doc_location`] stays pure and total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDocContainer {
    id: String,
    directory: Option<String>,
}

impl PlanDocContainer {
    /// The substitutions for `container` under a declaration that names no
    /// issue-scoped area.
    ///
    /// `{container.dir}` stays out of scope, which is what the apply engine
    /// resolves a node declaring no area under.
    pub fn unscoped(container: &Issue) -> Self {
        Self {
            id: container.id.clone(),
            directory: None,
        }
    }

    /// The substitutions for `container` under a declaration naming the
    /// issue-scoped `area`, with `{container.dir}` resolved to the directory the
    /// container owns there.
    ///
    /// The directory is the domain resolver's own answer
    /// ([`resolve_artifact_directory`]), so a plan located before it is written
    /// and a plan the apply engine writes name one directory.
    ///
    /// # Errors
    ///
    /// [`ArtifactDirectoryError::UndeclaredArea`] when `area` is absent from the
    /// configured issue-scoped registry, naming both the area and the registry
    /// it was matched against.
    pub fn in_area(
        container: &Issue,
        area: &str,
        documentation: &DocumentationConfig,
        hierarchy: &HierarchyConfig,
    ) -> Result<Self, ArtifactDirectoryError> {
        Ok(Self {
            id: container.id.clone(),
            directory: Some(resolve_artifact_directory(
                container,
                area,
                documentation,
                hierarchy,
            )?),
        })
    }

    /// The container id this resolves `{id}` / `{container.id}` to.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Substitute every supported placeholder in `template`.
    ///
    /// A placeholder this value does not carry — `{container.dir}` for a
    /// declaration naming no area — is left verbatim, matching what the apply
    /// engine leaves for a node declaring no area.
    fn substitute(&self, template: &str) -> String {
        let substituted = template
            .replace(CONTAINER_ID_PLACEHOLDER, &self.id)
            .replace(ID_PLACEHOLDER, &self.id);
        match &self.directory {
            Some(directory) => substituted.replace(CONTAINER_DIR_PLACEHOLDER, directory),
            None => substituted,
        }
    }
}

/// Error raised while resolving or loading a container's plan document.
///
/// File reading happens only at this boundary, so a missing or unreadable
/// external plan path surfaces here as a contextual `Result::Err` (naming the
/// container and the resolved path) rather than a panic or a silent empty body.
#[derive(Debug, thiserror::Error)]
pub enum PlanDocError {
    /// The external plan file could not be read (missing, unreadable, etc.).
    #[error(
        "plan document for issue {container_id} not found or unreadable at {}: {source}",
        path.display()
    )]
    Read {
        /// The container whose plan was being resolved.
        container_id: String,
        /// The resolved filesystem path the resolver attempted to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The content format selected a parser whose cargo feature is not compiled
    /// into this build. Surfaced rather than silently falling back to Markdown.
    #[error(transparent)]
    ContentParser(#[from] ContentParserError),
}

/// Where a container's plan document lives, after resolving the template.
///
/// Produced by [`resolve_plan_doc_location`], a pure function: an `"inline"`
/// template yields [`PlanDocLocation::Inline`]; any other template yields
/// [`PlanDocLocation::External`] with the container substitutions already
/// applied. This split is pure (no I/O) so the location decision is
/// independently testable; the actual file read happens later, at the boundary,
/// in [`load_plan_content`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanDocLocation {
    /// The plan is the issue's own body ([`Issue::description`]).
    Inline,
    /// The plan is an external file at this (already substituted) path.
    External(PathBuf),
}

/// Resolve a `plan_doc_location` template to a [`PlanDocLocation`] (PURE: no I/O).
///
/// The literal sentinel [`INLINE_LOCATION`] (`"inline"`) yields
/// [`PlanDocLocation::Inline`]. Any other value is treated as a path template:
/// every occurrence of the `{id}` placeholder — or its graph-template alias
/// `{container.id}` — is replaced with the container's id, `{container.dir}` is
/// replaced with the canonical artifact directory `container` carries, and the
/// result is wrapped in [`PlanDocLocation::External`]. A template with no
/// placeholder is used verbatim (a fixed shared plan path).
pub fn resolve_plan_doc_location(template: &str, container: &PlanDocContainer) -> PlanDocLocation {
    if template == INLINE_LOCATION {
        PlanDocLocation::Inline
    } else {
        PlanDocLocation::External(PathBuf::from(container.substitute(template)))
    }
}

/// Load the plan content string for `issue` (BOUNDARY: performs filesystem I/O).
///
/// Resolves `template` via [`resolve_plan_doc_location`]:
///
/// - [`PlanDocLocation::Inline`] returns a clone of the issue's body
///   ([`Issue::description`]); no file is read.
/// - [`PlanDocLocation::External`] reads the file. The path is joined onto
///   `base_dir` when relative, so callers pass the repo root and templates stay
///   repo-relative. A missing or unreadable file yields a contextual
///   [`PlanDocError::Read`] naming the container and the resolved path.
///
/// This is the ONLY function in the resolver that touches the filesystem; the
/// projection it feeds ([`project_plan_doc`]) is pure.
pub fn load_plan_content(
    issue: &Issue,
    template: &str,
    container: &PlanDocContainer,
    base_dir: &Path,
) -> Result<String, PlanDocError> {
    match resolve_plan_doc_location(template, container) {
        PlanDocLocation::Inline => Ok(issue.description.clone()),
        PlanDocLocation::External(relative) => {
            let path = if relative.is_absolute() {
                relative
            } else {
                base_dir.join(relative)
            };
            std::fs::read_to_string(&path).map_err(|source| PlanDocError::Read {
                container_id: container.id().to_string(),
                path,
                source,
            })
        }
    }
}

/// Resolve the plan content, then project `issue` with that content's sections
/// through the PURE engine.
///
/// This is the resolver's top-level seam (D9): it loads the plan content at the
/// boundary ([`load_plan_content`]) and feeds the loaded string into the pure
/// projection ([`project`] + [`Projection::with_sections`]). Because the only
/// content that varies is the string, an inline body and an external file with
/// identical content yield an IDENTICAL [`Projection`] — so the bracket
/// validates a plan the same way regardless of where it is stored.
///
/// The body parser is selected by [`content_parser_for`] (issue format → repo
/// default → Markdown), matching every other projection-with-sections site, so
/// dispatch never drifts.
pub fn project_plan_doc(
    issue: &Issue,
    template: &str,
    container: &PlanDocContainer,
    base_dir: &Path,
    repo_default_format: ContentFormat,
) -> Result<Projection, PlanDocError> {
    let content = load_plan_content(issue, template, container, base_dir)?;
    let parser = content_parser_for(issue.content_format, repo_default_format)?;
    // PURE from here: project the cheap selector fields, then attach the section
    // view computed from the RESOLVED content. No filesystem access.
    Ok(project(issue).with_sections(&content, parser.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::template_expand::test_declarations as declared;
    use tempfile::TempDir;

    fn container(description: &str) -> Issue {
        crate::domain::types::fixture_issue("Container".to_string(), description.to_string())
    }

    /// The substitutions for a container known by `id`, under a declaration
    /// naming no issue-scoped area.
    fn identified(id: &str) -> PlanDocContainer {
        let mut issue = container("");
        issue.id = id.to_string();
        PlanDocContainer::unscoped(&issue)
    }

    /// The substitutions for the shared container fixture, under a declaration
    /// naming the shared issue-scoped area.
    fn in_declared_area(issue: &Issue) -> PlanDocContainer {
        PlanDocContainer::in_area(
            issue,
            declared::AREA,
            &declared::documentation(),
            &declared::hierarchy(),
        )
        .unwrap()
    }

    /// The path the apply engine writes for `declaration` (resolved in
    /// `area`) — the answer this resolver must agree with.
    fn applied_document(issue: &Issue, declaration: &str, area: Option<&str>) -> String {
        let delta = declared::expand(
            &declared::document_template(declaration, area),
            issue,
            &declared::bindings(&issue.id),
            &declared::snapshots(&[]),
        )
        .unwrap();
        declared::planned_document(&delta).to_string()
    }

    /// The external path `location` names.
    fn external(location: PlanDocLocation) -> PathBuf {
        match location {
            PlanDocLocation::External(path) => path,
            PlanDocLocation::Inline => panic!("expected an external plan-doc location"),
        }
    }

    // --- resolve_plan_doc_location (pure) ---------------------------------

    #[test]
    fn test_resolve_inline_sentinel_is_inline() {
        assert_eq!(
            resolve_plan_doc_location("inline", &identified("abc123")),
            PlanDocLocation::Inline
        );
    }

    #[test]
    fn test_resolve_substitutes_id_placeholder() {
        assert_eq!(
            resolve_plan_doc_location("plans/{id}.md", &identified("abc123")),
            PlanDocLocation::External(PathBuf::from("plans/abc123.md"))
        );
    }

    #[test]
    fn test_resolve_substitutes_every_id_occurrence() {
        assert_eq!(
            resolve_plan_doc_location("{id}/plan-{id}.md", &identified("xyz")),
            PlanDocLocation::External(PathBuf::from("xyz/plan-xyz.md"))
        );
    }

    #[test]
    fn test_resolve_substitutes_container_id_alias() {
        // Graph-template `doc` strings use `{container.id}`; the resolver maps it
        // to the container id exactly like `{id}` so a template's plan-doc
        // location resolves at validation time.
        assert_eq!(
            resolve_plan_doc_location("dev/active/{container.id}-plan.md", &identified("abc123")),
            PlanDocLocation::External(PathBuf::from("dev/active/abc123-plan.md"))
        );
    }

    #[test]
    fn test_resolve_template_without_placeholder_is_verbatim() {
        assert_eq!(
            resolve_plan_doc_location("dev/plan.md", &identified("abc123")),
            PlanDocLocation::External(PathBuf::from("dev/plan.md"))
        );
    }

    #[test]
    fn test_resolve_plan_doc_location_substitutes_the_artifact_directory_field() {
        let issue = declared::container("c1");
        // Two declarations that both name the directory, one of them twice, so a
        // single-occurrence substitution is not enough to pass.
        for declaration in ["{container.dir}/plan.md", "{container.dir}/{container.dir}"] {
            let resolved = external(resolve_plan_doc_location(
                declaration,
                &in_declared_area(&issue),
            ));

            // The located plan is the file an apply writes for the same
            // declaration: one declaration resolves to one path, whichever
            // derivation resolves it.
            assert_eq!(
                resolved,
                PathBuf::from(applied_document(&issue, declaration, Some(declared::AREA))),
                "{declaration} must locate the applied document"
            );
            let located = resolved.to_string_lossy();
            assert!(
                !located.contains('{'),
                "no declaration field may survive the substitution: {located}"
            );
            assert!(
                resolved.starts_with(declared::AREA),
                "the declared area holds the resolved directory: {located}"
            );
        }
    }

    #[test]
    fn test_resolve_plan_doc_location_leaves_the_directory_field_out_of_scope_without_an_area() {
        // A declaration naming no issue-scoped area has no directory to resolve
        // in, so the field stays verbatim — what the apply engine resolves the
        // same declaration to.
        let issue = declared::container("c1");
        let declaration = "{container.dir}/plan.md";

        assert_eq!(
            external(resolve_plan_doc_location(
                declaration,
                &PlanDocContainer::unscoped(&issue)
            )),
            PathBuf::from(applied_document(&issue, declaration, None))
        );
    }

    #[test]
    fn test_plan_doc_container_rejects_an_area_the_registry_does_not_declare() {
        let issue = declared::container("c1");
        // A sibling of the declared area: close enough that only the registry
        // distinguishes it.
        let undeclared = format!("{}-drafts", declared::AREA);

        let error = PlanDocContainer::in_area(
            &issue,
            &undeclared,
            &declared::documentation(),
            &declared::hierarchy(),
        )
        .unwrap_err();

        let ArtifactDirectoryError::UndeclaredArea {
            area,
            declared: registry,
        } = &error;
        assert_eq!(area, &undeclared);
        assert_eq!(registry, &declared::documentation().issue_scoped_areas());
        // The message names the offending area and the registry it was matched
        // against, so it locates the declaration to fix.
        let message = error.to_string();
        assert!(message.contains(&undeclared), "{message}");
        assert!(message.contains(declared::AREA), "{message}");

        // The rejection is the registry's doing: the declared area resolves.
        assert!(PlanDocContainer::in_area(
            &issue,
            declared::AREA,
            &declared::documentation(),
            &declared::hierarchy()
        )
        .is_ok());
    }

    // --- load_plan_content (boundary) -------------------------------------

    #[test]
    fn test_load_inline_returns_issue_body() {
        let issue = container("## Plan\n\n- inline step\n");
        let dir = TempDir::new().unwrap();
        let content = load_plan_content(
            &issue,
            "inline",
            &PlanDocContainer::unscoped(&issue),
            dir.path(),
        )
        .unwrap();
        assert_eq!(content, issue.description);
    }

    #[test]
    fn test_load_external_reads_file_with_id_substitution() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("plans")).unwrap();
        std::fs::write(
            dir.path().join("plans/abc123.md"),
            "## Plan\n\n- external step\n",
        )
        .unwrap();

        let issue = container("the body is ignored when external");
        let content =
            load_plan_content(&issue, "plans/{id}.md", &identified("abc123"), dir.path()).unwrap();
        assert_eq!(content, "## Plan\n\n- external step\n");
    }

    #[test]
    fn test_load_missing_external_path_yields_contextual_error() {
        let dir = TempDir::new().unwrap();
        let issue = container("body");
        let err = load_plan_content(&issue, "plans/{id}.md", &identified("abc123"), dir.path())
            .unwrap_err();
        let message = err.to_string();
        // Names the container id and the resolved path.
        assert!(matches!(err, PlanDocError::Read { .. }));
        assert!(message.contains("abc123"), "{message}");
        assert!(message.contains("plans/abc123.md"), "{message}");
    }

    #[test]
    fn test_load_external_absolute_path_is_not_joined() {
        let dir = TempDir::new().unwrap();
        let abs = dir.path().join("absolute-plan.md");
        std::fs::write(&abs, "## Plan\n\n- absolute step\n").unwrap();

        let issue = container("body");
        // An absolute template is read as-is, ignoring base_dir.
        let other_base = TempDir::new().unwrap();
        let content = load_plan_content(
            &issue,
            abs.to_str().unwrap(),
            &identified("abc123"),
            other_base.path(),
        )
        .unwrap();
        assert_eq!(content, "## Plan\n\n- absolute step\n");
    }

    // --- project_plan_doc: inline == external for identical content -------

    #[test]
    fn test_inline_and_external_project_identical_content() {
        // The SAME plan content, stored inline in one issue and in an external
        // file for another, must project to the IDENTICAL canonical shape — the
        // D9 success criterion.
        let plan = "## Success Criteria\n\n- [hard] REQ-01: must hold\n- [soft] nice\n";

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("plan-c1.md"), plan).unwrap();

        // Inline issue: body IS the plan.
        let inline_issue = container(plan);
        let inline_projection = project_plan_doc(
            &inline_issue,
            "inline",
            &identified("c1"),
            dir.path(),
            ContentFormat::Markdown,
        )
        .unwrap();

        // External issue: same labels/state but an EMPTY body; the plan lives in
        // the file. Keep the non-body projection inputs identical so only the
        // resolved content path differs.
        let external_issue = container("");
        let external_projection = project_plan_doc(
            &external_issue,
            "plan-{id}.md",
            &identified("c1"),
            dir.path(),
            ContentFormat::Markdown,
        )
        .unwrap();

        // The projected `sections` view (the content-derived part) is identical.
        assert_eq!(inline_projection.sections, external_projection.sections);
        let sections = inline_projection.sections.as_ref().unwrap();
        let criteria = sections.get("success_criteria").unwrap();
        assert_eq!(criteria.items[0], "[hard] REQ-01: must hold");
    }

    #[test]
    fn test_project_plan_doc_feeds_sections_to_engine() {
        // The projection produced by the resolver carries the section view the
        // pure engine validates against (proving the content reached the engine).
        let issue = container("## Plan\n\n- only step\n");
        let dir = TempDir::new().unwrap();
        let projection = project_plan_doc(
            &issue,
            "inline",
            &PlanDocContainer::unscoped(&issue),
            dir.path(),
            ContentFormat::Markdown,
        )
        .unwrap();
        let sections = projection.sections.expect("sections populated by resolver");
        assert_eq!(
            sections.get("plan").unwrap().items,
            vec!["only step".to_string()]
        );
    }

    #[test]
    fn test_project_plan_doc_missing_external_errors_not_panics() {
        let issue = container("body");
        let dir = TempDir::new().unwrap();
        let result = project_plan_doc(
            &issue,
            "missing/{id}.md",
            &identified("abc123"),
            dir.path(),
            ContentFormat::Markdown,
        );
        assert!(matches!(result, Err(PlanDocError::Read { .. })));
    }
}
