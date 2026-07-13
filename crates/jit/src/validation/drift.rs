//! Enforcement-drift check (declaration-consistency, NOT execution).
//!
//! An invariant MAY bind to the rule or gate that enforces it via its
//! `enforced-by` field, addressed as `@/rule/<name>` or `@/gate/<key>` — the
//! addressable-item form the epic's D11 decision settles on (REQ-05), parsed
//! structurally via
//! [`parse_kind_segmented_address`](crate::domain::item::parse_kind_segmented_address).
//! That binding is a DECLARATION: `enforced-by` (and the `checker-command`
//! escape hatch it ultimately backs) is never executed by `jit validate` — the
//! local write path explicitly skips it
//! ([`evaluate_local`](crate::validation::local::evaluate_local)) and validate has
//! no execution site. Drift is therefore checked as DECLARATION CONSISTENCY
//! between the invariant registry and the set of loadable rules/gates, in a
//! single direction:
//!
//! - **declared-but-unenforced** — an invariant whose `enforced-by` does not
//!   resolve to a loadable rule or a known gate: the address parses to
//!   `@/rule/<name>` but no loaded rule carries `<name>`, it parses to
//!   `@/gate/<key>` but no known gate carries `<key>`, or its enforcement SOURCE
//!   (the rule set or gate registry) is UNLOADABLE (REQ-01 covers both). A
//!   binding that is not one of those two recognized address forms at all — a
//!   legacy bare name (`cargo-ci`), a colon-prefixed name (`legacy:old-rule`), a
//!   different item kind, or a non-project scope — is likewise unresolved: D11
//!   is a clean cut, with no bare-name fallback. The
//!   unloadable case is handled by [`enforcement_drift_tolerant`] / [`SourceState`].
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
//! names, gate keys) plus the structural address parse. The registries are
//! loaded at the command/validate boundary and injected, mirroring the rest of
//! the validation engine. The two kind literals `"rule"` and `"gate"` are the
//! only kind-name literals here, matching the two enforcement-target kinds
//! `.jit/config.toml` declares under `[item_kinds]`.

use crate::domain::item::{parse_kind_segmented_address, AddressScope};
use crate::validation::invariants::Invariant;
use std::collections::BTreeSet;

/// A single enforcement-drift finding: an invariant whose `enforced-by` binding
/// resolves to no loadable rule or gate (the sole drift direction,
/// declared-but-unenforced).
///
/// `subject` is the dangling binding and `invariant_id` is the offending
/// invariant's self-id (whose `@/invariant/<id>` qualified id addresses it).
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
/// "enforced" iff it parses as `@/rule/<name>` with `<name>` in `rule_names`, or
/// `@/gate/<key>` with `<key>` in `gate_keys`. For every other invariant — one
/// whose binding does not resolve that way — a
/// [declared-but-unenforced](DriftFinding) finding is produced. A loadable rule
/// or gate that no invariant claims is NOT reported (REQ-05).
///
/// Results are deterministic: findings follow the invariants' authored order.
/// The function is pure — it reads only its arguments.
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
/// whose address kind routes to it (`@/rule/<name>` to the rule set, `@/gate/<key>`
/// to the gate registry) is reported as declared-but-unenforced with the
/// `unloadable` flag set (REQ-01 covers a binding naming a missing OR
/// unloadable target).
#[derive(Debug, Clone, Copy)]
pub enum SourceState<'a> {
    /// The source loaded; the contained set names every entry it declares.
    Loaded(&'a BTreeSet<&'a str>),
    /// The source failed to load; its entries cannot be enumerated.
    Unloadable,
}

impl SourceState<'_> {
    /// Whether the loaded source contains `name` (always `false` when unloadable).
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
/// declared-but-unenforced when it does not resolve. The address form pins down
/// exactly ONE candidate source per binding — `@/rule/<name>` resolves only
/// against `rules`, `@/gate/<key>` only against `gates` — so the
/// [`DriftFinding::unloadable`] flag reflects THAT source's own state: set when
/// the binding's own source is [`SourceState::Unloadable`] (worded as "source
/// failed to load"), clear when that source loaded but lacks the target (worded
/// as "missing"), and also clear for a binding that is not a recognized
/// `@/rule/<name>` / `@/gate/<key>` address at all — a structural defect in the
/// binding's FORM, not a load-state question.
///
/// # Examples
///
/// ```
/// use jit::validation::drift::{enforcement_drift_tolerant, SourceState};
/// use jit::validation::invariants::InvariantRegistry;
/// use std::collections::BTreeSet;
///
/// let reg = InvariantRegistry::from_toml_str(
///     "[[invariants]]\nid = \"dag-acyclic\"\nstatement = \"s\"\nkind = \"enforced\"\n\
///      enforced-by = \"@/rule/coverage-preview\"\n",
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
/// assert_eq!(f.subject, "@/rule/coverage-preview");
/// assert!(f.unloadable, "binding into an unloadable source is flagged");
/// assert!(f.message().contains("failed to load"));
/// ```
pub fn enforcement_drift_tolerant(
    invariants: &[Invariant],
    rules: SourceState<'_>,
    gates: SourceState<'_>,
) -> Vec<DriftFinding> {
    invariants
        .iter()
        .filter_map(|inv| {
            let binding = inv.enforced_by.as_deref()?;
            resolve_binding(binding, rules, gates).map(|unloadable| DriftFinding {
                invariant_id: inv.id.clone(),
                subject: binding.to_string(),
                unloadable,
            })
        })
        .collect()
}

/// Resolve one `enforced-by` binding against the two enforcement sources.
///
/// `None` means the binding resolves (no finding is produced for it);
/// `Some(unloadable)` means it does not, carrying the flag that goes on the
/// resulting [`DriftFinding`].
///
/// Only the two recognized address forms can resolve: `@/rule/<name>` is
/// checked against `rules`, `@/gate/<key>` against `gates` — the binding's kind
/// segment pins down exactly which source is relevant, so the `unloadable` flag
/// reflects THAT source's own [`SourceState`] rather than an ambient "any source
/// failed" guess. Every other shape — a binding that fails to parse as a
/// kind-segmented address at all (a legacy bare name, a colon-prefixed name), a
/// non-project scope, or a kind other than `rule`/`gate` — is unconditionally
/// unresolved with `unloadable: false`: the defect is in the binding's FORM, not
/// in a source failing to load.
fn resolve_binding(binding: &str, rules: SourceState<'_>, gates: SourceState<'_>) -> Option<bool> {
    let Ok(addr) = parse_kind_segmented_address(binding) else {
        return Some(false);
    };
    if addr.scope != AddressScope::Project {
        return Some(false);
    }
    let source = match addr.kind.as_str() {
        "rule" => rules,
        "gate" => gates,
        _ => return Some(false),
    };
    match source {
        SourceState::Loaded(set) => (!set.contains(addr.self_id.as_str())).then_some(false),
        SourceState::Unloadable => Some(true),
    }
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
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/ghost-rule\"\n",
        );
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].invariant_id, "sample-invariant");
        assert_eq!(findings[0].subject, "@/rule/ghost-rule");
        assert!(findings[0].message().contains("sample-invariant"));
        assert!(findings[0].message().contains("@/rule/ghost-rule"));
    }

    #[test]
    fn test_binding_to_real_rule_is_not_drift() {
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/dag-no-cycles\"\n",
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
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/gate/code-review\"\n",
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
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/rule-a\"\n",
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
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"advisory\"\n",
        );
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

    #[test]
    fn test_unresolved_rule_name_is_drift() {
        // The address form parses fine, but no loaded rule carries the name.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/ghost\"\n",
        );
        let rules: BTreeSet<&str> = ["real-rule"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "@/rule/ghost");
        assert!(!findings[0].unloadable);
    }

    #[test]
    fn test_unrecognized_address_kind_is_drift() {
        // A syntactically valid address whose kind is neither `rule` nor `gate` is
        // unresolved: the drift check recognizes only those two enforcement-target
        // kinds, even though "label-format" is a real loaded rule name.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/definition/label-format\"\n",
        );
        let rules: BTreeSet<&str> = ["label-format"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(!findings[0].unloadable);
    }

    #[test]
    fn test_legacy_bare_name_binding_is_rejected() {
        // D11 is a clean cut: the pre-addressing bare-name form is unresolved even
        // when a gate of that exact name is loaded — there is no bare-name
        // fallback.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"cargo-ci\"\n",
        );
        let rules: BTreeSet<&str> = BTreeSet::new();
        let gates: BTreeSet<&str> = ["cargo-ci"].into_iter().collect();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "cargo-ci");
    }

    #[test]
    fn test_legacy_colon_form_binding_is_rejected() {
        // The old colon-prefixed rule-name form is also rejected outright, even
        // when the post-colon segment names a real loaded rule.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"legacy:old-rule\"\n",
        );
        let rules: BTreeSet<&str> = ["old-rule"].into_iter().collect();
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, &rules, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "legacy:old-rule");
    }

    // --- tolerant (unloadable source) -------------------------------------

    #[test]
    fn test_unloadable_rule_source_makes_binding_declared_but_unenforced() {
        // The rule set failed to parse; the binding names a rule that would have
        // lived there. It is declared-but-unenforced WITH the unloadable flag.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/bad-rule\"\n",
        );
        let gates: BTreeSet<&str> = BTreeSet::new();
        let findings = enforcement_drift_tolerant(
            &r.invariants,
            SourceState::Unloadable,
            SourceState::Loaded(&gates),
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "@/rule/bad-rule");
        assert!(findings[0].unloadable, "unloadable flag set");
        assert!(findings[0].message().contains("failed to load"));
    }

    #[test]
    fn test_unloadable_gate_source_makes_binding_declared_but_unenforced() {
        // The gate registry failed to load; a binding into it is
        // declared-but-unenforced with the unloadable flag, regardless of the
        // (irrelevant) rule set's own state.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/gate/some-gate\"\n",
        );
        let rules: BTreeSet<&str> = ["a-rule"].into_iter().collect();
        let findings = enforcement_drift_tolerant(
            &r.invariants,
            SourceState::Loaded(&rules),
            SourceState::Unloadable,
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "@/gate/some-gate");
        assert!(findings[0].unloadable);
    }

    #[test]
    fn test_binding_satisfied_by_loaded_source_not_flagged_when_other_unloadable() {
        // Even with the gate registry unloadable, a `@/rule/...` binding
        // satisfied by the LOADED rule set is NOT drift: only the rule source is
        // relevant to a rule-kind binding.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/a-rule\"\n",
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
