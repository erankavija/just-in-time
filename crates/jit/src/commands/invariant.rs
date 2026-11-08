//! Invariant enforcement drift over repository declarations.

use super::*;
use crate::validation::drift::DriftFinding;

/// Collection key of [`InvariantCheckResult`].
pub enum FindingsKey {}

impl CollectionKey for FindingsKey {
    const KEY: &'static str = "findings";
    const SCHEMA_NAME: Option<&'static str> = Some("InvariantCheckResult");
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

impl<S: IssueStore + crate::storage::RepositoryStateStore> ItemOps<'_, S> {
    /// Check the declarations of one lock-free capture; unloadable rules remain
    /// drift.
    pub fn check_invariants(&self) -> Result<InvariantCheckResult> {
        use crate::validation::drift::{enforcement_drift, SourceState};
        use std::collections::BTreeSet;

        let layout = self.exec.require_layout()?;
        let (_, declarations) =
            with_read_session(self.exec.storage(), &layout, "invariant check", |session| {
                self.exec.validation().capture_repair_base(session)
            })?;
        let declarations = declarations?;
        let rule_names: Option<BTreeSet<&str>> = declarations.rules_loaded().then(|| {
            declarations
                .rules()
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect()
        });
        let rules_state = match &rule_names {
            Some(names) => SourceState::Loaded(names),
            None => SourceState::Unloadable,
        };
        let gates = declarations.gates().gates.keys().cloned().collect();

        Ok(InvariantCheckResult::new(enforcement_drift(
            &declarations.config().invariants.invariants,
            rules_state,
            &gates,
        )))
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
    fn seed_manual_gates(
        storage: &InMemoryStorage,
        gate_keys: &[crate::domain::identity::GateKey],
    ) {
        let mut registry = GateRegistry::default();
        for key in gate_keys {
            registry.gates.insert(
                key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
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
        let result = executor.items().check_invariants().unwrap();
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
        seed_manual_gates(&storage, &["code-review".parse().unwrap()]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.items().check_invariants().unwrap();
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
        seed_manual_gates(&storage, &["code-review".parse().unwrap()]);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                rules: Some(rules),
                invariants: Some(inv),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = executor.items().check_invariants().unwrap();
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
        let result = executor.items().check_invariants().unwrap();
        let json = serde_json::to_value(&result).unwrap();
        let findings = json["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{json}");
        assert_eq!(findings[0]["invariant_id"], "sample-invariant");
        assert_eq!(findings[0]["subject"], "@/rule/ghost");
    }
}
