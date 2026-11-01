//! CLI-level tests for the `jit dep add` write-time redundancy guard (jit:7a50e021).
//!
//! Verifies a redundant add is rejected with a nonzero exit by default, and `--reduce` makes it succeed while leaving `jit validate`
//! clean.

use jit::test_cli::{seed_ambiguous_prefix, JitCli};
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

/// REQ-01: a redundant `jit dep add` exits nonzero and names the offending pair.
#[test]
fn test_cli_dep_add_redundant_edge_rejected_nonzero() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let x = cli.create_issue(["-t", "X"]).unwrap();
    let y = cli.create_issue(["-t", "Y"]).unwrap();
    let z = cli.create_issue(["-t", "Z"]).unwrap();

    cli.run(["dep", "add", &x, &y]).unwrap();
    cli.run(["dep", "add", &x, &z]).unwrap();

    // Adding Y → Z makes the pre-existing X → Z redundant: rejected, nonzero.
    let out = cli.command(["dep", "add", &y, &z]).output().unwrap();
    assert!(!out.status.success(), "redundant add must exit nonzero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&x[..8]) && stderr.contains(&z[..8]),
        "error must name the shadowed edge X→Z, got: {stderr}"
    );

    // The write did not happen: validate is still clean.
    let val = cli.command(["validate"]).output().unwrap();
    assert!(
        val.status.success(),
        "no partial write should have occurred"
    );
}

/// REQ-01 (JSON): the rejection surfaces as a VALIDATION_FAILED JSON error.
#[test]
fn test_cli_dep_add_redundant_edge_json_error() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let x = cli.create_issue(["-t", "X"]).unwrap();
    let y = cli.create_issue(["-t", "Y"]).unwrap();
    let z = cli.create_issue(["-t", "Z"]).unwrap();

    cli.run(["dep", "add", &x, &y]).unwrap();
    cli.run(["dep", "add", &x, &z]).unwrap();

    let out = cli
        .command(["dep", "add", &y, &z, "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("VALIDATION_FAILED"),
        "json error must carry VALIDATION_FAILED, got: {stdout}"
    );
}

/// REQ-02 / REQ-03: `--reduce` succeeds (exit 0), drops the shadowed edge, and
/// leaves `jit validate` clean.
#[test]
fn test_cli_dep_add_reduce_succeeds_and_validate_clean() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let x = cli.create_issue(["-t", "X"]).unwrap();
    let y = cli.create_issue(["-t", "Y"]).unwrap();
    let z = cli.create_issue(["-t", "Z"]).unwrap();

    cli.run(["dep", "add", &x, &y]).unwrap();
    cli.run(["dep", "add", &x, &z]).unwrap();

    let out = cli
        .command(["dep", "add", &y, &z, "--reduce"])
        .output()
        .unwrap();
    assert!(out.status.success(), "--reduce add must succeed");

    let val = cli.command(["validate"]).output().unwrap();
    assert!(
        val.status.success(),
        "validate must be clean after --reduce add: {}",
        String::from_utf8_lossy(&val.stderr)
    );
}

/// jit:c8518f2a REQ-01 (variadic): a redundant edge among several targets
/// fails the whole `jit dep add` atomically — the sibling edge that would
/// have succeeded on its own must NOT be persisted either. This is the
/// combination case: A -> C is only redundant once A -> B is ALSO in this
/// same call (neither edge is redundant on its own against the pre-existing
/// graph), so the would-be-final-graph check must consider both edges
/// together, not one at a time.
#[test]
fn test_cli_dep_add_variadic_redundant_among_valid_exits_nonzero() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let a = cli.create_issue(["-t", "A"]).unwrap();
    let b = cli.create_issue(["-t", "B"]).unwrap();
    let c = cli.create_issue(["-t", "C"]).unwrap();

    // Pre-existing B -> C, so a later A -> C is redundant once A -> B is added.
    cli.run(["dep", "add", &b, &c]).unwrap();

    // Variadic: A -> B (valid alone) and A -> C (redundant only in
    // combination with A -> B).
    let out = cli.command(["dep", "add", &a, &b, &c]).output().unwrap();
    assert!(
        !out.status.success(),
        "a redundant edge among the targets must fail the command; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // All-or-nothing: A -> B must NOT have been persisted either, even though
    // it would have succeeded on its own.
    let show = cli
        .command(["issue", "show", &a, "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let deps = json["dependencies"].as_array().unwrap();
    assert!(
        deps.is_empty(),
        "no edge should have been added when a sibling edge is rejected: {:?}",
        deps
    );

    // Nothing was written, so the pre-existing B -> C edge is exactly as it
    // was: still direct, not shadowed by any A edge.
    let show_b = cli
        .command(["issue", "show", &b, "--json"])
        .output()
        .unwrap();
    assert!(show_b.status.success());
    let json_b: serde_json::Value = serde_json::from_slice(&show_b.stdout).unwrap();
    let deps_b: Vec<&str> = json_b["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(deps_b, vec![c.as_str()]);
}

/// A batch mixing an id naming no issue with a self-redundant edge is rejected
/// when the CLI resolves its targets, before graph validation: it exits with
/// the not-found code (3), names only the unresolved id, and writes nothing.
#[test]
fn test_cli_dep_add_unresolved_target_rejects_batch_before_graph_validation() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let a = cli.create_issue(["-t", "A"]).unwrap();
    let b = cli.create_issue(["-t", "B"]).unwrap();
    let c = cli.create_issue(["-t", "C"]).unwrap();

    // A -> B -> C exist, so a direct A -> C is self-redundant.
    cli.run(["dep", "add", &a, &b]).unwrap();
    cli.run(["dep", "add", &b, &c]).unwrap();

    // Text mode: the unresolved id rejects the batch.
    let out = cli
        .command(["dep", "add", &a, &c, "zzzz"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("zzzz"),
        "must name the unresolved id: {stderr}"
    );

    // JSON mode: ISSUE_NOT_FOUND at top level, only the
    // unresolved id in details.rejected.
    let out_json = cli
        .command(["dep", "add", &a, &c, "zzzz", "--json"])
        .output()
        .unwrap();
    assert!(!out_json.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");

    let rejected = json["error"]["details"]["rejected"].as_array().unwrap();
    let codes: Vec<&str> = rejected
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["ISSUE_NOT_FOUND"], "{rejected:?}");

    // Nothing was written.
    let show = cli
        .command(["issue", "show", &a, "--json"])
        .output()
        .unwrap();
    let json_a: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let deps: Vec<&str> = json_a["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(deps, vec![b.as_str()]);
}

/// jit:c8518f2a (round 2): a cycle-failing edge must not suppress redundancy
/// analysis for a sibling in the same batch. Existing A -> B -> C and D -> A;
/// `dep add A D C` tries A -> D (cycle) and A -> C (self-redundant via
/// A -> B -> C). Both must be named, in both text and --json forms.
#[test]
fn test_cli_dep_add_cycle_failure_does_not_suppress_sibling_redundancy() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let a = cli.create_issue(["-t", "A"]).unwrap();
    let b = cli.create_issue(["-t", "B"]).unwrap();
    let c = cli.create_issue(["-t", "C"]).unwrap();
    let d = cli.create_issue(["-t", "D"]).unwrap();

    cli.run(["dep", "add", &a, &b]).unwrap();
    cli.run(["dep", "add", &b, &c]).unwrap();
    cli.run(["dep", "add", &d, &a]).unwrap();

    // Text mode: both edges named.
    let out = cli.command(["dep", "add", &a, &d, &c]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&d[..8]),
        "the cycle edge (D) must be named: {stderr}"
    );
    assert!(
        stderr.contains(&c[..8]),
        "the redundant edge (C) must ALSO be named, not suppressed by the cycle: {stderr}"
    );

    // JSON mode: both edges present in details.rejected with their own codes.
    let out_json = cli
        .command(["dep", "add", &a, &d, &c, "--json"])
        .output()
        .unwrap();
    assert!(!out_json.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();

    let rejected = json["error"]["details"]["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 2, "both edges must be named: {rejected:?}");
    let by_to: std::collections::HashMap<&str, &str> = rejected
        .iter()
        .map(|r| (r["to"].as_str().unwrap(), r["code"].as_str().unwrap()))
        .collect();
    assert_eq!(by_to.get(d.as_str()), Some(&"CYCLE_DETECTED"));
    assert_eq!(by_to.get(c.as_str()), Some(&"VALIDATION_FAILED"));
}

/// A too-short id prefix in either `dep add` position is refused as an invalid
/// prefix (human message and `INVALID_ID_PREFIX` under `--json`).
#[test]
fn test_cli_dep_add_too_short_prefix_rejected_in_both_positions() {
    let cli = JitCli::initialised().unwrap();
    let a = cli.create_issue(["--title", "A"]).unwrap();
    let b = cli.create_issue(["--title", "B"]).unwrap();

    let short_from = cli.command(["dep", "add", "ab", &b]).output().unwrap();
    assert!(!short_from.status.success());
    assert!(String::from_utf8_lossy(&short_from.stderr).contains("at least 4 characters"));

    [
        ["dep", "add", "ab", &b, "--json"],
        ["dep", "add", &a, "ab", "--json"],
    ]
    .into_iter()
    .for_each(|args| {
        let out = cli.command(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX", "{args:?}");
    });
}

/// `dep rm` validates `<from>` and `<target>` identically: a too-short prefix in
/// either position fails with the same message, not a silent no-op.
#[test]
fn test_cli_dep_rm_validates_both_positions_symmetrically() {
    let cli = JitCli::initialised().unwrap();
    let dependent = cli.create_issue(["--title", "Dependent"]).unwrap();
    let dependency = cli.create_issue(["--title", "Dependency"]).unwrap();
    cli.run(["dep", "add", &dependent, &dependency]).unwrap();

    [
        ["dep", "rm", "ab", &dependency],
        ["dep", "rm", &dependent, "ab"],
    ]
    .into_iter()
    .for_each(|args| {
        let out = cli.command(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("at least 4 characters"),
            "{args:?}"
        );
    });
}

/// An ambiguous id prefix in either `dep add` position is refused as
/// `AMBIGUOUS_ID` under `--json`.
#[test]
fn test_cli_dep_add_ambiguous_prefix_rejected_in_both_positions() {
    let cli = JitCli::initialised().unwrap();
    let (prefix, _) = seed_ambiguous_prefix(&cli).unwrap();
    let other = cli.create_issue(["--title", "Other"]).unwrap();

    [
        ["dep", "add", &prefix, &other, "--json"],
        ["dep", "add", &other, &prefix, "--json"],
    ]
    .into_iter()
    .for_each(|args| {
        let out = cli.command(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["error"]["code"], "AMBIGUOUS_ID", "{args:?}");
    });
}
