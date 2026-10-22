use super::*;
use crate::repository_state::{CaptureBudget, RepositoryAction, EVENT_LOG_TAIL_WINDOW};
use crate::storage::memory::test_fixtures as memory_fixtures;
use crate::storage::redo_apply::test_fixtures::{
    data_root as redo_root, record_relative_path, record_scratch_relative_path,
    repository as redo_repository, seed_repository, snapshot, SnapshotEntry, CAPTURED_LOG,
};
use crate::storage::{EventAppend, PutFile, RedoPoint, RECORD_FILE_NAME, RECORD_SCRATCH_DIR};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

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
    let mut spec = CaptureSpec::new([published_path.clone()], budget()).unwrap();
    spec.discover_listing(notes_path.clone()).unwrap();
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

/// Close a hand-built test delta over its captured image.
fn test_plan(image: &RepositoryImage, delta: &RepositoryDelta) -> MaterializationPlan {
    MaterializationPlan::new(image, delta.clone())
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
    CaptureSpec::new(
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
    let issue = crate::domain::fixture_issue("Crossing".to_string(), "body".to_string());
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
        memory
            .resolve_issue_id(
                &crate::domain::issue_id::IssueIdPrefix::parse(&issue.short_id()).unwrap()
            )
            .unwrap(),
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

/// Publish `issue` through a mutation session on `storage` rooted at `worktree`.
fn publish_issue<S: RepositoryStateStore>(storage: &S, worktree: &Path, issue: &Issue) {
    let layout = discover_repository_layout(worktree, worktree.join(".jit")).unwrap();
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image = session.capture(issue_creation_spec(&issue.id)).unwrap();
    let bytes = serialize_issue(issue).unwrap();
    session
        .apply(&test_plan(
            &image,
            &issue_creation_delta(&layout, issue, &bytes),
        ))
        .unwrap();
}

/// Each backend resolves a full id in any case or hyphenation to the issue.
#[test]
fn test_conformance_resolve_issue_id_normalises_full_ids_on_both_backends() {
    fn assert_resolution(backend: &str, storage: &impl IssueStore, id: &str) {
        for requested in [id.to_string(), id.to_uppercase(), id.replace('-', "")] {
            let prefix = crate::domain::issue_id::IssueIdPrefix::parse(&requested).unwrap();
            assert_eq!(
                storage.resolve_issue_id(&prefix).ok().as_deref(),
                Some(id),
                "{backend} must resolve {requested}"
            );
        }
    }

    let issue = crate::domain::fixture_issue("Resolve".to_string(), String::new());
    let memory = InMemoryStorage::new();
    let memory_tree = TempDir::new().unwrap();
    publish_issue(&memory, memory_tree.path(), &issue);
    assert_resolution("memory", &memory, &issue.id);

    let json_tree = TempDir::new().unwrap();
    let json = JsonFileStorage::new(json_tree.path().join(".jit"));
    publish_issue(&json, json_tree.path(), &issue);
    assert_resolution("json", &json, &issue.id);
}

#[test]
fn test_json_and_memory_share_absent_root_actions_and_outcome() {
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
    CaptureSpec::new(
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
fn test_json_plan_owns_exact_image_and_applies_only_against_it() {
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
    assert_eq!(outcome.actions_applied, delta.actions().len());
}

#[test]
fn test_memory_plan_owns_exact_image_and_applies_only_against_it() {
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
    assert_eq!(outcome.actions_applied, delta.actions().len());
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
        .capture(CaptureSpec::new([path.clone()], budget()).unwrap())
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
fn test_same_storage_reuses_its_own_absent_root_publication_evidence() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join(".jit");
    let stale_layout = discover_repository_layout(temp.path(), &data).unwrap();
    let storage = JsonFileStorage::new(&data);
    storage.configure_repository_layout(&stale_layout);
    let path = VirtualPath::data("index.json").unwrap();
    let mut first = storage.open_mutation_session(stale_layout.clone()).unwrap();
    let image = first
        .capture(CaptureSpec::new([path.clone()], budget()).unwrap())
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

    assert!(matches!(error, RepositoryStateStoreError::UnsafeTarget(_)));
}

#[test]
fn test_open_regular_file_nofollow_refuses_a_directory() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join("subdir")).unwrap();
    let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();

    let error = open_regular_file_nofollow(&dir, "subdir").unwrap_err();

    assert!(matches!(error, RepositoryStateStoreError::UnsafeTarget(_)));
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
                content: CapturedBytes::new("memory-tool", b"tool".to_vec()).unwrap(),
                mode: FileMode::Regular,
            },
        );
    }
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let spec = CaptureSpec::new([VirtualPath::data("tool").unwrap()], budget()).unwrap();
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

// --- Cross-backend conformance matrix ------------------------------------
//
// One suite exercises identical canonical semantics on the JSON and memory
// backends: equivalent captured semantics, complete per-image result hashes,
// every action kind, nested and disjoint existing/absent roots, and aliases.
// Object identity differs by construction (device/inode versus a synthesized
// memory id), so parity is asserted over the identity-independent semantic
// projection.

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemanticEntry {
    Absent,
    Directory(FileMode),
    File(Vec<u8>, FileMode),
    FileTail(u64, Vec<u8>, FileMode),
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

fn read_events_for_log<S: IssueStore>(store: &S) -> Option<Vec<Event>> {
    store.read_events().ok()
}

#[test]
fn test_conformance_event_log_skips_retired_tags_and_rejects_malformed_known_records() {
    let known = |id: &str| Event::IssueCreated {
        id: id.to_string(),
        issue_id: "issue-id".to_string(),
        timestamp: chrono::Utc::now(),
        title: "Known".to_string(),
        priority: crate::domain::Priority::Normal,
    };
    let (first, second) = (known("first"), known("second"));
    let retired = concat!(
        "{\"type\":\"retired_",
        "event\",\"id\":\"historical\",\"timestamp\":\"2025-01-01T00:00:00Z\"}"
    );
    let line = |event: &Event| serde_json::to_string(event).unwrap();
    let logs = [
        format!(
            "{retired}\n{}\n{retired}\n{}\n",
            line(&first),
            line(&second)
        ),
        "{\"type\":\"issue_created\"}\n".to_string(),
        "{not-json}\n".to_string(),
    ];
    let expected = [Some(vec![first, second]), None, None];

    for (log, expected) in logs.iter().zip(expected) {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("events.jsonl"), log).unwrap();
        let json = JsonFileStorage::new(&data);
        json.configure_repository_layout(
            &discover_repository_layout(worktree.path(), &data).unwrap(),
        );

        let memory = InMemoryStorage::new();
        seed_memory_existing(&memory, &[(VirtualPath::EVENTS, log.as_bytes())]);

        assert_eq!(read_events_for_log(&json), expected, "json log {log:?}");
        assert_eq!(read_events_for_log(&memory), expected, "memory log {log:?}");
    }
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
    CaptureSpec::new(
        [
            VirtualPath::data("").unwrap(),
            VirtualPath::data("keep.txt").unwrap(),
            VirtualPath::data("replace.txt").unwrap(),
            VirtualPath::data("remove.txt").unwrap(),
            VirtualPath::data("chmod.txt").unwrap(),
            VirtualPath::data("generated").unwrap(),
            VirtualPath::data("generated/new.txt").unwrap(),
            VirtualPath::worktree("out.txt").unwrap(),
        ],
        budget(),
    )
    .unwrap()
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
    let spec = || {
        CaptureSpec::new(
            [
                VirtualPath::data("").unwrap(),
                VirtualPath::data("nested").unwrap(),
                nested.clone(),
            ],
            budget(),
        )
        .unwrap()
    };
    let delta = RepositoryDelta::new(
        &layout,
        vec![RepositoryAction::write_file(
            nested.clone(),
            "p0",
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

/// A session over `files` on each backend, with its [`Writer`].
fn sessions_over(files: &[(VirtualPath, &[u8])]) -> (TempDir, [WrittenSession; 2]) {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let resolve = layout.clone();
    let json: Writer =
        Box::new(move |path, bytes| std::fs::write(resolve.resolve(path).unwrap(), bytes).unwrap());
    files.iter().for_each(|(path, bytes)| json(path, bytes));
    let memory = InMemoryStorage::new();
    seed_memory_existing(&memory, files);
    let sessions = [
        JsonFileStorage::new(&data).open_mutation_session(layout.clone()),
        memory.open_mutation_session(layout),
    ]
    .map(Result::unwrap);
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
    let log = vec![b'x'; EVENT_LOG_TAIL_WINDOW as usize + 1];
    let (_worktree, sessions) = sessions_over(&[
        (kept.clone(), b"first"),
        (added.clone(), b"added"),
        (VirtualPath::EVENTS, &log),
    ]);
    for (mut session, write) in sessions {
        let mut spec = CaptureSpec::new([kept.clone()], budget()).unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        let image = session.capture(spec).unwrap();
        write(&kept, b"second");
        let mut spec = image.capture_spec().clone();
        spec.discover_paths([added.clone(), VirtualPath::EVENTS])
            .unwrap();

        let grown = session.extend(image, spec).unwrap();

        assert_eq!(grown.file_bytes(&kept).unwrap(), Some(&b"first"[..]));
        assert_eq!(grown.file_bytes(&added).unwrap(), Some(&b"added"[..]));
        assert_eq!(
            grown.file_bytes(&VirtualPath::EVENTS).unwrap(),
            Some(&log[..])
        );
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
            let path = VirtualPath::data(format!("issues/{name}")).unwrap();
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
    let spec = CaptureSpec::new(
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
    let spec = CaptureSpec::new([VirtualPath::data("x").unwrap()], budget()).unwrap();
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
    let mut issue = crate::domain::fixture_issue("Determinism".into(), "Body".into());
    issue.id = id.to_string();
    issue.assignee = assignee;
    issue.created_at = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    issue.updated_at = issue.created_at;
    issue
}

fn req01_map_issue(reverse: bool) -> Issue {
    let mut issue = crate::domain::fixture_issue("Canonical backend retry".into(), "Body".into());
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
                run_id: None,
            },
        ),
        (
            "zeta-gate",
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("agent:zeta".parse().unwrap()),
                updated_at: req01_instant(),
                run_id: None,
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
            content: CapturedBytes::new("mem-issue", issue_bytes).unwrap(),
            mode: FileMode::Regular,
        },
    );
    if !events.is_empty() {
        state.entries.insert(
            VirtualPath::data("events.jsonl").unwrap(),
            RepositoryEntry::File {
                content: CapturedBytes::new("mem-events", events.to_vec()).unwrap(),
                mode: FileMode::Regular,
            },
        );
    }
}

fn claim_spec(id: &str) -> CaptureSpec {
    CaptureSpec::new(
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
            RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
            other => panic!("expected issue file, got {other:?}"),
        };
        let events = match state
            .entries
            .get(&VirtualPath::data("events.jsonl").unwrap())
            .unwrap()
        {
            RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
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
        crate::domain::event_log::parse_known_events(std::str::from_utf8(&json_events).unwrap())
            .unwrap();
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
fn test_conformance_identical_full_finalizer_plan_matches_both_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let spec = CaptureSpec::new([VirtualPath::data("events.jsonl").unwrap()], budget()).unwrap();
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
    assert_eq!(json_outcome, memory_outcome);
}

#[test]
fn test_conformance_reconstructed_issue_maps_match_across_backends() {
    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir_all(data.join("issues")).unwrap();
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let issue_id = crate::repository_state::IdAuthority::from_seed([31u8; 32]).uuid_at(0);
    let spec = CaptureSpec::new(
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
    assert_eq!(json_plan, memory_plan);
    let json_outcome = json_session.apply(&json_plan).unwrap();
    let memory_outcome = memory_session.apply(&memory_plan).unwrap();
    assert_eq!(json_outcome, memory_outcome);
}

#[test]
fn test_full_finalizer_allocated_ids_enter_delta_and_retry_reuses_plan() {
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
    assert_eq!(first, retry, "a retry reuses the same plan");

    let event_bytes = first.delta().appended_event_bytes().unwrap();
    let events =
        crate::domain::event_log::parse_known_events(std::str::from_utf8(event_bytes).unwrap())
            .unwrap();
    let Event::IssueClaimed { id: event_id, .. } = &events[0] else {
        panic!("expected allocated claim event");
    };
    assert_eq!(
        event_id,
        &crate::repository_state::IdAuthority::from_seed([23u8; 32]).uuid_at(0)
    );
}

#[test]
fn test_conformance_empty_finalizer_noop_plan_matches_without_clock_sampling() {
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
    let spec = CaptureSpec::new([VirtualPath::data("events.jsonl").unwrap()], budget()).unwrap();
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
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
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
                RepositoryAction::append_events(
                    "provenance",
                    ExpectedPreimage::Absent,
                    0,
                    event_line.clone(),
                ),
            ],
        )
        .unwrap()
    };
    let spec = || {
        CaptureSpec::new(
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
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
        other => panic!("expected gate-run file, got {other:?}"),
    };
    let memory_events = match state.entries.get(&events).unwrap() {
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
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
            content: CapturedBytes::new(format!("mem-file:{path:?}"), bytes.to_vec()).unwrap(),
            mode: FileMode::Regular,
        },
    )
}

fn memory_bytes(memory: &InMemoryStorage, path: &VirtualPath) -> Vec<u8> {
    match memory.repository_state().entries.get(path).unwrap() {
        RepositoryEntry::File { content, .. } => content.bytes().to_vec(),
        other => panic!("expected a file at {path:?}, got {other:?}"),
    }
}

fn lifecycle_event() -> Event {
    Event::ProfileLifecycle {
        id: String::new(),
        timestamp: req01_instant(),
        operation: crate::domain::ProfileLifecycleOperation::Apply,
        profiles: vec![crate::domain::ProfileLifecycleProfile {
            id: "example"
                .try_into()
                .expect("the conformance profile id is canonical"),
            version: "1.0.0".to_string(),
            status: crate::domain::ProfileLifecycleStatus::Installed,
            variables: Default::default(),
        }],
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
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
    };
    let intents = || {
        [
            MutationIntent::RecordGateRun {
                draft: Box::new(draft.clone()),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(lifecycle_event()),
            },
        ]
    };
    let spec = || {
        CaptureSpec::new(
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

    let mut draft = crate::domain::fixture_issue("New".into(), "Body".into());
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
        CaptureSpec::new(
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
        crate::domain::event_log::parse_known_events(std::str::from_utf8(&json_events).unwrap())
            .unwrap();
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

#[cfg(unix)]
fn capture_git_fixture() -> (TempDir, PathBuf, PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;

    let worktree = TempDir::new().unwrap();
    let data = worktree.path().join(".jit");
    std::fs::create_dir(&data).unwrap();
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
    run_git(&["init"]);
    run_git(&["config", "user.name", "Test User"]);
    run_git(&["config", "user.email", "test@example.com"]);
    std::fs::create_dir_all(worktree.path().join("docs")).unwrap();
    std::fs::write(worktree.path().join("docs/a.md"), b"first a").unwrap();
    std::fs::write(worktree.path().join("docs/b.md"), b"first b").unwrap();
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
    let log = worktree.path().join("git-processes.log");
    let wrapper = worktree.path().join("git-wrapper.sh");
    let log_literal = log.to_string_lossy().replace('\'', "'\\''");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log_literal}'\nexec git \"$@\"\n"),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).unwrap();
    (worktree, data, wrapper, first)
}

#[cfg(unix)]
fn capture_process_log(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
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

#[cfg(unix)]
#[test]
fn test_json_capture_reuses_pinned_target_cache_for_unchanged_commit() {
    let (worktree, data, wrapper, _) = capture_git_fixture();
    let storage = JsonFileStorage::with_git_program(&data, wrapper);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let spec = pinned_capture_spec(["docs/a.md", "docs/b.md"]);

    session.capture(spec.clone()).unwrap();
    let first_capture_processes = capture_process_log(&worktree.path().join("git-processes.log"));
    assert_eq!(first_capture_processes.len(), 3);
    session.capture(spec).unwrap();
    let all_processes = capture_process_log(&worktree.path().join("git-processes.log"));
    assert_eq!(all_processes.len(), 4);
    assert_eq!(
        all_processes
            .iter()
            .filter(|line| line.starts_with("rev-parse "))
            .count(),
        2
    );
    assert_eq!(
        all_processes
            .iter()
            .filter(|line| line.contains(" ls-tree "))
            .count(),
        1
    );
    assert_eq!(
        all_processes
            .iter()
            .filter(|line| *line == "cat-file --batch")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn test_json_capture_re_resolves_moved_head_and_rekeys_pinned_cache() {
    let (worktree, data, wrapper, first) = capture_git_fixture();
    let storage = JsonFileStorage::with_git_program(&data, wrapper);
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
    let (worktree, data, wrapper, revision) = capture_git_fixture();
    let storage = JsonFileStorage::with_git_program(&data, wrapper);
    let layout = discover_repository_layout(worktree.path(), &data).unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let spec = pinned_capture_spec(["docs/a.md", "docs/b.md"]);
    let log = worktree.path().join("git-processes.log");

    session.capture(spec.clone()).unwrap();
    let after_first = capture_process_log(&log).len();
    session.capture(pinned_capture_spec([])).unwrap();
    assert_eq!(capture_process_log(&log).len(), after_first);
    assert_eq!(
        storage.pinned_target_cache_keys(),
        vec![
            (revision.clone(), "docs/a.md".to_string()),
            (revision, "docs/b.md".to_string()),
        ]
    );
    session.capture(spec).unwrap();
    let repeated = capture_process_log(&log)[after_first..].to_vec();
    assert!(
        repeated.len() == 1 && repeated[0].starts_with("rev-parse "),
        "a capture after one that pins nothing must reuse cached evidence, spawned {repeated:?}"
    );
}

#[test]
fn test_json_capture_retains_only_pairs_used_by_the_most_recent_capture() {
    let (worktree, data, wrapper, revision) = capture_git_fixture();
    let storage = JsonFileStorage::with_git_program(&data, wrapper);
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
        std::fs::create_dir_all(data.join("issues")).unwrap();
        for (relative, bytes) in files {
            std::fs::write(data.join(relative), bytes).unwrap();
        }
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let storage = JsonFileStorage::new(&data);
        storage.configure_repository_layout(&layout);
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
        capture_capability_image(
            &self.layout,
            &self.roots,
            &self.storage,
            CaptureSpec::new(paths, budget()).unwrap(),
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

fn data_path(relative: &str) -> VirtualPath {
    VirtualPath::data(relative).unwrap()
}

fn put_file(path: VirtualPath, bytes: &[u8]) -> PutFile {
    PutFile {
        path,
        bytes: bytes.to_vec(),
        mode: FileMode::Regular,
    }
}

/// An append at `offset`, the log length the mutation read.
fn event_append(offset: u64) -> Option<EventAppend> {
    Some(EventAppend {
        offset,
        bytes: b"{\"tag\":\"issue_created\"}\n".to_vec(),
    })
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
        ("issues/one.json", b"{\"id\":\"one\"}"),
        ("issues/stale.json", b"{\"id\":\"stale\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let written = data_path("issues/one.json");
    let removed = data_path("issues/stale.json");
    let image = fixture.capture([written.clone(), removed.clone(), VirtualPath::EVENTS]);

    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(written, b"{\"id\":\"one\",\"state\":\"ready\"}")],
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
            RepositoryAction::create_directory(empty.clone(), "p0", ExpectedPreimage::Absent),
            RepositoryAction::create_directory(ancestor, "p0", ExpectedPreimage::Absent),
            RepositoryAction::write_file(
                written,
                "p0",
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
            "p0",
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
        ("issues/one.json", b"{\"id\":\"one\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let written = data_path("issues/one.json");
    let image = fixture.capture([written.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(
            written.clone(),
            b"{\"id\":\"one\",\"state\":\"ready\"}",
        )],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("issues/one.json", b"{\"id\":\"one\",\"raced\":true}");

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &written),
        "the conflict must name the write target that moved, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_refuses_a_delete_target_whose_bytes_changed() {
    let fixture = WriteSetFixture::new(&[
        ("issues/stale.json", b"{\"id\":\"stale\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let removed = data_path("issues/stale.json");
    let image = fixture.capture([removed.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        Vec::new(),
        vec![removed.clone()],
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("issues/stale.json", b"{\"id\":\"stale\",\"raced\":true}");

    let error = fixture.check(&image, &record).unwrap_err();
    assert!(
        names_moved_path(&error, &removed),
        "the conflict must name the delete target that moved, got {error:?}"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_a_read_but_unwritten_file_that_changed() {
    let fixture = WriteSetFixture::new(&[
        ("index.json", b"{\"issues\":[]}"),
        ("issues/one.json", b"{\"id\":\"one\"}"),
        ("events.jsonl", seeded_log()),
    ]);
    let read_only = data_path("index.json");
    let written = data_path("issues/one.json");
    let image = fixture.capture([read_only, written.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(written, b"{\"id\":\"one\",\"state\":\"ready\"}")],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("index.json", b"{\"issues\":[\"raced\"]}");

    assert!(
        fixture.check(&image, &record).is_ok(),
        "a file the mutation read but does not write stays out of the comparison"
    );
}

#[test]
fn test_ensure_write_set_unchanged_admits_a_target_expected_absent_that_is_still_absent() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let created = data_path("issues/new.json");
    let image = fixture.capture([created.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(created, b"{\"id\":\"new\"}")],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    assert!(fixture.check(&image, &record).is_ok());
}

#[test]
fn test_ensure_write_set_unchanged_refuses_a_target_expected_absent_that_became_occupied() {
    let fixture = WriteSetFixture::new(&[("events.jsonl", seeded_log())]);
    let created = data_path("issues/new.json");
    let image = fixture.capture([created.clone(), VirtualPath::EVENTS]);
    let record = RedoRecord::new(
        Vec::new(),
        vec![put_file(created.clone(), b"{\"id\":\"new\"}")],
        Vec::new(),
        event_append(seeded_log().len() as u64),
    )
    .unwrap();

    fixture.write("issues/new.json", b"{\"id\":\"new\",\"raced\":true}");

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
    let line = b"{\"tag\":\"issue_created\",\"id\":\"padding-padding-padding\"}\n";
    line.iter()
        .copied()
        .cycle()
        .take((EVENT_LOG_TAIL_WINDOW as usize) + line.len() * 2)
        .collect()
}

/// One contract, run on both backends, because applying a leftover record is
/// the session's behaviour rather than either backend's
/// (`@/inv/shared-test-contracts`).
trait SessionRecordBackend: Sized {
    /// What a record publishes, compared between two opens.
    type Published: std::fmt::Debug + PartialEq;

    /// Its event log holds [`CAPTURED_LOG`], the offset [`leftover_record`]
    /// appends at.
    fn seeded() -> Self;
    fn commit(&self, record: &RedoRecord);
    fn open(&self) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError>;
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

struct FileSessionBackend {
    worktree: TempDir,
    storage: JsonFileStorage,
    layout: RepositoryLayout,
}

impl FileSessionBackend {
    fn record_path(&self) -> PathBuf {
        self.worktree.path().join(record_relative_path())
    }

    fn scratch_path(&self) -> PathBuf {
        redo_root(&self.worktree).join(RECORD_SCRATCH_DIR)
    }
}

impl SessionRecordBackend for FileSessionBackend {
    type Published = BTreeMap<PathBuf, SnapshotEntry>;

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

    fn commit(&self, record: &RedoRecord) {
        record.commit(&redo_root(&self.worktree)).unwrap();
    }

    fn open(&self) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        self.storage.open_mutation_session(self.layout.clone())
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

struct MemorySessionBackend(InMemoryStorage);

impl SessionRecordBackend for MemorySessionBackend {
    type Published = BTreeMap<VirtualPath, RepositoryEntry>;

    fn seeded() -> Self {
        Self(memory_fixtures::seeded(&[], false))
    }

    fn commit(&self, record: &RedoRecord) {
        self.0.commit_record(record).unwrap();
    }

    fn open(&self) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        self.0.open_mutation_session(self.0.repository_layout())
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
struct InterruptAtMoment(MutationPoint);

impl InterruptAtMoment {
    fn new(point: MutationPoint) -> Arc<Self> {
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

/// One data file published and one event appended at the log length the
/// interrupted mutation captured.
fn leftover_record() -> RedoRecord {
    RedoRecord::new(
        Vec::new(),
        vec![put_file(leftover_target(), LEFTOVER_BYTES)],
        Vec::new(),
        event_append(CAPTURED_LOG.len() as u64),
    )
    .unwrap()
}

fn leftover_target() -> VirtualPath {
    data_path("issues/left.json")
}

const LEFTOVER_BYTES: &[u8] = b"{\"id\":\"left\"}";

fn appended_log() -> Vec<u8> {
    [CAPTURED_LOG, &event_append(0).unwrap().bytes].concat()
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
        let image = session
            .capture(CaptureSpec::new([target.clone()], budget()).unwrap())
            .unwrap();
        let delta = RepositoryDelta::new(
            session.layout(),
            vec![RepositoryAction::write_file(
                target.clone(),
                "interrupted-publication",
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

    let next = JsonFileStorage::new(&data);
    let session = next
        .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
        .unwrap();

    assert_eq!(session.recovery_report().applied_records, 1);
    assert_eq!(std::fs::read(&note).unwrap(), b"note");
}
