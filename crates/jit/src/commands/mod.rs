//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Issue breakdown operations
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations

mod breakdown;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod graph;
mod issue;
mod labels;
mod query;
mod search;
mod validate;

// Common imports used across modules
use crate::domain::{Event, Gate, GateState, GateStatus, Issue, Priority, State};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Get reference to the storage backend
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Initialize a new jit repository in the current directory
    pub fn init(&self) -> Result<()> {
        self.storage.init()?;
        println!("Initialized jit repository");
        Ok(())
    }
}

// Helper functions for parsing command-line arguments
pub fn parse_priority(s: &str) -> Result<Priority> {
    match s.to_lowercase().as_str() {
        "low" => Ok(Priority::Low),
        "normal" => Ok(Priority::Normal),
        "high" => Ok(Priority::High),
        "critical" => Ok(Priority::Critical),
        _ => Err(anyhow!("Invalid priority: {}", s)),
    }
}

pub fn parse_state(s: &str) -> Result<State> {
    match s.to_lowercase().as_str() {
        "backlog" => Ok(State::Backlog),
        "open" => Ok(State::Backlog), // Backward compatibility alias
        "ready" => Ok(State::Ready),
        "in_progress" | "inprogress" => Ok(State::InProgress),
        "gated" => Ok(State::Gated),
        "done" => Ok(State::Done),
        "archived" => Ok(State::Archived),
        _ => Err(anyhow!("Invalid state: {}", s)),
    }
}
