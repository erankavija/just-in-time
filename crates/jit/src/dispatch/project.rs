//! `jit project` handlers.

use super::{count_message, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::ProjectCommands;
use crate::commands::ProjectRenderResult;

fn render<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    name: Option<String>,
) -> HandlerResult<ProjectRenderResult> {
    let response = ctx.executor.items().project_render(name.as_deref())?;
    let message = count_message(response.count(), "Rendered ", " projection(s)");
    Ok(Outcome::success_with(response, message))
}

command_group! {
    command: ProjectCommands,
    storage: [HandlerStore],
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
