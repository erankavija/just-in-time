//! The [`IssueStore`] contract, written once and run against every backend.
//!
//! Each case is a generic function over `Backend`, whose one constructor
//! builds a store over a worktree holding a shared seed; `conformance_suite!`
//! runs every case against every listed backend, so a backend joins the suite
//! by implementing `Backend` and naming itself there. A backend's own tests
//! state only what that backend adds (`@/inv/orthogonal-tests`). Commit-pinned
//! path reads stay out until the in-memory backend serves them from its own
//! state rather than the real disk.

use crate::declarations::{serialize_gate_registry, GateRegistry, GateStage};
use crate::domain::artifact_plan::PlanTarget;
use crate::domain::identity::IssueIdPrefix;
use crate::domain::repository_inputs::{InputsDigest, InputsDigestBuilder};
use crate::domain::{
    Event, EventBody, GateRunResult, GateRunStatus, GateStatus, GateVerdictOrigin, Issue,
    SHORT_ID_LENGTH,
};
use crate::gate_presets::{GatePresetDefinition, GateTemplate};
use crate::repository_state::{
    gate_run_result_relative_path, serialize_gate_run, serialize_issue, CaptureBudget, CaptureSpec,
    ExpectedPreimage, FileMode, MaterializationPlan, RepositoryAction, RepositoryActionOwner,
    RepositoryDelta, RepositoryIndex, RepositoryLayout, RepositoryRootClass, VirtualPath,
};
use crate::storage::{
    discover_repository_layout, AmbiguousIdError, GateRunNotFoundError, InMemoryStorage,
    IssueNotFoundError, IssueStore, JsonFileStorage, PathReadError, PresetNotFoundError,
    RepositoryStateStore,
};
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
}

impl Backend for JsonFileStorage {
    fn over(worktree: &Path, seed: &Seed) -> Self {
        let data = worktree.join(".jit");
        seed.iter().for_each(|(path, content)| {
            let target = match path.root_class() {
                RepositoryRootClass::Data => data.clone(),
                RepositoryRootClass::Worktree => worktree.to_path_buf(),
            }
            .join(path.relative().as_path());
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, content).unwrap();
        });
        let store = JsonFileStorage::new(&data);
        store.configure_repository_layout(&discover_repository_layout(worktree, &data).unwrap());
        store
    }
}

impl Backend for InMemoryStorage {
    fn over(worktree: &Path, seed: &Seed) -> Self {
        let store = InMemoryStorage::rooted_at(worktree);
        seed.iter().for_each(|(path, content)| {
            let relative = path.relative().as_path();
            match path.root_class() {
                RepositoryRootClass::Data => store.add_data_file(relative, content),
                RepositoryRootClass::Worktree => store.add_worktree_file(relative, content),
            }
        });
        store
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

    fn layout(&self) -> RepositoryLayout {
        discover_repository_layout(self.worktree.path(), self.worktree.path().join(".jit")).unwrap()
    }

    /// Publish `issues` through one mutation session over an absent data root.
    fn publish_issues(&self, issues: &[Issue]) {
        let layout = self.layout();
        let issue_paths = issues
            .iter()
            .map(|issue| issue_path(&issue.id))
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
                index_bytes(issues),
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
            .apply(&MaterializationPlan::new(&image, delta))
            .unwrap();
    }
}

fn issue_path(id: &str) -> VirtualPath {
    VirtualPath::data(format!(
        "{}/{id}.json",
        VirtualPath::ISSUES.relative().as_str()
    ))
    .unwrap()
}

fn index_bytes(issues: &[Issue]) -> Vec<u8> {
    RepositoryIndex {
        all_ids: issues.iter().map(|issue| issue.id.clone()).collect(),
        ..RepositoryIndex::default()
    }
    .to_pretty_bytes()
    .unwrap()
}

/// A data root holding only an empty membership index.
fn initialized() -> Vec<(VirtualPath, String)> {
    vec![(
        VirtualPath::INDEX,
        String::from_utf8(index_bytes(&[])).unwrap(),
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

fn issue_created(id: &str) -> Event {
    let mut issue = Issue::draft("Known".to_string(), String::new());
    issue.id = crate::test_utils::issue_id(50);
    crate::test_utils::stamped_event(
        EventBody::draft_issue_created(&issue),
        id,
        chrono::Utc::now(),
    )
}

/// An executed run of `gate_key` for `issue_id`, started `second` seconds after
/// the epoch.
fn executed_run(
    run_id: &str,
    issue_id: &crate::domain::identity::IssueId,
    gate_key: &str,
    second: i64,
    digest: Option<InputsDigest>,
) -> GateRunResult {
    GateRunResult {
        run_id: run_id.to_string(),
        started_at: chrono::DateTime::from_timestamp(second, 0).unwrap(),
        inputs_digest: digest,
        origin: GateVerdictOrigin::Executed,
        ..GateRunResult::attestation(
            issue_id,
            gate_key,
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        )
    }
}

fn digest_of(content: &[u8]) -> InputsDigest {
    let mut builder = InputsDigestBuilder::new(b"gate declaration");
    builder.push_file("src/lib.rs", content);
    builder.finish()
}

fn gate_run_seed(runs: &[GateRunResult]) -> Vec<(VirtualPath, String)> {
    runs.iter()
        .map(|run| {
            let path = gate_run_result_relative_path(&run.run_id).unwrap();
            (
                VirtualPath::data(path.as_path()).unwrap(),
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
            key: "review".to_string(),
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
        repository.store.configure_repository_layout(&foreign);
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
    let unpublished = uuid::Uuid::new_v4().to_string();

    let error = repository.store.load_issue(&unpublished).unwrap_err();

    assert!(
        error.downcast_ref::<IssueNotFoundError>().is_some(),
        "{error:#}"
    );
}

fn test_load_issue_or_not_found_separates_an_absent_issue_from_an_unreadable_one<B: Backend>() {
    let corrupt = uuid::Uuid::new_v4().to_string();
    let repository = Repository::<B>::seeded(&[(issue_path(&corrupt), "{ corrupt".to_string())]);

    let absent = repository
        .store
        .load_issue_or_not_found(&uuid::Uuid::new_v4().to_string());
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
        id.to_uppercase(),
        id.replace('-', ""),
        id[..SHORT_ID_LENGTH + 2].to_string(),
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
    let shared = resolve(&id[..SHORT_ID_LENGTH]).unwrap_err();
    assert!(
        shared.downcast_ref::<AmbiguousIdError>().is_some(),
        "{shared}"
    );
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

fn test_read_events_skips_retired_tags_and_rejects_malformed_known_records<B: Backend>() {
    let (first, second) = (issue_created("first"), issue_created("second"));
    let retired = concat!(
        "{\"type\":\"retired_",
        "event\",\"id\":\"historical\",\"timestamp\":\"2025-01-01T00:00:00Z\"}"
    );
    let logs = [
        Some(format!(
            "{retired}\n{}\n{retired}\n{}\n",
            event_line(&first),
            event_line(&second)
        )),
        Some("{\"type\":\"issue_created\"}\n".to_string()),
        Some("{not-json}\n".to_string()),
        None,
    ];
    let expected = [Some(vec![first, second]), None, None, Some(Vec::new())];

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

fn test_read_artifact_archive_events_keeps_only_archive_executions<B: Backend>() {
    let archive = crate::test_utils::stamped_event(
        EventBody::draft_artifact_archive_executed(
            PlanTarget::Document {
                path: "dev/active/plan.md".to_string(),
            },
            "dev/archive".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        ),
        "archive",
        chrono::Utc::now(),
    );
    let log = format!(
        "{}\n{}\n{}\n",
        event_line(&issue_created("before")),
        event_line(&archive),
        event_line(&issue_created("after"))
    );
    let repository = Repository::<B>::seeded(&[(VirtualPath::EVENTS, log)]);

    assert_eq!(
        repository.store.read_artifact_archive_events().unwrap(),
        vec![archive]
    );
}

fn test_load_gate_run_result_reads_a_recorded_run_and_reports_others_not_found<B: Backend>() {
    let run = executed_run(
        "run-one",
        &crate::test_utils::issue_id(163),
        "tests",
        10,
        None,
    );
    let recorded = Repository::<B>::seeded(&gate_run_seed(std::slice::from_ref(&run)));
    let empty = Repository::<B>::empty();

    assert_eq!(recorded.store.load_gate_run_result("run-one").unwrap(), run);
    for error in [
        recorded.store.load_gate_run_result("run-two").unwrap_err(),
        empty.store.load_gate_run_result("run-one").unwrap_err(),
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
            "run-a1",
            &crate::test_utils::issue_id(153),
            "tests",
            10,
            None,
        ),
        executed_run(
            "run-a2",
            &crate::test_utils::issue_id(153),
            "lint",
            20,
            None,
        ),
        executed_run(
            "run-b1",
            &crate::test_utils::issue_id(154),
            "tests",
            30,
            None,
        ),
    ];
    let repository = Repository::<B>::seeded(&gate_run_seed(&runs));
    let listed = |issue: &str| {
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
        BTreeSet::from(["run-a1".to_string(), "run-a2".to_string()])
    );
    assert!(listed(&crate::test_utils::issue_id(164)).is_empty());
    assert!(Repository::<B>::empty()
        .store
        .list_gate_runs_for_issue(&crate::test_utils::issue_id(153))
        .unwrap()
        .is_empty());
}

/// Across issues, the latest run of the gate over the digest that executed its
/// own checker and reached a verdict is the source.
fn test_find_reusable_gate_run_selects_the_latest_executed_verdict_over_the_digest<B: Backend>() {
    let (digest, other) = (digest_of(b"current"), digest_of(b"moved"));
    let errored = GateRunResult {
        status: GateRunStatus::Error,
        ..executed_run(
            "errored",
            &crate::test_utils::issue_id(153),
            "tests",
            50,
            Some(digest.clone()),
        )
    };
    let attested = GateRunResult {
        origin: GateVerdictOrigin::Attested,
        ..executed_run(
            "attested",
            &crate::test_utils::issue_id(153),
            "tests",
            40,
            Some(digest.clone()),
        )
    };
    let failed = GateRunResult {
        status: GateRunStatus::Failed,
        ..executed_run(
            "failed",
            &crate::test_utils::issue_id(154),
            "tests",
            30,
            Some(digest.clone()),
        )
    };
    let runs = [
        executed_run(
            "older",
            &crate::test_utils::issue_id(153),
            "tests",
            10,
            Some(digest.clone()),
        ),
        failed.clone(),
        attested,
        errored,
        executed_run(
            "other-digest",
            &crate::test_utils::issue_id(153),
            "tests",
            60,
            Some(other),
        ),
        executed_run(
            "other-gate",
            &crate::test_utils::issue_id(153),
            "lint",
            70,
            Some(digest.clone()),
        ),
    ];
    let repository = Repository::<B>::seeded(&gate_run_seed(&runs));

    assert_eq!(
        repository
            .store
            .find_reusable_gate_run("tests", &digest)
            .unwrap(),
        Some(failed)
    );
    assert_eq!(
        repository
            .store
            .find_reusable_gate_run("undeclared", &digest)
            .unwrap(),
        None
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

fn test_gate_presets_list_and_get_the_declared_presets<B: Backend>() {
    let preset = ci_preset();
    let declared = Repository::<B>::seeded(&[(
        VirtualPath::data(format!(
            "{}/{}.json",
            VirtualPath::GATE_PRESETS.relative().as_str(),
            preset.name
        ))
        .unwrap(),
        serde_json::to_string(&preset).unwrap(),
    )]);
    let listed = declared
        .store
        .list_gate_presets()
        .unwrap()
        .into_iter()
        .map(|info| (info.name, info.description, info.gate_count))
        .collect::<Vec<_>>();
    let missing = declared.store.get_gate_preset("undeclared").unwrap_err();

    assert_eq!(
        listed,
        vec![(
            preset.name.clone(),
            preset.description.clone(),
            preset.gates.len()
        )]
    );
    assert_eq!(
        declared.store.get_gate_preset(&preset.name).unwrap(),
        preset
    );
    assert!(
        missing.downcast_ref::<PresetNotFoundError>().is_some(),
        "{missing:#}"
    );
    assert!(Repository::<B>::empty()
        .store
        .list_gate_presets()
        .unwrap()
        .is_empty());
}

/// Working-tree reads only; see the module documentation.
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

/// Working-tree reads only; see the module documentation.
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
        test_list_issues_is_empty_before_any_issue_is_published,
        test_load_gate_registry_reads_the_declared_gates_and_none_when_absent,
        test_read_events_skips_retired_tags_and_rejects_malformed_known_records,
        test_read_artifact_archive_events_keeps_only_archive_executions,
        test_load_gate_run_result_reads_a_recorded_run_and_reports_others_not_found,
        test_list_gate_runs_for_issue_lists_only_that_issues_runs,
        test_find_reusable_gate_run_selects_the_latest_executed_verdict_over_the_digest,
        test_root_is_shared_by_clones_and_names_the_data_root_when_file_backed,
        test_read_repo_file_reads_present_files_and_refuses_unsafe_paths,
        test_gate_presets_list_and_get_the_declared_presets,
        test_read_path_reports_a_missing_file_and_refuses_unsafe_paths,
        test_read_path_reads_a_seeded_file_and_refuses_a_directory,
    ]
);
