//! Core domain types and operations for the issue tracker.
//!
//! This module provides the domain layer containing:
//! - **types**: Core runtime records (Issue, State, Priority, GateState, Event, etc.);
//!   authored gate/rule declarations live in crate-root [`crate::declarations`]
//! - **projection**: Pure normalization of an Issue into the canonical validation shape
//! - **issue_id**: The id normalisation and prefix-matching rule every issue-id
//!   resolver applies
//! - **item**: Addressable structured items (qualified ids, item kinds) projected
//!   from issue descriptions
//! - **event_catalog**: The event-log tag vocabulary, each tag's association scope,
//!   and their projection into `jit --schema` and the committed events reference
//! - **gate_findings**: Pure parser extracting structured findings from checker stdout
//! - **repository_inputs**: Declared root-and-exclusion input sets and the content
//!   digest taken over one, used by gate input declarations
//! - **list_envelope**: The generic `{"count", "<collection>"}` list envelope
//! - **queries**: Pure query operations on issue collections
//! - **store_divergence**: Pure comparison naming the records two repository
//!   stores disagree about
//! - **type_taxonomy**: The taxonomy of type labels and their levels, and validation against it
//!
//! The domain layer is independent of CLI orchestration and can be used
//! directly for library integration.

pub mod artifact_classifier;
pub mod artifact_conformance;
pub mod artifact_directory;
pub mod artifact_discovery;
pub mod artifact_execution;
pub mod artifact_inventory;
pub mod artifact_plan;
pub mod event_catalog;
pub mod event_log;
pub mod gate_findings;
pub mod issue_id;
pub mod item;
pub mod key_reference;
pub mod list_envelope;
pub mod projection;
pub mod queries;
pub mod repository_inputs;
pub mod store_divergence;
pub mod type_taxonomy;
mod types;
pub mod vocabulary;

pub use types::*;
