//! Repository root discovery.
//!
//! Mirrors git's own algorithm for locating a repository from an arbitrary
//! working directory: start at the current directory and walk upward through
//! ancestors, stopping at the first directory that contains an *initialized*
//! `.jit` repository. A directory counts only when its `.jit` holds an
//! `index.json`; a bare or partial `.jit` (stray test litter, an interrupted
//! `jit init`, a lone `worktree.json`) is skipped and the walk continues, so
//! an unrelated `.jit` in a shared ancestor cannot hijack resolution. The walk
//! does not cross a `.git` boundary — once an ancestor containing `.git` has
//! been checked (inclusive) and found not to hold an initialized `.jit`,
//! discovery halts there rather than escaping into an unrelated ancestor's
//! `.jit`. The walk is naturally bounded: `Path::ancestors` yields a finite
//! chain that ends at the filesystem root, so there is no unbounded traversal.
//!
//! An explicit `JIT_DATA_DIR` override always takes precedence over discovery;
//! that check lives in the caller (`main.rs`), not here.

use std::path::{Path, PathBuf};

/// Marker file that distinguishes an initialized `.jit` repository from a bare
/// or partial `.jit` directory. Kept in sync with `json::INDEX_FILE`.
const INDEX_FILE: &str = "index.json";

/// Walk upward from `start`, using `exists` to probe each candidate path, to
/// find the nearest ancestor directory containing an initialized `.jit`
/// repository.
///
/// This is the pure core of [`discover_jit_dir`]: it takes an existence
/// predicate rather than touching the filesystem directly, so the traversal
/// and boundary logic can be unit tested against a synthetic directory layout.
///
/// A candidate `.jit` counts only when it holds an `index.json` — the predicate
/// is probed at `<ancestor>/.jit/index.json`, not merely `<ancestor>/.jit`. A
/// bare or partial `.jit` is ignored and the walk continues upward, so stray
/// or interrupted `.jit` directories in shared ancestors (e.g. a CI temp root)
/// cannot capture discovery.
///
/// Stops as soon as an ancestor containing `.git` has been checked (inclusive)
/// and found not to hold an initialized `.jit` — it does not consult any
/// ancestor above that directory.
///
/// # Examples
///
/// ```
/// use std::collections::HashSet;
/// use std::path::{Path, PathBuf};
/// use jit::storage::discovery::discover_from;
///
/// // A directory counts as a repository only when its `.jit` holds an
/// // `index.json`, so the predicate is probed at `.jit/index.json`.
/// let initialized: HashSet<PathBuf> = HashSet::from([PathBuf::from("/repo/.jit/index.json")]);
/// let found = discover_from(Path::new("/repo/crates/jit"), |p| initialized.contains(p));
/// assert_eq!(found, Some(PathBuf::from("/repo/.jit")));
///
/// // A bare `.jit` with no `index.json` is skipped, so nothing is bound.
/// let bare: HashSet<PathBuf> = HashSet::from([PathBuf::from("/repo/.jit")]);
/// let none = discover_from(Path::new("/repo/crates/jit"), |p| bare.contains(p));
/// assert_eq!(none, None);
/// ```
pub fn discover_from(start: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let candidate = ancestor.join(".jit");
        if exists(&candidate.join(INDEX_FILE)) {
            return Some(candidate);
        }
        if exists(&ancestor.join(".git")) {
            break;
        }
    }
    None
}

/// Find the nearest ancestor of `start` (inclusive) that contains an
/// initialized `.jit` repository, mirroring git's own repository-root
/// discovery.
///
/// Returns `None` when no such ancestor exists within the `.git` boundary (see
/// [`discover_from`]) or up to the filesystem root. Callers should fall back
/// to a default location (conventionally `<start>/.jit`) in that case, so the
/// eventual "repository not found" error names a sensible path.
///
/// Does not consult the `JIT_DATA_DIR` override — callers must check that
/// first, as it always takes precedence over discovery.
///
/// # Examples
///
/// ```
/// use jit::storage::discovery::discover_jit_dir;
///
/// let repo = tempfile::tempdir().unwrap();
/// let jit = repo.path().join(".jit");
/// std::fs::create_dir(&jit).unwrap();
/// std::fs::write(jit.join("index.json"), "{}").unwrap();
/// let nested = repo.path().join("crates/jit");
/// std::fs::create_dir_all(&nested).unwrap();
///
/// // Discovery from a nested subdirectory resolves the repository root's `.jit`.
/// assert_eq!(discover_jit_dir(&nested), Some(jit));
/// ```
pub fn discover_jit_dir(start: &Path) -> Option<PathBuf> {
    discover_from(start, Path::exists)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    /// Build an `exists` predicate backed by a fixed set of paths, so tests
    /// exercise the traversal/boundary logic without touching the real
    /// filesystem.
    fn exists_in(set: HashSet<PathBuf>) -> impl Fn(&Path) -> bool {
        move |p: &Path| set.contains(p)
    }

    /// Path to the `index.json` marker inside a `.jit` at `dir`, the presence
    /// of which is what makes a directory count as an initialized repository.
    fn index_in(dir: &str) -> PathBuf {
        PathBuf::from(dir).join(".jit").join("index.json")
    }

    #[test]
    fn test_discover_from_finds_jit_in_current_directory() {
        let set = HashSet::from([index_in("/repo")]);
        let found = discover_from(Path::new("/repo"), exists_in(set));
        assert_eq!(found, Some(PathBuf::from("/repo/.jit")));
    }

    #[test]
    fn test_discover_from_walks_up_from_nested_subdirectory() {
        // REQ-01: a deeply nested cwd resolves to the repo root's `.jit`.
        let set = HashSet::from([index_in("/repo")]);
        let found = discover_from(Path::new("/repo/crates/jit/src"), exists_in(set));
        assert_eq!(found, Some(PathBuf::from("/repo/.jit")));
    }

    #[test]
    fn test_discover_from_finds_repo_when_cwd_is_inside_dot_jit() {
        // REQ-01: running from inside `.jit/issues` itself still resolves —
        // `.jit` has no nested `.jit` of its own, so the walk climbs past it to
        // the repo root, which does.
        let set = HashSet::from([index_in("/repo")]);
        let found = discover_from(Path::new("/repo/.jit/issues"), exists_in(set));
        assert_eq!(found, Some(PathBuf::from("/repo/.jit")));
    }

    #[test]
    fn test_discover_from_skips_bare_jit_without_index() {
        // Regression: a bare `.jit` (no index.json) in a shared ancestor — e.g.
        // stray CI-temp litter — must NOT be bound. The walk skips it and finds
        // the initialized repo higher up instead.
        let set = HashSet::from([
            PathBuf::from("/shared/.jit"), // bare, no index.json -> skipped
            index_in("/shared/proj"),      // initialized repo
        ]);
        let found = discover_from(Path::new("/shared/proj/sub"), exists_in(set));
        assert_eq!(found, Some(PathBuf::from("/shared/proj/.jit")));
    }

    #[test]
    fn test_discover_from_bare_jit_only_yields_none() {
        // A bare `.jit` and nothing initialized anywhere resolves to None, so
        // the caller reports "not found" rather than binding to the partial dir.
        let set = HashSet::from([PathBuf::from("/shared/.jit")]);
        let found = discover_from(Path::new("/shared/tmp/xyz"), exists_in(set));
        assert_eq!(found, None);
    }

    #[test]
    fn test_discover_from_stops_at_git_boundary() {
        // REQ-02: an unrelated ancestor `.jit` beyond the `.git` boundary must
        // never be reached.
        let set = HashSet::from([
            index_in("/workspace"),                // unrelated ancestor repo
            PathBuf::from("/workspace/repo/.git"), // boundary
        ]);
        let found = discover_from(Path::new("/workspace/repo/sub"), exists_in(set));
        assert_eq!(found, None);
    }

    #[test]
    fn test_discover_from_checks_git_directory_itself_inclusive() {
        // The directory containing `.git` is still checked for `.jit` before
        // the boundary halts discovery (the boundary is inclusive).
        let set = HashSet::from([index_in("/repo"), PathBuf::from("/repo/.git")]);
        let found = discover_from(Path::new("/repo/sub"), exists_in(set));
        assert_eq!(found, Some(PathBuf::from("/repo/.jit")));
    }

    #[test]
    fn test_discover_from_returns_none_when_nothing_found_up_to_root() {
        // REQ-05: the walk terminates (rather than looping forever) when
        // nothing matches anywhere up to the filesystem root.
        let found = discover_from(Path::new("/a/b/c/d/e/f"), exists_in(HashSet::new()));
        assert_eq!(found, None);
    }

    #[test]
    fn test_discover_from_stops_at_root_even_without_git_boundary() {
        // REQ-05, restated at the root itself: with no `.git` anywhere and no
        // `.jit` anywhere, the walk still reaches "/" and halts rather than
        // panicking or looping.
        let found = discover_from(Path::new("/x/y"), exists_in(HashSet::new()));
        assert_eq!(found, None);
    }

    #[test]
    fn test_discover_jit_dir_uses_real_filesystem() {
        // Smoke test for the real-filesystem wrapper against a tempdir fixture.
        let temp = tempfile::tempdir().unwrap();
        let jit = temp.path().join(".jit");
        fs::create_dir(&jit).unwrap();
        fs::write(jit.join("index.json"), "{}").unwrap();
        let nested = temp.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();

        let found = discover_jit_dir(&nested);
        assert_eq!(found, Some(temp.path().join(".jit")));
    }

    #[test]
    fn test_discover_jit_dir_real_filesystem_skips_bare_jit() {
        // Regression for the shared-temp-root poisoning: a bare `.jit` (no
        // index.json) at an ancestor is ignored, so discovery reports None
        // rather than binding to it. `.git` bounds the walk within the tempdir.
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join(".jit")).unwrap(); // bare, no index.json
        let nested = temp.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();

        let found = discover_jit_dir(&nested);
        assert_eq!(found, None);
    }

    #[test]
    fn test_discover_jit_dir_real_filesystem_none_within_bounded_tree() {
        // A `.git` marker bounds discovery within the tempdir so this assertion
        // holds regardless of what exists further up the real filesystem.
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let nested = temp.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();

        let found = discover_jit_dir(&nested);
        assert_eq!(found, None);
    }
}
