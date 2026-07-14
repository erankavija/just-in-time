//! Gate checking and execution operations

use super::*;
use crate::domain::{GateContext, GateMode, GateRunResult, GateRunStatus, GateStage, GateStatus};
use crate::errors::TransitionBlockedError;
use crate::gate_execution;
use crate::output::IssueShowResponse;
use std::collections::HashMap;

/// Select and sanitize the latest run for one gate before it enters checker context.
///
/// Stored results are never mutated. Structured findings replace verbose stdout;
/// legacy unstructured runs retain stdout as their compatibility signal. Stderr is
/// always diagnostic noise and is removed from the context copy.
fn compact_run_history_for_context(runs: &[GateRunResult], gate_key: &str) -> Vec<GateRunResult> {
    runs.iter()
        .filter(|run| run.gate_key == gate_key)
        .max_by_key(|run| run.started_at)
        .cloned()
        .map(|mut run| {
            run.stderr.clear();
            if run.findings.is_some() {
                run.stdout.clear();
            }
            run
        })
        .into_iter()
        .collect()
}

/// Remove the gate currently being evaluated from the issue's gate projections.
///
/// Its recorded status necessarily predates the in-flight evaluation and is not
/// current evidence. The gate definition identifies the active gate, while its
/// compact prior result remains available through `run_history`.
fn omit_current_gate_projection(issue: &mut serde_json::Value, gate_key: &str) {
    if let Some(gates) = issue
        .get_mut("gates")
        .and_then(serde_json::Value::as_array_mut)
    {
        gates.retain(|gate| gate.get("key").and_then(serde_json::Value::as_str) != Some(gate_key));
    }
}

/// Compare the running binary's own build provenance
/// ([`build_info::version_info`](crate::build_info::version_info)) against
/// `repo_root`'s current `HEAD`, returning why it is stale, or `None` when it
/// is fresh or the comparison does not apply (REQ-03).
///
/// I/O boundary for [`domain::build_provenance`](crate::domain::build_provenance):
/// resolves `repo_root`'s `HEAD` and whether the build commit is a known
/// commit there via [`GitRevisionResolver`](crate::storage::GitRevisionResolver)'s
/// `git rev-parse --verify <rev>^{commit}` semantics (the REQ-03 identity
/// predicate — a plain string inequality is not enough, see the module docs),
/// then hands both to
/// [`assess_binary_provenance`](crate::domain::build_provenance::assess_binary_provenance)
/// for the actual decision. The second resolution (whether the build commit
/// is known) is skipped entirely when `HEAD` itself does not resolve, so a
/// non-git or git-unavailable `repo_root` costs a single git invocation.
fn stale_binary_reason(
    repo_root: &std::path::Path,
) -> Option<crate::domain::build_provenance::StaleBinaryReason> {
    use crate::domain::build_provenance::{assess_binary_provenance, BinaryProvenance};
    use crate::storage::GitRevisionResolver;

    let info = crate::build_info::version_info();
    let resolver = GitRevisionResolver::new(repo_root);
    let repo_head = resolver.resolve_commit("HEAD").ok();
    let known_in_repo = repo_head.is_some() && resolver.resolve_commit(info.git_commit).is_ok();

    match assess_binary_provenance(
        Some(info.git_commit),
        info.git_dirty,
        repo_head.as_ref().map(|v| v.as_str()),
        known_in_repo,
    ) {
        BinaryProvenance::Stale(reason) => Some(reason),
        BinaryProvenance::Fresh | BinaryProvenance::NotApplicable => None,
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Repository root used as the checker working directory and as the `git`
    /// context for stamping [`GateRunResult::commit`](crate::domain::GateRunResult).
    ///
    /// This is the parent of the `.jit` directory. For `InMemoryStorage` in tests
    /// the root is `"."`, whose parent is the empty path, so we fall back to the
    /// current working directory to keep the path usable.
    pub(crate) fn checker_repo_root(&self) -> std::path::PathBuf {
        self.storage
            .root()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
    }

    /// The repository's real on-disk root (parent of `.jit`), or `None` when
    /// storage names no real repository at all — e.g. `InMemoryStorage`,
    /// whose `root()` is the placeholder `"."`.
    ///
    /// Distinct from [`checker_repo_root`](Self::checker_repo_root), which
    /// falls back to the current working directory so the checker process
    /// always has SOME directory to run in. The stale-binary check (REQ-03)
    /// must never take that fallback: doing so would compare the running
    /// binary against whatever repository the test/host process happens to be
    /// executing inside, instead of staying silent for storage that names no
    /// real repository.
    pub(crate) fn real_repo_root(&self) -> Option<std::path::PathBuf> {
        self.storage
            .root()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
    }

    /// Check a single gate for an issue
    ///
    /// Runs the gate checker if it's an automated gate, updates the issue status,
    /// and returns the run result. When the checker has `pass_context: true`, builds
    /// structured context (issue data, gate definition, prompt, run history) and
    /// passes it to the checker process via a temp file.
    pub fn check_gate(&self, issue_id: &str, gate_key: &str) -> Result<GateRunResult> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Verify gate is required for this issue
        if !issue.gates_required.contains(&gate_key.to_string()) {
            anyhow::bail!(
                "Gate '{}' is not required for issue '{}'",
                gate_key,
                full_id
            );
        }

        // Load gate definition
        let registry = self.storage.load_gate_registry()?;
        let gate = registry
            .gates
            .get(gate_key)
            .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;

        // Check if gate is automated
        if gate.mode != GateMode::Auto {
            anyhow::bail!(
                "Gate '{}' is manual and cannot be automatically checked",
                gate_key
            );
        }

        // Get checker
        let checker = gate
            .checker
            .as_ref()
            .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", gate_key))?;

        // Determine working directory: repo root (parent of .jit dir).
        let repo_root = self.checker_repo_root();

        // REQ-01/02: refuse to produce a gate verdict from a binary that
        // predates, or no longer matches, the tree under review — checked
        // BEFORE the checker process is spawned, so a stale binary never
        // produces (or persists) a verdict at all. Uses `real_repo_root`, not
        // `repo_root` above: storage with no real on-disk root (e.g.
        // `InMemoryStorage`) must never compare against whatever repository
        // the host process happens to be running inside (REQ-03). See
        // `domain::build_provenance` for the identity predicate and the
        // warn-vs-fail rationale.
        if let Some(real_root) = self.real_repo_root() {
            if let Some(reason) = stale_binary_reason(&real_root) {
                return Err(
                    crate::errors::StaleBinaryError::new(&full_id, gate_key, &reason).into(),
                );
            }
        }

        let working_dir = match checker {
            crate::domain::GateChecker::Exec {
                working_dir: Some(subdir),
                ..
            } => repo_root.join(subdir),
            _ => repo_root.clone(),
        };

        // Build context if pass_context is enabled
        let context = self.build_gate_context(checker, &full_id, gate_key, gate, &repo_root)?;

        let result = gate_execution::execute_gate_checker_with_context(
            gate_key,
            &full_id,
            gate.stage,
            checker,
            &working_dir,
            context.as_ref(),
            &issue.documents,
        )?;

        // Save run result
        self.storage.save_gate_run_result(&result)?;

        // Parse the runner's actor once through the one `Assignee` path; reused
        // for both the gate state and the logged event.
        let by: Option<crate::domain::Assignee> = result
            .by
            .as_deref()
            .map(str::parse::<crate::domain::Assignee>)
            .transpose()?;

        // Update issue gate status
        let mut issue = self.storage.load_issue(&full_id)?;
        issue.gates_status.insert(
            gate_key.to_string(),
            GateState {
                status: match result.status {
                    GateRunStatus::Passed => GateStatus::Passed,
                    GateRunStatus::Failed | GateRunStatus::Error => GateStatus::Failed,
                    _ => GateStatus::Pending,
                },
                updated_by: by.clone(),
                updated_at: result.started_at,
            },
        );
        self.storage.save_issue(issue)?;

        // Log event
        let event = match result.status {
            GateRunStatus::Passed => {
                Event::new_gate_passed(full_id.clone(), gate_key.to_string(), by)
            }
            _ => Event::new_gate_failed(full_id.clone(), gate_key.to_string(), by),
        };
        self.storage.append_event(&event)?;

        Ok(result)
    }

    /// Return the most recent gate run result for a given issue and gate key.
    ///
    /// Returns `Ok(None)` if no runs have been recorded yet.
    pub fn get_last_gate_run(
        &self,
        issue_id: &str,
        gate_key: &str,
    ) -> Result<Option<GateRunResult>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut runs = self.storage.list_gate_runs_for_issue(&full_id)?;
        runs.retain(|r| r.gate_key == gate_key);
        runs.sort_by_key(|r| r.started_at);
        Ok(runs.into_iter().next_back())
    }

    /// Maximum prompt file size in bytes (~1000 lines of 80 chars).
    const MAX_PROMPT_FILE_SIZE: u64 = 100_000;

    /// Build structured context for a gate checker when `pass_context` is enabled.
    ///
    /// Returns `None` if the checker does not request context.
    fn build_gate_context(
        &self,
        checker: &crate::domain::GateChecker,
        issue_id: &str,
        gate_key: &str,
        gate: &crate::domain::Gate,
        repo_root: &std::path::Path,
    ) -> Result<Option<GateContext>> {
        let (pass_context, prompt, prompt_file) = match checker {
            crate::domain::GateChecker::Exec {
                pass_context,
                prompt,
                prompt_file,
                ..
            } => (*pass_context, prompt.as_deref(), prompt_file.as_deref()),
        };

        if !pass_context {
            return Ok(None);
        }

        // Resolve prompt: prompt_file takes precedence over inline prompt
        let resolved_prompt = if let Some(pf) = prompt_file {
            let path = repo_root.join(pf);

            // Path traversal guard: resolved path must stay within repo root
            let canonical = path
                .canonicalize()
                .with_context(|| format!("Failed to resolve prompt file: {}", path.display()))?;
            let canonical_root = repo_root
                .canonicalize()
                .with_context(|| format!("Failed to resolve repo root: {}", repo_root.display()))?;
            if !canonical.starts_with(&canonical_root) {
                anyhow::bail!("prompt_file '{}' resolves outside the repository", pf);
            }

            // Size guard
            let metadata = std::fs::metadata(&canonical).with_context(|| {
                format!("Failed to read prompt file metadata: {}", path.display())
            })?;
            if metadata.len() > Self::MAX_PROMPT_FILE_SIZE {
                anyhow::bail!(
                    "prompt_file '{}' exceeds size limit ({} bytes > {} byte limit)",
                    pf,
                    metadata.len(),
                    Self::MAX_PROMPT_FILE_SIZE
                );
            }

            Some(
                std::fs::read_to_string(&canonical)
                    .with_context(|| format!("Failed to read prompt file: {}", path.display()))?,
            )
        } else {
            prompt.map(|s| s.to_string())
        };

        // Load all runs for this issue once: they enrich the per-gate `gates`
        // array in IssueShowResponse and seed this gate's run history below.
        let all_runs = self.storage.list_gate_runs_for_issue(issue_id)?;

        // Build issue data (reuse IssueShowResponse for consistent JSON structure)
        let issue = self.storage.load_issue(issue_id)?;
        let enriched_deps = self.get_dependencies_enriched(&issue);
        let issue_response = IssueShowResponse::from_issue(issue, enriched_deps, &all_runs);
        let mut issue_json =
            serde_json::to_value(&issue_response).context("Failed to serialize issue to JSON")?;
        omit_current_gate_projection(&mut issue_json, gate_key);

        // Build gate definition JSON
        let gate_json = serde_json::json!({
            "key": gate.key,
            "title": gate.title,
            "description": gate.description,
            "stage": gate.stage,
        });

        let run_history = compact_run_history_for_context(&all_runs, gate_key);

        Ok(Some(GateContext {
            schema_version: 1,
            prompt: resolved_prompt,
            issue: issue_json,
            gate: gate_json,
            run_history,
        }))
    }

    /// Return the most recent recorded run for each automated gate on an issue.
    ///
    /// Results are ordered by gate priority, preserving insertion order for ties.
    /// The second vector contains automated gate keys that have not been run yet.
    pub fn get_last_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<(Vec<GateRunResult>, Vec<String>)> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        // Collect auto gates and sort by priority (stable sort preserves insertion order for ties)
        let mut auto_gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter_map(|key| registry.gates.get(key).map(|g| (key, g)))
            .filter(|(_, gate)| gate.mode == GateMode::Auto)
            .collect();
        auto_gates.sort_by_key(|(_, gate)| gate.priority);

        let latest_runs = self
            .storage
            .list_gate_runs_for_issue(&full_id)?
            .into_iter()
            .fold(
                HashMap::<String, GateRunResult>::new(),
                |mut latest_runs, run| {
                    match latest_runs.get(&run.gate_key) {
                        Some(existing) if existing.started_at >= run.started_at => {}
                        _ => {
                            latest_runs.insert(run.gate_key.clone(), run);
                        }
                    }
                    latest_runs
                },
            );

        let (results, not_run) = auto_gates.into_iter().fold(
            (Vec::new(), Vec::new()),
            |(mut results, mut not_run), (gate_key, _)| {
                if let Some(result) = latest_runs.get(gate_key).cloned() {
                    results.push(result);
                } else {
                    not_run.push(gate_key.clone());
                }
                (results, not_run)
            },
        );

        Ok((results, not_run))
    }

    /// Return the per-issue readiness status of every REQUIRED gate on an issue
    /// (automated AND manual), ordered by gate priority (insertion order for
    /// ties).
    ///
    /// The authoritative status is the issue's recorded `gates_status`: an auto
    /// gate's last run and a manual gate's attestation both write there. A gate
    /// with no recorded state — an auto gate never run, or a manual gate never
    /// attested — is reported as [`GateStatus::Pending`].
    ///
    /// This is the manual-aware companion to
    /// [`get_last_gate_runs_for_issue`](Self::get_last_gate_runs_for_issue),
    /// which covers automated gates only. `status-all` folds these statuses into
    /// its green/not-green readiness verdict.
    pub fn get_required_gate_statuses_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<(String, GateStatus)>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        // Order by gate priority; a required key missing from the registry sorts
        // last but is still reported (so it is never silently treated as green).
        let mut ordered: Vec<_> = issue
            .gates_required
            .iter()
            .map(|key| {
                let priority = registry.gates.get(key).map_or(u32::MAX, |g| g.priority);
                (key.clone(), priority)
            })
            .collect();
        ordered.sort_by_key(|(_, priority)| *priority);

        Ok(ordered
            .into_iter()
            .map(|(key, _)| {
                let status = issue
                    .gates_status
                    .get(&key)
                    .map_or(GateStatus::Pending, |state| state.status);
                (key, status)
            })
            .collect())
    }

    /// Run all prechecks for an issue
    ///
    /// Returns Ok(()) if all prechecks pass, Err otherwise.
    pub(crate) fn run_prechecks(&self, issue_id: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        let mut failed_gates = Vec::new();

        // Collect precheck gates and sort by priority (stable sort preserves insertion order for ties)
        let mut precheck_gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter_map(|key| registry.gates.get(key).map(|g| (key, g)))
            .filter(|(_, gate)| gate.stage == GateStage::Precheck)
            .collect();
        precheck_gates.sort_by_key(|(_, gate)| gate.priority);

        for (gate_key, gate) in precheck_gates {
            match gate.mode {
                GateMode::Auto => {
                    // Run automated precheck
                    let result = self.check_gate(&full_id, gate_key)?;
                    if result.status != GateRunStatus::Passed {
                        failed_gates.push((gate_key.clone(), result));
                    }
                }
                GateMode::Manual => {
                    // Check if manual precheck already passed
                    let gate_status = issue.gates_status.get(gate_key);
                    if !matches!(gate_status, Some(state) if state.status == GateStatus::Passed) {
                        let status = gate_status
                            .map(|state| state.status)
                            .unwrap_or(GateStatus::Pending);
                        return Err(TransitionBlockedError::gates(
                            full_id.clone(),
                            State::InProgress,
                            issue.state,
                            vec![(gate_key.clone(), status)],
                        )
                        .into());
                    }
                }
            }
        }

        if !failed_gates.is_empty() {
            return Err(TransitionBlockedError::gates(
                full_id,
                State::InProgress,
                issue.state,
                failed_gates
                    .into_iter()
                    .map(|(key, _)| (key, GateStatus::Failed))
                    .collect(),
            )
            .into());
        }

        Ok(())
    }

    /// Run all postchecks for an issue
    ///
    /// Runs all automated postchecks and auto-transitions to Done if all pass.
    pub(crate) fn run_postchecks(&self, issue_id: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.stage == GateStage::Postcheck && gate.mode == GateMode::Auto {
                    // Run automated postcheck (errors are logged but don't fail)
                    let _ = self.check_gate(&full_id, gate_key);
                }
            }
        }

        // Try to auto-transition to done
        self.auto_transition_to_done(&full_id)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_run_history_for_context, omit_current_gate_projection};
    use crate::commands::CommandExecutor;
    use crate::domain::{
        GateChecker, GateFindings, GateMode, GateRunResult, GateRunStatus, GateStage, State,
        GATE_RUN_SCHEMA_VERSION,
    };
    use crate::storage::{InMemoryStorage, IssueStore};
    use chrono::{TimeZone, Utc};
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

    fn prior_run(
        run_id: &str,
        gate_key: &str,
        started_at: i64,
        findings: Option<GateFindings>,
    ) -> GateRunResult {
        GateRunResult {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            run_id: run_id.to_string(),
            gate_key: gate_key.to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-1".to_string(),
            commit: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            status: GateRunStatus::Failed,
            started_at: Utc.timestamp_opt(started_at, 0).unwrap(),
            completed_at: Some(Utc.timestamp_opt(started_at + 1, 0).unwrap()),
            duration_ms: Some(1_000),
            exit_code: Some(1),
            stdout: format!("stdout-{run_id}"),
            stderr: format!("stderr-{run_id}"),
            command: "review".to_string(),
            by: Some("agent:reviewer".to_string()),
            message: Some("review complete".to_string()),
            findings,
        }
    }

    #[test]
    fn test_compact_run_history_selects_latest_matching_run_by_timestamp() {
        let latest = prior_run("latest", "review", 30, None);
        let runs = vec![
            latest.clone(),
            prior_run("other-gate", "cargo-ci", 40, None),
            prior_run("oldest", "review", 10, None),
            prior_run("middle", "review", 20, None),
        ];

        let compact = compact_run_history_for_context(&runs, "review");

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].run_id, latest.run_id);
        assert_eq!(compact[0].stdout, latest.stdout);
        assert!(compact[0].stderr.is_empty());
        assert_eq!(runs[0], latest, "source run must remain unchanged");
    }

    #[test]
    fn test_compact_run_history_prefers_structured_findings_over_stdout() {
        let findings = GateFindings {
            verdict: "fail".to_string(),
            summary: "one defect".to_string(),
            findings: Vec::new(),
        };
        let source = prior_run("structured", "review", 10, Some(findings.clone()));

        let compact = compact_run_history_for_context(std::slice::from_ref(&source), "review");

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].findings, Some(findings));
        assert!(compact[0].stdout.is_empty());
        assert!(compact[0].stderr.is_empty());
        assert_eq!(source.stdout, "stdout-structured");
        assert_eq!(source.stderr, "stderr-structured");
    }

    #[test]
    fn test_compact_run_history_is_empty_without_matching_runs() {
        let runs = vec![prior_run("cargo", "cargo-ci", 10, None)];

        assert!(compact_run_history_for_context(&runs, "review").is_empty());
    }

    #[test]
    fn test_omit_current_gate_projection_retains_other_gate_evidence() {
        let mut issue = serde_json::json!({
            "gates": [
                {"key": "cargo-ci", "status": "passed", "exit_code": 0},
                {"key": "review", "status": "failed", "exit_code": 1}
            ]
        });

        omit_current_gate_projection(&mut issue, "review");

        assert_eq!(
            issue["gates"],
            serde_json::json!([
                {"key": "cargo-ci", "status": "passed", "exit_code": 0}
            ])
        );
    }

    #[test]
    fn test_check_gate_automated_success() {
        let executor = setup();

        // Define an automated gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
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

        // Create issue with gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        // Check the gate
        let result = executor.check_gate(&issue_id, "test-gate").unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.exit_code, Some(0));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue.gates_status.get("test-gate").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_check_gate_automated_failure() {
        let executor = setup();

        // Define an automated gate that fails
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "failing-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "failing-gate".to_string(),
                title: "Failing Gate".to_string(),
                description: "Gate that fails".to_string(),
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

        // Create issue with gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "failing-gate".to_string())
            .unwrap();

        // Check the gate
        let result = executor.check_gate(&issue_id, "failing-gate").unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(1));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue.gates_status.get("failing-gate").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
    }

    #[test]
    fn test_check_gate_manual_not_checkable() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".to_string(),
            crate::domain::Gate {
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

        // Create issue with gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Try to check the gate - should fail
        let result = executor.check_gate(&issue_id, "manual-gate");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("manual"));
    }

    #[test]
    fn test_get_required_gate_statuses_covers_auto_and_manual() {
        use crate::domain::GateStatus;
        let executor = setup();

        // One auto gate (priority 10) and one manual gate (priority 20).
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Auto".to_string(),
                description: "Test".to_string(),
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
                priority: 10,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        registry.gates.insert(
            "manual-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "manual-gate".to_string(),
                title: "Manual".to_string(),
                description: "Test".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 20,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Before any evaluation: both required gates are pending, ordered by priority.
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".to_string(), GateStatus::Pending),
                ("manual-gate".to_string(), GateStatus::Pending),
            ]
        );

        // Running the auto gate flips only its status; the manual gate stays pending.
        executor.check_gate(&issue_id, "auto-gate").unwrap();
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".to_string(), GateStatus::Passed),
                ("manual-gate".to_string(), GateStatus::Pending),
            ]
        );

        // Attesting the manual gate makes every required gate green.
        executor
            .pass_gate(&issue_id, "manual-gate".to_string(), None, false)
            .unwrap();
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert!(statuses.iter().all(|(_, s)| *s == GateStatus::Passed));
    }

    #[test]
    fn test_get_last_gate_runs_for_issue() {
        let executor = setup();

        // Define two automated gates
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for (key, exit_code) in [("gate-1", 0), ("gate-2", 0)] {
            registry.gates.insert(
                key.to_string(),
                crate::domain::Gate {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: "Test".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: format!("exit {}", exit_code),
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
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with both gates
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-1".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-2".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-1").unwrap();
        executor.check_gate(&issue_id, "gate-2").unwrap();

        let (results, warnings) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == GateRunStatus::Passed));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_reports_not_run_gates() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["gate-1", "gate-2"] {
            registry
                .gates
                .insert(key.to_string(), make_auto_gate(key, "exit 0"));
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-1".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-2".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-1").unwrap();

        let (results, not_run) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].gate_key, "gate-1");
        assert_eq!(not_run, vec!["gate-2".to_string()]);
    }

    #[test]
    fn test_run_prechecks_before_starting_work() {
        let executor = setup();

        // Define a precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "precheck".to_string(),
                title: "Precheck".to_string(),
                description: "Precheck".to_string(),
                stage: GateStage::Precheck,
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

        // Create issue with precheck
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "precheck".to_string())
            .unwrap();

        // Try to start work - should run prechecks
        executor
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        // Verify prechecks ran and issue transitioned
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);

        let gate_state = issue.gates_status.get("precheck").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_precheck_failure_blocks_transition() {
        let executor = setup();

        // Define a failing precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck-fail".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "precheck-fail".to_string(),
                title: "Precheck".to_string(),
                description: "Precheck".to_string(),
                stage: GateStage::Precheck,
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

        // Create issue with failing precheck
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "precheck-fail".to_string())
            .unwrap();

        // Try to start work - should fail
        let result = executor.update_issue_state(&issue_id, State::InProgress);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("precheck"));

        // Verify issue didn't transition
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Ready);
    }

    #[test]
    fn test_run_postchecks_on_completion() {
        let executor = setup();

        // Define a postcheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "postcheck".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "postcheck".to_string(),
                title: "Postcheck".to_string(),
                description: "Postcheck".to_string(),
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

        // Create issue in progress with postcheck
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "postcheck".to_string())
            .unwrap();

        // Complete work
        executor
            .update_issue_state(&issue_id, State::Gated)
            .unwrap();

        // Verify postchecks ran and issue transitioned to done
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Done);

        let gate_state = issue.gates_status.get("postcheck").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_postcheck_failure_keeps_in_gated() {
        let executor = setup();

        // Define a failing postcheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "postcheck-fail".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "postcheck-fail".to_string(),
                title: "Postcheck".to_string(),
                description: "Postcheck".to_string(),
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

        // Create issue in progress with failing postcheck
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "postcheck-fail".to_string())
            .unwrap();

        // Complete work
        executor
            .update_issue_state(&issue_id, State::Gated)
            .unwrap();

        // Verify postchecks ran but issue stayed in gated
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);

        let gate_state = issue.gates_status.get("postcheck-fail").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
    }

    #[test]
    fn test_check_gate_with_pass_context_builds_context() {
        let executor = setup();

        // Define a gate with pass_context that reads the context file
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "AI-powered code review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review the implementation for correctness.".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create an issue with the gate
        let mut issue = crate::domain::Issue::new(
            "Implement feature X".to_string(),
            "Add the X feature".to_string(),
        );
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // Check the gate - should build context and pass it
        let result = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        // Parse the context JSON from stdout
        let context: serde_json::Value =
            serde_json::from_str(&result.stdout).expect("stdout should be valid context JSON");

        assert_eq!(context["schema_version"], 1);
        assert_eq!(
            context["prompt"],
            "Review the implementation for correctness."
        );
        assert_eq!(context["issue"]["title"], "Implement feature X");
        assert_eq!(context["gate"]["key"], "review");
        assert_eq!(context["gate"]["title"], "Code Review");
        assert!(
            context["issue"]["gates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|gate| gate["key"] != "review"),
            "the current gate must not receive its stale pre-run projection"
        );
        assert!(context["run_history"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_check_gate_with_prompt_file() {
        let executor = setup();

        // Write a prompt file relative to repo root
        let repo_root = executor.storage.root().parent().unwrap().to_path_buf();
        let prompt_path = repo_root.join("review-prompt.md");
        std::fs::write(
            &prompt_path,
            "You are a senior engineer. Review for security issues.",
        )
        .unwrap();

        // Define a gate with prompt_file
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "AI review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("This should be overridden by prompt_file".to_string()),
                    prompt_file: Some("review-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        // prompt_file takes precedence over inline prompt
        assert_eq!(
            context["prompt"],
            "You are a senior engineer. Review for security issues."
        );

        // Clean up
        let _ = std::fs::remove_file(&prompt_path);
    }

    #[test]
    fn test_check_gate_run_history_keeps_only_latest_legacy_run() {
        let executor = setup();

        // Define a gate with pass_context that always fails (exit 1) but outputs context
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    // Output context to stdout, emit noise to stderr, then fail.
                    // The stderr noise must NOT reappear in the next run's context.
                    command: "cat $JIT_CONTEXT_FILE; echo 'noisy diagnostic' >&2; exit 1"
                        .to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue
        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // First run - should have empty history
        let result1 = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result1.status, GateRunStatus::Failed);
        // The checker really did emit stderr (so the strip below is meaningful).
        assert!(
            result1.stderr.contains("noisy diagnostic"),
            "checker should produce stderr, got {:?}",
            result1.stderr
        );

        // stdout contains the context JSON (before the exit 1)
        let ctx1: serde_json::Value = serde_json::from_str(&result1.stdout).unwrap();
        assert_eq!(ctx1["run_history"].as_array().unwrap().len(), 0);

        // Second run - should include first run in history
        let result2 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx2: serde_json::Value = serde_json::from_str(&result2.stdout).unwrap();
        let history2 = ctx2["run_history"].as_array().unwrap();
        assert_eq!(history2.len(), 1);
        assert_eq!(history2[0]["status"], "failed");
        // stderr is stripped from the context handed to the next checker.
        assert_eq!(
            history2[0]["stderr"], "",
            "stderr must be stripped from run_history context"
        );

        // Third run still includes only the latest previous run.
        let result3 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx3: serde_json::Value = serde_json::from_str(&result3.stdout).unwrap();
        let history3 = ctx3["run_history"].as_array().unwrap();
        assert_eq!(history3.len(), 1);
        assert_eq!(history3[0]["run_id"], result2.run_id);
        assert!(!history3[0]["stdout"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_prompt_file_path_traversal_rejected() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo ok".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("../../etc/passwd".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("outside the repository"),
            "Expected path traversal error, got: {}",
            err
        );
    }

    #[test]
    fn test_run_history_is_compacted_to_one_latest_run() {
        let executor = setup();

        // Define a context-aware gate that always fails
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE; exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let mut issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // Run the gate repeatedly to exercise history compaction.
        for _ in 0..7 {
            let _ = executor.check_gate(&issue_id, "review").unwrap();
        }

        // The next run receives exactly one latest prior run.
        let result = executor.check_gate(&issue_id, "review").unwrap();
        let ctx: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        let history = ctx["run_history"].as_array().unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_prompt_file_size_limit_enforced() {
        let executor = setup();

        // Write a prompt file that exceeds the size limit (~100KB, well over 1000 lines)
        let repo_root = executor.storage.root().parent().unwrap().to_path_buf();
        let prompt_path = repo_root.join("huge-prompt.md");
        let big_content = (0..1100)
            .map(|_| "x".repeat(100))
            .collect::<Vec<_>>()
            .join("\n"); // 1100 lines of 100 chars = ~110KB
        std::fs::write(&prompt_path, &big_content).unwrap();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo ok".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("huge-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceeds"),
            "Expected size limit error, got: {}",
            err
        );

        let _ = std::fs::remove_file(&prompt_path);
    }

    #[test]
    fn test_check_gate_without_pass_context_unchanged() {
        let executor = setup();

        // Define a normal gate without pass_context
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"CTX=${JIT_CONTEXT_FILE:-unset}\"".to_string(),
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

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        // JIT_CONTEXT_FILE should not be set when pass_context is false
        assert!(result.stdout.contains("CTX=unset"));
    }

    #[test]
    fn test_check_gate_exposes_linked_documents_via_env_var() {
        // REQ-01 (jit:4af511fd): a gate checker process receives the issue's
        // linked-document list (paths + doc types + labels) via JIT_ISSUE_DOCS,
        // wired end to end through `check_gate` from `issue.documents`.
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
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

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        // Link a document via the same accessor `jit doc add` uses.
        executor
            .add_document_reference(
                &issue_id,
                "dev/active/my-plan.md",
                None,
                Some("Implementation Plan"),
                Some("design"),
                true, // skip_scan: the path need not exist on disk for this test
            )
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let docs: serde_json::Value = serde_json::from_str(result.stdout.trim())
            .expect("JIT_ISSUE_DOCS should be valid JSON");
        let entries = docs.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "dev/active/my-plan.md");
        assert_eq!(entries[0]["doc_type"], "design");
        assert_eq!(entries[0]["label"], "Implementation Plan");
    }

    #[test]
    fn test_check_gate_empty_documents_yields_empty_json_array() {
        // REQ-01 empty case: an issue with no linked documents still gets
        // JIT_ISSUE_DOCS set, to an empty JSON array (never absent/unset).
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
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

        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.stdout.trim(), "[]");
    }

    #[test]
    fn test_gate_context_includes_enriched_dependencies() {
        let executor = setup();

        // Create two dependency issues
        let dep1 = crate::domain::Issue::new(
            "Setup database schema".to_string(),
            "Create tables".to_string(),
        );
        let dep1_id = dep1.id.clone();
        executor.storage.save_issue(dep1).unwrap();

        let dep2 = crate::domain::Issue::new(
            "Implement auth module".to_string(),
            "OAuth2 flow".to_string(),
        );
        let dep2_id = dep2.id.clone();
        executor.storage.save_issue(dep2).unwrap();

        // Create main issue that depends on both
        let mut main_issue = crate::domain::Issue::new(
            "Build user dashboard".to_string(),
            "Dashboard feature".to_string(),
        );
        main_issue.state = State::InProgress;
        main_issue.dependencies.push(dep1_id.clone());
        main_issue.dependencies.push(dep2_id.clone());
        let main_id = main_issue.id.clone();
        executor.storage.save_issue(main_issue).unwrap();

        // Define a context-aware gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::domain::Gate {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review the dashboard.".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        executor.add_gate(&main_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&main_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let context: serde_json::Value =
            serde_json::from_str(&result.stdout).expect("stdout should be valid context JSON");

        let deps = context["issue"]["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 2, "Expected 2 enriched dependencies");

        let dep_titles: Vec<&str> = deps.iter().filter_map(|d| d["title"].as_str()).collect();
        assert!(dep_titles.contains(&"Setup database schema"));
        assert!(dep_titles.contains(&"Implement auth module"));
    }

    #[test]
    fn test_check_all_gates_respects_priority_order() {
        let executor = setup();

        // Define 3 auto gates with priorities 30, 10, 20
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for (key, priority) in [("gate-p30", 30u32), ("gate-p10", 10), ("gate-p20", 20)] {
            registry.gates.insert(
                key.to_string(),
                crate::domain::Gate {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: format!("Priority {}", priority),
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
                    priority,
                    reserved: HashMap::new(),
                    auto: true,
                    example_integration: None,
                },
            );
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with gates added in priority 30, 10, 20 order
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "gate-p30".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "gate-p10".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "gate-p20".to_string())
            .unwrap();

        executor.check_gate(&issue_id, "gate-p30").unwrap();
        executor.check_gate(&issue_id, "gate-p10").unwrap();
        executor.check_gate(&issue_id, "gate-p20").unwrap();

        let (results, _) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        // Results should arrive in priority order: 10, 20, 30
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "gate-p10");
        assert_eq!(results[1].gate_key, "gate-p20");
        assert_eq!(results[2].gate_key, "gate-p30");
    }

    #[test]
    fn test_check_all_gates_stable_sort_same_priority() {
        let executor = setup();

        // Define 3 auto gates all with default priority 100
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["alpha", "beta", "gamma"] {
            registry.gates.insert(
                key.to_string(),
                crate::domain::Gate {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: "Same priority".to_string(),
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
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Add gates in specific insertion order
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "alpha".to_string()).unwrap();
        executor.add_gate(&issue_id, "beta".to_string()).unwrap();
        executor.add_gate(&issue_id, "gamma".to_string()).unwrap();

        executor.check_gate(&issue_id, "alpha").unwrap();
        executor.check_gate(&issue_id, "beta").unwrap();
        executor.check_gate(&issue_id, "gamma").unwrap();

        let (results, _) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        // Same-priority gates should maintain insertion order
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "alpha");
        assert_eq!(results[1].gate_key, "beta");
        assert_eq!(results[2].gate_key, "gamma");
    }

    #[test]
    fn test_gate_priority_defaults_on_deserialization() {
        // Deserialize Gate JSON without priority field — should default to 100
        let json = r#"{
            "version": 1,
            "key": "old-gate",
            "title": "Old Gate",
            "description": "Pre-priority gate",
            "stage": "postcheck",
            "mode": "manual",
            "auto": false,
            "example_integration": null
        }"#;

        let gate: crate::domain::Gate = serde_json::from_str(json).unwrap();
        assert_eq!(gate.priority, 100);
    }

    fn make_auto_gate(key: &str, command: &str) -> crate::domain::Gate {
        crate::domain::Gate {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: command.to_string(),
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
        }
    }

    #[test]
    fn test_get_last_gate_run_returns_none_when_no_runs() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();

        let result = executor.get_last_gate_run(&issue_id, "g").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_gate_run_returns_most_recent() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();

        let first = executor.check_gate(&issue_id, "g").unwrap();
        // Small sleep so timestamps differ
        std::thread::sleep(std::time::Duration::from_millis(10));
        let second = executor.check_gate(&issue_id, "g").unwrap();

        let last = executor.get_last_gate_run(&issue_id, "g").unwrap().unwrap();
        assert_eq!(last.run_id, second.run_id);
        assert_ne!(last.run_id, first.run_id);
    }

    #[test]
    fn test_get_last_gate_run_filters_by_gate_key() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("gate-a".to_string(), make_auto_gate("gate-a", "exit 0"));
        registry
            .gates
            .insert("gate-b".to_string(), make_auto_gate("gate-b", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-a".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-b".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-a").unwrap();
        let b_run = executor.check_gate(&issue_id, "gate-b").unwrap();

        let last_a = executor
            .get_last_gate_run(&issue_id, "gate-a")
            .unwrap()
            .unwrap();
        let last_b = executor
            .get_last_gate_run(&issue_id, "gate-b")
            .unwrap()
            .unwrap();

        assert_eq!(last_a.gate_key, "gate-a");
        assert_eq!(last_b.gate_key, "gate-b");
        assert_eq!(last_b.run_id, b_run.run_id);
        assert_ne!(last_a.run_id, last_b.run_id);
    }

    #[test]
    fn test_get_last_gate_run_shows_failure_details() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "fail-gate".to_string(),
            make_auto_gate("fail-gate", "echo 'oops' && exit 1"),
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::Issue::new("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "fail-gate".to_string())
            .unwrap();

        executor.check_gate(&issue_id, "fail-gate").unwrap();

        let last = executor
            .get_last_gate_run(&issue_id, "fail-gate")
            .unwrap()
            .unwrap();
        assert_eq!(last.status, GateRunStatus::Failed);
        assert_eq!(last.exit_code, Some(1));
        assert!(
            last.stdout.contains("oops") || last.stderr.contains("oops"),
            "Expected 'oops' in output, got stdout={:?} stderr={:?}",
            last.stdout,
            last.stderr
        );
    }

    // --- Stale-binary check wiring (jit:7446af34) --------------------------
    //
    // `setup()` above uses `InMemoryStorage`, whose root is the placeholder
    // `"."`; `real_repo_root()` is always `None` there, so the stale-binary
    // check never runs — proven by every `InMemoryStorage`-backed test in
    // this file still passing unchanged. These tests instead use a real
    // `JsonFileStorage` rooted at a real (temporary) directory, so
    // `real_repo_root()` resolves and the check actually reaches git.

    /// A `JsonFileStorage`-backed executor with one automated gate `"g"`,
    /// rooted at `repo_root` (which the caller controls: git-initialized or
    /// not). Mirrors `setup()` above but with a real on-disk root.
    fn setup_at(repo_root: &std::path::Path) -> CommandExecutor<crate::storage::JsonFileStorage> {
        std::env::set_var("JIT_TEST_MODE", "1");
        let jit_root = repo_root.join(".jit");
        let storage = crate::storage::JsonFileStorage::new(&jit_root);
        storage.init().unwrap();
        std::fs::write(
            jit_root.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "echo ran"));
        executor.storage.save_gate_registry(&registry).unwrap();
        executor
    }

    /// Create an issue requiring gate `"g"` on `executor`.
    fn add_gated_issue(executor: &CommandExecutor<crate::storage::JsonFileStorage>) -> String {
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();
        issue_id
    }

    /// REQ-03: outside a git repository entirely, the check stays silent and
    /// the checker runs normally.
    #[test]
    fn test_check_gate_stays_silent_outside_git_repository() {
        let temp = tempfile::TempDir::new().unwrap();
        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let result = executor.check_gate(&issue_id, "g").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
    }

    /// REQ-03: an ordinary, unrelated git repository — one that shares no
    /// history with the running binary's build commit, exactly the "installed
    /// release validating a different repository" case — stays silent and the
    /// checker runs normally.
    #[test]
    fn test_check_gate_stays_silent_for_unrelated_git_repository() {
        let temp = tempfile::TempDir::new().unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .unwrap();
        assert!(init.success());
        let commit = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "unrelated init",
            ])
            .current_dir(temp.path())
            .status()
            .unwrap();
        assert!(commit.success());

        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let result = executor.check_gate(&issue_id, "g").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
    }

    /// Build a scratch git repository whose `HEAD` is one commit PAST the
    /// running test binary's own build commit — fetched from the real jit
    /// workspace this test binary was built from, so the REQ-03 identity
    /// predicate (the build commit is a known commit here) holds for real,
    /// entirely inside the disposable scratch repo (no ref in the real
    /// workspace is read, moved, or written).
    ///
    /// Returns `None` (the caller should skip) when the binary was built
    /// without git (`version_info().git_commit == "unknown"`, e.g. a tarball
    /// build) or any of the local git steps fails for environment reasons.
    fn scratch_repo_stale_against_own_build() -> Option<(tempfile::TempDir, String)> {
        let info = crate::build_info::version_info();
        if info.git_commit == "unknown" {
            eprintln!("SKIP: this binary was built without git; no build commit to compare");
            return None;
        }
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)?
            .to_str()?
            .to_string();

        let temp = tempfile::TempDir::new().ok()?;
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !run(&["init", "-q"]) {
            eprintln!("SKIP: git init failed in scratch repo");
            return None;
        }
        if !run(&["fetch", "-q", &workspace_root, info.git_commit]) {
            eprintln!("SKIP: git fetch of the running binary's build commit failed");
            return None;
        }
        if !run(&["checkout", "-q", "FETCH_HEAD"]) {
            eprintln!("SKIP: checkout of the fetched build commit failed");
            return None;
        }
        if !run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "advance past the build commit",
        ]) {
            eprintln!("SKIP: advancing HEAD past the build commit failed");
            return None;
        }
        Some((temp, info.git_commit.to_string()))
    }

    /// REQ-01/02: the running binary's build commit is a known commit in the
    /// repository under validation, but is no longer at `HEAD` — `check_gate`
    /// refuses BEFORE spawning the checker, and no verdict is ever produced or
    /// persisted.
    #[test]
    fn test_check_gate_refuses_stale_binary_and_persists_no_run() {
        let Some((temp, built_from)) = scratch_repo_stale_against_own_build() else {
            return;
        };

        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let err = executor.check_gate(&issue_id, "g").expect_err(
            "a binary whose build commit is a known, but no longer current, commit \
             in the repository under review must refuse to run the checker",
        );
        assert!(
            err.downcast_ref::<crate::errors::StaleBinaryError>()
                .is_some(),
            "expected StaleBinaryError, got: {err:?}"
        );
        assert!(
            err.to_string().contains(&built_from),
            "error message should name the build commit: {err}"
        );

        // No verdict was ever produced or persisted (REQ-02).
        assert!(
            executor
                .get_last_gate_run(&issue_id, "g")
                .unwrap()
                .is_none(),
            "no gate run should have been recorded for a refused, stale-binary check"
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert!(
            !issue
                .gates_status
                .get("g")
                .is_some_and(|s| s.status == crate::domain::GateStatus::Passed),
            "the gate must not have been marked passed by a refused, stale-binary check"
        );
    }
}
