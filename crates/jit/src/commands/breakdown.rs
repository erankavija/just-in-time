//! Issue breakdown operations

use super::*;

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
            // Apply preset via the proper flow (registers gates, initializes status, logs events)
            for subtask_id in &subtask_ids {
                self.apply_gate_preset(subtask_id, &preset_name, None, false, false, &[])?;
            }
        } else if inherit_gates {
            // Copy parent's gates to all subtasks via add_gates (validates registry, initializes status)
            let parent_gates = parent.gates_required.clone();
            if !parent_gates.is_empty() {
                for subtask_id in &subtask_ids {
                    self.add_gates(subtask_id, &parent_gates)?;
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
