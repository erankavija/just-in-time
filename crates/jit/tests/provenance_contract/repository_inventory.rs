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
//! The banned input categories — `.agents/worktrees`, nested Cargo `target/`
//! directories, Git metadata, Node `node_modules` trees, and the dogfooding
//! tracker data root — are dropped by explicit path filters
//! ([`is_banned_input`]), independent of the source checkout's ignore rules.
//! In this repository `.gitignore` already keeps most of them out of the
//! `git ls-files` listing, but that is an optimization, not the guarantee:
//! even force-added (`git add -f`) or unignored content in a banned category
//! never reaches the seeded copy. The tracker data root is banned whole: what
//! it holds is this repository's issue data, which would otherwise make every
//! issue mutation an input to a build-stability measurement, and no build
//! inside the seeded fixture reads anything beneath it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository-relative root of the tracker's own data: issue records, event
/// log, gate runs, and the repository-local configuration beside them.
pub(crate) const TRACKER_DATA_ROOT: &str = ".jit";

/// This workspace's root, the source checkout the provenance fixtures seed from.
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

/// List `source_root`'s repository-input inventory via
/// `git ls-files --cached --others --exclude-standard`, dropping every path
/// [`is_banned_input`] rejects. See the module docs for the split between the
/// listing (keeps the inventory small) and the filter (the guarantee).
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
        .filter(|path| !is_banned_input(path))
        .collect()
}

/// Banned repository-input categories (REQ-01): operational and generated
/// trees that must never seed a provenance fixture, whatever the source
/// checkout's git or ignore state says about them.
///
/// Git metadata, other agents' worktrees, and generated trees are banned
/// outright, and so is the tracker data root: what it holds is this
/// repository's issue data, which would otherwise make every issue mutation an
/// input to a build-stability measurement.
///
/// `Path::starts_with` matches whole components, so `.gitignore` and
/// `.gitattributes` are not caught by the `.git` prefix and stay in the
/// inventory.
fn is_banned_input(path: &Path) -> bool {
    path.starts_with(".git")
        || path.starts_with(".agents/worktrees")
        || path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some("target" | "node_modules")))
        || path.starts_with(TRACKER_DATA_ROOT)
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
