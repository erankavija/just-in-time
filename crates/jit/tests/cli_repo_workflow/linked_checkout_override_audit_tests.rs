//! Durability coverage for the linked-checkout override audit record (jit:0c8d38be) — the
//! record and the mutation it permits become durable through one materialization plan, or
//! neither does.

use assert_cmd::prelude::*;
use jit::commands::CommandExecutor;
use jit::domain::{Event, LinkedCheckoutWriteStance, Priority};
use jit::storage::{
    IssueStore, JsonFileStorage, RepositoryStateStore, TransactionFailureInjector,
    TransactionFailurePoint,
};
use std::path::Path;
use std::sync::Arc;

fn override_annotation(repository: &Path) -> jit::repository_state::MutationContextAnnotation {
    jit::repository_state::MutationContextAnnotation::LinkedCheckoutWriteOverridden {
        checkout: repository.join(".worktrees/feature"),
        declared_stance: LinkedCheckoutWriteStance::Refuse,
        invocation_override: LinkedCheckoutWriteStance::Allow,
    }
}

/// Fails the transaction kernel at the commit decision, after the mutation's
/// coupled issue and event materialization has been staged but before it commits.
struct FailAtCommitDecision;

impl TransactionFailureInjector for FailAtCommitDecision {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &TransactionFailurePoint::RepositoryBeforeCommitDecision {
            return Err(std::io::Error::other(
                "injected override-audit persistence failure",
            ));
        }
        Ok(())
    }
}

#[test]
fn test_create_issue_under_dispatch_override_persists_the_audit_record_with_the_issue_events() {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy()
        .expect("initialize the fixture repository");
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root())
        .expect("discover the repository layout");
    let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
    let events_before = storage
        .read_events()
        .expect("read the event log before the permitted mutation");
    let expected_checkout = temp.path().join(".worktrees/feature");
    let expected_declared_stance = LinkedCheckoutWriteStance::Refuse;
    let expected_invocation_override = LinkedCheckoutWriteStance::Allow;
    let annotation = override_annotation(temp.path());

    let (issue_id, _) = jit::commands::with_dispatch_mutation_annotation(annotation, || {
        executor.create_issue(
            "Permitted mutation".to_string(),
            "Issue created while an override permitted the invocation".to_string(),
            Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
    })
    .expect("the permitted mutation succeeds");

    let events_after = storage
        .read_events()
        .expect("read the event log after the permitted mutation");
    assert!(
        events_after.starts_with(&events_before),
        "the permitted mutation must preserve the prior event-log prefix"
    );
    let appended = &events_after[events_before.len()..];
    let override_records = appended
        .iter()
        .filter(|event| matches!(event, Event::LinkedCheckoutWriteOverridden { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        override_records.len(),
        1,
        "the permitted mutation must append exactly one linked-checkout override audit record"
    );
    let Event::LinkedCheckoutWriteOverridden {
        id,
        timestamp,
        checkout,
        declared_stance,
        invocation_override,
    } = override_records[0]
    else {
        unreachable!("the selected event is the linked-checkout override audit record");
    };
    assert_eq!(
        checkout, &expected_checkout,
        "the audit record must name the checkout selected by the dispatch site"
    );
    assert_eq!(
        declared_stance, &expected_declared_stance,
        "the audit record must preserve the declared linked-checkout write stance"
    );
    assert_eq!(
        invocation_override, &expected_invocation_override,
        "the audit record must preserve the invocation stance that permitted the mutation"
    );
    assert!(
        !id.is_empty(),
        "the finalizer must assign a non-empty identifier to the audit record"
    );

    let issue_created_timestamp = appended
        .iter()
        .find_map(|event| match event {
            Event::IssueCreated {
                issue_id: id,
                timestamp,
                ..
            } if id == &issue_id => Some(timestamp),
            _ => None,
        })
        .expect("the issue-created event must be in the same append as the audit record");
    assert_eq!(
        timestamp, issue_created_timestamp,
        "the audit record and issue-created event must share one mutation timestamp"
    );
    assert!(
        matches!(
            appended.last(),
            Some(Event::LinkedCheckoutWriteOverridden { .. })
        ),
        "canonical event ordering must place the override audit record last in the append"
    );
    storage
        .load_issue(&issue_id)
        .expect("the mutation explained by the audit record must be durable");
}

#[test]
fn test_create_issue_under_dispatch_override_persistence_failure_leaves_neither_issue_nor_audit_record(
) {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy()
        .expect("initialize the fixture repository");
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root())
        .expect("discover the repository layout");
    let events_before = storage
        .read_events()
        .expect("read the event log before the failed mutation");
    let issues_before = storage
        .read_issues()
        .expect("read the issue set before the failed mutation");
    let jit_dir = temp.path().join(".jit");
    let injected =
        JsonFileStorage::with_repository_state_failures(&jit_dir, Arc::new(FailAtCommitDecision));
    let executor = CommandExecutor::new(injected).with_layout(layout);
    let annotation = override_annotation(temp.path());

    let error = jit::commands::with_dispatch_mutation_annotation(annotation, || {
        executor.create_issue(
            "Permitted mutation".to_string(),
            "Issue created while an override permitted the invocation".to_string(),
            Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
    })
    .expect_err("an injected persistence failure must not be reported as a completed mutation");
    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("injected override-audit persistence failure"),
        "the surfaced error must identify the injected persistence failure; error: {error_text}"
    );

    let recovered = JsonFileStorage::new(&jit_dir);
    let recovered_layout = jit::storage::discover_repository_layout(temp.path(), recovered.root())
        .expect("rediscover the repository layout for recovery");
    drop(
        RepositoryStateStore::open_mutation_session(&recovered, recovered_layout)
            .expect("recovery converges"),
    );

    let recovered_events = recovered
        .read_events()
        .expect("read the event log after persistence recovery");
    assert_eq!(
        recovered_events, events_before,
        "a failed materialization must leave neither the audit record nor issue events durable"
    );
    let recovered_issues = recovered
        .read_issues()
        .expect("read the issue set after persistence recovery");
    assert_eq!(
        recovered_issues.len(),
        issues_before.len(),
        "a failed materialization must leave the durable issue count unchanged"
    );
    assert!(
        !recovered_events
            .iter()
            .any(|event| matches!(event, Event::LinkedCheckoutWriteOverridden { .. })),
        "the audit record cannot outlive the mutation it explains"
    );
}

// The tests below drive the real production dispatch path — the installed `jit`
// binary, its worktree detection, its declared-stance read, and the environment
// variable carrying one invocation's stance — rather than installing the
// annotation themselves.

/// Environment variable carrying one invocation's linked-checkout write stance.
const WRITE_STANCE_ENV: &str = "JIT_WORKTREE_WRITE_POLICY";

/// A linked non-primary checkout of a repository that declares no write stance,
/// so the refusing default applies there.
///
/// Answers the temporary directory holding both checkouts (kept alive by the
/// caller) and the linked checkout's root.
fn linked_checkout_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::TempDir::new().expect("create a temporary directory");
    let primary = temp.path().join("main");
    std::fs::create_dir(&primary).expect("create the primary checkout directory");

    for arguments in [
        vec!["init", "-q"],
        vec!["config", "user.email", "fixture@example.com"],
        vec!["config", "user.name", "Fixture"],
    ] {
        std::process::Command::new("git")
            .current_dir(&primary)
            .args(&arguments)
            .assert()
            .success();
    }
    std::process::Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&primary)
        .arg("init")
        .assert()
        .success();
    for arguments in [vec!["add", "-A"], vec!["commit", "-qm", "track with jit"]] {
        std::process::Command::new("git")
            .current_dir(&primary)
            .args(&arguments)
            .assert()
            .success();
    }

    let linked = temp.path().join("feature");
    std::process::Command::new("git")
        .current_dir(&primary)
        .args([
            "worktree",
            "add",
            "-q",
            linked.to_str().expect("utf-8 path"),
        ])
        .assert()
        .success();

    (temp, linked)
}

/// Every event durable in `checkout`'s store, one per log line.
fn durable_events(checkout: &Path) -> Vec<serde_json::Value> {
    let log = std::fs::read_to_string(checkout.join(".jit/events.jsonl"))
        .expect("read the checkout's event log");
    log.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every event log line is one JSON record"))
        .collect()
}

/// The events of `tag` among `events`.
fn events_tagged<'a>(events: &'a [serde_json::Value], tag: &str) -> Vec<&'a serde_json::Value> {
    events.iter().filter(|event| event["type"] == tag).collect()
}

/// Create one issue in `checkout`, optionally supplying an invocation stance.
fn create_issue_in(checkout: &Path, invocation_stance: Option<&str>) -> assert_cmd::assert::Assert {
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command.current_dir(checkout).args([
        "issue",
        "create",
        "Permitted mutation",
        "-d",
        "Created inside a linked checkout",
        "--json",
    ]);
    match invocation_stance {
        Some(stance) => command.env(WRITE_STANCE_ENV, stance),
        None => command.env_remove(WRITE_STANCE_ENV),
    };
    command.assert()
}

#[test]
fn test_linked_checkout_mutation_under_env_override_records_the_audit_event_with_its_mutation() {
    let (_temp, linked) = linked_checkout_fixture();
    create_issue_in(&linked, Some("allow")).success();

    let events = durable_events(&linked);
    let records = events_tagged(&events, "linked_checkout_write_overridden");
    assert_eq!(
        records.len(),
        1,
        "the permitted mutation must record exactly one override; events: {events:?}"
    );
    let record = records[0];
    assert_eq!(
        (&record["declared_stance"], &record["invocation_override"]),
        (&serde_json::json!("refuse"), &serde_json::json!("allow")),
        "the record must pair the declaration that would have refused with the stance that \
         permitted the invocation; record: {record}"
    );
    let recorded_checkout = std::fs::canonicalize(
        record["checkout"]
            .as_str()
            .expect("the record names a checkout path"),
    )
    .expect("the recorded checkout exists");
    assert_eq!(
        recorded_checkout,
        std::fs::canonicalize(&linked).expect("the linked checkout exists"),
        "the record must name the linked checkout the invocation actually mutated"
    );

    let creations = events_tagged(&events, "issue_created");
    let creation = creations
        .last()
        .expect("the mutation's own event is durable too");
    assert_eq!(
        creation["timestamp"], record["timestamp"],
        "sharing one mutation timestamp is what shows both records came from one plan; \
         events: {events:?}"
    );
}

#[test]
fn test_linked_checkout_mutation_without_env_override_records_no_audit_event() {
    let (_temp, linked) = linked_checkout_fixture();
    create_issue_in(&linked, None).success();

    let events = durable_events(&linked);
    assert!(
        events_tagged(&events, "linked_checkout_write_overridden").is_empty(),
        "an invocation supplying no override must record none; events: {events:?}"
    );
    assert!(
        !events_tagged(&events, "issue_created").is_empty(),
        "the mutation itself must still have happened; events: {events:?}"
    );
}

#[test]
fn test_linked_checkout_mutation_permitted_by_declared_stance_records_no_audit_event() {
    // The repository's own declaration permits the mutation, so no override
    // outranked a refusal and the record's "stance that would have refused" has
    // no referent.
    let (_temp, linked) = linked_checkout_fixture();
    let config = linked.join(".jit/config.toml");
    let declared = std::fs::read_to_string(&config).expect("read the repository configuration")
        + "\n[worktree]\nwrite_policy = \"allow\"\n";
    std::fs::write(&config, declared).expect("declare the allowing stance");

    create_issue_in(&linked, Some("allow")).success();

    let events = durable_events(&linked);
    assert!(
        events_tagged(&events, "linked_checkout_write_overridden").is_empty(),
        "a mutation permitted by the declared stance records no override; events: {events:?}"
    );
    assert!(
        !events_tagged(&events, "issue_created").is_empty(),
        "the mutation itself must still have happened; events: {events:?}"
    );
}

#[test]
fn test_invalid_env_write_stance_token_fails_the_invocation_naming_accepted_tokens() {
    // The stance is parsed before any repository work, so an unusable token
    // fails the invocation rather than being silently dropped.
    let temp = tempfile::TempDir::new().expect("create a temporary directory");
    let output = std::process::Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env(WRITE_STANCE_ENV, "sometimes")
        .arg("list")
        .output()
        .expect("spawn the jit process");

    assert!(
        !output.status.success(),
        "an unusable invocation stance must fail the invocation"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in ["sometimes", "refuse", "allow"] {
        assert!(
            stderr.contains(expected),
            "the failure must name the offending token and every accepted one; stderr: {stderr}"
        );
    }
}
