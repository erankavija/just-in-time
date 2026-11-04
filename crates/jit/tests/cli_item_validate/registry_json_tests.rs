//! JSON-output tests for the gate registry surface (`jit gate list` / `gate show`).
//!
//! Gate-registry configuration lives solely under `jit gate`; these tests cover
//! the machine-readable shapes of its list and show verbs.

use jit::test_cli::JitCli;

#[test]
fn test_gate_list_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "test-gate",
        "-t",
        "Test Gate",
        "-d",
        "A test gate",
        "--auto",
        "--checker-command",
        "true",
    ])
    .unwrap();

    let json = cli.run_json(["gate", "list"]).unwrap();

    assert_eq!(json["gates"].as_array().unwrap().len(), 1);
    assert_eq!(json["gates"][0]["key"], "test-gate");
    assert_eq!(json["gates"][0]["title"], "Test Gate");
    assert_eq!(json["gates"][0]["mode"], "auto");
}

#[test]
fn test_gate_show_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "test-gate",
        "-t",
        "Test Gate",
        "-d",
        "A test gate description",
        "--auto",
        "--checker-command",
        "true",
    ])
    .unwrap();

    let json = cli.run_json(["gate", "show", "test-gate"]).unwrap();

    assert_eq!(json["key"], "test-gate");
    assert_eq!(json["title"], "Test Gate");
    assert_eq!(json["description"], "A test gate description");
    assert_eq!(json["mode"], "auto");
}

#[test]
fn test_gate_list_empty_json_output() {
    let cli = JitCli::initialised().unwrap();

    let json = cli.run_json(["gate", "list"]).unwrap();

    assert_eq!(json["gates"].as_array().unwrap().len(), 0);
}

#[test]
fn test_gate_define_with_stage_option() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "tdd-reminder",
        "-t",
        "TDD Reminder",
        "-d",
        "Write tests first",
        "--stage",
        "precheck",
    ])
    .expect("define a gate with --stage precheck");

    let json = cli.run_json(["gate", "show", "tdd-reminder"]).unwrap();

    assert_eq!(json["key"], "tdd-reminder");
    assert_eq!(json["stage"], "precheck");
    assert_eq!(json["mode"], "manual");
}

#[test]
fn test_gate_define_defaults_to_postcheck() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "code-review",
        "-t",
        "Code Review",
        "-d",
        "Manual review",
    ])
    .unwrap();

    let json = cli.run_json(["gate", "show", "code-review"]).unwrap();

    assert_eq!(json["key"], "code-review");
    assert_eq!(json["stage"], "postcheck");
}

#[test]
fn test_gate_define_with_invalid_stage() {
    let cli = JitCli::initialised().unwrap();

    let output = cli
        .command([
            "gate",
            "define",
            "invalid-gate",
            "-t",
            "Invalid",
            "-d",
            "Invalid stage",
            "--stage",
            "invalid",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid") || stderr.contains("stage"));
}
