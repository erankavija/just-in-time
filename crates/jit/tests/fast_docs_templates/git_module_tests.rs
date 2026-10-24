use jit::storage::git::{Git, ProbedCheckout};
use jit::test_harness::git::GitRepository;
use std::path::Path;
use tempfile::TempDir;

fn canonical(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap()
}

fn probe(dir: &Path) -> Option<ProbedCheckout> {
    Git::default().probe(dir).unwrap()
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
