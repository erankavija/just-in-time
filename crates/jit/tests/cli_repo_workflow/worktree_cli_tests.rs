//! Integration tests for worktree CLI commands
//!
//! Tests the full CLI experience for worktree and validate commands.

use assert_cmd::prelude::*;
use jit::domain::store_divergence::{DivergenceClass, DivergentRecord};
use jit::errors::{GitRequirementGap, RequiresGitError};
use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;

/// Declared by every invocation writing a linked checkout's store: these tests
/// are about worktree identity, not about the write policy (jit:f52567ed REQ-05).
// === jit worktree info tests ===

#[test]
fn test_worktree_info_field_projection_preserves_value_types_and_raw_paths() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let store = cli.root().join("tracker space[1] \"quoted\"");
    fs::rename(cli.root().join(".jit"), &store).unwrap();
    for (field, expected) in [
        ("data_root", Value::from(store.to_str().unwrap())),
        ("is_main_worktree", Value::from(true)),
    ] {
        let plain = cli
            .command_with_env(
                ["worktree", "info", "--field", field],
                [("JIT_DATA_DIR", &store)],
            )
            .output()
            .unwrap();
        assert!(plain.status.success(), "{plain:?}");
        let text = match &expected {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        assert_eq!(
            String::from_utf8(plain.stdout).unwrap(),
            format!("{text}\n")
        );
        let json = cli
            .command_with_env(
                ["worktree", "info", "--field", field, "--json"],
                [("JIT_DATA_DIR", &store)],
            )
            .output()
            .unwrap();
        assert!(json.status.success(), "{json:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&json.stdout).unwrap(),
            expected
        );
    }
}

#[test]
fn test_worktree_info_field_projection_rejects_unknown_fields_in_both_modes() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    for json in [false, true] {
        let mut command = cli.command(["worktree", "info", "--field", "absent_field"]);
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        if json {
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["error"]["code"], "INVALID_ARGUMENT");
            assert!(response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("absent_field"));
        } else {
            assert!(String::from_utf8(output.stderr)
                .unwrap()
                .contains("absent_field"));
        }
    }
}

#[test]
fn test_worktree_info_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // Run worktree info
    cli.command(["worktree", "info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wt:"))
        .stdout(predicate::str::contains("main").or(predicate::str::contains("master")));
}

#[test]
fn test_worktree_info_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    let output = cli
        .command(["worktree", "info", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["worktree_id"].is_string());
    assert!(json["branch"].is_string());
    assert!(json["root_path"].is_string());
    assert_eq!(json["data_root"].as_str(), cli.root().join(".jit").to_str());

    // Worktree ID should start with "wt:"
    let wt_id = json["worktree_id"].as_str().unwrap();
    assert!(wt_id.starts_with("wt:"), "ID should start with 'wt:'");
}

#[test]
fn test_worktree_info_reports_resolved_data_root_in_json_and_human_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let custom = cli.root().join("tracker space");
    fs::rename(cli.root().join(".jit"), &custom).unwrap();
    let nested = cli.root().join("nested");
    fs::create_dir(&nested).unwrap();

    for selection in [custom.clone(), std::path::PathBuf::from("../tracker space")] {
        let output = cli
            .command_with_env(
                ["worktree", "info", "--json"],
                [("JIT_DATA_DIR", &selection)],
            )
            .current_dir(&nested)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["data_root"].as_str(), custom.to_str());
        cli.command_with_env(["worktree", "info"], [("JIT_DATA_DIR", &selection)])
            .current_dir(&nested)
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "Data root:  {}",
                custom.display()
            )));
    }
}

#[test]
fn test_worktree_info_follows_overridden_data_root_checkout() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_path = linked.root();

    let output = cli
        .command_with_env(
            ["worktree", "info", "--json"],
            [("JIT_DATA_DIR", worktree_path.join(".jit").to_str().unwrap())],
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "worktree info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["root_path"].as_str(),
        Some(worktree_path.to_str().unwrap())
    );
    assert_eq!(json["is_main_worktree"], false);
    assert_eq!(
        json["data_root"].as_str(),
        worktree_path.join(".jit").to_str()
    );
}

#[test]
fn test_init_follows_overridden_linked_data_root_and_replaces_copied_identity() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_path = linked.root();
    let primary_identity_path = cli.root().join(".jit/worktree.json");
    let linked_identity_path = worktree_path.join(".jit/worktree.json");
    let primary_before = fs::read(&primary_identity_path).unwrap();
    let primary_identity: Value = serde_json::from_slice(&primary_before).unwrap();
    let linked_branch = linked.git(["branch", "--show-current"]).unwrap();
    let linked_branch = linked_branch.trim();

    // Reproduce a linked checkout copied from a primary store that carried its
    // machine-local identity. Init must classify the selected linked store,
    // discard this copied identity, and leave the primary file untouched.
    fs::write(&linked_identity_path, &primary_before).unwrap();

    // The selected data root is the linked checkout's store, so the invocation
    // declares the stance that permits it to write there (jit:f52567ed REQ-05).
    let output = cli
        .command_with_env(
            ["init", "--json"],
            [("JIT_DATA_DIR", worktree_path.join(".jit").to_str().unwrap())]
                .into_iter()
                .chain([super::ALLOW_LINKED_WRITES]),
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "override init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&primary_identity_path).unwrap(),
        primary_before,
        "initializing the selected linked store must not refresh the primary identity"
    );
    let linked_identity: Value =
        serde_json::from_slice(&fs::read(&linked_identity_path).unwrap()).unwrap();
    assert_eq!(
        linked_identity["root"].as_str(),
        Some(worktree_path.to_str().unwrap())
    );
    assert_eq!(linked_identity["branch"].as_str(), Some(linked_branch));
    assert_ne!(
        linked_identity["branch"], primary_identity["branch"],
        "the linked identity must use the selected checkout's branch"
    );
    assert_ne!(
        linked_identity["worktree_id"], primary_identity["worktree_id"],
        "a copied primary identity must be replaced for the selected linked checkout"
    );
}

#[test]
fn test_worktree_info_creates_identity_file() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // Run worktree info to create identity
    cli.command(["worktree", "info"]).assert().success();

    // Verify worktree.json was created
    let wt_file = cli.root().join(".jit/worktree.json");
    assert!(wt_file.exists(), "worktree.json should be created");

    // Verify it's valid JSON
    let content = fs::read_to_string(&wt_file).unwrap();
    let _: Value = serde_json::from_str(&content).unwrap();
}

#[test]
fn test_worktree_info_stable_across_invocations() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // First invocation
    let output1 = cli
        .command(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json1: Value = serde_json::from_slice(&output1.stdout).unwrap();
    let id1 = json1["worktree_id"].as_str().unwrap();

    // Second invocation
    let output2 = cli
        .command(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json2: Value = serde_json::from_slice(&output2.stdout).unwrap();
    let id2 = json2["worktree_id"].as_str().unwrap();

    assert_eq!(id1, id2, "Worktree ID should be stable");
}

/// Relocation must surface as a shared success warning on the `worktree info` command
/// path (regression guard: storage no longer prints it, the output layer does).
/// Previously this condition was an `eprintln!` from storage on every command;
/// it must still surface here, not only via `recover`.
#[test]
fn test_worktree_info_surfaces_relocation_warning() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // First run creates worktree.json with root == this repo.
    cli.command(["worktree", "info"]).assert().success();

    // Rewrite the recorded root to a non-existent path. On the next run the
    // worktree is detected at a different (current) path, which is a relocation.
    let wt_file = cli.root().join(".jit/worktree.json");
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&wt_file).unwrap()).unwrap();
    identity["root"] = Value::String("/nonexistent/old/worktree/path".to_string());
    fs::write(&wt_file, serde_json::to_string_pretty(&identity).unwrap()).unwrap();

    // --json: the relocation must appear in the structured warnings payload,
    // and nothing must leak onto stderr.
    let json_output = cli
        .command(["worktree", "info", "--json"])
        .output()
        .unwrap();
    assert!(json_output.status.success());
    let json: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    let warnings = json["warnings"].as_array().expect("warnings array present");
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .is_some_and(|text| text.contains("Worktree relocated"))),
        "worktree info --json must include the relocation warning, got {warnings:?}"
    );
    let json_stderr = String::from_utf8_lossy(&json_output.stderr);
    assert!(
        json_stderr.is_empty(),
        "relocation diagnostic must not leak to stderr under --json, got: {json_stderr}"
    );

    // Re-seed the relocation and check the text path renders it on stderr.
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&wt_file).unwrap()).unwrap();
    identity["root"] = Value::String("/nonexistent/old/worktree/path".to_string());
    fs::write(&wt_file, serde_json::to_string_pretty(&identity).unwrap()).unwrap();

    cli.command(["worktree", "info"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Worktree relocated"));
}

// === jit worktree list tests ===

#[test]
fn test_worktree_list_without_checkout_reports_shared_git_requirement_and_remedy() {
    let cli = JitCli::initialised().unwrap();
    let remedy = RequiresGitError::new(GitRequirementGap::NoRepository).to_string();
    let output = cli
        .command(["worktree", "list", "--json"])
        .output()
        .unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], ErrorCode::RequiresGit.as_str());
    assert_eq!(response["error"]["message"], remedy);

    cli.command(["worktree", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(remedy));
}

#[test]
fn test_worktree_list_omits_detached_checkout() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    linked.git(["checkout", "--detach", "HEAD"]).unwrap();

    let response: Value = cli.run_json(["worktree", "list"]).unwrap();
    let paths: Vec<_> = response["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec![cli.root().to_str().unwrap()]);
}

#[test]
fn test_worktree_list_single_worktree() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // List worktrees (only main worktree)
    cli.command(["worktree", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main").or(predicate::str::contains("master")));
}

#[test]
fn test_worktree_list_multiple_worktrees() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    let linked_branch = linked.git(["branch", "--show-current"]).unwrap();

    linked.command(["worktree", "info"]).assert().success();

    // List from main repo - should see the worktree
    let stdout = cli.run(["worktree", "list"]).unwrap();
    assert!(
        stdout.contains(linked_branch.trim()),
        "Should list the new worktree: {}",
        stdout
    );
}

#[test]
fn test_worktree_list_from_linked_checkout_identifies_primary() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_path = linked.root();

    let output = linked
        .command(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "worktree list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let worktrees = json["worktrees"].as_array().unwrap();
    assert!(worktrees.iter().any(|entry| {
        entry["path"].as_str() == Some(cli.root().to_str().unwrap()) && entry["is_main"] == true
    }));
    assert!(worktrees.iter().any(|entry| {
        entry["path"].as_str() == Some(worktree_path.to_str().unwrap()) && entry["is_main"] == false
    }));
}

#[test]
fn test_worktree_list_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    let output = cli
        .command(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["worktrees"].is_array());

    let worktrees = json["worktrees"].as_array().unwrap();
    assert!(!worktrees.is_empty(), "Should have at least one worktree");

    // Check first worktree has expected fields
    assert!(worktrees[0]["path"].is_string());
    assert!(worktrees[0]["branch"].is_string());
}

/// Regression for the reported bug: `jit worktree list` showed stale
/// `active_claims` after a lease TTL expired, while `jit claim list` (which
/// evicts expired leases) correctly returned 0. The two views must agree.
/// CLI-boundary smoke test that `worktree list` and `claim list` are wired to
/// the same active-lease view for live leases (happy path).
///
/// Expiry-boundary exclusion (the deflaked behavior) is covered deterministically
/// in-process by `commands::worktree::tests::test_worktree_list_excludes_expired_leases`,
/// which drives an injected clock past a lease's TTL instead of sleeping. That
/// trade-off keeps this test free of any wall-clock timing while still exercising
/// the real CLI wiring for the common case.
#[test]
fn test_worktree_list_agrees_with_claim_list_for_live_leases() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    let short_id = cli.create_issue(["--title", "short-lived"]).unwrap();
    let long_id = cli.create_issue(["--title", "long-lived"]).unwrap();

    // Acquire two live leases from the main worktree.
    cli.command([
        "claim",
        "acquire",
        &short_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test",
    ])
    .assert()
    .success();
    cli.command([
        "claim",
        "acquire",
        &long_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test",
    ])
    .assert()
    .success();

    // Both leases are live, so `worktree list` and `claim list` must agree.
    let active: u64 = cli.run_json(["worktree", "list"]).unwrap()["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["active_claims"].as_u64().unwrap())
        .sum();
    let claims = cli.run_json(["claim", "list"]).unwrap()["leases"]
        .as_array()
        .unwrap()
        .len() as u64;
    assert_eq!(active, 2, "both live leases should be counted");
    assert_eq!(
        active, claims,
        "worktree list active_claims must agree with claim list count"
    );
}

// === jit validate tests ===

#[test]
fn test_validate_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    cli.command(["validate"]).assert().success();
}

#[test]
fn test_validate_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    let output = cli.command(["validate", "--json"]).output().unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["valid"], true);
}

fn seed_origin_main(cli: &JitCli, commit: &str) -> String {
    cli.git(["update-ref", "refs/remotes/origin/main", commit])
        .unwrap();
    cli.git(["rev-parse", "origin/main"])
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn test_git_guards_act_on_the_repository_they_serve_rather_than_the_working_directory() {
    let up_to_date = JitCli::initialised().unwrap();
    up_to_date.init_git().unwrap().commit("base").unwrap();
    seed_origin_main(&up_to_date, "HEAD");

    let diverged = JitCli::initialised().unwrap();
    diverged.init_git().unwrap().commit("base").unwrap();
    let ahead = diverged
        .git(["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "ahead"])
        .unwrap();
    let ahead = seed_origin_main(&diverged, ahead.trim());

    let without_git = JitCli::initialised().unwrap();
    let outside_git = JitCli::uninitialised().unwrap();

    let data_root = |target: &JitCli| [("JIT_DATA_DIR", target.root().join(".jit"))];
    let define = |key: &'static str| {
        [
            "gate",
            "define",
            key,
            "--title",
            "t",
            "--description",
            "d",
            "--mode",
            "manual",
        ]
    };

    // A diverged working directory must not refuse a repository that is up to date.
    diverged
        .command_with_env(define("wrong-refusal"), data_root(&up_to_date))
        .assert()
        .success();
    // A working directory outside git must not bypass the guard of a diverged repository.
    outside_git
        .command_with_env(define("bypass"), data_root(&diverged))
        .assert()
        .failure()
        .stderr(predicate::str::contains("origin/main"));
    // A repository without git has no history to guard (`@/charter/D-4`), and a
    // diverged working directory does not lend it one.
    diverged
        .command_with_env(define("git-absent"), data_root(&without_git))
        .assert()
        .success();

    diverged
        .command_with_env(["validate", "--branch-drift"], data_root(&up_to_date))
        .assert()
        .success();
    outside_git
        .command_with_env(["validate", "--branch-drift"], data_root(&diverged))
        .assert()
        .failure()
        .stderr(predicate::str::contains(ahead));
}

#[test]
fn test_validate_branch_drift_accepts_a_branch_that_matches_origin_main() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let remote = JitCli::uninitialised().unwrap();
    remote.git(["init", "--bare"]).unwrap();
    cli.git(["remote", "add", "origin", remote.root().to_str().unwrap()])
        .unwrap();
    cli.git(["push", "origin", "HEAD:refs/heads/main"]).unwrap();
    cli.git(["fetch", "origin"]).unwrap();

    cli.command(["validate", "--branch-drift"])
        .assert()
        .success();
}

#[test]
fn test_validate_branch_drift_detects_drifted_branch() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let remote = JitCli::uninitialised().unwrap();
    remote.git(["init", "--bare"]).unwrap();
    cli.git(["remote", "add", "origin", remote.root().to_str().unwrap()])
        .unwrap();
    cli.git(["push", "origin", "HEAD:refs/heads/main"]).unwrap();
    cli.git(["fetch", "origin"]).unwrap();

    let initial_commit = cli.git(["rev-parse", "HEAD"]).unwrap();
    let initial_commit = initial_commit.trim();

    fs::write(cli.root().join("README.md"), "# Test\nUpdated\n").unwrap();
    cli.commit("Advance main").unwrap();
    cli.git(["push", "origin", "HEAD:refs/heads/main"]).unwrap();
    cli.git(["fetch", "origin"]).unwrap();
    cli.git(["checkout", "-b", "stale", initial_commit])
        .unwrap();

    let output = cli
        .command(["validate", "--branch-drift"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Branch has diverged from origin/main"),
        "branch-drift failure explains the divergence: {stderr}"
    );
    assert!(
        stderr.contains("Fix: git rebase origin/main"),
        "branch-drift failure gives the rebase remedy: {stderr}"
    );

    let json_output = cli
        .command(["validate", "--branch-drift", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_output.status.code(), Some(4));
    assert!(json_output.stderr.is_empty(), "JSON stays stream-pure");
    let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
}

#[test]
fn test_validate_divergence_spelling_hints_branch_drift() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // `--divergence` names the membership-vs-DAG report, so `jit validate`
    // rejects it with a hint rather than a bare clap error.
    cli.command(["validate", "--divergence"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("jit validate --branch-drift"))
        .stderr(predicates::str::contains("jit query divergence"));
}

#[test]
fn test_validate_leases_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // Validate leases (should pass with no leases)
    cli.command(["validate", "--leases"]).assert().success();
}

// === jit recover tests ===

#[test]
fn test_recover_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    cli.command(["recover"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery"));
}

#[test]
fn test_recover_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    let output = cli.command(["recover", "--json"]).output().unwrap();

    assert!(output.status.success());

    let _json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Envelope removed - just check valid JSON
}

// === jit init in worktrees tests ===

#[test]
fn test_init_in_new_worktree_generates_unique_id() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();

    let worktree_path = linked.root();

    // Run jit init in the worktree
    linked
        .command_with_env(["init"], [super::ALLOW_LINKED_WRITES])
        .assert()
        .success();

    // Get worktree IDs from both locations
    let main_wt_file = cli.root().join(".jit/worktree.json");
    let worktree_wt_file = worktree_path.join(".jit/worktree.json");

    let main_content = fs::read_to_string(&main_wt_file).unwrap();
    let main_identity: Value = serde_json::from_str(&main_content).unwrap();

    let worktree_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let worktree_identity: Value = serde_json::from_str(&worktree_content).unwrap();

    // IDs should be different
    assert_ne!(
        main_identity["worktree_id"], worktree_identity["worktree_id"],
        "Main and worktree should have different IDs"
    );

    // Worktree should have correct root path
    assert_eq!(
        worktree_identity["root"].as_str().unwrap(),
        worktree_path.to_string_lossy().to_string()
    );
}

#[test]
fn test_init_is_idempotent_in_worktree() {
    let (_primary, linked) = JitCli::linked_checkouts().unwrap();

    let worktree_path = linked.root();

    // Run jit init first time
    linked
        .command_with_env(["init"], [super::ALLOW_LINKED_WRITES])
        .assert()
        .success();

    let worktree_wt_file = worktree_path.join(".jit/worktree.json");
    let first_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let first_identity: Value = serde_json::from_str(&first_content).unwrap();
    let first_id = first_identity["worktree_id"].as_str().unwrap();

    // Run jit init second time
    linked
        .command_with_env(["init"], [super::ALLOW_LINKED_WRITES])
        .assert()
        .success();

    let second_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let second_identity: Value = serde_json::from_str(&second_content).unwrap();
    let second_id = second_identity["worktree_id"].as_str().unwrap();

    // ID should be unchanged
    assert_eq!(first_id, second_id, "Init should be idempotent");
}

#[test]
fn test_worktree_list_shows_distinct_ids() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    let second = JitCli::uninitialised().unwrap();
    cli.git([
        "worktree",
        "add",
        "-q",
        "-b",
        "second",
        second.root().to_str().unwrap(),
    ])
    .unwrap();

    [&linked, &second].iter().for_each(|checkout| {
        checkout
            .command_with_env(["init"], [super::ALLOW_LINKED_WRITES])
            .assert()
            .success();
    });

    // List worktrees from main repo
    let output = cli
        .command(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let worktrees = json["worktrees"].as_array().unwrap();

    // Collect all worktree IDs
    let mut ids = std::collections::HashSet::new();
    for wt in worktrees {
        if let Some(id) = wt["worktree_id"].as_str() {
            ids.insert(id.to_string());
        }
    }

    // Should have at least 3 distinct IDs (main + 2 worktrees)
    assert!(
        ids.len() >= 3,
        "Should have at least 3 distinct worktree IDs"
    );
}

#[test]
fn test_git_worktree_move_preserves_id() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();
    linked
        .command_with_env(["init"], [super::ALLOW_LINKED_WRITES])
        .assert()
        .success();
    let id_before = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"].clone();

    let moved_path = linked.root().with_file_name("feature-moved");
    cli.git([
        "worktree",
        "move",
        linked.root().to_str().unwrap(),
        moved_path.to_str().unwrap(),
    ])
    .unwrap();

    let output = linked
        .command(["worktree", "info", "--json"])
        .current_dir(&moved_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let id_after = serde_json::from_slice::<Value>(&output.stdout).unwrap()["worktree_id"].clone();

    assert!(id_before.is_string());
    assert_eq!(
        id_before, id_after,
        "Worktree ID should be preserved after git worktree move"
    );
}

#[test]
fn test_worktree_store_divergence_json_and_rendered_output_name_the_same_findings() {
    let (cli, linked) = JitCli::linked_checkouts().unwrap();

    // Diverge the two stores in both directions: one record each store holds
    // alone, each carrying its own creation event into that store's log.
    cli.create_issue(["--title", "Held by the primary checkout alone"])
        .unwrap();
    linked
        .command_with_env(
            [
                "issue",
                "create",
                "--title",
                "Held by the linked checkout alone",
            ],
            [super::ALLOW_LINKED_WRITES],
        )
        .assert()
        .success();

    let machine = linked.run_json(["worktree", "store-divergence"]).unwrap();
    let divergences = machine["divergences"].as_array().unwrap();

    assert!(
        !divergences.is_empty(),
        "the two diverged stores report findings: {machine}"
    );

    let rendered = linked.run(["worktree", "store-divergence"]).unwrap();
    for divergence in divergences {
        let record = divergence["record"].as_str().unwrap();
        let class = divergence["class"].as_str().unwrap();
        let id = divergence["id"].as_str().unwrap();
        assert!(
            rendered
                .lines()
                .any(|line| line.contains(record) && line.contains(class) && line.contains(id)),
            "the rendered output names the finding {record}/{class}/{id}:\n{rendered}"
        );
    }
    assert!(
        rendered.contains(&format!("{} divergence(s)", divergences.len())),
        "the rendered output reports the same finding count as the envelope:\n{rendered}"
    );

    let tokens = |field: &str| -> Vec<String> {
        divergences
            .iter()
            .map(|divergence| divergence[field].as_str().unwrap().to_string())
            .collect()
    };
    let classes = tokens("class");
    assert!(
        [DivergenceClass::LocalOnly, DivergenceClass::ReferenceOnly]
            .iter()
            .all(|direction| classes.iter().any(|class| class == direction.as_str())),
        "a store divergence is reported in both directions: {machine}"
    );
    let records = tokens("record");
    assert!(
        DivergentRecord::ALL
            .iter()
            .all(|kind| records.iter().any(|record| record == kind.as_str())),
        "both issue records and event records are compared: {machine}"
    );
}

#[test]
fn test_worktree_info_failure_uses_shared_classifier_in_both_modes() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    cli.command(["worktree", "info"]).assert().success();
    fs::write(cli.root().join(".jit/worktree.json"), "{").unwrap();
    let human = cli.command(["worktree", "info"]).output().unwrap();
    let json = cli
        .command(["worktree", "info", "--json"])
        .output()
        .unwrap();
    let failure: Value = serde_json::from_slice(&json.stdout).unwrap();
    let expected = jit::output::failure::classify(&anyhow::Error::new(
        serde_json::from_str::<Value>("{").unwrap_err(),
    ));
    assert_eq!(failure["error"]["code"], expected.error.code.as_str());
    assert_eq!(human.status.code(), Some(expected.exit_code().code()));
    assert_eq!(human.status.code(), json.status.code());
    assert!(String::from_utf8_lossy(&human.stderr)
        .contains(failure["error"]["message"].as_str().unwrap()));
    assert!(json.stderr.is_empty());
}
