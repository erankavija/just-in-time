//! Core domain types and operations for the issue tracker.
//!
//! This module provides the domain layer containing:
//! - **types**: Core data structures (Issue, State, Priority, Gate, Event, etc.)
//! - **queries**: Pure query operations on issue collections
//! - **graph**: Dependency graph algorithms (cycle detection, topological sort, transitive reduction)
//! - **validation**: Issue validation against configuration rules
//! - **labels**: Label parsing, matching, and validation utilities
//!
//! The domain layer is independent of CLI orchestration and can be used
//! directly for library integration.

pub mod queries;
pub mod types;

// Re-export all types for backward compatibility
pub use types::*;

// Re-export domain operations from sibling modules so that
// `use jit::domain::*` gives access to types AND key operations.
// The original modules remain at their top-level paths for backward compatibility.
pub use crate::graph::{DependencyGraph, GraphError, GraphNode};
pub use crate::labels::{
    label_matches, matches_pattern, parse_label, validate_assignee_format, validate_label,
    validate_label_operations,
};
pub use crate::validation::IssueValidator;
