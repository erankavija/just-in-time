//! Command schema export for AI agent introspection.
//!
//! This module provides JSON schema generation from CLI definitions,
//! enabling AI agents to discover available commands, arguments, and types.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Complete command schema for the JIT CLI
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandSchema {
    /// CLI version
    pub version: String,
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
    /// Description
    pub description: String,
}

/// Output schema
#[derive(Debug, Serialize, Deserialize)]
pub struct OutputSchema {
    /// Success output type reference
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
    /// Generate the complete command schema
    pub fn generate() -> Self {
        let mut commands = HashMap::new();

        // Init
        commands.insert(
            "init".to_string(),
            Command {
                description: "Initialize the issue tracker in the current directory".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        // Issue commands
        commands.insert("issue".to_string(), Self::generate_issue_commands());

        // Dep commands
        commands.insert("dep".to_string(), Self::generate_dep_commands());

        // Gate commands
        commands.insert("gate".to_string(), Self::generate_gate_commands());

        // Registry commands
        commands.insert("registry".to_string(), Self::generate_registry_commands());

        // Events commands
        commands.insert("events".to_string(), Self::generate_events_commands());

        // Document commands
        commands.insert("doc".to_string(), Self::generate_doc_commands());

        // Graph commands
        commands.insert("graph".to_string(), Self::generate_graph_commands());

        // Query commands
        commands.insert("query".to_string(), Self::generate_query_commands());

        // Status
        commands.insert(
            "status".to_string(),
            Command {
                description: "Show overall repository status".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "StatusSummary".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        // Search
        commands.insert(
            "search".to_string(),
            Command {
                description: "Search issues and documents using ripgrep".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "query".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Search query string".to_string()),
                }],
                flags: vec![
                    Flag {
                        name: "regex".to_string(),
                        flag_type: "boolean".to_string(),
                        description: "Use regex pattern matching".to_string(),
                    },
                    Flag {
                        name: "case-sensitive".to_string(),
                        flag_type: "boolean".to_string(),
                        description: "Case sensitive search".to_string(),
                    },
                    Flag {
                        name: "context".to_string(),
                        flag_type: "number".to_string(),
                        description: "Show N lines of context".to_string(),
                    },
                    Flag {
                        name: "limit".to_string(),
                        flag_type: "number".to_string(),
                        description: "Maximum results to return".to_string(),
                    },
                    Flag {
                        name: "glob".to_string(),
                        flag_type: "string".to_string(),
                        description: "Search only matching files (e.g., '*.json' or '*.md')"
                            .to_string(),
                    },
                    Flag {
                        name: "json".to_string(),
                        flag_type: "boolean".to_string(),
                        description: "Output JSON format".to_string(),
                    },
                ],
                output: Some(OutputSchema {
                    success: "SearchResults".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        // Validate
        commands.insert(
            "validate".to_string(),
            Command {
                description: "Validate repository integrity".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        CommandSchema {
            version: "0.2.0".to_string(),
            commands,
            types: Self::generate_types(),
            exit_codes: Self::generate_exit_codes(),
        }
    }

    fn generate_issue_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "create".to_string(),
            Command {
                description: "Create a new issue".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "title".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue title".to_string()),
                    },
                    Argument {
                        name: "desc".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: Some("".to_string()),
                        description: Some("Issue description".to_string()),
                    },
                    Argument {
                        name: "priority".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: Some("normal".to_string()),
                        description: Some("Priority: low, normal, high, critical".to_string()),
                    },
                    Argument {
                        name: "gate".to_string(),
                        arg_type: "array[string]".to_string(),
                        required: false,
                        default: None,
                        description: Some("Required gate keys".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Issue".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "list".to_string(),
            Command {
                description: "List issues with optional filters".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "state".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by state".to_string()),
                    },
                    Argument {
                        name: "assignee".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by assignee".to_string()),
                    },
                    Argument {
                        name: "priority".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by priority".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "show".to_string(),
            Command {
                description: "Show issue details".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Issue ID".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Issue".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "search".to_string(),
            Command {
                description: "Search issues by text query".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "query".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some(
                            "Search query (searches title, description, ID)".to_string(),
                        ),
                    },
                    Argument {
                        name: "state".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by state".to_string()),
                    },
                    Argument {
                        name: "assignee".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by assignee".to_string()),
                    },
                    Argument {
                        name: "priority".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("Filter by priority".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "update".to_string(),
            Command {
                description: "Update an issue".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "title".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("New title".to_string()),
                    },
                    Argument {
                        name: "desc".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("New description".to_string()),
                    },
                    Argument {
                        name: "priority".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("New priority".to_string()),
                    },
                    Argument {
                        name: "state".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: None,
                        description: Some("New state".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Issue".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "delete".to_string(),
            Command {
                description: "Delete an issue".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Issue ID".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "breakdown".to_string(),
            Command {
                description:
                    "Break down an issue into subtasks with automatic dependency inheritance"
                        .to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "parent_id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Parent issue ID to break down".to_string()),
                }],
                flags: vec![
                    Flag {
                        name: "subtask".to_string(),
                        flag_type: "array<string>".to_string(),
                        description: "Subtask titles (use multiple times)".to_string(),
                    },
                    Flag {
                        name: "desc".to_string(),
                        flag_type: "array<string>".to_string(),
                        description: "Subtask descriptions (optional)".to_string(),
                    },
                    Flag {
                        name: "json".to_string(),
                        flag_type: "boolean".to_string(),
                        description: "Output JSON format".to_string(),
                    },
                ],
                output: Some(OutputSchema {
                    success: "BreakdownResult".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "claim".to_string(),
            Command {
                description: "Claim an issue for an assignee".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "assignee".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Assignee identifier (type:id format)".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Issue".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "unclaim".to_string(),
            Command {
                description: "Unclaim an issue".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Issue ID".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Issue".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Issue management commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_dep_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "add".to_string(),
            Command {
                description: "Add a dependency between issues".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "from".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue that depends on another".to_string()),
                    },
                    Argument {
                        name: "to".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue that must be completed first".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "rm".to_string(),
            Command {
                description: "Remove a dependency".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "from".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue with the dependency".to_string()),
                    },
                    Argument {
                        name: "to".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Dependency to remove".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Dependency management commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_gate_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "add".to_string(),
            Command {
                description: "Add a gate to an issue".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "gate".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Gate key".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "pass".to_string(),
            Command {
                description: "Mark a gate as passed".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "gate".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Gate key".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "fail".to_string(),
            Command {
                description: "Mark a gate as failed".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "gate".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Gate key".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Gate management commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_registry_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "add".to_string(),
            Command {
                description: "Register a new gate definition".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "key".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Gate key".to_string()),
                    },
                    Argument {
                        name: "name".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Gate name".to_string()),
                    },
                    Argument {
                        name: "description".to_string(),
                        arg_type: "string".to_string(),
                        required: false,
                        default: Some("".to_string()),
                        description: Some("Gate description".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "void".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "list".to_string(),
            Command {
                description: "List all registered gates".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "GateList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "show".to_string(),
            Command {
                description: "Show gate definition".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "key".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Gate key".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "GateDefinition".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Gate registry management commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_events_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "tail".to_string(),
            Command {
                description: "Show recent events".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "n".to_string(),
                    arg_type: "number".to_string(),
                    required: false,
                    default: Some("10".to_string()),
                    description: Some("Number of events to show".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "EventList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Event log commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_doc_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "add".to_string(),
            Command {
                description: "Add a document reference to an issue".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "path".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some(
                            "Path to document relative to repository root".to_string(),
                        ),
                    },
                ],
                flags: vec![
                    Flag {
                        name: "commit".to_string(),
                        flag_type: "string".to_string(),
                        description: "Git commit hash (optional, defaults to HEAD)".to_string(),
                    },
                    Flag {
                        name: "label".to_string(),
                        flag_type: "string".to_string(),
                        description: "Human-readable label".to_string(),
                    },
                    Flag {
                        name: "doc-type".to_string(),
                        flag_type: "string".to_string(),
                        description: "Document type (e.g., design, implementation, notes)"
                            .to_string(),
                    },
                    Flag {
                        name: "json".to_string(),
                        flag_type: "boolean".to_string(),
                        description: "Output JSON format".to_string(),
                    },
                ],
                output: Some(OutputSchema {
                    success: "DocumentAdded".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "list".to_string(),
            Command {
                description: "List document references for an issue".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Issue ID".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "DocumentList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "remove".to_string(),
            Command {
                description: "Remove a document reference from an issue".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "path".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Path to document to remove".to_string()),
                    },
                ],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "DocumentRemoved".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "show".to_string(),
            Command {
                description: "Show document content".to_string(),
                subcommands: None,
                args: vec![
                    Argument {
                        name: "id".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Issue ID".to_string()),
                    },
                    Argument {
                        name: "path".to_string(),
                        arg_type: "string".to_string(),
                        required: true,
                        default: None,
                        description: Some("Path to document".to_string()),
                    },
                ],
                flags: vec![],
                output: Some(OutputSchema {
                    success: "DocumentContent".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Document reference commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_graph_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "show".to_string(),
            Command {
                description: "Show dependency graph".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: false,
                    default: None,
                    description: Some("Issue ID to show subgraph for".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "Graph".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "roots".to_string(),
            Command {
                description: "Show root issues (no dependencies)".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "downstream".to_string(),
            Command {
                description: "Show downstream issues (dependents)".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "id".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Issue ID".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "export".to_string(),
            Command {
                description: "Export graph in various formats".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "format".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Export format: dot, mermaid".to_string()),
                }],
                flags: vec![],
                output: Some(OutputSchema {
                    success: "string".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Graph query commands".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_query_commands() -> Command {
        let mut subcommands = HashMap::new();

        subcommands.insert(
            "ready".to_string(),
            Command {
                description: "Query ready issues".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "blocked".to_string(),
            Command {
                description: "Query blocked issues".to_string(),
                subcommands: None,
                args: vec![],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "assignee".to_string(),
            Command {
                description: "Query issues by assignee".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "assignee".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Assignee identifier".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "state".to_string(),
            Command {
                description: "Query issues by state".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "state".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("State to query".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        subcommands.insert(
            "priority".to_string(),
            Command {
                description: "Query issues by priority".to_string(),
                subcommands: None,
                args: vec![Argument {
                    name: "priority".to_string(),
                    arg_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: Some("Priority to query".to_string()),
                }],
                flags: vec![Flag {
                    name: "json".to_string(),
                    flag_type: "boolean".to_string(),
                    description: "Output JSON format".to_string(),
                }],
                output: Some(OutputSchema {
                    success: "IssueList".to_string(),
                    error: "ErrorResponse".to_string(),
                }),
            },
        );

        Command {
            description: "Query issues for orchestrators".to_string(),
            subcommands: Some(subcommands),
            args: vec![],
            flags: vec![],
            output: None,
        }
    }

    fn generate_types() -> HashMap<String, Value> {
        let mut types = HashMap::new();

        types.insert(
            "State".to_string(),
            json!({
                "type": "enum",
                "enum": ["open", "ready", "in_progress", "done", "archived"],
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
            "GateStatus".to_string(),
            json!({
                "type": "enum",
                "enum": ["pending", "passed", "failed"],
                "description": "Quality gate status"
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
                    "context": { "type": "object" },
                    "created_at": { "type": "string", "format": "date-time" },
                    "updated_at": { "type": "string", "format": "date-time" }
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
                            "details": { "type": "object" },
                            "suggestions": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "metadata": { "$ref": "#/types/Metadata" }
                }
            }),
        );

        types.insert(
            "Metadata".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "timestamp": { "type": "string", "format": "date-time" },
                    "version": { "type": "string" }
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
