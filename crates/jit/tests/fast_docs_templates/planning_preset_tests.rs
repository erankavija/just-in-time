//! Integration tests for the planning-bracket gate presets (T6).
//!
//! Exercises `apply_gate_preset` end-to-end via the in-process harness:
//! applying `plan-review` attaches the visible review placeholder to a planning node, and
//! applying `coverage-preview` attaches the deterministic scoped-validate gate
//! to a breakdown node.

use crate::harness::TestHarness;
use jit::declarations::{GateChecker, GateMode, GateStage};
use jit::domain::Priority;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::CommandExecutor;
use tempfile::TempDir;

#[test]
fn test_apply_plan_review_attaches_agent_gate_to_planning_node() {
    let h = TestHarness::new();
    // The planning node P (type label is illustrative; the preset is type-agnostic).
    let planning = h.create_issue("Plan the auth epic");
    h.executor
        .add_label(&planning, "type:planning")
        .expect("label P");

    let (result, _warnings) = h
        .executor
        .apply_gate_preset(&planning, "plan-review", None, false, false, &[])
        .expect("apply plan-review preset");

    assert!(
        result.added.contains(&"plan-review".to_string()),
        "plan-review gate attached to the planning node, got {:?}",
        result.added
    );

    // The issue now requires the plan-review gate, and the registered gate is
    // the explicit review placeholder supplied by the production package.
    let issue = h.get_issue(&planning);
    assert!(issue.gates_required.contains(&"plan-review".to_string()));

    let registry = h.storage.load_gate_registry().expect("load registry");
    let gate = registry.gates.get("plan-review").expect("gate registered");
    assert_eq!(gate.mode, GateMode::Auto);
    assert_eq!(gate.checker, Some(GateChecker::ReviewPlaceholder));
}

#[test]
fn test_apply_coverage_preview_attaches_scoped_validate_gate_to_breakdown_node() {
    let h = TestHarness::new();
    // The breakdown node B, carrying a brackets: pointer to its container.
    let breakdown = h.create_issue("Breakdown of the auth epic");
    h.executor
        .add_label(&breakdown, "type:breakdown")
        .expect("label B");
    h.executor
        .add_label(&breakdown, "brackets:abc12345")
        .expect("brackets label");

    let (result, _warnings) = h
        .executor
        .apply_gate_preset(&breakdown, "coverage-preview", None, false, false, &[])
        .expect("apply coverage-preview preset");

    assert!(
        result.added.contains(&"coverage-preview".to_string()),
        "coverage-preview gate attached to the breakdown node, got {:?}",
        result.added
    );

    let issue = h.get_issue(&breakdown);
    assert!(issue
        .gates_required
        .contains(&"coverage-preview".to_string()));

    // The registered native checker resolves the container from the brackets:
    // label and runs scoped validation in-process.
    let registry = h.storage.load_gate_registry().expect("load registry");
    let gate = registry
        .gates
        .get("coverage-preview")
        .expect("gate registered");
    assert_eq!(gate.mode, GateMode::Auto);
    match gate.checker.as_ref().expect("coverage gate has a checker") {
        GateChecker::LabelTargetValidation { label_namespace } => {
            assert_eq!(label_namespace, "brackets")
        }
        other => panic!("expected label-target checker, got {other:?}"),
    }
}

/// @/inv/event-log (jit:bb7d57a2): a preset application that WRITES the gate
/// registry appends a registry-scoped audit event per definition write —
/// `gate_definition_created` for a new key, `gate_definition_updated` for a
/// timeout-override overwrite — and a no-write re-application appends none.
#[test]
fn test_apply_gate_preset_appends_definition_events() {
    let h = TestHarness::new();
    let planning = h.create_issue("Plan the auth epic");
    h.executor
        .add_label(&planning, "type:planning")
        .expect("label P");

    h.executor
        .apply_gate_preset(&planning, "plan-review", None, false, false, &[])
        .expect("apply plan-review preset");

    let created = |events: &[jit::domain::Event]| {
        events
            .iter()
            .filter(|e| e.get_type() == "gate_definition_created")
            .count()
    };
    let updated = |events: &[jit::domain::Event]| {
        events
            .iter()
            .filter(|e| e.get_type() == "gate_definition_updated")
            .count()
    };

    let events = h.storage.read_events().unwrap();
    assert_eq!(
        created(&events),
        1,
        "first application defines the preset gate -> one created event"
    );
    assert_eq!(updated(&events), 0);

    // Re-apply without an override: the key exists, nothing is written, no event.
    let second = h.create_issue("Another planning node");
    h.executor
        .apply_gate_preset(&second, "plan-review", None, false, false, &[])
        .expect("re-apply preset");
    let events = h.storage.read_events().unwrap();
    assert_eq!(
        created(&events),
        1,
        "no-write re-application appends nothing"
    );
    assert_eq!(updated(&events), 0);

    // Re-apply WITH a timeout override: the existing definition is overwritten.
    let third = h.create_issue("Overridden planning node");
    h.executor
        .apply_gate_preset(&third, "plan-review", Some(120), false, false, &[])
        .expect("re-apply preset with timeout override");
    let events = h.storage.read_events().unwrap();
    assert_eq!(created(&events), 1);
    assert_eq!(
        updated(&events),
        1,
        "timeout-override overwrite of an existing definition -> one updated event"
    );
}

/// REQ-02: the full lifecycle of a PROJECT-DEFINED preset — save, list, show,
/// apply — exercised in-process end to end. The bundle here is a project's own
/// gate collection (a test runner plus a review gate), the kind that used to be
/// a compiled-in language bundle and is now declared by the project alone; no
/// removed builtin is involved. This runs over a real `JsonFileStorage` because
/// the in-memory `TestHarness` does not persist custom presets under
/// `.jit/config/gate-presets/` (its store is builtin-only), so the save/list/show
/// path is only observable against the file-backed store.
#[test]
fn test_project_defined_preset_save_list_show_apply_in_process() {
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp.path());
    storage.init().unwrap();
    let layout =
        jit::storage::discover_repository_layout(temp.path().parent().unwrap(), storage.root())
            .unwrap();
    let executor = CommandExecutor::new(storage).with_layout(layout);

    // Two project-declared gates standing in for a language-specific CI bundle.
    executor
        .add_gate_definition(
            "tests".to_string(),
            "All tests pass".to_string(),
            "run the test suite".to_string(),
            false,
            None,
            GateStage::Postcheck,
        )
        .expect("define tests gate");
    executor
        .add_gate_definition(
            "code-review".to_string(),
            "Code review".to_string(),
            "human review".to_string(),
            false,
            None,
            GateStage::Postcheck,
        )
        .expect("define code-review gate");

    // A reference issue carrying both gates, from which the preset is captured.
    let (reference, _) = executor
        .create_issue(
            "Reference issue".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("create reference issue");
    executor.add_gate(&reference, "tests".to_string()).unwrap();
    executor
        .add_gate(&reference, "code-review".to_string())
        .unwrap();

    // SAVE: capture the reference issue's gates into a project-defined preset,
    // written under the project's gate-presets directory.
    let saved_path = executor
        .create_gate_preset("ci", &reference)
        .expect("save project-defined preset");
    assert!(
        saved_path.exists(),
        "preset should be written to disk at {saved_path:?}"
    );
    assert!(
        temp.path().join("config/gate-presets/ci.json").exists(),
        "project-defined preset should live under .jit/config/gate-presets/"
    );

    // LIST: the preset surfaces as project-defined (non-builtin) alongside the
    // planning-bracket builtins, and none of the removed language bundles appear.
    let presets = executor.list_gate_presets().expect("list presets");
    let ci = presets
        .iter()
        .find(|p| p.name == "ci")
        .expect("ci preset listed");
    assert!(!ci.builtin, "ci is project-defined, not builtin");
    assert_eq!(ci.gate_count, 2);
    for removed in [
        "rust-tdd",
        "python-tdd",
        "js-tdd",
        "minimal",
        "security-audit",
    ] {
        assert!(
            !presets.iter().any(|p| p.name == removed),
            "removed builtin {removed} must not be listed"
        );
    }

    // SHOW: the saved preset reports the bundled gate keys.
    let shown = executor.show_gate_preset("ci").expect("show preset");
    let keys: Vec<&str> = shown.gates.iter().map(|g| g.key.as_str()).collect();
    assert!(keys.contains(&"tests"), "ci bundles the tests gate");
    assert!(
        keys.contains(&"code-review"),
        "ci bundles the code-review gate"
    );

    // APPLY: attaching the preset to a fresh issue adds both bundled gates.
    let (target, _) = executor
        .create_issue(
            "Feature issue".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("create target issue");
    let (result, _warnings) = executor
        .apply_gate_preset(&target, "ci", None, false, false, &[])
        .expect("apply project-defined preset");
    assert!(result.added.contains(&"tests".to_string()));
    assert!(result.added.contains(&"code-review".to_string()));

    // The target issue now requires both gates.
    let issue = executor.storage().load_issue(&target).expect("load target");
    assert!(issue.gates_required.contains(&"tests".to_string()));
    assert!(issue.gates_required.contains(&"code-review".to_string()));
}
