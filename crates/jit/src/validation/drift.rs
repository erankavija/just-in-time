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
//!   or gate that is neither a loadable rule name nor a known gate key, because
//!   the target is MISSING or because its enforcement SOURCE (the rule set or
//!   gate registry) is UNLOADABLE (REQ-01 covers both). The unloadable case is
//!   handled by [`enforcement_drift_tolerant`] / [`SourceState`].
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
///     unloadable: false,
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
    /// For [`DriftDirection::DeclaredButUnenforced`] only: `true` when the binding
    /// could not be confirmed because its enforcement SOURCE (the rule set or gate
    /// registry) failed to load — distinct from a binding that names a target
    /// simply ABSENT from a source that loaded fine. Always `false` for the
    /// enforced-but-undeclared direction. The distinction is surfaced in
    /// [`message`](Self::message) so an unloadable source reads differently from a
    /// missing target (REQ-01).
    pub unloadable: bool,
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
    ///     unloadable: false,
    /// };
    /// assert!(undeclared.message().contains("dag-no-cycles"));
    /// assert!(undeclared.message().contains("no invariant"));
    /// ```
    pub fn message(&self) -> String {
        match self.direction {
            DriftDirection::DeclaredButUnenforced if self.unloadable => format!(
                "enforcement drift (declared-but-unenforced): invariant '{}' is enforced-by \
                 '{}', whose enforcement source (rule set or gate registry) failed to load, so \
                 the binding cannot be satisfied",
                self.invariant_id.as_deref().unwrap_or("<unknown>"),
                self.subject
            ),
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
/// binding naming a missing OR unloadable target), and the unloadable source
/// contributes nothing to the enforced-but-undeclared direction.
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

/// Compute bidirectional enforcement drift, tolerating an UNLOADABLE rule set or
/// gate registry rather than erroring (REQ-01 "missing OR unloadable").
///
/// Semantics extend [`enforcement_drift`]:
///
/// - **declared-but-unenforced** — a binding is reported when it is satisfied by
///   NEITHER source. When neither source loaded, or the binding's only candidate
///   source is unloadable, the finding's [`DriftFinding::unloadable`] flag is set
///   (worded as "source failed to load"); when both candidate sources loaded but
///   lack the target, the flag is clear (worded as "missing").
/// - **enforced-but-undeclared** — only LOADED sources contribute entries; an
///   unloadable source is skipped (its entries cannot be enumerated).
///
/// A finding is flagged `unloadable` whenever at least one source is unloadable
/// AND the binding is not satisfied by a LOADED source — because then the binding
/// might have been satisfied by the source that failed to parse.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::{enforcement_drift_tolerant, DriftDirection, SourceState};
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
/// assert_eq!(f.direction, DriftDirection::DeclaredButUnenforced);
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
    let declared = invariants.iter().filter_map(|inv| {
        inv.enforced_by.as_deref().and_then(|binding| {
            let enforced = rules.contains(binding) || gates.contains(binding);
            (!enforced).then(|| DriftFinding {
                direction: DriftDirection::DeclaredButUnenforced,
                invariant_id: Some(inv.id.clone()),
                subject: binding.to_string(),
                unloadable: any_unloadable,
            })
        })
    });

    // enforced-but-undeclared: a rule/gate no invariant claims. Only LOADED
    // sources contribute (an unloadable source cannot be enumerated). Union the
    // loaded name spaces and sort for deterministic output.
    let claimed: BTreeSet<&str> = invariants
        .iter()
        .filter_map(|inv| inv.enforced_by.as_deref())
        .collect();
    let loaded_names: BTreeSet<&str> = [rules, gates]
        .into_iter()
        .filter_map(|s| match s {
            SourceState::Loaded(set) => Some(set.iter().copied()),
            SourceState::Unloadable => None,
        })
        .flatten()
        .collect();
    let undeclared = loaded_names
        .into_iter()
        .filter(move |name| !claimed.contains(name))
        .map(|name| DriftFinding {
            direction: DriftDirection::EnforcedButUndeclared,
            invariant_id: None,
            subject: name.to_string(),
            unloadable: false,
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
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"ghost-rule\"\n",
        );
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
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"dag-no-cycles\"\n",
        );
        let rules: BTreeSet<&str> = ["dag-no-cycles"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        // No declared-but-unenforced (binding resolves); the rule is claimed, so
        // no enforced-but-undeclared either.
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
    fn test_enforced_but_undeclared_reports_unclaimed_rule() {
        // One invariant claims rule-a; rule-b and gate-x are unclaimed.
        let r = reg(
            "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"rule-a\"\n",
        );
        let rules: BTreeSet<&str> = ["rule-a", "rule-b"].into_iter().collect();
        let gates: BTreeSet<&str> = ["gate-x"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        let undeclared: Vec<_> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::EnforcedButUndeclared)
            .map(|f| f.subject.as_str())
            .collect();
        // rule-b and gate-x are unclaimed; rule-a is claimed.
        assert_eq!(
            undeclared,
            vec!["gate-x", "rule-b"],
            "sorted, claimed excluded"
        );
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
        let declared: Vec<_> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::DeclaredButUnenforced)
            .collect();
        assert_eq!(declared.len(), 1, "{findings:?}");
        assert_eq!(declared[0].subject, "bad-rule");
        assert!(declared[0].unloadable, "unloadable flag set");
        assert!(declared[0].message().contains("failed to load"));
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
        let declared: Vec<_> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::DeclaredButUnenforced)
            .collect();
        assert_eq!(declared.len(), 1, "{findings:?}");
        assert_eq!(declared[0].subject, "some-gate");
        assert!(declared[0].unloadable);
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
        assert!(
            !findings
                .iter()
                .any(|f| f.direction == DriftDirection::DeclaredButUnenforced),
            "{findings:?}"
        );
    }

    #[test]
    fn test_unloadable_source_skipped_in_enforced_but_undeclared() {
        // An unloadable rule set contributes no enforced-but-undeclared findings
        // (its entries cannot be enumerated); the loaded gate registry still does.
        let gates: BTreeSet<&str> = ["lonely-gate"].into_iter().collect();
        let findings =
            enforcement_drift_tolerant(&[], SourceState::Unloadable, SourceState::Loaded(&gates));
        let undeclared: Vec<&str> = findings
            .iter()
            .filter(|f| f.direction == DriftDirection::EnforcedButUndeclared)
            .map(|f| f.subject.as_str())
            .collect();
        assert_eq!(undeclared, vec!["lonely-gate"], "{findings:?}");
    }
}
