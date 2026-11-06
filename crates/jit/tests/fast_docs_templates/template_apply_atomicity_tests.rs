//! Publication-boundary tests for graph-template application.
//!
//! Template expansion is finalized into one repository plan and submitted by one
//! [`RepositoryMutationSession::apply`](jit::storage::RepositoryMutationSession::apply)
//! call. These tests inject a failure at that boundary and compare complete
//! captured preimages, rather than observing superseded per-record writes.

use jit::commands::test_helpers::ExecutorTestAccess;
use jit::test_utils::add_reducing_edge;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use jit::commands::test_helpers::{memory_executor_wrapping, MemoryDeclarations};
use jit::declarations::{GateChecker, GateMode, GateRegistry, GateStage};
use jit::domain::identity::IssueId;
use jit::domain::{Event, Issue, Priority};
use jit::repository_state::{CaptureBudget, CaptureSpec, RepositoryImage, VirtualPath};
use jit::storage::claim_coordinator::ClaimCoordinator;
use jit::storage::{
    FileLocker, InMemoryStorage, IssueStore, JsonFileStorage, PathReadError, RepositoryStateStore,
};
use jit::templates::{GraphTemplate, TemplateRegistry};
use jit::test_taxonomy::{test_taxonomy, TestTaxonomy};
use jit::CommandExecutor;

const SNAPSHOT_BUDGET: CaptureBudget = CaptureBudget {
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
    session_opens: Arc<AtomicUsize>,
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
            session_opens: Arc::new(AtomicUsize::new(0)),
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

    fn session_opens(&self) -> usize {
        self.session_opens.load(Ordering::SeqCst)
    }
}

impl<S: IssueStore> IssueStore for PublicationProbeStore<S> {
    fn configure_repository_layout(
        &self,
        layout: &jit::repository_state::RepositoryLayout,
    ) -> std::result::Result<(), jit::storage::RepositoryStateStoreError> {
        self.inner.configure_repository_layout(layout)
    }

    fn acquire_repo_write_lock(&self) -> Result<jit::storage::RepoWriteGuard> {
        self.inner.acquire_repo_write_lock()
    }
    fn load_issue(&self, id: &jit::domain::identity::IssueId) -> Result<Issue> {
        self.inner.load_issue(id)
    }
    fn load_issue_or_not_found(
        &self,
        id: &jit::domain::identity::IssueId,
    ) -> Result<Issue, PathReadError> {
        self.inner.load_issue_or_not_found(id)
    }
    fn resolve_issue_id(
        &self,
        prefix: &jit::domain::identity::IssueIdPrefix,
    ) -> Result<jit::domain::identity::IssueId> {
        assert_eq!(
            self.active_sessions.load(Ordering::SeqCst),
            0,
            "ambient issue resolution entered while a repository session was held"
        );
        self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_issue_id(prefix)
    }
    fn list_issues(&self) -> Result<Vec<Issue>> {
        self.inner.list_issues()
    }
    fn load_gate_registry(&self) -> Result<GateRegistry> {
        self.inner.load_gate_registry()
    }
    fn read_events(&self) -> Result<Vec<Event>> {
        self.inner.read_events()
    }
    fn load_gate_run_result(
        &self,
        issue_id: &jit::domain::identity::IssueId,
        run_id: &jit::domain::identity::GateRunId,
    ) -> Result<jit::domain::GateRunResult> {
        self.inner.load_gate_run_result(issue_id, run_id)
    }
    fn list_gate_runs_for_issue(
        &self,
        issue_id: &jit::domain::identity::IssueId,
    ) -> Result<Vec<jit::domain::GateRunResult>> {
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
}

impl Drop for PublicationProbeSession {
    fn drop(&mut self) {
        self.active_sessions.fetch_sub(1, Ordering::SeqCst);
    }
}

impl jit::storage::RepositoryReadSession for PublicationProbeSession {
    fn layout(&self) -> &jit::repository_state::RepositoryLayout {
        self.inner.layout()
    }

    fn capture(
        &mut self,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, jit::storage::RepositoryStateStoreError> {
        let image = self.inner.capture(spec)?;
        self.captures.lock().unwrap().push(image.clone());
        Ok(image)
    }

    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, jit::storage::RepositoryStateStoreError> {
        let image = self.inner.extend(image, spec)?;
        self.captures.lock().unwrap().push(image.clone());
        Ok(image)
    }

    fn read_history_log(
        &mut self,
    ) -> Result<Option<Vec<u8>>, jit::storage::RepositoryStateStoreError> {
        self.inner.read_history_log()
    }
}

impl jit::storage::RepositoryMutationSession for PublicationProbeSession {
    fn recovery_report(&self) -> &jit::storage::RecoveryDispatchReport {
        self.inner.recovery_report()
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
        let inner = self.inner.open_mutation_session(layout)?;
        self.active_sessions.fetch_add(1, Ordering::SeqCst);
        self.session_opens.fetch_add(1, Ordering::SeqCst);
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
        }))
    }

    fn open_read_session(
        &self,
        layout: jit::repository_state::RepositoryLayout,
    ) -> Result<Box<dyn jit::storage::OptimisticReadSession>, jit::storage::RepositoryStateStoreError>
    {
        self.inner.open_read_session(layout)
    }
}

fn plan_template(taxonomy: &TestTaxonomy) -> GraphTemplate {
    let container_type = taxonomy.type_at_level(2);
    let planning_type = taxonomy.type_at_level(3);
    let breakdown_type = taxonomy.type_at_level(4);
    let membership_namespace = taxonomy
        .label_associations
        .get(container_type)
        .expect("the test taxonomy associates the container type with a namespace");
    let hierarchy_types = taxonomy
        .hierarchy
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let toml = format!(
        r#"
[[template]]
name = "plan"
applies_to = ["{container_type}"]
  [[template.anchors]]
  name = "container"
  gates = ["repo-validate"]
  [[template.nodes]]
  role = "planning"
  type = "{planning_type}"
  doc = "dev/active/{{container.id}}-plan.md"
  description = "Planning node for {{container.title}}."
  [[template.nodes]]
  role = "breakdown"
  type = "{breakdown_type}"
  labels = ["{membership_namespace}:{{container.short_id}}"]
  description = "Breakdown node for {{container.title}}."
  depends_on = ["planning"]
  [[template.anchor_edges]]
  from = "container"
  to = "breakdown"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#
    );
    TemplateRegistry::from_toml_str(&toml, &hierarchy_types)
        .unwrap()
        .get("plan")
        .unwrap()
        .clone()
}

fn bindings(container: &IssueId) -> BTreeMap<String, IssueId> {
    BTreeMap::from([("container".to_string(), container.clone())])
}

fn fixture<S: IssueStore + RepositoryStateStore>(
    executor: CommandExecutor<PublicationProbeStore<S>>,
    taxonomy: &TestTaxonomy,
) -> (CommandExecutor<PublicationProbeStore<S>>, IssueId, IssueId) {
    assert!(executor
        .storage()
        .read_repo_file(".jit/config.toml")
        .unwrap()
        .is_some());
    executor
        .gates()
        .define_gate(
            &"repo-validate".parse().unwrap(),
            "Repo Validate".to_string(),
            "Whole-repository validation must pass".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            Some(GateChecker::RepositoryValidation),
            100,
        )
        .unwrap();
    let upstream = executor
        .issues()
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
        .unwrap()
        .issue
        .id;
    let container = executor
        .issues()
        .create_issue(
            "Auth epic".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec![
                format!("type:{}", taxonomy.type_at_level(2)),
                format!(
                    "{}:auth",
                    taxonomy
                        .label_associations
                        .get(taxonomy.type_at_level(2))
                        .expect("the test taxonomy associates the container type with a namespace")
                ),
            ],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;
    add_reducing_edge(&executor, &container, &upstream).unwrap();
    (executor, container, upstream)
}

fn repository_image<S: IssueStore + RepositoryStateStore>(store: &S) -> RepositoryImage {
    let layout =
        jit::storage::discover_repository_layout(store.root().parent().unwrap(), store.root())
            .unwrap();
    let mut paths = vec![
        VirtualPath::data("index.json").unwrap(),
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
    let mut spec = CaptureSpec::new(paths, SNAPSHOT_BUDGET).unwrap();
    spec.discover_tail(VirtualPath::EVENTS).unwrap();
    spec.discover_listing(VirtualPath::data("issues").unwrap())
        .unwrap();
    let mut session = store.open_mutation_session(layout).unwrap();
    session.capture(spec).unwrap()
}

fn assert_publication_failure_is_atomic<S: IssueStore + RepositoryStateStore>(
    executor: CommandExecutor<PublicationProbeStore<S>>,
    taxonomy: &TestTaxonomy,
) {
    let store = executor.storage().clone();
    let (executor, container, _) = fixture(executor, taxonomy);
    let before = repository_image(&store);
    store.arm();
    let error = executor
        .templates()
        .apply_template_with(
            &plan_template(taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap_err();
    assert!(store.fired());
    assert!(error.to_string().contains("publication failure"), "{error}");
    assert_eq!(repository_image(&store), before);

    // The one-shot fault is exhausted: a fresh mutation session can publish the
    // complete scaffold, proving the failure left no blocking redo record.
    let result = executor
        .templates()
        .apply_template_with(
            &plan_template(taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap()
        .0;
    assert_eq!(result.created_node_ids_by_role.len(), 2);
}

#[test]
fn test_failed_publication_preserves_memory_preimage_and_retry_succeeds() {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::new,
    );
    assert_publication_failure_is_atomic(executor, &taxonomy);
}

#[test]
fn test_failed_publication_preserves_file_preimage_and_retry_succeeds() {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy().unwrap();
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
    let executor = CommandExecutor::new(PublicationProbeStore::new(storage))
        .with_layout(layout)
        .unwrap();
    assert_publication_failure_is_atomic(executor, &taxonomy);
}

#[test]
fn test_retryable_conflict_reuses_created_ids_and_timestamp() {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::retry_once,
    );
    let store = executor.storage().clone();
    let (executor, container, _) = fixture(executor, &taxonomy);
    store.arm();
    let result = executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap()
        .0;
    let attempts = store.attempts();
    assert_eq!(attempts.len(), 2, "one conflict and one successful apply");
    assert_eq!(attempts[0], attempts[1]);
    let planning_label = format!("type:{}", taxonomy.type_at_level(3));
    let container_issue = executor.storage().load_issue(&container).unwrap();
    let breakdown_label = format!(
        "{}:{}",
        taxonomy
            .label_associations
            .get(taxonomy.type_at_level(2))
            .expect("the test taxonomy associates the container type with a namespace"),
        container_issue.short_id()
    );
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
                .any(|label| label == &planning_label || label == &breakdown_label)
        })
        .map(|issue| issue.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(attempted, created);
}

#[test]
fn test_concurrent_writer_observes_failed_apply_preimage_then_publishes() {
    let stall = Duration::from_millis(500);
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy().unwrap();
    let jit_root = temp.path().join(".jit");
    let store = PublicationProbeStore::with_stall(storage, stall);
    let layout = jit::storage::discover_repository_layout(temp.path(), store.root()).unwrap();
    let executor = CommandExecutor::new(store.clone())
        .with_layout(layout)
        .unwrap();
    let (executor, container, upstream) = fixture(executor, &taxonomy);
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
            let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();
            let started = Instant::now();
            let id = executor
                .issues()
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
                .issue
                .id;
            (id, started.elapsed())
        })
    };

    let error = executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
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
    let planning_label = format!("type:{}", taxonomy.type_at_level(3));
    let container_issue = executor.storage().load_issue(&container).unwrap();
    let breakdown_label = format!(
        "{}:{}",
        taxonomy
            .label_associations
            .get(taxonomy.type_at_level(2))
            .expect("the test taxonomy associates the container type with a namespace"),
        container_issue.short_id()
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
                .any(|label| label == &planning_label || label == &breakdown_label)
        }));
}

/// A template fixture in a Git checkout enforcing `mode`, whose container
/// another agent leases.
fn leased_container_fixture(
    mode: &str,
) -> (
    CommandExecutor<PublicationProbeStore<InMemoryStorage>>,
    PublicationProbeStore<InMemoryStorage>,
    IssueId,
) {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::new,
    );
    let store = executor.storage().clone();
    let (executor, container, _) = fixture(executor, &taxonomy);
    store.inner.add_data_file(
        "config.toml",
        &format!(
            "[worktree]\nenforce_leases = \"{mode}\"\n\n{}",
            taxonomy.config_fragment()
        ),
    );
    let root = store
        .inner
        .repository_layout()
        .worktree_root()
        .to_path_buf();
    jit::test_cli::init_git_repository_with_commit(&root).unwrap();
    let paths = jit::test_utils::primary_checkout_paths(&root);
    let holder = ClaimCoordinator::new(
        paths.clone(),
        FileLocker::new(Duration::from_secs(5)),
        "wt:holder".parse().unwrap(),
        "agent:holder".parse().unwrap(),
    );
    holder.init().unwrap();
    holder
        .acquire_claim_synchronized(
            &container,
            600,
            Some("test lease"),
            jit::storage::claim_coordinator::ClaimAcquireLimits::new(100, 100),
            |_| Ok(()),
        )
        .unwrap();
    (executor.with_worktree_paths(paths), store, container)
}

#[test]
fn test_lease_check_never_resolves_an_issue_under_repository_session() {
    let taxonomy = test_taxonomy();
    let (executor, store, container) = leased_container_fixture("warn");
    let resolves_before = store.resolve_calls();

    let (_, warnings) = executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap();

    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("agent:holder")),
        "the lease check must observe the lease: {warnings:?}"
    );
    assert_eq!(store.resolve_calls(), resolves_before);
    assert_eq!(store.active_sessions.load(Ordering::SeqCst), 0);
}

#[test]
fn test_apply_template_with_opens_one_session_per_successful_attempt() {
    let taxonomy = test_taxonomy();
    let (executor, store, container) = leased_container_fixture("warn");
    let opens_before = store.session_opens();

    executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap();

    assert_eq!(store.session_opens() - opens_before, 1);
}

#[test]
fn test_apply_template_with_strict_refuses_an_edited_issue_another_agent_leases() {
    let taxonomy = test_taxonomy();
    let (executor, store, container) = leased_container_fixture("strict");
    let before = repository_image(&store);
    let applies_before = store.apply_calls();

    let error = executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap_err();

    let refusal = error
        .downcast_ref::<jit::errors::ForeignLeaseError>()
        .expect("the refusal is a ForeignLeaseError");
    assert!(refusal
        .leases()
        .iter()
        .any(|lease| lease.issue_id == container));
    assert_eq!(store.apply_calls(), applies_before);
    assert_eq!(repository_image(&store), before);
}

/// A closure changed before the round reads the configuration is discovered
/// in that same round: the image that holds the new configuration also holds
/// the projection target it declares.
#[test]
fn test_template_round_discovers_a_changed_closure() {
    let changed_config = r#"
[item_kinds.invariant]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = ["enforces"]
scope = "project"
source = { toml = ".jit/invariants.toml", table = "invariants", id-field = "id", text-field = "statement" }
source-of-truth = "registry-first"

[projection.empty]
kind = "invariant"
mode = "region"
target = "docs/closure.md"
"#;
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy().unwrap();
    let jit_root = temp.path().join(".jit");
    let store = PublicationProbeStore::rewrite_config_once(
        storage,
        jit_root.join("config.toml"),
        changed_config.to_string(),
    );
    let layout = jit::storage::discover_repository_layout(temp.path(), store.root()).unwrap();
    let executor = CommandExecutor::new(store.clone())
        .with_layout(layout)
        .unwrap();
    let (executor, container, _) = fixture(executor, &taxonomy);
    let calls_before = store.apply_calls();
    store.arm();

    executor
        .templates()
        .apply_template_with(
            &plan_template(&taxonomy),
            &container,
            &bindings(&container),
            false,
        )
        .unwrap();

    let target = VirtualPath::worktree("docs/closure.md").unwrap();
    let config = VirtualPath::data("config.toml").unwrap();
    assert!(store.closure_rewritten.load(Ordering::SeqCst));
    let published = store.captures().pop().expect("the round captured");
    assert_eq!(
        published.file_bytes(&config).unwrap(),
        Some(changed_config.as_bytes())
    );
    assert!(published.capture_spec().read_mode(&target).is_some());
    assert_eq!(store.apply_calls(), calls_before + 1);
}

fn assert_planning_document_capture(
    executor: CommandExecutor<PublicationProbeStore<InMemoryStorage>>,
    taxonomy: &TestTaxonomy,
    existing: bool,
) {
    let store = executor.storage().clone();
    let (executor, container, _) = fixture(executor, taxonomy);
    let target = format!("dev/active/{container}-plan.md");
    if existing {
        store.inner.add_worktree_file(&target, "existing plan\n");
    }

    executor
        .templates()
        .apply_template_with(
            &plan_template(taxonomy),
            &container,
            &bindings(&container),
            false,
        )
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
}

#[test]
fn test_planning_document_absence_and_parents_are_captured_without_borrowing() {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::new,
    );
    assert_planning_document_capture(executor, &taxonomy, false);
}

#[test]
fn test_existing_planning_document_and_parents_are_captured_without_borrowing() {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::new,
    );
    assert_planning_document_capture(executor, &taxonomy, true);
}

#[test]
fn test_unchanged_force_is_exact_noop_without_publication_or_created_id_paths() {
    let taxonomy = test_taxonomy();
    let executor = memory_executor_wrapping(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&taxonomy.config_fragment()),
            ..MemoryDeclarations::default()
        },
        PublicationProbeStore::new,
    );
    let store = executor.storage().clone();
    let (executor, container, _) = fixture(executor, &taxonomy);
    let template = plan_template(&taxonomy);
    let applied = executor
        .templates()
        .apply_template_with(&template, &container, &bindings(&container), false)
        .unwrap()
        .0;
    let calls_before = store.apply_calls();
    let captures_before = store.captures().len();
    let events_before = store.read_repo_file(".jit/events.jsonl").unwrap();
    let mut issues_before = store.list_issues().unwrap();
    issues_before.sort_by(|left, right| left.id.cmp(&right.id));

    let refreshed = executor
        .templates()
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
