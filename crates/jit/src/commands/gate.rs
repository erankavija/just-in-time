//! Quality gate operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key.clone());
            // Note: Gates don't block Ready state, only Done state
            self.storage.save_issue(&issue)?;
        }
        Ok(())
    }

    pub fn pass_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

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
        self.auto_transition_to_done(issue_id)?;

        Ok(())
    }

    pub fn fail_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
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
    ) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(anyhow!("Gate '{}' already exists", key));
        }

        registry.gates.insert(
            key.clone(),
            Gate {
                version: 1,
                key,
                title,
                description,
                stage: crate::domain::GateStage::Postcheck, // Default to postcheck for backwards compat
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
