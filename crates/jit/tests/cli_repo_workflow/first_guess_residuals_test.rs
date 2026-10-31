//! Integration tests for REQ-05 first-guess command/flag residuals (jit:a1623187).
//!
//! Each test pins one residual from the issue's success criteria:
//!   1. `gate define --auto` records mode=auto.
//!   2. `doc add --title` is an alias for the human label (`--label`).
//!   3. A gate-inspection call with the gate key and issue id transposed yields an
//!      actionable did-you-mean (non-zero) rather than a silent misparse.
//!   4. A top-level reverse-deps / issue-listing first guess (`jit rdeps`,
//!      `jit list`) resolves to the canonical command.

use jit::test_cli::JitCli;
use std::fs;

// 1. `gate define --auto` sets mode to auto.
#[test]
fn test_gate_define_auto_sets_mode_auto() {
    let cli = JitCli::initialised().unwrap();

    cli.run([
        "gate",
        "define",
        "auto-gate",
        "--title",
        "Auto Gate",
        "--description",
        "Defined via --auto",
        "--auto",
        "--checker-command",
        "true",
    ])
    .unwrap();

    let json = cli.run_json(["gate", "show", "auto-gate"]).unwrap();
    assert_eq!(json["mode"], "auto", "expected --auto to record mode=auto");
}

// 2. `doc add --title X` sets the human label identically to `--label X`.
#[test]
fn test_doc_add_title_alias_sets_label() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doc host"]).unwrap();
    fs::write(cli.root().join("design.md"), "# Design\n").unwrap();

    let json = cli
        .run_json(["doc", "add", &id, "design.md", "--title", "Design Notes"])
        .unwrap();
    assert_eq!(
        json["document"]["label"], "Design Notes",
        "--title should populate the human label like --label"
    );
}

// 3. Transposed gate-inspection args yield an actionable did-you-mean (non-zero).
#[test]
fn test_gate_status_transposed_args_did_you_mean() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Gated issue"]).unwrap();
    cli.run([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Test gate",
    ])
    .unwrap();

    // Canonical form is `jit gate status <issue> <gate-key>`. Pass them transposed.
    let output = cli
        .command(["gate", "status", "tests", &id])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("transposed") && stderr.contains("did you mean"),
        "expected an actionable did-you-mean about transposed args, got: {stderr}"
    );

    // The --json surface carries the same actionable suggestion and a non-zero exit.
    let json_out = cli
        .command(["gate", "status", "tests", &id, "--json"])
        .output()
        .unwrap();
    assert!(!json_out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(json["error"]["details"]["transposed"], true);
    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|s| s
            .as_str()
            .unwrap()
            .contains(&format!("gate status {} tests", id))),
        "expected suggestion with the corrected argument order"
    );
}

#[test]
fn test_invariant_list_rejection_mentions_item_list_route() {
    let cli = JitCli::uninitialised().unwrap();
    let output = cli.command(["invariant", "list"]).output().unwrap();
    assert!(
        !output.status.success(),
        "an unsupported invariant subcommand is rejected"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("jit item list --kind invariant"),
        "the rejected invariant list should name the canonical enumeration route, got: {stderr}"
    );
}

// 4a. Top-level `jit rdeps` routes to the canonical `jit graph rdeps`.
#[test]
fn test_top_level_rdeps_routes_to_graph_rdeps() {
    let cli = JitCli::initialised().unwrap();
    let blocker = cli.create_issue(["--title", "Blocker"]).unwrap();
    let dependent = cli.create_issue(["--title", "Dependent"]).unwrap();
    // Dependent depends on blocker => blocker's rdeps includes dependent.
    cli.run(["dep", "add", &dependent, &blocker]).unwrap();

    let alias_json = cli.run_json(["rdeps", &blocker]).unwrap();
    let canonical_json = cli.run_json(["graph", "rdeps", &blocker]).unwrap();
    assert_eq!(
        alias_json, canonical_json,
        "`jit rdeps` must resolve identically to `jit graph rdeps`"
    );
    // And it actually surfaces the dependent.
    assert!(
        alias_json.to_string().contains(&dependent),
        "rdeps output should list the dependent issue"
    );
}

// 4a-bis. Top-level `jit rdeps` shares the canonical bounded default: depth 1
// (immediate dependents only) by default; `--depth 0` opts into transitive.
// Multi-level chain A <- B <- C so bounded and unbounded outputs differ.
#[test]
fn test_top_level_rdeps_default_is_bounded_multi_level() {
    let cli = JitCli::initialised().unwrap();
    let a = cli.create_issue(["--title", "A root"]).unwrap();
    let b = cli.create_issue(["--title", "B mid"]).unwrap();
    let c = cli.create_issue(["--title", "C leaf"]).unwrap();
    // B depends on A; C depends on B. So A's transitive dependents are B and C,
    // but A's immediate dependent is only B.
    for (dependent, blocker) in [(&b, &a), (&c, &b)] {
        cli.run(["dep", "add", dependent, blocker]).unwrap();
    }

    // Default `jit rdeps A`: immediate dependent B present, transitive C absent.
    let default_out = cli.run_json(["rdeps", &a]).unwrap();
    let default_str = default_out.to_string();
    assert!(
        default_str.contains(&b),
        "bounded default must include the immediate dependent B"
    );
    assert!(
        !default_str.contains(&c),
        "bounded default must NOT include the transitive dependent C (depth 1)"
    );

    // Opt-in `jit rdeps A --depth 0`: both B and C present.
    let unbounded_out = cli.run_json(["rdeps", &a, "--depth", "0"]).unwrap();
    let unbounded_str = unbounded_out.to_string();
    assert!(
        unbounded_str.contains(&b) && unbounded_str.contains(&c),
        "unbounded opt-in (--depth 0) must include transitive dependent C"
    );
}

// 4b. Top-level `jit list` routes to the canonical `jit issue list`.
#[test]
fn test_top_level_list_routes_to_issue_list() {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["--title", "Alpha"]).unwrap();
    cli.create_issue(["--title", "Beta"]).unwrap();

    let alias_json = cli.run_json(["list"]).unwrap();
    let canonical_json = cli.run_json(["issue", "list"]).unwrap();
    assert_eq!(
        alias_json, canonical_json,
        "`jit list` must resolve identically to `jit issue list`"
    );
}
