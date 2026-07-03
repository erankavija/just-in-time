//! Integration tests for the `[project]` config table (jit:3d9e9222).
//!
//! Covers `jit init` seeding a canonical project name from the repository
//! directory's basename, idempotency of an already-declared name, and
//! `jit config validate` rejecting an invalid `[project] name`.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn jit_init(dir: &std::path::Path) -> std::process::Output {
    Command::new(jit_binary())
        .arg("init")
        .current_dir(dir)
        .output()
        .expect("failed to run jit init")
}

// ---------------------------------------------------------------------------
// REQ-01: `jit init` seeds `[project] name` from the directory basename.
// ---------------------------------------------------------------------------

#[test]
fn test_init_seeds_project_name_from_slugified_dirname() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("My Cool Project!");
    fs::create_dir(&repo_dir).unwrap();

    let out = jit_init(&repo_dir);
    assert!(out.status.success(), "jit init failed: {:?}", out);

    let content = fs::read_to_string(repo_dir.join(".jit/config.toml")).unwrap();
    assert!(
        content.contains("[project]"),
        "config should have a [project] table"
    );

    let parsed: toml::Value = toml::from_str(&content).expect("config.toml must be valid TOML");
    let name = parsed["project"]["name"]
        .as_str()
        .expect("[project].name must be a string");
    assert_eq!(name, "my-cool-project");
}

#[test]
fn test_init_seeds_fallback_project_name_for_digit_leading_dirname() {
    let temp = TempDir::new().unwrap();
    // A basename starting with a digit can never slugify into something
    // matching ^[a-z][a-z0-9-]*$, so the seeded name must fall back.
    let repo_dir = temp.path().join("123-repo");
    fs::create_dir(&repo_dir).unwrap();

    let out = jit_init(&repo_dir);
    assert!(out.status.success(), "jit init failed: {:?}", out);

    let content = fs::read_to_string(repo_dir.join(".jit/config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();
    assert_eq!(parsed["project"]["name"].as_str().unwrap(), "project");
}

#[test]
fn test_init_seeded_project_name_round_trips_through_jitconfig_load() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("Round Trip Repo");
    fs::create_dir(&repo_dir).unwrap();

    let out = jit_init(&repo_dir);
    assert!(out.status.success());

    let config = jit::config::JitConfig::load(&repo_dir.join(".jit")).unwrap();
    assert_eq!(
        config
            .project
            .as_ref()
            .unwrap()
            .name
            .as_ref()
            .unwrap()
            .as_str(),
        "round-trip-repo"
    );
}

// ---------------------------------------------------------------------------
// REQ-03: re-running `jit init` must not touch an existing `[project]` table.
// ---------------------------------------------------------------------------

#[test]
fn test_init_second_run_preserves_existing_project_name() {
    let temp = TempDir::new().unwrap();

    let out = jit_init(temp.path());
    assert!(out.status.success());

    let config_path = temp.path().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"existing-name\"\n").unwrap();

    let out = jit_init(temp.path());
    assert!(out.status.success(), "second jit init failed");

    let content = fs::read_to_string(&config_path).unwrap();
    assert!(
        content.contains("name = \"existing-name\""),
        "second init must not touch an existing [project] table, got: {content}"
    );
}

// ---------------------------------------------------------------------------
// REQ-02: an invalid `[project] name` fails config load and `config validate`.
// ---------------------------------------------------------------------------

#[test]
fn test_config_validate_reports_invalid_project_name() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path());
    assert!(out.status.success());

    let config_path = temp.path().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();

    let out = Command::new(jit_binary())
        .args(["config", "validate"])
        .current_dir(temp.path())
        .output()
        .expect("failed to run jit config validate");

    assert!(
        !out.status.success(),
        "config validate must fail on an invalid [project] name"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("repo config") && stdout.contains("Failed to parse config.toml"),
        "validate output should report the invalid [project] name as a repo-config error, got: {stdout}"
    );
}

#[test]
fn test_config_validate_json_reports_invalid_project_name() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path());
    assert!(out.status.success());

    let config_path = temp.path().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"1abc\"\n").unwrap();

    let out = Command::new(jit_binary())
        .args(["config", "validate", "--json"])
        .current_dir(temp.path())
        .output()
        .expect("failed to run jit config validate --json");

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["valid"], false);
    let errors = parsed["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().unwrap().contains("repo config")),
        "errors should report the invalid [project] name as a repo-config error, got: {errors:?}"
    );
}
