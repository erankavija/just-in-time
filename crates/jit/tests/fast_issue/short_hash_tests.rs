//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes. Short
//! prefixes enter through the CLI, which resolves each issue-id argument once.
use jit::test_cli::JitCli;
use jit::test_harness::TestHarness;

/// The issue commands accept a unique short prefix and act on the issue it names.
#[test]
fn test_cli_issue_id_arguments_resolve_a_unique_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    let from = cli.create_issue(["From"]).unwrap();
    let to = cli.create_issue(["To"]).unwrap();

    let shown = cli.run_json(["issue", "show", &short(&from)]).unwrap();
    assert_eq!(shown["id"], from.as_str());

    cli.run(["issue", "update", &short(&from), "--title", "Renamed"])
        .unwrap();
    let updated = cli.run_json(["issue", "show", &from]).unwrap();
    assert_eq!(updated["title"], "Renamed");

    let deleted = cli
        .command_with_env(
            ["issue", "delete", &short(&to)],
            [("JIT_ALLOW_DELETION", "1")],
        )
        .output()
        .unwrap();
    assert!(deleted.status.success(), "{deleted:?}");
    assert!(cli.run(["issue", "show", &to]).is_err());
}

/// Every CLI issue-id argument refuses a prefix below the minimum as an invalid
/// prefix before reading any issue record: the only record is unreadable, so a
/// read before the refusal surfaces instead.
#[test]
fn test_cli_every_issue_id_argument_rejects_a_short_prefix_before_reading_records() {
    const SHORT: &str = "{short}";
    const FULL: &str = "{full}";
    let invocations: &[&[&str]] = &[
        &["issue", "show", SHORT],
        &["issue", "status", SHORT],
        &["issue", "children", SHORT],
        &["issue", "progress", SHORT],
        &["issue", "update", SHORT, "--title", "t"],
        &["issue", "delete", SHORT],
        &["issue", "assign", SHORT, "agent:a"],
        &["issue", "claim", SHORT, "agent:a"],
        &["issue", "unassign", SHORT],
        &["issue", "reject", SHORT],
        &["issue", "release", SHORT, "r"],
        &["dep", "add", SHORT, FULL],
        &["dep", "add", FULL, SHORT],
        &["dep", "rm", SHORT, FULL],
        &["dep", "rm", FULL, SHORT],
        &["apply", "t", SHORT],
        &["apply", "t", FULL, "--anchor", "a={short}"],
        &["gate", "add", SHORT, "g"],
        &["gate", "evaluate", SHORT, "g"],
        &["gate", "evaluate-all", SHORT],
        &["gate", "evaluate-many", "g", SHORT],
        &["gate", "fail", SHORT, "g"],
        &["gate", "status", SHORT, "g"],
        &["gate", "status", SHORT, "g", "--findings"],
        &["gate", "status", SHORT, "g", "--stdout"],
        &["gate", "status", SHORT, "--all"],
        &["gate", "status-all", SHORT],
        &["gate", "preset", "apply", "p", SHORT],
        &["gate", "preset", "create", SHORT, "p"],
        &["doc", "add", SHORT, "d.md"],
        &["doc", "list", SHORT],
        &["doc", "remove", SHORT, "d.md"],
        &["doc", "show", SHORT, "d.md"],
        &["doc", "dir", SHORT, "a"],
        &["doc", "history", SHORT, "d.md"],
        &["doc", "diff", SHORT, "d.md", "--from", "HEAD"],
        &["doc", "assets", "list", SHORT, "d.md"],
        &["doc", "check-links", "--scope", "issue:{short}"],
        &["graph", "deps", SHORT],
        &["graph", "rdeps", SHORT],
        &["graph", "tree", SHORT],
        &["graph", "export", "--scope", SHORT],
        &["graph", "export", "--format", "batch", "--scope", SHORT],
        &["validate", SHORT],
        &["validate", "--scope", SHORT],
        &["validate", SHORT, "--rule", "r"],
        &["validate", SHORT, "--explain"],
        &["item", "show", "{short}/x"],
        &["archive", "container", SHORT],
        &["events", "query", "--issue-id", SHORT],
        &["claim", "acquire", SHORT, "--agent-id", "agent:a"],
        &["claim", "release", SHORT],
        &["claim", "status", "--issue", SHORT],
        &["snapshot", "export", "--scope", "issue:{short}"],
    ];

    let cli = JitCli::initialised().unwrap();
    let full = cli.create_issue(["Unreadable"]).unwrap();
    std::fs::write(
        cli.root().join(format!(".jit/issues/{full}.json")),
        "not an issue record",
    )
    .unwrap();
    let short = &full[..jit::domain::identity::IssueIdPrefix::MIN_ID_PREFIX_LENGTH - 1];
    let refusal = jit::domain::identity::InvalidIdPrefixError::new(short).to_string();

    let unrefused: Vec<String> = invocations
        .iter()
        .map(|args| {
            args.iter()
                .map(|arg| arg.replace(SHORT, short).replace(FULL, &full))
                .collect::<Vec<_>>()
        })
        .filter(|args| {
            let output = cli
                .command_with_env(args, [("JIT_ALLOW_DELETION", "1")])
                .output()
                .unwrap();
            let reported = [&output.stdout, &output.stderr]
                .iter()
                .any(|stream| String::from_utf8_lossy(stream).contains(&refusal));
            output.status.success() || !reported
        })
        .map(|args| args.join(" "))
        .collect();
    assert!(
        unrefused.is_empty(),
        "read before refusing a short prefix: {unrefused:?}"
    );
}

/// The server document reads resolve a full id in any case or hyphenation to
/// the issue it names.
#[test]
fn test_read_document_content_resolves_any_spelling_of_a_full_id() {
    let h = TestHarness::new();
    let id = h.create_issue("Documented");
    h.executor
        .add_document_reference(&id, "d.md", None, None, None, true)
        .unwrap();
    // The in-memory store backs no document bytes, so a read that resolved the
    // issue fails on the document exactly as the canonical spelling does.
    let read = |id: &str| format!("{:?}", h.executor.read_document_content(id, "d.md", None));
    assert_ne!(read(&id), read("ffffffff"));
    for spelling in [id.to_uppercase(), id.replace('-', "")] {
        assert_eq!(read(&spelling), read(&id), "{spelling}");
    }
}

fn short(id: &str) -> String {
    id[..8].to_string()
}

/// `jit args --json` with `{short}` naming a too-short prefix refuses it as an
/// invalid prefix with the argument-error exit code.
fn assert_rejects_too_short_prefix(cli: &JitCli, args: &[&str]) {
    let too_short = "abc";
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.replace("{short}", too_short))
        .chain(std::iter::once("--json".to_string()))
        .collect();
    let output = cli.command(&args).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "INVALID_ID_PREFIX", "{args:?}");
}

fn events_of(cli: &JitCli, filter: &str) -> serde_json::Value {
    cli.run_json(["events", "query", "--issue-id", filter])
        .unwrap()["events"]
        .clone()
}

/// `dep add` resolves each target prefix at the CLI.
#[test]
fn test_cli_dep_add_targets_resolve_short_prefixes() {
    let cli = JitCli::initialised().unwrap();
    let from = cli.create_issue(["From"]).unwrap();
    let to = cli.create_issue(["To"]).unwrap();
    let other = cli.create_issue(["Other"]).unwrap();

    let added = cli
        .run_json(["dep", "add", &from, &short(&to), &short(&other)])
        .unwrap();
    assert_eq!(added["added"], serde_json::json!([to, other]));
    assert_rejects_too_short_prefix(&cli, &["dep", "add", &from, &to, "{short}"]);
}

/// `dep rm` parses each target prefix at the CLI and removes the edge it names.
#[test]
fn test_cli_dep_rm_targets_resolve_short_prefixes() {
    let cli = JitCli::initialised().unwrap();
    let from = cli.create_issue(["From"]).unwrap();
    let to = cli.create_issue(["To"]).unwrap();
    cli.run(["dep", "add", &from, &to]).unwrap();

    cli.run(["dep", "rm", &from, &short(&to)]).unwrap();
    let shown = cli.run_json(["issue", "show", &from]).unwrap();
    assert_eq!(shown["dependencies"], serde_json::json!([]));
    assert_rejects_too_short_prefix(&cli, &["dep", "rm", &from, "{short}"]);
}

/// The event-query issue filter resolves a short prefix or any spelling of a
/// live issue's id to that issue's events.
#[test]
fn test_cli_events_query_issue_filter_resolves_short_prefix_and_any_spelling() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["Filtered"]).unwrap();
    cli.create_issue(["Unrelated"]).unwrap();

    let events = events_of(&cli, &id);
    assert!(events
        .as_array()
        .is_some_and(|events| !events.is_empty() && events.iter().all(|e| e["issue_id"] == id)));
    for spelling in [short(&id), id.to_uppercase(), id.replace('-', "")] {
        assert_eq!(events_of(&cli, &spelling), events, "{spelling}");
    }
    assert_rejects_too_short_prefix(&cli, &["events", "query", "--issue-id", "{short}"]);
}

/// An event-query issue filter naming a deleted issue still selects its events,
/// in any spelling.
#[test]
fn test_cli_events_query_issue_filter_matches_a_deleted_issue_in_any_spelling() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["Deleted"]).unwrap();
    let deleted = cli
        .command_with_env(["issue", "delete", &id], [("JIT_ALLOW_DELETION", "1")])
        .output()
        .unwrap();
    assert!(deleted.status.success(), "{deleted:?}");

    let events = events_of(&cli, &id.to_uppercase());
    assert!(events
        .as_array()
        .is_some_and(|events| !events.is_empty() && events.iter().all(|e| e["issue_id"] == id)));
}

/// `doc check-links --scope issue:<prefix>` resolves the scope's issue at the CLI.
#[test]
fn test_cli_doc_check_links_issue_scope_resolves_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["Scoped"]).unwrap();

    let scope = format!("issue:{}", short(&id));
    cli.run(["doc", "check-links", "--scope", &scope]).unwrap();
    assert!(cli
        .run(["doc", "check-links", "--scope", "issue:ffffffff"])
        .is_err());
    assert_rejects_too_short_prefix(&cli, &["doc", "check-links", "--scope", "issue:{short}"]);
}

/// `snapshot export --scope issue:<prefix>` resolves the scope's issue at the CLI.
#[test]
fn test_cli_snapshot_export_issue_scope_resolves_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    let from = cli.create_issue(["From"]).unwrap();
    let to = cli.create_issue(["To"]).unwrap();
    cli.run(["dep", "add", &from, &to]).unwrap();

    let out = cli.root().join("snapshot");
    let scope = format!("issue:{}", short(&from));
    cli.run([
        "snapshot",
        "export",
        "--scope",
        &scope,
        "--out",
        out.to_str().unwrap(),
    ])
    .unwrap();
    assert!(out.exists());
    assert_rejects_too_short_prefix(
        &cli,
        &[
            "snapshot",
            "export",
            "--scope",
            "issue:{short}",
            "--out",
            "unused",
        ],
    );
}

/// `archive container` resolves its container prefix at the CLI.
#[test]
fn test_cli_archive_container_resolves_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["Container"]).unwrap();

    let plan = cli.run_json(["archive", "container", &short(&id)]).unwrap();
    assert_eq!(plan["target"]["id"], id.as_str());
    assert_rejects_too_short_prefix(&cli, &["archive", "container", "{short}"]);
}

/// `claim status --issue` resolves its filter prefix at the CLI.
#[test]
fn test_cli_claim_status_issue_filter_resolves_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    let commit = std::process::Command::new("git")
        .current_dir(cli.root())
        .args(["commit", "--allow-empty", "-m", "init"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(commit.status.success(), "{commit:?}");
    let claimed = cli.create_issue(["Claimed"]).unwrap();
    let agent = [("JIT_AGENT_ID", "agent:a")];
    let acquire = cli
        .command_with_env(["claim", "acquire", &claimed], agent)
        .output()
        .unwrap();
    assert!(acquire.status.success(), "{acquire:?}");

    let status = cli
        .command_with_env(
            ["claim", "status", "--issue", &short(&claimed), "--json"],
            agent,
        )
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    let leases: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(leases["leases"][0]["issue_id"], claimed.as_str());
    assert_rejects_too_short_prefix(&cli, &["claim", "status", "--issue", "{short}"]);
}
