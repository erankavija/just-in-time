use jit::test_cli::JitCli;

#[test]
fn test_issue_not_found_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Try to show non-existent issue
    let output = cli
        .command(["issue", "show", "nonexistent", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("NOT_FOUND"));
    assert!(json["error"]["message"].is_string());
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_cycle_detected_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Create two issues
    let id1 = cli.create_issue(["-t", "Task A", "-d", "First"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task B", "-d", "Second"]).unwrap();

    // Add A depends on B
    cli.command(["dep", "add", &id1, &id2]).output().unwrap();

    // Try to add B depends on A (creates cycle)
    let output = cli
        .command(["dep", "add", &id2, &id1, "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("CYCLE_DETECTED"));
    assert!(json["error"]["message"].as_str().unwrap().contains("cycle"));
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_deletion_not_confirmed_error_json() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Doomed"]).unwrap();

    // Delete without JIT_ALLOW_DELETION=1: refused (REQ-02).
    let output = cli
        .command(["issue", "delete", &id, "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        output.status.code(),
        Some(2),
        "refusal must exit 2 (REQ-01)"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["error"]["code"], "DELETION_NOT_CONFIRMED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("discouraged"));
    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str().unwrap().contains("JIT_ALLOW_DELETION=1")),
        "suggestions must carry the confirmation hint, got: {suggestions:?}"
    );

    // The issue must still exist: the refusal must not have deleted it.
    let show = cli.command(["issue", "show", &id]).output().unwrap();
    assert!(show.status.success(), "issue must survive a refused delete");
}

#[test]
fn test_invalid_state_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Try to query with invalid state
    let output = cli
        .command(["query", "all", "--state", "invalid_state", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("the payload stream must carry the JSON error envelope");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "the JSON error envelope must carry a non-empty message: {json}"
    );
}

#[test]
fn test_gate_operation_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let id = cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    // Try to evaluate a gate that wasn't added to the issue
    let output = cli
        .command(["gate", "evaluate", &id, "nonexistent-gate", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("the payload stream must carry the JSON error envelope");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "the JSON error envelope must carry a non-empty message: {json}"
    );
}
