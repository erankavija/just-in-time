//! Integration tests for the `[project]` config table (jit:3d9e9222).
//!
//! Covers `jit init` seeding a canonical project name from the repository
//! directory's basename, idempotency of an already-declared name, and
//! `jit config validate` rejecting an invalid `[project] name`.

use jit::storage::configuration_store::load_configuration;
use jit::test_cli::JitCli;
use std::fs;
use tempfile::{Builder, TempDir};

/// Replace `[project] name` in a scaffolded config, leaving every other
/// declaration in place so the rules and schemas derived from them stay current.
fn rewrite_project_name(config_path: &std::path::Path, name: &str) {
    let content = fs::read_to_string(config_path).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();
    let current = parsed["project"]["name"].as_str().unwrap();
    let rewritten = content.replacen(
        &format!("name = \"{current}\""),
        &format!("name = \"{name}\""),
        1,
    );
    fs::write(config_path, rewritten).unwrap();
}

// ---------------------------------------------------------------------------
// REQ-01: `jit init` seeds `[project] name` from the directory basename.
// ---------------------------------------------------------------------------

#[test]
fn test_init_seeds_project_name_from_slugified_dirname() {
    let parent = TempDir::new().unwrap();
    let cli = JitCli::with_repository(
        Builder::new()
            .prefix("My Cool Project!")
            .rand_bytes(0)
            .tempdir_in(parent.path())
            .unwrap(),
    );
    cli.run(["init"]).unwrap();

    let content = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
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
    // A basename starting with a digit can never slugify into something
    // matching ^[a-z][a-z0-9-]*$, so the seeded name must fall back.
    let parent = TempDir::new().unwrap();
    let cli = JitCli::with_repository(
        Builder::new()
            .prefix("123-repo")
            .rand_bytes(0)
            .tempdir_in(parent.path())
            .unwrap(),
    );
    cli.run(["init"]).unwrap();

    let content = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();
    assert_eq!(parsed["project"]["name"].as_str().unwrap(), "project");
}

#[test]
fn test_init_seeded_project_name_round_trips_through_jitconfig_load() {
    let parent = TempDir::new().unwrap();
    let cli = JitCli::with_repository(
        Builder::new()
            .prefix("Round Trip Repo")
            .rand_bytes(0)
            .tempdir_in(parent.path())
            .unwrap(),
    );
    cli.run(["init"]).unwrap();

    let config = load_configuration(&jit::test_utils::repository_layout_for_data_root(
        &cli.root().join(".jit"),
    ))
    .unwrap();
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
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    rewrite_project_name(&config_path, "existing-name");

    cli.run(["init"]).expect("second jit init failed");

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
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();

    let out = cli
        .command_with_env(
            ["config", "validate"],
            [("HOME", cli.root().join("isolated-home"))],
        )
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
fn test_config_validate_human_failure_lists_findings_on_stdout_and_summary_on_stderr() {
    let cli = JitCli::initialised().unwrap();
    fs::write(
        cli.root().join(".jit/config.toml"),
        "[project]\nname = \"Bad_Name\"\n",
    )
    .unwrap();

    let out = cli
        .command_with_env(
            ["config", "validate"],
            [("HOME", cli.root().join("isolated-home"))],
        )
        .output()
        .expect("failed to run jit config validate");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.starts_with("error [config] repo config:"),
        "each error is a finding line with severity and rule, got: {stdout}"
    );
    assert!(
        stderr.contains("Configuration validation failed with 1 error(s)"),
        "the failure summary goes to stderr, got: {stderr}"
    );
    assert!(!stdout.contains("Errors:"));
}

#[test]
fn test_config_validate_json_envelopes_every_invalid_configuration_finding() {
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"1abc\"\n").unwrap();

    let out = cli
        .command_with_env(
            ["config", "validate", "--json"],
            [("HOME", cli.root().join("isolated-home"))],
        )
        .output()
        .expect("failed to run jit config validate --json");

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(parsed["error"]["details"]["valid"], false);
    let errors = parsed["error"]["details"]["errors"]
        .as_array()
        .expect("validation details carry an errors array");
    assert!(
        errors.iter().any(|error| error
            .as_str()
            .is_some_and(|error| error.contains("repo config"))),
        "details must retain the repo config finding, got: {errors:?}"
    );
}

#[test]
fn test_config_validate_valid_configuration_keeps_existing_success_output() {
    let cli = JitCli::initialised().unwrap();

    let human = cli
        .command_with_env(
            ["config", "validate"],
            [("HOME", cli.root().join("isolated-home"))],
        )
        .output()
        .expect("run valid human config validation");
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8_lossy(&human.stdout),
        "Configuration is valid\n"
    );

    let json = cli
        .command_with_env(
            ["config", "validate", "--json"],
            [("HOME", cli.root().join("isolated-home"))],
        )
        .output()
        .expect("run valid machine-readable config validation");
    assert!(json.status.success());
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
    let cli = JitCli::initialised().unwrap();
    let before = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();

    let out = cli
        .command(["config", "set", "project.name", "Bad_Name"])
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

    let after = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
    assert_eq!(before, after, "a rejected set must not modify the file");
}

/// `config set` parses the edited document as configuration and refuses an edit
/// that would not load as an invalid argument, leaving the file untouched.
#[test]
fn test_config_set_rejects_unknown_key_without_writing() {
    let cli = JitCli::initialised().unwrap();
    let repo_config = cli.root().join(".jit/config.toml");
    let before = fs::read_to_string(&repo_config).unwrap();

    let out = cli
        .command_with_env(
            ["config", "set", "coordination.undeclared_key", "x"],
            [("HOME", cli.root().join("isolated-home"))],
        )
        .output()
        .expect("failed to run jit config set");
    assert!(!out.status.success(), "an unknown key is rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("undeclared_key"),
        "error names the key: {stderr}"
    );

    assert_eq!(fs::read_to_string(&repo_config).unwrap(), before);
}

/// The user's home carries no configuration layer: a `~/.config/jit/config.toml`
/// (here both malformed and holding overriding values) changes no command's
/// output, and `config set` has no global scope.
#[test]
fn test_user_home_config_toml_has_no_effect_on_any_command() {
    let cli = JitCli::initialised().unwrap();
    let home_dir = TempDir::new().unwrap();
    let home = home_dir.path().join("home");
    fs::create_dir_all(home.join(".config/jit")).unwrap();
    let user_config = home.join(".config/jit/config.toml");

    let run = |args: &[&str]| {
        cli.command_with_env(args, [("HOME", &home)])
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("failed to run jit")
    };
    let observe = || {
        [
            "config show --json",
            "config validate --json",
            "config get worktree.enforce_leases --json",
        ]
        .map(|line| {
            let args: Vec<&str> = line.split(' ').collect();
            let out = run(&args);
            (out.status.success(), out.stdout)
        })
    };

    let baseline = observe();
    assert!(baseline[1].0, "baseline repository is valid");

    let overriding = "[worktree]\nenforce_leases = \"off\"\n\n[coordination]\n\
                      max_indefinite_leases_per_agent = 99\nmax_indefinite_leases_per_repo = 99\n";
    for content in [overriding, "not = valid = toml"] {
        fs::write(&user_config, content).unwrap();
        assert_eq!(
            observe(),
            baseline,
            "user config {content:?} changed output"
        );
    }

    let before = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
    let out = run(&[
        "config",
        "set",
        "coordination.max_indefinite_leases_per_agent",
        "3",
        "--global",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && stderr.contains("unexpected argument '--global'"),
        "`--global` is no longer an option: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(&user_config).unwrap(),
        "not = valid = toml"
    );
}

#[test]
fn test_config_set_accepts_valid_project_name_and_round_trips() {
    let cli = JitCli::initialised().unwrap();

    let out = cli
        .command(["config", "set", "project.name", "renamed-project"])
        .output()
        .expect("failed to run jit config set");
    assert!(out.status.success(), "valid set failed: {:?}", out);

    let content = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
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
    let cli = JitCli::initialised().unwrap();

    // Corrupt the config with an invalid `[project]` name that a later, unrelated
    // set must not persist unvalidated.
    let config_path = cli.root().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();
    let before = fs::read_to_string(&config_path).unwrap();

    let out = cli
        .command([
            "config",
            "set",
            "coordination.max_indefinite_leases_per_agent",
            "3",
        ])
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
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    fs::write(&config_path, "[project]\nname = \"Bad_Name\"\n").unwrap();

    // Setting `project.name` to a valid value is the repair path: validation runs
    // on the POST-mutation document, so the good replacement passes and persists.
    let out = cli
        .command(["config", "set", "project.name", "good-name"])
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

#[cfg(unix)]
#[test]
fn test_config_validate_needs_only_initialized_data_without_a_code_layout() {
    let cli = JitCli::initialised().unwrap();
    let data = cli.root().join(".jit");
    let git = jit::test_harness::git::GitStandIn::new();
    let output = cli
        .command_with_env(["config", "validate", "--json"], [("JIT_DATA_DIR", &data)])
        .current_dir(&data)
        .env("PATH", git.search_path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], true);
    assert!(
        git.invocations().is_empty(),
        "data-only diagnostics need no code Git discovery"
    );
}

#[test]
fn test_config_validate_checks_sibling_declarations_when_config_is_absent() {
    let cli = JitCli::initialised().unwrap();
    let data = cli.root().join(".jit");
    fs::remove_file(data.join("config.toml")).unwrap();
    for source in ["templates.toml", "invariants.toml"] {
        fs::write(data.join("templates.toml"), "").unwrap();
        fs::write(data.join("invariants.toml"), "").unwrap();
        assert!(cli
            .command(["config", "validate"])
            .output()
            .unwrap()
            .status
            .success());
        fs::write(data.join(source), "not = valid = toml").unwrap();
        let output = cli
            .command(["config", "validate", "--json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(jit::output::ErrorCode::ValidationFailed.exit_code() as i32)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["error"]["details"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message.as_str().unwrap().contains(source)));
    }
}

#[test]
fn test_config_validate_explicit_root_input_precedes_authored_findings() {
    use jit::output::ErrorCode;
    let cli = JitCli::initialised().unwrap();
    for (declaration, selected, expected) in [
        (
            "broken = [",
            Some(cli.root().to_path_buf()),
            Some(ErrorCode::ValidationFailed),
        ),
        (
            "broken = [",
            Some(cli.root().join("missing-code")),
            Some(ErrorCode::InvalidArgument),
        ),
        (
            "[worktree]\nroot = 'missing'\n",
            None,
            Some(ErrorCode::InvalidArgument),
        ),
        (
            "[worktree]\nroot = 'missing'\n",
            Some(cli.root().to_path_buf()),
            None,
        ),
        (
            "[worktree]\nroot = 42\n",
            Some(cli.root().to_path_buf()),
            Some(ErrorCode::ValidationFailed),
        ),
    ] {
        fs::write(cli.root().join(".jit/config.toml"), declaration).unwrap();
        let mut command = cli.command(["config", "validate", "--json"]);
        if let Some(root) = selected {
            command.env("JIT_WORKTREE_ROOT", root);
        }
        let output = command.output().unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(expected) = expected {
            assert_eq!(report["error"]["code"], expected.as_str(), "{report}");
            assert_eq!(output.status.code(), Some(expected.exit_code() as i32));
        } else {
            assert!(output.status.success(), "{report}");
            assert_eq!(report["valid"], true);
        }
    }
}

#[test]
fn test_config_validate_pending_data_precedes_invalid_root_and_authored_findings() {
    use jit::repository_state::{FileMode, VirtualPath};
    use jit::storage::{
        commit_redo_record, DataRoot, PutFile, RedoRecord, RECORD_FILE_NAME, RECORD_SCRATCH_DIR,
    };
    let cli = JitCli::initialised().unwrap();
    let data = cli.root().join(".jit");
    let before = fs::read(data.join("config.toml")).unwrap();
    let record = RedoRecord::new(
        Vec::new(),
        vec![PutFile {
            path: VirtualPath::CONFIG,
            bytes: b"broken = [".to_vec(),
            mode: FileMode::Regular,
        }],
        Vec::new(),
        None,
    )
    .unwrap();
    commit_redo_record(&DataRoot::open(&data).unwrap(), &record).unwrap();
    let record_path = data.join(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME);
    let committed = fs::read(&record_path).unwrap();
    let output = cli
        .command_with_env(
            ["config", "validate", "--json"],
            [("JIT_WORKTREE_ROOT", cli.root().join("missing-code"))],
        )
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["error"]["code"],
        jit::output::ErrorCode::RetryableConflict.as_str()
    );
    assert_eq!(fs::read(record_path).unwrap(), committed);
    assert_eq!(fs::read(data.join("config.toml")).unwrap(), before);
}

#[test]
fn test_config_validate_missing_index_is_not_an_authored_finding() {
    let cli = JitCli::initialised().unwrap();
    let data = cli.root().join(".jit");
    fs::remove_file(data.join("index.json")).unwrap();
    fs::write(data.join("config.toml"), "broken = [").unwrap();
    let output = cli
        .command(["config", "validate", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(
        report["error"]["code"],
        jit::output::ErrorCode::ValidationFailed.as_str()
    );
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("index.json"));
}

#[test]
fn test_config_validate_declaration_capability_error_is_not_a_finding() {
    let cli = JitCli::initialised().unwrap();
    let config = cli.root().join(".jit/config.toml");
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    let output = cli
        .command(["config", "validate", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(
        report["error"]["code"],
        jit::output::ErrorCode::ValidationFailed.as_str()
    );
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("config.toml"));
}
