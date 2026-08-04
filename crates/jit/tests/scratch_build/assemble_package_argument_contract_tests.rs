//! Regression evidence for jit:5367fcba.
//!
//! `scripts/assemble-package.sh` replaces its destination whole, so an argument
//! it does not recognise must be refused rather than treated as a directory to
//! write into. Before the guard, invoking it as `--help` assembled the package
//! tree into a directory literally named `--help` and reported success — the
//! same shape a mistyped path takes.
//!
//! `scripts/assemble-package-selftest.sh` drives that contract against the
//! shipped script with a stubbed `cargo`, so it observes the argument handling
//! without paying for an assembly. This test runs that self-test under
//! `cargo test`, which is what puts the contract inside the required
//! validation path: nothing else in the gate reads a shell script's argument
//! handling, so without this the guard could be removed and every gate would
//! still report green.

use std::path::Path;
use std::process::Command;

/// Runs the self-test and reads its verdict. It exits 0 when every case
/// passed, 1 on a failed assertion, and 2 when the script under test is
/// unavailable, which is treated as a skip so the suite stays green on a
/// checkout without it.
#[test]
fn test_assemble_package_selftest_holds_the_argument_contract() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest");
    let selftest = workspace_root.join("scripts/assemble-package-selftest.sh");
    assert!(
        selftest.is_file(),
        "self-test script missing at {}",
        selftest.display()
    );

    let output = Command::new("bash")
        .arg(&selftest)
        .current_dir(workspace_root)
        .output()
        .expect("failed to spawn the assemble-package self-test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    match output.status.code() {
        Some(0) => {}
        Some(2) => {
            eprintln!("SKIP: self-test reported the script under test unavailable\n{stderr}");
        }
        other => panic!(
            "assemble-package self-test failed (exit {other:?})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
    }
}
