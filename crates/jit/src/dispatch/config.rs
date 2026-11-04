//! `jit config` handlers.

use super::{CommandContext, Outcome};
use crate::cli::ConfigCommands;
use crate::output::config::{
    ConfigGetResponse, ConfigSetResponse, ConfigShowResponse, ConfigValidateResponse,
    TypeHierarchyResponse,
};
use crate::output::ErrorCode;
use crate::storage::{IssueStore, RepositoryStateStore};

fn show<S: IssueStore>(ctx: &CommandContext<'_, S>) -> anyhow::Result<Outcome<ConfigShowResponse>> {
    Ok(Outcome::Success {
        response: ConfigShowResponse(ctx.executor.config().effective_config_snapshot()?),
        message: Some("Effective configuration".to_owned()),
    })
}

fn get<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
) -> anyhow::Result<Outcome<ConfigGetResponse>> {
    Ok(Outcome::Success {
        response: ConfigGetResponse(ctx.executor.config().get_config(&key)?),
        message: None,
    })
}

fn set<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
    value: String,
) -> anyhow::Result<Outcome<ConfigSetResponse>> {
    Ok(Outcome::Success {
        response: ConfigSetResponse(ctx.executor.config().set_config(&key, &value)?),
        message: None,
    })
}

/// An invalid configuration source is a registered validation failure; its
/// report carries the errors under `--json` and the summary is the message.
fn validate<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<ConfigValidateResponse>> {
    let response = ConfigValidateResponse(ctx.executor.config().validate()?);
    Ok(if response.0.valid {
        Outcome::Success {
            response,
            message: Some("Configuration is valid".to_owned()),
        }
    } else {
        Outcome::Findings {
            code: ErrorCode::ValidationFailed,
            message: format!(
                "Configuration validation failed with {} error(s)",
                response.0.errors.len()
            ),
            report: Box::new(response),
        }
    })
}

fn show_hierarchy<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<TypeHierarchyResponse>> {
    Ok(Outcome::Success {
        response: TypeHierarchyResponse(ctx.executor.config().declared_type_hierarchy()?),
        message: Some("Type hierarchy".to_owned()),
    })
}

command_group! {
    command: ConfigCommands,
    storage: [IssueStore + RepositoryStateStore],
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
