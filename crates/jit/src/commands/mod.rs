//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Issue breakdown operations
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations

mod breakdown;
pub mod bulk_update;
pub mod claim;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod gate_cli_tests;
pub mod graph;
pub mod hooks;
mod issue;
mod labels;
mod query;
mod search;
pub mod serve;
pub mod snapshot;
mod validate;
pub mod worktree;

#[cfg(test)]
pub mod test_helpers;

pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};
pub use gate::GatePassFailed;

// Re-export WorktreeIdentity for init return type
pub use crate::storage::worktree_identity::WorktreeIdentity;

// Common imports used across modules
use crate::config::JitConfig;
use crate::config_manager::ConfigManager;
use crate::domain::{Event, Gate, GateState, GateStatus, Issue, LabelNamespaces, Priority, State};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
use crate::validation::rules::{RuleConfigError, RuleSet};
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::sync::OnceLock;

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
    pub assets: Vec<crate::document::Asset>,
    pub summary: AssetSummary,
    pub warnings: Vec<String>,
}

/// Result of archiving a document
#[derive(Debug, Serialize)]
pub struct ArchiveResult {
    pub source_path: String,
    pub dest_path: String,
    pub category: String,
    pub assets_moved: usize,
    pub updated_issues: Vec<String>,
    pub dry_run: bool,
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

/// Outcome of the unified write-time validation pass.
///
/// Produced by the executor's `validate_for_write` entry point BEFORE an issue
/// is persisted. It carries the non-blocking warnings to surface to the caller
/// and the list of `enforce` rules that a `--force` write is bypassing. The
/// bypass events are intentionally NOT emitted during validation: the caller
/// emits them (via `log_rule_bypasses`) only AFTER the write succeeds, so a save
/// that fails never leaves a false "bypass happened" entry in the audit log.
///
/// # Examples
///
/// ```
/// use jit::commands::WriteValidation;
///
/// // A default outcome blocks nothing and defers no bypass events.
/// let outcome = WriteValidation::default();
/// assert!(outcome.warnings.is_empty());
/// assert!(outcome.bypassed_rules.is_empty());
/// ```
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
    /// Lazily-loaded `.jit/config.toml`, cached so the unified write-time
    /// validation entry point does not re-read and re-parse `config.toml` on
    /// every write. The parsed config drives the legacy
    /// [`IssueValidator`](crate::validation::IssueValidator) until
    /// task a0f0f342 migrates those checks to default rules. A malformed config
    /// is retained as an `Err` so it is surfaced rather than swallowed.
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// match executor.rules() {
    ///     Ok(rules) => println!("loaded {} rule(s)", rules.rules.len()),
    ///     Err(err) => eprintln!("invalid rules.toml: {err}"),
    /// }
    /// ```
    pub fn rules(&self) -> Result<&RuleSet, &RuleConfigError> {
        self.rules
            .get_or_init(|| RuleSet::load(self.storage.root()))
            .as_ref()
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
            .map_err(|err| anyhow!("invalid .jit/config.toml: {err}"))
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
            .map_err(|err| anyhow!("invalid namespace configuration: {err}"))
    }

    /// The single write-time validation entry point shared by issue create,
    /// update, and the batch path (DR §7.5).
    ///
    /// `issue` MUST be the FINAL persisted shape — i.e. all field and state
    /// mutations (create's auto-promotion to `Ready`, update's requested state
    /// transition, bulk's projected after-update shape) already applied — so that
    /// rules keyed on the final `state` are evaluated correctly.
    ///
    /// It performs two layers of validation in one place:
    ///
    /// 1. The legacy [`IssueValidator`](crate::validation::IssueValidator)
    ///    (driven by the cached `config.toml`), retained until task a0f0f342
    ///    migrates its label/type/namespace checks to default rules. Its
    ///    non-blocking warnings are collected.
    /// 2. The declarative local rules (`.jit/rules.toml`) via
    ///    [`evaluate_local`](crate::validation::evaluate_local), with blocking
    ///    semantics:
    ///    - An `error` finding from an `enforce = true` rule REJECTS the write
    ///      unless `force` is set (DR §7.2).
    ///    - With `force`, the write is allowed and the bypassed rule names are
    ///      returned in [`WriteValidation::bypassed_rules`] for the CALLER to log
    ///      AFTER the write commits (DR §7.6) — they are NOT logged here, so a
    ///      failed save cannot leave a false bypass entry in the audit log.
    ///    - `warn`/non-`enforce` findings never block; their messages are
    ///      returned as warnings.
    ///
    /// A genuinely misconfigured `.jit/rules.toml` or `config.toml` (parse/load
    /// error) is surfaced as an error rather than silently disabling enforcement.
    fn validate_for_write(&self, issue: &Issue, force: bool) -> Result<WriteValidation> {
        // Layer 1: legacy IssueValidator (config-driven). Retained until a0f0f342.
        let config = self.cached_config()?;
        let mut warnings = if let Some(ref validation_config) = config.validation {
            let validator = crate::validation::IssueValidator::new(
                validation_config.clone(),
                self.cached_namespaces()?.clone(),
            );
            validator.validate(issue)?
        } else {
            Vec::new()
        };

        // Layer 2: declarative local rules (.jit/rules.toml).
        let rules = match self.rules() {
            Ok(rules) => rules,
            Err(err) => return Err(anyhow!("invalid .jit/rules.toml: {err}")),
        };
        let evaluation = crate::validation::evaluate_local(issue, rules)
            .map_err(|err| anyhow!("rule evaluation failed: {err}"))?;

        let blocking = evaluation.blocking_rules();
        if !blocking.is_empty() && !force {
            // Ordinary rejection: NOT logged (only --force bypasses are).
            return Err(anyhow!(
                "{}",
                evaluation
                    .rejection_message()
                    .unwrap_or_else(|| "blocked by validation rule(s)".to_string())
            ));
        }

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

    /// Initialize a new jit repository in the current directory.
    /// Returns the worktree identity if in a git repository, None otherwise.
    pub fn init(&self) -> Result<Option<WorktreeIdentity>> {
        use crate::storage::worktree_identity::load_or_create_worktree_identity;
        use crate::storage::worktree_paths::WorktreePaths;

        self.storage.init()?;

        // Check if we're actually in a git repository
        let in_git_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !in_git_repo {
            return Ok(None);
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
        let identity =
            load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

        Ok(Some(identity))
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

        // Load claims index
        let claims_index_path = paths.shared_jit.join("claims.index.json");
        if !claims_index_path.exists() {
            // No claims index - no active leases
            return Ok(false);
        }

        let contents =
            std::fs::read_to_string(&claims_index_path).context("Failed to read claims index")?;
        let claims_index: ClaimsIndex =
            serde_json::from_str(&contents).context("Failed to parse claims index")?;

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
