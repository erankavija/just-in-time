//! Config-declared template role and anchor bindings (jit:136268f8).
//!
//! The bracket tooling reaches for three names by MEANING: the role of the node
//! that holds the plan, the role of the node that holds the fan-out, and the
//! anchor `jit apply <template> <container>` binds to its positional
//! `<container>`. All three are repository configuration — `.jit/templates.toml`'s
//! `[roles]` and `[anchors]` tables — so a repository that names them differently
//! still completes the whole bracket flow (`@/inv/domain-agnostic`).
//!
//! These drive `CommandExecutor` in-process against a real on-disk
//! `templates.toml` (the bindings are read from the repository, so an in-test TOML
//! string would not exercise them), covering the library contract:
//!
//! - a repository that renames all three bindings: apply, bracket breakdown, and
//!   `--force` refresh end to end;
//! - a repository that declares NO bindings: the shipped names, unchanged.
//!
//! `bracket_breakdown` is reachable only here: it is a library API with no CLI
//! surface. The container anchor's auto-binding is the CLI's own step — it fills
//! the binding map these tests pass explicitly — and is covered against the real
//! binary in `template_binding_cli_tests.rs`.

use jit::commands::{BracketChild, CommandExecutor};
use jit::domain::{GateStatus, Issue, Priority};
use jit::storage::{InMemoryStorage, IssueStore};
use std::collections::BTreeMap;

/// A `plan` template whose planning role is `spec`, breakdown role is `split`,
/// and container anchor is `target`, with the `[roles]`/`[anchors]` tables that
/// bind them. No engine-known name appears as a role or anchor.
const RENAMED_BINDINGS_TEMPLATE: &str = r#"
[roles]
planning  = "spec"
breakdown = "split"

[anchors]
container = "target"

[[template]]
name        = "plan"
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

/// The same bracket with NO `[roles]`/`[anchors]` tables: the shipped names.
const DEFAULT_BINDINGS_TEMPLATE: &str = r#"
[[template]]
name        = "plan"
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

/// An executor over an isolated repository whose `.jit/templates.toml` holds
/// `templates_toml`. No `config.toml`: the bindings and the bracket vocabulary
/// come entirely from the template registry.
fn executor_with_templates(templates_toml: &str) -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    crate::seed_memory_data_file(&storage, "templates.toml", templates_toml);
    let layout = storage.repository_layout();
    CommandExecutor::new(storage).with_layout(layout)
}

/// Create a breakable container, returning its id.
fn create_container(executor: &CommandExecutor<InMemoryStorage>, title: &str) -> String {
    executor
        .create_issue(
            title.to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            None,
            false,
        )
        .unwrap()
        .0
}

/// Mark the planning node's plan-quality gate PASSED, as breakdown requires.
fn approve_plan(executor: &CommandExecutor<InMemoryStorage>, planning_id: &str) {
    let mut p = executor.storage().load_issue(planning_id).unwrap();
    p.gates_status
        .get_mut("plan-review")
        .expect("the planning node carries the plan gate")
        .status = GateStatus::Passed;
    executor.storage().save_issue(p).unwrap();
}

/// The issue's `type:` label value, if any.
fn type_of(issue: &Issue) -> Option<String> {
    issue
        .labels
        .iter()
        .find_map(|l| l.strip_prefix("type:").map(str::to_string))
}

fn child(title: &str) -> BracketChild {
    BracketChild {
        title: title.to_string(),
        description: String::new(),
        priority: Priority::Normal,
        gates: vec![],
        labels: vec![],
        deps: vec![],
    }
}

// =========== REQ-2: renamed bindings complete the whole bracket flow ===========

#[test]
fn test_renamed_bindings_complete_the_bracket_flow_end_to_end() {
    let executor = executor_with_templates(RENAMED_BINDINGS_TEMPLATE);

    // The auto-bound anchor is the repository's, not a compiled-in name. This is
    // the binding `jit apply <template> <container>` fills from its positional
    // argument.
    let anchor = executor.container_anchor().unwrap().to_string();
    assert_eq!(anchor, "target");

    // --- apply ---
    let container = create_container(&executor, "Auth epic");
    let bindings = BTreeMap::from([(anchor.clone(), container.clone())]);
    let (applied, _) = executor
        .apply_template("plan", &container, &bindings, false)
        .expect("apply resolves the renamed anchor and creates the renamed roles");

    assert_eq!(
        applied.anchor_bindings.keys().collect::<Vec<_>>(),
        ["target"]
    );
    let spec_id = applied.created_node_ids_by_role["spec"].clone();
    let split_id = applied.created_node_ids_by_role["split"].clone();
    assert_eq!(applied.created_node_ids_by_role.len(), 2);

    // The bracket is wired C → B → P, with the node types the template declares.
    let split = executor.storage().load_issue(&split_id).unwrap();
    assert_eq!(type_of(&split).as_deref(), Some("breakdown"));
    assert!(split.dependencies.contains(&spec_id), "B → P");
    let c = executor.storage().load_issue(&container).unwrap();
    assert!(c.dependencies.contains(&split_id), "C → B");

    // --- breakdown ---
    approve_plan(&executor, &spec_id);
    let broken = executor
        .bracket_breakdown(&container, vec![child("Build login"), child("Wire logout")])
        .expect("breakdown locates B and P through the renamed roles");
    assert_eq!(broken.child_ids.len(), 2);
    assert_eq!(
        broken.breakdown_id, split_id,
        "B is consumed, not re-created"
    );
    assert_eq!(broken.planning_id, spec_id);

    // Every source child hangs off B, so the fan-out cannot release before the
    // breakdown node's gates pass.
    for child_id in &broken.child_ids {
        let issue = executor.storage().load_issue(child_id).unwrap();
        assert!(
            issue.dependencies.contains(&split_id),
            "source child {child_id} depends on B"
        );
    }

    // --- forced refresh ---
    let mut c = executor.storage().load_issue(&container).unwrap();
    c.title = "Auth epic (revised)".to_string();
    executor.storage().save_issue(c).unwrap();

    let (refreshed, _) = executor
        .apply_template("plan", &container, &bindings, true)
        .expect("--force refresh locates the applied bracket through the renamed roles");
    assert_eq!(
        refreshed.created_node_ids_by_role["spec"], spec_id,
        "refresh re-seeds the existing planning node rather than creating a second"
    );
    assert_eq!(refreshed.created_node_ids_by_role["split"], split_id);

    let spec = executor.storage().load_issue(&spec_id).unwrap();
    assert_eq!(spec.description, "Planning node for Auth epic (revised).");

    // No duplicate bracket nodes were created anywhere in the store.
    let planning_nodes = executor
        .storage()
        .list_issues()
        .unwrap()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("planning"))
        .count();
    assert_eq!(planning_nodes, 1);
}

// ============ REQ-1: absent bindings reproduce the shipped names ============

#[test]
fn test_absent_bindings_reproduce_the_shipped_names_end_to_end() {
    let executor = executor_with_templates(DEFAULT_BINDINGS_TEMPLATE);
    assert_eq!(executor.container_anchor().unwrap(), "container");

    let container = create_container(&executor, "Auth epic");
    let bindings = BTreeMap::from([("container".to_string(), container.clone())]);
    let (applied, _) = executor
        .apply_template("plan", &container, &bindings, false)
        .unwrap();
    let planning_id = applied.created_node_ids_by_role["planning"].clone();
    let breakdown_id = applied.created_node_ids_by_role["breakdown"].clone();

    approve_plan(&executor, &planning_id);
    let broken = executor
        .bracket_breakdown(&container, vec![child("Build login")])
        .unwrap();
    assert_eq!(broken.breakdown_id, breakdown_id);
    assert_eq!(broken.planning_id, planning_id);

    let (refreshed, _) = executor
        .apply_template("plan", &container, &bindings, true)
        .unwrap();
    assert_eq!(refreshed.created_node_ids_by_role["planning"], planning_id);
    assert_eq!(
        refreshed.created_node_ids_by_role["breakdown"],
        breakdown_id
    );
}

/// A repository with no `templates.toml` at all still reports the shipped anchor,
/// so the CLI's auto-bind is unchanged where nothing is configured.
#[test]
fn test_container_anchor_defaults_without_a_templates_file() {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    let layout = storage.repository_layout();
    let executor = CommandExecutor::new(storage).with_layout(layout);
    assert_eq!(executor.container_anchor().unwrap(), "container");
}
