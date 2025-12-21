//! Gate checking and execution operations

use super::*;
use crate::domain::{GateMode, GateRunResult, GateRunStatus, GateStage};
use crate::gate_execution;

impl<S: IssueStore> CommandExecutor<S> {
    /// Check a single gate for an issue
    ///
    /// Runs the gate checker if it's an automated gate, updates the issue status,
    /// and returns the run result.
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

        // Execute the checker
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
            _ => repo_root,
        };

        let result = gate_execution::execute_gate_checker(
            gate_key,
            &full_id,
            gate.stage,
            checker,
            &working_dir,
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
        self.storage.save_issue(&issue)?;

        // Log event
        let event = match result.status {
            GateRunStatus::Passed => Event::new_gate_passed(
                full_id.clone(),
                gate_key.to_string(),
                result.by.clone(),
            ),
            _ => Event::new_gate_failed(
                full_id.clone(),
                gate_key.to_string(),
                result.by.clone(),
            ),
        };
        self.storage.append_event(&event)?;

        Ok(result)
    }

    /// Check all automated gates for an issue
    ///
    /// Returns the results of all automated gate checks.
    pub fn check_all_gates(&self, issue_id: &str) -> Result<Vec<GateRunResult>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        let mut results = Vec::new();

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.mode == GateMode::Auto {
                    match self.check_gate(&full_id, gate_key) {
                        Ok(result) => results.push(result),
                        Err(e) => {
                            // Log error but continue checking other gates
                            eprintln!("Failed to check gate '{}': {}", gate_key, e);
                        }
                    }
                }
            }
        }

        Ok(results)
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
        executor.add_gate(&issue_id, "gate-1".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-2".to_string()).unwrap();

        // Check all gates
        let results = executor.check_all_gates(&issue_id).unwrap();

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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
        executor.storage.save_issue(&issue).unwrap();
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
}
