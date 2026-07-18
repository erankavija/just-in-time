//! Planning-bracket gate presets (design doc D8/D13, task T6).
//!
//! Three presets bundle the gates that bracket a breakable container:
//!
//! - [`plan_review_preset`] attaches the external-review placeholder to the
//!   planning node `P` (`type:planning`). It mirrors the `plan-review` gate
//!   authored by the embedded dogfood profile.
//! - [`coverage_preview_preset`] attaches the **deterministic** coverage-preview
//!   gate to the breakdown node `B` (`type:breakdown`). Its checker resolves the
//!   container `C` from `B`'s `brackets:<C-short-id>` label and runs
//!   `jit validate --scope <C>` (T2), which exits 4 when a `[hard]` criterion is
//!   left uncovered.
//! - [`breakdown_review_preset`] attaches the external-review placeholder to
//!   the same breakdown node `B` — the quality half of `B`'s quality-vs-coverage
//!   split. It reviews the decomposition itself (content standards, dependency-DAG
//!   coherence, right-sized depth) and does not re-check `[hard]` coverage.
//!
//! Plus [`preview_coverage_rule`], the pure constructor for the preview
//! `label-coverage` rule (D13): it is the closure rule with `child-state`
//! **omitted** (so drafted Backlog children count at plan time) and keyed on the
//! breakdown type so it fires only on `B`.
//!
//! Everything here is **domain-agnostic**: no `epic`/`planning`/`breakdown`
//! literal appears in engine logic. The breakdown type name is read from the
//! `plan` graph template (`TemplateRegistry`) and threaded in by the caller; the
//! coverage checker resolves its container generically from whatever issue it
//! runs on.

use super::{GatePresetDefinition, GateTemplate};
use crate::declarations::rules::{Assertion, Rule, Selector};
use crate::profile::jit_dogfood_gate;
use anyhow::{anyhow, Result};

/// Preset name for the agent plan-quality gate (planning node `P`).
pub const PLAN_REVIEW_PRESET: &str = "plan-review";

/// Preset name for the deterministic coverage-preview gate (breakdown node `B`).
pub const COVERAGE_PREVIEW_PRESET: &str = "coverage-preview";

/// Gate key bundled by [`coverage_preview_preset`].
pub const COVERAGE_PREVIEW_GATE: &str = "coverage-preview";

/// Preset name for the agent breakdown-review gate (breakdown node `B`).
pub const BREAKDOWN_REVIEW_PRESET: &str = "breakdown-review";

/// Build the `plan-review` preset: an agent review gate for the planning node.
///
/// The gate shape is derived from the embedded `jit-dogfood` package. Its
/// review-placeholder checker makes an unconfigured external reviewer visible
/// without making the built-in preset a second authored definition.
///
/// # Errors
///
/// Returns an error if the embedded dogfood package or its gate definition is
/// invalid.
pub fn plan_review_preset() -> Result<GatePresetDefinition> {
    package_gate_preset(PLAN_REVIEW_PRESET)
}

/// Build the `coverage-preview` preset: a deterministic coverage gate for the
/// breakdown node.
///
/// The package-authored native checker resolves the container from the gated
/// breakdown issue's `brackets:<short-id>` label and runs scoped validation
/// in-process. Nothing here is hardcoded to a particular container type.
///
/// # Errors
///
/// Returns an error if the embedded dogfood package or its gate definition is
/// invalid.
pub fn coverage_preview_preset() -> Result<GatePresetDefinition> {
    package_gate_preset(COVERAGE_PREVIEW_PRESET)
}

/// Build the `breakdown-review` preset: an **agent** quality review of the
/// drafted decomposition, attached to the breakdown node `B`.
///
/// It is the front-end counterpart to `coverage-preview`'s **quality-vs-coverage
/// split** on `B`: where `coverage-preview` (deterministic) answers *"is every
/// `[hard]` criterion mapped to a child?"*, `breakdown-review` (agent) answers
/// *"is the decomposition itself any good?"* — content standards per child,
/// dependency-DAG coherence (both missing prerequisites and over-constraining
/// false serialization), right-sized depth, and blank-workspace reachability. It
/// deliberately does **not** re-check `[hard]`-criterion coverage; that is the
/// deterministic gate's job.
///
/// The gate shape is derived from the embedded `jit-dogfood` package and begins
/// as a visible review placeholder. Because it is an ordinary postcheck gate on
/// `B`, jit's gate enforcement remains self-guiding after an adopter replaces
/// the placeholder with its reviewer integration.
///
/// # Errors
///
/// Returns an error if the embedded dogfood package or its gate definition is
/// invalid.
pub fn breakdown_review_preset() -> Result<GatePresetDefinition> {
    package_gate_preset(BREAKDOWN_REVIEW_PRESET)
}

pub(crate) fn package_gate_preset(key: &str) -> Result<GatePresetDefinition> {
    let gate = jit_dogfood_gate(key)?;
    Ok(GatePresetDefinition {
        name: key.to_string(),
        description: gate.description.clone(),
        gates: vec![GateTemplate {
            key: gate.key,
            title: gate.title,
            description: gate.description,
            stage: gate.stage,
            mode: gate.mode,
            checker: gate.checker,
        }],
    })
}

/// Derive the **preview** coverage rule from a **closure** `label-coverage`
/// rule (D13).
///
/// The preview and closure rules are the *same* rule kind differing only by the
/// `child-state` knob and what they key on:
///
/// - the **closure** rule keeps `child-state = "done"` and fires on the
///   container at its `→ done` transition (mapping *done*);
/// - the **preview** rule **omits `child-state` entirely** — an absent
///   `child-state` means "any state" in the evaluator, so drafted Backlog
///   children count (mapping *exists*) — and is keyed on `breakdown_type` with
///   `container-from-label = "brackets"`, so it fires only on the transient
///   breakdown node `B`, resolving `C`'s criteria via the `brackets:` label.
///
/// The closure rule's criteria knobs (`criteria-section`, `marker`,
/// `id-pattern`, `satisfies-namespace`, `child-link`, `child-type-exclude`) are
/// carried over verbatim, so the only authored difference is the dropped
/// `child-state`. `breakdown_type` comes from the `plan` graph template (the
/// breakdown node's `type`); no type literal is baked in here.
///
/// # Errors
///
/// Returns an error if `closure` is not a `label-coverage` rule or if
/// `breakdown_type` is blank.
///
/// # Examples
///
/// ```
/// use jit::gate_presets::preview_coverage_rule;
/// use jit::declarations::rules::RuleSet;
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "closure"
/// when = { type = "epic", state = "done" }
/// severity = "error"
/// enforce = true
/// assert = { label-coverage = { marker = "[hard]", satisfies-namespace = "satisfies", child-state = "done", child-link = "dependencies" } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/x")).unwrap();
/// let preview = preview_coverage_rule(&set.rules[0], "breakdown").unwrap();
///
/// // Keyed on the breakdown type, not epic+done.
/// assert_eq!(preview.when.type_.as_deref(), Some("breakdown"));
/// assert!(preview.when.state.is_none());
///
/// // Omits child-state (closure had "done"); resolves container via brackets:.
/// match (&set.rules[0].assert, &preview.assert) {
///     (
///         jit::declarations::rules::Assertion::LabelCoverage { config: closure_cfg },
///         jit::declarations::rules::Assertion::LabelCoverage { config: preview_cfg },
///     ) => {
///         assert_eq!(closure_cfg.get("child-state").unwrap().as_str(), Some("done"));
///         assert!(preview_cfg.get("child-state").is_none());
///         assert_eq!(
///             preview_cfg.get("container-from-label").unwrap().as_str(),
///             Some("brackets")
///         );
///         // The shared knobs are identical.
///         assert_eq!(preview_cfg.get("marker"), closure_cfg.get("marker"));
///     }
///     _ => panic!("expected label-coverage assertions"),
/// }
/// ```
pub fn preview_coverage_rule(closure: &Rule, breakdown_type: &str) -> Result<Rule> {
    let breakdown_type = breakdown_type.trim();
    if breakdown_type.is_empty() {
        return Err(anyhow!("breakdown_type must not be empty"));
    }

    let closure_config = match &closure.assert {
        Assertion::LabelCoverage { config } => config,
        _ => {
            return Err(anyhow!(
                "preview_coverage_rule requires a label-coverage closure rule, got a different \
                 assertion kind"
            ))
        }
    };

    // Start from the closure config so every shared knob (criteria-section,
    // marker, id-pattern, satisfies-namespace, child-link, child-type-exclude)
    // is carried verbatim; the ONLY authored differences are below.
    let mut preview_config = closure_config.clone();
    // D13: omit `child-state` so drafted children in any state count (preview =
    // mapping exists, not mapping done).
    preview_config.remove("child-state");
    // D6/T3: the rule fires on `B`, which resolves its criteria-bearing
    // container `C` from the `brackets:` label.
    preview_config.insert(
        "container-from-label".to_string(),
        toml::Value::String("brackets".to_string()),
    );

    let assert = Assertion::LabelCoverage {
        config: preview_config,
    };
    let scope = assert.scope();

    Ok(Rule {
        name: format!("{}-preview", closure.name),
        // Inherit the closure rule's provenance, if any.
        origin: closure.origin.clone(),
        // Inherit the closure rule's description, if any.
        description: closure.description.clone(),
        // Keyed on the breakdown type (config-driven), at any state — `B` is
        // the transient node the preview fires on.
        when: Selector {
            type_: Some(breakdown_type.to_string()),
            label: None,
            state: None,
            has_doc_type: None,
        },
        severity: closure.severity,
        enforce: closure.enforce,
        assert,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::rules::RuleSet;
    use crate::declarations::{GateMode, GateStage};
    use std::path::Path;

    fn closure_ruleset() -> RuleSet {
        // Mirrors the SDD/research closure `label-coverage` instance.
        let toml = r#"
[[rules]]
name = "hard-criteria-covered"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-state = "done", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"] } }
"#;
        RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap()
    }

    #[test]
    fn test_plan_review_preset_attaches_agent_gate() {
        let preset = plan_review_preset().unwrap();
        assert_eq!(preset.name, "plan-review");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "plan-review");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        assert_eq!(
            gate.checker,
            Some(crate::declarations::GateChecker::ReviewPlaceholder)
        );
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_breakdown_review_preset_attaches_agent_gate() {
        let preset = breakdown_review_preset().unwrap();
        assert_eq!(preset.name, "breakdown-review");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "breakdown-review");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        assert_eq!(
            gate.checker,
            Some(crate::declarations::GateChecker::ReviewPlaceholder)
        );
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_coverage_preview_preset_runs_scoped_validate() {
        let preset = coverage_preview_preset().unwrap();
        assert_eq!(preset.name, "coverage-preview");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "coverage-preview");
        assert_eq!(gate.mode, GateMode::Auto);

        match gate.checker.as_ref().expect("coverage gate has a checker") {
            crate::declarations::GateChecker::LabelTargetValidation { label_namespace } => {
                assert_eq!(label_namespace, "brackets");
            }
            other => panic!("expected label-target checker, got {other:?}"),
        }
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_preview_rule_omits_child_state_vs_closure() {
        let set = closure_ruleset();
        let closure = &set.rules[0];
        let preview = preview_coverage_rule(closure, "breakdown").unwrap();

        let closure_cfg = match &closure.assert {
            Assertion::LabelCoverage { config } => config,
            _ => panic!("closure must be label-coverage"),
        };
        let preview_cfg = match &preview.assert {
            Assertion::LabelCoverage { config } => config,
            _ => panic!("preview must be label-coverage"),
        };

        // Closure requires done; preview omits child-state (any state).
        assert_eq!(
            closure_cfg.get("child-state").and_then(|v| v.as_str()),
            Some("done")
        );
        assert!(
            preview_cfg.get("child-state").is_none(),
            "preview rule must OMIT child-state (D13)"
        );

        // The preview resolves its container via the brackets: label.
        assert_eq!(
            preview_cfg
                .get("container-from-label")
                .and_then(|v| v.as_str()),
            Some("brackets")
        );

        // Every other knob is carried over verbatim — the ONLY difference is the
        // child-state knob (plus the brackets indirection the closure does not
        // need because it fires directly on the container).
        for key in [
            "criteria-section",
            "marker",
            "id-pattern",
            "satisfies-namespace",
            "child-link",
            "child-type-exclude",
        ] {
            assert_eq!(
                preview_cfg.get(key),
                closure_cfg.get(key),
                "shared knob '{key}' must match the closure rule"
            );
        }
    }

    #[test]
    fn test_preview_rule_keyed_on_breakdown_type_any_state() {
        let set = closure_ruleset();
        let preview = preview_coverage_rule(&set.rules[0], "breakdown").unwrap();

        // Keyed on the breakdown type (config-driven), not epic+done.
        assert_eq!(preview.when.type_.as_deref(), Some("breakdown"));
        assert!(
            preview.when.state.is_none(),
            "preview fires on B at any state"
        );
        // Severity/enforce inherited from the closure rule.
        assert_eq!(preview.severity, set.rules[0].severity);
        assert_eq!(preview.enforce, set.rules[0].enforce);
        assert_eq!(preview.name, "hard-criteria-covered-preview");
    }

    #[test]
    fn test_preview_rule_uses_config_type_not_hardcoded() {
        // A different ruleset's breakdown type (research example) flows through
        // unchanged — proving the constructor is domain-agnostic.
        let set = closure_ruleset();
        let preview = preview_coverage_rule(&set.rules[0], "decomposition").unwrap();
        assert_eq!(preview.when.type_.as_deref(), Some("decomposition"));
    }

    #[test]
    fn test_preview_rule_rejects_non_coverage_closure() {
        let toml = r#"
[[rules]]
name = "needs-criteria"
when = { state = "ready" }
assert = { require-section = { heading = "Success Criteria" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/x")).unwrap();
        let err = preview_coverage_rule(&set.rules[0], "breakdown").unwrap_err();
        assert!(err.to_string().contains("label-coverage"));
    }

    #[test]
    fn test_preview_rule_rejects_blank_breakdown_type() {
        let set = closure_ruleset();
        let err = preview_coverage_rule(&set.rules[0], "   ").unwrap_err();
        assert!(err.to_string().contains("breakdown_type"));
    }
}
