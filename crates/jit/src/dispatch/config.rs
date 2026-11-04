//! `jit config` handlers.

use super::{CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::ConfigCommands;
use crate::output::config::{
    ConfigGetResponse, ConfigSetResponse, ConfigShowResponse, ConfigValidateResponse,
    TypeHierarchyResponse,
};
use crate::output::ErrorCode;
use crate::storage::IssueStore;

fn show<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<ConfigShowResponse> {
    Ok(Outcome::success_with(
        ConfigShowResponse(ctx.executor.config().effective_config_snapshot()?),
        "Effective configuration",
    ))
}

fn get<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
) -> HandlerResult<ConfigGetResponse> {
    Ok(ConfigGetResponse(ctx.executor.config().get_config(&key)?).into())
}

fn set<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
    value: String,
) -> HandlerResult<ConfigSetResponse> {
    Ok(ConfigSetResponse(ctx.executor.config().set_config(&key, &value)?).into())
}

/// An invalid configuration source is a registered validation failure; its
/// report carries the errors under `--json` and the summary is the message.
fn validate<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<ConfigValidateResponse> {
    let response = ConfigValidateResponse(ctx.executor.config().validate()?);
    Ok(if response.0.valid {
        Outcome::success_with(response, "Configuration is valid")
    } else {
        Outcome::findings(
            ErrorCode::ValidationFailed,
            format!(
                "Configuration validation failed with {} error(s)",
                response.0.errors.len()
            ),
            response,
        )
    })
}

fn show_hierarchy<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> HandlerResult<TypeHierarchyResponse> {
    Ok(Outcome::success_with(
        TypeHierarchyResponse(ctx.executor.config().declared_type_hierarchy()?),
        "Type hierarchy",
    ))
}

command_group! {
    command: ConfigCommands,
    storage: [HandlerStore],
    leaves: {
        "config_show" => ConfigCommands::Show => show() -> ConfigShowResponse,
        "config_get" => ConfigCommands::Get { key } => get(key) -> ConfigGetResponse,
        "config_set" => ConfigCommands::Set { key, value } => set(key, value) -> ConfigSetResponse,
        "config_validate" => ConfigCommands::Validate => validate() -> ConfigValidateResponse,
        "config_show-hierarchy" => ConfigCommands::ShowHierarchy => show_hierarchy() -> TypeHierarchyResponse,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_config_leaves_name_their_response_types() {
        let expected = [
            ("config_show", "ConfigShowResponse"),
            ("config_get", "ConfigGetResponse"),
            ("config_set", "ConfigSetResponse"),
            ("config_validate", "ConfigValidateResponse"),
            ("config_show-hierarchy", "TypeHierarchyResponse"),
        ];

        for (path, response) in expected {
            let registered = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(registered.success, response, "leaf {path}");
        }
    }
}
