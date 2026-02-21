//! Integration tests for `jit init`

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
        .to_string_lossy()
        .to_string()
}

fn jit_init(dir: &std::path::Path, extra_args: &[&str]) -> std::process::Output {
    Command::new(jit_binary())
        .arg("init")
        .args(extra_args)
        .current_dir(dir)
        .output()
        .expect("failed to run jit init")
}

// ---------------------------------------------------------------------------
// Basic init
// ---------------------------------------------------------------------------

#[test]
fn test_init_creates_config_toml() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success(), "jit init failed: {:?}", out);

    let config = temp.path().join(".jit/config.toml");
    assert!(
        config.exists(),
        ".jit/config.toml should be created by init"
    );

    let content = fs::read_to_string(&config).unwrap();
    // Should contain the default hierarchy types
    assert!(
        content.contains("milestone"),
        "config should mention milestone"
    );
    assert!(content.contains("epic"), "config should mention epic");
    assert!(content.contains("story"), "config should mention story");
    assert!(content.contains("task"), "config should mention task");
    // Should contain strategic_types
    assert!(
        content.contains("strategic_types"),
        "config should have strategic_types"
    );
    // Should be commented
    assert!(content.contains('#'), "config should have comments");
}

#[test]
fn test_init_creates_required_files() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success());

    let jit = temp.path().join(".jit");
    assert!(jit.join("index.json").exists(), "index.json missing");
    assert!(jit.join("gates.json").exists(), "gates.json missing");
    assert!(jit.join("events.jsonl").exists(), "events.jsonl missing");
    assert!(jit.join("config.toml").exists(), "config.toml missing");
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

#[test]
fn test_init_idempotent_does_not_overwrite_config() {
    let temp = TempDir::new().unwrap();

    // First init
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success());

    // Modify config to a sentinel value
    let config = temp.path().join(".jit/config.toml");
    fs::write(&config, "# CUSTOM SENTINEL\n").unwrap();

    // Second init — should succeed and leave config untouched
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success(), "second jit init failed");

    let content = fs::read_to_string(&config).unwrap();
    assert!(
        content.contains("CUSTOM SENTINEL"),
        "init should not overwrite existing config.toml"
    );
}

#[test]
fn test_init_idempotent_does_not_overwrite_index() {
    let temp = TempDir::new().unwrap();
    jit_init(temp.path(), &[]);

    // Create an issue so index has real data
    Command::new(jit_binary())
        .args(["issue", "create", "-t", "My issue"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let index_before = fs::read_to_string(temp.path().join(".jit/index.json")).unwrap();

    // Second init
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success());

    let index_after = fs::read_to_string(temp.path().join(".jit/index.json")).unwrap();
    assert_eq!(
        index_before, index_after,
        "second init should not reset index.json"
    );
}

// ---------------------------------------------------------------------------
// --hierarchy-template
// ---------------------------------------------------------------------------

#[test]
fn test_init_template_default() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--hierarchy-template", "default"]);
    assert!(
        out.status.success(),
        "init with default template failed: {:?}",
        out
    );

    let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
    assert!(
        content.contains("milestone"),
        "default template should include milestone"
    );
    assert!(
        content.contains("epic"),
        "default template should include epic"
    );
    assert!(
        content.contains("story"),
        "default template should include story"
    );
    assert!(
        content.contains("task"),
        "default template should include task"
    );
}

#[test]
fn test_init_template_agile() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--hierarchy-template", "agile"]);
    assert!(
        out.status.success(),
        "init with agile template failed: {:?}",
        out
    );

    let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
    // The types line should contain "release" and not "milestone"
    let types_line = content
        .lines()
        .find(|l| l.trim_start().starts_with("types ="))
        .expect("config should have a types = line");
    assert!(
        types_line.contains("release"),
        "agile types should include release"
    );
    assert!(
        !types_line.contains("milestone"),
        "agile types should not include milestone"
    );
}

#[test]
fn test_init_template_minimal() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--hierarchy-template", "minimal"]);
    assert!(
        out.status.success(),
        "init with minimal template failed: {:?}",
        out
    );

    let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
    let types_line = content
        .lines()
        .find(|l| l.trim_start().starts_with("types ="))
        .expect("config should have a types = line");
    assert!(
        types_line.contains("milestone"),
        "minimal types should include milestone"
    );
    assert!(
        types_line.contains("task"),
        "minimal types should include task"
    );
    assert!(
        !types_line.contains("story"),
        "minimal types should not include story"
    );
    assert!(
        !types_line.contains("epic"),
        "minimal types should not include epic"
    );
}

#[test]
fn test_init_template_extended() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--hierarchy-template", "extended"]);
    assert!(
        out.status.success(),
        "init with extended template failed: {:?}",
        out
    );

    let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
    assert!(
        content.contains("program"),
        "extended template should include program"
    );
}

#[test]
fn test_init_template_unknown_errors() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--hierarchy-template", "nonexistent"]);
    assert!(
        !out.status.success(),
        "unknown template should fail, but it succeeded"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Unknown hierarchy template"),
        "error message missing, got: {}",
        stderr
    );
}

#[test]
fn test_init_template_idempotent_does_not_overwrite() {
    let temp = TempDir::new().unwrap();

    // First init with template
    let out = jit_init(temp.path(), &["--hierarchy-template", "agile"]);
    assert!(out.status.success());

    // Modify config
    let config = temp.path().join(".jit/config.toml");
    fs::write(&config, "# AGILE CUSTOM\n").unwrap();

    // Second init with a different template — config must not be overwritten
    let out = jit_init(temp.path(), &["--hierarchy-template", "minimal"]);
    assert!(out.status.success());

    let content = fs::read_to_string(&config).unwrap();
    assert!(
        content.contains("AGILE CUSTOM"),
        "second init should not overwrite existing config.toml"
    );
}

// ---------------------------------------------------------------------------
// Generated config is valid TOML
// ---------------------------------------------------------------------------

#[test]
fn test_init_config_is_valid_toml() {
    let temp = TempDir::new().unwrap();
    jit_init(temp.path(), &[]);

    let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
    // toml crate can parse it — use the library directly
    let parsed: Result<toml::Value, _> = toml::from_str(&content);
    assert!(
        parsed.is_ok(),
        "generated config.toml is not valid TOML: {:?}",
        parsed.err()
    );
}

#[test]
fn test_init_template_config_is_valid_toml() {
    for template in &["default", "agile", "minimal", "extended"] {
        let temp = TempDir::new().unwrap();
        jit_init(temp.path(), &["--hierarchy-template", template]);

        let content = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
        let parsed: Result<toml::Value, _> = toml::from_str(&content);
        assert!(
            parsed.is_ok(),
            "config.toml for template '{}' is not valid TOML: {:?}",
            template,
            parsed.err()
        );
    }
}
