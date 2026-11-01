//! CLI integration tests for `jit graph export --scope` and `--format batch`
//! (issue 3e12ffbd), exercised as a subprocess end to end.

use jit::domain::identity::IssueId;
use jit::test_cli::JitCli;
use jit::test_taxonomy::{setup_test_repo_with_taxonomy, type_label, TaxonomyRepo};

#[test]
fn test_graph_export_batch_emits_parseable_array() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let epic = cli
        .create_issue(["-t", "Epic", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap(); // epic depends on task

    let stdout = cli.run(["graph", "export", "--format", "batch"]).unwrap();

    let defs: serde_json::Value =
        serde_json::from_str(&stdout).expect("batch output is a JSON array");
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // No lifecycle fields leak into the batch shape.
    assert!(!stdout.contains("\"state\""));
    assert!(!stdout.contains("\"created_at\""));
    // The epic's def depends on the task by its short key.
    let epic_def = arr
        .iter()
        .find(|d| d["title"] == "Epic")
        .expect("epic def present");
    let task_id: IssueId = task
        .parse()
        .expect("the binary prints a canonical issue id");
    assert_eq!(epic_def["depends_on"][0], task_id.short_id());
}

#[test]
fn test_graph_export_scope_composes_with_json_format() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let epic = cli
        .create_issue(["-t", "Epic", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    let other = cli
        .create_issue(["-t", "Other", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap();

    // Scoped JSON export lists only the epic's membership (epic + task), not the
    // unrelated `other` epic.
    let json = cli
        .run_json(["graph", "export", "--format", "json", "--scope", &epic])
        .unwrap();
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
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let source = JitCli::with_repository(temp);
    let epic = source
        .create_issue(["-t", "Auth", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = source
        .create_issue(["-t", "Login", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    source.run(["dep", "add", &epic, &task]).unwrap();

    let seed = source.root().join("seed.json");
    source
        .run([
            "graph",
            "export",
            "--format",
            "batch",
            "--scope",
            &epic,
            "--output",
            seed.to_str().unwrap(),
        ])
        .unwrap();

    // Import the seed into a fresh repository and confirm the subgraph recreates.
    let fresh = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    fresh
        .run([
            "issue",
            "batch-create",
            "--from-json",
            seed.to_str().unwrap(),
        ])
        .unwrap();

    // The fresh repo now holds two issues with the epic → task edge.
    let json = fresh
        .run_json(["graph", "export", "--format", "json"])
        .unwrap();
    assert_eq!(json["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(json["edges"].as_array().unwrap().len(), 1);
}

#[test]
fn test_graph_export_batch_rejects_conflicting_flags() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    cli.create_issue(["-t", "Solo", "-l", &type_label(&taxonomy, 4)])
        .unwrap();

    // `--json` (sugar for --format json) conflicts with `--format batch`.
    let json_conflict = cli
        .command(["graph", "export", "--format", "batch", "--json"])
        .output()
        .unwrap();
    assert!(!json_conflict.status.success());

    // `--full` is JSON-only; pairing it with batch is a usage error.
    let full_conflict = cli
        .command(["graph", "export", "--format", "batch", "--full"])
        .output()
        .unwrap();
    assert!(!full_conflict.status.success());
}
