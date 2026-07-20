//! Command-line interface definitions using clap.

use clap::{Parser, Subcommand, ValueEnum};

use crate::build_info;

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
#[command(after_help = "For JSON output shapes and exit code documentation, run `jit --schema`.")]
#[command(version = build_info::VERSION_TEXT)]
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

        /// Apply an embedded profile during initialization
        #[arg(long)]
        profile: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Inspect and apply embedded repository profiles
    #[command(subcommand)]
    Profile(ProfileCommands),

    /// Issue management commands
    #[command(subcommand)]
    Issue(IssueCommands),

    /// List issues (top-level alias for `jit issue list`)
    ///
    /// Convenience first-guess spelling that routes to the canonical
    /// `jit issue list`. Accepts the same filters and behaves identically.
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    List {
        /// Filter by state
        #[arg(short = 's', long)]
        state: Option<String>,

        /// Filter by assignee (format: type:identifier)
        #[arg(short = 'a', long)]
        assignee: Option<String>,

        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Apply a graph template to a container (plan-before-fan-out scaffold)
    ///
    /// Instantiates the named template from `.jit/templates.toml` onto
    /// `<container>`: creates the template's typed nodes (with their gate
    /// presets, docs, and interpolated descriptions), wires the declared edges,
    /// and runs its transforms (e.g. moving the container's upstream deps onto
    /// the planning node). The repository's container anchor (`.jit/templates.toml`
    /// `[anchors] container`, default `container`) is auto-bound to the positional
    /// `<container>`; bind any additional anchors with `--anchor role=id`.
    ///
    /// The container's `type:` label must be one of the template's `applies_to`
    /// types. Re-applying requires `--force`, which refreshes the existing nodes'
    /// prose in place rather than re-creating them.
    ///
    /// Examples:
    ///   jit apply plan epic-123                       # Apply the `plan` template
    ///   jit apply plan epic-123 --json                # Machine-readable result
    ///   jit apply plan epic-123 --anchor container=epic-123 --force
    Apply {
        /// Template name to apply (must be declared in `.jit/templates.toml`)
        template: String,

        /// Container issue ID to apply the template to
        container: String,

        /// Bind a template anchor: `role=id` (repeatable). The repository's
        /// container anchor is auto-bound to `<container>`; binding it explicitly
        /// overrides that.
        #[arg(long, value_name = "ROLE=ID")]
        anchor: Vec<String>,

        /// Bypass validation warnings and refresh an already-applied template
        #[arg(long)]
        force: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Dependency management commands
    #[command(subcommand, visible_alias = "dependency")]
    Dep(DepCommands),

    /// Gate management commands
    ///
    /// Gates are quality checkpoints (tests, reviews, scans) that enforce workflow quality.
    /// Unlike labels (which are arbitrary tags for organization), gates have executable logic
    /// and block state transitions until they pass.
    ///
    /// Verbs group by what they do to gate state:
    ///
    /// Produce a verdict (MUTATE gate state): evaluate (alias eval), evaluate-all, fail.
    ///
    /// Report state (READ-ONLY): list, show, status, status-all.
    ///
    /// Shape gates and requirements (MUTATE): define, update, remove, add, preset.
    ///
    /// Only the verdict and configuration verbs mutate; the report-state verbs
    /// never change anything. `status-all` is read-only but exits nonzero unless
    /// every required gate has passed.
    ///
    /// Common workflow:
    ///   1. Define gates in registry: jit gate define code-review --title "Code Review" ...
    ///   2. Add to issues: jit issue create --gate code-review ...
    ///   3. Evaluate gates: jit gate evaluate \<issue\> code-review
    #[command(subcommand)]
    Gate(GateCommands),

    /// Event log commands
    #[command(subcommand)]
    Events(EventCommands),

    /// Document reference commands
    #[command(subcommand, visible_alias = "document")]
    Doc(DocCommands),

    /// Preview dependency-aware artifact archival plans
    #[command(subcommand)]
    Archive(ArchiveCommands),

    /// Graph query commands
    #[command(subcommand)]
    Graph(GraphCommands),

    /// Reverse dependencies (top-level alias for `jit graph rdeps`)
    ///
    /// Convenience first-guess spelling that routes to the canonical
    /// `jit graph rdeps <id>`: shows the issues that depend on `<id>`.
    ///
    /// JSON output uses the list envelope `{"count": N, "dependents": [...]}`.
    Rdeps {
        /// Issue ID
        id: String,

        /// Depth of traversal: 1 = immediate dependents (default), 0 = all
        /// transitive dependents (unlimited, opt-in). Matches `jit graph rdeps`.
        #[arg(long, default_value = "1")]
        depth: u32,

        #[arg(long)]
        json: bool,
    },

    /// Query issues for orchestrators
    ///
    /// With no subcommand, returns all issues (equivalent to `jit query all`).
    /// Filters (`--state`, `--assignee`, `--priority`, `--label`) narrow the
    /// default listing. `--label` is repeatable and ANDed. Use `jit query
    /// ready` (alias of `available`) for unassigned, unblocked ready issues.
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    Query {
        /// Subcommand — omit to list all issues
        #[command(subcommand)]
        subcommand: Option<QueryCommands>,

        /// Filter by state (used when no subcommand is given)
        #[arg(short = 's', long)]
        state: Option<String>,

        /// Filter by assignee — format: type:identifier (used when no subcommand is given)
        #[arg(short = 'a', long)]
        assignee: Option<String>,

        /// Filter by priority (used when no subcommand is given)
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard) — used when no subcommand is given.
        /// Repeatable; patterns are ANDed, so an issue must match every
        /// pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries (used when no subcommand is given)
        #[arg(long)]
        full: bool,

        /// Output in JSON format (used when no subcommand is given)
        #[arg(long)]
        json: bool,
    },

    /// Label namespace management commands
    ///
    /// Inspects the label namespace registry itself (which namespaces exist,
    /// which values have been used) — it does NOT add or remove labels on an
    /// issue. To label an issue, use `jit issue update <id> --label
    /// <namespace:value>` (and `--remove-label` to remove one).
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

    /// Addressable structured item commands
    ///
    /// Items are structured lines in issue descriptions (e.g. requirements) that
    /// carry a self-id and are addressable by a uniform kind-segmented qualified id
    /// (`@/issue/<short-id>/<kind>/<self-id>` for an issue item, `@/<kind>/<self-id>`
    /// for a project item). Kinds are declared in `[item_kinds]` config; with no such table no kinds
    /// are declared (`jit init` scaffolds the table). Markdown stays the source of
    /// truth — the index is a projection. A kind may declare `aliases` in config;
    /// an alias is accepted anywhere a kind name is (the kind segment of an address,
    /// and `--kind` filters), e.g. `@/inv/<self-id>` for the `invariant` kind, while
    /// canonical output always uses the registry name.
    #[command(subcommand)]
    Item(ItemCommands),

    /// Invariant enforcement-drift check
    ///
    /// `check` reports invariants whose `enforced-by` binding names a missing or
    /// unloadable rule/gate. To render the invariant registry into documentation,
    /// use `jit project render` (the generic `[projection.*]` command).
    #[command(subcommand)]
    Invariant(InvariantCommands),

    /// Render documentation projections declared in `[projection.*]`
    ///
    /// Each `[projection.<name>]` config table projects an addressable item kind
    /// (or kinds) into a documentation target. `render` writes every declared
    /// projection, or a single named one, atomically; in region mode only the
    /// delimited block changes. Targets and delimiters come only from config.
    #[command(subcommand)]
    Project(ProjectCommands),

    /// Search issues and documents
    ///
    /// JSON output uses the list envelope `{"count": N, "results": [...]}`.
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

    /// Show CLI version and local build provenance
    Version {
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
        /// Issue id to validate (runs local + graph rules for this issue only).
        /// When omitted, validates the whole repository.
        id: Option<String>,

        #[arg(long)]
        json: bool,

        /// Explain rule outcomes for the issue: which rules matched and whether
        /// each passed or failed (requires an issue id)
        #[arg(long)]
        explain: bool,

        /// Validate a container's bracket subtree as a deterministic gate
        /// checker: evaluate the rules whose selector matches each issue in the
        /// container's transitive dependency closure (including the
        /// `type:breakdown` node, bounded there), excluding whole-repo rules.
        /// Exits 4 with findings shown when an enforcing rule fails, 0 when
        /// clean. Mutually exclusive with a positional id and with
        /// `--fix`/`--branch-drift`/`--leases`/`--explain`.
        #[arg(long, value_name = "ID")]
        scope: Option<String>,

        /// Attempt to automatically fix validation issues
        #[arg(long)]
        fix: bool,

        /// Show what would be fixed without applying changes (requires --fix)
        #[arg(long)]
        dry_run: bool,

        /// Validate that git's `origin/main` is an ancestor of the current
        /// branch, so the branch still sits on top of it. This is the git
        /// concern; membership labels the DAG does not back are reported by
        /// `jit query divergence`. Requires git.
        #[arg(long)]
        branch_drift: bool,

        /// Hidden stub: fails with a hint naming `--branch-drift`
        #[arg(long, hide = true)]
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

    /// Run one-time data migrations
    ///
    /// Migrations are idempotent: re-running one over an already-migrated
    /// repository changes nothing and is safe.
    #[command(subcommand)]
    Migrate(MigrateCommands),

    /// Start the JIT API and web UI server as a background process
    ///
    /// Launches `jit-server` as a daemon (detached from the terminal).
    /// If a server is already running for this repository it prints its
    /// status and exits without starting a second instance.
    ///
    /// Multiple repositories on the same host are supported — each gets its
    /// own port, auto-selected in the range 3000–3099.
    ///
    /// Examples:
    ///   jit serve                  # Start (or report already-running)
    ///   jit serve --port 3010      # Prefer a specific port
    ///   jit serve --status         # Check if server is running
    ///   jit serve --stop           # Stop the running server
    ///   jit serve --fg             # Run in foreground (for debugging)
    ///   jit serve --json           # Machine-readable output
    Serve {
        /// Preferred port to listen on (auto-selects from 3000–3099 if
        /// taken; pass 0 for any OS-assigned free port)
        #[arg(long, default_value = "3000")]
        port: u16,

        /// Stop the running server for this repository
        #[arg(long, conflicts_with_all = ["status", "fg"])]
        stop: bool,

        /// Show server status and exit
        #[arg(long, conflicts_with_all = ["stop", "fg"])]
        status: bool,

        /// Run in foreground instead of daemonizing (useful for debugging)
        #[arg(long)]
        fg: bool,

        /// Write server output to this file [default: .jit/server.log]
        #[arg(long)]
        log: Option<String>,

        /// Directory containing built web UI static files (auto-detected if omitted)
        #[arg(long)]
        web_dir: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Addressable structured item subcommands.
#[derive(Subcommand)]
pub enum ItemCommands {
    /// List addressable items across the repository
    ///
    /// Examples:
    ///   jit item list                       # All items, every kind
    ///   jit item list --kind requirement    # Only requirement items
    ///   jit item list --json                # Machine-readable
    ///
    /// JSON output uses the list envelope `{"count": N, "items": [...]}`.
    List {
        /// Filter to one item kind by name or config-declared alias
        /// (e.g. "requirement", or "inv" for the invariant kind)
        #[arg(long)]
        kind: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show / resolve a single item by its qualified id
    ///
    /// Accepts the uniform kind-segmented address (`@[<project>]/<kind>/<self-id>`
    /// for a project item — the optional `<project>` names a jit project and only
    /// the local project's name resolves — `@/issue/<short-id>/<kind>/<self-id>` for
    /// an issue item) and the `<short-id>/<self-id>` input sugar (the kind is
    /// inferred from the self-id's shape). An issue reference may be a full id,
    /// short id, or unique prefix. The kind segment accepts a config-declared alias
    /// (e.g. `@/inv/<self-id>` for the invariant kind); output uses the registry name.
    ///
    /// Examples:
    ///   jit item show @/issue/56ab0224/requirement/REQ-01
    ///   jit item show @/invariant/dag-acyclic
    ///   jit item show @/inv/dag-acyclic             # alias of @/invariant/dag-acyclic
    ///   jit item show 56ab0224/REQ-01 --json
    Show {
        /// Qualified id of the item (`@/<kind>/<self-id>`,
        /// `@/issue/<short-id>/<kind>/<self-id>`, or `<short-id>/<self-id>` sugar)
        qualified_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Resolve a qualified id to its item (alias of `show`)
    ///
    /// Provided as a distinct verb for orchestrators that think in terms of
    /// "resolve this qualified id"; behaves identically to `jit item show`.
    Resolve {
        /// Qualified id of the item (`@/<kind>/<self-id>`,
        /// `@/issue/<short-id>/<kind>/<self-id>`, or `<short-id>/<self-id>` sugar)
        qualified_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search items by self-id or text
    ///
    /// Examples:
    ///   jit item search atomic
    ///   jit item search "" --kind requirement   # Empty query: filter by kind
    ///
    /// JSON output uses the list envelope `{"count": N, "items": [...]}`.
    Search {
        /// Search query (matches self-id, qualified id, and item text). Empty
        /// matches all, so `--kind` can be used alone.
        #[arg(default_value = "")]
        query: String,

        /// Filter to one item kind by name or config-declared alias
        #[arg(long)]
        kind: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Project-invariant subcommands.
///
/// `check` runs the enforcement-drift check between the registry and the declared
/// rules/gates, reporting the sole declared-but-unenforced direction (an invariant
/// whose `enforced-by` resolves to no loadable rule/gate).
#[derive(Subcommand)]
pub enum InvariantCommands {
    /// Check enforcement drift between invariants and declared rules/gates
    ///
    /// Reports the declared-but-unenforced direction: an invariant whose
    /// `enforced-by` names a missing/unloadable rule or gate. This is a
    /// declaration-consistency check (bindings are never executed). Exits
    /// non-zero when any drift is present.
    ///
    /// Examples:
    ///   jit invariant check           # Human-readable drift report
    ///   jit invariant check --json    # Machine-readable result
    ///
    /// JSON output uses the list envelope `{"count": N, "findings": [...]}`.
    Check {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Documentation-projection subcommands.
///
/// `render` projects each `[projection.<name>]` config table (an addressable item
/// kind or kinds) into its documentation target: a markdown-first kind renders its
/// `- **{self-id}** — {text}` rows, the built-in invariant and rule+gate registries
/// render their rich views. Targets, modes, and region delimiters come only from
/// config; delimiters default to `<!-- jit:<name>:begin/end -->`.
#[derive(Subcommand)]
pub enum ProjectCommands {
    /// Render declared documentation projections into their configured targets
    ///
    /// Writes every `[projection.*]` table, or a single `--name`d one, atomically.
    /// In region mode only the delimited block is rewritten; everything outside is
    /// byte-preserved. A missing target/source, an unknown kind, or an absent
    /// region marker is a typed error and nothing is written.
    ///
    /// Examples:
    ///   jit project render                 # Render every declared projection
    ///   jit project render --name charter  # Render only the `charter` projection
    ///   jit project render --json          # Machine-readable result
    Render {
        /// Render only the projection with this `[projection.<name>]` name
        #[arg(long)]
        name: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum IssueCommands {
    /// Create a new issue.
    ///
    /// The title is the subject of the verb and may be given positionally
    /// (`jit issue create "Title"`) or via the `-t`/`--title` flag.
    /// Exactly one form is required; providing both is an error.
    ///
    /// To verify what was recorded, see `jit events query --issue-id <id>` or
    /// `jit events tail`.
    ///
    /// Examples:
    ///   jit issue create "Fix login bug"
    ///   jit issue create "Fix login bug" --type bug --priority high
    ///   jit issue create --title "Fix login bug" --gate tests
    Create {
        /// Issue title (positional form; alternative to `-t`/`--title`).
        #[arg(
            value_name = "TITLE",
            required_unless_present = "title",
            conflicts_with = "title"
        )]
        positional_title: Option<String>,

        /// Issue title (flag form; alternative to the positional argument).
        #[arg(short = 't', long, conflicts_with = "positional_title")]
        title: Option<String>,

        /// Initial description (body) of the issue, stored verbatim. Defaults
        /// to an empty string when omitted.
        #[arg(short = 'd', long = "description", default_value = "")]
        description: String,

        #[arg(short, long, default_value = "normal")]
        priority: String,

        /// Issue type (e.g. `task`, `story`, `epic`). Must be declared in
        /// `[type_hierarchy]` in config.toml. Writes a `type:<kind>` label and
        /// overrides any type label already carried by the issue. Long-only
        /// (`-t` is reserved for `--title`).
        #[arg(long = "type", value_name = "KIND")]
        issue_type: Option<String>,

        /// Gate keys from registry to require (e.g., 'tests', 'code-review').
        /// Gates are quality checkpoints that must pass before issue completion.
        /// Use comma-separated (--gate tests,clippy) or multiple flags (--gate tests --gate clippy).
        /// Gates must be defined in registry first with 'jit gate define'.
        #[arg(short, long, value_delimiter = ',')]
        gate: Vec<String>,

        /// Labels (format: namespace:value, repeatable)
        #[arg(short, long, value_delimiter = ',')]
        label: Vec<String>,

        /// Content format of the description body, selecting the parser used to
        /// extract sections during validation (markdown, html, or xml). When
        /// omitted the repo default ([validation].content_format) applies, with a
        /// Markdown fallback. html/xml require the matching cargo feature.
        #[arg(long, value_name = "FORMAT")]
        content_format: Option<String>,

        /// Bypass validation warnings
        #[arg(long)]
        force: bool,

        /// Explicitly allow orphaned leaf issues (tasks without parent labels)
        #[arg(long)]
        orphan: bool,

        #[arg(long)]
        json: bool,
    },

    /// Batch-create issues with dependency wiring from a JSON file.
    ///
    /// Reads a JSON array of issue definitions that reference each other by a
    /// symbolic `key`, then FULLY pre-validates the whole file before any write
    /// (duplicate/unknown keys, unknown `depends_on` references, cycles,
    /// type/label/gate validity, priority parse). On any validation failure it
    /// creates ZERO issues and exits 2, listing every offending entry. On success
    /// it creates all issues and dependency edges and returns a `{key: id}` map.
    ///
    /// The write phase is NOT atomic: if a write fails partway, the partial
    /// `{key: id}` map and the failing step are reported; recovery is manual.
    ///
    /// Schema (array of objects):
    ///   key         (required) symbolic key, unique within the file
    ///   title       (required) issue title
    ///   description (optional, default "")
    ///   type        (optional, default = project default type)
    ///   priority    (optional, default "normal")
    ///   labels      (optional, array of "namespace:value")
    ///   gates       (optional, array of registered gate keys)
    ///   depends_on  (optional, array of symbolic keys in the same file)
    ///
    /// Example file:
    ///   [
    ///     { "key": "spec", "title": "Write spec", "type": "story" },
    ///     { "key": "impl", "title": "Implement", "type": "task",
    ///       "depends_on": ["spec"] }
    ///   ]
    ///
    /// Example: jit issue batch-create --from-json plan.json --json
    BatchCreate {
        /// Path to the JSON file containing the array of issue definitions.
        #[arg(long)]
        from_json: std::path::PathBuf,

        #[arg(long)]
        json: bool,
    },

    /// Search issues by text query and/or filters.
    ///
    /// The positional query searches title, description, and ID. It is optional
    /// whenever at least one filter flag is given, in which case the search
    /// matches all issues and the filters narrow the result. `--label` is
    /// repeatable and ANDed: an issue must carry EVERY requested label.
    ///
    /// Examples:
    ///   jit issue search auth                       # text query only
    ///   jit issue search --label type:epic          # label filter, no query
    ///   jit issue search --label a:b --label c:d     # must carry BOTH labels
    ///   jit issue search task --state ready          # query + filter
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    Search {
        /// Search query (searches title, description, and ID). Optional when any
        /// filter flag is provided.
        query: Option<String>,

        #[arg(short, long)]
        state: Option<String>,

        #[arg(short, long)]
        assignee: Option<String>,

        #[arg(short, long)]
        priority: Option<String>,

        /// Filter by label (format: namespace:value). Repeatable; labels are
        /// ANDed, so an issue must carry every label given.
        #[arg(short = 'l', long = "label")]
        labels: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Show issue details. Use `--summary` for a compact response without the
    /// description field.
    ///
    /// JSON top-level fields: id, short_id, title, description, state,
    /// priority, assignee, dependencies, unmet_dependencies, gates, context,
    /// documents, labels, content_format, created_at, updated_at (plus
    /// first_ready_at/claimed_at/done_at once set).
    ///
    /// For a compact one-line status (state, per-gate status, unmet
    /// dependencies) without the description, see `jit issue status`. For
    /// per-gate readiness/history, see `jit gate status-all` (every required
    /// gate) or `jit gate status <id> <gate>` (one gate; add --all for its
    /// run history).
    ///
    /// Field projection (single id only):
    ///   --field <name>   print one top-level field as plain text (arrays/objects
    ///                    fall back to compact JSON for that field)
    ///   --fields a,b,c   print those fields as one compact JSON object
    ///
    /// Pass two or more ids with `--json` to get the list envelope
    /// `{"count": N, "issues": [...]}` with issue objects in argument order.
    /// Projection flags (`--field`/`--fields`) require exactly one id.
    ///
    /// Examples:
    ///   jit issue show abc123 --field state          # -> ready
    ///   jit issue show abc123 --fields state,title    # -> {"state":"ready",...}
    ///   jit issue show abc123 def456 --json           # -> {"count":2,"issues":[...]}
    Show {
        /// Issue id(s). Two or more ids with `--json` produce the
        /// `{"count": N, "issues": [...]}` list envelope.
        #[arg(required = true)]
        ids: Vec<String>,

        /// Return a compact response (id, short_id, title, state, priority,
        /// labels, gates) without the description or enriched dependencies.
        /// Affects --json output only.
        #[arg(long)]
        summary: bool,

        /// Print a single top-level field as plain text. String/scalar fields
        /// print raw; array/object fields fall back to compact JSON. Requires a
        /// single id.
        #[arg(long, value_name = "NAME", conflicts_with = "fields")]
        field: Option<String>,

        /// Print the named comma-separated top-level fields as one compact JSON
        /// object (`{"a":...,"b":...}`). Requires a single id.
        #[arg(long, value_name = "A,B,C", value_delimiter = ',')]
        fields: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print a compact "where does this issue stand" status: state, per-gate
    /// status, and the still-unmet dependencies — one line per issue.
    ///
    /// This is the orchestration one-liner agents otherwise rebuild by piping
    /// `issue show` JSON through jq. A dependency is *unmet* when it is not
    /// effectively terminal — `Done`, `Rejected`, or `Archived` from one of
    /// those (the same readiness test `query available` applies); the section
    /// reads `none` when nothing is blocking.
    ///
    /// Text form (default), one line per id:
    ///   <short_id> [<state>] gates: <key>=<status>,... unmet: <short_id>,... title: <title>
    ///
    /// `--json` emits the compact object
    /// `{short_id, state, gates:[{key,status}], unmet_dependencies:[short_id,...], title}`;
    /// with two or more ids it is wrapped in the list envelope
    /// `{"count": N, "issues": [...]}` in argument order.
    ///
    /// Examples:
    ///   jit issue status abc123                 # -> abc12345 [ready] gates: ... unmet: none title: ...
    ///   jit issue status abc123 def456 --json    # -> {"count":2,"issues":[...]}
    Status {
        /// Issue id(s). One object/line per id, in argument order.
        #[arg(required = true)]
        ids: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// List a container's direct children — its immediate dependencies (depth 1)
    /// — each rendered exactly like `issue status`.
    ///
    /// Containment follows the dependency DAG: a container's children are the
    /// issues it directly depends on (membership labels are advisory grouping,
    /// not consulted here). A non-container issue simply has no dependencies and
    /// lists nothing. For a deep rollup use `jit graph deps <id> --depth`.
    ///
    /// Text form is one `issue status` line per child (state, per-gate status,
    /// unmet deps), in ascending short-id order; an empty container prints
    /// nothing.
    ///
    /// `--json` emits `{container: {short_id, title, state}, count, issues: [...]}`
    /// where `issues` is one compact status object per child and `count` is their
    /// number.
    ///
    /// Examples:
    ///   jit issue children epic123
    ///   jit issue children epic123 --json
    Children {
        /// Container id whose direct children (dependencies) to list.
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Summarize a container's direct children (depth 1) as counts by state plus
    /// a done/total delivery rollup.
    ///
    /// Membership follows the dependency DAG (direct dependencies are the
    /// children; labels are advisory and not consulted). `by state` lists every
    /// lifecycle state, zero-count states included. `done` and `rejected` are
    /// counted distinctly — a rejected child is terminal but not delivered — and,
    /// because `archived` is terminality-preserving, a child archived from
    /// `done`/`rejected` counts toward that origin; `open` is every child that is
    /// not effectively terminal; the `done/total` ratio and percent measure
    /// delivery. For a deep rollup use `jit graph deps <id> --depth`; to
    /// aggregate a label bucket use `jit query count --by state --label ns:v`.
    ///
    /// `--json` emits `{container: {short_id, title, state}, count, by_state:
    /// [{state, count}], total, done, rejected, open, percent}` (`count` is the
    /// number of state buckets).
    ///
    /// Examples:
    ///   jit issue progress epic123
    ///   jit issue progress epic123 --json
    Progress {
        /// Container id whose direct children to aggregate.
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Update an issue or multiple issues. Returns a lightweight confirmation
    /// (id, short_id, state, updated_at); run `jit issue show` to fetch the
    /// full updated body.
    ///
    /// To verify what was recorded, see `jit events query --issue-id <id>` or
    /// `jit events tail`.
    ///
    /// Description flags — exactly one may be given (they are all mutually
    /// exclusive), and each has a replace form and an append form:
    ///   --description TEXT               replace with TEXT
    ///   --description-file PATH          replace with the contents of PATH
    ///   --append-description TEXT        append TEXT to the existing description
    ///   --append-description-file PATH   append the contents of PATH
    ///
    /// The `-file` forms accept `-` for PATH to read from stdin instead, and
    /// exist so large descriptions don't have to fit in argv or survive shell
    /// quoting. File/stdin content is used verbatim (including any trailing
    /// newline). Appending separates the existing and new text with exactly
    /// one blank line; appending to an empty/absent description produces just
    /// the new text, with no leading blank line.
    ///
    /// Examples:
    ///   jit issue update abc123 --description "New text"
    ///   jit issue update abc123 --append-description "Follow-up note"
    ///   jit issue update abc123 --append-description-file notes.txt
    ///   cat notes.txt | jit issue update abc123 --append-description-file -
    Update {
        /// Issue ID (for single issue mode, mutually exclusive with --filter)
        id: Option<String>,

        /// Boolean query filter (for batch mode, mutually exclusive with ID)
        #[arg(long, conflicts_with = "id")]
        filter: Option<String>,

        #[arg(short, long)]
        title: Option<String>,

        /// Replace the entire description with TEXT. Mutually exclusive with
        /// the other description flags; see `--append-description` to add
        /// text instead of replacing it.
        #[arg(
            short = 'd',
            long = "description",
            value_name = "TEXT",
            conflicts_with_all = ["description_file", "append_description", "append_description_file"]
        )]
        description: Option<String>,

        /// Replace the entire description with the contents of PATH (`-` for
        /// stdin), used verbatim. Avoids argv length limits and shell-quoting
        /// hazards for large text. Mutually exclusive with the other
        /// description flags.
        #[arg(
            long = "description-file",
            value_name = "PATH",
            conflicts_with_all = ["append_description", "append_description_file"]
        )]
        description_file: Option<String>,

        /// Append TEXT to the end of the existing description, separated by
        /// exactly one blank line (no leading blank line when the description
        /// is empty). Mutually exclusive with the other description flags.
        #[arg(
            long = "append-description",
            value_name = "TEXT",
            conflicts_with = "append_description_file"
        )]
        append_description: Option<String>,

        /// Append the contents of PATH (`-` for stdin), used verbatim, to the
        /// end of the existing description, separated by exactly one blank
        /// line. Mutually exclusive with the other description flags.
        #[arg(long = "append-description-file", value_name = "PATH")]
        append_description_file: Option<String>,

        #[arg(short, long)]
        priority: Option<String>,

        #[arg(short, long)]
        state: Option<String>,

        /// Issue type (e.g. `task`, `story`, `epic`). Must be declared in
        /// `[type_hierarchy]` in config.toml. Replaces the existing `type:*`
        /// label with `type:<kind>`. Long-only (`-t` is reserved for `--title`).
        #[arg(long = "type", value_name = "KIND")]
        issue_type: Option<String>,

        /// Add label(s) (format: namespace:value, repeatable)
        #[arg(short, long, value_delimiter = ',', visible_alias = "add-label")]
        label: Vec<String>,

        /// Remove label(s) (repeatable)
        #[arg(long, value_delimiter = ',')]
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

        /// Set the content format of the description body (markdown, html, or
        /// xml), selecting the parser used to extract sections during validation.
        /// html/xml require the matching cargo feature. Pass `inherit` (or
        /// `default`) to clear a previous override back to the repo default.
        #[arg(long, value_name = "FORMAT")]
        content_format: Option<String>,

        /// Bypass blocking (`enforce`) validation rules; the bypass is logged
        #[arg(long)]
        force: bool,

        #[arg(long)]
        json: bool,
    },

    /// Delete an issue
    Delete {
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Hidden stub: not a real command. `issue rm` is not the canonical
    /// spelling in this group (that's `issue delete`); this variant exists
    /// only to fail fast with a hint instead of clap's generic "unrecognized
    /// subcommand" error. See `verb_hint_error` in `main.rs`.
    #[command(hide = true, trailing_var_arg = true)]
    Rm {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Rm` above. `issue remove` is not a command.
    #[command(hide = true, trailing_var_arg = true)]
    Remove {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Rm` above. `issue complete` is not a command; use
    /// `issue update --state done`.
    #[command(hide = true, trailing_var_arg = true)]
    Complete {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Rm` above. `issue edit` is not a command; use
    /// `issue update`.
    #[command(hide = true, trailing_var_arg = true)]
    Edit {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Assign issue to someone
    ///
    /// Assignment bookkeeping only, not a lease — for an exclusive, time-boxed
    /// work lease, see `jit claim acquire` / `jit claim release`.
    Assign {
        /// Issue ID
        id: String,

        /// Assignee (format: type:identifier, e.g., agent:worker-1)
        assignee: String,

        #[arg(long)]
        json: bool,
    },

    /// Claim an issue: assign it and promote it to in_progress
    ///
    /// Claiming an unassigned issue assigns it to `assignee`; a Ready issue is
    /// promoted to in_progress. Re-claiming as the current assignee succeeds
    /// and promotes it the same way. Claiming an issue already assigned to
    /// someone else fails, naming the current holder.
    ///
    /// Assignment bookkeeping only, not a lease — for an exclusive, time-boxed
    /// work lease, see `jit claim acquire` / `jit claim release`.
    Claim {
        /// Issue ID
        id: String,

        /// Assignee (format: type:identifier, e.g., agent:worker-1)
        assignee: String,

        /// Assign the issue without transitioning its state to in_progress
        #[arg(long)]
        assign_only: bool,

        #[arg(long)]
        json: bool,
    },

    /// Unassign an issue
    ///
    /// Assignment bookkeeping only, not a lease — for an exclusive, time-boxed
    /// work lease, see `jit claim acquire` / `jit claim release`.
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
    ///
    /// Assignment bookkeeping only, not a lease — for an exclusive, time-boxed
    /// work lease, see `jit claim acquire` / `jit claim release`.
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

        /// Output JSON format
        #[arg(long)]
        json: bool,
    },

    /// List issues (equivalent to `jit query all`)
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    List {
        /// Filter by state
        #[arg(short = 's', long)]
        state: Option<String>,

        /// Filter by assignee (format: type:identifier)
        #[arg(short = 'a', long)]
        assignee: Option<String>,

        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum DepCommands {
    /// Add a work dependency: FROM is blocked until TO is effectively terminal
    /// (done, rejected, or archived from one of those)
    ///
    /// FROM and TO can be any issues. Work flows from TO (upstream) into FROM (downstream).
    /// Dependencies are orthogonal to labels - issues don't need matching labels to depend on each other.
    ///
    /// Examples:
    ///   jit dep add epic-123 task-456           # Single dependency
    ///   jit dep add epic-123 task-1 task-2 task-3  # Multiple dependencies
    Add {
        /// The blocked issue (depends on the others)
        from_id: String,

        /// The blocking issue(s) that must become effectively terminal first
        #[arg(required = true)]
        to_ids: Vec<String>,

        /// Drop any edge the add would make transitively redundant, in the same
        /// operation, instead of rejecting. Leaves the graph transitively reduced.
        #[arg(long)]
        reduce: bool,

        #[arg(long)]
        json: bool,
    },

    /// Remove a dependency
    ///
    /// Examples:
    ///   jit dep rm epic-123 task-456            # Single dependency
    ///   jit dep rm epic-123 task-1 task-2       # Multiple dependencies
    Rm {
        /// Issue to remove dependencies from
        from_id: String,

        /// Dependencies to remove (the blocking issues)
        #[arg(required = true)]
        to_ids: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Hidden stub: not a real command. The canonical spelling in this group
    /// is `dep rm`; this variant exists only to fail fast with a hint instead
    /// of clap's generic "unrecognized subcommand" error. See
    /// `verb_hint_error` in `main.rs`.
    #[command(hide = true, trailing_var_arg = true)]
    Remove {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Remove` above. `dep delete` is not a command.
    #[command(hide = true, trailing_var_arg = true)]
    Delete {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// Gate commands, partitioned along the grammar standard's three
/// responsibilities (see docs/reference/cli-command-grammar.md):
///
/// * Configuration — shapes what gates exist and which issues require them:
///   `define`, `update`, `remove`, `add`, and `preset *`. These MUTATE the
///   registry or an issue's requirements.
/// * Execution — produces a verdict and MUTATES per-issue gate state (and may
///   advance issue state): `evaluate` (alias `eval`), `evaluate-all`, `fail`.
///   `evaluate` runs an auto gate's checker or records a manual gate's
///   attestation; the verdict may be pass or fail.
/// * Inspection — reports definitions or recorded state with NO side effects:
///   `list`, `show`, `status`, `status-all`. `status-all` additionally exits
///   nonzero (4) unless every required gate has passed.
///
/// # Configuration
/// jit gate define code-review --title "Code Review" --description "Human review"
/// jit gate add abc123 code-review            # attach a registered gate to an issue
/// jit gate preset apply ci abc123            # attach a project-defined preset bundle
///
/// # Execution (mutating — produces a verdict)
/// jit gate evaluate abc123 code-review       # run/attest, record a verdict
/// jit gate evaluate-all abc123               # evaluate every required gate
///
/// # Inspection (non-mutating — reports state)
/// jit gate list                              # registered gate definitions
/// jit gate status abc123 code-review         # last recorded run for one gate
/// jit gate status-all abc123                 # readiness of every required gate
#[derive(Subcommand)]
pub enum GateCommands {
    // ===== Configuration: define what gates exist and which issues require them =====
    /// Define a new gate in the registry
    ///
    /// Mode resolution when `--mode` is not given: a gate defined with
    /// `--checker-command` becomes automated; otherwise it defaults to manual.
    /// An explicit `--mode manual` combined with `--checker-command` is a
    /// usage error (exit 2) — a manual gate cannot carry a checker, so the
    /// conflict is rejected rather than silently dropping the checker.
    Define {
        /// Unique gate key
        key: String,

        /// Human-readable title
        #[arg(short, long)]
        title: String,

        /// Description of what this gate checks
        #[arg(short = 'd', long)]
        description: String,

        /// Gate stage: precheck or postcheck (long-only; `-s` is reserved for `--state`)
        #[arg(long, value_enum, default_value_t = crate::declarations::GateStage::Postcheck)]
        stage: crate::declarations::GateStage,

        /// Gate mode: manual or auto. Defaults to auto when --checker-command
        /// is given, manual otherwise. Explicit `--mode manual` with
        /// --checker-command is a usage error (exit 2).
        #[arg(short, long, value_enum)]
        mode: Option<crate::declarations::GateMode>,

        /// Convenience flag for `--mode auto`: define the gate as automated.
        /// When set it overrides `--mode`.
        #[arg(long)]
        auto: bool,

        /// Optional example integration snippet recorded with the definition
        #[arg(long)]
        example: Option<String>,

        /// Command to execute for automated gates
        #[arg(long)]
        checker_command: Option<String>,

        /// Timeout in seconds for checker command
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// Working directory for checker (relative to repo root)
        #[arg(long)]
        working_dir: Option<String>,

        /// Pass structured context (issue data, run history, prompt) to checker
        #[arg(long)]
        pass_context: bool,

        /// Inline prompt/instructions for the checker process
        #[arg(long)]
        prompt: Option<String>,

        /// Path to a prompt file (relative to repo root), read at check time
        #[arg(long)]
        prompt_file: Option<String>,

        /// Environment variables to pass to the checker (repeatable, format: KEY=VALUE)
        #[arg(long)]
        env: Vec<String>,

        /// Execution priority (lower number runs first, default: 100)
        #[arg(long, default_value = "100")]
        priority: u32,

        #[arg(long)]
        json: bool,
    },

    /// Update an existing gate definition in the registry
    ///
    /// Edits only the fields you pass; every other field keeps its current
    /// value. The gate KEY is the gate's identity and cannot be changed. This
    /// edits the registry definition only — per-issue gate status is untouched.
    ///
    /// Examples:
    ///   jit gate update tests --title "All Tests Pass"
    ///   jit gate update tests --timeout 600 --checker-command "cargo test"
    Update {
        /// Gate key to update (exact registry key)
        key: String,

        /// New human-readable title
        #[arg(short, long)]
        title: Option<String>,

        /// New description of what this gate checks
        #[arg(short = 'd', long)]
        description: Option<String>,

        /// New gate stage: precheck or postcheck (long-only; `-s` is reserved for `--state`)
        #[arg(long, value_enum)]
        stage: Option<crate::declarations::GateStage>,

        /// New gate mode: manual or auto
        #[arg(short, long, value_enum)]
        mode: Option<crate::declarations::GateMode>,

        /// Convenience flag for `--mode auto`: switch the gate to automated.
        /// When set it overrides `--mode`.
        #[arg(long)]
        auto: bool,

        /// New command to execute for automated gates
        #[arg(long)]
        checker_command: Option<String>,

        /// New timeout in seconds for the checker command
        #[arg(long)]
        timeout: Option<u64>,

        /// New working directory for the checker (relative to repo root)
        #[arg(long)]
        working_dir: Option<String>,

        /// Clear the checker's working directory (mutually exclusive with --working-dir)
        #[arg(long)]
        clear_working_dir: bool,

        /// Set whether structured context (issue data, run history, prompt) is
        /// passed to the checker: `--pass-context true` or `--pass-context false`
        #[arg(long)]
        pass_context: Option<bool>,

        /// New inline prompt/instructions for the checker process
        #[arg(long)]
        prompt: Option<String>,

        /// Clear the checker's inline prompt (mutually exclusive with --prompt)
        #[arg(long)]
        clear_prompt: bool,

        /// New path to a prompt file (relative to repo root), read at check time
        #[arg(long)]
        prompt_file: Option<String>,

        /// Clear the checker's prompt file (mutually exclusive with --prompt-file)
        #[arg(long)]
        clear_prompt_file: bool,

        /// Environment variables for the checker (repeatable, format: KEY=VALUE).
        /// When provided, replaces the gate's existing environment set.
        #[arg(long)]
        env: Vec<String>,

        /// Clear the checker's environment set (mutually exclusive with --env)
        #[arg(long)]
        clear_env: bool,

        /// New execution priority (lower number runs first)
        #[arg(long)]
        priority: Option<u32>,

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

    /// Hidden stub: not a real command. The canonical spelling in this group
    /// is `gate remove`; this variant exists only to fail fast with a hint
    /// instead of clap's generic "unrecognized subcommand" error. See
    /// `verb_hint_error` in `main.rs`.
    #[command(hide = true, trailing_var_arg = true)]
    Rm {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Rm` above. `gate delete` is not a command.
    #[command(hide = true, trailing_var_arg = true)]
    Delete {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
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
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Gate key(s) from registry (e.g., 'tests', 'code-review', 'clippy')
        /// Can specify multiple: jit gate add <issue> gate1 gate2 gate3
        #[arg(required = true)]
        gate_keys: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Gate preset management
    #[command(subcommand)]
    Preset(PresetCommands),

    // ===== Execution: produce a verdict and may advance issue state =====
    /// Evaluate a gate: run its checker (auto) or record attestation (manual)
    ///
    /// Mutating. Produces a verdict — which may legitimately be *fail*, so this
    /// is not an override. `eval` is a short alias.
    ///
    /// The gate key may be supplied as a positional argument or via `--gate`:
    ///
    ///   jit gate evaluate <ISSUE_ID> <GATE_KEY>
    ///   jit gate evaluate <ISSUE_ID> --gate <GATE_KEY>
    ///
    /// Exactly one of the two forms must be used; supplying both or neither is
    /// an error.
    ///
    /// A manual gate has no checker to run, so `--by <attestor>` is required:
    /// bare evaluate on a manual gate is a usage error rather than a silent,
    /// unattributed pass. An automated gate ignores `--by`; its verdict comes
    /// from the checker.
    ///
    /// Exit codes:
    ///   0  - pass (checker passed or manual attestation recorded)
    ///   2  - bad arguments (gate not required for this issue; a manual gate
    ///        evaluated without --by)
    ///   3  - issue not found
    ///   4  - checker failure (the checker ran and the verdict was fail)
    ///   10 - runner error (timeout, command-not-found, crash; infra failure)
    ///
    /// With --json, the response carries a `verdict` field: `pass` on success,
    /// `fail` on checker failure, `error` on runner error. Pre-verdict argument
    /// and lookup errors (codes 2 and 3) carry no `verdict` field.
    ///
    /// When an automated gate already passed at the current HEAD commit, its
    /// checker is skipped and JSON reports `already_passed: true`. Every manual
    /// evaluation records a fresh attestation. Use --force to re-run an automated
    /// checker unconditionally.
    #[command(visible_alias = "eval")]
    Evaluate {
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Gate key (positional form)
        gate_key: Option<String>,

        /// Gate key (flag form — alternative to the positional)
        #[arg(long = "gate")]
        gate_flag: Option<String>,

        /// Who is passing the gate. Required for a manual gate (bare evaluate
        /// on a manual gate is a usage error); ignored for an automated gate,
        /// whose verdict comes from the checker.
        #[arg(short, long)]
        by: Option<String>,

        /// Re-run the checker even if the gate already passed at the current HEAD
        #[arg(long)]
        force: bool,

        #[arg(long)]
        json: bool,
    },

    /// Evaluate all of an issue's required gates in one command, fail-fast
    ///
    /// Mutating. Runs each required gate in declaration order, stopping at the
    /// FIRST gate that does not pass and exiting with that gate's code from the
    /// `gate evaluate` taxonomy (0 pass / 2 bad-args / 3 not-found / 4
    /// checker-failed / 10 runner-error). Later gates are not attempted once one
    /// fails.
    ///
    /// Each required gate is passed via the same `gate evaluate` semantics: a
    /// manual gate requires `--by <attestor>`, applied to every manual gate in
    /// the set. If the set mixes manual and automated gates and `--by` is
    /// omitted, evaluation fails fast at the first manual gate reached in
    /// declaration order — the order gates appear on the issue's required
    /// list (auto gates before it still run and record their verdict; later
    /// gates are not attempted) — it never silently passes a manual gate.
    ///
    /// Automated gates inherit skip-if-passed-at-HEAD; every manual gate records
    /// a fresh attestation. Use --force to re-run every automated checker. An
    /// issue with no required gates succeeds (exit 0) with an empty result set.
    EvaluateAll {
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Who is passing the gates. Required for every manual gate in the
        /// required set (applied uniformly); ignored for automated gates.
        #[arg(short, long)]
        by: Option<String>,

        /// Re-run checkers even if gates already passed at the current HEAD
        #[arg(long)]
        force: bool,

        #[arg(long)]
        json: bool,
    },

    /// Mark a gate as failed
    Fail {
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Gate key
        gate_key: String,

        /// Who failed the gate (optional)
        #[arg(short, long)]
        by: Option<String>,

        #[arg(long)]
        json: bool,
    },

    // ===== Inspection: report definitions or run results, no side effects =====
    /// List all gate definitions
    ///
    /// JSON output uses the list envelope `{"count": N, "gates": [...]}`.
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

    /// Inspect gate run results (inspection only, non-mutating)
    ///
    /// `gate status` is the unified gate-run inspection surface. It only reports
    /// recorded state and never exits nonzero on a pending/failed gate (that
    /// readiness contract belongs to `gate status-all`). By default it shows
    /// the latest run for one gate:
    ///
    ///   jit gate status <ISSUE_ID> <GATE_KEY>
    ///   jit gate status <ISSUE_ID> --gate <GATE_KEY>
    ///
    /// Exactly one of the two gate-key forms must be used for the default
    /// latest-run view; supplying both or neither is an error.
    ///
    /// History view (`--all` / `--limit <N>`) lists prior runs newest-first.
    /// Here the gate key is OPTIONAL and acts as a filter (positional or
    /// `--gate`); `--status <passed|failed|error|pending|skipped>` filters by
    /// outcome:
    ///
    ///   jit gate status <ISSUE_ID> --all
    ///   jit gate status <ISSUE_ID> --limit 5 --gate tests --status failed
    ///
    /// Flat view (`--stdout` / `--stderr` / `--tail <N>`) prints the latest
    /// run's stored report text verbatim, with no wrapping or decoration. The
    /// gate key is required here; `--tail <N>` keeps only the last N lines:
    ///
    ///   jit gate status <ISSUE_ID> <GATE_KEY> --stdout
    ///   jit gate status <ISSUE_ID> <GATE_KEY> --stderr --tail 40
    ///
    /// Findings view (`--findings`) prints only the latest run's structured
    /// findings and verdict, one finding per line in a stable greppable format.
    /// The gate key is required here:
    ///
    ///   jit gate status <ISSUE_ID> <GATE_KEY> --findings
    ///
    /// History, flat-output, and findings flags are mutually exclusive. Every
    /// view supports `--json`.
    ///
    /// The history view (`--all` / `--limit`) emits the list envelope
    /// `{"count": N, "results": [...]}`, where `count` is the number of runs
    /// returned after filtering.
    Status {
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Gate key (positional form). Required for the latest-run and flat
        /// views; an optional filter in the history view.
        gate_key: Option<String>,

        /// Gate key (flag form — alternative to the positional)
        #[arg(long = "gate")]
        gate_flag: Option<String>,

        /// History view: list all prior runs newest-first
        #[arg(long)]
        all: bool,

        /// History view: list the most recent N runs newest-first
        #[arg(long, value_name = "N")]
        limit: Option<usize>,

        /// History view: filter runs by status
        /// (passed|failed|error|pending|skipped)
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,

        /// Flat view: print the latest run's stdout verbatim
        #[arg(long)]
        stdout: bool,

        /// Flat view: print the latest run's stderr verbatim
        #[arg(long)]
        stderr: bool,

        /// Flat view: keep only the last N lines of the printed report text
        #[arg(long, value_name = "N")]
        tail: Option<usize>,

        /// Findings view: print only the latest run's structured findings and
        /// verdict, one finding per line. Requires a gate key.
        #[arg(long)]
        findings: bool,

        #[arg(long)]
        json: bool,
    },

    /// Report readiness of every required gate on an issue (strict exit)
    ///
    /// Inspection only — never mutates gate state. Considers EVERY required
    /// gate, automated and manual: a required gate is green only when its
    /// recorded status is `passed`; a pending (auto never run, or manual never
    /// attested) or failed gate is not green. Exits 0 only when all required
    /// gates are green, otherwise 4 (pending and failed share the one nonzero
    /// code; `--json` distinguishes them per gate). This readiness contract is
    /// a single behaviour with no flag.
    ///
    /// With `--json`, stdout/stderr are omitted from passing runs by default;
    /// pass `--full` to include them. Failing runs always include stdout/stderr.
    ///
    /// JSON output uses the list envelope `{"count": N, "gates": [...]}`,
    /// where `count` is the number of required gates (one `gates` entry
    /// each). The `results` / `not_run` / `total` / `passed` tallies remain
    /// alongside; `total` / `passed` are readiness counts, not the collection
    /// size.
    StatusAll {
        /// Issue ID (full UUID, 8-char short id, or unique prefix)
        id: String,

        /// Include stdout/stderr for every run in the --json response
        /// (default behaviour omits them for passing runs).
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum PresetCommands {
    /// List available gate presets
    ///
    /// JSON output uses the list envelope `{"count": N, "presets": [...]}`.
    List {
        #[arg(long)]
        json: bool,
    },

    /// Show preset details
    Show {
        /// Preset name
        name: String,

        #[arg(long)]
        json: bool,
    },

    /// Apply preset gates to issues
    ///
    /// Presets are pre-configured bundles of quality gates that can be quickly
    /// applied to one or more issues. Gates from the preset are added to the issue's
    /// required gates. Use 'jit gate preset list' to see available presets.
    ///
    /// Examples:
    ///   jit gate preset apply ci abc123                 # Single issue
    ///   jit gate preset apply ci abc123 def456          # Multiple issues
    ///   jit query all --json | jq -r '.issues[].id' | xargs jit gate preset apply ci  # From query
    ///   jit gate preset apply ci abc123 --except lint   # Skip specific gates
    Apply {
        /// Preset name
        name: String,

        /// Issue ID(s) - can specify multiple
        ids: Vec<String>,

        /// Override checker timeout (seconds)
        #[arg(long)]
        timeout: Option<u64>,

        /// Skip precheck gates
        #[arg(long, default_value_t = false)]
        no_precheck: bool,

        /// Skip postcheck gates
        #[arg(long, default_value_t = false)]
        no_postcheck: bool,

        /// Exclude specific gates (repeatable)
        #[arg(long, value_delimiter = ',')]
        except: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Create custom preset from issue gates
    ///
    /// Captures all gates currently required by an issue and saves them as a
    /// custom preset. The preset is stored in .jit/config/gate-presets/ and can
    /// be applied to other issues.
    ///
    /// Examples:
    ///   jit gate preset create abc123 my-workflow      # Create from issue
    ///   jit gate preset create abc123 team-standard    # Reusable preset
    Create {
        /// Issue ID to copy gates from
        from_issue: String,

        /// Preset name
        name: String,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum DocCommands {
    /// Add a document reference to an issue
    ///
    /// Idempotent on path: re-running this for a path already linked to the
    /// issue updates that reference in place instead of appending a
    /// duplicate. The commit pin always reflects this invocation — supplied
    /// `--commit` pins, omitted `--commit` records the reference unpinned
    /// (current version), re-pointing a stale pin. Omitted `--label`/
    /// `--doc-type` flags leave the existing value untouched; supplied ones
    /// overwrite it.
    ///
    /// To verify what was recorded, see `jit events query --issue-id <id>` or
    /// `jit events tail`.
    Add {
        /// Issue ID
        id: String,

        /// Path to document relative to repository root
        path: String,

        /// Git commit to pin the reference to; omitted, the reference is
        /// stored unpinned (reads as the current version)
        #[arg(short, long)]
        commit: Option<String>,

        /// Human-readable label (alias: --title)
        #[arg(short, long, visible_alias = "title")]
        label: Option<String>,

        /// Document type (e.g., design, implementation, notes)
        #[arg(long)]
        doc_type: Option<String>,

        /// Skip scanning document for assets (default: false)
        #[arg(long, default_value_t = false)]
        skip_scan: bool,

        #[arg(long)]
        json: bool,
    },

    /// List document references for an issue
    ///
    /// JSON output uses the list envelope `{"count": N, "documents": [...]}`.
    List {
        /// Issue ID
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Remove a document reference from an issue
    ///
    /// To verify what was recorded, see `jit events query --issue-id <id>` or
    /// `jit events tail`.
    Remove {
        /// Issue ID
        id: String,

        /// Path to document to remove
        path: String,

        #[arg(long)]
        json: bool,
    },

    /// Hidden stub: not a real command. The canonical spelling in this group
    /// is `doc remove`; this variant exists only to fail fast with a hint
    /// instead of clap's generic "unrecognized subcommand" error. See
    /// `verb_hint_error` in `main.rs`.
    #[command(hide = true, trailing_var_arg = true)]
    Rm {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Rm` above. `doc delete` is not a command.
    #[command(hide = true, trailing_var_arg = true)]
    Delete {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
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

        #[arg(long)]
        json: bool,
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

        #[arg(long)]
        json: bool,
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
}

/// Dependency-aware archive planning commands; mutation requires `--execute`.
#[derive(Subcommand)]
pub enum ArchiveCommands {
    /// Fully evaluate every effectively terminal configured non-leaf container
    /// (Done, Rejected, or Archived from one of those) without mutation
    Candidates {
        /// Output schema version 1 with count and complete candidate plans
        #[arg(long)]
        json: bool,
    },

    /// Preview archival of one repository-relative document and its supported bundle
    Document {
        /// Repository-relative document or opaque artifact path
        path: String,

        /// Recompute under the repository write guard and execute the eligible plan
        #[arg(long)]
        execute: bool,

        /// Output the binding schema-version-1 plan as JSON
        #[arg(long)]
        json: bool,
    },

    /// Preview archival of all issue-linked artifacts in a container subtree
    Container {
        /// Container issue ID or unique prefix
        id: String,

        /// Recompute under the repository write guard and execute the eligible plan
        #[arg(long)]
        execute: bool,

        /// Output the binding schema-version-1 plan as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum AssetCommands {
    /// List assets for a document
    ///
    /// JSON output uses the list envelope `{"count": N, "assets": [...]}`.
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
    /// Show what an issue depends on (upstream dependencies)
    ///
    /// Shows the issues that must become effectively terminal (done, rejected,
    /// or archived from one of those) before this issue can proceed.
    /// By default shows immediate dependencies only (depth 1).
    ///
    /// Examples:
    ///   jit graph deps epic-123              # immediate dependencies (default)
    ///   jit graph deps epic-123 --depth 2    # two levels deep
    ///   jit graph deps epic-123 --depth 0    # all transitive (unlimited, opt-in)
    ///
    /// "Dependencies" = what this issue needs (upstream in work flow).
    /// "Dependents" = what needs this issue (downstream in work flow).
    ///
    /// JSON output uses the list envelope `{"count": N, "nodes": [...]}`; `count`
    /// is the number of top-level `nodes`, distinct from `summary.total` (the
    /// unique-dependency count across the whole tree). The node collection was
    /// renamed from `tree` to `nodes` when the envelope landed.
    #[command(alias = "dependencies")]
    Deps {
        /// Issue ID
        id: String,

        /// Depth of dependency traversal (1 = immediate, 0 = unlimited)
        #[arg(long, default_value = "1")]
        depth: u32,

        #[arg(long)]
        json: bool,
    },

    /// Show reverse dependencies (issues that depend on this one)
    ///
    /// Symmetric counterpart to `deps`: where `deps` shows what this issue needs,
    /// `rdeps` shows what needs this issue.
    /// By default shows immediate dependents only (depth 1).
    ///
    /// Examples:
    ///   jit graph rdeps task-456              # immediate dependents (default)
    ///   jit graph rdeps task-456 --depth 2    # two levels deep
    ///   jit graph rdeps task-456 --depth 0    # all transitive (unlimited, opt-in)
    ///
    ///   Shows: epic-123, milestone-789 (they depend on this task)
    ///
    /// Note: This shows dependency relationships, not label hierarchy.
    ///
    /// JSON output uses the list envelope `{"count": N, "dependents": [...]}`.
    #[command(alias = "downstream")]
    Rdeps {
        /// Issue ID
        id: String,

        /// Depth of traversal (1 = immediate, 0 = unlimited)
        #[arg(long, default_value = "1")]
        depth: u32,

        #[arg(long)]
        json: bool,
    },

    /// Show root issues (no dependencies)
    ///
    /// JSON output uses the list envelope `{"count": N, "roots": [...]}`.
    Roots {
        #[arg(long)]
        json: bool,
    },

    /// Show the DAG-resolved containment hierarchy (parent/children per node)
    ///
    /// Resolution treats the dependency DAG as authoritative (membership labels
    /// are advisory and not consulted): a container depends on the work it
    /// contains, so each node's parent is the nearest dominating container, its
    /// children are the inverse, its cluster is the strategic root container, and
    /// its rank is the longest dependency-path depth.
    ///
    /// With no id the whole repository is shown; with a root id the view is that
    /// node plus its transitive containment subtree.
    ///
    /// JSON output uses the list envelope `{"count": N, "nodes": [...]}`.
    Tree {
        /// Optional root id to scope the tree to (its containment subtree).
        root: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Export dependency graph in various formats
    Export {
        /// Output format (dot, mermaid, json, batch)
        ///
        /// `batch` emits the `jit issue batch-create` input schema (a JSON
        /// array of issue definitions) — the structural inverse of batch
        /// creation. It strips lifecycle fields and identity-bound labels and
        /// excludes template bracket nodes; see `--scope` for capturing one
        /// container's subtree.
        #[arg(short, long, value_enum)]
        format: Option<crate::commands::GraphExportFormat>,

        /// Emit JSON to stdout — sugar for `--format json`.
        ///
        /// Equivalent to `--format json`; combining it with an explicit
        /// `--format dot`/`--format mermaid`/`--format batch` is a usage error.
        /// Composes with `--full`.
        #[arg(long)]
        json: bool,

        /// Emit complete issue records for each node (JSON only).
        ///
        /// Only valid with `--format json` (or `--json`); combining it with
        /// `dot`/`mermaid`/`batch` is a usage error. Without this flag the JSON
        /// output keeps the lean summary node shape.
        #[arg(long)]
        full: bool,

        /// Restrict the export to a container's DAG-authoritative containment
        /// membership (the container and its subtree).
        ///
        /// Composes with every format: `dot`/`mermaid`/`json` list only the
        /// in-scope nodes; `batch` additionally excludes bracket nodes and
        /// reports edges that cross the scope boundary. Omitted, the whole graph
        /// is exported.
        #[arg(long)]
        scope: Option<String>,

        /// Output file (optional - prints to stdout if omitted)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum MigrateCommands {
    /// Backfill lifecycle timestamps from the event log
    ///
    /// Fills `first_ready_at`, `claimed_at`, and `done_at` on issues that
    /// predate the transition-time write points, deriving each value from
    /// `.jit/events.jsonl` (first Ready transition, first claim, first Done
    /// transition). Only still-absent fields are filled; existing stamps are
    /// never overwritten. One-time and idempotent — a second run over an
    /// already-migrated repository writes nothing.
    ///
    /// Issues with no relevant events (predating event coverage) keep their
    /// fields unset.
    ///
    /// JSON output: `{"issues_scanned": N, "issues_updated": M}`.
    LifecycleTimestamps {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum EventCommands {
    /// Tail recent events
    ///
    /// JSON output uses the list envelope `{"count": N, "events": [...]}`.
    Tail {
        #[arg(short, long, default_value = "10")]
        n: usize,

        #[arg(long)]
        json: bool,
    },

    /// Query events by type or issue
    ///
    /// JSON output uses the list envelope `{"count": N, "events": [...]}`.
    Query {
        #[arg(short, long)]
        event_type: Option<String>,

        #[arg(short, long)]
        issue_id: Option<String>,

        #[arg(short, long, default_value = "50")]
        limit: usize,

        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum QueryCommands {
    /// Query all issues with optional filters
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
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

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query available issues (unassigned, state=ready, unblocked)
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    #[command(visible_alias = "ready")]
    Available {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query blocked issues with reasons
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    Blocked {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query strategic issues (those with labels from strategic namespaces)
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    Strategic {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Query closed issues: those effectively terminal (Done, Rejected, or
    /// Archived from one of those)
    ///
    /// JSON output uses the list envelope `{"count": N, "issues": [...]}`.
    Closed {
        /// Filter by priority
        #[arg(short = 'p', long)]
        priority: Option<String>,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        /// Return full issue objects instead of minimal summaries
        #[arg(long)]
        full: bool,

        #[arg(long)]
        json: bool,
    },

    /// Aggregate issues into counts by a dimension over a label bucket (or the
    /// whole repo), with a done/total delivery rollup.
    ///
    /// The bucket is every issue matching all `--label` patterns (AND-combined;
    /// none given means the whole repository); membership is by label here, the
    /// advisory grouping counterpart to the DAG-authoritative `issue progress`
    /// over a container's children. `--by state` lists every lifecycle state,
    /// zero-count states included. `done` and `rejected` are counted distinctly
    /// (a rejected issue is terminal but not delivered) and, because `archived` is
    /// terminality-preserving, an issue archived from `done`/`rejected` counts
    /// toward that origin; `open` is every issue that is not effectively terminal;
    /// the `done/total` ratio and percent measure delivery.
    ///
    /// `--json` emits `{count, by_state: [{state, count}], total, done, rejected,
    /// open, percent}` where `count` is the number of state buckets.
    ///
    /// Examples:
    ///   jit query count --by state
    ///   jit query count --by state --label milestone:m1 --json
    Count {
        /// Aggregation dimension.
        #[arg(long, value_name = "DIMENSION")]
        by: CountDimension,

        /// Filter by label pattern (namespace:value, exact match; or
        /// namespace:* wildcard). Repeatable; patterns are ANDed, so an issue
        /// must match every pattern given. None given aggregates the whole repo.
        #[arg(short = 'l', long)]
        label: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Report membership labels that disagree with DAG-resolved containment
    ///
    /// Advisory: lists each issue that carries a membership label (`epic:foo`,
    /// `milestone:v1.0`, …) while the dependency DAG does not place it inside the
    /// container that owns that label — i.e. the label claims a membership the
    /// authoritative DAG does not back. The dependency DAG is the source of
    /// truth; membership labels are advisory grouping.
    ///
    /// JSON output uses the list envelope `{"count": N, "divergences": [...]}`.
    Divergence {
        #[arg(long)]
        json: bool,
    },
}

/// Dimension to aggregate by in `jit query count --by <DIMENSION>`.
///
/// A typed `--by` value: clap accepts only the declared variants (kebab-case on
/// the CLI) and rejects anything else as a usage error, so a caller cannot
/// silently request an unsupported aggregation. `state` is the only dimension
/// today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CountDimension {
    /// Count by lifecycle state.
    State,
}

#[derive(Subcommand)]
pub enum LabelCommands {
    /// List all label namespaces
    ///
    /// JSON output uses the list envelope `{"count": N, "namespaces": [...]}`.
    Namespaces {
        #[arg(long)]
        json: bool,
    },

    /// List all values used in a namespace
    ///
    /// JSON output uses the list envelope `{"count": N, "values": [...]}`.
    Values {
        /// Namespace to query (e.g., 'milestone', 'epic')
        namespace: String,

        #[arg(long)]
        json: bool,
    },

    /// Hidden stub: not a real command. There is no `label add` — labeling an
    /// issue goes through `jit issue update --label`; this variant exists
    /// only to fail fast with a hint instead of clap's generic "unrecognized
    /// subcommand" error. See `verb_hint_error` in `main.rs`.
    #[command(hide = true, trailing_var_arg = true)]
    Add {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Add` above. `label rm` is not a command; use
    /// `jit issue update --remove-label`.
    #[command(hide = true, trailing_var_arg = true)]
    Rm {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Hidden stub: see `Add` above. `label remove` is not a command; use
    /// `jit issue update --remove-label`.
    #[command(hide = true, trailing_var_arg = true)]
    Remove {
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
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

    /// Validate configuration files for errors
    ///
    /// Checks configuration for:
    /// - Syntax errors in TOML files
    /// - Invalid values (e.g., unknown mode values)
    /// - Deprecated options
    /// - Missing required fields
    ///
    /// Exit codes (see docs/reference/exit-codes.md, the generated reference):
    ///   0 - Valid configuration
    ///   1 - Errors found (invalid configuration)
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
    ///
    /// JSON output uses the list envelope `{"count": N, "templates": [...]}`.
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
    /// A work lease, not issue assignment — for simple assignee bookkeeping,
    /// see `jit issue assign` / `jit issue claim` / `jit issue release` /
    /// `jit issue unassign`.
    ///
    /// Examples:
    ///   jit claim acquire abc123 --ttl 600        # 10-minute lease
    ///   jit claim acquire abc123 --ttl 3600       # 1-hour lease
    ///   jit claim acquire abc123 --ttl 0 --reason "Manual oversight"  # Indefinite (requires reason)
    Acquire {
        /// Issue ID to claim
        issue_id: String,

        /// Time-to-live in seconds (0 for indefinite, requires --reason)
        #[arg(long, default_value_t = crate::runtime_defaults::CLAIM_TTL_SECS)]
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

    /// Release the active lease on an issue
    ///
    /// Resolves the issue's active lease and releases it WITHOUT requiring the
    /// lease UUID, regardless of which agent owns it. The acting identity is
    /// recorded in the audit trail. Errors if the issue has no active lease.
    ///
    /// A work lease, not issue assignment — for simple assignee bookkeeping,
    /// see `jit issue assign` / `jit issue claim` / `jit issue release` /
    /// `jit issue unassign`.
    ///
    /// Examples:
    ///   jit claim release abc123          # release whatever lease is active on issue abc123
    ///   jit claim release abc123 --json
    Release {
        /// Issue ID whose active lease should be released (short ids accepted)
        issue_id: String,

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
        #[arg(long, default_value_t = crate::runtime_defaults::CLAIM_TTL_SECS)]
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
    ///
    /// JSON output uses the list envelope `{"count": N, "leases": [...]}`.
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
    ///
    /// JSON output uses the list envelope `{"count": N, "leases": [...]}`.
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
    ///
    /// JSON output uses the list envelope `{"count": N, "worktrees": [...]}`.
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Git hooks commands
#[derive(Debug, Subcommand)]
pub enum HooksCommands {
    /// Install git hooks for lease and branch-drift validation
    ///
    /// Copies hook templates to .git/hooks/ and makes them executable.
    ///
    /// Hooks installed:
    ///   - pre-commit: Validates leases and branch drift before commit
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

/// Embedded repository profile commands.
#[derive(Debug, Subcommand)]
pub enum ProfileCommands {
    /// List profiles embedded in this JIT binary
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show one embedded profile manifest and package identity
    Show {
        /// Stable embedded profile ID
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Apply an embedded profile to the current repository
    Apply {
        /// Stable embedded profile ID
        id: String,

        /// Build and validate the exact application plan without writing
        #[arg(long)]
        dry_run: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

impl Commands {
    /// Whether this invocation must run transaction recovery before repository
    /// validation or command-service construction.
    ///
    /// These matches are deliberately exhaustive. Adding any CLI enum variant
    /// fails compilation until its mutation classification is chosen, providing
    /// the generated-command guard required by the universal recovery boundary.
    pub fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Init { .. } | Self::Apply { .. } | Self::Recover { .. } | Self::Migrate(_) => {
                true
            }
            Self::Profile(command) => command.requires_recovery_dispatch(),
            Self::Issue(command) => command.requires_recovery_dispatch(),
            Self::Dep(command) => command.requires_recovery_dispatch(),
            Self::Gate(command) => command.requires_recovery_dispatch(),
            Self::Doc(command) => command.requires_recovery_dispatch(),
            Self::Archive(command) => command.requires_recovery_dispatch(),
            Self::Config(command) => command.requires_recovery_dispatch(),
            Self::Claim(command) => command.requires_recovery_dispatch(),
            Self::Hooks(command) => command.requires_recovery_dispatch(),
            Self::Invariant(command) => command.requires_recovery_dispatch(),
            Self::Project(command) => command.requires_recovery_dispatch(),
            Self::Snapshot(command) => command.requires_recovery_dispatch(),
            Self::Validate { fix, dry_run, .. } => *fix && !*dry_run,
            Self::Serve { status, .. } => !*status,
            Self::List { .. }
            | Self::Events(_)
            | Self::Graph(_)
            | Self::Rdeps { .. }
            | Self::Query { .. }
            | Self::Label(_)
            | Self::Worktree(_)
            | Self::Item(_)
            | Self::Search { .. }
            | Self::Version { .. }
            | Self::Status { .. } => false,
        }
    }
}

impl ProfileCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Apply { dry_run, .. } => !*dry_run,
            Self::List { .. } | Self::Show { .. } => false,
        }
    }
}

impl IssueCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Create { .. }
            | Self::BatchCreate { .. }
            | Self::Update { .. }
            | Self::Delete { .. }
            | Self::Assign { .. }
            | Self::Claim { .. }
            | Self::Unassign { .. }
            | Self::Reject { .. }
            | Self::Release { .. }
            | Self::ClaimNext { .. } => true,
            Self::Search { .. }
            | Self::Show { .. }
            | Self::Status { .. }
            | Self::Children { .. }
            | Self::Progress { .. }
            | Self::List { .. }
            | Self::Rm { .. }
            | Self::Remove { .. }
            | Self::Complete { .. }
            | Self::Edit { .. } => false,
        }
    }
}

impl DepCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Add { .. } | Self::Rm { .. } => true,
            Self::Remove { .. } | Self::Delete { .. } => false,
        }
    }
}

impl GateCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Define { .. }
            | Self::Update { .. }
            | Self::Remove { .. }
            | Self::Add { .. }
            | Self::Evaluate { .. }
            | Self::EvaluateAll { .. }
            | Self::Fail { .. } => true,
            Self::Preset(command) => command.requires_recovery_dispatch(),
            Self::Rm { .. }
            | Self::Delete { .. }
            | Self::List { .. }
            | Self::Show { .. }
            | Self::Status { .. }
            | Self::StatusAll { .. } => false,
        }
    }
}

impl PresetCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Apply { .. } | Self::Create { .. } => true,
            Self::List { .. } | Self::Show { .. } => false,
        }
    }
}

impl DocCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Add { .. } | Self::Remove { .. } => true,
            Self::Assets { command } => command.requires_recovery_dispatch(),
            Self::List { .. }
            | Self::Rm { .. }
            | Self::Delete { .. }
            | Self::Show { .. }
            | Self::History { .. }
            | Self::Diff { .. }
            | Self::CheckLinks { .. } => false,
        }
    }
}

impl AssetCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::List { rescan, .. } => *rescan,
        }
    }
}

impl ArchiveCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Document { execute, .. } | Self::Container { execute, .. } => *execute,
            Self::Candidates { .. } => false,
        }
    }
}

impl ConfigCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Set { .. } => true,
            Self::Show { .. }
            | Self::Get { .. }
            | Self::Validate { .. }
            | Self::ShowHierarchy { .. }
            | Self::ListTemplates { .. } => false,
        }
    }
}

impl ClaimCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Acquire { .. }
            | Self::Release { .. }
            | Self::Renew { .. }
            | Self::Heartbeat { .. }
            | Self::ForceEvict { .. } => true,
            Self::Status { .. } | Self::List { .. } => false,
        }
    }
}

impl HooksCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Install { .. } => true,
        }
    }
}

impl InvariantCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Check { .. } => false,
        }
    }
}

impl ProjectCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            // `render` writes documentation targets, so it must run under the
            // stale-binary recovery dispatch like the other write commands.
            Self::Render { .. } => true,
        }
    }
}

impl SnapshotCommands {
    fn requires_recovery_dispatch(&self) -> bool {
        match self {
            Self::Export { .. } => true,
        }
    }
}

#[cfg(test)]
mod recovery_dispatch_tests {
    use super::*;
    use clap::CommandFactory;
    use std::collections::BTreeSet;

    const CLI_LEAF_COMMANDS: &[&str] = &[
        "apply",
        "archive candidates",
        "archive container",
        "archive document",
        "claim acquire",
        "claim force-evict",
        "claim heartbeat",
        "claim list",
        "claim release",
        "claim renew",
        "claim status",
        "config get",
        "config list-templates",
        "config set",
        "config show",
        "config show-hierarchy",
        "config validate",
        "dep add",
        "dep delete",
        "dep remove",
        "dep rm",
        "doc add",
        "doc assets list",
        "doc check-links",
        "doc delete",
        "doc diff",
        "doc history",
        "doc list",
        "doc remove",
        "doc rm",
        "doc show",
        "events query",
        "events tail",
        "gate add",
        "gate define",
        "gate delete",
        "gate evaluate",
        "gate evaluate-all",
        "gate fail",
        "gate list",
        "gate preset apply",
        "gate preset create",
        "gate preset list",
        "gate preset show",
        "gate remove",
        "gate rm",
        "gate show",
        "gate status",
        "gate status-all",
        "gate update",
        "graph deps",
        "graph export",
        "graph rdeps",
        "graph roots",
        "graph tree",
        "hooks install",
        "init",
        "invariant check",
        "issue assign",
        "issue batch-create",
        "issue children",
        "issue claim",
        "issue claim-next",
        "issue complete",
        "issue create",
        "issue delete",
        "issue edit",
        "issue list",
        "issue progress",
        "issue reject",
        "issue release",
        "issue remove",
        "issue rm",
        "issue search",
        "issue show",
        "issue status",
        "issue unassign",
        "issue update",
        "item list",
        "item resolve",
        "item search",
        "item show",
        "label add",
        "label namespaces",
        "label remove",
        "label rm",
        "label values",
        "list",
        "migrate lifecycle-timestamps",
        "profile apply",
        "profile list",
        "profile show",
        "project render",
        "query all",
        "query available",
        "query blocked",
        "query closed",
        "query count",
        "query divergence",
        "query strategic",
        "rdeps",
        "recover",
        "search",
        "serve",
        "snapshot export",
        "status",
        "validate",
        "version",
        "worktree info",
        "worktree list",
    ];

    fn collect_leaf_commands(
        command: &clap::Command,
        path: Vec<String>,
        leaves: &mut BTreeSet<String>,
    ) {
        let subcommands = command
            .get_subcommands()
            .filter(|subcommand| subcommand.get_name() != "help")
            .collect::<Vec<_>>();
        if subcommands.is_empty() {
            leaves.insert(path.join(" "));
            return;
        }
        for subcommand in subcommands {
            let mut subcommand_path = path.clone();
            subcommand_path.push(subcommand.get_name().to_string());
            collect_leaf_commands(subcommand, subcommand_path, leaves);
        }
    }

    #[test]
    fn test_recovery_dispatch_inventory_covers_every_generated_cli_leaf() {
        let mut actual = BTreeSet::new();
        for command in Cli::command()
            .get_subcommands()
            .filter(|command| command.get_name() != "help")
        {
            collect_leaf_commands(command, vec![command.get_name().to_string()], &mut actual);
        }
        let expected = CLI_LEAF_COMMANDS
            .iter()
            .map(|command| (*command).to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "update the exhaustive recovery-dispatch classification and CLI leaf inventory"
        );
    }

    #[test]
    fn test_representative_writer_and_reader_classification() {
        assert!(Commands::Init {
            hierarchy_template: None,
            profile: None,
            json: false,
        }
        .requires_recovery_dispatch());
        assert!(Commands::Validate {
            id: None,
            json: false,
            explain: false,
            scope: None,
            fix: true,
            dry_run: false,
            branch_drift: false,
            divergence: false,
            leases: false,
        }
        .requires_recovery_dispatch());
        assert!(!Commands::Status { json: false }.requires_recovery_dispatch());
        assert!(!Commands::Serve {
            port: 3000,
            stop: false,
            status: true,
            fg: false,
            log: None,
            web_dir: None,
            json: false,
        }
        .requires_recovery_dispatch());
    }

    #[test]
    fn test_snapshot_export_requires_recovery_dispatch() {
        assert!(Commands::Snapshot(SnapshotCommands::Export {
            out: None,
            format: "dir".to_string(),
            scope: "all".to_string(),
            at: None,
            working_tree: false,
            committed_only: false,
            force: false,
            json: false,
        })
        .requires_recovery_dispatch());
    }
}
