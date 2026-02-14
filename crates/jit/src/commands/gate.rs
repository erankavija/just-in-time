//! Quality gate operations

use super::*;
use crate::domain::GateMode;

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
    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
        }

        let mut issue = self.storage.load_issue(&full_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key.clone());
            // Note: Gates don't block Ready state, only Done state
            self.storage.save_issue(issue)?;
        }
        Ok(())
    }

    /// Add multiple gates to an issue atomically
    pub fn add_gates(&self, issue_id: &str, gate_keys: &[String]) -> Result<GateAddResult> {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
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
            return Err(anyhow!(
                "Gates not found in registry: {}",
                not_found.join(", ")
            ));
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

        // Save issue
        self.storage.save_issue(issue)?;

        Ok(GateAddResult {
            added,
            already_exist,
        })
    }

    /// Remove multiple gates from an issue
    pub fn remove_gates(&self, issue_id: &str, gate_keys: &[String]) -> Result<GateRemoveResult> {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
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

        // Save issue
        self.storage.save_issue(issue)?;

        Ok(GateRemoveResult { removed, not_found })
    }

    pub fn pass_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
        }

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

        let issue_id = issue.id.clone();
        self.storage.save_issue(issue)?;

        // Log event
        let event = Event::new_gate_passed(issue_id, gate_key, by);
        self.storage.append_event(&event)?;

        // Check if Gated issue can now transition to Done
        self.auto_transition_to_done(&full_id)?;

        Ok(())
    }

    pub fn fail_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
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
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

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
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

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
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

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

    pub fn apply_gate_preset(
        &self,
        issue_id: &str,
        preset_name: &str,
        timeout_override: Option<u64>,
        skip_precheck: bool,
        skip_postcheck: bool,
        except_gates: &[String],
    ) -> Result<GateAddResult> {
        use crate::domain::{GateChecker, GateStage};

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Require active lease for structural operations
        if let Some(warning) = self.require_active_lease(&full_id)? {
            eprintln!("⚠️  Warning: {}", warning);
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
                .ok_or_else(|| anyhow!("Gate not found in registry: {}", gate_key))?;

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
        executor.storage.save_issue(issue).unwrap();
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
        executor.storage.save_issue(issue).unwrap();
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
        executor.storage.save_issue(issue).unwrap();
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
