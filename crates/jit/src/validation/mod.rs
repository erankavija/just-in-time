//! Declarative validation engine (driven by `.jit/rules.toml` + built-in defaults).
//!
//! All issue/label validation flows through ONE declarative engine; there is no
//! longer any hard-coded `IssueValidator`. `.jit/rules.toml` declares the
//! operative ruleset (DR §8.2/§8.4): `jit init` materializes the FIXED
//! [`default_ruleset`](crate::repository_state::default_ruleset) into the file, and
//! [`effective_rules`](crate::commands::CommandExecutor) builds the same defaults
//! IN MEMORY when the file is absent (no write on the read path). A scaffolded
//! file's `origin = "default"` rules are reconciled against the `config.toml`
//! registry at load, so their assertion and `namespace-unique-*` membership never
//! lag a hand edit of the registry.
//!
//! Rule declarations and parsing live in crate-root [`crate::declarations`]; the
//! default-rule family derivation and rules/schema serialization producers live in
//! crate-root [`crate::repository_state`], which this engine imports rather than
//! re-implements.
//!
//! Submodules:
//! - [`desugar`] — shorthand assertion kinds lowered to JSON Schema;
//! - [`engine`] — compiles and caches JSON Schema validators, producing
//!   [`engine::Finding`]s;
//! - [`local`] — write-path local-rule evaluation ([`evaluate_local`]);
//! - [`graph`] — cross-issue graph-rule evaluation (validate / gate checkers);
//! - [`strictness`] — the repo-wide [`Strictness`] enforcement modulator that
//!   globally widens or narrows which violations block a write or transition;
//! - [`invariants`] — the project-invariant registry and `.jit/invariants.toml`
//!   loader (registry-first, project-scoped);
//! - [`drift`] — the enforcement-drift check (declaration consistency between
//!   invariants and loadable rules/gates), reporting the sole
//!   declared-but-unenforced direction;
//! - [`report`] — structured rule reports for `jit validate [--explain]`.
//!
//! Rendering a [`RuleSet`](crate::declarations::rules::RuleSet) to `rules.toml` +
//! schema file CONTENT (no I/O) is
//! [`serialize_ruleset`](crate::repository_state::serialize_ruleset) in
//! [`crate::repository_state`]; the storage layer
//! ([`crate::storage::ruleset_store`]) persists it for `jit init`.

pub mod desugar;
pub mod drift;
pub mod engine;
pub mod graph;
pub mod invariants;
pub mod local;
pub mod project_render;
pub mod projection;
pub mod report;
pub mod repository;
pub mod rules_gates_projection;
pub mod strictness;

pub use engine::{Finding, KeywordFactory, SchemaCompileError, SchemaEngine};
pub use local::{evaluate_local, LocalEvalError, LocalEvaluation};
pub use report::{ExplainReport, ReportedFinding, RuleOutcome, RuleReport};
pub use strictness::Strictness;
