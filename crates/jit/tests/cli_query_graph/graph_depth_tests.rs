use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

/// Create an issue depending on each of `deps`.
fn create_issue_with_deps(cli: &JitCli, title: &str, desc: &str, deps: &[&str]) -> String {
    let id = cli.create_issue(["-t", title, "-d", desc]).unwrap();
    for dep in deps {
        cli.run(["dep", "add", &id, dep]).unwrap();
    }
    id
}

fn collect_ids(nodes: &[serde_json::Value], ids: &mut Vec<String>) {
    for node in nodes {
        ids.push(node["id"].as_str().unwrap().to_string());
        if let Some(children) = node["children"].as_array() {
            collect_ids(children, ids);
        }
    }
}

#[test]
fn test_graph_deps_depth_default() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create a 3-level dependency chain: A -> B -> C
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // Default behavior (no --depth flag) should show immediate deps only (depth 1)
    let json = cli.run_json(["graph", "deps", &a]).unwrap();

    let tree = json["nodes"].as_array().unwrap();
    assert_eq!(tree.len(), 1); // Only B (immediate dependency)
    assert_eq!(tree[0]["id"], b);

    // Verify depth is reported
    assert_eq!(json["depth"], 1);
}

#[test]
fn test_graph_deps_depth_2() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create: A -> B -> C
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // --depth 2 should show B and C
    let json = cli.run_json(["graph", "deps", &a, "--depth", "2"]).unwrap();

    // Collect all issue IDs from tree (including children)
    let mut all_ids = Vec::new();
    collect_ids(json["nodes"].as_array().unwrap(), &mut all_ids);

    assert_eq!(all_ids.len(), 2); // B and C
    assert!(all_ids.contains(&b));
    assert!(all_ids.contains(&c));

    assert_eq!(json["depth"], 2);
}

#[test]
fn test_graph_deps_depth_unlimited() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create: A -> B -> C -> D
    let d = create_issue_with_deps(&cli, "Task D", "Deep", &[]);
    let c = create_issue_with_deps(&cli, "Task C", "Level 3", &[&d]);
    let b = create_issue_with_deps(&cli, "Task B", "Level 2", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // --depth 0 should show all transitive dependencies
    let json = cli.run_json(["graph", "deps", &a, "--depth", "0"]).unwrap();

    let mut all_ids = Vec::new();
    collect_ids(json["nodes"].as_array().unwrap(), &mut all_ids);

    assert_eq!(all_ids.len(), 3); // B, C, D

    assert_eq!(json["depth"], 0); // 0 = unlimited
}

#[test]
fn test_graph_deps_tree_structure() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create tree: A -> B -> C
    //                -> D
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let d = create_issue_with_deps(&cli, "Task D", "Another leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b, &d]);

    let json = cli.run_json(["graph", "deps", &a, "--depth", "2"]).unwrap();

    // Should have tree structure
    let tree = &json["nodes"];
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
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create diamond: A -> B -> D
    //                   -> C -> D
    let d = create_issue_with_deps(&cli, "Task D", "Shared", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Path 1", &[&d]);
    let c = create_issue_with_deps(&cli, "Task C", "Path 2", &[&d]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b, &c]);

    let json = cli.run_json(["graph", "deps", &a, "--depth", "2"]).unwrap();

    let tree = json["nodes"].as_array().unwrap();

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
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    let done1 = create_issue_with_deps(&cli, "Done 1", "Complete", &[]);
    cli.run(["issue", "update", &done1, "--state", "done"])
        .unwrap();
    let done2 = create_issue_with_deps(&cli, "Done 2", "Also complete", &[]);
    cli.run(["issue", "update", &done2, "--state", "done"])
        .unwrap();
    let ready = create_issue_with_deps(&cli, "Ready", "To do", &[]);

    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&done1, &done2, &ready]);

    let json = cli.run_json(["graph", "deps", &a, "--depth", "1"]).unwrap();

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
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create a 3-level chain: C <- B <- A  (A depends on B, B depends on C)
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // Default (no --depth): rdeps of C should return B only, not A
    let json = cli.run_json(["graph", "rdeps", &c]).unwrap();

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
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create chain: C <- B <- A
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // --depth 0 = unlimited: rdeps of C should include both B and A
    let json = cli
        .run_json(["graph", "rdeps", &c, "--depth", "0"])
        .unwrap();

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
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Chain: C <- B <- A
    let c = create_issue_with_deps(&cli, "Task C", "Leaf", &[]);
    let b = create_issue_with_deps(&cli, "Task B", "Middle", &[&c]);
    let a = create_issue_with_deps(&cli, "Task A", "Root", &[&b]);

    // deps default: A's immediate dep is B only
    let deps_json = cli.run_json(["graph", "deps", &a]).unwrap();
    assert_eq!(
        deps_json["depth"], 1,
        "graph deps must report depth 1 by default"
    );

    // rdeps default: C's immediate dependent is B only
    let rdeps_json = cli.run_json(["graph", "rdeps", &c]).unwrap();
    let dependents = rdeps_json["dependents"].as_array().unwrap();
    // Exactly 1 immediate dependent
    assert_eq!(
        dependents.len(),
        1,
        "graph rdeps must also be bounded to depth 1 by default"
    );
}
