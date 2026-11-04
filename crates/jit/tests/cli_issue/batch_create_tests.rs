//! Integration tests for `jit issue batch-create --from-json`.
//!
//! Exercises the end-to-end CLI: a valid file creates all issues + edges and
//! returns the `{key:id}` map; each pre-validation failure (duplicate key,
//! unknown dependency, invalid label/gate/type) fails and creates ZERO
//! issues; created edges are asserted via `jit issue show --json`.

use jit::test_cli::JitCli;
use jit::test_taxonomy::{type_label, TestTaxonomy};

/// The declared leaf type, the one a manifest entry names when it wants the
/// most granular kind of work.
fn leaf_type(taxonomy: &TestTaxonomy) -> &str {
    taxonomy.type_at_level(4)
}

/// A declared container type, distinct from the leaf.
fn container_type(taxonomy: &TestTaxonomy) -> &str {
    taxonomy.type_at_level(3)
}

fn write_batch(temp: &JitCli, json: &str) -> std::path::PathBuf {
    let path = temp.root().join("batch.json");
    std::fs::write(&path, json).unwrap();
    path
}

fn count_issues(temp: &JitCli) -> usize {
    temp.run_json(["query", "all"])
        .unwrap()
        .get("issues")
        .and_then(|i| i.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

#[test]
fn test_batch_create_valid_creates_issues_and_edges() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let (container, leaf) = (
        container_type(&taxonomy.taxonomy),
        leaf_type(&taxonomy.taxonomy),
    );
    let file = write_batch(
        &temp,
        &format!(
            r#"[
          {{ "key": "spec", "title": "Write the spec", "type": "{container}" }},
          {{ "key": "impl", "title": "Implement it", "type": "{leaf}", "depends_on": ["spec"] }},
          {{ "key": "test", "title": "Test it", "type": "{leaf}", "depends_on": ["impl"] }}
        ]"#
        ),
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "batch-create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The success envelope carries the `{key: full_id}` map under `ids`.
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        v["message"].as_str().is_some_and(|m| m.contains('3')),
        "the envelope message counts the created issues: {stdout}"
    );
    let obj = v["ids"]
        .as_object()
        .expect("`ids` must be the key->id object");
    let keys: std::collections::BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["impl", "spec", "test"].into_iter().collect(),
        "`ids` must contain exactly the symbolic keys: {stdout}"
    );

    let spec_id = obj.get("spec").unwrap().as_str().unwrap().to_string();
    let impl_id = obj.get("impl").unwrap().as_str().unwrap().to_string();
    let test_id = obj.get("test").unwrap().as_str().unwrap().to_string();
    assert_ne!(spec_id, impl_id);

    // All three issues exist.
    assert_eq!(count_issues(&temp), 3);

    // The edge impl -> spec was wired (assert via `issue show --json`).
    let show = temp
        .command(["issue", "show", &impl_id, "--json"])
        .output()
        .unwrap();
    let show_out = String::from_utf8_lossy(&show.stdout);
    assert!(
        show_out.contains(&spec_id),
        "impl issue should depend on spec; got: {show_out}"
    );

    // And test -> impl.
    let show_test = temp
        .command(["issue", "show", &test_id, "--json"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&show_test.stdout).contains(&impl_id));
}

#[test]
fn test_batch_create_duplicate_key_fails_zero_created() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "First" },
          { "key": "a", "title": "Dup" }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate key 'a'"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0, "no issues should be created");
}

#[test]
fn test_batch_create_unknown_dependency_fails_zero_created() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "First", "depends_on": ["ghost"] }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost"),
        "stderr should name the ref: {stderr}"
    );
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_invalid_label_fails() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "labels": ["NoColonLabel"] }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // A malformed label is caught by the full write-time validation, attributed
    // to the offending key.
    assert!(stderr.contains("'a' fails validation"), "stderr: {stderr}");
    assert!(stderr.contains("label-format"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_unknown_gate_fails() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "gates": ["nonexistent-gate"] }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nonexistent-gate"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_unknown_type_fails() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    // The repository's own `[type_hierarchy]` declaration is what a type is
    // judged against, so a name it does not carry is rejected.
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A", "type": "widget" }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown type 'widget'"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_enumerates_multiple_problems() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    // Two independent problems: a duplicate key AND an unknown dependency.
    let file = write_batch(
        &temp,
        r#"[
          { "key": "a", "title": "A" },
          { "key": "a", "title": "Dup", "depends_on": ["ghost"] }
        ]"#,
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // BOTH problems enumerated, not just the first.
    assert!(stderr.contains("duplicate key 'a'"), "stderr: {stderr}");
    assert!(stderr.contains("ghost"), "stderr: {stderr}");
    assert_eq!(count_issues(&temp), 0);
}

#[test]
fn test_batch_create_write_time_violation_in_later_entry_zero_created() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    // The FIRST entry is valid. The SECOND has a write-time-only violation: a
    // `type` field PLUS an explicit `type:*` label yields two `type:` labels,
    // which namespace-uniqueness validation rejects only at write time. Without
    // full pre-validation this would save the first issue then fail on the
    // second, leaving a partial write. It must now be caught BEFORE any write.
    let leaf = leaf_type(&taxonomy.taxonomy);
    let container_label = type_label(&taxonomy.taxonomy, 3);
    let file = write_batch(
        &temp,
        &format!(
            r#"[
          {{ "key": "ok", "title": "Fine", "type": "{leaf}" }},
          {{ "key": "bad", "title": "Bad", "type": "{leaf}", "labels": ["{container_label}"] }}
        ]"#
        ),
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
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
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let leaf = leaf_type(&taxonomy.taxonomy);
    let file = write_batch(
        &temp,
        &format!(
            r#"[
          {{ "key": "only", "title": "Only one", "type": "{leaf}" }}
        ]"#
        ),
    );

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("only ->"), "human output: {stdout}");
}

#[test]
fn test_batch_create_dry_run_reports_manifest_without_mutating_repository() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let file = write_batch(
        &temp,
        r#"[
          {"key":"contract","title":"Contract","planning":{"outcome":"Define it"}},
          {"key":"consumer","title":"Consumer","depends_on":["contract"],
           "planning":{"outcome":"Use it"}}
        ]"#,
    );
    let events = std::fs::read(temp.root().join(".jit/events.jsonl")).unwrap();
    let index = std::fs::read(temp.root().join(".jit/index.json")).unwrap();

    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["issue_count"], 2);
    assert_eq!(value["dependency_count"], 1);
    assert_eq!(value["keys"], serde_json::json!(["contract", "consumer"]));
    assert_eq!(count_issues(&temp), 0);
    assert_eq!(
        std::fs::read(temp.root().join(".jit/events.jsonl")).unwrap(),
        events
    );
    assert_eq!(
        std::fs::read(temp.root().join(".jit/index.json")).unwrap(),
        index
    );

    assert!(temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            file.to_str().unwrap(),
            "--json"
        ])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(count_issues(&temp), 2);
}

#[test]
fn test_batch_create_dry_run_human_output_and_validation_error_match_creation() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy.temp);
    let valid = write_batch(&temp, r#"[{"key":"only","title":"Only"}]"#);
    let output = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            valid.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("no changes made"));

    let invalid = write_batch(
        &temp,
        r#"[{"key":"dup","title":"One"},{"key":"dup","title":"Two"}]"#,
    );
    let dry = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            invalid.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();
    let create = temp
        .command([
            "issue",
            "batch-create",
            "--from-json",
            invalid.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!dry.status.success());
    assert_eq!(dry.status.code(), create.status.code());
    assert_eq!(dry.stderr, create.stderr);
    assert_eq!(count_issues(&temp), 0);
}
