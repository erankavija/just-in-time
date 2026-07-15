//! End-to-end coverage for container child rollup and state aggregation
//! (jit:7fe5c743):
//! - `jit issue children <id>` lists a container's DIRECT children (its depth-1
//!   dependencies), each rendered exactly like `issue status`.
//! - `jit issue progress <id>` reports counts by state plus a done/total
//!   delivery rollup over those direct children.
//! - `jit query count --by state [--label ...]` aggregates a label bucket (or
//!   the whole repo) the same way.
//!
//! Containment follows the dependency DAG (a container's children are the
//! issues it directly depends on); membership labels are advisory and are the
//! basis only for the `query count` label bucket. Both rollups count `done` and
//! `rejected` distinctly (rejected is terminal but not delivered), `open` is
//! every non-terminal issue, and every lifecycle state appears in `by_state`
//! with a zero count when unpopulated.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .current_dir(temp.path())
        .arg("init")
        .output()
        .unwrap();
    temp
}

fn create_issue(dir: &std::path::Path, title: &str, labels: &[&str]) -> String {
    let mut args: Vec<String> = vec![
        "issue".into(),
        "create".into(),
        "--title".into(),
        title.into(),
        "--description".into(),
        "Body".into(),
    ];
    for label in labels {
        args.push("--label".into());
        args.push((*label).into());
    }
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(&args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "create failed for {title}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn set_state(dir: &std::path::Path, id: &str, state: &str) {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(["issue", "update", id, "--state", state])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "state update to {state} failed for {id}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_dep(dir: &std::path::Path, from_id: &str, to_id: &str) {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(["dep", "add", from_id, to_id])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dep add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Plant a dangling dependency edge: delete a child's storage file directly
/// (a raw mutation the CLI's delete-cascade would otherwise prevent), leaving
/// the container's dependency pointing at a now-missing issue.
fn delete_issue_file(dir: &std::path::Path, id: &str) {
    std::fs::remove_file(dir.join(".jit/issues").join(format!("{id}.json"))).unwrap();
}

fn run_ok(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run_json(dir: &std::path::Path, args: &[&str]) -> Value {
    let out = run_ok(dir, args);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("args {args:?} not JSON: {e}\n{out}"))
}

fn short(id: &str) -> &str {
    &id[0..8]
}

/// A container with children of several states, wired child-by-child as
/// dependencies. Returns `(container_id, [child ids in wiring order])`.
fn container_with_children(dir: &std::path::Path) -> (String, Vec<String>) {
    let epic = create_issue(dir, "Epic", &[]);
    let done = create_issue(dir, "Child done", &[]);
    let wip = create_issue(dir, "Child wip", &[]);
    let rejected = create_issue(dir, "Child rejected", &[]);
    let ready = create_issue(dir, "Child ready", &[]);
    for c in [&done, &wip, &rejected, &ready] {
        add_dep(dir, &epic, c);
    }
    set_state(dir, &done, "done");
    set_state(dir, &wip, "in_progress");
    set_state(dir, &rejected, "rejected");
    (epic, vec![done, wip, rejected, ready])
}

// ── issue children ───────────────────────────────────────────────────────

/// Text form prints one `issue status` line per direct child, one line each,
/// in ascending short-id order (stable listing).
#[test]
fn test_children_text_one_status_line_per_child() {
    let temp = setup();
    let (epic, children) = container_with_children(temp.path());

    let out = run_ok(temp.path(), &["issue", "children", &epic]);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 4, "one line per direct child: {out:?}");

    // Every child appears exactly once; the four states are all represented.
    let joined = lines.join("\n");
    for child in &children {
        assert!(
            joined.contains(short(child)),
            "child {child} listed: {out:?}"
        );
    }
    assert!(joined.contains("[done]") && joined.contains("[in_progress]"));
    assert!(joined.contains("[rejected]") && joined.contains("[ready]"));
    // Rendered exactly like `issue status`: the greppable field layout.
    assert!(
        joined.contains("gates: none unmet: none title: Child done"),
        "{out:?}"
    );

    // Deterministic order: ascending short id.
    let short_ids: Vec<&str> = lines
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    let mut sorted = short_ids.clone();
    sorted.sort_unstable();
    assert_eq!(short_ids, sorted, "children sorted by short id: {out:?}");
}

/// A child status line matches `issue status` byte-for-byte (same projection).
#[test]
fn test_children_line_matches_issue_status() {
    let temp = setup();
    let (epic, children) = container_with_children(temp.path());

    let children_out = run_ok(temp.path(), &["issue", "children", &epic]);

    // Locate this child's line by its short id, then compare to `issue status`.
    let target = short(&children[0]);
    let child_line = children_out
        .lines()
        .find(|l| l.starts_with(target))
        .unwrap_or_else(|| panic!("child {target} not listed:\n{children_out}"));

    let status_out = run_ok(temp.path(), &["issue", "status", &children[0]]);
    assert_eq!(child_line, status_out.trim_end());
}

/// JSON carries the container header, an envelope `count`, and one compact
/// status object per child.
#[test]
fn test_children_json_envelope_with_container_header() {
    let temp = setup();
    let (epic, _children) = container_with_children(temp.path());

    let json = run_json(temp.path(), &["issue", "children", &epic, "--json"]);

    assert_eq!(json["container"]["short_id"], short(&epic));
    assert_eq!(json["container"]["title"], "Epic");
    assert!(json["container"]["state"].is_string());

    let issues = json["issues"].as_array().unwrap();
    assert_eq!(json["count"].as_u64().unwrap() as usize, issues.len());
    assert_eq!(issues.len(), 4);
    // Compact status shape on every entry: short_id/state/gates/
    // unmet_dependencies/title (the same projection as `issue status`).
    for entry in issues {
        assert!(entry["short_id"].is_string());
        assert!(entry["state"].is_string());
        assert!(entry["gates"].is_array());
        assert!(entry["unmet_dependencies"].is_array());
        assert!(entry["title"].is_string());
    }
    // The done child is present with its projected state.
    let done = issues
        .iter()
        .find(|e| e["title"] == "Child done")
        .expect("done child listed");
    assert_eq!(done["state"], "done");

    // With no dangling edges, the `dangling` key is omitted entirely.
    assert!(
        json.get("dangling").is_none(),
        "clean response omits dangling: {json}"
    );
}

/// A dangling child edge (a dependency pointing at a missing issue) is surfaced
/// in `dangling`, not silently dropped; resolvable children still list normally.
#[test]
fn test_children_dangling_edge_surfaced() {
    let temp = setup();
    let epic = create_issue(temp.path(), "Epic", &[]);
    let alive = create_issue(temp.path(), "Alive", &[]);
    let doomed = create_issue(temp.path(), "Doomed", &[]);
    add_dep(temp.path(), &epic, &alive);
    add_dep(temp.path(), &epic, &doomed);
    delete_issue_file(temp.path(), &doomed);

    let json = run_json(temp.path(), &["issue", "children", &epic, "--json"]);
    // Only the resolvable child is counted/listed.
    assert_eq!(json["count"], 1);
    let listed: Vec<&str> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(listed, vec!["Alive"]);
    // The broken edge is visible in `dangling`.
    assert_eq!(json["dangling"], serde_json::json!([doomed]));

    // Text mode appends a `dangling:` line.
    let text = run_ok(temp.path(), &["issue", "children", &epic]);
    assert!(
        text.lines().any(|l| l == format!("dangling: {doomed}")),
        "text lists dangling: {text:?}"
    );
}

/// An empty container (no dependencies) prints nothing and JSON has count 0.
#[test]
fn test_children_empty_container() {
    let temp = setup();
    let epic = create_issue(temp.path(), "Empty epic", &[]);

    let text = run_ok(temp.path(), &["issue", "children", &epic]);
    assert!(
        text.trim().is_empty(),
        "empty container prints nothing: {text:?}"
    );

    let json = run_json(temp.path(), &["issue", "children", &epic, "--json"]);
    assert_eq!(json["count"], 0);
    assert_eq!(json["issues"].as_array().unwrap().len(), 0);
    assert_eq!(json["container"]["short_id"], short(&epic));
}

/// A non-container leaf issue (zero dependencies) is valid and lists nothing —
/// the same shape as an empty container, no special-casing of issue type.
#[test]
fn test_children_non_container_leaf_is_empty() {
    let temp = setup();
    let leaf = create_issue(temp.path(), "Leaf task", &[]);

    let json = run_json(temp.path(), &["issue", "children", &leaf, "--json"]);
    assert_eq!(json["count"], 0);
    assert_eq!(json["issues"].as_array().unwrap().len(), 0);
}

/// A bad id under `--json` routes through the JSON error path: an
/// `ISSUE_NOT_FOUND` envelope and the matching nonzero exit code.
#[test]
fn test_children_bad_id_json_error() {
    let temp = setup();
    let out = Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["issue", "children", "deadbeef", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success(), "bad id must fail");
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}

// ── issue progress ───────────────────────────────────────────────────────

/// Progress counts by state over direct children with distinct done/rejected/
/// open tallies and a done/total delivery ratio.
#[test]
fn test_progress_json_counts_mixed_states() {
    let temp = setup();
    let (epic, _children) = container_with_children(temp.path());

    let json = run_json(temp.path(), &["issue", "progress", &epic, "--json"]);

    assert_eq!(json["total"], 4);
    assert_eq!(json["done"], 1);
    assert_eq!(json["rejected"], 1);
    // open = total - done - rejected (in_progress + ready here).
    assert_eq!(json["open"], 2);
    // done/total delivery percent, rounded.
    assert_eq!(json["percent"], 25);
    assert_eq!(json["container"]["short_id"], short(&epic));
}

/// `by_state` enumerates every lifecycle state, unpopulated states present with
/// count 0 (stable, complete shape). `count` equals the number of buckets.
#[test]
fn test_progress_json_enumerates_all_states_with_zero() {
    let temp = setup();
    let (epic, _children) = container_with_children(temp.path());

    let json = run_json(temp.path(), &["issue", "progress", &epic, "--json"]);
    let by_state = json["by_state"].as_array().unwrap();

    // Seven lifecycle states, always present.
    assert_eq!(by_state.len(), 7);
    assert_eq!(json["count"].as_u64().unwrap() as usize, by_state.len());

    let count_for = |name: &str| -> u64 {
        by_state
            .iter()
            .find(|e| e["state"] == name)
            .unwrap_or_else(|| panic!("state {name} missing from by_state"))["count"]
            .as_u64()
            .unwrap()
    };
    // A populated and an unpopulated state both appear.
    assert_eq!(count_for("done"), 1);
    assert_eq!(count_for("gated"), 0);
    assert_eq!(count_for("backlog"), 0);
}

/// Text form leads with the container line, then the two rollup lines.
#[test]
fn test_progress_text_form() {
    let temp = setup();
    let (epic, _children) = container_with_children(temp.path());

    let out = run_ok(temp.path(), &["issue", "progress", &epic]);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3, "container line + two rollup lines: {out:?}");
    assert!(lines[0].starts_with(short(&epic)), "{:?}", lines[0]);
    assert!(lines[0].contains("title: Epic"), "{:?}", lines[0]);
    assert!(lines[1].starts_with("by state: "), "{:?}", lines[1]);
    assert!(lines[1].contains("done=1"), "{:?}", lines[1]);
    assert!(lines[2].contains("done 1/4 (25%)"), "{:?}", lines[2]);
    assert!(lines[2].contains("rejected 1"), "{:?}", lines[2]);
}

/// An empty container reports total 0 with percent 0 (no division by zero).
#[test]
fn test_progress_empty_container_zero_percent() {
    let temp = setup();
    let epic = create_issue(temp.path(), "Empty epic", &[]);

    let json = run_json(temp.path(), &["issue", "progress", &epic, "--json"]);
    assert_eq!(json["total"], 0);
    assert_eq!(json["done"], 0);
    assert_eq!(json["percent"], 0);
    assert_eq!(json["by_state"].as_array().unwrap().len(), 7);
}

/// A bad id under `--json` routes through the JSON error path.
#[test]
fn test_progress_bad_id_json_error() {
    let temp = setup();
    let out = Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["issue", "progress", "deadbeef", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}

/// A dangling child edge is surfaced in `dangling`; the rollup counts only the
/// resolvable children (documented resolvable-children `total` semantics).
#[test]
fn test_progress_dangling_edge_surfaced() {
    let temp = setup();
    let epic = create_issue(temp.path(), "Epic", &[]);
    let alive = create_issue(temp.path(), "Alive", &[]);
    let doomed = create_issue(temp.path(), "Doomed", &[]);
    add_dep(temp.path(), &epic, &alive);
    add_dep(temp.path(), &epic, &doomed);
    set_state(temp.path(), &alive, "done");
    delete_issue_file(temp.path(), &doomed);

    let json = run_json(temp.path(), &["issue", "progress", &epic, "--json"]);
    // Only the one resolvable child is counted.
    assert_eq!(json["total"], 1);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 100);
    assert_eq!(json["dangling"], serde_json::json!([doomed]));

    // Text mode appends a `dangling:` line after the rollup lines.
    let text = run_ok(temp.path(), &["issue", "progress", &epic]);
    assert!(
        text.lines().any(|l| l == format!("dangling: {doomed}")),
        "text lists dangling: {text:?}"
    );
}

/// With no broken edges, `progress` omits the `dangling` key entirely.
#[test]
fn test_progress_clean_omits_dangling() {
    let temp = setup();
    let (epic, _children) = container_with_children(temp.path());

    let json = run_json(temp.path(), &["issue", "progress", &epic, "--json"]);
    assert!(
        json.get("dangling").is_none(),
        "clean response omits dangling: {json}"
    );
}

// ── query count ──────────────────────────────────────────────────────────

/// `query count --by state` with no label aggregates the whole repository.
#[test]
fn test_query_count_whole_repo() {
    let temp = setup();
    let a = create_issue(temp.path(), "A", &[]);
    create_issue(temp.path(), "B", &[]);
    set_state(temp.path(), &a, "done");

    let json = run_json(temp.path(), &["query", "count", "--by", "state", "--json"]);
    assert_eq!(json["total"], 2);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 50);
    assert_eq!(json["count"], 7);
    assert_eq!(json["by_state"].as_array().unwrap().len(), 7);
}

/// Multiple `--label` patterns AND together: only issues carrying every label
/// are in the bucket.
#[test]
fn test_query_count_multi_label_and() {
    let temp = setup();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);
    create_issue(temp.path(), "ComponentOnly", &["component:api"]);
    set_state(temp.path(), &both, "done");

    let json = run_json(
        temp.path(),
        &[
            "query",
            "count",
            "--by",
            "state",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    // Only the issue carrying BOTH labels is counted.
    assert_eq!(json["total"], 1);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 100);
}

/// Every lifecycle state appears with a zero count when the bucket does not
/// populate it (dynamic enumeration from the domain State enum).
#[test]
fn test_query_count_enumerates_all_states_with_zero() {
    let temp = setup();
    create_issue(temp.path(), "Solo", &[]);

    let json = run_json(temp.path(), &["query", "count", "--by", "state", "--json"]);
    let by_state = json["by_state"].as_array().unwrap();
    assert_eq!(by_state.len(), 7);
    // Fresh unblocked issue is Ready; every other state present with 0.
    let count_for = |name: &str| -> u64 {
        by_state.iter().find(|e| e["state"] == name).unwrap()["count"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(count_for("ready"), 1);
    assert_eq!(count_for("done"), 0);
    assert_eq!(count_for("archived"), 0);
}

/// Text form is the two rollup lines (by-state row + done/total row).
#[test]
fn test_query_count_text_form() {
    let temp = setup();
    let a = create_issue(temp.path(), "A", &[]);
    set_state(temp.path(), &a, "done");

    let out = run_ok(temp.path(), &["query", "count", "--by", "state"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "two rollup lines: {out:?}");
    assert!(lines[0].starts_with("by state: "), "{:?}", lines[0]);
    assert!(lines[0].contains("done=1"), "{:?}", lines[0]);
    assert!(lines[1].contains("done 1/1 (100%)"), "{:?}", lines[1]);
}

/// An unknown `--by` value is a clap usage error (exit 2), not a silent no-op.
#[test]
fn test_query_count_invalid_dimension_rejected() {
    let temp = setup();
    let out = Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["query", "count", "--by", "bogus"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2), "clap usage error exit code");
}

/// Argument order and cwd independence: `--json` before or after positionals
/// and a run from a subdirectory produce the same aggregation.
#[test]
fn test_query_count_flag_order_and_cwd_independent() {
    let temp = setup();
    let a = create_issue(temp.path(), "A", &[]);
    set_state(temp.path(), &a, "done");
    create_issue(temp.path(), "B", &[]);

    let canonical = run_json(temp.path(), &["query", "count", "--by", "state", "--json"]);

    // Flag before the subcommand args is equivalent.
    let reordered = run_json(temp.path(), &["query", "count", "--json", "--by", "state"]);
    assert_eq!(canonical["total"], reordered["total"]);
    assert_eq!(canonical["done"], reordered["done"]);

    // Run from a nested subdirectory (repo discovery walks up).
    let sub = temp.path().join("nested/dir");
    std::fs::create_dir_all(&sub).unwrap();
    let from_sub = run_json(&sub, &["query", "count", "--by", "state", "--json"]);
    assert_eq!(canonical["total"], from_sub["total"]);
    assert_eq!(canonical["done"], from_sub["done"]);
}
