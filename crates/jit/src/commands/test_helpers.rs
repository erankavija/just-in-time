//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

use crate::commands::CommandExecutor;
use crate::declarations::GateRegistry;
use crate::domain::Issue;
use crate::repository_state::RepositoryIndex;
use crate::storage::{InMemoryStorage, IssueStore, MutationPoint};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

/// Seed one exact repository-file precondition in the aggregate memory image.
pub(crate) fn seed_repo_file(storage: &InMemoryStorage, path: &str, content: &str) {
    let layout = storage.repository_layout();
    let path = layout
        .classify_repository_relative(path)
        .expect("fixture path is canonical");
    match path.root_class() {
        crate::repository_state::RepositoryRootClass::Data => {
            storage.add_data_file(path.relative().as_path(), content)
        }
        crate::repository_state::RepositoryRootClass::Worktree => {
            storage.add_worktree_file(path.relative().as_path(), content)
        }
    }
}

/// Seed one issue record and its active index membership without invoking a
/// product mutation path. Lifecycle timestamps are preserved verbatim.
pub(crate) fn seed_issue(storage: &InMemoryStorage, issue: Issue) {
    storage.seed_issue_fixture(&issue);
}

/// Seed the data root's engine-owned ignore file when the fixture lacks it, with
/// the exact bytes its producer composes.
pub(crate) fn seed_data_root_ignore(storage: &InMemoryStorage) {
    if storage
        .read_repo_file(".jit/.gitignore")
        .expect("memory fixture ignore path is valid")
        .is_some()
    {
        return;
    }
    let bytes = crate::repository_state::render_data_root_ignore(b"")
        .expect("the engine-owned ignore region composes over an empty document");
    seed_repo_file(
        storage,
        ".jit/.gitignore",
        std::str::from_utf8(&bytes).expect("the composed ignore file is UTF-8"),
    );
}

/// Seed one exact authored gate-registry precondition.
pub(crate) fn seed_gate_registry(storage: &InMemoryStorage, registry: &GateRegistry) {
    let bytes = crate::declarations::serialize_gate_registry(registry)
        .expect("fixture gate registry serializes");
    seed_repo_file(
        storage,
        ".jit/gates.toml",
        std::str::from_utf8(&bytes).expect("gate registry TOML is UTF-8"),
    );
}

/// Count session opens via [`MutationPoint::SessionOpen`], the moment every
/// session open crosses exactly once, injecting no failure. Shared by tests
/// that assert a session-open budget.
pub(crate) struct SessionOpenCounter(AtomicUsize);

impl SessionOpenCounter {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(AtomicUsize::new(0)))
    }

    pub(crate) fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl crate::storage::MutationInterruptionInjector for SessionOpenCounter {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        if point == &MutationPoint::SessionOpen {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

pub(crate) enum SessionRaceAction {
    Save(Box<crate::domain::Issue>),
    Delete(String),
    WriteRepoFile {
        path: String,
        content: String,
    },
    /// A concurrent operation of its own, run against the raced store.
    Run(Box<dyn FnOnce(&InMemoryStorage) + Send>),
}

/// Performs one repository write when a session reaches `point` for the
/// `trigger`th time, standing in for a concurrent writer at that boundary.
struct SessionRace {
    point: MutationPoint,
    trigger: usize,
    hits: std::sync::atomic::AtomicUsize,
    storage: Mutex<Option<InMemoryStorage>>,
    action: Mutex<Option<SessionRaceAction>>,
}

impl crate::storage::MutationInterruptionInjector for SessionRace {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        if point == &self.point
            && self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == self.trigger
        {
            let storage = self.storage.lock().unwrap().clone().unwrap();
            match self.action.lock().unwrap().take().unwrap() {
                SessionRaceAction::Save(issue) => seed_issue(&storage, *issue),
                SessionRaceAction::Delete(id) => {
                    let bytes = storage
                        .read_repo_file(".jit/index.json")
                        .unwrap()
                        .expect("race fixture index exists");
                    let mut index = RepositoryIndex::parse(bytes.as_bytes()).unwrap();
                    index.mark_deleted(id);
                    let index = index.to_pretty_bytes().unwrap();
                    seed_repo_file(
                        &storage,
                        ".jit/index.json",
                        std::str::from_utf8(&index).unwrap(),
                    );
                }
                SessionRaceAction::WriteRepoFile { path, content } => {
                    seed_repo_file(&storage, &path, &content);
                }
                SessionRaceAction::Run(operation) => operation(&storage),
            }
        }
        Ok(())
    }
}

/// Race `action` against the `trigger`th session open, before that session
/// captures anything.
pub(crate) fn with_open_race(
    storage: InMemoryStorage,
    trigger: usize,
    action: SessionRaceAction,
) -> InMemoryStorage {
    with_session_race(storage, MutationPoint::SessionOpen, trigger, action)
}

/// Race `action` against the first apply, after its session captured the read
/// set and before apply revalidates it.
pub(crate) fn with_apply_race(
    storage: InMemoryStorage,
    action: SessionRaceAction,
) -> InMemoryStorage {
    with_session_race(storage, MutationPoint::BeforeCommitCheck, 1, action)
}

fn with_session_race(
    storage: InMemoryStorage,
    point: MutationPoint,
    trigger: usize,
    action: SessionRaceAction,
) -> InMemoryStorage {
    let race = Arc::new(SessionRace {
        point,
        trigger,
        hits: std::sync::atomic::AtomicUsize::new(0),
        storage: Mutex::new(None),
        action: Mutex::new(Some(action)),
    });
    let raced = storage.with_mutation_interruption_view(race.clone());
    *race.storage.lock().unwrap() = Some(raced.clone());
    raced
}

/// Attach the explicit synthetic layout to an in-memory executor fixture.
///
/// The fixture repository is back-filled with the state every real repository
/// carries and no test authors by hand: the minimum configuration, and the
/// data root's engine-owned ignore file, whose absence is derived-state drift.
pub fn memory_executor(storage: InMemoryStorage) -> CommandExecutor<InMemoryStorage> {
    if storage
        .read_repo_file(".jit/config.toml")
        .expect("memory fixture config path is valid")
        .is_none()
    {
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
    }
    seed_data_root_ignore(&storage);
    let layout = storage.repository_layout();
    CommandExecutor::new(storage).with_layout(layout)
}

/// Create an executor with enforcement mode configured.
///
/// # Arguments
///
/// * `mode` - Enforcement mode: "strict", "warn", or "off"
pub fn setup_with_enforcement(mode: &str) -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    let config_toml = format!(
        r#"
[worktree]
enforce_leases = "{}"
"#,
        mode
    );
    storage.add_data_file("config.toml", &config_toml);

    memory_executor(storage)
}

/// Create an executor with enforcement disabled (for backward compatibility tests).
pub fn setup() -> CommandExecutor<InMemoryStorage> {
    setup_with_enforcement("off")
}
