use serde_json::Value;
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn jit(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run jit")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn test_profile_list_and_show_work_without_repository() {
    let repo = TempDir::new().unwrap();

    let list = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(list.status.success(), "{list:?}");
    let list = json(&list);
    assert_eq!(list["count"], 1);
    assert_eq!(list["profiles"][0]["id"], "jit-dogfood");
    assert_eq!(list["profiles"][0]["applied"], false);

    let show = jit(repo.path(), &["profile", "show", "jit-dogfood", "--json"]);
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["manifest"]["profile"]["id"], "jit-dogfood");
    assert_eq!(show["origin"], serde_json::json!({ "source": "embedded" }));
    assert!(show["package_hash"].as_str().unwrap().len() >= 64);
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profile_unknown_id_has_typed_json_error_without_mutation() {
    let repo = TempDir::new().unwrap();
    let output = jit(
        repo.path(),
        &["init", "--profile", "missing-profile", "--json"],
    );

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_NOT_FOUND");
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profiled_init_publishes_valid_repo_and_applied_inventory() {
    let repo = TempDir::new().unwrap();
    let init = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(
        init.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );
    let init = json(&init);
    assert_eq!(init["profile"]["status"], "applied");
    assert!(repo.path().join(".jit/profiles/jit-dogfood.json").is_file());
    assert!(repo
        .path()
        .join(".agents/skills/jit-manage/SKILL.md")
        .is_file());
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    let validate = jit(repo.path(), &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);

    let list = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(list.status.success(), "{list:?}");
    assert_eq!(json(&list)["profiles"][0]["applied"], true);
}

#[test]
fn test_validate_plain_and_json_report_installed_profile_drift() {
    let repo = TempDir::new().unwrap();
    assert!(
        jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"])
            .status
            .success()
    );
    fs::write(
        repo.path().join(".agents/skills/jit-manage/SKILL.md"),
        "STALE\n",
    )
    .unwrap();

    let plain = jit(repo.path(), &["validate"]);
    assert!(!plain.status.success(), "{plain:?}");
    assert!(
        String::from_utf8_lossy(&plain.stdout).contains("jit-manage/SKILL.md"),
        "stdout={}",
        String::from_utf8_lossy(&plain.stdout)
    );

    let structured = jit(repo.path(), &["validate", "--json"]);
    assert!(!structured.status.success(), "{structured:?}");
    let structured = json(&structured);
    let details = &structured["error"]["details"];
    assert_eq!(details["valid"], false);
    assert!(details["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("jit-manage/SKILL.md")));
}

#[test]
fn test_profile_apply_dry_run_is_read_only_then_apply_is_exact_no_op() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let preview = jit(
        repo.path(),
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["status"], "would_apply");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_before
    );
    assert!(!repo.path().join(".jit/profiles").exists());

    let applied = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(json(&applied)["status"], "applied");
    let events_after = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let unchanged = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(json(&unchanged)["status"], "unchanged");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_after
    );
}

#[test]
fn test_profile_reapply_repairs_missing_and_stale_default_schemas_before_no_op() {
    let repo = TempDir::new().unwrap();
    let initialized = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(initialized.status.success(), "{initialized:?}");

    let namespace_schema = repo
        .path()
        .join(".jit/schemas/default-namespace-registry.json");
    let type_schema = repo
        .path()
        .join(".jit/schemas/default-type-hierarchy-known.json");
    let expected_namespace = fs::read(&namespace_schema).unwrap();
    let expected_types = fs::read(&type_schema).unwrap();
    let events_path = repo.path().join(".jit/events.jsonl");
    let event_count_before = fs::read_to_string(&events_path).unwrap().lines().count();

    fs::remove_file(&namespace_schema).unwrap();
    let preview = jit(
        repo.path(),
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["status"], "would_apply");
    assert!(!namespace_schema.exists(), "dry-run must remain read-only");

    let repaired_missing = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(repaired_missing.status.success(), "{repaired_missing:?}");
    assert_eq!(json(&repaired_missing)["status"], "applied");
    assert_eq!(fs::read(&namespace_schema).unwrap(), expected_namespace);

    fs::write(&type_schema, b"stale\n").unwrap();
    let repaired_stale = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(repaired_stale.status.success(), "{repaired_stale:?}");
    assert_eq!(json(&repaired_stale)["status"], "applied");
    assert_eq!(fs::read(&type_schema).unwrap(), expected_types);

    let events_after_repairs = fs::read(&events_path).unwrap();
    assert_eq!(
        std::str::from_utf8(&events_after_repairs)
            .unwrap()
            .lines()
            .count(),
        event_count_before + 2
    );
    let unchanged = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(json(&unchanged)["status"], "unchanged");
    assert_eq!(fs::read(events_path).unwrap(), events_after_repairs);
}

#[test]
fn test_profiled_init_conflict_leaves_no_jit_and_preserves_occupant() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    let occupant = repo.path().join("contrib/gates/ai-review.sh");
    fs::write(&occupant, b"local script\n").unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(fs::read(&occupant).unwrap(), b"local script\n");
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_existing_partial_profiled_init_conflict_does_not_plain_init_first() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    fs::write(
        repo.path().join("contrib/gates/ai-review.sh"),
        b"local script\n",
    )
    .unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index
    );
    for path in [
        "gates.toml",
        "events.jsonl",
        "config.toml",
        "rules.toml",
        "issues",
    ] {
        assert!(
            !repo.path().join(".jit").join(path).exists(),
            "profile preflight failure must not scaffold {path}"
        );
    }
}

#[test]
fn test_existing_partial_profiled_init_atomically_completes_neutral_scaffold() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for path in [
        "gates.toml",
        "events.jsonl",
        "config.toml",
        "rules.toml",
        "issues",
        "profiles/jit-dogfood.json",
    ] {
        assert!(
            repo.path().join(".jit").join(path).exists(),
            "missing {path}"
        );
    }
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index,
        "existing neutral bytes must be preserved"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(jit(repo.path(), &["validate", "--json"]).status.success());
}
