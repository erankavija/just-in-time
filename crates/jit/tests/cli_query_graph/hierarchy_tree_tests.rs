//! End-to-end coverage for the DAG-resolved hierarchy surfaces:
//! `jit graph tree`, `jit graph export --full` (resolved fields), and
//! `jit query divergence`.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> crate::TaxonomyRepo {
    crate::setup_test_repo_with_taxonomy()
}

/// Create an issue with the given type + membership labels; returns its full id.
fn create<L: AsRef<str>>(temp: &TempDir, title: &str, labels: &[L]) -> String {
    let mut args = vec!["issue", "create", "-t", title, "-d", "body"];
    for label in labels {
        args.push("-l");
        args.push(label.as_ref());
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

fn add_dep(temp: &TempDir, from: &str, to: &str) {
    Command::new(jit_binary())
        .args(["dep", "add", from, to])
        .current_dir(temp.path())
        .output()
        .unwrap();
}

fn run_json(temp: &TempDir, args: &[&str]) -> serde_json::Value {
    let output = Command::new(jit_binary())
        .args(args)
        .current_dir(temp.path())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "expected JSON from {args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn test_graph_tree_resolves_parent_and_children() {
    let temp = setup_test_repo();
    // epic → task (the epic depends on the work it contains)
    let epic_label = crate::membership_label(&temp.taxonomy, 2, "auth");
    let epic = create(
        &temp,
        "Epic",
        &[crate::type_label(&temp.taxonomy, 2), epic_label],
    );
    let task = create(&temp, "Task", &[crate::type_label(&temp.taxonomy, 4)]);
    add_dep(&temp, &epic, &task);

    let doc = run_json(&temp, &["graph", "tree", "--json"]);
    assert_eq!(doc["count"], 2);
    let nodes = doc["nodes"].as_array().unwrap();

    let epic_node = nodes.iter().find(|n| n["id"] == epic).unwrap();
    assert_eq!(epic_node["parent"], serde_json::Value::Null);
    assert_eq!(epic_node["children"][0], task);
    assert_eq!(epic_node["type"], temp.taxonomy.type_at_level(2));

    let task_node = nodes.iter().find(|n| n["id"] == task).unwrap();
    assert_eq!(task_node["parent"], epic);
    // The epic is the strategic root, so the task clusters to it.
    assert_eq!(task_node["cluster"], epic);
}

#[test]
fn test_graph_tree_scoped_to_root_uses_dependency_closure() {
    let temp = setup_test_repo();
    let epic = create(&temp, "Epic", &[crate::type_label(&temp.taxonomy, 2)]);
    let task = create(&temp, "Task", &[crate::type_label(&temp.taxonomy, 4)]);
    let other = create(&temp, "Unrelated", &[crate::type_label(&temp.taxonomy, 4)]);
    add_dep(&temp, &epic, &task);

    let doc = run_json(&temp, &["graph", "tree", &epic, "--json"]);
    // Only the epic and its dependency closure; `other` is excluded.
    assert_eq!(doc["count"], 2);
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
    let temp = setup_test_repo();
    let epic = create(&temp, "Epic", &[crate::type_label(&temp.taxonomy, 2)]);
    let task = create(&temp, "Task", &[crate::type_label(&temp.taxonomy, 4)]);
    add_dep(&temp, &epic, &task);

    let doc = run_json(&temp, &["graph", "export", "--format", "json", "--full"]);
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
    let summary = run_json(&temp, &["graph", "export", "--format", "json"]);
    let summary_node = &summary["nodes"][0];
    for field in ["parent", "children", "cluster", "rank"] {
        assert!(summary_node.get(field).is_none());
    }
}

/// The `--full` export and `graph tree` agree, node for node, on all four
/// resolution fields: they share one serialization path.
#[test]
fn test_graph_export_full_and_tree_agree_on_resolution_fields() {
    let temp = setup_test_repo();
    let milestone = create(&temp, "Milestone", &[crate::type_label(&temp.taxonomy, 1)]);
    let epic = create(&temp, "Epic", &[crate::type_label(&temp.taxonomy, 2)]);
    let task = create(&temp, "Task", &[crate::type_label(&temp.taxonomy, 4)]);
    add_dep(&temp, &milestone, &epic);
    add_dep(&temp, &epic, &task);

    let export = run_json(&temp, &["graph", "export", "--format", "json", "--full"]);
    let tree = run_json(&temp, &["graph", "tree", "--json"]);

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
    let temp = setup_test_repo();
    // The epic contains `inside` via the DAG but `stray` only claims the label.
    let membership = crate::membership_label(&temp.taxonomy, 2, "auth");
    let epic = create(
        &temp,
        "Auth",
        &[crate::type_label(&temp.taxonomy, 2), membership.clone()],
    );
    let inside = create(
        &temp,
        "Inside",
        &[crate::type_label(&temp.taxonomy, 4), membership.clone()],
    );
    let stray = create(
        &temp,
        "Stray",
        &[crate::type_label(&temp.taxonomy, 4), membership.clone()],
    );
    add_dep(&temp, &epic, &inside);

    let doc = run_json(&temp, &["query", "divergence", "--json"]);
    assert_eq!(doc["count"], 1);
    assert_eq!(doc["divergences"][0]["id"], stray);
    assert_eq!(doc["divergences"][0]["label"], membership);
}

#[test]
fn test_validate_surfaces_divergence_count_without_failing() {
    let temp = setup_test_repo();
    // A connected, otherwise-valid graph: `stray` carries epic:auth but the DAG
    // places it under `other`, not under the `auth` epic → one divergence.
    let auth_membership = crate::membership_label(&temp.taxonomy, 2, "auth");
    let other_membership = crate::membership_label(&temp.taxonomy, 2, "other");
    let auth = create(
        &temp,
        "Auth",
        &[
            crate::type_label(&temp.taxonomy, 2),
            auth_membership.clone(),
        ],
    );
    let inside = create(&temp, "Inside", &[crate::type_label(&temp.taxonomy, 4)]);
    let other = create(
        &temp,
        "Other",
        &[crate::type_label(&temp.taxonomy, 2), other_membership],
    );
    let stray = create(
        &temp,
        "Stray",
        &[crate::type_label(&temp.taxonomy, 4), auth_membership],
    );
    add_dep(&temp, &auth, &inside);
    add_dep(&temp, &other, &stray);

    let output = Command::new(jit_binary())
        .args(["validate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    // Advisory: validate still exits 0 despite the divergence.
    assert!(
        output.status.success(),
        "validate must not fail on divergence"
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doc["divergence_count"], 1);
    assert_eq!(doc["valid"], true);
}
