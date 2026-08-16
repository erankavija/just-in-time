//! Coverage for the linked-checkout write policy: which invocations it refuses
//! (jit:f52567ed) and what a permitted override leaves behind (jit:0c8d38be).
//!
//! The two halves are one subject. Refusal is what makes an override an override,
//! and the audit record is what a permitted override owes; a test moving an
//! invocation from one half to the other only changes the stance it declares.

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

/// A repository whose primary checkout has a linked non-primary checkout beside
/// it, declaring no write stance, so the refusing default applies in the linked
/// one.
///
/// Answers the temporary directory holding both checkouts (kept alive by the
/// caller), the primary checkout's root, and the linked checkout's root.
fn linked_checkout_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
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

    (temp, primary, linked)
}

/// Declare `stance` as `checkout`'s repository-level linked-checkout write stance.
fn declare_stance(checkout: &Path, stance: LinkedCheckoutWriteStance) {
    let token = serde_json::to_value(stance).expect("the stance vocabulary serializes");
    let token = token.as_str().expect("a stance serializes as its token");
    let config = checkout.join(".jit/config.toml");
    let declared = std::fs::read_to_string(&config).expect("read the repository configuration")
        + &format!("\n[worktree]\nwrite_policy = \"{token}\"\n");
    std::fs::write(&config, declared).expect("declare the repository's write stance");
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
    // The repository declares nothing, so the refusing default is what the
    // invocation's own stance outranks — the only shape that warrants a record.
    let (_temp, _primary, linked) = linked_checkout_fixture();
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
    // Permitted by the declaration this test states for itself, so the
    // invocation supplies no override and there is none to record.
    let (_temp, _primary, linked) = linked_checkout_fixture();
    declare_stance(&linked, LinkedCheckoutWriteStance::Allow);
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
    let (_temp, _primary, linked) = linked_checkout_fixture();
    declare_stance(&linked, LinkedCheckoutWriteStance::Allow);

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

// The refusal half of the policy (jit:f52567ed): the guard runs at dispatch,
// before the repository mutation session opens, so a refused invocation is not a
// state change and leaves nothing behind to recover from.

/// Every byte under `checkout`'s `.jit/`, keyed by its path relative to it.
///
/// A refusal is observable as this map being unchanged: it covers the issue
/// records, the event log, the machine-local worktree identity, and any residue
/// a partially-opened session would have staged.
fn store_bytes(checkout: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn collect(
        directory: &Path,
        root: &Path,
        into: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(directory).expect("read a store directory") {
            let path = entry.expect("read a store entry").path();
            if path.is_dir() {
                collect(&path, root, into);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("entries live under the store");
                into.insert(
                    relative.to_path_buf(),
                    std::fs::read(&path).expect("read a store file"),
                );
            }
        }
    }

    let root = checkout.join(".jit");
    let mut bytes = std::collections::BTreeMap::new();
    collect(&root, &root, &mut bytes);
    bytes
}

/// The parsed `--json` error envelope printed by a refused invocation.
fn refusal_envelope(assert: assert_cmd::assert::Assert) -> serde_json::Value {
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(&stdout)
        .unwrap_or_else(|error| panic!("the refusal prints one JSON document ({error}): {stdout}"))
        ["error"]
        .clone()
}

#[test]
fn test_linked_checkout_mutation_under_the_refusing_stance_leaves_the_store_and_event_log_untouched(
) {
    let (_temp, _primary, linked) = linked_checkout_fixture();
    let before = store_bytes(&linked);

    create_issue_in(&linked, None).failure();

    assert_eq!(
        store_bytes(&linked),
        before,
        "a refused invocation runs before the repository mutation session opens, so not one \
         byte of the linked checkout's store — its issue records, its event log, or its \
         machine-local identity — may differ afterwards"
    );
}

#[test]
fn test_linked_checkout_refusal_names_the_checkout_the_refusing_stance_and_how_to_permit() {
    let (_temp, _primary, linked) = linked_checkout_fixture();

    let assert = create_issue_in(&linked, None).failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();

    assert!(
        stderr.contains(&linked.display().to_string()),
        "the refusal must name the linked checkout it refused in; stderr: {stderr}"
    );
    assert!(
        stderr.contains("refuse"),
        "the refusal must name the stance that refused it; stderr: {stderr}"
    );
    for permit_surface in ["write_policy", "allow", WRITE_STANCE_ENV] {
        assert!(
            stderr.contains(permit_surface),
            "the refusal must name the surfaces that permit the operation, including \
             {permit_surface}; stderr: {stderr}"
        );
    }
}

#[test]
fn test_linked_checkout_refusal_json_carries_the_checkout_the_refusing_stance_and_how_to_permit() {
    let (_temp, _primary, linked) = linked_checkout_fixture();

    let error = refusal_envelope(create_issue_in(&linked, None).failure());

    let named_checkout = std::fs::canonicalize(
        error["details"]["checkout"]
            .as_str()
            .unwrap_or_else(|| panic!("the envelope names the refused checkout: {error}")),
    )
    .expect("the named checkout exists");
    assert_eq!(
        named_checkout,
        std::fs::canonicalize(&linked).expect("the linked checkout exists"),
        "the machine-readable refusal must name the same linked checkout the rendered one does"
    );
    assert_eq!(
        error["details"]["stance"],
        serde_json::to_value(LinkedCheckoutWriteStance::Refuse).expect("the stance serializes"),
        "the machine-readable refusal must name the stance that refused it; error: {error}"
    );
    let suggestions = error["suggestions"]
        .as_array()
        .unwrap_or_else(|| panic!("the envelope carries permit guidance: {error}"))
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    for permit_surface in ["write_policy", "allow", WRITE_STANCE_ENV] {
        assert!(
            suggestions.contains(permit_surface),
            "the machine-readable refusal must name every surface that permits the operation, \
             including {permit_surface}; suggestions: {suggestions}"
        );
    }
}

#[test]
fn test_read_only_invocation_in_a_linked_checkout_succeeds_under_either_stance() {
    let (_temp, _primary, linked) = linked_checkout_fixture();

    for stance in ["refuse", "allow"] {
        std::process::Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&linked)
            .env(WRITE_STANCE_ENV, stance)
            .args(["list", "--json"])
            .assert()
            .success();
    }
}

#[test]
fn test_mutating_invocation_in_the_primary_checkout_succeeds_under_either_stance() {
    let (_temp, primary, _linked) = linked_checkout_fixture();

    for stance in ["refuse", "allow"] {
        create_issue_in(&primary, Some(stance)).success();
    }
}

#[test]
fn test_mutating_invocation_outside_version_control_succeeds_under_either_stance() {
    for stance in ["refuse", "allow"] {
        let temp = tempfile::TempDir::new().expect("create a temporary directory");
        assert!(
            !temp.path().join(".git").exists(),
            "the fixture models a repository outside version control"
        );
        std::process::Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(temp.path())
            .env(WRITE_STANCE_ENV, stance)
            .arg("init")
            .assert()
            .success();
        create_issue_in(temp.path(), Some(stance)).success();
    }
}

// The write-policy journey (jit:995a1903): every fact above is proven in
// isolation, each with its own freshly created checkout. This test instead
// reuses ONE real-git-created linked checkout across all three stances in
// sequence, which is what the isolated tests above cannot observe: that a
// refusal leaves the checkout in a state where a later, permitted mutation
// still lands cleanly (the refused invocation runs before the repository
// mutation session opens, so it cannot poison what comes after — the shape
// of the field incident this policy exists to prevent), and that a stance
// change the adopter declares between invocations takes effect on an
// already-existing checkout without recreating it.
fn redeclare_stance(checkout: &Path, base_config: &str, stance: LinkedCheckoutWriteStance) {
    let config = checkout.join(".jit/config.toml");
    let declared = format!(
        "{base_config}\n[worktree]\nwrite_policy = \"{}\"\n",
        stance.as_token()
    );
    std::fs::write(&config, declared).expect("declare the repository's write stance");
}

#[test]
fn test_linked_checkout_write_policy_journey_walks_refusal_then_declared_permit_then_override_permit_on_one_checkout(
) {
    let (_temp, _primary, linked) = linked_checkout_fixture();
    let config_path = linked.join(".jit/config.toml");
    let base_config =
        std::fs::read_to_string(&config_path).expect("read the base repository configuration");

    // Leg 1: no declared stance (the refusing default), no invocation
    // override. The invocation is refused and the checkout's store is
    // byte-identical afterward.
    let before_refusal = store_bytes(&linked);
    create_issue_in(&linked, None).failure();
    assert_eq!(
        store_bytes(&linked),
        before_refusal,
        "a refused invocation must leave no partial state on the checkout for a later, \
         permitted mutation on the same checkout to inherit"
    );

    // Leg 2: the checkout now declares the permitting stance. The same
    // invocation, needing no override, lands cleanly and records no override.
    redeclare_stance(&linked, &base_config, LinkedCheckoutWriteStance::Allow);
    create_issue_in(&linked, None).success();
    let events_after_declared_permit = durable_events(&linked);
    assert!(
        events_tagged(
            &events_after_declared_permit,
            "linked_checkout_write_overridden"
        )
        .is_empty(),
        "a mutation permitted by a stance declared after the checkout already existed must \
         still record no override; events: {events_after_declared_permit:?}"
    );
    let declared_permit_creations =
        events_tagged(&events_after_declared_permit, "issue_created").len();
    assert_eq!(
        declared_permit_creations, 1,
        "the refused leg contributed no durable mutation, so only the declared-permit leg's own \
         mutation is durable so far; events: {events_after_declared_permit:?}"
    );

    // Leg 3: the checkout's declaration flips back to refuse, so only the
    // invocation's own override can permit the next mutation on this same
    // checkout.
    redeclare_stance(&linked, &base_config, LinkedCheckoutWriteStance::Refuse);
    create_issue_in(&linked, Some("allow")).success();
    let events_after_override = durable_events(&linked);
    let override_records =
        events_tagged(&events_after_override, "linked_checkout_write_overridden");
    assert_eq!(
        override_records.len(),
        1,
        "the override leg must leave exactly one durable audit record behind; events: \
         {events_after_override:?}"
    );
    let record = override_records[0];
    assert_eq!(
        (&record["declared_stance"], &record["invocation_override"]),
        (&serde_json::json!("refuse"), &serde_json::json!("allow")),
        "the audit record must pair the re-declared refusal with the invocation stance that \
         permitted it on this same checkout; record: {record}"
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
        "the record must name the very checkout that carried all three legs of the journey"
    );
    assert_eq!(
        events_tagged(&events_after_override, "issue_created").len(),
        declared_permit_creations + 1,
        "the override leg's mutation must land in addition to the earlier legs' durable state, \
         proving the checkout accumulated state across the journey rather than being reset \
         between legs; events: {events_after_override:?}"
    );
}
