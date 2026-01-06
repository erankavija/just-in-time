//! Just-In-Time Issue Tracker Library
//!
//! This library provides the core functionality for JIT issue tracking.
//! It is primarily used for testing, but can also be embedded in other applications.

pub mod agent_config;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_manager;
pub mod document;
pub mod domain;
pub mod gate_execution;
pub mod graph;
pub mod hierarchy_templates;
pub mod labels;
pub mod output;
pub mod query;
pub mod schema;
pub mod search;
pub mod snapshot;
pub mod storage;
pub mod type_hierarchy;
pub mod validation;
pub mod visualization;

// Re-export commonly used types
pub use commands::{CommandExecutor, CommitInfo};
pub use domain::{Issue, Priority, State};
pub use output::{ExitCode, JsonError, JsonOutput};
pub use schema::CommandSchema;
pub use storage::{InMemoryStorage, IssueStore, JsonFileStorage};

// Backwards compatibility alias
pub type Storage = JsonFileStorage;
