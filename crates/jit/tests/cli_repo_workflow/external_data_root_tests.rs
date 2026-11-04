//! A store outside every checkout, selected through `JIT_DATA_DIR`, coordinates
//! claims across the linked checkouts of the invoking repository, each of which
//! keeps its own identity.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use jit::test_lease_store::history_path;
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
    assert!(history_path(&common_dir.join("jit")).is_file());
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
/// writing checkout's worktree id differs from the id on the lease.
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
        Some(ErrorCode::RequiresGit.as_str())
    );
}

/// The checkout's reported id and the relocations reported on the way to it.
fn identity_over(cli: &JitCli, store: &Path) -> (String, usize) {
    let info = json_over(cli, store, &["worktree", "info"]);
    (
        info["worktree_id"].as_str().unwrap().to_owned(),
        relocations(&info),
    )
}

fn relocations(document: &Value) -> usize {
    document["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| {
            warning
                .as_str()
                .is_some_and(|text| text.contains("Worktree relocated"))
        })
        .count()
}

/// `(id, active claims)` the listing reports for the checkout at `root`.
fn listed_checkout(listing: &Value, root: &Path) -> (String, u64) {
    let root = root.canonicalize().unwrap();
    let entry = listing["worktrees"]
        .as_array()
        .expect("the listing carries worktrees")
        .iter()
        .find(|entry| Path::new(entry["path"].as_str().unwrap()) == root)
        .unwrap_or_else(|| panic!("{} is not listed in {listing}", root.display()));
    (
        entry["worktree_id"].as_str().unwrap().to_owned(),
        entry["active_claims"].as_u64().unwrap(),
    )
}

#[test]
fn test_worktree_info_alternating_checkouts_over_external_store_keeps_ids_without_relocation() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();

    let first = identity_over(&primary, &store);
    let second = identity_over(&linked, &store);
    assert_ne!(first.0, second.0);

    for _ in 0..3 {
        assert_eq!(identity_over(&primary, &store), (first.0.clone(), 0));
        assert_eq!(identity_over(&linked, &store), (second.0.clone(), 0));
    }
}

#[test]
fn test_worktree_info_from_checkout_subdirectory_over_external_store_is_the_same_checkout() {
    let (primary, _linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let (id, _) = identity_over(&primary, &store);
    let subdirectory = primary.at("main/nested");
    std::fs::create_dir(subdirectory.root()).unwrap();

    assert_eq!(identity_over(&subdirectory, &store), (id.clone(), 0));
    assert_eq!(identity_over(&primary, &store), (id, 0));
}

#[test]
fn test_worktree_move_over_external_store_reports_one_relocation_and_keeps_the_id() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let (id, _) = identity_over(&linked, &store);
    let moved = primary.at("moved");
    primary
        .git([
            "worktree",
            "move",
            linked.root().to_str().unwrap(),
            moved.root().to_str().unwrap(),
        ])
        .unwrap();

    assert_eq!(identity_over(&moved, &store), (id.clone(), 1));
    assert_eq!(identity_over(&moved, &store), (id, 0));
}

/// A listing creates a sibling's identity while that sibling creates its own,
/// so every process must still see one id per checkout.
#[test]
fn test_worktree_commands_concurrent_across_checkouts_over_external_store_leave_each_identity() {
    const PROCESSES_PER_CHECKOUT: usize = 6;
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let (primary, linked, store) = (&primary, &linked, store.as_path());

    let observed = std::thread::scope(|scope| {
        let processes = (0..PROCESSES_PER_CHECKOUT)
            .flat_map(|_| [primary, linked])
            .map(|viewer| {
                scope.spawn(move || {
                    let listing = json_over(viewer, store, &["worktree", "list"]);
                    let (own_id, _) = identity_over(viewer, store);
                    let (listed_id, _) = listed_checkout(&listing, viewer.root());
                    assert_eq!(relocations(&listing), 0, "{listing}");
                    assert_eq!(listed_id, own_id, "{listing}");
                    (
                        listed_checkout(&listing, primary.root()).0,
                        listed_checkout(&listing, linked.root()).0,
                    )
                })
            })
            .collect::<Vec<_>>();
        processes
            .into_iter()
            .map(|process| process.join().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
    });

    let settled = (
        identity_over(primary, store).0,
        identity_over(linked, store).0,
    );
    assert_eq!(observed, std::collections::BTreeSet::from([settled]));
}

#[test]
fn test_worktree_list_from_either_checkout_over_external_store_reports_each_id_and_claims() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let primary_issue = create_issue(&primary, &store, "Leased from the primary");
    let first_linked_issue = create_issue(&linked, &store, "First from the linked checkout");
    let second_linked_issue = create_issue(&linked, &store, "Second from the linked checkout");
    for (cli, issue) in [
        (&primary, &primary_issue),
        (&linked, &first_linked_issue),
        (&linked, &second_linked_issue),
    ] {
        let acquired = acquire(cli, &store, issue, "agent:holder");
        assert!(acquired.status.success(), "{acquired:?}");
    }
    let (primary_id, _) = identity_over(&primary, &store);
    let (linked_id, _) = identity_over(&linked, &store);

    for viewer in [&primary, &linked] {
        let listing = json_over(viewer, &store, &["worktree", "list"]);
        assert_eq!(relocations(&listing), 0, "{listing}");
        assert_eq!(
            listed_checkout(&listing, primary.root()),
            (primary_id.clone(), 1)
        );
        assert_eq!(
            listed_checkout(&listing, linked.root()),
            (linked_id.clone(), 2)
        );
    }
}

#[test]
fn test_worktree_list_over_external_store_records_the_id_the_unvisited_sibling_then_reports() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();

    let listing = json_over(&primary, &store, &["worktree", "list"]);

    let (listed_id, _) = listed_checkout(&listing, linked.root());
    assert_eq!(identity_over(&linked, &store), (listed_id, 0));
}

#[test]
fn test_validate_leases_over_external_store_recognises_a_lease_held_by_a_sibling_checkout() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let issue = create_issue(&linked, &store, "Leased from the linked checkout");
    let acquired = acquire(&linked, &store, &issue, "agent:holder");
    assert!(acquired.status.success(), "{acquired:?}");

    let validated = primary
        .command_with_env(["validate", "--leases"], [("JIT_DATA_DIR", &store)])
        .output()
        .unwrap();

    assert!(validated.status.success(), "{validated:?}");
}

/// The identity a shared store once held names whichever checkout wrote last.
#[test]
fn test_worktree_info_over_external_store_neither_reads_nor_rewrites_a_stale_store_identity() {
    let (primary, _linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let stale = serde_json::json!({
        "schema_version": 1,
        "worktree_id": "wt:deadbeef",
        "branch": "stale",
        "root": "/nonexistent/stale/checkout",
        "created_at": "2026-01-06T20:00:00Z",
    })
    .to_string();
    let stale_file = store.join("worktree.json");
    std::fs::write(&stale_file, &stale).unwrap();

    let (id, relocated) = identity_over(&primary, &store);

    assert_ne!(id, "wt:deadbeef");
    assert_eq!(relocated, 0);
    assert_eq!(std::fs::read_to_string(&stale_file).unwrap(), stale);
}
