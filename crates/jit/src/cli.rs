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
    ///
    /// Gates are quality checkpoints (tests, reviews, scans) that enforce workflow quality.
    /// Unlike labels (which are arbitrary tags for organization), gates have executable logic
    /// and block state transitions until they pass.
    ///
    /// Common workflow:
    ///   1. Define gates in registry: jit gate define code-review --title "Code Review" ...
    ///   2. Add to issues: jit issue create --gate code-review ...
    ///   3. Execute gates: jit gate pass \<issue\> code-review
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

        #[arg(short = 'd', long = "description", default_value = "")]
        description: String,

        #[arg(short, long, default_value = "normal")]
        priority: String,

        /// Gate keys from registry to require (e.g., 'tests', 'code-review').
        /// Gates are quality checkpoints that must pass before issue completion.
        /// Use comma-separated (--gate tests,clippy) or multiple flags (--gate tests --gate clippy).
        /// Gates must be defined in registry first with 'jit gate define'.
        #[arg(short, long, value_delimiter = ',')]
        gate: Vec<String>,

        /// Labels (format: namespace:value, repeatable)
        #[arg(short, long)]
        label: Vec<String>,

        /// Bypass validation warnings
        #[arg(long)]
        force: bool,

        /// Explicitly allow orphaned leaf issues (tasks without parent labels)
        #[arg(long)]
        orphan: bool,

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

        #[arg(short = 'd', long = "description")]
        description: Option<String>,

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
        #[arg(long = "description")]
        subtask_descriptions: Vec<String>,

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

    /// Reject an issue (convenience for --state rejected)
    Reject {
        /// Issue ID
        id: String,

        /// Reason for rejection (adds resolution:REASON label)
        #[arg(long)]
        reason: Option<String>,

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
    /// Add a work dependency: FROM is blocked until TO completes
    ///
    /// FROM and TO can be any issues. Work flows from TO (upstream) into FROM (downstream).
    /// Dependencies are orthogonal to labels - issues don't need matching labels to depend on each other.
    ///
    /// Examples:
    ///   jit dep add epic-123 task-456     # Epic blocked until task done
    ///   jit dep add planning-v2 release-v1 # v2.0 planning waits for v1.0 release
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
    /// Define a new gate in the registry
    Define {
        /// Unique gate key
        key: String,

        /// Human-readable title
        #[arg(short, long)]
        title: String,

        /// Description of what this gate checks
        #[arg(short = 'd', long)]
        description: String,

        /// Gate stage: precheck or postcheck
        #[arg(short, long, default_value = "postcheck")]
        stage: String,

        /// Gate mode: manual or auto
        #[arg(short, long, default_value = "manual")]
        mode: String,

        /// Command to execute for automated gates
        #[arg(long)]
        checker_command: Option<String>,

        /// Timeout in seconds for checker command
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// Working directory for checker (relative to repo root)
        #[arg(long)]
        working_dir: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// List all gate definitions
    List {
        #[arg(long)]
        json: bool,
    },

    /// Show gate definition details
    Show {
        /// Gate key
        key: String,

        #[arg(long)]
        json: bool,
    },

    /// Remove a gate definition from the registry
    Remove {
        /// Gate key
        key: String,

        #[arg(long)]
        json: bool,
    },

    /// Add a gate requirement to an issue
    ///
    /// Gates are quality checkpoints (e.g., tests, reviews) that must pass before
    /// an issue can transition to ready or done states. Gate keys must exist in the
    /// registry - use 'jit gate list' to see available gates or 'jit gate define'
    /// to create new ones.
    ///
    /// Example: jit gate add abc123 code-review
    Add {
        /// Issue ID
        id: String,

        /// Gate key from registry (e.g., 'tests', 'code-review', 'clippy')
        gate_key: String,

        #[arg(long)]
        json: bool,
    },

    /// Check a single gate for an issue (run automated checker)
    Check {
        /// Issue ID
        id: String,

        /// Gate key
        gate_key: String,

        #[arg(long)]
        json: bool,
    },

    /// Check all automated gates for an issue
    CheckAll {
        /// Issue ID
        id: String,

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

        /// Skip scanning document for assets (default: false)
        #[arg(long, default_value_t = false)]
        skip_scan: bool,

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

    /// Asset management commands
    Assets {
        #[command(subcommand)]
        command: AssetCommands,
    },

    /// Check document links and assets for validity
    CheckLinks {
        /// Scope of validation (all or issue:ID)
        #[arg(long, default_value = "all")]
        scope: String,

        /// Output results in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Archive a document with its assets
    Archive {
        /// Document path to archive (repo-relative)
        path: String,

        /// Archive category (must be configured in config.toml)
        #[arg(long = "type")]
        category: String,

        /// Show plan without executing
        #[arg(long)]
        dry_run: bool,

        /// Override safety checks (allow archival of docs linked to active issues)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum AssetCommands {
    /// List assets for a document
    List {
        /// Issue ID
        id: String,

        /// Path to document
        path: String,

        /// Rescan document to refresh asset metadata
        #[arg(long, default_value_t = false)]
        rescan: bool,

        #[arg(long)]
        json: bool,
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

    /// Show downstream dependents (issues that are blocked by this one)
    ///
    /// "Downstream" means work flow direction (toward completion/delivery).
    /// If Epic depends on Task, then Task is upstream and Epic is downstream.
    ///
    /// Example:
    ///   jit graph downstream task-456
    ///   Shows: epic-123, milestone-789 (they depend on this task)
    ///
    /// Note: This shows dependency relationships, not label hierarchy.
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

        #[arg(short = 'd', long = "description", default_value = "")]
        description: String,

        #[arg(short, long)]
        auto: bool,

        #[arg(short, long)]
        example: Option<String>,

        /// Gate execution stage (precheck or postcheck)
        #[arg(short, long, default_value = "postcheck")]
        stage: String,
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

    /// Query closed issues (Done or Rejected states)
    Closed {
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
