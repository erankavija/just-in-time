//! CLI integration tests for the additive `jit apply <template> <container>`
//! command (issue 9cccfe0c, SCFA-01..04).
//!
//! These exercise the real subprocess path end-to-end: the built `jit` binary
//! against an isolated `.jit/` repo whose `config.toml` declares
//! the planning hierarchy, whose `templates.toml` declares the `plan` template,
//! and whose gate registry declares the keys that template names.
//!
//! Every fixture lives in its own `TempDir` — the production `.jit/` is NEVER
//! touched. The tests assert that `jit apply plan` creates the `C → B → P`
//! bracket and emits a machine-readable result.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

/// Hierarchy declaring the bracket node types so `epic`/`planning`/`breakdown`
/// are valid types in the temp repo. The bracket vocabulary itself comes from the
/// `plan` template (`PLAN_TEMPLATE_TOML`), not from config.
pub(crate) const CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

/// The repo's `plan`-shaped graph template: a planning node `P` and a breakdown
/// node `B` (with `brackets:<short-id>`), each with the gate keys the fixture
/// declares, plus the `B → P` edge, the `C → B` anchor edge, and the
/// `move-upstream-to-role` transform onto `planning`.
const PLAN_TEMPLATE_TOML: &str = r#"
[[template]]
name        = "plan"
description = "Plan-before-fan-out bracket."
applies_to  = ["epic"]

  [[template.anchors]]
  name = "container"

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]
  doc         = "dev/active/{container.id}-plan.md"
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on  = ["planning"]

  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#;

/// SCFA-01/03: `jit apply plan <epic>` is wired (parse, dispatch, print) and
/// invokes the engine, creating the bracket's planning + breakdown nodes.
#[test]
fn test_apply_plan_creates_bracket_nodes() {
    let cli = JitCli::with_repository(
        jit::test_utils::setup_test_repo_with_config(CONFIG_TOML)
            .unwrap()
            .0,
    );
    super::install_templates(&cli, PLAN_TEMPLATE_TOML);
    let epic = super::create_epic(&cli, "Auth epic");

    let out = cli.run_json(["apply", "plan", &epic]).unwrap();
    let data = out.get("data").unwrap_or(&out);
    let roles = &data["created_node_ids_by_role"];
    let planning_id = roles["planning"].as_str().expect("planning role id");
    let breakdown_id = roles["breakdown"].as_str().expect("breakdown role id");

    // Each created node carries the type the template declared (confirming the
    // engine actually created them, not just reported ids).
    let labels_of = |id: &str| -> Vec<String> {
        let issue = cli.run_json(["issue", "show", id]).unwrap();
        let issue = issue.get("data").unwrap_or(&issue);
        issue
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(
        labels_of(planning_id).iter().any(|l| l == "type:planning"),
        "the planning node must carry type:planning"
    );
    assert!(
        labels_of(breakdown_id)
            .iter()
            .any(|l| l == "type:breakdown"),
        "the breakdown node must carry type:breakdown"
    );

    // The container now depends on the breakdown node (the C → B anchor edge).
    let container = cli.run_json(["issue", "show", &epic]).unwrap();
    let container = container.get("data").unwrap_or(&container);
    // `issue show --json` expands each dependency into an object; read its `id`.
    let deps: Vec<String> = container
        .get("dependencies")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        deps.iter().any(|d| d == breakdown_id),
        "container must depend on the breakdown node after apply, got: {deps:?}"
    );
}

/// SCFA-04: `--json` emits a machine-readable apply result carrying the template
/// name, resolved anchor bindings, and the created node ids by role.
#[test]
fn test_apply_plan_json_reports_template_and_created_nodes() {
    let cli = JitCli::with_repository(
        jit::test_utils::setup_test_repo_with_config(CONFIG_TOML)
            .unwrap()
            .0,
    );
    super::install_templates(&cli, PLAN_TEMPLATE_TOML);
    let epic = super::create_epic(&cli, "Auth epic");

    let out = cli.run_json(["apply", "plan", &epic]).unwrap();
    let data = out.get("data").unwrap_or(&out);

    assert_eq!(
        data["template"], "plan",
        "JSON result must name the applied template, got: {out}"
    );

    // The container anchor was auto-bound to the positional <epic>.
    assert_eq!(
        data["anchor_bindings"]["container"]
            .as_str()
            .map(str::to_string),
        Some(epic.clone()),
        "container anchor must auto-bind to <container>, got: {out}"
    );

    // created_node_ids_by_role names both bracket roles with non-empty ids.
    let roles = &data["created_node_ids_by_role"];
    let planning = roles["planning"].as_str().expect("planning role id");
    let breakdown = roles["breakdown"].as_str().expect("breakdown role id");
    assert!(!planning.is_empty(), "planning node id must be non-empty");
    assert!(!breakdown.is_empty(), "breakdown node id must be non-empty");
    assert_ne!(planning, breakdown, "roles must map to distinct nodes");
}

/// SCFA-04: the SUCCESS path of an EXPLICIT `--anchor container=<epic>` — not just
/// the positional auto-bind. Passing the container anchor explicitly must produce
/// the same `C → B → P` bracket: both roles created, the `B → P` internal edge,
/// and the `C → B` anchor edge.
#[test]
fn test_apply_with_explicit_container_anchor_creates_bracket() {
    let cli = JitCli::with_repository(
        jit::test_utils::setup_test_repo_with_config(CONFIG_TOML)
            .unwrap()
            .0,
    );
    super::install_templates(&cli, PLAN_TEMPLATE_TOML);
    let epic = super::create_epic(&cli, "Auth epic");

    let out = cli
        .run_json([
            "apply",
            "plan",
            &epic,
            "--anchor",
            &format!("container={epic}"),
        ])
        .unwrap();
    let data = out.get("data").unwrap_or(&out);

    // The explicit binding is reflected back in the result.
    assert_eq!(
        data["anchor_bindings"]["container"]
            .as_str()
            .map(str::to_string),
        Some(epic.clone()),
        "explicit --anchor container=<epic> must bind the container anchor, got: {out}"
    );

    // Both bracket roles were created with non-empty, distinct ids.
    let roles = &data["created_node_ids_by_role"];
    let planning_id = roles["planning"].as_str().expect("planning role id");
    let breakdown_id = roles["breakdown"].as_str().expect("breakdown role id");
    assert_ne!(
        planning_id, breakdown_id,
        "roles must map to distinct nodes"
    );

    // Read an issue's dependency ids (`issue show --json` expands deps into
    // objects; pull each `id`).
    let dep_ids = |id: &str| -> Vec<String> {
        let issue = cli.run_json(["issue", "show", id]).unwrap();
        let issue = issue.get("data").unwrap_or(&issue);
        issue
            .get("dependencies")
            .and_then(|d| d.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };

    // C → B: the container depends on the breakdown node (the anchor edge).
    assert!(
        dep_ids(&epic).iter().any(|d| d == breakdown_id),
        "container must depend on the breakdown node (C → B) under explicit anchor binding"
    );
    // B → P: the breakdown node depends on the planning node (the internal edge).
    assert!(
        dep_ids(breakdown_id).iter().any(|d| d == planning_id),
        "breakdown node must depend on the planning node (B → P) under explicit anchor binding"
    );
}

/// A malformed `--anchor` (missing `=`) is rejected with a clear error and
/// creates nothing.
#[test]
fn test_apply_rejects_malformed_anchor() {
    let cli = JitCli::with_repository(
        jit::test_utils::setup_test_repo_with_config(CONFIG_TOML)
            .unwrap()
            .0,
    );
    super::install_templates(&cli, PLAN_TEMPLATE_TOML);
    let epic = super::create_epic(&cli, "Auth epic");

    cli.command(["apply", "plan", &epic, "--anchor", "noequals"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("role=id"));
}

/// REQ-05 (jit:0723860e): `--json` carries every unmet prerequisite as a
/// structured `{kind, name}` entry, beside the template it belongs to.
#[test]
fn test_apply_json_reports_unmet_prerequisites_structurally() {
    let config = format!(
        "[type_hierarchy]\ntypes = {{ epic = 1 }}\n\
         [namespaces.{}]\ndescription = \"Issue type\"\nunique = true\n",
        jit::labels::TYPE_NAMESPACE
    );
    let cli = JitCli::with_repository(
        jit::test_utils::setup_test_repo_with_config(&config)
            .unwrap()
            .0,
    );
    let templates = r#"
[[template]]
name       = "unprepared"
applies_to = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["container-review"]
  [[template.nodes]]
  role     = "planning"
  type     = "planning"
  gates    = ["plan-review"]
  doc      = "{container.dir}/plan.md"
  doc_area = "plans"
  labels   = ["brackets:{container.short_id}"]
"#;
    std::fs::write(cli.root().join(".jit/templates.toml"), templates).unwrap();
    let epic = super::create_epic(&cli, "Unprepared epic");

    let output = cli
        .command(["apply", "unprepared", &epic, "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let error = &json["error"];
    assert_eq!(
        error["code"],
        serde_json::to_value(jit::output::ErrorCode::ValidationFailed).unwrap(),
        "{json}"
    );
    assert_eq!(error["details"]["template"], "unprepared", "{json}");
    // Everything the template requires except the type namespace the
    // repository declares, each as a structured entry.
    let met = jit::templates::TemplateRequirement::LabelNamespace(
        jit::labels::TYPE_NAMESPACE.to_string(),
    );
    let expected = jit::templates::TemplateRegistry::from_toml_str(templates, &[] as &[&str])
        .unwrap()
        .get("unprepared")
        .unwrap()
        .requirements()
        .into_iter()
        .filter(|requirement| requirement != &met)
        .collect::<Vec<_>>();
    assert_eq!(
        error["details"]["unmet"],
        serde_json::to_value(&expected).unwrap(),
        "{json}"
    );
    let kinds = error["details"]["unmet"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(kinds.len(), 4, "{json}");
}
