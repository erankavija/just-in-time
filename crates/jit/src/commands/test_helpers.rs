//! Test helper functions for command tests.
//!
//! Provides reusable setup functions to eliminate duplication across test modules.

use crate::commands::CommandExecutor;
use crate::declarations::GateRegistry;
use crate::domain::identity::IssueId;
use crate::domain::Issue;
use crate::repository_state::{RepositoryIndex, VirtualPath};
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
    Delete(IssueId),
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

/// Repository declarations a memory executor is built over; an absent one
/// keeps what the store holds, else the product default.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryDeclarations<'a> {
    pub config: Option<&'a str>,
    pub rules: Option<&'a str>,
    pub templates: Option<&'a str>,
}

/// Executor over `storage` whose repository is set up by product
/// initialization.
pub fn memory_executor(storage: InMemoryStorage) -> CommandExecutor<InMemoryStorage> {
    memory_executor_declaring(storage, MemoryDeclarations::default())
}

/// [`memory_executor`] for a scenario that declares configuration, rules or
/// templates.
///
/// Each supplied declaration replaces what `storage` holds at its repository
/// path before initialization, so initialization validates it and fills only
/// absent paths without changing seeded state. Initializes a fault-free
/// declaration copy, excluding [`VirtualPath::RECORDS`]. Seed declarations that
/// initialization rejects (such as malformed rules) after this call.
pub fn memory_executor_declaring(
    storage: InMemoryStorage,
    declarations: MemoryDeclarations<'_>,
) -> CommandExecutor<InMemoryStorage> {
    [
        (VirtualPath::CONFIG, declarations.config),
        (VirtualPath::RULES, declarations.rules),
        (VirtualPath::TEMPLATES, declarations.templates),
    ]
    .into_iter()
    .filter_map(|(path, body)| body.map(|body| (path, body)))
    .for_each(|(path, body)| seed_repo_file(&storage, &path.repository_relative(), body));
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
    mirror_memory_declarations(&storage, &layout);
    CommandExecutor::new(storage).with_layout(layout)
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
/// * `mode` - Enforcement mode: "strict", "warn", or "off"
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

/// Decode an issue identity at the fixture boundary.
pub(crate) fn fixture_id(value: &str) -> IssueId {
    value.parse().expect("fixture issue id is a canonical UUID")
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
