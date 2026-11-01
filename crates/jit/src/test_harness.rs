//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.
//! Uses in-memory storage for 10-100x faster test execution.

use crate::commands::CommandExecutor;
use crate::domain::identity::IssueId;
use crate::domain::{Issue, Priority, State};

use crate::storage::{InMemoryStorage, IssueStore};

pub mod git;

/// Test harness that provides isolated environment for each test
pub struct TestHarness {
    pub executor: CommandExecutor<InMemoryStorage>,
    pub storage: InMemoryStorage,
}

impl TestHarness {
    /// Create a new test harness with isolated in-memory storage
    pub fn new() -> Self {
        Self::from_storage(InMemoryStorage::new())
    }

    /// A harness whose in-memory store carries the synthetic layout rooted at
    /// `root`.
    ///
    /// The aggregate is still in memory. A repository whose applied-profile
    /// record names a worktree-relative package location needs that name to
    /// reach a real directory, because resolving such a record reads the
    /// package from the location it names.
    pub fn rooted_at(root: impl Into<std::path::PathBuf>) -> Self {
        Self::from_storage(InMemoryStorage::rooted_at(root))
    }

    fn from_storage(storage: InMemoryStorage) -> Self {
        // Disable worktree divergence checks in tests
        crate::test_mode::enable();

        // Product initialization seeds the repository and the store's own
        // canonical layout, so session-backed mutations (e.g. the validate-fix
        // path) can open the in-memory mutation session.
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        Self { executor, storage }
    }

    /// Declare the shipped item kinds, derived from the default package's manifest
    /// ([`crate::test_utils::shipped_item_kinds_toml`]), in this harness's repo, so item indexing and link resolution recognize the
    /// shipped kinds. The engine bakes in no kinds, so tests that exercise
    /// addressable items must opt in. Call before creating issues.
    pub fn with_item_kinds(self) -> Self {
        let item_kinds = crate::test_utils::shipped_item_kinds_toml();
        // Query paths still load declarations through ConfigManager's filesystem
        // reader, while mutation paths capture the aggregate in-memory image.
        // Keep both fixture views coherent until the read-side storage facade is
        // consolidated.
        let config_path = self
            .storage
            .repository_layout()
            .resolve(&crate::repository_state::VirtualPath::CONFIG)
            .unwrap();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(config_path, &item_kinds).unwrap();
        self.storage.add_data_file("config.toml", &item_kinds);
        self
    }

    // === Fluent API for common operations ===

    /// Create an issue with minimal parameters
    pub fn create_issue(&self, title: &str) -> IssueId {
        let (id, _) = self
            .executor
            .issues()
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
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> IssueId {
        let (id, _) = self
            .executor
            .issues()
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
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> IssueId {
        let (id, _) = self
            .executor
            .issues()
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
    pub fn create_ready_issue(&self, title: &str) -> IssueId {
        let id = self.create_issue(title);
        let _ = self
            .executor
            .issues()
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
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<String>) -> IssueId {
        let (id, _) = self
            .executor
            .issues()
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

    /// Define a postcheck gate through the production definition path.
    pub fn add_gate(
        &self,
        key: &str,
        title: &str,
        description: &str,
        mode: crate::declarations::GateMode,
        checker: Option<crate::declarations::GateChecker>,
    ) {
        self.executor
            .define_gate(
                key.to_string(),
                title.to_string(),
                description.to_string(),
                crate::declarations::GateStage::Postcheck,
                mode,
                checker,
                100,
            )
            .unwrap();
    }

    /// Get all issues
    pub fn all_issues(&self) -> Vec<Issue> {
        self.storage.list_issues().unwrap()
    }

    /// Get issue by ID
    pub fn get_issue(&self, id: &IssueId) -> Issue {
        self.storage.load_issue(id).unwrap()
    }
}

/// Seed one exact issue preimage into the in-memory aggregate without exercising
/// a repository publisher that the test is not about.
pub fn seed_memory_issue(storage: &InMemoryStorage, issue: &Issue) {
    storage.seed_issue_fixture(issue);
}

/// Seed one exact event-log preimage into the in-memory aggregate.
pub fn seed_memory_event(storage: &InMemoryStorage, event: &crate::domain::Event) {
    let mut events = storage
        .read_repo_file(".jit/events.jsonl")
        .unwrap()
        .unwrap_or_default();
    if !events.is_empty() && !events.ends_with('\n') {
        events.push('\n');
    }
    let event = crate::repository_state::serialize_event(event).unwrap();
    events.push_str(std::str::from_utf8(&event).unwrap());
    events.push('\n');
    storage.add_data_file("events.jsonl", &events);
}

/// Seed an exact authored gate registry into the in-memory aggregate.
pub fn seed_memory_gate_registry(
    storage: &InMemoryStorage,
    registry: &crate::declarations::GateRegistry,
) {
    let bytes = crate::declarations::serialize_gate_registry(registry).unwrap();
    storage.add_data_file("gates.toml", std::str::from_utf8(&bytes).unwrap());
}

/// Every gate key `template` names, on a node or an anchor.
///
/// A template's gate entries resolve against the repository's own presets and
/// its own gate registry, so a case that applies a template declares these in
/// the repository first. Reading them from the template is what keeps a changed
/// declaration from needing an edit beside it.
pub fn template_gate_keys(template: &crate::templates::GraphTemplate) -> Vec<String> {
    let mut keys: Vec<String> = template
        .nodes
        .iter()
        .flat_map(|node| node.gates.iter())
        .chain(
            template
                .anchors
                .iter()
                .flat_map(|anchor| anchor.gates.iter()),
        )
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Declare every gate key `template` names in the in-memory gate registry.
pub fn seed_memory_template_gates(
    storage: &InMemoryStorage,
    template: &crate::templates::GraphTemplate,
) {
    let keys = template_gate_keys(template);
    seed_memory_gate_keys(
        storage,
        &keys.iter().map(String::as_str).collect::<Vec<_>>(),
    );
}

/// Declare `keys` as manual postcheck gates in the in-memory gate registry.
///
/// A template node's `gates` entry resolves against the repository's own gate
/// presets and its own gate registry, so a suite whose template names gate keys
/// declares them through this one seeder rather than authoring a registry per
/// case (`@/invariant/shared-test-contracts`). The whole registry is replaced,
/// so a case that needs a specific definition seeds one itself.
pub fn seed_memory_gate_keys(storage: &InMemoryStorage, keys: &[&str]) {
    let mut registry = crate::declarations::GateRegistry::default();
    registry.gates.extend(keys.iter().map(|key| {
        (
            (*key).to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: (*key).parse().unwrap(),
                title: format!("{key} gate"),
                description: format!("Repository-declared {key} gate"),
                stage: crate::declarations::GateStage::Postcheck,
                mode: crate::declarations::GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: std::collections::HashMap::new(),
            },
        )
    }));
    seed_memory_gate_registry(storage, &registry);
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
