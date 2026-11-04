//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

pub use super::dependency::DependenciesAddResult;
use crate::commands::CommandExecutor;
use crate::declarations::GateRegistry;
use crate::domain::Issue;
use crate::repository_state::VirtualPath;
use crate::storage::{InMemoryStorage, IssueStore, MutationPoint};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

/// Kernel inspection for tests that own executor storage and rule fixtures.
pub trait ExecutorTestAccess<S: IssueStore> {
    fn storage(&self) -> &S;
    fn rules(&self) -> Result<&super::RuleSet, &super::RuleConfigError>;
    fn effective_rules(&self) -> anyhow::Result<&super::RuleSet>;
}

impl<S: IssueStore> ExecutorTestAccess<S> for CommandExecutor<S> {
    fn storage(&self) -> &S {
        CommandExecutor::storage(self)
    }

    fn rules(&self) -> Result<&super::RuleSet, &super::RuleConfigError> {
        CommandExecutor::rules(self)
    }

    fn effective_rules(&self) -> anyhow::Result<&super::RuleSet> {
        CommandExecutor::effective_rules(self)
    }
}

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

/// A session whose every capture is held to `max_bytes`, keeping each request's
/// read mode, so a test proves a capture fits a small budget without a large
/// fixture.
pub(crate) struct LimitedCaptureSession {
    pub(crate) inner: Box<dyn crate::storage::RepositoryMutationSession>,
    pub(crate) max_bytes: u64,
}

impl LimitedCaptureSession {
    fn limit(
        &self,
        spec: crate::repository_state::CaptureSpec,
    ) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, ReadMode};
        let mut limited = CaptureSpec::new(
            [],
            CaptureBudget {
                max_bytes: self.max_bytes,
                ..spec.budget()
            },
        )
        .unwrap();
        for path in spec.paths() {
            match spec.read_mode(path) {
                Some(ReadMode::Tail) => limited.discover_tail(path.clone()).unwrap(),
                Some(ReadMode::Kind) => limited.discover_kinds([path.clone()]).unwrap(),
                _ => limited.discover_paths([path.clone()]).unwrap(),
            }
        }
        limited
            .discover_listings(spec.listings().iter().cloned())
            .unwrap();
        for request in spec.pinned() {
            let (revision, path) = request;
            match spec.pinned_read_mode(request) {
                Some(ReadMode::Kind) => limited.discover_pinned_kind(revision, path).unwrap(),
                _ => limited.discover_pinned(revision, path).unwrap(),
            }
        }
        limited
    }
}

impl crate::storage::RepositoryReadSession for LimitedCaptureSession {
    fn layout(&self) -> &crate::repository_state::RepositoryLayout {
        self.inner.layout()
    }
    fn capture(
        &mut self,
        spec: crate::repository_state::CaptureSpec,
    ) -> std::result::Result<
        crate::repository_state::RepositoryImage,
        crate::storage::RepositoryStateStoreError,
    > {
        self.inner.capture(self.limit(spec))
    }
    fn extend(
        &mut self,
        image: crate::repository_state::RepositoryImage,
        spec: crate::repository_state::CaptureSpec,
    ) -> std::result::Result<
        crate::repository_state::RepositoryImage,
        crate::storage::RepositoryStateStoreError,
    > {
        self.inner.extend(image, self.limit(spec))
    }

    fn read_history_log(
        &mut self,
    ) -> std::result::Result<Option<Vec<u8>>, crate::storage::RepositoryStateStoreError> {
        self.inner.read_history_log()
    }
}

impl crate::storage::RepositoryMutationSession for LimitedCaptureSession {
    fn recovery_report(&self) -> &crate::storage::RecoveryDispatchReport {
        self.inner.recovery_report()
    }
    fn apply(
        &mut self,
        plan: &crate::repository_state::MaterializationPlan,
    ) -> std::result::Result<
        crate::storage::RepositoryApplyOutcome,
        crate::storage::RepositoryStateStoreError,
    > {
        self.inner.apply(plan)
    }
}

/// What `read` returns when it runs while another thread holds a mutation
/// session over `store`; panics when it has not finished within a bound, as a
/// read waiting on that writer would not.
pub(crate) fn read_beside_held_mutation_session<S, T>(
    store: &S,
    layout: &crate::repository_state::RepositoryLayout,
    read: impl FnOnce() -> T + Send,
) -> T
where
    S: crate::storage::RepositoryStateStore + Sync,
    T: Send,
{
    use std::sync::mpsc::channel;
    let (held, opened) = channel();
    let (release, released) = channel::<()>();
    let (finish, finished) = channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let session = store
                .open_mutation_session(layout.clone())
                .expect("the writer opens its session");
            held.send(()).unwrap();
            // Either a release or the test's end drops the session.
            let _ = released.recv();
            drop(session);
        });
        opened.recv().expect("the writer holds its session");
        scope.spawn(move || finish.send(read()));
        let outcome = finished.recv_timeout(std::time::Duration::from_secs(60));
        release.send(()).unwrap();
        outcome.expect("the read finishes while the writer holds its session")
    })
}

/// Seed one issue record without invoking a
/// product mutation path. Lifecycle timestamps are preserved verbatim.
pub(crate) fn seed_issue(storage: &InMemoryStorage, issue: Issue) {
    storage.seed_issue_fixture(&issue);
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
    WriteRepoFile {
        path: String,
        content: String,
    },
    /// A concurrent operation of its own, run against the raced store.
    Run(Box<dyn FnOnce(&InMemoryStorage) + Send>),
}

/// Performs one repository write when a session reaches `point` for the
/// `trigger`th time, standing in for a concurrent writer at that boundary.
/// Every moment first reaches the interruptions the raced store already had.
struct SessionRace {
    inner: Arc<dyn crate::storage::MutationInterruptionInjector>,
    point: MutationPoint,
    trigger: usize,
    hits: std::sync::atomic::AtomicUsize,
    storage: Mutex<Option<InMemoryStorage>>,
    action: Mutex<Option<SessionRaceAction>>,
}

impl crate::storage::MutationInterruptionInjector for SessionRace {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        self.inner.check(point)?;
        if point == &self.point
            && self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == self.trigger
        {
            let storage = self.storage.lock().unwrap().clone().unwrap();
            match self.action.lock().unwrap().take().unwrap() {
                SessionRaceAction::Save(issue) => seed_issue(&storage, *issue),
                SessionRaceAction::WriteRepoFile { path, content } => {
                    seed_repo_file(&storage, &path, &content);
                }
                SessionRaceAction::Run(operation) => operation(&storage),
            }
        }
        Ok(())
    }
}

/// Race `action` against the `trigger`th session open crossing `point`
/// ([`MutationPoint::SessionOpen`] or [`MutationPoint::ReadSessionOpen`]),
/// before that session captures anything.
pub(crate) fn with_open_race(
    storage: InMemoryStorage,
    point: MutationPoint,
    trigger: usize,
    action: SessionRaceAction,
) -> InMemoryStorage {
    with_session_race(storage, point, trigger, action)
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
        inner: storage.mutation_interruptions(),
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

/// Repository declarations a memory executor is built over; an absent one
/// keeps what the store holds, else the product default.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryDeclarations<'a> {
    pub config: Option<&'a str>,
    pub rules: Option<&'a str>,
    pub templates: Option<&'a str>,
    pub invariants: Option<&'a str>,
    pub gates: Option<&'a str>,
    /// Install the supplied declarations verbatim after initialization, which
    /// then neither validates nor rejects them (such as malformed rules).
    pub after_initialization: bool,
    /// Declarations removed from the store and its host mirror after
    /// initialization and installation, for a scenario that needs them absent.
    pub absent: &'a [VirtualPath],
}

impl MemoryDeclarations<'_> {
    fn install(&self, storage: &InMemoryStorage) {
        [
            (VirtualPath::CONFIG, self.config),
            (VirtualPath::RULES, self.rules),
            (VirtualPath::TEMPLATES, self.templates),
            (VirtualPath::INVARIANTS, self.invariants),
            (VirtualPath::GATES, self.gates),
        ]
        .into_iter()
        .filter_map(|(path, body)| body.map(|body| (path, body)))
        .for_each(|(path, body)| seed_repo_file(storage, &path.repository_relative(), body));
    }

    fn remove_absent(&self, storage: &InMemoryStorage) {
        let mut state = storage.repository_state();
        self.absent.iter().for_each(|path| {
            state.entries.remove(path);
        });
    }
}

/// Executor over `storage` whose repository is set up by product
/// initialization.
pub fn memory_executor(storage: InMemoryStorage) -> CommandExecutor<InMemoryStorage> {
    memory_executor_declaring(storage, MemoryDeclarations::default())
}

/// [`memory_executor`] for a scenario that declares repository declarations.
///
/// Each supplied declaration replaces what `storage` holds at its repository
/// path, before initialization unless
/// [`MemoryDeclarations::after_initialization`] is set; initialization fills
/// only absent paths without changing seeded state. Initializes a fault-free
/// declaration copy, excluding [`VirtualPath::RECORDS`], and mirrors the
/// declarations to the host paths their loaders read.
pub fn memory_executor_declaring(
    storage: InMemoryStorage,
    declarations: MemoryDeclarations<'_>,
) -> CommandExecutor<InMemoryStorage> {
    memory_executor_wrapping(storage, declarations, std::convert::identity)
}

/// [`memory_executor_declaring`] over the store `wrap` builds from the
/// initialized `storage`, such as a failure-injection wrapper; initialization
/// runs on `storage` itself, so the wrapper observes none of it.
pub fn memory_executor_wrapping<W: IssueStore>(
    storage: InMemoryStorage,
    declarations: MemoryDeclarations<'_>,
    wrap: impl FnOnce(InMemoryStorage) -> W,
) -> CommandExecutor<W> {
    if !declarations.after_initialization {
        declarations.install(&storage);
    }
    let layout = storage.repository_layout();
    let rules_source = storage.root().join(VirtualPath::RULES.relative().as_path());
    let rules_target = layout
        .resolve(&VirtualPath::RULES)
        .expect("test layout resolves its rules path");
    match std::fs::read(&rules_source) {
        Ok(bytes) => {
            if let Some(parent) = rules_target.parent() {
                std::fs::create_dir_all(parent).expect("create test rules directory");
            }
            std::fs::write(&rules_target, bytes).expect("mirror test rules through the layout");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if rules_target.exists() {
                std::fs::remove_file(&rules_target).expect("remove absent test rules mirror");
            }
        }
        Err(error) => panic!("read test rules fixture: {error}"),
    }
    let pristine = InMemoryStorage::rooted_at(storage.root());
    {
        let held = storage.repository_state();
        let mut copy = pristine.repository_state();
        copy.data_root_exists = held.data_root_exists;
        copy.entries = held
            .entries
            .iter()
            .filter(|(path, _)| {
                !VirtualPath::RECORDS.iter().any(|record| {
                    record.root_class() == path.root_class()
                        && record.relative().contains(path.relative().as_str())
                })
            })
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect();
    }
    CommandExecutor::new(pristine.clone())
        .with_layout(layout.clone())
        .profiles()
        .initialize_fresh_repository(storage.root(), None)
        .expect("product initialization seeds the memory repository");
    let initialized = pristine.repository_state().clone();
    {
        let mut state = storage.repository_state();
        state.data_root_exists |= initialized.data_root_exists;
        for (path, entry) in initialized.entries {
            state.entries.entry(path).or_insert(entry);
        }
    }
    if declarations.after_initialization {
        declarations.install(&storage);
    }
    declarations.remove_absent(&storage);
    mirror_memory_declarations(&storage, &layout);
    CommandExecutor::new(wrap(storage)).with_layout(layout)
}

fn mirror_memory_declarations(
    storage: &InMemoryStorage,
    layout: &crate::repository_state::RepositoryLayout,
) {
    let state = storage.repository_state();
    let schema_prefix = format!("{}/", VirtualPath::SCHEMAS.relative().as_str());
    let paths = state
        .entries
        .keys()
        .filter(|path| {
            path.root_class() == crate::repository_state::RepositoryRootClass::Data
                && ([
                    VirtualPath::CONFIG,
                    VirtualPath::TEMPLATES,
                    VirtualPath::INVARIANTS,
                    VirtualPath::RULES,
                ]
                .contains(*path)
                    || path.relative().as_str().starts_with(&schema_prefix))
        })
        .cloned()
        .collect::<Vec<_>>();
    drop(state);

    for virtual_path in [
        VirtualPath::CONFIG,
        VirtualPath::TEMPLATES,
        VirtualPath::INVARIANTS,
        VirtualPath::RULES,
    ]
    .into_iter()
    .chain(
        paths
            .into_iter()
            .filter(|path| path.relative().as_str().starts_with(&schema_prefix)),
    ) {
        let host_path = layout
            .resolve(&virtual_path)
            .expect("the memory layout resolves declaration paths");
        match storage
            .read_repo_file(&virtual_path.repository_relative())
            .expect("memory declaration paths are canonical")
        {
            Some(contents) => {
                if let Some(parent) = host_path.parent() {
                    std::fs::create_dir_all(parent).expect("create declaration fixture directory");
                }
                std::fs::write(host_path, contents).expect("mirror memory declaration");
            }
            None if host_path.exists()
                && !virtual_path.relative().as_str().starts_with(&schema_prefix) =>
            {
                std::fs::remove_file(host_path).expect("remove absent declaration fixture");
            }
            None => {}
        }
    }
}

/// Create an executor with enforcement mode configured.
///
/// # Arguments
///
/// * `mode` - An [`EnforcementMode`](crate::config::EnforcementMode) token
pub fn setup_with_enforcement(mode: &str) -> CommandExecutor<InMemoryStorage> {
    memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!("[worktree]\nenforce_leases = \"{mode}\"\n")),
            ..MemoryDeclarations::default()
        },
    )
}

/// Create an executor with enforcement disabled (for backward compatibility tests).
pub fn setup() -> CommandExecutor<InMemoryStorage> {
    setup_with_enforcement("off")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_executor_declaring_matches_product_initialization_plus_declared_config() {
        let declared = "[worktree]\nenforce_leases = \"warn\"\n";
        let storage = InMemoryStorage::new();
        memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                config: Some(declared),
                ..MemoryDeclarations::default()
            },
        );

        let reference = InMemoryStorage::rooted_at(storage.root());
        reference.add_data_file("config.toml", declared);
        CommandExecutor::new(reference.clone())
            .with_layout(reference.repository_layout())
            .profiles()
            .initialize_fresh_repository(reference.root(), None)
            .unwrap();

        let image = |store: &InMemoryStorage| store.repository_state().entries.clone();
        assert_eq!(image(&storage), image(&reference));
    }

    const DECLARED_CONFIG: &str = "[worktree]\nenforce_leases = \"warn\"\n";
    const DECLARED_RULES: &str = "[[rules]]\nname = \"declared-rule\"\n\
        assert = { require-section = { heading = \"Declared\" } }\n";
    const DECLARED_TEMPLATES: &str = "[[template]]\nname = \"declared-template\"\n\
        description = \"Declared.\"\napplies_to = [\"container\"]\n\
        [[template.anchors]]\nname = \"container\"\n\
        [[template.nodes]]\nrole = \"child\"\ntype = \"child\"\n\
        description = \"Child of {container.title}.\"\n";

    #[test]
    fn test_memory_executor_declaring_loads_supplied_declarations() {
        let executor = memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(DECLARED_CONFIG),
                rules: Some(DECLARED_RULES),
                templates: Some(DECLARED_TEMPLATES),
                ..MemoryDeclarations::default()
            },
        );

        let loaded = executor.config_manager.load().unwrap();
        let declared_config: crate::config::JitConfig = toml::from_str(DECLARED_CONFIG).unwrap();
        assert_eq!(
            loaded.enforcement_mode(),
            declared_config.enforcement_mode()
        );
        assert_eq!(
            executor.rules().unwrap(),
            &crate::declarations::rules::RuleSet::parse(DECLARED_RULES, Some(&loaded), []).unwrap()
        );
        assert_eq!(
            loaded.templates,
            crate::templates::TemplateRegistry::from_toml_str(DECLARED_TEMPLATES, &[] as &[&str])
                .unwrap()
        );
    }

    #[test]
    fn test_memory_executor_declaring_initializes_each_unsupplied_declaration_as_product_default() {
        let plain = InMemoryStorage::new();
        memory_executor(plain.clone());
        let supplied = [
            (VirtualPath::CONFIG, DECLARED_CONFIG),
            (VirtualPath::RULES, DECLARED_RULES),
            (VirtualPath::TEMPLATES, DECLARED_TEMPLATES),
        ];
        for (unsupplied, _) in &supplied {
            let declared = |path: VirtualPath| {
                supplied
                    .iter()
                    .find(|(candidate, _)| candidate == &path && candidate != unsupplied)
                    .map(|(_, body)| *body)
            };
            let storage = InMemoryStorage::rooted_at(plain.root());
            memory_executor_declaring(
                storage.clone(),
                MemoryDeclarations {
                    config: declared(VirtualPath::CONFIG),
                    rules: declared(VirtualPath::RULES),
                    templates: declared(VirtualPath::TEMPLATES),
                    ..MemoryDeclarations::default()
                },
            );

            let path = unsupplied.repository_relative();
            assert_eq!(
                storage.read_repo_file(&path).unwrap(),
                plain.read_repo_file(&path).unwrap(),
                "{path}"
            );
        }
    }

    #[test]
    fn test_memory_executor_declaring_installs_rejected_declaration_verbatim_after_initialization()
    {
        let malformed = "[[rules]\nname = \"unterminated\"\n";
        let storage = InMemoryStorage::new();
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                rules: Some(malformed),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );

        assert_eq!(
            storage
                .read_repo_file(&VirtualPath::RULES.repository_relative())
                .unwrap()
                .as_deref(),
            Some(malformed)
        );
        assert!(executor.rules().is_err());
    }

    #[test]
    fn test_memory_executor_declaring_removes_absent_declaration_from_store_and_host_mirror() {
        let path = VirtualPath::RULES;
        let storage = InMemoryStorage::new();
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                absent: std::slice::from_ref(&path),
                ..MemoryDeclarations::default()
            },
        );

        let reference = InMemoryStorage::new();
        memory_executor(reference.clone());
        let relative = path.repository_relative();
        assert!(reference.read_repo_file(&relative).unwrap().is_some());
        assert_eq!(storage.read_repo_file(&relative).unwrap(), None);
        let host = storage.repository_layout().resolve(&path).unwrap();
        assert!(!host.exists(), "{}", host.display());
        assert!(executor
            .storage()
            .read_repo_file(&relative)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_memory_executor_wrapping_wraps_the_declared_initialized_store() {
        let declarations = MemoryDeclarations {
            config: Some(DECLARED_CONFIG),
            ..MemoryDeclarations::default()
        };
        let reference = InMemoryStorage::new();
        memory_executor_declaring(reference.clone(), declarations);

        let image = |store: &InMemoryStorage| store.repository_state().entries.clone();
        let counter = SessionOpenCounter::new();
        let mut wrapped_image = None;
        let executor = memory_executor_wrapping(
            InMemoryStorage::rooted_at(reference.root()),
            declarations,
            |initialized| {
                wrapped_image = Some(image(&initialized));
                initialized.with_mutation_interruption_view(counter.clone())
            },
        );

        assert_eq!(wrapped_image, Some(image(&reference)));
        assert_eq!(counter.count(), 0, "initialization bypasses the wrapper");
        executor
            .issues()
            .create_issue(
                "through the wrapper".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        assert!(
            counter.count() > 0,
            "the executor mutates through the wrapper"
        );
    }

    #[test]
    fn test_memory_executor_leaves_seeded_state_and_faults_out_of_initialization() {
        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::with_mutation_interruptions(counter.clone());
        let issue = crate::test_utils::fixture_issue("seeded issue".into(), String::new());
        seed_issue(&storage, issue);
        storage.add_data_file("gates.toml", "# scenario gates\n");
        let seeded = storage.repository_state().entries.clone();

        memory_executor(storage.clone());

        assert_eq!(
            counter.count(),
            0,
            "initialization must not open the scenario store"
        );
        let initialized = storage.repository_state();
        for (path, entry) in seeded {
            assert_eq!(initialized.entries.get(&path), Some(&entry), "{path:?}");
        }
    }

    #[test]
    fn test_memory_executor_fills_every_initialization_path_a_partially_seeded_store_lacks() {
        let reference = InMemoryStorage::new();
        memory_executor(reference.clone());
        let held = [".jit/index.json", ".jit/rules.toml"];
        let storage = InMemoryStorage::rooted_at(reference.root());
        for path in held {
            let initialized = reference.read_repo_file(path).unwrap().unwrap();
            seed_repo_file(&storage, path, &format!("{initialized}\n"));
        }

        let image = |store: &InMemoryStorage| store.repository_state().entries.clone();
        let before = image(&storage);
        memory_executor(storage.clone());

        let (seeded, initialized) = (image(&storage), image(&reference));
        assert_eq!(
            seeded.keys().collect::<Vec<_>>(),
            initialized.keys().collect::<Vec<_>>()
        );
        for (path, entry) in &initialized {
            let is_file = matches!(entry, crate::repository_state::RepositoryEntry::File { .. });
            if is_file {
                let expected = before.get(path).unwrap_or(entry);
                assert_eq!(seeded.get(path), Some(expected), "{path:?}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod gate_reuse;
