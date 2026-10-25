//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes.
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;

#[test]
fn test_cli_show_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Use short hash in show command
    let prefix = &id[..8];
    let issue = h.executor.issues().show_issue(prefix).unwrap();
    assert_eq!(issue.id, id);
    assert_eq!(issue.title, "Test issue");
}

#[test]
fn test_cli_update_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Use short hash in update command
    let prefix = &id[..8];
    let _ = h
        .executor
        .issues()
        .update_issue(
            prefix,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    // Verify it worked
    let issue = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue.id, id);
}

#[test]
fn test_cli_dependency_with_short_hashes() {
    let h = TestHarness::new();
    let id1 = h.create_issue("Issue 1");
    let id2 = h.create_issue("Issue 2");

    // Use short hashes for both IDs
    let prefix1 = &id1[..8];
    let prefix2 = &id2[..8];

    h.executor.add_dependency(prefix1, prefix2).unwrap();

    // Verify dependency was added
    let issue1 = h.storage.load_issue(&id1).unwrap();
    assert!(issue1.dependencies.contains(&id2));
}

#[test]
fn test_cli_delete_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Delete using short hash
    let prefix = &id[..8];
    h.executor.issues().delete_issue(prefix).unwrap();

    // Verify it's gone
    let result = h.storage.load_issue(&id);
    assert!(result.is_err());
}

/// Every command entry point taking an issue id refuses a prefix below the
/// minimum as an invalid prefix before reading any issue record: the only
/// record is unreadable, so a read before the refusal surfaces instead.
#[test]
fn test_every_issue_id_entry_point_rejects_a_short_prefix_before_reading_records() {
    use jit::commands::GraphExportFormat;
    use jit::document::DocumentScope;
    use std::collections::BTreeMap;

    type EntryPoint = fn(&TestHarness, &str, &str) -> anyhow::Result<()>;
    let entry_points: &[(&str, EntryPoint)] = &[
        ("issue show", |h, short, _| {
            h.executor.issues().show_issue(short).map(drop)
        }),
        ("issue children", |h, short, _| {
            h.executor.issues().issue_children(short).map(drop)
        }),
        ("issue progress", |h, short, _| {
            h.executor.issues().issue_progress(short).map(drop)
        }),
        ("issue update", |h, short, _| {
            h.executor
                .issues()
                .update_issue(
                    short,
                    Some("t".into()),
                    None,
                    None,
                    None,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .map(drop)
        }),
        ("issue state", |h, short, _| {
            h.executor
                .issues()
                .update_issue_state(short, jit::domain::State::Done)
                .map(drop)
        }),
        ("issue reject", |h, short, _| {
            h.executor.issues().reject_issue(short, None).map(drop)
        }),
        ("issue reject with reason", |h, short, _| {
            h.executor
                .issues()
                .reject_issue(short, Some("dup"))
                .map(drop)
        }),
        ("issue assign", |h, short, _| {
            h.executor
                .issues()
                .assign_issue(short, "agent:a".into())
                .map(drop)
        }),
        ("issue claim", |h, short, _| {
            h.executor
                .issues()
                .claim_issue(short, "agent:a".into())
                .map(drop)
        }),
        ("issue unassign", |h, short, _| {
            h.executor.issues().unassign_issue(short).map(drop)
        }),
        ("issue release", |h, short, _| {
            h.executor.issues().release_issue(short, "r")
        }),
        ("issue delete", |h, short, _| {
            h.executor.issues().delete_issue(short).map(drop)
        }),
        ("dep add source", |h, short, full| {
            h.executor.add_dependency(short, full).map(drop)
        }),
        ("dep add target", |h, short, full| {
            h.executor.add_dependency(full, short).map(drop)
        }),
        ("dep add batch", |h, short, full| {
            h.executor
                .add_dependencies(full, &[short.to_string()])
                .map(drop)
        }),
        ("dep rm source", |h, short, full| {
            h.executor.remove_dependency(short, full).map(drop)
        }),
        ("dep rm target", |h, short, full| {
            h.executor.remove_dependency(full, short).map(drop)
        }),
        ("dep rm batch", |h, short, full| {
            h.executor
                .remove_dependencies(full, &[short.to_string()])
                .map(drop)
        }),
        ("template container", |h, short, _| {
            h.executor
                .apply_template("t", short, &BTreeMap::new(), false)
                .map(drop)
        }),
        ("template anchor", |h, short, full| {
            let anchors = BTreeMap::from([("a".to_string(), short.to_string())]);
            h.executor
                .apply_template("t", full, &anchors, false)
                .map(drop)
        }),
        ("gate add", |h, short, _| {
            h.executor.add_gates(short, &["g".into()]).map(drop)
        }),
        ("gate remove", |h, short, _| {
            h.executor.remove_gates(short, &["g".into()]).map(drop)
        }),
        ("gate pass", |h, short, _| {
            h.executor
                .pass_gate(short, "g".into(), None, false)
                .map(drop)
        }),
        ("gate pass all", |h, short, _| {
            h.executor.pass_all_gates(short, None, false).map(drop)
        }),
        ("gate fail", |h, short, _| {
            h.executor.fail_gate(short, "g".into(), None).map(drop)
        }),
        ("gate evaluate", |h, short, _| {
            h.executor.check_gate(short, "g").map(drop)
        }),
        ("gate preset apply", |h, short, _| {
            h.executor
                .apply_gate_preset(short, "p", None, false, false, &[])
                .map(drop)
        }),
        ("gate preset create", |h, short, _| {
            h.executor.create_gate_preset("p", short).map(drop)
        }),
        ("gate runs", |h, short, _| {
            h.executor.list_gate_runs(short, None).map(drop)
        }),
        ("gate last run", |h, short, _| {
            h.executor.get_last_gate_run(short, "g").map(drop)
        }),
        ("gate last runs", |h, short, _| {
            h.executor.get_last_gate_runs_for_issue(short).map(drop)
        }),
        ("gate statuses", |h, short, _| {
            h.executor
                .get_required_gate_statuses_for_issue(short)
                .map(drop)
        }),
        ("doc add", |h, short, _| {
            h.executor
                .add_document_reference(short, "d.md", None, None, None, true)
                .map(drop)
        }),
        ("doc list", |h, short, _| {
            h.executor.list_document_references(short).map(drop)
        }),
        ("doc remove", |h, short, _| {
            h.executor
                .remove_document_reference(short, "d.md")
                .map(drop)
        }),
        ("doc show", |h, short, _| {
            h.executor
                .show_document_content(short, "d.md", None)
                .map(drop)
        }),
        ("doc history", |h, short, _| {
            h.executor.document_history(short, "d.md").map(drop)
        }),
        ("doc diff", |h, short, _| {
            h.executor
                .document_diff(short, "d.md", "HEAD", None)
                .map(drop)
        }),
        ("doc assets", |h, short, _| {
            h.executor
                .list_document_assets(short, "d.md", false)
                .map(drop)
        }),
        ("doc artifact dir", |h, short, _| {
            h.executor
                .resolve_issue_artifact_directory(short, "a")
                .map(drop)
        }),
        ("doc check links", |h, short, _| {
            h.executor
                .check_document_links(&DocumentScope::Issue(short.to_string()))
                .map(drop)
        }),
        ("graph tree", |h, short, _| {
            h.executor.build_dependency_tree(short, 1).map(drop)
        }),
        ("graph deps", |h, short, _| {
            h.executor.show_dependencies_with_depth(short, 1).map(drop)
        }),
        ("graph rdeps", |h, short, _| {
            h.executor.show_rdeps_with_depth(short, 1).map(drop)
        }),
        ("graph export", |h, short, _| {
            h.executor
                .export_graph(GraphExportFormat::Dot, false, Some(short))
                .map(drop)
        }),
        ("graph batch", |h, short, _| {
            h.executor.export_graph_batch(Some(short)).map(drop)
        }),
        ("graph hierarchy", |h, short, _| {
            h.executor.resolve_hierarchy_tree(Some(short)).map(drop)
        }),
        ("validate issue", |h, short, _| {
            h.executor.run_rules(Some(short)).map(drop)
        }),
        ("validate scope", |h, short, _| {
            h.executor.validate_scope(short).map(drop)
        }),
        ("validate rule", |h, short, _| {
            h.executor.validate_rule(short, "r").map(drop)
        }),
        ("validate explain", |h, short, _| {
            h.executor.explain_rules(short).map(drop)
        }),
        ("item show", |h, short, _| {
            h.executor.show_item(&format!("{short}/x")).map(drop)
        }),
        ("archive preview", |h, short, _| {
            h.executor.preview_archive_container(short).map(drop)
        }),
        ("events query", |h, short, _| {
            h.executor
                .query_events(None, Some(short.into()), 10)
                .map(drop)
        }),
    ];

    let h = TestHarness::new();
    let full = h.create_issue("Unreadable");
    h.storage
        .add_data_file(format!("issues/{full}.json"), "not an issue record");
    let short = &full[..jit::domain::identity::IssueIdPrefix::MIN_ID_PREFIX_LENGTH - 1];

    let unrefused: Vec<String> = entry_points
        .iter()
        .filter_map(|(name, call)| {
            let refused = call(&h, short, &full).is_err_and(|error| {
                jit::output::failure::classify(&error).error.code
                    == jit::output::ErrorCode::InvalidIdPrefix
            });
            (!refused).then(|| name.to_string())
        })
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
