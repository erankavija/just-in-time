use jit::test_cli::JitCli;
use jit::test_harness::git::{GitRepository, GitStandIn};
use serde_json::Value;

fn assert_read_with_git_budget(
    cli: &JitCli,
    args: &[&str],
    reads_issue: impl FnOnce(&Value) -> bool,
) {
    let stand_in = GitStandIn::new();
    let output = cli
        .command_with_env(args, [("PATH", stand_in.search_path())])
        .output()
        .unwrap();
    let invocations = stand_in.invocations();

    assert!(
        invocations.len() <= 1,
        "jit {args:?} exceeded one git process: {invocations:#?}"
    );
    assert!(
        output.status.success(),
        "jit {args:?} failed: {output:?}; git invocations: {invocations:#?}"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("jit {args:?} returned invalid JSON: {error}; git invocations: {invocations:#?}")
    });
    assert!(
        reads_issue(&json),
        "jit {args:?} did not read the issue: {json}; git invocations: {invocations:#?}"
    );
}

fn assert_issue_read_budget(cli: &JitCli, id: &str) {
    assert_read_with_git_budget(cli, &["query", "available", "--json"], |json| {
        json["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| issue["id"] == id))
    });
    assert_read_with_git_budget(cli, &["issue", "show", id, "--json"], |json| {
        json["id"] == id
    });
}

fn create_ready_issue(cli: &JitCli) -> String {
    let id = cli.create_issue(["--title", "Available issue"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();
    id
}

#[test]
fn test_issue_reads_start_at_most_one_git_process_in_primary_checkout() {
    let cli = JitCli::uninitialised().unwrap();
    GitRepository::init(cli.root());
    cli.run(["init"]).unwrap();
    let id = create_ready_issue(&cli);

    assert_issue_read_budget(&cli, &id);
}

#[test]
fn test_issue_reads_start_at_most_one_git_process_for_primary_only_issue_in_linked_checkout() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let id = create_ready_issue(&primary);
    let issue_path = format!(".jit/issues/{id}.json");
    assert!(primary.root().join(&issue_path).is_file());
    assert!(!linked.root().join(&issue_path).exists());

    assert_issue_read_budget(&linked, &id);
}

fn json_over(cli: &JitCli, store: &std::path::Path, args: &[&str]) -> Value {
    let output = cli
        .command_with_env(args, [("JIT_DATA_DIR", store)])
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_layout_budget(
    cli: &JitCli,
    store: &std::path::Path,
    args: &[&str],
    bound: usize,
) -> (Value, usize, usize) {
    let stand_in = GitStandIn::new();
    let output = cli
        .command_with_env(
            args,
            [
                ("JIT_DATA_DIR", store.as_os_str()),
                ("PATH", stand_in.search_path().as_os_str()),
            ],
        )
        .output()
        .unwrap();
    let invocations = stand_in.invocations();
    let layouts = invocations
        .iter()
        .filter(|call| call.args().iter().any(|arg| arg == "--git-common-dir"))
        .count();
    let listings = invocations
        .iter()
        .filter(|call| matches!(call.args(), [command, operation, ..] if command == "worktree" && operation == "list"))
        .count();
    assert!(
        layouts <= bound,
        "{args:?}: {layouts} layout probes exceed {bound}: {invocations:#?}"
    );
    assert!(
        listings <= 1,
        "{args:?}: repeated worktree listing: {invocations:#?}"
    );
    assert!(
        output.status.success(),
        "{args:?}: {output:?}; {invocations:#?}"
    );
    let json = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{args:?}: {error}; {output:?}; {invocations:#?}"));
    (json, layouts, listings)
}

#[test]
fn test_external_bootstrap_reuses_two_layout_probes_from_primary_and_linked_checkouts() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let issue = json_over(
        &primary,
        &store,
        &[
            "issue",
            "create",
            "--title",
            "Shared external issue",
            "--json",
        ],
    );
    let id = issue["id"].as_str().unwrap();

    for viewer in [&primary, &linked] {
        let (read, _, _) =
            assert_layout_budget(viewer, &store, &["issue", "show", id, "--json"], 2);
        assert_eq!(read["id"], id);
    }
}

#[test]
fn test_data_root_in_another_checkout_remains_authoritative_with_one_layout_probe() {
    let selected = JitCli::uninitialised().unwrap();
    GitRepository::init(selected.root());
    selected.run(["init"]).unwrap();
    let id = create_ready_issue(&selected);
    let invoking = JitCli::uninitialised().unwrap();
    GitRepository::init(invoking.root());

    let (read, _, _) = assert_layout_budget(
        &invoking,
        &selected.root().join(".jit"),
        &["issue", "show", &id, "--json"],
        1,
    );
    assert_eq!(read["id"], id);
}

#[test]
fn test_external_traversal_from_subdirectory_reuses_moved_checkout_layout_regardless_of_leases() {
    let (primary, linked, store) = JitCli::linked_checkouts_over_external_store().unwrap();
    let before_move = json_over(&linked, &store, &["worktree", "info", "--json"]);
    let moved_root = linked.root().with_file_name("renamed");
    primary
        .git([
            "worktree",
            "move",
            linked.root().to_str().unwrap(),
            moved_root.to_str().unwrap(),
        ])
        .unwrap();
    let moved = primary.at(&moved_root);
    let subdir = primary.root().join("subdir");
    std::fs::create_dir(&subdir).unwrap();
    let viewer = primary.at(&subdir);
    let mut previous_counts = None;

    for count in 1..=3 {
        let issue = json_over(
            &moved,
            &store,
            &[
                "issue",
                "create",
                "--title",
                &format!("Lease {count}"),
                "--json",
            ],
        );
        json_over(
            &moved,
            &store,
            &[
                "claim",
                "acquire",
                issue["id"].as_str().unwrap(),
                "--ttl",
                "600",
                "--agent-id",
                "agent:holder",
                "--json",
            ],
        );
        let (listing, layouts, listings) =
            assert_layout_budget(&viewer, &store, &["worktree", "list", "--json"], 3);
        let entries = listing["worktrees"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        let entry = entries
            .iter()
            .find(|entry| entry["path"].as_str().unwrap() == moved_root.to_str().unwrap())
            .unwrap();
        assert_eq!(entry["worktree_id"], before_move["worktree_id"]);
        assert_eq!(entry["active_claims"], count);
        assert_eq!(listings, 1);

        let (_, validation_layouts, validation_listings) =
            assert_layout_budget(&viewer, &store, &["validate", "--leases", "--json"], 3);
        assert_eq!(validation_listings, 1);
        let counts = (layouts, listings, validation_layouts, validation_listings);
        if let Some(previous) = previous_counts {
            assert_eq!(
                counts, previous,
                "additional leases must not add Git discovery"
            );
        }
        previous_counts = Some(counts);
    }
}
