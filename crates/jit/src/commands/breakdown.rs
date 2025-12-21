//! Issue breakdown operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn breakdown_issue(
        &self,
        parent_id: &str,
        subtasks: Vec<(String, String)>,
    ) -> Result<Vec<String>> {
        // Load parent issue
        let full_parent_id = self.storage.resolve_issue_id(parent_id)?;
        let parent = self.storage.load_issue(&full_parent_id)?;
        let original_deps = parent.dependencies.clone();

        // Create subtasks with inherited priority and labels
        let mut subtask_ids = Vec::new();
        for (title, desc) in subtasks {
            let subtask_id = self.create_issue(
                title,
                desc,
                parent.priority,
                vec![],
                parent.labels.clone(), // Copy parent's labels
            )?;
            subtask_ids.push(subtask_id);
        }

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
