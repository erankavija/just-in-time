//! The [`IssueStore`] contract, written once and run against every backend.
//!
//! Each case is a generic function over `Backend`, whose one constructor
//! builds a store over a worktree holding a shared seed; `conformance_suite!`
//! runs every case against every listed backend, so a backend joins the suite
//! by implementing `Backend` and naming itself there. A backend's own tests
//! state only what that backend adds (`@/inv/orthogonal-tests`).

use crate::declarations::{serialize_gate_registry, GateRegistry, GateStage};
use crate::domain::artifact_execution::{ArchiveEvidenceRecord, ArchiveExecution};
use crate::domain::artifact_plan::PlanTarget;
use crate::domain::identity::{GateRunId, IssueId, IssueIdPrefix};
use crate::domain::{Event, EventBody, GateRunResult, GateStatus, GateVerdictOrigin, Issue};
use crate::gate_presets::{GatePresetDefinition, GateTemplate};
use crate::repository_state::{
    serialize_gate_run, serialize_issue, CaptureBudget, CaptureSpec, ExpectedPreimage, FileMode,
    MaterializationPlan, MutationContext, RepositoryAction, RepositoryActionOwner, RepositoryDelta,
    RepositoryIndex, RepositoryLayout, RepositoryRootClass, VirtualPath,
};
use crate::storage::{
    discover_repository_layout, AmbiguousIdError, GateRunNotFoundError, InMemoryStorage,
    IssueNotFoundError, IssueStore, JsonFileStorage, PathReadError, RepositoryStateStore,
    RepositoryStateStoreError,
};
use crate::test_harness::git::GitRepository;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;
use tempfile::TempDir;

/// Files by repository path: the one description every backend is seeded from.
type Seed = [(VirtualPath, String)];

/// What the suite needs of a backend: a store over `worktree` whose repository
/// holds exactly `seed`, and whose data root is absent when `seed` names no
/// data file.
trait Backend: IssueStore + RepositoryStateStore {
    fn over(worktree: &Path, seed: &Seed) -> Self;

    /// Replace `path` with exactly `content`.
    fn write(&self, worktree: &Path, path: &VirtualPath, content: &[u8]);

    /// Record the repository's current state as a commit and return its id.
    fn commit(&self, worktree: &Path) -> String;

    /// Place a symlink at `path` naming `target`.
    fn link(&self, worktree: &Path, path: &VirtualPath, target: &VirtualPath);

    /// Place an object of a kind no repository path holds at `path`.
    fn plant_special(&self, worktree: &Path, path: &VirtualPath);
}

fn physical_path(worktree: &Path, path: &VirtualPath) -> std::path::PathBuf {
    match path.root_class() {
        RepositoryRootClass::Data => worktree.join(".jit"),
        RepositoryRootClass::Worktree => worktree.to_path_buf(),
    }
    .join(path.relative().as_path())
}

fn write_json_file(worktree: &Path, path: &VirtualPath, content: &[u8]) {
    let target = physical_path(worktree, path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, content).unwrap();
}

impl Backend for JsonFileStorage {
    fn over(worktree: &Path, seed: &Seed) -> Self {
        let data = worktree.join(".jit");
        seed.iter()
            .for_each(|(path, content)| write_json_file(worktree, path, content.as_bytes()));
        let store = JsonFileStorage::new(&data);
        store
            .configure_repository_layout(&discover_repository_layout(worktree, &data).unwrap())
            .unwrap();
        store
    }

    fn write(&self, worktree: &Path, path: &VirtualPath, content: &[u8]) {
        write_json_file(worktree, path, content);
    }

    fn commit(&self, worktree: &Path) -> String {
        GitRepository::init(worktree).commit("conformance")
    }

    fn link(&self, worktree: &Path, path: &VirtualPath, target: &VirtualPath) {
        let (link, target) = (
            physical_path(worktree, path),
            physical_path(worktree, target),
        );
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, link).unwrap();
        }
    }

    fn plant_special(&self, worktree: &Path, path: &VirtualPath) {
        let special = physical_path(worktree, path);
        std::fs::create_dir_all(special.parent().unwrap()).unwrap();
        #[cfg(unix)]
        nix::unistd::mkfifo(&special, nix::sys::stat::Mode::S_IRWXU).unwrap();
        #[cfg(not(unix))]
        unimplemented!("no special object to plant at {}", special.display());
    }
}

impl Backend for InMemoryStorage {
    fn over(worktree: &Path, seed: &Seed) -> Self {
        let store = InMemoryStorage::rooted_at(worktree);
        seed.iter().for_each(|(path, content)| {
            store.add_repository_bytes(path.clone(), content.as_bytes());
        });
        store
    }

    fn write(&self, _worktree: &Path, path: &VirtualPath, content: &[u8]) {
        self.add_repository_bytes(path.clone(), content);
    }

    fn commit(&self, _worktree: &Path) -> String {
        self.record_commit()
    }

    fn link(&self, _worktree: &Path, path: &VirtualPath, target: &VirtualPath) {
        self.add_repository_symlink(path.clone(), target);
    }

    fn plant_special(&self, _worktree: &Path, path: &VirtualPath) {
        self.add_repository_special(path.clone());
    }
}

/// A store and the worktree it owns, removed when the repository drops.
struct Repository<B> {
    worktree: TempDir,
    store: B,
}

impl<B: Backend> Repository<B> {
    fn seeded(seed: &Seed) -> Self {
        let worktree = TempDir::new().unwrap();
        let store = B::over(worktree.path(), seed);
        Self { worktree, store }
    }

    fn empty() -> Self {
        Self::seeded(&[])
    }

    fn write(&self, path: &VirtualPath, content: &[u8]) {
        self.store.write(self.worktree.path(), path, content);
    }

    fn commit(&self) -> String {
        self.store.commit(self.worktree.path())
    }

    fn link(&self, path: &VirtualPath, target: &VirtualPath) {
        self.store.link(self.worktree.path(), path, target);
    }

    fn plant_special(&self, path: &VirtualPath) {
        self.store.plant_special(self.worktree.path(), path);
    }

    fn layout(&self) -> RepositoryLayout {
        discover_repository_layout(self.worktree.path(), self.worktree.path().join(".jit")).unwrap()
    }

    /// Publish `issues` through one mutation session over an absent data root.
    fn publish_issues(&self, issues: &[Issue]) {
        let layout = self.layout();
        let issue_paths = issues
            .iter()
            .map(|issue| VirtualPath::issue_record(&issue.id))
            .collect::<Vec<_>>();
        let roots = [
            VirtualPath::data("").unwrap(),
            VirtualPath::INDEX,
            VirtualPath::ISSUES,
        ];
        let budget = CaptureBudget {
            max_listings: 8,
            max_bytes: 1024 * 1024,
            max_depth: 8,
        };
        let mut session = self.store.open_mutation_session(layout.clone()).unwrap();
        let image = session
            .capture(CaptureSpec::new(roots.iter().chain(&issue_paths).cloned(), budget).unwrap())
            .unwrap();
        let [data, index, issues_dir] = roots;
        let actions = [
            RepositoryAction::create_directory(
                data,
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                index,
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
                index_bytes(),
                FileMode::Regular,
            ),
            RepositoryAction::create_directory(
                issues_dir,
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
            ),
        ]
        .into_iter()
        .chain(issues.iter().zip(issue_paths).map(|(issue, path)| {
            RepositoryAction::write_file(
                path,
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
                serialize_issue(issue).unwrap(),
                FileMode::Regular,
            )
        }))
        .collect();
        let delta = RepositoryDelta::new(&layout, actions).unwrap();
        session
            .apply(&MaterializationPlan::new(&image, delta, &MutationContext::preview()).unwrap())
            .unwrap();
    }
}

fn index_bytes() -> Vec<u8> {
    RepositoryIndex::default().to_pretty_bytes().unwrap()
}

/// A data root holding only an empty index.
fn initialized() -> Vec<(VirtualPath, String)> {
    vec![(
        VirtualPath::INDEX,
        String::from_utf8(index_bytes()).unwrap(),
    )]
}

fn issue_with_id(id: crate::domain::identity::IssueId) -> Issue {
    let mut issue = crate::test_utils::fixture_issue("Conformance".to_string(), String::new());
    issue.id = id;
    issue
}

fn event_line(event: &Event) -> String {
    serde_json::to_string(event).unwrap()
}

fn issue_created(event_id: &str) -> Event {
    let issue = Issue::draft(
        crate::test_utils::issue_id(50),
        "Known".to_string(),
        String::new(),
    );
    crate::test_utils::stamped_event(
        EventBody::draft_issue_created(&issue),
        event_id,
        chrono::Utc::now(),
    )
}

/// An executed run of `gate_key` for `issue_id`, started `second` seconds after
/// the epoch.
fn executed_run(
    run_id: &GateRunId,
    issue_id: &crate::domain::identity::IssueId,
    gate_key: &crate::domain::identity::GateKey,
    second: i64,
) -> GateRunResult {
    GateRunResult {
        started_at: chrono::DateTime::from_timestamp(second, 0).unwrap(),
        origin: GateVerdictOrigin::Executed,
        ..GateRunResult::attestation(
            run_id,
            issue_id,
            gate_key,
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        )
    }
}

fn gate_run_seed(runs: &[GateRunResult]) -> Vec<(VirtualPath, String)> {
    runs.iter()
        .map(|run| {
            (
                VirtualPath::gate_run_record(&run.issue_id, &run.run_id),
                String::from_utf8(serialize_gate_run(run).unwrap()).unwrap(),
            )
        })
        .collect()
}

fn ci_preset() -> GatePresetDefinition {
    GatePresetDefinition {
        name: "ci-checks".to_string(),
        description: "Continuous integration".to_string(),
        gates: vec![GateTemplate {
            key: "review".parse().unwrap(),
            title: "Review".to_string(),
            description: "A reviewer approves".to_string(),
            stage: GateStage::Postcheck,
            mode: crate::declarations::GateMode::Manual,
            checker: None,
        }],
    }
}

/// Path spellings every backend refuses before reading anything.
const INVALID_PATHS: [&str; 6] = [
    "",
    "/etc/passwd",
    "../etc/passwd",
    "a/../b",
    "foo/..",
    "a/../../b",
];

fn test_configure_repository_layout_never_redirects_a_bound_store<B: Backend>() {
    let repository = Repository::<B>::empty();
    let other = TempDir::new().unwrap();
    let bound = repository.store.repository_layout().unwrap();
    assert_eq!(
        (bound.worktree_root(), bound.data_root()),
        (
            repository.layout().worktree_root(),
            repository.layout().data_root()
        )
    );

    for foreign in [
        discover_repository_layout(other.path(), other.path().join(".jit")).unwrap(),
        discover_repository_layout(other.path(), bound.data_root()).unwrap(),
    ] {
        assert!(repository
            .store
            .configure_repository_layout(&foreign)
            .is_err());
        let current = repository.store.clone().repository_layout().unwrap();
        assert_eq!(
            (current.worktree_root(), current.data_root()),
            (bound.worktree_root(), bound.data_root()),
            "binding {} redirected the store",
            foreign.worktree_root().display()
        );
    }
}

fn test_acquire_repo_write_lock_is_reentered_by_a_clone_on_the_holding_thread<B: Backend>() {
    let repository = Repository::<B>::seeded(&initialized());

    let _outer = repository.store.acquire_repo_write_lock().unwrap();
    let _inner = repository.store.clone().acquire_repo_write_lock().unwrap();
}

fn test_run_external_process_returns_what_the_operation_returns_after_one_run<B: Backend>() {
    let repository = Repository::<B>::empty();
    let runs = Cell::new(0);

    let value = repository.store.run_external_process(|| {
        runs.set(runs.get() + 1);
        Ok(7)
    });
    let failure = repository
        .store
        .run_external_process::<()>(|| anyhow::bail!("checker failed"));

    assert_eq!(value.unwrap(), 7);
    assert_eq!(runs.get(), 1);
    assert_eq!(failure.unwrap_err().to_string(), "checker failed");
}

fn test_session_published_issues_read_back_typed<B: Backend>() {
    let repository = Repository::<B>::empty();
    let issue = crate::test_utils::fixture_issue("Crossing".to_string(), "body".to_string());

    repository.publish_issues(std::slice::from_ref(&issue));

    assert_eq!(repository.store.load_issue(&issue.id).unwrap(), issue);
    assert_eq!(
        repository.store.load_issue_or_not_found(&issue.id).unwrap(),
        issue
    );
    assert_eq!(repository.store.list_issues().unwrap(), vec![issue]);
}

fn test_load_issue_reports_an_unpublished_id_as_not_found<B: Backend>() {
    let repository = Repository::<B>::empty();
    repository.publish_issues(&[crate::test_utils::fixture_issue(
        "Published".to_string(),
        String::new(),
    )]);
    let unpublished = crate::test_utils::issue_id(51);

    let error = repository.store.load_issue(&unpublished).unwrap_err();

    assert!(
        error.downcast_ref::<IssueNotFoundError>().is_some(),
        "{error:#}"
    );
}

fn test_load_issue_or_not_found_separates_an_absent_issue_from_an_unreadable_one<B: Backend>() {
    let corrupt = crate::test_utils::issue_id(52);
    let repository =
        Repository::<B>::seeded(&[(VirtualPath::issue_record(&corrupt), "{ corrupt".to_string())]);

    let absent = repository
        .store
        .load_issue_or_not_found(&crate::test_utils::issue_id(53));
    let unreadable = repository.store.load_issue_or_not_found(&corrupt);

    assert!(
        matches!(absent, Err(PathReadError::NotFound(_))),
        "{absent:?}"
    );
    assert!(
        matches!(unreadable, Err(PathReadError::Other(_))),
        "{unreadable:?}"
    );
}

/// A unique prefix or any spelling of a full id names its issue, an unmatched
/// prefix is not found, and a prefix two ids share is ambiguous.
fn test_resolve_issue_id_resolves_prefixes_alike<B: Backend>() {
    let issues = [
        issue_with_id(crate::test_utils::issue_id(
            0x32f24671a4072d94e4d13d41b53beeba,
        )),
        issue_with_id(crate::test_utils::issue_id(
            0x4000_8000_0000_0000_0000_b53b_eeba,
        )),
    ];
    let repository = Repository::<B>::empty();
    repository.publish_issues(&issues);
    let resolve = |requested: &str| {
        repository
            .store
            .resolve_issue_id(&IssueIdPrefix::from_str(requested).unwrap())
    };
    let id = &issues[0].id;

    for requested in [
        id.to_string(),
        id.as_str().to_uppercase(),
        id.as_str().replace('-', ""),
        id.as_str().chars().take(id.short_id().len() + 2).collect(),
    ] {
        assert_eq!(
            resolve(&requested).ok().as_ref(),
            Some(id),
            "{requested} did not resolve"
        );
    }
    let unmatched = resolve("ffffffff").unwrap_err();
    assert!(
        unmatched.downcast_ref::<IssueNotFoundError>().is_some(),
        "{unmatched}"
    );
    let shared = resolve(id.short_id()).unwrap_err();
    assert!(
        shared.downcast_ref::<AmbiguousIdError>().is_some(),
        "{shared}"
    );
}

/// A spelling that names no stored id, a non-canonical one included, is not
/// found.
fn test_resolve_issue_id_reports_a_non_canonical_id_as_not_found<B: Backend>() {
    let repository = Repository::<B>::empty();
    repository.publish_issues(&[issue_with_id(crate::test_utils::issue_id(54))]);

    let error = repository
        .store
        .resolve_issue_id(&IssueIdPrefix::from_str("not-a-canonical-id").unwrap())
        .unwrap_err();

    assert!(
        error.downcast_ref::<IssueNotFoundError>().is_some(),
        "{error:#}"
    );
}

/// An issue record whose id or dependency is not canonical fails to decode,
/// and the error names the record.
fn test_load_issue_rejects_a_non_canonical_stored_id_naming_the_record<B: Backend>() {
    let id = crate::test_utils::issue_id(0xabcd);
    let non_canonical = |seed| crate::test_utils::issue_id(seed).as_str().to_uppercase();
    let path = VirtualPath::issue_record(&id);
    for (field, value) in [
        ("id", serde_json::json!(non_canonical(0xabcd))),
        ("dependencies", serde_json::json!([non_canonical(0xbcde)])),
    ] {
        let mut record = serde_json::to_value(issue_with_id(id.clone())).unwrap();
        record[field] = value;
        let repository = Repository::<B>::seeded(&[(path.clone(), record.to_string())]);

        let error = format!("{:#}", repository.store.load_issue(&id).unwrap_err());

        assert!(
            error.contains(path.relative().as_str()) && error.contains("invalid issue id"),
            "{field}: {error}"
        );
    }
}

fn test_list_issues_is_empty_before_any_issue_is_published<B: Backend>() {
    let repository = Repository::<B>::seeded(&initialized());

    assert_eq!(repository.store.list_issues().unwrap(), Vec::<Issue>::new());
}

fn test_load_gate_registry_reads_the_declared_gates_and_none_when_absent<B: Backend>() {
    let gate = ci_preset().gates[0].to_gate();
    let registry = GateRegistry {
        gates: [(gate.key.clone(), gate)].into_iter().collect(),
    };
    let declared = Repository::<B>::seeded(&[(
        VirtualPath::GATES,
        String::from_utf8(serialize_gate_registry(&registry).unwrap()).unwrap(),
    )]);
    let undeclared = Repository::<B>::seeded(&initialized());

    assert_eq!(
        declared.store.load_gate_registry().unwrap().gates,
        registry.gates
    );
    assert!(undeclared
        .store
        .load_gate_registry()
        .unwrap()
        .gates
        .is_empty());
}

fn test_read_events_rejects_unknown_tags_and_malformed_records<B: Backend>() {
    let (first, second) = (issue_created("first"), issue_created("second"));
    let logs = [
        Some(format!("{}\n{}\n", event_line(&first), event_line(&second))),
        Some("{\"type\":\"retired_event\"}\n".to_string()),
        Some("{\"type\":\"issue_created\"}\n".to_string()),
        Some("{not-json}\n".to_string()),
        None,
    ];
    let expected = [
        Some(vec![first, second]),
        None,
        None,
        None,
        Some(Vec::new()),
    ];

    for (log, expected) in logs.into_iter().zip(expected) {
        let seed = log
            .clone()
            .map(|log| (VirtualPath::EVENTS, log))
            .into_iter()
            .collect::<Vec<_>>();
        let repository = Repository::<B>::seeded(&seed);

        assert_eq!(repository.store.read_events().ok(), expected, "log {log:?}");
    }
}

fn test_read_archive_evidence_record_is_target_addressed<B: Backend>() {
    let target = PlanTarget::Document {
        path: "dev/active/plan.md".to_string(),
    };
    let record = ArchiveEvidenceRecord {
        target: target.clone(),
        executions: vec![ArchiveExecution {
            target: target.clone(),
            destination_root: "dev/archive".to_string(),
            publications: Vec::new(),
            reference_changes: Vec::new(),
            planned_deletions: Vec::new(),
            reconciling: false,
        }],
    };
    let path = VirtualPath::archive_evidence_record(&target);
    let repository = Repository::<B>::seeded(&[(path, serde_json::to_string(&record).unwrap())]);

    assert_eq!(
        repository
            .store
            .read_archive_evidence_record(&target)
            .unwrap(),
        Some(record)
    );
    assert_eq!(
        repository
            .store
            .read_archive_evidence_record(&PlanTarget::Document {
                path: "dev/active/other.md".to_string(),
            })
            .unwrap(),
        None
    );
}

fn test_load_gate_run_result_reads_a_recorded_run_and_reports_others_not_found<B: Backend>() {
    let run = executed_run(
        &crate::test_utils::gate_run_id(1),
        &crate::test_utils::issue_id(163),
        &"tests".parse().unwrap(),
        10,
    );
    let recorded = Repository::<B>::seeded(&gate_run_seed(std::slice::from_ref(&run)));
    let empty = Repository::<B>::empty();

    assert_eq!(
        recorded
            .store
            .load_gate_run_result(&run.issue_id, &run.run_id)
            .unwrap(),
        run
    );
    for error in [
        recorded
            .store
            .load_gate_run_result(&crate::test_utils::issue_id(164), &run.run_id)
            .unwrap_err(),
        recorded
            .store
            .load_gate_run_result(&run.issue_id, &crate::test_utils::gate_run_id(2))
            .unwrap_err(),
        empty
            .store
            .load_gate_run_result(&run.issue_id, &run.run_id)
            .unwrap_err(),
    ] {
        assert!(
            error.downcast_ref::<GateRunNotFoundError>().is_some(),
            "{error:#}"
        );
    }
}

fn test_list_gate_runs_for_issue_lists_only_that_issues_runs<B: Backend>() {
    let runs = [
        executed_run(
            &crate::test_utils::gate_run_id(3),
            &crate::test_utils::issue_id(153),
            &"tests".parse().unwrap(),
            10,
        ),
        executed_run(
            &crate::test_utils::gate_run_id(4),
            &crate::test_utils::issue_id(153),
            &"lint".parse().unwrap(),
            20,
        ),
        executed_run(
            &crate::test_utils::gate_run_id(5),
            &crate::test_utils::issue_id(154),
            &"tests".parse().unwrap(),
            30,
        ),
    ];
    let repository = Repository::<B>::seeded(&gate_run_seed(&runs));
    let listed = |issue: &IssueId| {
        repository
            .store
            .list_gate_runs_for_issue(issue)
            .unwrap()
            .into_iter()
            .map(|run| run.run_id)
            .collect::<BTreeSet<_>>()
    };

    assert_eq!(
        listed(&crate::test_utils::issue_id(153)),
        BTreeSet::from([
            crate::test_utils::gate_run_id(3),
            crate::test_utils::gate_run_id(4)
        ])
    );
    assert!(listed(&crate::test_utils::issue_id(164)).is_empty());
    assert!(Repository::<B>::empty()
        .store
        .list_gate_runs_for_issue(&crate::test_utils::issue_id(153))
        .unwrap()
        .is_empty());
}

/// The path `name` inside `issue`'s gate-run directory.
fn in_issue_directory(issue: &IssueId, name: impl AsRef<Path>) -> VirtualPath {
    VirtualPath::data(
        VirtualPath::gate_run_issue_directory(issue)
            .relative()
            .as_path()
            .join(name),
    )
    .unwrap()
}

/// Inside an issue's directory, only a file named `<run-id>.json` is a run: an
/// unparseable stem, a missing or other extension, and a deeper file list nothing.
fn test_list_gate_runs_for_issue_ignores_clutter_in_the_issue_directory<B: Backend>() {
    let issue = crate::test_utils::issue_id(153);
    let run_id = crate::test_utils::gate_run_id(1);
    let run = String::from_utf8(
        serialize_gate_run(&executed_run(
            &run_id,
            &issue,
            &"tests".parse().unwrap(),
            10,
        ))
        .unwrap(),
    )
    .unwrap();
    let record = VirtualPath::gate_run_record(&issue, &run_id);
    let seed = [
        (in_issue_directory(&issue, "not-a-run-id.json"), run.clone()),
        (in_issue_directory(&issue, run_id.as_str()), run.clone()),
        (in_issue_directory(&issue, format!("{run_id}.txt")), run),
        (
            in_issue_directory(
                &issue,
                Path::new("nested").join(record.file_name().unwrap()),
            ),
            "{ corrupt".to_string(),
        ),
    ];

    assert!(Repository::<B>::seeded(&seed)
        .store
        .list_gate_runs_for_issue(&issue)
        .unwrap()
        .is_empty());
}

/// Each reader opens only the named issue's directory, so a corrupt record of
/// another issue fails neither.
fn test_gate_run_readers_ignore_a_corrupt_record_of_another_issue<B: Backend>() {
    let run = executed_run(
        &crate::test_utils::gate_run_id(1),
        &crate::test_utils::issue_id(153),
        &"tests".parse().unwrap(),
        10,
    );
    let mut seed = gate_run_seed(std::slice::from_ref(&run));
    seed.push((
        VirtualPath::gate_run_record(
            &crate::test_utils::issue_id(154),
            &crate::test_utils::gate_run_id(2),
        ),
        "{ corrupt".to_string(),
    ));
    let repository = Repository::<B>::seeded(&seed);

    assert_eq!(
        repository
            .store
            .list_gate_runs_for_issue(&run.issue_id)
            .unwrap(),
        std::slice::from_ref(&run)
    );
    assert_eq!(
        repository
            .store
            .load_gate_run_result(&run.issue_id, &run.run_id)
            .unwrap(),
        run
    );
}

/// A record in the issue's directory, whatever its shape.
fn test_gate_run_readers_name_a_malformed_result<B: Backend>() {
    let (issue, run_id) = (
        crate::test_utils::issue_id(1),
        crate::test_utils::gate_run_id(1),
    );
    let path = VirtualPath::gate_run_record(&issue, &run_id);
    let repository = Repository::<B>::seeded(&[(path.clone(), "{ corrupt".to_string())]);

    for error in [
        repository
            .store
            .load_gate_run_result(&issue, &run_id)
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&issue)
            .unwrap_err(),
    ] {
        assert!(
            format!("{error:#}").contains(path.relative().as_str()),
            "{error:#}"
        );
    }
}

fn test_gate_run_readers_refuse_a_file_as_the_gate_run_root<B: Backend>() {
    let repository =
        Repository::<B>::seeded(&[(VirtualPath::GATE_RUNS, "not a directory".to_string())]);

    for error in [
        repository
            .store
            .load_gate_run_result(
                &crate::test_utils::issue_id(2),
                &crate::test_utils::gate_run_id(1),
            )
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&crate::test_utils::issue_id(2))
            .unwrap_err(),
    ] {
        assert!(
            format!("{error:#}").contains("ordinary directory"),
            "{error:#}"
        );
    }
}

fn test_gate_run_readers_refuse_a_directory_as_a_result<B: Backend>() {
    let (issue, run_id) = (
        crate::test_utils::issue_id(1),
        crate::test_utils::gate_run_id(1),
    );
    let record = VirtualPath::gate_run_record(&issue, &run_id);
    let repository = Repository::<B>::seeded(&[(
        VirtualPath::data(record.relative().as_path().join("inner")).unwrap(),
        String::new(),
    )]);

    for error in [
        repository
            .store
            .load_gate_run_result(&issue, &run_id)
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&issue)
            .unwrap_err(),
    ] {
        let message = format!("{error:#}");
        assert!(message.contains(record.relative().as_str()), "{message}");
        assert!(message.contains("ordinary file"), "{message}");
    }
}

/// A recorded run of `issue` placed outside the gate-run root, for a symlink
/// to name: a reader that followed the link would read it.
fn record_outside_gate_runs<B: Backend>(
    issue: &IssueId,
    run_id: &GateRunId,
) -> (Repository<B>, VirtualPath) {
    let record = VirtualPath::gate_run_record(issue, run_id);
    let outside =
        VirtualPath::data(Path::new("outside").join(record.relative().as_path())).unwrap();
    let run = executed_run(run_id, issue, &"tests".parse().unwrap(), 10);
    let repository = Repository::<B>::seeded(&[(
        outside.clone(),
        String::from_utf8(serialize_gate_run(&run).unwrap()).unwrap(),
    )]);
    (repository, outside)
}

/// The leading `depth` components of `path`, under the same root.
fn path_prefix(path: &VirtualPath, depth: usize) -> VirtualPath {
    VirtualPath::data(
        path.relative()
            .as_path()
            .components()
            .take(depth)
            .collect::<std::path::PathBuf>(),
    )
    .unwrap()
}

/// Both readers refuse `directory` when it is a symlink to a directory holding
/// the record.
fn assert_gate_run_readers_refuse_a_symlinked<B: Backend>(directory: &VirtualPath) {
    let (issue, run_id) = (
        crate::test_utils::issue_id(1),
        crate::test_utils::gate_run_id(1),
    );
    let (repository, outside) = record_outside_gate_runs::<B>(&issue, &run_id);
    let depth = directory.relative().as_path().components().count();
    repository.link(directory, &path_prefix(&outside, depth + 1));

    for error in [
        repository
            .store
            .load_gate_run_result(&issue, &run_id)
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&issue)
            .unwrap_err(),
    ] {
        assert!(
            format!("{error:#}").contains("ordinary directory"),
            "{error:#}"
        );
    }
}

fn test_gate_run_readers_refuse_a_symlink_as_the_gate_run_root<B: Backend>() {
    assert_gate_run_readers_refuse_a_symlinked::<B>(&VirtualPath::GATE_RUNS);
}

fn test_gate_run_readers_refuse_a_symlinked_issue_directory<B: Backend>() {
    assert_gate_run_readers_refuse_a_symlinked::<B>(&VirtualPath::gate_run_issue_directory(
        &crate::test_utils::issue_id(1),
    ));
}

fn test_gate_run_readers_refuse_a_symlink_as_a_result<B: Backend>() {
    let (issue, run_id) = (
        crate::test_utils::issue_id(1),
        crate::test_utils::gate_run_id(1),
    );
    let (repository, outside) = record_outside_gate_runs::<B>(&issue, &run_id);
    let record = VirtualPath::gate_run_record(&issue, &run_id);
    repository.link(&record, &outside);

    for error in [
        repository
            .store
            .load_gate_run_result(&issue, &run_id)
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&issue)
            .unwrap_err(),
    ] {
        let message = format!("{error:#}");
        assert!(message.contains(record.relative().as_str()), "{message}");
        assert!(message.contains("ordinary file"), "{message}");
    }
}

/// Both readers refuse a special object at `directory`.
fn assert_gate_run_readers_refuse_a_special<B: Backend>(directory: &VirtualPath) {
    let issue = crate::test_utils::issue_id(2);
    let repository = Repository::<B>::seeded(&initialized());
    repository.plant_special(directory);

    for error in [
        repository
            .store
            .load_gate_run_result(&issue, &crate::test_utils::gate_run_id(1))
            .unwrap_err(),
        repository
            .store
            .list_gate_runs_for_issue(&issue)
            .unwrap_err(),
    ] {
        assert!(
            format!("{error:#}").contains("ordinary directory"),
            "{error:#}"
        );
    }
}

fn test_gate_run_readers_refuse_a_special_object_as_the_gate_run_root<B: Backend>() {
    assert_gate_run_readers_refuse_a_special::<B>(&VirtualPath::GATE_RUNS);
}

fn test_gate_run_readers_refuse_a_special_object_as_the_issue_directory<B: Backend>() {
    assert_gate_run_readers_refuse_a_special::<B>(&VirtualPath::gate_run_issue_directory(
        &crate::test_utils::issue_id(2),
    ));
}

/// An issue directory not holding the named record holds no such run.
fn test_load_gate_run_result_reports_an_issue_directory_without_the_record_as_not_found<
    B: Backend,
>() {
    let (issue, run_id) = (
        crate::test_utils::issue_id(1),
        crate::test_utils::gate_run_id(1),
    );
    let repository =
        Repository::<B>::seeded(&[(in_issue_directory(&issue, "other.json"), "{}".to_string())]);

    let error = repository
        .store
        .load_gate_run_result(&issue, &run_id)
        .unwrap_err();
    assert!(
        error.downcast_ref::<GateRunNotFoundError>().is_some(),
        "{error:#}"
    );
}

/// A plan applies only against the image it was built over, even when the
/// images agree on every path the plan writes.
fn test_apply_admits_a_plan_only_against_its_exact_image<B: Backend>() {
    let target = VirtualPath::data("target.txt").unwrap();
    let evidence = VirtualPath::data("evidence.txt").unwrap();
    let repository = Repository::<B>::seeded(&[(evidence.clone(), "image-a".to_string())]);
    let layout = repository.layout();
    let budget = CaptureBudget {
        max_listings: 8,
        max_bytes: 1024 * 1024,
        max_depth: 8,
    };
    let spec = || {
        let mut spec = CaptureSpec::new([target.clone(), evidence.clone()], budget).unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        spec
    };
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            target.clone(),
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"materialized".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    let image_a = repository
        .store
        .open_mutation_session(layout.clone())
        .unwrap()
        .capture(spec())
        .unwrap();
    let plan_a =
        MaterializationPlan::new(&image_a, delta.clone(), &MutationContext::preview()).unwrap();
    assert_eq!(plan_a.image(), &image_a);

    repository.write(&evidence, b"image-b");
    let mut session = repository.store.open_mutation_session(layout).unwrap();
    let image_b = session.capture(spec()).unwrap();

    assert_eq!(
        image_a.entry(&target).unwrap(),
        image_b.entry(&target).unwrap()
    );
    assert_ne!(plan_a.image(), &image_b);
    assert!(matches!(
        session.apply(&plan_a),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    let plan_b =
        MaterializationPlan::new(&image_b, delta.clone(), &MutationContext::preview()).unwrap();
    let outcome = session.apply(&plan_b).unwrap();
    assert_eq!(
        outcome.actions_applied,
        plan_b.publication().actions().len()
    );
}

/// A session recognises only the image it captured: a plan over a
/// byte-identical image captured by another session is a retryable conflict,
/// and a plan shares its image's storage.
fn test_apply_refuses_a_plan_over_a_byte_identical_image_of_another_session<B: Backend>() {
    let target = VirtualPath::data("target.txt").unwrap();
    let repository = Repository::<B>::seeded(&[]);
    let layout = repository.layout();
    let budget = CaptureBudget {
        max_listings: 8,
        max_bytes: 1024 * 1024,
        max_depth: 8,
    };
    let spec = || CaptureSpec::new([target.clone()], budget).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            target.clone(),
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"materialized".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    let first_image = repository
        .store
        .open_mutation_session(layout.clone())
        .unwrap()
        .capture(spec())
        .unwrap();
    let foreign_plan =
        MaterializationPlan::new(&first_image, delta.clone(), &MutationContext::preview()).unwrap();
    assert!(foreign_plan.image().shares_data_with(&first_image));

    let mut session = repository.store.open_mutation_session(layout).unwrap();
    let image = session.capture(spec()).unwrap();

    assert_eq!(first_image, image, "the two captures are byte-identical");
    assert!(matches!(
        session.apply(&foreign_plan),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    let own_plan =
        MaterializationPlan::new(&image, delta.clone(), &MutationContext::preview()).unwrap();
    assert_eq!(
        session.apply(&own_plan).unwrap().actions_applied,
        delta.actions().len()
    );
}

fn test_root_is_shared_by_clones_and_names_the_data_root_when_file_backed<B: Backend>() {
    let repository = Repository::<B>::empty();
    let store = &repository.store;

    assert_eq!(store.clone().root(), store.root());
    assert!(
        !store.is_file_backed() || store.root() == repository.layout().data_root(),
        "a file-backed store's root is not its data root"
    );
}

fn test_read_repo_file_reads_present_files_and_refuses_unsafe_paths<B: Backend>() {
    let repository = Repository::<B>::seeded(&[
        (
            VirtualPath::worktree("project-items.md").unwrap(),
            "hello".to_string(),
        ),
        (VirtualPath::data("notes.md").unwrap(), "data".to_string()),
    ]);
    let read = |path: &str| repository.store.read_repo_file(path);

    assert_eq!(read("project-items.md").unwrap().as_deref(), Some("hello"));
    assert_eq!(read(".jit/notes.md").unwrap().as_deref(), Some("data"));
    assert_eq!(read("missing.md").unwrap(), None);
    for path in ["", "/etc/passwd", "../escape.md"] {
        assert!(
            matches!(read(path), Err(PathReadError::InvalidPath(_))),
            "{path:?} was not refused"
        );
    }
}

fn test_read_path_reports_a_missing_file_and_refuses_unsafe_paths<B: Backend>() {
    let repository = Repository::<B>::empty();
    let store = &repository.store;

    assert!(matches!(
        store.read_path_bytes("does-not-exist.md", None),
        Err(PathReadError::NotFound(_))
    ));
    assert!(matches!(
        store.read_path_text("does-not-exist.md", None),
        Err(PathReadError::NotFound(_))
    ));
    for path in INVALID_PATHS {
        assert!(
            matches!(
                store.read_path_bytes(path, None),
                Err(PathReadError::InvalidPath(_))
            ),
            "bytes read of {path:?} was not refused"
        );
        assert!(
            matches!(
                store.read_path_text(path, None),
                Err(PathReadError::InvalidPath(_))
            ),
            "text read of {path:?} was not refused"
        );
    }
}

fn test_read_path_reads_a_seeded_file_and_refuses_a_directory<B: Backend>() {
    let repository = Repository::<B>::seeded(&[
        (
            VirtualPath::worktree("docs/archive..2024.md").unwrap(),
            "# Archive\n".to_string(),
        ),
        (VirtualPath::data("notes.md").unwrap(), "data".to_string()),
    ]);
    let store = &repository.store;

    for (path, content) in [
        ("docs/archive..2024.md", "# Archive\n"),
        (".jit/notes.md", "data"),
    ] {
        assert_eq!(
            store.read_path_bytes(path, None).unwrap(),
            (content.as_bytes().to_vec(), "working-tree".to_string()),
            "bytes of {path}"
        );
        assert_eq!(
            store.read_path_text(path, None).unwrap(),
            (content.to_string(), "working-tree".to_string()),
            "text of {path}"
        );
    }
    assert!(matches!(
        store.read_path_bytes("docs", None),
        Err(PathReadError::OutsideRepoRoot(_))
    ));
}

fn test_read_path_keeps_exact_bytes_and_decodes_invalid_utf8_lossily<B: Backend>() {
    let repository = Repository::<B>::empty();
    let path = VirtualPath::worktree("docs/latin1.txt").unwrap();
    repository.write(&path, b"caf\xE9");
    let store = &repository.store;

    assert_eq!(
        store
            .read_path_bytes(path.relative().as_str(), None)
            .unwrap()
            .0,
        b"caf\xE9"
    );
    assert_eq!(
        store
            .read_path_text(path.relative().as_str(), None)
            .unwrap()
            .0,
        "caf\u{FFFD}"
    );
}

fn test_read_repo_file_refuses_invalid_utf8<B: Backend>() {
    let repository = Repository::<B>::empty();
    let path = VirtualPath::worktree("docs/latin1.txt").unwrap();
    repository.write(&path, b"caf\xE9");

    let outcome = repository.store.read_repo_file(path.relative().as_str());

    assert!(
        matches!(outcome, Err(PathReadError::Other(_))),
        "{outcome:?}"
    );
}

/// Each commit's reads see that commit's state alone, whatever came after it.
fn test_read_path_at_commit_returns_the_state_recorded_for_that_commit<B: Backend>() {
    let plan = VirtualPath::worktree("docs/plan.md").unwrap();
    let later = VirtualPath::worktree("docs/later.md").unwrap();
    let notes = VirtualPath::data("notes.md").unwrap();
    let repository = Repository::<B>::seeded(&[
        (plan.clone(), "one".to_string()),
        (notes, "data".to_string()),
    ]);
    let first = repository.commit();
    repository.write(&plan, b"two");
    repository.write(&later, b"added");
    let second = repository.commit();
    repository.write(&plan, b"uncommitted");
    let store = &repository.store;
    let label = |commit: &str| commit.chars().take(7).collect::<String>();

    for (commit, content) in [(&first, "one"), (&second, "two")] {
        assert_eq!(
            store.read_path_bytes("docs/plan.md", Some(commit)).unwrap(),
            (content.as_bytes().to_vec(), label(commit))
        );
        assert_eq!(
            store.read_path_text("docs/plan.md", Some(commit)).unwrap(),
            (content.to_string(), label(commit))
        );
    }
    assert_eq!(
        store
            .read_path_bytes(".jit/notes.md", Some(&first))
            .unwrap()
            .0,
        b"data"
    );
    assert!(matches!(
        store.read_path_bytes("docs/later.md", Some(&first)),
        Err(PathReadError::NotFound(_))
    ));
    assert_eq!(
        store
            .read_path_bytes("docs/later.md", Some(&second))
            .unwrap()
            .0,
        b"added"
    );
    assert_eq!(
        store.read_path_bytes("docs/plan.md", None).unwrap().0,
        b"uncommitted"
    );
}

fn test_read_path_at_an_unknown_commit_reports_commit_not_found<B: Backend>() {
    let repository = Repository::<B>::seeded(&[(
        VirtualPath::worktree("docs/plan.md").unwrap(),
        "one".to_string(),
    )]);
    repository.commit();
    let store = &repository.store;

    for commit in ["no-such-commit", "0000000000000000000000000000000000000000"] {
        let bytes = store.read_path_bytes("docs/plan.md", Some(commit));
        let text = store.read_path_text("docs/plan.md", Some(commit));

        assert!(
            matches!(bytes, Err(PathReadError::CommitNotFound(_))),
            "{commit}: {bytes:?}"
        );
        assert!(
            matches!(text, Err(PathReadError::CommitNotFound(_))),
            "{commit}: {text:?}"
        );
    }
}

fn test_read_path_at_commit_refuses_unsafe_paths_and_directories<B: Backend>() {
    let repository = Repository::<B>::seeded(&[(
        VirtualPath::worktree("docs/plan.md").unwrap(),
        "one".to_string(),
    )]);
    let commit = repository.commit();
    let store = &repository.store;

    for path in INVALID_PATHS {
        assert!(
            matches!(
                store.read_path_bytes(path, Some(&commit)),
                Err(PathReadError::InvalidPath(_))
            ),
            "pinned read of {path:?} was not refused"
        );
    }
    let directory = store.read_path_bytes("docs", Some(&commit));
    assert!(
        matches!(directory, Err(PathReadError::Other(_))),
        "{directory:?}"
    );
}

/// A kind read classifies each target as a whole read would, without reading a
/// file's bytes: a file larger than the whole byte budget still captures, and a
/// producer asking for its bytes fails typed.
fn test_capture_kind_classifies_targets_without_reading_file_bytes<B: Backend>() {
    use crate::repository_state::{CaptureError, ReadMode, RepositoryTargetKind};

    let file = VirtualPath::worktree("receipts/large.log").unwrap();
    let directory = VirtualPath::worktree("receipts").unwrap();
    let absent = VirtualPath::worktree("receipts/absent.log").unwrap();
    let repository = Repository::<B>::seeded(&[(file.clone(), "x".repeat(4096))]);
    let layout = repository.layout();
    let budget = CaptureBudget {
        max_listings: 0,
        max_bytes: 1024,
        max_depth: 8,
    };
    let mut spec = CaptureSpec::new([], budget).unwrap();
    spec.discover_kinds([file.clone(), directory.clone(), absent.clone()])
        .unwrap();
    assert_eq!(spec.read_mode(&file), Some(ReadMode::Kind));
    let mut session = repository.store.open_mutation_session(layout).unwrap();

    let image = session.capture(spec).unwrap();

    let kind = |path: &VirtualPath| RepositoryTargetKind::from_entry(image.entry(path).unwrap());
    assert_eq!(kind(&file), RepositoryTargetKind::File);
    assert_eq!(kind(&directory), RepositoryTargetKind::Directory);
    assert_eq!(kind(&absent), RepositoryTargetKind::Missing);
    assert!(matches!(
        image.file_bytes(&file),
        Err(CaptureError::KindOnlyCapture(_))
    ));
}

/// Runs every case against every backend, one `#[test]` per pair, in a module
/// named for the backend.
macro_rules! conformance_suite {
    ($($backend:ident => $store:ty),+ ; $cases:tt) => {
        $(conformance_suite!(@backend $backend, $store, $cases);)+
    };
    (@backend $backend:ident, $store:ty, [$($case:ident),+ $(,)?]) => {
        mod $backend {
            $(
                #[test]
                fn $case() {
                    super::$case::<$store>()
                }
            )+
        }
    };
}

conformance_suite!(
    json => crate::storage::JsonFileStorage,
    memory => crate::storage::InMemoryStorage;
    [
        test_configure_repository_layout_never_redirects_a_bound_store,
        test_acquire_repo_write_lock_is_reentered_by_a_clone_on_the_holding_thread,
        test_run_external_process_returns_what_the_operation_returns_after_one_run,
        test_session_published_issues_read_back_typed,
        test_load_issue_reports_an_unpublished_id_as_not_found,
        test_load_issue_or_not_found_separates_an_absent_issue_from_an_unreadable_one,
        test_resolve_issue_id_resolves_prefixes_alike,
        test_resolve_issue_id_reports_a_non_canonical_id_as_not_found,
        test_load_issue_rejects_a_non_canonical_stored_id_naming_the_record,
        test_list_issues_is_empty_before_any_issue_is_published,
        test_load_gate_registry_reads_the_declared_gates_and_none_when_absent,
        test_read_events_rejects_unknown_tags_and_malformed_records,
        test_read_archive_evidence_record_is_target_addressed,
        test_load_gate_run_result_reads_a_recorded_run_and_reports_others_not_found,
        test_list_gate_runs_for_issue_lists_only_that_issues_runs,
        test_list_gate_runs_for_issue_ignores_clutter_in_the_issue_directory,
        test_gate_run_readers_ignore_a_corrupt_record_of_another_issue,
        test_gate_run_readers_name_a_malformed_result,
        test_gate_run_readers_refuse_a_file_as_the_gate_run_root,
        test_gate_run_readers_refuse_a_directory_as_a_result,
        test_gate_run_readers_refuse_a_symlink_as_the_gate_run_root,
        test_gate_run_readers_refuse_a_symlinked_issue_directory,
        test_gate_run_readers_refuse_a_symlink_as_a_result,
        test_gate_run_readers_refuse_a_special_object_as_the_gate_run_root,
        test_gate_run_readers_refuse_a_special_object_as_the_issue_directory,
        test_load_gate_run_result_reports_an_issue_directory_without_the_record_as_not_found,
        test_apply_admits_a_plan_only_against_its_exact_image,
        test_apply_refuses_a_plan_over_a_byte_identical_image_of_another_session,
        test_root_is_shared_by_clones_and_names_the_data_root_when_file_backed,
        test_read_repo_file_reads_present_files_and_refuses_unsafe_paths,
        test_read_path_reports_a_missing_file_and_refuses_unsafe_paths,
        test_read_path_reads_a_seeded_file_and_refuses_a_directory,
        test_read_path_keeps_exact_bytes_and_decodes_invalid_utf8_lossily,
        test_read_repo_file_refuses_invalid_utf8,
        test_read_path_at_commit_returns_the_state_recorded_for_that_commit,
        test_read_path_at_an_unknown_commit_reports_commit_not_found,
        test_read_path_at_commit_refuses_unsafe_paths_and_directories,
        test_capture_kind_classifies_targets_without_reading_file_bytes,
    ]
);
