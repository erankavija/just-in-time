//! Per-document archival integrity (jit:f067678a).
//!
//! `archive_document` is reference-aware and transactional: archiving a file
//! relocates it to the archive root AND re-links every issue doc-reference that
//! points at it, in one operation; archiving a MISSING source is a typed no-op
//! error that makes no filesystem or `.jit` change.
//!
//! These are in-process tests over a real `JsonFileStorage` rooted at
//! `<tmp>/.jit`, with the document files at the REPO-ROOT-relative `dev/active/`
//! (the parent of `.jit`). No subprocess and no git are required.

use jit::commands::{ArchiveError, CommandExecutor};
use jit::domain::{DocumentReference, Issue, State};
use jit::storage::{IssueStore, JsonFileStorage};
use tempfile::TempDir;

/// A `[documentation]` config mapping the `design` category to the `features`
/// archive subdirectory, with `dev/active` managed and `docs/` permanent.
const DOC_CONFIG: &str = r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#;

/// Build a `JsonFileStorage`-backed executor rooted at `<tmp>/.jit`, with the
/// documentation config written into `.jit/config.toml`. Returns the temp dir
/// (kept alive = the repo root) and the executor.
fn executor() -> (TempDir, CommandExecutor<JsonFileStorage>) {
    std::env::set_var("JIT_TEST_MODE", "1");
    let repo_root = TempDir::new().unwrap();
    let jit_dir = repo_root.path().join(".jit");
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    std::fs::write(jit_dir.join("config.toml"), DOC_CONFIG).unwrap();
    (repo_root, CommandExecutor::new(storage))
}

/// Seed a `Done` issue carrying a single `Design`-labeled doc reference to
/// `doc_path`, returning its id. `Done` is terminal, so archival is allowed
/// without `--force`.
fn seed_issue_referencing(
    executor: &CommandExecutor<JsonFileStorage>,
    title: &str,
    doc_path: &str,
) -> String {
    let mut issue = Issue::new(title.to_string(), String::new());
    issue.state = State::Done;
    issue.documents.push(DocumentReference {
        path: doc_path.to_string(),
        commit: None,
        label: Some("Design".to_string()),
        doc_type: None,
        format: None,
        assets: Vec::new(),
    });
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();
    id
}

/// The single doc-reference path currently recorded on the issue.
fn referenced_path(executor: &CommandExecutor<JsonFileStorage>, id: &str) -> String {
    executor.storage().load_issue(id).unwrap().documents[0]
        .path
        .clone()
}

#[test]
fn test_archive_relinks_all_referencing_issues() {
    // Happy path: a document referenced by N issues is relocated AND all N
    // references are re-linked to the archive path in one operation.
    let (repo_root, executor) = executor();

    let src = "dev/active/spec.md";
    std::fs::create_dir_all(repo_root.path().join("dev/active")).unwrap();
    std::fs::write(
        repo_root.path().join(src),
        "# Spec\n\nNo assets, just prose.\n",
    )
    .unwrap();

    let a = seed_issue_referencing(&executor, "alpha", src);
    let b = seed_issue_referencing(&executor, "beta", src);
    let c = seed_issue_referencing(&executor, "gamma", src);

    let (result, _warnings) = executor
        .archive_document(src, "design", false, false)
        .expect("archiving a present, terminally-linked document succeeds");

    let dest = "dev/archive/features/spec.md";
    assert_eq!(result.dest_path, dest);
    assert!(
        repo_root.path().join(dest).exists(),
        "document relocated to the archive root"
    );
    assert!(
        !repo_root.path().join(src).exists(),
        "source removed after the move"
    );

    // Every referencing issue is re-linked to the new path (all N, not just one).
    for id in [&a, &b, &c] {
        assert_eq!(
            referenced_path(&executor, id),
            dest,
            "issue {id} re-linked to the archived path"
        );
    }
    assert_eq!(
        result.updated_issues.len(),
        3,
        "the result reports every re-linked issue"
    );
}

#[test]
fn test_archive_missing_source_is_typed_noop() {
    // A missing source must fail with the typed `ArchiveError::SourceMissing` and
    // make NO filesystem or `.jit` change: the reference stays put and no archive
    // file is created. (Never leave or create a dangling reference.)
    let (repo_root, executor) = executor();

    let ghost = "dev/active/ghost.md"; // referenced but never written to disk
    let id = seed_issue_referencing(&executor, "ghost-ref", ghost);

    let err = executor
        .archive_document(ghost, "design", false, false)
        .expect_err("archiving a missing source must error");

    match err.downcast_ref::<ArchiveError>() {
        Some(ArchiveError::SourceMissing { path }) => assert_eq!(path, ghost),
        other => panic!("expected ArchiveError::SourceMissing, got {other:?}"),
    }

    // No-op: the reference is unchanged and nothing was written to the archive.
    assert_eq!(
        referenced_path(&executor, &id),
        ghost,
        "the dangling-source reference is left untouched"
    );
    assert!(
        !repo_root
            .path()
            .join("dev/archive/features/ghost.md")
            .exists(),
        "no archive file is created when the source is missing"
    );
}

/// Consistency guarantee under a mid-operation failure: if re-linking the
/// references fails after the document has been copied to the archive, the
/// archive must roll back cleanly. We force the failure by making `.jit/issues`
/// read-only so `save_issue` cannot persist the re-linked reference; the copy
/// (to `dev/archive`) and the event lock (in `.jit/`) are unaffected, so the
/// failure lands precisely on the relink step.
///
/// Asserts: (a) the operation errors, (b) the SOURCE file still exists, and
/// (c) NO issue reference was changed — i.e. nothing dangles. The just-created
/// destination copy is rolled back too.
#[cfg(unix)]
#[test]
fn test_archive_relink_failure_leaves_consistent_state() {
    use std::os::unix::fs::PermissionsExt;

    let (repo_root, executor) = executor();

    let src = "dev/active/spec.md";
    std::fs::create_dir_all(repo_root.path().join("dev/active")).unwrap();
    std::fs::write(repo_root.path().join(src), "# Spec\n\nProse only.\n").unwrap();

    let id = seed_issue_referencing(&executor, "alpha", src);

    // Make issue persistence fail: a read-only `.jit/issues` directory blocks
    // the temp-file write inside `save_issue`. (Reads still succeed because the
    // issue's `.lock` file already exists from seeding.)
    let issues_dir = repo_root.path().join(".jit/issues");
    let original_perms = std::fs::metadata(&issues_dir).unwrap().permissions();
    std::fs::set_permissions(&issues_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let result = executor.archive_document(src, "design", false, false);

    // Restore write access before any further storage read or temp-dir cleanup.
    std::fs::set_permissions(&issues_dir, original_perms).unwrap();

    // (a) The operation errored.
    assert!(
        result.is_err(),
        "a relink/persist failure during archive must surface as an error"
    );

    // (b) The source document is left in place — the move was never committed.
    assert!(
        repo_root.path().join(src).exists(),
        "the source document is left in place on relink failure"
    );

    // (c) No reference was changed: the issue still points at the source, so
    // there is no dangling reference.
    assert_eq!(
        referenced_path(&executor, &id),
        src,
        "no issue reference is changed when the relink fails"
    );

    // The just-created destination copy was rolled back.
    assert!(
        !repo_root
            .path()
            .join("dev/archive/features/spec.md")
            .exists(),
        "the destination copy is removed on rollback"
    );
}

/// Assert the referential-consistency invariant: every issue's recorded
/// doc-reference resolves to a file that exists on disk (nothing dangles).
fn assert_no_dangling_reference(
    repo_root: &TempDir,
    executor: &CommandExecutor<JsonFileStorage>,
    ids: &[&String],
) {
    for id in ids {
        let referenced = referenced_path(executor, id);
        assert!(
            repo_root.path().join(&referenced).exists(),
            "issue {id} references {referenced}, which must resolve to an existing file"
        );
    }
}

#[test]
fn test_archive_event_failure_after_relink_leaves_no_dangling_reference() {
    // The references are re-linked successfully, then the archive event fails to
    // persist. The rollback must restore every reference to the still-present
    // source so nothing dangles. We force the event failure by replacing
    // `.jit/events.jsonl` with a directory, which `append_event` cannot open for
    // appending; the issue saves (relink + rollback) are unaffected.
    let (repo_root, executor) = executor();

    let src = "dev/active/spec.md";
    std::fs::create_dir_all(repo_root.path().join("dev/active")).unwrap();
    std::fs::write(repo_root.path().join(src), "# Spec\n\nProse only.\n").unwrap();

    let a = seed_issue_referencing(&executor, "alpha", src);
    let b = seed_issue_referencing(&executor, "beta", src);

    // Make event persistence fail: a directory where the event log file belongs.
    let events_path = repo_root.path().join(".jit/events.jsonl");
    let _ = std::fs::remove_file(&events_path);
    std::fs::create_dir(&events_path).unwrap();

    let result = executor.archive_document(src, "design", false, false);

    assert!(
        result.is_err(),
        "an event-append failure after relink must surface as an error"
    );

    // Invariant: every reference resolves to an existing file. The rollback
    // restored both references to the source.
    assert_no_dangling_reference(&repo_root, &executor, &[&a, &b]);
    assert!(
        repo_root.path().join(src).exists(),
        "the source is left in place when the event append fails"
    );
    assert_eq!(
        referenced_path(&executor, &a),
        src,
        "references are restored to the source on rollback"
    );

    // The destination copy was removed once the rollback confirmed no reference
    // points at it.
    assert!(
        !repo_root
            .path()
            .join("dev/archive/features/spec.md")
            .exists(),
        "the destination copy is removed after a confirmed rollback"
    );
}

#[cfg(unix)]
#[test]
fn test_archive_partial_relink_failure_leaves_no_dangling_reference() {
    // Three issues reference the document. The first relink succeeds, then a
    // later one fails mid-batch. The rollback must leave every reference
    // resolving to an existing file. We force the second issue's `save_issue` to
    // fail by pre-occupying its atomic-write temp path (`{id}.json.tmp`) with a
    // directory: reads of `{id}.json` still succeed (so the issue is relinked),
    // but the temp-file write during save fails.
    let (repo_root, executor) = executor();

    let src = "dev/active/spec.md";
    std::fs::create_dir_all(repo_root.path().join("dev/active")).unwrap();
    std::fs::write(repo_root.path().join(src), "# Spec\n\nProse only.\n").unwrap();

    // Seed order fixes the relink order (index push order), so `beta` is the
    // second issue processed and the one we make fail.
    let a = seed_issue_referencing(&executor, "alpha", src);
    let b = seed_issue_referencing(&executor, "beta", src);
    let c = seed_issue_referencing(&executor, "gamma", src);

    // Block `beta`'s save: a directory at its atomic-write temp path.
    let beta_tmp = repo_root.path().join(format!(".jit/issues/{b}.json.tmp"));
    std::fs::create_dir(&beta_tmp).unwrap();

    let result = executor.archive_document(src, "design", false, false);

    // Remove the injected blocker so subsequent saves/reads are unaffected.
    std::fs::remove_dir(&beta_tmp).unwrap();

    assert!(
        result.is_err(),
        "a mid-batch relink failure must surface as an error"
    );

    // Invariant: every reference resolves to an existing file, regardless of how
    // far the partial relink got. The rollback restored the already-relinked
    // `alpha` to the source; `beta` and `gamma` were never moved.
    assert_no_dangling_reference(&repo_root, &executor, &[&a, &b, &c]);
    for id in [&a, &b, &c] {
        assert_eq!(
            referenced_path(&executor, id),
            src,
            "issue {id} is restored to / left at the source on rollback"
        );
    }
    assert!(
        repo_root.path().join(src).exists(),
        "the source is left in place when the relink fails partway"
    );
    assert!(
        !repo_root
            .path()
            .join("dev/archive/features/spec.md")
            .exists(),
        "the destination copy is removed after a confirmed rollback"
    );
}
