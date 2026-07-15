//! REQ-02/REQ-06 regression for jit:5d862134: a Git-metadata-only change must
//! not invalidate any Rust test target.
//!
//! Before this fix, `crates/jit/build.rs` declared `.git/index`, `HEAD`, and
//! refs as `rerun-if-changed` inputs and stamped the wall clock, so staging or
//! committing UNCHANGED sources reran the build script and relinked every test
//! target (~179s per the rust-build-efficiency baseline). This test reproduces
//! the exact loop in an isolated temporary Git repository: it builds all test
//! targets once, then stages a metadata-only change and rebuilds, then commits
//! a metadata-only change and rebuilds, asserting that neither rebuild produces
//! a non-fresh `profile.test` compiler artifact.
//!
//! Ignored from the default suite: the first build is a full cold workspace
//! compile (minutes), intrinsic to what the test exercises, so plain
//! `cargo test` skips it. `scripts/cargo-ci.sh`'s `provenance` step runs it (and
//! the `version_cli_tests` contract suite) with `--ignored` on every gate, so
//! this hard REQ-06 contract is exercised by CI. Run it directly with:
//!   cargo test -p jit --test build_provenance_metadata_stability_tests -- --ignored
//!
//! The isolated repository is seeded via `repository_inventory::seed_isolated_repository`,
//! which bounds the fixture to this workspace's repository-input inventory
//! (jit:83efbcb4) rather than walking the whole working tree.

use crate::repository_inventory::seed_isolated_repository;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

/// Seed `dest` as an isolated Git repository built from this workspace's
/// repository-input inventory (see `repository_inventory` module docs; that
/// is what bounds this fixture to tracked repository inputs rather than
/// walking the whole working tree — jit:83efbcb4). Prints the seeded file and
/// byte counts (REQ-04) before returning, so a cold-build run makes future
/// snapshot growth observable without inspecting the fixture by hand.
fn seed_source_repo(dest: &Path) {
    let (file_count, total_bytes) = seed_isolated_repository(&workspace_root(), dest);
    println!("provenance-fixture: files={file_count} bytes={total_bytes}");
}

/// Run `cargo test --workspace --no-run --message-format=json` in `source_dir`
/// with a dedicated `CARGO_TARGET_DIR`, and count `compiler-artifact` messages
/// for Rust TEST targets (`profile.test == true`): the total, and how many were
/// non-fresh (recompiled/relinked rather than reused).
fn count_test_artifacts(source_dir: &Path, target_dir: &Path) -> (usize, usize) {
    let output = Command::new("cargo")
        .current_dir(source_dir)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(["test", "--workspace", "--no-run", "--message-format=json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo test --no-run should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut total = 0usize;
    let mut non_fresh = 0usize;
    for line in stdout.lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if msg["reason"] != "compiler-artifact" {
            continue;
        }
        if msg["profile"]["test"].as_bool() != Some(true) {
            continue;
        }
        total += 1;
        if msg["fresh"].as_bool() == Some(false) {
            non_fresh += 1;
        }
    }
    (total, non_fresh)
}

#[test]
#[ignore = "full cold workspace build; run explicitly with --ignored"]
fn test_metadata_only_change_relinks_no_test_targets() {
    // Keep the copied sources and the cold build target off tmpfs (a full
    // workspace target is large): base the scratch tree under the on-disk
    // `target/` directory, which the copy above excludes.
    let base = workspace_root().join("target");
    std::fs::create_dir_all(&base).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("jit-metadata-stability-")
        .tempdir_in(&base)
        .unwrap();
    let source_dir = temp.path().join("source");
    let target_dir = temp.path().join("cargo-target");
    seed_source_repo(&source_dir);

    // Cold baseline: every test target compiles, so all are non-fresh. This
    // also proves the harness actually built test targets to reason about.
    let (baseline_total, baseline_non_fresh) = count_test_artifacts(&source_dir, &target_dir);
    assert!(
        baseline_total > 0,
        "the baseline build should produce at least one test target"
    );
    assert_eq!(
        baseline_non_fresh, baseline_total,
        "a cold baseline build compiles every test target"
    );

    // Metadata-only STAGED change: a new non-source file, staged (updates
    // .git/index). Rust sources are untouched, so no test target may relink.
    std::fs::write(
        source_dir.join("metadata-only.txt"),
        "staged, no source change",
    )
    .unwrap();
    let stage = Command::new("git")
        .current_dir(&source_dir)
        .args(["add", "metadata-only.txt"])
        .status()
        .unwrap();
    assert!(stage.success());

    let (_, staged_non_fresh) = count_test_artifacts(&source_dir, &target_dir);
    assert_eq!(
        staged_non_fresh, 0,
        "staging a metadata-only change must relink no test targets (got {staged_non_fresh})"
    );

    // Metadata-only COMMIT: moves HEAD and refs. Still no Rust source change,
    // so still no test target may relink.
    let commit = Command::new("git")
        .current_dir(&source_dir)
        .args(["commit", "-q", "-m", "metadata only"])
        .status()
        .unwrap();
    assert!(commit.success());

    let (_, committed_non_fresh) = count_test_artifacts(&source_dir, &target_dir);
    assert_eq!(
        committed_non_fresh, 0,
        "a metadata-only commit must relink no test targets (got {committed_non_fresh})"
    );
}
