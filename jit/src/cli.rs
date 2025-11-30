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

        #[arg(short, long)]
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
        id: String,

        #[arg(short, long)]
        to: String,

        #[arg(long)]
        json: bool,
    },

    /// Claim an unassigned issue (atomic)
    Claim {
        id: String,

        #[arg(short, long)]
        to: String,

        #[arg(long)]
        json: bool,
    },

    /// Unassign an issue
    Unassign {
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Release an issue from its assignee (for timeout recovery)
    Release {
        id: String,

        #[arg(short, long)]
        reason: String,

        #[arg(long)]
        json: bool,
    },

    /// Claim the next available ready issue
    ClaimNext {
        #[arg(short, long)]
        to: String,

        #[arg(short, long)]
        filter: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DepCommands {
    /// Add a dependency
    Add {
        /// Issue that depends on another
        id: String,

        /// Dependency to add
        #[arg(short, long)]
        on: String,
    },

    /// Remove a dependency
    Rm {
        /// Issue to modify
        id: String,

        /// Dependency to remove
        #[arg(short, long)]
        on: String,
    },
}

#[derive(Subcommand)]
pub enum GateCommands {
    /// Add a gate requirement to an issue
    Add { id: String, gate_key: String },

    /// Mark a gate as passed
    Pass {
        id: String,
        gate_key: String,

        #[arg(short, long)]
        by: Option<String>,
    },

    /// Mark a gate as failed
    Fail {
        id: String,
        gate_key: String,

        #[arg(short, long)]
        by: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum GraphCommands {
    /// Show dependency tree for an issue
    Show { id: String },

    /// Show issues that depend on this issue
    Downstream { id: String },

    /// Show root issues (no dependencies)
    Roots,

    /// Export dependency graph in various formats
    Export {
        #[arg(short, long, default_value = "dot")]
        format: String,

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
