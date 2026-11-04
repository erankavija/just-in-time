use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_graph_downstream_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli.create_issue(["-t", "Task A", "-d", "First"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task B", "-d", "Second"]).unwrap();
    // id2 depends on id1
    cli.run(["dep", "add", &id2, &id1]).unwrap();

    let json = cli.run_json(["graph", "downstream", &id1]).unwrap();

    assert_eq!(json["issue_id"], id1);
    assert_eq!(json["dependents"].as_array().unwrap().len(), 1);
}

#[test]
fn test_graph_roots_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli
        .create_issue(["-t", "Root Task", "-d", "First"])
        .unwrap();
    let id2 = cli
        .create_issue(["-t", "Dependent Task", "-d", "Second"])
        .unwrap();
    // id2 depends on id1
    cli.run(["dep", "add", &id2, &id1]).unwrap();

    let json = cli.run_json(["graph", "roots"]).unwrap();

    assert_eq!(json["roots"].as_array().unwrap().len(), 1);
    assert_eq!(json["roots"][0]["id"], id1);
}

/// Create a root issue and a dependent of it; returns `(root_id, dependent_id)`.
fn seed_root_and_dependent(cli: &JitCli) -> (String, String) {
    let root = cli.create_issue(["-t", "Root Task", "-d", "Desc"]).unwrap();
    let dependent = cli
        .create_issue(["-t", "Dependent Task", "-d", "Desc"])
        .unwrap();
    cli.run(["dep", "add", &dependent, &root]).unwrap();
    (root, dependent)
}

fn assert_indented_row_and_total(stdout: &str, id: &str) {
    let row = stdout
        .lines()
        .find(|line| line.contains(id))
        .unwrap_or_else(|| panic!("output lists {id}: {stdout}"));
    assert!(row.starts_with("  "), "row is indented: {row:?}");
    assert!(stdout.trim_end().ends_with("Total: 1"), "{stdout}");
}

#[test]
fn test_graph_rdeps_human_output_ends_with_total() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let (root, dependent) = seed_root_and_dependent(&cli);

    let stdout = cli.run(["graph", "rdeps", &root]).unwrap();

    assert_indented_row_and_total(&stdout, &dependent);
}

#[test]
fn test_graph_roots_human_output_ends_with_total() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let (root, _) = seed_root_and_dependent(&cli);

    let stdout = cli.run(["graph", "roots"]).unwrap();

    assert_indented_row_and_total(&stdout, &root);
}

// ============================================================================
// REQ-03 (jit:1a63ef75): `jit graph export --json` is sugar for `--format
// json` on stdout, composes with `--full`, and conflicts with an explicit
// non-json `--format`.
// ============================================================================

#[test]
fn test_graph_export_json_is_sugar_for_format_json() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "A"]).unwrap();

    let via_json_flag = cli.run(["graph", "export", "--json"]).unwrap();
    let via_format_flag = cli.run(["graph", "export", "--format", "json"]).unwrap();

    let a: serde_json::Value = serde_json::from_str(&via_json_flag).unwrap();
    let b: serde_json::Value = serde_json::from_str(&via_format_flag).unwrap();
    assert_eq!(
        a, b,
        "--json should produce the same output as --format json"
    );
}

#[test]
fn test_graph_export_json_composes_with_full() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "A"]).unwrap();

    let json = cli.run_json(["graph", "export", "--full"]).unwrap();
    // The full shape carries resolved-hierarchy fields absent from the summary shape.
    let node = json["nodes"][0].as_object().unwrap();
    for field in ["parent", "children", "cluster", "rank"] {
        assert!(node.contains_key(field), "missing {field} in --full node");
    }
}

#[test]
fn test_graph_export_json_conflicts_with_explicit_dot_format() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    let output = cli
        .command(["graph", "export", "--json", "--format", "dot"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_graph_export_json_conflicts_with_explicit_mermaid_format() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    let output = cli
        .command(["graph", "export", "--json", "--format", "mermaid"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}
