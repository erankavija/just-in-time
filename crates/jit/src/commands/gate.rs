//! Quality gate operations

use super::*;
use crate::domain::{GateMode, GateRunResult, GateRunStatus};

/// Error returned when `jit gate pass` runs an automated checker that does not pass.
///
/// The checker result is preserved so CLI callers can report the failed status
/// without disagreeing with the persisted `gates_status` value.
///
/// # Examples
///
/// ```rust
/// use jit::commands::GatePassFailed;
///
/// fn remediation(error: &GatePassFailed) -> String {
///     format!("jit gate check {} {}", error.issue_id, error.gate_key)
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[error(
    "Gate '{gate_key}' failed for issue {issue_id}. Checker status: {status:?}, exit code: {exit_code:?}. Inspect details with: jit gate check {issue_id} {gate_key}"
)]
pub struct GatePassFailed {
    /// Issue whose gate was checked.
    pub issue_id: String,
    /// Gate key that failed.
    pub gate_key: String,
    /// Raw checker status.
    pub status: GateRunStatus,
    /// Checker process exit code, when available.
    pub exit_code: Option<i32>,
    /// Full checker result persisted under `.jit/gate-runs/`.
    pub result: GateRunResult,
    /// Warnings gathered before running the checker.
    pub warnings: Vec<String>,
}

/// Error returned when `jit gate pass` targets a gate that the issue does not require.
///
/// This is an argument/lookup error raised before any checker runs: the named
/// gate is simply not in the issue's `gates_required` list. CLI callers classify
/// it as an invalid-argument condition (exit code `2`), distinct from a checker
/// failure or a runner error.
///
/// # Examples
///
/// ```rust
/// use jit::commands::GateNotRequiredError;
///
/// fn remediation(error: &GateNotRequiredError) -> String {
///     format!("jit gate add {} {}", error.issue_id, error.gate_key)
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[error("Gate '{gate_key}' is not required for issue {issue_id}")]
pub struct GateNotRequiredError {
    /// Issue that was targeted.
    pub issue_id: String,
    /// Gate key that is not in the issue's required set.
    pub gate_key: String,
}

/// Outcome of a successful [`pass_gate`](CommandExecutor::pass_gate) call.
///
/// Carries any warnings gathered along the way (e.g. lease warnings) plus
/// `already_passed`, which is `true` when the gate was found to have already
/// passed at the current `HEAD` commit and the (potentially expensive) checker
/// was skipped. On a normal run — manual attestation or a freshly executed
/// checker — `already_passed` is `false`.
///
/// # Examples
///
/// ```rust
/// use jit::commands::GatePassOutcome;
///
/// fn describe(outcome: &GatePassOutcome) -> &'static str {
///     if outcome.already_passed {
///         "already passed at HEAD; checker skipped"
///     } else {
///         "gate passed"
///     }
/// }
///
/// let outcome = GatePassOutcome {
///     warnings: Vec::new(),
///     already_passed: true,
/// };
/// assert_eq!(describe(&outcome), "already passed at HEAD; checker skipped");
/// ```
#[derive(Debug, Clone)]
pub struct GatePassOutcome {
    /// Warnings gathered while passing the gate (e.g. lease warnings).
    pub warnings: Vec<String>,
    /// `true` when the checker was skipped because the gate already passed at
    /// the current `HEAD` commit.
    pub already_passed: bool,
}

/// Per-gate entry in a [`PassAllOutcome`].
///
/// Records which gate was passed and whether its checker actually ran
/// (`already_passed == false`) or was skipped because it already passed at the
/// current `HEAD` (`already_passed == true`), plus any warnings gathered.
///
/// # Examples
///
/// ```rust
/// use jit::commands::GatePassAllEntry;
///
/// let entry = GatePassAllEntry {
///     gate_key: "tests".into(),
///     already_passed: false,
///     warnings: Vec::new(),
/// };
/// assert_eq!(entry.gate_key, "tests");
/// ```
#[derive(Debug, Clone)]
pub struct GatePassAllEntry {
    /// The gate that was passed.
    pub gate_key: String,
    /// `true` when the checker was skipped (gate already passed at `HEAD`).
    pub already_passed: bool,
    /// Warnings gathered while passing this gate.
    pub warnings: Vec<String>,
}

/// Outcome of a successful [`pass_all_gates`](CommandExecutor::pass_all_gates) run.
///
/// `results` holds one [`GatePassAllEntry`] per required gate, in declaration
/// order. When the issue has no required gates the list is empty (the command
/// still succeeds with exit `0`). On the first non-passing gate `pass_all_gates`
/// fails fast and returns the underlying error instead of this outcome, so a
/// `PassAllOutcome` always describes an all-green run.
///
/// # Examples
///
/// ```rust
/// use jit::commands::{GatePassAllEntry, PassAllOutcome};
///
/// let outcome = PassAllOutcome {
///     results: vec![GatePassAllEntry {
///         gate_key: "tests".into(),
///         already_passed: true,
///         warnings: Vec::new(),
///     }],
/// };
/// assert_eq!(outcome.results.len(), 1);
/// assert!(outcome.results[0].already_passed);
/// ```
#[derive(Debug, Clone)]
pub struct PassAllOutcome {
    /// Per-gate results, in `gates_required` order.
    pub results: Vec<GatePassAllEntry>,
}

/// Result of adding multiple gates
#[derive(Debug, Serialize)]
pub struct GateAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
}

/// Result of removing multiple gates
#[derive(Debug, Serialize)]
pub struct GateRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Add a single gate to an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<Vec<String>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key.clone());
            // Note: Gates don't block Ready state, only Done state
            self.storage.save_issue(issue)?;
        }
        Ok(warnings)
    }

    /// Add multiple gates to an issue atomically.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn add_gates(
        &self,
        issue_id: &str,
        gate_keys: &[String],
    ) -> Result<(GateAddResult, Vec<String>)> {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let registry = self.storage.load_gate_registry()?;
        let mut issue = self.storage.load_issue(&full_id)?;

        let mut added = Vec::new();
        let mut already_exist = Vec::new();
        let mut not_found = Vec::new();

        // First pass: validate all gates exist in registry
        for gate_key in gate_keys {
            if !registry.gates.contains_key(gate_key) {
                not_found.push(gate_key.clone());
            }
        }

        // Atomic: fail entirely if any gate doesn't exist
        if !not_found.is_empty() {
            return Err(crate::storage::GateNotFoundError::new(not_found).into());
        }

        // Second pass: add gates (now safe since all are validated)
        for gate_key in gate_keys {
            if issue.gates_required.contains(gate_key) {
                already_exist.push(gate_key.clone());
            } else {
                issue.gates_required.push(gate_key.clone());

                // Initialize status if not present
                if !issue.gates_status.contains_key(gate_key) {
                    issue.gates_status.insert(
                        gate_key.clone(),
                        GateState {
                            status: GateStatus::Pending,
                            updated_by: None,
                            updated_at: Utc::now(),
                        },
                    );
                }

                added.push(gate_key.clone());

                // Log gate added event
                self.storage
                    .append_event(&Event::new_gate_added(full_id.clone(), gate_key.clone()))?;
            }
        }

        // Save only if at least one gate was actually added. Re-adding gates
        // that already exist is a no-op and must not bump `updated_at`.
        if !added.is_empty() {
            self.storage.save_issue(issue)?;
        }

        Ok((
            GateAddResult {
                added,
                already_exist,
            },
            warnings,
        ))
    }

    /// Remove multiple gates from an issue.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn remove_gates(
        &self,
        issue_id: &str,
        gate_keys: &[String],
    ) -> Result<(GateRemoveResult, Vec<String>)> {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;

        let mut removed = Vec::new();
        let mut not_found = Vec::new();

        for gate_key in gate_keys {
            if issue.gates_required.contains(gate_key) {
                issue.gates_required.retain(|g| g != gate_key);
                issue.gates_status.remove(gate_key);
                removed.push(gate_key.clone());

                // Log gate removed event
                self.storage
                    .append_event(&Event::new_gate_removed(full_id.clone(), gate_key.clone()))?;
            } else {
                not_found.push(gate_key.clone());
            }
        }

        // Save only if at least one gate was actually removed. Removing gates
        // that are not present is a no-op and must not bump `updated_at`.
        if !removed.is_empty() {
            self.storage.save_issue(issue)?;
        }

        Ok((GateRemoveResult { removed, not_found }, warnings))
    }

    /// Mark a gate as passed.
    ///
    /// For an automated gate this runs the checker; for a manual gate it records
    /// the attestation. When `force` is `false` and the gate's latest run already
    /// passed at the current `HEAD` commit, the (often expensive) checker is
    /// skipped and the returned outcome has `already_passed == true`. Passing
    /// `force = true` always re-runs the checker.
    ///
    /// Returns a [`GatePassOutcome`] carrying any warnings (e.g. lease warnings)
    /// and whether the checker was skipped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jit::commands::CommandExecutor;
    /// use jit::{InMemoryStorage, IssueStore};
    ///
    /// let executor = CommandExecutor::new(InMemoryStorage::new());
    /// let issue = jit::domain::Issue::new("Title".into(), "Body".into());
    /// let id = issue.id.clone();
    /// executor.storage().save_issue(issue).unwrap();
    /// executor.add_gate(&id, "review".into()).unwrap();
    ///
    /// // Manual gate: not already passed, so the checker is not skipped.
    /// let outcome = executor
    ///     .pass_gate(&id, "review".into(), Some("human:alice".into()), false)
    ///     .unwrap();
    /// assert!(!outcome.already_passed);
    /// ```
    pub fn pass_gate(
        &self,
        issue_id: &str,
        gate_key: String,
        by: Option<String>,
        force: bool,
    ) -> Result<GatePassOutcome> {
        // Validate the actor through the one `Assignee` path before it is stored
        // on the gate state.
        let by = by
            .map(|s| s.parse::<crate::domain::Assignee>())
            .transpose()?;
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(GateNotRequiredError {
                issue_id: full_id,
                gate_key,
            }
            .into());
        }

        // Skip the checker when the gate is CURRENTLY passed AND its latest run
        // passed at the current HEAD, unless --force was given. Requiring the
        // current `gates_status` to be Passed prevents a false success when the
        // gate was reset to Pending (e.g. removed and re-added) while a stale
        // passing run still lingers at this HEAD. A `None` HEAD (no git / no
        // commit) cannot prove the prior pass is still valid, so we fall through.
        let current_passed = matches!(
            issue.gates_status.get(&gate_key),
            Some(s) if s.status == GateStatus::Passed
        );
        if !force && current_passed && self.gate_passed_at_head(&full_id, &gate_key)? {
            return Ok(GatePassOutcome {
                warnings,
                already_passed: true,
            });
        }

        // Check if gate is automated - if so, run the checker instead
        let registry = self.storage.load_gate_registry()?;
        if let Some(gate) = registry.gates.get(&gate_key) {
            if gate.mode == GateMode::Auto {
                // Smart behavior: auto-run the checker
                let result = self.check_gate(&full_id, &gate_key)?;
                if result.status != GateRunStatus::Passed {
                    return Err(GatePassFailed {
                        issue_id: full_id,
                        gate_key,
                        status: result.status,
                        exit_code: result.exit_code,
                        result,
                        warnings,
                    }
                    .into());
                }
                return Ok(GatePassOutcome {
                    warnings,
                    already_passed: false,
                });
            }
        }

        // Manual gate: mark as passed
        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Passed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;

        // Log event
        let event = Event::new_gate_passed(issue_id, gate_key, by);
        self.storage.append_event(&event)?;

        // Check if Gated issue can now transition to Done
        self.auto_transition_to_done(&full_id)?;

        Ok(GatePassOutcome {
            warnings,
            already_passed: false,
        })
    }

    /// Pass every required gate for an issue in declaration order, fail-fast.
    ///
    /// Each gate is passed via [`pass_gate`](Self::pass_gate), so it inherits the
    /// skip-if-passed-at-`HEAD` behaviour (already-passed gates are not re-run)
    /// and the same error classification. On the FIRST gate that does not pass,
    /// the underlying error is propagated unchanged and no later gate is
    /// attempted, so the caller can map it to the right exit code (checker
    /// failure, runner error, etc.). An issue with no required gates succeeds
    /// with an empty result set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jit::commands::CommandExecutor;
    /// use jit::{InMemoryStorage, IssueStore};
    ///
    /// let executor = CommandExecutor::new(InMemoryStorage::new());
    /// let issue = jit::domain::Issue::new("Title".into(), "Body".into());
    /// let id = issue.id.clone();
    /// executor.storage().save_issue(issue).unwrap();
    ///
    /// // No required gates: succeeds with an empty result set.
    /// let outcome = executor.pass_all_gates(&id, None, false).unwrap();
    /// assert!(outcome.results.is_empty());
    /// ```
    pub fn pass_all_gates(
        &self,
        issue_id: &str,
        by: Option<String>,
        force: bool,
    ) -> Result<PassAllOutcome> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Fail-fast: pass each required gate in order, stopping at the first that
        // does not pass and propagating its error unchanged. `try_fold` short-
        // circuits on the first `Err`, so later gates are never attempted.
        let results = issue.gates_required.iter().try_fold(
            Vec::with_capacity(issue.gates_required.len()),
            |mut results, gate_key| {
                let outcome = self.pass_gate(&full_id, gate_key.clone(), by.clone(), force)?;
                results.push(GatePassAllEntry {
                    gate_key: gate_key.clone(),
                    already_passed: outcome.already_passed,
                    warnings: outcome.warnings,
                });
                Ok::<_, anyhow::Error>(results)
            },
        )?;

        Ok(PassAllOutcome { results })
    }

    /// Whether the gate's latest run passed at the current `HEAD` commit.
    ///
    /// This is the historical-run half of the skip predicate. The full skip in
    /// [`pass_gate`](Self::pass_gate) ALSO requires the issue's current
    /// `gates_status` for this gate to be `Passed`, so a gate reset to `Pending`
    /// (e.g. removed and re-added) is never skipped even if a stale passing run
    /// lingers at this HEAD.
    ///
    /// Compares the current `HEAD` (resolved from the same repo root the checker
    /// runs in, so the commit is apples-to-apples with the value stamped into
    /// [`GateRunResult::commit`](crate::domain::GateRunResult)) against the latest
    /// recorded run for `(issue, gate)`. Returns `true` only when that run passed
    /// AND both commits are present and equal. A missing `HEAD` (no git / no
    /// commit) yields `false`, since the prior pass cannot be proven current.
    fn gate_passed_at_head(&self, full_id: &str, gate_key: &str) -> Result<bool> {
        let head = crate::gate_execution::get_git_commit(&self.checker_repo_root());
        let Some(head) = head else {
            return Ok(false);
        };
        let passed = self
            .get_last_gate_run(full_id, gate_key)?
            .is_some_and(|run| {
                run.status == GateRunStatus::Passed && run.commit.as_deref() == Some(head.as_str())
            });
        Ok(passed)
    }

    /// Mark a gate as failed.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn fail_gate(
        &self,
        issue_id: &str,
        gate_key: String,
        by: Option<String>,
    ) -> Result<Vec<String>> {
        // Validate the actor through the one `Assignee` path before it is stored
        // on the gate state.
        let by = by
            .map(|s| s.parse::<crate::domain::Assignee>())
            .transpose()?;
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        // Check if gate is automated - reject manual pass/fail
        let registry = self.storage.load_gate_registry()?;
        if let Some(gate) = registry.gates.get(&gate_key) {
            if gate.mode == GateMode::Auto {
                return Err(anyhow!(
                    "Gate '{}' is automated and cannot be manually failed. Use 'jit gate check {} {}' to run the checker.",
                    gate_key, &full_id, gate_key
                ));
            }
        }

        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Failed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;

        // Log event
        let event = Event::new_gate_failed(issue_id, gate_key, by);
        self.storage.append_event(&event)?;

        Ok(warnings)
    }

    pub fn list_gates(&self) -> Result<Vec<Gate>> {
        let registry = self.storage.load_gate_registry()?;
        Ok(registry.gates.into_values().collect())
    }

    pub fn add_gate_definition(
        &self,
        key: String,
        title: String,
        description: String,
        auto: bool,
        example_integration: Option<String>,
        stage: crate::domain::GateStage,
    ) -> Result<()> {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(crate::storage::GateAlreadyExistsError::new(key.as_str()).into());
        }

        registry.gates.insert(
            key.clone(),
            Gate {
                version: 1,
                key,
                title,
                description,
                stage,
                mode: if auto {
                    crate::domain::GateMode::Auto
                } else {
                    crate::domain::GateMode::Manual
                },
                checker: None,
                priority: 100,
                reserved: std::collections::HashMap::new(),
                auto,
                example_integration,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    /// Define a new gate with full control over stage, mode, and checker
    #[allow(clippy::too_many_arguments)]
    pub fn define_gate(
        &self,
        key: String,
        title: String,
        description: String,
        stage: crate::domain::GateStage,
        mode: crate::domain::GateMode,
        checker: Option<crate::domain::GateChecker>,
        priority: u32,
    ) -> Result<()> {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(crate::storage::GateAlreadyExistsError::new(key.as_str()).into());
        }

        // Validate: auto gates must have checker
        if mode == crate::domain::GateMode::Auto && checker.is_none() {
            return Err(anyhow!(
                "Automated gates must have a checker configured. Add --checker-command or use --mode manual"
            ));
        }

        // For manual gates, ignore any provided checker
        let final_checker = if mode == crate::domain::GateMode::Manual {
            None
        } else {
            checker
        };

        registry.gates.insert(
            key.clone(),
            Gate {
                version: 1,
                key,
                title,
                description,
                stage,
                mode,
                checker: final_checker,
                priority,
                reserved: std::collections::HashMap::new(),
                auto: mode == crate::domain::GateMode::Auto,
                example_integration: None,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn remove_gate_definition(&self, key: &str) -> Result<()> {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        let mut registry = self.storage.load_gate_registry()?;

        if !registry.gates.contains_key(key) {
            return Err(
                crate::errors::NotFoundError::new(format!("Gate '{}' not found", key)).into(),
            );
        }

        registry.gates.remove(key);
        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn show_gate_definition(&self, key: &str) -> Result<Gate> {
        let registry = self.storage.load_gate_registry()?;
        registry.gates.get(key).cloned().ok_or_else(|| {
            crate::errors::NotFoundError::new(format!("Gate '{}' not found", key)).into()
        })
    }

    // Preset management methods

    pub fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        self.storage.list_gate_presets()
    }

    pub fn show_gate_preset(
        &self,
        name: &str,
    ) -> Result<crate::gate_presets::GatePresetDefinition> {
        self.storage.get_gate_preset(name)
    }

    /// Apply a gate preset to an issue.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn apply_gate_preset(
        &self,
        issue_id: &str,
        preset_name: &str,
        timeout_override: Option<u64>,
        skip_precheck: bool,
        skip_postcheck: bool,
        except_gates: &[String],
    ) -> Result<(GateAddResult, Vec<String>)> {
        use crate::domain::{GateChecker, GateStage};

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        // Load preset
        let preset = self.storage.get_gate_preset(preset_name)?;

        // Filter gates based on options
        let gates_to_apply: Vec<_> = preset
            .gates
            .iter()
            .filter(|g| {
                // Skip prechecks if requested
                if skip_precheck && g.stage == GateStage::Precheck {
                    return false;
                }
                // Skip postchecks if requested
                if skip_postcheck && g.stage == GateStage::Postcheck {
                    return false;
                }
                // Skip excepted gates
                if except_gates.contains(&g.key) {
                    return false;
                }
                true
            })
            .collect();

        if gates_to_apply.is_empty() {
            return Err(anyhow!("No gates to apply after filtering"));
        }

        // First, define gates in registry if they don't exist
        let mut registry = self.storage.load_gate_registry()?;
        for gate_template in &gates_to_apply {
            let mut gate = gate_template.to_gate();

            // Apply timeout override if specified
            if let Some(timeout) = timeout_override {
                if let Some(checker) = &mut gate.checker {
                    match checker {
                        GateChecker::Exec {
                            timeout_seconds, ..
                        } => {
                            *timeout_seconds = timeout;
                        }
                    }
                }
            }

            // Add to registry (update if exists and timeout override specified, or add if new)
            if timeout_override.is_some() || !registry.gates.contains_key(&gate.key) {
                registry.gates.insert(gate.key.clone(), gate);
            }
        }

        // Save updated registry
        self.storage.save_gate_registry(&registry)?;

        // Now add gates to issue
        let gate_keys: Vec<String> = gates_to_apply.iter().map(|g| g.key.clone()).collect();
        self.add_gates(issue_id, &gate_keys)
    }

    /// List all gate run results for an issue, optionally filtered by gate key.
    ///
    /// Results are sorted newest-first by `started_at`.
    pub fn list_gate_runs(
        &self,
        issue_id: &str,
        gate_key_filter: Option<&str>,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut runs = self.storage.list_gate_runs_for_issue(&full_id)?;

        if let Some(key) = gate_key_filter {
            runs.retain(|r| r.gate_key == key);
        }

        runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        Ok(runs)
    }

    /// Load a single gate run result by run ID.
    pub fn get_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult> {
        self.storage.load_gate_run_result(run_id)
    }

    pub fn create_gate_preset(
        &self,
        preset_name: &str,
        from_issue_id: &str,
    ) -> Result<std::path::PathBuf> {
        use crate::gate_presets::{GatePresetDefinition, GateTemplate};

        // Validate preset name
        if preset_name.is_empty() {
            return Err(anyhow!("Preset name cannot be empty"));
        }
        if crate::gate_presets::BuiltinPresets::names().contains(&preset_name.to_string()) {
            return Err(anyhow!("Cannot override builtin preset: {}", preset_name));
        }

        let full_id = self.storage.resolve_issue_id(from_issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        if issue.gates_required.is_empty() {
            return Err(anyhow!("Issue has no gates to create preset from"));
        }

        // Load gate definitions from registry
        let registry = self.storage.load_gate_registry()?;
        let mut gates = Vec::new();

        for gate_key in &issue.gates_required {
            let gate = registry
                .gates
                .get(gate_key)
                .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;

            gates.push(GateTemplate {
                key: gate.key.clone(),
                title: gate.title.clone(),
                description: gate.description.clone(),
                stage: gate.stage,
                mode: gate.mode,
                checker: gate.checker.clone(),
            });
        }

        // Create preset
        let preset = GatePresetDefinition {
            name: preset_name.to_string(),
            description: format!("Custom preset created from issue {}", issue.short_id()),
            gates,
        };

        // Save preset via storage
        self.storage.save_gate_preset(&preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Gate, GateChecker, GateMode, GateStage};
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create config with enforcement off for test backward compatibility
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        CommandExecutor::new(storage)
    }

    #[test]
    fn test_pass_of_automated_gate_runs_checker() {
        let executor = setup();

        // Define an automated gate that will pass
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            Gate {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Automated Gate".to_string(),
                description: "Auto gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass should auto-run the checker
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
            false,
        );

        assert!(
            result.is_ok(),
            "Pass of automated gate should run checker and succeed"
        );

        // Verify gate is marked as passed
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("auto-gate").unwrap().status,
            crate::domain::GateStatus::Passed
        );
    }

    /// Register a simple manual gate so it can be added to issues in tests.
    fn define_manual_gate(executor: &CommandExecutor<InMemoryStorage>, key: &str) {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            key.to_string(),
            Gate {
                version: 1,
                key: key.to_string(),
                title: key.to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();
    }

    #[test]
    fn test_add_existing_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, "g1");

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        // Add the gate for real, then snapshot updated_at.
        let (res1, _) = executor.add_gates(&issue_id, &["g1".to_string()]).unwrap();
        assert_eq!(res1.added, vec!["g1".to_string()]);
        let updated_after_add = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Re-adding the same gate is a no-op: it must not bump updated_at.
        let (res2, _) = executor.add_gates(&issue_id, &["g1".to_string()]).unwrap();
        assert!(res2.added.is_empty());
        assert_eq!(res2.already_exist, vec!["g1".to_string()]);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_after_add,
            "re-adding an existing gate must not bump updated_at"
        );
    }

    #[test]
    fn test_remove_absent_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, "g1");

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Removing a gate the issue does not have is a no-op.
        let (res, _) = executor
            .remove_gates(&issue_id, &["g1".to_string()])
            .unwrap();
        assert!(res.removed.is_empty());
        assert_eq!(res.not_found, vec!["g1".to_string()]);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_before,
            "removing an absent gate must not bump updated_at"
        );
    }

    #[test]
    fn test_pass_of_automated_gate_that_fails() {
        let executor = setup();

        // Define an automated gate that will fail
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            Gate {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Automated Gate".to_string(),
                description: "Auto gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass runs the checker, which fails
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
            false,
        );

        assert!(
            result.is_err(),
            "Pass should fail when the automated checker fails"
        );

        // Verify gate is marked as failed (checker failed)
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("auto-gate").unwrap().status,
            crate::domain::GateStatus::Failed
        );
    }

    #[test]
    fn test_manual_pass_of_manual_gate_should_succeed() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".to_string(),
            Gate {
                version: 1,
                key: "manual-gate".to_string(),
                title: "Manual Gate".to_string(),
                description: "Manual gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Manual pass of manual gate should succeed
        let result = executor.pass_gate(
            &issue_id,
            "manual-gate".to_string(),
            Some("human:reviewer".to_string()),
            false,
        );
        assert!(result.is_ok(), "Manual pass of manual gate should succeed");

        // Verify gate is marked as passed
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("manual-gate").unwrap().status,
            crate::domain::GateStatus::Passed
        );
    }
}
