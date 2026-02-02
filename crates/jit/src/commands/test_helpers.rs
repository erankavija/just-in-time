//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

use crate::commands::CommandExecutor;
use crate::storage::{InMemoryStorage, IssueStore};

/// Create an executor with enforcement mode configured.
///
/// # Arguments
///
/// * `mode` - Enforcement mode: "strict", "warn", or "off"
pub fn setup_with_enforcement(mode: &str) -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    storage.init().unwrap();

    // Create config directory and enforcement config
    std::fs::create_dir_all(storage.root()).unwrap();
    let config_toml = format!(
        r#"
[worktree]
enforce_leases = "{}"
"#,
        mode
    );
    std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

    CommandExecutor::new(storage)
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
