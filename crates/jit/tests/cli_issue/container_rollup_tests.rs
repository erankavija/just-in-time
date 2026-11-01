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

use jit::test_cli::JitCli;
use serde_json::Value;

/// Plant a dangling dependency edge for the projection tests.
fn delete_issue_file(cli: &JitCli, id: &str) {
    std::fs::remove_file(cli.root().join(".jit/issues").join(format!("{id}.json"))).unwrap();
}

fn container_with_children(cli: &JitCli) -> (String, Vec<String>) {
    let epic = cli
        .create_issue(["--title", "Epic", "--description", "Body"])
        .unwrap();
    let done = cli
        .create_issue(["--title", "Child done", "--description", "Body"])
        .unwrap();
    let wip = cli
        .create_issue(["--title", "Child wip", "--description", "Body"])
        .unwrap();
    let rejected = cli
        .create_issue(["--title", "Child rejected", "--description", "Body"])
        .unwrap();
    let ready = cli
        .create_issue(["--title", "Child ready", "--description", "Body"])
        .unwrap();
    for child in [&done, &wip, &rejected, &ready] {
        cli.run(["dep", "add", &epic, child]).unwrap();
    }
    cli.run(["issue", "update", &done, "--state", "done"])
        .unwrap();
    cli.run(["issue", "update", &wip, "--state", "in_progress"])
        .unwrap();
    cli.run(["issue", "reject", &rejected]).unwrap();
    (epic, vec![done, wip, rejected, ready])
}

fn short(id: &str) -> &str {
    &id[0..8]
}

// ── issue children ───────────────────────────────────────────────────────

/// Text form prints one `issue status` line per direct child, one line each,
/// in ascending short-id order (stable listing).
#[test]
fn test_children_text_one_status_line_per_child() {
    let temp = JitCli::initialised().unwrap();
    let (epic, children) = container_with_children(&temp);

    let out = temp.run(["issue", "children", &epic]).unwrap();
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
    let mut sorted: Vec<&str> = children.iter().map(|child| short(child)).collect();
    sorted.sort_unstable();
    assert!(
        lines
            .iter()
            .zip(sorted)
            .all(|(line, child)| line.starts_with(child)),
        "children sorted by short id: {out:?}"
    );
}

/// A child status line matches `issue status` byte-for-byte (same projection).
#[test]
fn test_children_line_matches_issue_status() {
    let temp = JitCli::initialised().unwrap();
    let (epic, children) = container_with_children(&temp);

    let children_out = temp.run(["issue", "children", &epic]).unwrap();

    // Locate this child's line by its short id, then compare to `issue status`.
    let target = short(&children[0]);
    let child_line = children_out
        .lines()
        .find(|l| l.starts_with(target))
        .unwrap_or_else(|| panic!("child {target} not listed:\n{children_out}"));

    let status_out = temp.run(["issue", "status", &children[0]]).unwrap();
    assert_eq!(child_line, status_out.trim_end());
}

/// JSON carries the container header and one compact status object per child.
#[test]
fn test_children_json_envelope_with_container_header() {
    let temp = JitCli::initialised().unwrap();
    let (epic, _children) = container_with_children(&temp);

    let json = temp.run_json(["issue", "children", &epic]).unwrap();

    assert_eq!(json["container"]["short_id"], short(&epic));
    assert_eq!(json["container"]["title"], "Epic");
    assert!(json["container"]["state"].is_string());

    let issues = json["issues"].as_array().unwrap();
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
    let temp = JitCli::initialised().unwrap();
    let epic = temp
        .create_issue(["--title", "Epic", "--description", "Body"])
        .unwrap();
    let alive = temp
        .create_issue(["--title", "Alive", "--description", "Body"])
        .unwrap();
    let doomed = temp
        .create_issue(["--title", "Doomed", "--description", "Body"])
        .unwrap();
    temp.run(["dep", "add", &epic, &alive]).unwrap();
    temp.run(["dep", "add", &epic, &doomed]).unwrap();
    delete_issue_file(&temp, &doomed);

    let json = temp.run_json(["issue", "children", &epic]).unwrap();
    // Only the resolvable child is listed.
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
    let text = temp.run(["issue", "children", &epic]).unwrap();
    assert!(
        text.lines().any(|l| l == format!("dangling: {doomed}")),
        "text lists dangling: {text:?}"
    );
}

/// An empty container (no dependencies) prints nothing and JSON lists no issues.
#[test]
fn test_children_empty_container() {
    let temp = JitCli::initialised().unwrap();
    let epic = temp
        .create_issue(["--title", "Empty epic", "--description", "Body"])
        .unwrap();

    let text = temp.run(["issue", "children", &epic]).unwrap();
    assert!(
        text.trim().is_empty(),
        "empty container prints nothing: {text:?}"
    );

    let json = temp.run_json(["issue", "children", &epic]).unwrap();
    assert_eq!(json["issues"].as_array().unwrap().len(), 0);
    assert_eq!(json["container"]["short_id"], short(&epic));
}

/// A non-container leaf issue (zero dependencies) is valid and lists nothing —
/// the same shape as an empty container, no special-casing of issue type.
#[test]
fn test_children_non_container_leaf_is_empty() {
    let temp = JitCli::initialised().unwrap();
    let leaf = temp
        .create_issue(["--title", "Leaf task", "--description", "Body"])
        .unwrap();

    let json = temp.run_json(["issue", "children", &leaf]).unwrap();
    assert_eq!(json["issues"].as_array().unwrap().len(), 0);
}

/// A bad id under `--json` routes through the JSON error path: an
/// `ISSUE_NOT_FOUND` envelope and a nonzero exit.
#[test]
fn test_children_bad_id_json_error() {
    let temp = JitCli::initialised().unwrap();
    let out = temp
        .command(["issue", "children", "deadbeef", "--json"])
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
    let temp = JitCli::initialised().unwrap();
    let (epic, _children) = container_with_children(&temp);

    let json = temp.run_json(["issue", "progress", &epic]).unwrap();

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
/// count 0 (stable, complete shape).
#[test]
fn test_progress_json_enumerates_all_states_with_zero() {
    let temp = JitCli::initialised().unwrap();
    let (epic, _children) = container_with_children(&temp);

    let json = temp.run_json(["issue", "progress", &epic]).unwrap();
    let by_state = json["by_state"].as_array().unwrap();

    // Seven lifecycle states, always present.
    assert_eq!(by_state.len(), 7);

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
    let temp = JitCli::initialised().unwrap();
    let (epic, _children) = container_with_children(&temp);

    let out = temp.run(["issue", "progress", &epic]).unwrap();
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
    let temp = JitCli::initialised().unwrap();
    let epic = temp
        .create_issue(["--title", "Empty epic", "--description", "Body"])
        .unwrap();

    let json = temp.run_json(["issue", "progress", &epic]).unwrap();
    assert_eq!(json["total"], 0);
    assert_eq!(json["done"], 0);
    assert_eq!(json["percent"], 0);
    assert_eq!(json["by_state"].as_array().unwrap().len(), 7);
}

/// A bad id under `--json` routes through the JSON error path.
#[test]
fn test_progress_bad_id_json_error() {
    let temp = JitCli::initialised().unwrap();
    let out = temp
        .command(["issue", "progress", "deadbeef", "--json"])
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
    let temp = JitCli::initialised().unwrap();
    let epic = temp
        .create_issue(["--title", "Epic", "--description", "Body"])
        .unwrap();
    let alive = temp
        .create_issue(["--title", "Alive", "--description", "Body"])
        .unwrap();
    let doomed = temp
        .create_issue(["--title", "Doomed", "--description", "Body"])
        .unwrap();
    temp.run(["dep", "add", &epic, &alive]).unwrap();
    temp.run(["dep", "add", &epic, &doomed]).unwrap();
    temp.run(["issue", "update", &alive, "--state", "done"])
        .unwrap();
    delete_issue_file(&temp, &doomed);

    let json = temp.run_json(["issue", "progress", &epic]).unwrap();
    // Only the one resolvable child is counted.
    assert_eq!(json["total"], 1);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 100);
    assert_eq!(json["dangling"], serde_json::json!([doomed]));

    // Text mode appends a `dangling:` line after the rollup lines.
    let text = temp.run(["issue", "progress", &epic]).unwrap();
    assert!(
        text.lines().any(|l| l == format!("dangling: {doomed}")),
        "text lists dangling: {text:?}"
    );
}

/// With no broken edges, `progress` omits the `dangling` key entirely.
#[test]
fn test_progress_clean_omits_dangling() {
    let temp = JitCli::initialised().unwrap();
    let (epic, _children) = container_with_children(&temp);

    let json = temp.run_json(["issue", "progress", &epic]).unwrap();
    assert!(
        json.get("dangling").is_none(),
        "clean response omits dangling: {json}"
    );
}

// ── query count ──────────────────────────────────────────────────────────

/// `query count --by state` with no label aggregates the whole repository.
#[test]
fn test_query_count_whole_repo() {
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "A", "--description", "Body"])
        .unwrap();
    temp.create_issue(["--title", "B", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &a, "--state", "done"])
        .unwrap();

    let json = temp.run_json(["query", "count", "--by", "state"]).unwrap();
    assert_eq!(json["total"], 2);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 50);
    assert_eq!(json["by_state"].as_array().unwrap().len(), 7);
}

/// Multiple `--label` patterns AND together: only issues carrying every label
/// are in the bucket.
#[test]
fn test_query_count_multi_label_and() {
    let taxonomy = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let epic_label = jit::test_taxonomy::membership_label(&taxonomy.taxonomy, 2, "auth");
    let component_label = jit::test_taxonomy::membership_label(&taxonomy.taxonomy, 3, "api");
    let temp = JitCli::with_repository(taxonomy.temp);
    let both = temp
        .create_issue([
            "--title",
            "Both",
            "--description",
            "Body",
            "--label",
            epic_label.as_str(),
            "--label",
            component_label.as_str(),
        ])
        .unwrap();
    temp.create_issue([
        "--title",
        "EpicOnly",
        "--description",
        "Body",
        "--label",
        epic_label.as_str(),
    ])
    .unwrap();
    temp.create_issue([
        "--title",
        "ComponentOnly",
        "--description",
        "Body",
        "--label",
        component_label.as_str(),
    ])
    .unwrap();
    temp.run(["issue", "update", &both, "--state", "done"])
        .unwrap();

    let json = temp
        .run_json([
            "query",
            "count",
            "--by",
            "state",
            "--label",
            epic_label.as_str(),
            "--label",
            component_label.as_str(),
        ])
        .unwrap();
    // Only the issue carrying BOTH labels is counted.
    assert_eq!(json["total"], 1);
    assert_eq!(json["done"], 1);
    assert_eq!(json["percent"], 100);
}

/// Every lifecycle state appears with a zero count when the bucket does not
/// populate it (dynamic enumeration from the domain State enum).
#[test]
fn test_query_count_enumerates_all_states_with_zero() {
    let temp = JitCli::initialised().unwrap();
    temp.create_issue(["--title", "Solo", "--description", "Body"])
        .unwrap();

    let json = temp.run_json(["query", "count", "--by", "state"]).unwrap();
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
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "A", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &a, "--state", "done"])
        .unwrap();

    let out = temp.run(["query", "count", "--by", "state"]).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "two rollup lines: {out:?}");
    assert!(lines[0].starts_with("by state: "), "{:?}", lines[0]);
    assert!(lines[0].contains("done=1"), "{:?}", lines[0]);
    assert!(lines[1].contains("done 1/1 (100%)"), "{:?}", lines[1]);
}

/// An unknown `--by` value is a clap usage error, not a silent no-op.
#[test]
fn test_query_count_invalid_dimension_rejected() {
    let temp = JitCli::initialised().unwrap();
    let out = temp
        .command(["query", "count", "--by", "bogus"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("invalid value 'bogus'"),
        "clap usage error"
    );
}

/// Argument order and cwd independence: `--json` before or after positionals
/// and a run from a subdirectory produce the same aggregation.
#[test]
fn test_query_count_flag_order_and_cwd_independent() {
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "A", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &a, "--state", "done"])
        .unwrap();
    temp.create_issue(["--title", "B", "--description", "Body"])
        .unwrap();

    let canonical = temp.run_json(["query", "count", "--by", "state"]).unwrap();

    // Flag before the subcommand args is equivalent.
    let reordered: Value = serde_json::from_str(
        &temp
            .run(["query", "count", "--json", "--by", "state"])
            .unwrap(),
    )
    .unwrap();
    assert_eq!(canonical["total"], reordered["total"]);
    assert_eq!(canonical["done"], reordered["done"]);

    // Run from a nested subdirectory (repo discovery walks up).
    let sub = temp.root().join("nested/dir");
    std::fs::create_dir_all(&sub).unwrap();
    let output = temp
        .command(["query", "count", "--by", "state", "--json"])
        .current_dir(&sub)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "command from subdirectory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let from_sub: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(canonical["total"], from_sub["total"]);
    assert_eq!(canonical["done"], from_sub["done"]);
}
