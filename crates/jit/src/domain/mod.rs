//! Core domain types and operations for the issue tracker.
//!
//! This module provides the domain layer containing:
//! - **types**: Core data structures (Issue, State, Priority, Gate, Event, etc.)
//! - **projection**: Pure normalization of an Issue into the canonical validation shape
//! - **item**: Addressable structured items (qualified ids, item kinds) projected
//!   from issue descriptions
//! - **event_catalog**: The event-log tag vocabulary, each tag's association scope,
//!   and their projection into `jit --schema` and the committed events reference
//! - **gate_findings**: Pure parser extracting structured findings from checker stdout
//! - **queries**: Pure query operations on issue collections
//! - **type_taxonomy**: The taxonomy of type labels and their levels, and validation against it
//! - **graph**: Dependency graph algorithms (cycle detection, topological sort, transitive reduction)
//! - **validation**: Issue validation against configuration rules
//! - **labels**: Label parsing, matching, and validation utilities
//!
//! The domain layer is independent of CLI orchestration and can be used
//! directly for library integration.

pub mod artifact_classifier;
pub mod artifact_discovery;
pub mod artifact_execution;
pub mod artifact_inventory;
pub mod artifact_plan;
pub mod event_catalog;
pub mod gate_findings;
pub mod item;
pub mod projection;
pub mod queries;
pub mod type_taxonomy;
pub mod types;

// Re-export all types for backward compatibility
pub use types::*;

// Re-export the structured gate-findings parser and its types.
pub use gate_findings::{parse_gate_findings, GateFinding, GateFindings};

// Re-export the event-tag catalog and its projection.
pub use event_catalog::{
    event_catalog, render_event_reference, EventScope, EventTag, EventTagDoc,
    REFERENCE_PATH as EVENT_REFERENCE_PATH,
};

// Re-export the projection layer for `use jit::domain::*` ergonomics.
pub use projection::{project, ProjectedSection, Projection};

// Re-export the addressable-item model.
pub use item::{
    derive_scope_items, index_items, index_markdown_items, index_project_sources,
    is_qualified_reference, qualified_id, resolve_item_kinds, AddressableItem, ItemError, ItemKind,
    KindScope, ProjectSource, RawScopeItem, Scope,
};

// Re-export domain operations from sibling modules so that
// `use jit::domain::*` gives access to types AND key operations.
// The original modules remain at their top-level paths for backward compatibility.
pub use crate::graph::{DependencyGraph, GraphError, GraphNode};
pub use crate::labels::{
    label_matches, matches_pattern, parse_label, validate_assignee_format, validate_label,
};
