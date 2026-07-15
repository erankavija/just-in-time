//! Regression fixtures for jit:3f73423b's `scripts/rust-build-budget.sh`.
//!
//! REQ-04: every budget and policy the checker enforces has an independent
//! fixture that needs no compilation. Each fixture feeds the checker synthetic
//! `cargo metadata` / `cargo test --no-run --message-format=json` JSON and a
//! synthetic policy root (a workspace manifest, the jit crate manifest, and the
//! gate script), so the failure modes are exercised deterministically here
//! rather than only when the real repository happens to drift.
//!
//! Executable sizes are modelled with sparse files (`File::set_len`): `stat`
//! reports the requested byte count while the file occupies no disk blocks, so a
//! 2 GiB boundary is asserted exactly without writing 2 GiB. Real executables
//! are not sparse, so this matches what the checker measures on a live run.
//!
//! The compile-time siblings `build_profile_policy_tests.rs` and
//! `dependency_feature_policy_tests.rs` assert the same policies against the
//! real manifests; these tests assert that the gate-runtime checker reports each
//! violation with an actionable diagnostic (observed value, limit, corrective
//! area).

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

fn checker_script() -> PathBuf {
    workspace_root().join("scripts/rust-build-budget.sh")
}

/// The checker shells out to `jq`; a machine without it (matching the skip
/// convention `dependency_feature_policy_tests.rs` uses for a missing `cargo`)
/// cannot exercise these fixtures.
fn jq_available() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

const COMPLIANT_WORKSPACE_MANIFEST: &str = "\
[profile.dev]
debug = \"line-tables-only\"
incremental = true

[profile.test]
debug = \"line-tables-only\"
incremental = true
";

const COMPLIANT_JIT_MANIFEST: &str = "\
[dependencies]
jsonschema = { version = \"0.46\", default-features = false }
ureq = { version = \"3\", default-features = false, features = [\"rustls\", \"gzip\"] }
";

const COMPLIANT_GATE_SCRIPT: &str = "#!/usr/bin/env bash\nexport CARGO_INCREMENTAL=0\n";

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent dir");
    }
    fs::write(path, contents).expect("write fixture file");
}

/// A synthetic policy root plus the two Cargo JSON inputs, all under one temp
/// dir. Defaults are fully compliant; each test overwrites exactly the file
/// whose drift it exercises.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn compliant() -> Self {
        let dir = tempfile::tempdir().expect("create fixture tempdir");
        let root = dir.path();
        write(&root.join("Cargo.toml"), COMPLIANT_WORKSPACE_MANIFEST);
        write(&root.join("crates/jit/Cargo.toml"), COMPLIANT_JIT_MANIFEST);
        write(&root.join("scripts/cargo-ci.sh"), COMPLIANT_GATE_SCRIPT);
        Fixture { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Write `cargo metadata` JSON exposing `test_targets` integration-test
    /// targets plus one non-test target, so the checker's kind filter is
    /// exercised, not just a length.
    fn write_metadata(&self, test_targets: usize) -> PathBuf {
        let targets: Vec<String> = (0..test_targets)
            .map(|i| format!(r#"{{"kind":["test"],"name":"t{i}"}}"#))
            .chain(std::iter::once(
                r#"{"kind":["lib"],"name":"jit"}"#.to_string(),
            ))
            .collect();
        let json = format!(r#"{{"packages":[{{"targets":[{}]}}]}}"#, targets.join(","));
        let path = self.root().join("metadata.json");
        write(&path, &json);
        path
    }

    /// Create a sparse executable of exactly `bytes` and return its path.
    fn sparse_exe(&self, name: &str, bytes: u64) -> PathBuf {
        let path = self.root().join(name);
        File::create(&path)
            .expect("create sparse executable")
            .set_len(bytes)
            .expect("size sparse executable");
        path
    }

    /// Write an artifact JSON stream from `(executable, profile_test)` entries.
    /// A `build-script-executed` line and repeated executables let the fixtures
    /// prove non-test/non-artifact exclusion and path deduplication.
    fn write_artifacts(&self, entries: &[(&Path, bool)]) -> PathBuf {
        let mut lines: Vec<String> = entries
            .iter()
            .map(|(exe, test)| {
                format!(
                    r#"{{"reason":"compiler-artifact","profile":{{"test":{test}}},"executable":"{}"}}"#,
                    exe.display()
                )
            })
            .collect();
        lines.push(r#"{"reason":"build-script-executed","package_id":"x"}"#.to_string());
        let path = self.root().join("artifacts.jsonl");
        write(&path, &lines.join("\n"));
        path
    }

    fn run(&self, metadata: &Path, artifacts: &Path) -> Output {
        // Invoked through `bash` so the exec bit is irrelevant to the test; the
        // gate's own `./scripts/cargo-ci.sh` self-run exercises direct
        // execution.
        Command::new("bash")
            .arg(checker_script())
            .args([
                "--root",
                self.root().to_str().unwrap(),
                "--metadata-json",
                metadata.to_str().unwrap(),
                "--artifacts-json",
                artifacts.to_str().unwrap(),
            ])
            .output()
            .expect("run rust-build-budget.sh")
    }
}

const EXACT_HALF_GIB: u64 = 1024 * 1024 * 1024;
const TWO_GIB: u64 = 2 * 1024 * 1024 * 1024;

#[test]
fn test_checker_passes_at_exact_target_and_executable_budget_boundaries() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    let metadata = fx.write_metadata(12);
    // Two 1 GiB executables sum to exactly 2 GiB; the first is listed twice to
    // prove deduplication counts it once.
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let b = fx.sparse_exe("b", EXACT_HALF_GIB);
    // A non-test bin that must be excluded from the executable-byte sum.
    let bin = fx.sparse_exe("jit-server", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true), (&b, true), (&a, true), (&bin, false)]);

    let out = fx.run(&metadata, &artifacts);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "exactly 12 targets and exactly 2 GiB must pass.\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("integration-targets=12/12"),
        "summary must report the target count at its boundary: {stdout}"
    );
    assert!(
        stdout.contains(&format!("bytes={TWO_GIB}/{TWO_GIB}")),
        "summary must report the executable bytes at the 2 GiB boundary: {stdout}"
    );
    assert!(
        stdout.contains("active-executables=2"),
        "duplicate executable paths must be counted once, and the non-test bin \
         excluded: {stdout}"
    );
}

#[test]
fn test_checker_fails_on_thirteen_integration_targets() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    let metadata = fx.write_metadata(13);
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "13 targets must fail: {stderr}");
    assert!(
        stderr.contains("13") && stderr.contains("12") && stderr.contains("consolidate"),
        "diagnostic must name observed (13), limit (12), and corrective area: {stderr}"
    );
}

#[test]
fn test_checker_fails_when_active_executables_exceed_two_gib() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    let metadata = fx.write_metadata(11);
    // One byte over the 2 GiB budget.
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let b = fx.sparse_exe("b", EXACT_HALF_GIB + 1);
    let artifacts = fx.write_artifacts(&[(&a, true), (&b, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "2 GiB + 1 byte must fail: {stderr}");
    assert!(
        stderr.contains(&(TWO_GIB + 1).to_string())
            && stderr.contains("2 GiB")
            && stderr.contains("shrink"),
        "diagnostic must name observed bytes, the 2 GiB limit, and corrective area: {stderr}"
    );
}

#[test]
fn test_checker_fails_on_profile_drift() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    write(
        &fx.root().join("Cargo.toml"),
        "\
[profile.dev]
debug = \"full\"
incremental = true

[profile.test]
debug = \"line-tables-only\"
incremental = true
",
    );
    let metadata = fx.write_metadata(11);
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "profile drift must fail: {stderr}");
    assert!(
        stderr.contains("profile drift")
            && stderr.contains("\"full\"")
            && stderr.contains("line-tables-only"),
        "diagnostic must name observed debug value, the expected value, and \
         corrective area: {stderr}"
    );
}

#[test]
fn test_checker_fails_on_incremental_policy_drift() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    write(
        &fx.root().join("scripts/cargo-ci.sh"),
        "#!/usr/bin/env bash\n# incremental no longer disabled\n",
    );
    let metadata = fx.write_metadata(11);
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "incremental-policy drift must fail: {stderr}"
    );
    assert!(
        stderr.contains("incremental-policy drift") && stderr.contains("CARGO_INCREMENTAL=0"),
        "diagnostic must name the missing policy and corrective area: {stderr}"
    );
}

#[test]
fn test_checker_fails_on_remote_resolver_reintroduction() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    write(
        &fx.root().join("crates/jit/Cargo.toml"),
        "\
[dependencies]
jsonschema = { version = \"0.46\", features = [\"resolve-http\"] }
ureq = { version = \"3\", default-features = false, features = [\"rustls\", \"gzip\"] }
",
    );
    let metadata = fx.write_metadata(11);
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "resolver reintroduction must fail: {stderr}"
    );
    assert!(
        stderr.contains("resolver reintroduction")
            && stderr.contains("jsonschema")
            && stderr.contains("default-features"),
        "diagnostic must name the offending dependency and corrective area: {stderr}"
    );
}

#[test]
fn test_checker_fails_on_duplicate_tls_backend() {
    if !jq_available() {
        eprintln!("SKIP: jq not on PATH");
        return;
    }
    let fx = Fixture::compliant();
    write(
        &fx.root().join("crates/jit/Cargo.toml"),
        "\
[dependencies]
jsonschema = { version = \"0.46\", default-features = false }
ureq = { version = \"3\", default-features = false, features = [\"rustls\", \"native-tls\", \"gzip\"] }
",
    );
    let metadata = fx.write_metadata(11);
    let a = fx.sparse_exe("a", EXACT_HALF_GIB);
    let artifacts = fx.write_artifacts(&[(&a, true)]);

    let out = fx.run(&metadata, &artifacts);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "duplicate TLS backend must fail: {stderr}"
    );
    assert!(
        stderr.contains("duplicate TLS backend")
            && stderr.contains("native-tls")
            && stderr.contains("ureq"),
        "diagnostic must name the duplicate backend and corrective area: {stderr}"
    );
}
