//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

use crate::commands::CommandExecutor;
use crate::storage::{InMemoryStorage, IssueStore};
use std::sync::{Arc, Mutex};

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
                OpenRaceAction::Save(issue) => storage.save_issue(*issue).unwrap(),
                OpenRaceAction::Delete(id) => {
                    let bytes = storage
                        .read_repo_file(".jit/index.json")
                        .unwrap()
                        .expect("race fixture index exists");
                    let mut index: serde_json::Value = serde_json::from_str(&bytes).unwrap();
                    index["all_ids"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|active| active.as_str() != Some(id.as_str()));
                    index["deleted_ids"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::Value::String(id));
                    storage
                        .write_repo_file(
                            ".jit/index.json",
                            &serde_json::to_string_pretty(&index).unwrap(),
                        )
                        .unwrap();
                }
                OpenRaceAction::WriteRepoFile { path, content } => {
                    storage.write_repo_file(&path, &content).unwrap();
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
