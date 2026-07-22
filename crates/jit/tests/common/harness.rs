//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.
//! Uses in-memory storage for 10-100x faster test execution.

#![allow(dead_code)]

use jit::commands::CommandExecutor;
use jit::domain::{Issue, Priority, State};
use jit::repository_state::{RepositoryLayout, RepositoryRootEvidence};
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
        // Session-backed declaration mutations capture config from the same
        // aggregate image as gates/events; an empty file is the minimal valid
        // repository declaration set for generic harness tests.
        storage.add_repo_file(".jit/config.toml", "");
        // A synthetic canonical layout so session-backed mutations (e.g. the
        // validate-fix path) can open the in-memory mutation session. The in-memory
        // backend models its state in one aggregate map keyed by virtual path and
        // never touches these paths on the real filesystem, so any valid nested
        // worktree/data layout serves.
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/jit-test-harness", "harness-worktree", true),
            RepositoryRootEvidence::new("/jit-test-harness/.jit", "harness-data", true),
        )
        .expect("synthetic harness layout is valid");
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
        Self { executor, storage }
    }

    /// Declare the canonical `[item_kinds]` table (the set `jit init` authors) in
    /// this harness's repo, so item indexing and link resolution recognize the
    /// shipped kinds. The engine bakes in no kinds, so tests that exercise
    /// addressable items must opt in. Call before creating issues.
    #[allow(dead_code)]
    pub fn with_item_kinds(self) -> Self {
        const CANONICAL_ITEM_KINDS: &str = "\
[item_kinds.requirement]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = [\"[hard]\", \"[aspirational]\"]
link-namespaces = [\"satisfies\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.decision]
section = \"decisions\"
id-pattern = \"D-[0-9]+\"
markers = []
link-namespaces = [\"per\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.risk]
section = \"risks\"
id-pattern = \"RISK-[0-9]+\"
markers = []
link-namespaces = [\"mitigates\", \"resolves\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.invariant]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }
source-of-truth = \"registry-first\"
";
        // Query paths still load declarations through ConfigManager's filesystem
        // reader, while mutation paths capture the aggregate in-memory image.
        // Keep both fixture views coherent until the read-side storage facade is
        // consolidated.
        std::fs::create_dir_all(self.storage.root()).unwrap();
        std::fs::write(
            self.storage.root().join("config.toml"),
            CANONICAL_ITEM_KINDS,
        )
        .unwrap();
        self.storage
            .add_repo_file(".jit/config.toml", CANONICAL_ITEM_KINDS);
        self
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
                None,
                None,
                false,
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
                None,
                None,
                false,
            )
            .unwrap();
        id
    }

    /// Create an issue with priority
    #[allow(dead_code)]
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> String {
        let (id, _) = self
            .executor
            .create_issue(
                title.to_string(),
                String::new(),
                priority,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        id
    }

    /// Create an issue that's ready to work on
    #[allow(dead_code)]
    pub fn create_ready_issue(&self, title: &str) -> String {
        let id = self.create_issue(title);
        let _ = self
            .executor
            .update_issue(
                &id,
                None,
                None,
                None,
                Some(State::Ready),
                vec![],
                vec![],
                None,
                None,
                false,
            )
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
                None,
                None,
                false,
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
                jit::declarations::GateStage::Postcheck,
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

/// Seed one exact issue preimage into the in-memory aggregate without exercising
/// a repository publisher that the test is not about.
pub(crate) fn seed_memory_issue(storage: &InMemoryStorage, issue: &Issue) {
    storage.seed_issue_fixture(issue);
}

/// Seed one exact event-log preimage into the in-memory aggregate.
pub(crate) fn seed_memory_event(storage: &InMemoryStorage, event: &jit::domain::Event) {
    let mut events = storage
        .read_repo_file(".jit/events.jsonl")
        .unwrap()
        .unwrap_or_default();
    if !events.is_empty() && !events.ends_with('\n') {
        events.push('\n');
    }
    let event = jit::repository_state::serialize_event(event).unwrap();
    events.push_str(std::str::from_utf8(&event).unwrap());
    events.push('\n');
    storage.add_repo_file(".jit/events.jsonl", &events);
}

/// Seed an exact authored gate registry into the in-memory aggregate.
pub(crate) fn seed_memory_gate_registry(
    storage: &InMemoryStorage,
    registry: &jit::declarations::GateRegistry,
) {
    let bytes = jit::declarations::serialize_gate_registry(registry).unwrap();
    storage.add_repo_file(".jit/gates.toml", std::str::from_utf8(&bytes).unwrap());
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
