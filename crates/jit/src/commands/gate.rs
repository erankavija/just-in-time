//! Quality gate operations

use super::*;
use crate::domain::GateMode;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key.clone());
            // Note: Gates don't block Ready state, only Done state
            self.storage.save_issue(&issue)?;
        }
        Ok(())
    }

    pub fn pass_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        // Check if gate is automated - if so, run the checker instead
        let registry = self.storage.load_gate_registry()?;
        if let Some(gate) = registry.gates.get(&gate_key) {
            if gate.mode == GateMode::Auto {
                // Smart behavior: auto-run the checker
                self.check_gate(&full_id, &gate_key)?;
                return Ok(());
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

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_passed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        // Check if Gated issue can now transition to Done
        self.auto_transition_to_done(&full_id)?;

        Ok(())
    }

    pub fn fail_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
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

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_failed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        Ok(())
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
        stage_str: String,
    ) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(anyhow!("Gate '{}' already exists", key));
        }

        // Parse stage string
        let stage = match stage_str.to_lowercase().as_str() {
            "precheck" => crate::domain::GateStage::Precheck,
            "postcheck" => crate::domain::GateStage::Postcheck,
            _ => {
                return Err(anyhow!(
                    "Invalid stage '{}'. Must be 'precheck' or 'postcheck'",
                    stage_str
                ))
            }
        };

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
                reserved: std::collections::HashMap::new(),
                auto,
                example_integration,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    /// Define a new gate with full control over stage, mode, and checker
    pub fn define_gate(
        &self,
        key: String,
        title: String,
        description: String,
        stage: crate::domain::GateStage,
        mode: crate::domain::GateMode,
        checker: Option<crate::domain::GateChecker>,
    ) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(anyhow!("Gate '{}' already exists", key));
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
                reserved: std::collections::HashMap::new(),
                auto: mode == crate::domain::GateMode::Auto,
                example_integration: None,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn remove_gate_definition(&self, key: &str) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if !registry.gates.contains_key(key) {
            return Err(anyhow!("Gate '{}' not found", key));
        }

        registry.gates.remove(key);
        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn show_gate_definition(&self, key: &str) -> Result<Gate> {
        let registry = self.storage.load_gate_registry()?;
        registry
            .gates
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow!("Gate '{}' not found", key))
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
                }),
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(&issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass should auto-run the checker
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
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
                }),
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(&issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass runs the checker, which fails
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
        );

        assert!(result.is_ok(), "Pass should succeed even if checker fails");

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
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with the gate
        let issue = crate::domain::Issue::new("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(&issue).unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Manual pass of manual gate should succeed
        let result = executor.pass_gate(
            &issue_id,
            "manual-gate".to_string(),
            Some("human:reviewer".to_string()),
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
