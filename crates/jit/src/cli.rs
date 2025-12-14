//! Command-line interface definitions using clap.

use clap::{Parser, Subcommand};

/// Just-In-Time Issue Tracker
///
/// A repository-local CLI issue tracker with dependency graph enforcement and quality gating.
/// Designed for deterministic, machine-friendly outputs and process automation.
///
/// Exit Codes:
///   0  - Command succeeded
///   1  - Generic error occurred
///   2  - Invalid arguments or usage error
///   3  - Resource not found (issue, gate, etc.)
///   4  - Validation failed (cycle detected, broken references, etc.)
///   5  - Permission denied
///   6  - Resource already exists
///  10  - External dependency failed (git, file system, etc.)
#[derive(Parser)]
#[command(name = "jit")]
#[command(about = "Just-In-Time issue tracker", long_about = None)]
pub struct Cli {
    /// Export command schema in JSON format for AI agent introspection
    #[arg(long)]
    pub schema: bool,

    /// Export command schema using automatic generation (for testing)
    #[arg(long)]
    pub schema_auto: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize the issue tracker in the current directory
    Init {
        /// Hierarchy template to use (default, extended, agile, minimal)
        #[arg(long)]
        hierarchy_template: Option<String>,
    },

    /// Issue management commands
    #[command(subcommand)]
    Issue(IssueCommands),

    /// Dependency management commands
    #[command(subcommand)]
    Dep(DepCommands),

    /// Gate management commands
    #[command(subcommand)]
    Gate(GateCommands),

    /// Gate registry management commands
    #[command(subcommand)]
    Registry(RegistryCommands),

    /// Event log commands
    #[command(subcommand)]
    Events(EventCommands),

    /// Document reference commands
    #[command(subcommand)]
    Doc(DocCommands),

    /// Graph query commands
    #[command(subcommand)]
    Graph(GraphCommands),

    /// Query issues for orchestrators
    #[command(subcommand)]
    Query(QueryCommands),

    /// Label namespace management commands
    #[command(subcommand)]
    Label(LabelCommands),

    /// Configuration commands
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Search issues and documents
    Search {
        /// Search query string
        query: String,

        /// Use regex pattern matching
        #[arg(short, long)]
        regex: bool,

        /// Case sensitive search
        #[arg(short = 'C', long)]
        case_sensitive: bool,

        /// Show N lines of context
        #[arg(short = 'c', long, default_value = "0")]
        context: usize,

        /// Maximum results to return
        #[arg(short = 'n', long)]
        limit: Option<usize>,

        /// Search only in specific files (glob pattern, e.g., "*.json" or "*.md")
        #[arg(short = 'g', long)]
        glob: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show overall status
    Status {
        #[arg(long)]
        json: bool,
    },

    /// Validate repository integrity
    Validate {
        #[arg(long)]
        json: bool,

        /// Attempt to automatically fix validation issues
        #[arg(long)]
        fix: bool,

        /// Show what would be fixed without applying changes (requires --fix)
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum IssueCommands {
    /// Create a new issue
    Create {
        #[arg(short, long)]
        title: String,

        #[arg(short, long, default_value = "")]
        desc: String,

        #[arg(short, long, default_value = "normal")]
        priority: String,

        /// Gate keys (comma-separated or multiple --gate flags)
        #[arg(short, long, value_delimiter = ',')]
        gate: Vec<String>,

        /// Labels (format: namespace:value, repeatable)
        #[arg(short, long)]
        label: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// List issues
    List {
        #[arg(short, long)]
        state: Option<String>,

        #[arg(short, long)]
        assignee: Option<String>,

        #[arg(short, long)]
        priority: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Search issues by text query
    Search {
        /// Search query (searches title, description, and ID)
        query: String,

        #[arg(short, long)]
        state: Option<String>,

        #[arg(short, long)]
        assignee: Option<String>,

        #[arg(short, long)]
        priority: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Show issue details
    Show {
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Update an issue
    Update {
        id: String,

        #[arg(short, long)]
        title: Option<String>,

        #[arg(short, long)]
        desc: Option<String>,

        #[arg(short, long)]
        priority: Option<String>,

        #[arg(short, long)]
        state: Option<String>,

        /// Add label(s) (format: namespace:value, repeatable)
        #[arg(short, long)]
        label: Vec<String>,

        /// Remove label(s) (repeatable)
        #[arg(long)]
        remove_label: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Delete an issue
    Delete {
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Break down an issue into subtasks with automatic dependency inheritance
    Breakdown {
        /// Parent issue ID to break down
        parent_id: String,

        /// Subtask titles (use multiple times)
        #[arg(long = "subtask", required = true)]
        subtask_titles: Vec<String>,

        /// Subtask descriptions (optional, must match number of subtasks)
        #[arg(long = "desc")]
        subtask_descs: Vec<String>,

        /// Output as JSON for machine consumption
        #[arg(long)]
        json: bool,
    },

    /// Assign issue to someone
    Assign {
        /// Issue ID
        id: String,

        /// Assignee (format: type:identifier, e.g., agent:worker-1)
        assignee: String,

        #[arg(long)]
        json: bool,
    },

    /// Claim an unassigned issue (atomic)
    Claim {
        /// Issue ID
        id: String,

        /// Assignee (format: type:identifier, e.g., agent:worker-1)
        assignee: String,

        #[arg(long)]
        json: bool,
    },

    /// Unassign an issue
    Unassign {
        /// Issue ID
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Release an issue from its assignee (for timeout recovery)
    Release {
        /// Issue ID
        id: String,

        /// Reason for release (e.g., timeout, error)
        reason: String,

        #[arg(long)]
        json: bool,
    },

    /// Claim the next available ready issue
    ClaimNext {
        /// Assignee (format: type:identifier, e.g., agent:worker-1)
        assignee: String,

        #[arg(short, long)]
        filter: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DepCommands {
    /// Add a dependency (FROM depends on TO)
    Add {
        /// Issue that depends on another (FROM)
        from_id: String,

        /// Dependency required (TO)
        to_id: String,

        #[arg(long)]
        json: bool,
    },

    /// Remove a dependency
    Rm {
        /// Issue to modify (FROM)
        from_id: String,

        /// Dependency to remove (TO)
        to_id: String,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum GateCommands {
    /// Add a gate requirement to an issue
    Add {
        /// Issue ID
        id: String,

        /// Gate key (from registry)
        gate_key: String,

        #[arg(long)]
        json: bool,
    },

    /// Mark a gate as passed
    Pass {
        /// Issue ID
        id: String,

        /// Gate key
        gate_key: String,

        /// Who passed the gate (optional)
        #[arg(short, long)]
        by: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Mark a gate as failed
    Fail {
        /// Issue ID
        id: String,

        /// Gate key
        gate_key: String,

        /// Who failed the gate (optional)
        #[arg(short, long)]
        by: Option<String>,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum DocCommands {
    /// Add a document reference to an issue
    Add {
        /// Issue ID
        id: String,

        /// Path to document relative to repository root
        path: String,

        /// Git commit hash (optional, defaults to HEAD)
        #[arg(short, long)]
        commit: Option<String>,

        /// Human-readable label
        #[arg(short, long)]
        label: Option<String>,

        /// Document type (e.g., design, implementation, notes)
        #[arg(short = 't', long)]
        doc_type: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// List document references for an issue
    List {
        /// Issue ID
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Remove a document reference from an issue
    Remove {
        /// Issue ID
        id: String,

        /// Path to document to remove
        path: String,

        #[arg(long)]
        json: bool,
    },

    /// Show document content
    Show {
        /// Issue ID
        id: String,

        /// Path to document
        path: String,

        /// View document at specific commit (defaults to HEAD)
        #[arg(long)]
        at: Option<String>,
    },

    /// List commit history for a document
    History {
        /// Issue ID
        id: String,

        /// Path to document
        path: String,

        #[arg(long)]
        json: bool,
    },

    /// Show diff between document versions
    Diff {
        /// Issue ID
        id: String,

        /// Path to document
        path: String,

        /// Source commit (required)
        #[arg(long)]
        from: String,

        /// Target commit (defaults to HEAD)
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum GraphCommands {
    /// Show dependency tree for an issue
    Show {
        /// Issue ID (optional - shows all if omitted)
        id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Show issues that depend on this issue
    Downstream {
        /// Issue ID
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Show root issues (no dependencies)
    Roots {
        #[arg(long)]
        json: bool,
    },

    /// Export dependency graph in various formats
    Export {
        /// Output format (dot, mermaid)
        #[arg(short, long, default_value = "dot")]
        format: String,

        /// Output file (optional - prints to stdout if omitted)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RegistryCommands {
    /// List all gate definitions
    List {
        #[arg(long)]
        json: bool,
    },

    /// Add a gate definition to the registry
    Add {
        /// Unique gate key
        key: String,

        #[arg(short, long)]
        title: String,

        #[arg(short, long, default_value = "")]
        desc: String,

        #[arg(short, long)]
        auto: bool,

        #[arg(short, long)]
        example: Option<String>,
    },

    /// Remove a gate definition
    Remove { key: String },

    /// Show gate definition details
    Show {
        key: String,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum EventCommands {
    /// Tail recent events
    Tail {
        #[arg(short, long, default_value = "10")]
        n: usize,
    },

    /// Query events by type or issue
    Query {
        #[arg(short, long)]
        event_type: Option<String>,

        #[arg(short, long)]
        issue_id: Option<String>,

        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum QueryCommands {
    /// Query ready issues (unassigned, state=ready, unblocked)
    Ready {
        #[arg(long)]
        json: bool,
    },

    /// Query blocked issues with reasons
    Blocked {
        #[arg(long)]
        json: bool,
    },

    /// Query issues by assignee
    Assignee {
        assignee: String,

        #[arg(long)]
        json: bool,
    },

    /// Query issues by state
    State {
        state: String,

        #[arg(long)]
        json: bool,
    },

    /// Query issues by priority
    Priority {
        priority: String,

        #[arg(long)]
        json: bool,
    },

    /// Query issues by label (exact match or wildcard)
    Label {
        /// Label pattern: 'namespace:value' for exact match, 'namespace:*' for wildcard
        pattern: String,

        #[arg(long)]
        json: bool,
    },

    /// Query strategic issues (those with labels from strategic namespaces)
    Strategic {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum LabelCommands {
    /// List all label namespaces
    Namespaces {
        #[arg(long)]
        json: bool,
    },

    /// List all values used in a namespace
    Values {
        /// Namespace to query (e.g., 'milestone', 'epic')
        namespace: String,

        #[arg(long)]
        json: bool,
    },

    /// Add a custom label namespace
    AddNamespace {
        /// Namespace name (lowercase alphanumeric with hyphens)
        name: String,

        /// Human-readable description
        #[arg(short, long)]
        description: String,

        /// Only one label from this namespace per issue
        #[arg(long)]
        unique: bool,

        /// Namespace is for strategic planning (appears in strategic queries)
        #[arg(long)]
        strategic: bool,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show current type hierarchy
    ShowHierarchy {
        #[arg(long)]
        json: bool,
    },

    /// List available hierarchy templates
    ListTemplates {
        #[arg(long)]
        json: bool,
    },
}
