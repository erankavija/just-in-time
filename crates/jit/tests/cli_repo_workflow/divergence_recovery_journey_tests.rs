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
//! Two points in the procedure gave no usable instruction when this was first
//! run — the index conflict a two-sided merge raises, and the second merge the
//! confirming re-run needs — and both were defects in the page rather than in
//! the check. The page now instructs them (jit:62518095), so the sequence below
//! is the documented one step for step; each such step names the page step it
//! executes.

use crate::linked_checkout_write_policy_tests::{
    create_issue_in, created_issue_id, durable_events, linked_checkout_fixture,
};
use crate::worktree_cli_tests::store_divergence_output;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `git` with `arguments` in `checkout`, answering its captured output
/// whether or not it succeeded.
fn git(checkout: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(checkout)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("spawn git {arguments:?}: {error}"))
}

/// Run `git` with `arguments` in `checkout`, requiring success, and answer its
/// trimmed standard output.
fn git_ok(checkout: &Path, arguments: &[&str]) -> String {
    let output = git(checkout, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?} failed in {}: {}",
        checkout.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The branch `checkout` has checked out.
fn current_branch(checkout: &Path) -> String {
    git_ok(checkout, &["branch", "--show-current"])
}

/// Every path git left unmerged in `checkout`; empty once no merge is pending.
fn unmerged_paths(checkout: &Path) -> BTreeSet<String> {
    git_ok(checkout, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .map(str::to_owned)
        .collect()
}

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
/// `working_directory`, inspecting `selected_store` when one is named.
fn divergence_report(working_directory: &Path, selected_store: Option<&Path>) -> Value {
    let stdout = store_divergence_output(working_directory, selected_store, true);
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

/// The id set `checkout`'s committed index claims the store holds.
fn indexed_ids(checkout: &Path) -> BTreeSet<String> {
    let path = store_of(checkout).join("index.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let index: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("the index is one JSON document ({error}): {raw}"));
    index["all_ids"]
        .as_array()
        .unwrap_or_else(|| panic!("the index carries its id collection: {index}"))
        .iter()
        .map(|id| {
            id.as_str()
                .unwrap_or_else(|| panic!("every indexed id is a string: {index}"))
                .to_owned()
        })
        .collect()
}

/// The `.jit/index.json` content git staged for `stage` of the pending merge.
fn index_stage(checkout: &Path, stage: u8) -> Value {
    let specifier = format!(":{stage}:.jit/index.json");
    let raw = git_ok(checkout, &["show", &specifier]);
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("merge stage {stage} of the index parses ({error}): {raw}"))
}

/// Resolve a conflicted `.jit/index.json` by keeping every record either side
/// indexed.
///
/// This is the resolution the procedure's step 4 instructs: a merge of two
/// stores that each created an issue conflicts here rather than in
/// `.jit/issues/`, every time, because both sides inserted an id into one small
/// array over a common base, and the union of the two sides is what the
/// finding-class table's `local_only` and `reference_only` rows already decided
/// to keep. The page also stops routing this conflict to `jit validate --fix`,
/// which fails to parse the conflicted file and, once it is resolved to one
/// side alone, reports that no fixes were needed while `jit validate` rejects
/// the same repository for the index disagreeing with the issue files
/// (jit:62518095).
///
/// The union is read out of git's own merge stages rather than reconstructed,
/// so the resolution inherits whatever the two sides actually indexed
/// (jit:86ec2c50).
fn resolve_index_conflict_keeping_every_record(checkout: &Path) {
    let ours = index_stage(checkout, 2);
    let theirs = index_stage(checkout, 3);
    assert_eq!(
        ours["schema_version"], theirs["schema_version"],
        "both sides of a store divergence carry one repository format version; \
         ours: {ours}, theirs: {theirs}"
    );
    let side_ids = |side: &Value, field: &str| -> BTreeSet<String> {
        side[field]
            .as_array()
            .unwrap_or_else(|| panic!("an index side carries {field}: {side}"))
            .iter()
            .map(|id| {
                id.as_str()
                    .unwrap_or_else(|| panic!("every id under {field} is a string: {side}"))
                    .to_owned()
            })
            .collect()
    };
    let union =
        |field: &str| -> BTreeSet<String> { &side_ids(&ours, field) | &side_ids(&theirs, field) };
    let resolved = serde_json::json!({
        "all_ids": union("all_ids"),
        "deleted_ids": union("deleted_ids"),
        "schema_version": ours["schema_version"],
    });
    std::fs::write(
        store_of(checkout).join("index.json"),
        serde_json::to_vec_pretty(&resolved).expect("the resolved index serializes"),
    )
    .expect("write the resolved index");
}

/// Apply the procedure's commit step in `checkout`: make every record it holds
/// durable enough to survive the checkout's removal.
fn commit_store(checkout: &Path, message: &str) {
    git_ok(checkout, &["add", ".jit/"]);
    git_ok(checkout, &["commit", "-qm", message]);
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
fn depend_on(checkout: &Path, issue: &str, dependency: &str, invocation_stance: Option<&str>) {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command
        .current_dir(checkout)
        .args(["dep", "add", issue, dependency]);
    match invocation_stance {
        Some(stance) => command.env("JIT_WORKTREE_WRITE_POLICY", stance),
        None => command.env_remove("JIT_WORKTREE_WRITE_POLICY"),
    };
    let output = command.output().expect("spawn jit dep add");
    assert!(
        output.status.success(),
        "declaring {issue} dependent on {dependency} in {} failed: {}",
        checkout.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run plain `jit validate` in `checkout`, answering its process outcome.
///
/// Plain, never `--fix`: the repair path reports that no fixes were needed
/// whenever it applies none, and only re-runs validation when it applied at
/// least one, so it exits 0 on a repository plain validation rejects. The
/// unadorned command is the one whose exit status means what it says.
fn validate(checkout: &Path) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(checkout)
        .arg("validate")
        .output()
        .expect("spawn jit validate")
}

#[test]
fn test_divergence_recovery_journey_preserves_both_checkouts_records_through_the_documented_procedure(
) {
    let (_temp, primary, linked) = linked_checkout_fixture();
    let primary_store = store_of(&primary);
    let linked_store = store_of(&linked);
    let retained_branch = current_branch(&primary);
    let checkout_branch = current_branch(&linked);

    // A shared baseline first: one record both stores hold, landed in the
    // primary and taken by the linked checkout through an ordinary git merge.
    // Without it every record in the repository would be one-sided, and the
    // report naming only disjoint records would be indistinguishable from a
    // report naming everything.
    let shared_issue =
        created_issue_id(create_issue_in(&primary, "Held by both checkouts", None).success());
    commit_store(&primary, "land the shared record");
    git_ok(&linked, &["merge", &retained_branch, "--no-edit"]);

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
    let primary_issue = created_issue_id(
        create_issue_in(&primary, "Held by the primary checkout alone", None).success(),
    );
    depend_on(&primary, &primary_issue, &shared_issue, None);
    let linked_issue = created_issue_id(
        create_issue_in(&linked, "Held by the linked checkout alone", Some("allow")).success(),
    );
    depend_on(&linked, &linked_issue, &shared_issue, Some("allow"));
    let primary_events = event_ids(&primary);
    let linked_events = event_ids(&linked);
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
    let primary_record_before_merge = issue_record(&primary, &primary_issue);
    let linked_record_before_merge = issue_record(&linked, &linked_issue);

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
        reported
            .iter()
            .all(|(_, class, _)| class == "local_only" || class == "reference_only"),
        "this reproduction diverges by records held on one side only; report: {diverged}"
    );

    // Procedure step 4: merge the checkout's branch into the branch being kept,
    // from the primary. This is the step that actually preserves a record.
    let merge = git(
        &primary,
        &[
            "merge",
            &checkout_branch,
            "-m",
            "merge the linked checkout's records",
        ],
    );
    let conflicted = unmerged_paths(&primary);
    assert!(
        conflicted.iter().all(|path| path == ".jit/index.json"),
        "one-sided findings must merge without a decision in the records themselves — the event \
         log merges by union and each issue is its own file; unmerged: {conflicted:?}, merge \
         output: {}",
        String::from_utf8_lossy(&merge.stderr)
    );
    if !conflicted.is_empty() {
        // Step 4's index resolution; see
        // `resolve_index_conflict_keeping_every_record`.
        resolve_index_conflict_keeping_every_record(&primary);
        git_ok(&primary, &["add", ".jit/"]);
        git_ok(&primary, &["commit", "-q", "--no-edit"]);
    }

    // The recovered store validates as it stands. This is what makes the index
    // resolution above a repair rather than a patch that merely parses: plain
    // validation is the check that rejects an index disagreeing with the issue
    // files, so its passing here says the union kept exactly the records the
    // merge preserved.
    let validation = validate(&primary);
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
        issue_record_ids(&primary),
        expected_records,
        "the merged store must hold the shared record and both checkouts' own"
    );
    assert_eq!(
        indexed_ids(&primary),
        expected_records,
        "the merged store's index must claim exactly the records the merge preserved"
    );
    assert_eq!(
        issue_record(&primary, &primary_issue),
        primary_record_before_merge,
        "the merge must not have rewritten the record the primary already held"
    );
    assert_eq!(
        issue_record(&primary, &linked_issue),
        linked_record_before_merge,
        "the record taken from the linked checkout must arrive as that checkout wrote it"
    );
    let merged_events = event_ids(&primary);
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
    git_ok(&linked, &["merge", &retained_branch, "--no-edit"]);

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
