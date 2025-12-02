//! Storage abstraction layer for persisting issues, gates, and events.
//!
//! This module defines the `IssueStore` trait that abstracts storage operations,
//! allowing different backends (JSON files, SQLite, in-memory, etc.) to be used
//! interchangeably.

use crate::domain::{Event, Issue};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod json;
pub mod memory;

// Re-export for convenience
pub use json::JsonFileStorage;
pub use memory::InMemoryStorage;

/// Registry of all gate definitions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GateRegistry {
    /// Map of gate key to gate definition
    pub gates: HashMap<String, crate::domain::Gate>,
}

/// Trait for storage backends that persist issues, gates, and events.
///
/// This trait allows the core business logic to be decoupled from the specific
/// storage implementation. Implementations must be `Clone` to support shared
/// access patterns.
///
/// # Examples
///
/// ```no_run
/// use jit::storage::{IssueStore, JsonFileStorage};
/// use jit::domain::Issue;
///
/// let storage = JsonFileStorage::new(".");
/// storage.init().unwrap();
///
/// let issue = Issue::new("Fix bug".to_string(), "Details".to_string());
/// storage.save_issue(&issue).unwrap();
///
/// let loaded = storage.load_issue(&issue.id).unwrap();
/// assert_eq!(loaded.title, "Fix bug");
/// ```
pub trait IssueStore: Clone {
    /// Initialize the storage backend (idempotent).
    ///
    /// Creates necessary directories, files, or database tables.
    fn init(&self) -> Result<()>;

    /// Save an issue (create or update).
    ///
    /// # Errors
    ///
    /// Returns an error if the issue cannot be serialized or persisted.
    fn save_issue(&self, issue: &Issue) -> Result<()>;

    /// Load an issue by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be deserialized.
    fn load_issue(&self, id: &str) -> Result<Issue>;

    /// Delete an issue by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be deleted.
    fn delete_issue(&self, id: &str) -> Result<()>;

    /// List all issues in the repository.
    ///
    /// # Errors
    ///
    /// Returns an error if issues cannot be loaded.
    fn list_issues(&self) -> Result<Vec<Issue>>;

    /// Load the gate registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be loaded.
    fn load_gate_registry(&self) -> Result<GateRegistry>;

    /// Save the gate registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be persisted.
    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()>;

    /// Append an event to the event log.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be appended.
    fn append_event(&self, event: &Event) -> Result<()>;

    /// Read all events from the event log.
    ///
    /// # Errors
    ///
    /// Returns an error if events cannot be read.
    fn read_events(&self) -> Result<Vec<Event>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Issue, Priority, State};

    /// Test that JsonFileStorage implements IssueStore correctly
    #[test]
    fn test_json_storage_implements_trait() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp_dir.path());

        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Description".to_string());
        storage.save_issue(&issue).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.title, "Test");
        assert_eq!(loaded.description, "Description");
    }

    #[test]
    fn test_trait_save_and_load() {
        fn test_with_storage<S: IssueStore>(storage: S) {
            storage.init().unwrap();

            let mut issue = Issue::new("Trait test".to_string(), "Works".to_string());
            issue.priority = Priority::High;
            issue.state = State::Ready;

            storage.save_issue(&issue).unwrap();
            let loaded = storage.load_issue(&issue.id).unwrap();

            assert_eq!(loaded.title, issue.title);
            assert_eq!(loaded.priority, Priority::High);
            assert_eq!(loaded.state, State::Ready);
        }

        // Test with both backends
        let temp_dir = tempfile::tempdir().unwrap();
        test_with_storage(JsonFileStorage::new(temp_dir.path()));
        test_with_storage(InMemoryStorage::new());
    }

    #[test]
    fn test_trait_list_issues() {
        fn test_with_storage<S: IssueStore>(storage: S) {
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

        // Test with both backends
        let temp_dir = tempfile::tempdir().unwrap();
        test_with_storage(JsonFileStorage::new(temp_dir.path()));
        test_with_storage(InMemoryStorage::new());
    }

    #[test]
    fn test_trait_delete_issue() {
        fn test_with_storage<S: IssueStore>(storage: S) {
            storage.init().unwrap();

            let issue = Issue::new("Delete me".to_string(), "Test".to_string());
            storage.save_issue(&issue).unwrap();

            storage.delete_issue(&issue.id).unwrap();

            let result = storage.load_issue(&issue.id);
            assert!(result.is_err());
        }

        // Test with both backends
        let temp_dir = tempfile::tempdir().unwrap();
        test_with_storage(JsonFileStorage::new(temp_dir.path()));
        test_with_storage(InMemoryStorage::new());
    }

    #[test]
    fn test_trait_gate_registry() {
        fn test_with_storage<S: IssueStore>(storage: S) {
            storage.init().unwrap();

            let registry = storage.load_gate_registry().unwrap();
            assert_eq!(registry.gates.len(), 0);

            let mut new_registry = GateRegistry::default();
            let gate = crate::domain::Gate {
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "A test gate".to_string(),
                auto: false,
                example_integration: None,
            };
            new_registry.gates.insert("test-gate".to_string(), gate);

            storage.save_gate_registry(&new_registry).unwrap();

            let loaded = storage.load_gate_registry().unwrap();
            assert_eq!(loaded.gates.len(), 1);
            assert!(loaded.gates.contains_key("test-gate"));
        }

        // Test with both backends
        let temp_dir = tempfile::tempdir().unwrap();
        test_with_storage(JsonFileStorage::new(temp_dir.path()));
        test_with_storage(InMemoryStorage::new());
    }

    #[test]
    fn test_trait_event_log() {
        fn test_with_storage<S: IssueStore>(storage: S) {
            storage.init().unwrap();

            let issue = Issue::new("Event test".to_string(), "Test".to_string());
            let event = Event::new_issue_created(&issue);

            storage.append_event(&event).unwrap();

            let events = storage.read_events().unwrap();
            assert_eq!(events.len(), 1);
            // Event is an enum, check the variant
            matches!(events[0], crate::domain::Event::IssueCreated { .. });
        }

        // Test with both backends
        let temp_dir = tempfile::tempdir().unwrap();
        test_with_storage(JsonFileStorage::new(temp_dir.path()));
        test_with_storage(InMemoryStorage::new());
    }
}
