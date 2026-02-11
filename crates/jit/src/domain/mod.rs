//! Core domain types and operations for the issue tracker.
//!
//! This module provides the domain layer containing:
//! - **types**: Core data structures (Issue, State, Priority, Gate, Event, etc.)
//! - **queries**: Pure query operations on issue collections
//!
//! The domain layer is independent of CLI orchestration and can be used
//! directly for library integration.

pub mod queries;
pub mod types;

// Re-export all types for backward compatibility
pub use types::*;
