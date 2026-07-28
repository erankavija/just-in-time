//! Integration tests for the `[project]` config table (jit:3d9e9222).
//!
//! Covers `jit init` seeding a canonical project name from the repository
//! directory's basename, idempotency of an already-declared name, and
//! `jit config validate` rejecting an invalid `[project] name`.

use jit::output::ErrorCode;
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
        .env("HOME", temp.path().join("isolated-home"))
        .env_remove("JIT_WORKTREE_MODE")
        .env_remove("JIT_ENFORCE_LEASES")
        .output()
        .expect("failed to run jit config validate");

    assert!(
        !out.status.success(),
        "config validate must fail on an invalid [project] name"
    );
    assert_eq!(
        out.status.code(),
        Some(ErrorCode::ValidationFailed.exit_code().code()),
        "human validation failures must use the registered validation-failed status"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("repo config") && stdout.contains("Failed to parse config.toml"),
        "validate output should report the invalid [project] name as a repo-config error, got: {stdout}"
    );
}

#[test]
fn test_config_validate_json_envelopes_every_invalid_configuration_finding() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path());
    assert!(out.status.success());

    let config_path = temp.path().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"1abc\"\n").unwrap();

    let out = Command::new(jit_binary())
        .args(["config", "validate", "--json"])
        .current_dir(temp.path())
        .env("HOME", temp.path().join("isolated-home"))
        .env("JIT_WORKTREE_MODE", "not-a-worktree-mode")
        .env("JIT_ENFORCE_LEASES", "not-an-enforcement-mode")
        .output()
        .expect("failed to run jit config validate --json");

    assert!(!out.status.success());
    assert_eq!(
        out.status.code(),
        Some(ErrorCode::ValidationFailed.exit_code().code())
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(parsed["error"]["details"]["valid"], false);
    let errors = parsed["error"]["details"]["errors"]
        .as_array()
        .expect("validation details carry an errors array");
    assert_eq!(errors.len(), 3, "every actionable finding is retained");
    for source in ["repo config", "JIT_WORKTREE_MODE", "JIT_ENFORCE_LEASES"] {
        assert!(
            errors
                .iter()
                .any(|error| error.as_str().is_some_and(|error| error.contains(source))),
            "details must retain the {source} finding, got: {errors:?}"
        );
    }
}

#[test]
fn test_config_validate_valid_configuration_keeps_existing_success_output() {
    let temp = TempDir::new().unwrap();
    assert!(jit_init(temp.path()).status.success());

    let human = Command::new(jit_binary())
        .args(["config", "validate"])
        .current_dir(temp.path())
        .env("HOME", temp.path().join("isolated-home"))
        .env_remove("JIT_WORKTREE_MODE")
        .env_remove("JIT_ENFORCE_LEASES")
        .output()
        .expect("run valid human config validation");
    assert_eq!(human.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&human.stdout),
        "✓ Configuration is valid\n"
    );

    let json = Command::new(jit_binary())
        .args(["config", "validate", "--json"])
        .current_dir(temp.path())
        .env("HOME", temp.path().join("isolated-home"))
        .env_remove("JIT_WORKTREE_MODE")
        .env_remove("JIT_ENFORCE_LEASES")
        .output()
        .expect("run valid machine-readable config validation");
    assert_eq!(json.status.code(), Some(0));
    let output: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("valid JSON success report");
    assert_eq!(output["valid"], true);
    assert_eq!(output["errors"], serde_json::json!([]));
    assert_eq!(output["message"], "Configuration is valid");
}

// ---------------------------------------------------------------------------
// REQ-03 (story 9a7106ae): the `jit config set` write path validates
// project.name BEFORE writing — an invalid value never reaches the file.
// ---------------------------------------------------------------------------

#[test]
fn test_config_set_rejects_invalid_project_name_without_writing() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("write-path-repo");
    fs::create_dir(&repo_dir).unwrap();
    jit_init(&repo_dir);
    let before = fs::read_to_string(repo_dir.join(".jit/config.toml")).unwrap();

    let out = Command::new(jit_binary())
        .args(["config", "set", "project.name", "Bad_Name"])
        .current_dir(&repo_dir)
        .output()
        .expect("failed to run jit config set");
    assert!(
        !out.status.success(),
        "config set must reject an invalid project name on write"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Bad_Name"),
        "error names the offending value: {stderr}"
    );

    let after = fs::read_to_string(repo_dir.join(".jit/config.toml")).unwrap();
    assert_eq!(before, after, "a rejected set must not modify the file");
}

#[test]
fn test_config_set_accepts_valid_project_name_and_round_trips() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("write-path-repo");
    fs::create_dir(&repo_dir).unwrap();
    jit_init(&repo_dir);

    let out = Command::new(jit_binary())
        .args(["config", "set", "project.name", "renamed-project"])
        .current_dir(&repo_dir)
        .output()
        .expect("failed to run jit config set");
    assert!(out.status.success(), "valid set failed: {:?}", out);

    let content = fs::read_to_string(repo_dir.join(".jit/config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();
    assert_eq!(
        parsed["project"]["name"].as_str().unwrap(),
        "renamed-project"
    );
}

// REQ-03: the write path validates `[project].name` on EVERY document write, not
// only when `project.name` is the key being set. A set on an UNRELATED key must
// not silently rewrite a document that already holds an invalid name.

#[test]
fn test_config_set_unrelated_key_rejects_preexisting_invalid_project_name() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("write-path-repo");
    fs::create_dir(&repo_dir).unwrap();
    jit_init(&repo_dir);

    // Corrupt the config with an invalid `[project]` name that a later, unrelated
    // set must not persist unvalidated.
    let config_path = repo_dir.join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();
    let before = fs::read_to_string(&config_path).unwrap();

    let out = Command::new(jit_binary())
        .args(["config", "set", "coordination.default_ttl_secs", "900"])
        .current_dir(&repo_dir)
        .output()
        .expect("failed to run jit config set");
    assert!(
        !out.status.success(),
        "setting an unrelated key must fail while [project].name is invalid"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Bad_Name"),
        "error names the offending value: {stderr}"
    );

    let after = fs::read_to_string(&config_path).unwrap();
    assert_eq!(before, after, "a rejected set must not modify the file");
}

#[test]
fn test_config_set_project_name_repairs_preexisting_invalid_name() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("write-path-repo");
    fs::create_dir(&repo_dir).unwrap();
    jit_init(&repo_dir);

    let config_path = repo_dir.join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();

    // Setting `project.name` to a valid value is the repair path: validation runs
    // on the POST-mutation document, so the good replacement passes and persists.
    let out = Command::new(jit_binary())
        .args(["config", "set", "project.name", "good-name"])
        .current_dir(&repo_dir)
        .output()
        .expect("failed to run jit config set");
    assert!(
        out.status.success(),
        "valid project.name repair failed: {:?}",
        out
    );

    let content = fs::read_to_string(&config_path).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();
    assert_eq!(parsed["project"]["name"].as_str().unwrap(), "good-name");
}
