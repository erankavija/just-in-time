//! Integration tests for `jit doc conformance`, the advisory report naming
//! artifacts that sit outside their owning issue's canonical directory.
//!
//! The report is advice, not enforcement (`@/issue/8e071e18/decision/D-7`), so
//! these cases pin both halves: what it names, and that naming it changes
//! nothing. The repository, issue, and area fixtures come from the sibling
//! `doc_show_tests` module, and the canonical directory a case compares
//! against is whatever `jit doc dir` resolves in that same repository, so no
//! expectation here restates the naming rule.

use crate::doc_show_tests::{
    create_issue, declared_area, initialized_repo, jit, membership_type_and_namespace,
    resolve_directory, undeclared_area, CreatedIssue,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A short-id-shaped name no issue answers to: eight lowercase hex characters,
/// which is the width and alphabet of the short id `jit` prints.
const UNOWNED_PREFIX: &str = "deadbeef";

/// One parsed `doc conformance --json` run.
struct Report {
    /// Areas the run states it scanned.
    areas: Vec<String>,
    /// The collection's declared length.
    count: usize,
    /// Reported artifacts, keyed by the path each one names.
    artifacts: BTreeMap<String, Value>,
}

impl Report {
    /// The single entry naming `path`, or `None` when the run passed over it.
    fn entry(&self, path: &str) -> Option<&Value> {
        self.artifacts.get(path)
    }

    /// The entry naming `path`, failing the test when the run passed over it.
    fn require(&self, path: &str) -> &Value {
        self.entry(path)
            .unwrap_or_else(|| panic!("the report names {path}; it named {:?}", self.paths()))
    }

    fn paths(&self) -> Vec<&String> {
        self.artifacts.keys().collect()
    }
}

/// Run the report with `--json` in `repo`, requiring success, and parse it.
fn conformance_report(repo: &Path) -> Report {
    let output = jit(repo, &["doc", "conformance", "--json"]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "doc conformance --json exits successfully: {} {stdout}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("doc conformance --json parses ({error}): {stdout}"));
    let artifacts = payload["artifacts"]
        .as_array()
        .unwrap_or_else(|| panic!("the report wraps a collection named artifacts: {payload}"))
        .iter()
        .map(|entry| {
            (
                entry["path"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a reported artifact names its path: {entry}"))
                    .to_string(),
                entry.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    Report {
        areas: payload["areas"]
            .as_array()
            .unwrap_or_else(|| panic!("the report states the areas it scanned: {payload}"))
            .iter()
            .map(|area| area.as_str().expect("an area is a string").to_string())
            .collect(),
        count: payload["count"]
            .as_u64()
            .unwrap_or_else(|| panic!("the report counts its collection: {payload}"))
            as usize,
        artifacts,
    }
}

/// Write a file at the repository-relative `path`, creating its parents.
fn write_artifact(repo: &Path, path: &str) {
    let absolute = repo.join(path);
    fs::create_dir_all(absolute.parent().expect("an artifact sits in a directory")).unwrap();
    fs::write(&absolute, format!("# {path}\n")).unwrap();
}

/// Create the repository-relative directory `path` and put no file in it.
fn create_artifact_directory(repo: &Path, path: &str) {
    fs::create_dir_all(repo.join(path)).unwrap();
}

/// An issue whose labels resolve a single membership value, so the directory it
/// owns is distinguishable from the bare area it is written into.
fn issue_owning_a_directory(repo: &Path, title: &str) -> CreatedIssue {
    let (issue_type, namespace) = membership_type_and_namespace();
    create_issue(
        repo,
        title,
        &[
            format!("type:{issue_type}"),
            format!("{namespace}:artifact-layout"),
        ],
    )
}

/// Replace the `issue_scoped_areas` registry in the repository's configuration.
///
/// `jit init` scaffolds the shipped declaration as an explicit list, so this
/// rewrites that list in place and leaves the rest of the table alone.
fn declare_issue_scoped_areas(repo: &Path, areas: &[&str]) {
    let config_path = repo.join(".jit/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    let start = config
        .find("issue_scoped_areas = [")
        .expect("the scaffolded configuration declares issue_scoped_areas");
    let end = config[start..]
        .find(']')
        .map(|offset| start + offset + 1)
        .expect("the declaration is a closed list");
    let rendered = areas
        .iter()
        .map(|area| format!("\"{area}\""))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        &config_path,
        format!(
            "{}issue_scoped_areas = [{rendered}]{}",
            &config[..start],
            &config[end..]
        ),
    )
    .unwrap();
}

/// The areas a freshly initialized repository declares issue-scoped. `jit init`
/// scaffolds the shipped policy, so this is that repository's configured
/// registry rather than a second copy of the list.
fn scaffolded_areas() -> Vec<String> {
    jit::config::SHIPPED_DOCUMENTATION_POLICY
        .issue_scoped_areas
        .iter()
        .map(|area| (*area).to_string())
        .collect()
}

/// Every file under `root` with its bytes, so a run can be shown to have
/// changed nothing.
fn tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(directory: &Path, prefix: &str, into: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        entries.iter().for_each(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if path.is_dir() {
                into.insert(format!("{relative}/"), Vec::new());
                walk(path, &relative, into);
            } else {
                into.insert(relative, fs::read(path).unwrap());
            }
        });
    }

    let mut snapshot = BTreeMap::new();
    walk(root, "", &mut snapshot);
    snapshot
}

/// The paths that appeared, vanished, or changed bytes between two snapshots.
fn differences(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[test]
fn test_doc_conformance_names_an_artifact_sitting_outside_its_owning_issues_canonical_directory() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Owns a canonical directory");
    let canonical = resolve_directory(repo.path(), &issue.short_id, area);

    // Two artifacts of the same shape and the same owner, separated only by
    // where they were written.
    let inside = format!("{canonical}/{}-plan.md", issue.short_id);
    let outside = format!("{area}/{}-plan.md", issue.short_id);
    [&inside, &outside].iter().for_each(|path| {
        write_artifact(repo.path(), path);
    });

    let report = conformance_report(repo.path());
    let entry = report.require(&outside);
    assert_eq!(
        entry["canonical_directory"].as_str(),
        Some(canonical.as_str()),
        "the entry names the directory its owner owns in that area: {entry}"
    );
    assert_eq!(
        entry["issue_id"].as_str(),
        Some(issue.id.as_str()),
        "the entry attributes the artifact to the issue that owns it: {entry}"
    );
    assert!(
        report.entry(&inside).is_none(),
        "an artifact already inside that directory is not reported: {:?}",
        report.paths()
    );
    assert!(
        report.entry(&canonical).is_none(),
        "the directory its owner owns is not itself reported: {:?}",
        report.paths()
    );

    // The verdict follows the location and nothing else: relocating the same
    // bytes under the same owner empties the report.
    fs::rename(
        repo.path().join(&outside),
        repo.path().join(format!("{canonical}/relocated.md")),
    )
    .unwrap();
    let relocated = conformance_report(repo.path());
    assert!(
        relocated.artifacts.is_empty(),
        "moving the artifact into its canonical directory clears the report: {:?}",
        relocated.paths()
    );
}

#[test]
fn test_doc_conformance_names_a_misplaced_prefixed_directory_holding_no_file() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Owns directories rather than files");
    let canonical = resolve_directory(repo.path(), &issue.short_id, area);

    // Three misplaced directories carrying the owner's prefix, differing only
    // in what they hold: nothing at all, directories alone at every depth, and
    // one file. Their location is what makes them nonconforming, so what they
    // contain must not decide whether they are named.
    let empty = format!("{area}/legacy/{}-plan", issue.short_id);
    let directories_only = format!("{area}/{}-notes", issue.short_id);
    let populated = format!("{area}/{}-draft", issue.short_id);
    create_artifact_directory(repo.path(), &empty);
    create_artifact_directory(repo.path(), &format!("{directories_only}/part/section"));
    write_artifact(repo.path(), &format!("{populated}/plan.md"));

    let misplaced = [empty, directories_only, populated];
    let report = conformance_report(repo.path());
    misplaced.iter().for_each(|directory| {
        let entry = report.require(directory);
        assert_eq!(
            entry["canonical_directory"].as_str(),
            Some(canonical.as_str()),
            "the directory is reported against the one its owner owns: {entry}"
        );
        assert_eq!(
            entry["issue_id"].as_str(),
            Some(issue.id.as_str()),
            "the directory is attributed to the issue whose prefix it carries: {entry}"
        );
    });

    // Each is one occurrence: no path beneath a reported directory is reported
    // in its own right, and the enclosing `legacy/` carries no prefix to name.
    assert!(
        report
            .paths()
            .into_iter()
            .all(|path| misplaced.contains(path)),
        "a misplaced directory is named once rather than once per descendant: {:?}",
        report.paths()
    );
    assert_eq!(
        report.count,
        misplaced.len(),
        "the three misplaced directories are the whole report: {:?}",
        report.paths()
    );
}

#[test]
fn test_doc_conformance_scans_exactly_the_areas_the_configuration_registry_declares() {
    let repo = initialized_repo();
    let scaffolded = declared_area();
    let outside = undeclared_area();
    let issue = issue_owning_a_directory(repo.path(), "Artifacts in two areas");

    // The same misplaced shape in both areas, so what separates them is the
    // registry rather than anything about the artifact.
    let in_registry = format!("{scaffolded}/{}-plan.md", issue.short_id);
    let out_of_registry = format!("{outside}/{}-plan.md", issue.short_id);
    [&in_registry, &out_of_registry].iter().for_each(|path| {
        write_artifact(repo.path(), path);
    });

    let scaffolded_report = conformance_report(repo.path());
    assert_eq!(
        scaffolded_report.areas,
        scaffolded_areas(),
        "the scanned areas are the ones the initialized repository declares"
    );
    assert!(
        scaffolded_report.entry(&in_registry).is_some()
            && scaffolded_report.entry(&out_of_registry).is_none(),
        "only the declared area is walked: {:?}",
        scaffolded_report.paths()
    );

    // Declaring the other area instead moves the whole scan with it, which no
    // built-in list of areas could do.
    declare_issue_scoped_areas(repo.path(), &[outside]);
    let redeclared = conformance_report(repo.path());
    assert_eq!(
        redeclared.areas,
        vec![outside.to_string()],
        "the scanned areas follow the authored registry"
    );
    assert!(
        redeclared.entry(&out_of_registry).is_some() && redeclared.entry(&in_registry).is_none(),
        "the redeclared area is the one walked: {:?}",
        redeclared.paths()
    );

    // A lexical spelling of an area names that area, and an entry that
    // normalizes to nothing names none: the registry decides membership under
    // the same rules `doc dir` applies to a caller's spelling.
    declare_issue_scoped_areas(repo.path(), &["", &format!("./{scaffolded}/")]);
    let respelled = conformance_report(repo.path());
    assert_eq!(
        respelled.areas,
        vec![scaffolded.to_string()],
        "an entry naming no area is neither walked nor claimed as walked"
    );
    assert!(
        respelled.entry(&in_registry).is_some(),
        "the respelled area is still walked: {:?}",
        respelled.paths()
    );

    // An empty registry opts every area out, so nothing is walked at all.
    declare_issue_scoped_areas(repo.path(), &[]);
    let none_declared = conformance_report(repo.path());
    assert!(
        none_declared.areas.is_empty() && none_declared.artifacts.is_empty(),
        "declaring no issue-scoped area leaves nothing to scan: {:?}",
        none_declared.paths()
    );
}

#[test]
fn test_doc_conformance_exits_successfully_and_changes_nothing_whatever_it_finds() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Unchanged by its own report");

    // One unrelated read first. The storage layer publishes its advisory index
    // lock on the first read of any command, so warming it here keeps that
    // shared artifact out of the comparisons below and leaves them exact:
    // every file, with its bytes.
    assert!(jit(repo.path(), &["issue", "list"]).status.success());

    // Planted after that warming read, so nothing the report could delete has
    // already been swept away before the comparisons start. An empty
    // `<id>.lock` beside an issue record is what the retired per-issue read
    // lock left behind, and the read-all maintenance path collects exactly
    // that shape — so a run that reaches it unlinks this file and the exact
    // snapshots below say so.
    let sidecar = repo.path().join(format!(".jit/issues/{}.lock", issue.id));
    fs::write(&sidecar, "").unwrap();

    let nonconforming = format!("{area}/{}-plan.md", issue.short_id);
    let unattributed = format!("{area}/{UNOWNED_PREFIX}-notes.md");

    // A clean repository, then each finding the report can make.
    [None, Some(&nonconforming), Some(&unattributed)]
        .iter()
        .for_each(|added| {
            if let Some(path) = added {
                write_artifact(repo.path(), path);
            }
            let before = tree_snapshot(repo.path());
            let report = conformance_report(repo.path());
            let after = tree_snapshot(repo.path());

            assert!(
                differences(&before, &after).is_empty(),
                "the run mutates nothing while reporting {:?}, but touched {:?}",
                report.paths(),
                differences(&before, &after)
            );
            // The plain-text run is the same statement without --json, and it
            // exits alike.
            let plain = jit(repo.path(), &["doc", "conformance"]);
            assert!(
                plain.status.success(),
                "doc conformance exits successfully: {}",
                String::from_utf8_lossy(&plain.stderr)
            );
            assert!(
                differences(&after, &tree_snapshot(repo.path())).is_empty(),
                "the plain-text run mutates nothing either, but touched {:?}",
                differences(&after, &tree_snapshot(repo.path()))
            );
        });

    assert!(
        sidecar.exists(),
        "every run above left the planted issue-lock sidecar in place"
    );

    // The successful exits above were not vacuous: by the last pass the report
    // had both kinds of finding to make.
    let report = conformance_report(repo.path());
    assert!(
        report.entry(&nonconforming).is_some() && report.entry(&unattributed).is_some(),
        "the runs above had findings to report: {:?}",
        report.paths()
    );
}

#[test]
fn test_doc_conformance_findings_block_no_state_transition() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Completes with a misplaced artifact");
    let nonconforming = format!("{area}/{}-plan.md", issue.short_id);
    write_artifact(repo.path(), &nonconforming);

    // The precondition the rest of this case rests on.
    assert!(
        conformance_report(repo.path())
            .entry(&nonconforming)
            .is_some(),
        "the artifact this issue owns is reported as nonconforming"
    );

    // The reported artifact stays put while its owner runs the lifecycle out.
    ["in_progress", "done"].iter().for_each(|state| {
        let output = jit(
            repo.path(),
            &["issue", "update", &issue.short_id, "--state", state],
        );
        assert!(
            output.status.success(),
            "a nonconforming artifact blocks the transition to {state}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    });

    let validate = jit(repo.path(), &["validate"]);
    assert!(
        validate.status.success(),
        "a nonconforming artifact does not fail repository validation: {} {}",
        String::from_utf8_lossy(&validate.stdout),
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(
        repo.path().join(&nonconforming).exists(),
        "the reported artifact is left where it is"
    );
}

#[test]
fn test_doc_conformance_reports_an_artifact_with_no_resolvable_owner_as_unattributed() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Resolvable owner");

    let owned = format!("{area}/{}-plan.md", issue.short_id);
    let orphan = format!("{area}/{UNOWNED_PREFIX}-plan.md");
    [&owned, &orphan].iter().for_each(|path| {
        write_artifact(repo.path(), path);
    });

    // The precondition: nothing in this repository answers to the orphan's
    // prefix, while the sibling artifact's prefix resolves an issue.
    assert!(
        !jit(repo.path(), &["issue", "show", UNOWNED_PREFIX])
            .status
            .success(),
        "no issue answers to {UNOWNED_PREFIX}"
    );

    let report = conformance_report(repo.path());
    let orphan_entry = report.require(&orphan);
    let owned_entry = report.require(&owned);

    assert_ne!(
        orphan_entry["status"], owned_entry["status"],
        "an unresolvable owner is a different verdict from a nonconforming location: \
         {orphan_entry} vs {owned_entry}"
    );
    assert!(
        orphan_entry["status"]
            .as_str()
            .is_some_and(|status| status.contains("unattributed")),
        "the orphan is reported as unattributed: {orphan_entry}"
    );
    assert!(
        orphan_entry["issue_id"].as_str().is_none()
            && orphan_entry["canonical_directory"].as_str().is_none(),
        "an unattributed artifact names neither an owner nor a directory it belongs in: \
         {orphan_entry}"
    );
    assert!(
        owned_entry["issue_id"].as_str() == Some(issue.id.as_str())
            && owned_entry["canonical_directory"].as_str().is_some(),
        "the resolvable sibling still carries its attribution: {owned_entry}"
    );
}

#[test]
fn test_doc_conformance_wraps_its_collection_in_the_count_and_collection_envelope() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Counted artifacts");

    let empty = conformance_report(repo.path());
    assert_eq!(
        (empty.count, empty.artifacts.len()),
        (0, 0),
        "an empty collection is still counted"
    );

    // The count tracks the collection rather than being a constant.
    (1..=3).for_each(|index| {
        write_artifact(
            repo.path(),
            &format!("{area}/{}-note-{index}.md", issue.short_id),
        );
        let report = conformance_report(repo.path());
        assert_eq!(
            (report.count, report.artifacts.len()),
            (index, index),
            "count states the number of reported artifacts"
        );
    });
}

#[test]
fn test_doc_conformance_human_output_names_each_artifact_and_where_it_belongs() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = issue_owning_a_directory(repo.path(), "Readable report");
    let canonical = resolve_directory(repo.path(), &issue.short_id, area);
    let nonconforming = format!("{area}/{}-plan.md", issue.short_id);
    let unattributed = format!("{area}/{UNOWNED_PREFIX}-notes.md");
    [&nonconforming, &unattributed].iter().for_each(|path| {
        write_artifact(repo.path(), path);
    });

    let output = jit(repo.path(), &["doc", "conformance"]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "doc conformance exits successfully"
    );

    [
        nonconforming.as_str(),
        canonical.as_str(),
        unattributed.as_str(),
    ]
    .iter()
    .for_each(|expected| {
        assert!(
            stdout.contains(expected),
            "the human report names {expected}: {stdout}"
        );
    });

    // A clean repository says so rather than printing an empty list.
    let clean = initialized_repo();
    let clean_output = jit(clean.path(), &["doc", "conformance"]);
    assert!(clean_output.status.success());
    assert!(
        !String::from_utf8_lossy(&clean_output.stdout)
            .trim()
            .is_empty(),
        "a report with no findings still states its result"
    );
}
