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

/// Result of a `jit invariant check` enforcement-drift run.
///
/// Returned by [`CommandExecutor::check_invariants`] and serialized as the
/// `--json` payload: the list of [`DriftFinding`]s (each carrying the offending
/// invariant id and the dangling subject) and the total `count`. An empty
/// `findings` list means the registry and the declared rules/gates are
/// consistent.
#[derive(Debug, Serialize)]
pub struct InvariantCheckResult {
    /// Every drift finding (declared-but-unenforced, in the invariants' authored
    /// order).
    pub findings: Vec<DriftFinding>,
    /// The number of drift findings (mirrors `findings.len()`).
    pub count: usize,
}

impl InvariantCheckResult {
    /// Whether any enforcement drift was found (the caller exits non-zero iff so).
    pub fn has_drift(&self) -> bool {
        !self.findings.is_empty()
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
        Ok(InvariantCheckResult {
            count: findings.len(),
            findings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateRegistry;
    use crate::storage::{InMemoryStorage, IssueStore};

    /// Build an executor over an in-memory `.jit` carrying the given
    /// `invariants.toml`, `rules.toml`, and a gate registry with `gate_keys`.
    fn exec(
        invariants_toml: &str,
        rules_toml: &str,
        gate_keys: &[&str],
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("invariants.toml"), invariants_toml).unwrap();
        std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
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
                    auto: false,
                    example_integration: None,
                },
            );
        }
        crate::commands::test_helpers::seed_gate_registry(&storage, &registry);
        CommandExecutor::new(storage)
    }

    #[test]
    fn test_check_reports_declared_but_unenforced() {
        // sample-invariant binds to a rule/gate that does not exist -> the sole drift.
        let inv =
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"@/rule/ghost-rule\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let executor = exec(inv, rules, &[]);
        let result = executor.check_invariants().unwrap();
        assert!(result.has_drift());
        assert_eq!(result.findings.len(), 1, "{:?}", result.findings);
        assert_eq!(result.findings[0].invariant_id, "sample-invariant");
        assert_eq!(result.findings[0].subject, "@/rule/ghost-rule");
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
        let executor = exec(inv, rules, &["code-review"]);
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.findings);
        assert_eq!(result.count, 0);
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
        let executor = exec(inv, rules, &["code-review"]);
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.findings);
        assert_eq!(result.count, 0);
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
        let executor = exec(inv, rules, &[]);
        let result = executor.check_invariants().unwrap();
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["count"], result.count);
        let findings = json["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{json}");
        assert_eq!(findings[0]["invariant_id"], "sample-invariant");
        assert_eq!(findings[0]["subject"], "@/rule/ghost");
    }
}
