//! In-memory storage implementation for testing.
//!
//! This backend stores all data in RAM using HashMaps, providing 10-100x faster
//! test execution compared to JSON file I/O. Thread-safe for concurrent access.

use crate::domain::{Event, Issue};
use crate::storage::{GateRegistry, IssueStore};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory storage backend using HashMaps.
///
/// All data is stored in memory and lost when the instance is dropped.
/// Uses `Arc<Mutex<>>` for thread-safe shared interior mutability.
///
/// # Examples
///
/// ```
/// use jit::storage::{InMemoryStorage, IssueStore};
/// use jit::domain::Issue;
///
/// let storage = InMemoryStorage::new();
/// storage.init().unwrap();
///
/// let issue = Issue::new("Test".to_string(), "Description".to_string());
/// storage.save_issue(&issue).unwrap();
///
/// let loaded = storage.load_issue(&issue.id).unwrap();
/// assert_eq!(loaded.title, "Test");
/// ```
#[derive(Clone)]
#[allow(dead_code)] // Public API used only in tests, not in binary
pub struct InMemoryStorage {
    issues: Arc<Mutex<HashMap<String, Issue>>>,
    gate_registry: Arc<Mutex<GateRegistry>>,
    events: Arc<Mutex<Vec<Event>>>,
    label_namespaces: Arc<Mutex<crate::domain::LabelNamespaces>>,
    gate_runs: Arc<Mutex<HashMap<String, crate::domain::GateRunResult>>>,
}

impl InMemoryStorage {
    /// Create a new in-memory storage instance.
    #[allow(dead_code)] // Public API used only in tests, not in binary
    pub fn new() -> Self {
        Self {
            issues: Arc::new(Mutex::new(HashMap::new())),
            gate_registry: Arc::new(Mutex::new(GateRegistry::default())),
            events: Arc::new(Mutex::new(Vec::new())),
            label_namespaces: Arc::new(Mutex::new(crate::domain::LabelNamespaces::new())),
            gate_runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueStore for InMemoryStorage {
    fn init(&self) -> Result<()> {
        // Initialize with default namespaces
        let namespaces = crate::domain::LabelNamespaces::with_defaults();
        self.save_label_namespaces(&namespaces)?;
        Ok(())
    }

    fn save_issue(&self, issue: &Issue) -> Result<()> {
        self.issues
            .lock()
            .unwrap()
            .insert(issue.id.clone(), issue.clone());
        Ok(())
    }

    fn load_issue(&self, id: &str) -> Result<Issue> {
        self.issues
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Issue not found: {}", id))
    }

    fn delete_issue(&self, id: &str) -> Result<()> {
        self.issues
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| anyhow!("Issue not found: {}", id))?;
        Ok(())
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        Ok(self.issues.lock().unwrap().values().cloned().collect())
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        Ok(self.gate_registry.lock().unwrap().clone())
    }

    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        *self.gate_registry.lock().unwrap() = registry.clone();
        Ok(())
    }

    fn append_event(&self, event: &Event) -> Result<()> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        Ok(self.events.lock().unwrap().clone())
    }

    fn load_label_namespaces(&self) -> Result<crate::domain::LabelNamespaces> {
        Ok(self.label_namespaces.lock().unwrap().clone())
    }

    fn save_label_namespaces(&self, namespaces: &crate::domain::LabelNamespaces) -> Result<()> {
        *self.label_namespaces.lock().unwrap() = namespaces.clone();
        Ok(())
    }

    fn root(&self) -> &std::path::Path {
        // In-memory storage doesn't have a real root, return current directory
        std::path::Path::new(".")
    }

    fn save_gate_run_result(&self, result: &crate::domain::GateRunResult) -> Result<()> {
        self.gate_runs
            .lock()
            .unwrap()
            .insert(result.run_id.clone(), result.clone());
        Ok(())
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult> {
        self.gate_runs
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow!("Gate run '{}' not found", run_id))
    }

    fn list_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        Ok(self
            .gate_runs
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.issue_id == issue_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Gate, Priority, State};

    #[test]
    fn test_init_is_noop() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        storage.init().unwrap(); // Should be idempotent
    }

    #[test]
    fn test_save_and_load_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Description".to_string());
        storage.save_issue(&issue).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.id, issue.id);
        assert_eq!(loaded.title, "Test");
        assert_eq!(loaded.description, "Description");
    }

    #[test]
    fn test_save_updates_existing_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let mut issue = Issue::new("Original".to_string(), "Desc".to_string());
        storage.save_issue(&issue).unwrap();

        issue.title = "Updated".to_string();
        storage.save_issue(&issue).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.title, "Updated");

        // Should only have one issue
        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn test_load_nonexistent_issue_fails() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let result = storage.load_issue("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_delete_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Delete me".to_string(), "Test".to_string());
        storage.save_issue(&issue).unwrap();

        storage.delete_issue(&issue.id).unwrap();

        let result = storage.load_issue(&issue.id);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_nonexistent_issue_fails() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let result = storage.delete_issue("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_issues() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "First".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Second".to_string());

        storage.save_issue(&issue1).unwrap();
        storage.save_issue(&issue2).unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 2);

        let titles: Vec<_> = issues.iter().map(|i| i.title.as_str()).collect();
        assert!(titles.contains(&"Issue 1"));
        assert!(titles.contains(&"Issue 2"));
    }

    #[test]
    fn test_list_issues_empty() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn test_gate_registry_operations() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let registry = storage.load_gate_registry().unwrap();
        assert_eq!(registry.gates.len(), 0);

        let mut new_registry = GateRegistry::default();
        let gate = Gate {
            version: 1,
            key: "test-gate".to_string(),
            title: "Test Gate".to_string(),
            description: "A test gate".to_string(),
            stage: crate::domain::GateStage::Postcheck,
            mode: crate::domain::GateMode::Manual,
            checker: None,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
        };
        new_registry.gates.insert("test-gate".to_string(), gate);

        storage.save_gate_registry(&new_registry).unwrap();

        let loaded = storage.load_gate_registry().unwrap();
        assert_eq!(loaded.gates.len(), 1);
        assert!(loaded.gates.contains_key("test-gate"));
    }

    #[test]
    fn test_event_log_operations() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Event test".to_string(), "Test".to_string());
        let event = Event::new_issue_created(&issue);

        storage.append_event(&event).unwrap();

        let events = storage.read_events().unwrap();
        assert_eq!(events.len(), 1);
        matches!(events[0], Event::IssueCreated { .. });
    }

    #[test]
    fn test_multiple_events() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "Test".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Test".to_string());

        storage
            .append_event(&Event::new_issue_created(&issue1))
            .unwrap();
        storage
            .append_event(&Event::new_issue_created(&issue2))
            .unwrap();

        let events = storage.read_events().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_clone_shares_storage() {
        let storage1 = InMemoryStorage::new();
        storage1.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "In storage 1".to_string());
        storage1.save_issue(&issue1).unwrap();

        // Clone shares the same underlying storage (via RefCell)
        let storage2 = storage1.clone();
        let loaded = storage2.load_issue(&issue1.id).unwrap();
        assert_eq!(loaded.title, "Issue 1");

        // Verify they share the same underlying storage
        let issue2 = Issue::new("Issue 2".to_string(), "In storage 2".to_string());
        storage2.save_issue(&issue2).unwrap();

        // Both see the same data because they share the RefCell
        let issues1 = storage1.list_issues().unwrap();
        let issues2 = storage2.list_issues().unwrap();
        assert_eq!(issues1.len(), 2);
        assert_eq!(issues2.len(), 2);
    }

    #[test]
    fn test_works_with_complex_issue_state() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let mut issue = Issue::new("Complex".to_string(), "Test".to_string());
        issue.priority = Priority::Critical;
        issue.state = State::InProgress;
        issue.assignee = Some("agent:test".to_string());
        issue.dependencies = vec!["dep1".to_string(), "dep2".to_string()];
        issue.gates_required = vec!["gate1".to_string()];
        issue.context.insert("key".to_string(), "value".to_string());

        storage.save_issue(&issue).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.priority, Priority::Critical);
        assert_eq!(loaded.state, State::InProgress);
        assert_eq!(loaded.assignee, Some("agent:test".to_string()));
        assert_eq!(loaded.dependencies.len(), 2);
        assert_eq!(loaded.gates_required.len(), 1);
        assert_eq!(loaded.context.get("key").unwrap(), "value");
    }
}
