//! Just-In-Time Issue Tracker Library
//!
//! This library provides the core functionality for JIT issue tracking.
//! It is primarily used for testing, but can also be embedded in other applications.

pub mod agent_config;
pub mod build_info;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_manager;
#[cfg(test)]
mod crate_source_digest;
pub mod declarations;
pub mod dispatch;
pub mod document;
pub mod domain;
pub mod errors;
pub mod gate_execution;
pub mod gate_presets;
#[cfg(any(test, feature = "test-support"))]
pub mod generated_artifacts;
pub mod graph;
pub mod labels;
pub mod output;
pub mod profile;
pub mod query_engine;
pub mod repository_state;
pub mod runtime_defaults;
pub mod schema;
pub mod search;
pub mod snapshot;
pub mod storage;
// Reproducible tar headers, shared by the two archive writers and named by
// neither's vocabulary; nothing outside this crate writes a tar.
pub(crate) mod tar_format;
pub mod templates;
#[cfg(any(test, feature = "test-support"))]
pub mod test_cli;
#[cfg(any(test, feature = "test-support"))]
pub mod test_harness;
#[cfg(any(test, feature = "test-support"))]
pub mod test_lease_store;
pub mod test_taxonomy;
pub mod test_utils;
pub mod type_icons;
pub mod validation;
pub mod visualization;

// Re-export commonly used types
pub use commands::{CommandExecutor, CommitInfo};
pub use output::{ExitCode, JsonError, JsonOutput};
pub use schema::CommandSchema;
pub use storage::{InMemoryStorage, IssueStore, JsonFileStorage};
