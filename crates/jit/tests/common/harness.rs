//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.
//! Uses in-memory storage for 10-100x faster test execution.

#![allow(dead_code)]

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
        std::env::set_var("JIT_TEST_MODE", "1");

        // Session-backed declaration mutations capture config from the same
        // aggregate image as gates/events; an empty file is the minimal valid
        // repository declaration set for generic harness tests.
        storage.add_data_file("config.toml", "");
        // The store's own canonical layout, so session-backed mutations (e.g.
        // the validate-fix path) can open the in-memory mutation session. The
        // in-memory backend models its state in one aggregate map keyed by
        // virtual path and never touches these paths on the real filesystem
        // unless a case roots the store somewhere it wants read.
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
        Self { executor, storage }
    }

    /// Declare the canonical `[item_kinds]` table (the package-supplied set) in
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
            .add_data_file("config.toml", CANONICAL_ITEM_KINDS);
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
    storage.add_data_file("events.jsonl", &events);
}

/// Seed an exact authored gate registry into the in-memory aggregate.
pub(crate) fn seed_memory_gate_registry(
    storage: &InMemoryStorage,
    registry: &jit::declarations::GateRegistry,
) {
    let bytes = jit::declarations::serialize_gate_registry(registry).unwrap();
    storage.add_data_file("gates.toml", std::str::from_utf8(&bytes).unwrap());
}

/// Every gate key `template` names, on a node or an anchor.
///
/// A template's gate entries resolve against the repository's own presets and
/// its own gate registry, so a case that applies a template declares these in
/// the repository first. Reading them from the template is what keeps a changed
/// declaration from needing an edit beside it.
pub(crate) fn template_gate_keys(template: &jit::templates::GraphTemplate) -> Vec<String> {
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
pub(crate) fn seed_memory_template_gates(
    storage: &InMemoryStorage,
    template: &jit::templates::GraphTemplate,
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
pub(crate) fn seed_memory_gate_keys(storage: &InMemoryStorage, keys: &[&str]) {
    let mut registry = jit::declarations::GateRegistry::default();
    registry.gates.extend(keys.iter().map(|key| {
        (
            (*key).to_string(),
            jit::declarations::GateDefinition {
                version: 1,
                key: (*key).to_string(),
                title: format!("{key} gate"),
                description: format!("Repository-declared {key} gate"),
                stage: jit::declarations::GateStage::Postcheck,
                mode: jit::declarations::GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: std::collections::HashMap::new(),
                auto: false,
                example_integration: None,
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
