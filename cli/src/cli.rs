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

    /// Graph query commands
    #[command(subcommand)]
    Graph(GraphCommands),

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

    /// Show issue details
    Show { id: String },

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
    },

    /// Delete an issue
    Delete { id: String },

    /// Assign issue to someone
    Assign {
        id: String,

        #[arg(short, long)]
        to: String,
    },

    /// Claim an unassigned issue (atomic)
    Claim {
        id: String,

        #[arg(short, long)]
        to: String,
    },

    /// Unassign an issue
    Unassign { id: String },

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
}
