//! Wiring of repeatable `--label` filters across the query family, `jit issue
//! list`, the top-level `jit list` alias and the bare `jit query` form; AND
//! semantics are owned by `domain::queries`.

use jit::test_cli::JitCli;
use jit::test_taxonomy::{
    membership_label, setup_test_repo_with_taxonomy, type_label, TaxonomyRepo,
};
use serde_json::Value;

/// Create an issue with the given labels, returning its id.
fn create_labeled_issue<L: AsRef<str>>(cli: &JitCli, title: &str, labels: &[L]) -> String {
    let description = format!("Description for {title}");
    let args = ["--title", title, "--description", &description]
        .into_iter()
        .chain(labels.iter().flat_map(|label| ["--label", label.as_ref()]));
    cli.create_issue(args).unwrap()
}

fn ids(json: &Value) -> Vec<String> {
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

// ── jit query all ────────────────────────────────────────────────────────

#[test]
fn test_query_all_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);
    create_labeled_issue(&cli, "ComponentOnly", &["component:api"]);

    let json = cli
        .run_json([
            "query",
            "all",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit query available ──────────────────────────────────────────────────

#[test]
fn test_query_available_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);

    let json = cli
        .run_json([
            "query",
            "available",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit query blocked ─────────────────────────────────────────────────────

#[test]
fn test_query_blocked_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let parent = create_labeled_issue(&cli, "Parent", &[] as &[&str]);

    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    cli.run(["dep", "add", &both, &parent]).unwrap();

    let epic_only = create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);
    cli.run(["dep", "add", &epic_only, &parent]).unwrap();

    let json = cli
        .run_json([
            "query",
            "blocked",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit query strategic ──────────────────────────────────────────────────

#[test]
fn test_query_strategic_repeatable_label_ands() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let strategic = type_label(&taxonomy, 2);
    let component = membership_label(&taxonomy, 3, "api");
    let team = membership_label(&taxonomy, 1, "backend");
    let both = create_labeled_issue(
        &cli,
        "Both",
        &[strategic.clone(), component.clone(), team.clone()],
    );
    create_labeled_issue(&cli, "ComponentOnly", &[strategic, component.clone()]);

    let json = cli
        .run_json([
            "query",
            "strategic",
            "--label",
            &component,
            "--label",
            &team,
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit query closed ─────────────────────────────────────────────────────

#[test]
fn test_query_closed_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    cli.run(["issue", "update", &both, "--state", "done"])
        .unwrap();
    let epic_only = create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);
    cli.run(["issue", "update", &epic_only, "--state", "done"])
        .unwrap();

    let json = cli
        .run_json([
            "query",
            "closed",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit issue list ───────────────────────────────────────────────────────

#[test]
fn test_issue_list_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);

    let json = cli
        .run_json([
            "issue",
            "list",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
        ])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── jit list (top-level alias) ───────────────────────────────────────────

#[test]
fn test_top_level_list_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);

    let json = cli
        .run_json(["list", "--label", "epic:auth", "--label", "component:api"])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}

// ── bare `jit query` ─────────────────────────────────────────────────────

#[test]
fn test_query_bare_repeatable_label_ands() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let both = create_labeled_issue(&cli, "Both", &["epic:auth", "component:api"]);
    create_labeled_issue(&cli, "EpicOnly", &["epic:auth"]);

    let json = cli
        .run_json(["query", "--label", "epic:auth", "--label", "component:api"])
        .unwrap();
    assert_eq!(ids(&json), vec![both]);
}
