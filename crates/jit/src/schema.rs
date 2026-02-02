//! Command schema export for AI agent introspection.
//!
//! This module provides automatic JSON schema generation from CLI definitions
//! using clap's introspection API. The schema enables AI agents to discover
//! available commands, arguments, and types.

use clap::{Arg, ArgAction, CommandFactory};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Complete command schema for the JIT CLI
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandSchema {
    /// CLI version
    pub version: String,
    /// Global options available on all commands
    pub global_options: Vec<Flag>,
    /// Available commands mapped by name
    pub commands: HashMap<String, Command>,
    /// Type definitions (Issue, State, Priority, etc.)
    pub types: HashMap<String, Value>,
    /// Exit code documentation
    pub exit_codes: Vec<ExitCodeDoc>,
}

/// Command definition
#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    /// Command description
    pub description: String,
    /// Subcommands (for issue, dep, gate, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subcommands: Option<HashMap<String, Command>>,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<Argument>,
    /// Command flags
    #[serde(default)]
    pub flags: Vec<Flag>,
    /// Output schema reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputSchema>,
}

/// Argument definition
#[derive(Debug, Serialize, Deserialize)]
pub struct Argument {
    /// Argument name
    pub name: String,
    /// Argument type (string, number, boolean, array)
    #[serde(rename = "type")]
    pub arg_type: String,
    /// Whether argument is required
    pub required: bool,
    /// Default value if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Flag definition
#[derive(Debug, Serialize, Deserialize)]
pub struct Flag {
    /// Flag name (without --)
    pub name: String,
    /// Flag type
    #[serde(rename = "type")]
    pub flag_type: String,
    /// Whether flag is required
    pub required: bool,
    /// Description
    pub description: String,
}

/// Output schema - contains actual JSON Schema for structured responses
#[derive(Debug, Serialize, Deserialize)]
pub struct OutputSchema {
    /// Success output JSON Schema (full schema, not just a reference)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_schema: Option<Value>,
    /// Success output type name (for documentation)
    pub success: String,
    /// Error output type reference
    pub error: String,
}

/// Exit code documentation
#[derive(Debug, Serialize, Deserialize)]
pub struct ExitCodeDoc {
    /// Exit code number
    pub code: i32,
    /// Description
    pub description: String,
}

impl CommandSchema {
    /// Generate schema automatically from clap definitions
    pub fn generate() -> Self {
        let cli = crate::cli::Cli::command();

        // Extract global options (flags that apply to all commands)
        let global_options: Vec<Flag> = cli
            .get_arguments()
            .filter(|arg| {
                !arg.is_positional()
                    && arg.is_global_set()
                    && arg.get_id() != "help"
                    && arg.get_id() != "version"
            })
            .map(Self::extract_flag)
            .collect();

        let mut commands = HashMap::new();

        // Extract top-level commands
        for subcmd in cli.get_subcommands() {
            let name = subcmd.get_name();

            // Skip help command
            if name == "help" {
                continue;
            }

            let cmd = Self::extract_command_with_path(subcmd, name);
            commands.insert(name.to_string(), cmd);
        }

        CommandSchema {
            version: env!("CARGO_PKG_VERSION").to_string(),
            global_options,
            commands,
            types: Self::generate_types(),
            exit_codes: Self::generate_exit_codes(),
        }
    }

    /// Extract command from clap Command with full path for output schema lookup
    fn extract_command_with_path(clap_cmd: &clap::Command, cmd_path: &str) -> Command {
        let description = clap_cmd
            .get_about()
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Check if this has subcommands
        let subcommands_vec: Vec<_> = clap_cmd.get_subcommands().collect();

        let subcommands = if !subcommands_vec.is_empty() {
            let mut sub_map = HashMap::new();
            for subcmd in subcommands_vec {
                let name = subcmd.get_name();
                if name != "help" {
                    // Build path for subcommand (e.g., "issue_show")
                    let sub_path = format!("{}_{}", cmd_path, name);
                    sub_map.insert(
                        name.to_string(),
                        Self::extract_command_with_path(subcmd, &sub_path),
                    );
                }
            }
            Some(sub_map)
        } else {
            None
        };

        // Extract arguments and flags
        let mut args = Vec::new();
        let mut flags = Vec::new();

        for arg in clap_cmd.get_arguments() {
            // Skip built-in help/version flags
            if arg.get_id() == "help" || arg.get_id() == "version" {
                continue;
            }

            if arg.is_positional() {
                args.push(Self::extract_argument(arg));
            } else {
                flags.push(Self::extract_flag(arg));
            }
        }

        Command {
            description,
            subcommands,
            args,
            flags,
            output: Self::get_output_schema_for_command(cmd_path),
        }
    }

    /// Extract argument from clap Arg
    fn extract_argument(arg: &Arg) -> Argument {
        let name = arg.get_id().to_string();
        let arg_type = Self::infer_arg_type(arg);
        let required = arg.is_required_set();
        let default = arg
            .get_default_values()
            .first()
            .and_then(|v| v.to_str())
            .map(|s| s.to_string());
        let description = arg
            .get_help()
            .map(|s| s.to_string())
            .or_else(|| arg.get_long_help().map(|s| s.to_string()));

        Argument {
            name,
            arg_type,
            required,
            default,
            description,
        }
    }

    /// Extract flag from clap Arg
    fn extract_flag(arg: &Arg) -> Flag {
        // Use the long flag name if available (--doc-type), otherwise use ID (doc_type)
        let name = arg
            .get_long()
            .map(|s| s.to_string())
            .unwrap_or_else(|| arg.get_id().to_string());

        let flag_type = Self::infer_flag_type(arg);
        let required = arg.is_required_set();

        // Get description, with fallback for common flags
        let description = arg
            .get_help()
            .map(|s| s.to_string())
            .or_else(|| arg.get_long_help().map(|s| s.to_string()))
            .unwrap_or_else(|| {
                // Provide default descriptions for common flags
                match name.as_str() {
                    "json" => "Output JSON format".to_string(),
                    _ => String::new(),
                }
            });

        Flag {
            name,
            flag_type,
            required,
            description,
        }
    }

    /// Infer argument type from clap Arg
    fn infer_arg_type(arg: &Arg) -> String {
        // Check if it's a repeating argument
        if matches!(arg.get_action(), ArgAction::Append) {
            return "array<string>".to_string();
        }

        // Check value parser hints
        let value_parser = arg.get_value_parser();
        let type_id = value_parser.type_id();

        if type_id == std::any::TypeId::of::<String>() {
            "string".to_string()
        } else if type_id == std::any::TypeId::of::<i32>()
            || type_id == std::any::TypeId::of::<i64>()
            || type_id == std::any::TypeId::of::<u32>()
            || type_id == std::any::TypeId::of::<u64>()
            || type_id == std::any::TypeId::of::<usize>()
        {
            "number".to_string()
        } else if type_id == std::any::TypeId::of::<bool>() {
            "boolean".to_string()
        } else {
            // Default to string
            "string".to_string()
        }
    }

    /// Infer flag type from clap Arg
    fn infer_flag_type(arg: &Arg) -> String {
        match arg.get_action() {
            ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count => "boolean".to_string(),
            ArgAction::Append => {
                // For repeatable flags, use array<string> format for consistency
                "array<string>".to_string()
            }
            _ => Self::infer_arg_type(arg),
        }
    }

    /// Get output schema for a specific command path (e.g., "issue_show", "query_available")
    fn get_output_schema_for_command(cmd_path: &str) -> Option<OutputSchema> {
        use crate::domain::Issue;
        use crate::output::*;

        // Helper to convert schemars schema to serde_json Value
        fn schema_to_value<T: JsonSchema>() -> Value {
            serde_json::to_value(schema_for!(T)).unwrap_or(json!({}))
        }

        let (schema, type_name) = match cmd_path {
            // Status command
            "status" => (Some(schema_to_value::<StatusResponse>()), "StatusResponse"),

            // Issue commands
            "issue_show" => (Some(schema_to_value::<Issue>()), "Issue"),
            "issue_create" => (Some(schema_to_value::<Issue>()), "Issue"),
            "issue_update" => (Some(schema_to_value::<Issue>()), "Issue"),

            // Query commands
            "query_available" | "query_all" | "query_ready" | "query_strategic" | "query_closed" => {
                (Some(schema_to_value::<IssueListResponse>()), "IssueListResponse")
            }
            "query_blocked" => (
                Some(schema_to_value::<BlockedListResponse>()),
                "BlockedListResponse",
            ),

            // Graph commands
            "graph_deps" => (Some(schema_to_value::<GraphDepsResponse>()), "GraphDepsResponse"),
            "graph_downstream" => (
                Some(schema_to_value::<GraphDownstreamResponse>()),
                "GraphDownstreamResponse",
            ),
            "graph_roots" => (Some(schema_to_value::<GraphRootsResponse>()), "GraphRootsResponse"),

            // Registry commands
            "registry_list" | "gate_list" => (
                Some(schema_to_value::<RegistryListResponse>()),
                "RegistryListResponse",
            ),

            // Default: no specific schema
            _ => (None, "CommandResponse"),
        };

        Some(OutputSchema {
            success_schema: schema,
            success: type_name.to_string(),
            error: "ErrorResponse".to_string(),
        })
    }

    fn generate_types() -> HashMap<String, Value> {
        let mut types = HashMap::new();

        types.insert(
            "State".to_string(),
            json!({
                "type": "enum",
                "enum": ["backlog", "ready", "in_progress", "gated", "done", "archived"],
                "description": "Issue lifecycle state"
            }),
        );

        types.insert(
            "Priority".to_string(),
            json!({
                "type": "enum",
                "enum": ["low", "normal", "high", "critical"],
                "description": "Issue priority level"
            }),
        );

        types.insert(
            "Issue".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "state": { "$ref": "#/types/State" },
                    "priority": { "$ref": "#/types/Priority" },
                    "assignee": { "type": ["string", "null"] },
                    "dependencies": { "type": "array", "items": { "type": "string" } },
                    "gates_required": { "type": "array", "items": { "type": "string" } },
                    "gates_status": { "type": "object" },
                    "context": { "type": "object" }
                }
            }),
        );

        types.insert(
            "ErrorResponse".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean", "const": false },
                    "error": {
                        "type": "object",
                        "properties": {
                            "code": { "type": "string" },
                            "message": { "type": "string" },
                            "suggestion": { "type": ["string", "null"] }
                        }
                    }
                }
            }),
        );

        types
    }

    fn generate_exit_codes() -> Vec<ExitCodeDoc> {
        vec![
            ExitCodeDoc {
                code: 0,
                description: "Command succeeded".to_string(),
            },
            ExitCodeDoc {
                code: 1,
                description: "Generic error occurred".to_string(),
            },
            ExitCodeDoc {
                code: 2,
                description: "Invalid arguments or usage error".to_string(),
            },
            ExitCodeDoc {
                code: 3,
                description: "Resource not found (issue, gate, etc.)".to_string(),
            },
            ExitCodeDoc {
                code: 4,
                description: "Validation failed (cycle detected, broken references, etc.)"
                    .to_string(),
            },
            ExitCodeDoc {
                code: 5,
                description: "Permission denied".to_string(),
            },
            ExitCodeDoc {
                code: 6,
                description: "Resource already exists".to_string(),
            },
            ExitCodeDoc {
                code: 10,
                description: "External dependency failed (git, file system, etc.)".to_string(),
            },
        ]
    }
}
