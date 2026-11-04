//! Session fixtures shared by the store's own suites and the read driver's:
//! one seeded repository per backend, and the leftover record it carries
//! (`@/inv/shared-test-contracts`).

use super::*;
use crate::storage::capability_fs::DataRoot;
use crate::storage::memory::test_fixtures as memory_fixtures;
use crate::storage::redo_apply::test_fixtures::{
    data_root as redo_root, record_relative_path, record_scratch_relative_path,
    repository as redo_repository, seed_repository, snapshot, SnapshotEntry, CAPTURED_LOG,
};
use tempfile::TempDir;

pub(crate) fn data_path(relative: &str) -> VirtualPath {
    VirtualPath::data(relative).unwrap()
}

pub(crate) fn put_file(path: VirtualPath, bytes: &[u8]) -> PutFile {
    PutFile {
        path,
        bytes: bytes.to_vec(),
        mode: FileMode::Regular,
    }
}

/// An append at `offset`, the log's committed length when the mutation read it.
pub(crate) fn event_append(offset: u64) -> Option<EventAppend> {
    Some(EventAppend {
        offset,
        bytes: b"{\"tag\":\"issue_created\"}\n".to_vec(),
    })
}

/// One contract, run on both backends, because applying a leftover record is
/// the session's behaviour rather than either backend's
/// (`@/inv/shared-test-contracts`).
pub(crate) trait SessionRecordBackend: Sized {
    /// What a record publishes, compared between two opens.
    type Published: std::fmt::Debug + PartialEq;
    /// The store sessions open over, shareable across threads.
    type Store: RepositoryStateStore + Clone + Send + 'static;

    /// Its event log holds [`CAPTURED_LOG`], the offset [`leftover_record`]
    /// appends at.
    fn seeded() -> Self;
    fn store(&self) -> &Self::Store;
    fn layout(&self) -> RepositoryLayout;
    fn commit(&self, record: &RedoRecord);
    fn open(&self) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        self.store().open_mutation_session(self.layout())
    }
    fn open_read(&self) -> Result<Box<dyn OptimisticReadSession>, RepositoryStateStoreError> {
        self.store().open_read_session(self.layout())
    }
    /// Open a session whose publication is interrupted at `point`, over the
    /// same repository every other method reads.
    fn open_interrupted(
        &self,
        point: MutationPoint,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError>;
    fn published(&self) -> Self::Published;
    /// Whether a record is still waiting for an open to carry it forward.
    fn holds_record(&self) -> bool;
    fn bytes_at(&self, path: &VirtualPath) -> Option<Vec<u8>>;
}

pub(crate) struct FileSessionBackend {
    pub(crate) worktree: TempDir,
    pub(crate) storage: JsonFileStorage,
    pub(crate) layout: RepositoryLayout,
}

impl FileSessionBackend {
    pub(crate) fn record_path(&self) -> PathBuf {
        self.worktree.path().join(record_relative_path())
    }

    pub(crate) fn scratch_path(&self) -> PathBuf {
        redo_root(&self.worktree).join(RECORD_SCRATCH_DIR)
    }
}

impl SessionRecordBackend for FileSessionBackend {
    type Published = BTreeMap<PathBuf, SnapshotEntry>;
    type Store = JsonFileStorage;

    fn seeded() -> Self {
        let (worktree, _roots) = redo_repository();
        seed_repository(&worktree, &[], false);
        let data = redo_root(&worktree);
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let storage = JsonFileStorage::new(&data);
        Self {
            worktree,
            storage,
            layout,
        }
    }

    fn store(&self) -> &Self::Store {
        &self.storage
    }

    fn layout(&self) -> RepositoryLayout {
        self.layout.clone()
    }

    fn commit(&self, record: &RedoRecord) {
        commit_redo_record(&DataRoot::open(&redo_root(&self.worktree)).unwrap(), record).unwrap();
    }

    fn open_interrupted(
        &self,
        point: MutationPoint,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        JsonFileStorage::with_mutation_interruptions(
            redo_root(&self.worktree),
            InterruptAtMoment::new(point),
        )
        .open_mutation_session(self.layout.clone())
    }

    fn published(&self) -> Self::Published {
        let mut entries = snapshot(self.worktree.path());
        entries.remove(&record_relative_path());
        entries.remove(&record_scratch_relative_path());
        entries
    }

    fn holds_record(&self) -> bool {
        self.record_path().exists()
    }

    fn bytes_at(&self, path: &VirtualPath) -> Option<Vec<u8>> {
        let root = match path.root_class() {
            RepositoryRootClass::Data => redo_root(&self.worktree),
            RepositoryRootClass::Worktree => self.worktree.path().to_path_buf(),
        };
        std::fs::read(root.join(path.relative().as_path())).ok()
    }
}

pub(crate) struct MemorySessionBackend(pub(crate) InMemoryStorage);

impl SessionRecordBackend for MemorySessionBackend {
    type Published = BTreeMap<VirtualPath, RepositoryEntry>;
    type Store = InMemoryStorage;

    fn seeded() -> Self {
        Self(memory_fixtures::seeded(&[], false))
    }

    fn store(&self) -> &Self::Store {
        &self.0
    }

    fn layout(&self) -> RepositoryLayout {
        self.0.repository_layout()
    }

    fn commit(&self, record: &RedoRecord) {
        self.0.commit_record(record).unwrap();
    }

    fn open_interrupted(
        &self,
        point: MutationPoint,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        self.0
            .with_mutation_interruption_view(InterruptAtMoment::new(point))
            .open_mutation_session(self.0.repository_layout())
    }

    fn published(&self) -> Self::Published {
        memory_fixtures::semantic_state(&self.0).0
    }

    fn holds_record(&self) -> bool {
        memory_fixtures::semantic_state(&self.0).2.is_some()
    }

    fn bytes_at(&self, path: &VirtualPath) -> Option<Vec<u8>> {
        match memory_fixtures::semantic_state(&self.0).0.get(path) {
            Some(RepositoryEntry::File { content, .. }) => Some(content.bytes().to_vec()),
            _ => None,
        }
    }
}

/// Interrupts a session at one named moment, so exactly the steps before it
/// ran.
pub(crate) struct InterruptAtMoment(MutationPoint);

impl InterruptAtMoment {
    pub(crate) fn new(point: MutationPoint) -> Arc<Self> {
        Arc::new(Self(point))
    }
}

impl crate::storage::MutationInterruptionInjector for InterruptAtMoment {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        match *point == self.0 {
            true => Err(std::io::Error::other(format!("interrupted at {point:?}"))),
            false => Ok(()),
        }
    }
}

/// One data file published and one event appended at the committed length the
/// interrupted mutation captured.
pub(crate) fn leftover_record() -> RedoRecord {
    RedoRecord::new(
        Vec::new(),
        vec![put_file(leftover_target(), LEFTOVER_BYTES)],
        Vec::new(),
        event_append(CAPTURED_LOG.len() as u64),
    )
    .unwrap()
}

pub(crate) fn leftover_target() -> VirtualPath {
    data_path("records/left.json")
}

pub(crate) const LEFTOVER_BYTES: &[u8] = b"{\"value\":\"left\"}";

pub(crate) fn appended_log() -> Vec<u8> {
    [CAPTURED_LOG, &event_append(0).unwrap().bytes].concat()
}
