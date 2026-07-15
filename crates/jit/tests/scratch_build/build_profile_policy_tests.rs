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

use std::fs;
use std::path::{Path, PathBuf};

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
    let first_step_pos = script
        .find("run_step fmt")
        .expect("scripts/cargo-ci.sh must run the fmt step");

    assert!(
        export_pos < first_step_pos,
        "CARGO_INCREMENTAL=0 must be exported before the first gate step so it \
         covers every Rust compilation the gate performs (fmt, clippy, test, \
         provenance)"
    );
}
