use jit::storage::{GitRevisionError, GitRevisionResolver};
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
        git(&root, &["init"]);
        git(&root, &["config", "user.name", "Test User"]);
        git(&root, &["config", "user.email", "test@example.com"]);
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

#[test]
fn test_git_revision_resolver_lists_committed_and_worktree_changes_without_losing_untracked_paths()
{
    let repo = Repo::new();
    repo.write("src/main.rs", b"fn main() {}\n");
    let base = repo.commit("source");

    repo.write("docs/metadata.md", b"metadata-only change\n");
    let head = repo.commit("documentation");
    repo.write("src/main.rs", b"fn main() { println!(\"changed\"); }\n");
    repo.write("Cargo.toml", b"[package]\nname = \"changed\"\n");
    repo.write("new-top-level-tooling.txt", b"not compiled\n");

    let resolver = GitRevisionResolver::new(&repo.root);
    let committed = resolver.changed_paths_between(&base, &head).unwrap();
    assert_eq!(committed, vec!["docs/metadata.md"]);

    let worktree = resolver.changed_worktree_paths().unwrap();
    assert!(worktree.iter().any(|path| path == "src/main.rs"));
    assert!(worktree.iter().any(|path| path == "Cargo.toml"));
    assert!(worktree
        .iter()
        .any(|path| path == "new-top-level-tooling.txt"));
}
