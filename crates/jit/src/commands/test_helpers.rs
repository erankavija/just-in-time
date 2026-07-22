//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

use crate::commands::CommandExecutor;
use crate::declarations::GateRegistry;
use crate::domain::Issue;
use crate::repository_state::RepositoryIndex;
use crate::storage::{InMemoryStorage, IssueStore};
use std::sync::{Arc, Mutex};

/// Seed one exact repository-file precondition in the aggregate memory image.
pub(crate) fn seed_repo_file(storage: &InMemoryStorage, path: &str, content: &str) {
    storage.add_repo_file(path, content);
}

/// Seed one issue record and its active index membership without invoking a
/// product mutation path. Lifecycle timestamps are preserved verbatim.
pub(crate) fn seed_issue(storage: &InMemoryStorage, issue: Issue) {
    storage.seed_issue_fixture(&issue);
}

/// Seed one exact authored gate-registry precondition.
pub(crate) fn seed_gate_registry(storage: &InMemoryStorage, registry: &GateRegistry) {
    let bytes = crate::declarations::serialize_gate_registry(registry)
        .expect("fixture gate registry serializes");
    seed_repo_file(
        storage,
        ".jit/gates.toml",
        std::str::from_utf8(&bytes).expect("gate registry TOML is UTF-8"),
    );
}

pub(crate) enum OpenRaceAction {
    Save(Box<crate::domain::Issue>),
    Delete(String),
    WriteRepoFile { path: String, content: String },
}

struct OpenRace {
    trigger: usize,
    opens: std::sync::atomic::AtomicUsize,
    storage: Mutex<Option<InMemoryStorage>>,
    action: Mutex<Option<OpenRaceAction>>,
}

impl crate::storage::TransactionFailureInjector for OpenRace {
    fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
        if point == &crate::storage::TransactionFailurePoint::RepositoryRecoveryExternal
            && self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == self.trigger
        {
            let storage = self.storage.lock().unwrap().clone().unwrap();
            match self.action.lock().unwrap().take().unwrap() {
                OpenRaceAction::Save(issue) => seed_issue(&storage, *issue),
                OpenRaceAction::Delete(id) => {
                    let bytes = storage
                        .read_repo_file(".jit/index.json")
                        .unwrap()
                        .expect("race fixture index exists");
                    let mut index = RepositoryIndex::parse(bytes.as_bytes()).unwrap();
                    index.mark_deleted(id);
                    let index = index.to_pretty_bytes().unwrap();
                    seed_repo_file(
                        &storage,
                        ".jit/index.json",
                        std::str::from_utf8(&index).unwrap(),
                    );
                }
                OpenRaceAction::WriteRepoFile { path, content } => {
                    seed_repo_file(&storage, &path, &content);
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn with_open_race(
    storage: InMemoryStorage,
    trigger: usize,
    action: OpenRaceAction,
) -> InMemoryStorage {
    let race = Arc::new(OpenRace {
        trigger,
        opens: std::sync::atomic::AtomicUsize::new(0),
        storage: Mutex::new(None),
        action: Mutex::new(Some(action)),
    });
    let raced = storage.with_repository_state_failure_view(race.clone());
    *race.storage.lock().unwrap() = Some(raced.clone());
    raced
}

/// Attach the explicit synthetic layout to an in-memory executor fixture.
pub fn memory_executor(storage: InMemoryStorage) -> CommandExecutor<InMemoryStorage> {
    if storage
        .read_repo_file(".jit/config.toml")
        .expect("memory fixture config path is valid")
        .is_none()
    {
        storage.add_repo_file(".jit/config.toml", "[worktree]\nenforce_leases = \"off\"\n");
    }
    let layout = storage.repository_layout();
    CommandExecutor::new(storage).with_layout(layout)
}

/// Create an executor with enforcement mode configured.
///
/// # Arguments
///
/// * `mode` - Enforcement mode: "strict", "warn", or "off"
pub fn setup_with_enforcement(mode: &str) -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    let config_toml = format!(
        r#"
[worktree]
enforce_leases = "{}"
"#,
        mode
    );
    storage.add_repo_file(".jit/config.toml", &config_toml);

    memory_executor(storage)
}

/// Create an executor with enforcement disabled (for backward compatibility tests).
pub fn setup() -> CommandExecutor<InMemoryStorage> {
    setup_with_enforcement("off")
}

/// Set the current agent ID via environment variable (for testing).
///
/// This allows tests to simulate a configured agent without requiring
/// ~/.config/jit/agent.toml or CLI flags.
pub fn set_test_agent(agent_id: &str) {
    std::env::set_var("JIT_AGENT_ID", agent_id);
}

/// Clear the test agent ID.
pub fn clear_test_agent() {
    std::env::remove_var("JIT_AGENT_ID");
}
