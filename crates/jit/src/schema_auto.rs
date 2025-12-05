//! Automatic schema generation from clap command definitions.
//!
//! This module uses clap's introspection API to automatically generate
//! JSON schemas from CLI definitions, eliminating manual duplication.

use crate::schema::{Argument, Command, CommandSchema, Flag, OutputSchema};
use clap::{Arg, ArgAction, CommandFactory};
use std::collections::HashMap;

impl CommandSchema {
    /// Generate schema automatically from clap definitions
    pub fn generate_auto() -> Self {
        let cli = crate::cli::Cli::command();

        let mut commands = HashMap::new();

        // Extract top-level commands
        for subcmd in cli.get_subcommands() {
            let name = subcmd.get_name();

            // Skip help command
            if name == "help" {
                continue;
            }

            let cmd = Self::extract_command(subcmd);
            commands.insert(name.to_string(), cmd);
        }

        // Get types and exit codes from the manual schema generation
        // (these are stable and don't need auto-generation)
        let manual = crate::schema::CommandSchema::generate();

        CommandSchema {
            version: env!("CARGO_PKG_VERSION").to_string(),
            commands,
            types: manual.types,
            exit_codes: manual.exit_codes,
        }
    }

    /// Extract command from clap Command
    fn extract_command(clap_cmd: &clap::Command) -> Command {
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
                    sub_map.insert(name.to_string(), Self::extract_command(subcmd));
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
            output: Self::infer_output_schema(clap_cmd.get_name()),
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
            description,
        }
    }

    /// Infer argument type from clap Arg
    fn infer_arg_type(arg: &Arg) -> String {
        // Check if it's a repeating argument
        if matches!(arg.get_action(), ArgAction::Append) {
            return "array".to_string();
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

    /// Infer output schema based on command name
    fn infer_output_schema(cmd_name: &str) -> Option<OutputSchema> {
        // Commands that don't have structured output
        let no_output = ["init", "validate"];
        if no_output.contains(&cmd_name) {
            return Some(OutputSchema {
                success: "void".to_string(),
                error: "ErrorResponse".to_string(),
            });
        }

        // Most commands have some output
        Some(OutputSchema {
            success: "CommandResponse".to_string(),
            error: "ErrorResponse".to_string(),
        })
    }


}
