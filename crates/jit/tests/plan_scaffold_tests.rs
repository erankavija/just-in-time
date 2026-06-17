//! Tests for the bracket scaffolding command (T5): `jit plan <id>` (retrofit)
//! and `jit issue create --with-planning`.
//!
//! These exercise the `CommandExecutor` core methods in-process via the
//! `TestHarness` (InMemoryStorage). Because the in-memory store carries no
//! `.jit/config.toml`, the `[planning]` vocabulary is injected explicitly via a
//! `PlanningConfig` passed to the `*_with_config` core methods — the thin
//! config-reading public wrappers are covered by the CLI integration tests.

mod harness;

use harness::TestHarness;
use jit::commands::CommandExecutor;
use jit::config::PlanningConfig;
use jit::domain::Priority;
use jit::labels::parse_label;
use jit::storage::{InMemoryStorage, IssueStore};

/// A `[planning]` vocabulary mirroring the SDD example: `epic` containers are
/// breakable, the planning node is `type:planning`, breakdown is
/// `type:breakdown`, the plan lives inline, and the agent plan-review preset is
/// applied to `P`.
fn sdd_planning_config() -> PlanningConfig {
    PlanningConfig {
        breakable_types: vec!["epic".to_string()],
        planning_type: "planning".to_string(),
        breakdown_type: "breakdown".to_string(),
        plan_doc_location: "inline".to_string(),
        plan_gate_preset: "plan-review".to_string(),
        coverage_gate_preset: "coverage-preview".to_string(),
    }
}

/// Return the `type:*` value of an issue, or None if it carries no type label.
fn type_of(issue: &jit::domain::Issue) -> Option<String> {
    issue.labels.iter().find_map(|l| {
        parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}

fn create_epic(h: &TestHarness, title: &str) -> String {
    let (id, _) = h
        .executor
        .create_issue(
            title.to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();
    id
}

// ===================== jit plan <id> (retrofit) =====================

#[test]
fn test_plan_existing_creates_planning_node_wired_before_container() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let container = create_epic(&h, "Auth epic");

    let (result, _warnings) = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    // A planning node P was created, typed from config.
    let planning = h.get_issue(&result.planning_id);
    assert_eq!(type_of(&planning).as_deref(), Some("planning"));

    // The container depends on P (C -> P).
    let c = h.get_issue(&container);
    assert!(
        c.dependencies.contains(&result.planning_id),
        "container must depend on the planning node, got {:?}",
        c.dependencies
    );
}

#[test]
fn test_plan_existing_applies_plan_review_gate_to_planning_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let container = create_epic(&h, "Auth epic");

    let (result, _warnings) = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    let planning = h.get_issue(&result.planning_id);
    assert!(
        planning.gates_required.contains(&"plan-review".to_string()),
        "planning node must carry the plan-review gate, got {:?}",
        planning.gates_required
    );
    // The gate is registered in the registry by the preset-apply path.
    let registry = h.storage.load_gate_registry().unwrap();
    assert!(registry.gates.contains_key("plan-review"));
}

#[test]
fn test_plan_existing_moves_upstream_deps_onto_planning_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();

    // Two upstream issues the container currently depends on.
    let up1 = h.create_issue("Upstream one");
    let up2 = h.create_issue("Upstream two");
    let container = create_epic(&h, "Auth epic");
    h.executor.add_dependency(&container, &up1).unwrap();
    h.executor.add_dependency(&container, &up2).unwrap();

    let (result, _warnings) = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    // P now carries the upstream deps.
    let planning = h.get_issue(&result.planning_id);
    assert!(
        planning.dependencies.contains(&up1) && planning.dependencies.contains(&up2),
        "upstream deps must move onto P, got {:?}",
        planning.dependencies
    );

    // The container's ONLY remaining dependency is P (closure node at the back).
    let c = h.get_issue(&container);
    assert_eq!(
        c.dependencies,
        vec![result.planning_id.clone()],
        "container must depend only on P after retrofit, got {:?}",
        c.dependencies
    );
}

#[test]
fn test_plan_existing_does_not_create_breakdown_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let container = create_epic(&h, "Auth epic");

    let _ = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    // No issue carries the breakdown type — B is the breakdown step's job (T10).
    let breakdown_nodes = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("breakdown"))
        .count();
    assert_eq!(
        breakdown_nodes, 0,
        "scaffolding must NOT create a breakdown node"
    );
}

#[test]
fn test_plan_existing_rejects_non_breakable_container_type() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();

    // A task is not in breakable_types (only epic is).
    let (task, _) = h
        .executor
        .create_issue(
            "Just a task".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            false,
        )
        .unwrap();

    let err = h
        .executor
        .plan_existing_with_config(&cfg, &task, false)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("breakable") || msg.contains("task"),
        "error should explain the type is not breakable, got: {msg}"
    );
}

#[test]
fn test_plan_existing_sets_external_plan_doc_location_reference() {
    let h = TestHarness::new();
    let mut cfg = sdd_planning_config();
    // External template: P should get a document reference to the resolved path.
    cfg.plan_doc_location = "dev/plans/{id}.md".to_string();
    let container = create_epic(&h, "Auth epic");

    let (result, _warnings) = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    let planning = h.get_issue(&result.planning_id);
    let expected = format!("dev/plans/{container}.md");
    assert!(
        planning.documents.iter().any(|d| d.path == expected),
        "P must carry a plan-doc reference at the resolved path {expected}, got {:?}",
        planning.documents
    );
    assert_eq!(result.plan_doc_location, expected);
}

#[test]
fn test_plan_existing_inline_location_adds_no_document_reference() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config(); // inline
    let container = create_epic(&h, "Auth epic");

    let (result, _warnings) = h
        .executor
        .plan_existing_with_config(&cfg, &container, false)
        .unwrap();

    let planning = h.get_issue(&result.planning_id);
    // Inline plan == P's own body; no external doc reference is attached.
    assert!(
        planning.documents.is_empty(),
        "inline location must not attach an external doc reference, got {:?}",
        planning.documents
    );
    assert_eq!(result.plan_doc_location, "inline");
}

// ===================== issue create --with-planning =====================

#[test]
fn test_create_with_planning_brackets_new_container() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();

    let (result, _warnings) = h
        .executor
        .create_with_planning_with_config(
            &cfg,
            "Brand new epic".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: ok\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();

    // The container was created with the epic type.
    let c = h.get_issue(&result.container_id);
    assert_eq!(type_of(&c).as_deref(), Some("epic"));

    // A planning node exists, the container depends on it, and it carries the gate.
    let planning = h.get_issue(&result.planning_id);
    assert_eq!(type_of(&planning).as_deref(), Some("planning"));
    assert!(c.dependencies.contains(&result.planning_id));
    assert!(planning.gates_required.contains(&"plan-review".to_string()));
}

#[test]
fn test_create_with_planning_does_not_create_breakdown_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();

    let _ = h
        .executor
        .create_with_planning_with_config(
            &cfg,
            "Brand new epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();

    let breakdown_nodes = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("breakdown"))
        .count();
    assert_eq!(
        breakdown_nodes, 0,
        "create --with-planning must NOT create a breakdown node"
    );
}

#[test]
fn test_create_with_planning_rejects_non_breakable_type() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();

    let err = h
        .executor
        .create_with_planning_with_config(
            &cfg,
            "A task".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            false,
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("breakable") || err.to_string().contains("task"),
        "error should explain the type is not breakable, got: {}",
        err
    );
}

// ============= config-reading public wrappers (read [planning]) =============

const PLANNING_CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }

[planning]
breakable_types = ["epic"]
planning_type = "planning"
breakdown_type = "breakdown"
plan_doc_location = "inline"
plan_gate_preset = "plan-review"
coverage_gate_preset = "coverage-preview"
"#;

/// Build an executor whose `.jit/config.toml` declares the `[planning]` bracket,
/// so the config-reading public wrappers resolve the vocabulary from disk.
fn executor_with_planning_config() -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), PLANNING_CONFIG_TOML).unwrap();
    CommandExecutor::new(storage)
}

#[test]
fn test_plan_existing_reads_planning_config_from_disk() {
    let executor = executor_with_planning_config();
    let (container, _) = executor
        .create_issue(
            "Auth epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();

    // The public wrapper reads [planning] from config.toml; no config injected.
    let (result, _warnings) = executor.plan_existing(&container, false).unwrap();
    assert_eq!(result.planning_type, "planning");
    assert_eq!(result.plan_gate_preset, "plan-review");

    let planning = executor.storage().load_issue(&result.planning_id).unwrap();
    assert!(type_of(&planning).as_deref() == Some("planning"));
    let c = executor.storage().load_issue(&result.container_id).unwrap();
    assert!(c.dependencies.contains(&result.planning_id));
}

#[test]
fn test_plan_existing_errors_when_no_planning_section() {
    // No config.toml at all -> empty config -> no [planning] section.
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    let executor = CommandExecutor::new(storage.clone());
    let (container, _) = executor
        .create_issue(
            "Auth epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();

    let err = executor.plan_existing(&container, false).unwrap_err();
    assert!(
        err.to_string().contains("[planning]"),
        "error should name the missing [planning] section, got: {err}"
    );
}
