//! Declarative validation engine (driven by `.jit/rules.toml` + built-in defaults).
//!
//! All issue/label validation flows through ONE declarative engine; there is no
//! longer any hard-coded `IssueValidator`. The former hard-coded checks
//! (type-label requirement, label format, namespace registry, and the
//! per-namespace value/pattern/uniqueness/required constraints) are re-expressed
//! as built-in DEFAULT rules in [`defaults`] and composed ahead of the user's
//! `.jit/rules.toml` (a0f0f342 migration; DR §8.3).
//!
//! Submodules:
//! - [`rules`] — the rule data model and `.jit/rules.toml` loader;
//! - [`defaults`] — the built-in default rule set derived from `config.toml`;
//! - [`desugar`] — shorthand assertion kinds lowered to JSON Schema;
//! - [`engine`] — compiles and caches JSON Schema validators, producing
//!   [`engine::Finding`]s;
//! - [`local`] — write-path local-rule evaluation ([`evaluate_local`]);
//! - [`graph`] — cross-issue graph-rule evaluation (validate / gate checkers);
//! - [`report`] — structured rule reports for `jit validate [--explain]`.

pub mod defaults;
pub mod desugar;
pub mod engine;
pub mod graph;
pub mod local;
pub mod report;
pub mod rules;

pub use engine::{Finding, KeywordFactory, SchemaCompileError, SchemaEngine};
pub use local::{evaluate_local, LocalEvalError, LocalEvaluation};
pub use report::{ExplainReport, ReportedFinding, RuleOutcome, RuleReport};
