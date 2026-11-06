//! Test harness for in-process CLI testing
//!
//! Provides a fluent API for testing CLI commands without spawning processes.
//! Uses in-memory storage for 10-100x faster test execution.

use crate::commands::CommandExecutor;
use crate::domain::identity::GateKey;
use crate::domain::identity::IssueId;
use crate::domain::{Issue, Priority, State};

use crate::storage::{InMemoryStorage, IssueStore};
#[cfg(test)]
use proptest::prelude::*;

pub mod git;
pub mod image;

/// Generate nodes as (optional hierarchy level, dependency indices), with each
/// node depending only on lower-indexed nodes to ensure acyclicity.
#[cfg(test)]
pub(crate) fn arbitrary_dag() -> impl Strategy<Value = Vec<(Option<u8>, Vec<usize>)>> {
    (1usize..=7)
        .prop_flat_map(|n| {
            (
                Just(n),
                prop::collection::vec(0u8..5, n),
                prop::collection::vec(any::<u8>(), n),
            )
        })
        .prop_map(|(n, types, masks)| {
            (0..n)
                .map(|k| {
                    let ty = match types[k] {
                        0 => Some(1),
                        1 => Some(2),
                        2 => Some(3),
                        3 => Some(4),
                        _ => None,
                    };
                    let deps = (0..k).filter(|&i| masks[k] & (1u8 << i) != 0).collect();
                    (ty, deps)
                })
                .collect()
        })
}

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
    /// The aggregate is still in memory. A worktree-relative package location
    /// must reach a real directory, because a package is read from it.
    pub fn rooted_at(root: impl Into<std::path::PathBuf>) -> Self {
        Self::from_storage(InMemoryStorage::rooted_at(root))
    }

    fn from_storage(storage: InMemoryStorage) -> Self {
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
        let id = self
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
            .unwrap()
            .issue
            .id;
        id
    }

    /// Create an issue with description
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> IssueId {
        let id = self
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
            .unwrap()
            .issue
            .id;
        id
    }

    /// Create an issue with priority
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> IssueId {
        let id = self
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
            .unwrap()
            .issue
            .id;
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
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<GateKey>) -> IssueId {
        let id = self
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
            .unwrap()
            .issue
            .id;
        id
    }

    /// Define a postcheck gate through the production definition path.
    pub fn add_gate(
        &self,
        key: &GateKey,
        title: &str,
        description: &str,
        mode: crate::declarations::GateMode,
        checker: Option<crate::declarations::GateChecker>,
    ) {
        self.executor
            .gates()
            .define_gate(
                key,
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

/// Seed one exact repository-file precondition, at its repository-relative
/// `path`, into the in-memory aggregate.
pub fn seed_memory_repository_file(storage: &InMemoryStorage, path: &str, content: &str) {
    let layout = storage.repository_layout();
    let path = layout
        .classify_repository_relative(path)
        .expect("fixture path is canonical");
    match path.root_class() {
        crate::repository_state::RepositoryRootClass::Data => {
            storage.add_data_file(path.relative().as_path(), content)
        }
        crate::repository_state::RepositoryRootClass::Worktree => {
            storage.add_worktree_file(path.relative().as_path(), content)
        }
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
pub fn template_gate_keys(template: &crate::templates::GraphTemplate) -> Vec<GateKey> {
    template
        .requirements()
        .into_iter()
        .filter_map(|requirement| match requirement {
            crate::templates::TemplateRequirement::Gate(key) => Some(key),
            _ => None,
        })
        .collect()
}

/// Declare every gate key `template` names in the in-memory gate registry.
pub fn seed_memory_template_gates(
    storage: &InMemoryStorage,
    template: &crate::templates::GraphTemplate,
) {
    let keys = template_gate_keys(template);
    seed_memory_gate_keys(storage, &keys);
}

/// Declare `keys` as manual postcheck gates in the in-memory gate registry.
///
/// A template node's `gates` entry resolves against the repository's own gate
/// presets and its own gate registry, so a suite whose template names gate keys
/// declares them through this one seeder rather than authoring a registry per
/// case (`@/invariant/shared-test-contracts`). The whole registry is replaced,
/// so a case that needs a specific definition seeds one itself.
pub fn seed_memory_gate_keys(storage: &InMemoryStorage, keys: &[GateKey]) {
    seed_memory_gate_registry(storage, &manual_gate_registry(keys));
}

/// A gate registry declaring `keys` as manual postcheck gates.
pub fn manual_gate_registry(keys: &[GateKey]) -> crate::declarations::GateRegistry {
    let mut registry = crate::declarations::GateRegistry::default();
    registry.gates.extend(keys.iter().map(|key| {
        (
            key.clone(),
            crate::declarations::GateDefinition {
                version: 1,
                key: key.clone(),
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
    registry
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
