use super::*;
use crate::repository_state::{
    plan_hash, CaptureBudget, MaterializationIntent, RepositoryAction, RepositorySeed,
    RepositorySeedKind,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn test_advisory_error_classification_only_accepts_permission_denied_io() {
    let errors = [
        RepositoryStateStoreError::Io(std::io::Error::other("hard I/O failure")),
        RepositoryStateStoreError::Capture(CaptureError::PathBudgetExceeded {
            actual: 2,
            maximum: 1,
        }),
        RepositoryStateStoreError::RetryableConflict {
            path: "changed".into(),
        },
        RepositoryStateStoreError::UnsafeTarget("unsafe".into()),
    ];

    assert!(is_advisory_permission_denied(
        &RepositoryStateStoreError::Io(std::io::Error::new(ErrorKind::PermissionDenied, "denied",))
    ));
    assert!(errors
        .iter()
        .all(|error| !is_advisory_permission_denied(error)));
}

#[cfg(unix)]
#[test]
fn test_json_apply_retries_when_advisory_unreadable_listing_becomes_readable() {
    use std::os::unix::fs::PermissionsExt as _;

    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let secret = worktree.path().join("notes/secret");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(secret.join("citation.md"), b"citation").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();

    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let secret_path = VirtualPath::worktree("notes/secret").unwrap();
    let published_path = VirtualPath::worktree("published.txt").unwrap();
    let spec = || {
        let mut spec = CaptureSpec::phase_one([], budget()).unwrap();
        spec.discover_advisory_paths([secret_path.clone()]).unwrap();
        spec.discover_paths([published_path.clone()]).unwrap();
        spec.discover_listing(secret_path.clone()).unwrap();
        spec
    };
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            published_path.clone(),
            "archive",
            ExpectedPreimage::Absent,
            b"published".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    let mut session = storage.open_mutation_session(layout).unwrap();
    let unreadable = session.capture(spec()).unwrap();
    assert!(unreadable.listing_fingerprints()[&secret_path].is_advisory_unreadable());
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(matches!(
        session.apply(&test_plan(&unreadable, &delta)),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    assert!(!worktree.path().join("published.txt").exists());

    let readable = session.capture(spec()).unwrap();
    assert!(!readable.listing_fingerprints()[&secret_path].is_advisory_unreadable());
    session.apply(&test_plan(&readable, &delta)).unwrap();
    assert_eq!(
        std::fs::read(worktree.path().join("published.txt")).unwrap(),
        b"published"
    );
}

#[cfg(windows)]
#[test]
fn test_windows_metadata_identity_distinguishes_equal_size_files_and_directories() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::write(&first, b"same").unwrap();
    std::fs::write(&second, b"same").unwrap();
    assert_ne!(
        std_metadata_identity(&std::fs::metadata(&first).unwrap()).unwrap(),
        std_metadata_identity(&std::fs::metadata(&second).unwrap()).unwrap()
    );

    let first_dir = temp.path().join("first-dir");
    let second_dir = temp.path().join("second-dir");
    std::fs::create_dir(&first_dir).unwrap();
    std::fs::create_dir(&second_dir).unwrap();
    let first_cap = Dir::open_ambient_dir(&first_dir, ambient_authority()).unwrap();
    let second_cap = Dir::open_ambient_dir(&second_dir, ambient_authority()).unwrap();
    assert_ne!(
        capability_metadata_identity(&first_cap.dir_metadata().unwrap()).unwrap(),
        capability_metadata_identity(&second_cap.dir_metadata().unwrap()).unwrap()
    );
}

fn budget() -> CaptureBudget {
    CaptureBudget {
        max_paths: 32,
        max_listings: 8,
        max_bytes: 1024 * 1024,
        max_depth: 8,
    }
}

fn initial_spec() -> CaptureSpec {
    let mut spec = CaptureSpec::phase_one(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("index.json").unwrap(),
        ],
        budget(),
    )
    .unwrap();
    spec.discover_paths([VirtualPath::worktree("note.txt").unwrap()])
        .unwrap();
    spec
}

fn initialization_delta(layout: &RepositoryLayout) -> RepositoryDelta {
    RepositoryDelta::new(
        layout,
        vec![
            RepositoryAction::create_directory(
                VirtualPath::data("").unwrap(),
                "init",
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("index.json").unwrap(),
                "init",
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                VirtualPath::worktree("note.txt").unwrap(),
                "init",
                ExpectedPreimage::Absent,
                b"note".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap()
}

/// Close a hand-built test delta through the same complete semantic hash
/// contract as production materialization plans.
fn test_plan(image: &RepositoryImage, delta: &RepositoryDelta) -> MaterializationPlan {
    let seed = RepositorySeed::new(
        RepositorySeedKind::Command {
            name: "repository-state-store-test".to_string(),
        },
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();
    MaterializationPlan::new(
        image,
        &seed,
        &MaterializationIntent::SemanticMutation,
        delta.clone(),
    )
    .unwrap()
}

/// A fresh-init delta that publishes one issue record plus the enclosing
/// data root, `index.json`, and `issues/` directory, using the canonical
/// issue serializer so the published bytes match every other mutation path.
fn issue_creation_delta(
    layout: &RepositoryLayout,
    issue: &Issue,
    issue_bytes: &[u8],
) -> RepositoryDelta {
    RepositoryDelta::new(
        layout,
        vec![
            RepositoryAction::create_directory(
                VirtualPath::data("").unwrap(),
                "init",
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("index.json").unwrap(),
                "init",
                ExpectedPreimage::Absent,
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 2,
                    "all_ids": [issue.id],
                    "deleted_ids": [],
                }))
                .unwrap(),
                FileMode::Regular,
            ),
            RepositoryAction::create_directory(
                VirtualPath::data("issues").unwrap(),
                "issue",
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data(format!("issues/{}.json", issue.id)).unwrap(),
                "issue",
                ExpectedPreimage::Absent,
                issue_bytes.to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap()
}

fn issue_creation_spec(issue_id: &str) -> CaptureSpec {
    CaptureSpec::phase_one(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("index.json").unwrap(),
            VirtualPath::data("issues").unwrap(),
            VirtualPath::data(format!("issues/{issue_id}.json")).unwrap(),
        ],
        budget(),
    )
    .unwrap()
}

/// Crossing regression: an issue published by a mutation session reads back
/// as the identical typed record through `load_issue`/`list_issues` on both
/// backends. Wave-3 session tests never crossed apply → typed read, so a
/// divergence between the published bytes and the reader's deserialization
/// would have gone unnoticed; this pins that parity.
#[test]
fn test_session_published_issue_reads_back_typed_on_both_backends() {
    let issue = crate::domain::types::fixture_issue("Crossing".to_string(), "body".to_string());
    let issue_bytes = serialize_issue(&issue).unwrap();

    // Memory backend.
    let memory = InMemoryStorage::new();
    let mtemp = TempDir::new().unwrap();
    let mlayout = discover_repository_layout(mtemp.path(), mtemp.path().join(".jit")).unwrap();
    let mut msession = memory.open_mutation_session(mlayout.clone()).unwrap();
    let mimage = msession.capture(issue_creation_spec(&issue.id)).unwrap();
    msession
        .apply(&test_plan(
            &mimage,
            &issue_creation_delta(&mlayout, &issue, &issue_bytes),
        ))
        .unwrap();
    assert_eq!(memory.load_issue(&issue.id).unwrap(), issue);
    assert_eq!(memory.list_issues().unwrap(), vec![issue.clone()]);
    assert_eq!(
        memory.resolve_issue_id(&issue.short_id()).unwrap(),
        issue.id
    );

    // JSON backend.
    let jtemp = TempDir::new().unwrap();
    let jdata = jtemp.path().join(".jit");
    let jlayout = discover_repository_layout(jtemp.path(), &jdata).unwrap();
    let json = JsonFileStorage::new(&jdata);
    let mut jsession = json.open_mutation_session(jlayout.clone()).unwrap();
    let jimage = jsession.capture(issue_creation_spec(&issue.id)).unwrap();
    jsession
        .apply(&test_plan(
            &jimage,
            &issue_creation_delta(&jlayout, &issue, &issue_bytes),
        ))
        .unwrap();
    assert_eq!(json.load_issue(&issue.id).unwrap(), issue);
    assert_eq!(json.list_issues().unwrap(), vec![issue]);
}

#[derive(Default)]
struct SelectedFailures(Mutex<HashSet<TransactionFailurePoint>>);

impl SelectedFailures {
    fn one(point: TransactionFailurePoint) -> Arc<Self> {
        Arc::new(Self(Mutex::new(HashSet::from([point]))))
    }

    fn is_consumed(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }
}

impl crate::storage::TransactionFailureInjector for SelectedFailures {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        // Fire once, then clear, so a later recovery on the same in-memory
        // instance (whose state lives with the storage, not on disk) proceeds
        // with a clean boundary instead of re-tripping the injected point.
        if self.0.lock().unwrap().remove(point) {
            Err(std::io::Error::other(format!("injected {point:?}")))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct RecordingFailures(Mutex<Vec<TransactionFailurePoint>>);

impl RecordingFailures {
    fn observed(&self) -> Vec<TransactionFailurePoint> {
        self.0.lock().unwrap().clone()
    }
}

impl crate::storage::TransactionFailureInjector for RecordingFailures {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        self.0.lock().unwrap().push(point.clone());
        Ok(())
    }
}

struct RecordingSelectedFailure(
    Mutex<(
        Option<TransactionFailurePoint>,
        Vec<TransactionFailurePoint>,
    )>,
);

impl RecordingSelectedFailure {
    fn one(point: TransactionFailurePoint) -> Arc<Self> {
        Arc::new(Self(Mutex::new((Some(point), Vec::new()))))
    }

    fn is_consumed(&self) -> bool {
        self.0.lock().unwrap().0.is_none()
    }

    fn observed(&self) -> Vec<TransactionFailurePoint> {
        self.0.lock().unwrap().1.clone()
    }
}

impl crate::storage::TransactionFailureInjector for RecordingSelectedFailure {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.1.push(point.clone());
        if state.0.as_ref() == Some(point) {
            state.0 = None;
            Err(std::io::Error::other(format!("injected {point:?}")))
        } else {
            Ok(())
        }
    }
}

struct HookAt {
    point: TransactionFailurePoint,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl crate::storage::TransactionFailureInjector for HookAt {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &self.point {
            if let Some(hook) = self.hook.lock().unwrap().take() {
                hook();
            }
        }
        Ok(())
    }
}

struct HookThenFail {
    hook_point: TransactionFailurePoint,
    failure_point: TransactionFailurePoint,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl crate::storage::TransactionFailureInjector for HookThenFail {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &self.hook_point {
            if let Some(hook) = self.hook.lock().unwrap().take() {
                hook();
            }
        }
        if point == &self.failure_point {
            Err(std::io::Error::other(format!("injected {point:?}")))
        } else {
            Ok(())
        }
    }
}

struct OccupyDataRoot {
    path: PathBuf,
    fired: std::sync::atomic::AtomicBool,
}

impl crate::storage::TransactionFailureInjector for OccupyDataRoot {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &TransactionFailurePoint::RepositoryBeforeDataRootPublication
            && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            std::fs::create_dir(&self.path)?;
        }
        Ok(())
    }
}

#[test]
fn test_json_and_memory_share_absent_root_actions_and_hash() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let delta = initialization_delta(&layout);

    let memory = InMemoryStorage::new();
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(initial_spec()).unwrap();
    let memory_outcome = memory_session
        .apply(&test_plan(&memory_image, &delta))
        .unwrap();

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout).unwrap();
    let json_image = json_session.capture(initial_spec()).unwrap();
    let json_outcome = json_session.apply(&test_plan(&json_image, &delta)).unwrap();

    assert_eq!(memory_outcome, json_outcome);
    assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
    assert_eq!(
        std::fs::read(temp.path().join("note.txt")).unwrap(),
        b"note"
    );
}

fn image_binding_spec() -> CaptureSpec {
    CaptureSpec::phase_one(
        [
            VirtualPath::data("target.txt").unwrap(),
            VirtualPath::data("evidence.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap()
}

fn image_binding_delta(layout: &RepositoryLayout) -> RepositoryDelta {
    RepositoryDelta::new(
        layout,
        vec![RepositoryAction::write_file(
            VirtualPath::data("target.txt").unwrap(),
            "image-binding-test",
            ExpectedPreimage::Absent,
            b"materialized".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap()
}

#[test]
fn test_json_plan_owns_exact_image_and_honest_plan_returns_own_hash() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("evidence.txt"), b"image-a").unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let delta = image_binding_delta(&layout);
    let json_a = JsonFileStorage::new(&data);
    let image_a = {
        let mut session = json_a.open_mutation_session(layout.clone()).unwrap();
        session.capture(image_binding_spec()).unwrap()
    };
    let plan_a = test_plan(&image_a, &delta);
    assert_eq!(plan_a.image(), &image_a);

    std::fs::write(data.join("evidence.txt"), b"image-b").unwrap();
    let json = JsonFileStorage::new(&data);
    let mut session = json.open_mutation_session(layout).unwrap();
    let image_b = session.capture(image_binding_spec()).unwrap();
    let target = VirtualPath::data("target.txt").unwrap();
    assert_eq!(
        image_a.entry(&target).unwrap(),
        image_b.entry(&target).unwrap()
    );
    assert_ne!(plan_a.image(), &image_b);
    assert!(matches!(
        session.apply(&plan_a),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    let plan_b = test_plan(&image_b, &delta);
    let outcome = session.apply(&plan_b).unwrap();
    assert_eq!(outcome.transaction_hash, plan_b.hash());
}

#[test]
fn test_memory_plan_owns_exact_image_and_honest_plan_returns_own_hash() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "wt", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
    )
    .unwrap();
    let evidence = VirtualPath::data("evidence.txt").unwrap();
    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[(evidence.clone(), b"image-a")]);
    let delta = image_binding_delta(&layout);
    let image_a = {
        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        session.capture(image_binding_spec()).unwrap()
    };
    let plan_a = test_plan(&image_a, &delta);
    assert_eq!(plan_a.image(), &image_a);

    seed_memory_existing(&memory, &[(evidence, b"image-b")]);
    let mut session = memory.open_mutation_session(layout).unwrap();
    let image_b = session.capture(image_binding_spec()).unwrap();
    let target = VirtualPath::data("target.txt").unwrap();
    assert_eq!(
        image_a.entry(&target).unwrap(),
        image_b.entry(&target).unwrap()
    );
    assert_ne!(plan_a.image(), &image_b);
    assert!(matches!(
        session.apply(&plan_a),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    let plan_b = test_plan(&image_b, &delta);
    let outcome = session.apply(&plan_b).unwrap();
    assert_eq!(outcome.transaction_hash, plan_b.hash());
}

#[test]
fn test_interruption_boundaries_recover_complete_old_or_new_state() {
    let points = [
        TransactionFailurePoint::RepositoryPrepareIntent,
        TransactionFailurePoint::RepositoryPrepareAction { action: 0 },
        TransactionFailurePoint::RepositorySyncPreparedJournal,
        TransactionFailurePoint::RepositoryBeforeAction { action: 0 },
        TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        TransactionFailurePoint::RepositoryBeforeDataRootPublication,
        TransactionFailurePoint::RepositoryAfterDataRootPublication,
        TransactionFailurePoint::RepositoryAfterCommit,
        TransactionFailurePoint::RepositoryCleanup,
    ];
    for point in points {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(point.clone()),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(initial_spec()).unwrap();
        let delta = initialization_delta(&layout);
        assert!(session.apply(&test_plan(&image, &delta)).is_err());
        drop(session);

        let recovered_layout = discover_repository_layout(temp.path(), &data).unwrap();
        let clean = JsonFileStorage::new(&data);
        let _recovered = clean
            .open_mutation_session(recovered_layout)
            .unwrap_or_else(|error| panic!("recovery failed at {point:?}: {error:#}"));
        let root_exists = data.exists();
        assert_eq!(
            temp.path().join("note.txt").exists(),
            root_exists,
            "{point:?}"
        );
        assert_eq!(data.join("index.json").exists(), root_exists, "{point:?}");
    }
}

#[test]
fn test_session_reports_recovered_transactions() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryCleanup),
    );
    let mut session = storage.open_mutation_session(layout).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    let delta = initialization_delta(image.layout());
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    let recovered_layout = discover_repository_layout(temp.path(), &data).unwrap();
    let recovered = JsonFileStorage::new(&data)
        .open_mutation_session(recovered_layout)
        .unwrap();
    assert_eq!(recovered.recovery_report().recovered_count(), 1);
    assert_eq!(recovered.recovery_report().external_transactions.len(), 1);
    assert!(recovered.recovery_report().internal_transactions.is_empty());
}

#[test]
fn test_session_reclaims_empty_external_and_internal_control_roots() {
    let markerless = TempDir::new().unwrap();
    let markerless_data = markerless.path().join(".jit");
    std::fs::create_dir(markerless.path().join(".jit-bootstrap")).unwrap();
    let layout = discover_repository_layout(markerless.path(), &markerless_data).unwrap();
    drop(
        JsonFileStorage::new(&markerless_data)
            .open_mutation_session(layout)
            .unwrap(),
    );
    assert!(!markerless.path().join(".jit-bootstrap").exists());

    let external = TempDir::new().unwrap();
    let external_data = external.path().join(".jit");
    std::fs::create_dir_all(external.path().join(".jit-bootstrap/transactions")).unwrap();
    std::fs::write(
        external
            .path()
            .join(".jit-bootstrap/transaction-protocol-v1"),
        b"1\n",
    )
    .unwrap();
    let layout = discover_repository_layout(external.path(), &external_data).unwrap();
    drop(
        JsonFileStorage::new(&external_data)
            .open_mutation_session(layout)
            .unwrap(),
    );
    assert!(!external.path().join(".jit-bootstrap").exists());

    let internal = TempDir::new().unwrap();
    let internal_data = internal.path().join(".jit");
    std::fs::create_dir_all(internal_data.join("tmp/transactions")).unwrap();
    let layout = discover_repository_layout(internal.path(), &internal_data).unwrap();
    drop(
        JsonFileStorage::new(&internal_data)
            .open_mutation_session(layout)
            .unwrap(),
    );
    assert!(!internal_data.join("tmp").exists());
}

#[test]
fn test_backup_sync_interruption_recovers_original_file() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("replace.txt"), b"original").unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositorySyncBackup { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("replace.txt").unwrap();
    let spec = CaptureSpec::phase_one([path.clone()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            path,
            "replace",
            ExpectedPreimage::of(
                image
                    .entry(&VirtualPath::data("replace.txt").unwrap())
                    .unwrap(),
            ),
            b"replacement".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    drop(
        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap(),
    );
    assert_eq!(
        std::fs::read(data.join("replace.txt")).unwrap(),
        b"original"
    );
    assert!(!data.join("tmp/transactions").exists());
}

#[cfg(unix)]
#[test]
fn test_rollback_backup_is_independent_of_mutated_original_inode() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let target = data.join("a.txt");
    let alias = temp.path().join("outside-alias");
    std::fs::write(&target, b"original").unwrap();
    std::fs::hard_link(&target, &alias).unwrap();
    let hook_alias = alias.clone();
    let injector = Arc::new(HookThenFail {
        hook_point: TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        failure_point: TransactionFailurePoint::RepositoryBeforeAction { action: 1 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::write(&hook_alias, b"mutated through alias").unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let first = VirtualPath::data("a.txt").unwrap();
    let second = VirtualPath::data("b.txt").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([first.clone(), second.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![
            RepositoryAction::write_file(
                first.clone(),
                "replace",
                ExpectedPreimage::of(image.entry(&first).unwrap()),
                b"replacement".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                second,
                "later",
                ExpectedPreimage::Absent,
                b"later".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap();

    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);
    drop(
        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap(),
    );

    assert_eq!(std::fs::read(target).unwrap(), b"original");
    assert_eq!(std::fs::read(alias).unwrap(), b"mutated through alias");
    assert!(!data.join("b.txt").exists());
    assert!(!data.join("tmp/transactions").exists());
}

#[cfg(unix)]
#[test]
fn test_set_mode_replaces_inode_without_chmodding_external_hard_link() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let target = data.join("mode.txt");
    let alias = outside.path().join("alias.txt");
    std::fs::write(&target, b"same bytes").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::hard_link(&target, &alias).unwrap();
    let original_inode = std::fs::metadata(&target).unwrap().ino();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("mode.txt").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::set_mode(
            path,
            "mode",
            ExpectedPreimage::of(
                image
                    .entry(&VirtualPath::data("mode.txt").unwrap())
                    .unwrap(),
            ),
            FileMode::Executable,
        )],
    )
    .unwrap();

    session.apply(&test_plan(&image, &delta)).unwrap();

    let target_metadata = std::fs::metadata(&target).unwrap();
    let alias_metadata = std::fs::metadata(&alias).unwrap();
    assert_ne!(target_metadata.ino(), original_inode);
    assert_eq!(alias_metadata.ino(), original_inode);
    assert_ne!(target_metadata.permissions().mode() & 0o111, 0);
    assert_eq!(alias_metadata.permissions().mode() & 0o111, 0);
    assert_eq!(std::fs::read(target).unwrap(), b"same bytes");
    assert_eq!(std::fs::read(alias).unwrap(), b"same bytes");
}

#[test]
fn test_rollback_reverse_action_interruption_remains_recoverable() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let raced = data.join("a.txt");
    let hook_raced = raced.clone();
    let injector = Arc::new(HookThenFail {
        hook_point: TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
        failure_point: TransactionFailurePoint::RepositoryAfterReverseAction { action: 1 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::write(&hook_raced, b"bystander").unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let paths = [
        VirtualPath::data("a.txt").unwrap(),
        VirtualPath::data("b.txt").unwrap(),
    ];
    let spec = CaptureSpec::phase_one(paths.clone(), budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        paths
            .into_iter()
            .map(|path| {
                RepositoryAction::write_file(
                    path,
                    "rollback",
                    ExpectedPreimage::Absent,
                    b"planned".to_vec(),
                    FileMode::Regular,
                )
            })
            .collect(),
    )
    .unwrap();

    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);
    assert!(data.join("tmp/transactions").exists());
    assert_eq!(std::fs::read(&raced).unwrap(), b"bystander");

    std::fs::remove_file(&raced).unwrap();
    drop(
        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap(),
    );
    assert!(!data.join("a.txt").exists());
    assert!(!data.join("b.txt").exists());
    assert!(!data.join("tmp/transactions").exists());
}

#[test]
fn test_delete_recovery_refuses_post_crash_occupant() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("victim"), b"old").unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("victim").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected =
        ExpectedPreimage::of(image.entry(&VirtualPath::data("victim").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::delete_file(
            VirtualPath::data("victim").unwrap(),
            "delete",
            expected,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);
    std::fs::write(data.join("victim"), b"new occupant").unwrap();

    let clean_storage = JsonFileStorage::new(&data);
    let recovery = clean_storage.open_mutation_session(layout);
    assert!(recovery.is_err());
    assert_eq!(std::fs::read(data.join("victim")).unwrap(), b"new occupant");
}

#[test]
fn test_delete_restores_occupant_raced_in_before_atomic_rename() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let victim = data.join("victim");
    let moved = data.join("victim-original");
    std::fs::write(&victim, b"old").unwrap();
    let hook_victim = victim.clone();
    let hook_moved = moved.clone();
    let injector = Arc::new(HookAt {
        point: TransactionFailurePoint::RepositoryBeforeDeleteRename { action: 0 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::rename(&hook_victim, &hook_moved).unwrap();
            std::fs::write(&hook_victim, b"new occupant").unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("victim").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected =
        ExpectedPreimage::of(image.entry(&VirtualPath::data("victim").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::delete_file(
            VirtualPath::data("victim").unwrap(),
            "delete",
            expected,
        )],
    )
    .unwrap();

    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"new occupant");
    assert_eq!(std::fs::read(&moved).unwrap(), b"old");
}

#[test]
fn test_write_replacement_restores_occupant_raced_in_before_atomic_rename() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let target = data.join("target");
    let original = data.join("target-original");
    std::fs::write(&target, b"old").unwrap();
    let hook_target = target.clone();
    let hook_original = original.clone();
    let injector = Arc::new(HookAt {
        point: TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::rename(&hook_target, &hook_original).unwrap();
            std::fs::write(&hook_target, b"bystander").unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("target").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            path.clone(),
            "replace",
            ExpectedPreimage::of(image.entry(&path).unwrap()),
            b"planned".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"bystander");
    assert_eq!(std::fs::read(original).unwrap(), b"old");
}

#[test]
fn test_missing_data_stage_identity_fails_closed_during_recovery() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryBeforeDataStageJournal),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("index.json").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            path,
            "initialize",
            ExpectedPreimage::Absent,
            b"{}".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);
    assert!(JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .is_err());
    assert!(!data.exists());
    assert!(temp
        .path()
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("jit-stage-")));
}

#[test]
fn test_same_storage_reuses_its_own_absent_root_publication_evidence() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let stale_layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    storage.configure_repository_layout(&stale_layout);
    let path = VirtualPath::data("index.json").unwrap();
    let mut first = storage.open_mutation_session(stale_layout.clone()).unwrap();
    let image = first
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &stale_layout,
        vec![RepositoryAction::write_file(
            path.clone(),
            "initialize",
            ExpectedPreimage::Absent,
            b"{}".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    first.apply(&test_plan(&image, &delta)).unwrap();
    drop(first);

    let mut second = storage.open_mutation_session(stale_layout).unwrap();
    let refreshed = second
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    assert!(matches!(
        refreshed.entry(&path),
        Ok(RepositoryEntry::File { bytes, .. }) if bytes == b"{}"
    ));
}

#[test]
fn test_no_replace_root_race_preserves_occupant_and_rolls_back_worktree() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        Arc::new(OccupyDataRoot {
            path: data.clone(),
            fired: std::sync::atomic::AtomicBool::new(false),
        }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    let delta = initialization_delta(&layout);
    // The occupied destination fails with the typed occupied-data-root error,
    // not merely some error.
    assert!(matches!(
        session.apply(&test_plan(&image, &delta)),
        Err(RepositoryStateStoreError::OccupiedDataRoot { .. })
    ));
    assert!(data.is_dir());
    assert!(!data.join("index.json").exists());
    assert!(!temp.path().join("note.txt").exists());
}

#[test]
fn test_journal_rejects_traversal_control_name() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let transaction = data.join("tmp/transactions/bad");
    std::fs::create_dir_all(transaction.join("stages")).unwrap();
    std::fs::create_dir(transaction.join("backups")).unwrap();
    std::fs::write(
            transaction.join("journal.json"),
            br#"{"version":3,"transaction_id":"bad","layout_digest":"x","plan_hash":"x","data_root_was_absent":false,"data_stage":"../escape","data_stage_identity":null,"decision":"prepared","actions":[]}"#,
        )
        .unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    assert!(JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .is_err());
    assert!(transaction.exists());
}

#[test]
fn test_recovery_refuses_pre_cutover_journal_with_removed_action_progress() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("victim"), b"old").unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("victim").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let expected = ExpectedPreimage::of(image.entry(&path).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::delete_file(path, "delete", expected)],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    let transaction = std::fs::read_dir(data.join("tmp/transactions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal_path = transaction.join("journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&journal_path).unwrap()).unwrap();
    journal["version"] = serde_json::json!(2);
    journal["actions"][0]["progress"] = serde_json::json!("planned");
    std::fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();

    let recovery = JsonFileStorage::new(&data).open_mutation_session(layout);
    assert!(matches!(
        &recovery,
        Err(RepositoryStateStoreError::Transaction(error))
            if error.downcast_ref::<serde_json::Error>().is_some()
    ));
    assert!(transaction.is_dir());
}

#[test]
fn test_recovery_refuses_a_recoverable_journal_at_the_superseded_version() {
    // The companion test above reintroduces the removed per-action progress
    // field, so its refusal comes from parsing. Here the journal is one recovery
    // would otherwise complete — a real interrupted transaction, its layout
    // digest and actions untouched — and only its version is moved back. That
    // isolates the version boundary: the record parses, and the superseded
    // version alone refuses it.
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("victim"), b"old").unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("victim").unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([path.clone()], budget()).unwrap())
        .unwrap();
    let expected = ExpectedPreimage::of(image.entry(&path).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::delete_file(path, "delete", expected)],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    let transaction = std::fs::read_dir(data.join("tmp/transactions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let journal_path = transaction.join("journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&journal_path).unwrap()).unwrap();
    journal["version"] = serde_json::json!(2);
    std::fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();

    let recovery = JsonFileStorage::new(&data).open_mutation_session(layout);

    // Parsing succeeded, so the refusal is the version boundary's and not a
    // deserialization accident.
    assert!(matches!(
        &recovery,
        Err(RepositoryStateStoreError::Transaction(error))
            if error.downcast_ref::<serde_json::Error>().is_none()
    ));
    assert!(transaction.is_dir());
}

#[cfg(unix)]
#[test]
fn test_control_path_symlink_is_never_followed() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let outside = temp.path().join("outside");
    let data = temp.path().join(".jit");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(&data).unwrap();
    symlink(&outside, data.join("tmp")).unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    assert!(JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .is_err());
    assert!(std::fs::read_dir(outside).unwrap().next().is_none());
}

#[test]
fn test_publishing_an_absent_data_root_admits_the_next_session_under_a_retained_one() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let storage = JsonFileStorage::new(&data);
    // The layout a repository is created through: the selected data root does
    // not exist yet, so its evidence is the parent it will be published under.
    let absent = discover_repository_layout(temp.path(), &data).unwrap();
    let retained = storage
        .open_and_retain_mutation_session(absent.clone())
        .unwrap();

    let mut session = storage.open_mutation_session(absent.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    session
        .apply(&test_plan(&image, &initialization_delta(&absent)))
        .unwrap();
    drop(session);

    // The published root is what the repository is at from here on, and the
    // roots the retained session selected are the ones it was selecting all
    // along, so the next session over that repository opens through it.
    let published = discover_repository_layout(temp.path(), &data).unwrap();
    assert_ne!(published.data_identity(), absent.data_identity());
    let next = storage.open_mutation_session(absent).unwrap();
    assert_eq!(next.layout().data_identity(), published.data_identity());
    drop(next);
    drop(retained);
}

#[test]
fn test_shared_retained_lock_chain_reenters_same_layout() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let _bootstrap = storage.acquire_bootstrap_write_lock().unwrap();
    let _repository = storage.acquire_repo_write_lock_raw().unwrap();
    let session = storage.open_mutation_session(layout).unwrap();
    assert_eq!(session.layout().data_root(), data);
}

#[test]
fn test_memory_prepared_residue_recovers_original_aggregate_state() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
    )
    .unwrap();
    let storage = InMemoryStorage::with_repository_state_failures(SelectedFailures::one(
        TransactionFailurePoint::RepositoryAfterAction { action: 0 },
    ));
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    let delta = initialization_delta(&layout);
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    let mut recovered = storage.open_mutation_session(layout).unwrap();
    let image = recovered.capture(initial_spec()).unwrap();
    assert!(matches!(
        image.entry(&VirtualPath::worktree("note.txt").unwrap()),
        Ok(RepositoryEntry::Absent)
    ));
    assert!(matches!(
        image.entry(&VirtualPath::data("index.json").unwrap()),
        Ok(RepositoryEntry::Absent)
    ));
}

fn barrier_spec() -> CaptureSpec {
    let mut spec = CaptureSpec::phase_one(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("record.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap();
    spec.discover_paths([
        VirtualPath::worktree("first.txt").unwrap(),
        VirtualPath::worktree("second.txt").unwrap(),
    ])
    .unwrap();
    spec
}

fn seed_memory_barrier_fixture(storage: &InMemoryStorage, existing_data_root: bool) {
    let mut state = storage.repository_state();
    state.data_root_exists = existing_data_root;
    if existing_data_root {
        state.entries.insert(
            VirtualPath::data("").unwrap(),
            RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("memory:data-root", b"directory").unwrap(),
                mode: FileMode::Executable,
            },
        );
        state.entries.insert(
            VirtualPath::data("record.txt").unwrap(),
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes("memory:data-record", b"old-data").unwrap(),
                bytes: b"old-data".to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
    for (name, bytes) in [
        ("first.txt", b"old-first".as_slice()),
        ("second.txt", b"old-second".as_slice()),
    ] {
        let path = VirtualPath::worktree(name).unwrap();
        state.entries.insert(
            path.clone(),
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes(format!("memory:{path:?}"), bytes).unwrap(),
                bytes: bytes.to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
}

fn prepare_memory_barrier_rollback(
    existing_data_root: bool,
) -> (TempDir, RepositoryLayout, InMemoryStorage) {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
    )
    .unwrap();
    let interruption =
        SelectedFailures::one(TransactionFailurePoint::RepositoryBeforeAction { action: 1 });
    let storage = InMemoryStorage::with_repository_state_failures(interruption.clone());
    seed_memory_barrier_fixture(&storage, existing_data_root);

    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(barrier_spec()).unwrap();
    let delta = barrier_delta(&layout, &image, BarrierDeltaKind::Replace);
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert!(interruption.is_consumed());
    drop(session);
    assert!(matches!(
        storage.repository_state().recovery,
        Some(MemoryRecoveryResidue::Prepared { .. })
    ));
    (temp, layout, storage)
}

fn rollback_points(
    points: impl IntoIterator<Item = TransactionFailurePoint>,
) -> Vec<TransactionFailurePoint> {
    points
        .into_iter()
        .filter(|point| {
            matches!(
                point,
                TransactionFailurePoint::RepositoryBeforeReverseAction { .. }
                    | TransactionFailurePoint::RepositoryAfterReverseAction { .. }
                    | TransactionFailurePoint::RepositoryBeforeStageCleanup
                    | TransactionFailurePoint::RepositoryBeforeRollbackDecision
            )
        })
        .collect()
}

#[test]
fn test_memory_prepared_recovery_reports_reverse_live_action_order_for_both_root_shapes() {
    use TransactionFailurePoint::{
        RepositoryAfterReverseAction as After, RepositoryBeforeReverseAction as Before,
        RepositoryBeforeRollbackDecision as Decision, RepositoryBeforeStageCleanup as Cleanup,
    };

    for existing_data_root in [true, false] {
        let (_temp, layout, storage) = prepare_memory_barrier_rollback(existing_data_root);
        let recording = Arc::new(RecordingFailures::default());
        let recovered = storage.with_repository_state_failure_view(recording.clone());
        drop(recovered.open_mutation_session(layout).unwrap());

        let expected = if existing_data_root {
            vec![
                Before { action: 2 },
                After { action: 2 },
                Before { action: 1 },
                After { action: 1 },
                Before { action: 0 },
                After { action: 0 },
                Cleanup,
                Decision,
            ]
        } else {
            // The absent-root Data action at index 2 was staged and is not a
            // live action, so recovery mirrors the file kernel and skips it.
            vec![
                Before { action: 1 },
                After { action: 1 },
                Before { action: 0 },
                After { action: 0 },
                Cleanup,
                Decision,
            ]
        };
        assert_eq!(rollback_points(recording.observed()), expected);
        assert!(storage.repository_state().recovery.is_none());
    }
}

#[test]
fn test_memory_rollback_edge_failure_retains_prepared_residue_for_retry() {
    for existing_data_root in [true, false] {
        for point in [
            TransactionFailurePoint::RepositoryBeforeReverseAction { action: 1 },
            TransactionFailurePoint::RepositoryAfterReverseAction { action: 1 },
            TransactionFailurePoint::RepositoryBeforeStageCleanup,
            TransactionFailurePoint::RepositoryBeforeRollbackDecision,
        ] {
            let (_temp, layout, storage) = prepare_memory_barrier_rollback(existing_data_root);
            let failure = SelectedFailures::one(point.clone());
            let recovering = storage.with_repository_state_failure_view(failure.clone());
            assert!(recovering.open_mutation_session(layout.clone()).is_err());
            assert!(
                failure.is_consumed(),
                "rollback point did not fire: {point:?}, existing_data_root={existing_data_root}"
            );
            assert!(matches!(
                storage.repository_state().recovery,
                Some(MemoryRecoveryResidue::Prepared { .. })
            ));

            let mut recovered = recovering.open_mutation_session(layout).unwrap();
            let image = recovered.capture(barrier_spec()).unwrap();
            assert!(matches!(
                image
                    .entry(&VirtualPath::worktree("first.txt").unwrap())
                    .unwrap(),
                RepositoryEntry::File { bytes, .. } if bytes == b"old-first"
            ));
            let data_record = image
                .entry(&VirtualPath::data("record.txt").unwrap())
                .unwrap();
            if existing_data_root {
                assert!(matches!(
                    data_record,
                    RepositoryEntry::File { bytes, .. } if bytes == b"old-data"
                ));
            } else {
                assert!(matches!(data_record, RepositoryEntry::Absent));
            }
            assert!(storage.repository_state().recovery.is_none());
        }
    }
}

#[test]
fn test_memory_committed_recovery_stays_forward_without_rollback_edges() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
    )
    .unwrap();
    let committed = SelectedFailures::one(TransactionFailurePoint::RepositoryAfterCommit);
    let storage = InMemoryStorage::with_repository_state_failures(committed.clone());
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    assert!(session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .is_err());
    assert!(committed.is_consumed());
    drop(session);

    let recording = Arc::new(RecordingFailures::default());
    let recovered = storage.with_repository_state_failure_view(recording.clone());
    let mut session = recovered.open_mutation_session(layout).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    assert!(matches!(
        image
            .entry(&VirtualPath::worktree("note.txt").unwrap())
            .unwrap(),
        RepositoryEntry::File { bytes, .. } if bytes == b"note"
    ));
    assert!(rollback_points(recording.observed()).is_empty());
    assert!(storage.repository_state().recovery.is_none());
}

#[test]
fn test_memory_set_mode_uses_exact_file_preimage() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
    )
    .unwrap();
    let storage = InMemoryStorage::new();
    {
        let mut state = storage.repository_state();
        let path = VirtualPath::data("tool").unwrap();
        state.data_root_exists = true;
        state.entries.insert(
            path,
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes("memory-tool", b"tool").unwrap(),
                bytes: b"tool".to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("tool").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let path = VirtualPath::data("tool").unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::set_mode(
            path.clone(),
            "mode",
            ExpectedPreimage::of(image.entry(&path).unwrap()),
            FileMode::Executable,
        )],
    )
    .unwrap();
    session.apply(&test_plan(&image, &delta)).unwrap();
    drop(session);
    let state = storage.repository_state();
    assert!(matches!(
        state.entries.get(&path),
        Some(RepositoryEntry::File {
            mode: FileMode::Executable,
            ..
        })
    ));
}

#[cfg(unix)]
#[test]
fn test_canonical_set_mode_leaf_symlink_swap_cannot_mutate_external_target() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let target = data.join("mode.txt");
    let moved = data.join("mode-original.txt");
    std::fs::write(&target, b"inside").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let outside = TempDir::new().unwrap();
    let outside_target = outside.path().join("outside.txt");
    std::fs::write(&outside_target, b"outside").unwrap();
    std::fs::set_permissions(&outside_target, std::fs::Permissions::from_mode(0o640)).unwrap();

    let hook_target = target.clone();
    let hook_moved = moved.clone();
    let hook_outside = outside_target.clone();
    let injector = Arc::new(HookAt {
        point: TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::rename(&hook_target, &hook_moved).unwrap();
            symlink(&hook_outside, &hook_target).unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("mode.txt").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(
        image
            .entry(&VirtualPath::data("mode.txt").unwrap())
            .unwrap(),
    );
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::set_mode(
            VirtualPath::data("mode.txt").unwrap(),
            "mode",
            expected,
            FileMode::Executable,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert_eq!(
        std::fs::metadata(&outside_target)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn test_canonical_set_mode_revalidates_identity_on_mutated_handle() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let target = data.join("mode.txt");
    let moved = data.join("mode-original.txt");
    std::fs::write(&target, b"inside").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let hook_target = target.clone();
    let hook_moved = moved.clone();
    let injector = Arc::new(HookAt {
        point: TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
        hook: Mutex::new(Some(Box::new(move || {
            std::fs::rename(&hook_target, &hook_moved).unwrap();
            std::fs::write(&hook_target, b"unexpected").unwrap();
            std::fs::set_permissions(&hook_target, std::fs::Permissions::from_mode(0o640)).unwrap();
        }))),
    });
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("mode.txt").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(
        image
            .entry(&VirtualPath::data("mode.txt").unwrap())
            .unwrap(),
    );
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::set_mode(
            VirtualPath::data("mode.txt").unwrap(),
            "mode",
            expected,
            FileMode::Executable,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

// --- Cross-backend conformance matrix ------------------------------------
//
// One suite exercises identical canonical semantics on the JSON and memory
// backends: equivalent captured semantics, complete per-image result hashes,
// every action
// kind, nested and disjoint existing/absent roots, aliases, and convergence
// to a complete old/new state at every declared failure edge. Object identity
// differs by construction (device/inode versus a synthesized memory id), so
// parity is asserted over the identity-independent semantic projection.

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemanticEntry {
    Absent,
    Directory(FileMode),
    File(Vec<u8>, FileMode),
    Symlink(Vec<u8>),
    Unsupported,
}

fn semantic_entry(entry: &RepositoryEntry) -> SemanticEntry {
    match entry {
        RepositoryEntry::Absent => SemanticEntry::Absent,
        RepositoryEntry::Directory { mode, .. } => SemanticEntry::Directory(*mode),
        RepositoryEntry::File { bytes, mode, .. } => SemanticEntry::File(bytes.clone(), *mode),
        RepositoryEntry::Symlink { target, .. } => SemanticEntry::Symlink(target.clone()),
        RepositoryEntry::Unsupported { .. } => SemanticEntry::Unsupported,
    }
}

fn semantic_view(image: &RepositoryImage) -> BTreeMap<VirtualPath, SemanticEntry> {
    image
        .entries()
        .iter()
        .map(|(path, entry)| (path.clone(), semantic_entry(entry)))
        .collect()
}

#[test]
fn test_conformance_absent_root_publishes_identically_across_topologies() {
    for disjoint in [false, true] {
        let worktree = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let data = if disjoint {
            elsewhere.path().join("store")
        } else {
            worktree.path().join(".jit")
        };
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let delta = initialization_delta(&layout);

        let memory = InMemoryStorage::new();
        let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
        let memory_image = memory_session.capture(initial_spec()).unwrap();
        let memory_outcome = memory_session
            .apply(&test_plan(&memory_image, &delta))
            .unwrap();
        drop(memory_session);

        let json = JsonFileStorage::new(&data);
        let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
        let json_image = json_session.capture(initial_spec()).unwrap();
        let json_outcome = json_session.apply(&test_plan(&json_image, &delta)).unwrap();
        drop(json_session);

        // Identical all-absent capture and therefore identical complete plan
        // hash, for nested and disjoint absent roots alike.
        assert_eq!(
            semantic_view(&memory_image),
            semantic_view(&json_image),
            "capture disjoint={disjoint}"
        );
        assert_eq!(memory_outcome, json_outcome, "outcome disjoint={disjoint}");
        assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
        assert_eq!(
            std::fs::read(worktree.path().join("note.txt")).unwrap(),
            b"note"
        );
    }
}

#[cfg(unix)]
fn existing_conformance_spec() -> CaptureSpec {
    let mut spec = CaptureSpec::phase_one(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("keep.txt").unwrap(),
            VirtualPath::data("replace.txt").unwrap(),
            VirtualPath::data("remove.txt").unwrap(),
            VirtualPath::data("chmod.txt").unwrap(),
            VirtualPath::data("generated").unwrap(),
            VirtualPath::data("generated/new.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap();
    spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
        .unwrap();
    spec
}

fn seed_memory_existing(memory: &InMemoryStorage, files: &[(VirtualPath, &[u8])]) {
    let mut state = memory.repository_state();
    state.data_root_exists = true;
    state.entries.insert(
        VirtualPath::data("").unwrap(),
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("mem-data-root", b"directory").unwrap(),
            mode: FileMode::Executable,
        },
    );
    for (path, bytes) in files {
        state.entries.insert(
            path.clone(),
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes(format!("mem:{path:?}"), bytes).unwrap(),
                bytes: bytes.to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
}

#[cfg(unix)]
fn apply_all_action_kinds<S: RepositoryStateStore>(
    store: &S,
    layout: &RepositoryLayout,
) -> (RepositoryApplyOutcome, BTreeMap<VirtualPath, SemanticEntry>) {
    let mut session = store.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(existing_conformance_spec()).unwrap();
    let preimage =
        |name: &str| ExpectedPreimage::of(image.entry(&VirtualPath::data(name).unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        layout,
        vec![
            RepositoryAction::create_directory(
                VirtualPath::data("generated").unwrap(),
                "conformance",
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("generated/new.txt").unwrap(),
                "conformance",
                ExpectedPreimage::Absent,
                b"created".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("replace.txt").unwrap(),
                "conformance",
                preimage("replace.txt"),
                b"replaced".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::set_mode(
                VirtualPath::data("chmod.txt").unwrap(),
                "conformance",
                preimage("chmod.txt"),
                FileMode::Executable,
            ),
            RepositoryAction::delete_file(
                VirtualPath::data("remove.txt").unwrap(),
                "conformance",
                preimage("remove.txt"),
            ),
            RepositoryAction::write_file(
                VirtualPath::worktree("out.txt").unwrap(),
                "conformance",
                ExpectedPreimage::Absent,
                b"worktree".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap();
    let outcome = session.apply(&test_plan(&image, &delta)).unwrap();
    drop(session);

    let mut after = store.open_mutation_session(layout.clone()).unwrap();
    let post = after.capture(existing_conformance_spec()).unwrap();
    (outcome, semantic_view(&post))
}

#[cfg(unix)]
#[test]
fn test_conformance_existing_root_all_action_kinds_match_across_backends() {
    for disjoint in [false, true] {
        let worktree = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let data = if disjoint {
            elsewhere.path().join("store")
        } else {
            worktree.path().join(".jit")
        };
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("keep.txt"), b"keep").unwrap();
        std::fs::write(data.join("replace.txt"), b"old").unwrap();
        std::fs::write(data.join("remove.txt"), b"gone").unwrap();
        std::fs::write(data.join("chmod.txt"), b"exec").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        let memory = InMemoryStorage::new();
        seed_memory_existing(
            &memory,
            &[
                (VirtualPath::data("keep.txt").unwrap(), b"keep"),
                (VirtualPath::data("replace.txt").unwrap(), b"old"),
                (VirtualPath::data("remove.txt").unwrap(), b"gone"),
                (VirtualPath::data("chmod.txt").unwrap(), b"exec"),
            ],
        );
        let json = JsonFileStorage::new(&data);

        let (json_outcome, json_view) = apply_all_action_kinds(&json, &layout);
        let (memory_outcome, memory_view) = apply_all_action_kinds(&memory, &layout);

        assert_eq!(
            json_outcome.actions_applied, memory_outcome.actions_applied,
            "action count disjoint={disjoint}"
        );
        assert_eq!(json_view, memory_view, "post-state disjoint={disjoint}");
        // Every action kind reached its expected terminal state.
        assert_eq!(
            json_view[&VirtualPath::data("replace.txt").unwrap()],
            SemanticEntry::File(b"replaced".to_vec(), FileMode::Regular)
        );
        assert_eq!(
            json_view[&VirtualPath::data("chmod.txt").unwrap()],
            SemanticEntry::File(b"exec".to_vec(), FileMode::Executable)
        );
        assert_eq!(
            json_view[&VirtualPath::data("remove.txt").unwrap()],
            SemanticEntry::Absent
        );
    }
}

#[test]
fn test_conformance_worktree_only_absent_root_materializes_no_data_root() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit"); // absent
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let make_spec = || {
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("note.txt").unwrap()])
            .unwrap();
        spec
    };
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("note.txt").unwrap(),
            "wt",
            ExpectedPreimage::Absent,
            b"note".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    let memory = InMemoryStorage::new();
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(make_spec()).unwrap();
    let memory_outcome = memory_session
        .apply(&test_plan(&memory_image, &delta))
        .unwrap();
    drop(memory_session);

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(make_spec()).unwrap();
    let json_outcome = json_session.apply(&test_plan(&json_image, &delta)).unwrap();
    drop(json_session);

    // Identical complete plan hash/action count for identical absent capture,
    // and neither backend materialized a data root.
    assert_eq!(memory_outcome, json_outcome);
    assert!(!data.exists(), "json must not publish an empty data root");
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    assert_eq!(
        std::fs::read(worktree.path().join("note.txt")).unwrap(),
        b"note"
    );

    // Post-apply capture agrees on both backends: data root absent, note present.
    let mut memory_after = memory.open_mutation_session(layout).unwrap();
    let memory_view = semantic_view(&memory_after.capture(make_spec()).unwrap());
    let json_after = JsonFileStorage::new(&data);
    let mut json_after_session = json_after
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();
    let json_view = semantic_view(&json_after_session.capture(make_spec()).unwrap());
    assert_eq!(memory_view, json_view);
    assert_eq!(
        memory_view[&VirtualPath::data("").unwrap()],
        SemanticEntry::Absent
    );
}

#[test]
fn test_conformance_empty_delta_is_a_noop_on_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = || CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    let delta = RepositoryDelta::new(&layout, vec![]).unwrap();

    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[]);
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(spec()).unwrap();
    let memory_outcome = memory_session
        .apply(&test_plan(&memory_image, &delta))
        .unwrap();

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(spec()).unwrap();
    let json_outcome = json_session.apply(&test_plan(&json_image, &delta)).unwrap();

    // An empty delta is a no-op with zero actions; each result still carries
    // its complete captured-image plan hash. No residue is created.
    assert_eq!(memory_outcome.actions_applied, json_outcome.actions_applied);
    assert_eq!(json_outcome.actions_applied, 0);
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    assert!(!data.join("tmp/transactions").exists());
}

fn apply_once<S: RepositoryStateStore>(
    store: &S,
    layout: &RepositoryLayout,
    spec: CaptureSpec,
    delta: &RepositoryDelta,
) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
    let mut session = store.open_mutation_session(layout.clone())?;
    let image = session.capture(spec)?;
    session.apply(&test_plan(&image, delta))
}

#[test]
fn test_conformance_parent_existence_enforced_on_both_backends() {
    for existing_root in [false, true] {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        if existing_root {
            std::fs::create_dir(&data).unwrap();
        }
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || {
            CaptureSpec::phase_one(
                [
                    VirtualPath::data("").unwrap(),
                    VirtualPath::data("nested").unwrap(),
                    VirtualPath::data("nested/file.txt").unwrap(),
                ],
                budget(),
            )
            .unwrap()
        };

        // A nested WriteFile whose parent directory is not created fails on both
        // backends (JSON: open_parent MissingParent; memory: parent-existence
        // rule), leaving the old state untouched.
        let missing_parent = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::data("nested/file.txt").unwrap(),
                "p0",
                ExpectedPreimage::Absent,
                b"x".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        let json = JsonFileStorage::new(&data);
        assert!(apply_once(&json, &layout, spec(), &missing_parent).is_err());
        assert!(
            !data.join("nested").exists(),
            "existing_root={existing_root}"
        );
        let memory = InMemoryStorage::new();
        if existing_root {
            seed_memory_existing(&memory, &[]);
        }
        assert!(apply_once(&memory, &layout, spec(), &missing_parent).is_err());

        // The same delta with the explicit CreateDirectory parent succeeds
        // identically on both backends (fresh trees to avoid carrying residue).
        let worktree2 = TempDir::new().unwrap();
        let data2 = worktree2.path().join(".jit");
        if existing_root {
            std::fs::create_dir(&data2).unwrap();
        }
        let layout2 = discover_repository_layout(worktree2.path(), &data2).unwrap();
        let with_parent = RepositoryDelta::new(
            &layout2,
            vec![
                RepositoryAction::create_directory(
                    VirtualPath::data("nested").unwrap(),
                    "p0",
                    ExpectedPreimage::Absent,
                ),
                RepositoryAction::write_file(
                    VirtualPath::data("nested/file.txt").unwrap(),
                    "p0",
                    ExpectedPreimage::Absent,
                    b"x".to_vec(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap();
        let json2 = JsonFileStorage::new(&data2);
        let json_outcome = apply_once(&json2, &layout2, spec(), &with_parent).unwrap();
        let memory2 = InMemoryStorage::new();
        if existing_root {
            seed_memory_existing(&memory2, &[]);
        }
        let memory_outcome = apply_once(&memory2, &layout2, spec(), &with_parent).unwrap();
        assert_eq!(
            json_outcome.actions_applied, memory_outcome.actions_applied,
            "complete hashes may differ with backend-specific captured identities; \
                 action counts must agree for existing_root={existing_root}"
        );
    }
}

#[test]
fn test_conformance_listing_over_file_errors_on_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("file.txt"), b"content").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = || {
        let mut spec =
            CaptureSpec::phase_one([VirtualPath::data("file.txt").unwrap()], budget()).unwrap();
        spec.discover_listing(VirtualPath::data("file.txt").unwrap())
            .unwrap();
        spec
    };

    // A complete-listing request over a regular file is an error on both
    // backends, never a fabricated directory fingerprint.
    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    assert!(json_session.capture(spec()).is_err());

    let memory = InMemoryStorage::new();
    seed_memory_existing(
        &memory,
        &[(VirtualPath::data("file.txt").unwrap(), b"content")],
    );
    let mut memory_session = memory.open_mutation_session(layout).unwrap();
    assert!(memory_session.capture(spec()).is_err());
}

#[test]
fn test_conformance_worktree_root_listing_canonicalizes_nested_data_child() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = || {
        let root = VirtualPath::worktree("").unwrap();
        let mut spec = CaptureSpec::phase_one([], budget()).unwrap();
        spec.discover_paths([root.clone()]).unwrap();
        spec.discover_listing(root).unwrap();
        spec
    };

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(spec()).unwrap();

    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[]);
    memory.repository_state().entries.insert(
        VirtualPath::worktree("").unwrap(),
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("mem-worktree-root", b"directory").unwrap(),
            mode: FileMode::Executable,
        },
    );
    let mut memory_session = memory.open_mutation_session(layout).unwrap();
    let memory_image = memory_session.capture(spec()).unwrap();

    for image in [&json_image, &memory_image] {
        let listing = &image.listing_fingerprints()[&VirtualPath::worktree("").unwrap()];
        assert!(listing.children().contains_key(".jit"));
    }
}

#[cfg(unix)]
#[test]
fn test_json_session_opens_beneath_non_utf8_ancestor() {
    use std::os::unix::ffi::OsStringExt as _;

    let temp = TempDir::new().unwrap();
    let ancestor = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()));
    let worktree = ancestor.join("repo");
    let data = worktree.join(".jit");
    std::fs::create_dir_all(&data).unwrap();
    let layout = discover_repository_layout(&worktree, &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let mut session = storage.open_mutation_session(layout).unwrap();
    let image = session
        .capture(CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap())
        .unwrap();

    assert!(matches!(
        image.entry(&VirtualPath::data("").unwrap()).unwrap(),
        RepositoryEntry::Directory { .. }
    ));
}

fn capture_listing_errors(
    children: &[(&str, &[u8])],
    budget: CaptureBudget,
) -> [RepositoryStateStoreError; 2] {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir_all(data.join("issues")).unwrap();
    for (name, bytes) in children {
        std::fs::write(data.join("issues").join(name), bytes).unwrap();
    }
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = || {
        let mut spec = CaptureSpec::phase_one([], budget).unwrap();
        spec.discover_listing(VirtualPath::data("issues").unwrap())
            .unwrap();
        spec
    };

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_error = json_session.capture(spec()).unwrap_err();

    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[]);
    {
        let mut state = memory.repository_state();
        state.entries.insert(
            VirtualPath::data("issues").unwrap(),
            RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("mem-issues", b"directory").unwrap(),
                mode: FileMode::Executable,
            },
        );
        for (name, bytes) in children {
            let path = VirtualPath::data(format!("issues/{name}")).unwrap();
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes(format!("mem:{path:?}"), bytes).unwrap(),
                    bytes: bytes.to_vec(),
                    mode: FileMode::Regular,
                },
            );
        }
    }
    let mut memory_session = memory.open_mutation_session(layout).unwrap();
    let memory_error = memory_session.capture(spec()).unwrap_err();
    [json_error, memory_error]
}

#[test]
fn test_conformance_listing_rogue_children_exceed_path_budget_on_both_backends() {
    for error in capture_listing_errors(
        &[("one.json", b"one"), ("two.json", b"two")],
        CaptureBudget {
            max_paths: 1,
            max_listings: 1,
            max_bytes: 64,
            max_depth: 2,
        },
    ) {
        assert!(matches!(
            error,
            RepositoryStateStoreError::Capture(CaptureError::PathBudgetExceeded {
                actual: 2,
                maximum: 1
            })
        ));
    }
}

#[test]
fn test_conformance_listing_rogue_name_bytes_exceed_budget_on_both_backends() {
    for error in capture_listing_errors(
        &[("é", b"content")],
        CaptureBudget {
            max_paths: 1,
            max_listings: 1,
            max_bytes: 1,
            max_depth: 2,
        },
    ) {
        assert!(matches!(
            error,
            RepositoryStateStoreError::Capture(CaptureError::ByteBudgetExceeded {
                actual: 2,
                maximum: 1
            })
        ));
    }
}

#[test]
fn test_conformance_empty_delta_ignores_armed_injectors() {
    for point in [
        TransactionFailurePoint::RepositoryPrepareIntent,
        TransactionFailurePoint::RepositoryAfterCommit,
        TransactionFailurePoint::RepositoryCleanup,
    ] {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        let delta = RepositoryDelta::new(&layout, vec![]).unwrap();

        // An empty delta short-circuits before any apply-phase injector on both
        // backends, so an armed apply-phase failure never fires.
        let json = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(point.clone()),
        );
        let json_outcome = apply_once(&json, &layout, spec(), &delta).unwrap();

        let memory =
            InMemoryStorage::with_repository_state_failures(SelectedFailures::one(point.clone()));
        seed_memory_existing(&memory, &[]);
        let memory_outcome = apply_once(&memory, &layout, spec(), &delta).unwrap();

        assert_eq!(
            json_outcome.actions_applied, memory_outcome.actions_applied,
            "{point:?}"
        );
        assert_eq!(json_outcome.actions_applied, 0);
    }
}

#[test]
fn test_conformance_absent_root_data_only_materializes_data_root_dir() {
    // JSON publishes the staged root WITHOUT requiring an explicit
    // CreateDirectory Data("") (staging creates the stage dir, which becomes the
    // published root); memory materializes the Data("") Directory to match. Both
    // spellings — with and without an explicit root-dir action — must produce
    // identical post-apply captures on both backends.
    for explicit_root_dir in [false, true] {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || {
            CaptureSpec::phase_one(
                [
                    VirtualPath::data("").unwrap(),
                    VirtualPath::data("index.json").unwrap(),
                ],
                budget(),
            )
            .unwrap()
        };
        let mut actions = Vec::new();
        if explicit_root_dir {
            actions.push(RepositoryAction::create_directory(
                VirtualPath::data("").unwrap(),
                "g",
                ExpectedPreimage::Absent,
            ));
        }
        actions.push(RepositoryAction::write_file(
            VirtualPath::data("index.json").unwrap(),
            "g",
            ExpectedPreimage::Absent,
            b"{}".to_vec(),
            FileMode::Regular,
        ));
        let delta = RepositoryDelta::new(&layout, actions).unwrap();

        let memory = InMemoryStorage::new();
        apply_once(&memory, &layout, spec(), &delta).unwrap();
        let mut memory_after = memory.open_mutation_session(layout.clone()).unwrap();
        let memory_view = semantic_view(&memory_after.capture(spec()).unwrap());

        let json = JsonFileStorage::new(&data);
        apply_once(&json, &layout, spec(), &delta).unwrap();
        let json_after_store = JsonFileStorage::new(&data);
        let mut json_after = json_after_store
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        let json_view = semantic_view(&json_after.capture(spec()).unwrap());

        assert_eq!(
            memory_view, json_view,
            "explicit_root_dir={explicit_root_dir}"
        );
        assert!(matches!(
            memory_view[&VirtualPath::data("").unwrap()],
            SemanticEntry::Directory(_)
        ));
        assert_eq!(
            memory_view[&VirtualPath::data("index.json").unwrap()],
            SemanticEntry::File(b"{}".to_vec(), FileMode::Regular)
        );
    }
}

#[test]
fn test_conformance_rejects_worktree_data_alias() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    // A worktree-classed path physically inside the nested data root is a
    // cross-root alias; the layout rejects it wherever a canonical path is
    // required, so neither backend can capture or mutate through it.
    if let Ok(alias) = VirtualPath::worktree(".jit/index.json") {
        assert!(layout.ensure_canonical(&alias).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarrierRootShape {
    Existing,
    Absent,
}

impl BarrierRootShape {
    fn data_root_exists(self) -> bool {
        self == Self::Existing
    }
}

#[derive(Debug, Clone, Copy)]
enum BarrierDeltaKind {
    Replace,
    DeleteFirst,
}

#[derive(Debug, Clone, Copy)]
enum BarrierRecoveryDirection {
    Original,
    Final,
}

#[derive(Debug, PartialEq, Eq)]
struct BarrierView {
    first: SemanticEntry,
    second: SemanticEntry,
    data_root: SemanticEntry,
    data_record: SemanticEntry,
}

fn barrier_view(image: &RepositoryImage) -> BarrierView {
    let entry = |path: VirtualPath| semantic_entry(image.entry(&path).unwrap());
    BarrierView {
        first: entry(VirtualPath::worktree("first.txt").unwrap()),
        second: entry(VirtualPath::worktree("second.txt").unwrap()),
        data_root: entry(VirtualPath::data("").unwrap()),
        data_record: entry(VirtualPath::data("record.txt").unwrap()),
    }
}

fn expected_barrier_view(
    shape: BarrierRootShape,
    direction: BarrierRecoveryDirection,
) -> BarrierView {
    let final_state = matches!(direction, BarrierRecoveryDirection::Final);
    let file = |bytes: &[u8]| SemanticEntry::File(bytes.to_vec(), FileMode::Regular);
    BarrierView {
        first: file(if final_state {
            b"new-first"
        } else {
            b"old-first"
        }),
        second: file(if final_state {
            b"new-second"
        } else {
            b"old-second"
        }),
        data_root: if shape.data_root_exists() || final_state {
            SemanticEntry::Directory(FileMode::Executable)
        } else {
            SemanticEntry::Absent
        },
        data_record: if shape.data_root_exists() {
            file(if final_state {
                b"new-data"
            } else {
                b"old-data"
            })
        } else if final_state {
            file(b"new-data")
        } else {
            SemanticEntry::Absent
        },
    }
}

fn seed_json_barrier_fixture(worktree: &Path, data: &Path, shape: BarrierRootShape) {
    std::fs::write(worktree.join("first.txt"), b"old-first").unwrap();
    std::fs::write(worktree.join("second.txt"), b"old-second").unwrap();
    if shape.data_root_exists() {
        std::fs::create_dir(data).unwrap();
        std::fs::write(data.join("record.txt"), b"old-data").unwrap();
    }
}

fn barrier_delta(
    layout: &RepositoryLayout,
    image: &RepositoryImage,
    kind: BarrierDeltaKind,
) -> RepositoryDelta {
    let expected = |path: &VirtualPath| ExpectedPreimage::of(image.entry(path).unwrap());
    let first = VirtualPath::worktree("first.txt").unwrap();
    let second = VirtualPath::worktree("second.txt").unwrap();
    let data_record = VirtualPath::data("record.txt").unwrap();
    let first_action = match kind {
        BarrierDeltaKind::Replace => RepositoryAction::write_file(
            first.clone(),
            "barrier",
            expected(&first),
            b"new-first".to_vec(),
            FileMode::Regular,
        ),
        BarrierDeltaKind::DeleteFirst => {
            RepositoryAction::delete_file(first.clone(), "barrier", expected(&first))
        }
    };
    RepositoryDelta::new(
        layout,
        vec![
            first_action,
            RepositoryAction::write_file(
                second.clone(),
                "barrier",
                expected(&second),
                b"new-second".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                data_record.clone(),
                "barrier",
                expected(&data_record),
                b"new-data".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap()
}

fn converge_forward_barrier_crash(
    point: TransactionFailurePoint,
    kind: BarrierDeltaKind,
    shape: BarrierRootShape,
) {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    seed_json_barrier_fixture(worktree.path(), &data, shape);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // JSON derives every preimage from a real capture, so a failure after a
    // rename exercises identity-based recovery rather than a hand-built journal
    // whose layout or identity digest could fail first.
    let json_failures = SelectedFailures::one(point.clone());
    let json = JsonFileStorage::with_repository_state_failures(&data, json_failures.clone());
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(barrier_spec()).unwrap();
    assert_eq!(
        barrier_view(&json_image),
        expected_barrier_view(shape, BarrierRecoveryDirection::Original)
    );
    let json_delta = barrier_delta(&layout, &json_image, kind);
    assert!(json_session
        .apply(&test_plan(&json_image, &json_delta))
        .is_err());
    assert!(json_failures.is_consumed(), "JSON did not reach {point:?}");
    drop(json_session);
    let json_recovered = JsonFileStorage::new(&data);
    let mut json_session = json_recovered
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap_or_else(|error| panic!("JSON recovery failed at {point:?} {shape:?}: {error:#}"));
    let json_view = barrier_view(&json_session.capture(barrier_spec()).unwrap());

    let memory_failures = SelectedFailures::one(point.clone());
    let memory = InMemoryStorage::with_repository_state_failures(memory_failures.clone());
    seed_memory_barrier_fixture(&memory, shape.data_root_exists());
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(barrier_spec()).unwrap();
    assert_eq!(
        barrier_view(&memory_image),
        expected_barrier_view(shape, BarrierRecoveryDirection::Original)
    );
    let memory_delta = barrier_delta(&layout, &memory_image, kind);
    assert!(memory_session
        .apply(&test_plan(&memory_image, &memory_delta))
        .is_err());
    assert!(
        memory_failures.is_consumed(),
        "memory did not reach {point:?}"
    );
    drop(memory_session);
    let memory_recovered = memory.without_repository_state_failures();
    let mut memory_session = memory_recovered
        .open_mutation_session(layout)
        .unwrap_or_else(|error| panic!("memory recovery failed at {point:?} {shape:?}: {error:#}"));
    let memory_view = barrier_view(&memory_session.capture(barrier_spec()).unwrap());

    let direction = if point == TransactionFailurePoint::RepositoryBeforeCommitDecision
        && shape == BarrierRootShape::Absent
    {
        // Publishing the absent Data root is the irreversible commit point; a
        // Prepared record discovered after that rename completes forward.
        BarrierRecoveryDirection::Final
    } else {
        BarrierRecoveryDirection::Original
    };
    let expected = expected_barrier_view(shape, direction);
    assert_eq!(json_view, expected, "JSON {point:?} {shape:?}");
    assert_eq!(memory_view, expected, "memory {point:?} {shape:?}");
}

#[test]
fn test_conformance_single_barrier_forward_crashes_recover_exact_views() {
    use TransactionFailurePoint::*;

    for (point, kind) in [
        (
            RepositorySyncBackup { action: 0 },
            BarrierDeltaKind::Replace,
        ),
        (RepositoryBeforePreparedJournal, BarrierDeltaKind::Replace),
        (RepositorySyncPreparedJournal, BarrierDeltaKind::Replace),
        (
            RepositoryAfterTargetMutation { action: 0 },
            BarrierDeltaKind::Replace,
        ),
        (
            RepositoryBeforeDeleteRename { action: 0 },
            BarrierDeltaKind::DeleteFirst,
        ),
        (RepositoryBeforeCommitDecision, BarrierDeltaKind::Replace),
    ] {
        for shape in [BarrierRootShape::Existing, BarrierRootShape::Absent] {
            converge_forward_barrier_crash(point.clone(), kind, shape);
        }
    }

    // RepositoryBeforeDataStageJournal remains covered by the dedicated
    // fail-closed test: it precedes publication of the stage identity and is
    // intentionally not a recoverable old/new convergence boundary.
}

fn complete_rollback_sequence(shape: BarrierRootShape) -> Vec<TransactionFailurePoint> {
    use TransactionFailurePoint::{
        RepositoryAfterReverseAction as After, RepositoryBeforeReverseAction as Before,
        RepositoryBeforeRollbackDecision as Decision, RepositoryBeforeStageCleanup as Cleanup,
    };
    let mut points = if shape.data_root_exists() {
        vec![
            Before { action: 2 },
            After { action: 2 },
            Before { action: 1 },
            After { action: 1 },
            Before { action: 0 },
            After { action: 0 },
        ]
    } else {
        // Data action 2 lives in the unpublished stage and is never reversed.
        vec![
            Before { action: 1 },
            After { action: 1 },
            Before { action: 0 },
            After { action: 0 },
        ]
    };
    points.extend([Cleanup, Decision]);
    points
}

fn rollback_sequence_through(
    shape: BarrierRootShape,
    point: &TransactionFailurePoint,
) -> Vec<TransactionFailurePoint> {
    let sequence = complete_rollback_sequence(shape);
    let end = sequence
        .iter()
        .position(|observed| observed == point)
        .unwrap_or_else(|| panic!("{point:?} is not reachable for {shape:?}"));
    sequence[..=end].to_vec()
}

fn json_transaction_residue_exists(worktree: &Path, data: &Path, shape: BarrierRootShape) -> bool {
    let transactions = if shape.data_root_exists() {
        data.join("tmp/transactions")
    } else {
        worktree.join(".jit-bootstrap/transactions")
    };
    std::fs::read_dir(transactions)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

fn converge_rollback_barrier_crash(point: TransactionFailurePoint, shape: BarrierRootShape) {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    seed_json_barrier_fixture(worktree.path(), &data, shape);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let interruption =
        SelectedFailures::one(TransactionFailurePoint::RepositoryBeforeAction { action: 1 });
    let interrupted = JsonFileStorage::with_repository_state_failures(&data, interruption.clone());
    let mut session = interrupted.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(barrier_spec()).unwrap();
    let delta = barrier_delta(&layout, &image, BarrierDeltaKind::Replace);
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    assert!(interruption.is_consumed());
    drop(session);
    assert_eq!(
        std::fs::read(worktree.path().join("first.txt")).unwrap(),
        b"new-first",
        "the first live rename must precede rollback recovery"
    );

    let failure = RecordingSelectedFailure::one(point.clone());
    let recovering = JsonFileStorage::with_repository_state_failures(&data, failure.clone());
    assert!(recovering
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .is_err());
    assert!(failure.is_consumed(), "JSON did not reach {point:?}");
    assert_eq!(
        rollback_points(failure.observed()),
        rollback_sequence_through(shape, &point),
        "JSON rollback order at {point:?} {shape:?}"
    );
    assert!(json_transaction_residue_exists(
        worktree.path(),
        &data,
        shape
    ));

    let mut session = recovering
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap_or_else(|error| panic!("JSON retry failed at {point:?} {shape:?}: {error:#}"));
    let json_view = barrier_view(&session.capture(barrier_spec()).unwrap());
    assert_eq!(
        json_view,
        expected_barrier_view(shape, BarrierRecoveryDirection::Original)
    );

    let (_memory_temp, memory_layout, memory) =
        prepare_memory_barrier_rollback(shape.data_root_exists());
    let failure = RecordingSelectedFailure::one(point.clone());
    let recovering = memory.with_repository_state_failure_view(failure.clone());
    assert!(recovering
        .open_mutation_session(memory_layout.clone())
        .is_err());
    assert!(failure.is_consumed(), "memory did not reach {point:?}");
    assert_eq!(
        rollback_points(failure.observed()),
        rollback_sequence_through(shape, &point),
        "memory rollback order at {point:?} {shape:?}"
    );
    assert!(matches!(
        memory.repository_state().recovery,
        Some(MemoryRecoveryResidue::Prepared { .. })
    ));

    let mut session = recovering.open_mutation_session(memory_layout).unwrap();
    let memory_view = barrier_view(&session.capture(barrier_spec()).unwrap());
    assert_eq!(
        memory_view,
        expected_barrier_view(shape, BarrierRecoveryDirection::Original)
    );
    assert_eq!(json_view, memory_view);
    assert!(memory.repository_state().recovery.is_none());
}

#[test]
fn test_conformance_single_barrier_rollback_crashes_retain_prepared_for_retry() {
    for point in [
        TransactionFailurePoint::RepositoryBeforeReverseAction { action: 0 },
        TransactionFailurePoint::RepositoryAfterReverseAction { action: 0 },
        TransactionFailurePoint::RepositoryBeforeStageCleanup,
        TransactionFailurePoint::RepositoryBeforeRollbackDecision,
    ] {
        for shape in [BarrierRootShape::Existing, BarrierRootShape::Absent] {
            converge_rollback_barrier_crash(point.clone(), shape);
        }
    }
}

/// Failure points reached by the shared absent-preimage conformance scenarios.
/// The identity-bearing single-barrier cases above supplement this broad base
/// with backup, delete-rename, and prepared-rollback edges.
fn absent_preimage_failure_points() -> Vec<TransactionFailurePoint> {
    use TransactionFailurePoint::*;
    vec![
        RepositoryRecoveryExternal,
        RepositoryRecoveryInternal,
        RepositoryBeforeControlCreation,
        RepositoryCreateControl,
        RepositoryBeforeInitialJournal,
        RepositorySyncInitialJournal,
        RepositoryPrepareIntent,
        RepositoryCreateCompanion,
        RepositoryPrepareAction { action: 0 },
        RepositoryStageAction { action: 0 },
        RepositorySyncStage { action: 0 },
        RepositoryBeforePreparedJournal,
        RepositorySyncPreparedJournal,
        RepositoryBeforeAction { action: 0 },
        RepositoryBeforeTargetMutation { action: 0 },
        RepositoryAfterRootBindingCheck { action: 0 },
        RepositoryAfterTargetMutation { action: 0 },
        RepositoryVerifyFinalIdentity { action: 0 },
        RepositoryAfterAction { action: 0 },
        RepositoryBeforeDataRootPublication,
        RepositoryAfterDataParentBindingCheck,
        RepositoryAfterDataRootPublication,
        RepositoryBeforeCommitDecision,
        RepositoryAfterCommit,
        RepositoryCleanup,
        RepositoryBeforeStageCleanup,
        RepositoryBeforeCompanionCleanup,
        RepositoryBeforeControlCleanup,
        RepositorySweepCompanions,
    ]
}

#[derive(Debug, PartialEq, Eq)]
enum EdgeOutcome {
    OpenFailed,
    ApplyFailed,
    Applied,
}

fn drive_edge<S: RepositoryStateStore>(
    store: &S,
    layout: &RepositoryLayout,
    spec: CaptureSpec,
    delta: &RepositoryDelta,
) -> EdgeOutcome {
    let mut session = match store.open_mutation_session(layout.clone()) {
        Ok(session) => session,
        Err(_) => return EdgeOutcome::OpenFailed,
    };
    let image = session.capture(spec).unwrap();
    match session.apply(&test_plan(&image, delta)) {
        Ok(_) => EdgeOutcome::Applied,
        Err(_) => EdgeOutcome::ApplyFailed,
    }
}

#[derive(Debug, Clone, Copy)]
enum EdgeScenario {
    /// Absent root, worktree action at index 0 (sort puts Worktree < Data).
    AbsentInit,
    /// Existing root, worktree + data action (companion + recovery/sweep fire).
    ExistingMixed,
    /// Absent root, DATA action at index 0 — exercises the absent-root
    /// Data-action publication boundary that AbsentInit hides behind its
    /// index-0 worktree action.
    AbsentDataOnly,
}

#[test]
fn test_conformance_failure_edges_converge_on_both_backends() {
    for point in absent_preimage_failure_points() {
        for scenario in [
            EdgeScenario::AbsentInit,
            EdgeScenario::ExistingMixed,
            EdgeScenario::AbsentDataOnly,
        ] {
            converge_failure_edge(&point, scenario);
        }
    }
}

fn converge_failure_edge(point: &TransactionFailurePoint, scenario: EdgeScenario) {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    if matches!(scenario, EdgeScenario::ExistingMixed) {
        std::fs::create_dir(&data).unwrap();
    }
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Every scenario uses only absent-preimage actions so one shared delta is
    // valid on both backends.
    let make_spec = || match scenario {
        EdgeScenario::ExistingMixed => {
            let mut spec =
                CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
            spec.discover_paths([
                VirtualPath::worktree("out.txt").unwrap(),
                VirtualPath::data("gen.txt").unwrap(),
            ])
            .unwrap();
            spec
        }
        EdgeScenario::AbsentInit => initial_spec(),
        EdgeScenario::AbsentDataOnly => CaptureSpec::phase_one(
            [
                VirtualPath::data("").unwrap(),
                VirtualPath::data("index.json").unwrap(),
            ],
            budget(),
        )
        .unwrap(),
    };
    let delta = match scenario {
        EdgeScenario::ExistingMixed => RepositoryDelta::new(
            &layout,
            vec![
                RepositoryAction::write_file(
                    VirtualPath::worktree("out.txt").unwrap(),
                    "edge",
                    ExpectedPreimage::Absent,
                    b"worktree".to_vec(),
                    FileMode::Regular,
                ),
                RepositoryAction::write_file(
                    VirtualPath::data("gen.txt").unwrap(),
                    "edge",
                    ExpectedPreimage::Absent,
                    b"data".to_vec(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap(),
        EdgeScenario::AbsentInit => initialization_delta(&layout),
        EdgeScenario::AbsentDataOnly => RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::data("index.json").unwrap(),
                "edge",
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap(),
    };

    let json_failures = SelectedFailures::one(point.clone());
    let json = JsonFileStorage::with_repository_state_failures(&data, json_failures.clone());
    let json_outcome = drive_edge(&json, &layout, make_spec(), &delta);
    let json_post = {
        let recovered = JsonFileStorage::new(&data);
        let mut session = recovered
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap_or_else(|error| panic!("json recovery {point:?} {scenario:?}: {error:#}"));
        semantic_view(&session.capture(make_spec()).unwrap())
    };

    let memory_failures = SelectedFailures::one(point.clone());
    let memory = InMemoryStorage::with_repository_state_failures(memory_failures.clone());
    if matches!(scenario, EdgeScenario::ExistingMixed) {
        seed_memory_existing(&memory, &[]);
    }
    let memory_outcome = drive_edge(&memory, &layout, make_spec(), &delta);
    let memory_post = {
        let recovered = memory.without_repository_state_failures();
        let mut session = recovered
            .open_mutation_session(layout.clone())
            .unwrap_or_else(|error| panic!("memory recovery {point:?} {scenario:?}: {error:#}"));
        semantic_view(&session.capture(make_spec()).unwrap())
    };

    // Both backends fire the injected point at the same phase (open vs apply)
    // with the same outcome, and recover to the same complete old-or-new state.
    assert_eq!(
        json_outcome, memory_outcome,
        "outcome parity at {point:?} {scenario:?}"
    );
    assert_eq!(
        json_post, memory_post,
        "convergence parity at {point:?} {scenario:?}"
    );
    if json_outcome != EdgeOutcome::Applied {
        assert!(
            json_failures.is_consumed(),
            "JSON failure point did not fire at {point:?} {scenario:?}"
        );
        assert!(
            memory_failures.is_consumed(),
            "memory failure point did not fire at {point:?} {scenario:?}"
        );
    }
}

#[test]
fn test_recovery_decision_failure_points_are_consumed_and_recoverable() {
    for point in [
        TransactionFailurePoint::RepositoryBeforeReverseAction { action: 0 },
        TransactionFailurePoint::RepositoryBeforeRollbackDecision,
    ] {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let first = VirtualPath::data("first.txt").unwrap();
        let second = VirtualPath::data("second.txt").unwrap();

        let interruption =
            SelectedFailures::one(TransactionFailurePoint::RepositoryBeforeAction { action: 1 });
        let interrupted =
            JsonFileStorage::with_repository_state_failures(&data, interruption.clone());
        let mut session = interrupted.open_mutation_session(layout.clone()).unwrap();
        let image = session
            .capture(CaptureSpec::phase_one([first.clone(), second.clone()], budget()).unwrap())
            .unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![
                RepositoryAction::write_file(
                    first,
                    "recovery-decision",
                    ExpectedPreimage::Absent,
                    b"first".to_vec(),
                    FileMode::Regular,
                ),
                RepositoryAction::write_file(
                    second,
                    "recovery-decision",
                    ExpectedPreimage::Absent,
                    b"second".to_vec(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap();
        let apply_error = session.apply(&test_plan(&image, &delta)).unwrap_err();
        assert!(
            interruption.is_consumed(),
            "unexpected apply error: {apply_error:#}"
        );
        drop(session);

        let failures = SelectedFailures::one(point.clone());
        assert!(
            JsonFileStorage::with_repository_state_failures(&data, failures.clone())
                .open_mutation_session(layout.clone())
                .is_err(),
            "recovery unexpectedly succeeded at {point:?}"
        );
        assert!(
            failures.is_consumed(),
            "failure point did not fire: {point:?}"
        );

        drop(
            JsonFileStorage::new(&data)
                .open_mutation_session(layout.clone())
                .unwrap_or_else(|error| panic!("recovery failed at {point:?}: {error:#}")),
        );
        assert!(!data.join("first.txt").exists());
        assert!(!data.join("second.txt").exists());
        assert!(!data.join("tmp/transactions").exists());
    }
}

// --- Finding 2: cross-filesystem per-root staging (worktree companion) ----
//
// An internal transaction publishing Worktree actions gets a worktree-side
// companion control area so those actions stage and back up on the worktree
// filesystem. True cross-filesystem (a data root on a different mount) cannot
// be reproduced in CI; these tests prove the companion is worktree-colocated
// by construction (device-id equality) and that its full lifecycle — create,
// route, rollback, cleanup, orphan sweep — is crash-recoverable. InMemoryStorage
// has no filesystem and stages nothing, so it needs no companion; its mixed
// Worktree+Data parity is already covered by the conformance matrix above.

#[cfg(unix)]
#[test]
fn test_session_worktree_companion_is_worktree_colocated_and_cleaned() {
    use std::os::unix::fs::MetadataExt as _;
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Fail after commit but before cleanup so the companion is observable.
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterCommit),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("out.txt").unwrap(),
            "f2",
            ExpectedPreimage::Absent,
            b"worktree".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    // The companion lives under the worktree, same device as worktree targets
    // by construction — the CI-reproducible proof that its staging/backup
    // authority is same-filesystem with the Worktree actions it publishes.
    let transactions = worktree.path().join(".jit-bootstrap/transactions");
    let companion = std::fs::read_dir(&transactions)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let transaction_id = companion.file_name().unwrap().to_str().unwrap().to_string();
    assert!(companion.join("companion").is_file());
    assert!(companion.join("stages").is_dir());
    assert!(companion.join("backups").is_dir());
    assert_eq!(
        std::fs::metadata(companion.join("stages")).unwrap().dev(),
        std::fs::metadata(worktree.path()).unwrap().dev()
    );

    // Recovery reclaims the companion and converges to the published state.
    let recovered = JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();
    assert!(recovered.recovery_report().external_transactions.is_empty());
    assert_eq!(
        recovered.recovery_report().internal_transactions,
        vec![transaction_id]
    );
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    assert_eq!(
        std::fs::read(worktree.path().join("out.txt")).unwrap(),
        b"worktree"
    );
}

#[cfg(unix)]
#[test]
fn test_session_rollback_restores_worktree_action_from_companion_backup() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(worktree.path().join("out.txt"), b"old").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(
        image
            .entry(&VirtualPath::worktree("out.txt").unwrap())
            .unwrap(),
    );
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("out.txt").unwrap(),
            "f2",
            expected,
            b"new".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    // Recovery rolls the Worktree replace back to the backup held in the
    // companion (worktree filesystem), then reclaims the companion.
    JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();
    assert_eq!(
        std::fs::read(worktree.path().join("out.txt")).unwrap(),
        b"old"
    );
    assert!(!worktree.path().join(".jit-bootstrap").exists());
}

#[test]
fn test_session_companion_creation_failure_recovers_clean() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(worktree.path().join("keep.txt"), b"keep").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryCreateCompanion),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("new.txt").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("new.txt").unwrap(),
            "f2",
            ExpectedPreimage::Absent,
            b"new".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();
    assert!(!worktree.path().join("new.txt").exists());
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    assert_eq!(
        std::fs::read(worktree.path().join("keep.txt")).unwrap(),
        b"keep"
    );
}

#[test]
fn test_session_orphan_companion_is_swept_on_open() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Fabricate a companion OWNED BY THIS data root whose internal transaction
    // never existed. Its marker records the owner digest the running session
    // recomputes, so the sweep recognizes it as its own orphan.
    let owner = owner_digest_of(&layout);
    fabricate_companion(worktree.path(), "z-orphan", &owner);
    fabricate_companion(worktree.path(), "a-orphan", &owner);

    // Opening a session skips the companion in external recovery, then the
    // orphan sweep (data-root guards held) removes it.
    let recovered = JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .unwrap();
    assert_eq!(
        recovered.recovery_report().external_transactions,
        vec!["a-orphan", "z-orphan"]
    );
    assert!(recovered.recovery_report().internal_transactions.is_empty());
    assert!(!worktree.path().join(".jit-bootstrap").exists());
}

// Owner digest recomputed exactly as the kernel does (worktree/data paths).
fn owner_digest_of(layout: &RepositoryLayout) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(layout.worktree_root().to_string_lossy().as_bytes());
    hasher.update([0u8]);
    hasher.update(layout.data_root().to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn fabricate_companion(worktree: &std::path::Path, id: &str, owner: &str) {
    let companion = worktree.join(".jit-bootstrap/transactions").join(id);
    std::fs::create_dir_all(companion.join("stages")).unwrap();
    std::fs::create_dir(companion.join("backups")).unwrap();
    std::fs::write(companion.join("companion"), owner.as_bytes()).unwrap();
    std::fs::write(
        worktree.join(".jit-bootstrap/transaction-protocol-v1"),
        b"1\n",
    )
    .unwrap();
}

#[test]
fn test_session_foreign_owner_companion_is_not_swept() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // A companion owned by a DIFFERENT data root sharing this worktree. Its
    // owning session may be mid-flight or crashed; only that owner may reap it.
    fabricate_companion(
        worktree.path(),
        "foreign-id",
        "a-different-data-root-owner-digest",
    );

    let recovered = JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .unwrap();
    assert_eq!(
        recovered.recovery_report(),
        &RecoveryDispatchReport::default()
    );

    // The foreign companion survives — this session never reaps another data
    // root's transaction, so it cannot destroy a live companion's backup.
    assert!(worktree
        .path()
        .join(".jit-bootstrap/transactions/foreign-id/companion")
        .is_file());
}

#[test]
fn test_session_foreign_owner_external_journal_is_skipped_not_failed() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // A committed absent-root external journal left by a DIFFERENT data root.
    // Recovering it against our layout would be wrong; our open must skip it,
    // not fail. Its foreign owner digest keeps it out of our recovery.
    let txn = worktree
        .path()
        .join(".jit-bootstrap/transactions/foreign-journal");
    std::fs::create_dir_all(txn.join("stages")).unwrap();
    std::fs::create_dir(txn.join("backups")).unwrap();
    std::fs::write(
            txn.join("journal.json"),
            br#"{"version":3,"transaction_id":"foreign-journal","layout_digest":"x","owner_digest":"a-different-owner","plan_hash":"x","data_root_was_absent":true,"data_stage":null,"data_stage_identity":null,"decision":"committed","actions":[]}"#,
        )
        .unwrap();
    std::fs::write(
        worktree
            .path()
            .join(".jit-bootstrap/transaction-protocol-v1"),
        b"1\n",
    )
    .unwrap();

    // Open succeeds (does not fail against the foreign journal) and leaves the
    // foreign residue intact for its owning data root's session.
    let recovered = JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .unwrap();
    assert_eq!(
        recovered.recovery_report(),
        &RecoveryDispatchReport::default()
    );
    assert!(worktree
        .path()
        .join(".jit-bootstrap/transactions/foreign-journal/journal.json")
        .is_file());
}

#[test]
fn test_session_disjoint_sessions_serialize_on_worktree_bootstrap() {
    use crate::storage::contention_probe::{admitted_when_reached, Contenders};
    use crate::storage::repo_lock::RepoWriteLock;
    use crate::storage::repository_state_store::is_lock_timeout;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Two data roots under DIFFERENT parents, both sharing one worktree. Their
    // data-root-parent bootstrap locks differ, so only the worktree-root
    // bootstrap lock serializes their shared `.jit-bootstrap` namespace.
    let worktree = TempDir::new().unwrap();
    let parent_one = TempDir::new().unwrap();
    let parent_two = TempDir::new().unwrap();
    let data_one = parent_one.path().join("store");
    let data_two = parent_two.path().join("store");
    std::fs::create_dir(&data_one).unwrap();
    std::fs::create_dir(&data_two).unwrap();
    let layout_one = discover_repository_layout(worktree.path(), &data_one).unwrap();
    let layout_two = discover_repository_layout(worktree.path(), &data_two).unwrap();

    // Both sessions reach the worktree bootstrap lock through the process-wide
    // registry, so registering it here with a wait shorter than any critical
    // section makes the session that meets it held report itself refused rather
    // than queue silently. Retained for the whole test to keep that entry live.
    let _bootstrap_lock = RepoWriteLock::shared_for_lock_path(
        layout_one.worktree_root().join(".jit-bootstrap.lock"),
        std::time::Duration::from_millis(1),
    );

    let first = JsonFileStorage::new(&data_one);
    let session_one = first.open_mutation_session(layout_one).unwrap();

    let contenders = Contenders::new();
    let holder_released = Arc::new(AtomicBool::new(false));
    let waiter = {
        let contenders = Arc::clone(&contenders);
        let holder_released = Arc::clone(&holder_released);
        std::thread::spawn(move || {
            let second = JsonFileStorage::new(&data_two);
            // An expired wait says this session was still queued, so it asks
            // again; the session it eventually opens is the lock's answer.
            let _session_two = admitted_when_reached(
                &contenders,
                "one worktree's bootstrap lock",
                || second.open_mutation_session(layout_two.clone()),
                is_lock_timeout,
            )
            .unwrap();
            assert!(
                holder_released.load(Ordering::SeqCst),
                "a disjoint-data-root session opened one worktree's bootstrap concurrently"
            );
        })
    };

    // The second session has met the bootstrap lock held, so the one it opens
    // next follows the release below rather than racing it.
    contenders.await_refusals(1);
    holder_released.store(true, Ordering::SeqCst);
    drop(session_one);
    waiter.join().unwrap();
}

#[cfg(unix)]
#[test]
fn test_session_committed_recovery_tolerates_edited_worktree_target() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Internal transaction with a Worktree target; fail after commit so the
    // committed journal and companion residue survive the crash.
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterCommit),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("attributes").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("attributes").unwrap(),
            "f2",
            ExpectedPreimage::Absent,
            b"committed".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    // The user legitimately edits the published worktree target before the
    // interrupted cleanup ever runs.
    std::fs::write(worktree.path().join("attributes"), b"user-edited").unwrap();

    // A later open must SUCCEED: past the commit point recovery converges
    // forward, tolerating the diverged worktree target rather than wedging.
    JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .expect("committed-journal recovery must not wedge on an edited worktree target");
    // Residue is cleaned (companion gone, internal transaction id removed) and
    // the user edit survives untouched.
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    let internal = data.join("tmp/transactions");
    assert!(
        std::fs::read_dir(&internal)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true),
        "internal transaction residue not cleaned"
    );
    assert_eq!(
        std::fs::read(worktree.path().join("attributes")).unwrap(),
        b"user-edited"
    );
}

#[cfg(unix)]
#[test]
fn test_json_apply_rejects_data_root_replaced_since_capture_as_retryable_conflict() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("keep.txt"), b"keep").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let storage = JsonFileStorage::new(&data);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("keep.txt").unwrap(),
            VirtualPath::data("new.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap();
    let image = session.capture(spec).unwrap();

    // Externally replace the WHOLE data-root directory with a fresh inode
    // between capture and apply. The held capability still points to the old,
    // now-unlinked inode; a tautological FD stat would miss this.
    let replacement = worktree.path().join(".jit-replacement");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("keep.txt"), b"keep").unwrap();
    std::fs::remove_dir_all(&data).unwrap();
    std::fs::rename(&replacement, &data).unwrap();

    // Applying re-resolves the root path no-follow, sees the new identity, and
    // aborts with a retryable conflict rather than writing into the old inode.
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::data("new.txt").unwrap(),
            "f4",
            ExpectedPreimage::Absent,
            b"new".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(matches!(
        session.apply(&test_plan(&image, &delta)),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_existing_data_root_replacement_during_publication_never_succeeds() {
    for point in [
        TransactionFailurePoint::RepositoryBeforeTargetMutation { action: 0 },
        TransactionFailurePoint::RepositoryAfterRootBindingCheck { action: 0 },
        TransactionFailurePoint::RepositoryBeforeCommitDecision,
    ] {
        let outer = TempDir::new().unwrap();
        let worktree = outer.path().join("project");
        let data = worktree.join(".jit");
        let replacement = worktree.join(".jit-replacement");
        let detached = worktree.join(".jit-detached");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(replacement.join("replacement-marker"), b"live").unwrap();

        let hook_data = data.clone();
        let hook_replacement = replacement.clone();
        let hook_detached = detached.clone();
        let injector = Arc::new(HookAt {
            point,
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&hook_data, &hook_detached).unwrap();
                std::fs::rename(&hook_replacement, &hook_data).unwrap();
            }))),
        });
        let layout = discover_repository_layout(&worktree, &data).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let path = VirtualPath::data("new.txt").unwrap();
        let spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap(), path.clone()], budget())
            .unwrap();
        let image = session.capture(spec).unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                path,
                "root-swap",
                ExpectedPreimage::Absent,
                b"planned".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();

        assert!(session.apply(&test_plan(&image, &delta)).is_err());
        assert_eq!(
            std::fs::read(data.join("replacement-marker")).unwrap(),
            b"live"
        );
        assert!(!data.join("new.txt").exists());
        assert!(!detached.join("new.txt").exists());
    }
}

#[cfg(unix)]
#[test]
fn test_absent_data_parent_replacement_during_publication_never_succeeds() {
    for point in [
        TransactionFailurePoint::RepositoryBeforeDataRootPublication,
        TransactionFailurePoint::RepositoryAfterDataParentBindingCheck,
        TransactionFailurePoint::RepositoryBeforeCommitDecision,
    ] {
        let outer = TempDir::new().unwrap();
        let worktree = outer.path().join("project");
        let data = worktree.join(".jit");
        let replacement = outer.path().join("replacement-project");
        let detached = outer.path().join("detached-project");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(replacement.join("replacement-marker"), b"live").unwrap();

        let hook_worktree = worktree.clone();
        let hook_replacement = replacement.clone();
        let hook_detached = detached.clone();
        let injector = Arc::new(HookAt {
            point,
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&hook_worktree, &hook_detached).unwrap();
                std::fs::rename(&hook_replacement, &hook_worktree).unwrap();
            }))),
        });
        let layout = discover_repository_layout(&worktree, &data).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(&data, injector);
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let path = VirtualPath::data("index.json").unwrap();
        let spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap(), path.clone()], budget())
            .unwrap();
        let image = session.capture(spec).unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                path,
                "root-publication",
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();

        assert!(session.apply(&test_plan(&image, &delta)).is_err());
        assert_eq!(
            std::fs::read(worktree.join("replacement-marker")).unwrap(),
            b"live"
        );
        assert!(!data.exists());
    }
}

#[cfg(unix)]
#[test]
fn test_delta_rejects_captured_cross_root_hard_link_alias() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(worktree.path().join("shared"), b"linked").unwrap();
    // One inode reachable through both roots: a cross-root hard-link alias.
    std::fs::hard_link(worktree.path().join("shared"), data.join("shared")).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let storage = JsonFileStorage::new(&data);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let mut spec =
        CaptureSpec::phase_one([VirtualPath::data("shared").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("shared").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();

    // Capture recorded ONE physical identity (dev:ino) at both virtual paths,
    // so building the delta rejects the pair with the typed alias error.
    // (InMemoryStorage cannot represent this: each memory entry's object id is
    // derived from its path, so two distinct paths never share one identity.)
    let worktree_pre = ExpectedPreimage::of(
        image
            .entry(&VirtualPath::worktree("shared").unwrap())
            .unwrap(),
    );
    let data_pre =
        ExpectedPreimage::of(image.entry(&VirtualPath::data("shared").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![
            RepositoryAction::set_mode(
                VirtualPath::worktree("shared").unwrap(),
                "one",
                worktree_pre,
                FileMode::Executable,
            ),
            RepositoryAction::set_mode(
                VirtualPath::data("shared").unwrap(),
                "two",
                data_pre,
                FileMode::Executable,
            ),
        ],
    );
    assert!(matches!(
        delta,
        Err(crate::repository_state::DeltaError::PhysicalAlias { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_session_rolledback_recovery_tolerates_edited_worktree_target() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(worktree.path().join("attributes"), b"old").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // 1) A worktree replace interrupted right after publication → Prepared.
    let staged = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = staged.open_mutation_session(layout.clone()).unwrap();
    let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
    spec.discover_paths([VirtualPath::worktree("attributes").unwrap()])
        .unwrap();
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(
        image
            .entry(&VirtualPath::worktree("attributes").unwrap())
            .unwrap(),
    );
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("attributes").unwrap(),
            "a1",
            expected,
            b"new".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    // 2) Recovery rolls back (restoring "old" and writing the RolledBack
    //    decision) but its cleanup is interrupted → RolledBack residue remains.
    let interrupted_cleanup = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryCleanup),
    );
    assert!(interrupted_cleanup
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .is_err());
    assert_eq!(
        std::fs::read(worktree.path().join("attributes")).unwrap(),
        b"old"
    );

    // 3) The user edits the restored worktree target.
    std::fs::write(worktree.path().join("attributes"), b"user-edited").unwrap();

    // 4) A later open recovers the RolledBack residue WITHOUT re-asserting the
    //    worktree preimage — open succeeds, residue cleaned, edit survives.
    JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .expect("rolledback recovery must not wedge on an edited worktree target");
    assert!(!worktree.path().join(".jit-bootstrap").exists());
    assert_eq!(
        std::fs::read(worktree.path().join("attributes")).unwrap(),
        b"user-edited"
    );
}

#[test]
fn test_memory_apply_rejects_stale_preimage_as_retryable_conflict() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "wt", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
    )
    .unwrap();
    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[(VirtualPath::data("x").unwrap(), b"v1")]);

    let mut session = memory.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("x").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(image.entry(&VirtualPath::data("x").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::data("x").unwrap(),
            "a4",
            expected,
            b"v2".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    // A concurrent mutation changes the read set between capture and apply, so
    // apply must reject with the same typed retryable conflict the JSON kernel
    // returns from its pre-journal revalidation.
    {
        let mut state = memory.repository_state();
        state.entries.insert(
            VirtualPath::data("x").unwrap(),
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes("mem:x", b"changed").unwrap(),
                bytes: b"changed".to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
    assert!(matches!(
        session.apply(&test_plan(&image, &delta)),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

#[test]
fn test_session_prepared_delete_recovery_restores_original() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("victim"), b"old").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Publish the DeleteFile, then interrupt before commit → Prepared residue.
    let storage = JsonFileStorage::with_repository_state_failures(
        &data,
        SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
    );
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::phase_one([VirtualPath::data("victim").unwrap()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let expected =
        ExpectedPreimage::of(image.entry(&VirtualPath::data("victim").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::delete_file(
            VirtualPath::data("victim").unwrap(),
            "a5",
            expected,
        )],
    )
    .unwrap();
    assert!(session.apply(&test_plan(&image, &delta)).is_err());
    drop(session);

    // Prepared-journal recovery restores the original file from its backup.
    JsonFileStorage::new(&data)
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();
    assert_eq!(std::fs::read(data.join("victim")).unwrap(), b"old");
    assert!(!worktree.path().join(".jit-bootstrap").exists());
}

// --- REQ-01: typed mutation determinism across both backends -------------
//
// The finalizer's serialized issue/gate/provenance/event bytes and the exact
// identifiers/timestamps it assigns are a pure function of the captured image
// and the injected mutation context, so a fixed seed and clock yield
// byte-identical results regardless of backend. No-op operations sample
// neither an identifier nor the mutation timestamp.

use crate::domain::{Assignee, Event, GateState, GateStatus, Issue};
use crate::repository_state::{
    finalize, serialize_gate_run, serialize_issue, MutationClock, MutationContext, MutationIntent,
};

fn req01_instant() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

fn req01_issue(id: &str, assignee: Option<Assignee>) -> Issue {
    let mut issue = crate::domain::types::fixture_issue("Determinism".into(), "Body".into());
    issue.id = id.to_string();
    issue.assignee = assignee;
    issue.created_at = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    issue.updated_at = issue.created_at;
    issue
}

fn req01_map_issue(reverse: bool) -> Issue {
    let mut issue =
        crate::domain::types::fixture_issue("Canonical backend retry".into(), "Body".into());
    issue.id = "11111111-1111-4111-8111-111111111111".into();
    let order = if reverse { [1, 0] } else { [0, 1] };
    let context = [("alpha", "one"), ("zeta", "two")];
    let gates = [
        (
            "alpha-gate",
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("agent:alpha".parse().unwrap()),
                updated_at: req01_instant(),
            },
        ),
        (
            "zeta-gate",
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("agent:zeta".parse().unwrap()),
                updated_at: req01_instant(),
            },
        ),
    ];
    issue.context = order
        .iter()
        .map(|index| {
            let (key, value) = context[*index];
            (key.to_string(), value.to_string())
        })
        .collect::<HashMap<_, _>>();
    issue.gates_status = order
        .iter()
        .map(|index| gates[*index].clone())
        .map(|(key, value)| (key.to_string(), value))
        .collect::<HashMap<_, _>>();
    issue
}

/// Seed one existing issue (and optional event log) into the memory backend's
/// repository-state image, including the `issues` directory parent.
fn seed_memory_issue(memory: &InMemoryStorage, id: &str, issue: &Issue, events: &[u8]) {
    let mut state = memory.repository_state();
    state.data_root_exists = true;
    for dir in ["", "issues"] {
        state.entries.insert(
            VirtualPath::data(dir).unwrap(),
            RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes(format!("mem-dir:{dir}"), b"directory").unwrap(),
                mode: FileMode::Executable,
            },
        );
    }
    let issue_bytes = serialize_issue(issue).unwrap();
    state.entries.insert(
        VirtualPath::data(format!("issues/{id}.json")).unwrap(),
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes("mem-issue", &issue_bytes).unwrap(),
            bytes: issue_bytes,
            mode: FileMode::Regular,
        },
    );
    if !events.is_empty() {
        state.entries.insert(
            VirtualPath::data("events.jsonl").unwrap(),
            RepositoryEntry::File {
                identity: EntryIdentity::for_bytes("mem-events", events).unwrap(),
                bytes: events.to_vec(),
                mode: FileMode::Regular,
            },
        );
    }
}

fn claim_spec(id: &str) -> CaptureSpec {
    CaptureSpec::phase_one(
        [
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
        ],
        budget(),
    )
    .unwrap()
}

#[test]
fn test_conformance_claim_finalizer_bytes_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let id = "44444444-4444-4444-8444-444444444444";
    let agent: Assignee = "agent:worker-1".parse().unwrap();
    let issue = req01_issue(id, None);
    let issue_bytes = serialize_issue(&issue).unwrap();

    // Seed identical state on the JSON backend (real files).
    std::fs::create_dir_all(data.join("issues")).unwrap();
    std::fs::write(data.join(format!("issues/{id}.json")), &issue_bytes).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    // Seed identical state on the memory backend's image.
    let memory = InMemoryStorage::new();
    seed_memory_issue(&memory, id, &issue, &[]);

    let intents = [MutationIntent::ClaimIssue {
        issue_id: id.to_string(),
        agent: agent.clone(),
    }];

    // A deterministic context: fixed seed and clock, one per backend.
    let json = JsonFileStorage::new(&data);
    let json_actions = {
        let mut session = json.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(claim_spec(id)).unwrap();
        let context = MutationContext::deterministic([7u8; 32], req01_instant());
        let delta = finalize(&layout, &image, &context, &intents).unwrap();
        let outcome = session.apply(&delta).unwrap();
        outcome.actions_applied
    };
    let memory_actions = {
        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(claim_spec(id)).unwrap();
        let context = MutationContext::deterministic([7u8; 32], req01_instant());
        let delta = finalize(&layout, &image, &context, &intents).unwrap();
        let outcome = session.apply(&delta).unwrap();
        outcome.actions_applied
    };
    assert_eq!(json_actions, memory_actions, "identical action count");

    // Post-state bytes are byte-identical across backends.
    let json_issue = std::fs::read(data.join(format!("issues/{id}.json"))).unwrap();
    let json_events = std::fs::read(data.join("events.jsonl")).unwrap();
    let (memory_issue, memory_events) = {
        let state = memory.repository_state();
        let issue = match state
            .entries
            .get(&VirtualPath::data(format!("issues/{id}.json")).unwrap())
            .unwrap()
        {
            RepositoryEntry::File { bytes, .. } => bytes.clone(),
            other => panic!("expected issue file, got {other:?}"),
        };
        let events = match state
            .entries
            .get(&VirtualPath::data("events.jsonl").unwrap())
            .unwrap()
        {
            RepositoryEntry::File { bytes, .. } => bytes.clone(),
            other => panic!("expected events file, got {other:?}"),
        };
        (issue, events)
    };
    assert_eq!(json_issue, memory_issue, "issue bytes deterministic");
    assert_eq!(json_events, memory_events, "event bytes deterministic");

    // The finalized issue carries the assignee and the exact mutation time.
    let written: Issue = serde_json::from_slice(&json_issue).unwrap();
    assert_eq!(written.assignee, Some(agent));
    assert_eq!(written.claimed_at, Some(req01_instant()));
    assert_eq!(written.updated_at, req01_instant());
    // Exactly one event line, stamped with the mutation timestamp.
    let events =
        crate::domain::parse_known_events(std::str::from_utf8(&json_events).unwrap()).unwrap();
    assert_eq!(events.len(), 1);
    let Event::IssueClaimed { timestamp, .. } = &events[0] else {
        panic!("expected an issue_claimed event");
    };
    assert_eq!(*timestamp, req01_instant());
}

#[test]
fn test_conformance_claim_noop_samples_nothing_on_both_backends() {
    // A clock that panics on use proves neither backend samples time for a
    // fully reflected claim.
    struct PanicClock;
    impl MutationClock for PanicClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            panic!("no-op must not sample the mutation clock");
        }
    }

    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let id = "55555555-5555-4555-8555-555555555555";
    let agent: Assignee = "agent:worker-1".parse().unwrap();
    let issue = req01_issue(id, Some(agent.clone()));
    // A committed claim event so both issue and tail already reflect the claim.
    let claim_event = Event::IssueClaimed {
        id: "evt".into(),
        issue_id: id.to_string(),
        timestamp: req01_instant(),
        assignee: agent.clone(),
    };
    let mut events_bytes: Vec<u8> = crate::repository_state::serialize_event(&claim_event).unwrap();
    events_bytes.push(b'\n');

    std::fs::create_dir_all(data.join("issues")).unwrap();
    std::fs::write(
        data.join(format!("issues/{id}.json")),
        serialize_issue(&issue).unwrap(),
    )
    .unwrap();
    std::fs::write(data.join("events.jsonl"), &events_bytes).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let memory = InMemoryStorage::new();
    seed_memory_issue(&memory, id, &issue, &events_bytes);

    let intents = [MutationIntent::ClaimIssue {
        issue_id: id.to_string(),
        agent,
    }];

    for run_memory in [false, true] {
        let context = MutationContext::new(
            crate::repository_state::IdAuthority::from_seed([3u8; 32]),
            Box::new(PanicClock),
        );
        let actions = if run_memory {
            let mut session = memory.open_mutation_session(layout.clone()).unwrap();
            let image = session.capture(claim_spec(id)).unwrap();
            let delta = finalize(&layout, &image, &context, &intents).unwrap();
            session.apply(&delta).unwrap().actions_applied
        } else {
            let json = JsonFileStorage::new(&data);
            let mut session = json.open_mutation_session(layout.clone()).unwrap();
            let image = session.capture(claim_spec(id)).unwrap();
            let delta = finalize(&layout, &image, &context, &intents).unwrap();
            session.apply(&delta).unwrap().actions_applied
        };
        assert_eq!(
            actions, 0,
            "no-op emits no actions (run_memory={run_memory})"
        );
    }
}

#[test]
fn test_conformance_claim_noop_transaction_hash_covers_context_seed() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "wt", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
    )
    .unwrap();
    let id = "56565656-5656-4565-8565-565656565656";
    let agent: Assignee = "agent:worker-1".parse().unwrap();
    let issue = req01_issue(id, Some(agent.clone()));
    let claim_event = Event::IssueClaimed {
        id: "evt".into(),
        issue_id: id.to_string(),
        timestamp: req01_instant(),
        assignee: agent.clone(),
    };
    let mut events = crate::repository_state::serialize_event(&claim_event).unwrap();
    events.push(b'\n');
    let intents = [MutationIntent::ClaimIssue {
        issue_id: id.to_string(),
        agent,
    }];

    let hashes = [[11u8; 32], [12u8; 32]].map(|seed| {
        let memory = InMemoryStorage::new();
        seed_memory_issue(&memory, id, &issue, &events);
        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(claim_spec(id)).unwrap();
        let context = MutationContext::deterministic(seed, req01_instant());
        let plan = finalize(&layout, &image, &context, &intents).unwrap();
        assert!(plan.delta().actions().is_empty());
        session.apply(&plan).unwrap().transaction_hash
    });

    assert_ne!(
        hashes[0], hashes[1],
        "the unchanged context seed identity must enter even a no-op plan hash"
    );
}

#[test]
fn test_conformance_identical_full_finalizer_plan_hash_matches_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec =
        CaptureSpec::phase_one([VirtualPath::data("events.jsonl").unwrap()], budget()).unwrap();
    let event = Event::GateDefinitionCreated {
        id: String::new(),
        timestamp: req01_instant(),
        gate_key: "cargo-ci".to_string(),
    };
    let intents = [MutationIntent::RecordEvent {
        phase: 0,
        event: Box::new(event),
    }];

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(spec.clone()).unwrap();
    let json_context = MutationContext::deterministic([21u8; 32], req01_instant());
    let json_plan = finalize(&layout, &json_image, &json_context, &intents).unwrap();

    let memory = InMemoryStorage::new();
    memory.repository_state().data_root_exists = true;
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(spec).unwrap();
    let memory_context = MutationContext::deterministic([21u8; 32], req01_instant());
    let memory_plan = finalize(&layout, &memory_image, &memory_context, &intents).unwrap();

    assert_eq!(
        json_image, memory_image,
        "captured evidence must be identical"
    );
    assert_eq!(json_plan, memory_plan);
    let json_outcome = json_session.apply(&json_plan).unwrap();
    let memory_outcome = memory_session.apply(&memory_plan).unwrap();
    assert_eq!(json_outcome.transaction_hash, json_plan.hash());
    assert_eq!(memory_outcome.transaction_hash, json_plan.hash());
}

#[test]
fn test_conformance_reconstructed_issue_maps_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir_all(data.join("issues")).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let issue_id = crate::repository_state::IdAuthority::from_seed([31u8; 32]).uuid_at(0);
    let spec = CaptureSpec::phase_one(
        [
            VirtualPath::data("issues").unwrap(),
            VirtualPath::data("index.json").unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
            VirtualPath::data(format!("issues/{issue_id}.json")).unwrap(),
        ],
        budget(),
    )
    .unwrap();
    let json_draft = req01_map_issue(false);
    let memory_draft = req01_map_issue(true);
    assert_eq!(json_draft, memory_draft);
    assert_eq!(
        serialize_issue(&json_draft).unwrap(),
        serialize_issue(&memory_draft).unwrap()
    );

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(spec.clone()).unwrap();
    let json_plan = finalize(
        &layout,
        &json_image,
        &MutationContext::deterministic([31u8; 32], req01_instant()),
        &[MutationIntent::CreateIssue {
            draft: Box::new(json_draft),
        }],
    )
    .unwrap();

    let memory = InMemoryStorage::new();
    {
        let mut state = memory.repository_state();
        state.data_root_exists = true;
        let issues_dir = VirtualPath::data("issues").unwrap();
        state.entries.insert(
            issues_dir.clone(),
            json_image.entries().get(&issues_dir).unwrap().clone(),
        );
    }
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(spec).unwrap();
    let memory_plan = finalize(
        &layout,
        &memory_image,
        &MutationContext::deterministic([31u8; 32], req01_instant()),
        &[MutationIntent::CreateIssue {
            draft: Box::new(memory_draft),
        }],
    )
    .unwrap();

    assert_eq!(json_image, memory_image);
    assert_eq!(
        serde_json::to_vec(json_plan.delta()).unwrap(),
        serde_json::to_vec(memory_plan.delta()).unwrap()
    );
    assert_eq!(json_plan.hash(), memory_plan.hash());
    let json_outcome = json_session.apply(&json_plan).unwrap();
    let memory_outcome = memory_session.apply(&memory_plan).unwrap();
    assert_eq!(
        json_outcome.transaction_hash,
        memory_outcome.transaction_hash
    );
}

#[test]
fn test_full_finalizer_hash_changes_with_captured_evidence() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "wt", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
    )
    .unwrap();
    let id = "57575757-5757-4575-8575-575757575757";
    let agent: Assignee = "agent:worker-1".parse().unwrap();
    let issue = req01_issue(id, Some(agent.clone()));
    let intents = [MutationIntent::ClaimIssue {
        issue_id: id.to_string(),
        agent: agent.clone(),
    }];

    let hashes = ["evidence-a", "evidence-b"].map(|event_id| {
        let event = Event::IssueClaimed {
            id: event_id.to_string(),
            issue_id: id.to_string(),
            timestamp: req01_instant(),
            assignee: agent.clone(),
        };
        let mut events = crate::repository_state::serialize_event(&event).unwrap();
        events.push(b'\n');
        let memory = InMemoryStorage::new();
        seed_memory_issue(&memory, id, &issue, &events);
        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(claim_spec(id)).unwrap();
        let context = MutationContext::deterministic([22u8; 32], req01_instant());
        let plan = finalize(&layout, &image, &context, &intents).unwrap();
        assert!(plan.delta().actions().is_empty());
        plan.hash().to_string()
    });

    assert_ne!(hashes[0], hashes[1]);
}

#[test]
fn test_full_finalizer_allocated_ids_enter_delta_and_retry_reuses_hash() {
    let temp = TempDir::new().unwrap();
    let layout = RepositoryLayout::new(
        RepositoryRootEvidence::new(temp.path(), "wt", true),
        RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
    )
    .unwrap();
    let id = "58585858-5858-4585-8585-585858585858";
    let agent: Assignee = "agent:worker-1".parse().unwrap();
    let issue = req01_issue(id, None);
    let memory = InMemoryStorage::new();
    seed_memory_issue(&memory, id, &issue, &[]);
    let mut session = memory.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(claim_spec(id)).unwrap();
    let intents = [MutationIntent::ClaimIssue {
        issue_id: id.to_string(),
        agent,
    }];
    let context = MutationContext::deterministic([23u8; 32], req01_instant());

    let first = finalize(&layout, &image, &context, &intents).unwrap();
    let retry = finalize(&layout, &image, &context, &intents).unwrap();
    assert_eq!(first, retry, "a retry reuses the complete plan hash");

    let event_bytes = first
        .delta()
        .actions()
        .iter()
        .find_map(|action| match action {
            RepositoryAction::WriteFile { path, bytes, .. }
                if path == &VirtualPath::data("events.jsonl").unwrap() =>
            {
                Some(bytes)
            }
            _ => None,
        })
        .unwrap();
    let events =
        crate::domain::parse_known_events(std::str::from_utf8(event_bytes).unwrap()).unwrap();
    let Event::IssueClaimed { id: event_id, .. } = &events[0] else {
        panic!("expected allocated claim event");
    };
    assert_eq!(
        event_id,
        &crate::repository_state::IdAuthority::from_seed([23u8; 32]).uuid_at(0)
    );
    let seed = context.repository_seed(&intents).unwrap();
    assert_eq!(
        first.hash(),
        plan_hash(
            &image,
            &seed,
            &MaterializationIntent::SemanticMutation,
            first.delta()
        )
        .unwrap(),
        "the delta containing every allocated id is part of the hash"
    );
}

#[test]
fn test_conformance_empty_finalizer_noop_hash_matches_without_clock_sampling() {
    struct PanicClock;
    impl MutationClock for PanicClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            panic!("empty no-op must not sample time");
        }
    }

    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec =
        CaptureSpec::phase_one([VirtualPath::data("events.jsonl").unwrap()], budget()).unwrap();
    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(spec.clone()).unwrap();
    let json_context = MutationContext::new(
        crate::repository_state::IdAuthority::from_seed([24u8; 32]),
        Box::new(PanicClock),
    );
    let json_plan = finalize(&layout, &json_image, &json_context, &[]).unwrap();

    let memory = InMemoryStorage::new();
    memory.repository_state().data_root_exists = true;
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(spec).unwrap();
    let memory_context = MutationContext::new(
        crate::repository_state::IdAuthority::from_seed([24u8; 32]),
        Box::new(PanicClock),
    );
    let memory_plan = finalize(&layout, &memory_image, &memory_context, &[]).unwrap();

    assert_eq!(json_plan, memory_plan);
    assert!(json_plan.delta().actions().is_empty());
    let json_outcome = json_session.apply(&json_plan).unwrap();
    let memory_outcome = memory_session.apply(&memory_plan).unwrap();
    assert_eq!(json_outcome, memory_outcome);
    assert_eq!(json_outcome.actions_applied, 0);
}

#[test]
fn test_conformance_gate_run_and_provenance_bytes_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let run_id = "run-deterministic-1";
    let result = crate::domain::GateRunResult {
        schema_version: 1,
        run_id: run_id.to_string(),
        gate_key: "cargo-ci".to_string(),
        stage: crate::declarations::GateStage::Postcheck,
        issue_id: "issue-1".to_string(),
        commit: Some("abc123".to_string()),
        branch: Some("main".to_string()),
        tree_dirty: Some(false),
        status: crate::domain::GateRunStatus::Passed,
        started_at: req01_instant(),
        completed_at: Some(req01_instant()),
        duration_ms: Some(1500),
        exit_code: Some(0),
        stdout: "ok".to_string(),
        stderr: String::new(),
        command: "cargo test".to_string(),
        by: Some("agent:worker-1".to_string()),
        message: None,
        findings: None,
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
    };
    let provenance = Event::GateDefinitionCreated {
        id: "prov-1".into(),
        timestamp: req01_instant(),
        gate_key: "cargo-ci".to_string(),
    };
    let mut event_line = crate::repository_state::serialize_event(&provenance).unwrap();
    event_line.push(b'\n');
    let gate_bytes = serialize_gate_run(&result).unwrap();
    assert!(gate_bytes.starts_with(b"{\n"));
    assert!(!gate_bytes.ends_with(b"\n"));

    // Build one delta persisting the gate-run artifact plus the provenance
    // event; both backends must write byte-identical content.
    let run_dir = VirtualPath::data(format!("gate-runs/{run_id}")).unwrap();
    let run_file = VirtualPath::data(format!("gate-runs/{run_id}/result.json")).unwrap();
    let events = VirtualPath::data("events.jsonl").unwrap();
    let build_delta = |layout: &RepositoryLayout| {
        RepositoryDelta::new(
            layout,
            vec![
                RepositoryAction::create_directory(
                    run_dir.clone(),
                    "gate",
                    ExpectedPreimage::Absent,
                ),
                RepositoryAction::write_file(
                    run_file.clone(),
                    "gate",
                    ExpectedPreimage::Absent,
                    gate_bytes.clone(),
                    FileMode::Regular,
                ),
                RepositoryAction::write_file(
                    events.clone(),
                    "provenance",
                    ExpectedPreimage::Absent,
                    event_line.clone(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap()
    };
    let spec = || {
        CaptureSpec::phase_one(
            [
                VirtualPath::data("gate-runs").unwrap(),
                run_dir.clone(),
                run_file.clone(),
                events.clone(),
            ],
            budget(),
        )
        .unwrap()
    };

    std::fs::create_dir_all(data.join("gate-runs")).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let json = JsonFileStorage::new(&data);
    {
        let mut session = json.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(spec()).unwrap();
        let delta = build_delta(&layout);
        session.apply(&test_plan(&image, &delta)).unwrap();
    }

    let memory = InMemoryStorage::new();
    {
        let mut state = memory.repository_state();
        state.data_root_exists = true;
        for dir in ["", "gate-runs"] {
            state.entries.insert(
                VirtualPath::data(dir).unwrap(),
                RepositoryEntry::Directory {
                    identity: EntryIdentity::for_bytes(format!("mem-dir:{dir}"), b"directory")
                        .unwrap(),
                    mode: FileMode::Executable,
                },
            );
        }
    }
    {
        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(spec()).unwrap();
        let delta = build_delta(&layout);
        session.apply(&test_plan(&image, &delta)).unwrap();
    }

    // Gate-run and provenance bytes are byte-identical across backends.
    let json_gate = std::fs::read(data.join(format!("gate-runs/{run_id}/result.json"))).unwrap();
    let json_events = std::fs::read(data.join("events.jsonl")).unwrap();
    let state = memory.repository_state();
    let memory_gate = match state.entries.get(&run_file).unwrap() {
        RepositoryEntry::File { bytes, .. } => bytes.clone(),
        other => panic!("expected gate-run file, got {other:?}"),
    };
    let memory_events = match state.entries.get(&events).unwrap() {
        RepositoryEntry::File { bytes, .. } => bytes.clone(),
        other => panic!("expected events file, got {other:?}"),
    };
    assert_eq!(
        json_gate, gate_bytes,
        "gate-run bytes are the serializer output"
    );
    assert_eq!(json_gate, memory_gate, "gate-run bytes deterministic");
    assert_eq!(json_events, memory_events, "provenance bytes deterministic");
}

// Drive the full finalizer (not a hand-built delta) on both backends and
// assert byte-identical serialized output, deterministic identifiers, and
// canonical event ordering.

fn finalize_apply<S: RepositoryStateStore>(
    store: &S,
    layout: &RepositoryLayout,
    spec: CaptureSpec,
    intents: &[MutationIntent],
) {
    let mut session = store.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(spec).unwrap();
    let context = MutationContext::deterministic([7u8; 32], req01_instant());
    let delta = finalize(layout, &image, &context, intents).unwrap();
    session.apply(&delta).unwrap();
}

fn memory_dir(name: &str) -> (VirtualPath, RepositoryEntry) {
    (
        VirtualPath::data(name).unwrap(),
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes(format!("mem-dir:{name}"), b"directory").unwrap(),
            mode: FileMode::Executable,
        },
    )
}

fn memory_file(path: &VirtualPath, bytes: &[u8]) -> (VirtualPath, RepositoryEntry) {
    (
        path.clone(),
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes(format!("mem-file:{path:?}"), bytes).unwrap(),
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        },
    )
}

fn memory_bytes(memory: &InMemoryStorage, path: &VirtualPath) -> Vec<u8> {
    match memory.repository_state().entries.get(path).unwrap() {
        RepositoryEntry::File { bytes, .. } => bytes.clone(),
        other => panic!("expected a file at {path:?}, got {other:?}"),
    }
}

fn provenance_event() -> Event {
    Event::ProfileApplied {
        id: String::new(),
        timestamp: req01_instant(),
        profile_id: "example".into(),
        version: "1.0".into(),
        origin: crate::domain::ProfileOrigin::Directory(
            crate::repository_state::RootRelativePath::parse("packages/example")
                .expect("a canonical package location"),
        ),
        package_hash: "hash".into(),
        target_hashes: std::collections::BTreeMap::new(),
        isolated_torn_tail: false,
    }
}

fn gate_def_created(gate_key: &str) -> Event {
    Event::GateDefinitionCreated {
        id: String::new(),
        timestamp: req01_instant(),
        gate_key: gate_key.into(),
    }
}

#[test]
fn test_conformance_finalize_gate_run_and_provenance_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let run_id = crate::repository_state::IdAuthority::from_seed([7u8; 32]).uuid_at(0);
    let run_dir = VirtualPath::data(format!("gate-runs/{run_id}")).unwrap();
    let run_file = VirtualPath::data(format!("gate-runs/{run_id}/result.json")).unwrap();
    let events = VirtualPath::data("events.jsonl").unwrap();
    let draft = crate::domain::GateRunResult {
        schema_version: 1,
        run_id: "PLACEHOLDER".to_string(),
        gate_key: "cargo-ci".to_string(),
        stage: crate::declarations::GateStage::Postcheck,
        issue_id: "issue-1".to_string(),
        commit: Some("abc123".to_string()),
        branch: Some("main".to_string()),
        tree_dirty: Some(false),
        status: crate::domain::GateRunStatus::Passed,
        started_at: req01_instant(),
        completed_at: Some(req01_instant()),
        duration_ms: Some(1500),
        exit_code: Some(0),
        stdout: "ok".to_string(),
        stderr: String::new(),
        command: "cargo test".to_string(),
        by: Some("agent:worker-1".to_string()),
        message: None,
        findings: None,
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
    };
    let intents = || {
        [
            MutationIntent::RecordGateRun {
                draft: Box::new(draft.clone()),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(provenance_event()),
            },
        ]
    };
    let spec = || {
        CaptureSpec::phase_one(
            [
                VirtualPath::data("gate-runs").unwrap(),
                run_dir.clone(),
                run_file.clone(),
                events.clone(),
            ],
            budget(),
        )
        .unwrap()
    };

    std::fs::create_dir_all(data.join("gate-runs")).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    finalize_apply(&JsonFileStorage::new(&data), &layout, spec(), &intents());

    let memory = InMemoryStorage::new();
    {
        let mut state = memory.repository_state();
        state.data_root_exists = true;
        for (path, entry) in [memory_dir(""), memory_dir("gate-runs")] {
            state.entries.insert(path, entry);
        }
    }
    finalize_apply(&memory, &layout, spec(), &intents());

    let json_gate = std::fs::read(data.join(format!("gate-runs/{run_id}/result.json"))).unwrap();
    let json_events = std::fs::read(data.join("events.jsonl")).unwrap();
    assert_eq!(
        json_gate,
        memory_bytes(&memory, &run_file),
        "finalize gate-run bytes deterministic across backends"
    );
    assert_eq!(
        json_events,
        memory_bytes(&memory, &events),
        "finalize provenance bytes deterministic across backends"
    );
    // The finalizer assigned the deterministic run id.
    let persisted: crate::domain::GateRunResult = serde_json::from_slice(&json_gate).unwrap();
    assert_eq!(persisted.run_id, run_id);
}

#[test]
fn test_conformance_finalize_create_issue_multi_event_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let ids = crate::repository_state::IdAuthority::from_seed([7u8; 32]);
    let issue_id = ids.uuid_at(0);
    let issue_file = VirtualPath::data(format!("issues/{issue_id}.json")).unwrap();
    let index = VirtualPath::data("index.json").unwrap();
    let events = VirtualPath::data("events.jsonl").unwrap();
    let index_bytes = crate::repository_state::fresh_index_bytes().unwrap();

    let mut draft = crate::domain::types::fixture_issue("New".into(), "Body".into());
    draft.state = crate::domain::State::Ready;
    let intents = || {
        [
            MutationIntent::CreateIssue {
                draft: Box::new(draft.clone()),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(gate_def_created("aaa")),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(gate_def_created("bbb")),
            },
        ]
    };
    let spec = || {
        CaptureSpec::phase_one(
            [
                VirtualPath::data("issues").unwrap(),
                issue_file.clone(),
                index.clone(),
                events.clone(),
            ],
            budget(),
        )
        .unwrap()
    };

    std::fs::create_dir_all(data.join("issues")).unwrap();
    std::fs::write(data.join("index.json"), &index_bytes).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    finalize_apply(&JsonFileStorage::new(&data), &layout, spec(), &intents());

    let memory = InMemoryStorage::new();
    {
        let mut state = memory.repository_state();
        state.data_root_exists = true;
        for (path, entry) in [memory_dir(""), memory_dir("issues")] {
            state.entries.insert(path, entry);
        }
        let (path, entry) = memory_file(&index, &index_bytes);
        state.entries.insert(path, entry);
    }
    finalize_apply(&memory, &layout, spec(), &intents());

    // Byte-identical issue, index membership, and event log across backends.
    let json_issue = std::fs::read(data.join(format!("issues/{issue_id}.json"))).unwrap();
    let json_index = std::fs::read(data.join("index.json")).unwrap();
    let json_events = std::fs::read(data.join("events.jsonl")).unwrap();
    assert_eq!(json_issue, memory_bytes(&memory, &issue_file));
    assert_eq!(json_index, memory_bytes(&memory, &index));
    assert_eq!(json_events, memory_bytes(&memory, &events));
    assert!(String::from_utf8_lossy(&json_index).contains(&issue_id));

    // Canonical event order and the frozen identifier sequence (issue id is
    // uuid_at(0), so the three events take uuid_at(1..=3)).
    let parsed =
        crate::domain::parse_known_events(std::str::from_utf8(&json_events).unwrap()).unwrap();
    assert_eq!(parsed.len(), 3);
    let event_id = |event: &Event| match event {
        Event::IssueCreated { id, .. } | Event::GateDefinitionCreated { id, .. } => id.clone(),
        other => panic!("unexpected event {other:?}"),
    };
    assert!(matches!(&parsed[0], Event::IssueCreated { .. }));
    assert!(
        matches!(&parsed[1], Event::GateDefinitionCreated { gate_key, .. } if gate_key == "aaa")
    );
    assert!(
        matches!(&parsed[2], Event::GateDefinitionCreated { gate_key, .. } if gate_key == "bbb")
    );
    assert_eq!(event_id(&parsed[0]), ids.uuid_at(1));
    assert_eq!(event_id(&parsed[1]), ids.uuid_at(2));
    assert_eq!(event_id(&parsed[2]), ids.uuid_at(3));
}
