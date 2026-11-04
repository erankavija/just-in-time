//! Integration tests for `jit config get` (jit:043ae624).
//!
//! `jit config get <dotted.key>` walks the WHOLE configuration surface (type
//! hierarchy, label associations, documentation paths, validation settings,
//! item kinds, namespaces, project identity, version, the worktree and
//! coordination settings) via a generic dotted-path walk
//! over an assembled JSON snapshot, rather than a hand-maintained per-key
//! match. See `crates/jit/src/commands/config.rs` (`resolve_dotted_key`,
//! `ConfigOps::get_config`) and `EffectiveConfig::full_snapshot` in
//! `crates/jit/src/config.rs`.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use std::fs;
use std::str::FromStr;

fn config_get(cli: &JitCli, args: &[&str]) -> std::process::Output {
    cli.command(["config", "get"])
        .args(args)
        .output()
        .expect("failed to run jit config get")
}

const FULL_CONFIG_TOML: &str = r#"
[version]
schema = 1

[project]
name = "sample-project"

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
strictness = "loose"
default_type = "task"

[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies"]
archive_root = "dev/archive"

[namespaces.type]
description = "Issue type"
unique = true
examples = ["bug", "feature"]

[item_kinds.requirement]
section = "success_criteria"
id-pattern = "REQ-\\d+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"

[coordination]
max_indefinite_leases_per_agent = 3
"#;

// ---------------------------------------------------------------------------
// REQ-01: a scalar get, covering sections beyond the old hand-mapped keys.
// ---------------------------------------------------------------------------

#[test]
fn test_get_scalar_from_type_hierarchy() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["type_hierarchy.types.epic"]);
    assert!(out.status.success(), "get failed: {:?}", out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2");
}

#[test]
fn test_get_scalar_from_validation_section() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["validation.strictness"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "loose");
}

#[test]
fn test_get_scalar_from_version_section() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["version.schema"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "1");
}

#[test]
fn test_get_scalar_from_project_section() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["project.name"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "sample-project"
    );
}

// ---------------------------------------------------------------------------
// REQ-01: array get.
// ---------------------------------------------------------------------------

#[test]
fn test_get_array_value_pretty_printed() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["type_hierarchy.strategic_types"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Non-JSON output pretty-prints compound values.
    assert_eq!(stdout.trim(), "[\n  \"milestone\",\n  \"epic\"\n]");
}

// ---------------------------------------------------------------------------
// REQ-01: nested table get (a user-declared map entry, not a struct field).
// ---------------------------------------------------------------------------

#[test]
fn test_get_nested_table_entry_from_namespaces() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["namespaces.type.unique"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "true");
}

#[test]
fn test_get_nested_table_entry_from_item_kinds() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    // Uses the SAME kebab-case spelling config.toml declares.
    let out = config_get(&cli, &["item_kinds.requirement.id-pattern"]);
    assert!(out.status.success(), "get failed: {:?}", out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "REQ-\\d+");
}

// ---------------------------------------------------------------------------
// REQ-01: whole-section get (intermediate key returns the whole subtree).
// ---------------------------------------------------------------------------

#[test]
fn test_get_whole_section_returns_subtree() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["documentation", "--json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["key"], "documentation");
    assert_eq!(parsed["value"]["development_root"], "dev");
    assert_eq!(
        parsed["value"]["managed_paths"],
        serde_json::json!(["dev/active", "dev/studies"])
    );
}

// ---------------------------------------------------------------------------
// REQ-02: an unknown TOP-LEVEL key fails, listing valid sections.
// ---------------------------------------------------------------------------

#[test]
fn test_get_unknown_top_level_key_fails_with_sections() {
    let cli = JitCli::initialised().unwrap();

    let out = config_get(&cli, &["bogus_section"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("valid top-level sections"),
        "stderr should list valid sections: {stderr}"
    );
    // A handful of the sections must be named.
    assert!(stderr.contains("documentation"));
    assert!(stderr.contains("worktree"));
    assert!(stderr.contains("coordination"));
}

#[test]
fn test_get_unknown_top_level_key_json_reports_invalid_argument() {
    let cli = JitCli::initialised().unwrap();

    let out = config_get(&cli, &["bogus_section", "--json"]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["error"]["code"], "INVALID_ARGUMENT");
    assert!(parsed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("valid top-level sections"));
}

// ---------------------------------------------------------------------------
// Unknown NESTED key also fails, naming the missing segment.
// ---------------------------------------------------------------------------

#[test]
fn test_get_unknown_nested_key_fails() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["documentation.bogus_field"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bogus_field") && stderr.contains("documentation"),
        "stderr should name the missing segment and its parent: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// A malformed `config.toml` is a LOAD failure, not an unknown-key argument
// error: it must NOT be reported as `INVALID_ARGUMENT`, in either mode, and `--json` must not force it into a JSON envelope it never had
// before (matching `jit config show` / `jit config validate`'s existing
// behavior for the same underlying failure).
// ---------------------------------------------------------------------------

#[test]
fn test_get_corrupt_config_toml_is_not_invalid_argument() {
    let cli = JitCli::initialised().unwrap();
    // Invalid TOML syntax (unclosed table header).
    fs::write(
        cli.root().join(".jit/config.toml"),
        "[type_hierarchy\nbroken = true\n",
    )
    .unwrap();

    let out = config_get(&cli, &["worktree.enforce_leases"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("valid top-level sections"),
        "a malformed config.toml must not be classified as a bad CLI argument: {stderr}"
    );
    assert!(
        stderr.contains("Failed to parse config.toml"),
        "stderr should surface the load failure: {stderr}"
    );
}

#[test]
fn test_get_corrupt_config_toml_json_emits_registered_parse_error() {
    let cli = JitCli::initialised().unwrap();
    fs::write(
        cli.root().join(".jit/config.toml"),
        "[type_hierarchy\nbroken = true\n",
    )
    .unwrap();

    let out = config_get(&cli, &["worktree.enforce_leases", "--json"]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let envelope: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout contains exactly one JSON document");
    let code = ErrorCode::from_str(
        envelope["error"]["code"]
            .as_str()
            .expect("error envelope carries a code"),
    )
    .expect("error envelope code is registered");
    assert_eq!(code, ErrorCode::ParseError);
    assert!(envelope["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Failed to parse config.toml")));
}

// ---------------------------------------------------------------------------
// `--json` envelope shape for a successful leaf get.
// ---------------------------------------------------------------------------

#[test]
fn test_get_json_envelope_shape() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let out = config_get(&cli, &["validation.default_type", "--json"]);
    assert!(out.status.success(), "get --json failed: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["key"], "validation.default_type");
    assert_eq!(parsed["value"], "task");
}

// ---------------------------------------------------------------------------
// Missing config.toml: `jit config get` still works, resolving to defaults /
// empty sections rather than erroring — matches `jit config show`'s existing
// no-config-file behavior.
// ---------------------------------------------------------------------------

#[test]
fn test_get_on_fresh_repo_without_config_toml_resolves_defaults() {
    let cli = JitCli::initialised().unwrap();
    // `jit init` seeds a minimal config.toml with just `[project]`; remove it
    // entirely to exercise the fully-absent-file path.
    let config_path = cli.root().join(".jit/config.toml");
    fs::remove_file(&config_path).ok();

    // A merged section with a built-in default still resolves.
    let out = config_get(&cli, &["coordination.max_indefinite_leases_per_agent"]);
    assert!(out.status.success(), "get failed: {:?}", out);
    assert!(String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .is_ok());

    // A raw, repo-only section with nothing configured is an intermediate key
    // resolving to an empty object, not an error.
    let out = config_get(&cli, &["documentation", "--json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON output");
    assert_eq!(parsed["value"], serde_json::json!({}));
}

// ---------------------------------------------------------------------------
// `jit config show` renders the snapshot `jit config get` walks.
// ---------------------------------------------------------------------------

#[test]
fn test_show_json_data_equals_every_section_config_get_resolves() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), FULL_CONFIG_TOML).unwrap();

    let json_of = |args: &[&str]| -> serde_json::Value {
        let out = cli
            .command(args)
            .arg("--json")
            .output()
            .expect("failed to run jit");
        assert!(out.status.success(), "{args:?} failed: {out:?}");
        serde_json::from_slice(&out.stdout).expect("valid JSON output")
    };

    let shown = json_of(&["config", "show"]);
    let sections = shown.as_object().expect("show output is an object");
    assert!(sections.len() > 1);
    // `message` is the response envelope's, not a configuration section.
    for (section, value) in sections.iter().filter(|(key, _)| *key != "message") {
        assert_eq!(
            &json_of(&["config", "get", section])["value"],
            value,
            "section {section}"
        );
    }
}
