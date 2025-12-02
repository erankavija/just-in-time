//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.

use jit::commands::CommandExecutor;
use jit::domain::{Issue, Priority, State};
use jit::storage::{IssueStore, JsonFileStorage};
use std::path::PathBuf;
use tempfile::TempDir;

/// Test harness that provides isolated environment for each test
pub struct TestHarness {
    _temp: TempDir,
    pub executor: CommandExecutor<JsonFileStorage>,
    pub storage: JsonFileStorage,
}

impl TestHarness {
    /// Create a new test harness with isolated storage
    pub fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path());
        storage.init().unwrap();
        let executor = CommandExecutor::new(storage.clone());
        Self {
            _temp: temp,
            executor,
            storage,
        }
    }

    /// Get the data directory path
    #[allow(dead_code)]
    pub fn data_dir(&self) -> PathBuf {
        self._temp.path().join("data")
    }

    // === Fluent API for common operations ===

    /// Create an issue with minimal parameters
    pub fn create_issue(&self, title: &str) -> String {
        self.executor
            .create_issue(title.to_string(), String::new(), Priority::Normal, vec![])
            .unwrap()
    }

    /// Create an issue with description
    #[allow(dead_code)]
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> String {
        self.executor
            .create_issue(
                title.to_string(),
                desc.to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap()
    }

    /// Create an issue with priority
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> String {
        self.executor
            .create_issue(title.to_string(), String::new(), priority, vec![])
            .unwrap()
    }

    /// Create an issue that's ready to work on
    pub fn create_ready_issue(&self, title: &str) -> String {
        let id = self.create_issue(title);
        self.executor
            .update_issue(&id, None, None, None, Some(State::Ready))
            .unwrap();
        id
    }

    /// Create an issue with gates
    #[allow(dead_code)]
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<String>) -> String {
        self.executor
            .create_issue(title.to_string(), String::new(), Priority::Normal, gates)
            .unwrap()
    }

    /// Add a gate definition to the registry
    pub fn add_gate(&self, key: &str, title: &str, description: &str, auto: bool) {
        self.executor
            .add_gate_definition(
                key.to_string(),
                title.to_string(),
                description.to_string(),
                auto,
                None,
            )
            .unwrap();
    }

    /// Get all issues
    #[allow(dead_code)]
    pub fn all_issues(&self) -> Vec<Issue> {
        self.storage.list_issues().unwrap()
    }

    /// Get issue by ID
    #[allow(dead_code)]
    pub fn get_issue(&self, id: &str) -> Issue {
        self.storage.load_issue(id).unwrap()
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
