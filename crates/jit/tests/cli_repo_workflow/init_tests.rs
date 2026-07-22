//! Integration tests for `jit init`

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
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
    assert!(jit.join("gates.toml").exists(), "gates.toml missing");
    assert!(jit.join("events.jsonl").exists(), "events.jsonl missing");
    assert!(jit.join("config.toml").exists(), "config.toml missing");
}

#[test]
fn test_init_outside_git_scaffolds_rules_without_creating_dot_git() {
    // `jit init` in a directory that is NOT a git repo must still scaffold the
    // default rules.toml, but it must NOT create a bogus `.git` control plane:
    // the scaffold write lock (.git/jit/locks/rules.lock) is git-only, and the
    // non-git path is intentionally lockless. (Regression: gating the lock on
    // `WorktreePaths::detect()` succeeding created `<dir>/.git/jit/locks`,
    // because detect() returns Ok even outside git.)
    // Hermetic by construction: the control plane (and thus the scaffold lock) is
    // derived from the working tree that owns this `.jit` -- i.e. whether
    // `<temp>/.git` exists -- not from the ambient cwd. A fresh TempDir has no
    // `.git` of its own, so the non-git lockless path is taken even when $TMPDIR
    // itself lives inside an unrelated git worktree (as in some CI sandboxes).
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &[]);
    assert!(out.status.success(), "jit init failed: {:?}", out);

    assert!(
        temp.path().join(".jit/rules.toml").exists(),
        "init should scaffold .jit/rules.toml even outside a git repo"
    );
    assert!(
        !temp.path().join(".git").exists(),
        "init outside a git repo must not create a .git directory"
    );
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

// ---------------------------------------------------------------------------
// REQ-03 (jit:1a63ef75): `--json` machine output
// ---------------------------------------------------------------------------

#[test]
fn test_init_json_reports_repository_id_and_created_paths_outside_git() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "repository_root": temp.path().to_string_lossy(),
            "data_dir": temp.path().join(".jit").to_string_lossy(),
            "repository_id": null,
            "hierarchy_template": "default",
            "gitattributes_status": "not_applicable",
            "created_paths": [
                ".jit/index.json",
                ".jit/gates.toml",
                ".jit/events.jsonl",
                ".jit/config.toml",
                ".jit/rules.toml"
            ],
            "modified_paths": [],
            "profile": null,
            "message": "Initialized jit repository"
        })
    );
}

#[test]
fn test_init_json_reports_repository_id_inside_git() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let repository_id = json["repository_id"]
        .as_str()
        .expect("repository_id should be a string inside a git repository");
    assert!(
        repository_id.starts_with("wt:"),
        "repository_id should be a worktree id, got: {repository_id}"
    );
}

// ---------------------------------------------------------------------------
// Review follow-up (jit:1a63ef75 F1/F2): `.gitattributes` created/modified
// reporting, and the atomic-write path it now goes through.
// ---------------------------------------------------------------------------

#[test]
fn test_init_json_inside_git_reports_gitattributes_created() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "created");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        created.contains(&".gitattributes"),
        "created_paths should report .gitattributes on a fresh init inside git, got: {created:?}"
    );
    let modified = json["modified_paths"]
        .as_array()
        .expect("modified_paths should be an array");
    assert!(
        modified.is_empty(),
        "a fresh .gitattributes is created, not modified, got: {modified:?}"
    );

    let content = fs::read_to_string(temp.path().join(".gitattributes")).unwrap();
    assert!(content.contains("# JIT merge drivers"));
}

#[test]
fn test_init_nested_glob_data_dir_claims_only_literal_events_path() {
    let temp = TempDir::new().unwrap();
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let data_dir = r#""lead space/glob*query?[set]"#;
    fs::create_dir(temp.path().join(r#""lead space"#)).unwrap();
    let out = Command::new(jit_binary())
        .arg("init")
        .env("JIT_DATA_DIR", data_dir)
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "jit init failed: {out:?}");

    let attributes = fs::read_to_string(temp.path().join(".gitattributes")).unwrap();
    assert!(
        attributes.contains(
            "\"\\\"lead space/glob\\\\*query\\\\?\\\\[set\\\\]/events.jsonl\" merge=union"
        ),
        "the literal nested data root must be Git-escaped: {attributes:?}"
    );

    let literal = Command::new("git")
        .args(["check-attr", "merge", "--"])
        .arg(format!("{data_dir}/events.jsonl"))
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(literal.status.success());
    assert!(String::from_utf8(literal.stdout)
        .unwrap()
        .ends_with("merge: union\n"));

    let wildcard_neighbor = Command::new("git")
        .args(["check-attr", "merge", "--"])
        .arg(r#""lead space/glob-neighborqueryXs/events.jsonl"#)
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(wildcard_neighbor.status.success());
    assert!(String::from_utf8(wildcard_neighbor.stdout)
        .unwrap()
        .ends_with("merge: unspecified\n"));
}

// Regression (jit:7a60f987): `.jit/claims.jsonl` never gets a merge-driver
// entry. The claim log actually lives under the shared `.git/jit/` control
// plane (see storage/claim_coordinator.rs), never under the versioned
// `.jit/` data plane that `.gitattributes` covers.
#[test]
fn test_init_json_gitattributes_has_no_stale_claims_jsonl_entry() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let content = fs::read_to_string(temp.path().join(".gitattributes")).unwrap();
    assert!(
        !content.contains("claims.jsonl"),
        "claim coordination writes its log under .git/jit/, not .jit/, so \
         .gitattributes should have no merge-driver entry for claims.jsonl, got: {content:?}"
    );
    // Positive control: the events.jsonl entry stays — events.jsonl is
    // actually persisted under the versioned .jit/ data plane.
    assert!(
        content.contains(".jit/events.jsonl merge=union"),
        "the events.jsonl merge-driver entry should still be present, got: {content:?}"
    );
}

// A re-init runs the same session-backed init transaction as a fresh init
// (jit:49adf23b increment 7 unified plain re-init onto `run_initialization`), so
// it too claims the worktree `.gitattributes` merge driver. When the block is
// already configured the claim is `Unchanged`, so a re-init reports
// `.gitattributes` in neither created nor modified.
#[test]
fn test_init_json_reinit_reports_gitattributes_absent_from_created_paths() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let first = jit_init(temp.path(), &["--json"]);
    assert!(first.status.success());

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success());

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "unchanged");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let modified: Vec<&str> = json["modified_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !created.contains(&".gitattributes"),
        "an already-configured .gitattributes should not be reported as created, got: {created:?}"
    );
    assert!(
        !modified.contains(&".gitattributes"),
        "an already-configured .gitattributes should not be reported as modified, got: {modified:?}"
    );
}

// jit:49adf23b increment 7: pin that a PLAIN re-init asserts the worktree
// `.gitattributes` merge-driver claim, not only a fresh init. Before the plain
// re-init was unified onto the session-backed `run_initialization`, a re-init
// reported `.gitattributes` as `NotApplicable` and never restored it. Here the
// first init creates it, the file is then deleted, and the re-init must re-create
// it — proving the claim runs on the ordinary (non-fresh, non-profiled) re-init
// path and that Git evidence, not the fresh/existing-root distinction, gates it.
#[test]
fn test_init_json_reinit_reclaims_deleted_gitattributes() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    let first = jit_init(temp.path(), &["--json"]);
    assert!(first.status.success());
    let gitattributes = temp.path().join(".gitattributes");
    assert!(gitattributes.is_file(), "fresh init creates .gitattributes");
    fs::remove_file(&gitattributes).unwrap();

    // A plain re-init (no template, no profile) over the existing root.
    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "re-init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "created");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        created.contains(&".gitattributes"),
        "a plain re-init must re-create the deleted .gitattributes, got: {created:?}"
    );
    let content = fs::read_to_string(&gitattributes).unwrap();
    assert!(
        content.contains(".jit/events.jsonl merge=union"),
        "the re-created .gitattributes carries the events.jsonl merge driver, got: {content:?}"
    );
}

#[test]
fn test_init_json_appends_to_existing_gitattributes_reports_modified() {
    let temp = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(temp.path())
        .status()
        .unwrap();
    assert!(status.success());

    // Pre-existing .gitattributes without the jit merge-driver block.
    fs::write(temp.path().join(".gitattributes"), "*.txt text\n").unwrap();

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "modified");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let modified: Vec<&str> = json["modified_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !created.contains(&".gitattributes"),
        "an appended .gitattributes should not be reported as created, got: {created:?}"
    );
    assert!(
        modified.contains(&".gitattributes"),
        "an appended .gitattributes should be reported as modified, got: {modified:?}"
    );

    let content = fs::read_to_string(temp.path().join(".gitattributes")).unwrap();
    assert!(content.contains("*.txt text"), "original content preserved");
    assert!(
        content.contains("# JIT merge drivers"),
        "jit block appended"
    );

    // Atomic-write regression: no stray temp sibling left behind.
    let stray: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".gitattributes.") && name.ends_with(".tmp"))
        .collect();
    assert!(
        stray.is_empty(),
        "no .gitattributes.*.tmp sibling should remain, got: {stray:?}"
    );
}

#[test]
fn test_init_json_idempotent_reports_empty_created_paths() {
    let temp = TempDir::new().unwrap();
    let first = jit_init(temp.path(), &["--json"]);
    assert!(first.status.success());

    let out = jit_init(temp.path(), &["--json"]);
    assert!(
        out.status.success(),
        "second jit init --json failed: {:?}",
        out
    );

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["created_paths"], serde_json::json!([]));
    assert_eq!(json["modified_paths"], serde_json::json!([]));
}

#[test]
fn test_init_json_disjoint_data_root_reports_canonical_plan_paths() {
    let worktree = TempDir::new().unwrap();
    let external = TempDir::new().unwrap();
    let data_dir = external.path().join("repository-data");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(worktree.path())
        .status()
        .unwrap();
    assert!(status.success());

    let out = Command::new(jit_binary())
        .args(["init", "--json"])
        .env("JIT_DATA_DIR", &data_dir)
        .current_dir(worktree.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "disjoint init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["data_dir"], data_dir.to_string_lossy().as_ref());
    assert_eq!(json["gitattributes_status"], "not_applicable");
    assert_eq!(
        json["created_paths"],
        serde_json::json!([
            ".jit/index.json",
            ".jit/gates.toml",
            ".jit/events.jsonl",
            ".jit/config.toml",
            ".jit/rules.toml"
        ])
    );
    assert!(!worktree.path().join(".gitattributes").exists());
    assert!(json["modified_paths"].as_array().unwrap().is_empty());
}

#[test]
fn test_init_json_reinit_reports_only_file_restored_by_plan() {
    let temp = TempDir::new().unwrap();
    let first = jit_init(temp.path(), &["--json"]);
    assert!(first.status.success());
    fs::remove_file(temp.path().join(".jit/gates.toml")).unwrap();

    let out = jit_init(temp.path(), &["--json"]);
    assert!(out.status.success(), "re-init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["created_paths"],
        serde_json::json!([".jit/gates.toml"])
    );
    assert_eq!(json["modified_paths"], serde_json::json!([]));
}

#[test]
fn test_init_json_unknown_template_emits_json_error() {
    let temp = TempDir::new().unwrap();
    let out = jit_init(
        temp.path(),
        &["--hierarchy-template", "nonexistent", "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));

    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("--json should emit a structured error object on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown hierarchy template"));
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
