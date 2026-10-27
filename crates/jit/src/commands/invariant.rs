//! Invariant enforcement-drift command (`jit invariant check`).
//!
//! `check` computes the enforcement-drift between the invariant registry and the
//! declared rules/gates. It is a thin boundary: it pulls the effective ruleset and
//! gate registry and delegates ALL drift logic to the pure engine
//! ([`drift`](crate::validation::drift)). It owns no CLI parsing or output
//! formatting (the layer boundary in AGENTS.md "Separation of Concerns").
//!
//! Projecting the registry into documentation is no longer an invariant-specific
//! command: it is the generic [`project_render`](crate::commands::project) command
//! (`jit project render`), driven by the `[projection.*]` config.

use super::*;
use crate::validation::drift::DriftFinding;

/// Collection key of [`InvariantCheckResult`].
pub enum FindingsKey {}

impl CollectionKey for FindingsKey {
    const KEY: &'static str = "findings";
}

/// Result of a `jit invariant check` enforcement-drift run: every
/// declared-but-unenforced [`DriftFinding`], in the invariants' authored order.
/// No findings means the registry and the declared rules/gates are consistent.
pub type InvariantCheckResult = ListEnvelope<FindingsKey, DriftFinding>;

impl InvariantCheckResult {
    /// Whether any enforcement drift was found (the caller exits non-zero iff so).
    pub fn has_drift(&self) -> bool {
        !self.items.is_empty()
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Compute the enforcement-drift between the invariant registry and the
    /// declared rules/gates, returning every drift finding.
    ///
    /// Delegates to
    /// [`compute_drift_findings`](crate::commands::CommandExecutor::compute_drift_findings),
    /// the SAME tolerant drift computation the built-in `jit validate` pass
    /// ([`enforcement_drift_findings`](crate::commands::CommandExecutor::enforcement_drift_findings))
    /// reports — the sole declared-but-unenforced direction — and exits non-zero
    /// on ANY drift. A `.jit/rules.toml` (or gate registry) load failure is
    /// tolerated, NOT propagated as an `Err`: it is resolved defensively to
    /// [`SourceState::Unloadable`](crate::validation::drift::SourceState::Unloadable),
    /// so it surfaces as a declared-but-unenforced finding with
    /// [`DriftFinding::unloadable`](crate::validation::drift::DriftFinding::unloadable)
    /// set (REQ-01 "missing OR unloadable") rather than crashing the command.
    pub fn check_invariants(&self) -> Result<InvariantCheckResult> {
        // Share the SINGLE tolerant drift computation with the built-in validate
        // pass so both surfaces report identically, including the unloadable-source
        // case (REQ-01 "missing OR unloadable"): a malformed `.jit/rules.toml`
        // referenced by an `enforced-by` binding yields a declared-but-unenforced
        // finding here too, not a raw parse error.
        let findings: Vec<DriftFinding> = self.compute_drift_findings()?;
        Ok(InvariantCheckResult::new(findings))
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::test_helpers::{
        memory_executor_declaring, seed_gate_registry, MemoryDeclarations,
    };
    use crate::declarations::GateRegistry;
    use crate::storage::InMemoryStorage;

    /// Seed a gate registry holding a manual postcheck gate per key.
    fn seed_manual_gates(storage: &InMemoryStorage, gate_keys: &[&str]) {
        let mut registry = GateRegistry::default();
        for key in gate_keys {
            registry.gates.insert(
                (*key).to_string(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: (*key).to_string(),
                    title: (*key).to_string(),
                    description: String::new(),
                    stage: crate::declarations::GateStage::Postcheck,
                    mode: crate::declarations::GateMode::Manual,
                    checker: None,
                    priority: 100,
                    reserved: std::collections::HashMap::new(),
                    inputs: None,
                },
            );
        }
        seed_gate_registry(storage, &registry);
    }

    #[test]
    fn test_check_reports_declared_but_unenforced() {
        // sample-invariant binds to a rule/gate that does not exist -> the sole drift.
        let inv =
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/rule/ghost-rule\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let storage = InMemoryStorage::new();
        seed_manual_gates(&storage, &[]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.check_invariants().unwrap();
        assert!(result.has_drift());
        assert_eq!(result.items.len(), 1, "{:?}", result.items);
        assert_eq!(result.items[0].invariant_id, "sample-invariant");
        assert_eq!(result.items[0].subject, "@/rule/ghost-rule");
    }

    #[test]
    fn test_check_clean_with_unclaimed_rules_and_gates() {
        // A rule and a gate that NO invariant claims no longer drift (REQ-05): the
        // single resolving binding leaves the check clean despite the unclaimed
        // `real-rule` and `code-review` gate.
        let inv =
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/rule/real-rule\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let storage = InMemoryStorage::new();
        seed_manual_gates(&storage, &["code-review"]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.items);
    }

    #[test]
    fn test_check_clean_when_consistent() {
        // Two invariants claim exactly the one rule and the one gate present.
        let inv = "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/rule/real-rule\"\n\
                   [[invariants]]\nid = \"second-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/gate/code-review\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let storage = InMemoryStorage::new();
        seed_manual_gates(&storage, &["code-review"]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.items);
    }

    #[test]
    fn test_check_result_serializes_to_json() {
        // sample-invariant's `ghost` binding dangles -> one declared-but-unenforced finding;
        // the unclaimed real rule `other` is NOT drift (REQ-05).
        let inv =
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/rule/ghost\"\n";
        let rules = "[[rules]]\nname = \"other\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let storage = InMemoryStorage::new();
        seed_manual_gates(&storage, &[]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.check_invariants().unwrap();
        let json = serde_json::to_value(&result).unwrap();
        let findings = json["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{json}");
        assert_eq!(findings[0]["invariant_id"], "sample-invariant");
        assert_eq!(findings[0]["subject"], "@/rule/ghost");
    }
}
