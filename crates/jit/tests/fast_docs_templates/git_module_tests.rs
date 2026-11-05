use jit::storage::git::{Git, GitCheckout, GitError, ProbedCheckout};
use jit::test_harness::git::GitRepository;
#[cfg(unix)]
use jit::test_harness::git::GitStandIn;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn canonical(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap()
}

fn probe(dir: &Path) -> Option<ProbedCheckout> {
    Git::default().probe(dir).unwrap()
}

fn checkout(git: Git, root: &Path) -> GitCheckout {
    GitCheckout::new(git, root.to_path_buf())
}

#[cfg(unix)]
fn assert_one_head_lookup(stand_in: &jit::test_harness::git::GitStandIn, root: &Path) {
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(root));
    assert_eq!(
        invocations[0].args(),
        ["rev-parse", "HEAD", "--abbrev-ref", "HEAD"]
    );
}

fn committed_repository(dir: &TempDir) -> GitRepository {
    let repository = GitRepository::init(dir.path().join("primary"));
    repository.write("README.md", "readme");
    repository.commit("first");
    repository
}

#[test]
fn test_probe_primary_checkout_yields_common_dir_and_top_level() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);

    let probed = probe(repository.root()).unwrap();

    assert_eq!(
        probed.common_dir,
        canonical(&repository.root().join(".git"))
    );
    assert_eq!(probed.worktree_root, canonical(repository.root()));
}

#[test]
fn test_probe_subdirectory_resolves_relative_common_dir_against_probed_directory() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let nested = repository.root().join("a/b");
    std::fs::create_dir_all(&nested).unwrap();

    let probed = probe(&nested).unwrap();

    assert_eq!(
        probed.common_dir,
        canonical(&repository.root().join(".git"))
    );
    assert_eq!(probed.worktree_root, canonical(repository.root()));
}

#[test]
fn test_probe_linked_checkout_yields_shared_common_dir_and_linked_top_level() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let linked = dir.path().join("linked");
    repository.git(&["worktree", "add", "--quiet", linked.to_str().unwrap()]);

    let probed = probe(&linked).unwrap();

    assert_eq!(
        probed.common_dir,
        canonical(&repository.root().join(".git"))
    );
    assert_eq!(probed.worktree_root, canonical(&linked));
}

#[test]
fn test_probe_yields_each_checkouts_own_git_directory() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let linked = dir.path().join("linked");
    repository.git(&["worktree", "add", "--quiet", linked.to_str().unwrap()]);

    let primary = probe(repository.root()).unwrap();
    let linked = probe(&linked).unwrap();

    assert_eq!(primary.git_dir, primary.common_dir);
    assert_ne!(linked.git_dir, linked.common_dir);
    assert!(linked.git_dir.starts_with(&linked.common_dir));
}

#[test]
fn test_probe_repository_without_commits_yields_checkout() {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("empty"));

    let probed = probe(repository.root()).unwrap();

    assert_eq!(
        probed.common_dir,
        canonical(&repository.root().join(".git"))
    );
    assert_eq!(probed.worktree_root, canonical(repository.root()));
}

#[test]
fn test_probe_directory_outside_repository_yields_no_checkout() {
    let dir = TempDir::new().unwrap();

    assert_eq!(probe(dir.path()), None);
}

#[test]
fn test_probe_git_directory_yields_no_checkout() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);

    assert_eq!(probe(&repository.root().join(".git")), None);
}

#[test]
fn test_probe_unstartable_executable_yields_no_checkout() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let git = Git::with_program(dir.path().join("missing-git"));

    assert_eq!(git.probe(repository.root()).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn test_probe_runs_one_git_process_in_the_probed_directory() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let stand_in = GitStandIn::new();

    Git::with_program(stand_in.program())
        .probe(repository.root())
        .unwrap()
        .unwrap();

    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
}

#[cfg(unix)]
#[test]
fn test_head_branch_checkout_yields_commit_and_branch_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let commit = repository.git(&["rev-parse", "HEAD"]);
    let branch = repository.git(&["branch", "--show-current"]);
    let stand_in = GitStandIn::new();

    let head = checkout(Git::with_program(stand_in.program()), repository.root())
        .head()
        .unwrap()
        .unwrap();

    assert_eq!(head.commit().as_str(), commit);
    assert_eq!(head.branch(), Some(branch.as_str()));
    assert_eq!(head.branch_label(), branch);
    assert_one_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_detached_checkout_yields_commit_and_head_label_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let commit = repository.git(&["rev-parse", "HEAD"]);
    repository.git(&["checkout", "--quiet", "--detach", "HEAD"]);
    let stand_in = GitStandIn::new();

    let head = checkout(Git::with_program(stand_in.program()), repository.root())
        .head()
        .unwrap()
        .unwrap();

    assert_eq!(head.commit().as_str(), commit);
    assert_eq!(head.branch(), None);
    assert_eq!(head.branch_label(), "HEAD");
    assert_one_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_unborn_checkout_yields_none_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("unborn"));
    let stand_in = GitStandIn::new();

    let head = checkout(Git::with_program(stand_in.program()), repository.root())
        .head()
        .unwrap();

    assert_eq!(head, None);
    assert_one_head_lookup(&stand_in, repository.root());
}

#[test]
fn test_head_unstartable_program_returns_unavailable_error() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let program = dir.path().join("missing-git");

    let error = checkout(Git::with_program(&program), repository.root())
        .head()
        .unwrap_err();

    assert!(matches!(error, GitError::GitUnavailable { program: named, .. } if named == program));
}

fn dirty_paths(repository: &GitRepository) -> BTreeSet<PathBuf> {
    GitCheckout::new(Git::default(), repository.root().to_path_buf())
        .dirty_paths()
        .unwrap()
        .into_iter()
        .collect()
}

fn paths(names: &[&str]) -> BTreeSet<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

#[test]
fn test_dirty_paths_clean_tree_is_empty() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);

    assert!(dirty_paths(&repository).is_empty());
}

#[test]
fn test_dirty_paths_reports_modified_file() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write("README.md", "changed");

    assert_eq!(dirty_paths(&repository), paths(&["README.md"]));
}

#[test]
fn test_dirty_paths_reports_staged_file() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write("added.txt", "new");
    repository.git(&["add", "added.txt"]);

    assert_eq!(dirty_paths(&repository), paths(&["added.txt"]));
}

#[test]
fn test_dirty_paths_reports_deleted_file() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    std::fs::remove_file(repository.root().join("README.md")).unwrap();

    assert_eq!(dirty_paths(&repository), paths(&["README.md"]));
}

#[test]
fn test_dirty_paths_reports_both_names_of_staged_rename() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.git(&["mv", "README.md", "MOVED.md"]);

    assert_eq!(dirty_paths(&repository), paths(&["README.md", "MOVED.md"]));
}

#[test]
fn test_dirty_paths_reports_untracked_file_inside_new_directory() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write("new/deep/file.txt", "x");

    assert_eq!(dirty_paths(&repository), paths(&["new/deep/file.txt"]));
}

#[test]
fn test_dirty_paths_excludes_ignored_file() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write(".gitignore", "ignored.txt\n");
    repository.commit("ignore");
    repository.write("ignored.txt", "x");

    assert!(dirty_paths(&repository).is_empty());
}

#[test]
fn test_dirty_paths_reports_path_with_spaces_unquoted() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write("dir with space/a file.txt", "x");

    assert_eq!(
        dirty_paths(&repository),
        paths(&["dir with space/a file.txt"])
    );
}

#[cfg(unix)]
#[test]
fn test_dirty_paths_runs_one_git_process_in_the_checkout() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.write("README.md", "changed");
    let stand_in = GitStandIn::new();

    GitCheckout::new(
        Git::with_program(stand_in.program()),
        repository.root().to_path_buf(),
    )
    .dirty_paths()
    .unwrap();

    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
}

#[test]
fn test_git_checkout_canonicalizes_symbolic_abbreviated_and_full_revisions() {
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    repo.write("docs/history.bin", b"historical\0bytes");
    let oid = repo.commit("history");
    let abbreviated = &oid[..8];
    let resolver = checkout(Git::default(), repo.root());

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
fn test_git_checkout_never_falls_back_on_invalid_revision_or_failed_blob_read() {
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    repo.write("docs/working.md", b"working tree content");
    let empty_oid = repo.commit("working file exists");
    fs::remove_file(repo.root().join("docs/working.md")).unwrap();
    repo.commit("remove working file");
    repo.write(
        "docs/working.md",
        b"new uncommitted working tree fallback bait",
    );
    let resolver = checkout(Git::default(), repo.root());

    assert!(matches!(
        resolver.read_pinned_path("not-a-revision", "docs/working.md"),
        Err(GitError::RevisionNotFound { .. })
    ));
    assert!(matches!(
        resolver.read_pinned_path("HEAD", "docs/working.md"),
        Err(GitError::PinnedReadFailed { .. })
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

    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    repo.write("docs/file.md", b"pinned regular file");
    repo.write("docs/section/.keep", b"pinned directory");
    symlink("file.md", repo.root().join("docs/link.md")).unwrap();
    let revision = repo.commit("add typed pinned targets");

    fs::remove_file(repo.root().join("docs/file.md")).unwrap();
    fs::remove_dir_all(repo.root().join("docs/section")).unwrap();
    fs::remove_file(repo.root().join("docs/link.md")).unwrap();
    repo.write("docs/link.md", b"working tree fallback bait");

    let resolver = checkout(Git::default(), repo.root());
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
fn test_git_checkout_reports_unavailable_git() {
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    let resolver = checkout(
        Git::with_program(repo.root().join("definitely-no-git-here")),
        repo.root(),
    );

    assert!(matches!(
        resolver.resolve_commit("HEAD"),
        Err(GitError::GitUnavailable { .. })
    ));
}

#[cfg(unix)]
#[test]
fn test_inspect_pinned_targets_classifies_every_target_class() {
    use jit::repository_state::RepositoryTargetKind;
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    repo.write("docs/file.md", b"first version");
    repo.write("docs/executable.sh", b"#!/bin/sh\nexit 0\n");
    fs::set_permissions(
        repo.root().join("docs/executable.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    repo.write("docs/section/.keep", b"directory");
    repo.write("docs/space name.md", b"space\tpath\0\n");
    repo.write("docs/[x].md", b"glob characters");
    std::os::unix::fs::symlink("file.md", repo.root().join("docs/link.md")).unwrap();
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
    let batched = checkout(Git::with_program(stand_in.program()), repo.root());
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
        Err(GitError::RevisionNotFound { .. })
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
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
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
    let resolver = checkout(Git::with_program(stand_in.program()), repo.root());
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
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
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
    let resolver = checkout(Git::with_program(stand_in.program()), repo.root());

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
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
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
    let resolver = checkout(Git::with_program(stand_in.program()), repo.root());
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
    let dir = TempDir::new().unwrap();
    let repo = GitRepository::init(dir.path().join("repo"));
    repo.write("docs/file.md", b"content");
    repo.commit("content");
    let missing_git = repo.root().join("definitely-no-git-here");
    let resolver = checkout(Git::with_program(&missing_git), repo.root());
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
        assert!(matches!(error, GitError::GitUnavailable { .. }));
    });
}
