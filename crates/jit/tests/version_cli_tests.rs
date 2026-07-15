use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use std::process::Command as StdCommand;

#[test]
fn test_global_version_flag_reports_local_provenance() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success(), "--version should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(stdout.contains("commit"));
    assert!(stdout.contains("profile"));
}

#[test]
fn test_version_command_reports_human_readable_provenance_without_repo() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .arg("version")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "version command should not require .jit"
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Version:"));
    assert!(stdout.contains("Commit:"));
    assert!(stdout.contains("Dirty:"));
    assert!(stdout.contains("Profile:"));
    assert!(stdout.contains("Built:"));
    assert!(stdout.contains("Target:"));
}

#[test]
fn test_version_command_reports_json_provenance_without_repo() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["version", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "version --json should not require .jit"
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("version output is JSON");
    assert_eq!(json["package"].as_str(), Some("jit"));
    assert_eq!(json["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    assert!(json.get("git_commit").is_some());
    assert!(json.get("git_short_commit").is_some());
    assert!(json.get("git_dirty").is_some());
    assert!(json.get("build_profile").is_some());
    assert!(json.get("build_timestamp").is_some());
    assert!(json.get("target").is_some());
}

// Ignored from the default suite: these spawn `cargo run` into dedicated
// CARGO_TARGET_DIRs, forcing a cold compile (~85s) to exercise the build script
// under specific provenance environments. Compilation is intrinsic to what they
// test, so they cannot meet the per-test speed budget. Run on demand / in CI
// with:
//   cargo test -p jit --test version_cli_tests -- --ignored

// REQ-05: a build that injects NO provenance succeeds and reports the
// documented fallbacks — `unknown` commit fields, null dirty, and an `unknown`
// timestamp rather than a wall-clock-dependent value. The build reads no
// ambient Git or clock at all, so the surrounding workspace's Git state does
// not leak into the reported provenance.
#[test]
#[ignore = "full cold rebuild (~85s); run explicitly with --ignored"]
fn test_version_build_without_injected_provenance_reports_unknowns() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let json = version_json(&temp_dir.path().join("target-no-provenance"), &[]);

    assert_eq!(json["git_commit"].as_str(), Some("unknown"));
    assert_eq!(json["git_short_commit"].as_str(), Some("unknown"));
    assert!(json["git_dirty"].is_null());
    // REQ-05: the fallback, not the wall clock.
    assert_eq!(json["build_timestamp"].as_str(), Some("unknown"));
}

// REQ-03/REQ-04: injected provenance is reported exactly, reproduced on an
// unchanged rebuild, and invalidated when an injected value changes; the clean
// (`dirty=false`) and dirty (`dirty=true`) reporting contracts both hold.
//
// Ignored from the default suite (cold rebuild, plus incremental rebuilds as
// each injected environment changes). See the note above. Run with --ignored.
#[test]
#[ignore = "cold rebuild + incremental rebuilds; run explicitly with --ignored"]
fn test_injected_provenance_is_reported_reproduced_and_invalidated() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let target = temp_dir.path().join("target-injected");

    let hash_a = "1111111111111111111111111111111111111111";
    let clean_env = [
        ("JIT_BUILD_GIT_HASH", hash_a),
        ("JIT_BUILD_GIT_SHORT_HASH", "11111111"),
        ("JIT_BUILD_GIT_DIRTY", "false"),
        ("SOURCE_DATE_EPOCH", "1700000000"),
    ];

    // REQ-03: `jit version --json` reports the injected values exactly. Clean
    // reporting contract: injected `dirty=false` reports `false`.
    let first = version_json(&target, &clean_env);
    assert_eq!(first["git_commit"].as_str(), Some(hash_a));
    assert_eq!(first["git_short_commit"].as_str(), Some("11111111"));
    assert_eq!(first["git_dirty"].as_bool(), Some(false));
    assert_eq!(first["build_timestamp"].as_str(), Some("1700000000"));

    // REQ-04 (reproducible): repeating the build with the same injected fields
    // reports identical provenance.
    let repeat = version_json(&target, &clean_env);
    assert_eq!(
        first, repeat,
        "identical injected provenance must reproduce identically"
    );

    // REQ-04 (invalidation): changing an injected value invalidates the build
    // output — the newly reported value follows the change.
    let hash_b = "2222222222222222222222222222222222222222";
    let changed_env = [
        ("JIT_BUILD_GIT_HASH", hash_b),
        ("JIT_BUILD_GIT_SHORT_HASH", "22222222"),
        ("JIT_BUILD_GIT_DIRTY", "false"),
        ("SOURCE_DATE_EPOCH", "1700000000"),
    ];
    let changed = version_json(&target, &changed_env);
    assert_eq!(changed["git_commit"].as_str(), Some(hash_b));
    assert_eq!(changed["git_short_commit"].as_str(), Some("22222222"));

    // Dirty reporting contract: injected `dirty=true` reports `true`.
    let dirty_env = [
        ("JIT_BUILD_GIT_HASH", hash_b),
        ("JIT_BUILD_GIT_SHORT_HASH", "22222222"),
        ("JIT_BUILD_GIT_DIRTY", "true"),
        ("SOURCE_DATE_EPOCH", "1700000000"),
    ];
    let dirty = version_json(&target, &dirty_env);
    assert_eq!(dirty["git_dirty"].as_bool(), Some(true));
}

// MSRV drift guard. The supported Rust version is declared once, as
// `rust-version` under `[workspace.package]` in the workspace `Cargo.toml`, and
// every member inherits it via `rust-version.workspace = true`. CI's `msrv` job
// compiles the workspace on exactly that toolchain, but that only catches a
// member that still inherits; this test catches a member that silently *stops*
// inheriting (dropping out of the enforced policy). It asserts structure, not a
// specific number, so it never becomes a second hand-maintained copy of the
// version literal.
#[test]
fn test_workspace_declares_and_inherits_rust_version() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let root_manifest = std::fs::read_to_string(workspace_root.join("Cargo.toml")).unwrap();
    let declared = root_manifest
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("rust-version")?
                .trim_start()
                .strip_prefix('=')
                .map(|rest| rest.trim().trim_matches('"').to_string())
        })
        .expect("workspace Cargo.toml must declare rust-version under [workspace.package]");

    // A concrete "major.minor" MSRV, not a moving channel like "stable".
    let mut parts = declared.split('.');
    let major: u32 = parts.next().unwrap().parse().unwrap_or_else(|_| {
        panic!("rust-version must be a numeric version, got {declared:?}");
    });
    let minor: u32 = parts
        .next()
        .expect("rust-version must include a minor component")
        .parse()
        .unwrap_or_else(|_| panic!("rust-version must be numeric, got {declared:?}"));
    assert!(
        major >= 1 && (major > 1 || minor > 0),
        "rust-version {declared:?} is implausibly low"
    );

    // Every workspace member must inherit the single declared value rather than
    // pin its own, so the MSRV job enforces one policy for the whole workspace.
    for member in ["crates/jit", "crates/server"] {
        let manifest =
            std::fs::read_to_string(workspace_root.join(member).join("Cargo.toml")).unwrap();
        assert!(
            manifest.contains("rust-version.workspace = true")
                || manifest.contains("rust-version = { workspace = true }"),
            "{member}/Cargo.toml must inherit rust-version from the workspace"
        );
    }
}

/// Build `jit` from the real workspace into `target_dir` with the given
/// provenance environment injected, then run `version --json` and return the
/// parsed output. `target_dir` is a dedicated `CARGO_TARGET_DIR`, so the first
/// call into a fresh directory pays a full cold compile and later calls into
/// the same directory rebuild incrementally as the injected environment
/// changes.
fn version_json(target_dir: &Path, env: &[(&str, &str)]) -> Value {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let mut cmd = StdCommand::new("cargo");
    cmd.current_dir(workspace_root)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(["run", "-p", "jit", "--quiet", "--", "version", "--json"]);
    for (key, value) in env {
        cmd.env(key, value);
    }

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "cargo run should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("version output is JSON")
}
