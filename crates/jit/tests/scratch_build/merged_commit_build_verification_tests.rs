//! Regression test for jit:45e1b7e8 REQ-03.
//!
//! Gate checkers compile the working tree of whoever invokes them, so a clean
//! merge can leave the mainline non-compiling while every gate reports passed.
//! The observed case: a worker branch anchored before a module deletion, merged
//! after it, cleanly re-added a `mod` declaration for a file that no longer
//! existed.
//!
//! `scripts/verify-commit-builds.sh` closes that gap by judging the merged
//! commit's sources in isolation, and `scripts/verify-commit-builds-selftest.sh`
//! constructs the resurrection scenario in a scratch git repo and asserts that
//! the new check reports failure on a tree where the post-wave leak check reports
//! success. This test runs that self-test under `cargo test` so cargo-ci enforces
//! the regression on every build.

use std::path::Path;
use std::process::Command;

/// Runs the shell self-test that constructs the resurrection case end to end.
/// The self-test exits 0 on success, 1 on a failed assertion, and 2 on an
/// environment error (missing `git`/`cargo`), which we treat as a skip so the
/// suite stays green on hosts without the toolchain.
#[test]
fn test_verify_commit_builds_selftest_reproduces_resurrection() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest");
    let selftest = workspace_root.join("scripts/verify-commit-builds-selftest.sh");
    assert!(
        selftest.is_file(),
        "self-test script missing at {}",
        selftest.display()
    );

    let output = Command::new("bash")
        .arg(&selftest)
        .current_dir(workspace_root)
        .output()
        .expect("failed to spawn the verify-commit-builds self-test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    match output.status.code() {
        Some(0) => {}
        Some(2) => {
            eprintln!("SKIP: self-test reported an environment error (missing git/cargo)\n{stderr}");
        }
        other => panic!(
            "verify-commit-builds self-test failed (exit {other:?})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
    }
}
