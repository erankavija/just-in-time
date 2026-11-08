//! Config-declared template role and anchor bindings (jit:136268f8).
//!
//! The bracket tooling reaches for three names by MEANING: the role of the node
//! that holds the plan, the role of the node that holds the fan-out, and the
//! anchor `jit apply <template> <container>` binds to its positional
//! `<container>`. All three are repository configuration — `.jit/templates.toml`'s
//! `[roles]` and `[anchors]` tables — so a repository that names them differently
//! still completes the whole bracket flow (`@/inv/domain-agnostic`).
//!
//! These drive `CommandExecutor` in-process against a `templates.toml` seeded
//! into the store (the bindings are read from the repository, so an in-test TOML
//! string would not exercise them), covering the library contract:
//!
//! - a repository that renames all three bindings: apply and `--force` refresh end
//!   to end;
//! - a repository that declares NO bindings: the shipped names, unchanged.
//!
//! The container anchor's auto-binding is the CLI's own step — it fills
//! the binding map these tests pass explicitly — and is covered against the real
//! binary in `template_binding_cli_tests.rs`.

use jit::commands::test_helpers::ExecutorTestAccess;
use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::commands::CommandExecutor;
use jit::domain::identity::IssueId;
use jit::domain::{Issue, Priority};
use jit::storage::{InMemoryStorage, IssueStore};
use std::collections::BTreeMap;

/// The container anchor `jit apply` auto-binds, read from captured declarations.
fn container_anchor(executor: &CommandExecutor<InMemoryStorage>) -> String {
    executor
        .config()
        .captured_declarations()
        .unwrap()
        .config()
        .templates
        .anchors
        .container_anchor()
        .to_string()
}

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

/// Create a breakable container, returning its id.
fn create_container(executor: &CommandExecutor<InMemoryStorage>, title: &str) -> IssueId {
    executor
        .issues()
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
        .issue
        .id
}

/// The issue's `type:` label value, if any.
fn type_of(issue: &Issue) -> Option<String> {
    jit::labels::type_label_value(&issue.labels).map(str::to_string)
}

// =========== REQ-2: renamed bindings complete the whole bracket flow ===========

#[test]
fn test_renamed_bindings_complete_the_bracket_flow_end_to_end() {
    let storage = InMemoryStorage::new();
    let unchecked_hierarchy: [&str; 0] = [];
    let gate_keys: Vec<_> = jit::templates::TemplateRegistry::from_toml_str(
        RENAMED_BINDINGS_TEMPLATE,
        &unchecked_hierarchy,
    )
    .expect("the template registry parses")
    .templates
    .iter()
    .flat_map(jit::test_harness::template_gate_keys)
    .collect();
    let gates_toml = String::from_utf8(
        jit::declarations::serialize_gate_registry(&jit::test_harness::manual_gate_registry(
            &gate_keys,
        ))
        .unwrap(),
    )
    .unwrap();
    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            templates: Some(RENAMED_BINDINGS_TEMPLATE),
            gates: Some(&gates_toml),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    // The auto-bound anchor is the repository's, not a compiled-in name. This is
    // the binding `jit apply <template> <container>` fills from its positional
    // argument.
    let anchor = container_anchor(&executor);
    assert_eq!(anchor, "target");

    // --- apply ---
    let container = create_container(&executor, "Auth epic");
    let bindings = BTreeMap::from([(anchor.clone(), container.clone())]);
    let (applied, _) = executor
        .templates()
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

    // --- forced refresh ---
    let mut c = executor.storage().load_issue(&container).unwrap();
    c.title = "Auth epic (revised)".to_string();
    jit::test_harness::seed_memory_issue(executor.storage(), &c);

    let (refreshed, _) = executor
        .templates()
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
    let storage = InMemoryStorage::new();
    let unchecked_hierarchy: [&str; 0] = [];
    let gate_keys: Vec<_> = jit::templates::TemplateRegistry::from_toml_str(
        DEFAULT_BINDINGS_TEMPLATE,
        &unchecked_hierarchy,
    )
    .expect("the template registry parses")
    .templates
    .iter()
    .flat_map(jit::test_harness::template_gate_keys)
    .collect();
    let gates_toml = String::from_utf8(
        jit::declarations::serialize_gate_registry(&jit::test_harness::manual_gate_registry(
            &gate_keys,
        ))
        .unwrap(),
    )
    .unwrap();
    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            templates: Some(DEFAULT_BINDINGS_TEMPLATE),
            gates: Some(&gates_toml),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    assert_eq!(container_anchor(&executor), "container");

    let container = create_container(&executor, "Auth epic");
    let bindings = BTreeMap::from([("container".to_string(), container.clone())]);
    let (applied, _) = executor
        .templates()
        .apply_template("plan", &container, &bindings, false)
        .unwrap();
    let planning_id = applied.created_node_ids_by_role["planning"].clone();
    let breakdown_id = applied.created_node_ids_by_role["breakdown"].clone();

    let (refreshed, _) = executor
        .templates()
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
    let executor = memory_executor_declaring(InMemoryStorage::new(), MemoryDeclarations::default());
    assert_eq!(container_anchor(&executor), "container");
}
