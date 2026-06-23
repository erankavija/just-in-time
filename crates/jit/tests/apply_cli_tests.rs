//! CLI integration tests for the additive `jit apply <template> <container>`
//! command (issue 9cccfe0c, SCFA-01..04).
//!
//! These exercise the real subprocess path end-to-end: the built `jit` binary
//! against an isolated `TempDir`-backed `.jit/` repo whose `config.toml` declares
//! the planning hierarchy and whose `templates.toml` declares the `plan`
//! template. The referenced gate presets (`plan-review`, `coverage-preview`,
//! `breakdown-review`) are builtins, so no preset registration is needed.
//!
//! Every fixture lives in its own `TempDir` — the production `.jit/` is NEVER
//! touched. The tests assert that `jit apply plan` creates the `C → B → P`
//! bracket and emits a machine-readable result.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

/// Hierarchy declaring the bracket node types so `epic`/`planning`/`breakdown`
/// are valid types in the temp repo. The bracket vocabulary itself comes from the
/// `plan` template (`PLAN_TEMPLATE_TOML`), not from config.
const CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

/// The repo's `plan`-shaped graph template: a planning node `P` and a breakdown
/// node `B` (with `brackets:<short-id>`), each with builtin gate presets, plus
/// the `B → P` edge, the `C → B` anchor edge, and the `move-upstream-to-role`
/// transform onto `planning`.
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

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// Run a jit command, asserting success, and parse its JSON stdout.
fn jit_json(temp: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = jit(temp)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap_or_else(|e| {
        panic!(
            "expected JSON from `jit {}`: {e}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out)
        )
    })
}

/// Initialize an isolated temp repo with the planning hierarchy + the `plan`
/// graph template on disk.
fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();
    let jit_dir = temp.path().join(".jit");
    std::fs::write(jit_dir.join("config.toml"), CONFIG_TOML).unwrap();
    std::fs::write(jit_dir.join("templates.toml"), PLAN_TEMPLATE_TOML).unwrap();
    temp
}

/// Create an `epic` container, returning its full id.
fn create_epic(temp: &TempDir, title: &str) -> String {
    let json = jit_json(
        temp,
        &[
            "issue",
            "create",
            "--title",
            title,
            "--description",
            "## Success Criteria\n\n- [hard] REQ-01: it works\n",
            "--label",
            "type:epic",
            "--json",
        ],
    );
    json["id"].as_str().expect("created epic id").to_string()
}

/// SCFA-01/03: `jit apply plan <epic>` is wired (parse, dispatch, print) and
/// invokes the engine, creating the bracket's planning + breakdown nodes.
#[test]
fn test_apply_plan_creates_bracket_nodes() {
    let temp = setup_repo();
    let epic = create_epic(&temp, "Auth epic");

    let out = jit_json(&temp, &["apply", "plan", &epic, "--json"]);
    let data = out.get("data").unwrap_or(&out);
    let roles = &data["created_node_ids_by_role"];
    let planning_id = roles["planning"].as_str().expect("planning role id");
    let breakdown_id = roles["breakdown"].as_str().expect("breakdown role id");

    // Each created node carries the type the template declared (confirming the
    // engine actually created them, not just reported ids).
    let labels_of = |id: &str| -> Vec<String> {
        let issue = jit_json(&temp, &["issue", "show", id, "--json"]);
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
    let container = jit_json(&temp, &["issue", "show", &epic, "--json"]);
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
    let temp = setup_repo();
    let epic = create_epic(&temp, "Auth epic");

    let out = jit_json(&temp, &["apply", "plan", &epic, "--json"]);
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

/// A malformed `--anchor` (missing `=`) is rejected with a clear error and
/// creates nothing.
#[test]
fn test_apply_rejects_malformed_anchor() {
    let temp = setup_repo();
    let epic = create_epic(&temp, "Auth epic");

    jit(&temp)
        .args(["apply", "plan", &epic, "--anchor", "noequals"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("role=id"));
}
