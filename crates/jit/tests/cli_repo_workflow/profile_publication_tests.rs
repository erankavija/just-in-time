//! Profile-lifecycle publication coverage: applying a changed profile package
//! reaches the repository through the shared mutation session.
//!
//! What an interrupted publication leaves and what a leftover one converges on
//! is the publication mechanism's own behaviour, proven once against it in
//! `storage::redo_conformance` (`@/inv/orthogonal-tests`). The lifecycle
//! fixture below is shared with the concurrency suite beside it.

use jit::commands::CommandExecutor;
use jit::profile::ProfileApplicationStatus;
use jit::storage::{
    discover_repository_layout, JsonFileStorage, MutationInterruptionInjector, MutationPoint,
    RedoPoint,
};
use jit::test_cli::JitCli;
use jit::test_utils::file_executor;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const PROFILE_ID: &str = "lifecycle-interruption";
pub(super) const PACKAGE_LOCATION: &str = "packages/lifecycle-interruption";
const EVENT_PATH: &str = ".jit/events.jsonl";
const MANAGED_TARGETS: [&str; 2] = [
    "docs/lifecycle-interruption/added.txt",
    "docs/lifecycle-interruption/retained.txt",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LifecycleState {
    worktree_bytes: BTreeMap<&'static str, Option<Vec<u8>>>,
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
            audit_bytes,
            audit,
        }
    }

    pub(super) fn is_semantically_equivalent_to(&self, other: &Self) -> bool {
        self.worktree_bytes == other.worktree_bytes && self.audit == other.audit
    }
}

#[derive(Default)]
struct PublicationCounter(AtomicUsize);

impl PublicationCounter {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl MutationInterruptionInjector for PublicationCounter {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        if point == &MutationPoint::Redo(RedoPoint::BeforeApply) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

pub(super) fn selector() -> PathBuf {
    PathBuf::from(PACKAGE_LOCATION)
}

pub(super) fn executor_with_interruptions(
    repository: &Path,
    interruptions: Arc<dyn MutationInterruptionInjector>,
) -> CommandExecutor<JsonFileStorage> {
    let data = repository.join(".jit");
    let storage = JsonFileStorage::with_mutation_interruptions(&data, interruptions);
    let layout = discover_repository_layout(repository, &data).unwrap();
    CommandExecutor::new(storage).with_layout(layout).unwrap()
}

fn write_package(repository: &Path, version: &str) {
    let package = repository.join(PACKAGE_LOCATION);
    let assets = package.join("assets");
    fs::create_dir_all(&assets).unwrap();
    let manifest = if version == "1.0.0" {
        format!(
            "[profile]\nmanifest-version = 2\nid = \"{PROFILE_ID}\"\nversion = \"1.0.0\"\ncompatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/retained.txt\"\ntarget = \"docs/lifecycle-interruption/retained.txt\"\n"
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
    if version != "1.0.0" {
        fs::write(assets.join("added.txt"), "newly added content\n").unwrap();
    }
}

/// Apply version 1.0.0 of the lifecycle package to `repository`, stage 2.0.0
/// beside it, and answer the state before 2.0.0 is applied.
pub(super) fn prepare_changed_profile(repository: &JitCli) -> LifecycleState {
    write_package(repository.root(), "1.0.0");
    let initial = jit::test_utils::apply_selected_packages(
        &file_executor(repository.root()).unwrap(),
        &[selector()],
    )
    .unwrap();
    assert_eq!(
        initial.requested().unwrap().status,
        ProfileApplicationStatus::Applied
    );
    write_package(repository.root(), "2.0.0");
    let prior = LifecycleState::capture(repository.root());
    assert_eq!(prior.audit.len(), 1);
    assert_eq!(
        prior.worktree_bytes["docs/lifecycle-interruption/retained.txt"],
        Some(b"retained content from 1.0.0\n".to_vec())
    );
    assert!(prior.worktree_bytes["docs/lifecycle-interruption/added.txt"].is_none());
    prior
}

/// One `profile apply` publishes its change through the shared mutation
/// session, and an application with nothing to publish enters no session of its
/// own.
#[test]
fn test_profile_apply_publishes_its_change_through_the_shared_mutation_session() {
    let repository = JitCli::initialised().unwrap();
    let prior = prepare_changed_profile(&repository);
    let publications = Arc::new(PublicationCounter::default());
    let executor = executor_with_interruptions(repository.root(), publications.clone());

    let applied = jit::test_utils::apply_selected_packages(&executor, &[selector()]).unwrap();

    assert_eq!(
        applied.requested().unwrap().status,
        ProfileApplicationStatus::Applied
    );
    assert_eq!(
        publications.count(),
        1,
        "the applied change must reach the repository through one shared publication"
    );
    let published = LifecycleState::capture(repository.root());
    assert_eq!(
        published.audit.len(),
        prior.audit.len() + 1,
        "the publication carries the lifecycle run's audit event"
    );
    assert_eq!(
        published.worktree_bytes["docs/lifecycle-interruption/retained.txt"],
        Some(b"retained content from 2.0.0\n".to_vec())
    );
    assert_eq!(
        published.worktree_bytes["docs/lifecycle-interruption/added.txt"],
        Some(b"newly added content\n".to_vec())
    );

    let repeated = jit::test_utils::apply_selected_packages(&executor, &[selector()]).unwrap();

    assert_eq!(
        repeated.requested().unwrap().status,
        ProfileApplicationStatus::Unchanged,
        "an application with nothing to publish must report unchanged"
    );
    assert_eq!(
        publications.count(),
        1,
        "an unchanged application must not enter publication again"
    );
    assert_eq!(
        LifecycleState::capture(repository.root()),
        published,
        "an unchanged application must not rewrite targets or audit bytes"
    );
}
