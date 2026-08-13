use jit::commands::{CommandExecutor, ProfileSelector};
use jit::profile::ProfileApplicationStatus;
use jit::storage::{
    discover_repository_layout, JsonFileStorage, RepositoryStateStore, TransactionFailureInjector,
    TransactionFailurePoint,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const PROFILE_ID: &str = "lifecycle-interruption";
pub(super) const PACKAGE_LOCATION: &str = "packages/lifecycle-interruption";
const RECORD_PATH: &str = ".jit/profiles/lifecycle-interruption.json";
const EVENT_PATH: &str = ".jit/events.jsonl";
const MANAGED_TARGETS: [&str; 3] = [
    "docs/lifecycle-interruption/added.txt",
    "docs/lifecycle-interruption/departing.txt",
    "docs/lifecycle-interruption/retained.txt",
];

/// The forward stages an initialized-repository profile publication reaches.
///
/// The shared kernel's four absent-data-root stages cannot be reached by a
/// profile lifecycle command: resolving an installed profile already requires
/// the repository data root and its ownership record. The three reverse-action
/// stages are rollback-recovery stages, not forward publication stages. All
/// other canonical stages are required below and discovered from a real run.
const EXPECTED_PROFILE_PUBLICATION_STAGES: [&str; 28] = [
    "recovery_external",
    "recovery_internal",
    "sweep_companions",
    "before_control_creation",
    "create_control",
    "before_initial_journal",
    "sync_initial_journal",
    "prepare_intent",
    "create_companion",
    "prepare_action",
    "stage_action",
    "sync_backup",
    "sync_stage",
    "before_prepared_journal",
    "sync_prepared_journal",
    "before_action",
    "before_target_mutation",
    "after_root_binding_check",
    "before_delete_rename",
    "after_target_mutation",
    "verify_final_identity",
    "after_action",
    "before_commit_decision",
    "after_commit",
    "cleanup",
    "before_stage_cleanup",
    "before_companion_cleanup",
    "before_control_cleanup",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LifecycleState {
    worktree_bytes: BTreeMap<&'static str, Option<Vec<u8>>>,
    ownership_bytes: Vec<u8>,
    ownership: Value,
    audit_bytes: Vec<u8>,
    audit: Vec<Value>,
}

impl LifecycleState {
    pub(super) fn capture(repository: &Path) -> Self {
        let worktree_bytes = MANAGED_TARGETS
            .into_iter()
            .map(|path| {
                let bytes = match fs::read(repository.join(path)) {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => panic!("read managed target {path}: {error}"),
                };
                (path, bytes)
            })
            .collect();
        let ownership_bytes = fs::read(repository.join(RECORD_PATH)).unwrap();
        let ownership = serde_json::from_slice(&ownership_bytes).unwrap();
        let audit_bytes = fs::read(repository.join(EVENT_PATH)).unwrap();
        let audit = audit_bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut event: Value = serde_json::from_slice(line).unwrap();
                let object = event.as_object_mut().expect("an audit event is an object");
                object.remove("id");
                object.remove("timestamp");
                event
            })
            .collect();
        Self {
            worktree_bytes,
            ownership_bytes,
            ownership,
            audit_bytes,
            audit,
        }
    }

    pub(super) fn is_semantically_equivalent_to(&self, other: &Self) -> bool {
        self.worktree_bytes == other.worktree_bytes
            && self.ownership_bytes == other.ownership_bytes
            && self.ownership == other.ownership
            && self.audit == other.audit
    }
}

#[derive(Default)]
struct RecordingFailures(Mutex<Vec<TransactionFailurePoint>>);

impl RecordingFailures {
    fn representatives(&self) -> Vec<TransactionFailurePoint> {
        let observed = self.0.lock().unwrap();
        let mut stages = BTreeSet::new();
        observed
            .iter()
            .filter(|point| stages.insert(stage_name(point)))
            .cloned()
            .collect()
    }
}

impl TransactionFailureInjector for RecordingFailures {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        self.0.lock().unwrap().push(point.clone());
        Ok(())
    }
}

struct FailOnce {
    point: TransactionFailurePoint,
    fired: AtomicBool,
}

impl FailOnce {
    fn at(point: TransactionFailurePoint) -> Arc<Self> {
        Arc::new(Self {
            point,
            fired: AtomicBool::new(false),
        })
    }

    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

impl TransactionFailureInjector for FailOnce {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &self.point
            && self
                .fired
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Err(std::io::Error::other(format!(
                "injected profile publication interruption at {point:?}"
            )));
        }
        Ok(())
    }
}

#[derive(Default)]
struct PublicationCounter(AtomicUsize);

impl PublicationCounter {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl TransactionFailureInjector for PublicationCounter {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &TransactionFailurePoint::RepositoryBeforeControlCreation {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Collapse per-action hook occurrences to the materially distinct canonical
/// transaction stages. This match is exhaustive so a new shared failure point
/// forces this lifecycle coverage to classify it.
fn stage_name(point: &TransactionFailurePoint) -> &'static str {
    use TransactionFailurePoint::*;
    match point {
        RepositoryRecoveryExternal => "recovery_external",
        RepositoryRecoveryInternal => "recovery_internal",
        RepositoryBeforeControlCreation => "before_control_creation",
        RepositoryCreateControl => "create_control",
        RepositoryBeforeInitialJournal => "before_initial_journal",
        RepositoryBeforeDataStageJournal => "before_data_stage_journal",
        RepositoryBeforePreparedJournal => "before_prepared_journal",
        RepositorySyncInitialJournal => "sync_initial_journal",
        RepositoryCreateCompanion => "create_companion",
        RepositorySweepCompanions => "sweep_companions",
        RepositoryPrepareIntent => "prepare_intent",
        RepositoryPrepareAction { .. } => "prepare_action",
        RepositoryStageAction { .. } => "stage_action",
        RepositorySyncStage { .. } => "sync_stage",
        RepositorySyncBackup { .. } => "sync_backup",
        RepositorySyncPreparedJournal => "sync_prepared_journal",
        RepositoryBeforeAction { .. } => "before_action",
        RepositoryBeforeTargetMutation { .. } => "before_target_mutation",
        RepositoryAfterRootBindingCheck { .. } => "after_root_binding_check",
        RepositoryBeforeDeleteRename { .. } => "before_delete_rename",
        RepositoryAfterTargetMutation { .. } => "after_target_mutation",
        RepositoryVerifyFinalIdentity { .. } => "verify_final_identity",
        RepositoryAfterAction { .. } => "after_action",
        RepositoryBeforeDataRootPublication => "before_data_root_publication",
        RepositoryAfterDataParentBindingCheck => "after_data_parent_binding_check",
        RepositoryAfterDataRootPublication => "after_data_root_publication",
        RepositoryBeforeCommitDecision => "before_commit_decision",
        RepositoryAfterCommit => "after_commit",
        RepositoryBeforeReverseAction { .. } => "before_reverse_action",
        RepositoryAfterReverseAction { .. } => "after_reverse_action",
        RepositoryBeforeRollbackDecision => "before_rollback_decision",
        RepositoryBeforeStageCleanup => "before_stage_cleanup",
        RepositoryBeforeCompanionCleanup => "before_companion_cleanup",
        RepositoryBeforeControlCleanup => "before_control_cleanup",
        RepositoryCleanup => "cleanup",
    }
}

pub(super) fn selector() -> ProfileSelector {
    ProfileSelector::path(PathBuf::from(PACKAGE_LOCATION))
}

pub(super) fn executor_with_failures(
    repository: &Path,
    failures: Arc<dyn TransactionFailureInjector>,
) -> CommandExecutor<JsonFileStorage> {
    let data = repository.join(".jit");
    let storage = JsonFileStorage::with_repository_state_failures(&data, failures);
    let layout = discover_repository_layout(repository, &data).unwrap();
    CommandExecutor::new(storage).with_layout(layout)
}

pub(super) fn ordinary_executor(repository: &Path) -> CommandExecutor<JsonFileStorage> {
    let data = repository.join(".jit");
    let storage = JsonFileStorage::new(&data);
    let layout = discover_repository_layout(repository, &data).unwrap();
    CommandExecutor::new(storage).with_layout(layout)
}

fn write_package(repository: &Path, version: &str) {
    let package = repository.join(PACKAGE_LOCATION);
    let assets = package.join("assets");
    fs::create_dir_all(&assets).unwrap();
    let manifest = if version == "1.0.0" {
        format!(
            "[profile]\nmanifest-version = 2\nid = \"{PROFILE_ID}\"\nversion = \"1.0.0\"\ncompatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/retained.txt\"\ntarget = \"docs/lifecycle-interruption/retained.txt\"\n\n\
             [[asset]]\nsource = \"assets/departing.txt\"\ntarget = \"docs/lifecycle-interruption/departing.txt\"\n"
        )
    } else {
        format!(
            "[profile]\nmanifest-version = 2\nid = \"{PROFILE_ID}\"\nversion = \"2.0.0\"\ncompatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/retained.txt\"\ntarget = \"docs/lifecycle-interruption/retained.txt\"\n\n\
             [[asset]]\nsource = \"assets/added.txt\"\ntarget = \"docs/lifecycle-interruption/added.txt\"\n"
        )
    };
    fs::write(package.join("manifest.toml"), manifest).unwrap();
    fs::write(
        assets.join("retained.txt"),
        format!("retained content from {version}\n"),
    )
    .unwrap();
    if version == "1.0.0" {
        fs::write(assets.join("departing.txt"), "departing content\n").unwrap();
    } else {
        match fs::remove_file(assets.join("departing.txt")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove departing package asset: {error}"),
        }
        fs::write(assets.join("added.txt"), "newly added content\n").unwrap();
    }
}

pub(super) fn changed_profile_fixture() -> (TempDir, LifecycleState) {
    let (repository, _storage) = jit::test_utils::setup_test_repo().unwrap();
    write_package(repository.path(), "1.0.0");
    let initial = ordinary_executor(repository.path())
        .apply_profile(&[selector()])
        .unwrap();
    assert_eq!(
        initial.requested().unwrap().status,
        ProfileApplicationStatus::Applied
    );
    write_package(repository.path(), "2.0.0");
    let prior = LifecycleState::capture(repository.path());
    assert_eq!(prior.ownership["version"], "1.0.0");
    assert_eq!(prior.audit.len(), 1);
    assert_eq!(
        prior.worktree_bytes["docs/lifecycle-interruption/retained.txt"],
        Some(b"retained content from 1.0.0\n".to_vec())
    );
    assert!(prior.worktree_bytes["docs/lifecycle-interruption/added.txt"].is_none());
    assert!(prior.worktree_bytes["docs/lifecycle-interruption/departing.txt"].is_some());
    (repository, prior)
}

fn recover(repository: &Path) {
    let data = repository.join(".jit");
    let storage = JsonFileStorage::new(&data);
    let layout = discover_repository_layout(repository, &data).unwrap();
    drop(storage.open_mutation_session(layout).unwrap());
    assert!(
        !repository.join(".jit/tmp/transactions").exists(),
        "ordinary recovery must remove internal transaction residue"
    );
    assert!(
        !repository.join(".jit-bootstrap").exists(),
        "ordinary recovery must remove worktree companion residue"
    );
}

fn assert_complete_new_state(actual: &LifecycleState, uninterrupted: &LifecycleState) {
    assert_eq!(actual.worktree_bytes, uninterrupted.worktree_bytes);
    assert_eq!(actual.ownership_bytes, uninterrupted.ownership_bytes);
    assert_eq!(actual.ownership, uninterrupted.ownership);
    assert_eq!(actual.audit, uninterrupted.audit);
}

#[test]
fn test_profile_apply_interruption_recovers_every_publication_stage_and_retry_is_idempotent() {
    let (uninterrupted_repository, uninterrupted_prior) = changed_profile_fixture();
    let recording = Arc::new(RecordingFailures::default());
    let uninterrupted = executor_with_failures(uninterrupted_repository.path(), recording.clone())
        .apply_profile(&[selector()])
        .unwrap();
    assert_eq!(
        uninterrupted.requested().unwrap().status,
        ProfileApplicationStatus::Applied
    );
    let uninterrupted_new = LifecycleState::capture(uninterrupted_repository.path());
    assert_eq!(uninterrupted_new.ownership["version"], "2.0.0");
    assert_eq!(
        uninterrupted_new.audit.len(),
        uninterrupted_prior.audit.len() + 1
    );
    assert!(
        uninterrupted_new.worktree_bytes["docs/lifecycle-interruption/departing.txt"].is_none()
    );
    assert_eq!(
        uninterrupted_new.worktree_bytes["docs/lifecycle-interruption/retained.txt"],
        Some(b"retained content from 2.0.0\n".to_vec())
    );
    assert_eq!(
        uninterrupted_new.worktree_bytes["docs/lifecycle-interruption/added.txt"],
        Some(b"newly added content\n".to_vec())
    );

    let stages = recording.representatives();
    assert_eq!(
        stages.iter().map(stage_name).collect::<BTreeSet<_>>(),
        EXPECTED_PROFILE_PUBLICATION_STAGES.into_iter().collect(),
        "the uninterrupted lifecycle run must traverse every applicable shared stage"
    );

    for point in stages {
        let stage = stage_name(&point);
        let (repository, prior) = changed_profile_fixture();
        let failure = FailOnce::at(point.clone());
        let interrupted =
            executor_with_failures(repository.path(), failure.clone()).apply_profile(&[selector()]);
        assert!(interrupted.is_err(), "{stage} must interrupt the command");
        assert!(failure.fired(), "{stage} must consume its canonical hook");

        recover(repository.path());
        let recovered = LifecycleState::capture(repository.path());
        let recovered_prior = recovered == prior;
        let recovered_new = recovered.is_semantically_equivalent_to(&uninterrupted_new);
        assert!(
            recovered_prior || recovered_new,
            "{stage} recovery left neither the complete prior nor complete new lifecycle state:\n{recovered:#?}"
        );

        let publications = Arc::new(PublicationCounter::default());
        let executor = executor_with_failures(repository.path(), publications.clone());
        let retried = executor.apply_profile(&[selector()]).unwrap();
        let retried_status = retried.requested().unwrap().status;
        assert_eq!(
            retried_status,
            if recovered_prior {
                ProfileApplicationStatus::Applied
            } else {
                ProfileApplicationStatus::Unchanged
            },
            "{stage} retry status must state whether recovery kept the prior or new state"
        );
        assert_eq!(
            publications.count(),
            usize::from(recovered_prior),
            "{stage} retry publishes exactly when recovery restored the prior state"
        );
        let after_retry = LifecycleState::capture(repository.path());
        assert_complete_new_state(&after_retry, &uninterrupted_new);

        let writes_after_retry = publications.count();
        let repeated = executor.apply_profile(&[selector()]).unwrap();
        assert_eq!(
            repeated.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged,
            "{stage} completed operation must report unchanged"
        );
        assert_eq!(
            publications.count(),
            writes_after_retry,
            "{stage} completed operation must not enter publication again"
        );
        assert_eq!(
            LifecycleState::capture(repository.path()),
            after_retry,
            "{stage} completed operation must not rewrite targets, ownership, or audit bytes"
        );
    }
}
