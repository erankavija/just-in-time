//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Bracket-aware breakdown operations
//! - `template`: Graph-template apply engine (`jit apply <template> <container>`):
//!   the plan-before-fan-out scaffold (planning node `P` + breakdown node `B`),
//!   validating, snapshotting anchors, and committing the expanded delta
//! - `template_expand`: pure expansion of a graph template into the delta the
//!   apply engine commits (issues to create, edges to add/remove, anchor gates)
//! - `config`: `jit config set` write (target-file resolution, typed value
//!   validation incl. `project.name`, atomic write) and `jit config get`'s
//!   generic dotted-path accessor over the whole configuration surface
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `plan_doc`: Plan-document location resolver (boundary; loads inline/external
//!   plan content and feeds the pure projection engine)
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations
//! - `item`: Addressable-item queries (`jit item list/show/search`) over the pure
//!   item model

mod archive;
pub mod batch_create;
mod breakdown;
pub mod bulk_update;
pub mod claim;
mod config;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod gate_cli_tests;
pub mod graph;
pub mod hooks;
mod init;
pub mod invariant;
mod issue;
pub mod item;
mod labels;
pub mod migrate;
pub mod plan_doc;
mod profile;
mod query;
pub mod reference;
mod search;
pub mod serve;
pub mod snapshot;
mod template;
pub mod template_expand;
mod validate;
pub mod worktree;

#[cfg(test)]
pub mod test_helpers;

pub use archive::ArchiveExecutionHooks;
pub use batch_create::{
    BatchCreateOutcome, BatchIssueDef, BatchValidationError, BatchValidationProblem,
    BatchWriteError,
};
pub use breakdown::{BracketBreakdownResult, BracketChild};
pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};
pub use config::{resolve_dotted_key, ConfigGetOutcome, ConfigKeyError, ConfigSetOutcome};
pub use gate::{
    FieldEdit, GateNotRequiredError, GatePassAllEntry, GatePassFailed, GatePassOutcome, GateUpdate,
    ManualGateAttestationRequiredError, PassAllOutcome,
};
pub use graph::{BatchExport, BoundaryEdge, GraphExportFormat};
pub use init::FreshInitResult;
pub use invariant::{InvariantCheckResult, InvariantRenderResult};
pub use issue::DescriptionUpdate;
pub use item::{ItemListResult, ItemShowResult};
pub use migrate::LifecycleBackfillResult;
pub use profile::ProfileApplyError;
pub use reference::RulesGatesRenderResult;
pub use template::TemplateApplyResult;
pub use template_expand::{
    expand_template, validate_delta_acyclic, AnchorGates, DeltaEdge, DeltaEndpoint, PlannedNode,
    TemplateDelta,
};
pub(crate) use validate::{find_planning_node, planning_node_plan_path, validate_claims_index_at};
pub use validate::{DANGLING_LINK_RULE, ENFORCEMENT_DRIFT_RULE, REVIEW_PLACEHOLDER_RULE};

// Re-export WorktreeIdentity for init return type
pub use crate::storage::worktree_identity::WorktreeIdentity;

// Common imports used across modules
use crate::config::JitConfig;
use crate::config_manager::ConfigManager;
use crate::domain::{
    is_dependency_met, Event, Gate, GateMode, GateState, GateStatus, Issue, LabelNamespaces,
    Priority, State,
};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
use crate::validation::rules::{RuleConfigError, RuleSet};
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::sync::OnceLock;

/// The unpassed gates of `issue` paired with their current status and registry
/// mode, in the order [`Issue::get_unpassed_gates`] reports them.
///
/// A required gate with no recorded run counts as [`GateStatus::Pending`]. A
/// gate key missing from `registry` (should not normally happen) defaults to
/// [`GateMode::Manual`], matching [`CommandExecutor::pass_gate`]'s own
/// fallback: an unregistered gate never matches the `Auto` branch there
/// either, so it is treated as requiring `--by` just the same. Shared by the
/// transition guard and the gate-diversion path so both describe a
/// gate-blocked completion with the same blockers, and so the remediation
/// hint can name the `--by <attestor>` form for a manual gate (jit:1d59070d
/// REQ-03).
fn unpassed_gate_blockers(
    issue: &Issue,
    registry: &crate::storage::GateRegistry,
) -> Vec<(String, GateStatus, GateMode)> {
    issue
        .get_unpassed_gates()
        .into_iter()
        .map(|gate_key| {
            let status = issue
                .gates_status
                .get(&gate_key)
                .map(|gate| gate.status)
                .unwrap_or(GateStatus::Pending);
            let mode = registry
                .gates
                .get(&gate_key)
                .map(|gate| gate.mode)
                .unwrap_or(GateMode::Manual);
            (gate_key, status, mode)
        })
        .collect()
}

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize, // New: count of rejected issues
    pub blocked: usize,
    pub total: usize,
}

/// Result of listing document references for an issue
#[derive(Debug, Serialize)]
pub struct DocumentListResult {
    pub issue_id: String,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub count: usize,
}

/// Git history for a document
#[derive(Debug, Serialize)]
pub struct DocumentHistory {
    pub path: String,
    pub commits: Vec<CommitInfo>,
}

/// Result of listing assets for a document
#[derive(Debug, Serialize)]
pub struct AssetListResult {
    pub issue_id: String,
    pub document_path: String,
    /// Number of assets in `assets` (list envelope `count`; equals `assets.len()`).
    pub count: usize,
    pub assets: Vec<crate::document::Asset>,
    pub summary: AssetSummary,
    pub warnings: Vec<String>,
}

/// Document content display result
#[derive(Debug, Serialize)]
pub struct DocumentContentResult {
    pub path: String,
    pub label: Option<String>,
    pub commit: String,
    pub doc_type: Option<String>,
    pub content: String,
}

/// Document diff result
#[derive(Debug, Serialize)]
pub struct DocumentDiffResult {
    pub path: String,
    pub from_commit: String,
    pub to_commit: String,
    pub diff: String,
}

/// Result of adding a document reference
#[derive(Debug, Serialize)]
pub struct DocumentAddResult {
    pub issue_id: String,
    pub document: crate::domain::DocumentReference,
    /// `true` when `path` was already linked to the issue and this call
    /// refreshed that entry in place; `false` when it appended a new one.
    pub updated: bool,
}

/// Result of removing a document reference
#[derive(Debug, Serialize)]
pub struct DocumentRemoveResult {
    pub issue_id: String,
    pub path: String,
}

/// Summary of asset counts by category
#[derive(Debug, Serialize)]
pub struct AssetSummary {
    pub total: usize,
    pub per_doc: usize,
    pub shared: usize,
    pub external: usize,
    pub missing: usize,
}

/// Result of exporting a snapshot
#[derive(Debug, Serialize)]
pub struct SnapshotExportResult {
    pub path: String,
    pub issue_count: usize,
    pub document_count: usize,
    pub format: String,
    pub size_bytes: Option<u64>,
}

/// Result of checking document links
#[derive(Debug, Serialize)]
pub struct LinkCheckResult {
    pub valid: bool,
    pub errors: Vec<serde_json::Value>,
    pub warnings: Vec<serde_json::Value>,
    #[serde(skip)]
    pub exit_code: i32,
    #[serde(skip)]
    pub scope: String,
    pub summary: LinkCheckSummary,
}

/// Summary of link check results
#[derive(Debug, Serialize)]
pub struct LinkCheckSummary {
    pub total_documents: usize,
    pub valid: usize,
    pub errors: usize,
    pub warnings: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
}

/// How `jit dep add` treats an edge that would break transitive reduction.
///
/// Cycle detection is a write-time guard (@/inv/dag-acyclic); this policy makes the
/// transitive-reduction property a write-time guard too, closing the asymmetry
/// where a redundant edge was written silently and only rejected at a later
/// `jit validate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedundancyPolicy {
    /// Reject the add, naming the offending edge pair (nonzero exit). Default.
    #[default]
    Reject,
    /// Add the edge and drop the now-redundant edge(s) in the same operation,
    /// leaving the graph transitively reduced.
    Reduce,
}

/// Outcome of the unified write-time validation pass.
///
/// Produced by the executor's `validate_for_write` entry point BEFORE an issue
/// is persisted. It carries the non-blocking warnings to surface to the caller
/// and the list of `enforce` rules that a `--force` write is bypassing. The
/// bypass events are intentionally NOT emitted during validation: the caller
/// emits them (via `log_rule_bypasses`) only AFTER the write succeeds, so a save
/// that fails never leaves a false "bypass happened" entry in the audit log.
#[derive(Debug, Clone, Default)]
pub struct WriteValidation {
    /// Non-blocking warnings (legacy validator + local `warn`/non-enforce
    /// findings) to surface to the caller.
    pub warnings: Vec<String>,
    /// Names of `enforce` rules whose blocking findings were overridden by
    /// `--force`. One [`Event::LocalRuleBypassed`] must be logged per entry,
    /// AFTER the write commits.
    pub bypassed_rules: Vec<String>,
}

/// Outcome of [`CommandExecutor::sync_default_rule_membership`]: the
/// `namespace-unique-*` default-rule NAMES appended to `.jit/rules.toml` and
/// those dropped from it. Both empty means the file already matched the
/// `[namespaces]` registry (a no-op write).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleMembershipSync {
    /// Names of the `origin = "default"` `namespace-unique-<ns>` rows
    /// appended, in [`default_ruleset`](crate::validation::defaults::default_ruleset)'s
    /// emission order.
    pub added: Vec<String>,
    /// Names of the `origin = "default"` `namespace-unique-<ns>` rows dropped.
    pub dropped: Vec<String>,
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
    pub config_manager: ConfigManager,
    /// Lazily-parsed `.jit/rules.toml`, cached for the lifetime of the
    /// executor so the validation ruleset is read at most once per process
    /// rather than re-parsed on every write. The cached value retains any
    /// load/parse error so callers can surface a misconfigured rules file
    /// instead of silently treating it as "no rules".
    rules: OnceLock<Result<RuleSet, RuleConfigError>>,
    /// Lazily-built EFFECTIVE rule set. When `.jit/rules.toml` is present it
    /// supplies the rules, with the `origin = "default"` family reconciled against
    /// this repo's `config.toml` registry at load
    /// ([`reconcile_default_rules_with_config`](crate::validation::defaults::reconcile_default_rules_with_config));
    /// when absent, the built-in
    /// [`default_ruleset`](crate::validation::defaults::default_ruleset) is built
    /// IN MEMORY. The former hard-coded checks (a0f0f342 migration) now live as
    /// default rules here. A load/parse error from either source is retained as an
    /// `Err` so a misconfigured repo surfaces the problem rather than silently
    /// disabling enforcement.
    effective_rules: OnceLock<Result<RuleSet, String>>,
    /// Lazily-loaded `.jit/config.toml`, cached so the unified write-time
    /// validation entry point does not re-read and re-parse `config.toml` on
    /// every write. The parsed config seeds the built-in default rules. A
    /// malformed config is retained as an `Err` so it is surfaced rather than
    /// swallowed.
    config: OnceLock<Result<JitConfig, String>>,
    /// Lazily-built label namespace registry, derived from the cached config and
    /// cached alongside it for the same reason.
    namespaces: OnceLock<Result<LabelNamespaces, String>>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        let config_manager = ConfigManager::new(storage.root());
        Self {
            storage,
            config_manager,
            rules: OnceLock::new(),
            effective_rules: OnceLock::new(),
            config: OnceLock::new(),
            namespaces: OnceLock::new(),
        }
    }

    /// Get reference to the storage backend
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Return the parsed validation ruleset, loading `.jit/rules.toml` on first
    /// access and caching the result for subsequent calls.
    ///
    /// A MISSING `.jit/rules.toml` is not an error: it yields `Ok(`an empty
    /// [`RuleSet`]`)`. A genuine parse or load failure (malformed TOML, an
    /// invalid `assert` table, an unsafe schema reference, etc.) is returned as
    /// `Err` rather than being swallowed, so a misconfigured repository cannot
    /// silently disable all rule enforcement. The load is performed at most once
    /// and the outcome (success or failure) is cached.
    pub fn rules(&self) -> Result<&RuleSet, &RuleConfigError> {
        self.rules
            .get_or_init(|| RuleSet::load(self.storage.root()))
            .as_ref()
    }

    /// Return the EFFECTIVE rule set, with `.jit/rules.toml` as the operative
    /// ruleset and its `origin = "default"` rules reconciled against `config.toml`
    /// at load (DR §8.2/§8.4).
    ///
    /// Semantics depend on whether the file EXISTS, not whether it is empty:
    ///
    /// - **File present (even with zero rules):** the parsed file supplies the
    ///   operative rule set — which rules exist and, for the built-in rules `jit
    ///   init` scaffolds (marked `origin = "default"`), their editable policy
    ///   fields (severity, enforce, selector). The default family is reconciled
    ///   against the declared `[namespaces]` / `[type_hierarchy]` registry IN
    ///   MEMORY at load
    ///   ([`reconcile_default_rules_with_config`](crate::validation::defaults::reconcile_default_rules_with_config)):
    ///   assertions are re-derived, a rule is added for a newly-declared namespace
    ///   and dropped for a removed one, so a hand edit of the registry cannot
    ///   desync validation against a stale `schemas/default-*.json` projection.
    ///   Custom rules (any other `origin`)
    ///   are used verbatim, reading their own declared schema files. An
    ///   intentionally-emptied file yields an empty set.
    /// - **File ABSENT (pre-init repo or deleted file):** build the FIXED
    ///   [`default_ruleset`](crate::validation::defaults::default_ruleset) from the
    ///   repo's namespace registry IN MEMORY (MF4). This is read-only — NO disk
    ///   write, NO warning, NO error — so gates, the server, read-only checkouts,
    ///   and multiple worktrees stay safe. Only `jit init` materializes the file
    ///   to disk (under the write lock).
    ///
    /// The result is built at most once and cached. A malformed `config.toml` or
    /// `.jit/rules.toml` is surfaced as an `Err` rather than silently dropping
    /// enforcement.
    pub fn effective_rules(&self) -> Result<&RuleSet> {
        self.effective_rules
            .get_or_init(|| {
                let rules_path = self.storage.root().join("rules.toml");
                if rules_path.exists() {
                    // File present (even empty) supplies the operative rule set.
                    // Its default-origin rules are a config PROJECTION: reconcile
                    // the default family against the declared registry IN MEMORY at
                    // load — re-derive assertions, add a rule for a newly-declared
                    // namespace, drop one whose namespace is gone — so the baked
                    // `schemas/default-*.json` files are never the authority. Custom
                    // rules and the default rules' editable policy fields
                    // (severity/enforce/selector) are taken from the file unchanged.
                    let user = self.rules().map_err(|e| e.to_string())?.clone();
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(
                        crate::validation::defaults::reconcile_default_rules_with_config(
                            user, namespaces,
                        ),
                    )
                } else {
                    // File absent: build the fixed defaults IN MEMORY (no write,
                    // no warning) from the repo's namespace registry. Materialized
                    // to disk only by `jit init`.
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(crate::validation::defaults::default_ruleset(namespaces))
                }
            })
            .as_ref()
            .map_err(|err| anyhow!("{err}"))
    }

    /// Return the cached parsed `.jit/config.toml`, loading it on first access.
    ///
    /// The config is read at most once per executor (cached in a `OnceLock`) so
    /// the unified write-validation path does not re-parse `config.toml` on every
    /// write. A malformed config is surfaced as an error rather than swallowed.
    fn cached_config(&self) -> Result<&JitConfig> {
        self.config
            .get_or_init(|| self.config_manager.load().map_err(|err| err.to_string()))
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!("invalid .jit/config.toml: {err}"))
                    .into()
            })
    }

    /// Return the cached label namespace registry, building it on first access.
    ///
    /// Cached alongside [`cached_config`](Self::cached_config) so the registry is
    /// derived at most once per executor.
    fn cached_namespaces(&self) -> Result<&LabelNamespaces> {
        self.namespaces
            .get_or_init(|| {
                // Derive namespaces from the already-cached config so a single
                // write command reads `config.toml` at most once (the prior
                // `get_namespaces()` call re-loaded it from disk).
                match self.cached_config() {
                    Ok(config) => Ok(self.config_manager.namespaces_from_config(config)),
                    Err(err) => Err(format!("{err}")),
                }
            })
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!(
                    "invalid namespace configuration: {err}"
                ))
                .into()
            })
    }

    /// Resolve the repo-level default content format from `[validation]
    /// .content_format` (defaulting to Markdown when unset/absent), used as the
    /// fallback for issues that carry no per-issue `content_format` when selecting
    /// the [`ContentParser`](crate::document::ContentParser). A malformed value is
    /// surfaced as an error rather than silently picking the wrong parser.
    fn repo_content_format(&self) -> Result<crate::domain::ContentFormat> {
        match self.cached_config()?.validation.as_ref() {
            Some(validation) => validation.content_format(),
            None => Ok(crate::domain::ContentFormat::Markdown),
        }
    }

    /// Resolve the repository-wide validation strictness from
    /// `[validation].strictness`, defaulting to [`Strictness::Loose`] when the
    /// key (or the whole `[validation]` section) is absent. A malformed value is
    /// surfaced as an error rather than silently defaulting, so a misconfigured
    /// `config.toml` cannot quietly disable or widen enforcement.
    ///
    /// [`Strictness::Loose`]: crate::validation::Strictness::Loose
    fn validation_strictness(&self) -> Result<crate::validation::Strictness> {
        match self.cached_config()?.validation.as_ref() {
            Some(validation) => validation.strictness(),
            None => Ok(crate::validation::Strictness::Loose),
        }
    }

    /// The single write-time validation entry point shared by issue create,
    /// update, and the batch path (DR §7.5).
    ///
    /// `issue` MUST be the FINAL persisted shape — i.e. all field and state
    /// mutations (create's auto-promotion to `Ready`, update's requested state
    /// transition, bulk's projected after-update shape) already applied — so that
    /// rules keyed on the final `state` are evaluated correctly.
    ///
    /// It evaluates the EFFECTIVE local rules (built-in defaults + user
    /// `.jit/rules.toml`) via
    /// [`evaluate_local`](crate::validation::evaluate_local), then applies the
    /// repository's `[validation].strictness`
    /// ([`Strictness`](crate::validation::Strictness)) to the block/allow
    /// decision. Under the default [`Loose`](crate::validation::Strictness::Loose)
    /// level the blocking semantics are:
    ///
    /// - An `error` finding from an `enforce = true` rule REJECTS the write
    ///   unless `force` is set (DR §7.2).
    /// - With `force`, the write is allowed and the bypassed rule names are
    ///   returned in [`WriteValidation::bypassed_rules`] for the CALLER to log
    ///   AFTER the write commits (DR §7.6) — they are NOT logged here, so a
    ///   failed save cannot leave a false bypass entry in the audit log.
    /// - `warn`/non-`enforce` findings never block; their messages are
    ///   returned as warnings.
    ///
    /// [`Strict`](crate::validation::Strictness::Strict) widens the block set to
    /// EVERY violation (any warning or error blocks);
    /// [`Permissive`](crate::validation::Strictness::Permissive) empties it (no
    /// violation blocks — all findings become warnings). Strictness modulates only
    /// this decision; it never changes a rule's severity or `enforce` flag.
    ///
    /// The former hard-coded `IssueValidator` checks are now default rules inside
    /// the effective rule set, so they run through this same path. A genuinely
    /// misconfigured `.jit/rules.toml` or `config.toml` (parse/load error) is
    /// surfaced as an error rather than silently disabling enforcement.
    fn validate_for_write(&self, issue: &Issue, force: bool) -> Result<WriteValidation> {
        let rules = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let strictness = self.validation_strictness()?;
        let evaluation = crate::validation::evaluate_local(issue, rules, repo_format)
            .map_err(|err| anyhow!("rule evaluation failed: {err}"))?
            .with_strictness(strictness);

        let blocking = evaluation.blocking_rules();
        if !blocking.is_empty() && !force {
            // Ordinary rejection: NOT logged (only --force bypasses are). Typed as a
            // validation failure (exit 4) carrying the message verbatim, so the
            // top-level handler classifies it by downcast rather than message text.
            return Err(crate::errors::ValidationFailedError::new(
                evaluation
                    .rejection_message()
                    .unwrap_or_else(|| "blocked by validation rule(s)".to_string()),
            )
            .into());
        }

        let mut warnings = Vec::new();
        warnings.extend(evaluation.warnings());

        // On a forced write `blocking` names the enforce rules being overridden;
        // the caller logs them AFTER the write succeeds. When nothing blocks (or
        // not forced), `blocking` is empty so no events are deferred.
        Ok(WriteValidation {
            warnings,
            bypassed_rules: blocking,
        })
    }

    /// Append one [`Event::LocalRuleBypassed`] per bypassed `enforce` rule.
    ///
    /// Pass the rule names from [`WriteValidation::bypassed_rules`]. A non-empty
    /// list means the caller explicitly forced an override, which always merits an
    /// audit entry — including a forced no-op write that changed no other field.
    /// When the override accompanies an issue write, call this AFTER the write
    /// commits so a failed save leaves no false bypass entry. A no-op when `rules`
    /// is empty (ordinary writes, rejections, and read-only/preview runs log
    /// nothing).
    fn log_rule_bypasses(&self, issue_id: &str, rules: &[String]) -> Result<()> {
        for rule in rules {
            let event = Event::new_local_rule_bypassed(issue_id.to_string(), rule.clone());
            self.storage.append_event(&event)?;
        }
        Ok(())
    }

    /// The SINGLE chokepoint through which ALL issue state changes must flow.
    ///
    /// ALL issue state changes must flow through this function; do not set
    /// `issue.state` directly in command code. Centralizing the transition here
    /// guarantees that transition-time graph-rule enforcement (CC-2) cannot be
    /// bypassed by a new code path that forgets to wire it in — the bug class that
    /// motivated this refactor (jit bc86f54c), where bulk update set
    /// `issue.state` directly and slipped past an enforcing graph rule.
    ///
    /// Responsibilities, in order:
    ///
    /// 1. **No-op guard.** If `issue.state == target` there is nothing to
    ///    transition: returns `Ok(vec![])` without enforcing, saving, or logging.
    /// 2. **Archived revive guard (`jit:45a140ae`).** Leaving
    ///    [`State::Archived`] is a revive: it may only restore the recorded
    ///    pre-archive origin ([`Issue::archived_from`]). Targeting any other state
    ///    returns a
    ///    [`TransitionBlockedError`](crate::errors::TransitionBlockedError) (exit
    ///    4, `ArchivedRevive` blocker) and persists NOTHING, so the archive
    ///    round-trip cannot resurrect a completed issue into the active lifecycle.
    ///    A legacy Archived record with no recorded origin keeps the prior
    ///    unconstrained revive but returns an advisory warning.
    /// 3. **Dependency and gate guards.** Runs
    ///    [`transition_blockers`](Self::transition_blockers): a transition into
    ///    [`State::Ready`] or [`State::Done`] requires every dependency met, and
    ///    [`State::Done`] additionally requires every required gate passed
    ///    (`@/inv/gate-semantics`). A blocker returns a
    ///    [`TransitionBlockedError`](crate::errors::TransitionBlockedError)
    ///    carrying the structured blockers and persists NOTHING. Callers that
    ///    divert an unpassed-gate `done` into `gated` resolve that target BEFORE
    ///    calling in, so they reach the chokepoint with the target they intend to
    ///    land; the guard is a pure read, so running it there as well is
    ///    idempotent.
    /// 4. **Graph-rule enforcement.** Runs
    ///    [`enforce_transition_graph_rules`](Self::enforce_transition_graph_rules)
    ///    on the issue projected into its TARGET state, EXCEPT when `target` is
    ///    [`State::Rejected`] or [`State::Archived`] — rejection and
    ///    archival/parking deliberately bypass validation (abandoning or retiring
    ///    an issue must not be gated on coverage). That policy is encoded HERE, not
    ///    at call sites, so no caller can accidentally enforce (or fail to skip) on
    ///    those targets. A blocking enforce rule returns a
    ///    [`TransitionBlockedError`](crate::errors::TransitionBlockedError) (exit
    ///    4) and persists NOTHING; non-blocking findings are returned as warnings.
    /// 5. **State mutation.** Sets `issue.state = target`, and maintains
    ///    [`Issue::archived_from`]: entering [`State::Archived`] records the state
    ///    left behind, reviving out of it clears the field.
    /// 6. **Persistence + audit (when `persist`).** When `persist` is true, saves
    ///    the issue and appends the `issue_state_changed` event (plus
    ///    `issue_completed` when landing [`State::Done`]). Including the event
    ///    write here — not only at call sites — means a future caller that forgets
    ///    to log cannot leave a state change unaudited.
    ///
    /// # The `persist` flag
    ///
    /// Persistence conventions differ per caller: the state-only paths
    /// (`update_issue_state`, the auto-transition helpers, `release_issue`) want
    /// the chokepoint to own the save + event, so they pass `persist = true`. The
    /// batch paths that combine a state change with other field edits in ONE save
    /// (`update_issue`, bulk update) pass `persist = false`: they enforce + mutate
    /// through the chokepoint, then perform their own combined save and emit the
    /// `issue_state_changed` event themselves (with their existing idempotency and
    /// bypass-event ordering). Enforcement and the no-op/rejection policy are
    /// always centralized regardless of `persist`.
    ///
    /// `pre_save` runs after a successful (non-blocked) enforcement and state
    /// mutation but BEFORE the save when `persist` is true, letting a caller stamp
    /// additional fields (e.g. clearing the assignee on release) into the same
    /// write. It is unused (a no-op closure) for callers that only change state.
    fn apply_state_transition(
        &self,
        issue: &mut Issue,
        target: State,
        force: bool,
        persist: bool,
        pre_save: impl FnOnce(&mut Issue),
    ) -> Result<Vec<String>> {
        let old_state = issue.state;

        // No-op: nothing to transition, enforce, save, or log.
        if old_state == target {
            return Ok(Vec::new());
        }

        // Archived is terminality-preserving (`jit:45a140ae`): a revive out of
        // Archived may only restore the recorded pre-archive origin, so the
        // archive round-trip cannot resurrect a completed issue into the active
        // lifecycle. A legacy Archived record (no recorded origin) keeps the prior
        // unconstrained revive, with an advisory warning.
        let mut revive_warnings = Vec::new();
        if old_state == State::Archived {
            match issue.archived_from {
                Some(origin) if target != origin => {
                    return Err(crate::errors::TransitionBlockedError::archived_revive(
                        issue.id.clone(),
                        target,
                        origin,
                    )
                    .into());
                }
                None => revive_warnings.push(format!(
                    "issue {} was archived before its pre-archive state was recorded; reviving to \
                     '{}' without a verified origin",
                    issue.short_id(),
                    target.as_str()
                )),
                Some(_) => {}
            }
        }

        // Dependency and gate guards, ahead of any mutation.
        self.transition_blockers(issue, target)?;

        // Rejection and archival deliberately bypass graph-rule enforcement:
        // abandoning or retiring/parking an issue must not be gated on rules such
        // as coverage. Every other target runs enforcement against the TARGET-state
        // projection of the issue.
        let warnings = if matches!(target, State::Rejected | State::Archived) {
            Vec::new()
        } else {
            let mut projected = issue.clone();
            projected.state = target;
            self.enforce_transition_graph_rules(&projected, target, force)?
        };

        // Enforcement passed (or was bypassed/skipped): land the new state.
        issue.state = target;

        // Maintain the pre-archive origin (`jit:45a140ae`): entering Archived
        // records the state left behind (never Archived — the no-op guard above
        // rules that out); leaving Archived (revive) clears it, so `archived_from`
        // is `Some` only while the issue is Archived.
        if target == State::Archived {
            issue.archived_from = Some(old_state);
        } else if old_state == State::Archived {
            issue.archived_from = None;
        }

        // Stamp the lifecycle timestamp for this transition (first-occurrence
        // only; see `Issue::mark_*`). Done HERE, at the single chokepoint every
        // state change flows through, so no Ready/Done path can forget it —
        // including `persist = false` callers, which carry this mutated issue
        // into their own combined save. Claim/assignment stamps `claimed_at`
        // separately (assignment is not a state transition).
        match target {
            State::Ready => issue.mark_first_ready(Utc::now()),
            State::Done => issue.mark_done(Utc::now()),
            _ => {}
        }

        if persist {
            pre_save(issue);
            let issue_id = issue.id.clone();
            self.storage.save_issue(issue.clone())?;

            let event = Event::new_issue_state_changed(issue_id.clone(), old_state, target);
            self.storage.append_event(&event)?;

            if target == State::Done {
                let event = Event::new_issue_completed(issue_id);
                self.storage.append_event(&event)?;
            }
        }

        // Surface the legacy-revive advisory ahead of any enforcement warnings.
        revive_warnings.extend(warnings);
        Ok(revive_warnings)
    }

    /// The dependency and gate guards a transition must clear, evaluated against
    /// `issue` in its CURRENT state.
    ///
    /// Invoked exclusively by [`apply_state_transition`](Self::apply_state_transition):
    /// entering [`State::Ready`] or [`State::Done`] requires every dependency to
    /// have reached a terminal state, and entering [`State::Done`] additionally
    /// requires every required gate to have passed (`@/inv/gate-semantics`).
    /// Dependencies are checked before gates, so an issue that is both
    /// dependency-blocked and gate-blocked reports its dependencies.
    ///
    /// Every other target (including [`State::Rejected`], which abandons an issue)
    /// is unguarded. `force` has no bearing here: it overrides validation rules,
    /// never the graph's own semantics.
    fn transition_blockers(&self, issue: &Issue, target: State) -> Result<()> {
        if !matches!(target, State::Ready | State::Done) {
            return Ok(());
        }

        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);
        let blockers = self.blocking_dependencies(issue, &resolved);
        if !blockers.is_empty() {
            return Err(crate::errors::TransitionBlockedError::dependencies(
                issue.id.clone(),
                target,
                issue.state,
                blockers,
            )
            .into());
        }

        if target == State::Done && issue.has_unpassed_gates() {
            let registry = self.storage.load_gate_registry()?;
            return Err(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                State::Done,
                issue.state,
                unpassed_gate_blockers(issue, &registry),
            )
            .into());
        }

        Ok(())
    }

    /// Enforce the graph rules applicable to an issue at a state transition (CC-2).
    ///
    /// Invoked exclusively by [`apply_state_transition`](Self::apply_state_transition),
    /// the single chokepoint for state changes; command code never calls this
    /// directly. `issue` MUST already carry its TARGET state so a rule selector
    /// keyed on `state` (e.g. `when = { state = "done" }`) matches only at the
    /// transition that lands that state.
    ///
    /// Behavior (CC-2 / CC-2a):
    ///
    /// - Selects `RuleScope::Graph` rules (severity != `off`) whose `when` matches the
    ///   issue in its target state, SKIPPING rules with repo-wide semantics
    ///   ([`Assertion::is_repo_wide_at_transition`]) — those stay `jit validate`
    ///   concerns because they need the whole repository, not a slice.
    /// - Evaluates the selected rules over the issue's dependency NEIGHBORHOOD
    ///   (the issue plus its transitive dependencies and dependents), not
    ///   `list_issues()` wholesale, via [`DependencyGraph`] reachability.
    /// - Whether a finding ATTRIBUTED to this issue BLOCKS the transition is the
    ///   repo-wide [`Strictness`](crate::validation::Strictness) decision applied
    ///   to the finding's rule `enforce` flag and severity — the SAME modulator as
    ///   the write path. Under the default
    ///   [`Loose`](crate::validation::Strictness::Loose) level that is an
    ///   `enforce = true` / `error` finding; [`Strict`](crate::validation::Strictness::Strict)
    ///   widens it to any violation and [`Permissive`](crate::validation::Strictness::Permissive)
    ///   blocks nothing. A blocking finding appends one
    ///   [`Event::TransitionBlocked`] per rule (the attempted transition is the
    ///   auditable act) and returns a
    ///   [`TransitionBlockedError`](crate::errors::TransitionBlockedError) (exit
    ///   4), unless `force` is set.
    /// - With `force`, blocking findings do NOT block; one
    ///   [`Event::GraphRuleBypassed`] is appended per overridden rule.
    /// - A `config-error` finding (a malformed rule: bad regex, missing key) whose
    ///   selector applies to this issue BLOCKS whenever the strictness/enforce
    ///   decision blocks it — a broken guard must not silently pass. The blocker
    ///   message makes clear the rule itself is misconfigured.
    /// - Findings the strictness decision does not block (and findings attributed
    ///   to OTHER issues in the slice) never block; their `[rule] message` strings
    ///   are returned as warnings for the caller to surface.
    fn enforce_transition_graph_rules(
        &self,
        issue: &Issue,
        target: State,
        force: bool,
    ) -> Result<Vec<String>> {
        use crate::validation::graph::evaluate_graph;
        use crate::validation::rules::{RuleScope, Severity};

        let ruleset = self.effective_rules()?;

        // Graph rules that apply to THIS issue in its target state, minus the
        // repo-wide ones that cannot be evaluated correctly on a slice.
        let rules: Vec<&crate::validation::rules::Rule> = ruleset
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
            .filter(|rule| !rule.assert.is_repo_wide_at_transition())
            .filter(|rule| rule.when.matches(issue))
            .collect();

        // Nothing to enforce: skip the (potentially large) store read entirely.
        if rules.is_empty() {
            return Ok(Vec::new());
        }

        // Neighborhood slice: the issue plus its transitive dependency closure in
        // BOTH directions. This is what coverage/reference rules need (the issue's
        // children/parents); rules are evaluated over this narrowed slice rather
        // than the whole issue set.
        let slice = self.transition_neighborhood(issue)?;

        let namespaces = self.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
        let hierarchy = crate::validation::defaults::hierarchy_config(namespaces);
        let repo_format = self.repo_content_format()?;

        // Resolve external plan docs for the neighborhood so closure-time coverage
        // honors a container whose criteria live in an external plan file too.
        let plan_content = self.resolve_plan_content(&slice)?;

        let findings = evaluate_graph(
            &rules,
            &slice,
            &hierarchy,
            repo_format,
            chrono::Utc::now(),
            &plan_content,
        );

        // Per-rule `enforce` flag, looked up by rule name so the strictness
        // modulator can read it per finding. A finding whose rule is not in the
        // selected set (should not happen) is treated as non-enforcing.
        let enforcing: std::collections::HashSet<&str> = rules
            .iter()
            .filter(|r| r.enforce)
            .map(|r| r.name.as_str())
            .collect();

        // Repo-wide strictness modulates which violations block this transition,
        // exactly as on the write path: loose blocks only an enforced error,
        // strict blocks any violation, permissive blocks nothing.
        let strictness = self.validation_strictness()?;

        let mut blocking: Vec<(String, String)> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for gf in &findings {
            // A config-error finding (issue_id = None, e.g. a bad id-pattern regex
            // or a missing key) carries no issue attribution, but it means the
            // rule itself is broken. If the strictness/enforce decision blocks it,
            // the broken guard must BLOCK the transition rather than degrade to a
            // warning — a typo in an enforcing rule must not silently disable the
            // guard. (The rule is already known to apply to this issue: `rules`
            // was filtered by `when.matches(issue)`.)
            let is_config_error = gf.is_config_error();
            let attributed_to_self = gf.issue_id.as_deref() == Some(issue.id.as_str());
            let pertains = attributed_to_self || is_config_error;
            let rule_enforces = enforcing.contains(gf.finding.rule.as_str());
            let is_blocker = pertains && strictness.blocks(rule_enforces, gf.finding.severity);
            if is_blocker {
                let message = if is_config_error {
                    // Make clear the rule itself is misconfigured, not the issue.
                    format!(
                        "rule '{}' is misconfigured: {}; fix the rule or use --force",
                        gf.finding.rule, gf.finding.message
                    )
                } else {
                    gf.finding.message.clone()
                };
                blocking.push((gf.finding.rule.clone(), message));
            } else {
                warnings.push(format!("[{}] {}", gf.finding.rule, gf.finding.message));
            }
        }

        if !blocking.is_empty() {
            if force {
                // Forced override: record one bypass event per blocked rule. The
                // caller commits the issue save right after this returns; the
                // append-only audit log records the deliberate override.
                for (rule, _) in &blocking {
                    let event =
                        Event::new_graph_rule_bypassed(issue.id.clone(), target, rule.clone());
                    self.storage.append_event(&event)?;
                }
            } else {
                // Blocked: log the attempted transition (one event per blocking
                // rule) BEFORE returning the error, then persist nothing.
                for (rule, _) in &blocking {
                    let event =
                        Event::new_transition_blocked(issue.id.clone(), target, rule.clone());
                    self.storage.append_event(&event)?;
                }
                return Err(crate::errors::TransitionBlockedError::graph_rules(
                    issue.id.clone(),
                    target,
                    issue.state,
                    blocking,
                )
                .into());
            }
        }

        Ok(warnings)
    }

    /// Build the dependency-neighborhood issue slice for transition-time graph
    /// evaluation (CC-2a): the issue itself, every issue it transitively depends
    /// on, and every issue that transitively depends on it.
    ///
    /// The passed `issue` already carries its TARGET state, so the slice contains
    /// that projected shape (not the stale persisted copy) for the issue under
    /// transition; all OTHER members are the persisted issues from the store.
    /// Built from [`DependencyGraph`] reachability so the rule evaluators see only
    /// the reachable slice, not every issue. (The graph itself is built from
    /// `list_issues()`, so the read is repo-wide; the narrowing is in what gets
    /// materialized and evaluated.)
    fn transition_neighborhood(&self, issue: &Issue) -> Result<Vec<Issue>> {
        use std::collections::HashSet;

        let all = self.storage.list_issues()?;
        let refs: Vec<&Issue> = all.iter().collect();
        let graph = DependencyGraph::new(&refs);

        // Ids in the neighborhood: self + transitive deps + transitive dependents.
        // Building the graph still reads every issue (`list_issues()`), but only
        // the reachable slice is materialized and handed to the rule evaluators.
        let mut ids: HashSet<String> = HashSet::new();
        ids.insert(issue.id.clone());
        for dep in graph.get_transitive_dependents(&issue.id) {
            ids.insert(dep.id.clone());
        }
        for dep in graph.get_transitive_dependencies(&issue.id) {
            ids.insert(dep.id.clone());
        }

        // Materialize the slice, substituting the target-state projection of the
        // issue under transition for its persisted copy.
        let slice = all
            .into_iter()
            .filter(|i| ids.contains(&i.id))
            .map(|i| if i.id == issue.id { issue.clone() } else { i })
            .collect();
        Ok(slice)
    }

    /// Initialize a new jit repository in the current directory.
    ///
    /// Returns the worktree identity if in a git repository (`None` otherwise),
    /// paired with any non-fatal [`StorageWarning`](crate::storage::StorageWarning)s
    /// observed while loading the identity (e.g. a relocation). This method never
    /// writes to stderr; the calling command surfaces the warnings.
    pub fn init(
        &self,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        self.storage.init()?;
        self.initialize_worktree_identity()
    }

    /// Create or refresh machine-local worktree identity after repository
    /// scaffold publication.
    ///
    /// Kept separate from [`Self::init`] so the fresh profiled path can publish
    /// all repository bytes transactionally before performing optional Git host
    /// integration.
    pub fn initialize_worktree_identity(
        &self,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
        use crate::storage::worktree_paths::WorktreePaths;

        // Check if we're actually in a git repository
        let in_git_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !in_git_repo {
            return Ok((None, Vec::new()));
        }

        // Create worktree identity
        let paths = WorktreePaths::detect()?;

        // Get git branch name
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&paths.worktree_root)
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "main".to_string())
            .trim()
            .to_string();

        // Create or update worktree identity
        // This handles copied files in git worktrees automatically
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
            &paths.local_jit,
            &paths.worktree_root,
            &branch,
        )?;

        Ok((Some(identity), warnings))
    }

    /// Scaffold the FIXED default `.jit/rules.toml` (+ `.jit/schemas/*.json`) for a
    /// new repo (DR §8.4). Called from the `jit init` path AFTER `config.toml` has
    /// been written/exists, so the default rules derive from the repo's real
    /// namespace registry + type hierarchy.
    ///
    /// Idempotent: a no-op when `.jit/rules.toml` already exists (the present file
    /// is left untouched, so re-init never clobbers user edits) beyond republishing
    /// its projections (the header comment and `schemas/default-*.json`) and
    /// write-through syncing `namespace-unique-*` MEMBERSHIP
    /// ([`sync_default_rule_membership`](Self::sync_default_rule_membership)).
    /// Returns `true` when it wrote a fresh file, `false` when it was a
    /// projection-and-membership-only refresh.
    pub fn scaffold_default_rules(&self) -> Result<bool> {
        let jit_root = self.storage.root();

        // Hold the repo write lock across the existence check AND the scaffold
        // writes, so two concurrent `jit init` runs cannot both pass the
        // absent-file check and then race on the fixed temp paths for rules.toml
        // and the schema files (MF4: materialize under the write lock).
        let _guard = self.control_plane_write_lock("rules.lock")?;

        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);

        if crate::storage::ruleset_store::has_validation_ruleset(jit_root) {
            // `rules.toml` is never clobbered when present. Its default-origin
            // rules validate against the config registry derived in memory, but the
            // `schemas/default-*.json` files and the file's header comment are
            // projections; republish both from the current registry / contract so a
            // re-init refreshes them. The `namespace-unique-*` MEMBERSHIP is
            // additionally write-through synced into the file itself (not just
            // projected) so `@/rule/<name>` addressability tracks the registry too.
            // The header rewrite preserves every rule body (including custom-rule
            // comments). Idempotent and atomic; a no-op when already current.
            self.refresh_default_schema_projections()?;
            self.sync_default_rule_membership()?;
            crate::storage::ruleset_store::rewrite_rules_header(
                jit_root,
                crate::validation::serialize::rules_file_header(),
            )?;
            return Ok(false);
        }

        // Validation produces the content; storage performs the atomic writes.
        let serialized = crate::validation::serialize::serialize_ruleset(
            &crate::validation::defaults::default_ruleset(&namespaces),
        );
        let schema_files: Vec<(String, String)> = serialized
            .schema_files
            .into_iter()
            .map(|file| (file.name, file.content))
            .collect();
        crate::storage::ruleset_store::write_validation_ruleset(
            jit_root,
            &serialized.rules_toml,
            &schema_files,
        )?;
        Ok(true)
    }

    /// Refresh every default-origin `schemas/default-*.json` projection from the
    /// current `[namespaces]` / `[type_hierarchy]` config, returning the names of
    /// the files (re)written.
    ///
    /// The default rules validate against the registry reconciled in memory at
    /// load
    /// ([`reconcile_default_rules_with_config`](crate::validation::defaults::reconcile_default_rules_with_config)),
    /// so these files are write-through projections for external consumers, never
    /// the validation authority. jit republishes them whenever it writes
    /// `config.toml` or `rules.toml` (init/re-init, `config set`) so a projection
    /// a tool reads stays current after a jit-driven registry change.
    ///
    /// Idempotent (rewriting current content yields identical bytes) and atomic
    /// (temp + rename per file). A no-op for a repo with no materialized `schemas/`
    /// layout — the read path builds the default schemas in memory — so it is safe
    /// to call unconditionally.
    pub fn refresh_default_schema_projections(&self) -> Result<Vec<String>> {
        let jit_root = self.storage.root();
        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);
        // Validation builds the schema content (the SAME files `jit init`
        // scaffolds); storage performs the atomic per-file write.
        let serialized = crate::validation::serialize::serialize_ruleset(
            &crate::validation::defaults::default_ruleset(&namespaces),
        );
        let mut written = Vec::new();
        for file in serialized.schema_files {
            if crate::storage::ruleset_store::write_baked_schema(
                jit_root,
                &file.name,
                &file.content,
            )? {
                written.push(file.name);
            }
        }
        Ok(written)
    }

    /// Write-through the `namespace-unique-*` DEFAULT-rule file MEMBERSHIP into
    /// `.jit/rules.toml` itself (not just its `schemas/*.json` projections), so
    /// the registry-first `rule` item kind — which resolves `@/rule/<name>`
    /// straight from the file, not the in-memory-reconciled ruleset (`jit item
    /// show`/`list`, docs-mechanical citation checking) — never dangles behind
    /// [`reconcile_default_rules_with_config`](crate::validation::defaults::reconcile_default_rules_with_config)'s
    /// load-time-only reconciliation.
    ///
    /// Computes [`default_rule_membership_diff`](crate::validation::defaults::default_rule_membership_diff)
    /// between the CURRENT on-disk `rules.toml` and the CURRENT `[namespaces]`
    /// registry, then appends the row for each newly-unique namespace and drops
    /// the row for each namespace no longer unique or no longer declared —
    /// `origin = "default"` rows ONLY. Every other byte of the file (custom
    /// rules, hand-edited policy fields on surviving default rules, comments,
    /// formatting) is untouched (REQ-01, jit:d74a9ed1).
    ///
    /// Called from the same jit-driven-write triggers as
    /// [`refresh_default_schema_projections`](Self::refresh_default_schema_projections)
    /// (init/re-init, `config set`), so a registry edit that changes derived
    /// membership propagates to the file on the next jit write, not only in
    /// memory. A no-op when `rules.toml` is absent or the diff is empty.
    ///
    /// In-memory reconciliation stays the validation authority (out of scope
    /// for this write-through) — this exists only so the file cannot lag it
    /// for addressability.
    pub fn sync_default_rule_membership(&self) -> Result<RuleMembershipSync> {
        let jit_root = self.storage.root();
        if !jit_root.join("rules.toml").exists() {
            return Ok(RuleMembershipSync::default());
        }

        // Identity-only read (never full RuleSet validation): a custom rule
        // whose assertion fails to load must not strand this sync after
        // config.toml was already saved (jit:d74a9ed1 review F1).
        let identities = crate::storage::ruleset_store::read_rule_identities(jit_root)?;
        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);
        let diff = crate::validation::defaults::default_rule_membership_diff_from_identities(
            &identities,
            &namespaces,
        );
        if diff.is_empty() {
            return Ok(RuleMembershipSync::default());
        }

        let to_add_blocks: Vec<String> = diff
            .to_add
            .iter()
            .map(crate::validation::serialize::render_rule_block)
            .collect();
        crate::storage::ruleset_store::sync_namespace_unique_rules(
            jit_root,
            &to_add_blocks,
            &diff.to_drop,
        )?;

        Ok(RuleMembershipSync {
            added: diff.to_add.into_iter().map(|r| r.name).collect(),
            dropped: diff.to_drop,
        })
    }

    /// Acquire the control-plane lock named `lock_file`, held until the returned
    /// guard drops. Returns `None` when this storage root's working tree is not a
    /// git repository of its own: it then has no control plane, so the caller
    /// proceeds lockless.
    ///
    /// The lock lives in the repo's git control plane (`.git/jit/locks/`), the
    /// same plane claims coordination uses, derived from the repository that OWNS
    /// this storage root rather than the process's ambient cwd (see
    /// [`repo_control_plane_dir`](Self::repo_control_plane_dir)).
    ///
    /// It guards control-plane state, NOT the issue store: no `IssueStore` write
    /// path takes it, and it is absent outside git. A sequence that mutates issues,
    /// gates, or events must instead hold
    /// [`IssueStore::acquire_repo_write_lock`](crate::storage::IssueStore::acquire_repo_write_lock),
    /// the lock every ordinary writer takes, which lives in the storage root and
    /// exists with or without git (`@/charter/D-4`).
    fn control_plane_write_lock(
        &self,
        lock_file: &str,
    ) -> Result<Option<crate::storage::lock::LockGuard>> {
        use crate::storage::FileLocker;
        use std::time::Duration;

        let Some(control_plane) = self.repo_control_plane_dir() else {
            return Ok(None);
        };
        let locks_dir = control_plane.join("locks");
        std::fs::create_dir_all(&locks_dir)
            .context("Failed to create control-plane locks directory")?;
        FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ))
        .lock_exclusive(&locks_dir.join(lock_file))
        .map(Some)
    }

    /// Resolve the git control-plane dir (`<git-common-dir>/jit`) for the
    /// repository that OWNS this storage root's working tree, or `None` when that
    /// working tree is not a git repository of its own.
    ///
    /// Determined from the storage root's parent (the working dir) rather than the
    /// process cwd, so a `.jit` nested under an unrelated ancestor repo is treated
    /// as non-git instead of borrowing the ancestor's control plane. Returns the
    /// shared common dir for linked worktrees (where `.git` is a gitdir pointer
    /// file) so siblings serialize on the same lock.
    fn repo_control_plane_dir(&self) -> Option<std::path::PathBuf> {
        let work_dir = self.storage.root().parent()?;
        let dot_git = work_dir.join(".git");
        if dot_git.is_dir() {
            // Main worktree: the common dir IS `<work_dir>/.git`.
            Some(dot_git.join("jit"))
        } else if dot_git.is_file() {
            // Linked worktree: `.git` is a gitdir pointer; ask git (with cwd at
            // this working tree, so it resolves THIS repo) for the shared common
            // dir that all linked worktrees share.
            let out = std::process::Command::new("git")
                .args(["rev-parse", "--git-common-dir"])
                .current_dir(work_dir)
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let raw = String::from_utf8(out.stdout).ok()?;
            let common = std::path::PathBuf::from(raw.trim());
            let common = if common.is_absolute() {
                common
            } else {
                work_dir.join(common)
            };
            Some(common.join("jit"))
        } else {
            None
        }
    }

    /// Check if an active lease exists for the given issue by the current agent.
    ///
    /// Returns true if the current agent has an active lease (not expired or stale).
    /// Returns false if no lease exists, lease is stale, or belongs to another agent.
    ///
    /// In tests: If JIT_AGENT_ID is not set, any valid lease counts (single-user mode).
    fn check_active_lease(&self, issue_id: &str) -> Result<bool> {
        use crate::agent_config::resolve_agent_id;
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;

        // Get worktree paths to access shared control plane
        let paths = match WorktreePaths::detect() {
            Ok(p) => p,
            Err(_) => {
                // Not in a git repository - no claims possible
                return Ok(false);
            }
        };

        // Load the active-lease index through storage (an absent index yields an
        // empty one, i.e. no active leases).
        let claims_index = ClaimsIndex::load(&paths)?;

        // Resolve current agent identity (or None for single-user mode)
        let current_agent = resolve_agent_id(None).ok();

        // Check if active lease exists for this issue
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let now = chrono::Utc::now();

        let has_active_lease = claims_index.leases.iter().any(|lease| {
            // Must match issue ID
            if lease.issue_id != full_id {
                return false;
            }

            // Must not be expired
            if let Some(expires) = lease.expires_at {
                if expires <= now {
                    return false;
                }
            }

            // Must not be stale
            if claims_index.is_stale(lease) {
                return false;
            }

            // Agent verification:
            // - If current_agent is Some, lease must belong to this agent
            // - If current_agent is None (single-user mode), any valid lease counts
            match &current_agent {
                Some(agent_id) => lease.agent_id == *agent_id,
                None => true, // Single-user mode: any valid lease
            }
        });

        Ok(has_active_lease)
    }

    /// Require an active lease for the given issue, respecting enforcement mode.
    ///
    /// Checks the configured enforcement mode and either blocks, warns, or bypasses
    /// the lease requirement. Used before structural operations that modify issues.
    ///
    /// # Errors
    ///
    /// Returns an error in `strict` mode when no active lease exists.
    /// In `warn` mode, returns Ok with a warning message.
    /// In `off` mode, always returns Ok with None.
    ///
    /// Returns: Result<Option<String>> where Some(warning) should be printed by the CLI layer.
    pub fn require_active_lease(&self, issue_id: &str) -> Result<Option<String>> {
        use crate::config::EnforcementMode;

        // Derive the mode from the cached config so a write command that also runs
        // `validate_for_write` parses `config.toml` at most once (DR §6.1).
        let mode = self
            .config_manager
            .enforcement_mode_from_config(self.cached_config()?)?;

        match mode {
            EnforcementMode::Off => Ok(None),
            EnforcementMode::Warn | EnforcementMode::Strict => {
                let has_lease = self.check_active_lease(issue_id)?;

                if !has_lease {
                    let msg = format!(
                        "No active lease for issue {}.\nAcquire lease with: jit claim acquire {}",
                        issue_id, issue_id
                    );

                    match mode {
                        EnforcementMode::Warn => Ok(Some(msg)),
                        EnforcementMode::Strict => {
                            anyhow::bail!("{}", msg)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }
}

// Helper functions for parsing command-line arguments
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rules_are_parsed_once_and_cached() {
        use crate::storage::InMemoryStorage;
        use crate::validation::rules::Assertion;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        let rules_path = storage.root().join("rules.toml");
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "first"
assert = { require-section = { heading = "Goals" } }
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses and caches the file.
        let first = executor.rules().expect("valid rules");
        assert_eq!(first.rules.len(), 1);
        assert_eq!(first.rules[0].name, "first");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate the file on disk AFTER the first parse.
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "second"
assert = { require-section = { heading = "Other" } }

[[rules]]
name = "third"
assert = { require-doc-type = { doc-type = "design" } }
"#,
        )
        .unwrap();

        // Second access must return the cached (original) parse, NOT re-read.
        let second = executor.rules().expect("valid rules");
        assert_eq!(second.rules.len(), 1, "ruleset was re-read from disk");
        assert_eq!(second.rules[0].name, "first");
        assert!(matches!(
            second.rules[0].assert,
            Assertion::RequireSection { .. }
        ));
        // Same cached instance is returned each time.
        assert_eq!(ptr_first, std::ptr::from_ref(second));
    }

    #[test]
    fn test_namespaces_are_parsed_once_and_cached() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        let config_path = storage.root().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses config.toml and caches the namespace registry.
        let first = executor.cached_namespaces().expect("valid namespaces");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate config.toml on disk AFTER the first parse. A re-read would pick
        // up the new namespace; a cached registry must not.
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false

[namespaces.owner]
description = "Ownership tags"
unique = false
"#,
        )
        .unwrap();

        // Second access must return the SAME cached instance, proving the
        // namespace config is parsed once per executor and not re-read per write.
        let second = executor.cached_namespaces().expect("valid namespaces");
        assert_eq!(
            ptr_first,
            std::ptr::from_ref(second),
            "namespace registry was re-read from disk instead of cached"
        );
    }

    #[test]
    fn test_rules_missing_file_yields_empty_ok() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        // No rules.toml written.

        let executor = CommandExecutor::new(storage);
        let rules = executor.rules().expect("missing file is not an error");
        assert!(rules.rules.is_empty());
    }

    #[test]
    fn test_rules_malformed_file_yields_err() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        // A rule whose assert table has no kind is a genuine config error and
        // must NOT be downgraded to an empty (no-rules) set.
        std::fs::write(
            storage.root().join("rules.toml"),
            r#"
[[rules]]
name = "broken"
assert = {}
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);
        assert!(
            executor.rules().is_err(),
            "malformed rules.toml must surface an error, not an empty set"
        );
    }

    // Enforcement tests
    #[test]
    fn test_require_active_lease_off_mode() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // Create the root directory and config with enforcement off
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should always succeed in off mode, even without lease, and return None
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_check_active_lease_no_claims_index() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        let executor = CommandExecutor::new(storage);

        // No claims index - should return false
        let result = executor.check_active_lease(&issue_id);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_require_active_lease_strict_mode_no_lease() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // Create the root directory and config with enforcement strict
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should fail in strict mode without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No active lease"));
        assert!(err_msg.contains("jit claim acquire"));
    }

    #[test]
    fn test_require_active_lease_off_mode_default() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // No config file - should default to off mode (single-agent development)
        let executor = CommandExecutor::new(storage);

        // Should succeed in off mode (default) without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
    }

    // Agent identity verification tests
    #[test]
    fn test_check_active_lease_verifies_agent_identity() {
        // This test documents the agent identity verification behavior.
        // Since check_active_lease() now uses resolve_agent_id(),
        // it verifies agent ownership in multi-agent scenarios:
        //
        // 1. If JIT_AGENT_ID is set (or --agent-id / ~/.config/jit/agent.toml),
        //    only leases belonging to that agent count as active.
        // 2. If not set (single-user mode), any valid lease counts.
        //
        // This prevents Agent A from modifying issues claimed by Agent B.
        //
        // Full workflow testing requires integration tests with git repos
        // and actual claims.index.json files.
    }
}
