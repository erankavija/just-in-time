//! Gate checking and execution operations

use super::*;
use crate::domain::{GateContext, GateMode, GateRunResult, GateRunStatus, GateStage};
use crate::gate_execution;
use crate::output::IssueShowResponse;

impl<S: IssueStore> CommandExecutor<S> {
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
            .ok_or_else(|| anyhow!("Gate '{}' not found in registry", gate_key))?;

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

        // Determine working directory: repo root (parent of .jit dir)
        // For InMemoryStorage in tests, parent of "." is "", so fallback to current_dir
        let repo_root = self
            .storage
            .root()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

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
        )?;

        // Save run result
        self.storage.save_gate_run_result(&result)?;

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
                updated_by: result.by.clone(),
                updated_at: result.started_at,
            },
        );
        self.storage.save_issue(issue)?;

        // Log event
        let event = match result.status {
            GateRunStatus::Passed => {
                Event::new_gate_passed(full_id.clone(), gate_key.to_string(), result.by.clone())
            }
            _ => Event::new_gate_failed(full_id.clone(), gate_key.to_string(), result.by.clone()),
        };
        self.storage.append_event(&event)?;

        Ok(result)
    }

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
            Some(
                std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read prompt file: {}", path.display()))?,
            )
        } else {
            prompt.map(|s| s.to_string())
        };

        // Build issue data (reuse IssueShowResponse for consistent JSON structure)
        let issue = self.storage.load_issue(issue_id)?;
        let issue_response = IssueShowResponse::from_issue(issue, vec![]);
        let issue_json =
            serde_json::to_value(&issue_response).context("Failed to serialize issue to JSON")?;

        // Build gate definition JSON
        let gate_json = serde_json::json!({
            "key": gate.key,
            "title": gate.title,
            "description": gate.description,
            "stage": gate.stage,
        });

        // Load run history for this gate+issue pair, sorted chronologically
        let all_runs = self.storage.list_gate_runs_for_issue(issue_id)?;
        let mut run_history: Vec<_> = all_runs
            .into_iter()
            .filter(|r| r.gate_key == gate_key)
            .collect();
        run_history.sort_by(|a, b| a.started_at.cmp(&b.started_at));

        Ok(Some(GateContext {
            schema_version: 1,
            prompt: resolved_prompt,
            issue: issue_json,
            gate: gate_json,
            run_history,
        }))
    }

    /// Check all automated gates for an issue
    ///
    /// Returns the results of all automated gate checks and any warnings.
    pub fn check_all_gates(&self, issue_id: &str) -> Result<(Vec<GateRunResult>, Vec<String>)> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        let mut results = Vec::new();
        let mut warnings = Vec::new();

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.mode == GateMode::Auto {
                    match self.check_gate(&full_id, gate_key) {
                        Ok(result) => results.push(result),
                        Err(e) => {
                            // Log error but continue checking other gates
                            warnings.push(format!("Failed to check gate '{}': {}", gate_key, e));
                        }
                    }
                }
            }
        }

        Ok((results, warnings))
    }

    /// Run all prechecks for an issue
    ///
    /// Returns Ok(()) if all prechecks pass, Err otherwise.
    pub(crate) fn run_prechecks(&self, issue_id: &str) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        let mut failed_gates = Vec::new();

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.stage == GateStage::Precheck {
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
                            if !matches!(gate_status, Some(state) if state.status == GateStatus::Passed)
                            {
                                anyhow::bail!(
                                    "Manual precheck '{}' has not been passed. Pass it first with: jit gate pass {} {}",
                                    gate_key, full_id, gate_key
                                );
                            }
                        }
                    }
                }
            }
        }

        if !failed_gates.is_empty() {
            let failures: Vec<String> = failed_gates
                .iter()
                .map(|(key, result)| {
                    format!(
                        "  ✗ {} (exit {}): {}",
                        key,
                        result
                            .exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "timeout".to_string()),
                        result.stderr.lines().next().unwrap_or_else(|| result
                            .stdout
                            .lines()
                            .next()
                            .unwrap_or(""))
                    )
                })
                .collect();

            anyhow::bail!(
                "Prechecks failed:\n{}\n\nFix the issues and try again",
                failures.join("\n")
            );
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
    use crate::commands::CommandExecutor;
    use crate::domain::{GateChecker, GateMode, GateRunStatus, GateStage, State};
    use crate::storage::{InMemoryStorage, IssueStore};
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
    fn test_check_all_gates_for_issue() {
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

        // Check all gates
        let (results, _warnings) = executor.check_all_gates(&issue_id).unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == GateRunStatus::Passed));
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
    fn test_check_gate_run_history_accumulates() {
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
                    // Output context then fail
                    command: "cat $JIT_CONTEXT_FILE; exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review".to_string()),
                    prompt_file: None,
                }),
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

        // stdout contains the context JSON (before the exit 1)
        let ctx1: serde_json::Value = serde_json::from_str(&result1.stdout).unwrap();
        assert_eq!(ctx1["run_history"].as_array().unwrap().len(), 0);

        // Second run - should include first run in history
        let result2 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx2: serde_json::Value = serde_json::from_str(&result2.stdout).unwrap();
        let history2 = ctx2["run_history"].as_array().unwrap();
        assert_eq!(history2.len(), 1);
        assert_eq!(history2[0]["status"], "failed");

        // Third run - should include both previous runs
        let result3 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx3: serde_json::Value = serde_json::from_str(&result3.stdout).unwrap();
        let history3 = ctx3["run_history"].as_array().unwrap();
        assert_eq!(history3.len(), 2);
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
}
