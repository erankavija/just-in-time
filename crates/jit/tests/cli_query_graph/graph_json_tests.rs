use std::process::Command;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> crate::TaxonomyRepo {
    crate::setup_test_repo_with_taxonomy()
}

#[test]
fn test_graph_downstream_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues with dependency
    let output1 = Command::new(jit)
        .args(["issue", "create", "-t", "Task A", "-d", "First"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    let output2 = Command::new(jit)
        .args(["issue", "create", "-t", "Task B", "-d", "Second"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    // Add dependency: id2 depends on id1
    Command::new(jit)
        .args(["dep", "add", &id2, &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query downstream dependents
    let output = Command::new(jit)
        .args(["graph", "downstream", &id1, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    // success field removed
    assert_eq!(json["issue_id"], id1);
    assert!(json["dependents"].is_array());
    assert_eq!(json["count"], 1);
}

#[test]
fn test_graph_roots_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues with dependency
    let output1 = Command::new(jit)
        .args(["issue", "create", "-t", "Root Task", "-d", "First"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    let output2 = Command::new(jit)
        .args(["issue", "create", "-t", "Dependent Task", "-d", "Second"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    // Add dependency: id2 depends on id1
    Command::new(jit)
        .args(["dep", "add", &id2, &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query root issues
    let output = Command::new(jit)
        .args(["graph", "roots", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    // success field removed
    assert!(json["roots"].is_array());
    assert_eq!(json["count"], 1);
    assert_eq!(json["roots"][0]["id"], id1);
}

/// Create a root issue and a dependent of it; returns `(root_id, dependent_id)`.
fn create_root_and_dependent(temp: &crate::TaxonomyRepo) -> (String, String) {
    let create = |title: &str| {
        let output = Command::new(jit_binary())
            .args(["issue", "create", "-t", title, "-d", "Desc"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(2)
            .unwrap()
            .to_string()
    };
    let root = create("Root Task");
    let dependent = create("Dependent Task");
    Command::new(jit_binary())
        .args(["dep", "add", &dependent, &root])
        .current_dir(temp.path())
        .output()
        .unwrap();
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
    let temp = setup_test_repo();
    let (root, dependent) = create_root_and_dependent(&temp);

    let output = Command::new(jit_binary())
        .args(["graph", "rdeps", &root])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert_indented_row_and_total(&String::from_utf8_lossy(&output.stdout), &dependent);
}

#[test]
fn test_graph_roots_human_output_ends_with_total() {
    let temp = setup_test_repo();
    let (root, _) = create_root_and_dependent(&temp);

    let output = Command::new(jit_binary())
        .args(["graph", "roots"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert_indented_row_and_total(&String::from_utf8_lossy(&output.stdout), &root);
}

// ============================================================================
// REQ-03 (jit:1a63ef75): `jit graph export --json` is sugar for `--format
// json` on stdout, composes with `--full`, and conflicts with an explicit
// non-json `--format`.
// ============================================================================

#[test]
fn test_graph_export_json_is_sugar_for_format_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();
    Command::new(jit)
        .args(["issue", "create", "-t", "A", "--orphan"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let via_json_flag = Command::new(jit)
        .args(["graph", "export", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let via_format_flag = Command::new(jit)
        .args(["graph", "export", "--format", "json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(via_json_flag.status.success());
    assert!(via_format_flag.status.success());
    let a: serde_json::Value = serde_json::from_slice(&via_json_flag.stdout).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&via_format_flag.stdout).unwrap();
    assert_eq!(
        a, b,
        "--json should produce the same output as --format json"
    );
}

#[test]
fn test_graph_export_json_composes_with_full() {
    let temp = setup_test_repo();
    let jit = jit_binary();
    Command::new(jit)
        .args(["issue", "create", "-t", "A", "--orphan"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(jit)
        .args(["graph", "export", "--json", "--full"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "--json --full failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // The full shape carries resolved-hierarchy fields absent from the summary shape.
    let node = json["nodes"][0].as_object().unwrap();
    for field in ["parent", "children", "cluster", "rank"] {
        assert!(node.contains_key(field), "missing {field} in --full node");
    }
}

#[test]
fn test_graph_export_json_conflicts_with_explicit_dot_format() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(jit)
        .args(["graph", "export", "--json", "--format", "dot"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_graph_export_json_conflicts_with_explicit_mermaid_format() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(jit)
        .args(["graph", "export", "--json", "--format", "mermaid"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}
