//! `jit label` handlers.

use super::{CommandContext, HandlerResult, Outcome};
use crate::cli::LabelCommands;
use crate::config_manager::ConfigManager;
use crate::output::{
    LabelValuesResponse, LabelValuesScope, NamespacesHumanDetails, NamespacesResponse,
};
use crate::storage::IssueStore;

fn namespaces<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<NamespacesResponse> {
    let namespaces = ConfigManager::new(ctx.executor.storage().repository_layout()?)
        .get_namespaces()?
        .namespaces;
    let (names, details): (Vec<_>, Vec<_>) = namespaces.into_iter().unzip();
    let response = NamespacesResponse::with_extras(
        names,
        NamespacesHumanDetails {
            namespaces: details,
        },
    );
    let message = format!("{} namespace(s)", response.count());
    Ok(Outcome::success_with(response, message))
}

fn values<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    namespace: String,
) -> HandlerResult<LabelValuesResponse> {
    let values = ctx.executor.issues().list_label_values(&namespace)?;
    let response = LabelValuesResponse::with_extras(values, LabelValuesScope { namespace });
    let message = format!("{} value(s)", response.count());
    Ok(Outcome::success_with(response, message))
}

command_group! {
    command: LabelCommands,
    storage: [IssueStore],
    leaves: {
        "label_namespaces" => LabelCommands::Namespaces => namespaces() -> NamespacesResponse,
        "label_values" => LabelCommands::Values { namespace } => values(namespace) -> LabelValuesResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use crate::dispatch::registered_output_schema;
    use crate::output::{ExitCode, OutputMode};
    use crate::storage::InMemoryStorage;

    fn executor() -> crate::commands::CommandExecutor<InMemoryStorage> {
        memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(
                    "[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n\
                     [namespaces.component]\ndescription = \"Software component\"\nunique = false\n",
                ),
                ..MemoryDeclarations::default()
            },
        )
    }

    #[test]
    fn test_dispatch_namespaces_json_keeps_list_payload_and_named_schema() {
        let executor = executor();
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode {
                json: true,
                quiet: false,
            },
        };
        let mut out = Vec::new();

        let mut err = Vec::new();
        let status = dispatch(LabelCommands::Namespaces, &ctx, &mut out, &mut err).unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(printed["count"], 2);
        assert_eq!(printed["namespaces"].as_array().unwrap().len(), 2);
        assert!(printed.get("namespace").is_none());
        assert_eq!(printed["message"], "2 namespace(s)");
        assert_eq!(status, ExitCode::Success);
        assert!(err.is_empty());
        assert_eq!(
            registered_output_schema("label_namespaces")
                .unwrap()
                .success,
            "NamespacesResponse"
        );
        let schema = registered_output_schema("label_namespaces")
            .unwrap()
            .success_schema
            .unwrap();
        assert!(schema["properties"]["namespaces"].is_object());
        assert!(schema["properties"]["namespace"].is_null());
    }

    #[test]
    fn test_dispatch_values_json_keeps_namespace_as_list_extra_and_named_schema() {
        let executor = executor();
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode {
                json: true,
                quiet: false,
            },
        };
        let mut out = Vec::new();

        let mut err = Vec::new();
        let status = dispatch(
            LabelCommands::Values {
                namespace: "type".to_owned(),
            },
            &ctx,
            &mut out,
            &mut err,
        )
        .unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(printed["count"], 0);
        assert_eq!(printed["values"], serde_json::json!([]));
        assert_eq!(printed["namespace"], "type");
        assert_eq!(printed["message"], "0 value(s)");
        assert_eq!(status, ExitCode::Success);
        assert!(err.is_empty());
        let schema = registered_output_schema("label_values").unwrap();
        assert_eq!(schema.success, "LabelValuesResponse");
        let success_schema = schema.success_schema.unwrap();
        assert!(success_schema["properties"]["values"].is_object());
        assert!(success_schema["properties"]["namespace"].is_object());
    }
}
