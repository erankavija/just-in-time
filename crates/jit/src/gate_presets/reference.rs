//! Projection of the gate-preset contract into a committed markdown reference.
//!
//! [`render_reference_markdown`] projects the preset contract — how a project
//! captures, lists, and applies a bundle — together with the canonical
//! `.jit/gates.toml` syntax for every portable checker type, into the committed
//! reference. A conformance test in this module asserts the
//! committed copy equals the projection, so changing the projected syntax
//! without regenerating the reference fails the test suite
//! (`@/inv/single-source-prose`).

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    /// Repo-relative path of the committed gate-preset reference.
    pub const REFERENCE_PATH: &str = "docs/reference/gate-presets.md";

    /// The command that renders [`REFERENCE_PATH`], named in the conformance test's
    /// message so a stale reference carries its own repair.
    pub const REFERENCE_GENERATOR: &str = "./scripts/generate-gate-presets-reference.sh";
}

/// Canonical `.jit/gates.toml` syntax for every native checker type.
const PORTABLE_CHECKER_REGISTRY_EXAMPLES: &str = r#"[[gates]]
version = 1
key = "repository-policy"
title = "Repository validation"
description = "Validate the whole repository"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "repository_validation"

[[gates]]
version = 1
key = "work-item-policy"
title = "Issue validation"
description = "Validate the gated issue"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "issue_validation"

[[gates]]
version = 1
key = "container-coverage"
title = "Container coverage"
description = "Validate the container named by the covers label"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "label_target_validation"
label_namespace = "covers"

[[gates]]
version = 1
key = "external-review"
title = "External review"
description = "Passing placeholder until a reviewer is configured"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "review_placeholder""#;

/// Render the gate-preset reference as the committed markdown page.
///
/// The returned string is the full contents of the committed reference. The
/// portable-checker syntax block is the projected value, read from the same
/// constant the loader test parses, so the page cannot drift from the syntax
/// the gate registry accepts. The conformance test in this module asserts the
/// committed file equals this output.
pub fn render_reference_markdown() -> String {
    format!(
        "<!-- Generated from `crate::gate_presets::reference` — do not edit by hand. -->\n\
         \n\
         # Gate Presets\n\
         \n\
         > **Diátaxis Type:** Reference\n\
         \n\
         A gate preset is a named bundle of gate definitions a project declares for\n\
         itself. `jit gate preset create <issue> <name>` captures an issue's gates into\n\
         `.jit/config/gate-presets/<name>.json`, and every JSON file in that directory\n\
         loads as a preset whose name must equal its filename stem. `jit gate preset list`\n\
         reports them, `jit gate preset show <preset>` prints one, and\n\
         `jit gate preset apply <preset> <id>...` inserts each bundled gate into the\n\
         project's gate registry (`.jit/gates.toml`) under its key — for keys the registry\n\
         does not already carry, and, with `--timeout <seconds>`, overwriting the key with\n\
         the overridden checker timeout — and then adds those keys to each issue's required\n\
         gates. `--no-precheck`, `--no-postcheck`, and `--except <key>` narrow which of the\n\
         preset's gates are applied. Each gate materializes into the registry with\n\
         `version = 1`, `priority = 100`, and `auto` set from its mode.\n\
         \n\
         The gates a project enforces live in its own `.jit/gates.toml`, its settings in\n\
         `.jit/config.toml`; render those with `jit project render` (see\n\
         [Rules and Gates](rules-and-gates.md)). A profile package can contribute gate\n\
         definitions when it is applied; see [Repository Profiles](profiles.md).\n\
         \n\
         ## Portable checker types\n\
         \n\
         Automated gate definitions can use `exec` or one of four in-process checker types.\n\
         The in-process checkers do not invoke a shell, a second `jit` binary, or `jq`, and the\n\
         configured gate key does not change their behavior:\n\
         \n\
         - `repository_validation` runs structural and declarative validation for the whole\n\
           repository.\n\
         - `issue_validation` runs declarative validation for the gated issue.\n\
         - `label_target_validation` reads exactly one `<label_namespace>:<target-id>` label\n\
           from the gated issue and runs scoped validation for that target. Its checker table\n\
           must set `label_namespace`.\n\
         - `review_placeholder` passes so a workflow can be installed before an external\n\
           reviewer is selected, but records an advisory structured finding and prints\n\
           `WARNING: EXTERNAL REVIEW PLACEHOLDER`. Whole-repository validation also warns\n\
           while any gate uses it. Replace it with an `exec` checker (for example, `jit gate\n\
           update <key> --checker-command <command>`) before treating the gate as review\n\
           evidence.\n\
         \n\
         Native checker types are selected in the gate registry; `jit gate define` does not\n\
         have a checker-type option. These four independent definitions show the canonical\n\
         `.jit/gates.toml` syntax. The keys are examples and can be replaced with any\n\
         configured gate keys:\n\
         \n\
         ```toml\n\
         {portable_checker_registry_examples}\n\
         ```\n",
        portable_checker_registry_examples = PORTABLE_CHECKER_REGISTRY_EXAMPLES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateChecker;
    use std::path::PathBuf;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(test_support::REFERENCE_PATH)
    }

    /// REQ-03 conformance: the committed reference must equal the projection.
    /// Changing the projected checker syntax without regenerating the reference
    /// fails here.
    #[test]
    fn test_committed_reference_matches_projection() {
        let committed = std::fs::read_to_string(reference_path())
            .expect("committed gate-presets reference should exist");
        assert_eq!(
            committed,
            render_reference_markdown(),
            "{} is stale — regenerate it (run: {})",
            test_support::REFERENCE_PATH,
            test_support::REFERENCE_GENERATOR,
        );
    }

    #[test]
    fn test_portable_checker_registry_examples_load_as_canonical_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("gates.toml"),
            PORTABLE_CHECKER_REGISTRY_EXAMPLES,
        )
        .unwrap();

        let registry = crate::storage::gate_store::load_gate_registry(dir.path()).unwrap();
        assert_eq!(registry.gates.len(), 4);
        assert!(matches!(
            registry.gates["repository-policy"].checker.as_ref(),
            Some(GateChecker::RepositoryValidation)
        ));
        assert!(matches!(
            registry.gates["work-item-policy"].checker.as_ref(),
            Some(GateChecker::IssueValidation)
        ));
        assert!(matches!(
            registry.gates["container-coverage"].checker.as_ref(),
            Some(GateChecker::LabelTargetValidation {
                label_namespace
            }) if label_namespace == "covers"
        ));
        assert!(matches!(
            registry.gates["external-review"].checker.as_ref(),
            Some(GateChecker::ReviewPlaceholder)
        ));
    }

    /// Every checker type the registry accepts reaches the page, so a new
    /// variant cannot ship undocumented.
    #[test]
    fn test_render_documents_every_portable_checker_the_examples_declare() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("gates.toml"),
            PORTABLE_CHECKER_REGISTRY_EXAMPLES,
        )
        .unwrap();
        let registry = crate::storage::gate_store::load_gate_registry(dir.path()).unwrap();
        let page = render_reference_markdown();

        for gate in registry.gates.values() {
            let wire = match gate.checker.as_ref().expect("an example checker") {
                GateChecker::RepositoryValidation => "repository_validation",
                GateChecker::IssueValidation => "issue_validation",
                GateChecker::LabelTargetValidation { .. } => "label_target_validation",
                GateChecker::ReviewPlaceholder => "review_placeholder",
                GateChecker::Exec { .. } => panic!("the examples declare no exec checker"),
            };
            assert!(
                page.contains(&format!("`{wire}`")),
                "{wire} is declared in the examples but not described on the page"
            );
        }
    }

    /// Rendering is deterministic: repeated renders are byte-identical.
    #[test]
    fn test_render_is_deterministic() {
        let first = render_reference_markdown();
        for _ in 0..8 {
            assert_eq!(first, render_reference_markdown());
        }
    }
}
