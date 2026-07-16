//! CLI integration tests for `jit graph export --scope` and `--format batch`
//! (issue 3e12ffbd), exercised as a subprocess end to end.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

/// Create an issue and return its id (the third whitespace token of the success
/// line, matching the sibling graph tests).
fn create(temp: &TempDir, title: &str, labels: &[&str]) -> String {
    let mut args = vec!["issue", "create", "-t", title];
    for label in labels {
        args.push("-l");
        args.push(label);
    }
    let output = Command::new(jit_binary())
        .args(&args)
        .current_dir(temp.path())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string()
}

fn dep_add(temp: &TempDir, issue: &str, dep: &str) {
    Command::new(jit_binary())
        .args(["dep", "add", issue, dep])
        .current_dir(temp.path())
        .output()
        .unwrap();
}

#[test]
fn test_graph_export_batch_emits_parseable_array() {
    let temp = init_repo();
    let epic = create(&temp, "Epic", &["type:epic"]);
    let task = create(&temp, "Task", &["type:task"]);
    dep_add(&temp, &epic, &task); // epic depends on task

    let output = Command::new(jit_binary())
        .args(["graph", "export", "--format", "batch"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let defs: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("batch output is a JSON array");
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // No lifecycle fields leak into the batch shape.
    let raw = String::from_utf8_lossy(&output.stdout);
    assert!(!raw.contains("\"state\""));
    assert!(!raw.contains("\"created_at\""));
    // The epic's def depends on the task by its short key.
    let epic_def = arr
        .iter()
        .find(|d| d["title"] == "Epic")
        .expect("epic def present");
    let short_task: String = task.chars().take(8).collect();
    assert_eq!(epic_def["depends_on"][0], short_task);
}

#[test]
fn test_graph_export_scope_composes_with_json_format() {
    let temp = init_repo();
    let epic = create(&temp, "Epic", &["type:epic"]);
    let task = create(&temp, "Task", &["type:task"]);
    let other = create(&temp, "Other", &["type:epic"]);
    dep_add(&temp, &epic, &task);

    // Scoped JSON export lists only the epic's membership (epic + task), not the
    // unrelated `other` epic.
    let output = Command::new(jit_binary())
        .args(["graph", "export", "--format", "json", "--scope", &epic])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let ids: Vec<String> = json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&epic));
    assert!(ids.contains(&task));
    assert!(!ids.contains(&other));
}

#[test]
fn test_graph_export_batch_roundtrips_into_fresh_repo() {
    let source = init_repo();
    let epic = create(&source, "Auth", &["type:epic"]);
    let task = create(&source, "Login", &["type:task"]);
    dep_add(&source, &epic, &task);

    let seed = source.path().join("seed.json");
    let export = Command::new(jit_binary())
        .args([
            "graph",
            "export",
            "--format",
            "batch",
            "--scope",
            &epic,
            "--output",
            seed.to_str().unwrap(),
        ])
        .current_dir(source.path())
        .output()
        .unwrap();
    assert!(export.status.success());

    // Import the seed into a fresh repository and confirm the subgraph recreates.
    let fresh = init_repo();
    let created = Command::new(jit_binary())
        .args([
            "issue",
            "batch-create",
            "--from-json",
            seed.to_str().unwrap(),
            "--json",
        ])
        .current_dir(fresh.path())
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "batch-create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );

    // The fresh repo now holds two issues with the epic → task edge.
    let tree = Command::new(jit_binary())
        .args(["graph", "export", "--format", "json"])
        .current_dir(fresh.path())
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&tree.stdout).unwrap();
    assert_eq!(json["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(json["edges"].as_array().unwrap().len(), 1);
}

#[test]
fn test_graph_export_batch_rejects_conflicting_flags() {
    let temp = init_repo();
    create(&temp, "Solo", &["type:task"]);

    // `--json` (sugar for --format json) conflicts with `--format batch`.
    let json_conflict = Command::new(jit_binary())
        .args(["graph", "export", "--format", "batch", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!json_conflict.status.success());

    // `--full` is JSON-only; pairing it with batch is a usage error.
    let full_conflict = Command::new(jit_binary())
        .args(["graph", "export", "--format", "batch", "--full"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!full_conflict.status.success());
}
