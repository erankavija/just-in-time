//! All-or-nothing tests for the graph-template apply engine (jit:2f447380 — REQ-2).
//!
//! Apply commits its expanded delta under one repository lock; a write failure
//! anywhere in the sequence must leave the issue store observably unchanged. The
//! failure is INJECTED by [`FaultyStore`], an [`IssueStore`] wrapper over
//! `InMemoryStorage` that fails the FIRST `save_issue` matching a caller-supplied
//! predicate and then behaves normally, so the engine's own rollback writes go
//! through. That models a transient I/O failure mid-apply.
//!
//! Each test drives the real `create_issue` + gate + `add_dependency` path through
//! a `CommandExecutor` over the faulty store, fully isolated from the production
//! `.jit/`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use jit::domain::{Event, GateStage, Issue, Priority};
use jit::gate_presets::{GatePresetDefinition, PresetInfo};
use jit::storage::{GateRegistry, InMemoryStorage, IssueStore, PathReadError};
use jit::templates::{GraphTemplate, TemplateRegistry};
use jit::CommandExecutor;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

/// Predicate deciding which `save_issue` call to fail.
type SavePredicate = Arc<dyn Fn(&Issue) -> bool + Send + Sync>;

/// An [`IssueStore`] that delegates to `InMemoryStorage` but, once ARMED, fails
/// the first `save_issue` whose issue matches `fail_on`.
///
/// Arming separates fixture setup from the apply under test. Failing once
/// (rather than always) is what makes the injected fault a TRANSIENT write
/// failure: the apply's compensating writes still land, so the test observes the
/// rollback the engine performs rather than a store wedged by the injector.
#[derive(Clone)]
struct FaultyStore {
    inner: InMemoryStorage,
    fail_on: SavePredicate,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
}

impl FaultyStore {
    fn new(fail_on: impl Fn(&Issue) -> bool + Send + Sync + 'static) -> Self {
        Self {
            inner: InMemoryStorage::new(),
            fail_on: Arc::new(fail_on),
            armed: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start honoring the failure predicate (called once the fixture is built).
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    /// Whether the injected failure was actually reached (a test asserting a
    /// mid-apply rollback is meaningless if the write never failed).
    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

impl IssueStore for FaultyStore {
    fn save_issue(&self, issue: Issue) -> Result<()> {
        let should_fail = self.armed.load(Ordering::SeqCst)
            && !self.fired.load(Ordering::SeqCst)
            && (self.fail_on)(&issue);
        if should_fail {
            self.fired.store(true, Ordering::SeqCst);
            return Err(anyhow!("injected write failure saving issue {}", issue.id));
        }
        self.inner.save_issue(issue)
    }

    fn init(&self) -> Result<()> {
        self.inner.init()
    }
    fn load_issue(&self, id: &str) -> Result<Issue> {
        self.inner.load_issue(id)
    }
    fn load_issue_or_not_found(&self, id: &str) -> Result<Issue, PathReadError> {
        self.inner.load_issue_or_not_found(id)
    }
    fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
        self.inner.resolve_issue_id(partial_id)
    }
    fn delete_issue(&self, id: &str) -> Result<()> {
        self.inner.delete_issue(id)
    }
    fn list_issues(&self) -> Result<Vec<Issue>> {
        self.inner.list_issues()
    }
    fn load_gate_registry(&self) -> Result<GateRegistry> {
        self.inner.load_gate_registry()
    }
    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        self.inner.save_gate_registry(registry)
    }
    fn append_event(&self, event: &Event) -> Result<()> {
        self.inner.append_event(event)
    }
    fn read_events(&self) -> Result<Vec<Event>> {
        self.inner.read_events()
    }
    fn save_gate_run_result(&self, result: &jit::domain::GateRunResult) -> Result<()> {
        self.inner.save_gate_run_result(result)
    }
    fn load_gate_run_result(&self, run_id: &str) -> Result<jit::domain::GateRunResult> {
        self.inner.load_gate_run_result(run_id)
    }
    fn list_gate_runs_for_issue(&self, issue_id: &str) -> Result<Vec<jit::domain::GateRunResult>> {
        self.inner.list_gate_runs_for_issue(issue_id)
    }
    fn root(&self) -> &std::path::Path {
        self.inner.root()
    }
    fn read_repo_file(&self, rel_path: &str) -> Result<Option<String>, PathReadError> {
        self.inner.read_repo_file(rel_path)
    }
    fn write_repo_file(&self, rel_path: &str, content: &str) -> Result<(), PathReadError> {
        self.inner.write_repo_file(rel_path, content)
    }
    fn list_gate_presets(&self) -> Result<Vec<PresetInfo>> {
        self.inner.list_gate_presets()
    }
    fn get_gate_preset(&self, name: &str) -> Result<GatePresetDefinition> {
        self.inner.get_gate_preset(name)
    }
    fn save_gate_preset(&self, preset: &GatePresetDefinition) -> Result<std::path::PathBuf> {
        self.inner.save_gate_preset(preset)
    }
    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), PathReadError> {
        self.inner.read_path_bytes(path, at_commit)
    }
}

/// A `plan`-shaped template: `P` and `B` (`B → P`), the container anchor
/// depending on `B`, a `move-upstream-to-role` transform onto `P`, and a
/// registry-gate on the anchor.
fn plan_template() -> GraphTemplate {
    let toml = r#"
[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.anchors]]
  name  = "container"
  gates = ["repo-validate"]

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
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
    TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("plan")
        .unwrap()
        .clone()
}

/// The pre-apply fixture: an upstream issue `U` and an epic `C` depending on it,
/// plus the config-declared `repo-validate` gate the template's anchor names.
/// Arms the store on the way out, so only the apply under test can trip the
/// injected failure. Returns the executor, the container id, and the upstream id.
fn fixture(store: FaultyStore) -> (CommandExecutor<FaultyStore>, String, String) {
    std::env::set_var("JIT_TEST_MODE", "1");
    store.init().unwrap();
    let executor = CommandExecutor::new(store.clone());

    executor
        .add_gate_definition(
            "repo-validate".to_string(),
            "Repo Validate".to_string(),
            "Whole-repository validation must pass".to_string(),
            true,
            None,
            GateStage::Postcheck,
        )
        .unwrap();

    let (upstream, _) = executor
        .create_issue(
            "Upstream U".to_string(),
            "Upstream work".to_string(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();
    let (container, _) = executor
        .create_issue(
            "Auth epic".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "area:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();
    executor.add_dependency(&container, &upstream).unwrap();

    store.arm();
    (executor, container, upstream)
}

fn bindings(container: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("container".to_string(), container.to_string())])
}

fn type_of(issue: &Issue) -> Option<&str> {
    issue
        .labels
        .iter()
        .find_map(|l| l.strip_prefix("type:"))
        .map(|v| v.trim())
}

/// Assert the store holds exactly the pre-apply shape: `C` and `U` only, `C`
/// still depending on `U`, no scaffold node, no anchor gate attached.
fn assert_pre_apply_shape(
    executor: &CommandExecutor<FaultyStore>,
    container: &str,
    upstream: &str,
) {
    let issues = executor.storage().list_issues().unwrap();
    assert_eq!(
        issues.len(),
        2,
        "rollback must leave exactly the two pre-apply issues, found {:?}",
        issues.iter().map(|i| i.title.clone()).collect::<Vec<_>>()
    );
    assert!(
        !issues
            .iter()
            .any(|i| matches!(type_of(i), Some("planning") | Some("breakdown"))),
        "no scaffold node may survive a failed apply"
    );

    let container = executor.storage().load_issue(container).unwrap();
    assert_eq!(
        container.dependencies,
        vec![upstream.to_string()],
        "the container's pre-apply upstream edge must be intact"
    );
    assert!(
        container.gates_required.is_empty(),
        "no anchor gate may survive a failed apply"
    );

    let upstream = executor.storage().load_issue(upstream).unwrap();
    assert!(upstream.dependencies.is_empty());
}

// === REQ-2: a write failure mid-apply leaves the issue store unchanged ===

#[test]
fn test_apply_rolls_back_created_nodes_when_a_node_write_fails() {
    // The injected failure lands on the SECOND created node, after the first was
    // persisted: the rollback must delete the orphaned first node.
    let store = FaultyStore::new(|issue| issue.labels.iter().any(|l| l == "type:breakdown"));
    let (executor, container, upstream) = fixture(store.clone());

    let err = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();

    assert!(store.fired(), "the injected write failure must be reached");
    assert!(err.to_string().contains("injected write failure"), "{err}");
    assert_pre_apply_shape(&executor, &container, &upstream);
}

#[test]
fn test_apply_rolls_back_nodes_and_edges_when_an_edge_write_fails() {
    // The injected failure lands on the first write to the CONTAINER, which the
    // engine performs while wiring the anchor edge `C → B`. By then both scaffold
    // nodes exist and the internal edge `B → P` is persisted, so the rollback has
    // to delete two issues and revert the edge writes.
    let container_title = "Auth epic";
    let store = FaultyStore::new(move |issue| issue.title == container_title);
    let (executor, container, upstream) = fixture(store.clone());

    let err = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();

    assert!(store.fired(), "the injected write failure must be reached");
    assert!(err.to_string().contains("injected write failure"), "{err}");
    assert_pre_apply_shape(&executor, &container, &upstream);

    // The append-only event log records both directions: the scaffold nodes were
    // created, then deleted by the compensating rollback (`@/inv/event-log`).
    let events = executor.storage().read_events().unwrap();
    let created = events
        .iter()
        .filter(|e| e.get_type() == "issue_created")
        .count();
    let deleted = events
        .iter()
        .filter(|e| e.get_type() == "issue_deleted")
        .count();
    assert_eq!(created, 4, "U, C, P and B were each created");
    assert_eq!(deleted, 2, "P and B were each deleted by the rollback");
}

#[test]
fn test_apply_succeeds_and_wires_the_spine_when_no_write_fails() {
    // The control case: with no injected failure the same fixture and template
    // produce the `C → B → P → U` spine, so the rollback tests above are pinned
    // against a working apply rather than a broken engine.
    let store = FaultyStore::new(|_| false);
    let (executor, container, upstream) = fixture(store);

    let (result, _warnings) = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap();

    let planning = &result.created_node_ids_by_role["planning"];
    let breakdown = &result.created_node_ids_by_role["breakdown"];
    let container_issue = executor.storage().load_issue(&container).unwrap();
    assert_eq!(container_issue.dependencies, vec![breakdown.clone()]);
    assert_eq!(
        executor
            .storage()
            .load_issue(breakdown)
            .unwrap()
            .dependencies,
        vec![planning.clone()]
    );
    assert_eq!(
        executor
            .storage()
            .load_issue(planning)
            .unwrap()
            .dependencies,
        vec![upstream]
    );
    assert_eq!(
        container_issue.gates_required,
        vec!["repo-validate".to_string()]
    );
}
