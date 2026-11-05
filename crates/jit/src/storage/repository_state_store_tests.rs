use super::test_fixtures::*;
use super::*;
use crate::domain::EventBody;
use crate::repository_state::{
    serialize_gate_run, CaptureBudget, MutationContext, RepositoryAction, RepositoryActionOwner,
    EVENT_LOG_TAIL_WINDOW,
};
#[cfg(windows)]
use crate::storage::capability_fs::capability_metadata_identity;
use crate::storage::capability_fs::{
    open_regular_file_nofollow, CapabilityFsError, CapabilityRoots,
};
use crate::storage::redo_apply::test_fixtures::snapshot;
use crate::storage::{EventAppend, RedoPoint, RECORD_FILE_NAME, RECORD_SCRATCH_DIR};
use crate::test_harness::git::GitStandIn;
use cap_std::ambient_authority;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn test_admit_read_sources_discards_errors_when_any_source_moved() {
    let admissions = vec![
        Err(RepositoryStateStoreError::Io(std::io::Error::other(
            "transient",
        ))),
        Ok(ReadAdmission::Moved),
    ];
    assert!(admit_read_sources(admissions).unwrap().is_none());
}

#[test]
fn test_admit_read_sources_returns_stable_admission_errors() {
    let admissions = vec![
        Err(RepositoryStateStoreError::Io(std::io::Error::other(
            "stable",
        ))),
        Ok(ReadAdmission::Granted),
    ];
    assert!(admit_read_sources(admissions).is_err());
}

fn assert_issue_listing_rejects_pending_redo<B: SessionRecordBackend>()
where
    B::Store: IssueStore,
{
    let backend = B::seeded();
    backend
        .store()
        .configure_repository_layout(&backend.layout())
        .unwrap();
    backend.commit(&leftover_record());
    let error = backend.store().list_issues().unwrap_err();
    assert!(format!("{error:#}").contains("jit recover"), "{error:#}");
    drop(backend.open().unwrap());
    backend.store().list_issues().unwrap();
}

#[test]
fn test_list_issues_refuses_contended_memory_state_without_waiting() {
    let store = InMemoryStorage::new();
    let held = store.repository_state();
    let reader = store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || tx.send(reader.list_issues()).unwrap());
    let result = rx.recv_timeout(std::time::Duration::from_secs(2));
    drop(held);
    thread.join().unwrap();
    let error = result.unwrap().unwrap_err();
    assert!(matches!(
        error.downcast_ref::<RepositoryStateStoreError>(),
        Some(RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

#[test]
fn test_list_issues_rejects_pending_redo_on_both_backends() {
    assert_issue_listing_rejects_pending_redo::<FileSessionBackend>();
    assert_issue_listing_rejects_pending_redo::<MemorySessionBackend>();
}

#[test]
fn test_advisory_error_classification_only_accepts_permission_denied_io() {
    let errors = [
        RepositoryStateStoreError::Io(std::io::Error::other("hard I/O failure")),
        RepositoryStateStoreError::Capture(CaptureError::ListingBudgetExceeded {
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

/// A listing is captured evidence the mutation reads and never writes, so its
/// movement leaves the mutation admissible — the cost `@/charter/D-17` names,
/// stated here for the one kind of evidence the record cannot carry at all.
#[test]
fn test_json_apply_admits_a_listing_that_changed_since_capture() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let notes = worktree.path().join("notes");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&notes).unwrap();
    std::fs::write(notes.join("first.md"), b"first").unwrap();

    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let notes_path = VirtualPath::worktree("notes").unwrap();
    let published_path = VirtualPath::worktree("published.txt").unwrap();
    let mut spec = publishing_spec([published_path.clone()]);
    spec.discover_listing(notes_path.clone()).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            published_path.clone(),
            RepositoryActionOwner::ArchiveExecution,
            ExpectedPreimage::Absent,
            b"published".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    let mut session = storage.open_mutation_session(layout).unwrap();
    let captured = session.capture(spec).unwrap();
    std::fs::write(notes.join("second.md"), b"second").unwrap();

    session.apply(&test_plan(&captured, &delta)).unwrap();

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
        max_listings: 8,
        max_bytes: 1024 * 1024,
        max_depth: 8,
    }
}

fn initial_spec() -> CaptureSpec {
    CaptureSpec::new(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("index.json").unwrap(),
            VirtualPath::worktree("note.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap()
}

fn initialization_delta(layout: &RepositoryLayout) -> RepositoryDelta {
    RepositoryDelta::new(
        layout,
        vec![
            RepositoryAction::create_directory(
                VirtualPath::data("").unwrap(),
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("index.json").unwrap(),
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                VirtualPath::worktree("note.txt").unwrap(),
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
                b"note".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap()
}

/// A capture of `paths` that also reads what a publication over a present
/// data root needs: the root itself and the event log's tail.
fn publishing_spec(paths: impl IntoIterator<Item = VirtualPath>) -> CaptureSpec {
    let mut spec = CaptureSpec::new(
        paths.into_iter().chain([VirtualPath::data("").unwrap()]),
        budget(),
    )
    .unwrap();
    spec.discover_tail(VirtualPath::EVENTS).unwrap();
    spec
}

/// Close a hand-built test delta over its captured image.
fn test_plan(image: &RepositoryImage, delta: &RepositoryDelta) -> MaterializationPlan {
    MaterializationPlan::new(image, delta.clone(), &MutationContext::preview()).unwrap()
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
    let image = session.capture(publishing_spec([path.clone()])).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::set_mode(
            path,
            RepositoryActionOwner::RepositoryStateMutation,
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
fn test_same_storage_reuses_its_own_absent_root_publication_evidence() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let stale_layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    storage.configure_repository_layout(&stale_layout).unwrap();
    let path = VirtualPath::data("index.json").unwrap();
    let mut first = storage.open_mutation_session(stale_layout.clone()).unwrap();
    let image = first
        .capture(CaptureSpec::new([path.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        &stale_layout,
        vec![RepositoryAction::write_file(
            path.clone(),
            RepositoryActionOwner::RepositoryInit,
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
        .capture(CaptureSpec::new([path.clone()], budget()).unwrap())
        .unwrap();
    assert!(matches!(
        refreshed.entry(&path),
        Ok(RepositoryEntry::File { content, .. }) if content.bytes() == b"{}"
    ));
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

#[cfg(unix)]
#[test]
fn test_open_regular_file_nofollow_refuses_a_symbolic_link() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("target.txt");
    std::fs::write(&target, b"outside content").unwrap();
    symlink(&target, temp.path().join("link.txt")).unwrap();
    let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();

    let error = open_regular_file_nofollow(&dir, "link.txt").unwrap_err();

    assert!(matches!(error, CapabilityFsError::UnsafeTarget(_)));
}

#[test]
fn test_open_regular_file_nofollow_refuses_a_directory() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join("subdir")).unwrap();
    let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();

    let error = open_regular_file_nofollow(&dir, "subdir").unwrap_err();

    assert!(matches!(error, CapabilityFsError::UnsafeTarget(_)));
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

// --- Cross-backend session matrix ---------------------------------------
//
// These cases exercise identical mutation-session semantics on the JSON and
// memory backends: equivalent captured semantics, complete per-image result
// hashes, every action kind, nested and disjoint existing/absent roots, and
// aliases. The `IssueStore` contract is `issue_store_conformance`.
// Object identity differs by construction (device/inode versus a synthesized
// memory id), so parity is asserted over the identity-independent semantic
// projection.

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemanticEntry {
    Absent,
    Directory(FileMode),
    File(Vec<u8>, FileMode),
    FileTail(u64, Vec<u8>, FileMode),
    FileKind(FileMode),
    Symlink(Vec<u8>),
    Unsupported,
}

fn semantic_entry(entry: &RepositoryEntry) -> SemanticEntry {
    match entry {
        RepositoryEntry::Absent => SemanticEntry::Absent,
        RepositoryEntry::Directory { mode, .. } => SemanticEntry::Directory(*mode),
        RepositoryEntry::File { content, mode } => {
            SemanticEntry::File(content.bytes().to_vec(), *mode)
        }
        RepositoryEntry::FileTail { tail, mode } => {
            SemanticEntry::FileTail(tail.length(), tail.tail().to_vec(), *mode)
        }
        RepositoryEntry::FileKind { mode } => SemanticEntry::FileKind(*mode),
        RepositoryEntry::Symlink { target, .. } => SemanticEntry::Symlink(target.bytes().to_vec()),
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
    publishing_spec([
        VirtualPath::data("keep.txt").unwrap(),
        VirtualPath::data("replace.txt").unwrap(),
        VirtualPath::data("remove.txt").unwrap(),
        VirtualPath::data("chmod.txt").unwrap(),
        VirtualPath::data("generated").unwrap(),
        VirtualPath::data("generated/new.txt").unwrap(),
        VirtualPath::worktree("out.txt").unwrap(),
    ])
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
                content: CapturedBytes::new(format!("mem:{path:?}"), bytes.to_vec()).unwrap(),
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
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("generated/new.txt").unwrap(),
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
                b"created".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::write_file(
                VirtualPath::data("replace.txt").unwrap(),
                RepositoryActionOwner::RepositoryStateMutation,
                preimage("replace.txt"),
                b"replaced".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::set_mode(
                VirtualPath::data("chmod.txt").unwrap(),
                RepositoryActionOwner::RepositoryStateMutation,
                preimage("chmod.txt"),
                FileMode::Executable,
            ),
            RepositoryAction::delete_file(
                VirtualPath::data("remove.txt").unwrap(),
                RepositoryActionOwner::RepositoryStateMutation,
                preimage("remove.txt"),
            ),
            RepositoryAction::write_file(
                VirtualPath::worktree("out.txt").unwrap(),
                RepositoryActionOwner::RepositoryStateMutation,
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

/// A record's home is inside the data root, so a mutation writing worktree
/// targets while creating no root has no commit point. Both backends refuse it
/// with the same typed error rather than publishing it unrecoverably, and
/// neither writes anything.
#[test]
fn test_conformance_worktree_only_absent_root_is_refused_as_uncommittable() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit"); // absent
    let note = worktree.path().join("note.txt");
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let make_spec = || {
        let mut spec = CaptureSpec::new([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("note.txt").unwrap()])
            .unwrap();
        spec
    };
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::worktree("note.txt").unwrap(),
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"note".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    let memory = InMemoryStorage::new();
    let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_image = memory_session.capture(make_spec()).unwrap();
    let memory_outcome = memory_session.apply(&test_plan(&memory_image, &delta));
    drop(memory_session);

    let json = JsonFileStorage::new(&data);
    let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
    let json_image = json_session.capture(make_spec()).unwrap();
    let json_outcome = json_session.apply(&test_plan(&json_image, &delta));
    drop(json_session);

    for (backend, outcome) in [("memory", memory_outcome), ("json", json_outcome)] {
        assert!(
            matches!(
                outcome,
                Err(RepositoryStateStoreError::UncommittableWorktreeMutation { ref path })
                    if path == &data
            ),
            "{backend} must refuse the uncommittable mutation, got {outcome:?}"
        );
    }
    assert!(!note.exists(), "the refusal must write no worktree target");
    assert!(!data.exists(), "the refusal must publish no data root");

    // The refusal changed nothing either backend can see: both still capture an
    // absent data root and an absent worktree target.
    let mut memory_after = memory.open_mutation_session(layout.clone()).unwrap();
    let memory_view = semantic_view(&memory_after.capture(make_spec()).unwrap());
    let json_after = JsonFileStorage::new(&data);
    let mut json_after_session = json_after.open_mutation_session(layout).unwrap();
    let json_view = semantic_view(&json_after_session.capture(make_spec()).unwrap());
    assert_eq!(memory_view, json_view);
    assert_eq!(
        memory_view[&VirtualPath::data("").unwrap()],
        SemanticEntry::Absent
    );
    assert_eq!(
        memory_view[&VirtualPath::worktree("note.txt").unwrap()],
        SemanticEntry::Absent
    );
}

#[test]
fn test_conformance_empty_delta_is_a_noop_on_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = || CaptureSpec::new([VirtualPath::data("").unwrap()], budget()).unwrap();
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

    // An empty delta is a no-op with zero actions; no residue is created.
    assert_eq!(memory_outcome.actions_applied, json_outcome.actions_applied);
    assert_eq!(json_outcome.actions_applied, 0);
    assert!(!data
        .join(RECORD_SCRATCH_DIR)
        .join(RECORD_FILE_NAME)
        .exists());
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

/// A put creates the directories its path needs (OD-7), so a nested write
/// publishes without an explicit parent action — identically on both backends,
/// because each one's record application creates the ancestors.
#[test]
fn test_conformance_a_nested_write_creates_its_parent_on_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let nested = VirtualPath::data("nested/file.txt").unwrap();
    let spec = || publishing_spec([VirtualPath::data("nested").unwrap(), nested.clone()]);
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            nested.clone(),
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"x".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    let json = JsonFileStorage::new(&data);
    let json_outcome = apply_once(&json, &layout, spec(), &delta).unwrap();
    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, &[]);
    let memory_outcome = apply_once(&memory, &layout, spec(), &delta).unwrap();

    assert_eq!(
        std::fs::read(data.join("nested/file.txt")).unwrap(),
        b"x",
        "the filesystem backend did not create the directory its put needed"
    );
    assert_eq!(
        json_outcome.actions_applied, memory_outcome.actions_applied,
        "the backends disagree on what the nested write published"
    );
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
            CaptureSpec::new([VirtualPath::data("file.txt").unwrap()], budget()).unwrap();
        spec.discover_listing(VirtualPath::data("file.txt").unwrap())
            .unwrap();
        spec
    };

    // A complete-listing request over a regular file is an error on both
    // backends, never a fabricated directory listing.
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
        let mut spec = CaptureSpec::new([root.clone()], budget()).unwrap();
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
        let listing = &image.directory_listings()[&VirtualPath::worktree("").unwrap()];
        assert!(listing.names().contains(".jit"));
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
        .capture(CaptureSpec::new([VirtualPath::data("").unwrap()], budget()).unwrap())
        .unwrap();

    assert!(matches!(
        image.entry(&VirtualPath::data("").unwrap()).unwrap(),
        RepositoryEntry::Directory { .. }
    ));
}

/// Replaces a file's bytes behind a session's back.
type Writer = Box<dyn Fn(&VirtualPath, &[u8])>;
type WrittenSession = (Box<dyn RepositoryMutationSession>, Writer);

/// A store over `files` on each backend, sharing one layout.
fn stores_over(
    files: &[(VirtualPath, &[u8])],
) -> (TempDir, RepositoryLayout, JsonFileStorage, InMemoryStorage) {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    files
        .iter()
        .for_each(|(path, bytes)| std::fs::write(layout.resolve(path).unwrap(), bytes).unwrap());
    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, files);
    (worktree, layout, JsonFileStorage::new(&data), memory)
}

/// A session over `files` on each backend, with its [`Writer`].
fn sessions_over(files: &[(VirtualPath, &[u8])]) -> (TempDir, [WrittenSession; 2]) {
    let (worktree, layout, json, memory) = stores_over(files);
    let resolve = layout.clone();
    let sessions = [
        json.open_mutation_session(layout.clone()),
        memory.open_mutation_session(layout),
    ]
    .map(Result::unwrap);
    let json: Writer =
        Box::new(move |path, bytes| std::fs::write(resolve.resolve(path).unwrap(), bytes).unwrap());
    let memory: Writer =
        Box::new(move |path, bytes| seed_memory_existing(&memory, &[(path.clone(), bytes)]));
    let [json_session, memory_session] = sessions;
    (worktree, [(json_session, json), (memory_session, memory)])
}

/// Growing an image reads only what it lacks: a path it holds keeps the bytes
/// first read, and a path asked for in another read mode is read again.
#[test]
fn test_conformance_extend_reads_only_what_the_image_lacks_on_both_backends() {
    let (kept, added) = (VirtualPath::INDEX, VirtualPath::CONFIG);
    let upgraded = VirtualPath::data("upgrade.txt").unwrap();
    let log = vec![b'x'; EVENT_LOG_TAIL_WINDOW as usize + 1];
    let (_worktree, sessions) = sessions_over(&[
        (kept.clone(), b"first"),
        (added.clone(), b"added"),
        (upgraded.clone(), &log),
    ]);
    for (mut session, write) in sessions {
        let mut spec = CaptureSpec::new([kept.clone()], budget()).unwrap();
        spec.discover_tail(upgraded.clone()).unwrap();
        let image = session.capture(spec).unwrap();
        write(&kept, b"second");
        let mut spec = image.capture_spec().clone();
        spec.discover_paths([added.clone(), upgraded.clone()])
            .unwrap();

        let grown = session.extend(image, spec).unwrap();

        assert_eq!(grown.file_bytes(&kept).unwrap(), Some(&b"first"[..]));
        assert_eq!(grown.file_bytes(&added).unwrap(), Some(&b"added"[..]));
        assert_eq!(grown.file_bytes(&upgraded).unwrap(), Some(&log[..]));
    }
}

/// A tail read holds the file's length and its last window of bytes, reaching
/// back to the newline before a final line that starts earlier (or to the
/// file's start), and a producer asking for the whole file fails typed.
#[test]
fn test_conformance_tail_capture_holds_length_and_final_line_on_both_backends() {
    let window = EVENT_LOG_TAIL_WINDOW as usize;
    let lines = (0..window * 2 + 7)
        .map(|byte| byte as u8)
        .collect::<Vec<_>>();
    let long_line = [vec![b'h'; window * 2], vec![b'\n'], vec![b'x'; window * 3]].concat();
    let one_line = vec![b'x'; window * 2 + 1];
    for (log, kept) in [
        (lines, window),
        (long_line, window * 3 + 1),
        (one_line, window * 2 + 1),
    ] {
        let (_worktree, sessions) = sessions_over(&[(VirtualPath::EVENTS, &log)]);
        for (mut session, _) in sessions {
            let mut spec = CaptureSpec::new([], budget()).unwrap();
            spec.discover_tail(VirtualPath::EVENTS).unwrap();

            let image = session.capture(spec).unwrap();

            let RepositoryEntry::FileTail { tail, .. } = image.entry(&VirtualPath::EVENTS).unwrap()
            else {
                panic!("a tail read captures a tail");
            };
            assert_eq!(tail.length(), log.len() as u64);
            assert_eq!(tail.tail(), &log[log.len() - kept..]);
            assert!(matches!(
                image.file_bytes(&VirtualPath::EVENTS),
                Err(CaptureError::TailOnlyCapture(_))
            ));
        }
    }
}

/// The history read returns the whole log, longer than any tail, or `None`
/// over an absent one, through both session kinds.
#[test]
fn test_conformance_read_history_log_returns_the_whole_log_on_both_backends() {
    let log = vec![b'x'; EVENT_LOG_TAIL_WINDOW as usize * 3];
    for (files, expected) in [
        (
            &[(VirtualPath::EVENTS, log.as_slice())][..],
            Some(log.clone()),
        ),
        (&[][..], None),
    ] {
        let (_worktree, layout, json, memory) = stores_over(files);
        let sessions: [Box<dyn RepositoryReadSession>; 4] = [
            json.open_mutation_session(layout.clone()).unwrap(),
            json.open_read_session(layout.clone()).unwrap(),
            memory.open_mutation_session(layout.clone()).unwrap(),
            memory.open_read_session(layout.clone()).unwrap(),
        ];
        for mut session in sessions {
            assert_eq!(session.read_history_log().unwrap(), expected);
        }
    }
}

#[cfg(unix)]
#[test]
fn test_read_history_log_refuses_a_symbolic_link() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("linked.jsonl"), b"{}\n").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    std::os::unix::fs::symlink(
        "linked.jsonl",
        layout.resolve(&VirtualPath::EVENTS).unwrap(),
    )
    .unwrap();
    let mut session = JsonFileStorage::new(&data)
        .open_mutation_session(layout)
        .unwrap();

    let outcome = session.read_history_log();
    assert!(
        matches!(outcome, Err(RepositoryStateStoreError::UnsafeTarget(_))),
        "{outcome:?}"
    );
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
        let mut spec = CaptureSpec::new([], budget).unwrap();
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
            let path =
                VirtualPath::data(VirtualPath::ISSUES.relative().as_path().join(name)).unwrap();
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    content: CapturedBytes::new(format!("mem:{path:?}"), bytes.to_vec()).unwrap(),
                    mode: FileMode::Regular,
                },
            );
        }
    }
    let mut memory_session = memory.open_mutation_session(layout).unwrap();
    let memory_error = memory_session.capture(spec()).unwrap_err();
    [json_error, memory_error]
}

/// Children a declaration never asked for cost what their names cost, so a
/// directory that has gained more of them than the budget admits is refused by
/// every backend rather than silently expanding the capture.
#[test]
fn test_conformance_listing_rogue_children_exceed_the_byte_budget_on_both_backends() {
    let children: [(&str, &[u8]); 2] = [("one.json", b"one"), ("two.json", b"two")];
    let names = children
        .iter()
        .map(|(name, _)| name.len() as u64)
        .sum::<u64>();

    for error in capture_listing_errors(
        &children,
        CaptureBudget {
            max_listings: 1,
            max_bytes: names - 1,
            max_depth: 2,
        },
    ) {
        assert!(matches!(
            error,
            RepositoryStateStoreError::Capture(CaptureError::ByteBudgetExceeded {
                actual,
                maximum,
            }) if actual >= names && maximum == names - 1
        ));
    }
}

#[test]
fn test_conformance_listing_rogue_name_bytes_exceed_budget_on_both_backends() {
    for error in capture_listing_errors(
        &[("é", b"content")],
        CaptureBudget {
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
        MutationPoint::Redo(RedoPoint::BeforeApply),
        MutationPoint::Redo(RedoPoint::BeforeRecordRemoval),
        MutationPoint::Redo(RedoPoint::AfterApply),
    ] {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || CaptureSpec::new([VirtualPath::data("").unwrap()], budget()).unwrap();
        let delta = RepositoryDelta::new(&layout, vec![]).unwrap();

        // An empty delta writes no record on either backend, so no step of a
        // record's application is reached and an armed interruption never fires.
        let json = JsonFileStorage::with_mutation_interruptions(
            &data,
            InterruptAtMoment::new(point.clone()),
        );
        let json_outcome = apply_once(&json, &layout, spec(), &delta).unwrap();

        let memory =
            InMemoryStorage::with_mutation_interruptions(InterruptAtMoment::new(point.clone()));
        seed_memory_existing(&memory, &[]);
        let memory_outcome = apply_once(&memory, &layout, spec(), &delta).unwrap();

        assert_eq!(
            json_outcome.actions_applied, memory_outcome.actions_applied,
            "{point:?}"
        );
        assert_eq!(json_outcome.actions_applied, 0);
        assert!(
            !data
                .join(RECORD_SCRATCH_DIR)
                .join(RECORD_FILE_NAME)
                .exists(),
            "an empty delta committed a record, at {point:?}"
        );
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
            CaptureSpec::new(
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
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
            ));
        }
        actions.push(RepositoryAction::write_file(
            VirtualPath::data("index.json").unwrap(),
            RepositoryActionOwner::RepositoryStateMutation,
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

#[test]
fn test_session_disjoint_sessions_serialize_on_worktree_bootstrap() {
    use crate::storage::contention_probe::{admitted_when_reached, Contenders};
    use crate::storage::repo_lock::RepoWriteLock;
    use crate::storage::repository_state_store::is_lock_timeout;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Two data roots under DIFFERENT parents, both sharing one worktree. Their
    // data-root-parent bootstrap locks differ, so only the worktree-root
    // bootstrap lock serializes their shared bootstrap namespace.
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
fn test_json_apply_rejects_data_root_replaced_since_capture_as_retryable_conflict() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("keep.txt"), b"keep").unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let storage = JsonFileStorage::new(&data);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = publishing_spec([
        VirtualPath::data("keep.txt").unwrap(),
        VirtualPath::data("new.txt").unwrap(),
    ]);
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
            RepositoryActionOwner::RepositoryStateMutation,
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

/// A whole worktree swapped between capture and the commit check is what the
/// held capabilities cannot see by themselves, so the apply's revalidation is
/// what refuses to publish a new data root beneath the replacement.
#[cfg(unix)]
#[test]
fn test_absent_data_parent_replacement_before_publication_never_succeeds() {
    let outer = TempDir::new().unwrap();
    let worktree = outer.path().join("project");
    let data = worktree.join(".jit");
    let replacement = outer.path().join("replacement-project");
    let detached = outer.path().join("detached-project");
    std::fs::create_dir(&worktree).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("replacement-marker"), b"live").unwrap();

    let hook_worktree = worktree.clone();
    let injector = HookAtMoment::new(MutationPoint::BeforeCommitCheck, move || {
        std::fs::rename(&hook_worktree, &detached).unwrap();
        std::fs::rename(&replacement, &hook_worktree).unwrap();
    });
    let layout = discover_repository_layout(&worktree, &data).unwrap();
    let storage = JsonFileStorage::with_mutation_interruptions(&data, injector);
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let path = VirtualPath::data("index.json").unwrap();
    let spec = CaptureSpec::new([VirtualPath::data("").unwrap(), path.clone()], budget()).unwrap();
    let image = session.capture(spec).unwrap();
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            path,
            RepositoryActionOwner::RepositoryInit,
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
    let mut spec = CaptureSpec::new([VirtualPath::data("shared").unwrap()], budget()).unwrap();
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
                RepositoryActionOwner::RepositoryStateMutation,
                worktree_pre,
                FileMode::Executable,
            ),
            RepositoryAction::set_mode(
                VirtualPath::data("shared").unwrap(),
                RepositoryActionOwner::ProfileApplication,
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
    let spec = publishing_spec([VirtualPath::data("x").unwrap()]);
    let image = session.capture(spec).unwrap();
    let expected = ExpectedPreimage::of(image.entry(&VirtualPath::data("x").unwrap()).unwrap());
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            VirtualPath::data("x").unwrap(),
            RepositoryActionOwner::RepositoryStateMutation,
            expected,
            b"v2".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();

    // A write target moves between capture and apply, so the pre-commit check
    // refuses the mutation before its record is written.
    {
        let mut state = memory.repository_state();
        state.entries.insert(
            VirtualPath::data("x").unwrap(),
            RepositoryEntry::File {
                content: CapturedBytes::new("mem:x", b"changed".to_vec()).unwrap(),
                mode: FileMode::Regular,
            },
        );
    }
    assert!(matches!(
        session.apply(&test_plan(&image, &delta)),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

// --- Backend publication of prebuilt payload bytes -----------------------

#[test]
fn test_backends_publish_prebuilt_gate_run_and_event_bytes() {
    let worktree = TempDir::new().unwrap();
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let data = worktree.path().join(".jit");
    let run_id = crate::test_utils::gate_run_id(1);
    let result = crate::domain::GateRunResult {
        schema_version: 1,
        run_id: run_id.clone(),
        gate_key: "cargo-ci".parse().unwrap(),
        stage: crate::declarations::GateStage::Postcheck,
        issue_id: crate::test_utils::issue_id(7),
        commit: Some("abc123".to_string()),
        branch: Some("main".to_string()),
        status: crate::domain::GateRunStatus::Passed,
        started_at: timestamp,
        completed_at: Some(timestamp),
        duration_ms: Some(1500),
        exit_code: Some(0),
        stdout: "ok".to_string(),
        stderr: String::new(),
        command: "cargo test".to_string(),
        by: Some("agent:worker-1".parse().unwrap()),
        message: None,
        findings: None,
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
    };
    let provenance = crate::test_utils::stamped_event(
        EventBody::draft_gate_definition_created("cargo-ci".parse().unwrap()),
        "prov-1",
        timestamp,
    );
    let mut event_line = crate::repository_state::serialize_event(&provenance).unwrap();
    event_line.push(b'\n');
    let gate_bytes = serialize_gate_run(&result).unwrap();
    // Both backends must publish these prebuilt payloads unchanged.
    let run_dir = VirtualPath::gate_run_issue_directory(&result.issue_id);
    let run_file = VirtualPath::gate_run_record(&result.issue_id, &run_id);
    let events = VirtualPath::data("events.jsonl").unwrap();
    let build_delta = |layout: &RepositoryLayout| {
        RepositoryDelta::new(
            layout,
            vec![
                RepositoryAction::create_directory(
                    run_dir.clone(),
                    RepositoryActionOwner::RepositoryStateMutation,
                    ExpectedPreimage::Absent,
                ),
                RepositoryAction::write_file(
                    run_file.clone(),
                    RepositoryActionOwner::RepositoryStateMutation,
                    ExpectedPreimage::Absent,
                    gate_bytes.clone(),
                    FileMode::Regular,
                ),
                RepositoryAction::append_events(
                    RepositoryActionOwner::RepositoryStateMutation,
                    ExpectedPreimage::Absent,
                    0,
                    event_line.clone(),
                ),
            ],
        )
        .unwrap()
    };
    let spec = || {
        let mut spec = CaptureSpec::new(
            [VirtualPath::GATE_RUNS, run_dir.clone(), run_file.clone()],
            budget(),
        )
        .unwrap();
        spec.discover_tail(events.clone()).unwrap();
        spec
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
    let json_gate = std::fs::read(data.join(run_file.relative().as_path())).unwrap();
    let json_events = std::fs::read(data.join("events.jsonl")).unwrap();
    let state = memory.repository_state();
    let memory_gate = match state.entries.get(&run_file).unwrap() {
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
        other => panic!("expected gate-run file, got {other:?}"),
    };
    let memory_events = match state.entries.get(&events).unwrap() {
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
        other => panic!("expected events file, got {other:?}"),
    };
    assert_eq!(
        json_gate, gate_bytes,
        "JSON storage publishes the gate-run payload"
    );
    assert_eq!(
        memory_gate, gate_bytes,
        "memory storage publishes the gate-run payload"
    );
    assert_eq!(
        json_events, event_line,
        "JSON storage publishes the event payload"
    );
    assert_eq!(
        memory_events, event_line,
        "memory storage publishes the event payload"
    );
}

#[cfg(unix)]
fn pinned_capture_spec(paths: impl IntoIterator<Item = &'static str>) -> CaptureSpec {
    let mut spec = CaptureSpec::new([], budget()).unwrap();
    paths
        .into_iter()
        .try_for_each(|path| spec.discover_pinned("HEAD", path))
        .unwrap();
    spec
}

/// A kind-only pinned request classifies its file from the commit tree without
/// reading the blob, and its bytes-free answer never serves a later whole read.
#[cfg(unix)]
#[test]
fn test_json_capture_reads_no_blob_for_a_kind_only_pinned_request() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    crate::test_cli::init_git_repository(worktree.path()).unwrap();
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"pinned a").unwrap();
    for args in [&["add", "-A"][..], &["commit", "-m", "first"]] {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed");
    }
    let stand_in = GitStandIn::new();
    let storage = JsonFileStorage::with_git(
        &data,
        crate::storage::git::Git::with_program(stand_in.program()),
    );
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let request = ("HEAD".to_string(), "docs/a.md".to_string());
    let blob_reads = || {
        stand_in
            .invocations()
            .iter()
            .filter(|invocation| invocation.args() == ["cat-file", "--batch"])
            .count()
    };
    let mut kind = CaptureSpec::new([], budget()).unwrap();
    kind.discover_pinned_kind(&request.0, &request.1).unwrap();

    let image = session.capture(kind).unwrap();

    let evidence = &image.pinned_evidence()[&request];
    assert!(evidence.exists());
    assert_eq!(evidence.bytes(), None);
    assert_eq!(blob_reads(), 0);

    let whole = session.capture(pinned_capture_spec(["docs/a.md"])).unwrap();

    assert_eq!(
        whole.pinned_evidence()[&request].bytes(),
        Some(&b"pinned a"[..])
    );
    assert_eq!(blob_reads(), 1);
}

#[cfg(unix)]
#[test]
fn test_json_capture_reuses_pinned_target_cache_for_unchanged_commit() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    crate::test_cli::init_git_repository(worktree.path()).unwrap();
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"first a").unwrap();
    std::fs::write(worktree.path().join("docs/b.md"), b"first b").unwrap();
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "first"]);
    let _revision = {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let stand_in = GitStandIn::new();
    let storage = JsonFileStorage::with_git(
        &data,
        crate::storage::git::Git::with_program(stand_in.program()),
    );
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let spec = pinned_capture_spec(["docs/a.md", "docs/b.md"]);

    session.capture(spec.clone()).unwrap();
    let first_capture_processes = stand_in.invocations();
    assert_eq!(first_capture_processes.len(), 3);
    session.capture(spec).unwrap();
    let all_processes = stand_in.invocations();
    assert_eq!(all_processes.len(), 4);
    assert_eq!(
        all_processes
            .iter()
            .filter(|invocation| invocation
                .args()
                .first()
                .is_some_and(|argument| argument == "rev-parse"))
            .count(),
        2
    );
    assert_eq!(
        all_processes
            .iter()
            .filter(|invocation| invocation
                .args()
                .iter()
                .any(|argument| argument == "ls-tree"))
            .count(),
        1
    );
    assert_eq!(
        all_processes
            .iter()
            .filter(|invocation| invocation.args() == ["cat-file", "--batch"])
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn test_json_capture_re_resolves_moved_head_and_rekeys_pinned_cache() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    crate::test_cli::init_git_repository(worktree.path()).unwrap();
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"first a").unwrap();
    std::fs::write(worktree.path().join("docs/b.md"), b"first b").unwrap();
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "first"]);
    let first = {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let stand_in = GitStandIn::new();
    let storage = JsonFileStorage::with_git(
        &data,
        crate::storage::git::Git::with_program(stand_in.program()),
    );
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let spec = pinned_capture_spec(["docs/a.md"]);
    let initial = session.capture(spec.clone()).unwrap();
    assert_eq!(
        initial.pinned_evidence()[&("HEAD".into(), "docs/a.md".into())].bytes(),
        Some(b"first a".as_slice())
    );

    std::fs::write(worktree.path().join("docs/a.md"), b"second a").unwrap();
    let output = Command::new("git")
        .current_dir(worktree.path())
        .args(["add", "docs/a.md"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = Command::new("git")
        .current_dir(worktree.path())
        .args(["commit", "-m", "second"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let second = {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };

    let moved = session.capture(spec).unwrap();
    assert_eq!(
        moved.pinned_evidence()[&("HEAD".into(), "docs/a.md".into())].bytes(),
        Some(b"second a".as_slice())
    );
    assert_eq!(
        storage.pinned_target_cache_keys(),
        vec![(second, "docs/a.md".to_string())]
    );
    assert_ne!(first, storage.pinned_target_cache_keys()[0].0);
}

#[cfg(unix)]
#[test]
fn test_json_capture_keeps_pinned_target_cache_across_captures_that_pin_nothing() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    crate::test_cli::init_git_repository(worktree.path()).unwrap();
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"first a").unwrap();
    std::fs::write(worktree.path().join("docs/b.md"), b"first b").unwrap();
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "first"]);
    let revision = {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let stand_in = GitStandIn::new();
    let storage = JsonFileStorage::with_git(
        &data,
        crate::storage::git::Git::with_program(stand_in.program()),
    );
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let spec = pinned_capture_spec(["docs/a.md", "docs/b.md"]);
    session.capture(spec.clone()).unwrap();
    let after_first = stand_in.invocations().len();
    session.capture(pinned_capture_spec([])).unwrap();
    assert_eq!(stand_in.invocations().len(), after_first);
    assert_eq!(
        storage.pinned_target_cache_keys(),
        vec![
            (revision.clone(), "docs/a.md".to_string()),
            (revision, "docs/b.md".to_string()),
        ]
    );
    session.capture(spec).unwrap();
    let repeated = stand_in.invocations()[after_first..].to_vec();
    assert!(
        repeated.len() == 1
            && repeated[0]
                .args()
                .first()
                .is_some_and(|argument| argument == "rev-parse"),
        "a capture after one that pins nothing must reuse cached evidence, spawned {repeated:?}"
    );
}

#[test]
fn test_json_capture_retains_only_pairs_used_by_the_most_recent_capture() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    crate::test_cli::init_git_repository(worktree.path()).unwrap();
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"first a").unwrap();
    std::fs::write(worktree.path().join("docs/b.md"), b"first b").unwrap();
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "first"]);
    let revision = {
        let output = Command::new("git")
            .current_dir(worktree.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let stand_in = GitStandIn::new();
    let storage = JsonFileStorage::with_git(
        &data,
        crate::storage::git::Git::with_program(stand_in.program()),
    );
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();

    session
        .capture(pinned_capture_spec(["docs/a.md", "docs/b.md"]))
        .unwrap();
    assert_eq!(storage.pinned_target_cache_keys().len(), 2);
    assert_eq!(storage.clone().pinned_target_cache_keys().len(), 2);
    session.capture(pinned_capture_spec(["docs/b.md"])).unwrap();
    assert_eq!(
        storage.pinned_target_cache_keys(),
        vec![(revision, "docs/b.md".to_string())]
    );
}

/// A repository the pre-commit conflict check reads: seeded data-root files,
/// the capability roots over them, and the capture a mutation took of them.
struct WriteSetFixture {
    _worktree: TempDir,
    data: PathBuf,
    storage: JsonFileStorage,
    layout: RepositoryLayout,
    roots: CapabilityRoots,
}

impl WriteSetFixture {
    fn new(files: &[(&str, &[u8])]) -> Self {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir_all(data.join("records")).unwrap();
        for (relative, bytes) in files {
            std::fs::write(data.join(relative), bytes).unwrap();
        }
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let storage = JsonFileStorage::new(&data);
        storage.configure_repository_layout(&layout).unwrap();
        let roots = CapabilityRoots::open(&layout).unwrap().0;
        Self {
            _worktree: worktree,
            data,
            storage,
            layout,
            roots,
        }
    }

    /// The image a mutation reading `paths` captured.
    fn capture(&self, paths: impl IntoIterator<Item = VirtualPath>) -> RepositoryImage {
        let (tails, whole): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|path| *path == VirtualPath::EVENTS);
        let mut spec = CaptureSpec::new(whole, budget()).unwrap();
        for path in tails {
            spec.discover_tail(path).unwrap();
        }
        capture_capability_image(
            &self.layout,
            &self.roots,
            &self.storage,
            spec,
            CapturedAnswers::default(),
        )
        .unwrap()
    }

    /// Change a data-root file behind the mutation's back.
    fn write(&self, relative: &str, bytes: &[u8]) {
        std::fs::write(self.data.join(relative), bytes).unwrap();
    }

    fn check(
        &self,
        image: &RepositoryImage,
        record: &RedoRecord,
    ) -> Result<(), RepositoryStateStoreError> {
        ensure_write_set_unchanged(&self.roots, image, record)
    }
}

/// Whether `error` is the retryable conflict naming `path`.
fn names_moved_path(error: &RepositoryStateStoreError, path: &VirtualPath) -> bool {
    matches!(
        error,
        RepositoryStateStoreError::RetryableConflict { path: named }
            if named.contains(path.relative().as_str())
    )
}

fn seeded_log() -> &'static [u8] {
    b"{\"tag\":\"issue_created\"}\n"
}

#[test]
fn test_ensure_write_set_unchanged_admits_a_write_set_nothing_moved() {
    let fixture = WriteSetFixture::new(&[
        ("records/one.json", b"{\"value\":\"one\"}"),
        ("records/stale.json", b"{\"value\":\"stale\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let written = data_path("records/one.json");
    let removed = data_path("records/stale.json");
    let image = fixture.capture([written.clone(), removed.clone(), VirtualPath::EVENTS]);

    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(
            written,
            b"{\"value\":\"one\",\"state\":\"ready\"}",
        )],
        vec![removed],
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    assert!(fixture.check(&image, &record).is_ok());
}

/// A directory the record makes is a write target like any other, so a target
/// something occupied since the capture is the retryable conflict rather than a
/// committed record that can never apply.
#[test]
fn test_ensure_write_set_unchanged_refuses_a_made_directory_target_something_occupies() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let made = data_path("templates");
    let image = fixture.capture([made.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        vec![made.clone()],
        Vec::new(),
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write(
        "templates",
        b"a file another writer left at the directory's path",
    );

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &made),
        "the conflict must name the directory target that moved, got {error:?}"
    );
}

/// A directory another action's path lies beneath is created by that action, so
/// only the rest reach the record — where an interruption leaves them for the
/// next session to replay (OD-7).
#[test]
fn test_redo_record_for_carries_only_a_directory_no_other_action_lies_beneath() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let empty = data_path("templates");
    let ancestor = data_path("gates");
    let written = data_path("gates/review.json");
    let image = fixture.capture([empty.clone(), ancestor.clone(), written.clone()]);
    let delta = RepositoryDelta::new(
        &fixture.layout,
        vec![
            RepositoryAction::create_directory(
                empty.clone(),
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::create_directory(
                ancestor,
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
            ),
            RepositoryAction::write_file(
                written,
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            ),
        ],
    )
    .unwrap();

    let record = redo_record_for(&image, delta.actions().iter()).unwrap();

    assert_eq!(
        record.make_directories(),
        [empty],
        "the record carries a directory its own put already creates"
    );
}

#[test]
fn test_redo_record_for_carries_the_event_log_change_as_its_offset_and_appended_bytes() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let offset = seeded_log().len() as u64;
    let delta = RepositoryDelta::new(
        &fixture.layout,
        vec![RepositoryAction::append_events(
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::of(image.entry(&VirtualPath::EVENTS).unwrap()),
            offset,
            b"{\"tag\":\"next\"}\n".to_vec(),
        )],
    )
    .unwrap();

    let record = redo_record_for(&image, delta.actions().iter()).unwrap();

    assert_eq!(
        record.event_append(),
        Some(&EventAppend {
            offset,
            bytes: b"{\"tag\":\"next\"}\n".to_vec(),
        })
    );
    assert!(
        record.puts().is_empty(),
        "the appended bytes are the only log bytes the record writes"
    );
}

#[test]
fn test_ensure_write_set_unchanged_refuses_a_put_target_whose_bytes_changed() {
    let fixture = WriteSetFixture::new(&[
        ("records/one.json", b"{\"value\":\"one\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let written = data_path("records/one.json");
    let image = fixture.capture([written.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(
            written.clone(),
            b"{\"value\":\"one\",\"state\":\"ready\"}",
        )],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("records/one.json", b"{\"value\":\"one\",\"raced\":true}");

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &written),
        "the conflict must name the write target that moved, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_refuses_a_delete_target_whose_bytes_changed() {
    let fixture = WriteSetFixture::new(&[
        ("records/stale.json", b"{\"value\":\"stale\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let removed = data_path("records/stale.json");
    let image = fixture.capture([removed.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        vec![removed.clone()],
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write(
        "records/stale.json",
        b"{\"value\":\"stale\",\"raced\":true}",
    );

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &removed),
        "the conflict must name the delete target that moved, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_a_read_but_unwritten_file_that_changed() {
    let fixture = WriteSetFixture::new(&[
        ("index.json", b"{\"values\":[]}"),
        ("records/one.json", b"{\"value\":\"one\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let read_only = data_path("index.json");
    let written = data_path("records/one.json");
    let image = fixture.capture([read_only, written.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(
            written,
            b"{\"value\":\"one\",\"state\":\"ready\"}",
        )],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("index.json", b"{\"values\":[\"raced\"]}");

    assert!(
        fixture.check(&image, &record).is_ok(),
        "a file the mutation read but does not write stays out of the comparison"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_a_target_expected_absent_that_is_still_absent() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let created = data_path("records/new.json");
    let image = fixture.capture([created.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(created, b"{\"value\":\"new\"}")],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    assert!(fixture.check(&image, &record).is_ok());
}

#[test]
fn test_ensure_write_set_unchanged_refuses_a_target_expected_absent_that_became_occupied() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let created = data_path("records/new.json");
    let image = fixture.capture([created.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(created.clone(), b"{\"value\":\"new\"}")],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("records/new.json", b"{\"value\":\"new\",\"raced\":true}");

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &created),
        "the conflict must name the occupied target, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_refuses_an_event_log_whose_length_changed() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    let mut grown = seeded_log().to_vec();
    grown.extend_from_slice(b"{\"tag\":\"issue_claimed\"}\n");
    fixture.write("events.jsonl", &grown);

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &VirtualPath::EVENTS),
        "the conflict must name the event log, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_refuses_an_event_log_shorter_than_the_tail_window_edited_in_place(
) {
    let log = seeded_log();
    assert!((log.len() as u64) < EVENT_LOG_TAIL_WINDOW);
    let fixture = WriteSetFixture::new(&[("events.jsonl", log)]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        event_append(log.len() as u64),
    )
    .unwrap();

    let mut edited = log.to_vec();
    let last = edited.len() - 2;
    edited[last] = b'X';
    fixture.write("events.jsonl", &edited);

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &VirtualPath::EVENTS),
        "an in-place tail edit of a short log must conflict, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_an_empty_event_log_that_is_still_empty() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", b"")]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let record = RedoRecord::new(Vec::new(), Vec::new(), Vec::new(), event_append(0)).unwrap();

    assert!(fixture.check(&image, &record).is_ok());
}

#[test]
fn test_ensure_write_set_unchanged_refuses_an_event_log_edited_inside_the_tail_window() {
    let log = long_log();
    let fixture = WriteSetFixture::new(&[("events.jsonl", &log)]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        event_append(log.len() as u64),
    )
    .unwrap();

    let mut edited = log.clone();
    let last = edited.len() - 2;
    edited[last] = b'X';
    fixture.write("events.jsonl", &edited);

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &VirtualPath::EVENTS),
        "an edit inside the tail window must conflict, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_an_event_log_edited_before_the_tail_window() {
    let log = long_log();
    let fixture = WriteSetFixture::new(&[("events.jsonl", &log)]);
    let image = fixture.capture([VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        event_append(log.len() as u64),
    )
    .unwrap();

    let mut edited = log.clone();
    edited[0] = b'X';
    fixture.write("events.jsonl", &edited);

    assert!(
        fixture.check(&image, &record).is_ok(),
        "the log is compared as its length and tail window, not byte for byte"
    );
}

/// An event log longer than the tail window, so a prefix edit falls outside it.
fn long_log() -> Vec<u8> {
    let line = b"{\"tag\":\"issue_created\",\"padding\":\"padding-padding-padding\"}\n";
    line.iter()
        .copied()
        .cycle()
        .take((EVENT_LOG_TAIL_WINDOW as usize) + line.len() * 2)
        .collect()
}

/// REQ-01, observed through the session's own first capture: nothing else can
/// tell an application that preceded it from one the capture triggered.
fn assert_open_applies_the_leftover<B: SessionRecordBackend>() {
    let backend = B::seeded();
    backend.commit(&leftover_record());

    let mut session = backend.open().unwrap();
    let image = session
        .capture(CaptureSpec::new([leftover_target()], budget()).unwrap())
        .unwrap();

    assert!(
        matches!(
            image.entry(&leftover_target()),
            Ok(RepositoryEntry::File { content, .. }) if content.bytes() == LEFTOVER_BYTES
        ),
        "the session's first capture must read the applied record, got {:?}",
        image.entry(&leftover_target())
    );
    drop(session);
    assert_eq!(
        backend.bytes_at(&VirtualPath::EVENTS).as_deref(),
        Some(appended_log().as_slice()),
        "the record's appended events must reach the log"
    );
    assert!(
        !backend.holds_record(),
        "an applied record must leave the commit point"
    );
}

/// REQ-01 and REQ-04, at the seam every publisher reaches: an apply interrupted
/// inside its own record application has already passed the commit point, so
/// the repository holds the record and the complete old state, and the next
/// session open carries it to the complete new state.
///
/// The record's own crash matrix is proven against the record in
/// `redo_conformance`; what this states is that a session's publication goes
/// through it (`@/inv/orthogonal-tests`).
fn assert_an_interrupted_publication_converges<B: SessionRecordBackend>() {
    let target = leftover_target();
    let old = B::seeded();
    let published = {
        let mut session = old
            .open_interrupted(MutationPoint::Redo(RedoPoint::BeforePut { index: 0 }))
            .unwrap();
        let image = session.capture(publishing_spec([target.clone()])).unwrap();
        let delta = RepositoryDelta::new(
            session.layout(),
            vec![RepositoryAction::write_file(
                target.clone(),
                RepositoryActionOwner::RepositoryStateMutation,
                ExpectedPreimage::Absent,
                LEFTOVER_BYTES.to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        let before = old.published();

        assert!(
            session.apply(&test_plan(&image, &delta)).is_err(),
            "the injected interruption must stop the publication"
        );

        assert_eq!(
            old.published(),
            before,
            "the interruption preceded the first live write, so nothing may be published"
        );
        before
    };
    assert!(
        old.holds_record(),
        "an interrupted publication must leave the record that carries it forward"
    );

    drop(old.open().unwrap());

    assert_eq!(
        old.bytes_at(&target).as_deref(),
        Some(LEFTOVER_BYTES),
        "the next open must carry the interrupted mutation to its complete new state"
    );
    assert_ne!(old.published(), published);
    assert!(
        !old.holds_record(),
        "a converged repository must leave the commit point"
    );
}

/// A publication over a present data root that appends no event is refused
/// before its record commits: the event log's length counts every commit.
fn assert_a_publication_without_an_event_append_is_refused<B: SessionRecordBackend>() {
    let backend = B::seeded();
    let note = VirtualPath::worktree("note.txt").unwrap();
    let mut session = backend.open().unwrap();
    let before = backend.published();
    // The image shows no data-root entry, so the plan composes no generic event.
    let image = session
        .capture(CaptureSpec::new([note.clone()], budget()).unwrap())
        .unwrap();
    let delta = RepositoryDelta::new(
        session.layout(),
        vec![RepositoryAction::write_file(
            note,
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"note".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    let plan = test_plan(&image, &delta);
    assert_eq!(
        plan.publication(),
        &delta,
        "the publication appends nothing"
    );

    let outcome = session.apply(&plan);

    assert!(
        matches!(
            outcome,
            Err(RepositoryStateStoreError::RecordWithoutEventAppend)
        ),
        "got {outcome:?}"
    );
    drop(session);
    assert_eq!(backend.published(), before, "the repository is unchanged");
    assert!(!backend.holds_record(), "no record is left behind");
}

#[test]
fn test_apply_refuses_a_record_without_an_event_append_on_both_backends() {
    assert_a_publication_without_an_event_append_is_refused::<FileSessionBackend>();
    assert_a_publication_without_an_event_append_is_refused::<MemorySessionBackend>();
}

#[test]
fn test_apply_interrupted_inside_its_record_converges_at_the_next_open_on_both_backends() {
    assert_an_interrupted_publication_converges::<FileSessionBackend>();
    assert_an_interrupted_publication_converges::<MemorySessionBackend>();
}

#[test]
fn test_open_mutation_session_applies_a_leftover_record_before_capture_on_both_backends() {
    assert_open_applies_the_leftover::<FileSessionBackend>();
    assert_open_applies_the_leftover::<MemorySessionBackend>();
}

/// REQ-02.
fn assert_two_opens_converge_as_one<B: SessionRecordBackend>() {
    let backend = B::seeded();
    backend.commit(&leftover_record());

    drop(backend.open().unwrap());
    let after_one = backend.published();
    drop(backend.open().unwrap());

    assert_eq!(
        backend.published(),
        after_one,
        "opening twice over one seeded record must leave what opening once left"
    );
}

#[test]
fn test_open_mutation_session_twice_over_one_record_converges_as_once_on_both_backends() {
    assert_two_opens_converge_as_one::<FileSessionBackend>();
    assert_two_opens_converge_as_one::<MemorySessionBackend>();
}

/// REQ-03, at the report. Its two consumers are covered where they read it.
fn assert_report_counts_applied_records<B: SessionRecordBackend>() {
    let backend = B::seeded();
    let quiet = {
        let session = backend.open().unwrap();
        session.recovery_report().clone()
    };
    backend.commit(&leftover_record());
    let applied = {
        let session = backend.open().unwrap();
        session.recovery_report().clone()
    };

    assert_eq!(
        quiet.applied_records, 0,
        "a repository with no leftover record applies none"
    );
    assert_eq!(
        applied.applied_records, 1,
        "one leftover record is one applied record"
    );
    assert_eq!(
        applied.recovered_count(),
        quiet.recovered_count() + 1,
        "the applied record must reach the count both consumers read"
    );
}

#[test]
fn test_recovery_dispatch_report_counts_applied_records_on_both_backends() {
    assert_report_counts_applied_records::<FileSessionBackend>();
    assert_report_counts_applied_records::<MemorySessionBackend>();
}

/// REQ-05. The first open creates the session's own advisory lock files, so
/// the snapshot the refusal is measured against excludes nothing.
#[test]
fn test_open_mutation_session_failed_recovery_leaves_storage_unbound() {
    let worktree = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let scratch = data.join(RECORD_SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join(RECORD_FILE_NAME), b"invalid record").unwrap();
    let storage = JsonFileStorage::new(&data);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    assert!(storage.open_mutation_session(layout).is_err());
    assert!(storage.repository_layout().is_err());
    let alternate = discover_repository_layout(other.path(), &data).unwrap();
    storage.configure_repository_layout(&alternate).unwrap();
}

#[test]
fn test_open_mutation_session_refuses_foreign_binding_during_recovery() {
    let worktree = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let foreign = discover_repository_layout(other.path(), &data).unwrap();
    let slot = Arc::new(Mutex::new(None::<JsonFileStorage>));
    let source = Arc::clone(&slot);
    let same = layout.clone();
    let storage = JsonFileStorage::with_mutation_interruptions(
        &data,
        HookAtMoment::new(MutationPoint::SessionOpen, move || {
            let storage = source.lock().unwrap().clone().unwrap();
            let competing = storage.clone();
            assert!(
                std::thread::spawn(move || competing.configure_repository_layout(&foreign))
                    .join()
                    .unwrap()
                    .is_err()
            );
            storage.configure_repository_layout(&same).unwrap();
        }),
    );
    *slot.lock().unwrap() = Some(storage.clone());
    storage.open_mutation_session(layout).unwrap();
    slot.lock().unwrap().take();
}

#[test]
fn test_open_mutation_session_refuses_a_malformed_leftover_record_naming_it() {
    let backend = FileSessionBackend::seeded();
    drop(backend.open().unwrap());
    backend.commit(&leftover_record());
    std::fs::write(backend.record_path(), b"not a redo record").unwrap();
    let before = snapshot(backend.worktree.path());

    let error = match backend.open() {
        Err(error) => error,
        Ok(_) => panic!("a malformed leftover record must refuse the session open"),
    };

    assert!(
        matches!(
            &error,
            RepositoryStateStoreError::UnreadableRedoRecord { path, .. }
                if path.contains(RECORD_FILE_NAME)
        ),
        "a malformed leftover must name the record it refused, got {error:?}"
    );
    assert_eq!(
        snapshot(backend.worktree.path()),
        before,
        "a refused open must leave every byte it found, the record included"
    );
}

/// REQ-04. Every scratch entry but the record is sealed against opening, so a
/// pass that read one fails here; the listing is compared across the open, so
/// one that wrote there shows up too. A permission trap on the directory
/// itself cannot serve: cap-std needs a read handle on it even to open the
/// record by name.
#[cfg(unix)]
#[test]
fn test_open_mutation_session_reads_nothing_in_the_scratch_directory_but_the_record() {
    use std::os::unix::fs::PermissionsExt as _;

    let backend = FileSessionBackend::seeded();
    let scratch = backend.scratch_path();
    std::fs::create_dir(&scratch).unwrap();
    std::fs::write(scratch.join("decoy"), b"not a redo record").unwrap();
    std::fs::create_dir(scratch.join("nested")).unwrap();
    let sealed = std::fs::Permissions::from_mode(0o000);
    std::fs::set_permissions(scratch.join("decoy"), sealed.clone()).unwrap();
    std::fs::set_permissions(scratch.join("nested"), sealed).unwrap();
    let before = scratch_entries(&scratch);

    let opened = backend.open();

    let readable = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(scratch.join("nested"), readable).unwrap();
    let session = opened.expect("a poisoned scratch entry is never read, so the open succeeds");
    assert_eq!(
        session.recovery_report().applied_records,
        0,
        "no record is waiting, so none is applied"
    );
    assert_eq!(
        scratch_entries(&scratch),
        before,
        "checking for the record neither creates nor removes a scratch entry"
    );
}

#[cfg(unix)]
fn scratch_entries(scratch: &Path) -> BTreeSet<std::ffi::OsString> {
    std::fs::read_dir(scratch)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect()
}

/// A hook run once at one mutation moment, after which the session proceeds.
struct HookAtMoment {
    point: MutationPoint,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl HookAtMoment {
    fn new(point: MutationPoint, hook: impl FnOnce() + Send + 'static) -> Arc<Self> {
        Arc::new(Self {
            point,
            hook: Mutex::new(Some(Box::new(hook))),
        })
    }
}

impl crate::storage::MutationInterruptionInjector for HookAtMoment {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        if point == &self.point {
            if let Some(hook) = self.hook.lock().unwrap().take() {
                hook();
            }
        }
        Ok(())
    }
}

/// REQ-01: the complete scaffold stands in the sibling stage while nothing
/// occupies the destination, so the rename alone is what makes it the root.
#[test]
fn test_absent_data_root_publishes_a_complete_sibling_stage_by_rename() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let stage = crate::storage::init_publish::stage_path(&data);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let observed = Arc::new(Mutex::new(None));
    let (seen, staged, destination) = (Arc::clone(&observed), stage.clone(), data.clone());
    let storage = JsonFileStorage::with_mutation_interruptions(
        &data,
        HookAtMoment::new(MutationPoint::BeforeRootPublication, move || {
            *seen.lock().unwrap() = Some((
                std::fs::read(staged.join("index.json")).ok(),
                destination.exists(),
            ));
        }),
    );

    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .unwrap();

    assert_eq!(
        observed.lock().unwrap().take(),
        Some((Some(b"{}".to_vec()), false)),
        "the stage must hold the whole scaffold while the destination is still absent"
    );
    assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
    assert!(!stage.exists(), "the rename must leave no stage behind");
}

/// REQ-02: the refusal is the no-replace rename itself, so it holds against a
/// root published after every check this session could make.
#[test]
fn test_occupied_destination_refuses_absent_root_publication_and_leaves_it_untouched() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let occupant = data.clone();
    let storage = JsonFileStorage::with_mutation_interruptions(
        &data,
        HookAtMoment::new(MutationPoint::BeforeRootPublication, move || {
            std::fs::create_dir(&occupant).unwrap();
            std::fs::write(occupant.join("index.json"), b"occupied").unwrap();
        }),
    );

    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();

    assert!(matches!(
        session.apply(&test_plan(&image, &initialization_delta(&layout))),
        Err(RepositoryStateStoreError::OccupiedDataRoot { .. })
    ));
    assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"occupied");
}

/// REQ-03: an interruption before the rename publishes nothing at all, and the
/// stage it leaves is the next publication's to reclaim.
#[test]
fn test_interrupted_absent_root_publication_leaves_no_root_and_the_next_reclaims_its_stage() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let stage = crate::storage::init_publish::stage_path(&data);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let interrupted = JsonFileStorage::with_mutation_interruptions(
        &data,
        InterruptAtMoment::new(MutationPoint::BeforeRootPublication),
    );
    let mut session = interrupted.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    assert!(session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .is_err());
    drop(session);

    assert!(
        !data.exists(),
        "an interruption before the rename publishes nothing"
    );
    assert!(
        stage.exists(),
        "the interrupted publication's stage outlives it"
    );

    let next = JsonFileStorage::new(&data);
    let mut session = next.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .unwrap();

    assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
    assert!(
        !stage.exists(),
        "the next publication reclaims the orphan stage"
    );
}

/// REQ-04: the worktree targets travel in a redo record the stage carries into
/// the published root, so the session that publishes the root commits them.
#[test]
fn test_absent_root_publication_carries_its_worktree_targets_in_the_redo_record() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let note = worktree.path().join("note.txt");
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();

    let interrupted = JsonFileStorage::with_mutation_interruptions(
        &data,
        InterruptAtMoment::new(MutationPoint::Redo(RedoPoint::BeforePut { index: 0 })),
    );
    let mut session = interrupted.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    assert!(session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .is_err());
    drop(session);

    assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
    assert!(!note.exists(), "no worktree target precedes its record");
    assert!(
        data.join(RECORD_SCRATCH_DIR)
            .join(RECORD_FILE_NAME)
            .exists(),
        "the published root must carry the record its worktree targets wait in"
    );

    let committed = RedoRecord::parse(
        &std::fs::read(data.join(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        committed.worktree_binding(),
        Some(&crate::storage::redo_record::WorktreeRootBinding::from_layout(&layout))
    );
    let other = TempDir::new().unwrap();
    let next = JsonFileStorage::new(&data);
    let session = next
        .open_mutation_session(discover_repository_layout(other.path(), &data).unwrap())
        .unwrap();

    assert_eq!(session.recovery_report().applied_records, 1);
    assert_eq!(std::fs::read(&note).unwrap(), b"note");
    assert!(!other.path().join("note.txt").exists());
}

/// The delta's path order puts the run record first.
#[test]
fn test_redo_record_for_finalized_gate_run_precedes_issue_naming_it() {
    use crate::repository_state::{finalize, serialize_issue, MutationContext, MutationIntent};
    use crate::test_harness::image::{synthetic_layout, ImageBuilder};

    let context = MutationContext::deterministic([7; 32], chrono::Utc::now());
    let mut issue = crate::test_utils::fixture_issue("T".into(), String::new());
    let issue_path = VirtualPath::issue_record(&issue.id);
    let image = ImageBuilder::new()
        .file(issue_path.clone(), serialize_issue(&issue).unwrap())
        .entry(VirtualPath::GATE_RUNS, RepositoryEntry::Absent)
        .entry(
            VirtualPath::gate_run_issue_directory(&issue.id),
            RepositoryEntry::Absent,
        )
        .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
        .build();
    let key = "review".parse().unwrap();
    let run = crate::domain::GateRunResult::attestation(
        &context.gate_run_id_at(0),
        &issue.id,
        &key,
        crate::declarations::GateStage::Postcheck,
        crate::domain::GateStatus::Passed,
        None,
    );
    issue.gates_status.insert(
        key,
        crate::domain::GateState {
            status: crate::domain::GateStatus::Passed,
            updated_by: None,
            updated_at: run.started_at,
            run_id: Some(run.run_id.clone()),
        },
    );
    let run_path = VirtualPath::gate_run_record(&run.issue_id, &run.run_id);
    let plan = finalize(
        &synthetic_layout(),
        &image,
        &context,
        &[
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordGateRun {
                draft: Box::new(run),
            },
        ],
        None,
    )
    .unwrap();
    let record = redo_record_for(&image, plan.delta().actions().iter()).unwrap();
    let position = |path: &VirtualPath| {
        record
            .puts()
            .iter()
            .position(|put| &put.path == path)
            .unwrap()
    };
    assert!(position(&run_path) < position(&issue_path));
}

/// The file [`publish_every_operation`] deletes, published by
/// [`publish_doomed`] first.
fn doomed() -> VirtualPath {
    data_path("records/doomed.json")
}

fn made_directory() -> VirtualPath {
    data_path("made")
}

/// One complete publication: [`doomed`] is written, with the event its plan
/// composes.
fn publish_doomed(session: &mut dyn RepositoryMutationSession) {
    let image = session.capture(publishing_spec([doomed()])).unwrap();
    let delta = RepositoryDelta::new(
        session.layout(),
        vec![RepositoryAction::write_file(
            doomed(),
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::Absent,
            b"doomed".to_vec(),
            FileMode::Regular,
        )],
    )
    .unwrap();
    session.apply(&test_plan(&image, &delta)).unwrap();
}

/// A publication whose record makes a directory, puts a file, deletes
/// [`doomed`] and appends an event, so it crosses every [`RedoPoint`].
fn publish_every_operation(
    session: &mut dyn RepositoryMutationSession,
) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
    let image = session.capture(publishing_spec([
        made_directory(),
        leftover_target(),
        doomed(),
    ]))?;
    let owner = RepositoryActionOwner::RepositoryStateMutation;
    let delta = RepositoryDelta::new(
        session.layout(),
        vec![
            RepositoryAction::create_directory(made_directory(), owner, ExpectedPreimage::Absent),
            RepositoryAction::write_file(
                leftover_target(),
                owner,
                ExpectedPreimage::Absent,
                LEFTOVER_BYTES.to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::delete_file(
                doomed(),
                owner,
                ExpectedPreimage::of(image.entry(&doomed()).unwrap()),
            ),
        ],
    )
    .unwrap();
    session.apply(&test_plan(&image, &delta))
}

fn assert_admission_grants_a_capture_nothing_crossed<B: SessionRecordBackend>() {
    let backend = B::seeded();
    let mut read = backend.open_read().unwrap();
    let image = read.capture(publishing_spec([doomed()])).unwrap();
    read.extend(image, publishing_spec([doomed(), leftover_target()]))
        .unwrap();

    assert_eq!(read.admit().unwrap(), ReadAdmission::Granted);
}

fn assert_admission_refuses_a_mutation_between_capture_and_extend<B: SessionRecordBackend>() {
    let backend = B::seeded();
    let mut read = backend.open_read().unwrap();
    let image = read.capture(publishing_spec([doomed()])).unwrap();
    publish_doomed(backend.open().unwrap().as_mut());
    read.extend(image, publishing_spec([doomed(), leftover_target()]))
        .unwrap();

    assert_eq!(read.admit().unwrap(), ReadAdmission::Moved);
}

/// A record left at any point before [`RedoPoint::AfterApply`] is present at
/// admission, and at the next open; past it the record is gone and the log
/// grew.
fn assert_admission_names_an_interrupted_publication<B: SessionRecordBackend>() {
    let points = [
        RedoPoint::BeforeApply,
        RedoPoint::BeforeMakeDirectory { index: 0 },
        RedoPoint::BeforePut { index: 0 },
        RedoPoint::BeforeDelete { index: 0 },
        RedoPoint::BeforeEventAppend,
        RedoPoint::BeforeRecordRemoval,
        RedoPoint::AfterApply,
    ];
    points.into_iter().for_each(|point| {
        let backend = B::seeded();
        publish_doomed(backend.open().unwrap().as_mut());
        let mut read = backend.open_read().unwrap();
        read.capture(publishing_spec([doomed()])).unwrap();
        let mut writer = backend
            .open_interrupted(MutationPoint::Redo(point.clone()))
            .unwrap();
        assert!(
            publish_every_operation(writer.as_mut()).is_err(),
            "interrupted at {point:?}"
        );
        drop(writer);

        let (expected, at_open) = match point {
            RedoPoint::AfterApply => (ReadAdmission::Moved, ReadAdmission::Granted),
            _ => (ReadAdmission::Busy, ReadAdmission::Busy),
        };
        assert_eq!(read.admit().unwrap(), expected, "at {point:?}");
        assert_eq!(
            backend.open_read().unwrap().admit().unwrap(),
            at_open,
            "a read opened after an interruption at {point:?}"
        );
    });
}

#[test]
fn test_read_admission_grants_a_capture_nothing_crossed_on_both_backends() {
    assert_admission_grants_a_capture_nothing_crossed::<FileSessionBackend>();
    assert_admission_grants_a_capture_nothing_crossed::<MemorySessionBackend>();
}

#[test]
fn test_read_admission_refuses_a_mutation_between_capture_and_extend_on_both_backends() {
    assert_admission_refuses_a_mutation_between_capture_and_extend::<FileSessionBackend>();
    assert_admission_refuses_a_mutation_between_capture_and_extend::<MemorySessionBackend>();
}

#[test]
fn test_read_admission_is_busy_until_a_record_is_removed_on_both_backends() {
    assert_admission_names_an_interrupted_publication::<FileSessionBackend>();
    assert_admission_names_an_interrupted_publication::<MemorySessionBackend>();
}

#[test]
fn test_json_read_admission_refuses_a_capture_over_a_root_initialization_published() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    let mut read = storage.open_read_session(layout.clone()).unwrap();
    read.capture(initial_spec()).unwrap();

    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(initial_spec()).unwrap();
    session
        .apply(&test_plan(&image, &initialization_delta(&layout)))
        .unwrap();
    drop(session);

    assert!(data.exists(), "initialization published the root");
    assert_eq!(read.admit().unwrap(), ReadAdmission::Moved);
}

#[test]
fn test_open_mutation_session_recovers_original_worktree_before_opening_new_selection() {
    let backend = FileSessionBackend::seeded();
    let next = TempDir::new().unwrap();
    let record = RedoRecord::for_layout(
        &backend.layout(),
        Vec::new(),
        vec![put_file(
            VirtualPath::worktree("original.md").unwrap(),
            b"committed",
        )],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();
    backend.commit(&record);
    let selected = discover_repository_layout(next.path(), backend.layout.data_root()).unwrap();
    let session = backend.store().open_mutation_session(selected).unwrap();
    assert_eq!(session.recovery_report().recovered_count(), 1);
    assert_eq!(
        std::fs::read(backend.worktree.path().join("original.md")).unwrap(),
        b"committed"
    );
    assert!(!next.path().join("original.md").exists());
    assert!(!backend.holds_record());
}

fn bind_configuration(backend: &FileSessionBackend) {
    backend
        .storage
        .configure_repository_layout(&backend.layout)
        .unwrap();
    backend.storage.bind_selection(
        ConfigurationObservation::capture(backend.layout.data_root()).unwrap(),
        false,
        false,
    );
}

fn commit_configuration(backend: &FileSessionBackend, bytes: &[u8]) {
    let offset = std::fs::metadata(backend.layout.data_root().join("events.jsonl"))
        .unwrap()
        .len();
    backend.commit(
        &RedoRecord::new(
            Vec::new(),
            vec![put_file(VirtualPath::CONFIG, bytes)],
            Vec::new(),
            event_append(offset),
        )
        .unwrap(),
    );
}

#[test]
fn test_selection_observation_spans_handoff_and_consumer_capture() {
    for before_open in [true, false] {
        let backend = FileSessionBackend::seeded();
        bind_configuration(&backend);
        let mut read = (!before_open).then(|| backend.open_read().unwrap());
        commit_configuration(&backend, b"[worktree]\nroot = '../other'\n");
        JsonFileStorage::new(backend.layout.data_root())
            .recover_pending_records()
            .unwrap();
        if let Some(read) = read.as_mut() {
            read.capture(CaptureSpec::new([VirtualPath::CONFIG], budget()).unwrap())
                .unwrap();
            assert_eq!(read.admit().unwrap(), ReadAdmission::Moved);
        } else {
            assert!(matches!(
                backend.open_read(),
                Err(RepositoryStateStoreError::RetryableConflict { .. })
            ));
        }
    }
}

#[test]
fn test_selection_observation_refreshes_after_an_unrelated_commit() {
    let backend = FileSessionBackend::seeded();
    bind_configuration(&backend);
    assert_eq!(
        backend.open_read().unwrap().admit().unwrap(),
        ReadAdmission::Granted
    );
    backend.commit(&leftover_record());
    backend.storage.recover_pending_records().unwrap();
    let mut read = backend.open_read().unwrap();
    let image = read
        .capture(CaptureSpec::new([leftover_target()], budget()).unwrap())
        .unwrap();
    assert!(matches!(
        image.entry(&leftover_target()).unwrap(),
        RepositoryEntry::File { .. }
    ));
    assert_eq!(read.admit().unwrap(), ReadAdmission::Granted);
}

#[test]
fn test_selection_observation_revalidates_retained_and_claim_first_sessions() {
    for retained in [true, false] {
        let backend = FileSessionBackend::seeded();
        bind_configuration(&backend);
        let mut session = retained.then(|| backend.open().unwrap());
        commit_configuration(&backend, b"[worktree]\nroot = '../different'\n");
        backend.storage.recover_pending_records().unwrap();
        if let Some(session) = session.as_mut() {
            assert!(matches!(
                session.capture(CaptureSpec::new([VirtualPath::CONFIG], budget()).unwrap()),
                Err(RepositoryStateStoreError::RetryableConflict { .. })
            ));
        } else {
            assert!(matches!(
                backend.open(),
                Err(RepositoryStateStoreError::RetryableConflict { .. })
            ));
        }
    }
}

#[test]
fn test_configuration_read_with_held_writer_and_pending_record_conflicts_without_replay() {
    let backend = FileSessionBackend::seeded();
    let held = backend.open().unwrap();
    backend.commit(&leftover_record());
    let record = std::fs::read(backend.record_path()).unwrap();
    let data = backend.layout.data_root().to_path_buf();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        send.send(ConfigurationObservation::capture(&data).map(|_| ()))
            .unwrap()
    });
    let result = receive.recv_timeout(std::time::Duration::from_secs(2));
    drop(held);
    reader.join().unwrap();
    let error = result
        .expect("a read never waits on the held writer")
        .unwrap_err();
    assert!(error
        .downcast_ref::<RepositoryStateStoreError>()
        .is_some_and(|error| matches!(error, RepositoryStateStoreError::RetryableConflict { .. })));
    assert_eq!(std::fs::read(backend.record_path()).unwrap(), record);
    assert!(backend.bytes_at(&leftover_target()).is_none());
}

#[test]
fn test_configuration_observation_admits_absent_ancestry_until_publication_or_replacement() {
    let outer = TempDir::new().unwrap();
    let ancestor = outer.path().join("new");
    let data = ancestor.join("nested/data");
    let observation = ConfigurationObservation::capture(&data).unwrap();
    assert!(observation.config.worktree.is_none());
    assert_eq!(observation.admit().unwrap(), ReadAdmission::Granted);
    assert!(std::fs::read_dir(outer.path()).unwrap().next().is_none());

    std::fs::create_dir(&ancestor).unwrap();
    assert_eq!(observation.admit().unwrap(), ReadAdmission::Moved);
    let observation = ConfigurationObservation::capture(&data).unwrap();
    std::fs::rename(&ancestor, outer.path().join("old")).unwrap();
    std::fs::create_dir(&ancestor).unwrap();
    assert_eq!(observation.admit().unwrap(), ReadAdmission::Moved);

    let observation = ConfigurationObservation::capture(&data).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("config.toml"), "[worktree]\nroot = '../code'\n").unwrap();
    assert_eq!(observation.admit().unwrap(), ReadAdmission::Moved);
    let published = ConfigurationObservation::capture(&data).unwrap();
    assert_eq!(
        published.config.worktree.unwrap().root,
        Some(PathBuf::from("../code"))
    );
}

#[test]
fn test_data_read_rejects_ancestor_published_during_absence_discovery() {
    let outer = TempDir::new().unwrap();
    let ancestor = outer.path().join("new/nested");
    let data = ancestor.join("data");
    let result = DataReadSession::open_after_missing(&data, || {
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(ancestor.join("config.toml"), "invalid ancestor declaration").unwrap();
        std::fs::write(data.join("config.toml"), "[worktree]\nroot = '../code'\n").unwrap();
    });
    assert!(matches!(
        result,
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    let selected = ConfigurationObservation::capture(&data).unwrap();
    assert_eq!(
        selected.config.worktree.unwrap().root,
        Some(PathBuf::from("../code"))
    );
}

#[test]
fn test_data_read_admission_detects_root_replacement() {
    let outer = TempDir::new().unwrap();
    let data = outer.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let read = DataReadSession::open(&data).unwrap();
    std::fs::rename(&data, outer.path().join("old")).unwrap();
    std::fs::create_dir(&data).unwrap();
    assert_eq!(read.admit().unwrap(), ReadAdmission::Moved);
}

#[cfg(unix)]
#[test]
fn test_recovery_original_root_preflight_refuses_all_writes_until_identity_returns() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for failure in ["missing", "replaced", "symlink", "inaccessible"] {
        let outer = TempDir::new().unwrap();
        let code = outer.path().join("code");
        let data = outer.path().join("data");
        std::fs::create_dir(&code).unwrap();
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("marker"), b"old").unwrap();
        let layout = discover_repository_layout(&code, &data).unwrap();
        let record = RedoRecord::for_layout(
            &layout,
            Vec::new(),
            vec![
                put_file(VirtualPath::data("marker").unwrap(), b"new"),
                put_file(VirtualPath::worktree("note").unwrap(), b"original"),
            ],
            Vec::new(),
            None,
        )
        .unwrap();
        commit_redo_record(&crate::storage::DataRoot::open(&data).unwrap(), &record).unwrap();
        let record_path = data.join(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME);
        let committed = std::fs::read(&record_path).unwrap();
        let old = outer.path().join("original");
        if failure == "inaccessible" {
            std::fs::set_permissions(&code, std::fs::Permissions::from_mode(0o0)).unwrap();
        } else {
            std::fs::rename(&code, &old).unwrap();
            match failure {
                "replaced" => std::fs::create_dir(&code).unwrap(),
                "symlink" => symlink(&old, &code).unwrap(),
                _ => {}
            }
        }
        let recovery = JsonFileStorage::new(&data).recover_pending_records();
        if failure == "inaccessible" {
            std::fs::set_permissions(&code, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(
            matches!(recovery, Err(RepositoryStateStoreError::RedoApply(error)) if matches!(*error, RedoApplyError::OriginalWorktree { .. })),
            "{failure}"
        );
        assert_eq!(
            std::fs::read(data.join("marker")).unwrap(),
            b"old",
            "{failure}"
        );
        assert_eq!(std::fs::read(&record_path).unwrap(), committed);
        if failure != "inaccessible" {
            match failure {
                "replaced" => std::fs::remove_dir_all(&code).unwrap(),
                "symlink" => std::fs::remove_file(&code).unwrap(),
                _ => {}
            }
            std::fs::rename(&old, &code).unwrap();
        }
        assert_eq!(
            JsonFileStorage::new(&data)
                .recover_pending_records()
                .unwrap()
                .recovered_count(),
            1
        );
        assert_eq!(std::fs::read(code.join("note")).unwrap(), b"original");
    }
}

#[test]
fn test_clean_session_refuses_record_appearing_after_recovery_handoff() {
    let backend = FileSessionBackend::seeded();
    let data = backend.layout.data_root().to_path_buf();
    let record = leftover_record();
    let storage = JsonFileStorage::with_mutation_interruptions(
        &data,
        HookAtMoment::new(MutationPoint::SessionOpen, {
            let data = data.clone();
            move || {
                commit_redo_record(&crate::storage::DataRoot::open(&data).unwrap(), &record)
                    .unwrap()
            }
        }),
    );
    assert!(matches!(
        storage.open_mutation_session(backend.layout()),
        Err(RepositoryStateStoreError::RetryableConflict { .. })
    ));
    assert!(backend.holds_record());
    assert!(backend.bytes_at(&leftover_target()).is_none());
    assert_eq!(
        storage
            .open_mutation_session(backend.layout())
            .unwrap()
            .recovery_report()
            .recovered_count(),
        1
    );
}

#[test]
fn test_recovery_restarts_when_record_identity_changes_after_lock_selection_peek() {
    let backend = FileSessionBackend::seeded();
    backend.commit(&leftover_record());
    let path = backend.record_path();
    let mut peek_count = 0;
    let report = backend
        .storage
        .recover_after_peek(|| {
            peek_count += 1;
            if peek_count == 1 {
                let old = path.with_extension("old");
                std::fs::rename(&path, &old).unwrap();
                std::fs::write(&path, std::fs::read(old).unwrap()).unwrap();
            }
        })
        .unwrap();
    assert_eq!(
        peek_count, 2,
        "identical record bytes on a different object still require another ordered acquisition"
    );
    assert_eq!(report.recovered_count(), 1);
}

#[test]
fn test_recovery_restarts_with_new_binding_without_worktree_lock_inversion() {
    let backend = FileSessionBackend::seeded();
    let original_code = TempDir::new().unwrap();
    let original =
        discover_repository_layout(original_code.path(), backend.layout().data_root()).unwrap();
    let next = TempDir::new().unwrap();
    let layout = discover_repository_layout(next.path(), original.data_root()).unwrap();
    let record = |layout: &RepositoryLayout| {
        RedoRecord::for_layout(
            layout,
            Vec::new(),
            vec![put_file(VirtualPath::worktree("note").unwrap(), b"bound")],
            Vec::new(),
            None,
        )
        .unwrap()
    };
    backend.commit(&record(&original));
    let path = backend.record_path();
    let replacement = record(&layout).serialize().unwrap();
    let mut replace = Some(replacement);
    backend
        .storage
        .recover_after_peek(|| {
            if let Some(bytes) = replace.take() {
                std::fs::remove_file(&path).unwrap();
                std::fs::write(&path, bytes).unwrap();
                std::fs::remove_dir(original.worktree_root()).unwrap();
            }
        })
        .unwrap();
    assert!(!original.worktree_root().join("note").exists());
    assert_eq!(std::fs::read(next.path().join("note")).unwrap(), b"bound");
}

#[test]
fn test_recovery_restarts_when_data_root_changes_after_lock_selection_peek() {
    let outer = TempDir::new().unwrap();
    let data = outer.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let record = leftover_record();
    commit_redo_record(&crate::storage::DataRoot::open(&data).unwrap(), &record).unwrap();
    let mut replaced = false;
    let report = JsonFileStorage::new(&data)
        .recover_after_peek(|| {
            if !replaced {
                replaced = true;
                std::fs::rename(&data, outer.path().join("old")).unwrap();
                std::fs::create_dir(&data).unwrap();
            }
        })
        .unwrap();
    assert_eq!(report.recovered_count(), 0);
    assert!(outer
        .path()
        .join("old")
        .join(RECORD_SCRATCH_DIR)
        .join(RECORD_FILE_NAME)
        .exists());
    assert!(!data.join(leftover_target().relative().as_path()).exists());
}
