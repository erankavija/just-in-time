//! A store outside every checkout, selected through `JIT_DATA_DIR`, coordinates
//! claims across the linked checkouts of the invoking repository.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;

fn run_over(cli: &JitCli, store: &Path, args: &[&str]) -> Output {
    cli.command_with_env(
        args.iter().copied().chain(["--json"]),
        [("JIT_DATA_DIR", store)],
    )
    .output()
    .expect("spawn jit")
}

fn json_over(cli: &JitCli, store: &Path, args: &[&str]) -> Value {
    let output = run_over(cli, store, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    serde_json::from_slice(&output.stdout).expect("--json prints one document")
}

fn create_issue(cli: &JitCli, store: &Path, title: &str) -> String {
    json_over(cli, store, &["issue", "create", "--title", title])["id"]
        .as_str()
        .expect("creation names the issue")
        .to_owned()
}

fn acquire(cli: &JitCli, store: &Path, issue: &str, agent: &str) -> Output {
    run_over(
        cli,
        store,
        &[
            "claim",
            "acquire",
            issue,
            "--ttl",
            "600",
            "--agent-id",
            agent,
        ],
    )
}

fn listed_issues(cli: &JitCli, store: &Path) -> Vec<String> {
    let listed = json_over(cli, store, &["claim", "list"]);
    listed["leases"]
        .as_array()
        .expect("the listing carries leases")
        .iter()
        .map(|lease| lease["issue_id"].as_str().unwrap().to_owned())
        .collect()
}

fn assert_recovers_claims(cli: &JitCli, store: &Path) {
    let report = json_over(cli, store, &["recover"]);
    assert_eq!(report["claims_lease_recovery"]["status"], "completed");
    assert!(
        !report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["kind"] == "claims_lease_recovery_skipped"),
        "{report}"
    );
}

#[test]
fn test_linked_checkouts_over_external_store_share_claims_under_the_repository_common_dir() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let common_dir = primary.root().join(".git").canonicalize().unwrap();

    let primary_info = json_over(&primary, &store, &["worktree", "info"]);
    let linked_info = json_over(&linked, &store, &["worktree", "info"]);
    assert_eq!(primary_info["is_main_worktree"], true);
    assert_eq!(linked_info["is_main_worktree"], false);
    assert_eq!(
        Path::new(linked_info["common_dir"].as_str().unwrap()),
        common_dir
    );
    assert_eq!(
        Path::new(primary_info["common_dir"].as_str().unwrap()),
        common_dir
    );

    let from_primary = create_issue(&primary, &store, "Leased from the primary");
    let from_linked = create_issue(&linked, &store, "Leased from the linked checkout");

    let acquired = acquire(&primary, &store, &from_primary, "agent:primary");
    assert!(acquired.status.success(), "{acquired:?}");
    assert!(common_dir.join("jit/claims.jsonl").is_file());
    assert_eq!(
        listed_issues(&linked, &store),
        std::slice::from_ref(&from_primary)
    );
    let refused = acquire(&linked, &store, &from_primary, "agent:linked");
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stdout).contains("already claimed"),
        "{refused:?}"
    );

    let acquired = acquire(&linked, &store, &from_linked, "agent:linked");
    assert!(acquired.status.success(), "{acquired:?}");
    assert_eq!(listed_issues(&primary, &store).len(), 2);
    let refused = acquire(&primary, &store, &from_linked, "agent:primary");
    assert!(!refused.status.success(), "{refused:?}");

    assert_recovers_claims(&primary, &store);
    assert_recovers_claims(&linked, &store);
}

#[test]
fn test_issue_create_from_linked_checkout_over_external_store_passes_the_refusing_write_policy() {
    let (_primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let config = store.join("config.toml");
    let declared = std::fs::read_to_string(&config).unwrap()
        + "\n[worktree]\nenforce_leases = \"off\"\nwrite_policy = \"refuse\"\n";
    std::fs::write(&config, declared).unwrap();

    create_issue(&linked, &store, "Written from a linked checkout");
}

/// The owner's own write passes and another agent's is refused, though the
/// guard's fallback worktree id differs from the id on the lease.
#[test]
fn test_issue_update_over_external_store_enforces_the_lease_of_another_checkout() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let config = store.join("config.toml");
    let declared =
        std::fs::read_to_string(&config).unwrap() + "\n[worktree]\nenforce_leases = \"strict\"\n";
    std::fs::write(&config, declared).unwrap();
    let issue = create_issue(&primary, &store, "Leased");
    let acquired = acquire(&primary, &store, &issue, "agent:owner");
    assert!(acquired.status.success(), "{acquired:?}");

    let update = |agent: &str| {
        linked
            .command_with_env(
                ["issue", "update", &issue, "--priority", "high", "--json"],
                [
                    ("JIT_DATA_DIR", store.as_os_str()),
                    ("JIT_AGENT_ID", agent.as_ref()),
                ],
            )
            .output()
            .unwrap()
    };

    let owned = update("agent:owner");
    assert!(owned.status.success(), "{owned:?}");
    let foreign = update("agent:other");
    assert!(!foreign.status.success(), "{foreign:?}");
    assert!(
        String::from_utf8_lossy(&foreign.stdout).contains("agent:owner"),
        "{foreign:?}"
    );
}

#[test]
fn test_recover_and_claims_outside_every_checkout_over_external_store_report_the_git_requirement() {
    let outside = JitCli::uninitialised().unwrap();
    let holder = tempfile::TempDir::new().unwrap();
    let store: PathBuf = holder.path().join("store");
    assert!(run_over(&outside, &store, &["init"]).status.success());

    let report = json_over(&outside, &store, &["recover"]);
    assert_eq!(report["claims_lease_recovery"]["status"], "skipped");
    assert_eq!(report["claims_lease_recovery"]["reason"], "requires_git");

    let listed = run_over(&outside, &store, &["claim", "list"]);
    assert!(!listed.status.success(), "{listed:?}");
    let failure: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"].as_str(),
        Some(ErrorCode::ClaimRequiresGit.as_str())
    );
}
