use jit::storage::git::{Git, GitCheckout, GitError, ProbedCheckout};
use jit::test_harness::git::GitRepository;
use std::collections::BTreeSet;
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
