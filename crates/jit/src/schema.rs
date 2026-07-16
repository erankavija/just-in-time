//! Command schema export for AI agent introspection.
//!
//! This module provides automatic JSON schema generation from CLI definitions
//! using clap's introspection API. The schema enables AI agents to discover
//! available commands, arguments, and types.

use crate::domain::EventTagDoc;
use clap::{Arg, ArgAction, CommandFactory};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

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
    /// Per-command-family exit-code mappings, including exceptions to the global
    /// taxonomy in `exit_codes`.
    pub command_exit_codes: Vec<CommandExitCode>,
    /// The event-log tag vocabulary: every `type` tag `.jit/events.jsonl` stores,
    /// its association scope, and whether its records carry an `issue_id`.
    /// Projected from the `Event` type by
    /// [`event_catalog`](crate::domain::event_catalog).
    pub events: Vec<EventTagDoc>,
}

/// Command definition
#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    /// Command description
    pub description: String,
    /// Whether this command is hidden from default MCP tool listing
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Alternate accepted invocations (visible command aliases, without a
    /// leading path; e.g. `dependency` for `dep`, `pass`/`eval` for
    /// `gate evaluate`).
    ///
    /// Empty for commands with no visible alias, and then skipped in the
    /// serialized schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
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
    /// Alternate accepted long spellings (visible aliases, without `--`).
    ///
    /// Empty for flags with no visible alias, and then skipped in the
    /// serialized schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
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

/// Exit code a command family emits under a named condition.
///
/// The global taxonomy in [`ExitCodeDoc`] states what each code means in
/// general; this maps the codes back to the command families and conditions
/// that produce them. `exception` flags a row where the code's meaning here
/// departs from the global entry — a completed run signalling findings with a
/// non-zero code, or a pass-through of a subprocess's own code — rather than the
/// ordinary error classification.
///
/// The classification rows derive from the typed-error classifier
/// (`error_to_exit_code` in `crates/jit/src/main.rs`); the exception rows derive
/// from the direct-exit sites in the same dispatch. A test in `main.rs` verifies
/// the classification rows against `error_to_exit_code`, and
/// [`CommandExitCode::code`] is verified against the global taxonomy in
/// `schema.rs` tests, so this projection cannot silently drift from runtime
/// behavior (@/inv/single-source-prose).
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandExitCode {
    /// Command or command family (e.g. `gate evaluate`, `validate`), or `*` for
    /// every command.
    pub command: String,
    /// Exit code emitted, when the command chooses a fixed code (always a member
    /// of the global taxonomy in [`CommandSchema::exit_codes`]). `None` marks a
    /// pass-through, where the command exits with a subprocess's own code rather
    /// than a jit-assigned one (e.g. `serve`).
    pub code: Option<i32>,
    /// Condition that produces the code.
    pub condition: String,
    /// True when the code's meaning here departs from the global taxonomy entry
    /// for `code` (a completed-run findings signal, or a pass-through).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exception: bool,
}

impl CommandSchema {
    /// Generate schema automatically from clap definitions
    pub fn generate() -> Self {
        let cli = crate::cli::Cli::command();

        // Extract global options: every non-positional flag accepted at the
        // top level, including `--schema` and `--quiet`.
        let mut global_options: Vec<Flag> = cli
            .get_arguments()
            .filter(|arg| !arg.is_positional())
            .map(Self::extract_flag)
            .collect();

        // clap synthesizes `--help`/`--version` at parse time, so they are
        // absent from `get_arguments()`. Surface them explicitly so the schema
        // documents every globally accepted spelling (@/inv/single-source-prose:
        // these are clap's fixed spellings, not project-owned facts).
        global_options.extend(Self::builtin_global_flags());

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
            command_exit_codes: Self::generate_command_exit_codes(),
            events: crate::domain::event_catalog(),
        }
    }

    /// The `--help` and `--version` flags clap generates automatically.
    ///
    /// clap injects these during parsing rather than storing them as `Arg`
    /// entries, so they must be described here to appear in the schema.
    fn builtin_global_flags() -> Vec<Flag> {
        ["help", "version"]
            .into_iter()
            .map(|name| Flag {
                name: name.to_string(),
                flag_type: "boolean".to_string(),
                required: false,
                description: match name {
                    "help" => "Print help".to_string(),
                    _ => "Print version".to_string(),
                },
                aliases: Vec::new(),
            })
            .collect()
    }

    /// Commands hidden from the default MCP tool listing.
    /// These are still present in the schema and executable, but the MCP server
    /// filters them from `tools/list` unless `JIT_MCP_ALL_TOOLS=1` is set.
    fn hidden_commands() -> HashSet<&'static str> {
        [
            "claim_force-evict",
            "claim_heartbeat",
            "claim_list",
            "claim_status",
            "config_get",
            "config_list-templates",
            "config_set",
            "config_show",
            "config_show-hierarchy",
            "config_validate",
            "doc_assets_list",
            "doc_check-links",
            "doc_diff",
            "doc_history",
            "events_query",
            "events_tail",
            "gate_check",
            "gate_define",
            "gate_preset_apply",
            "gate_preset_create",
            "gate_preset_list",
            "gate_preset_show",
            "gate_remove",
            "gate_show",
            "graph_export",
            "graph_roots",
            "hooks_install",
            "init",
            "issue_assign",
            "issue_unassign",
            "query_closed",
            "query_strategic",
            "snapshot_export",
            "serve",
            "worktree_info",
            "worktree_list",
        ]
        .into_iter()
        .collect()
    }

    /// Extract command from clap Command with full path for output schema lookup
    fn extract_command_with_path(clap_cmd: &clap::Command, cmd_path: &str) -> Command {
        Self::extract_command_with_path_hidden(clap_cmd, cmd_path, false, &Self::hidden_commands())
    }

    fn extract_command_with_path_hidden(
        clap_cmd: &clap::Command,
        cmd_path: &str,
        parent_hidden: bool,
        hidden_commands: &HashSet<&str>,
    ) -> Command {
        let description = clap_cmd
            .get_about()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let hidden = parent_hidden || hidden_commands.contains(cmd_path);

        // Visible command aliases are additional accepted invocations (e.g.
        // `dependency` for `dep`). Hidden aliases stay out of the contract,
        // matching what `--help` advertises.
        let aliases: Vec<String> = clap_cmd
            .get_visible_aliases()
            .map(|s| s.to_string())
            .collect();

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
                        Self::extract_command_with_path_hidden(
                            subcmd,
                            &sub_path,
                            hidden,
                            hidden_commands,
                        ),
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
            hidden,
            aliases,
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

        // Visible long aliases are additional accepted spellings for this flag
        // (e.g. `--add-label` for `--label`). Hidden aliases stay out of the
        // contract, matching what `--help` advertises.
        let aliases = arg
            .get_visible_aliases()
            .map(|names| names.into_iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();

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
            aliases,
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
            // Version command
            "version" => (
                Some(schema_to_value::<crate::build_info::VersionInfo>()),
                "VersionInfo",
            ),

            // Status command
            "status" => (Some(schema_to_value::<StatusResponse>()), "StatusResponse"),

            // Embedded profile commands.
            "profile_list" => (
                Some(schema_to_value::<crate::profile::ProfileListResult>()),
                "ProfileListResult",
            ),
            "profile_show" => (
                Some(schema_to_value::<crate::profile::ProfileShowResult>()),
                "ProfileShowResult",
            ),
            "profile_apply" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<crate::profile::ProfileApplyResult>(),
                        schema_to_value::<crate::profile::ProfilePlanResult>(),
                    ],
                    "description": "Normal execution returns ProfileApplyResult. \
                        With --dry-run, returns the exact non-mutating \
                        ProfilePlanResult."
                });
                (Some(union), "ProfileApplyResult")
            }

            // Issue commands
            //
            // `issue show --json` returns `IssueShowResponse` by default and
            // `IssueShowSummaryResponse` when `--summary` is set. Expose both
            // shapes via a JSON Schema `oneOf` so MCP clients see an honest
            // contract.
            "issue_show" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<IssueShowResponse>(),
                        schema_to_value::<IssueShowSummaryResponse>(),
                        schema_to_value::<IssueShowListResponse>(),
                    ],
                    "description": "Single id: a bare IssueShowResponse \
                        (IssueShowSummaryResponse with --summary). Multiple ids: \
                        the {count, issues} envelope of full IssueShowResponse \
                        objects in argument order."
                });
                (Some(union), "IssueShowResponse")
            }
            // `issue create --json` returns the same enriched projection as
            // `issue show`, so its gate list is the `gates` array, not the raw
            // stored record.
            "issue_create" => (
                Some(schema_to_value::<IssueShowResponse>()),
                "IssueShowResponse",
            ),
            // Lifecycle mutation confirmations echo the raw stored record, so the
            // gate list stays under gates_required / gates_status. `claim` /
            // `claim-next` add the advisory `warnings` array (ClaimResponse);
            // `claim`'s `--assign-only` mode omits it, hence the `oneOf`.
            "issue_assign" | "issue_unassign" | "issue_reject" | "issue_release" => {
                (Some(schema_to_value::<Issue>()), "Issue")
            }
            "issue_claim" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<ClaimResponse>(),
                        schema_to_value::<Issue>(),
                    ],
                    "description": "The claimed issue's complete stored record \
                        (gate list under gates_required / gates_status). The \
                        default claim adds an advisory `warnings` array; \
                        `--assign-only` omits it."
                });
                (Some(union), "ClaimResponse")
            }
            "issue_claim-next" => (Some(schema_to_value::<ClaimResponse>()), "ClaimResponse"),
            "issue_update" => (
                Some(schema_to_value::<IssueUpdateResponse>()),
                "IssueUpdateResponse",
            ),
            // `issue status` projects each issue's gate list under the compact
            // `gates` array, the same field name `issue show` uses.
            "issue_status" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<IssueStatusResponse>(),
                        schema_to_value::<IssueStatusListResponse>(),
                    ],
                    "description": "Single id: a bare IssueStatusResponse. \
                        Multiple ids: the {count, issues} envelope of \
                        IssueStatusResponse objects in argument order."
                });
                (Some(union), "IssueStatusResponse")
            }
            // `issue children` wraps one `IssueStatusResponse` per child, so the
            // same compact `gates` array reaches each entry.
            "issue_children" => (
                Some(schema_to_value::<IssueChildrenResponse>()),
                "IssueChildrenResponse",
            ),

            // Gate commands
            "gate_check-all" => (
                Some(schema_to_value::<GateCheckAllResponse>()),
                "GateCheckAllResponse",
            ),

            // Issue-record LIST surfaces emit two shapes: the default lean summary
            // (`MinimalIssue` entries, no gate fields) and, under `--full`, the
            // complete stored records (gate list under `gates_required` /
            // `gates_status`). Declaring both via `oneOf` lets a consumer attribute
            // a missing gate field to the summary projection rather than the data.
            // `issue list` and its top-level `list` alias share the query shape.
            // Bare `jit query` (no subcommand) is documented as equivalent to
            // `query all`, so it declares the same contract.
            "query" | "query_available" | "query_all" | "query_strategic" | "query_closed"
            | "issue_list" | "list" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<IssueListResponse>(),
                        schema_to_value::<IssueListFullResponse>(),
                    ],
                    "description": "Default shape: lean MinimalIssue entries (id, \
                        short_id, title, state, priority) with no gate fields. \
                        With --full: complete stored issue records, whose gate list \
                        is carried under gates_required / gates_status."
                });
                (Some(union), "IssueListResponse")
            }
            // `issue search` mirrors the list surfaces, plus an echoed `query`.
            "issue_search" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<IssueSearchResponse>(),
                        schema_to_value::<IssueSearchFullResponse>(),
                    ],
                    "description": "Default shape: lean MinimalIssue entries with no \
                        gate fields, plus the echoed query. With --full: complete \
                        stored issue records, whose gate list is carried under \
                        gates_required / gates_status."
                });
                (Some(union), "IssueSearchResponse")
            }
            // `query blocked` is the one query whose --full variant is NOT a
            // record dump: both shapes build on MinimalIssue and carry no
            // gate-list fields; a blocking gate appears only as a reason.
            "query_blocked" => {
                let union = json!({
                    "oneOf": [
                        schema_to_value::<BlockedListResponse>(),
                        schema_to_value::<BlockedFullListResponse>(),
                    ],
                    "description": "Default shape: MinimalBlockedIssue entries \
                        (lean fields plus blocked_reasons strings). With --full: \
                        BlockedIssue entries (lean fields plus structured \
                        blocked_reasons). Neither shape carries gate-list fields; \
                        a blocking gate appears only as a reason entry."
                });
                (Some(union), "BlockedListResponse")
            }

            // `jit apply` echoes each created node's raw stored record under
            // `created_issues`, so their gate lists are carried under
            // gates_required / gates_status.
            "apply" => (
                Some(schema_to_value::<TemplateApplyResponse>()),
                "TemplateApplyResponse",
            ),

            // Graph commands
            "graph_deps" => (
                Some(schema_to_value::<GraphDepsTreeResponse>()),
                "GraphDepsTreeResponse",
            ),
            "graph_downstream" => (
                Some(schema_to_value::<GraphDownstreamResponse>()),
                "GraphDownstreamResponse",
            ),
            "graph_roots" => (
                Some(schema_to_value::<GraphRootsResponse>()),
                "GraphRootsResponse",
            ),
            "graph_tree" => (
                Some(schema_to_value::<GraphTreeResponse>()),
                "GraphTreeResponse",
            ),
            // `graph export --format json` emits two shapes: the default lean
            // summary nodes (no gate fields) and, under `--full`, complete stored
            // issue records (gate list under `gates_required` / `gates_status`)
            // with the resolved hierarchy flattened on. Declaring both via `oneOf`
            // makes the gate-free summary distinguishable from the full record.
            "graph_export" => {
                use crate::visualization::{GraphExportFullResponse, GraphExportSummaryResponse};
                let union = json!({
                    "oneOf": [
                        schema_to_value::<GraphExportSummaryResponse>(),
                        schema_to_value::<GraphExportFullResponse>(),
                    ],
                    "description": "Default shape: lean summary nodes (id, short_id, \
                        title, state, priority, labels) with no gate fields. With \
                        --full: complete stored issue records, whose gate list is \
                        carried under gates_required / gates_status, plus the \
                        resolved-hierarchy fields (parent, children, cluster, rank)."
                });
                (Some(union), "GraphExportSummaryResponse")
            }

            "gate_list" => (
                Some(schema_to_value::<GateListResponse>()),
                "GateListResponse",
            ),
            "label_namespaces" => (
                Some(schema_to_value::<NamespacesResponse>()),
                "NamespacesResponse",
            ),
            "search" => (Some(schema_to_value::<SearchResponse>()), "SearchResponse"),
            "worktree_list" => (
                Some(schema_to_value::<WorktreeListResponse>()),
                "WorktreeListResponse",
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
        use crate::domain::{GateChecker, State};

        let mut types = HashMap::new();

        // Derive the enum from the authoritative `State` enumeration so every
        // real variant (including `rejected`) surfaces and future variants
        // cannot silently drift out of the contract (@/inv/single-source-prose).
        let state_values: Vec<Value> = State::all()
            .iter()
            .map(|state| Value::String(state.as_str().to_string()))
            .collect();
        types.insert(
            "State".to_string(),
            json!({
                "type": "enum",
                "enum": state_values,
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
            "GateChecker".to_string(),
            serde_json::to_value(schema_for!(GateChecker)).unwrap_or(json!({})),
        );
        types.insert(
            "ProfileManifest".to_string(),
            serde_json::to_value(crate::profile::profile_manifest_schema()).unwrap_or(json!({})),
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

    /// Project the per-command-family exit-code mappings.
    ///
    /// The rows are grouped as: universal codes every command can emit, codes
    /// specific to a command family, and exceptions — codes a completed run
    /// emits to signal findings, or codes whose meaning departs from the global
    /// taxonomy. Classification rows mirror the typed-error classifier
    /// (`error_to_exit_code`, `crates/jit/src/main.rs`); exception rows mirror
    /// the direct process-exit sites in the same dispatch. Both are verified by
    /// test rather than hand-copied (@/inv/single-source-prose).
    fn generate_command_exit_codes() -> Vec<CommandExitCode> {
        let row = |command: &str, code: i32, condition: &str, exception: bool| CommandExitCode {
            command: command.to_string(),
            code: Some(code),
            condition: condition.to_string(),
            exception,
        };
        vec![
            // Universal: any command reaches these — `0` on successful completion,
            // the failure codes through the shared classifier.
            row("*", 0, "Command completed successfully.", false),
            row(
                "*",
                1,
                "No typed classifier matched the failure (generic error).",
                false,
            ),
            row(
                "*",
                2,
                "Invalid arguments or usage error, including an unresolvable, \
                 ambiguous, or too-short id prefix.",
                false,
            ),
            row(
                "*",
                3,
                "A referenced issue, gate, gate-run, preset, lease, repository, \
                 or file path was not found.",
                false,
            ),
            row("*", 5, "A filesystem operation was denied.", false),
            row(
                "*",
                10,
                "The repository's on-disk format is newer than this binary, a gate \
                 checker refused to run because the binary predates the repository \
                 under review, or a filesystem/subprocess I/O operation failed.",
                false,
            ),
            // Command-family classification (standard taxonomy meaning).
            row(
                "any command that writes an issue",
                4,
                "An enforcing validation rule rejected the write (the shared \
                 write-validation path blocks on rule findings).",
                false,
            ),
            row(
                "dep add",
                4,
                "The edge would create a cycle, or a redundant \
                 (transitively-implied) edge was rejected.",
                false,
            ),
            row(
                "issue update, issue claim, issue claim-next",
                4,
                "A state transition is blocked by unmet dependencies or unpassed \
                 gates.",
                false,
            ),
            row(
                "gate define",
                6,
                "The gate key is already registered.",
                false,
            ),
            row(
                "issue delete",
                2,
                "Deletion was refused for missing operator confirmation \
                 (JIT_ALLOW_DELETION=1 not set in the process environment).",
                false,
            ),
            row(
                "issue batch-create",
                2,
                "The batch file failed pre-validation; no issues were created.",
                false,
            ),
            row(
                "issue batch-create",
                10,
                "A write failed after some issues were already created.",
                false,
            ),
            row(
                "snapshot export",
                6,
                "The snapshot output path already exists.",
                false,
            ),
            row(
                "claim",
                10,
                "A lease subcommand was run outside a git repository (leases \
                 require git for worktree identity).",
                false,
            ),
            // Gate evaluation: the code carries the checker's verdict.
            row(
                "gate evaluate, gate evaluate-all",
                4,
                "A checker ran and its verdict was fail.",
                true,
            ),
            row(
                "gate evaluate, gate evaluate-all",
                10,
                "A checker could not run to a verdict (timeout, crash, or \
                 command not found).",
                true,
            ),
            // Findings signalled by a completed run via a non-zero exit code.
            row(
                "validate",
                4,
                "Repository-integrity, scope, or drift validation found \
                 error-severity findings.",
                true,
            ),
            row(
                "validate",
                1,
                "Rule evaluation reported error-severity findings.",
                true,
            ),
            row(
                "validate --branch-drift",
                1,
                "Branch-drift validation failed: the check reported drift from \
                 the upstream branch or could not run.",
                true,
            ),
            row(
                "validate --leases",
                1,
                "Lease validation found one or more invalid leases, or the lease \
                 check could not run.",
                true,
            ),
            row(
                "gate status-all",
                4,
                "One or more required gates have not passed.",
                true,
            ),
            row(
                "invariant check",
                4,
                "Enforcement drift was found (declared enforcement not backed by \
                 an enforcing rule).",
                true,
            ),
            row(
                "config validate",
                1,
                "The repo, user, or environment-variable configuration failed to \
                 load or carried an invalid value.",
                true,
            ),
            row(
                "config validate",
                2,
                "Reserved: the handler has an exit(2) branch for configuration \
                 warnings, but no warning condition is defined today, so 2 is \
                 never emitted.",
                true,
            ),
            row(
                "doc check-links",
                1,
                "One or more documents have broken links.",
                true,
            ),
            row(
                "doc check-links",
                2,
                "Documents have only risky-link warnings; here 2 means warnings, \
                 not a usage error.",
                true,
            ),
            row(
                "gate preset apply",
                1,
                "One or more issues failed to apply the preset (partial batch).",
                true,
            ),
            // `serve` default (daemon), `--stop`, and `--status` follow the
            // standard taxonomy: 0 on success, 1 on a start/stop/status error.
            row(
                "serve, serve --stop, serve --status",
                1,
                "The daemon start, stop, or status operation failed (exits 0 on \
                 success).",
                false,
            ),
            // `serve --fg` passes the inline server child's own code through.
            CommandExitCode {
                command: "serve --fg".to_string(),
                code: None,
                condition: "Foreground mode passes through the inline dev-server \
                            child's own exit code (1 when the child produced none)."
                    .to_string(),
                exception: true,
            },
        ]
    }
}

/// Render the exit-code reference page (`docs/reference/exit-codes.md`).
///
/// The page projects the global taxonomy ([`CommandSchema::exit_codes`]) and the
/// per-command mappings ([`CommandSchema::command_exit_codes`]) into markdown, so
/// the committed doc is generated rather than hand-copied. A test asserts the
/// committed file equals this output (@/inv/single-source-prose).
pub fn render_exit_code_reference() -> String {
    let mut out = String::new();
    out.push_str("# Exit Codes\n\n");
    out.push_str(
        "<!-- GENERATED by `jit::schema::render_exit_code_reference` \
         (`crates/jit/src/schema.rs`). Do not edit by hand; regenerate instead. -->\n\n",
    );
    out.push_str(
        "`jit` returns a small, stable set of process exit codes so scripts and \
         agents can branch on outcomes without parsing output. This page is \
         projected from the exit-code taxonomy and the command mappings in \
         `jit --schema`. Each emitted mapping is bound by a test to the runtime \
         that produces it: the shared classifier (`error_to_exit_code` in \
         `crates/jit/src/main.rs`) for codes raised as typed errors, and the \
         command's own `std::process::exit` site for codes a completed run emits \
         directly (the findings signals and the `serve --fg` pass-through). One \
         row is documented as *reserved* rather than bound: `config validate` `2` \
         names a branch the handler carries but no condition reaches, so nothing \
         emits it.\n\n",
    );

    out.push_str("## Global taxonomy\n\n");
    out.push_str("| Code | Meaning |\n|------|---------|\n");
    for doc in CommandSchema::generate_exit_codes() {
        out.push_str(&format!("| `{}` | {} |\n", doc.code, doc.description));
    }
    out.push('\n');

    out.push_str("## Command-specific mappings\n\n");
    out.push_str(
        "Most commands draw only from the global taxonomy above. The rows below \
         identify the codes a specific command family emits. An **exception** is \
         a code a completed run emits to signal findings, or a code whose meaning \
         departs from the global entry (for example, `doc check-links` exits `2` \
         for warnings, not a usage error). A `child` code marks a pass-through, \
         where the command exits with a subprocess's own code. `*` marks a code \
         every command can reach: `0` on successful completion, and the failure \
         codes through the shared classifier.\n\n",
    );
    out.push_str("| Command | Code | Condition | Exception |\n");
    out.push_str("|---------|------|-----------|-----------|\n");
    for entry in CommandSchema::generate_command_exit_codes() {
        let code = match entry.code {
            Some(c) => format!("`{c}`"),
            None => "`child`".to_string(),
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            entry.command,
            code,
            entry.condition,
            if entry.exception { "yes" } else { "" }
        ));
    }
    out.push('\n');

    out.push_str(
        "For the full `jit gate evaluate` verdict taxonomy and the `--json` \
         `verdict` field, see [the gate command reference](cli-commands.md).\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hidden_field_set_for_known_commands() {
        let schema = CommandSchema::generate();

        // Known hidden commands should have hidden=true on their leaf
        let init = schema.commands.get("init");
        assert!(init.is_some(), "init command should exist");
        assert!(init.unwrap().hidden, "init should be hidden");

        // issue_assign should be hidden (leaf under issue)
        let issue = schema.commands.get("issue").expect("issue command");
        let assign = issue.subcommands.as_ref().and_then(|s| s.get("assign"));
        assert!(assign.is_some(), "issue assign should exist");
        assert!(assign.unwrap().hidden, "issue assign should be hidden");

        // Non-hidden commands should not have hidden=true
        let status = schema.commands.get("status");
        assert!(status.is_some(), "status command should exist");
        assert!(!status.unwrap().hidden, "status should not be hidden");

        let issue_create = issue.subcommands.as_ref().and_then(|s| s.get("create"));
        assert!(issue_create.is_some(), "issue create should exist");
        assert!(
            !issue_create.unwrap().hidden,
            "issue create should not be hidden"
        );
    }

    #[test]
    fn test_hidden_propagates_to_children() {
        let schema = CommandSchema::generate();

        // gate_preset is hidden, so its children should inherit hidden
        let gate = schema.commands.get("gate").expect("gate command");
        if let Some(preset) = gate.subcommands.as_ref().and_then(|s| s.get("preset")) {
            if let Some(children) = &preset.subcommands {
                for (name, child) in children {
                    assert!(
                        child.hidden,
                        "gate preset {name} should be hidden (inherited from parent)"
                    );
                }
            }
        }
    }

    #[test]
    fn test_hidden_not_serialized_when_false() {
        let cmd = Command {
            description: "test".to_string(),
            hidden: false,
            aliases: vec![],
            subcommands: None,
            args: vec![],
            flags: vec![],
            output: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            !json.contains("hidden"),
            "hidden:false should be skipped in serialization"
        );
    }

    #[test]
    fn test_hidden_serialized_when_true() {
        let cmd = Command {
            description: "test".to_string(),
            hidden: true,
            aliases: vec![],
            subcommands: None,
            args: vec![],
            flags: vec![],
            output: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            json.contains("\"hidden\":true"),
            "hidden:true should be present in serialization"
        );
    }

    /// `graph tree` publishes its response shape through the schema command, so
    /// MCP clients and other consumers read the resolved-hierarchy contract
    /// instead of re-deriving it.
    #[test]
    fn test_graph_tree_schema_is_published() {
        let schema = CommandSchema::generate();

        let tree = schema
            .commands
            .get("graph")
            .and_then(|g| g.subcommands.as_ref())
            .and_then(|s| s.get("tree"))
            .expect("graph tree subcommand should exist");

        let output = tree
            .output
            .as_ref()
            .expect("graph tree should have an output schema");
        assert_eq!(output.success, "GraphTreeResponse");

        let schema_val = output
            .success_schema
            .as_ref()
            .expect("success_schema should be present");
        let props = schema_val
            .pointer("/definitions/GraphTreeResponse/properties")
            .or_else(|| schema_val.pointer("/properties"))
            .expect("GraphTreeResponse properties should be present in schema");
        assert!(props.get("nodes").is_some(), "list envelope collection");
        assert!(props.get("count").is_some(), "list envelope count");
        assert!(props.get("root").is_some(), "scoping root id");

        // The node shape carries the four resolved-hierarchy fields.
        let node_schema = serde_json::to_string(
            schema_val
                .pointer("/definitions/HierarchyNodeView")
                .expect("HierarchyNodeView definition"),
        )
        .unwrap();
        for field in ["parent", "children", "cluster", "rank"] {
            assert!(
                node_schema.contains(&format!("\"{field}\"")),
                "node schema should describe the {field} field"
            );
        }
    }

    /// REQ-01: the schema's `State` enum must list every real `State` variant,
    /// derived from the authoritative `State::all()` enumeration. A hand-copied
    /// list that drops `rejected` (or any future variant) must fail this test.
    #[test]
    fn test_schema_state_enum_covers_every_state() {
        use crate::domain::State;

        let schema = CommandSchema::generate();
        let state_type = schema.types.get("State").expect("State type in schema");
        let enum_vals: Vec<String> = state_type
            .get("enum")
            .and_then(|v| v.as_array())
            .expect("State type has an enum array")
            .iter()
            .map(|v| v.as_str().expect("enum entries are strings").to_string())
            .collect();

        // `rejected` is the state historically dropped from the hand-maintained list.
        assert!(
            enum_vals.iter().any(|s| s == "rejected"),
            "schema State enum must include `rejected`; found {enum_vals:?}"
        );

        // Guard against drift in either direction: the schema enum must equal
        // the authoritative `State::all()` enumeration exactly.
        let expected: Vec<String> = State::all()
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_eq!(
            enum_vals, expected,
            "schema State enum must match State::all() exactly (single source of truth)"
        );
    }

    #[test]
    fn test_schema_publishes_every_gate_checker_wire_variant() {
        let schema = CommandSchema::generate();
        let checker = schema
            .types
            .get("GateChecker")
            .expect("GateChecker type in schema");
        let encoded = serde_json::to_string(checker).unwrap();

        for checker_type in [
            "exec",
            "repository_validation",
            "issue_validation",
            "label_target_validation",
            "review_placeholder",
        ] {
            assert!(
                encoded.contains(checker_type),
                "GateChecker schema omits {checker_type}: {encoded}"
            );
        }
        assert!(encoded.contains("label_namespace"));
    }

    #[test]
    fn test_schema_publishes_profile_manifest_runtime_contract() {
        let schema = CommandSchema::generate();
        let profile = schema
            .types
            .get("ProfileManifest")
            .expect("ProfileManifest schema");
        let text = profile.to_string();
        assert!(text.contains("manifest-version"));
        assert!(text.contains("map-entry"));
        assert!(text.contains("singleton-table"));
        assert!(text.contains("rules-gates-projection"));
    }

    /// REQ-02/REQ-03: every global flag accepted at the top level — `quiet`,
    /// `schema`, `help`, `version` — must be documented in `global_options`.
    /// Dropping any one fails this test.
    #[test]
    fn test_schema_exposes_global_flags() {
        let schema = CommandSchema::generate();
        let names: HashSet<&str> = schema
            .global_options
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        for expected in ["quiet", "schema", "help", "version"] {
            assert!(
                names.contains(expected),
                "global flag `{expected}` must appear in schema global_options; found {names:?}"
            );
        }
    }

    /// REQ-02/REQ-03: a command flag's visible aliases (alternate accepted
    /// spellings) must be exposed alongside its primary name. Dropping either
    /// the primary flag or an alias fails this test.
    #[test]
    fn test_schema_exposes_visible_flag_aliases() {
        let schema = CommandSchema::generate();

        // `issue update --label` accepts the visible alias `--add-label`.
        let update_label = schema
            .commands
            .get("issue")
            .and_then(|c| c.subcommands.as_ref())
            .and_then(|s| s.get("update"))
            .expect("issue update command")
            .flags
            .iter()
            .find(|f| f.name == "label")
            .expect("issue update has a --label flag");
        assert!(
            update_label.aliases.iter().any(|a| a == "add-label"),
            "issue update --label must expose its `add-label` alias; found {:?}",
            update_label.aliases
        );

        // `doc add --label` accepts the visible alias `--title`.
        let doc_add_label = schema
            .commands
            .get("doc")
            .and_then(|c| c.subcommands.as_ref())
            .and_then(|s| s.get("add"))
            .expect("doc add command")
            .flags
            .iter()
            .find(|f| f.name == "label")
            .expect("doc add has a --label flag");
        assert!(
            doc_add_label.aliases.iter().any(|a| a == "title"),
            "doc add --label must expose its `title` alias; found {:?}",
            doc_add_label.aliases
        );
    }

    /// REQ-02/REQ-03: command and subcommand visible aliases (alternate
    /// accepted invocations) must be exposed in the schema. Dropping a command
    /// alias fails this test.
    #[test]
    fn test_schema_exposes_command_visible_aliases() {
        let schema = CommandSchema::generate();

        // The top-level `dep` command accepts the visible alias `dependency`.
        let dep = schema.commands.get("dep").expect("dep command");
        assert!(
            dep.aliases.iter().any(|a| a == "dependency"),
            "dep command must expose its `dependency` alias; found {:?}",
            dep.aliases
        );

        // The `gate evaluate` subcommand accepts the visible aliases `pass`
        // and `eval`.
        let evaluate = schema
            .commands
            .get("gate")
            .and_then(|c| c.subcommands.as_ref())
            .and_then(|s| s.get("evaluate"))
            .expect("gate evaluate subcommand");
        for alias in ["pass", "eval"] {
            assert!(
                evaluate.aliases.iter().any(|a| a == alias),
                "gate evaluate must expose its `{alias}` alias; found {:?}",
                evaluate.aliases
            );
        }
    }

    #[test]
    fn test_graph_deps_schema_matches_tree_response() {
        let schema = CommandSchema::generate();

        let graph = schema
            .commands
            .get("graph")
            .expect("graph command should exist");
        let deps = graph
            .subcommands
            .as_ref()
            .and_then(|s| s.get("deps"))
            .expect("graph deps subcommand should exist");

        let output = deps
            .output
            .as_ref()
            .expect("graph deps should have output schema");
        assert_eq!(
            output.success,
            "GraphDepsTreeResponse",
            "graph deps schema should reference GraphDepsTreeResponse (the actual emitted type), not GraphDepsResponse"
        );

        // Verify the schema contains the tree-structure properties actually emitted
        let schema_val = output
            .success_schema
            .as_ref()
            .expect("success_schema should be present");
        let props = schema_val
            .pointer("/definitions/GraphDepsTreeResponse/properties")
            .or_else(|| schema_val.pointer("/properties"))
            .expect("GraphDepsTreeResponse properties should be present in schema");

        assert!(
            props.get("nodes").is_some(),
            "schema should contain 'nodes' property (the node collection, renamed from 'tree')"
        );
        assert!(
            props.get("count").is_some(),
            "schema should contain 'count' property (list envelope)"
        );
        assert!(
            props.get("tree").is_none(),
            "schema must not contain the old 'tree' property (renamed to 'nodes')"
        );
        assert!(
            props.get("summary").is_some(),
            "schema should contain 'summary' property (from GraphDepsTreeResponse)"
        );
        assert!(
            props.get("dependencies").is_none(),
            "schema must not contain 'dependencies' property (that belongs to the removed GraphDepsResponse)"
        );
    }

    #[test]
    fn test_command_exit_codes_present_in_schema() {
        let schema = CommandSchema::generate();
        assert!(
            !schema.command_exit_codes.is_empty(),
            "command_exit_codes projection must be populated"
        );
        // A few representative families must surface so the projection stays
        // discoverable.
        for command in ["*", "gate evaluate, gate evaluate-all", "validate"] {
            assert!(
                schema
                    .command_exit_codes
                    .iter()
                    .any(|c| c.command == command),
                "command_exit_codes must document `{command}`"
            );
        }
    }

    #[test]
    fn test_command_exit_codes_within_global_taxonomy() {
        // Every projected fixed code must be a defined member of the global
        // taxonomy, so the per-command surface can never introduce a code the
        // taxonomy does not explain. Pass-through rows (`code: None`, e.g.
        // `serve`) carry no jit-assigned code and are exempt.
        let taxonomy: std::collections::HashSet<i32> = CommandSchema::generate_exit_codes()
            .iter()
            .map(|d| d.code)
            .collect();
        for entry in CommandSchema::generate_command_exit_codes() {
            if let Some(code) = entry.code {
                assert!(
                    taxonomy.contains(&code),
                    "command `{}` documents code {code}, which is absent from the global taxonomy",
                    entry.command,
                );
            } else {
                assert!(
                    entry.exception,
                    "pass-through row for `{}` (code: None) must be an exception",
                    entry.command,
                );
            }
        }
    }

    /// The committed reference page is a projection: it must equal the rendered
    /// output exactly. Regenerate with `UPDATE_EXIT_CODE_DOC=1` when the
    /// projection changes.
    #[test]
    fn test_exit_code_reference_doc_is_current() {
        let rendered = render_exit_code_reference();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/reference/exit-codes.md"
        );
        if std::env::var_os("UPDATE_EXIT_CODE_DOC").is_some() {
            std::fs::write(path, &rendered).expect("write exit-codes.md");
            return;
        }
        let committed = std::fs::read_to_string(path).expect("read docs/reference/exit-codes.md");
        assert_eq!(
            committed, rendered,
            "docs/reference/exit-codes.md is stale; regenerate with \
             UPDATE_EXIT_CODE_DOC=1 cargo test test_exit_code_reference_doc_is_current"
        );
    }

    /// Fetch the published success schema for a command addressed by its path
    /// segments (`["issue", "show"]`, or `["list"]` for a top-level command).
    fn published_output_schema(path: &[&str]) -> Value {
        let schema = CommandSchema::generate();
        let (last, parents) = path.split_last().expect("non-empty command path");
        let mut commands = &schema.commands;
        for seg in parents {
            commands = commands
                .get(*seg)
                .and_then(|c| c.subcommands.as_ref())
                .unwrap_or_else(|| panic!("{} should have subcommands", path.join(" ")));
        }
        let cmd = commands
            .get(*last)
            .unwrap_or_else(|| panic!("{} command should exist", path.join(" ")));
        let output = cmd
            .output
            .as_ref()
            .unwrap_or_else(|| panic!("{} should have an output schema", path.join(" ")));
        output
            .success_schema
            .clone()
            .unwrap_or_else(|| panic!("{} success_schema should be present", path.join(" ")))
    }

    /// Recursively union the keys of every `properties` object anywhere in a JSON
    /// Schema value, so the check is robust against `oneOf` arms, `$defs`, and
    /// flattened sub-schemas — and against field names that merely appear in a
    /// `description` string.
    fn declared_property_names(schema: &Value) -> std::collections::HashSet<String> {
        let mut names = std::collections::HashSet::new();
        fn walk(value: &Value, names: &mut std::collections::HashSet<String>) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::Object(props)) = map.get("properties") {
                        names.extend(props.keys().cloned());
                    }
                    for child in map.values() {
                        walk(child, names);
                    }
                }
                Value::Array(items) => items.iter().for_each(|i| walk(i, names)),
                _ => {}
            }
        }
        walk(schema, &mut names);
        names
    }

    /// REQ-01/REQ-02/REQ-04: every projected issue view (`issue show`, its
    /// `--summary`, and `issue status`) declares the gate list under the unified
    /// `gates` field and never the raw storage names. A rename of the response
    /// struct's field changes the derived schema and fails this test.
    #[test]
    fn test_schema_projected_issue_views_declare_unified_gates_field() {
        // `issue create` returns the `issue show` projection, so it joins the
        // projected views that expose `gates` and never the storage split.
        for path in [["issue", "show"], ["issue", "status"], ["issue", "create"]] {
            let label = path.join(" ");
            let props = declared_property_names(&published_output_schema(&path));
            assert!(
                props.contains("gates"),
                "{label} schema must declare the `gates` property; got: {props:?}"
            );
            assert!(
                !props.contains("gates_required") && !props.contains("gates_status"),
                "{label} is a projected view and must not declare the storage \
                 gate properties; got: {props:?}"
            );
        }
    }

    /// REQ-02/REQ-04: EVERY surface that hands back a stored issue record
    /// declares the storage gate fields. Two families: the `--full` list dumps
    /// (query family, graph export, `issue list` + top-level `list`, `issue
    /// search`) and the single-issue record echoes (`issue assign`, `unassign`,
    /// `reject`, `release`, `claim`, `claim-next`, and `apply`'s created-issues
    /// map). Deriving the arms from the `Issue` struct keeps the declared names
    /// in lockstep with the serialized record.
    #[test]
    fn test_schema_record_dumps_declare_storage_gate_fields() {
        let surfaces: &[&[&str]] = &[
            &["query"],
            &["query", "all"],
            &["graph", "export"],
            &["issue", "list"],
            &["list"],
            &["issue", "search"],
            &["issue", "assign"],
            &["issue", "unassign"],
            &["issue", "reject"],
            &["issue", "release"],
            &["issue", "claim"],
            &["issue", "claim-next"],
            &["apply"],
        ];
        for path in surfaces {
            let label = path.join(" ");
            let props = declared_property_names(&published_output_schema(path));
            assert!(
                props.contains("gates_required") && props.contains("gates_status"),
                "{label} record dump must declare the storage gate properties; \
                 got: {props:?}"
            );
        }
    }

    /// REQ-03/REQ-04: the storage reference's gate-field rules and this
    /// schema must not drift apart. The test reads the actual section text
    /// from docs/reference/storage-format.md and cross-checks every member
    /// spelling it documents against the live schema declaration for that
    /// command, so renaming a field or dropping a surface on EITHER side
    /// fails the build instead of silently de-synchronizing prose and code.
    #[test]
    fn test_schema_matches_storage_reference_gate_field_rules() {
        let doc_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/reference/storage-format.md");
        let doc = std::fs::read_to_string(&doc_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", doc_path.display()));
        let section_start = doc
            .find("#### Gate fields in command output")
            .expect("storage-format.md must keep the gate-fields section");
        let section_end = doc[section_start + 4..]
            .find("\n#")
            .map(|i| section_start + 4 + i)
            .unwrap_or(doc.len());
        // Collapse whitespace so hard-wrapped member spellings match.
        let section = doc[section_start..section_end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        for field in ["`gates_required`", "`gates_status`", "`gates`"] {
            assert!(
                section.contains(field),
                "the gate-fields section must name {field}"
            );
        }

        // (documented member spelling, schema command path, family)
        enum Family {
            RecordDump,
            Projected,
        }
        let members: &[(&str, &[&str], Family)] = &[
            (
                "`jit graph export --format json --full`",
                &["graph", "export"],
                Family::RecordDump,
            ),
            ("`jit query all`", &["query", "all"], Family::RecordDump),
            ("`available`", &["query", "available"], Family::RecordDump),
            ("`strategic`", &["query", "strategic"], Family::RecordDump),
            ("`closed`", &["query", "closed"], Family::RecordDump),
            ("bare `jit query --full`", &["query"], Family::RecordDump),
            (
                "`jit issue list --full`",
                &["issue", "list"],
                Family::RecordDump,
            ),
            ("`jit list --full`", &["list"], Family::RecordDump),
            (
                "`jit issue search --full`",
                &["issue", "search"],
                Family::RecordDump,
            ),
            (
                "`jit issue assign`",
                &["issue", "assign"],
                Family::RecordDump,
            ),
            ("`unassign`", &["issue", "unassign"], Family::RecordDump),
            ("`reject`", &["issue", "reject"], Family::RecordDump),
            ("`release`", &["issue", "release"], Family::RecordDump),
            ("`claim`", &["issue", "claim"], Family::RecordDump),
            ("`claim-next`", &["issue", "claim-next"], Family::RecordDump),
            ("`jit apply`", &["apply"], Family::RecordDump),
            (
                "`jit issue create`",
                &["issue", "create"],
                Family::Projected,
            ),
            ("`jit issue show`", &["issue", "show"], Family::Projected),
            (
                "`jit issue status`",
                &["issue", "status"],
                Family::Projected,
            ),
            (
                "`jit issue children`",
                &["issue", "children"],
                Family::Projected,
            ),
        ];
        for (spelling, path, family) in members {
            assert!(
                section.contains(spelling),
                "the gate-fields section must list the member {spelling}"
            );
            let props = declared_property_names(&published_output_schema(path));
            match family {
                Family::RecordDump => assert!(
                    props.contains("gates_required") && props.contains("gates_status"),
                    "{spelling}: documented as a record dump but the schema for \
                     {path:?} does not declare the storage gate fields; got {props:?}"
                ),
                Family::Projected => assert!(
                    props.contains("gates") && !props.contains("gates_required"),
                    "{spelling}: documented as a projected view but the schema \
                     for {path:?} does not declare the gates array (or leaks \
                     the storage names); got {props:?}"
                ),
            }
        }
    }
}
