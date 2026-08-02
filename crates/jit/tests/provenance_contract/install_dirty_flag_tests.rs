//! REQ-11 (jit:32779829): what the installer's dirty flag means.
//!
//! `scripts/install-jit.sh` embeds a flag that makes the installed binary
//! report itself stale for its whole life
//! ([`assess_binary_provenance`](jit::domain::build_provenance)). It must be
//! true when a declared build input is uncommitted and false otherwise — an
//! uncommitted plan note or changelog entry feeds no build and must leave an
//! installed binary current.
//!
//! The installer's last act is `exec cargo install`, so the behaviour is
//! exercised by `scripts/install-jit-selftest.sh`, which runs the real script
//! against throwaway repositories with a stub `cargo` that reports the
//! provenance it was handed. This test is what puts that evidence inside
//! `cargo test`; the cases live in the script.

use std::path::Path;
use std::process::Command;

/// The self-test exits 0 on success, 1 on a failed assertion, and 2 on an
/// environment error (missing `git`), which we treat as a skip so the suite
/// stays green on hosts without the toolchain.
#[test]
fn test_install_jit_selftest_reports_only_declared_build_inputs_as_dirty() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest");
    let selftest = workspace_root.join("scripts/install-jit-selftest.sh");
    assert!(
        selftest.is_file(),
        "self-test script missing at {}",
        selftest.display()
    );

    let output = Command::new("bash")
        .arg(&selftest)
        .current_dir(workspace_root)
        .output()
        .expect("failed to spawn the install-jit self-test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    match output.status.code() {
        Some(0) => {}
        Some(2) => {
            eprintln!("SKIP: self-test reported an environment error (missing git)\n{stderr}");
        }
        other => panic!(
            "install-jit self-test failed (exit {other:?})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
    }
}
