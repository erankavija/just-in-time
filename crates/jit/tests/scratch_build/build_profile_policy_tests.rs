//! Regression coverage for jit:57d0eb79's build-profile and
//! incremental-compilation policy.
//!
//! REQ-01/02: the workspace manifest bounds `[profile.dev]`/`[profile.test]`
//! debug info to `line-tables-only` and states the incremental-compilation
//! intent explicitly, rather than relying on Cargo's undocumented default, so
//! ordinary interactive builds keep line-number backtraces without embedding a
//! full debugger payload in every test executable.
//! REQ-03: `scripts/cargo-ci.sh` overrides that interactive default and
//! disables incremental compilation for every Rust compilation step the gate
//! runs, so broad gate builds do not accumulate incremental state.
//! REQ-04: the gate makes that property deterministic itself — a dedicated
//! step fails the run when a non-empty `incremental` directory remains under
//! the target directory the run actually used, rather than relying on a
//! one-off manual isolated-run observation.
//!
//! jit:6d10e5d4 added two more policies this file covers. First, the
//! dependency-package override `[profile.dev.package."*"].opt-level = 1`:
//! third-party dependency code is compiled once and then executed by every
//! test process, so leaving it unoptimized is repeated cost, and level 1 was
//! the measured selection over level 2
//! (dev/benchmarks/dependency-profile-6d10e5d4/). Second, that issue's REQ-04
//! requires the profiler that measures these very policies to pass its own
//! `--self-test` mode; this file re-invokes
//! `scripts/profile-test-suite.sh --self-test` so that evidence is
//! re-established on every suite run rather than attested once.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

fn workspace_manifest() -> toml::Value {
    let text =
        fs::read_to_string(workspace_root().join("Cargo.toml")).expect("read workspace Cargo.toml");
    toml::from_str(&text).expect("workspace Cargo.toml must be valid TOML")
}

fn profile_table(manifest: &toml::Value, profile: &str) -> toml::Value {
    manifest
        .get("profile")
        .and_then(|p| p.get(profile))
        .unwrap_or_else(|| panic!("workspace Cargo.toml has no [profile.{profile}] table"))
        .clone()
}

#[test]
fn test_dev_and_test_profiles_bound_debug_info_to_line_tables_only() {
    let manifest = workspace_manifest();

    for profile in ["dev", "test"] {
        let table = profile_table(&manifest, profile);
        assert_eq!(
            table.get("debug").and_then(|v| v.as_str()),
            Some("line-tables-only"),
            "[profile.{profile}].debug must stay \"line-tables-only\": full \
             debug info bloats every test executable with a debugger payload \
             most gate runs never use"
        );
    }
}

#[test]
fn test_dev_and_test_profiles_state_incremental_intent_explicitly() {
    let manifest = workspace_manifest();

    for profile in ["dev", "test"] {
        let table = profile_table(&manifest, profile);
        assert_eq!(
            table.get("incremental").and_then(|v| v.as_bool()),
            Some(true),
            "[profile.{profile}].incremental must explicitly state the \
             interactive default (true) rather than leaving Cargo's default \
             undocumented"
        );
    }
}

#[test]
fn test_cargo_ci_disables_incremental_compilation_before_the_first_step() {
    let script = fs::read_to_string(workspace_root().join("scripts/cargo-ci.sh"))
        .expect("read scripts/cargo-ci.sh");

    let export_pos = script
        .find("export CARGO_INCREMENTAL=0")
        .unwrap_or_else(|| panic!("scripts/cargo-ci.sh must export CARGO_INCREMENTAL=0"));
    let preflight_pos = script
        .find("run_step incremental-preflight")
        .expect("scripts/cargo-ci.sh must check existing incremental state first");
    let first_step_pos = script
        .find("run_step fmt")
        .expect("scripts/cargo-ci.sh must run the fmt step");

    assert!(
        export_pos < preflight_pos && preflight_pos < first_step_pos,
        "CARGO_INCREMENTAL=0 must be exported before the first gate step so it \
         covers every Rust compilation the gate performs (fmt, clippy, test, \
         provenance)"
    );
    assert!(
        script[preflight_pos..first_step_pos].contains("if [ \"$failed\" -ne 0 ]"),
        "a failed incremental-state preflight must exit before the expensive \
         fmt, clippy, test, provenance, and budget steps"
    );
}

#[test]
fn test_cargo_ci_fails_the_gate_on_non_empty_incremental_state_after_compiling() {
    let script = fs::read_to_string(workspace_root().join("scripts/cargo-ci.sh"))
        .expect("read scripts/cargo-ci.sh");

    let step_pos = script
        .find("run_step incremental-state")
        .unwrap_or_else(|| {
            panic!(
                "scripts/cargo-ci.sh must run a deterministic \"incremental-state\" \
             gate step; exporting CARGO_INCREMENTAL=0 alone does not verify \
             that no incremental state came back"
            )
        });
    let test_step_pos = script
        .find("run_step test ")
        .expect("scripts/cargo-ci.sh must run the test step");
    assert!(
        step_pos > test_step_pos,
        "the incremental-state check must run after compilation (the test \
         step), so it observes what the gate's own builds actually left \
         behind on disk"
    );

    assert!(
        script.contains("-name incremental") && script.contains("-not -empty"),
        "the incremental-state step must search for non-empty `incremental` \
         directories (the actual regression signal), not merely assert that \
         a variable is exported"
    );
}

#[test]
fn test_cargo_build_scripts_share_the_host_lock_and_enable_available_sccache() {
    let root = workspace_root();
    let gate =
        fs::read_to_string(root.join("scripts/cargo-ci.sh")).expect("read scripts/cargo-ci.sh");

    assert!(
        gate.contains("${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock"),
        "the gate must default to the cross-repository host build lock"
    );
    assert!(
        gate.contains("CARGO_CI_NO_SCCACHE")
            && gate.contains("[ -z \"${RUSTC_WRAPPER:-}\" ]")
            && gate.contains("command -v sccache")
            && gate.contains("export RUSTC_WRAPPER=sccache"),
        "the gate must use host sccache when available while preserving an explicit wrapper and an opt-out"
    );
    assert!(
        gate.contains("[ \"${1:-}\" = \"--cargo\" ]")
            && gate.contains("exec cargo \"$@\""),
        "focused Cargo checks must be able to reuse the gate's host lock and cache setup without running the full gate"
    );

    for script in [
        "scripts/benchmark-rust-build.sh",
        "scripts/benchmark-session-cost.sh",
    ] {
        let contents = fs::read_to_string(root.join(script))
            .unwrap_or_else(|error| panic!("read {script}: {error}"));
        assert!(
            contents.contains("${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock"),
            "{script} must serialize against the same cross-repository host build lock as cargo-ci.sh"
        );
    }
}

#[test]
fn test_dev_profile_package_override_sets_dependency_opt_level_to_one() {
    let manifest = workspace_manifest();
    let dev = profile_table(&manifest, "dev");
    let opt_level = dev
        .get("package")
        .and_then(|package| package.get("*"))
        .and_then(|wildcard| wildcard.get("opt-level"))
        .and_then(toml::Value::as_integer);

    assert_eq!(
        opt_level,
        Some(1),
        "[profile.dev.package.\"*\"].opt-level must stay 1: dependencies are \
         compiled once and then executed by every test process, so their \
         unoptimized code is repeated cost, and level 1 was the measured \
         selection over level 2 (dev/benchmarks/dependency-profile-6d10e5d4/)"
    );
}

#[test]
fn test_profile_test_suite_script_passes_its_own_self_test() {
    let workspace = workspace_root();
    let script = workspace.join("scripts/profile-test-suite.sh");
    let self_test = Command::new(&script)
        .arg("--self-test")
        .current_dir(&workspace)
        .output()
        .expect("run scripts/profile-test-suite.sh --self-test");

    assert!(
        self_test.status.success()
            && String::from_utf8_lossy(&self_test.stdout)
                .contains("profile-test-suite: self-test passed"),
        "profiler self-test must pass: stdout={} stderr={}",
        String::from_utf8_lossy(&self_test.stdout),
        String::from_utf8_lossy(&self_test.stderr)
    );
}
