//! End-to-end coverage for the DAG-resolved hierarchy surfaces:
//! `jit graph tree`, `jit graph export --full` (resolved fields), and
//! `jit query divergence`.

use jit::test_cli::JitCli;
use jit::test_taxonomy::{
    membership_label, setup_test_repo_with_taxonomy, type_label, TaxonomyRepo,
};

#[test]
fn test_graph_tree_resolves_parent_and_children() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    // epic → task (the epic depends on the work it contains)
    let epic_label = membership_label(&taxonomy, 2, "auth");
    let epic = cli
        .create_issue([
            "-t",
            "Epic",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 2),
            "-l",
            &epic_label,
        ])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-d", "body", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap();

    let doc = cli.run_json(["graph", "tree"]).unwrap();
    assert_eq!(doc["nodes"].as_array().unwrap().len(), 2);
    let nodes = doc["nodes"].as_array().unwrap();

    let epic_node = nodes.iter().find(|n| n["id"] == epic).unwrap();
    assert_eq!(epic_node["parent"], serde_json::Value::Null);
    assert_eq!(epic_node["children"][0], task);
    assert_eq!(epic_node["type"], taxonomy.type_at_level(2));

    let task_node = nodes.iter().find(|n| n["id"] == task).unwrap();
    assert_eq!(task_node["parent"], epic);
    // The epic is the strategic root, so the task clusters to it.
    assert_eq!(task_node["cluster"], epic);
}

#[test]
fn test_graph_tree_scoped_to_root_uses_dependency_closure() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let epic = cli
        .create_issue(["-t", "Epic", "-d", "body", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-d", "body", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    let other = cli
        .create_issue([
            "-t",
            "Unrelated",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 4),
        ])
        .unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap();

    let doc = cli.run_json(["graph", "tree", &epic]).unwrap();
    // Only the epic and its dependency closure; `other` is excluded.
    assert_eq!(doc["nodes"].as_array().unwrap().len(), 2);
    let ids: Vec<&str> = doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&epic.as_str()));
    assert!(ids.contains(&task.as_str()));
    assert!(!ids.contains(&other.as_str()));
}

#[test]
fn test_graph_export_full_carries_resolved_fields() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let epic = cli
        .create_issue(["-t", "Epic", "-d", "body", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-d", "body", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap();

    let doc = cli
        .run_json(["graph", "export", "--format", "json", "--full"])
        .unwrap();
    let nodes = doc["nodes"].as_array().unwrap();
    let task_node = nodes.iter().find(|n| n["id"] == task).unwrap();
    assert_eq!(task_node["parent"], epic);
    assert_eq!(task_node["cluster"], epic);
    assert_eq!(task_node["children"], serde_json::json!([]));
    assert_eq!(task_node["rank"], 0);

    let epic_node = nodes.iter().find(|n| n["id"] == epic).unwrap();
    assert_eq!(epic_node["children"], serde_json::json!([task]));
    assert_eq!(epic_node["rank"], 1);

    // Default summary shape stays free of the resolved fields.
    let summary = cli
        .run_json(["graph", "export", "--format", "json"])
        .unwrap();
    let summary_node = &summary["nodes"][0];
    for field in ["parent", "children", "cluster", "rank"] {
        assert!(summary_node.get(field).is_none());
    }
}

/// The `--full` export and `graph tree` agree, node for node, on all four
/// resolution fields: they share one serialization path.
#[test]
fn test_graph_export_full_and_tree_agree_on_resolution_fields() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let milestone = cli
        .create_issue([
            "-t",
            "Milestone",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 1),
        ])
        .unwrap();
    let epic = cli
        .create_issue(["-t", "Epic", "-d", "body", "-l", &type_label(&taxonomy, 2)])
        .unwrap();
    let task = cli
        .create_issue(["-t", "Task", "-d", "body", "-l", &type_label(&taxonomy, 4)])
        .unwrap();
    cli.run(["dep", "add", &milestone, &epic]).unwrap();
    cli.run(["dep", "add", &epic, &task]).unwrap();

    let export = cli
        .run_json(["graph", "export", "--format", "json", "--full"])
        .unwrap();
    let tree = cli.run_json(["graph", "tree"]).unwrap();

    let pick = |node: &serde_json::Value| {
        serde_json::json!({
            "parent": node["parent"],
            "children": node["children"],
            "cluster": node["cluster"],
            "rank": node["rank"],
        })
    };

    let tree_nodes = tree["nodes"].as_array().unwrap();
    assert_eq!(tree_nodes.len(), 3);
    for tree_node in tree_nodes {
        let export_node = export["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == tree_node["id"])
            .expect("every tree node is exported");
        assert_eq!(pick(export_node), pick(tree_node), "id {}", tree_node["id"]);
    }
}

#[test]
fn test_query_divergence_flags_unbacked_membership_label() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    // The epic contains `inside` via the DAG but `stray` only claims the label.
    let membership = membership_label(&taxonomy, 2, "auth");
    let epic = cli
        .create_issue([
            "-t",
            "Auth",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 2),
            "-l",
            &membership,
        ])
        .unwrap();
    let inside = cli
        .create_issue([
            "-t",
            "Inside",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 4),
            "-l",
            &membership,
        ])
        .unwrap();
    let stray = cli
        .create_issue([
            "-t",
            "Stray",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 4),
            "-l",
            &membership,
        ])
        .unwrap();
    cli.run(["dep", "add", &epic, &inside]).unwrap();

    let doc = cli.run_json(["query", "divergence"]).unwrap();
    assert_eq!(doc["divergences"].as_array().unwrap().len(), 1);
    assert_eq!(doc["divergences"][0]["id"], stray);
    assert_eq!(doc["divergences"][0]["label"], membership);
}

#[test]
fn test_validate_surfaces_divergence_count_without_failing() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    // A connected, otherwise-valid graph: `stray` carries epic:auth but the DAG
    // places it under `other`, not under the `auth` epic → one divergence.
    let auth_membership = membership_label(&taxonomy, 2, "auth");
    let other_membership = membership_label(&taxonomy, 2, "other");
    let auth = cli
        .create_issue([
            "-t",
            "Auth",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 2),
            "-l",
            &auth_membership,
        ])
        .unwrap();
    let inside = cli
        .create_issue([
            "-t",
            "Inside",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 4),
        ])
        .unwrap();
    let other = cli
        .create_issue([
            "-t",
            "Other",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 2),
            "-l",
            &other_membership,
        ])
        .unwrap();
    let stray = cli
        .create_issue([
            "-t",
            "Stray",
            "-d",
            "body",
            "-l",
            &type_label(&taxonomy, 4),
            "-l",
            &auth_membership,
        ])
        .unwrap();
    cli.run(["dep", "add", &auth, &inside]).unwrap();
    cli.run(["dep", "add", &other, &stray]).unwrap();

    // Advisory: validate still exits 0 despite the divergence.
    let doc = cli.run_json(["validate"]).unwrap();
    assert_eq!(doc["divergence_count"], 1);
    assert_eq!(doc["valid"], true);
}
