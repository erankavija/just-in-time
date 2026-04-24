//! Storage abstraction layer for persisting issues, gates, and events.
//!
//! This module defines the `IssueStore` trait that abstracts storage operations,
//! allowing different backends (JSON files, SQLite, in-memory, etc.) to be used
//! interchangeably.

use crate::domain::{Event, Issue};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod claim_coordinator;
pub mod claims_log;
pub mod control_plane;
pub mod gate_runs;
pub mod heartbeat;
pub mod json;
pub mod lease;
pub mod lock;
pub mod lock_cleanup;
pub mod memory;
pub mod path_errors;
pub mod temp_cleanup;
pub mod worktree_identity;
pub mod worktree_paths;

// Re-export for convenience
pub use claim_coordinator::{ClaimCoordinator, Lease};
pub use json::JsonFileStorage;
pub use lock::FileLocker;
pub use path_errors::PathReadError;

#[allow(unused_imports)] // Public API used only in tests, not in binary
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
/// use jit::domain::Issue;
/// use jit::storage::{IssueStore, JsonFileStorage};
///
/// let storage = JsonFileStorage::new(".");
/// storage.init().unwrap();
///
/// let issue = Issue::new("Fix bug".to_string(), "Details".to_string());
/// storage.save_issue(issue.clone()).unwrap();
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
    /// Takes ownership of the issue and automatically updates the `updated_at`
    /// timestamp before persisting. This ensures timestamps are always current
    /// without requiring callers to remember to update them.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue cannot be serialized or persisted.
    fn save_issue(&self, issue: Issue) -> Result<()>;

    /// Load an issue by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be deserialized.
    fn load_issue(&self, id: &str) -> Result<Issue>;

    /// Resolve a partial issue ID to its full UUID.
    ///
    /// Accepts either a full UUID or a unique prefix (minimum 4 characters).
    /// Returns the full UUID if a unique match is found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::domain::Issue;
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    ///
    /// let issue = Issue::new("Fix bug".to_string(), "Details".to_string());
    /// storage.save_issue(issue.clone()).unwrap();
    ///
    /// // Can use short prefix
    /// let full_id = storage.resolve_issue_id(&issue.short_id()).unwrap();
    /// assert_eq!(full_id, issue.id);
    ///
    /// // Or full UUID
    /// let full_id = storage.resolve_issue_id(&issue.id).unwrap();
    /// assert_eq!(full_id, issue.id);
    /// ```
    ///
    /// # Errors
    ///
    /// - Prefix too short (< 4 chars): "Issue ID prefix must be at least 4 characters"
    /// - No matching issue found: "Issue not found: {prefix}"
    /// - Multiple issues match (ambiguous): "Ambiguous ID '{prefix}' matches multiple issues: ..."
    fn resolve_issue_id(&self, partial_id: &str) -> Result<String>;

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

    /// Save a gate run result.
    ///
    /// # Errors
    ///
    /// Returns an error if the result cannot be persisted.
    fn save_gate_run_result(&self, result: &crate::domain::GateRunResult) -> Result<()>;

    /// Load a gate run result by run ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the result does not exist or cannot be deserialized.
    fn load_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult>;

    /// List all gate run results for a specific issue.
    ///
    /// # Errors
    ///
    /// Returns an error if results cannot be loaded.
    fn list_gate_runs_for_issue(&self, issue_id: &str)
        -> Result<Vec<crate::domain::GateRunResult>>;

    /// Get the root directory path for this storage backend.
    ///
    /// Returns the path where configuration files are stored.
    /// For file-based storage, this is the .jit directory.
    /// For in-memory storage, this returns a temporary path.
    fn root(&self) -> &std::path::Path;

    /// List all available gate presets (builtin and custom).
    ///
    /// # Errors
    ///
    /// Returns an error if presets cannot be loaded.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jit::storage::{IssueStore, JsonFileStorage};
    /// let store = JsonFileStorage::new(".jit");
    /// let presets = store.list_gate_presets().unwrap();
    /// for p in &presets { println!("{}: {}", p.name, p.description); }
    /// ```
    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>>;

    /// Get a specific gate preset by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the preset is not found or cannot be loaded.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jit::storage::{IssueStore, JsonFileStorage};
    /// let store = JsonFileStorage::new(".jit");
    /// let preset = store.get_gate_preset("rust-tdd").unwrap();
    /// assert_eq!(preset.name, "rust-tdd");
    /// ```
    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition>;

    /// Save a custom gate preset.
    ///
    /// Uses atomic writes (temp file + rename) to prevent corruption.
    ///
    /// # Errors
    ///
    /// Returns an error if the preset is invalid or cannot be saved.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jit::storage::{IssueStore, JsonFileStorage};
    /// # use jit::gate_presets::{GatePresetDefinition, GateTemplate};
    /// # use jit::domain::{GateMode, GateStage};
    /// let store = JsonFileStorage::new(".jit");
    /// let preset = GatePresetDefinition {
    ///     name: "my-preset".to_string(),
    ///     description: "Custom workflow".to_string(),
    ///     gates: vec![],
    /// };
    /// // let path = store.save_gate_preset(&preset).unwrap();
    /// ```
    fn save_gate_preset(
        &self,
        preset: &crate::gate_presets::GatePresetDefinition,
    ) -> Result<std::path::PathBuf>;

    /// Read file bytes from the repository, optionally at a specific git commit.
    ///
    /// When `at_commit` is `None`, reads from the working tree.  Returns
    /// `(bytes, commit_label)` where `commit_label` is the short git hash when
    /// reading from git, or `"working-tree"` when reading from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist or cannot be read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::storage::{IssueStore, JsonFileStorage};
    ///
    /// let store = JsonFileStorage::new(".jit");
    ///
    /// // Read from the working tree (path is resolved relative to the repo root,
    /// // i.e. the parent of the .jit directory, regardless of process CWD):
    /// let (bytes, label) = store.read_path_bytes("README.md", None).unwrap();
    /// assert_eq!(label, "working-tree");
    ///
    /// // Read from a specific git commit:
    /// // let (bytes, hash) = store.read_path_bytes("README.md", Some("HEAD")).unwrap();
    /// ```
    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), PathReadError>;

    /// Read file content as UTF-8 text, optionally at a specific git commit.
    ///
    /// Delegates to [`IssueStore::read_path_bytes`] and decodes the result as
    /// UTF-8.  Bytes that are not valid UTF-8 are replaced with the Unicode
    /// replacement character (U+FFFD) so that text-oriented callers receive a
    /// `String` without an extra error path for encoding failures.
    ///
    /// Returns `(text, commit_label)` with the same commit-label semantics as
    /// `read_path_bytes`.
    ///
    /// # Errors
    ///
    /// Propagates `PathReadError::NotFound`, `PathReadError::CommitNotFound`,
    /// and `PathReadError::Other` from the underlying `read_path_bytes` call.
    fn read_path_text(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(String, String), PathReadError> {
        let (bytes, label) = self.read_path_bytes(path, at_commit)?;
        Ok((String::from_utf8_lossy(&bytes).into_owned(), label))
    }
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
        storage.save_issue(issue.clone()).unwrap();

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

            storage.save_issue(issue.clone()).unwrap();
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

            storage.save_issue(issue1.clone()).unwrap();
            storage.save_issue(issue2.clone()).unwrap();

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
            storage.save_issue(issue.clone()).unwrap();

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
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "A test gate".to_string(),
                stage: crate::domain::GateStage::Postcheck,
                mode: crate::domain::GateMode::Manual,
                checker: None,
                priority: 100,
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
