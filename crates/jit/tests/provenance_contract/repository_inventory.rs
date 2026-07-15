//! Repository-input inventory for the provenance fixtures (jit:83efbcb4).
//!
//! `build_provenance_metadata_stability_tests.rs` seeds an isolated repository
//! to exercise the build script under real Git states. It used to do that by
//! tar-copying the whole working tree with a short exclusion list, which in a
//! checkout carrying generated agent worktrees (each with its own `target/`)
//! walked and copied gigabytes of unrelated generated content before the cold
//! build even started (jit:83efbcb4). This module seeds from an explicit
//! inventory instead: `git ls-files --cached --others --exclude-standard`
//! (tracked files, plus untracked files git itself would not ignore — so a
//! source file added to the working tree but not yet `git add`ed still
//! appears), rooted at the given source directory.
//!
//! That inventory structurally cannot contain `.agents/worktrees`, nested
//! Cargo `target/` directories, Git internals, or Node `node_modules` trees —
//! all are ignored (`.gitignore`) in this repository, so `--exclude-standard`
//! never returns them. `.jit/` is different: this repository dogfoods jit on
//! itself, so `.jit/` is tracked (and not gitignored) here. It is excluded by
//! an explicit path filter below rather than by git's ignore rules.

use std::path::{Path, PathBuf};
use std::process::Command;

/// List `source_root`'s repository-input inventory via
/// `git ls-files --cached --others --exclude-standard`, excluding the
/// dogfooding `.jit/` tree. See the module docs for which mechanism keeps each
/// banned category out.
pub(crate) fn repository_inputs(source_root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .current_dir(source_root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .expect("git ls-files should run");
    assert!(
        output.status.success(),
        "git ls-files should succeed in {}",
        source_root.display()
    );

    String::from_utf8(output.stdout)
        .expect("repository-input paths are valid UTF-8")
        .lines()
        .map(PathBuf::from)
        .filter(|path| !path.starts_with(".jit"))
        .collect()
}

/// Seed `dest` as an isolated, independent Git repository built from
/// `source_root`'s repository-input inventory, plus the `web/dist/index.html`
/// stub required so `crates/server`'s build script never materializes
/// `web/dist` mid-run (that would make a later rebuild's
/// `rerun-if-changed=web/dist/` fire and relink the server test target, a
/// confound unrelated to whatever the caller is testing).
///
/// Returns the copied file count and total byte count, so callers can report
/// snapshot-growth observability (REQ-04) before paying for a cold build.
pub(crate) fn seed_isolated_repository(source_root: &Path, dest: &Path) -> (usize, u64) {
    std::fs::create_dir_all(dest).unwrap();
    let stats = copy_repository_inputs(source_root, dest);
    ensure_web_dist_stub(dest);
    init_git_repo(dest);
    stats
}

/// Copy every path in `source_root`'s repository-input inventory into `dest`,
/// preserving symlinks (this repository tracks a couple, e.g. `CLAUDE.md` ->
/// `AGENTS.md`) rather than dereferencing them.
fn copy_repository_inputs(source_root: &Path, dest: &Path) -> (usize, u64) {
    let inputs = repository_inputs(source_root);
    let mut total_bytes = 0u64;
    for rel_path in &inputs {
        let src = source_root.join(rel_path);
        let dst = dest.join(rel_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        let meta = std::fs::symlink_metadata(&src)
            .unwrap_or_else(|e| panic!("stat {}: {e}", src.display()));
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&src).unwrap();
            std::os::unix::fs::symlink(&target, &dst)
                .unwrap_or_else(|e| panic!("symlink {}: {e}", dst.display()));
        } else {
            total_bytes +=
                std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {}: {e}", src.display()));
        }
    }
    (inputs.len(), total_bytes)
}

fn ensure_web_dist_stub(dest: &Path) {
    let dist = dest.join("web").join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("index.html"),
        "<!doctype html><title>stub</title>",
    )
    .unwrap();
}

fn init_git_repo(dest: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "jit-test@example.invalid"],
        vec!["config", "user.name", "JIT Test"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "seed"],
    ] {
        let status = Command::new("git")
            .current_dir(dest)
            .args(&args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "git setup command {args:?} should succeed"
        );
    }
}
