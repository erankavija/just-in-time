//! `jit apply` handler.

use super::{CommandContext, Outcome};
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::output::TemplateApplyResponse;
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::anyhow;
use std::collections::BTreeMap;

/// The parsed arguments of `jit apply`.
pub struct Invocation<'a> {
    pub template: &'a str,
    pub container: &'a str,
    /// Repeatable `role=id` anchor bindings.
    pub anchors: &'a [String],
    pub force: bool,
}

fn apply<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    template: &str,
    container: &str,
    anchors: &[String],
    force: bool,
) -> anyhow::Result<Outcome<TemplateApplyResponse>> {
    let templates = ctx.executor.templates();
    let resolve = |requested: &str| -> anyhow::Result<IssueId> {
        templates.resolve_issue(&requested.parse::<IssueIdPrefix>()?)
    };
    let bindings = anchors
        .iter()
        .map(|pair| {
            let (role, id) = pair.split_once('=').ok_or_else(|| {
                anyhow!("malformed --anchor '{pair}'; expected role=id (with an '=')")
            })?;
            if role.is_empty() {
                return Err(anyhow!(
                    "malformed --anchor '{pair}'; the role (left of '=') must not be empty"
                ));
            }
            Ok((role.to_owned(), resolve(id)?))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let container = resolve(container)?;
    let (result, warnings) = templates.apply_template(template, &container, &bindings, force)?;
    warnings
        .iter()
        .for_each(|warning| eprintln!("Warning: {warning}"));
    let response = TemplateApplyResponse::new(result, container, ctx.mode.quiet);
    let message = Some(response.message());
    Ok(Outcome::Success { response, message })
}

command_group! {
    command: Invocation<'_>,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "apply" => Invocation { template, container, anchors, force } => apply(template, container, anchors, force) -> TemplateApplyResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::memory_executor;
    use crate::errors::InvalidArgumentError;
    use crate::output::{ExitCode, OutputMode};
    use crate::storage::InMemoryStorage;

    fn apply_with_anchor(anchor: &str) -> anyhow::Error {
        let executor = memory_executor(InMemoryStorage::new());
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode::default(),
        };
        let anchors = [anchor.to_owned()];
        dispatch(
            Invocation {
                template: "plan",
                container: "abcdef12",
                anchors: &anchors,
                force: false,
            },
            &ctx,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err()
    }

    #[test]
    fn test_dispatch_anchor_without_equals_fails_before_any_lookup() {
        let error = apply_with_anchor("container");

        assert!(error.to_string().contains("expected role=id"));
    }

    #[test]
    fn test_dispatch_anchor_with_empty_role_fails_before_any_lookup() {
        let error = apply_with_anchor("=abcdef12");

        assert!(error.to_string().contains("role"));
    }

    #[test]
    fn test_dispatch_unresolvable_container_classifies_as_not_found() {
        let error = apply_with_anchor("container=abcdef12");

        assert_eq!(
            crate::output::failure::classify(&error).exit_code(),
            ExitCode::NotFound
        );
        assert!(!error.is::<InvalidArgumentError>());
    }

    #[test]
    fn test_registered_output_schema_apply_is_named_for_its_response_type() {
        let registered = crate::dispatch::registered_output_schema("apply").unwrap();

        assert_eq!(registered.success, "TemplateApplyResponse");
    }
}
