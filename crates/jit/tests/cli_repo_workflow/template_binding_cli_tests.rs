//! CLI integration tests for config-declared template role and anchor bindings
//! (jit:136268f8, REQ-2).
//!
//! These spawn the real `jit` binary against a `TempDir`-backed repository whose
//! `.jit/templates.toml` renames all three bracket bindings: the planning role is
//! `spec`, the breakdown role is `split`, and the container anchor is `target`.
//! No engine-known name appears as a role or anchor anywhere in the fixture.
//!
//! Driving the subprocess is what exercises the auto-bind: named apply captures
//! the repository's `[anchors]` table and binds its container anchor to the
//! positional container. `apply_template` aborts before its first write unless
//! EVERY declared anchor is bound, so reaching for the shipped anchor name would
//! leave `target` unbound and fail.
//! `test_apply_aborts_when_the_bound_anchor_names_no_declared_anchor` pins that
//! failure directly; the other tests pass no `--anchor` flag and assert the
//! reported `anchor_bindings` carry exactly `target`.
//!
//! Coverage of `bracket_breakdown` lives in `template_binding_tests.rs`: that
//! operation is a library API with no CLI surface, so its renamed-role path is
//! reachable only in-process.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

/// Declares the bracket node types. Role and anchor names live in
/// `templates.toml`, not here.
const CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

/// The `plan` bracket under renamed bindings: `[roles]` names the planning role
/// `spec` and the breakdown role `split`, `[anchors]` names the container anchor
/// `target`. The `move-upstream-to-role` transform names its own target role.
const RENAMED_BINDINGS_TEMPLATE: &str = r#"
[roles]
planning  = "spec"
breakdown = "split"

[anchors]
container = "target"

[[template]]
name        = "plan"
description = "Plan-before-fan-out bracket."
applies_to  = ["epic"]

  [[template.anchors]]
  name = "target"

  [[template.nodes]]
  role        = "spec"
  type        = "planning"
  gates       = ["plan-review"]
  doc         = "dev/active/{container.id}-plan.md"
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "split"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on  = ["spec"]

  [[template.anchor_edges]]
  from = "target"
  to   = "split"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "spec"
"#;

/// The same template with the `[anchors]` table dropped, so the repository's
/// container anchor resolves to the shipped `container` while the template
/// declares only `target`. This is the state a `jit apply` with a hardcoded
/// anchor name would put the renamed repository in.
const UNBOUND_ANCHOR_TEMPLATE: &str = r#"
[roles]
planning  = "spec"
breakdown = "split"

[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.anchors]]
  name = "target"

  [[template.nodes]]
  role        = "spec"
  type        = "planning"
  gates       = ["plan-review"]
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "split"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on  = ["spec"]

  [[template.anchor_edges]]
  from = "target"
  to   = "split"
"#;

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// Run a jit command, asserting success, and parse its JSON stdout.
fn jit_json(temp: &TempDir, args: &[&str]) -> Value {
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

/// Initialize an isolated temp repo carrying `templates_toml`.
///
/// The configuration is written before initialization, which preserves it and
/// derives the coupled rules and schemas from the registry it declares.
fn setup_repo(templates_toml: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();
    std::fs::write(jit_dir.join("config.toml"), CONFIG_TOML).unwrap();
    jit(&temp).arg("init").assert().success();
    std::fs::write(jit_dir.join("templates.toml"), templates_toml).unwrap();
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

/// The `data` payload of a command's JSON envelope.
fn data(out: &Value) -> &Value {
    out.get("data").unwrap_or(out)
}

/// The dependency ids of an issue (`issue show --json` expands each into an
/// object; read its `id`).
fn dep_ids(temp: &TempDir, id: &str) -> Vec<String> {
    let issue = jit_json(temp, &["issue", "show", id, "--json"]);
    data(&issue)
        .get("dependencies")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// How many issues carry `type:planning`.
fn planning_node_count(temp: &TempDir) -> u64 {
    let listed = jit_json(
        temp,
        &["query", "all", "--label", "type:planning", "--json"],
    );
    listed["count"]
        .as_u64()
        .unwrap_or_else(|| panic!("query all must report a count, got: {listed}"))
}

/// Assert the reported anchor bindings are exactly `{target: <epic>}`: the CLI
/// read the anchor name from `[anchors]`, so the shipped name is absent.
fn assert_bound_to_configured_anchor(out: &Value, epic: &str) {
    let bindings = data(out)["anchor_bindings"]
        .as_object()
        .unwrap_or_else(|| panic!("apply must report anchor_bindings, got: {out}"));
    assert_eq!(
        bindings.keys().collect::<Vec<_>>(),
        ["target"],
        "the CLI must auto-bind the anchor named by [anchors], got: {out}"
    );
    assert_eq!(
        bindings["target"].as_str(),
        Some(epic),
        "the configured anchor must bind to the positional <container>, got: {out}"
    );
}

// ===================== REQ-2: apply, at the CLI boundary =====================

/// `jit apply plan <epic>` with NO `--anchor` flag binds the anchor the
/// repository's `[anchors]` table names (`target`) and creates the bracket under
/// the renamed roles.
#[test]
fn test_apply_auto_binds_the_configured_container_anchor() {
    let temp = setup_repo(RENAMED_BINDINGS_TEMPLATE);
    let epic = create_epic(&temp, "Auth epic");

    let out = jit_json(&temp, &["apply", "plan", &epic, "--json"]);
    assert_bound_to_configured_anchor(&out, &epic);

    // The created nodes answer to the renamed roles, and to no others.
    let roles = data(&out)["created_node_ids_by_role"]
        .as_object()
        .expect("created_node_ids_by_role");
    assert_eq!(roles.keys().collect::<Vec<_>>(), ["spec", "split"]);
    let spec_id = roles["spec"].as_str().expect("spec id");
    let split_id = roles["split"].as_str().expect("split id");

    // The bracket is wired C → B → P through the renamed anchor edge.
    assert!(
        dep_ids(&temp, &epic).iter().any(|d| d == split_id),
        "the container must depend on the breakdown node (C → B)"
    );
    assert!(
        dep_ids(&temp, split_id).iter().any(|d| d == spec_id),
        "the breakdown node must depend on the planning node (B → P)"
    );
}

/// The binding the CLI auto-fills is load-bearing: when it names an anchor the
/// template does not declare, `jit apply` rejects the template before its first
/// write and nothing is created. A `jit apply` that reached for the shipped
/// anchor name instead of the configured one would fail exactly this way against
/// `RENAMED_BINDINGS_TEMPLATE`.
#[test]
fn test_apply_aborts_when_the_bound_anchor_names_no_declared_anchor() {
    let temp = setup_repo(UNBOUND_ANCHOR_TEMPLATE);
    let epic = create_epic(&temp, "Auth epic");

    jit(&temp)
        .args(["apply", "plan", &epic])
        .assert()
        .failure()
        .stderr(predicates::str::contains("target"));

    assert_eq!(
        planning_node_count(&temp),
        0,
        "a rejected apply must create no bracket node"
    );
}

/// `jit apply plan <epic> --force`, again with NO `--anchor`, re-seeds the
/// existing bracket in place: it locates `B` by the `roles.breakdown` type and
/// reaches `P` through it, so both renamed roles map back to the SAME ids and no
/// duplicate node is created. The refreshed planning description reflects the
/// container's new title.
#[test]
fn test_apply_force_refreshes_the_bracket_through_the_configured_bindings() {
    let temp = setup_repo(RENAMED_BINDINGS_TEMPLATE);
    let epic = create_epic(&temp, "Auth epic");

    let applied = jit_json(&temp, &["apply", "plan", &epic, "--json"]);
    let roles = &data(&applied)["created_node_ids_by_role"];
    let spec_id = roles["spec"].as_str().expect("spec id").to_string();
    let split_id = roles["split"].as_str().expect("split id").to_string();

    jit(&temp)
        .args(["issue", "update", &epic, "--title", "Auth epic (revised)"])
        .assert()
        .success();

    let refreshed = jit_json(&temp, &["apply", "plan", &epic, "--force", "--json"]);
    assert_bound_to_configured_anchor(&refreshed, &epic);

    let roles = &data(&refreshed)["created_node_ids_by_role"];
    assert_eq!(
        roles["spec"].as_str(),
        Some(spec_id.as_str()),
        "--force must re-seed the existing planning node, got: {refreshed}"
    );
    assert_eq!(
        roles["split"].as_str(),
        Some(split_id.as_str()),
        "--force must re-seed the existing breakdown node, got: {refreshed}"
    );

    let spec = jit_json(&temp, &["issue", "show", &spec_id, "--json"]);
    assert_eq!(
        data(&spec)["description"].as_str(),
        Some("Planning node for Auth epic (revised)."),
        "the refreshed planning node carries the re-interpolated description"
    );

    assert_eq!(
        planning_node_count(&temp),
        1,
        "--force must not duplicate the planning node"
    );
}

#[test]
fn test_explicit_bindings_override_captured_default_and_bind_secondary_anchor() {
    let templates = RENAMED_BINDINGS_TEMPLATE.replacen(
        "  [[template.nodes]]",
        "  [[template.anchors]]\n  name = \"reviewer\"\n\n  [[template.nodes]]",
        1,
    );
    let temp = setup_repo(&templates);
    let positional = create_epic(&temp, "Positional epic");
    let target = create_epic(&temp, "Target epic");
    let reviewer = create_epic(&temp, "Reviewer epic");
    let target_binding = format!("target={target}");
    let reviewer_binding = format!("reviewer={reviewer}");

    let out = jit_json(
        &temp,
        &[
            "apply",
            "plan",
            &positional,
            "--anchor",
            &target_binding,
            "--anchor",
            &reviewer_binding,
            "--json",
        ],
    );
    let bindings = data(&out)["anchor_bindings"].as_object().unwrap();
    assert_eq!(bindings["target"].as_str(), Some(target.as_str()));
    assert_eq!(bindings["reviewer"].as_str(), Some(reviewer.as_str()));

    let split = data(&out)["created_node_ids_by_role"]["split"]
        .as_str()
        .unwrap();
    assert!(dep_ids(&temp, &target).iter().any(|id| id == split));
    assert!(!dep_ids(&temp, &positional).iter().any(|id| id == split));
}
