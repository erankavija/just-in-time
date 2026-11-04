//! The divergence-and-recovery journey (jit:86ec2c50).
//!
//! The field incident this covers ended with issue and event history
//! recoverable only from a dangling commit. Nothing of that repository was
//! preserved, so the reproduction is built rather than restored: a real linked
//! checkout is diverged from its primary on purpose, and then the check and the
//! recovery procedure documented in
//! `docs/how-to/multi-agent-coordination.md` ("Divergent Checkout Stores") are
//! run exactly as an adopter runs them — the CLI and git, no library calls.
//!
//! The journey is one test over one reproduction because the criteria are
//! phases of one sequence: what the check reports about a diverged pair is what
//! the procedure then has to preserve, and the procedure's own last step is the
//! check reporting nothing. Splitting them would prove each phase against a
//! different divergence than the one before it.
//!
//! The sequence below is the documented one step for step; each such step names
//! the page step it executes.

use crate::linked_checkout_write_policy_tests::durable_events;
use jit::domain::store_divergence::DivergenceClass;
use jit::test_cli::{created_issue_id, JitCli};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const WRITE_STANCE_ENV: &str = "JIT_WORKTREE_WRITE_POLICY";

/// The store `checkout` owns.
fn store_of(checkout: &Path) -> PathBuf {
    checkout.join(".jit")
}

/// `path` with symlinks and relative components resolved, so two spellings of
/// one location compare equal.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|error| panic!("resolve {}: {error}", path.display()))
}

/// The store path `report` names under `field`, resolved.
fn reported_store(report: &Value, field: &str) -> PathBuf {
    resolved(Path::new(report[field].as_str().unwrap_or_else(|| {
        panic!("the report names a store under {field}; report: {report}")
    })))
}

/// The report `jit worktree store-divergence --json` prints when run from
/// `working_directory`, inspecting `selected_store` (the `JIT_DATA_DIR` the
/// invocation selects) when one is named.
fn divergence_report(working_directory: &JitCli, selected_store: Option<&Path>) -> Value {
    let output = working_directory
        .command_with_env(
            ["worktree", "store-divergence", "--json"],
            selected_store.map(|store| ("JIT_DATA_DIR", store)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "worktree store-divergence failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("the check prints one JSON document ({error}): {stdout}"))
}

/// Every `(record, class, id)` triple `report` names.
fn findings(report: &Value) -> BTreeSet<(String, String, String)> {
    report["divergences"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a divergence collection; report: {report}"))
        .iter()
        .map(|divergence| {
            let field = |name: &str| {
                divergence[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("every finding names its {name}: {divergence}"))
                    .to_owned()
            };
            (field("record"), field("class"), field("id"))
        })
        .collect()
}

/// The ids `findings` names for `record` records of `class`.
fn finding_ids(
    findings: &BTreeSet<(String, String, String)>,
    record: &str,
    class: &str,
) -> BTreeSet<String> {
    findings
        .iter()
        .filter(|(reported_record, reported_class, _)| {
            reported_record == record && reported_class == class
        })
        .map(|(_, _, id)| id.clone())
        .collect()
}

/// The ids of every event durable in `checkout`'s log.
fn event_ids(checkout: &Path) -> BTreeSet<String> {
    durable_events(checkout)
        .iter()
        .map(|event| {
            event["id"]
                .as_str()
                .unwrap_or_else(|| panic!("every event carries an id: {event}"))
                .to_owned()
        })
        .collect()
}

/// The bytes of `checkout`'s record for issue `id`.
fn issue_record(checkout: &Path, id: &str) -> Vec<u8> {
    let path = store_of(checkout).join("issues").join(format!("{id}.json"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Every issue id `checkout`'s store holds a record for.
fn issue_record_ids(checkout: &Path) -> BTreeSet<String> {
    std::fs::read_dir(store_of(checkout).join("issues"))
        .expect("read the checkout's issue records")
        .map(|entry| entry.expect("read one issue record entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .expect("an issue record is named for its id")
                .to_owned()
        })
        .collect()
}

/// Stage exactly what the procedure's commit steps stage in `checkout`: the
/// records the check reports and the index holding their tombstones.
///
/// The page names these paths rather than `.jit/` wholesale, because a
/// repository that has not gitignored the machine-local runtime files would
/// otherwise commit one machine's lock and worktree state into the shared store
/// — and `jit init` writes no ignore rule for them (jit:ea1745ee). Staging the
/// same paths here keeps the witness performing the documented instruction
/// rather than a wholesale add that only works where such an ignore file
/// happens to exist.
fn stage_documented_store_paths(checkout: &JitCli) {
    checkout
        .git([
            "add",
            ".jit/issues/",
            ".jit/events.jsonl",
            ".jit/index.json",
        ])
        .unwrap();
}

/// Apply the procedure's commit step in `checkout`: make every record it holds
/// durable enough to survive the checkout's removal.
fn commit_store(checkout: &JitCli, message: &str) {
    stage_documented_store_paths(checkout);
    checkout.git(["commit", "-qm", message]).unwrap();
}

/// Make `issue` depend on `dependency` in `checkout`, supplying an invocation
/// write stance where the checkout needs one.
///
/// An issue with neither dependencies nor dependents is an isolated node, which
/// repository-integrity validation rejects, so each side's own record is wired
/// to the shared one — otherwise nothing this journey recovers could validate
/// whatever the recovery did. The edge lives in the depending issue's record
/// alone, so the shared record stays byte-identical in both stores and the
/// divergence under test stays one-sided.
fn depend_on(checkout: &JitCli, issue: &str, dependency: &str, invocation_stance: Option<&str>) {
    let output = checkout
        .command_with_env(
            ["dep", "add", issue, dependency],
            invocation_stance.map(|stance| (WRITE_STANCE_ENV, stance)),
        )
        .output()
        .expect("spawn jit dep add");
    assert!(
        output.status.success(),
        "declaring {issue} dependent on {dependency} in {} failed: {}",
        checkout.root().display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_divergence_recovery_journey_preserves_both_checkouts_records_through_the_documented_procedure(
) {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let primary_store = store_of(primary.root());
    let linked_store = store_of(linked.root());
    let current_branch = |checkout: &JitCli| {
        checkout
            .git(["branch", "--show-current"])
            .unwrap()
            .trim()
            .to_owned()
    };
    let retained_branch = current_branch(&primary);
    let checkout_branch = current_branch(&linked);

    // A shared baseline first: one record both stores hold, landed in the
    // primary and taken by the linked checkout through an ordinary git merge.
    // Without it every record in the repository would be one-sided, and the
    // report naming only disjoint records would be indistinguishable from a
    // report naming everything.
    let issue_description = ["-d", "Created by the divergence journey"];
    let shared_issue = primary
        .create_issue(
            ["Held by both checkouts"]
                .into_iter()
                .chain(issue_description),
        )
        .unwrap();
    commit_store(&primary, "land the shared record");
    linked
        .git(["merge", &retained_branch, "--no-edit"])
        .unwrap();

    // Nothing to report while the two stores agree — and the reference store is
    // named, so this is the agreement the procedure's final step looks for
    // rather than the empty result it warns proves nothing.
    let agreed = divergence_report(&primary, Some(&linked_store));
    assert_eq!(
        reported_store(&agreed, "reference_store"),
        resolved(&primary_store),
        "the check run from the primary must compare the selected linked store against the \
         primary's own; report: {agreed}"
    );
    assert!(
        findings(&agreed).is_empty(),
        "two stores holding the same records must produce no finding; report: {agreed}"
    );

    // Diverge them on purpose, in both directions: each checkout creates a
    // record its counterpart's store never sees, and each creation carries its
    // own events into that store's log alone. Writing inside the linked
    // checkout needs the permitting stance, supplied per invocation
    // (jit:f52567ed).
    let primary_issue = primary
        .create_issue(
            ["Held by the primary checkout alone"]
                .into_iter()
                .chain(issue_description),
        )
        .unwrap();
    depend_on(&primary, &primary_issue, &shared_issue, None);
    let linked_creation = linked
        .command_with_env(
            ["issue", "create", "Held by the linked checkout alone", "-d"]
                .into_iter()
                .chain(["Created by the divergence journey", "--json"]),
            [(WRITE_STANCE_ENV, "allow")],
        )
        .output()
        .unwrap();
    let linked_issue =
        created_issue_id(&serde_json::from_slice(&linked_creation.stdout).unwrap()).unwrap();
    depend_on(&linked, &linked_issue, &shared_issue, Some("allow"));
    let primary_events = event_ids(primary.root());
    let linked_events = event_ids(linked.root());
    let primary_only_events = &primary_events - &linked_events;
    let linked_only_events = &linked_events - &primary_events;
    let shared_events = &primary_events & &linked_events;
    assert!(
        !shared_events.is_empty()
            && !primary_only_events.is_empty()
            && !linked_only_events.is_empty(),
        "the reproduction needs history both logs share and history each holds alone; \
         primary: {primary_events:?}, linked: {linked_events:?}"
    );
    let primary_record_before_merge = issue_record(primary.root(), &primary_issue);
    let linked_record_before_merge = issue_record(linked.root(), &linked_issue);

    // Procedure step 1, run from the primary checkout's working directory with
    // the linked checkout's store selected as the one under inspection. That
    // selection is what lets a primary-side invocation observe the linked
    // store at all: the selected root decides which checkout is inspected, so
    // the primary becomes the reference rather than the subject.
    let diverged = divergence_report(&primary, Some(&linked_store));
    assert_eq!(
        reported_store(&diverged, "checkout_store"),
        resolved(&linked_store),
        "the inspected store must be the linked checkout's; report: {diverged}"
    );
    assert_eq!(
        reported_store(&diverged, "reference_store"),
        resolved(&primary_store),
        "the primary's store must be the one it is compared against; report: {diverged}"
    );
    let reported = findings(&diverged);
    assert_eq!(
        finding_ids(&reported, "issue", "local_only"),
        BTreeSet::from([linked_issue.clone()]),
        "the record only the linked checkout holds must be reported as its own; \
         report: {diverged}"
    );
    assert_eq!(
        finding_ids(&reported, "issue", "reference_only"),
        BTreeSet::from([primary_issue.clone()]),
        "the record only the primary holds must be reported as the reference's; \
         report: {diverged}"
    );
    assert_eq!(
        finding_ids(&reported, "event", "local_only"),
        linked_only_events,
        "the events must be reported as disjoint exactly where the two logs are; \
         report: {diverged}"
    );
    assert_eq!(
        finding_ids(&reported, "event", "reference_only"),
        primary_only_events,
        "the events must be reported as disjoint exactly where the two logs are; \
         report: {diverged}"
    );
    assert!(
        reported
            .iter()
            .all(|(_, _, id)| !shared_events.contains(id) && id != &shared_issue),
        "a record both stores hold is not a divergence and must go unreported; \
         report: {diverged}"
    );

    // Procedure step 2, in whichever checkouts still hold a reported record
    // uncommitted — here, both.
    commit_store(&primary, "preserve the primary checkout's record");
    commit_store(&linked, "preserve the linked checkout's record");

    // Procedure step 3, read each finding class before acting on it: every
    // finding here is one-sided, so the table's automatic path applies and no
    // record needs a human decision.
    assert!(
        reported.iter().all(|(_, class, _)| [
            DivergenceClass::LocalOnly,
            DivergenceClass::ReferenceOnly
        ]
        .iter()
        .any(|one_sided| class == one_sided.as_str())),
        "this reproduction diverges by records held on one side only; report: {diverged}"
    );

    // Procedure step 4: merge the checkout's branch into the branch being kept,
    // from the primary. This is the step that actually preserves a record.
    primary
        .git([
            "merge",
            &checkout_branch,
            "-m",
            "merge the linked checkout's records",
        ])
        .unwrap();

    // The recovered store validates as it stands.
    let validation = primary.command(["validate"]).output().unwrap();
    assert!(
        validation.status.success(),
        "the recovered repository must validate as it stands; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&validation.stdout),
        String::from_utf8_lossy(&validation.stderr)
    );

    // REQ-02: the retained branch's store now holds both checkouts' records and
    // both event histories, with neither side's record replaced by the other's.
    let expected_records = BTreeSet::from([
        shared_issue.clone(),
        primary_issue.clone(),
        linked_issue.clone(),
    ]);
    assert_eq!(
        issue_record_ids(primary.root()),
        expected_records,
        "the merged store must hold the shared record and both checkouts' own"
    );
    assert_eq!(
        issue_record(primary.root(), &primary_issue),
        primary_record_before_merge,
        "the merge must not have rewritten the record the primary already held"
    );
    assert_eq!(
        issue_record(primary.root(), &linked_issue),
        linked_record_before_merge,
        "the record taken from the linked checkout must arrive as that checkout wrote it"
    );
    let merged_events = event_ids(primary.root());
    assert!(
        merged_events.is_superset(&primary_events) && merged_events.is_superset(&linked_events),
        "both event histories must survive the merge whole; merged: {merged_events:?}, \
         primary: {primary_events:?}, linked: {linked_events:?}"
    );

    // Procedure step 5: bring the linked checkout onto the retained branch.
    // Step 4 propagates records one way only, so the primary-only records are
    // still absent from the linked checkout's store and the confirming re-run
    // cannot come back clean until the linked checkout takes the retained
    // branch too (jit:62518095).
    linked
        .git(["merge", &retained_branch, "--no-edit"])
        .unwrap();

    // Procedure step 6: confirm agreement. Both the primary-side invocation
    // this journey has used throughout and the in-checkout one the page names
    // must report nothing while still naming a real reference store.
    for (working_directory, selected_store) in
        [(&primary, Some(linked_store.as_path())), (&linked, None)]
    {
        let confirmed = divergence_report(working_directory, selected_store);
        assert_eq!(
            reported_store(&confirmed, "reference_store"),
            resolved(&primary_store),
            "a confirming re-run must still name the real reference store the first run did, or \
             it proves nothing; report: {confirmed}"
        );
        assert!(
            findings(&confirmed).is_empty(),
            "once the recovery has landed on both sides the two stores agree and the check \
             reports nothing; report: {confirmed}"
        );
    }
}
