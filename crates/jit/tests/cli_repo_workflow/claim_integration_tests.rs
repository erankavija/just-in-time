//! Integration tests for claim CLI commands
//!
//! Tests the full claim workflow: acquire → list → release
//! Verifies actual binary execution, exit codes, and output formats.

use assert_cmd::prelude::*;
use jit::domain::identity::IssueId;
use jit::output::ErrorCode;
use jit::storage::worktree_paths::WorktreePaths;
use jit::test_cli::JitCli;
use jit::test_lease_store::{history_path, lease_path, read_lease};
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use std::str::FromStr;

#[test]
fn test_issue_release() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Task"]).unwrap();
    cli.run(["issue", "claim", &id, "agent:worker-1"]).unwrap();

    cli.run(["issue", "release", &id, "timeout"]).unwrap();

    assert!(cli.run_json(["issue", "show", &id]).unwrap()["assignee"].is_null());
}

/// Declared by every invocation selecting a linked checkout's store, rather
/// than inheriting a default (jit:f52567ed REQ-05).
#[test]
fn test_claim_acquire_uses_selected_linked_worktree_identity_from_primary_cwd() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let issue_id = primary
        .create_issue(["--title", "Selected linked claim"])
        .unwrap();
    primary.commit("Add selected linked claim").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    let linked_branch = linked.git(["branch", "--show-current"]).unwrap();
    let linked_branch = linked_branch.trim();
    let primary_identity_path = primary.root().join(".jit/worktree.json");
    let primary_identity_before = fs::read(&primary_identity_path).unwrap();
    let linked_store = linked.root().join(".jit");

    let output = primary
        .command_with_env(
            [
                "claim",
                "acquire",
                &issue_id,
                "--ttl",
                "600",
                "--agent-id",
                "agent:selected-linked",
                "--json",
            ],
            [
                ("JIT_DATA_DIR", linked_store.to_str().unwrap()),
                super::ALLOW_LINKED_WRITES,
            ],
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "claim acquire failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&primary_identity_path).unwrap(),
        primary_identity_before
    );

    let primary_identity: Value = serde_json::from_slice(&primary_identity_before).unwrap();
    let linked_identity: Value =
        serde_json::from_slice(&fs::read(linked_store.join("worktree.json")).unwrap()).unwrap();
    assert_eq!(
        Path::new(linked_identity["root"].as_str().unwrap()),
        linked.root()
    );
    assert_eq!(linked_identity["branch"], linked_branch);
    assert_ne!(
        linked_identity["worktree_id"],
        primary_identity["worktree_id"]
    );

    let paths = WorktreePaths::detect_for_data_root(&linked_store, primary.root()).unwrap();
    let lease = read_lease(&paths.shared_jit, &parse_issue_id(&issue_id))
        .unwrap()
        .expect("selected issue has a lease");
    assert_eq!(
        lease.worktree_id.as_str(),
        linked_identity["worktree_id"].as_str().unwrap()
    );
    assert_eq!(lease.branch.as_deref(), Some(linked_branch));
}

#[test]
fn test_lifecycle_lease_check_uses_selected_linked_authority_from_foreign_primary_cwd() {
    let (selected_primary, linked) = JitCli::linked_checkouts().unwrap();
    let issue_id = selected_primary
        .create_issue(["--title", "Selected linked lifecycle"])
        .unwrap();
    selected_primary
        .commit("Add selected linked lifecycle")
        .unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    let agent_id = "agent:selected-linked-lifecycle";
    let linked_store = linked.root().join(".jit");
    let selected_store = ("JIT_DATA_DIR", linked_store.to_str().unwrap());
    let acquire = selected_primary
        .command_with_env(
            [
                "claim",
                "acquire",
                &issue_id,
                "--ttl",
                "600",
                "--agent-id",
                agent_id,
                "--json",
            ],
            [selected_store, super::ALLOW_LINKED_WRITES],
        )
        .output()
        .unwrap();
    assert!(
        acquire.status.success(),
        "claim acquire failed: {}",
        String::from_utf8_lossy(&acquire.stderr)
    );
    let mut config = fs::read_to_string(linked_store.join("config.toml")).unwrap();
    config.push_str("\n[worktree]\nenforce_leases = \"strict\"\n");
    fs::write(linked_store.join("config.toml"), config).unwrap();
    let foreign_primary = JitCli::initialised().unwrap();
    foreign_primary
        .init_git()
        .unwrap()
        .commit("Initial commit")
        .unwrap();

    let output = foreign_primary
        .command_with_env(
            [
                "issue",
                "assign",
                &issue_id,
                "human:selected-owner",
                "--json",
            ],
            [
                selected_store,
                super::ALLOW_LINKED_WRITES,
                ("JIT_AGENT_ID", agent_id),
            ],
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "selected-root lease was rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let issue: Value = serde_json::from_slice(
        &fs::read(linked_store.join("issues").join(format!("{issue_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(issue["assignee"], "human:selected-owner");
}

fn assert_claim_failure_parity(plain: &Output, json: &Output, expected_code: ErrorCode) {
    assert!(!plain.status.success());
    assert!(!json.status.success());

    assert!(plain.stdout.is_empty(), "plain failure leaked to stdout");
    assert!(
        !plain.stderr.is_empty(),
        "plain failure omitted its diagnostic"
    );
    assert!(json.stderr.is_empty(), "JSON failure leaked to stderr");

    let envelope: Value = serde_json::from_slice(&json.stdout)
        .expect("JSON failure stdout contains exactly one document");
    assert_eq!(envelope.as_object().map(|object| object.len()), Some(1));
    let error = envelope["error"]
        .as_object()
        .expect("canonical failure envelope contains an error object");
    assert_eq!(error.len(), 2);
    let actual_code = ErrorCode::from_str(
        error["code"]
            .as_str()
            .expect("canonical failure carries a registered code"),
    )
    .expect("claim failure code belongs to the registered vocabulary");
    assert_eq!(actual_code, expected_code);
    assert!(error["message"].is_string());
}

/// The id a binary printed, parsed once where a test hands it to suite helpers.
fn parse_issue_id(printed: &str) -> IssueId {
    printed
        .parse()
        .expect("the binary prints a canonical issue id")
}

fn acquire_lease(cli: &JitCli, issue_id: &IssueId, agent_id: &str) -> String {
    cli.run_json([
        "claim",
        "acquire",
        issue_id.as_str(),
        "--ttl",
        "600",
        "--agent-id",
        agent_id,
    ])
    .expect("acquire fixture lease")["lease_id"]
        .as_str()
        .expect("acquire response carries lease id")
        .to_string()
}

#[cfg(unix)]
#[test]
fn test_claim_renew_permission_denied_is_identical_in_plain_and_json_forms() {
    use std::os::unix::fs::PermissionsExt;

    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = parse_issue_id(
        &cli.create_issue(["--title", "Permission-denied claim"])
            .unwrap(),
    );
    let agent = "agent:permission-probe";
    acquire_lease(&cli, &issue_id, agent);
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    // Renew reads the lease under the lock from whichever path stores it.
    let stored = [
        lease_path(&paths.shared_jit, &issue_id),
        history_path(&paths.shared_jit),
    ];
    let original_modes = stored
        .iter()
        .map(|path| fs::metadata(path).unwrap().permissions().mode())
        .collect::<Vec<_>>();
    for path in &stored {
        fs::set_permissions(path, fs::Permissions::from_mode(0o0)).unwrap();
    }

    let invoke = |json: bool| {
        let mut command = cli.command_with_env(
            ["claim", "renew", issue_id.as_str(), "--extension", "300"],
            [("JIT_AGENT_ID", agent)],
        );
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    let plain = invoke(false);
    let json = invoke(true);

    for (path, mode) in stored.iter().zip(original_modes) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    assert_claim_failure_parity(&plain, &json, ErrorCode::PermissionDenied);
}

/// A failure the classifier leaves generic reports the generic code and status
/// on both paths, not a per-verb claim code.
#[test]
fn test_claim_acquire_already_claimed_reports_generic_error_on_both_paths() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Contended claim"]).unwrap();
    let invoke = |agent: &str, json: bool| {
        let mut command = cli.command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            agent,
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    assert!(invoke("agent:holder", false).status.success());

    let plain = invoke("agent:rival", false);
    let json = invoke("agent:rival", true);

    assert_claim_failure_parity(&plain, &json, ErrorCode::GenericError);
    assert_eq!(plain.status.code(), json.status.code());
    assert_eq!(
        json.status.code(),
        Some(ErrorCode::GenericError.exit_code().code())
    );
}

#[test]
fn test_claim_acquire_external_io_is_identical_in_plain_and_json_forms() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "External-I/O claim"]).unwrap();
    let locks_path = cli.root().join(".git/jit/locks");
    fs::create_dir_all(&locks_path).unwrap();
    fs::remove_dir(&locks_path).unwrap();
    fs::write(&locks_path, "not a directory").unwrap();

    let invoke = |json: bool| {
        let mut command = cli.command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:io-probe",
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    let plain = invoke(false);
    let json = invoke(true);

    assert_claim_failure_parity(&plain, &json, ErrorCode::IoError);
}

#[test]
fn test_claim_acquire_happy_path() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire claim
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Acquired lease"));
}

#[test]
fn test_claim_acquire_stamps_claimed_at_and_logs_event() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    let acquire = |agent: &str| {
        cli.command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            agent,
        ])
        .assert()
        .success();
    };

    acquire("agent:test-1");

    // The lease-acquire path stamped claimed_at on the issue record.
    let show = cli
        .command(["issue", "show", &issue_id, "--json"])
        .output()
        .unwrap();
    let show_json: Value = serde_json::from_slice(&show.stdout).unwrap();
    let first_claimed = show_json["claimed_at"]
        .as_str()
        .expect("claimed_at must be set after claim acquire")
        .to_string();

    // @/inv/event-log: an issue_claimed event was appended.
    let events = cli
        .command(["events", "query", "--event-type", "issue_claimed", "--json"])
        .output()
        .unwrap();
    let events_json: Value = serde_json::from_slice(&events.stdout).unwrap();
    assert_eq!(events_json["events"].as_array().unwrap().len(), 1);

    // First-occurrence: re-acquiring as the same agent (release then re-acquire)
    // must not move claimed_at.
    cli.command(["claim", "release", &issue_id])
        .assert()
        .success();
    acquire("agent:test-1");

    let show2 = cli
        .command(["issue", "show", &issue_id, "--json"])
        .output()
        .unwrap();
    let show2_json: Value = serde_json::from_slice(&show2.stdout).unwrap();
    assert_eq!(show2_json["claimed_at"].as_str().unwrap(), first_claimed);
}

#[test]
fn test_claim_acquire_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire claim with JSON output
    let output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // success field removed
    assert!(json["lease_id"].is_string());
    assert_eq!(json["issue_id"], issue_id);
    assert_eq!(json["ttl_secs"], 600);
}

#[test]
fn test_claim_acquire_already_claimed_error() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // First claim succeeds
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .success();

    // Second claim should fail with actionable error
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-2",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("already claimed"))
    .stderr(predicate::str::contains("Possible causes:"))
    .stderr(predicate::str::contains("To fix:"))
    .stderr(predicate::str::contains("jit claim status"));
}

#[test]
fn test_claim_list_shows_active_leases() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire a claim
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .success();

    // List should show the lease
    cli.command(["claim", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_list_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire a claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // List claims with JSON
    let list_output = cli.command(["claim", "list", "--json"]).output().unwrap();

    assert!(list_output.status.success());

    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    assert!(list_json["leases"].is_array());

    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);
    assert_eq!(leases[0]["issue_id"], issue_id);
}

#[test]
fn test_claim_release_happy_path() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let _lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Release claim by ISSUE id (no lease UUID required).
    cli.command_with_env(
        ["claim", "release", &issue_id],
        [("JIT_AGENT_ID", "agent:test-1")],
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Released lease"));
}

#[test]
fn test_claim_release_no_active_lease_error() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Unclaimed Issue"]).unwrap();

    // Releasing an existing issue with no active lease should fail with an
    // actionable error that mentions "not found".
    cli.command_with_env(
        ["claim", "release", &issue_id],
        [("JIT_AGENT_ID", "agent:test")],
    )
    .assert()
    .failure()
    .stderr(predicate::str::contains("no active lease"))
    .stderr(predicate::str::contains("not found"))
    .stderr(predicate::str::contains("Possible causes:"))
    .stderr(predicate::str::contains("To fix:"))
    .stderr(predicate::str::contains("jit claim list"));
}

#[test]
fn test_claim_release_by_issue_succeeds_for_different_owner() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Owned Issue"]).unwrap();

    // Agent A acquires the lease.
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:owner",
        "--json",
    ])
    .assert()
    .success();

    // A DIFFERENT agent releases by issue id; the audit reports the prior owner
    // and the acting identity.
    let release_output = cli
        .command_with_env(
            ["claim", "release", &issue_id, "--json"],
            [("JIT_AGENT_ID", "agent:stranger")],
        )
        .output()
        .unwrap();

    assert!(release_output.status.success());
    let release_json: Value = serde_json::from_slice(&release_output.stdout).unwrap();
    assert_eq!(release_json["issue_id"], issue_id);
    assert_eq!(release_json["previous_owner"], "agent:owner");
    assert_eq!(release_json["actor"], "agent:stranger");

    // Lease is gone.
    let list_output = cli.command(["claim", "list", "--json"]).output().unwrap();
    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    assert_eq!(list_json["leases"].as_array().unwrap().len(), 0);
}

#[test]
fn test_claim_workflow_end_to_end() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Workflow"])
        .unwrap();

    // Step 1: Acquire claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:workflow-test",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());
    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Step 2: List claims and verify
    let list_output = cli.command(["claim", "list", "--json"]).output().unwrap();

    assert!(list_output.status.success());
    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);

    // Step 3: Release claim by issue id
    cli.command_with_env(
        ["claim", "release", &issue_id],
        [("JIT_AGENT_ID", "agent:workflow-test")],
    )
    .assert()
    .success();

    // Step 4: Verify lease is gone
    let list_after_output = cli.command(["claim", "list", "--json"]).output().unwrap();

    let list_after_json: Value = serde_json::from_slice(&list_after_output.stdout).unwrap();
    let leases_after = list_after_json["leases"].as_array().unwrap();
    assert_eq!(
        leases_after.len(),
        0,
        "Lease should be removed after release"
    );
}

#[test]
fn test_recover_reports_a_reclaimed_stale_lease_without_appending_an_event() {
    use chrono::{Duration, Utc};

    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = parse_issue_id(&cli.create_issue(["--title", "Abandoned lease"]).unwrap());
    let silent_since = Utc::now()
        - Duration::seconds(jit::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64 + 60);
    let lease = jit::storage::Lease {
        lease_id: "00000000-0000-4000-8000-0000000000ab".parse().unwrap(),
        issue_id: issue_id.clone(),
        agent_id: "agent:gone".parse().unwrap(),
        worktree_id: "wt:gone".parse().unwrap(),
        branch: None,
        ttl_secs: 0,
        acquired_at: silent_since,
        expires_at: None,
        last_beat: silent_since,
    };
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    jit::test_lease_store::plant_lease(&paths.shared_jit, &lease).unwrap();
    let events = || fs::read(cli.root().join(".jit/events.jsonl")).unwrap_or_default();
    let events_before = events();

    let recovered = cli.command(["recover", "--json"]).output().unwrap();

    assert!(recovered.status.success(), "{recovered:?}");
    let report: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(report["stale_leases_reclaimed"], 1, "{report}");
    assert_eq!(report["expired_leases_evicted"], 0, "{report}");
    assert_eq!(
        jit::test_lease_store::read_lease(&paths.shared_jit, &issue_id).unwrap(),
        None
    );
    assert_eq!(events(), events_before, "reclamation appends no event");
}

#[test]
fn test_claim_status_shows_lease_details() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    // Acquire claim
    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .success();

    // Check status
    cli.command_with_env(["claim", "status"], [("JIT_AGENT_ID", "agent:test-1")])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_acquire_nonexistent_issue_error() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // Try to claim non-existent issue
    cli.command([
        "claim",
        "acquire",
        "00000000-0000-0000-0000-000000000000",
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not found").or(predicate::str::contains("Issue")));
}

#[test]
fn test_claim_renew_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Renew"])
        .unwrap();

    // Acquire claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());

    // Renew claim
    cli.command_with_env(
        ["claim", "renew", &issue_id, "--extension", "300"],
        [("JIT_AGENT_ID", "agent:test-1")],
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Renewed lease"));
}

#[test]
fn test_claim_renew_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Renew JSON"])
        .unwrap();

    // Acquire claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Renew with JSON output
    let renew_output = cli
        .command_with_env(
            ["claim", "renew", &issue_id, "--extension", "300", "--json"],
            [("JIT_AGENT_ID", "agent:test-1")],
        )
        .output()
        .unwrap();

    assert!(renew_output.status.success());

    let renew_json: Value = serde_json::from_slice(&renew_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    // Check that lease object is returned
    assert!(renew_json["lease"].is_object());
    assert_eq!(renew_json["lease"]["lease_id"], lease_id);
}

#[test]
fn test_claim_renew_of_an_issue_without_a_lease_fails_as_not_found() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    // A held lease on another issue brings up the claim control plane.
    let leased = parse_issue_id(&cli.create_issue(["--title", "Leased"]).unwrap());
    acquire_lease(&cli, &leased, "agent:holder");
    let unleased = cli.create_issue(["--title", "Unleased"]).unwrap();

    let invoke = |json: bool| {
        let mut command = cli.command_with_env(
            ["claim", "renew", &unleased, "--extension", "300"],
            [("JIT_AGENT_ID", "agent:test")],
        );
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };

    assert_claim_failure_parity(&invoke(false), &invoke(true), ErrorCode::IssueNotFound);
}

#[test]
fn test_claim_force_evict_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Force-Evict"])
        .unwrap();

    // Acquire claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());

    // Force-evict (admin operation - doesn't need same agent)
    cli.command([
        "claim",
        "force-evict",
        &issue_id,
        "--reason",
        "test cleanup",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Force-evicted"));

    // Verify lease is gone
    let list_output = cli.command(["claim", "list", "--json"]).output().unwrap();

    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 0, "Lease should be evicted");
}

#[test]
fn test_claim_force_evict_removes_the_lease_of_a_deleted_issue() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = parse_issue_id(&cli.create_issue(["--title", "Deleted"]).unwrap());
    let lease_id = acquire_lease(&cli, &issue_id, "agent:holder");
    cli.command_with_env(
        ["issue", "delete", issue_id.as_str()],
        [
            ("JIT_ALLOW_DELETION", "1"),
            ("JIT_AGENT_ID", "agent:holder"),
        ],
    )
    .assert()
    .success();

    let evicted = cli
        .run_json([
            "claim",
            "force-evict",
            issue_id.short_id(),
            "--reason",
            "issue deleted",
        ])
        .unwrap();

    assert_eq!(evicted["lease_id"], lease_id.as_str());
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    assert!(!lease_path(&paths.shared_jit, &issue_id).exists());
}

#[test]
fn test_claim_force_evict_requires_reason() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();

    // Force-evict without reason should fail (clap validation)
    cli.command([
        "claim",
        "force-evict",
        "00000000-0000-4000-8000-000000000001",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("--reason"));
}

#[test]
fn test_claim_heartbeat_success() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Heartbeat"])
        .unwrap();

    // Acquire indefinite claim (TTL=0)
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:test-1",
            "--reason",
            "manual review",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        acquire_output.status.success(),
        "Acquire should succeed: {}",
        String::from_utf8_lossy(&acquire_output.stderr)
    );

    // Heartbeat to update last_beat
    cli.command_with_env(
        ["claim", "heartbeat", &issue_id],
        [("JIT_AGENT_ID", "agent:test-1")],
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Heartbeat sent"));
}

#[test]
fn test_claim_heartbeat_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue for Heartbeat JSON"])
        .unwrap();

    // Acquire indefinite claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:test-1",
            "--reason",
            "manual review",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Heartbeat with JSON output
    let heartbeat_output = cli
        .command_with_env(
            ["claim", "heartbeat", &issue_id, "--json"],
            [("JIT_AGENT_ID", "agent:test-1")],
        )
        .output()
        .unwrap();

    assert!(heartbeat_output.status.success());

    let hb_json: Value = serde_json::from_slice(&heartbeat_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    assert_eq!(hb_json["lease_id"], lease_id);
    assert!(hb_json["message"].is_string());
}

#[test]
fn test_claim_ttl0_workflow_acquire_heartbeat_release() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "TTL=0 Workflow Test"])
        .unwrap();

    // Step 1: Acquire indefinite claim
    let acquire_output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:workflow-test",
            "--reason",
            "integration test",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());
    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();
    assert!(
        acquire_json["expires_at"].is_null(),
        "Indefinite lease should have no expiry"
    );
    assert_eq!(acquire_json["ttl_secs"], 0);

    // Step 2: List claims and verify indefinite marker
    let list_output = cli.command(["claim", "list"]).output().unwrap();

    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_stdout.contains("indefinite") || list_stdout.contains("never"),
        "List should indicate indefinite lease"
    );

    // Step 3: Heartbeat
    cli.command_with_env(
        ["claim", "heartbeat", &issue_id],
        [("JIT_AGENT_ID", "agent:workflow-test")],
    )
    .assert()
    .success();

    // Step 4: Status check
    let status_output = cli
        .command_with_env(
            ["claim", "status", "--json"],
            [("JIT_AGENT_ID", "agent:workflow-test")],
        )
        .output()
        .unwrap();

    let status_json: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    let leases = status_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);

    // Step 5: Release by issue id
    cli.command_with_env(
        ["claim", "release", &issue_id],
        [("JIT_AGENT_ID", "agent:workflow-test")],
    )
    .assert()
    .success();

    // Verify lease is gone
    let final_list = cli.command(["claim", "list", "--json"]).output().unwrap();

    let final_json: Value = serde_json::from_slice(&final_list.stdout).unwrap();
    let final_leases = final_json["leases"].as_array().unwrap();
    assert_eq!(final_leases.len(), 0, "Lease should be released");
}

#[test]
fn test_claim_acquire_outside_git_repo_emits_git_requirement_error() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue Outside Git"])
        .unwrap();

    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("git repository"))
    .stderr(predicate::str::contains("claim").or(predicate::str::contains("lease")))
    // REQ-04 (jit:30a3b5c1): no git repository at all hints `git init`.
    .stderr(predicate::str::contains("git init"));
}

#[test]
fn test_claim_list_outside_git_repo_emits_git_requirement_error() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["claim", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"))
        .stderr(predicate::str::contains("claim").or(predicate::str::contains("lease")));
}

/// Running `claim acquire --json` outside a git repository emits a JSON error
/// naming the git requirement.
#[test]
fn test_claim_acquire_json_outside_git_repo_reports_git_requirement() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue Outside Git JSON"])
        .unwrap();

    let output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("git repository"),
        "message must name the git requirement, got: {message}"
    );
}

/// `claim list --json` outside a git repository emits a JSON error naming the
/// git requirement.
#[test]
fn test_claim_list_json_outside_git_repo_reports_git_requirement() {
    let cli = JitCli::initialised().unwrap();

    let output = cli.command(["claim", "list", "--json"]).output().unwrap();

    assert!(!output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("git repository"),
        "message must name the git requirement, got: {message}"
    );
}

/// REQ-01 (jit:30a3b5c1): the already-claimed error is an
/// [`jit::errors::ActionableError`] rendered through the top-level CLI
/// printer; the printer must add the "Error: " prefix exactly once, never a
/// doubled "Error: Error: ".
#[test]
fn test_claim_acquire_already_claimed_error_has_single_error_prefix() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .success();

    let output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-2",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("Error:").count(),
        1,
        "expected exactly one 'Error:' prefix, got: {stderr}"
    );
}

/// REQ-04 (jit:30a3b5c1): a git repository with zero commits fails
/// `--abbrev-ref HEAD` exactly like a directory with no git repository at
/// all, but the hint must point at making a commit rather than `git init`
/// (the repository already exists).
#[test]
fn test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue No Commits"])
        .unwrap();

    cli.command([
        "claim",
        "acquire",
        &issue_id,
        "--ttl",
        "600",
        "--agent-id",
        "agent:test-1",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("git repository"))
    .stderr(predicate::str::contains("commit"))
    .stderr(predicate::str::contains("git init").not());
}

/// JSON-mode counterpart of
/// [`test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init`]:
/// same error code as the no-repository case, but the message hints a commit
/// instead of `git init`.
#[test]
fn test_claim_acquire_json_in_git_repo_without_commits_hints_commit_not_git_init() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue No Commits JSON"])
        .unwrap();

    let output = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("commit"),
        "message must hint at making a commit, got: {message}"
    );
    assert!(
        !message.contains("git init"),
        "must not suggest `git init` for a repository that already exists, got: {message}"
    );
}

// ============================================================================
// REQ-03 (jit:30a3b5c1): assignment-command help and lease-command help each
// cross-reference the other, since `issue claim`/`release` and `claim
// acquire`/`release` share verbs but are different mechanisms (assignee
// bookkeeping vs. an exclusive, time-boxed lease).
// ============================================================================

#[test]
fn test_issue_assign_help_cross_references_lease_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["issue", "assign", "--help"])
        .unwrap();
    assert!(
        help.contains("jit claim acquire"),
        "issue assign --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_issue_claim_help_cross_references_lease_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["issue", "claim", "--help"])
        .unwrap();
    assert!(
        help.contains("jit claim acquire"),
        "issue claim --help should cross-reference jit claim acquire, got: {help}"
    );
}

// ============================================================================
// REQ-01 (jit:1a63ef75): `issue claim --help` documents the idempotent
// same-assignee re-claim and the in_progress promotion, replacing the stale
// "Claim an unassigned issue" wording (same-assignee re-claim was already
// idempotent by the time of this sweep — jit:30a3b5c1 landed the behavior).
// ============================================================================

#[test]
fn test_issue_claim_help_documents_promotion_and_reclaim_idempotency() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["issue", "claim", "--help"])
        .unwrap();
    assert!(
        help.contains("in_progress"),
        "issue claim --help should document promotion to in_progress, got: {help}"
    );
    assert!(
        help.to_lowercase().contains("re-claiming"),
        "issue claim --help should document idempotent same-assignee re-claim, got: {help}"
    );
    assert!(
        !help.contains("Claim an unassigned issue"),
        "issue claim --help should not claim the target must be unassigned, got: {help}"
    );
}

#[test]
fn test_issue_release_help_cross_references_lease_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["issue", "release", "--help"])
        .unwrap();
    assert!(
        help.contains("jit claim acquire"),
        "issue release --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_issue_unassign_help_cross_references_lease_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["issue", "unassign", "--help"])
        .unwrap();
    assert!(
        help.contains("jit claim acquire"),
        "issue unassign --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_claim_acquire_help_cross_references_assignment_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["claim", "acquire", "--help"])
        .unwrap();
    assert!(
        help.contains("jit issue claim"),
        "claim acquire --help should cross-reference jit issue claim, got: {help}"
    );
}

#[test]
fn test_claim_release_help_cross_references_assignment_commands() {
    let help = JitCli::uninitialised()
        .unwrap()
        .run(["claim", "release", "--help"])
        .unwrap();
    assert!(
        help.contains("jit issue claim"),
        "claim release --help should cross-reference jit issue claim, got: {help}"
    );
}

#[test]
fn test_config_show_reports_the_lease_enforcement_mode_issue_update_applies() {
    // Each state once diverged: `config show` honoured the env var while write
    // commands read only the repository config.
    let states = [("", "strict"), ("[worktree]\n", "off")];
    for (worktree_section, env_mode) in states {
        let cli = JitCli::initialised().unwrap();
        cli.init_git().unwrap().commit("Initial commit").unwrap();
        let issue_id = cli
            .create_issue(["--title", "Lease enforcement probe"])
            .unwrap();
        let config_path = cli.root().join(".jit/config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        config.push_str(&format!("\n{worktree_section}"));
        fs::write(&config_path, config).unwrap();
        let home = cli.root().join("isolated-home");
        let home = home.to_str().unwrap();
        let jit_as = |agent: &str, args: &[&str]| {
            cli.command_with_env(
                args,
                [
                    ("HOME", home),
                    ("JIT_ENFORCE_LEASES", env_mode),
                    ("JIT_AGENT_ID", agent),
                ],
            )
            .output()
            .unwrap()
        };
        let jit = |args: &[&str]| jit_as("agent:enforcement-probe", args);
        let holder = jit_as(
            "agent:holder",
            &["claim", "acquire", &issue_id, "--ttl", "600"],
        );
        assert!(holder.status.success(), "claim acquire failed: {holder:?}");

        let show = jit(&["config", "show", "--json"]);
        assert!(show.status.success(), "config show failed: {show:?}");
        let reported: Value = serde_json::from_slice(&show.stdout).unwrap();
        let reported_strict = reported["worktree"]["enforce_leases"] == "strict";
        let update = jit(&["issue", "update", &issue_id, "--priority", "high"]);

        assert_eq!(
            !update.status.success(),
            reported_strict,
            "section {worktree_section:?}, JIT_ENFORCE_LEASES={env_mode}: config show \
             reported {} but an update of another agent's leased issue {}: {}",
            reported["worktree"]["enforce_leases"],
            if update.status.success() {
                "succeeded"
            } else {
                "was refused"
            },
            String::from_utf8_lossy(&update.stderr)
        );
    }
}

#[test]
fn test_config_show_reports_the_lease_limits_claim_acquire_enforces() {
    // Each state once diverged: `config show` and `claim acquire` resolved the
    // limits independently.
    let section = |agent: Option<u32>, repo: Option<u32>| {
        let line = |key: &str, value: Option<u32>| {
            value.map_or(String::new(), |v| format!("{key} = {v}\n"))
        };
        format!(
            "[coordination]\n{}{}",
            line("max_indefinite_leases_per_agent", agent),
            line("max_indefinite_leases_per_repo", repo)
        )
    };
    // (repo section, whether the per-agent or per-repo limit is probed)
    let states = [
        (String::new(), "per_agent"),
        (section(Some(1), None), "per_agent"),
        (section(None, Some(2)), "per_repo"),
    ];
    for (repo_section, probe) in states {
        let cli = JitCli::initialised().unwrap();
        cli.init_git().unwrap().commit("Initial commit").unwrap();
        let config_path = cli.root().join(".jit/config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        config.push_str(&format!("\n{repo_section}"));
        fs::write(&config_path, config).unwrap();
        let home = cli.root().join("isolated-home");
        let home = home.to_str().unwrap();
        let jit = |args: &[&str]| {
            cli.command_with_env(args, [("HOME", home)])
                .output()
                .unwrap()
        };

        let show = jit(&["config", "show", "--json"]);
        assert!(show.status.success(), "config show failed: {show:?}");
        let reported: Value = serde_json::from_slice(&show.stdout).unwrap();
        let reported = reported["coordination"][format!("max_indefinite_leases_{probe}")]
            .as_u64()
            .expect("config show reports the limit");

        let mut granted = 0;
        for attempt in 0..=reported {
            let issue_id = cli
                .create_issue(["--title", &format!("Limit probe {attempt}")])
                .unwrap();
            let agent = match probe {
                "per_agent" => "agent:limit-probe".to_string(),
                _ => format!("agent:limit-probe-{attempt}"),
            };
            let acquire = jit(&[
                "claim",
                "acquire",
                &issue_id,
                "--ttl",
                "0",
                "--reason",
                "limit probe",
                "--agent-id",
                &agent,
            ]);
            if acquire.status.success() {
                granted += 1;
            }
        }
        assert_eq!(
            granted, reported,
            "repo {repo_section:?}: config show reported {probe} \
             limit {reported} but claim acquire granted {granted}"
        );
    }
}

#[test]
fn test_claim_acquire_fails_on_a_malformed_repository_config() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli
        .create_issue(["--title", "Malformed config probe"])
        .unwrap();
    fs::write(cli.root().join(".jit/config.toml"), "not = valid = toml").unwrap();

    let acquire = cli
        .command([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:malformed",
        ])
        .output()
        .unwrap();

    assert!(!acquire.status.success(), "{acquire:?}");
}

/// `claim status --issue` resolves a full id in any case or hyphenation to the
/// issue's lease.
#[test]
fn test_claim_status_issue_filter_resolves_any_spelling_of_a_full_id() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = parse_issue_id(&cli.create_issue(["--title", "Leased"]).unwrap());
    let lease_id = acquire_lease(&cli, &issue_id, "agent:status");

    for spelling in [
        issue_id.as_str().to_uppercase(),
        issue_id.as_str().replace('-', ""),
    ] {
        let output = cli
            .command_with_env(
                ["claim", "status", "--issue", &spelling, "--json"],
                [("JIT_AGENT_ID", "agent:status")],
            )
            .output()
            .unwrap();
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        let leases = json["leases"].as_array().unwrap();
        assert_eq!(leases.len(), 1, "{spelling}: {json}");
        assert_eq!(leases[0]["lease_id"], lease_id);
    }
}

/// Returns `(dependency, dependent)`: `dependent` is in backlog behind the
/// unmet `dependency`.
fn create_dependency_blocked_pair(cli: &JitCli) -> (String, String) {
    let dependency = cli.create_issue(["--title", "Prerequisite"]).unwrap();
    let dependent = cli.create_issue(["--title", "Dependent work"]).unwrap();
    cli.run(["dep", "add", &dependent, &dependency]).unwrap();
    (dependency, dependent)
}

fn remediation_commands(envelope: &Value) -> Vec<&str> {
    envelope["error"]["details"]["remediation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command.as_str().unwrap())
        .collect()
}

#[test]
fn test_issue_claim_blocked_by_dependencies_json_names_blocker_and_remediation() {
    let cli = JitCli::initialised().unwrap();
    let (dependency, dependent) = create_dependency_blocked_pair(&cli);

    let output = cli
        .command(["issue", "claim", &dependent, "agent:test", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    assert_eq!(json["error"]["details"]["issue_id"], dependent);
    assert_eq!(json["error"]["details"]["requested_state"], "in_progress");
    assert_eq!(json["error"]["details"]["actual_state"], "backlog");
    assert_eq!(
        json["error"]["details"]["blockers"][0]["issue_id"],
        dependency
    );
    assert!(remediation_commands(&json)
        .iter()
        .any(|command| command.contains(&format!("jit graph deps {dependent}"))));
}

#[test]
fn test_issue_claim_blocked_by_precheck_gate_json_names_gate_and_remediation() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "tdd-reminder",
        "--title",
        "TDD Reminder",
        "-d",
        "Write tests first",
        "--stage",
        "precheck",
        "--mode",
        "manual",
    ])
    .unwrap();
    let issue = cli
        .create_issue(["--title", "Precheck work", "--gate", "tdd-reminder"])
        .unwrap();

    let output = cli
        .command(["issue", "claim", &issue, "agent:test", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(json["error"]["details"]["requested_state"], "in_progress");
    assert_eq!(json["error"]["details"]["actual_state"], "ready");
    assert_eq!(json["error"]["details"]["blockers"][0]["type"], "gate");
    assert_eq!(
        json["error"]["details"]["blockers"][0]["key"],
        "tdd-reminder"
    );
    assert!(remediation_commands(&json)
        .iter()
        .any(|command| command.contains(&format!("jit gate evaluate {issue} tdd-reminder"))));
}

#[test]
fn test_issue_claim_next_blocked_by_precheck_gate_json_names_issue_and_gate() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "tdd-reminder",
        "--title",
        "TDD Reminder",
        "-d",
        "Write tests first",
        "--stage",
        "precheck",
        "--mode",
        "manual",
    ])
    .unwrap();
    let issue = cli
        .create_issue([
            "--title",
            "Claim-next precheck work",
            "--gate",
            "tdd-reminder",
        ])
        .unwrap();

    let output = cli
        .command(["issue", "claim-next", "agent:test", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(json["error"]["details"]["issue_id"], issue);
    assert_eq!(
        json["error"]["details"]["blockers"][0]["key"],
        "tdd-reminder"
    );
}

#[test]
fn test_issue_claim_next_skips_non_ready_issues() {
    let cli = JitCli::initialised().unwrap();
    let done_issue = cli.create_issue(["--title", "Already done"]).unwrap();
    let ready_issue = cli.create_issue(["--title", "Ready work"]).unwrap();
    cli.run(["issue", "update", &done_issue, "--state", "done"])
        .unwrap();

    let claimed = cli.run_json(["issue", "claim-next", "agent:test"]).unwrap();

    assert_eq!(claimed["id"], ready_issue);
    assert_eq!(claimed["state"], "in_progress");
    assert_eq!(claimed["assignee"], "agent:test");
    assert!(cli.run_json(["issue", "show", &done_issue]).unwrap()["assignee"].is_null());
}

#[test]
fn test_issue_claim_blocked_by_dependencies_human_error_names_assign() {
    let cli = JitCli::initialised().unwrap();
    let (_dependency, dependent) = create_dependency_blocked_pair(&cli);

    let output = cli
        .command(["issue", "claim", &dependent, "agent:test"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jit issue assign"), "stderr: {stderr}");
    assert!(stderr.contains(&dependent), "stderr: {stderr}");
}

#[test]
fn test_issue_claim_blocked_by_dependencies_json_error_names_assign() {
    let cli = JitCli::initialised().unwrap();
    let (_dependency, dependent) = create_dependency_blocked_pair(&cli);

    let output = cli
        .command(["issue", "claim", &dependent, "agent:test", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    let assign = format!("jit issue assign {dependent}");
    assert!(json["error"]["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion.as_str().unwrap().contains(&assign)));
    assert!(remediation_commands(&json)
        .iter()
        .any(|command| command.contains(&assign)));
}

#[test]
fn test_issue_claim_assign_only_sets_assignee_without_transition() {
    let cli = JitCli::initialised().unwrap();
    let issue = cli.create_issue(["--title", "Assign-only work"]).unwrap();

    cli.run(["issue", "claim", &issue, "agent:test", "--assign-only"])
        .unwrap();

    let shown = cli.run_json(["issue", "show", &issue]).unwrap();
    assert_eq!(shown["assignee"], "agent:test");
    assert_eq!(shown["state"], "ready");
}

#[test]
fn test_issue_claim_ready_issue_transitions_to_in_progress() {
    let cli = JitCli::initialised().unwrap();
    let issue = cli.create_issue(["--title", "Ready work"]).unwrap();
    cli.run(["issue", "update", &issue, "--state", "ready"])
        .unwrap();

    cli.run(["issue", "claim", &issue, "agent:test"]).unwrap();

    let shown = cli.run_json(["issue", "show", &issue]).unwrap();
    assert_eq!(shown["assignee"], "agent:test");
    assert_eq!(shown["state"], "in_progress");
}

#[test]
fn test_claim_list_text_marks_a_stale_unexpired_lease_with_its_heartbeat_remedy() {
    use chrono::{Duration, Utc};

    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = parse_issue_id(&cli.create_issue(["--title", "Stale lease"]).unwrap());
    let silent_since = Utc::now()
        - Duration::seconds(jit::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64 + 60);
    let lease = jit::storage::Lease {
        lease_id: "00000000-0000-4000-8000-0000000000aa".parse().unwrap(),
        issue_id,
        agent_id: "agent:silent".parse().unwrap(),
        worktree_id: "wt:silent".parse().unwrap(),
        branch: None,
        ttl_secs: 0,
        acquired_at: silent_since,
        expires_at: None,
        last_beat: silent_since,
    };
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    jit::test_lease_store::plant_lease(&paths.shared_jit, &lease).unwrap();

    cli.command(["claim", "list"]).assert().success().stdout(
        predicate::str::contains("STALE").and(predicate::str::contains(format!(
            "jit claim heartbeat {}",
            lease.issue_id.short_id()
        ))),
    );
}
