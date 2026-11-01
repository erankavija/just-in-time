//! CLI integration tests for project-defined gate presets (jit:52665a07,
//! REQ-02).
//!
//! Every preset is a project's own, stored under `.jit/config/gate-presets/`.
//! These tests exercise the whole mechanism against that surface end-to-end via
//! the real `jit` subprocess: capture an issue's gates into a preset
//! (`create`), see it in `list`, inspect it with `show`, and attach it to
//! another issue with `apply`.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

/// REQ-02: a project-defined preset is created from an issue's gates, then
/// listed, shown, and applied — the full mechanism against a project-declared
/// bundle, with no undeclared preset in play.
#[test]
fn test_project_defined_preset_create_list_show_apply() {
    let cli = JitCli::initialised().unwrap();

    // A reference issue configured with two gates that stand in for a project's
    // language-specific CI bundle (a test runner plus a review gate).
    let reference = cli.create_issue(["--title", "Reference issue"]).unwrap();
    cli.command([
        "gate",
        "define",
        "tests",
        "--title",
        "All tests pass",
        "--description",
        "run the test suite",
        "--stage",
        "postcheck",
        "--mode",
        "auto",
        "--checker-command",
        "make test",
        "--timeout",
        "300",
    ])
    .assert()
    .success();
    cli.command([
        "gate",
        "define",
        "code-review",
        "--title",
        "Code review",
        "--description",
        "human review",
    ])
    .assert()
    .success();
    cli.command(["gate", "add", &reference, "tests", "code-review"])
        .assert()
        .success();

    // Capture those gates into a project-defined preset.
    cli.command(["gate", "preset", "create", &reference, "ci"])
        .assert()
        .success();

    // The preset is stored on disk under the project's gate-presets directory.
    assert!(
        cli.root().join(".jit/config/gate-presets/ci.json").exists(),
        "project-defined preset should be written to .jit/config/gate-presets/"
    );

    // `list` reports exactly what the project declares. The planning-bracket
    // names are checked beside it because a listing that carried presets from
    // the binary would carry those.
    let list = cli.run_json(["gate", "preset", "list"]).unwrap();
    let presets = list["presets"].as_array().expect("presets array");
    assert_eq!(
        presets
            .iter()
            .map(|preset| preset["name"].as_str().expect("a named preset"))
            .collect::<Vec<_>>(),
        vec!["ci"],
        "the listing reports the project's own presets and nothing else"
    );
    assert_eq!(presets[0]["gate_count"], 2);
    for absent in ["plan-review", "coverage-preview", "breakdown-review"] {
        let show = cli
            .command(["gate", "preset", "show", absent, "--json"])
            .output()
            .expect("preset show should run");
        assert!(
            !show.status.success(),
            "{absent} must not resolve as a preset the project never declared"
        );
    }

    // `show` reports the bundled gates.
    let show = cli.run_json(["gate", "preset", "show", "ci"]).unwrap();
    assert_eq!(show["name"], "ci");
    let keys: Vec<&str> = show["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .map(|g| g["key"].as_str().expect("gate key"))
        .collect();
    assert!(keys.contains(&"tests"), "ci bundles the tests gate");
    assert!(
        keys.contains(&"code-review"),
        "ci bundles the code-review gate"
    );

    // `apply` attaches the bundled gates to a fresh issue.
    let target = cli.create_issue(["--title", "Feature issue"]).unwrap();
    let applied = cli
        .run_json(["gate", "preset", "apply", "ci", &target])
        .unwrap();
    let added: Vec<&str> = applied["success"][0]["gates_added"]
        .as_array()
        .expect("gates_added array")
        .iter()
        .map(|g| g.as_str().expect("added gate key"))
        .collect();
    assert!(added.contains(&"tests"));
    assert!(added.contains(&"code-review"));

    // The target issue now requires both gates.
    let status = cli.run_json(["issue", "show", &target]).unwrap();
    let required: Vec<&str> = status["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .map(|g| g["key"].as_str().expect("required gate key"))
        .collect();
    assert!(required.contains(&"tests"));
    assert!(required.contains(&"code-review"));
}

#[test]
fn test_gate_preset_create_missing_registry_gate_names_the_gate() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "mygate",
        "--title",
        "My gate",
        "--description",
        "My gate",
    ])
    .unwrap();
    let issue = cli.create_issue(["--title", "Has gate"]).unwrap();
    cli.run(["gate", "add", &issue, "mygate"]).unwrap();
    cli.run(["gate", "remove", "mygate"]).unwrap();

    let output = cli
        .command(["gate", "preset", "create", &issue, "mypreset"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Gate not found in registry: mygate"),
        "stderr was: {stderr}"
    );
}
