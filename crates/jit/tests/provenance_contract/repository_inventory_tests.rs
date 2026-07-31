//! REQ-02 regression for jit:83efbcb4: `repository_inventory::seed_isolated_repository`
//! must exclude generated/worktree content while keeping tracked repository
//! inputs. Builds a small standalone Git repository fixture with
//! representative junk and representative real content, so it proves the
//! actual seeding path used by the provenance cold-build test — not a
//! reimplementation of it. Fast: no cargo build, so plain `cargo test` runs
//! this unlike its `#[ignore]`d siblings.
//!
//! The same fixture carries the tracker data root's split (jit:c7a548c3): the
//! sources the profile package draws from there are seeded, so the cold build
//! the fixture hosts can read them, and the issue data beside them is not. The
//! last test observes that split against this repository's own inventory,
//! which is the checkout the cold-build fixture seeds from.

use crate::repository_inventory::{
    packaged_tracker_data_sources, repository_inputs, seed_isolated_repository, workspace_root,
    TRACKER_DATA_ROOT,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a standalone Git repository under `root`: tracked repository inputs
/// representative of every category REQ-02 requires to survive seeding (Rust
/// source, web build input, manifest, lock file), plus a tracked tracker-data
/// tree mirroring this repository's dogfooding setup — issue data, which is
/// banned, alongside every path the package declares as a live-asset target
/// there, which a build inside the fixture reads (jit:c7a548c3). Then untracked
/// generated/worktree junk matching every category REQ-01 bans: a nested
/// agent worktree with its own `target/`, a top-level `target/`, and Node
/// `node_modules` trees. The junk is created after the commit so it stays
/// untracked and gitignored, exactly as it does in a live checkout.
fn seed_fixture_repo(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
    std::fs::write(root.join("Cargo.lock"), "# lock\n").unwrap();
    std::fs::create_dir_all(root.join("web/src")).unwrap();
    std::fs::write(root.join("web/package.json"), "{}\n").unwrap();
    std::fs::write(
        root.join("web/src/App.tsx"),
        "export default function App() {}\n",
    )
    .unwrap();

    std::fs::create_dir_all(root.join(".jit/issues")).unwrap();
    std::fs::write(root.join(".jit/issues/example.json"), "{}\n").unwrap();
    for target in packaged_tracker_data_sources() {
        let path = root.join(&target);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "packaged source\n").unwrap();
    }

    // Deliberately NO ignore rules for the banned categories: the exclusion
    // under test is the inventory's own path filter, not this fixture's (or any
    // checkout's) .gitignore.
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();

    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "jit-test@example.invalid"],
        vec!["config", "user.name", "JIT Test"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "seed"],
    ] {
        let status = Command::new("git")
            .current_dir(root)
            .args(&args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "fixture git setup command {args:?} should succeed"
        );
    }

    let nested_worktree_target = root.join(".agents/worktrees/agent-fake/target/debug/deps");
    std::fs::create_dir_all(&nested_worktree_target).unwrap();
    std::fs::write(nested_worktree_target.join("junk.rlib"), vec![0u8; 4096]).unwrap();
    std::fs::write(
        root.join(".agents/worktrees/agent-fake/src_copy.rs"),
        "fn generated() {}\n",
    )
    .unwrap();

    std::fs::create_dir_all(root.join("target/debug/deps")).unwrap();
    std::fs::write(
        root.join("target/debug/deps/top_level.rlib"),
        vec![0u8; 4096],
    )
    .unwrap();

    std::fs::create_dir_all(root.join("node_modules/leftpad")).unwrap();
    std::fs::write(
        root.join("node_modules/leftpad/index.js"),
        "module.exports = {};\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("web/node_modules/react")).unwrap();
    std::fs::write(
        root.join("web/node_modules/react/index.js"),
        "module.exports = {};\n",
    )
    .unwrap();

    // Force-add one banned file: even content git tracks (add -f) must never
    // reach the seeded copy — the exclusion is the filter, not git state.
    let status = Command::new("git")
        .current_dir(root)
        .args([
            "add",
            "-f",
            "target/debug/deps/top_level.rlib",
            "node_modules/leftpad/index.js",
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "force-adding banned fixtures should succeed"
    );
}

#[test]
fn test_seed_isolated_repository_excludes_generated_and_worktree_content() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");
    std::fs::create_dir_all(&source).unwrap();
    seed_fixture_repo(&source);

    let (file_count, total_bytes) = seed_isolated_repository(&source, &dest);
    assert!(file_count > 0, "should copy at least one file");
    assert!(total_bytes > 0, "should copy at least one byte");

    // REQ-01: every banned category absent from the seeded repository.
    assert!(
        !dest.join(".agents").exists(),
        "must not copy .agents/worktrees content"
    );
    assert!(
        !dest.join("target").exists(),
        "must not copy nested target/ directories"
    );
    assert!(
        !dest.join("node_modules").exists(),
        "must not copy top-level node_modules"
    );
    assert!(
        !dest.join("web/node_modules").exists(),
        "must not copy web/node_modules"
    );
    assert!(
        !dest.join(".jit/issues/example.json").exists(),
        "must not copy the dogfooding tracker data root's issue data"
    );

    // REQ-02: tracked Rust, web build inputs, manifests, locks, and the
    // required web/dist/index.html fixture remain present.
    assert!(
        dest.join("src/main.rs").exists(),
        "tracked Rust source must be present"
    );
    assert!(dest.join("Cargo.toml").exists(), "manifest must be present");
    assert!(
        dest.join("Cargo.lock").exists(),
        "lock file must be present"
    );
    assert!(
        dest.join("web/package.json").exists(),
        "web build input must be present"
    );
    assert!(
        dest.join("web/src/App.tsx").exists(),
        "web build input must be present"
    );
    assert!(
        dest.join("web/dist/index.html").exists(),
        "the required web/dist/index.html stub must be present"
    );

    // The seeded destination is itself a fresh, independent Git repository.
    assert!(
        dest.join(".git").is_dir(),
        "the seeded repository must be its own Git repository"
    );
}

/// Every file beneath `root`, as paths relative to `root`. Absent directory:
/// the empty set, so a caller can express "nothing under here survived".
fn relative_files(root: &Path) -> BTreeSet<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return BTreeSet::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .flat_map(|path| {
            if path.is_dir() {
                relative_files(&path)
                    .into_iter()
                    .map(|nested| path.join(nested))
                    .collect()
            } else {
                vec![path]
            }
        })
        .map(|path| path.strip_prefix(root).unwrap().to_path_buf())
        .collect()
}

#[test]
fn test_seed_isolated_repository_keeps_only_the_packaged_sources_under_the_tracker_data_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let dest = temp.path().join("dest");
    std::fs::create_dir_all(&source).unwrap();
    seed_fixture_repo(&source);

    seed_isolated_repository(&source, &dest);

    // Without a packaged source under that root the equality below would hold
    // vacuously, and the fixture would prove nothing about what a build inside
    // it can read.
    let packaged = packaged_tracker_data_sources();
    assert!(
        !packaged.is_empty(),
        "the package must declare at least one live-asset target under the tracker data root"
    );

    // The packaged sources reach the fixture, so a build inside it finds every
    // source the package is assembled from; nothing else under that root does,
    // so issue data is not an input to a build-stability measurement.
    let seeded: BTreeSet<PathBuf> = relative_files(&dest.join(TRACKER_DATA_ROOT))
        .into_iter()
        .map(|path| Path::new(TRACKER_DATA_ROOT).join(path))
        .collect();
    assert_eq!(
        seeded, packaged,
        "the seeded tracker data root must carry exactly the packaged live-asset targets"
    );
}

#[test]
fn test_repository_inputs_carry_the_packaged_tracker_data_sources_of_this_repository() {
    let inputs: BTreeSet<PathBuf> = repository_inputs(&workspace_root()).into_iter().collect();
    let packaged = packaged_tracker_data_sources();
    assert!(
        !packaged.is_empty(),
        "the package must declare at least one live-asset target under the tracker data root"
    );

    // Same property as the fixture test above, observed against this
    // repository's own inventory: the sources the package draws from are
    // inputs, and the issue data beside them is not.
    let from_tracker_data: BTreeSet<PathBuf> = inputs
        .into_iter()
        .filter(|path| path.starts_with(TRACKER_DATA_ROOT))
        .collect();
    assert_eq!(
        from_tracker_data, packaged,
        "this repository's inventory must carry exactly the packaged live-asset targets \
         from its tracker data root"
    );
}
