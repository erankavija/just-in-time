//! REQ-02 regression for jit:83efbcb4: `repository_inventory::seed_isolated_repository`
//! must exclude generated/worktree content while keeping tracked repository
//! inputs. Builds a small standalone Git repository fixture with
//! representative junk and representative real content, so it proves the
//! actual seeding path used by the provenance cold-build test — not a
//! reimplementation of it. Fast: no cargo build, so plain `cargo test` runs
//! this unlike its `#[ignore]`d siblings.

use crate::repository_inventory::seed_isolated_repository;
use std::path::Path;
use std::process::Command;

/// Build a standalone Git repository under `root`: tracked repository inputs
/// representative of every category REQ-02 requires to survive seeding (Rust
/// source, web build input, manifest, lock file), plus a tracked `.jit/` entry
/// (mirroring this repository's dogfooding setup), then untracked
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

    std::fs::write(
        root.join(".gitignore"),
        "/target/\n**/target/\nnode_modules/\n.agents/worktrees/\n",
    )
    .unwrap();

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
        !dest.join(".jit").exists(),
        "must not copy the dogfooding .jit tree"
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
