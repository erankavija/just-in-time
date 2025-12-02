//! Command-line interface definitions using clap.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "jit")]
#[command(about = "Just-In-Time issue tracker", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize the issue tracker in the current directory
    Init,

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

    /// Graph query commands
    #[command(subcommand)]
    Graph(GraphCommands),

    /// Query issues for orchestrators
    #[command(subcommand)]
    Query(QueryCommands),

    /// Show overall status
    Status,

    /// Validate repository integrity
    Validate,
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

        #[arg(long)]
        json: bool,
    },

    /// Delete an issue
    Delete {
        id: String,

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
    },

    /// Remove a dependency
    Rm {
        /// Issue to modify (FROM)
        from_id: String,

        /// Dependency to remove (TO)
        to_id: String,
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
    },
}

#[derive(Subcommand)]
pub enum GraphCommands {
    /// Show dependency tree for an issue
    Show {
        /// Issue ID (optional - shows all if omitted)
        id: Option<String>,
    },

    /// Show issues that depend on this issue
    Downstream {
        /// Issue ID
        id: String,
    },

    /// Show root issues (no dependencies)
    Roots,

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
    List,

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
    Show { key: String },
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
}
