//! Just-In-Time Issue Tracker Library
//!
//! This library provides the core functionality for JIT issue tracking.
//! It is primarily used for testing, but can also be embedded in other applications.

pub mod commands;
pub mod domain;
pub mod graph;
pub mod output;
pub mod schema;
pub mod storage;
pub mod visualization;

// Re-export commonly used types
pub use commands::CommandExecutor;
pub use domain::{Issue, Priority, State};
pub use output::{ExitCode, JsonError, JsonOutput};
pub use schema::CommandSchema;
pub use storage::{InMemoryStorage, IssueStore, JsonFileStorage};

// Backwards compatibility alias
pub type Storage = JsonFileStorage;
