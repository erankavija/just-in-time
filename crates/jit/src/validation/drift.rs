//! Enforcement-drift check (declaration-consistency, NOT execution).
//!
//! An invariant MAY bind to the rule or gate that enforces it via its
//! `enforced-by` field. That binding is a DECLARATION: `enforced-by` (and the
//! `checker-command` escape hatch it ultimately backs) is never executed by
//! `jit validate` — the local write path explicitly skips it
//! ([`evaluate_local`](crate::validation::local::evaluate_local)) and validate has
//! no execution site. Drift is therefore checked as DECLARATION CONSISTENCY
//! between the invariant registry and the set of loadable rules/gates, in a
//! single direction:
//!
//! - **declared-but-unenforced** — an invariant whose `enforced-by` names a rule
//!   or gate that is neither a loadable rule name nor a known gate key, because
//!   the target is MISSING or because its enforcement SOURCE (the rule set or
//!   gate registry) is UNLOADABLE (REQ-01 covers both). The unloadable case is
//!   handled by [`enforcement_drift_tolerant`] / [`SourceState`].
//!
//! The reverse direction — a loadable rule or gate that NO invariant claims — is
//! deliberately NOT reported (REQ-05). Unioning every rule name and gate key and
//! treating each as something some invariant must claim produced indiscriminate
//! nag (a seed repo flagged ~18 unclaimed rules/gates) with no value, so
//! declared-but-unenforced is the SOLE drift direction.
//!
//! Recorded limitation (Decision D5): a binding that names a real-but-disabled
//! rule still reads as "enforced" here — drift is consistency of declarations,
//! not of runtime behavior.
//!
//! This module is PURE: it reads only the slices it is handed (invariants, rule
//! names, gate keys). The registries are loaded at the command/validate boundary
//! and injected, mirroring the rest of the validation engine. No kind-name
//! literal appears here.

use crate::validation::invariants::Invariant;
use std::collections::BTreeSet;

/// A single enforcement-drift finding: an invariant whose `enforced-by` binding
/// resolves to no loadable rule or gate (the sole drift direction,
/// declared-but-unenforced).
///
/// `subject` is the dangling binding and `invariant_id` is the offending
/// invariant's self-id (whose `@/<id>` qualified id addresses it).
///
/// # Examples
///
/// ```
/// use jit::validation::drift::DriftFinding;
///
/// let f = DriftFinding {
///     invariant_id: "INV-01".to_string(),
///     subject: "dag-no-cycles".to_string(),
///     unloadable: false,
/// };
/// // The human message names the invariant and the dangling binding.
/// let msg = f.message();
/// assert!(msg.contains("INV-01"));
/// assert!(msg.contains("dag-no-cycles"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DriftFinding {
    /// The offending invariant's self-id.
    pub invariant_id: String,
    /// The dangling binding: the `enforced-by` value that resolves to neither a
    /// loadable rule nor a known gate.
    pub subject: String,
    /// `true` when the binding could not be confirmed because its enforcement
    /// SOURCE (the rule set or gate registry) failed to load — distinct from a
    /// binding that names a target simply ABSENT from a source that loaded fine.
    /// The distinction is surfaced in [`message`](Self::message) so an unloadable
    /// source reads differently from a missing target (REQ-01).
    pub unloadable: bool,
}

impl DriftFinding {
    /// Render a human-readable message for this finding.
    ///
    /// The message names the offending invariant and the dangling binding, so it
    /// stands alone in CLI output and in a
    /// [`Finding`](crate::validation::engine::Finding). The unloadable case is
    /// worded distinctly from a missing target (REQ-01).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::drift::DriftFinding;
    ///
    /// let f = DriftFinding {
    ///     invariant_id: "INV-01".to_string(),
    ///     subject: "ghost-rule".to_string(),
    ///     unloadable: false,
    /// };
    /// assert!(f.message().contains("INV-01"));
    /// assert!(f.message().contains("ghost-rule"));
    /// assert!(f.message().contains("declared-but-unenforced"));
    /// ```
    pub fn message(&self) -> String {
        if self.unloadable {
            format!(
                "enforcement drift (declared-but-unenforced): invariant '{}' is enforced-by \
                 '{}', whose enforcement source (rule set or gate registry) failed to load, so \
                 the binding cannot be satisfied",
                self.invariant_id, self.subject
            )
        } else {
            format!(
                "enforcement drift (declared-but-unenforced): invariant '{}' is enforced-by \
                 '{}', which is neither a loadable rule nor a known gate",
                self.invariant_id, self.subject
            )
        }
    }
}

/// Compute enforcement drift between the invariant registry and the declared
/// rules/gates.
///
/// `rule_names` and `gate_keys` are the names of every LOADABLE rule and every
/// known gate key, respectively. A binding (an invariant's `enforced-by`) is
/// "enforced" iff it appears in either set. For each invariant with
/// `enforced-by = X` where `X` is in neither set, a
/// [declared-but-unenforced](DriftFinding) finding is produced. A loadable rule
/// or gate that no invariant claims is NOT reported (REQ-05).
///
/// Results are deterministic: findings follow the invariants' authored order.
/// The function is pure — it reads only its arguments.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::enforcement_drift;
/// use jit::validation::invariants::InvariantRegistry;
/// use std::collections::BTreeSet;
///
/// let reg = InvariantRegistry::from_toml_str(
///     "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
///      enforced-by = \"missing-rule\"\n",
/// )
/// .unwrap();
/// // A repo with one real rule and no gates.
/// let rules: BTreeSet<&str> = ["dag-no-cycles"].into_iter().collect();
/// let gates: BTreeSet<&str> = BTreeSet::new();
///
/// let findings = enforcement_drift(&reg.invariants, &rules, &gates);
/// // The dangling binding is reported; the unclaimed real rule is NOT drift.
/// assert_eq!(findings.len(), 1);
/// assert_eq!(findings[0].subject, "missing-rule");
/// ```
pub fn enforcement_drift(
    invariants: &[Invariant],
    rule_names: &BTreeSet<&str>,
    gate_keys: &BTreeSet<&str>,
) -> Vec<DriftFinding> {
    enforcement_drift_tolerant(
        invariants,
        SourceState::Loaded(rule_names),
        SourceState::Loaded(gate_keys),
    )
}

/// The load state of an enforcement SOURCE (the rule set or the gate registry).
///
/// An invariant's `enforced-by` binding can only be confirmed "enforced" against
/// a source that LOADED. When a source fails to parse it is
/// [`SourceState::Unloadable`]: its entries cannot be enumerated, so a binding
/// that is not satisfied by the OTHER (loaded) source is reported as
/// declared-but-unenforced with the `unloadable` flag set (REQ-01 covers a
/// binding naming a missing OR unloadable target).
///
/// # Examples
///
/// ```
/// use jit::validation::drift::SourceState;
/// use std::collections::BTreeSet;
///
/// let names: BTreeSet<&str> = ["dag-no-cycles"].into_iter().collect();
/// let loaded = SourceState::Loaded(&names);
/// assert!(loaded.contains("dag-no-cycles"));
/// assert!(!SourceState::Unloadable.contains("anything"));
/// ```
#[derive(Debug, Clone, Copy)]
pub enum SourceState<'a> {
    /// The source loaded; the contained set names every entry it declares.
    Loaded(&'a BTreeSet<&'a str>),
    /// The source failed to load; its entries cannot be enumerated.
    Unloadable,
}

impl SourceState<'_> {
    /// Whether the loaded source contains `name` (always `false` when unloadable).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::drift::SourceState;
    /// use std::collections::BTreeSet;
    ///
    /// let names: BTreeSet<&str> = ["g"].into_iter().collect();
    /// assert!(SourceState::Loaded(&names).contains("g"));
    /// assert!(!SourceState::Unloadable.contains("g"));
    /// ```
    pub fn contains(&self, name: &str) -> bool {
        match self {
            SourceState::Loaded(set) => set.contains(name),
            SourceState::Unloadable => false,
        }
    }
}

/// Compute enforcement drift, tolerating an UNLOADABLE rule set or gate registry
/// rather than erroring (REQ-01 "missing OR unloadable").
///
/// Semantics extend [`enforcement_drift`]: a binding is reported
/// declared-but-unenforced when it is satisfied by NEITHER source. When neither
/// source loaded, or the binding's only candidate source is unloadable, the
/// finding's [`DriftFinding::unloadable`] flag is set (worded as "source failed
/// to load"); when both candidate sources loaded but lack the target, the flag is
/// clear (worded as "missing").
///
/// A finding is flagged `unloadable` whenever at least one source is unloadable
/// AND the binding is not satisfied by a LOADED source — because then the binding
/// might have been satisfied by the source that failed to parse.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::{enforcement_drift_tolerant, SourceState};
/// use jit::validation::invariants::InvariantRegistry;
/// use std::collections::BTreeSet;
///
/// let reg = InvariantRegistry::from_toml_str(
///     "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
///      enforced-by = \"bad-rule\"\n",
/// )
/// .unwrap();
/// let gates: BTreeSet<&str> = BTreeSet::new();
/// // The rule set failed to parse; the gate registry loaded (and is empty).
/// let findings = enforcement_drift_tolerant(
///     &reg.invariants,
///     SourceState::Unloadable,
///     SourceState::Loaded(&gates),
/// );
/// let f = &findings[0];
/// assert_eq!(f.subject, "bad-rule");
/// assert!(f.unloadable, "binding into an unloadable source is flagged");
/// assert!(f.message().contains("failed to load"));
/// ```
pub fn enforcement_drift_tolerant(
    invariants: &[Invariant],
    rules: SourceState<'_>,
    gates: SourceState<'_>,
) -> Vec<DriftFinding> {
    let any_unloadable =
        matches!(rules, SourceState::Unloadable) || matches!(gates, SourceState::Unloadable);

    // declared-but-unenforced: a binding satisfied by neither source. When at
    // least one source is unloadable, an unsatisfied binding is flagged
    // `unloadable` (it might live in the source that failed to parse).
    invariants
        .iter()
        .filter_map(|inv| {
            inv.enforced_by.as_deref().and_then(|binding| {
                let enforced = rules.contains(binding) || gates.contains(binding);
                (!enforced).then(|| DriftFinding {
                    invariant_id: inv.id.clone(),
                    subject: binding.to_string(),
                    unloadable: any_unloadable,
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::invariants::InvariantRegistry;

    fn reg(toml: &str) -> InvariantRegistry {
        InvariantRegistry::from_toml_str(toml).unwrap()
    }

    #[test]
    fn test_declared_but_unenforced_reports_dangling_binding() {
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"ghost-rule\"\n",
        );
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].invariant_id, "INV-01");
        assert_eq!(findings[0].subject, "ghost-rule");
        assert!(findings[0].message().contains("INV-01"));
        assert!(findings[0].message().contains("ghost-rule"));
    }

    #[test]
    fn test_binding_to_real_rule_is_not_drift() {
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"dag-no-cycles\"\n",
        );
        let rules: BTreeSet<&str> = ["dag-no-cycles"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        // The binding resolves, so no drift.
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_binding_to_real_gate_is_not_drift() {
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"code-review\"\n",
        );
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = ["code-review"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_unclaimed_rules_and_gates_are_not_drift() {
        // A rule and a gate that NO invariant claims must NOT produce any finding:
        // the enforced-but-undeclared direction is gone (REQ-05).
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"rule-a\"\n",
        );
        let rules: BTreeSet<&str> = ["rule-a", "rule-b"].into_iter().collect();
        let gates: BTreeSet<&str> = ["gate-x"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        // rule-b and gate-x are unclaimed but reported nowhere; rule-a resolves.
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_advisory_invariant_with_no_binding_is_clean() {
        // An advisory invariant with no enforced-by has no binding to dangle, and
        // unclaimed rules are no longer drift, so the result is clean.
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"advisory\"\n");
        let rules: BTreeSet<&str> = ["only-rule"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_no_invariants_no_rules_no_gates_is_clean() {
        let findings = enforcement_drift(&[], &BTreeSet::new(), &BTreeSet::new());
        assert!(findings.is_empty());
    }

    // --- tolerant (unloadable source) -------------------------------------

    #[test]
    fn test_unloadable_rule_source_makes_binding_declared_but_unenforced() {
        // The rule set failed to parse; the binding names a rule that would have
        // lived there. It is declared-but-unenforced WITH the unloadable flag.
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"bad-rule\"\n",
        );
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift_tolerant(
            &r.invariants,
            SourceState::Unloadable,
            SourceState::Loaded(&gates),
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "bad-rule");
        assert!(findings[0].unloadable, "unloadable flag set");
        assert!(findings[0].message().contains("failed to load"));
    }

    #[test]
    fn test_unloadable_gate_source_makes_binding_declared_but_unenforced() {
        // The gate registry failed to load; a binding not satisfied by the loaded
        // rule set is declared-but-unenforced with the unloadable flag.
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"some-gate\"\n",
        );
        let rules: BTreeSet<&str> = ["a-rule"].into_iter().collect();
        let findings = enforcement_drift_tolerant(
            &r.invariants,
            SourceState::Loaded(&rules),
            SourceState::Unloadable,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "some-gate");
        assert!(findings[0].unloadable);
    }

    #[test]
    fn test_binding_satisfied_by_loaded_source_not_flagged_when_other_unloadable() {
        // Even with the gate registry unloadable, a binding satisfied by the
        // LOADED rule set is NOT drift.
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"a-rule\"\n",
        );
        let rules: BTreeSet<&str> = ["a-rule"].into_iter().collect();
        let findings = enforcement_drift_tolerant(
            &r.invariants,
            SourceState::Loaded(&rules),
            SourceState::Unloadable,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }
}
