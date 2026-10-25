//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes. Short
//! prefixes enter through the CLI, which resolves each issue-id argument once.
use jit::test_cli::JitCli;
use jit::test_harness::TestHarness;

/// Every issue-id position of the CLI accepts a unique short prefix and acts on
/// the issue it names.
#[test]
fn test_cli_issue_id_arguments_resolve_a_unique_short_prefix() {
    let cli = JitCli::initialised().unwrap();
    let from = cli.create_issue(["From"]).unwrap();
    let to = cli.create_issue(["To"]).unwrap();
    let short = |id: &str| id[..8].to_string();

    let shown = cli.run_json(["issue", "show", &short(&from)]).unwrap();
    assert_eq!(shown["id"], from.as_str());

    cli.run(["issue", "update", &short(&from), "--title", "Renamed"])
        .unwrap();
    cli.run(["dep", "add", &short(&from), &short(&to)]).unwrap();
    let updated = cli.run_json(["issue", "show", &from]).unwrap();
    assert_eq!(updated["title"], "Renamed");
    assert_eq!(updated["dependencies"][0]["id"], to.as_str());

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

/// The event-query issue filter and the server document reads resolve a full
/// id in any case or hyphenation to the issue it names.
#[test]
fn test_issue_id_filters_and_document_reads_resolve_any_spelling_of_a_full_id() {
    let h = TestHarness::new();
    let id = h.create_issue("Documented");
    h.executor
        .add_document_reference(&id, "d.md", None, None, None, true)
        .unwrap();
    let events = h
        .executor
        .query_events(None, Some(id.clone()), 100)
        .unwrap();
    assert!(!events.is_empty());
    // The in-memory store backs no document bytes, so a read that resolved the
    // issue fails on the document exactly as the canonical spelling does.
    let read = |id: &str| format!("{:?}", h.executor.read_document_content(id, "d.md", None));
    assert_ne!(read(&id), read("ffffffff"));

    for spelling in [id.to_uppercase(), id.replace('-', "")] {
        assert_eq!(
            h.executor
                .query_events(None, Some(spelling.clone()), 100)
                .unwrap(),
            events,
            "{spelling}"
        );
        assert_eq!(read(&spelling), read(&id), "{spelling}");
    }
}

/// An event-query issue filter naming a deleted issue still selects its
/// events, in any spelling.
#[test]
fn test_query_events_issue_filter_matches_a_deleted_issue_in_any_spelling() {
    let h = TestHarness::new();
    let id = h.create_issue("Deleted");
    h.executor.issues().delete_issue(&id).unwrap();

    let events = h
        .executor
        .query_events(None, Some(id.to_uppercase()), 100)
        .unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.get_issue_id() == id));
}
