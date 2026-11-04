//! `jit apply` handler.

use super::{resolve_issue, CommandContext, Outcome};
use crate::cli::ApplyArgs;
use crate::output::TemplateApplyResponse;
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::anyhow;
use std::collections::BTreeMap;

fn apply<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    template: &str,
    container: &str,
    anchors: &[String],
    force: bool,
) -> anyhow::Result<Outcome<TemplateApplyResponse>> {
    let templates = ctx.executor.templates();
    let resolve = |requested: &str| resolve_issue(ctx.executor.storage(), requested);
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
    let response = TemplateApplyResponse {
        template: result.template,
        container,
        anchor_bindings: result.anchor_bindings,
        created_node_ids_by_role: result.created_node_ids_by_role,
        anchor_dependency_snapshots: result.anchor_dependency_snapshots,
        created_issues: result.created_issues,
    };
    let message = response.message();
    Ok(Outcome::success_with(response, message).with_warnings(warnings))
}

command_group! {
    command: ApplyArgs,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "apply" => ApplyArgs { template, container, anchor, force } => apply(&template, &container, &anchor, force) -> TemplateApplyResponse,
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
        let anchors = vec![anchor.to_owned()];
        dispatch(
            ApplyArgs {
                template: "plan".to_owned(),
                container: "abcdef12".to_owned(),
                anchor: anchors,
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
