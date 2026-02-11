//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.
//! Uses in-memory storage for 10-100x faster test execution.

use jit::commands::CommandExecutor;
use jit::domain::{Issue, Priority, State};
use jit::storage::{InMemoryStorage, IssueStore};

/// Test harness that provides isolated environment for each test
pub struct TestHarness {
    pub executor: CommandExecutor<InMemoryStorage>,
    pub storage: InMemoryStorage,
}

impl TestHarness {
    /// Create a new test harness with isolated in-memory storage
    pub fn new() -> Self {
        // Disable worktree divergence checks in tests
        std::env::set_var("JIT_TEST_MODE", "1");

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        let executor = CommandExecutor::new(storage.clone());
        Self { executor, storage }
    }

    // === Fluent API for common operations ===

    /// Create an issue with minimal parameters
    pub fn create_issue(&self, title: &str) -> String {
        let (id, _) = self
            .executor
            .create_issue(
                title.to_string(),
                String::new(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        id
    }

    /// Create an issue with description
    #[allow(dead_code)]
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> String {
        let (id, _) = self
            .executor
            .create_issue(
                title.to_string(),
                desc.to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        id
    }

    /// Create an issue with priority
    #[allow(dead_code)]
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> String {
        let (id, _) = self
            .executor
            .create_issue(title.to_string(), String::new(), priority, vec![], vec![])
            .unwrap();
        id
    }

    /// Create an issue that's ready to work on
    #[allow(dead_code)]
    pub fn create_ready_issue(&self, title: &str) -> String {
        let id = self.create_issue(title);
        let _ = self
            .executor
            .update_issue(&id, None, None, None, Some(State::Ready), vec![], vec![])
            .unwrap();
        id
    }

    /// Create an issue with gates
    #[allow(dead_code)]
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<String>) -> String {
        let (id, _) = self
            .executor
            .create_issue(
                title.to_string(),
                String::new(),
                Priority::Normal,
                gates,
                vec![],
            )
            .unwrap();
        id
    }

    /// Add a gate definition to the registry
    #[allow(dead_code)]
    pub fn add_gate(&self, key: &str, title: &str, description: &str, auto: bool) {
        self.executor
            .add_gate_definition(
                key.to_string(),
                title.to_string(),
                description.to_string(),
                auto,
                None,
                "postcheck".to_string(), // Default to postcheck
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
