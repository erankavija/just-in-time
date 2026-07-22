//! Transaction-boundary tests for graph-template application.
//!
//! Template expansion is finalized into one repository plan and submitted by one
//! [`RepositoryMutationSession::apply`](jit::storage::RepositoryMutationSession::apply)
//! call. These tests inject a failure at that boundary and compare complete
//! captured preimages, rather than observing superseded per-record writes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use jit::declarations::{GateRegistry, GateStage};
use jit::domain::{Event, Issue, Priority};
use jit::gate_presets::{GatePresetDefinition, PresetInfo};
use jit::repository_state::{CaptureBudget, CaptureSpec, RepositoryImage, VirtualPath};
use jit::storage::{
    InMemoryStorage, IssueStore, JsonFileStorage, PathReadError, RepositoryStateStore,
};
use jit::templates::{GraphTemplate, TemplateRegistry};
use jit::CommandExecutor;
use tempfile::TempDir;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];
const SNAPSHOT_BUDGET: CaptureBudget = CaptureBudget {
    max_paths: 1 << 12,
    max_listings: 1,
    max_bytes: 64 * 1024 * 1024,
    max_depth: 8,
};

#[derive(Clone)]
struct PublicationProbeStore<S: IssueStore> {
    inner: S,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    stall: Option<Duration>,
    retryable: bool,
    fail_apply: bool,
    entered: Arc<(Mutex<bool>, Condvar)>,
    attempts: Arc<Mutex<Vec<Vec<Issue>>>>,
    apply_calls: Arc<AtomicUsize>,
    active_sessions: Arc<AtomicUsize>,
    resolve_calls: Arc<AtomicUsize>,
    captures: Arc<Mutex<Vec<RepositoryImage>>>,
    closure_rewrite: Option<(PathBuf, String)>,
    closure_rewritten: Arc<AtomicBool>,
}

impl<S: IssueStore> PublicationProbeStore<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            armed: Arc::new(AtomicBool::new(false)),
            fired: Arc::new(AtomicBool::new(false)),
            stall: None,
            retryable: false,
            fail_apply: true,
            entered: Arc::new((Mutex::new(false), Condvar::new())),
            attempts: Arc::new(Mutex::new(Vec::new())),
            apply_calls: Arc::new(AtomicUsize::new(0)),
            active_sessions: Arc::new(AtomicUsize::new(0)),
            resolve_calls: Arc::new(AtomicUsize::new(0)),
            captures: Arc::new(Mutex::new(Vec::new())),
            closure_rewrite: None,
            closure_rewritten: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with_stall(inner: S, stall: Duration) -> Self {
        Self {
            stall: Some(stall),
            ..Self::new(inner)
        }
    }

    fn retry_once(inner: S) -> Self {
        Self {
            retryable: true,
            ..Self::new(inner)
        }
    }

    fn rewrite_config_once(inner: S, path: PathBuf, content: String) -> Self {
        Self {
            fail_apply: false,
            closure_rewrite: Some((path, content)),
            ..Self::new(inner)
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    fn wait_until_apply(&self) {
        let (lock, ready) = &*self.entered;
        let mut entered = lock.lock().unwrap();
        while !*entered {
            entered = ready.wait(entered).unwrap();
        }
    }

    fn attempts(&self) -> Vec<Vec<Issue>> {
        self.attempts.lock().unwrap().clone()
    }

    fn apply_calls(&self) -> usize {
        self.apply_calls.load(Ordering::SeqCst)
    }

    fn captures(&self) -> Vec<RepositoryImage> {
        self.captures.lock().unwrap().clone()
    }

    fn resolve_calls(&self) -> usize {
        self.resolve_calls.load(Ordering::SeqCst)
    }
}

impl<S: IssueStore> IssueStore for PublicationProbeStore<S> {
    fn acquire_repo_write_lock(&self) -> Result<jit::storage::RepoWriteGuard> {
        self.inner.acquire_repo_write_lock()
    }
    fn init(&self) -> Result<()> {
        self.inner.init()
    }
    fn save_issue(&self, issue: Issue) -> Result<()> {
        self.inner.save_issue(issue)
    }
    fn restore_issue_verbatim(&self, issue: Issue) -> Result<()> {
        self.inner.restore_issue_verbatim(issue)
    }
    fn load_issue(&self, id: &str) -> Result<Issue> {
        self.inner.load_issue(id)
    }
    fn load_issue_or_not_found(&self, id: &str) -> Result<Issue, PathReadError> {
        self.inner.load_issue_or_not_found(id)
    }
    fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
        assert_eq!(
            self.active_sessions.load(Ordering::SeqCst),
            0,
            "ambient issue resolution entered while a repository session was held"
        );
        self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_issue_id(partial_id)
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

struct PublicationProbeSession {
    inner: Box<dyn jit::storage::RepositoryMutationSession>,
    armed: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    stall: Option<Duration>,
    retryable: bool,
    fail_apply: bool,
    entered: Arc<(Mutex<bool>, Condvar)>,
    attempts: Arc<Mutex<Vec<Vec<Issue>>>>,
    apply_calls: Arc<AtomicUsize>,
    active_sessions: Arc<AtomicUsize>,
    captures: Arc<Mutex<Vec<RepositoryImage>>>,
    closure_rewrite: Option<(PathBuf, String)>,
    closure_rewritten: Arc<AtomicBool>,
}

impl Drop for PublicationProbeSession {
    fn drop(&mut self) {
        self.active_sessions.fetch_sub(1, Ordering::SeqCst);
    }
}

impl jit::storage::RepositoryMutationSession for PublicationProbeSession {
    fn layout(&self) -> &jit::repository_state::RepositoryLayout {
        self.inner.layout()
    }

    fn recovery_report(&self) -> &jit::storage::RecoveryDispatchReport {
        self.inner.recovery_report()
    }

    fn capture(
        &mut self,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, jit::storage::RepositoryStateStoreError> {
        let image = self.inner.capture(spec)?;
        self.captures.lock().unwrap().push(image.clone());
        if self.armed.load(Ordering::SeqCst)
            && self
                .closure_rewritten
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            if let Some((path, content)) = &self.closure_rewrite {
                std::fs::write(path, content).unwrap();
            }
        }
        Ok(image)
    }

    fn apply(
        &mut self,
        plan: &jit::repository_state::MaterializationPlan,
    ) -> Result<jit::storage::RepositoryApplyOutcome, jit::storage::RepositoryStateStoreError> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        let mut issues = plan
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
        issues.sort_by(|left, right| left.id.cmp(&right.id));
        if self.armed.load(Ordering::SeqCst) {
            self.attempts.lock().unwrap().push(issues);
        }
        if self.fail_apply
            && self.armed.load(Ordering::SeqCst)
            && self
                .fired
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            let (lock, ready) = &*self.entered;
            *lock.lock().unwrap() = true;
            ready.notify_all();
            if let Some(stall) = self.stall {
                std::thread::sleep(stall);
            }
            return if self.retryable {
                Err(jit::storage::RepositoryStateStoreError::RetryableConflict {
                    path: "injected template publication conflict".to_string(),
                })
            } else {
                Err(jit::storage::RepositoryStateStoreError::UnsafeTarget(
                    "injected template publication failure".to_string(),
                ))
            };
        }
        self.inner.apply(plan)
    }
}

impl<S: IssueStore + RepositoryStateStore> RepositoryStateStore for PublicationProbeStore<S> {
    fn open_mutation_session(
        &self,
        layout: jit::repository_state::RepositoryLayout,
    ) -> Result<
        Box<dyn jit::storage::RepositoryMutationSession>,
        jit::storage::RepositoryStateStoreError,
    > {
        let inner = self.inner.open_mutation_session(layout)?;
        self.active_sessions.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(PublicationProbeSession {
            inner,
            armed: Arc::clone(&self.armed),
            fired: Arc::clone(&self.fired),
            stall: self.stall,
            retryable: self.retryable,
            fail_apply: self.fail_apply,
            entered: Arc::clone(&self.entered),
            attempts: Arc::clone(&self.attempts),
            apply_calls: Arc::clone(&self.apply_calls),
            active_sessions: Arc::clone(&self.active_sessions),
            captures: Arc::clone(&self.captures),
            closure_rewrite: self.closure_rewrite.clone(),
            closure_rewritten: Arc::clone(&self.closure_rewritten),
        }))
    }
}

fn plan_template() -> GraphTemplate {
    let toml = r#"
[[template]]
name = "plan"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
  gates = ["repo-validate"]
  [[template.nodes]]
  role = "planning"
  type = "planning"
  doc = "dev/active/{container.id}-plan.md"
  description = "Planning node for {container.title}."
  [[template.nodes]]
  role = "breakdown"
  type = "breakdown"
  labels = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on = ["planning"]
  [[template.anchor_edges]]
  from = "container"
  to = "breakdown"
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

fn bindings(container: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("container".to_string(), container.to_string())])
}

fn fixture<S: IssueStore + RepositoryStateStore>(
    store: PublicationProbeStore<S>,
) -> (CommandExecutor<PublicationProbeStore<S>>, String, String) {
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
    (executor, container, upstream)
}

fn repository_image<S: IssueStore + RepositoryStateStore>(store: &S) -> RepositoryImage {
    let layout =
        jit::storage::discover_repository_layout(store.root().parent().unwrap(), store.root())
            .unwrap();
    let mut paths = vec![
        VirtualPath::data("index.json").unwrap(),
        VirtualPath::data("events.jsonl").unwrap(),
        VirtualPath::data("gates.toml").unwrap(),
        VirtualPath::data("issues").unwrap(),
    ];
    paths.extend(
        store
            .list_issues()
            .unwrap()
            .into_iter()
            .map(|issue| VirtualPath::data(format!("issues/{}.json", issue.id)).unwrap()),
    );
    let mut spec = CaptureSpec::phase_one(paths, SNAPSHOT_BUDGET).unwrap();
    spec.discover_listing(VirtualPath::data("issues").unwrap())
        .unwrap();
    let mut session = store.open_mutation_session(layout).unwrap();
    session.capture(spec).unwrap()
}

fn assert_publication_failure_is_atomic<S: IssueStore + RepositoryStateStore>(
    store: PublicationProbeStore<S>,
) {
    let (executor, container, _) = fixture(store.clone());
    let before = repository_image(&store);
    store.arm();
    let error = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();
    assert!(store.fired());
    assert!(error.to_string().contains("publication failure"), "{error}");
    assert_eq!(repository_image(&store), before);

    // The one-shot fault is exhausted: a fresh recovered session can publish the
    // complete scaffold, proving the failure left no blocking transaction state.
    let result = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap()
        .0;
    assert_eq!(result.created_node_ids_by_role.len(), 2);
}

#[test]
fn test_failed_publication_preserves_memory_preimage_and_retry_succeeds() {
    assert_publication_failure_is_atomic(PublicationProbeStore::new(InMemoryStorage::new()));
}

#[test]
fn test_failed_publication_preserves_file_preimage_and_retry_succeeds() {
    let temp = TempDir::new().unwrap();
    assert_publication_failure_is_atomic(PublicationProbeStore::new(JsonFileStorage::new(
        temp.path().join(".jit"),
    )));
}

#[test]
fn test_retryable_conflict_reuses_created_ids_and_timestamp() {
    let store = PublicationProbeStore::retry_once(InMemoryStorage::new());
    let (executor, container, _) = fixture(store.clone());
    store.arm();
    let result = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap()
        .0;
    let attempts = store.attempts();
    assert_eq!(attempts.len(), 2, "one conflict and one successful apply");
    assert_eq!(attempts[0], attempts[1]);
    let created = result
        .created_node_ids_by_role
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let attempted = attempts[0]
        .iter()
        .filter(|issue| {
            issue
                .labels
                .iter()
                .any(|label| matches!(label.as_str(), "type:planning" | "type:breakdown"))
        })
        .map(|issue| issue.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(attempted, created);
}

#[test]
fn test_concurrent_writer_observes_failed_apply_preimage_then_publishes() {
    let temp = TempDir::new().unwrap();
    let jit_root = temp.path().join(".jit");
    let stall = Duration::from_millis(500);
    let store = PublicationProbeStore::with_stall(JsonFileStorage::new(&jit_root), stall);
    let (executor, container, upstream) = fixture(store.clone());
    let original_container = executor.storage().load_issue(&container).unwrap();
    store.arm();

    let writer = {
        let probe = store.clone();
        let jit_root = jit_root.clone();
        std::thread::spawn(move || {
            probe.wait_until_apply();
            let storage = JsonFileStorage::new(&jit_root);
            let layout = jit::storage::discover_repository_layout(
                jit_root.parent().unwrap(),
                storage.root(),
            )
            .unwrap();
            let executor = CommandExecutor::new(storage).with_layout(layout);
            let started = Instant::now();
            let id = executor
                .create_issue(
                    "Concurrent bystander".to_string(),
                    "Independent write".to_string(),
                    Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .0;
            (id, started.elapsed())
        })
    };

    let error = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap_err();
    let (bystander, elapsed) = writer.join().unwrap();
    assert!(error.to_string().contains("publication failure"), "{error}");
    assert!(elapsed >= stall / 2, "writer waited only {elapsed:?}");
    assert_eq!(
        executor.storage().load_issue(&container).unwrap(),
        original_container
    );
    assert_eq!(
        executor
            .storage()
            .load_issue(&container)
            .unwrap()
            .dependencies,
        vec![upstream]
    );
    assert_eq!(
        executor.storage().load_issue(&bystander).unwrap().title,
        "Concurrent bystander"
    );
    assert!(!executor
        .storage()
        .list_issues()
        .unwrap()
        .iter()
        .any(|issue| {
            issue
                .labels
                .iter()
                .any(|label| matches!(label.as_str(), "type:planning" | "type:breakdown"))
        }));
}

#[test]
fn test_lease_preflight_never_resolves_an_issue_under_repository_session() {
    let store = PublicationProbeStore::new(InMemoryStorage::new());
    let (executor, container, _) = fixture(store.clone());
    store.inner.add_repo_file(
        ".jit/config.toml",
        "[worktree]\nenforce_leases = \"warn\"\n",
    );
    let resolves_before = store.resolve_calls();

    let (_, warnings) = executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap();

    assert!(
        !warnings.is_empty(),
        "warn mode must exercise lease preflight"
    );
    assert_eq!(store.resolve_calls(), resolves_before);
    assert_eq!(store.active_sessions.load(Ordering::SeqCst), 0);
}

#[test]
fn test_phase_two_closure_change_retries_with_same_template_node_count() {
    let temp = TempDir::new().unwrap();
    let jit_root = temp.path().join(".jit");
    let changed_config = r#"
[item_kinds.invariant]
scope = "project"
source = { toml = ".jit/invariants.toml", table = "invariants", id-field = "id", text-field = "statement" }
source-of-truth = "registry-first"

[projection.empty]
kind = "invariant"
mode = "region"
target = "docs/closure.md"
"#;
    let store = PublicationProbeStore::rewrite_config_once(
        JsonFileStorage::new(&jit_root),
        jit_root.join("config.toml"),
        changed_config.to_string(),
    );
    let (executor, container, _) = fixture(store.clone());
    let calls_before = store.apply_calls();
    store.arm();

    executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap();

    let target = VirtualPath::worktree("docs/closure.md").unwrap();
    let captures = store.captures();
    assert!(store.closure_rewritten.load(Ordering::SeqCst));
    assert!(captures.iter().any(|image| {
        let config = VirtualPath::data("config.toml").unwrap();
        image.capture_spec().paths().any(|path| path == &config)
            && image
                .file_bytes(&config)
                .unwrap()
                .is_some_and(|bytes| bytes == changed_config.as_bytes())
            && !image.capture_spec().paths().any(|path| path == &target)
    }));
    assert!(captures
        .iter()
        .any(|image| image.capture_spec().paths().any(|path| path == &target)));
    assert_eq!(store.apply_calls(), calls_before + 1);
}

fn assert_planning_document_capture(existing: bool) {
    let store = PublicationProbeStore::new(InMemoryStorage::new());
    let (executor, container, _) = fixture(store.clone());
    let target = format!("dev/active/{container}-plan.md");
    if existing {
        store.inner.add_repo_file(&target, "existing plan\n");
    }

    executor
        .apply_template_with(&plan_template(), &container, &bindings(&container), false)
        .unwrap();

    let target = VirtualPath::worktree(&target).unwrap();
    let final_image = store
        .captures()
        .into_iter()
        .rev()
        .find(|image| image.capture_spec().paths().any(|path| path == &target))
        .expect("planning document target must enter the held-session closure");
    assert!(final_image
        .capture_spec()
        .paths()
        .any(|path| path == &VirtualPath::worktree("dev").unwrap()));
    assert!(final_image
        .capture_spec()
        .paths()
        .any(|path| path == &VirtualPath::worktree("dev/active").unwrap()));
    assert_eq!(final_image.file_bytes(&target).unwrap().is_some(), existing);
    assert!(final_image.pinned_evidence().is_empty());
    assert!(final_image.linked_worktree_evidence().is_empty());
}

#[test]
fn test_planning_document_absence_and_parents_are_captured_without_borrowing() {
    assert_planning_document_capture(false);
}

#[test]
fn test_existing_planning_document_and_parents_are_captured_without_borrowing() {
    assert_planning_document_capture(true);
}

#[test]
fn test_unchanged_force_is_exact_noop_without_publication_or_created_id_paths() {
    let store = PublicationProbeStore::new(InMemoryStorage::new());
    let (executor, container, _) = fixture(store.clone());
    let template = plan_template();
    let applied = executor
        .apply_template_with(&template, &container, &bindings(&container), false)
        .unwrap()
        .0;
    let calls_before = store.apply_calls();
    let captures_before = store.captures().len();
    let events_before = store.read_repo_file(".jit/events.jsonl").unwrap();
    let mut issues_before = store.list_issues().unwrap();
    issues_before.sort_by(|left, right| left.id.cmp(&right.id));

    let refreshed = executor
        .apply_template_with(&template, &container, &bindings(&container), true)
        .unwrap()
        .0;

    let mut issues_after = store.list_issues().unwrap();
    issues_after.sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(
        refreshed.created_node_ids_by_role,
        applied.created_node_ids_by_role
    );
    assert_eq!(store.apply_calls(), calls_before);
    assert_eq!(
        store.read_repo_file(".jit/events.jsonl").unwrap(),
        events_before
    );
    assert_eq!(issues_after, issues_before);

    let existing = issues_after
        .iter()
        .map(|issue| format!("issues/{}.json", issue.id))
        .collect::<std::collections::BTreeSet<_>>();
    for image in store.captures().into_iter().skip(captures_before) {
        for path in image.capture_spec().paths() {
            let relative = path.relative().as_path().to_string_lossy();
            if relative.starts_with("issues/") && relative.ends_with(".json") {
                assert!(
                    existing.contains(relative.as_ref()),
                    "unchanged force captured a newly allocated issue path: {relative}"
                );
            }
        }
    }
}
