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
