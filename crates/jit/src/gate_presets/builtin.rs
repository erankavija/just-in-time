//! Built-in gate presets

use super::{
    breakdown_review_preset, coverage_preview_preset, plan_review_preset, repo_validate_preset,
    GatePresetDefinition, GateTemplate, BREAKDOWN_REVIEW_PRESET, COVERAGE_PREVIEW_PRESET,
    PLAN_REVIEW_PRESET, REPO_VALIDATE_PRESET,
};
use crate::domain::{GateChecker, GateMode, GateStage};
use anyhow::Result;
use std::collections::HashMap;

/// Built-in presets bundled with the binary.
///
/// # Examples
///
/// ```
/// use jit::gate_presets::BuiltinPresets;
///
/// let presets = BuiltinPresets::load().unwrap();
/// assert!(presets.contains_key("rust-tdd"));
/// assert!(presets.contains_key("minimal"));
/// assert!(presets.contains_key("python-tdd"));
/// assert!(presets.contains_key("js-tdd"));
/// assert!(presets.contains_key("security-audit"));
/// assert!(presets.contains_key("plan-review"));
/// assert!(presets.contains_key("coverage-preview"));
/// assert!(presets.contains_key("breakdown-review"));
/// assert!(presets.contains_key("repo-validate"));
/// ```
pub struct BuiltinPresets;

impl BuiltinPresets {
    /// Load all built-in presets.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::gate_presets::BuiltinPresets;
    ///
    /// let presets = BuiltinPresets::load().unwrap();
    /// assert_eq!(presets.len(), 9);
    /// let security = presets.get("security-audit").unwrap();
    /// assert_eq!(security.gates.len(), 3);
    /// ```
    pub fn load() -> Result<HashMap<String, GatePresetDefinition>> {
        let mut presets = HashMap::new();

        // rust-tdd preset
        let rust_tdd = GatePresetDefinition {
            name: "rust-tdd".to_string(),
            description: "Test-driven development workflow for Rust projects".to_string(),
            gates: vec![
                GateTemplate {
                    key: "tdd-reminder".to_string(),
                    title: "Write tests first (TDD)".to_string(),
                    description: "Reminder to write failing tests before implementation"
                        .to_string(),
                    stage: GateStage::Precheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "tests".to_string(),
                    title: "All tests pass".to_string(),
                    description: "cargo test must pass".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo test".to_string(),
                        timeout_seconds: 300,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "clippy".to_string(),
                    title: "Clippy lints pass".to_string(),
                    description: "No clippy warnings allowed".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo clippy --all-targets -- -D warnings".to_string(),
                        timeout_seconds: 120,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "fmt".to_string(),
                    title: "Code formatted".to_string(),
                    description: "Code must be formatted with cargo fmt".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo fmt --check".to_string(),
                        timeout_seconds: 30,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "code-review".to_string(),
                    title: "Code review completed".to_string(),
                    description: "Another developer reviewed the code".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
            ],
        };

        // minimal preset
        let minimal = GatePresetDefinition {
            name: "minimal".to_string(),
            description: "Minimal workflow with just code review".to_string(),
            gates: vec![GateTemplate {
                key: "code-review".to_string(),
                title: "Code review completed".to_string(),
                description: "Code has been reviewed".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };

        // python-tdd preset
        let python_tdd = GatePresetDefinition {
            name: "python-tdd".to_string(),
            description: "Test-driven development workflow for Python projects".to_string(),
            gates: vec![
                GateTemplate {
                    key: "tdd-reminder".to_string(),
                    title: "Write tests first (TDD)".to_string(),
                    description: "Reminder to write failing tests before implementation"
                        .to_string(),
                    stage: GateStage::Precheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "pytest".to_string(),
                    title: "All tests pass".to_string(),
                    description: "pytest must pass".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "pytest".to_string(),
                        timeout_seconds: 300,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "black".to_string(),
                    title: "Code formatted (Black)".to_string(),
                    description: "Code must be formatted with Black".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "black --check .".to_string(),
                        timeout_seconds: 30,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "mypy".to_string(),
                    title: "Type checking passes".to_string(),
                    description: "mypy type checking must pass".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "mypy .".to_string(),
                        timeout_seconds: 120,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "code-review".to_string(),
                    title: "Code review completed".to_string(),
                    description: "Another developer reviewed the code".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
            ],
        };

        // js-tdd preset
        let js_tdd = GatePresetDefinition {
            name: "js-tdd".to_string(),
            description: "Test-driven development workflow for JavaScript/TypeScript".to_string(),
            gates: vec![
                GateTemplate {
                    key: "tdd-reminder".to_string(),
                    title: "Write tests first (TDD)".to_string(),
                    description: "Reminder to write failing tests before implementation"
                        .to_string(),
                    stage: GateStage::Precheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "jest".to_string(),
                    title: "All tests pass".to_string(),
                    description: "npm test must pass".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "npm test".to_string(),
                        timeout_seconds: 300,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "eslint".to_string(),
                    title: "ESLint passes".to_string(),
                    description: "ESLint must pass with no errors".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "npm run lint".to_string(),
                        timeout_seconds: 120,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "code-review".to_string(),
                    title: "Code review completed".to_string(),
                    description: "Another developer reviewed the code".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
            ],
        };

        // security-audit preset
        let security_audit = GatePresetDefinition {
            name: "security-audit".to_string(),
            description: "Security review workflow".to_string(),
            gates: vec![
                GateTemplate {
                    key: "security-review".to_string(),
                    title: "Security review completed".to_string(),
                    description:
                        "Review code for security vulnerabilities: injection, auth, crypto, secrets"
                            .to_string(),
                    stage: GateStage::Precheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "secret-detection".to_string(),
                    title: "No secrets in code".to_string(),
                    description: "Detect hardcoded secrets and credentials".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "gitleaks detect --no-git".to_string(),
                        timeout_seconds: 20,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
                GateTemplate {
                    key: "dependency-audit".to_string(),
                    title: "Dependency vulnerabilities checked".to_string(),
                    description: "Audit dependencies for known vulnerabilities".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo audit".to_string(),
                        timeout_seconds: 60,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                },
            ],
        };

        // Planning-bracket presets (T6): the agent plan-quality gate on the
        // planning node and the deterministic coverage-preview gate on the
        // breakdown node. Defined in `planning.rs` so the gate shapes live next
        // to the preview-rule constructor.
        let plan_review = plan_review_preset();
        let coverage_preview = coverage_preview_preset();
        // The agent quality review on the breakdown node `B`, the front-end
        // counterpart to the deterministic coverage-preview gate (quality vs
        // coverage split on `B`).
        let breakdown_review = breakdown_review_preset();
        // The whole-repo integrity gate attached to the container anchor `C` by
        // the `plan` template: `jit validate` with no issue id (run_rules(None)),
        // so a breakable container cannot reach Done until the whole repo
        // validates. Distinct from the per-issue jit-validate gate.
        let repo_validate = repo_validate_preset();

        // Validate all presets
        let all_presets = [
            &rust_tdd,
            &minimal,
            &python_tdd,
            &js_tdd,
            &security_audit,
            &plan_review,
            &coverage_preview,
            &breakdown_review,
            &repo_validate,
        ];
        for preset in &all_presets {
            preset.validate()?;
        }

        presets.insert(rust_tdd.name.clone(), rust_tdd);
        presets.insert(minimal.name.clone(), minimal);
        presets.insert(python_tdd.name.clone(), python_tdd);
        presets.insert(js_tdd.name.clone(), js_tdd);
        presets.insert(security_audit.name.clone(), security_audit);
        presets.insert(plan_review.name.clone(), plan_review);
        presets.insert(coverage_preview.name.clone(), coverage_preview);
        presets.insert(breakdown_review.name.clone(), breakdown_review);
        presets.insert(repo_validate.name.clone(), repo_validate);

        Ok(presets)
    }

    /// Get list of builtin preset names.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::gate_presets::BuiltinPresets;
    /// let names = BuiltinPresets::names();
    /// assert!(names.contains(&"rust-tdd".to_string()));
    /// assert!(names.contains(&"minimal".to_string()));
    /// ```
    pub fn names() -> Vec<String> {
        vec![
            "rust-tdd".to_string(),
            "minimal".to_string(),
            "python-tdd".to_string(),
            "js-tdd".to_string(),
            "security-audit".to_string(),
            PLAN_REVIEW_PRESET.to_string(),
            COVERAGE_PREVIEW_PRESET.to_string(),
            BREAKDOWN_REVIEW_PRESET.to_string(),
            REPO_VALIDATE_PRESET.to_string(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_presets() {
        let presets = BuiltinPresets::load().unwrap();
        assert_eq!(presets.len(), 9);
        assert!(presets.contains_key("rust-tdd"));
        assert!(presets.contains_key("minimal"));
        assert!(presets.contains_key("python-tdd"));
        assert!(presets.contains_key("js-tdd"));
        assert!(presets.contains_key("security-audit"));
        assert!(presets.contains_key("plan-review"));
        assert!(presets.contains_key("coverage-preview"));
        assert!(presets.contains_key("breakdown-review"));
        assert!(presets.contains_key("repo-validate"));
    }

    #[test]
    fn test_planning_bracket_presets_registered() {
        let presets = BuiltinPresets::load().unwrap();

        let plan_review = presets.get("plan-review").unwrap();
        assert_eq!(plan_review.gates.len(), 1);
        assert_eq!(plan_review.gates[0].key, "plan-review");
        assert_eq!(plan_review.gates[0].mode, GateMode::Auto);

        let coverage = presets.get("coverage-preview").unwrap();
        assert_eq!(coverage.gates.len(), 1);
        assert_eq!(coverage.gates[0].key, "coverage-preview");
        assert_eq!(coverage.gates[0].mode, GateMode::Auto);

        let breakdown_review = presets.get("breakdown-review").unwrap();
        assert_eq!(breakdown_review.gates.len(), 1);
        assert_eq!(breakdown_review.gates[0].key, "breakdown-review");
        assert_eq!(breakdown_review.gates[0].mode, GateMode::Auto);

        assert!(BuiltinPresets::names().contains(&"plan-review".to_string()));
        assert!(BuiltinPresets::names().contains(&"coverage-preview".to_string()));
        assert!(BuiltinPresets::names().contains(&"breakdown-review".to_string()));
    }

    // REQ-13: the production preset-load path (the one the apply engine resolves
    // anchor gates through) yields `repo-validate` wired to WHOLE-REPO validation
    // — `jit validate` with no issue id — and registered in `names()`. This proves
    // the gate the `plan` template's container anchor references is the real,
    // loaded preset, distinct from the per-issue `jit-validate` gate.
    #[test]
    fn test_repo_validate_loaded_with_whole_repo_checker() {
        use crate::domain::GateChecker;

        let presets = BuiltinPresets::load().unwrap();
        let repo_validate = presets
            .get("repo-validate")
            .expect("repo-validate must be in the loaded built-in presets");
        assert_eq!(repo_validate.gates.len(), 1);
        let gate = &repo_validate.gates[0];
        assert_eq!(gate.key, "repo-validate");
        assert_eq!(gate.mode, GateMode::Auto);
        match gate.checker.as_ref().expect("repo-validate has a checker") {
            GateChecker::Exec { command, .. } => assert_eq!(
                command, "jit validate",
                "repo-validate must run whole-repo validation (no issue id), \
                 distinct from the per-issue `jit validate \"$JIT_ISSUE_ID\"` gate"
            ),
        }
        assert!(BuiltinPresets::names().contains(&"repo-validate".to_string()));
    }

    #[test]
    fn test_rust_tdd_preset_structure() {
        let presets = BuiltinPresets::load().unwrap();
        let rust_tdd = presets.get("rust-tdd").unwrap();

        assert_eq!(rust_tdd.name, "rust-tdd");
        assert_eq!(rust_tdd.gates.len(), 5);

        // Verify gate keys
        let keys: Vec<_> = rust_tdd.gates.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["tdd-reminder", "tests", "clippy", "fmt", "code-review"]
        );

        // Verify tdd-reminder is precheck
        assert_eq!(rust_tdd.gates[0].stage, GateStage::Precheck);

        // Verify auto gates have checkers
        for gate in &rust_tdd.gates {
            if gate.mode == GateMode::Auto {
                assert!(gate.checker.is_some(), "Gate {} missing checker", gate.key);
            }
        }
    }

    #[test]
    fn test_minimal_preset_structure() {
        let presets = BuiltinPresets::load().unwrap();
        let minimal = presets.get("minimal").unwrap();

        assert_eq!(minimal.name, "minimal");
        assert_eq!(minimal.gates.len(), 1);
        assert_eq!(minimal.gates[0].key, "code-review");
        assert_eq!(minimal.gates[0].mode, GateMode::Manual);
    }

    #[test]
    fn test_builtin_presets_are_valid() {
        let presets = BuiltinPresets::load().unwrap();
        for preset in presets.values() {
            assert!(
                preset.validate().is_ok(),
                "Preset {} is invalid",
                preset.name
            );
        }
    }
}
