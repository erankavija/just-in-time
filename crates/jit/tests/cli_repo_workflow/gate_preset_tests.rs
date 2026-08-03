//! CLI integration tests for project-defined gate presets (jit:52665a07,
//! REQ-02).
//!
//! Every preset is a project's own, stored under `.jit/config/gate-presets/`.
//! These tests exercise the whole mechanism against that surface end-to-end via
//! the real `jit` subprocess: capture an issue's gates into a preset
//! (`create`), see it in `list`, inspect it with `show`, and attach it to
//! another issue with `apply`.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// Run a jit command, assert success, and parse its JSON stdout.
fn jit_json(temp: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = jit(temp)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap_or_else(|e| {
        panic!(
            "expected JSON from `jit {}`: {e}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out)
        )
    })
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();
    temp
}

fn create_issue(temp: &TempDir, title: &str) -> String {
    let json = jit_json(temp, &["issue", "create", "--title", title, "--json"]);
    json["id"].as_str().expect("created issue id").to_string()
}

/// REQ-02: a project-defined preset is created from an issue's gates, then
/// listed, shown, and applied — the full mechanism against a project-declared
/// bundle, with no undeclared preset in play.
#[test]
fn test_project_defined_preset_create_list_show_apply() {
    let temp = setup_repo();

    // A reference issue configured with two gates that stand in for a project's
    // language-specific CI bundle (a test runner plus a review gate).
    let reference = create_issue(&temp, "Reference issue");
    jit(&temp)
        .args([
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
    jit(&temp)
        .args([
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
    jit(&temp)
        .args(["gate", "add", &reference, "tests", "code-review"])
        .assert()
        .success();

    // Capture those gates into a project-defined preset.
    jit(&temp)
        .args(["gate", "preset", "create", &reference, "ci"])
        .assert()
        .success();

    // The preset is stored on disk under the project's gate-presets directory.
    assert!(
        temp.path()
            .join(".jit/config/gate-presets/ci.json")
            .exists(),
        "project-defined preset should be written to .jit/config/gate-presets/"
    );

    // `list` reports exactly what the project declares. The planning-bracket
    // names are checked beside it because a listing that carried presets from
    // the binary would carry those.
    let list = jit_json(&temp, &["gate", "preset", "list", "--json"]);
    let presets = list["presets"].as_array().expect("presets array");
    assert_eq!(
        presets
            .iter()
            .map(|preset| preset["name"].as_str().expect("a named preset"))
            .collect::<Vec<_>>(),
        vec!["ci"],
        "the listing reports the project's own presets and nothing else"
    );
    assert_eq!(list["count"], 1);
    assert_eq!(presets[0]["gate_count"], 2);
    for absent in ["plan-review", "coverage-preview", "breakdown-review"] {
        let show = jit(&temp)
            .args(["gate", "preset", "show", absent, "--json"])
            .output()
            .expect("preset show should run");
        assert!(
            !show.status.success(),
            "{absent} must not resolve as a preset the project never declared"
        );
    }

    // `show` reports the bundled gates.
    let show = jit_json(&temp, &["gate", "preset", "show", "ci", "--json"]);
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
    let target = create_issue(&temp, "Feature issue");
    let applied = jit_json(&temp, &["gate", "preset", "apply", "ci", &target, "--json"]);
    let added: Vec<&str> = applied["success"][0]["gates_added"]
        .as_array()
        .expect("gates_added array")
        .iter()
        .map(|g| g.as_str().expect("added gate key"))
        .collect();
    assert!(added.contains(&"tests"));
    assert!(added.contains(&"code-review"));

    // The target issue now requires both gates.
    let status = jit_json(&temp, &["issue", "show", &target, "--json"]);
    let required: Vec<&str> = status["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .map(|g| g["key"].as_str().expect("required gate key"))
        .collect();
    assert!(required.contains(&"tests"));
    assert!(required.contains(&"code-review"));
}
