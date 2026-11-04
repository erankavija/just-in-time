//! Integration tests for gate modification flags in issue update command
//!
//! Tests the `--add-gate` and `--remove-gate` flags for `jit issue update`

use super::create_titled_issue;
use jit::test_cli::JitCli;

fn define_titled_gate(temp: &JitCli, key: &str, title: &str) {
    let output = temp
        .command([
            "gate",
            "define",
            key,
            "--title",
            title,
            "--description",
            "Test gate",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to define gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn get_issue_json(temp: &JitCli, id: &str) -> serde_json::Value {
    let output = temp
        .command(["issue", "show", id, "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to show issue");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json.clone()
}

/// Extract the set of required gate keys from an `issue show --json` response.
///
/// `issue show --json` emits `gates: [{ key, status, last_run_at, exit_code }]`;
/// these tests assert membership of gate keys, so collapse it to the key list.
fn gate_keys(issue: &serde_json::Value) -> Vec<String> {
    issue["gates"]
        .as_array()
        .expect("issue show --json must include a gates array")
        .iter()
        .map(|g| g["key"].as_str().expect("gate key is a string").to_string())
        .collect()
}

#[test]
fn test_add_single_gate() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command(["issue", "update", &id, "--add-gate", "tests"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(gates.contains(&"tests".to_string()));
}

#[test]
fn test_add_multiple_gates_multiple_flags() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    define_titled_gate(&temp, "clippy", "Run clippy");
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--add-gate",
            "tests",
            "--add-gate",
            "clippy",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(gates.contains(&"tests".to_string()));
    assert!(gates.contains(&"clippy".to_string()));
}

#[test]
fn test_add_multiple_gates_repeated_flags() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    define_titled_gate(&temp, "clippy", "Run clippy");
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--add-gate",
            "tests",
            "--add-gate",
            "clippy",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(gates.contains(&"tests".to_string()));
    assert!(gates.contains(&"clippy".to_string()));
}

#[test]
fn test_remove_gate() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    define_titled_gate(&temp, "clippy", "Run clippy");
    // Create issue with gates
    let id = temp
        .create_issue(["-t", "Test issue", "--gate", "tests", "--gate", "clippy"])
        .unwrap();

    // Remove one gate
    let output = temp
        .command(["issue", "update", &id, "--remove-gate", "tests"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to remove gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(!gates.contains(&"tests".to_string()));
    assert!(gates.contains(&"clippy".to_string()));
}

#[test]
fn test_add_and_remove_gates_combined() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    define_titled_gate(&temp, "clippy", "Run clippy");
    define_titled_gate(&temp, "fmt", "Run fmt");
    // Create issue with one gate
    let id = temp
        .create_issue(["-t", "Test issue", "--gate", "tests"])
        .unwrap();

    // Add clippy and fmt, remove tests
    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--add-gate",
            "clippy",
            "--add-gate",
            "fmt",
            "--remove-gate",
            "tests",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to modify gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(!gates.contains(&"tests".to_string()));
    assert!(gates.contains(&"clippy".to_string()));
    assert!(gates.contains(&"fmt".to_string()));
}

#[test]
fn test_add_gate_invalid_key_error() {
    let temp = JitCli::initialised().unwrap();
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command(["issue", "update", &id, "--add-gate", "nonexistent"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "Should fail for invalid gate");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("Gate"),
        "Error should mention gate not found: {}",
        stderr
    );
}

#[test]
fn test_add_gate_idempotent() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    let id = create_titled_issue(&temp, "Test issue");

    // Add gate first time
    let output = temp
        .command(["issue", "update", &id, "--add-gate", "tests"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Add same gate again (should be no-op)
    let output = temp
        .command(["issue", "update", &id, "--add-gate", "tests"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    // Should have exactly one "tests" gate
    let test_count = gates.iter().filter(|g| g.as_str() == "tests").count();
    assert_eq!(test_count, 1, "Gate should only appear once");
}

#[test]
fn test_batch_add_gates() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "code-review", "Code review");

    // Create multiple issues with same label
    let _id1 = create_titled_issue(&temp, "Task 1");
    let _id2 = create_titled_issue(&temp, "Task 2");

    // Update issues by setting label first (new issues start in ready state)
    temp.command([
        "issue",
        "update",
        "--filter",
        "state:ready",
        "--label",
        "type:task",
    ])
    .output()
    .unwrap();

    // Batch add gate using filter
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--add-gate",
            "code-review",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed batch gate add: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify both issues have the gate
    let output = temp
        .command(["query", "all", "--label", "type:task", "--full", "--json"])
        .output()
        .unwrap();

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = result["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2);

    for issue in issues {
        let gates = issue["gates_required"].as_array().unwrap();
        assert!(gates.contains(&serde_json::json!("code-review")));
    }
}

#[test]
fn test_gates_in_json_output() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    let id = create_titled_issue(&temp, "Test issue");

    let update = temp
        .command(["issue", "update", &id, "--add-gate", "tests", "--json"])
        .output()
        .unwrap();

    assert!(update.status.success());

    // `issue update --json` returns a lean confirmation; verify the gate was
    // attached by reading it back via `issue show --json`.
    let show = temp
        .command(["issue", "show", &id, "--json"])
        .output()
        .unwrap();

    assert!(show.status.success());
    let result: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let gates = gate_keys(&result);
    assert!(gates.contains(&"tests".to_string()));
}

#[test]
fn test_combine_with_other_updates() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--state",
            "ready",
            "--add-gate",
            "tests",
            "--label",
            "type:task",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed combined update: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    assert_eq!(issue["state"], "ready");
    assert!(gate_keys(&issue).contains(&"tests".to_string()));
    assert!(issue["labels"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("type:task")));
}

#[test]
fn test_remove_nonexistent_gate_is_noop() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    let id = create_titled_issue(&temp, "Test issue");

    let output = temp
        .command(["issue", "update", &id, "--remove-gate", "tests"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Removing non-existent gate should succeed as no-op"
    );

    let issue = get_issue_json(&temp, &id);
    let gates = gate_keys(&issue);
    assert!(gates.is_empty());
}

#[test]
fn test_batch_remove_gates() {
    let temp = JitCli::initialised().unwrap();
    define_titled_gate(&temp, "tests", "Run tests");
    // Create issues with gates
    temp.command([
        "issue",
        "create",
        "-t",
        "Task 1",
        "--gate",
        "tests",
        "--label",
        "type:task",
    ])
    .output()
    .unwrap();

    temp.command([
        "issue",
        "create",
        "-t",
        "Task 2",
        "--gate",
        "tests",
        "--label",
        "type:task",
    ])
    .output()
    .unwrap();

    // Batch remove gate
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--remove-gate",
            "tests",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed batch gate remove: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify gates removed
    let output = temp
        .command(["query", "all", "--label", "type:task", "--full", "--json"])
        .output()
        .unwrap();

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = result["issues"].as_array().unwrap();

    for issue in issues {
        let gates = issue["gates_required"].as_array().unwrap();
        assert!(!gates.contains(&serde_json::json!("tests")));
    }
}

#[test]
fn test_gate_key_arguments_reject_malformed_tokens_at_dispatch() {
    let cli = JitCli::initialised().unwrap();
    let id = create_titled_issue(&cli, "Malformed gate argument target");
    for arguments in [
        vec![
            "issue",
            "create",
            "--title",
            "Invalid gate",
            "--gate",
            "Bad_Key",
        ],
        vec!["issue", "update", &id, "--add-gate", "Bad_Key"],
        vec!["issue", "update", &id, "--remove-gate", "Bad_Key"],
        vec![
            "gate",
            "define",
            "Bad_Key",
            "--title",
            "Invalid gate",
            "--description",
            "Invalid gate",
        ],
        vec!["gate", "update", "Bad_Key", "--title", "Invalid gate"],
        vec!["gate", "show", "Bad_Key"],
        vec!["gate", "remove", "Bad_Key"],
        vec!["gate", "add", &id, "Bad_Key"],
        vec!["gate", "evaluate", &id, "Bad_Key"],
        vec!["gate", "evaluate-many", "Bad_Key", &id],
        vec!["gate", "fail", &id, "Bad_Key"],
        vec!["gate", "status", &id, "Bad_Key"],
        vec!["gate", "status", &id, "--all", "--gate", "Bad_Key"],
        vec!["gate", "preset", "apply", "ci", &id, "--except", "Bad_Key"],
    ] {
        let output = cli
            .command(arguments.iter().copied().chain(["--json"]))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            envelope["error"]["code"], "INVALID_ARGUMENT",
            "{arguments:?}"
        );
        assert!(envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Bad_Key"));
    }
}
