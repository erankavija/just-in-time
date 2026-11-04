//! `jit project` handlers.

use super::{CommandContext, Outcome};
use crate::cli::ProjectCommands;
use crate::commands::ProjectRenderResult;
use crate::storage::{IssueStore, RepositoryStateStore};

fn render<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    name: Option<String>,
) -> anyhow::Result<Outcome<ProjectRenderResult>> {
    let response = ctx.executor.items().project_render(name.as_deref())?;
    let message = format!("Rendered {} projection(s)", response.count());
    Ok(Outcome::success_with(response, message))
}

command_group! {
    command: ProjectCommands,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "project_render" => ProjectCommands::Render { name } => render(name) -> ProjectRenderResult,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_project_render_is_named_for_its_list_result() {
        let registered = crate::dispatch::registered_output_schema("project_render").unwrap();

        assert_eq!(registered.success, "ProjectRenderResult");
        let schema = registered.success_schema.unwrap();
        assert!(schema["properties"]["count"].is_object());
        assert_eq!(schema["properties"]["projections"]["type"], "array");
    }
}
