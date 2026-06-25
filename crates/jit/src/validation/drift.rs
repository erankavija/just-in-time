//! Bidirectional enforcement-drift check (declaration-consistency, NOT execution).
//!
//! An invariant MAY bind to the rule or gate that enforces it via its
//! `enforced-by` field. That binding is a DECLARATION: `enforced-by` (and the
//! `checker-command` escape hatch it ultimately backs) is never executed by
//! `jit validate` — the local write path explicitly skips it
//! ([`evaluate_local`](crate::validation::local::evaluate_local)) and validate has
//! no execution site. Drift is therefore checked as DECLARATION CONSISTENCY
//! between the invariant registry and the set of loadable rules/gates, in two
//! directions:
//!
//! - **declared-but-unenforced** — an invariant whose `enforced-by` names a rule
//!   or gate that is neither a loadable rule name nor a known gate key (the
//!   binding dangles).
//! - **enforced-but-undeclared** — a loadable rule or gate that NO invariant
//!   claims (no invariant's `enforced-by` references it).
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

/// Which direction an enforcement-drift finding points.
///
/// The two directions are reported distinctly so a caller can tell a dangling
/// invariant binding (declared-but-unenforced) from an unclaimed rule/gate
/// (enforced-but-undeclared) without parsing the message text.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::DriftDirection;
///
/// // The two directions are distinct values, not parsed from a message.
/// assert_ne!(
///     DriftDirection::DeclaredButUnenforced,
///     DriftDirection::EnforcedButUndeclared
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DriftDirection {
    /// An invariant's `enforced-by` names a rule/gate that does not exist (or is
    /// unloadable). The invariant claims enforcement that is not present.
    DeclaredButUnenforced,
    /// A loadable rule or gate that no invariant's `enforced-by` references. The
    /// enforcement exists but no invariant declares the property it protects.
    EnforcedButUndeclared,
}

impl DriftDirection {
    /// The kebab-case token used in human-readable messages and JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::drift::DriftDirection;
    ///
    /// assert_eq!(
    ///     DriftDirection::DeclaredButUnenforced.as_token(),
    ///     "declared-but-unenforced"
    /// );
    /// assert_eq!(
    ///     DriftDirection::EnforcedButUndeclared.as_token(),
    ///     "enforced-but-undeclared"
    /// );
    /// ```
    pub fn as_token(self) -> &'static str {
        match self {
            DriftDirection::DeclaredButUnenforced => "declared-but-unenforced",
            DriftDirection::EnforcedButUndeclared => "enforced-but-undeclared",
        }
    }
}

/// A single enforcement-drift finding in one of the two directions.
///
/// `subject` is the dangling binding for [`DriftDirection::DeclaredButUnenforced`]
/// and the unclaimed rule/gate name for [`DriftDirection::EnforcedButUndeclared`].
/// `invariant_id` is the offending invariant's self-id for the declared-but-
/// unenforced direction (whose `@/<id>` qualified id addresses it), and `None`
/// for the enforced-but-undeclared direction (which pertains to a rule/gate, not
/// an invariant).
///
/// # Examples
///
/// ```
/// use jit::validation::drift::{DriftDirection, DriftFinding};
///
/// let f = DriftFinding {
///     direction: DriftDirection::DeclaredButUnenforced,
///     invariant_id: Some("INV-01".to_string()),
///     subject: "dag-no-cycles".to_string(),
/// };
/// // The human message names the invariant and the dangling binding.
/// let msg = f.message();
/// assert!(msg.contains("INV-01"));
/// assert!(msg.contains("dag-no-cycles"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DriftFinding {
    /// Which direction the drift points.
    pub direction: DriftDirection,
    /// The offending invariant's self-id (declared-but-unenforced), else `None`.
    pub invariant_id: Option<String>,
    /// The dangling binding (declared-but-unenforced) or the unclaimed rule/gate
    /// name (enforced-but-undeclared).
    pub subject: String,
}

impl DriftFinding {
    /// Render a human-readable, direction-specific message for this finding.
    ///
    /// The message names the offending invariant (when applicable) and the
    /// subject (the dangling binding or the unclaimed rule/gate), so it stands
    /// alone in CLI output and in a [`Finding`](crate::validation::engine::Finding).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::drift::{DriftDirection, DriftFinding};
    ///
    /// let undeclared = DriftFinding {
    ///     direction: DriftDirection::EnforcedButUndeclared,
    ///     invariant_id: None,
    ///     subject: "dag-no-cycles".to_string(),
    /// };
    /// assert!(undeclared.message().contains("dag-no-cycles"));
    /// assert!(undeclared.message().contains("no invariant"));
    /// ```
    pub fn message(&self) -> String {
        match self.direction {
            DriftDirection::DeclaredButUnenforced => format!(
                "enforcement drift (declared-but-unenforced): invariant '{}' is enforced-by \
                 '{}', which is neither a loadable rule nor a known gate",
                self.invariant_id.as_deref().unwrap_or("<unknown>"),
                self.subject
            ),
            DriftDirection::EnforcedButUndeclared => format!(
                "enforcement drift (enforced-but-undeclared): rule/gate '{}' is claimed by no \
                 invariant's enforced-by",
                self.subject
            ),
        }
    }
}

/// Compute bidirectional enforcement drift between the invariant registry and the
/// declared rules/gates.
///
/// `rule_names` and `gate_keys` are the names of every LOADABLE rule and every
/// known gate key, respectively. A binding (an invariant's `enforced-by`) is
/// "enforced" iff it appears in either set. Findings are produced in two
/// directions:
///
/// - For each invariant with `enforced-by = X` where `X` is in neither set, a
///   [`DriftDirection::DeclaredButUnenforced`] finding.
/// - For each rule name or gate key that no invariant's `enforced-by` references,
///   a [`DriftDirection::EnforcedButUndeclared`] finding.
///
/// Results are deterministic: declared-but-unenforced findings follow the
/// invariants' authored order, and enforced-but-undeclared findings are sorted by
/// subject name (the inputs are unordered sets). The function is pure — it reads
/// only its arguments.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::{enforcement_drift, DriftDirection};
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
/// // Both directions fire: the binding dangles AND the real rule is unclaimed.
/// assert!(findings
///     .iter()
///     .any(|f| f.direction == DriftDirection::DeclaredButUnenforced
///         && f.subject == "missing-rule"));
/// assert!(findings
///     .iter()
///     .any(|f| f.direction == DriftDirection::EnforcedButUndeclared
///         && f.subject == "dag-no-cycles"));
/// ```
pub fn enforcement_drift(
    invariants: &[Invariant],
    rule_names: &BTreeSet<&str>,
    gate_keys: &BTreeSet<&str>,
) -> Vec<DriftFinding> {
    // The set of bindings any invariant claims (used for the reverse direction).
    let claimed: BTreeSet<&str> = invariants
        .iter()
        .filter_map(|inv| inv.enforced_by.as_deref())
        .collect();

    // declared-but-unenforced: a binding that names neither a rule nor a gate.
    let declared = invariants.iter().filter_map(|inv| {
        inv.enforced_by.as_deref().and_then(|binding| {
            let enforced = rule_names.contains(binding) || gate_keys.contains(binding);
            (!enforced).then(|| DriftFinding {
                direction: DriftDirection::DeclaredButUnenforced,
                invariant_id: Some(inv.id.clone()),
                subject: binding.to_string(),
            })
        })
    });

    // enforced-but-undeclared: a rule/gate no invariant claims. Union the two
    // name spaces (a name shared by a rule and a gate is reported once) and sort
    // for deterministic output.
    let undeclared = rule_names
        .iter()
        .chain(gate_keys.iter())
        .copied()
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .filter(|name| !claimed.contains(name))
        .map(|name| DriftFinding {
            direction: DriftDirection::EnforcedButUndeclared,
            invariant_id: None,
            subject: name.to_string(),
        });

    declared.chain(undeclared).collect()
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
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"ghost-rule\"\n");
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        let declared: Vec<_> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::DeclaredButUnenforced)
            .collect();
        assert_eq!(declared.len(), 1, "{findings:?}");
        assert_eq!(declared[0].invariant_id.as_deref(), Some("INV-01"));
        assert_eq!(declared[0].subject, "ghost-rule");
        assert!(declared[0].message().contains("INV-01"));
        assert!(declared[0].message().contains("ghost-rule"));
    }

    #[test]
    fn test_binding_to_real_rule_is_not_drift() {
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"dag-no-cycles\"\n");
        let rules: BTreeSet<&str> = ["dag-no-cycles"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        // No declared-but-unenforced (binding resolves); the rule is claimed, so
        // no enforced-but-undeclared either.
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_binding_to_real_gate_is_not_drift() {
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"code-review\"\n");
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = ["code-review"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_enforced_but_undeclared_reports_unclaimed_rule() {
        // One invariant claims rule-a; rule-b and gate-x are unclaimed.
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"rule-a\"\n");
        let rules: BTreeSet<&str> = ["rule-a", "rule-b"].into_iter().collect();
        let gates: BTreeSet<&str> = ["gate-x"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        let undeclared: Vec<_> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::EnforcedButUndeclared)
            .map(|f| f.subject.as_str())
            .collect();
        // rule-b and gate-x are unclaimed; rule-a is claimed.
        assert_eq!(undeclared, vec!["gate-x", "rule-b"], "sorted, claimed excluded");
    }

    #[test]
    fn test_advisory_invariant_with_no_binding_does_not_claim() {
        // An advisory invariant with no enforced-by claims nothing, so every
        // rule/gate is undeclared.
        let r = reg("[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"advisory\"\n");
        let rules: BTreeSet<&str> = ["only-rule"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].direction, DriftDirection::EnforcedButUndeclared);
        assert_eq!(findings[0].subject, "only-rule");
    }

    #[test]
    fn test_no_invariants_no_rules_no_gates_is_clean() {
        let findings = enforcement_drift(&[], &BTreeSet::new(), &BTreeSet::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn test_rule_and_gate_sharing_a_name_reported_once() {
        // A name present as BOTH a rule and a gate, claimed by no invariant, is
        // one finding (the union dedups).
        let rules: BTreeSet<&str> = ["shared"].into_iter().collect();
        let gates: BTreeSet<&str> = ["shared"].into_iter().collect();
        let findings = enforcement_drift(&[], &rules, &gates);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "shared");
    }
}
