//! Regression evidence for jit:3019eacd.
//!
//! A textually clean merge can still leave the mainline broken: one branch
//! deletes a module file while another declares it, or one changes a function's
//! signature while another adds a caller of the old form. Every per-issue gate
//! passed against a tree that predates the merge, so the only check that sees
//! the combination is the build-and-test gate run on the merged tree —
//! `scripts/cargo-ci.sh`.
//!
//! Relying on that is sound only while the gate can still fail. A build-only
//! check compiles neither test targets nor dev-dependencies, so a merge that
//! breaks only test code passes it, as does one that compiles and fails when the
//! tests run. `scripts/cargo-ci-selftest.sh` seeds clean merges that break in
//! each of those ways, runs the shipped gate script against them, and asserts
//! the verdict it reports. This test runs that self-test under `cargo test`, so
//! the gate enforces its own non-vacuity on every run.

use std::path::Path;
use std::process::Command;

/// Runs the shell self-test that seeds the broken merges and reads the gate's
/// verdict on each. The self-test exits 0 on success, 1 on a failed assertion,
/// and 2 on an environment error (missing `git`/`cargo`/`jq`), which we treat as
/// a skip so the suite stays green on hosts without the toolchain.
#[test]
fn test_cargo_ci_selftest_fails_seeded_broken_merges() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest");
    let selftest = workspace_root.join("scripts/cargo-ci-selftest.sh");
    assert!(
        selftest.is_file(),
        "self-test script missing at {}",
        selftest.display()
    );

    let output = Command::new("bash")
        .arg(&selftest)
        .current_dir(workspace_root)
        .output()
        .expect("failed to spawn the cargo-ci self-test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    match output.status.code() {
        Some(0) => {}
        Some(2) => {
            eprintln!(
                "SKIP: self-test reported an environment error (missing git/cargo/jq)\n{stderr}"
            );
        }
        other => panic!(
            "cargo-ci self-test failed (exit {other:?})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
    }
}
