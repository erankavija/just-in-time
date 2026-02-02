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
    /// Suppress non-essential output (for scripting)
    #[arg(short, long, global = true)]
    pub quiet: bool,

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

    /// Snapshot export commands
    #[command(subcommand)]
    Snapshot(SnapshotCommands),

    /// Claim coordination commands
    ///
    /// Manage lease-based claims on issues for parallel work coordination.
    /// Leases prevent conflicting edits across multiple agents and worktrees.
    #[command(subcommand)]
    Claim(ClaimCommands),

    /// Worktree information commands
    ///
    /// Display and manage git worktree context for parallel work.
    #[command(subcommand)]
    Worktree(WorktreeCommands),

    /// Git hooks installation and management
    #[command(subcommand)]
    Hooks(HooksCommands),

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

        /// Validate branch hasn't diverged from main
        #[arg(long)]
        divergence: bool,

        /// Validate active leases are consistent and not stale
        #[arg(long)]
        leases: bool,
    },

    /// Run recovery routines to fix common issues
    ///
    /// Performs automatic recovery operations:
    /// - Cleans up stale locks from crashed processes (PID check)
    /// - Rebuilds corrupted claims index from append-only log
    /// - Evicts expired leases
    /// - Removes orphaned temp files (older than 1 hour)
    ///
    /// Safe to run at any time - only removes provably stale data.
    Recover {
        #[arg(long)]
        json: bool,
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

    /// Update an issue or multiple issues
    Update {
        /// Issue ID (for single issue mode, mutually exclusive with --filter)
        id: Option<String>,

        /// Boolean query filter (for batch mode, mutually exclusive with ID)
        #[arg(long, conflicts_with = "id")]
        filter: Option<String>,

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

        /// Add gate(s) to issue (gate keys from registry, repeatable)
        #[arg(long, value_delimiter = ',')]
        add_gate: Vec<String>,

        /// Remove gate(s) from issue (repeatable)
        #[arg(long, value_delimiter = ',')]
        remove_gate: Vec<String>,

        /// Set assignee (format: type:identifier)
        #[arg(long)]
        assignee: Option<String>,

        /// Clear assignee
        #[arg(long)]
        unassign: bool,

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
    ///   jit dep add epic-123 task-456           # Single dependency
    ///   jit dep add epic-123 task-1 task-2 task-3  # Multiple dependencies
    Add {
        /// Issue that depends on another (FROM)
        from_id: String,

        /// Dependency/dependencies required (TO)
        /// Can specify multiple: jit dep add <from> to1 to2 to3
        #[arg(required = true)]
        to_ids: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Remove a dependency
    ///
    /// Examples:
    ///   jit dep rm epic-123 task-456            # Single dependency
    ///   jit dep rm epic-123 task-1 task-2       # Multiple dependencies
    Rm {
        /// Issue to modify (FROM)
        from_id: String,

        /// Dependency/dependencies to remove (TO)
        #[arg(required = true)]
        to_ids: Vec<String>,

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
    /// Examples:
    ///   jit gate add abc123 code-review
    ///   jit gate add abc123 tests clippy fmt
    Add {
        /// Issue ID
        id: String,

        /// Gate key(s) from registry (e.g., 'tests', 'code-review', 'clippy')
        /// Can specify multiple: jit gate add <issue> gate1 gate2 gate3
        #[arg(required = true)]
        gate_keys: Vec<String>,

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

    /// Show what an issue depends on (upstream dependencies)
    ///
    /// Shows the issues that must be completed before this issue can proceed.
    /// By default shows immediate dependencies only (depth 1).
    ///
    /// Example:
    ///   jit graph deps epic-123
    ///   Shows: task-456, task-789 (epic depends on these)
    ///
    /// "Dependencies" = what this issue needs (upstream in work flow).
    /// "Dependents" = what needs this issue (downstream in work flow).
    #[command(alias = "dependencies")]
    Deps {
        /// Issue ID
        id: String,

        /// Show transitive dependencies (all levels)
        #[arg(long)]
        transitive: bool,

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
    /// Query all issues with optional filters
    All {
        /// Filter by state
        #[arg(short = 's', long)]
        state: Option<String>,

        /// Filter by assignee (format: type:identifier)
        #[arg(short = 'a', long)]
        assignee: Option<String>,

        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (exact match or wildcard)
        #[arg(short = 'l', long)]
        label: Option<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query available issues (unassigned, state=ready, unblocked)
    Available {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (exact match or wildcard)
        #[arg(short = 'l', long)]
        label: Option<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query blocked issues with reasons
    Blocked {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (exact match or wildcard)
        #[arg(short = 'l', long)]
        label: Option<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query strategic issues (those with labels from strategic namespaces)
    Strategic {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (exact match or wildcard)
        #[arg(short = 'l', long)]
        label: Option<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query closed issues (Done or Rejected states)
    Closed {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (exact match or wildcard)
        #[arg(short = 'l', long)]
        label: Option<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

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
    /// Show effective configuration from all sources
    ///
    /// Displays merged configuration with values from:
    /// 1. Repository config (.jit/config.toml) - highest priority
    /// 2. User config (~/.config/jit/config.toml)
    /// 3. System config (/etc/jit/config.toml)
    /// 4. Defaults - lowest priority
    Show {
        #[arg(long)]
        json: bool,
    },

    /// Get a specific configuration value
    ///
    /// Examples:
    ///   jit config get coordination.default_ttl_secs
    ///   jit config get worktree.mode
    Get {
        /// Configuration key (e.g., coordination.default_ttl_secs)
        key: String,
        #[arg(long)]
        json: bool,
    },

    /// Set a configuration value
    ///
    /// By default, sets value in repository config (.jit/config.toml).
    /// Use --global to set in user config (~/.config/jit/config.toml).
    ///
    /// Examples:
    ///   jit config set coordination.default_ttl_secs 1200
    ///   jit config set --global worktree.enforce_leases warn
    Set {
        /// Configuration key (e.g., coordination.default_ttl_secs)
        key: String,
        /// Value to set
        value: String,
        /// Set in user config instead of repository config
        #[arg(long)]
        global: bool,
        #[arg(long)]
        json: bool,
    },

    /// Validate configuration files for errors and warnings
    ///
    /// Checks configuration for:
    /// - Syntax errors in TOML files
    /// - Invalid values (e.g., unknown mode values)
    /// - Deprecated options
    /// - Missing required fields
    ///
    /// Exit codes:
    ///   0 - Valid configuration
    ///   1 - Errors found (invalid configuration)
    ///   2 - Warnings only (valid but may cause issues)
    Validate {
        #[arg(long)]
        json: bool,
    },

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

#[derive(Subcommand)]
pub enum SnapshotCommands {
    /// Archive a complete snapshot of issues and documents
    ///
    /// Creates a portable snapshot containing:
    /// - Issue state (.jit/issues/*.json)
    /// - Documents referenced by issues
    /// - Assets (images, diagrams) used in documents  
    /// - Manifest with SHA256 hashes for verification
    /// - README with instructions
    ///
    /// Snapshots are self-contained and can be archived, transferred, or audited.
    /// They preserve complete provenance (git commit, source mode, timestamps).
    ///
    /// Examples:
    ///   # Export all issues to directory
    ///   jit snapshot export
    ///
    ///   # Export specific epic as tar archive
    ///   jit snapshot export --scope label:epic:auth --format tar --out auth-snapshot.tar
    ///
    ///   # Export from specific git commit
    ///   jit snapshot export --at abc123 --out release-v1.0
    ///
    ///   # Export only working tree files (no git)
    ///   jit snapshot export --working-tree
    Export {
        /// Output path (default: snapshot-YYYYMMDD-HHMMSS)
        #[arg(long)]
        out: Option<String>,

        /// Output format: dir or tar
        #[arg(long, default_value = "dir")]
        format: String,

        /// Scope: all (default), issue:ID, or label:namespace:value
        ///
        /// Examples:
        ///   --scope all                    All issues
        ///   --scope issue:abc123           Single issue
        ///   --scope label:epic:auth        Issues with label epic:auth
        ///   --scope label:milestone:v1.0   Issues in milestone v1.0
        #[arg(long, default_value = "all")]
        scope: String,

        /// Git commit/tag to export (requires git repository)
        #[arg(long)]
        at: Option<String>,

        /// Export from working tree instead of git
        #[arg(long)]
        working_tree: bool,

        /// Reject if uncommitted docs/assets exist (requires git, implies --at HEAD)
        #[arg(long)]
        committed_only: bool,

        /// Skip repository validation before export
        #[arg(long)]
        force: bool,

        /// Output metadata as JSON instead of human-readable format
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ClaimCommands {
    /// Acquire a lease on an issue
    ///
    /// Acquires an exclusive lease to work on an issue. Only one agent can hold
    /// a lease on an issue at a time, preventing conflicting edits.
    ///
    /// Examples:
    ///   jit claim acquire abc123 --ttl 600        # 10-minute lease
    ///   jit claim acquire abc123 --ttl 3600       # 1-hour lease
    ///   jit claim acquire abc123 --ttl 0 --reason "Manual oversight"  # Indefinite (requires reason)
    Acquire {
        /// Issue ID to claim
        issue_id: String,

        /// Time-to-live in seconds (0 for indefinite, requires --reason)
        #[arg(long, default_value = "600")]
        ttl: u64,

        /// Agent identifier (defaults to current agent from config)
        #[arg(long)]
        agent_id: Option<String>,

        /// Reason for claim (required for TTL=0)
        #[arg(long)]
        reason: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Release a lease
    ///
    /// Explicitly releases a lease before it expires, making the issue available
    /// for other agents to claim.
    ///
    /// Examples:
    ///   jit claim release abc12345-6789-...
    ///   jit claim release abc12345-6789-... --json
    Release {
        /// Lease ID to release
        lease_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Renew a lease
    ///
    /// Extends the expiry time of an existing lease. Allows agents to continue
    /// working on an issue beyond the original TTL.
    ///
    /// Examples:
    ///   jit claim renew abc12345-6789-... --extension 600   # Extend by 10 minutes
    ///   jit claim renew abc12345-6789-... --extension 3600  # Extend by 1 hour
    ///   jit claim renew abc12345-6789-...                   # Use default (10 minutes)
    Renew {
        /// Lease ID to renew
        lease_id: String,

        /// How many seconds to extend the lease by
        #[arg(long, default_value = "600")]
        extension: u64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Send heartbeat for an indefinite lease
    ///
    /// Updates the last_beat timestamp without changing expiration.
    /// Used to signal the agent is still actively working on the issue.
    /// Only needed for TTL=0 (indefinite) leases to prevent staleness.
    ///
    /// Examples:
    ///   jit claim heartbeat abc12345-6789-...       # Send heartbeat
    ///   jit claim heartbeat abc12345-6789-... --json
    Heartbeat {
        /// Lease ID to heartbeat
        lease_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show active lease status
    ///
    /// By default, shows leases for the current agent.
    /// Use --issue or --agent to filter by specific issue or agent.
    ///
    /// Examples:
    ///   jit claim status                        # Show my leases
    ///   jit claim status --issue 01ABC          # Check who has issue
    ///   jit claim status --agent agent:copilot  # Show copilot's leases
    ///   jit claim status --json
    Status {
        /// Filter by issue ID
        #[arg(long)]
        issue: Option<String>,

        /// Filter by agent ID (format: type:identifier)
        #[arg(long)]
        agent: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all active leases
    ///
    /// Shows all active leases across all agents and worktrees. Useful for
    /// seeing the global state of who is working on what.
    ///
    /// Examples:
    ///   jit claim list             # Show all leases
    ///   jit claim list --json      # JSON output
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Force-evict a lease (admin operation)
    ///
    /// Removes a lease immediately, regardless of who owns it. This is an
    /// administrative operation for handling stale leases or emergency situations.
    /// The eviction is logged with the provided reason for audit trail.
    ///
    /// Examples:
    ///   jit claim force-evict abc12345-6789-... --reason "Stale after crash"
    ///   jit claim force-evict abc12345-6789-... --reason "Admin override" --json
    ForceEvict {
        /// Lease ID to evict
        lease_id: String,

        /// Reason for eviction (required for audit trail)
        #[arg(long)]
        reason: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Worktree commands
#[derive(Debug, Subcommand)]
pub enum WorktreeCommands {
    /// Show current worktree information
    ///
    /// Displays worktree ID, branch, root path, and whether this is the
    /// main worktree or a secondary one.
    ///
    /// Examples:
    ///   jit worktree info          # Show current worktree info
    ///   jit worktree info --json   # JSON output
    Info {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all git worktrees with JIT status
    ///
    /// Shows all worktrees with their worktree ID, branch, path,
    /// and count of active claims.
    ///
    /// Examples:
    ///   jit worktree list          # List all worktrees
    ///   jit worktree list --json   # JSON output
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Git hooks commands
#[derive(Debug, Subcommand)]
pub enum HooksCommands {
    /// Install git hooks for lease and divergence validation
    ///
    /// Copies hook templates to .git/hooks/ and makes them executable.
    ///
    /// Hooks installed:
    ///   - pre-commit: Validates leases and divergence before commit
    ///   - pre-push: Validates leases before push
    ///
    /// Examples:
    ///   jit hooks install          # Install hooks
    ///   jit hooks install --json   # JSON output
    Install {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}
