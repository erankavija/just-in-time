//! Issue breakdown operations

use super::*;
use crate::gate_presets::PresetManager;

impl<S: IssueStore> CommandExecutor<S> {
    /// Break down an issue into subtasks with optional gate handling
    pub fn breakdown_issue(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        gate_preset: Option<String>,
    ) -> Result<Vec<String>> {
        self.breakdown_issue_impl(parent_id, child_type, subtasks, gate_preset, false)
    }

    /// Break down an issue with inherited gates
    pub fn breakdown_issue_with_inherit(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        inherit_gates: bool,
    ) -> Result<Vec<String>> {
        self.breakdown_issue_impl(parent_id, child_type, subtasks, None, inherit_gates)
    }

    /// Internal implementation for breakdown with all gate options
    fn breakdown_issue_impl(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        gate_preset: Option<String>,
        inherit_gates: bool,
    ) -> Result<Vec<String>> {
        // Load parent issue
        let full_parent_id = self.storage.resolve_issue_id(parent_id)?;
        let parent = self.storage.load_issue(&full_parent_id)?;
        let original_deps = parent.dependencies.clone();

        // Transform labels: replace type: with child_type
        let mut child_labels = parent.labels.clone();
        child_labels.retain(|l| !l.starts_with("type:"));
        child_labels.push(format!("type:{}", child_type));

        // Create subtasks with transformed labels and no gates initially
        let mut subtask_ids = Vec::new();
        for (title, desc) in subtasks {
            let (subtask_id, _warnings) = self.create_issue(
                title,
                desc,
                parent.priority,
                vec![], // No gates initially
                child_labels.clone(),
            )?;
            subtask_ids.push(subtask_id);
        }

        // Apply gate option after creating all subtasks
        if let Some(preset_name) = gate_preset {
            // Apply preset to all subtasks
            let preset_manager = PresetManager::new(self.storage.root().to_path_buf())?;
            let preset = preset_manager.get_preset(&preset_name)?;

            for subtask_id in &subtask_ids {
                // Add each gate from the preset to the subtask
                for gate_spec in &preset.gates {
                    self.add_gate(subtask_id, gate_spec.key.clone())?;
                }
            }
        } else if inherit_gates {
            // Copy parent's gates to all subtasks
            for subtask_id in &subtask_ids {
                for gate_key in &parent.gates_required {
                    self.add_gate(subtask_id, gate_key.clone())?;
                }
            }
        }
        // else: no gates (default)

        // Copy parent's dependencies to each subtask
        for subtask_id in &subtask_ids {
            for dep_id in &original_deps {
                self.add_dependency(subtask_id, dep_id)?;
            }
        }

        // Make parent depend on all subtasks
        for subtask_id in &subtask_ids {
            self.add_dependency(&full_parent_id, subtask_id)?;
        }

        // Remove parent's original dependencies (now transitive through subtasks)
        for dep_id in &original_deps {
            self.remove_dependency(&full_parent_id, dep_id)?;
        }

        Ok(subtask_ids)
    }
}
