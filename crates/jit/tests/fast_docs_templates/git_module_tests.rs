use jit::storage::git::{BaseComparison, Git, GitCheckout, GitError, ProbedCheckout};
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
    GitCheckout::for_test(git, root.to_path_buf())
}

#[cfg(unix)]
fn assert_bounded_head_lookup(stand_in: &jit::test_harness::git::GitStandIn, root: &Path) {
    let invocations = stand_in.invocations();
    assert!(
        !invocations.is_empty() && invocations.len() <= 2,
        "{invocations:?}"
    );
    assert!(invocations
        .iter()
        .all(|call| call.directory() == Some(root)));
}

#[cfg(unix)]
fn assert_one_origin_url_lookup(stand_in: &jit::test_harness::git::GitStandIn, root: &Path) {
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(root));
    assert_eq!(invocations[0].args(), ["remote", "get-url", "origin"]);
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
fn test_head_branch_checkout_yields_commit_and_branch_with_bounded_git_processes() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let commit = repository.git(&["rev-parse", "HEAD"]);
    let branch = "feature/head";
    repository.git(&["checkout", "--quiet", "-b", branch]);
    let linked = dir.path().join("linked");
    let linked_branch = "feature/linked";
    repository.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        linked_branch,
        linked.to_str().unwrap(),
    ]);

    [
        (repository.root(), branch),
        (linked.as_path(), linked_branch),
    ]
    .into_iter()
    .for_each(|(root, branch)| {
        let stand_in = GitStandIn::new();
        let head = checkout(Git::with_program(stand_in.program()), root)
            .head()
            .unwrap()
            .unwrap();

        assert_eq!(head.commit().as_str(), commit);
        assert_eq!(head.branch(), Some(branch));
        assert_eq!(head.branch_label(), branch);
        assert_bounded_head_lookup(&stand_in, root);
    });
}

#[cfg(unix)]
#[test]
fn test_head_detached_checkout_yields_commit_and_head_label_with_bounded_git_processes() {
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
    assert_bounded_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_unborn_checkout_yields_none_with_bounded_git_processes() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("unborn"));
    let stand_in = GitStandIn::new();

    let head = checkout(Git::with_program(stand_in.program()), repository.root())
        .head()
        .unwrap();

    assert_eq!(head, None);
    assert_bounded_head_lookup(&stand_in, repository.root());
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

#[cfg(unix)]
#[test]
fn test_origin_url_existing_remote_returns_url_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("repo"));
    let url = format!("{}\nremote.git", dir.path().display());
    repository.git(&["remote", "add", "origin", &url]);
    let stand_in = GitStandIn::new();

    let result = checkout(Git::with_program(stand_in.program()), repository.root())
        .origin_url()
        .unwrap();

    assert_eq!(result.as_deref(), Some(url.as_str()));
    assert_one_origin_url_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_origin_url_missing_remote_returns_none_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("repo"));
    let stand_in = GitStandIn::new();

    let result = checkout(Git::with_program(stand_in.program()), repository.root())
        .origin_url()
        .unwrap();

    assert_eq!(result, None);
    assert_one_origin_url_lookup(&stand_in, repository.root());
}

#[test]
fn test_origin_url_git_failure_returns_typed_error() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");

    let error = checkout(Git::default(), &missing).origin_url().unwrap_err();

    assert!(matches!(error, GitError::UnexpectedOutput { .. }));
}

#[cfg(unix)]
fn assert_head_failure(root: &Path) {
    let stand_in = jit::test_harness::git::GitStandIn::new();
    let error = checkout(Git::with_program(stand_in.program()), root)
        .head()
        .unwrap_err();
    assert!(
        matches!(error, GitError::UnexpectedOutput { .. }),
        "{error}"
    );
    assert_bounded_head_lookup(&stand_in, root);
}

#[cfg(unix)]
#[test]
fn test_head_invalid_checkout_returns_an_error() {
    let dir = TempDir::new().unwrap();
    [dir.path().to_path_buf(), dir.path().join("missing")]
        .iter()
        .for_each(|root| assert_head_failure(root));
}

#[cfg(unix)]
#[test]
fn test_head_previously_recognized_checkout_becoming_invalid_returns_an_error() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let stand_in = jit::test_harness::git::GitStandIn::new();
    let recognized = Git::default().probe(repository.root()).unwrap().unwrap();
    let handle = checkout(
        Git::with_program(stand_in.program()),
        &recognized.worktree_root,
    );
    std::fs::remove_dir_all(repository.root().join(".git")).unwrap();

    assert!(matches!(
        handle.head(),
        Err(GitError::UnexpectedOutput { .. })
    ));
    assert_bounded_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_malformed_refs_are_not_classified_as_unborn() {
    [
        (".git/HEAD", "nonsense\n"),
        (".git/HEAD", ""),
        (".git/HEAD", "ref: refs/heads/a..b\n"),
        (".git/HEAD", "ref: refs/tags/missing\n"),
        (".git/refs/heads/feature/head", "nonsense\n"),
        (
            ".git/refs/heads/feature/head",
            "ref: refs/heads/feature/head\n",
        ),
    ]
    .iter()
    .for_each(|(path, contents)| {
        let dir = TempDir::new().unwrap();
        let repository = committed_repository(&dir);
        repository.git(&["branch", "-m", "feature/head"]);
        repository.write(path, contents);
        assert_head_failure(repository.root());
    });
}

#[cfg(unix)]
fn loose_object(repository: &GitRepository, oid: &str) -> PathBuf {
    repository
        .root()
        .join(".git/objects")
        .join(&oid[..2])
        .join(&oid[2..])
}

#[cfg(unix)]
fn damage_object(repository: &GitRepository, oid: &str, corrupt: bool) {
    let path = loose_object(repository, oid);
    std::fs::remove_file(&path).unwrap();
    if corrupt {
        std::fs::write(path, b"invalid compressed object").unwrap();
    }
}

#[cfg(unix)]
#[test]
fn test_head_missing_or_corrupt_commit_returns_an_error() {
    [false, true].into_iter().for_each(|corrupt| {
        let dir = TempDir::new().unwrap();
        let repository = committed_repository(&dir);
        let oid = repository.git(&["rev-parse", "HEAD"]);
        damage_object(&repository, &oid, corrupt);
        assert_head_failure(repository.root());
    });
}

#[cfg(unix)]
#[test]
fn test_head_non_commit_object_returns_an_error() {
    [false, true].into_iter().for_each(|tag| {
        let dir = TempDir::new().unwrap();
        let repository = committed_repository(&dir);
        let oid = if tag {
            repository.git(&["-c", "tag.gpgSign=false", "tag", "-a", "-m", "tag", "named"]);
            repository.git(&["rev-parse", "refs/tags/named"])
        } else {
            repository.git(&["hash-object", "-w", "README.md"])
        };
        repository.write(".git/HEAD", format!("{oid}\n"));
        assert_head_failure(repository.root());
    });
}

#[cfg(unix)]
#[test]
fn test_head_cached_or_replaced_commit_cannot_mask_object_damage() {
    [false, true].into_iter().for_each(|replace| {
        [false, true].into_iter().for_each(|corrupt| {
            let dir = TempDir::new().unwrap();
            let repository = committed_repository(&dir);
            let original = repository.git(&["rev-parse", "HEAD"]);
            if replace {
                repository.write("README.md", "replacement");
                let replacement = repository.commit("replacement");
                repository.git(&["replace", &original, &replacement]);
                repository.git(&["update-ref", "HEAD", &original]);
            } else {
                repository.git(&["commit-graph", "write", "--reachable"]);
            }
            damage_object(&repository, &original, corrupt);
            assert_head_failure(repository.root());
        });
    });
}

#[cfg(unix)]
#[test]
fn test_head_does_not_traverse_commit_ancestors() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let parent = repository.git(&["rev-parse", "HEAD"]);
    repository.write("README.md", "second");
    let tip = repository.commit("second");
    damage_object(&repository, &parent, false);
    let stand_in = jit::test_harness::git::GitStandIn::new();

    let head = checkout(Git::with_program(stand_in.program()), repository.root())
        .head()
        .unwrap()
        .unwrap();

    assert_eq!(head.commit().as_str(), tip);
    assert_bounded_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_missing_promisor_commit_does_not_start_a_fetch() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let oid = repository.git(&["rev-parse", "HEAD"]);
    repository.git(&[
        "config",
        "remote.origin.url",
        dir.path().join("absent-remote").to_str().unwrap(),
    ]);
    repository.git(&["config", "remote.origin.promisor", "true"]);
    damage_object(&repository, &oid, false);
    let trace = dir.path().join("trace.json");
    let stand_in = jit::test_harness::git::GitStandIn::with_environment(&[(
        "GIT_TRACE2_EVENT",
        trace.to_str().unwrap(),
    )]);

    assert!(
        checkout(Git::with_program(stand_in.program()), repository.root())
            .head()
            .is_err()
    );

    let events: Vec<serde_json::Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        !events.iter().any(|event| event["event"] == "child_start"),
        "{events:?}"
    );
    assert_bounded_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_head_does_not_read_unrelated_refs_reflog_index_or_worktree_files() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.git(&["checkout", "--quiet", "--detach"]);
    let traps: Vec<PathBuf> = [
        ".git/logs/HEAD",
        ".git/refs/heads/unrelated",
        ".git/index",
        "untracked",
    ]
    .iter()
    .map(|path| repository.root().join(path))
    .collect();
    traps.iter().for_each(|path| {
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
        nix::unistd::mkfifo(
            path,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .unwrap();
    });
    let stand_in = jit::test_harness::git::GitStandIn::new();
    let handle = checkout(Git::with_program(stand_in.program()), repository.root());

    let (result, accessed) = std::thread::scope(|scope| {
        let (stop, stopped) = mpsc::channel::<()>();
        // A forbidden reader receives EOF, so a regression fails instead of hanging.
        let monitor = scope.spawn(move || {
            let mut accessed = BTreeSet::new();
            loop {
                traps.iter().for_each(|path| {
                    if std::fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits())
                        .open(path)
                        .is_ok()
                    {
                        accessed.insert(path.clone());
                    }
                });
                match stopped.recv_timeout(Duration::from_millis(1)) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => return accessed,
                }
            }
        });
        let result = handle.head();
        drop(stop);
        (result, monitor.join().unwrap())
    });

    assert!(result.is_ok(), "{result:?}");
    assert!(accessed.is_empty(), "unexpected reads: {accessed:?}");
    assert_bounded_head_lookup(&stand_in, repository.root());
}

#[cfg(unix)]
#[test]
fn test_user_name_returns_configured_name_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("primary"));
    let stand_in = GitStandIn::new();

    let user_name = checkout(Git::with_program(stand_in.program()), repository.root())
        .user_name()
        .unwrap();

    assert_eq!(user_name.as_deref(), Some("Test User"));
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
    assert_eq!(invocations[0].args(), ["config", "user.name"]);
}

#[cfg(unix)]
#[test]
fn test_user_name_returns_none_when_unset_with_one_git_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("primary"));
    repository.git(&["config", "--local", "--unset", "user.name"]);
    let empty_global_config = dir.path().join("empty-gitconfig");
    std::fs::write(&empty_global_config, "").unwrap();
    let stand_in = GitStandIn::with_environment(&[
        ("GIT_CONFIG_GLOBAL", empty_global_config.to_str().unwrap()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]);

    let user_name = checkout(Git::with_program(stand_in.program()), repository.root())
        .user_name()
        .unwrap();

    assert_eq!(user_name, None);
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
    assert_eq!(invocations[0].args(), ["config", "user.name"]);
}

fn dirty_paths(repository: &GitRepository) -> BTreeSet<PathBuf> {
    GitCheckout::for_test(Git::default(), repository.root().to_path_buf())
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

    GitCheckout::for_test(
        Git::with_program(stand_in.program()),
        repository.root().to_path_buf(),
    )
    .dirty_paths()
    .unwrap();

    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
}

#[cfg(unix)]
#[test]
fn test_list_worktrees_primary_linked_detached_and_slash_branches_use_one_process() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let primary_branch = repository.git(&["branch", "--show-current"]);
    let linked = dir.path().join("linked with spaces");
    let detached = dir.path().join("detached");
    let slash_branch = dir.path().join("topic");
    repository.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "linked",
        linked.to_str().unwrap(),
    ]);
    repository.git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        detached.to_str().unwrap(),
        "HEAD",
    ]);
    repository.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "topic/nested/branch",
        slash_branch.to_str().unwrap(),
    ]);
    repository.git(&[
        "worktree",
        "lock",
        "--reason",
        "worktree undergoing maintenance",
        detached.to_str().unwrap(),
    ]);
    let stand_in = GitStandIn::new();

    let worktrees = checkout(Git::with_program(stand_in.program()), repository.root())
        .list_worktrees()
        .unwrap();

    let entries: BTreeSet<_> = worktrees
        .iter()
        .map(|worktree| {
            (
                worktree.path().to_path_buf(),
                worktree.branch().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(worktrees.len(), entries.len());
    assert_eq!(
        entries,
        BTreeSet::from([
            (canonical(repository.root()), Some(primary_branch)),
            (canonical(&linked), Some("linked".to_owned())),
            (canonical(&detached), None),
            (
                canonical(&slash_branch),
                Some("topic/nested/branch".to_owned())
            ),
        ])
    );
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
    assert_eq!(
        invocations[0].args(),
        ["worktree", "list", "--porcelain", "-z"]
    );
}

#[test]
fn test_list_worktrees_from_linked_checkout_includes_bare_repository_without_branch() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let bare = dir.path().join("bare.git");
    let linked = dir.path().join("linked-from-bare");
    repository.git(&[
        "clone",
        "--quiet",
        "--bare",
        repository.root().to_str().unwrap(),
        bare.to_str().unwrap(),
    ]);
    repository.git(&[
        "--git-dir",
        bare.to_str().unwrap(),
        "worktree",
        "add",
        "--quiet",
        "-b",
        "linked-from-bare",
        linked.to_str().unwrap(),
    ]);

    let worktrees = checkout(Git::default(), &linked).list_worktrees().unwrap();

    assert_eq!(worktrees.len(), 2);
    assert!(worktrees
        .iter()
        .any(|worktree| worktree.path() == canonical(&bare) && worktree.branch().is_none()));
    assert!(worktrees
        .iter()
        .any(|worktree| worktree.path() == canonical(&linked)
            && worktree.branch() == Some("linked-from-bare")));
}

#[cfg(unix)]
#[test]
fn test_list_worktrees_preserves_newlines_inside_worktree_paths() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let linked = dir.path().join("linked\n\nworktree misleading");
    repository.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "delimiter-path",
        linked.to_str().unwrap(),
    ]);

    let worktrees = checkout(Git::default(), repository.root())
        .list_worktrees()
        .unwrap();

    let linked_entry = worktrees
        .iter()
        .find(|worktree| worktree.branch() == Some("delimiter-path"))
        .unwrap();
    assert_eq!(linked_entry.path(), canonical(&linked));
}

#[test]
fn test_file_history_returns_added_changed_and_deleted_commits_newest_first() {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("history"));
    repository.write("document.md", "first");
    let added = repository.commit("add document");
    repository.write("document.md", "second");
    let changed = repository.commit("change document");
    repository.write("other.md", "unrelated");
    repository.commit("unrelated change");
    std::fs::remove_file(repository.root().join("document.md")).unwrap();
    let deleted = repository.commit("delete document");

    let history = checkout(Git::default(), repository.root())
        .file_history("document.md")
        .unwrap();

    let commits: Vec<_> = history
        .iter()
        .map(|entry| entry.commit().as_str())
        .collect();
    assert_eq!(commits, [deleted, changed, added]);
}

#[test]
fn test_file_history_preserves_author_commit_time_and_multiline_message() {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("metadata"));
    let author = "History Author";
    let message = "subject with café\n\nbody line one\nbody line two";
    repository.git(&["config", "user.name", author]);
    repository.write("document.md", "first");
    repository.git(&["add", "--all"]);
    repository.git(&[
        "commit",
        "--quiet",
        "--message",
        message,
        "--date",
        "2001-09-09T01:46:40Z",
    ]);
    let commit_time = repository.git(&["show", "-s", "--format=%ct", "HEAD"]);

    let history = checkout(Git::default(), repository.root())
        .file_history("document.md")
        .unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].author(), author);
    assert_eq!(
        history[0].commit_time(),
        commit_time.parse::<i64>().unwrap()
    );
    assert_eq!(history[0].message(), format!("{message}\n"));
}

#[test]
fn test_file_history_omits_merge_identical_to_one_parent() {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("merge"));
    repository.write("document.md", "base");
    let added = repository.commit("add document");
    let primary = repository.git(&["branch", "--show-current"]);
    repository.git(&["checkout", "--quiet", "-b", "side"]);
    repository.write("document.md", "side change");
    let changed = repository.commit("side changes document");
    repository.git(&["checkout", "--quiet", &primary]);
    repository.write("other.md", "primary change");
    repository.commit("primary changes another path");
    repository.git(&["merge", "--quiet", "--no-ff", "-m", "merge side", "side"]);

    let history = checkout(Git::default(), repository.root())
        .file_history("document.md")
        .unwrap();

    let commits: Vec<_> = history
        .iter()
        .map(|entry| entry.commit().as_str())
        .collect();
    assert_eq!(commits, [changed, added]);
}

#[cfg(unix)]
#[test]
fn test_file_history_treats_option_and_pathspec_metacharacters_literally() {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path().join("literal"));
    repository.write(":(glob)*", "literal pathspec");
    let magic = repository.commit("pathspec-looking name");
    repository.write("--all", "literal option");
    let option = repository.commit("option-looking name");
    repository.write("other.md", "unrelated");
    repository.commit("unrelated path");
    let git = checkout(Git::default(), repository.root());

    for (path, expected) in [(":(glob)*", magic), ("--all", option)] {
        let history = git.file_history(path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].commit().as_str(), expected);
    }
}

#[test]
fn test_file_history_missing_path_returns_empty_history() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);

    assert!(checkout(Git::default(), repository.root())
        .file_history("absent.md")
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[test]
fn test_file_history_runs_one_log_process_per_query_in_the_checkout() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let stand_in = GitStandIn::new();
    let git = checkout(Git::with_program(stand_in.program()), repository.root());

    git.file_history("README.md").unwrap();
    assert_eq!(stand_in.invocations().len(), 1);
    git.file_history("absent.md").unwrap();

    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 2);
    for invocation in invocations {
        assert_eq!(invocation.directory(), Some(repository.root()));
        assert!(invocation.args().iter().any(|arg| arg == "log"));
    }
}

#[test]
fn test_file_history_failed_log_returns_typed_error() {
    let dir = TempDir::new().unwrap();

    let error = checkout(Git::default(), dir.path())
        .file_history("document.md")
        .unwrap_err();

    assert!(matches!(error, GitError::UnexpectedOutput { command, .. } if command.contains("log")));
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
            format!("docs/file-{index}.md"),
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
        repo.write(format!("other/file-{index}.md"), b"unrequested");
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

#[test]
fn test_compare_with_base_descendant_reports_base_as_merge_base() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let base = repository.git(&["rev-parse", "HEAD"]);
    repository.git(&["branch", "base"]);
    repository.write("next.txt", "next").commit("next");

    let comparison = checkout(Git::default(), repository.root())
        .compare_with_base("base")
        .unwrap();

    assert!(
        matches!(comparison, BaseComparison::Compared { merge_base, base_commit }
        if merge_base.as_str() == base && base_commit.as_str() == base)
    );
}

#[cfg(unix)]
#[test]
fn test_compare_with_base_diverged_linked_worktree_uses_selected_checkout() {
    use jit::test_harness::git::GitStandIn;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let common = repository.git(&["rev-parse", "HEAD"]);
    let linked = dir.path().join("linked");
    let linked_path = linked.to_str().unwrap();
    repository.git(&["worktree", "add", "--quiet", "-b", "topic", linked_path]);
    repository.write("base.txt", "base");
    let base = repository.commit("advance base");
    std::fs::write(linked.join("topic.txt"), "topic").unwrap();
    repository.git(&["-C", linked_path, "add", "topic.txt"]);
    repository.git(&[
        "-C",
        linked_path,
        "commit",
        "--quiet",
        "-m",
        "advance topic",
    ]);
    let stand_in = GitStandIn::new();

    let comparison = checkout(Git::with_program(stand_in.program()), &linked)
        .compare_with_base(&repository.git(&["branch", "--show-current"]))
        .unwrap();

    assert!(
        matches!(comparison, BaseComparison::Compared { merge_base, base_commit }
        if merge_base.as_str() == common && base_commit.as_str() == base)
    );
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 2);
    assert!(invocations
        .iter()
        .all(|invocation| invocation.directory() == Some(linked.as_path())));
}

#[test]
fn test_compare_with_base_missing_ref_reports_base_unresolved() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);

    let comparison = checkout(Git::default(), repository.root())
        .compare_with_base("refs/heads/missing")
        .unwrap();

    assert_eq!(comparison, BaseComparison::BaseUnresolved);
}

#[test]
fn test_compare_with_base_unrelated_history_reports_no_merge_base() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let unrelated = GitRepository::init(dir.path().join("unrelated"));
    unrelated.write("other.txt", "other").commit("other");
    repository.git(&[
        "fetch",
        unrelated.root().to_str().unwrap(),
        "HEAD:refs/heads/unrelated",
    ]);

    let comparison = checkout(Git::default(), repository.root())
        .compare_with_base("unrelated")
        .unwrap();

    assert_eq!(comparison, BaseComparison::NoMergeBase);
}

#[cfg(unix)]
#[test]
fn test_object_reader_answers_incremental_binary_tree_and_missing_requests_in_one_process() {
    use jit::storage::git::GitObjectType;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let bytes = b"asset\0with\nbytes\xff\n";
    repository
        .write("assets/binary file", bytes)
        .commit("binary asset");
    let stand_in = GitStandIn::new();
    let handle = checkout(Git::with_program(stand_in.program()), repository.root());
    let mut reader = handle.object_reader().unwrap();
    let document = reader.read("HEAD:README.md").unwrap().unwrap();
    assert_eq!(document.bytes(), b"readme");
    assert_eq!(document.object_type(), GitObjectType::Blob);
    let binary = reader.read("HEAD:assets/binary file").unwrap().unwrap();
    assert_eq!(binary.bytes(), bytes);
    assert_eq!(
        binary.id().as_str(),
        repository.git(&["rev-parse", "HEAD:assets/binary file"])
    );
    assert_eq!(
        reader.read("HEAD:assets").unwrap().unwrap().object_type(),
        GitObjectType::Tree
    );
    assert_eq!(reader.read("HEAD:absent path").unwrap(), None);
    assert_eq!(reader.read("unresolvable:missing path").unwrap(), None);
    assert_eq!(reader.read(binary.id().as_str()).unwrap().unwrap(), binary);
    drop(reader);
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
    assert_eq!(invocations[0].args(), ["cat-file", "--batch"]);
}

#[cfg(unix)]
#[test]
fn test_object_types_preserves_input_order_and_absence_in_one_process() {
    use jit::storage::git::GitObjectType;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository.git(&["tag", "-a", "annotated", "-m", "tag"]);
    let stand_in = GitStandIn::new();
    let handle = checkout(Git::with_program(stand_in.program()), repository.root());
    assert_eq!(
        handle
            .object_types([
                "HEAD:README.md",
                "HEAD:",
                "HEAD:missing",
                "unresolvable",
                "HEAD",
                "annotated",
                "HEAD:README.md"
            ])
            .unwrap(),
        [
            Some(GitObjectType::Blob),
            Some(GitObjectType::Tree),
            None,
            None,
            Some(GitObjectType::Commit),
            Some(GitObjectType::Tag),
            Some(GitObjectType::Blob)
        ]
    );
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].directory(), Some(repository.root()));
    assert_eq!(invocations[0].args(), ["cat-file", "--batch-check"]);
}

#[cfg(unix)]
#[test]
fn test_object_operations_treat_gitlink_as_absent_between_valid_objects() {
    use jit::storage::git::GitObjectType;

    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    repository
        .write("assets/one", b"first")
        .write("assets/two", b"second");
    repository.commit("add assets");
    repository.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000,1111111111111111111111111111111111111111,assets/module",
    ]);
    repository.git(&["commit", "--quiet", "--message", "add gitlink"]);

    let stand_in = GitStandIn::new();
    let handle = checkout(Git::with_program(stand_in.program()), repository.root());
    assert_eq!(
        handle
            .object_types(["HEAD:assets/one", "HEAD:assets/module", "HEAD:assets/two",])
            .unwrap(),
        [Some(GitObjectType::Blob), None, Some(GitObjectType::Blob)]
    );
    let mut reader = handle.object_reader().unwrap();
    assert_eq!(
        reader.read("HEAD:assets/one").unwrap().unwrap().bytes(),
        b"first"
    );
    assert_eq!(reader.read("HEAD:assets/module").unwrap(), None);
    assert_eq!(
        reader.read("HEAD:assets/two").unwrap().unwrap().bytes(),
        b"second"
    );
    drop(reader);
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 2, "{invocations:?}");
    assert_eq!(invocations[0].args(), ["cat-file", "--batch-check"]);
    assert_eq!(invocations[1].args(), ["cat-file", "--batch"]);
}

#[test]
fn test_object_reader_rejects_invalid_names_without_corrupting_the_next_request() {
    let dir = TempDir::new().unwrap();
    let repository = committed_repository(&dir);
    let handle = checkout(Git::default(), repository.root());
    let mut reader = handle.object_reader().unwrap();
    ["HEAD\nHEAD", "HEAD\0suffix"].into_iter().for_each(|name| {
        assert!(matches!(reader.read(name), Err(GitError::InvalidPath(_))));
    });
    assert_eq!(
        reader.read("HEAD:README.md").unwrap().unwrap().bytes(),
        b"readme"
    );
}

#[test]
fn test_object_types_validates_every_name_before_starting_git() {
    let dir = TempDir::new().unwrap();
    let handle = checkout(
        Git::with_program(dir.path().join("missing-git")),
        dir.path(),
    );
    ["HEAD\nHEAD", "HEAD\0suffix"].into_iter().for_each(|name| {
        assert!(matches!(
            handle.object_types(["HEAD", name]),
            Err(GitError::InvalidPath(_))
        ));
    });
}

#[test]
fn test_object_operations_preserve_missing_executable_errors() {
    let dir = TempDir::new().unwrap();
    let handle = checkout(
        Git::with_program(dir.path().join("missing-git")),
        dir.path(),
    );
    assert!(matches!(
        handle.object_reader(),
        Err(GitError::GitUnavailable { .. })
    ));
    assert!(matches!(
        handle.object_types(["HEAD"]),
        Err(GitError::GitUnavailable { .. })
    ));
}

#[cfg(unix)]
fn scripted_object_checkout(dir: &TempDir, body: &str) -> GitCheckout {
    let program = dir.path().join("object-git");
    jit::test_utils::write_executable(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    checkout(Git::with_program(program), dir.path())
}

#[cfg(unix)]
#[test]
fn test_object_reader_rejects_malformed_headers_and_truncated_object_frames() {
    let oid = "1".repeat(40);
    [
        "not-an-oid blob 0\n\n".to_owned(),
        format!("{oid} unknown 0\n\n"),
        format!("{oid} blob -1\n"),
        format!("{oid} blob 18446744073709551616\n"),
        "different-name missing\n".to_owned(),
        "not-an-oid submodule\n".to_owned(),
        format!("{oid} submodule extra\n"),
        format!("{oid} unknown\n"),
        format!("{oid} blob 0"),
        format!("{oid} blob 2\na"),
        format!("{oid} blob 1\nax"),
    ]
    .into_iter()
    .for_each(|response| {
        let dir = TempDir::new().unwrap();
        let handle =
            scripted_object_checkout(&dir, &format!("read -r request\nprintf '%s' '{response}'"));
        let mut reader = handle.object_reader().unwrap();
        assert!(
            matches!(
                reader.read("requested-name"),
                Err(GitError::UnexpectedOutput { .. })
            ),
            "{response:?}"
        );
        assert!(matches!(
            reader.read("another-name"),
            Err(GitError::UnexpectedOutput { .. })
        ));
    });
}

#[cfg(unix)]
#[test]
fn test_object_reader_drop_and_protocol_failure_reap_the_child() {
    use nix::errno::Errno;
    use nix::sys::wait::{waitpid, WaitPidFlag};
    use nix::unistd::Pid;

    [false, true].into_iter().for_each(|fail| {
        let dir = TempDir::new().unwrap();
        let handle = scripted_object_checkout(
            &dir,
            r#"
read -r request
pid=$$
printf '%040d blob %s\n%s\n' 0 "${#pid}" "$pid"
read -r request
printf 'malformed\n'
read -r request
"#,
        );
        let mut reader = handle.object_reader().unwrap();
        let object = reader.read("child-pid").unwrap().unwrap();
        let pid = std::str::from_utf8(object.bytes())
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert!(pid > 0);
        if fail {
            assert!(matches!(
                reader.read("fail"),
                Err(GitError::UnexpectedOutput { .. })
            ));
            assert_eq!(
                waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)),
                Err(Errno::ECHILD)
            );
        }
        drop(reader);
        assert_eq!(
            waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)),
            Err(Errno::ECHILD)
        );
    });
}

#[cfg(unix)]
#[test]
fn test_object_reader_drains_stderr_while_waiting_for_a_response() {
    let dir = TempDir::new().unwrap();
    let handle = scripted_object_checkout(
        &dir,
        r#"
read -r request
printf '%131072s' diagnostic >&2
printf '%040d blob 0\n\n' 0
read -r request
"#,
    );
    assert_eq!(
        handle
            .object_reader()
            .unwrap()
            .read("empty")
            .unwrap()
            .unwrap()
            .bytes(),
        b""
    );
}

#[cfg(unix)]
#[test]
fn test_object_types_checks_process_exit_and_rejects_extra_responses() {
    ["exit 9", "printf 'extra\\n'"]
        .into_iter()
        .for_each(|ending| {
            let dir = TempDir::new().unwrap();
            let handle = scripted_object_checkout(
                &dir,
                &format!("read -r request\nprintf '%040d blob 0\\n' 0\nread -r request\n{ending}"),
            );
            assert!(matches!(
                handle.object_types(["object"]),
                Err(GitError::UnexpectedOutput { .. })
            ));
        });
}

#[cfg(unix)]
#[test]
fn test_object_reader_reports_failed_process_diagnostics() {
    let dir = TempDir::new().unwrap();
    let handle =
        scripted_object_checkout(&dir, "read -r request\nprintf 'object failure' >&2\nexit 7");
    let mut reader = handle.object_reader().unwrap();
    assert!(
        matches!(reader.read("object"), Err(GitError::UnexpectedOutput { output, .. }) if output.contains("object failure"))
    );
}
