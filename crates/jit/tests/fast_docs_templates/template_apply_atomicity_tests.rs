//! All-or-nothing tests for the graph-template apply engine (jit:2f447380 — REQ-2).
//!
//! Apply commits its expanded delta under one repository lock; a write failure
//! anywhere in the sequence must leave the issue store observably unchanged. The
//! failure is INJECTED by [`FaultyStore`], an [`IssueStore`] wrapper that fails
//! the FIRST `save_issue` matching a caller-supplied predicate and then behaves
//! normally, so the engine's own rollback writes go through. That models a
//! transient I/O failure mid-apply. It wraps either backend, so the rollback
//! assertions run against `InMemoryStorage` and the real `JsonFileStorage` alike.
//!
//! [`StallingStore`] extends the same idea over a real `JsonFileStorage`: it holds
//! the apply's lock window open long enough for a second writer to try to write
//! into it, so the tests can prove that an ordinary write cannot interleave with
//! an apply and that the rollback only ever undoes the apply's own writes.
//!
//! Each test drives the real `create_issue` + gate + `add_dependency` path through
//! a `CommandExecutor` over the wrapped store, fully isolated from the production
//! `.jit/`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use jit::declarations::GateRegistry;
use jit::declarations::GateStage;
use jit::domain::{Event, Issue, Priority};
use jit::gate_presets::{GatePresetDefinition, PresetInfo};
use jit::storage::{InMemoryStorage, IssueStore, JsonFileStorage, PathReadError};
use jit::templates::{GraphTemplate, TemplateRegistry};
use jit::CommandExecutor;
use tempfile::TempDir;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

/// Predicate deciding which `save_issue` call to fail.
type SavePredicate = Arc<dyn Fn(&Issue) -> bool + Send + Sync>;

/// An [`IssueStore`] that delegates to an inner store but, once ARMED, fails the
/// first `save_issue` whose issue matches `fail_on`.
///
/// Arming separates fixture setup from the apply under test. Failing once
/// (rather than always) is what makes the injected fault a TRANSIENT write
/// failure: the apply's compensating writes still land, so the test observes the
/// rollback the engine performs rather than a store wedged by the injector.
///
/// Generic over the inner store so one rollback assertion runs against both
/// `InMemoryStorage` and the real `JsonFileStorage`, which is the only way an
/// in-memory atomicity test says anything about the file backend.
#[derive(Clone)]
struct FaultyStore<S: IssueStore> {
    inner: S,
    fail_on: SavePredicate,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    /// Ids the ARMED store saw go through `save_issue`, i.e. the apply's own
    /// writes. A rollback assertion over an issue absent here would be vacuous.
    saved: Arc<Mutex<Vec<String>>>,
}

impl<S: IssueStore> FaultyStore<S> {
    fn new(inner: S, fail_on: impl Fn(&Issue) -> bool + Send + Sync + 'static) -> Self {
        Self {
            inner,
            fail_on: Arc::new(fail_on),
            armed: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
            saved: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Whether the apply under test wrote `id` before its injected failure.
    fn saved(&self, id: &str) -> bool {
        self.saved.lock().unwrap().iter().any(|saved| saved == id)
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

impl<S: IssueStore> IssueStore for FaultyStore<S> {
    /// Delegated, so the apply under test holds the SAME repository write lock the
    /// wrapped store's write paths take.
    fn acquire_repo_write_lock(&self) -> Result<jit::storage::RepoWriteGuard> {
        self.inner.acquire_repo_write_lock()
    }

    fn save_issue(&self, issue: Issue) -> Result<()> {
        let should_fail = self.armed.load(Ordering::SeqCst)
            && !self.fired.load(Ordering::SeqCst)
            && (self.fail_on)(&issue);
        if should_fail {
            self.fired.store(true, Ordering::SeqCst);
            return Err(anyhow!("injected write failure saving issue {}", issue.id));
        }
        if self.armed.load(Ordering::SeqCst) {
            self.saved.lock().unwrap().push(issue.id.clone());
        }
        self.inner.save_issue(issue)
    }

    /// Delegated, never failed: the rollback's compensating writes must land so a
    /// test observes the engine's rollback rather than a store wedged by the
    /// injector.
    fn restore_issue_verbatim(&self, issue: Issue) -> Result<()> {
        self.inner.restore_issue_verbatim(issue)
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
    fn is_file_backed(&self) -> bool {
        self.inner.is_file_backed()
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

struct FaultyMutationSession<'a> {
    inner: Box<dyn jit::storage::RepositoryMutationSession + 'a>,
    fail_on: SavePredicate,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    saved: Arc<Mutex<Vec<String>>>,
}

impl jit::storage::RepositoryMutationSession for FaultyMutationSession<'_> {
    fn layout(&self) -> &jit::repository_state::RepositoryLayout {
        self.inner.layout()
    }

    fn capture(
        &mut self,
        spec: jit::repository_state::CaptureSpec,
    ) -> Result<jit::repository_state::RepositoryImage, jit::storage::RepositoryStateStoreError>
    {
        self.inner.capture(spec)
    }

    fn apply(
        &mut self,
        plan: &jit::repository_state::MaterializationPlan,
    ) -> Result<jit::storage::RepositoryApplyOutcome, jit::storage::RepositoryStateStoreError> {
        let issues = plan
            .delta()
            .actions()
            .iter()
            .filter_map(|action| match action {
                jit::repository_state::RepositoryAction::WriteFile { bytes, .. } => {
                    serde_json::from_slice::<Issue>(bytes).ok()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if self.armed.load(Ordering::SeqCst)
            && !self.fired.load(Ordering::SeqCst)
            && issues.iter().any(|issue| (self.fail_on)(issue))
        {
            self.fired.store(true, Ordering::SeqCst);
            return Err(jit::storage::RepositoryStateStoreError::UnsafeTarget(
                "injected write failure publishing issue mutation".to_string(),
            ));
        }
        let outcome = self.inner.apply(plan)?;
        if self.armed.load(Ordering::SeqCst) {
            self.saved
                .lock()
                .unwrap()
                .extend(issues.into_iter().map(|issue| issue.id));
        }
        Ok(outcome)
    }
}

/// Inject the fault at the canonical typed publication boundary. The legacy
/// `save_issue` override remains only for template's not-yet-migrated rollback
/// choreography; newly created/updated issues fail through session `apply`.
impl<S: IssueStore + jit::storage::RepositoryStateStore> jit::storage::RepositoryStateStore
    for FaultyStore<S>
{
    fn open_mutation_session(
        &self,
        layout: jit::repository_state::RepositoryLayout,
    ) -> Result<
        Box<dyn jit::storage::RepositoryMutationSession + '_>,
        jit::storage::RepositoryStateStoreError,
    > {
        Ok(Box::new(FaultyMutationSession {
            inner: self.inner.open_mutation_session(layout)?,
            fail_on: self.fail_on.clone(),
            armed: self.armed.clone(),
            fired: self.fired.clone(),
            saved: self.saved.clone(),
        }))
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
fn fixture<S: IssueStore + jit::storage::RepositoryStateStore>(
    store: FaultyStore<S>,
) -> (CommandExecutor<FaultyStore<S>>, String, String) {
    std::env::set_var("JIT_TEST_MODE", "1");
    store.init().unwrap();
    if store.is_file_backed() {
        std::fs::write(store.root().join("config.toml"), "").unwrap();
    } else {
        store.write_repo_file(".jit/config.toml", "").unwrap();
    }
    let layout =
        jit::storage::discover_repository_layout(store.root().parent().unwrap(), store.root())
            .unwrap();
    let executor = CommandExecutor::new(store.clone()).with_layout(layout);

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
fn assert_pre_apply_shape<S: IssueStore>(
    executor: &CommandExecutor<FaultyStore<S>>,
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
    let store = FaultyStore::new(InMemoryStorage::new(), |issue| {
        issue.labels.iter().any(|l| l == "type:breakdown")
    });
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
    let store = FaultyStore::new(InMemoryStorage::new(), move |issue| {
        issue.title == container_title
    });
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

/// Drive an apply that mutates the pre-existing container and then fails, and
/// assert the rollback puts every pre-existing record back FIELD FOR FIELD.
///
/// The injected failure lands on the anchor-gate write, the last step of the
/// commit. By then the apply has rewritten the container twice (adding `C → B`,
/// removing `C → U`), so the rollback has to restore a genuinely mutated
/// pre-existing issue. Comparing whole `Issue` values is the point: a rollback
/// that writes the snapshot back through `save_issue` restores the content and
/// leaves a fresh `updated_at`, which `jit issue show --json` and
/// `jit graph export --full` both surface, so the store is observably changed by
/// an apply that failed (REQ-2).
fn check_rollback_restores_records_verbatim<S: IssueStore + jit::storage::RepositoryStateStore>(
    inner: S,
) {
    let store = FaultyStore::new(inner, |issue| {
        issue.gates_required.iter().any(|g| g == "repo-validate")
    });
    let (executor, container, _upstream) = fixture(store.clone());

    let sorted_records = || {
        let mut issues = executor.storage().list_issues().unwrap();
        issues.sort_by(|a, b| a.id.cmp(&b.id));
        issues
    };
    let before = sorted_records();

    let err = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();

    assert!(store.fired(), "the injected write failure must be reached");
    assert!(err.to_string().contains("injected write failure"), "{err}");

    // The apply really did write the pre-existing container before failing, so the
    // assertion below exercises the restore path rather than an untouched store.
    assert!(
        store.saved(&container),
        "the apply must have mutated the pre-existing container"
    );

    assert_eq!(
        sorted_records(),
        before,
        "every pre-existing record must survive a failed apply unchanged, \
         `updated_at` included"
    );
}

#[test]
fn test_rollback_restores_pre_existing_records_verbatim_in_memory() {
    check_rollback_restores_records_verbatim(InMemoryStorage::new());
}

#[test]
fn test_rollback_restores_pre_existing_records_verbatim_on_disk() {
    // The same assertion over the real file backend: both stores must stamp
    // `updated_at` on `save_issue` and preserve it on `restore_issue_verbatim`,
    // or the in-memory test above proves nothing about `.jit/issues/<id>.json`.
    let temp = TempDir::new().unwrap();
    check_rollback_restores_records_verbatim(JsonFileStorage::new(temp.path().join(".jit")));
}

#[test]
fn test_apply_succeeds_and_wires_the_spine_when_no_write_fails() {
    // The control case: with no injected failure the same fixture and template
    // produce the `C → B → P → U` spine, so the rollback tests above are pinned
    // against a working apply rather than a broken engine.
    let store = FaultyStore::new(InMemoryStorage::new(), |_| false);
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

// === REQ-2: apply excludes ordinary writers for its whole window ===

/// An [`IssueStore`] over a REAL [`JsonFileStorage`] that, once armed, STALLS
/// inside the first `save_issue` matching `stall_on` and then fails it.
///
/// The stall holds the apply's repository write lock open for a known interval,
/// which is what lets a second thread try to write into the middle of the apply's
/// read-validate-write-rollback window. The wrapper also records what the
/// rollback observed (`list_issues` after the failure) and every issue it
/// deleted, so a test can assert that no concurrent write ever became visible to
/// the rollback and that the rollback touched only nodes the apply created.
#[derive(Clone)]
struct StallingStore {
    inner: JsonFileStorage,
    stall_on: SavePredicate,
    stall_for: Duration,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    /// Set + notified the moment the apply enters the stall (lock held).
    stalling: Arc<(Mutex<bool>, Condvar)>,
    /// Titles the rollback's `list_issues` saw, in first-seen order.
    rollback_saw: Arc<Mutex<Vec<String>>>,
    /// Ids the rollback deleted.
    deleted: Arc<Mutex<Vec<String>>>,
}

impl StallingStore {
    fn new(
        inner: JsonFileStorage,
        stall_on: impl Fn(&Issue) -> bool + Send + Sync + 'static,
        stall_for: Duration,
    ) -> Self {
        Self {
            inner,
            stall_on: Arc::new(stall_on),
            stall_for,
            armed: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
            stalling: Arc::new((Mutex::new(false), Condvar::new())),
            rollback_saw: Arc::new(Mutex::new(Vec::new())),
            deleted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    /// Block until the apply is inside the stall, i.e. inside its lock window.
    fn await_stall(&self) {
        let (mutex, condvar) = &*self.stalling;
        let mut inside = mutex.lock().unwrap();
        while !*inside {
            inside = condvar.wait(inside).unwrap();
        }
    }

    fn rollback_saw(&self) -> Vec<String> {
        self.rollback_saw.lock().unwrap().clone()
    }

    fn deleted(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }
}

impl IssueStore for StallingStore {
    /// Delegated: the apply and the wrapped store's write paths must contend for
    /// ONE lock, not two.
    fn acquire_repo_write_lock(&self) -> Result<jit::storage::RepoWriteGuard> {
        self.inner.acquire_repo_write_lock()
    }

    fn save_issue(&self, issue: Issue) -> Result<()> {
        let should_stall = self.armed.load(Ordering::SeqCst)
            && !self.fired.load(Ordering::SeqCst)
            && (self.stall_on)(&issue);
        if !should_stall {
            return self.inner.save_issue(issue);
        }
        self.fired.store(true, Ordering::SeqCst);

        let (mutex, condvar) = &*self.stalling;
        *mutex.lock().unwrap() = true;
        condvar.notify_all();

        std::thread::sleep(self.stall_for);
        Err(anyhow!("injected write failure saving issue {}", issue.id))
    }

    /// Delegated, never stalled: this is the rollback's compensating write, which
    /// runs after the stall inside the same lock window.
    fn restore_issue_verbatim(&self, issue: Issue) -> Result<()> {
        self.inner.restore_issue_verbatim(issue)
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let issues = self.inner.list_issues()?;
        if self.fired.load(Ordering::SeqCst) {
            let mut seen = self.rollback_saw.lock().unwrap();
            for issue in &issues {
                if !seen.contains(&issue.title) {
                    seen.push(issue.title.clone());
                }
            }
        }
        Ok(issues)
    }

    fn delete_issue(&self, id: &str) -> Result<()> {
        self.deleted.lock().unwrap().push(id.to_string());
        self.inner.delete_issue(id)
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

struct StallingMutationSession<'a> {
    inner: Box<dyn jit::storage::RepositoryMutationSession + 'a>,
    stall_on: SavePredicate,
    stall_for: Duration,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    stalling: Arc<(Mutex<bool>, Condvar)>,
}

impl jit::storage::RepositoryMutationSession for StallingMutationSession<'_> {
    fn layout(&self) -> &jit::repository_state::RepositoryLayout {
        self.inner.layout()
    }

    fn capture(
        &mut self,
        spec: jit::repository_state::CaptureSpec,
    ) -> Result<jit::repository_state::RepositoryImage, jit::storage::RepositoryStateStoreError>
    {
        self.inner.capture(spec)
    }

    fn apply(
        &mut self,
        plan: &jit::repository_state::MaterializationPlan,
    ) -> Result<jit::storage::RepositoryApplyOutcome, jit::storage::RepositoryStateStoreError> {
        let should_stall = self.armed.load(Ordering::SeqCst)
            && !self.fired.load(Ordering::SeqCst)
            && plan.delta().actions().iter().any(|action| match action {
                jit::repository_state::RepositoryAction::WriteFile { bytes, .. } => {
                    serde_json::from_slice::<Issue>(bytes)
                        .is_ok_and(|issue| (self.stall_on)(&issue))
                }
                _ => false,
            });
        if should_stall {
            self.fired.store(true, Ordering::SeqCst);
            let (mutex, condvar) = &*self.stalling;
            *mutex.lock().unwrap() = true;
            condvar.notify_all();
            std::thread::sleep(self.stall_for);
            return Err(jit::storage::RepositoryStateStoreError::UnsafeTarget(
                "injected write failure publishing issue mutation".to_string(),
            ));
        }
        self.inner.apply(plan)
    }
}

/// Inject the stall/failure at the typed publication boundary while preserving
/// template rollback's still-legacy `IssueStore` observation hooks.
impl jit::storage::RepositoryStateStore for StallingStore {
    fn open_mutation_session(
        &self,
        layout: jit::repository_state::RepositoryLayout,
    ) -> Result<
        Box<dyn jit::storage::RepositoryMutationSession + '_>,
        jit::storage::RepositoryStateStoreError,
    > {
        Ok(Box::new(StallingMutationSession {
            inner: self.inner.open_mutation_session(layout)?,
            stall_on: self.stall_on.clone(),
            stall_for: self.stall_for,
            armed: self.armed.clone(),
            fired: self.fired.clone(),
            stalling: self.stalling.clone(),
        }))
    }
}

/// The same pre-apply fixture as [`fixture`], over a file-backed store.
fn json_fixture(store: StallingStore) -> (CommandExecutor<StallingStore>, String, String) {
    std::env::set_var("JIT_TEST_MODE", "1");
    store.init().unwrap();
    std::fs::write(store.root().join("config.toml"), "").unwrap();
    let layout =
        jit::storage::discover_repository_layout(store.root().parent().unwrap(), store.root())
            .unwrap();
    let executor = CommandExecutor::new(store.clone()).with_layout(layout);

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

#[test]
fn test_apply_excludes_a_concurrent_writer_and_rollback_spares_its_issue() {
    // A second writer (a stand-in for a concurrent `jit issue create`) starts its
    // write the moment the apply is inside its lock window, through a SEPARATE
    // storage instance over the same root — the cross-process shape. The apply
    // holds the repository write lock, so:
    //   * the writer's `save_issue` cannot land until the apply (including its
    //     rollback) has finished, which the elapsed-time assertion pins down;
    //   * the rollback's own `list_issues` therefore never sees the writer's
    //     issue, and the rollback deletes only the node the apply created.
    //
    // The working tree is NOT a git repository, so this also pins `jit apply`'s
    // atomicity without git (`@/charter/D-4`): the lock lives in the storage root.
    let temp = TempDir::new().unwrap();
    let jit_root = temp.path().join(".jit");
    assert!(
        !temp.path().join(".git").exists(),
        "the fixture must be a non-git working tree"
    );

    let stall = Duration::from_millis(600);
    let store = StallingStore::new(
        JsonFileStorage::new(&jit_root),
        |issue| issue.labels.iter().any(|l| l == "type:breakdown"),
        stall,
    );
    let (executor, container, upstream) = json_fixture(store.clone());

    // The concurrent ordinary writer: blocks until the apply is stalled inside its
    // lock, then creates an issue through its own `JsonFileStorage`.
    let writer = {
        let store = store.clone();
        let jit_root = jit_root.clone();
        std::thread::spawn(move || {
            store.await_stall();
            let writer_storage = JsonFileStorage::new(&jit_root);
            let layout = jit::storage::discover_repository_layout(
                jit_root.parent().unwrap(),
                writer_storage.root(),
            )
            .unwrap();
            let writer_executor = CommandExecutor::new(writer_storage).with_layout(layout);
            let started = Instant::now();
            let (id, _) = writer_executor
                .create_issue(
                    "Concurrent bystander".to_string(),
                    "Written by another writer during the apply".to_string(),
                    Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap();
            (id, started.elapsed())
        })
    };

    let err = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();
    let (bystander_id, writer_elapsed) = writer.join().unwrap();

    assert!(store.fired(), "the injected write failure must be reached");
    assert!(err.to_string().contains("injected write failure"), "{err}");

    // 1. The ordinary write was excluded: it could not complete until the apply
    //    released the lock, so it waited out most of the stall.
    assert!(
        writer_elapsed >= stall / 2,
        "the concurrent writer must block on the apply's lock, waited only {writer_elapsed:?}"
    );

    // 2. Nothing the writer did was ever visible inside the apply's window: the
    //    rollback saw only the pre-apply issues plus the node the apply created.
    let saw = store.rollback_saw();
    assert!(
        !saw.contains(&"Concurrent bystander".to_string()),
        "no concurrent write may become visible to the rollback, saw {saw:?}"
    );
    assert_eq!(
        saw.len(),
        3,
        "the rollback must see exactly U, C and the created planning node, saw {saw:?}"
    );

    // 3. The rollback deleted only what the apply created.
    let deleted = store.deleted();
    assert_eq!(deleted.len(), 1, "exactly the created planning node");
    assert_ne!(
        deleted[0], bystander_id,
        "the rollback must never delete an issue the apply did not create"
    );

    // 4. Final state: the pre-apply shape plus the bystander, no scaffold node.
    let issues = executor.storage().list_issues().unwrap();
    let titles: Vec<&str> = issues.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(issues.len(), 3, "U, C and the bystander, found {titles:?}");
    assert!(titles.contains(&"Concurrent bystander"));
    assert!(
        !issues
            .iter()
            .any(|i| matches!(type_of(i), Some("planning") | Some("breakdown"))),
        "no scaffold node may survive a failed apply"
    );

    let container_issue = executor.storage().load_issue(&container).unwrap();
    assert_eq!(container_issue.dependencies, vec![upstream]);
    assert!(container_issue.gates_required.is_empty());

    // The lock that provided all of this lives with the data, not in a git dir.
    assert!(
        jit_root.join(".repo-write.lock").exists(),
        "the repository write lock must live in the storage root"
    );
}
