use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit = jit_binary();
    Command::new(jit)
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

#[test]
fn test_graph_deps_depth_default() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create a 3-level dependency chain: A -> B -> C
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // Default behavior (no --depth flag) should show immediate deps only (depth 1)
    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let tree = json["tree"].as_array().unwrap();
    assert_eq!(tree.len(), 1); // Only B (immediate dependency)
    assert_eq!(tree[0]["id"], b);

    // Verify depth is reported
    assert_eq!(json["depth"], 1);
}

#[test]
fn test_graph_deps_depth_2() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create: A -> B -> C
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // --depth 2 should show B and C
    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--depth", "2", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Now uses tree structure
    let tree = json["tree"].as_array().unwrap();

    // Collect all issue IDs from tree (including children)
    let mut all_ids = Vec::new();
    fn collect_ids(nodes: &[serde_json::Value], ids: &mut Vec<String>) {
        for node in nodes {
            ids.push(node["id"].as_str().unwrap().to_string());
            if let Some(children) = node["children"].as_array() {
                collect_ids(children, ids);
            }
        }
    }
    collect_ids(tree, &mut all_ids);

    assert_eq!(all_ids.len(), 2); // B and C
    assert!(all_ids.contains(&b));
    assert!(all_ids.contains(&c));

    assert_eq!(json["depth"], 2);
}

#[test]
fn test_graph_deps_depth_unlimited() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create: A -> B -> C -> D
    let d = create_issue(jit, &temp, "Task D", "Deep");
    let c = create_issue_with_dep(jit, &temp, "Task C", "Level 3", &d);
    let b = create_issue_with_dep(jit, &temp, "Task B", "Level 2", &c);
    let a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // --depth 0 should show all transitive dependencies
    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--depth", "0", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Collect all IDs from tree
    let tree = json["tree"].as_array().unwrap();
    let mut all_ids = Vec::new();
    fn collect_ids(nodes: &[serde_json::Value], ids: &mut Vec<String>) {
        for node in nodes {
            ids.push(node["id"].as_str().unwrap().to_string());
            if let Some(children) = node["children"].as_array() {
                collect_ids(children, ids);
            }
        }
    }
    collect_ids(tree, &mut all_ids);

    assert_eq!(all_ids.len(), 3); // B, C, D

    assert_eq!(json["depth"], 0); // 0 = unlimited
}

// Helper functions
fn create_issue(jit: &str, temp: &TempDir, title: &str, desc: &str) -> String {
    let output = Command::new(jit)
        .args(["issue", "create", "-t", title, "-d", desc])
        .current_dir(temp.path())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn create_issue_with_dep(
    jit: &str,
    temp: &TempDir,
    title: &str,
    desc: &str,
    dep_id: &str,
) -> String {
    let id = create_issue(jit, temp, title, desc);
    Command::new(jit)
        .args(["dep", "add", &id, dep_id])
        .current_dir(temp.path())
        .output()
        .unwrap();
    id
}

fn create_issue_with_deps(
    jit: &str,
    temp: &TempDir,
    title: &str,
    desc: &str,
    deps: &[&str],
) -> String {
    let id = create_issue(jit, temp, title, desc);
    for dep in deps {
        Command::new(jit)
            .args(["dep", "add", &id, dep])
            .current_dir(temp.path())
            .output()
            .unwrap();
    }
    id
}

#[test]
fn test_graph_deps_tree_structure() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create tree: A -> B -> C
    //                -> D
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let d = create_issue(jit, &temp, "Task D", "Another leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_deps(jit, &temp, "Task A", "Root", &[&b, &d]);

    // Test tree output with depth 2
    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--depth", "2", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should have tree structure
    let tree = &json["tree"];
    assert!(tree.is_array());

    let tree_nodes = tree.as_array().unwrap();
    assert_eq!(tree_nodes.len(), 2); // B and D at level 1

    // Find node B and check it has child C
    let node_b = tree_nodes.iter().find(|n| n["id"] == b).unwrap();
    assert_eq!(node_b["level"], 1);
    assert_eq!(node_b["children"].as_array().unwrap().len(), 1);
    assert_eq!(node_b["children"][0]["id"], c);
    assert_eq!(node_b["children"][0]["level"], 2);

    // Node D should have no children
    let node_d = tree_nodes.iter().find(|n| n["id"] == d).unwrap();
    assert_eq!(node_d["level"], 1);
    assert_eq!(node_d["children"].as_array().unwrap().len(), 0);
}

#[test]
fn test_graph_deps_diamond_detection() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create diamond: A -> B -> D
    //                   -> C -> D
    let d = create_issue(jit, &temp, "Task D", "Shared");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Path 1", &d);
    let c = create_issue_with_dep(jit, &temp, "Task C", "Path 2", &d);
    let a = create_issue_with_deps(jit, &temp, "Task A", "Root", &[&b, &c]);

    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--depth", "2", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let tree = json["tree"].as_array().unwrap();

    // Both B and C should have D as child, and D should be marked as shared
    let node_b = tree.iter().find(|n| n["id"] == b).unwrap();
    let d_in_b = &node_b["children"][0];
    assert_eq!(d_in_b["id"], d);
    assert_eq!(d_in_b["shared"], true);

    let node_c = tree.iter().find(|n| n["id"] == c).unwrap();
    let d_in_c = &node_c["children"][0];
    assert_eq!(d_in_c["id"], d);
    assert_eq!(d_in_c["shared"], true);
}

#[test]
fn test_graph_deps_summary_stats() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different states
    let done1 = create_issue(jit, &temp, "Done 1", "Complete");
    Command::new(jit)
        .args(["issue", "update", &done1, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let done2 = create_issue(jit, &temp, "Done 2", "Also complete");
    Command::new(jit)
        .args(["issue", "update", &done2, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let ready = create_issue(jit, &temp, "Ready", "To do");

    let a = create_issue_with_deps(jit, &temp, "Task A", "Root", &[&done1, &done2, &ready]);

    let output = Command::new(jit)
        .args(["graph", "deps", &a, "--depth", "1", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let summary = &json["summary"];
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["by_state"]["done"], 2);
    assert_eq!(summary["by_state"]["ready"], 1);
}

// ── rdeps depth tests ────────────────────────────────────────────────────────

/// `graph rdeps` with no `--depth` flag must be bounded (depth 1 = immediate
/// dependents only), consistent with `graph deps` defaulting to depth 1.
#[test]
fn test_graph_rdeps_depth_default_is_bounded() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create a 3-level chain: C <- B <- A  (A depends on B, B depends on C)
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // Default (no --depth): rdeps of C should return B only, not A
    let output = Command::new(jit)
        .args(["graph", "rdeps", &c, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let dependents = json["dependents"].as_array().unwrap();
    // Only immediate dependent B, not transitive A
    assert_eq!(
        dependents.len(),
        1,
        "default rdeps must be bounded to depth 1"
    );
    assert_eq!(dependents[0]["id"], b);
    assert!(
        !dependents.iter().any(|d| d["id"] == a),
        "A must not appear at depth 1"
    );
}

/// `graph rdeps --depth 0` must traverse all transitive dependents (opt-in unbounded).
#[test]
fn test_graph_rdeps_depth_zero_is_unbounded() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create chain: C <- B <- A
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // --depth 0 = unlimited: rdeps of C should include both B and A
    let output = Command::new(jit)
        .args(["graph", "rdeps", &c, "--depth", "0", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let dependents = json["dependents"].as_array().unwrap();
    let ids: Vec<&str> = dependents
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        dependents.len(),
        2,
        "--depth 0 must include all transitive dependents"
    );
    assert!(ids.contains(&b.as_str()), "B must be present");
    assert!(ids.contains(&a.as_str()), "A must be present");
}

/// Both `graph deps` and `graph rdeps` default to depth 1 (symmetric behaviour).
#[test]
fn test_graph_rdeps_and_deps_both_default_to_depth_1() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Chain: C <- B <- A
    let c = create_issue(jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(jit, &temp, "Task B", "Middle", &c);
    let _a = create_issue_with_dep(jit, &temp, "Task A", "Root", &b);

    // deps default: A's immediate dep is B only
    let deps_out = Command::new(jit)
        .args(["graph", "deps", &_a, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(deps_out.status.success());
    let deps_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&deps_out.stdout)).unwrap();
    assert_eq!(
        deps_json["depth"], 1,
        "graph deps must report depth 1 by default"
    );

    // rdeps default: C's immediate dependent is B only
    let rdeps_out = Command::new(jit)
        .args(["graph", "rdeps", &c, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(rdeps_out.status.success());
    let rdeps_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&rdeps_out.stdout)).unwrap();
    let dependents = rdeps_json["dependents"].as_array().unwrap();
    // Exactly 1 immediate dependent
    assert_eq!(
        dependents.len(),
        1,
        "graph rdeps must also be bounded to depth 1 by default"
    );
}
