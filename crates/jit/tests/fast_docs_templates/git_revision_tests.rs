use jit::storage::{GitRevisionError, GitRevisionResolver};
use jit::test_cli::init_git_repository;
#[cfg(unix)]
use jit::test_harness::git::GitStandIn;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

struct Repo {
    _temp: TempDir,
    root: std::path::PathBuf,
}

impl Repo {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        init_git_repository(&root).unwrap();
        Self { _temp: temp, root }
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-m", message]);
        git_output(&self.root, &["rev-parse", "HEAD"])
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn test_git_revision_resolver_canonicalizes_symbolic_abbreviated_and_full_revisions() {
    let repo = Repo::new();
    repo.write("docs/history.bin", b"historical\0bytes");
    let oid = repo.commit("history");
    let abbreviated = &oid[..8];
    let resolver = GitRevisionResolver::new(&repo.root);

    for revision in ["HEAD", abbreviated, oid.as_str()] {
        let resolved = resolver.resolve_commit(revision).unwrap();
        assert_eq!(resolved.as_str(), oid);
        let read = resolver
            .read_pinned_path(revision, "docs/history.bin")
            .unwrap();
        assert_eq!(read.version().as_str(), oid);
    }
}

#[test]
fn test_git_revision_resolver_never_falls_back_on_invalid_revision_or_failed_blob_read() {
    let repo = Repo::new();
    repo.write("docs/working.md", b"working tree content");
    let empty_oid = repo.commit("working file exists");
    fs::remove_file(repo.root.join("docs/working.md")).unwrap();
    repo.commit("remove working file");
    repo.write(
        "docs/working.md",
        b"new uncommitted working tree fallback bait",
    );
    let resolver = GitRevisionResolver::new(&repo.root);

    assert!(matches!(
        resolver.read_pinned_path("not-a-revision", "docs/working.md"),
        Err(GitRevisionError::RevisionNotFound { .. })
    ));
    assert!(matches!(
        resolver.read_pinned_path("HEAD", "docs/working.md"),
        Err(GitRevisionError::PinnedReadFailed { .. })
    ));

    let historical = resolver
        .read_pinned_path(&empty_oid, "docs/working.md")
        .unwrap();
    assert_eq!(historical.bytes(), b"working tree content");
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_classifies_tree_modes_without_worktree_fallback() {
    use jit::repository_state::RepositoryTargetKind;
    use std::os::unix::fs::symlink;

    let repo = Repo::new();
    repo.write("docs/file.md", b"pinned regular file");
    repo.write("docs/section/.keep", b"pinned directory");
    symlink("file.md", repo.root.join("docs/link.md")).unwrap();
    let revision = repo.commit("add typed pinned targets");

    fs::remove_file(repo.root.join("docs/file.md")).unwrap();
    fs::remove_dir_all(repo.root.join("docs/section")).unwrap();
    fs::remove_file(repo.root.join("docs/link.md")).unwrap();
    repo.write("docs/link.md", b"working tree fallback bait");

    let resolver = GitRevisionResolver::new(&repo.root);
    let inspect = |path: &str| {
        let request = (revision.clone(), path.to_string());
        resolver
            .inspect_pinned_targets(
                &[request.clone()].into_iter().collect(),
                &BTreeSet::new(),
                &mut BTreeMap::new(),
            )
            .remove(&request)
            .unwrap()
            .unwrap()
    };
    let regular = inspect("docs/file.md");
    let directory = inspect("docs/section");
    let symlink = inspect("docs/link.md");
    let missing = inspect("docs/missing.md");

    assert_eq!(regular.target_kind(), RepositoryTargetKind::File);
    assert_eq!(regular.bytes(), Some(b"pinned regular file".as_slice()));
    assert_eq!(directory.target_kind(), RepositoryTargetKind::Directory);
    assert_eq!(directory.bytes(), None);
    assert_eq!(symlink.target_kind(), RepositoryTargetKind::Unsupported);
    assert_eq!(symlink.bytes(), None);
    assert_eq!(missing.target_kind(), RepositoryTargetKind::Missing);
    assert_eq!(missing.bytes(), None);
}

#[test]
fn test_git_revision_resolver_reports_unavailable_git() {
    let repo = Repo::new();
    let resolver =
        GitRevisionResolver::with_git_program(&repo.root, repo.root.join("definitely-no-git-here"));

    assert!(matches!(
        resolver.resolve_commit("HEAD"),
        Err(GitRevisionError::GitUnavailable { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_classifies_every_target_class() {
    use jit::repository_state::RepositoryTargetKind;
    use std::os::unix::fs::PermissionsExt;

    let repo = Repo::new();
    repo.write("docs/file.md", b"first version");
    repo.write("docs/executable.sh", b"#!/bin/sh\nexit 0\n");
    fs::set_permissions(
        repo.root.join("docs/executable.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    repo.write("docs/section/.keep", b"directory");
    repo.write("docs/space name.md", b"space\tpath\0\n");
    repo.write("docs/[x].md", b"glob characters");
    std::os::unix::fs::symlink("file.md", repo.root.join("docs/link.md")).unwrap();
    let first = repo.commit("first");
    repo.write("docs/file.md", b"second version");
    repo.commit("second");
    let abbreviated = first[..8].to_string();
    let range = format!("{first}..HEAD");
    let requests = [
        ("HEAD", "docs/file.md"),
        ("HEAD", "docs/executable.sh"),
        ("HEAD", "docs/section"),
        ("HEAD", "docs/link.md"),
        ("HEAD", "docs/missing.md"),
        ("HEAD", "docs/file.md/x"),
        ("HEAD", "docs/space name.md"),
        ("HEAD", "docs"),
        ("HEAD", "docs/[x].md"),
        ("HEAD", "docs/*.md"),
        ("HEAD", ":(glob)docs/*.md"),
        (first.as_str(), "docs/file.md"),
        (abbreviated.as_str(), "docs/file.md"),
        (range.as_str(), "docs/file.md"),
        ("not-a-revision", "docs/file.md"),
    ]
    .into_iter()
    .map(|(revision, path)| (revision.to_string(), path.to_string()))
    .collect::<std::collections::BTreeSet<_>>();

    let stand_in = GitStandIn::new();
    let batched = GitRevisionResolver::with_git_program(&repo.root, stand_in.program());
    let mut cache = BTreeMap::new();
    let actual = batched.inspect_pinned_targets(&requests, &BTreeSet::new(), &mut cache);

    assert_eq!(actual.len(), requests.len());
    let expected_kinds = [
        ("HEAD", "docs/file.md", Some(RepositoryTargetKind::File)),
        (
            "HEAD",
            "docs/executable.sh",
            Some(RepositoryTargetKind::File),
        ),
        (
            "HEAD",
            "docs/section",
            Some(RepositoryTargetKind::Directory),
        ),
        (
            "HEAD",
            "docs/link.md",
            Some(RepositoryTargetKind::Unsupported),
        ),
        (
            "HEAD",
            "docs/missing.md",
            Some(RepositoryTargetKind::Missing),
        ),
        (
            "HEAD",
            "docs/file.md/x",
            Some(RepositoryTargetKind::Missing),
        ),
        (
            "HEAD",
            "docs/space name.md",
            Some(RepositoryTargetKind::File),
        ),
        ("HEAD", "docs", Some(RepositoryTargetKind::Directory)),
        ("HEAD", "docs/[x].md", Some(RepositoryTargetKind::File)),
        ("HEAD", "docs/*.md", Some(RepositoryTargetKind::Missing)),
        (
            "HEAD",
            ":(glob)docs/*.md",
            Some(RepositoryTargetKind::Missing),
        ),
        (
            first.as_str(),
            "docs/file.md",
            Some(RepositoryTargetKind::File),
        ),
        (
            abbreviated.as_str(),
            "docs/file.md",
            Some(RepositoryTargetKind::File),
        ),
        (range.as_str(), "docs/file.md", None),
        ("not-a-revision", "docs/file.md", None),
    ];
    assert_eq!(expected_kinds.len(), requests.len());
    expected_kinds.iter().for_each(|(revision, path, kind)| {
        let request = (revision.to_string(), path.to_string());
        let result = actual.get(&request).expect("every request has a result");
        assert_eq!(
            result.as_ref().ok().map(|read| read.target_kind()),
            *kind,
            "{request:?}"
        );
    });
    assert!(matches!(
        actual[&("not-a-revision".to_string(), "docs/file.md".to_string())],
        Err(GitRevisionError::RevisionNotFound { .. })
    ));
    assert_eq!(
        actual[&(first.clone(), "docs/file.md".to_string())]
            .as_ref()
            .unwrap()
            .bytes(),
        Some(b"first version".as_slice())
    );
    assert_eq!(
        actual[&("HEAD".to_string(), "docs/file.md".to_string())]
            .as_ref()
            .unwrap()
            .bytes(),
        Some(b"second version".as_slice())
    );
    assert_eq!(
        actual[&("HEAD".to_string(), "docs/file.md/x".to_string())]
            .as_ref()
            .unwrap()
            .unavailable_reason(),
        Some("not found in commit tree")
    );
    assert!(stand_in.invocations().iter().any(
        |invocation| invocation.args() == ["rev-parse", "--verify", "not-a-revision^{commit}"]
    ));
    assert_eq!(cache.len(), 12);
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_scales_one_revision_independently_of_reference_count() {
    let repo = Repo::new();
    (0..40).for_each(|index| {
        repo.write(
            &format!("docs/file-{index}.md"),
            format!("file-{index}").as_bytes(),
        );
    });
    let revision = repo.commit("many files");
    let requests = (0..40)
        .map(|index| ("HEAD".to_string(), format!("docs/file-{index}.md")))
        .collect::<std::collections::BTreeSet<_>>();
    let stand_in = GitStandIn::new();
    let resolver = GitRevisionResolver::with_git_program(&repo.root, stand_in.program());
    let mut cache = BTreeMap::new();
    resolver.inspect_pinned_targets(
        &[("HEAD".to_string(), "docs/file-0.md".to_string())]
            .into_iter()
            .collect(),
        &BTreeSet::new(),
        &mut cache,
    );
    assert_eq!(stand_in.invocations().len(), 3);
    resolver.inspect_pinned_targets(&requests, &BTreeSet::new(), &mut cache);

    let processes = stand_in.invocations();
    assert_eq!(processes.len(), 6);
    assert!(processes.iter().any(|invocation| invocation
        .args()
        .first()
        .is_some_and(|argument| argument == "rev-parse")));
    let listing = processes
        .iter()
        .skip(3)
        .filter(|invocation| {
            invocation
                .args()
                .iter()
                .any(|argument| argument == "ls-tree")
        })
        .collect::<Vec<_>>();
    assert_eq!(listing.len(), 1);
    assert!(listing[0]
        .args()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .starts_with(&[
            "--literal-pathspecs",
            "ls-tree",
            "-t",
            "-z",
            "--full-tree",
            revision.as_str(),
            "--",
        ]));
    assert!((1..40).all(|index| listing[0].args().contains(&format!("docs/file-{index}.md"))));
    assert!(processes
        .iter()
        .any(|invocation| invocation.args() == ["cat-file", "--batch"]));
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_lists_only_requested_paths_of_a_commit_tree() {
    let repo = Repo::new();
    (0..40).for_each(|index| {
        repo.write(&format!("other/file-{index}.md"), b"unrequested");
    });
    repo.write("docs/nested/file.md", b"requested");
    repo.commit("wide tree");
    let requests = [("HEAD", "docs"), ("HEAD", "docs/nested/file.md")]
        .into_iter()
        .map(|(revision, path)| (revision.to_string(), path.to_string()))
        .collect::<std::collections::BTreeSet<_>>();
    let stand_in = GitStandIn::new();
    let resolver = GitRevisionResolver::with_git_program(&repo.root, stand_in.program());

    let results =
        resolver.inspect_pinned_targets(&requests, &BTreeSet::new(), &mut BTreeMap::new());

    assert!(results.values().all(Result::is_ok));
    let listings = stand_in
        .invocations()
        .into_iter()
        .filter(|invocation| {
            invocation
                .args()
                .iter()
                .any(|argument| argument == "ls-tree")
        })
        .collect::<Vec<_>>();
    assert_eq!(listings.len(), 1);
    assert!(
        !listings[0].args().iter().any(|argument| argument == "-r"),
        "pinned classification listed the commit tree recursively: {:?}",
        listings[0].args()
    );
    assert!(!listings[0]
        .args()
        .iter()
        .any(|argument| argument.contains("other")));
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_resolves_distinct_revisions_in_one_success_process() {
    let repo = Repo::new();
    repo.write("docs/file.md", b"first");
    let first = repo.commit("first");
    repo.write("docs/file.md", b"second");
    let second = repo.commit("second");
    let requests = [
        ("HEAD".to_string(), "docs/file.md".to_string()),
        (first.clone(), "docs/file.md".to_string()),
        (second[..8].to_string(), "docs/file.md".to_string()),
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    let stand_in = GitStandIn::new();
    let resolver = GitRevisionResolver::with_git_program(&repo.root, stand_in.program());
    let mut cache = BTreeMap::new();
    let results = resolver.inspect_pinned_targets(&requests, &BTreeSet::new(), &mut cache);

    assert!(results.values().all(Result::is_ok));
    let processes = stand_in.invocations();
    assert_eq!(
        processes
            .iter()
            .filter(|invocation| invocation
                .args()
                .first()
                .is_some_and(|argument| argument == "rev-parse"))
            .count(),
        1
    );
    assert_eq!(
        processes
            .iter()
            .filter(|invocation| invocation
                .args()
                .iter()
                .any(|argument| argument == "ls-tree"))
            .count(),
        2
    );
    assert_eq!(
        processes
            .iter()
            .filter(|invocation| invocation.args() == ["cat-file", "--batch"])
            .count(),
        1
    );
}

#[test]
fn test_inspect_pinned_targets_reports_git_unavailable_for_every_request() {
    let repo = Repo::new();
    repo.write("docs/file.md", b"content");
    repo.commit("content");
    let missing_git = repo.root.join("definitely-no-git-here");
    let resolver = GitRevisionResolver::with_git_program(&repo.root, &missing_git);
    let requests = [
        ("HEAD".to_string(), "docs/file.md".to_string()),
        ("HEAD".to_string(), "docs/other.md".to_string()),
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    let mut cache = BTreeMap::new();
    let actual = resolver.inspect_pinned_targets(&requests, &BTreeSet::new(), &mut cache);

    assert_eq!(actual.len(), requests.len());
    requests.iter().for_each(|request| {
        let error = actual.get(request).unwrap().as_ref().unwrap_err();
        assert!(matches!(error, GitRevisionError::GitUnavailable { .. }));
    });
}
