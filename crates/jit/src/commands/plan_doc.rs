//! Plan-document location resolver (boundary, design doc D9 / Wave-1 task T4).
//!
//! A container's plan lives in one of two places, selected by the plan-doc
//! location template (the container's graph template's planning-node `doc`, via
//! [`GraphTemplate::plan_doc_location`](crate::templates::GraphTemplate::plan_doc_location)):
//!
//! - the absence of a `doc` (modeled by the caller as the literal sentinel
//!   `"inline"`) — the plan is the issue's own body
//!   ([`Issue::description`](crate::domain::Issue)); or
//! - an external path template — an `{id}` / `{container.id}` placeholder (if
//!   present) is substituted with the container id and the resulting file is read
//!   from disk.
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

use crate::document::{content_parser_for, ContentParserError};
use crate::domain::{project, ContentFormat, Issue, Projection};

/// The plan-doc location value that means "the plan is the issue body".
///
/// A resolver-value convention: a graph template whose planning node declares no
/// `doc` has no external plan, which the caller models with this sentinel. The
/// resolver compares the template against it to choose the inline path. It is NOT
/// a domain type name, so comparing against it keeps the engine domain-agnostic.
///
/// # Examples
///
/// ```
/// use jit::commands::plan_doc::{resolve_plan_doc_location, PlanDocLocation, INLINE_LOCATION};
///
/// // The sentinel selects the inline path regardless of the container id.
/// assert_eq!(INLINE_LOCATION, "inline");
/// assert_eq!(
///     resolve_plan_doc_location(INLINE_LOCATION, "abc123"),
///     PlanDocLocation::Inline
/// );
/// ```
pub const INLINE_LOCATION: &str = "inline";

/// The `{id}` placeholder substituted with the container id in an external
/// plan-doc location template.
const ID_PLACEHOLDER: &str = "{id}";

/// The `{container.id}` placeholder, an alias for [`ID_PLACEHOLDER`] used by
/// graph-template `doc` strings (e.g. `dev/active/{container.id}-plan.md`). The
/// apply engine interpolates the full `container.*` token family at apply time;
/// at validation time the only available value is the container id, so the
/// resolver substitutes this token with it exactly like `{id}`.
const CONTAINER_ID_PLACEHOLDER: &str = "{container.id}";

/// Error raised while resolving or loading a container's plan document.
///
/// File reading happens only at this boundary, so a missing or unreadable
/// external plan path surfaces here as a contextual `Result::Err` (naming the
/// container and the resolved path) rather than a panic or a silent empty body.
///
/// # Examples
///
/// ```
/// use jit::commands::plan_doc::PlanDocError;
///
/// // The error message names the container id and the path it tried to read.
/// let err = PlanDocError::Read {
///     container_id: "abc123".to_string(),
///     path: "plans/abc123.md".into(),
///     source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
/// };
/// let message = err.to_string();
/// assert!(message.contains("abc123"));
/// assert!(message.contains("plans/abc123.md"));
/// ```
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
/// [`PlanDocLocation::External`] with `{id}` already substituted. This split is
/// pure (no I/O) so the location decision is independently testable; the actual
/// file read happens later, at the boundary, in [`load_plan_content`].
///
/// # Examples
///
/// ```
/// use jit::commands::plan_doc::{resolve_plan_doc_location, PlanDocLocation};
///
/// // The sentinel resolves to the inline body.
/// assert_eq!(resolve_plan_doc_location("inline", "abc123"), PlanDocLocation::Inline);
///
/// // A template substitutes `{id}` with the container id.
/// match resolve_plan_doc_location("plans/{id}.md", "abc123") {
///     PlanDocLocation::External(path) => assert_eq!(path.to_str(), Some("plans/abc123.md")),
///     PlanDocLocation::Inline => panic!("expected an external path"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanDocLocation {
    /// The plan is the issue's own body ([`Issue::description`]).
    Inline,
    /// The plan is an external file at this (already `{id}`-substituted) path.
    External(PathBuf),
}

/// Resolve a `plan_doc_location` template to a [`PlanDocLocation`] (PURE: no I/O).
///
/// The literal sentinel [`INLINE_LOCATION`] (`"inline"`) yields
/// [`PlanDocLocation::Inline`]. Any other value is treated as a path template:
/// every occurrence of the `{id}` placeholder — or its graph-template alias
/// `{container.id}` — is replaced with `container_id` and the result wrapped in
/// [`PlanDocLocation::External`]. A template with no placeholder is used verbatim
/// (a fixed shared plan path).
///
/// # Examples
///
/// ```
/// use jit::commands::plan_doc::{resolve_plan_doc_location, PlanDocLocation};
///
/// // A template without `{id}` is used as-is.
/// assert_eq!(
///     resolve_plan_doc_location("dev/plan.md", "abc123"),
///     PlanDocLocation::External("dev/plan.md".into())
/// );
/// ```
pub fn resolve_plan_doc_location(template: &str, container_id: &str) -> PlanDocLocation {
    if template == INLINE_LOCATION {
        PlanDocLocation::Inline
    } else {
        PlanDocLocation::External(PathBuf::from(
            template
                .replace(CONTAINER_ID_PLACEHOLDER, container_id)
                .replace(ID_PLACEHOLDER, container_id),
        ))
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
///
/// # Examples
///
/// ```no_run
/// use jit::commands::plan_doc::load_plan_content;
/// use jit::domain::Issue;
/// use std::path::Path;
///
/// let issue = Issue::new("Container".into(), "## Plan\n\n- step one\n".into());
/// // Inline: the body is returned without reading any file.
/// let content = load_plan_content(&issue, "inline", &issue.id, Path::new(".")).unwrap();
/// assert!(content.contains("step one"));
/// ```
pub fn load_plan_content(
    issue: &Issue,
    template: &str,
    container_id: &str,
    base_dir: &Path,
) -> Result<String, PlanDocError> {
    match resolve_plan_doc_location(template, container_id) {
        PlanDocLocation::Inline => Ok(issue.description.clone()),
        PlanDocLocation::External(relative) => {
            let path = if relative.is_absolute() {
                relative
            } else {
                base_dir.join(relative)
            };
            std::fs::read_to_string(&path).map_err(|source| PlanDocError::Read {
                container_id: container_id.to_string(),
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
///
/// # Examples
///
/// ```no_run
/// use jit::commands::plan_doc::project_plan_doc;
/// use jit::domain::{ContentFormat, Issue};
/// use std::path::Path;
///
/// let issue = Issue::new("Container".into(), "## Plan\n\n- step one\n".into());
/// let projection = project_plan_doc(
///     &issue,
///     "inline",
///     &issue.id,
///     Path::new("."),
///     ContentFormat::Markdown,
/// )
/// .unwrap();
/// // The plan body parsed into a `sections` view the pure engine can validate.
/// assert!(projection.sections.is_some());
/// ```
pub fn project_plan_doc(
    issue: &Issue,
    template: &str,
    container_id: &str,
    base_dir: &Path,
    repo_default_format: ContentFormat,
) -> Result<Projection, PlanDocError> {
    let content = load_plan_content(issue, template, container_id, base_dir)?;
    let parser = content_parser_for(issue.content_format, repo_default_format)?;
    // PURE from here: project the cheap selector fields, then attach the section
    // view computed from the RESOLVED content. No filesystem access.
    Ok(project(issue).with_sections(&content, parser.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn container(description: &str) -> Issue {
        Issue::new("Container".to_string(), description.to_string())
    }

    // --- resolve_plan_doc_location (pure) ---------------------------------

    #[test]
    fn test_resolve_inline_sentinel_is_inline() {
        assert_eq!(
            resolve_plan_doc_location("inline", "abc123"),
            PlanDocLocation::Inline
        );
    }

    #[test]
    fn test_resolve_substitutes_id_placeholder() {
        assert_eq!(
            resolve_plan_doc_location("plans/{id}.md", "abc123"),
            PlanDocLocation::External(PathBuf::from("plans/abc123.md"))
        );
    }

    #[test]
    fn test_resolve_substitutes_every_id_occurrence() {
        assert_eq!(
            resolve_plan_doc_location("{id}/plan-{id}.md", "xyz"),
            PlanDocLocation::External(PathBuf::from("xyz/plan-xyz.md"))
        );
    }

    #[test]
    fn test_resolve_substitutes_container_id_alias() {
        // Graph-template `doc` strings use `{container.id}`; the resolver maps it
        // to the container id exactly like `{id}` so a template's plan-doc
        // location resolves at validation time.
        assert_eq!(
            resolve_plan_doc_location("dev/active/{container.id}-plan.md", "abc123"),
            PlanDocLocation::External(PathBuf::from("dev/active/abc123-plan.md"))
        );
    }

    #[test]
    fn test_resolve_template_without_placeholder_is_verbatim() {
        assert_eq!(
            resolve_plan_doc_location("dev/plan.md", "abc123"),
            PlanDocLocation::External(PathBuf::from("dev/plan.md"))
        );
    }

    // --- load_plan_content (boundary) -------------------------------------

    #[test]
    fn test_load_inline_returns_issue_body() {
        let issue = container("## Plan\n\n- inline step\n");
        let dir = TempDir::new().unwrap();
        let content = load_plan_content(&issue, "inline", &issue.id, dir.path()).unwrap();
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
        let content = load_plan_content(&issue, "plans/{id}.md", "abc123", dir.path()).unwrap();
        assert_eq!(content, "## Plan\n\n- external step\n");
    }

    #[test]
    fn test_load_missing_external_path_yields_contextual_error() {
        let dir = TempDir::new().unwrap();
        let issue = container("body");
        let err = load_plan_content(&issue, "plans/{id}.md", "abc123", dir.path()).unwrap_err();
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
        let content =
            load_plan_content(&issue, abs.to_str().unwrap(), "abc123", other_base.path()).unwrap();
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
            "c1",
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
            "c1",
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
            &issue.id,
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
            "abc123",
            dir.path(),
            ContentFormat::Markdown,
        );
        assert!(matches!(result, Err(PlanDocError::Read { .. })));
    }
}
