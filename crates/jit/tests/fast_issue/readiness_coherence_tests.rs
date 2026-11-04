//! Readiness coherence between stored state and the dependency graph
//! (jit:533da2df, `@/invariant/derived-state-coherence`).
//!
//! Readiness is stored on the issue and is also derivable from the graph through
//! [`Issue::is_blocked`]. These tests pin the two views to each other: at
//! creation (REQ-02), across whole-repository validation and its `--fix` remedy
//! (REQ-04), and over arbitrary dependency edits and state changes (REQ-06).

use jit::commands::BatchIssueDef;
use jit::domain::identity::{IssueId, IssueIdPrefix};
use jit::domain::{queries::build_issue_map, Issue, State};
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;
use jit::test_utils::add_reducing_edge;
use proptest::prelude::*;

/// A batch entry with the given symbolic key, title, and in-batch dependencies.
fn batch_def(key: &str, title: &str, depends_on: &[&str]) -> BatchIssueDef {
    BatchIssueDef {
        key: key.to_string(),
        title: title.to_string(),
        description: String::new(),
        r#type: None,
        priority: None,
        labels: Vec::new(),
        gates: Vec::new(),
        depends_on: depends_on.iter().map(|key| key.to_string()).collect(),
        planning: None,
    }
}

/// Titles of every issue whose stored `Ready` contradicts the graph — the exact
/// population the whole-repository check reports.
fn ready_but_blocked(issues: &[Issue]) -> Vec<&str> {
    let resolved = build_issue_map(issues);
    issues
        .iter()
        .filter(|issue| issue.state == State::Ready && issue.is_blocked(&resolved))
        .map(|issue| issue.title.as_str())
        .collect()
}

/// REQ-02: an entry created while it already depends on a non-terminal sibling
/// is born in `Backlog`, while the sibling it waits on — carrying no dependency
/// of its own — is born workable.
#[test]
fn test_batch_create_starts_an_entry_carrying_an_unmet_dependency_in_backlog() {
    let h = TestHarness::new();

    let outcome = h
        .executor
        .issues()
        .batch_create_from_json(vec![
            batch_def("upstream", "Upstream", &[]),
            batch_def("dependent", "Dependent", &["upstream"]),
        ])
        .unwrap();

    let issues = h.all_issues();
    assert_eq!(outcome.key_to_id.len(), 2);
    let dependent = issues
        .iter()
        .find(|issue| issue.title == "Dependent")
        .expect("the dependent entry was created");
    let upstream = issues
        .iter()
        .find(|issue| issue.title == "Upstream")
        .expect("the upstream entry was created");
    assert!(dependent.dependencies.contains(&upstream.id));
    assert_eq!(dependent.state, State::Backlog);
    assert!(dependent.first_ready_at.is_none());
    assert!(
        !h.storage
            .read_events()
            .unwrap()
            .iter()
            .any(|event| matches!(
                event.body(),
                jit::domain::EventBody::IssueStateChanged { issue_id, from, to, .. }
                    if *issue_id == dependent.id && *from == State::Ready && *to == State::Backlog
            )),
        "the blocked entry is never Ready, so it records no demotion"
    );
    assert_eq!(upstream.state, State::Ready);
    assert!(ready_but_blocked(&issues).is_empty());
}

/// Local rules validate each batch entry in its created state: a rule keyed on
/// `Ready` refuses an unblocked entry and ignores one an open sibling blocks.
#[test]
fn test_batch_create_validates_each_entry_in_its_created_state() {
    let h = TestHarness::new();
    h.storage.add_data_file(
        "rules.toml",
        r#"
[[rules]]
name = "ready-needs-req"
when = { state = "ready" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
    );
    let mut upstream = batch_def("upstream", "Upstream", &[]);
    upstream.labels = vec!["req:REQ-1".to_string()];

    h.executor
        .issues()
        .batch_create_from_json(vec![
            upstream,
            batch_def("dependent", "Dependent", &["upstream"]),
        ])
        .expect("the blocked entry is validated as Backlog, which the rule ignores");
    h.executor
        .issues()
        .batch_create_from_json(vec![batch_def("lone", "Lone", &[])])
        .expect_err("the unblocked entry is validated as Ready, which the rule refuses");

    assert!(h.all_issues().iter().all(|issue| issue.title != "Lone"));
}

/// REQ-01: adding a blocking dependency to a `Ready` issue demotes it and
/// records the demotion, so the event log explains the stored state.
#[test]
fn test_add_dependency_demotes_a_ready_dependent_and_records_the_state_change() {
    let h = TestHarness::new();
    let upstream = h.create_issue("Upstream");
    let dependent = h.create_issue("Dependent");
    assert_eq!(h.get_issue(&dependent).state, State::Ready);

    add_reducing_edge(&h.executor, &dependent, &upstream).unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Backlog);
    let events = h.storage.read_events().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event.body(),
            jit::domain::EventBody::IssueStateChanged { issue_id, from, to, .. }
                if *issue_id == dependent && *from == State::Ready && *to == State::Backlog
        )),
        "the demotion must append its own state-change event"
    );
}

/// REQ-04: whole-repository validation reports an issue stored as `Ready` while
/// its unmet-dependency list is non-empty, and names the remedy that repairs it.
#[test]
fn test_validate_reports_a_ready_issue_with_an_unmet_dependency_as_a_violation() {
    let h = TestHarness::new();
    let upstream = h.create_issue("Upstream");
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &upstream).unwrap();
    assert!(
        h.executor.validation().validate_silent().is_ok(),
        "the coherent repository must validate"
    );

    // Reintroduce the exact incoherence the defect used to persist: stored
    // `Ready` while the dependency that blocks it is untouched.
    let mut incoherent = h.get_issue(&dependent);
    incoherent.state = State::Ready;
    jit::test_harness::seed_memory_issue(&h.storage, &incoherent);

    let error = h
        .executor
        .validation()
        .validate_silent()
        .expect_err("a Ready issue with an unmet dependency must fail validation");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains(&incoherent.short_id()),
        "the violation must name the offending issue, got: {rendered}"
    );
    assert!(
        rendered.contains("jit validate --fix"),
        "the violation must name the remedy that repairs it, got: {rendered}"
    );
}

/// REQ-04: `jit validate --fix` repairs the violation by demoting the issue to
/// the state its dependencies imply, leaving the repository valid.
#[test]
fn test_validate_fix_demotes_a_ready_issue_that_carries_an_unmet_dependency() {
    let h = TestHarness::new();
    let upstream = h.create_issue("Upstream");
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &upstream).unwrap();
    let mut incoherent = h.get_issue(&dependent);
    incoherent.state = State::Ready;
    jit::test_harness::seed_memory_issue(&h.storage, &incoherent);

    let (fixes, _messages, _, _) = h.executor.validation().validate_with_fix(false).unwrap();

    assert_eq!(fixes, 1, "exactly the one incoherent issue is repaired");
    assert_eq!(h.get_issue(&dependent).state, State::Backlog);
    assert!(ready_but_blocked(&h.all_issues()).is_empty());
    assert!(h.executor.validation().validate_silent().is_ok());
}

/// REQ-04: the repair is auditable — the demotion `--fix` performs appends its
/// own state-change event, like every other state change.
#[test]
fn test_validate_fix_records_the_repair_demotion_in_the_event_log() {
    let h = TestHarness::new();
    let upstream = h.create_issue("Upstream");
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &upstream).unwrap();
    let mut incoherent = h.get_issue(&dependent);
    incoherent.state = State::Ready;
    jit::test_harness::seed_memory_issue(&h.storage, &incoherent);
    let events_before = h.storage.read_events().unwrap().len();

    h.executor.validation().validate_with_fix(false).unwrap();

    let events = h.storage.read_events().unwrap();
    assert!(events.len() > events_before);
    assert!(
        events.iter().skip(events_before).any(|event| matches!(
            event.body(),
            jit::domain::EventBody::IssueStateChanged { issue_id, from, to, .. }
                if *issue_id == dependent && *from == State::Ready && *to == State::Backlog
        )),
        "the repair must append its own Ready → Backlog event"
    );
}

/// REQ-04: a dry run reports the violation without changing stored state.
#[test]
fn test_validate_fix_dry_run_reports_the_incoherent_issue_without_repairing_it() {
    let h = TestHarness::new();
    let upstream = h.create_issue("Upstream");
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &upstream).unwrap();
    let mut incoherent = h.get_issue(&dependent);
    incoherent.state = State::Ready;
    jit::test_harness::seed_memory_issue(&h.storage, &incoherent);

    let (fixes, messages, _, _) = h.executor.validation().validate_with_fix(true).unwrap();

    assert_eq!(fixes, 1);
    assert!(
        messages
            .iter()
            .any(|message| message.contains(&incoherent.short_id())),
        "the dry run must name the issue it would repair, got: {messages:?}"
    );
    assert_eq!(
        h.get_issue(&dependent).state,
        State::Ready,
        "a dry run must not change stored state"
    );
}

/// One edit in a generated sequence: a dependency operation and the positions
/// of its endpoints in the issue set, or a state change of one issue.
#[derive(Debug, Clone, Copy)]
enum GraphEdit {
    Add { from: usize, to: usize },
    Remove { from: usize, to: usize },
    SetState { issue: usize, state: State },
}

/// The states a generated state change moves a blocker into or out of.
const EDITED_STATES: [State; 3] = [State::InProgress, State::Done, State::Rejected];

/// Generated sequences stay short enough that each case runs a bounded number of
/// real (in-memory) writes.
fn graph_edits(issue_count: usize) -> impl Strategy<Value = Vec<GraphEdit>> {
    let endpoints = (0..issue_count, 0..issue_count);
    proptest::collection::vec(
        prop_oneof![
            2 => endpoints
                .clone()
                .prop_map(|(from, to)| GraphEdit::Add { from, to }),
            1 => endpoints
                .clone()
                .prop_map(|(from, to)| GraphEdit::Remove { from, to }),
            3 => (0..issue_count, 0..EDITED_STATES.len()).prop_map(|(issue, state)| {
                GraphEdit::SetState {
                    issue,
                    state: EDITED_STATES[state],
                }
            }),
        ],
        0..12,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// After every edit of ANY sequence of dependency additions and removals and
    /// state changes into and out of `Done` and `Rejected`, no issue is stored
    /// `Ready` while blocked or `Backlog` while unblocked. Refused edits
    /// (self-edges, cycles, redundant edges, absent edges, refused transitions)
    /// change nothing, so they must not perturb the agreement either.
    #[test]
    fn prop_stored_readiness_agrees_with_the_blocked_predicate_after_dependency_edits(
        edits in graph_edits(3)
    ) {
        let h = TestHarness::new();
        let ids: Vec<IssueId> = (0..3)
            .map(|index| h.create_issue(&format!("Issue {index}")))
            .collect();

        for edit in edits {
            // A rejected edit (cycle, self-edge, redundancy, absent edge) is a
            // no-op by design; the property must hold over the whole sequence
            // either way.
            let _ = match edit {
                GraphEdit::Add { from, to } => add_reducing_edge(&h.executor, &ids[from], &ids[to]).map(|_| ()),
                GraphEdit::Remove { from, to } => h
                    .executor
                    .graph().remove_dependency(&ids[from], &ids[to])
                    .map(|_| ()),
                GraphEdit::SetState { issue, state: State::Rejected } => h
                    .executor
                    .issues()
                    .reject_issue(&IssueIdPrefix::from(&ids[issue]), None)
                    .map(|_| ()),
                GraphEdit::SetState { issue, state } => h
                    .executor
                    .issues()
                    .update_issue(&ids[issue], None, None, None, Some(state), vec![], vec![], None, None, false)
                    .map(|_| ()),
            };

            let issues = h.all_issues();
            let resolved = build_issue_map(&issues);
            for issue in &issues {
                let blocked = issue.is_blocked(&resolved);
                prop_assert!(
                    !(issue.state == State::Ready && blocked)
                        && !(issue.state == State::Backlog && !blocked),
                    "stored state {:?} disagrees with the blocked predicate ({}) for {} after {:?}",
                    issue.state,
                    blocked,
                    issue.short_id(),
                    edit
                );
            }
        }
    }
}

/// Two blockers each closed on a separate branch leave the merged dependent in
/// `Backlog` with nothing unmet. Validation reports it, and `--fix` promotes it.
#[test]
fn test_validate_reports_and_fix_promotes_a_backlog_dependent_whose_blockers_are_all_terminal() {
    let h = TestHarness::new();
    let first = h.create_issue("First blocker");
    let second = h.create_issue("Second blocker");
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &first).unwrap();
    add_reducing_edge(&h.executor, &dependent, &second).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Backlog);
    for blocker in [&first, &second] {
        let mut closed = h.get_issue(blocker);
        closed.state = State::Done;
        jit::test_harness::seed_memory_issue(&h.storage, &closed);
    }

    let rendered = format!(
        "{:#}",
        h.executor
            .validation()
            .validate_silent()
            .expect_err("a Backlog issue nothing blocks must fail validation")
    );
    assert!(rendered.contains(&h.get_issue(&dependent).short_id()));
    assert!(rendered.contains("jit validate --fix"));

    h.executor.validation().validate_with_fix(false).unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Ready);
    assert!(h.executor.validation().validate_silent().is_ok());
}
