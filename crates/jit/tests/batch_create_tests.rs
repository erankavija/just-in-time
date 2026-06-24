//! Integration tests for `jit issue batch-create --from-json`.
//!
//! Exercises the end-to-end CLI: a valid file creates all issues + edges and
//! returns the `{key:id}` map; each pre-validation failure (duplicate key,
//! unknown dependency, cycle, invalid label/gate/type) exits 2 and creates ZERO
//! issues; created edges are asserted via `jit issue show --json`.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

/// Write a batch JSON file into the repo and return its path.
fn write_batch(temp: &TempDir, json: &str) -> std::path::PathBuf {
    let path = temp.path().join("batch.json");
    std::fs::write(&path, json).unwrap();
    path
}

fn run_batch(temp: &TempDir, file: &std::path::Path, json: bool) -> std::process::Output {
    let mut args = vec![
        "issue".to_string(),
        "batch-create".to_string(),
        "--from-json".to_string(),
        file.to_string_lossy().to_string(),
    ];
    if json {
        args.push("--json".to_string());
    }
    Command::new(jit_binary())
        .args(&args)
        .current_dir(temp.path())
        .output()
        .unwrap()
}

fn count_issues(temp: &TempDir) -> usize {
    let output = Command::new(jit_binary())
        .args(["query", "all", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    v.get("issues")
        .and_then(|i| i.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

#[test]
fn test_batch_create_valid_creates_issues_and_edges() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "spec", "title": "Write the spec", "type": "story" },
          { "key": "impl", "title": "Implement it", "type": "task", "depends_on": ["spec"] },
          { "key": "test", "title": "Test it", "type": "task", "depends_on": ["impl"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, true);
    assert!(
        output.status.success(),
        "batch-create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The success `--json` output is EXACTLY the pure `{key: full_id}` map:
    // every top-level entry is a symbolic key, with no envelope/`message` key.
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = v
        .as_object()
        .expect("top-level value must be the key->id object");
    let keys: std::collections::BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["impl", "spec", "test"].into_iter().collect(),
        "top-level object must contain ONLY the symbolic keys (no 'message'): {stdout}"
    );
    assert!(
        !obj.contains_key("message"),
        "no 'message' key allowed: {stdout}"
    );

    let spec_id = obj.get("spec").unwrap().as_str().unwrap().to_string();
    let impl_id = obj.get("impl").unwrap().as_str().unwrap().to_string();
    let test_id = obj.get("test").unwrap().as_str().unwrap().to_string();
    assert_ne!(spec_id, impl_id);

    // All three issues exist.
    assert_eq!(count_issues(&temp), 3);

    // The edge impl -> spec was wired (assert via `issue show --json`).
    let show = Command::new(jit_binary())
        .args(["issue", "show", &impl_id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let show_out = String::from_utf8_lossy(&show.stdout);
    assert!(
        show_out.contains(&spec_id),
        "impl issue should depend on spec; got: {show_out}"
    );

    // And test -> impl.
    let show_test = Command::new(jit_binary())
        .args(["issue", "show", &test_id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&show_test.stdout).contains(&impl_id));
}

#[test]
fn test_batch_create_duplicate_key_exits_2_zero_created() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "First" },
          { "key": "a", "title": "Dup" }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate key 'a'"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0, "no issues should be created");
}

#[test]
fn test_batch_create_unknown_dependency_exits_2_zero_created() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "First", "depends_on": ["ghost"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost"),
        "stderr should name the ref: {stderr}"
    );
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_cycle_exits_2_zero_created() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "depends_on": ["b"] },
          { "key": "b", "title": "B", "depends_on": ["a"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cycle"),
        "stderr should report a cycle: {stderr}"
    );
    // Both keys named in the cycle.
    assert!(stderr.contains('a') && stderr.contains('b'));
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_invalid_label_exits_2() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "labels": ["NoColonLabel"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // A malformed label is caught by the full write-time validation, attributed
    // to the offending key.
    assert!(stderr.contains("'a' fails validation"), "stderr: {stderr}");
    assert!(stderr.contains("default:label-format"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_unknown_gate_exits_2() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "gates": ["nonexistent-gate"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nonexistent-gate"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_unknown_type_exits_2() {
    let temp = setup_test_repo();
    // The default `jit init` config configures a type hierarchy
    // (milestone/epic/story/task), so an unknown type is rejected.
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "type": "widget" }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown type 'widget'"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_enumerates_multiple_problems() {
    let temp = setup_test_repo();
    // Two independent problems: a duplicate key AND an unknown dependency.
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A" },
          { "key": "a", "title": "Dup", "depends_on": ["ghost"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // BOTH problems enumerated, not just the first.
    assert!(stderr.contains("duplicate key 'a'"), "stderr: {stderr}");
    assert!(stderr.contains("ghost"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_write_time_violation_in_later_entry_zero_created() {
    let temp = setup_test_repo();
    // The FIRST entry is valid. The SECOND has a write-time-only violation: a
    // `type` field PLUS an explicit `type:*` label yields two `type:` labels,
    // which namespace-uniqueness validation rejects only at write time. Without
    // full pre-validation this would save the first issue then fail on the
    // second, leaving a partial write. It must now be caught BEFORE any write.
    let file = write_batch(
        &temp,
        r#"[
          { "key": "ok", "title": "Fine", "type": "task" },
          { "key": "bad", "title": "Bad", "type": "task", "labels": ["type:story"] }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("'bad'"),
        "error should name the offending key 'bad': {stderr}"
    );
    // ZERO issues created: the valid first entry must NOT have been saved.
    assert_eq!(count_issues(&temp), 0, "no issues should be created");
}

#[test]
fn test_batch_create_human_output_lists_key_to_id() {
    let temp = setup_test_repo();
    let file = write_batch(
        &temp,
        r#"[
          { "key": "only", "title": "Only one", "type": "task" }
        ]"#,
    );

    let output = run_batch(&temp, &file, false);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("only ->"), "human output: {stdout}");
}
