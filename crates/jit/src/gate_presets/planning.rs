//! Planning-bracket gate presets (design doc D8/D13, task T6).
//!
//! Three presets bundle the gates that bracket a breakable container:
//!
//! - [`plan_review_preset`] attaches the **agent** plan-quality gate to the
//!   planning node `P` (`type:planning`). It mirrors the `plan-review` gate
//!   already registered in adopting repos so applying the preset is sufficient
//!   even when that gate has not been hand-defined yet.
//! - [`coverage_preview_preset`] attaches the **deterministic** coverage-preview
//!   gate to the breakdown node `B` (`type:breakdown`). Its checker resolves the
//!   container `C` from `B`'s `brackets:<C-short-id>` label and runs
//!   `jit validate --scope <C>` (T2), which exits 4 when a `[hard]` criterion is
//!   left uncovered.
//! - [`breakdown_review_preset`] attaches the **agent** breakdown-quality gate to
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
use crate::domain::{GateChecker, GateMode, GateStage};
use crate::validation::rules::{Assertion, Rule, Selector};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Preset name for the agent plan-quality gate (planning node `P`).
pub const PLAN_REVIEW_PRESET: &str = "plan-review";

/// Preset name for the deterministic coverage-preview gate (breakdown node `B`).
pub const COVERAGE_PREVIEW_PRESET: &str = "coverage-preview";

/// Gate key bundled by [`coverage_preview_preset`].
pub const COVERAGE_PREVIEW_GATE: &str = "coverage-preview";

/// Preset name for the agent breakdown-review gate (breakdown node `B`).
pub const BREAKDOWN_REVIEW_PRESET: &str = "breakdown-review";

/// Preset name for the whole-repo integrity gate on the container anchor `C`.
pub const REPO_VALIDATE_PRESET: &str = "repo-validate";

/// Build the `plan-review` preset: an agent review gate for the planning node.
///
/// The single bundled gate is an auto (command-backed) postcheck mirroring the
/// `plan-review` gate registered in adopting repos (checker
/// `./scripts/ai-review.sh`, reviewer agent + prompt file supplied via env /
/// `prompt_file`, structured context passed). Applying this preset to a
/// `type:planning` issue attaches that gate, so the plan is reviewed before the
/// fan-out (D8).
///
/// # Examples
///
/// ```
/// use jit::gate_presets::plan_review_preset;
/// use jit::domain::GateMode;
///
/// let preset = plan_review_preset();
/// assert_eq!(preset.name, "plan-review");
/// assert_eq!(preset.gates.len(), 1);
/// let gate = &preset.gates[0];
/// assert_eq!(gate.key, "plan-review");
/// assert_eq!(gate.mode, GateMode::Auto);
/// assert!(gate.checker.is_some());
/// assert!(preset.validate().is_ok());
/// ```
pub fn plan_review_preset() -> GatePresetDefinition {
    let mut env = HashMap::new();
    env.insert("REVIEWER_AGENT".to_string(), "codex exec".to_string());

    GatePresetDefinition {
        name: PLAN_REVIEW_PRESET.to_string(),
        description: "Agent plan-quality review on the planning node before fan-out".to_string(),
        gates: vec![GateTemplate {
            key: "plan-review".to_string(),
            title: "AI Plan Review".to_string(),
            description:
                "AI-powered plan/design review before fan-out, against the planning issue's \
                 success criteria and linked design document"
                    .to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: "./scripts/ai-review.sh".to_string(),
                timeout_seconds: 1800,
                working_dir: None,
                env,
                pass_context: true,
                prompt: None,
                prompt_file: Some("./scripts/plan-review-prompt.md".to_string()),
            }),
        }],
    }
}

/// Build the `coverage-preview` preset: a deterministic coverage gate for the
/// breakdown node.
///
/// The bundled gate is an auto postcheck whose checker is
/// `./scripts/coverage-preview.sh`. JIT sets `JIT_ISSUE_ID` to the gated
/// issue (the breakdown node `B`); the script reads `B`'s `brackets:<C-short-id>`
/// label to recover the container `C` and runs `jit validate --scope <C>` (T2),
/// which exits 4 — failing the gate — when the drafted children leave a
/// `[hard]` criterion uncovered. The container thus reaches the checker via
/// issue context, mirroring the other context-bearing gates; nothing here is
/// hardcoded to a particular container type.
///
/// # Examples
///
/// ```
/// use jit::gate_presets::coverage_preview_preset;
/// use jit::domain::{GateChecker, GateMode};
///
/// let preset = coverage_preview_preset();
/// assert_eq!(preset.name, "coverage-preview");
/// let gate = &preset.gates[0];
/// assert_eq!(gate.mode, GateMode::Auto);
/// match gate.checker.as_ref().unwrap() {
///     GateChecker::Exec { command, .. } => {
///         assert_eq!(command, "./scripts/coverage-preview.sh");
///     }
/// }
/// assert!(preset.validate().is_ok());
/// ```
pub fn coverage_preview_preset() -> GatePresetDefinition {
    GatePresetDefinition {
        name: COVERAGE_PREVIEW_PRESET.to_string(),
        description: "Deterministic coverage preview on the breakdown node (scoped validate)"
            .to_string(),
        gates: vec![GateTemplate {
            key: COVERAGE_PREVIEW_GATE.to_string(),
            title: "Coverage Preview".to_string(),
            description:
                "Run scoped validation for the container resolved from the breakdown node's \
                 brackets: label; blocks when a [hard] criterion is uncovered at plan time"
                    .to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: "./scripts/coverage-preview.sh".to_string(),
                timeout_seconds: 300,
                working_dir: None,
                env: HashMap::new(),
                // Context is passed so the script can fall back to the gate
                // context file; it primarily uses JIT_ISSUE_ID.
                pass_context: true,
                prompt: None,
                prompt_file: None,
            }),
        }],
    }
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
/// The single bundled gate mirrors [`plan_review_preset`]'s command-backed agent
/// mechanism (checker `./scripts/ai-review.sh`, reviewer agent via env, structured
/// context passed) but points at `./scripts/breakdown-review-prompt.md`. Because
/// it is an ordinary postcheck gate on `B`, jit's gate enforcement is
/// self-guiding: `B` cannot complete — and the impl fan-out it gates cannot be
/// released — until the review passes.
///
/// # Examples
///
/// ```
/// use jit::gate_presets::breakdown_review_preset;
/// use jit::domain::GateMode;
///
/// let preset = breakdown_review_preset();
/// assert_eq!(preset.name, "breakdown-review");
/// assert_eq!(preset.gates.len(), 1);
/// let gate = &preset.gates[0];
/// assert_eq!(gate.key, "breakdown-review");
/// assert_eq!(gate.mode, GateMode::Auto);
/// assert!(gate.checker.is_some());
/// assert!(preset.validate().is_ok());
/// ```
pub fn breakdown_review_preset() -> GatePresetDefinition {
    let mut env = HashMap::new();
    env.insert("REVIEWER_AGENT".to_string(), "codex exec".to_string());

    GatePresetDefinition {
        name: BREAKDOWN_REVIEW_PRESET.to_string(),
        description: "Agent quality review of the decomposition on the breakdown node before \
                      fan-out"
            .to_string(),
        gates: vec![GateTemplate {
            key: "breakdown-review".to_string(),
            title: "AI Breakdown Review".to_string(),
            description:
                "AI-powered adversarial review of a breakdown against the design doc and content \
                 standards: per-child content standards, dependency-DAG coherence, and right-sized \
                 decomposition (coverage of [hard] criteria is the separate coverage-preview gate)"
                    .to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: "./scripts/ai-review.sh".to_string(),
                timeout_seconds: 1800,
                working_dir: None,
                env,
                pass_context: true,
                prompt: None,
                prompt_file: Some("./scripts/breakdown-review-prompt.md".to_string()),
            }),
        }],
    }
}

/// Build the `repo-validate` preset: a **whole-repository** integrity gate for
/// the container anchor `C`.
///
/// The single bundled gate is an auto postcheck whose checker is `jit validate`
/// with **no issue id** — the whole-repo validation entry that runs
/// `run_rules(None)` plus the repo-integrity checks (broken deps, gates, labels,
/// DAG, transitive reduction, claims index). Attached to the bound container
/// anchor by the `plan` template, it makes the container un-completable until the
/// entire repository validates (INV-GATE-SEMANTICS): a breakable container cannot
/// reach Done while whole-repo validation fails.
///
/// This is deliberately **distinct from the per-issue `jit-validate` gate**
/// (checker `jit validate "$JIT_ISSUE_ID"`, which evaluates only the issue under
/// review): this gate passes no issue id, so it validates the repository as a
/// whole. The checker is config/preset-declared; nothing about the container or
/// the gate is baked into engine logic — the wiring lives in the `plan` template
/// (`.jit/templates.toml`).
///
/// # Examples
///
/// ```
/// use jit::gate_presets::repo_validate_preset;
/// use jit::domain::{GateChecker, GateMode};
///
/// let preset = repo_validate_preset();
/// assert_eq!(preset.name, "repo-validate");
/// assert_eq!(preset.gates.len(), 1);
/// let gate = &preset.gates[0];
/// assert_eq!(gate.key, "repo-validate");
/// assert_eq!(gate.mode, GateMode::Auto);
/// match gate.checker.as_ref().unwrap() {
///     GateChecker::Exec { command, .. } => {
///         // Whole-repo validation: NO issue id (distinct from jit-validate).
///         assert_eq!(command, "jit validate");
///     }
/// }
/// assert!(preset.validate().is_ok());
/// ```
pub fn repo_validate_preset() -> GatePresetDefinition {
    GatePresetDefinition {
        name: REPO_VALIDATE_PRESET.to_string(),
        description:
            "Whole-repository integrity validation on the container before it can complete"
                .to_string(),
        gates: vec![GateTemplate {
            key: REPO_VALIDATE_PRESET.to_string(),
            title: "Repo Validate".to_string(),
            description:
                "Whole-repository validation must pass (`jit validate` with no issue id \
                          runs run_rules(None) plus the repo-integrity checks); blocks the bound \
                          container from reaching Done until the entire repository validates. \
                          Distinct from the per-issue jit-validate gate, which scopes to one issue."
                    .to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                // Whole-repo validation: `jit validate` with NO issue id. The
                // per-issue jit-validate gate instead runs `jit validate
                // "$JIT_ISSUE_ID"`; this one is deliberately repo-wide.
                command: "jit validate".to_string(),
                timeout_seconds: 120,
                working_dir: None,
                env: HashMap::new(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            }),
        }],
    }
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
/// use jit::validation::rules::RuleSet;
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
///         jit::validation::rules::Assertion::LabelCoverage { config: closure_cfg },
///         jit::validation::rules::Assertion::LabelCoverage { config: preview_cfg },
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
    use crate::validation::rules::RuleSet;
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
        let preset = plan_review_preset();
        assert_eq!(preset.name, "plan-review");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "plan-review");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        match gate.checker.as_ref().expect("agent gate has a checker") {
            GateChecker::Exec {
                command,
                pass_context,
                prompt_file,
                env,
                ..
            } => {
                assert_eq!(command, "./scripts/ai-review.sh");
                assert!(*pass_context, "agent gate passes structured context");
                assert_eq!(
                    prompt_file.as_deref(),
                    Some("./scripts/plan-review-prompt.md")
                );
                assert_eq!(
                    env.get("REVIEWER_AGENT").map(String::as_str),
                    Some("codex exec")
                );
            }
        }
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_breakdown_review_preset_attaches_agent_gate() {
        let preset = breakdown_review_preset();
        assert_eq!(preset.name, "breakdown-review");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "breakdown-review");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        match gate.checker.as_ref().expect("agent gate has a checker") {
            GateChecker::Exec {
                command,
                pass_context,
                prompt_file,
                env,
                ..
            } => {
                assert_eq!(command, "./scripts/ai-review.sh");
                assert!(*pass_context, "agent gate passes structured context");
                assert_eq!(
                    prompt_file.as_deref(),
                    Some("./scripts/breakdown-review-prompt.md")
                );
                assert_eq!(
                    env.get("REVIEWER_AGENT").map(String::as_str),
                    Some("codex exec")
                );
            }
        }
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_coverage_preview_preset_runs_scoped_validate() {
        let preset = coverage_preview_preset();
        assert_eq!(preset.name, "coverage-preview");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "coverage-preview");
        assert_eq!(gate.mode, GateMode::Auto);

        match gate.checker.as_ref().expect("coverage gate has a checker") {
            GateChecker::Exec {
                command,
                pass_context,
                ..
            } => {
                // The checker resolves C from B's brackets: label (via
                // JIT_ISSUE_ID) and runs `jit validate --scope <C>`.
                assert_eq!(command, "./scripts/coverage-preview.sh");
                assert!(*pass_context);
            }
        }
        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_repo_validate_preset_runs_whole_repo_validate() {
        let preset = repo_validate_preset();
        assert_eq!(preset.name, "repo-validate");
        assert_eq!(preset.gates.len(), 1);

        let gate = &preset.gates[0];
        assert_eq!(gate.key, "repo-validate");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        match gate
            .checker
            .as_ref()
            .expect("repo-validate gate has a checker")
        {
            GateChecker::Exec { command, .. } => {
                // Whole-repo validation: `jit validate` with NO issue id (→
                // run_rules(None)). It must NOT be the per-issue jit-validate
                // checker, which scopes to a single issue via $JIT_ISSUE_ID.
                assert_eq!(command, "jit validate");
                assert!(
                    !command.contains("JIT_ISSUE_ID"),
                    "repo-validate must be whole-repo, not the per-issue jit-validate checker"
                );
            }
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
