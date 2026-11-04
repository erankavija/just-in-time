//! Pure declaration-consistency checks for invariant enforcement bindings.
//!
//! Only project-scoped `@/rule/<name>` and `@/gate/<key>` bindings resolve.
//! Unclaimed rules and gates are not drift; disabled rules still resolve.

use crate::declarations::invariants::Invariant;
use crate::domain::identity::GateKey;
use crate::domain::item::{parse_kind_segmented_address, AddressScope};
use std::collections::BTreeSet;

/// A single enforcement-drift finding: an invariant whose `enforced-by` binding
/// resolves to no loadable rule or gate (the sole drift direction,
/// declared-but-unenforced).
///
/// `subject` is the dangling binding and `invariant_id` is the offending
/// invariant's self-id (whose `@/invariant/<id>` qualified id addresses it).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct DriftFinding {
    /// The offending invariant's self-id.
    pub invariant_id: String,
    /// The dangling binding: the `enforced-by` value that resolves to neither a
    /// loadable rule nor a known gate.
    pub subject: String,
    /// Whether the binding names a rule in an unloadable rule set.
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

/// The rule set's load state; an unloadable set cannot satisfy rule bindings.
#[derive(Debug, Clone, Copy)]
pub enum SourceState<'a, T> {
    /// The source loaded; the contained set names every entry it declares.
    Loaded(&'a BTreeSet<T>),
    /// The source failed to load; its entries cannot be enumerated.
    Unloadable,
}

impl<T: Ord> SourceState<'_, T> {
    /// Whether the loaded source contains `name` (always `false` when unloadable).
    pub fn contains(&self, name: &T) -> bool {
        match self {
            SourceState::Loaded(set) => set.contains(name),
            SourceState::Unloadable => false,
        }
    }
}

/// Report unresolved enforcement bindings in authored invariant order.
///
/// Gates must be loaded; an unloadable rule set yields findings with
/// [`DriftFinding::unloadable`] set for rule bindings only.
///
/// # Examples
///
/// ```
/// use jit::domain::identity::GateKey;
/// use jit::validation::drift::{enforcement_drift, SourceState};
/// use jit::declarations::invariants::InvariantRegistry;
/// use std::collections::BTreeSet;
///
/// let reg = InvariantRegistry::from_toml_str(
///     "[[invariants]]\nid = \"dag-acyclic\"\nstatement = \"s\"\nkind = \"enforced\"\n\
///      enforced-by = \"@/rule/coverage-preview\"\n",
/// )
/// .unwrap();
/// let gates: BTreeSet<GateKey> = BTreeSet::new();
/// // The rule set failed to parse; the gate registry loaded (and is empty).
/// let findings = enforcement_drift(
///     &reg.invariants,
///     SourceState::Unloadable,
///     &gates,
/// );
/// let f = &findings[0];
/// assert_eq!(f.subject, "@/rule/coverage-preview");
/// assert!(f.unloadable, "binding into an unloadable source is flagged");
/// assert!(f.message().contains("failed to load"));
/// ```
pub fn enforcement_drift(
    invariants: &[Invariant],
    rules: SourceState<'_, &str>,
    gates: &BTreeSet<GateKey>,
) -> Vec<DriftFinding> {
    invariants
        .iter()
        .filter_map(|inv| {
            let binding = inv.enforced_by.as_deref()?;
            resolve_binding(binding, &rules, gates).map(|unloadable| DriftFinding {
                invariant_id: inv.id.clone(),
                subject: binding.to_string(),
                unloadable,
            })
        })
        .collect()
}

/// Compute drift from the same parsed declarations used by validation.
pub(crate) fn from_declarations(
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
) -> Vec<DriftFinding> {
    let rule_names: BTreeSet<&str> = declarations
        .rules()
        .rules
        .iter()
        .map(|rule| rule.name.as_str())
        .collect();
    let rules = if declarations.rules_loaded() {
        SourceState::Loaded(&rule_names)
    } else {
        SourceState::Unloadable
    };
    let gates = declarations.gates().gates.keys().cloned().collect();
    enforcement_drift(&declarations.config().invariants.invariants, rules, &gates)
}

/// `None` means resolved; `Some` carries the finding's unloadable-rule flag.
fn resolve_binding(
    binding: &str,
    rules: &SourceState<'_, &str>,
    gates: &BTreeSet<GateKey>,
) -> Option<bool> {
    let Ok(addr) = parse_kind_segmented_address(binding) else {
        return Some(false);
    };
    if addr.scope != AddressScope::Project {
        return Some(false);
    }
    match addr.kind.as_str() {
        "rule" => match rules {
            SourceState::Loaded(set) => (!set.contains(addr.self_id.as_str())).then_some(false),
            SourceState::Unloadable => Some(true),
        },
        "gate" => addr
            .self_id
            .parse::<GateKey>()
            .map_or(Some(false), |key| (!gates.contains(&key)).then_some(false)),
        _ => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::invariants::InvariantRegistry;

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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = ["code-review"]
            .into_iter()
            .map(|key| key.parse().unwrap())
            .collect();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = ["gate-x"]
            .into_iter()
            .map(|key| key.parse().unwrap())
            .collect();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_no_invariants_no_rules_no_gates_is_clean() {
        let findings =
            enforcement_drift(&[], SourceState::Loaded(&BTreeSet::new()), &BTreeSet::new());
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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = ["cargo-ci"]
            .into_iter()
            .map(|key| key.parse().unwrap())
            .collect();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
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
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Loaded(&rules), &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "legacy:old-rule");
    }

    // --- unloadable source ------------------------------------------------

    #[test]
    fn test_unloadable_rule_source_makes_binding_declared_but_unenforced() {
        // The rule set failed to parse; the binding names a rule that would have
        // lived there. It is declared-but-unenforced WITH the unloadable flag.
        let r = reg(
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                     enforced-by = \"@/rule/bad-rule\"\n",
        );
        let gates: BTreeSet<GateKey> = BTreeSet::new();
        let findings = enforcement_drift(&r.invariants, SourceState::Unloadable, &gates);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "@/rule/bad-rule");
        assert!(findings[0].unloadable, "unloadable flag set");
        assert!(findings[0].message().contains("failed to load"));
    }
}
