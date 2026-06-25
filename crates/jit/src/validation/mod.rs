//! Declarative validation engine (driven by `.jit/rules.toml` + built-in defaults).
//!
//! All issue/label validation flows through ONE declarative engine; there is no
//! longer any hard-coded `IssueValidator`. `.jit/rules.toml` is the SOLE source of
//! truth (DR §8.2/§8.4): `jit init` materializes the FIXED [`defaults`] ruleset
//! into the file, and [`effective_rules`](crate::commands::CommandExecutor) builds
//! the same defaults IN MEMORY when the file is absent (no write on the read path).
//!
//! Submodules:
//! - [`rules`] — the rule data model and `.jit/rules.toml` loader;
//! - [`defaults`] — the fixed built-in default rule set derived from the namespace
//!   registry + type hierarchy;
//! - [`desugar`] — shorthand assertion kinds lowered to JSON Schema;
//! - [`engine`] — compiles and caches JSON Schema validators, producing
//!   [`engine::Finding`]s;
//! - [`local`] — write-path local-rule evaluation ([`evaluate_local`]);
//! - [`graph`] — cross-issue graph-rule evaluation (validate / gate checkers);
//! - [`invariants`] — the project-invariant registry and `.jit/invariants.toml`
//!   loader (registry-first, project-scoped);
//! - [`drift`] — the bidirectional enforcement-drift check (declaration
//!   consistency between invariants and loadable rules/gates);
//! - [`report`] — structured rule reports for `jit validate [--explain]`;
//! - [`serialize`] — render a [`rules::RuleSet`] to `rules.toml` + schema files,
//!   and the `jit init` scaffold writer ([`serialize::scaffold_default_rules`]).

pub mod defaults;
pub mod desugar;
pub mod drift;
pub mod engine;
pub mod graph;
pub mod invariants;
pub mod local;
pub mod projection;
pub mod report;
pub mod rules;
pub mod serialize;

pub use engine::{Finding, KeywordFactory, SchemaCompileError, SchemaEngine};
pub use local::{evaluate_local, LocalEvalError, LocalEvaluation};
pub use report::{ExplainReport, ReportedFinding, RuleOutcome, RuleReport};
