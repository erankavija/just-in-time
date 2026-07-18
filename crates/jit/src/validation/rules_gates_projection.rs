//! Project the rule and gate registries into a CONFIGURABLE documentation target.
//!
//! The rules/gates analogue of the invariant projection
//! ([`projection`](crate::validation::projection)): it renders BOTH the effective
//! [`RuleSet`] and the [`GateRegistry`] into one reference document and writes it
//! into a config-selected target, reusing the SAME projection primitives:
//!
//! - the [`ProjectionMode`] / [`ProjectionStyle`] configuration knobs,
//! - the region-splice function [`splice_region`], and
//! - the shared, typed [`ProjectionError`] plus the storage atomic-write boundary.
//!
//! [`render_rules_and_gates_markdown`] is the built-in `full`-style render for the
//! `rule` + `gate` registry-first kinds: it takes the two registries directly,
//! since their typed fields (severity, enforcement, gate title) are absent from a
//! generic addressable row. The generic projection command
//! ([`project_render`](crate::validation::project_render)) selects this renderer for
//! a `full`-style projection over the rule + gate kinds, then writes the body
//! through the shared [`write_projection`](crate::validation::projection::write_projection).
//!
//! Every rule/gate is addressed by its canonical kind-segmented form —
//! `@/rule/<name>` and `@/gate/<key>` — matching the `[item_kinds.rule]` /
//! `[item_kinds.gate]` registry projection.
//!
//! Rendering is PURE and unit-testable.

use crate::config::ProjectionStyle;
use crate::declarations::rules::RuleSet;
use crate::declarations::GateRegistry;

/// The canonical kind-segmented address of a rule (`@/rule/<name>`).
fn rule_address(name: &str) -> String {
    format!("@/rule/{name}")
}

/// The canonical kind-segmented address of a gate (`@/gate/<key>`).
fn gate_address(key: &str) -> String {
    format!("@/gate/{key}")
}

/// Render `rules` and `gates` into a deterministic reference document in `style`.
///
/// Pure: performs no I/O and reads no configuration beyond the `style` argument.
/// Rules are listed in authored order; gates are sorted by key so the output is
/// deterministic despite the registry's hash-map storage. Every entry is addressed
/// by its canonical kind-segmented form (`@/rule/<name>`, `@/gate/<key>`). The two
/// styles ([`ProjectionStyle`]) differ only in framing:
///
/// - [`ProjectionStyle::Full`] (the default) renders a `## Rules` section of
///   `- **@/rule/{name}** — {description} ({severity}, {enforced|advisory})`
///   bullets (the description falls back to the rule name, and the prose suffix
///   mirrors the gate line's framing) followed by a `## Gates` section of
///   `- **@/gate/{key}** — {title}: {description}` bullets (the `: {description}`
///   suffix is omitted when the description is empty).
/// - [`ProjectionStyle::IdAnchor`] renders a HEADING-LESS bullet list — one
///   `- **{address}** — {display-text}` line per rule (display text = the rule
///   description, or its name when absent — the rule kind's registry text-field)
///   then per gate (display text = the gate description, or its title when the
///   description is empty) — for embedding beneath a hand-authored heading.
///
/// An empty registry renders an explicit "none declared" line in its section (Full
/// style) or, when BOTH registries are empty, a single "none declared" line
/// (IdAnchor style), so the projected region is never blank.
///
/// # Examples
///
/// ```
/// use jit::config::ProjectionStyle;
/// use jit::declarations::GateRegistry;
/// use jit::declarations::rules::RuleSet;
/// use jit::validation::rules_gates_projection::render_rules_and_gates_markdown;
///
/// let rules = RuleSet::parse(
///     "[[rules]]\nname = \"label-format\"\ndescription = \"Labels are namespace:value.\"\n\
///      severity = \"error\"\nenforce = true\n\
///      assert = { require-label = { label = \"type:*\" } }\n",
///     None,
///     [],
/// )
/// .unwrap();
/// let gates = GateRegistry::default();
///
/// // Full style keeps the section headings and the canonical rule address.
/// let full = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::Full);
/// assert!(full.contains("## Rules"));
/// assert!(full.contains("## Gates"));
/// assert!(full.contains("- **@/rule/label-format** — Labels are namespace:value. (error, enforced)"));
/// assert!(full.contains("_No gates declared._"));
///
/// // Id-anchor style is heading-less; the rule's description is its display text.
/// let anchored = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::IdAnchor);
/// assert!(!anchored.contains("## Rules"));
/// assert_eq!(anchored, "- **@/rule/label-format** — Labels are namespace:value.\n");
/// ```
pub fn render_rules_and_gates_markdown(
    rules: &RuleSet,
    gates: &GateRegistry,
    style: ProjectionStyle,
) -> String {
    match style {
        ProjectionStyle::Full => render_full(rules, gates),
        ProjectionStyle::IdAnchor => render_id_anchor(rules, gates),
    }
}

/// A rule's display text: its `description`, falling back to its `name` (the
/// rule kind's registry text-field fallback) when no description is authored, so
/// a description-less rule never trails a bare em-dash.
fn rule_display_text(rule: &crate::declarations::rules::Rule) -> &str {
    rule.description.as_deref().unwrap_or(&rule.name)
}

/// A rule's prose metadata suffix — `(error, enforced)` / `(warn, advisory)` —
/// mirroring the gate line's framing instead of the raw `severity: …, enforce: …`
/// key/value form. The severity token is rendered as-is; `enforce` becomes
/// `enforced` (blocks writes) or `advisory` (never blocks).
fn rule_metadata_suffix(rule: &crate::declarations::rules::Rule) -> String {
    let enforcement = if rule.enforce { "enforced" } else { "advisory" };
    format!("({}, {})", rule.severity.token(), enforcement)
}

/// Gate `(key, gate)` pairs in ascending key order (deterministic).
fn gates_sorted(gates: &GateRegistry) -> Vec<(&String, &crate::declarations::GateDefinition)> {
    let mut pairs: Vec<_> = gates.gates.iter().collect();
    pairs.sort_by_key(|(key, _)| *key);
    pairs
}

/// The gate's display text: its description, falling back to its title when the
/// description is empty (so a bullet never trails a bare em-dash).
fn gate_display_text(gate: &crate::declarations::GateDefinition) -> &str {
    if gate.description.is_empty() {
        &gate.title
    } else {
        &gate.description
    }
}

/// Render the `full` style: `## Rules` and `## Gates` sections with metadata.
fn render_full(rules: &RuleSet, gates: &GateRegistry) -> String {
    let mut out = String::from("## Rules\n\n");
    if rules.rules.is_empty() {
        out.push_str("_No rules declared._\n");
    } else {
        for rule in &rules.rules {
            out.push_str(&format!(
                "- **{address}** — {text} {suffix}\n",
                address = rule_address(&rule.name),
                text = rule_display_text(rule),
                suffix = rule_metadata_suffix(rule),
            ));
        }
    }

    out.push_str("\n## Gates\n\n");
    let sorted = gates_sorted(gates);
    if sorted.is_empty() {
        out.push_str("_No gates declared._\n");
    } else {
        for (key, gate) in sorted {
            let description = if gate.description.is_empty() {
                String::new()
            } else {
                format!(": {}", gate.description)
            };
            out.push_str(&format!(
                "- **{address}** — {title}{description}\n",
                address = gate_address(key),
                title = gate.title,
            ));
        }
    }
    out
}

/// Render the `id-anchor` style: a heading-less `- **{address}** — {text}` list.
///
/// Rules (display text = description, or name when absent) then gates (display
/// text = description or title), in deterministic order. When BOTH registries are
/// empty, a single explicit line is emitted so the projected region is never blank.
fn render_id_anchor(rules: &RuleSet, gates: &GateRegistry) -> String {
    let sorted = gates_sorted(gates);
    if rules.rules.is_empty() && sorted.is_empty() {
        return String::from("_No rules or gates declared._\n");
    }
    let mut out = String::new();
    for rule in &rules.rules {
        out.push_str(&format!(
            "- **{address}** — {text}\n",
            address = rule_address(&rule.name),
            text = rule_display_text(rule),
        ));
    }
    for (key, gate) in sorted {
        out.push_str(&format!(
            "- **{address}** — {text}\n",
            address = gate_address(key),
            text = gate_display_text(gate),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateDefinition;
    use crate::declarations::{GateMode, GateStage};
    use std::collections::HashMap;

    fn ruleset() -> RuleSet {
        // `label-format` carries a description (rendered verbatim); `orphan-leaf`
        // has none (exercises the name fallback), so one fixture covers both.
        RuleSet::parse(
            r#"
[[rules]]
name = "label-format"
description = "Every label is namespace:value."
severity = "error"
enforce = true
assert = { require-label = { label = "type:*" } }

[[rules]]
name = "orphan-leaf"
severity = "warn"
enforce = false
assert = { require-section = { heading = "Goal" } }
"#,
            None,
            [],
        )
        .unwrap()
    }

    fn gate(key: &str, title: &str, description: &str) -> GateDefinition {
        GateDefinition {
            version: 1,
            key: key.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: HashMap::new(),
            auto: false,
            example_integration: None,
        }
    }

    fn gate_registry() -> GateRegistry {
        let mut registry = GateRegistry::default();
        // Inserted out of key order to prove the render sorts by key.
        registry.gates.insert(
            "cargo-ci".to_string(),
            gate("cargo-ci", "Cargo CI", "fmt + clippy + tests"),
        );
        registry.gates.insert(
            "breakdown-review".to_string(),
            gate(
                "breakdown-review",
                "AI Breakdown Review",
                "adversarial review",
            ),
        );
        registry
    }

    #[test]
    fn test_render_full_lists_rules_then_gates_deterministically() {
        let md =
            render_rules_and_gates_markdown(&ruleset(), &gate_registry(), ProjectionStyle::Full);
        // Section headings frame the two registries.
        let rules_at = md.find("## Rules").unwrap();
        let gates_at = md.find("## Gates").unwrap();
        assert!(rules_at < gates_at);
        // Rules in authored order with canonical addresses, description (or name
        // fallback), and the prose metadata suffix mirroring the gate line.
        assert!(md.contains(
            "- **@/rule/label-format** — Every label is namespace:value. (error, enforced)"
        ));
        assert!(md.contains("- **@/rule/orphan-leaf** — orphan-leaf (warn, advisory)"));
        // Gates sorted by key (breakdown-review before cargo-ci despite insert order).
        let brk = md.find("@/gate/breakdown-review").unwrap();
        let cci = md.find("@/gate/cargo-ci").unwrap();
        assert!(brk < cci, "gates must be key-sorted: {md}");
        assert!(md.contains("- **@/gate/cargo-ci** — Cargo CI: fmt + clippy + tests"));
        // Deterministic: identical input renders identical output.
        assert_eq!(
            md,
            render_rules_and_gates_markdown(&ruleset(), &gate_registry(), ProjectionStyle::Full)
        );
    }

    #[test]
    fn test_render_full_rule_prose_suffix_and_description_fallback() {
        // REQ-04: a described rule renders its description then the prose suffix;
        // a description-less rule falls back to its name. The suffix is prose
        // (`error, enforced` / `warn, advisory`), never `severity: …, enforce: …`.
        let rules = RuleSet::parse(
            r#"
[[rules]]
name = "described"
description = "A hand-authored explanation."
severity = "error"
enforce = true
assert = { require-label = { label = "type:*" } }

[[rules]]
name = "bare"
severity = "warn"
enforce = false
assert = { require-section = { heading = "Goal" } }
"#,
            None,
            [],
        )
        .unwrap();
        let md = render_rules_and_gates_markdown(
            &rules,
            &GateRegistry::default(),
            ProjectionStyle::Full,
        );
        assert!(
            md.contains("- **@/rule/described** — A hand-authored explanation. (error, enforced)")
        );
        assert!(md.contains("- **@/rule/bare** — bare (warn, advisory)"));
        // The legacy key/value metadata form is gone.
        assert!(!md.contains("severity:"));
        assert!(!md.contains("enforce:"));
    }

    #[test]
    fn test_render_full_empty_registries_have_explicit_lines() {
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &GateRegistry::default(),
            ProjectionStyle::Full,
        );
        assert!(md.contains("## Rules"));
        assert!(md.contains("_No rules declared._"));
        assert!(md.contains("## Gates"));
        assert!(md.contains("_No gates declared._"));
    }

    #[test]
    fn test_render_full_gate_without_description_omits_suffix() {
        let mut registry = GateRegistry::default();
        registry
            .gates
            .insert("bare".to_string(), gate("bare", "Bare Gate", ""));
        let md =
            render_rules_and_gates_markdown(&RuleSet::empty(), &registry, ProjectionStyle::Full);
        // No trailing "`: `" when the description is empty.
        assert!(md.contains("- **@/gate/bare** — Bare Gate\n"), "{md}");
    }

    #[test]
    fn test_render_id_anchor_is_heading_less() {
        let md = render_rules_and_gates_markdown(
            &ruleset(),
            &gate_registry(),
            ProjectionStyle::IdAnchor,
        );
        assert!(!md.contains("## Rules"));
        assert!(!md.contains("## Gates"));
        assert!(!md.contains("severity:"));
        // Rules use the description (or name fallback) as display text; gates use
        // the description.
        assert_eq!(
            md,
            "- **@/rule/label-format** — Every label is namespace:value.\n\
             - **@/rule/orphan-leaf** — orphan-leaf\n\
             - **@/gate/breakdown-review** — adversarial review\n\
             - **@/gate/cargo-ci** — fmt + clippy + tests\n"
        );
    }

    #[test]
    fn test_render_id_anchor_both_empty_has_single_explicit_line() {
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &GateRegistry::default(),
            ProjectionStyle::IdAnchor,
        );
        assert_eq!(md, "_No rules or gates declared._\n");
    }

    #[test]
    fn test_render_id_anchor_gate_without_description_falls_back_to_title() {
        let mut registry = GateRegistry::default();
        registry
            .gates
            .insert("bare".to_string(), gate("bare", "Bare Gate", ""));
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &registry,
            ProjectionStyle::IdAnchor,
        );
        assert_eq!(md, "- **@/gate/bare** — Bare Gate\n");
    }
}
