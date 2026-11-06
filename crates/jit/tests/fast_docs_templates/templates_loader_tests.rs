//! Integration tests for the `.jit/templates.toml` loader wired into config
//! load (issue 45e7f203, REGA-02).
//!
//! These exercise `load_configuration` end-to-end against a tempdir-backed `.jit/`
//! so the full config-load path runs the template registry's load-time
//! validation. All fixtures live in isolated `TempDir`s — never the production
//! `.jit/`.

use jit::storage::configuration_store::load_configuration;
use jit::test_utils::declaration_dir;

const CONFIG_WITH_HIERARCHY: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

const VALID_PLAN_TEMPLATE: &str = r#"
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

#[test]
fn test_config_load_succeeds_with_valid_templates() {
    let temp = declaration_dir(&[
        ("config.toml", CONFIG_WITH_HIERARCHY),
        ("templates.toml", VALID_PLAN_TEMPLATE),
    ])
    .unwrap();
    // Config load runs template validation; a valid file must not error.
    let config = load_configuration(&jit::test_utils::repository_layout_for_data_root(
        temp.path(),
    ))
    .expect("valid templates.toml must load");
    assert!(config.templates.get("plan").is_some());
}

#[test]
fn test_config_load_succeeds_without_templates_file() {
    let temp = declaration_dir(&[("config.toml", CONFIG_WITH_HIERARCHY)]).unwrap();
    load_configuration(&jit::test_utils::repository_layout_for_data_root(
        temp.path(),
    ))
    .expect("absent templates.toml is fine");
}

#[test]
fn test_config_load_fails_on_invalid_template_unknown_container_type() {
    // `applies_to = ["bogus"]` is absent from [type_hierarchy].types, so config
    // load must fail with a clear, attributable error.
    let bad = r#"
[[template]]
name = "plan"
applies_to = ["bogus"]
"#;
    let temp = declaration_dir(&[
        ("config.toml", CONFIG_WITH_HIERARCHY),
        ("templates.toml", bad),
    ])
    .unwrap();
    let err = load_configuration(&jit::test_utils::repository_layout_for_data_root(
        temp.path(),
    ))
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("templates.toml"),
        "error should name the file: {msg}"
    );
    assert!(
        msg.contains("bogus"),
        "error should name the offending type: {msg}"
    );
}

#[test]
fn test_config_load_fails_on_cyclic_template() {
    let bad = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "a"
type = "planning"
depends_on = ["b"]
[[template.nodes]]
role = "b"
type = "breakdown"
depends_on = ["a"]
"#;
    let temp = declaration_dir(&[
        ("config.toml", CONFIG_WITH_HIERARCHY),
        ("templates.toml", bad),
    ])
    .unwrap();
    let err = load_configuration(&jit::test_utils::repository_layout_for_data_root(
        temp.path(),
    ))
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("cycle"),
        "error should mention the cycle: {msg}"
    );
}

#[test]
fn test_config_load_fails_on_unknown_transform_kind() {
    // An unsupported transform kind is a static config error: config load fails,
    // naming the template and the kind, so `jit apply` never receives it
    // (jit:2f447380 — REQ-3).
    let bad = r#"
[[template]]
name = "weird"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.transforms]]
kind = "teleport"
role = "planning"
"#;
    let temp = declaration_dir(&[
        ("config.toml", CONFIG_WITH_HIERARCHY),
        ("templates.toml", bad),
    ])
    .unwrap();
    let err = load_configuration(&jit::test_utils::repository_layout_for_data_root(
        temp.path(),
    ))
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("weird"),
        "error should name the template: {msg}"
    );
    assert!(
        msg.contains("teleport"),
        "error should name the offending kind: {msg}"
    );
}

#[test]
fn test_config_load_fails_on_malformed_templates_toml() {
    let temp = declaration_dir(&[
        ("config.toml", CONFIG_WITH_HIERARCHY),
        ("templates.toml", "[[template"),
    ])
    .unwrap();
    assert!(
        load_configuration(&jit::test_utils::repository_layout_for_data_root(
            temp.path()
        ))
        .is_err()
    );
}

/// `jit validate` judges a template's node types against the declared
/// hierarchy, as `jit apply` does; the template's other requirements are
/// prerequisites of applying it and stay with `jit apply`.
#[test]
fn test_validate_repository_report_fails_a_template_node_type_outside_the_hierarchy() {
    let (temp, executor) = jit::test_utils::setup_test_repo_with_config(CONFIG_WITH_HIERARCHY)
        .expect("fixture repository initializes");
    let template = r#"
[[template]]
name = "plan"
applies_to = ["epic"]
[[template.nodes]]
role     = "planning"
type     = "bogus"
gates    = ["undeclared-gate"]
doc_area = "undeclared/area"
"#;
    std::fs::write(temp.path().join(".jit/templates.toml"), template).unwrap();

    let report = executor.validation().validate_repository_report().unwrap();

    let failure = report
        .structural_failure
        .as_ref()
        .expect("an undeclared node type fails validation");
    let unmet = failure
        .downcast_ref::<jit::templates::TemplatePrerequisitesUnmetError>()
        .unwrap_or_else(|| panic!("expected the prerequisite report, got {failure:#}"));
    assert_eq!(
        unmet.unmet(),
        [jit::templates::TemplateRequirement::Type(
            "bogus".to_string()
        )]
    );
}
