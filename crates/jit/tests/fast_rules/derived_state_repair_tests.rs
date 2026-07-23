use crate::harness::TestHarness;
use jit::repository_state::{
    CaptureBudget, CaptureSpec, FileMode, RepositoryEntry, RepositoryRootClass, VirtualPath,
};
use jit::storage::{IssueStore, RepositoryStateStore};
use std::collections::BTreeMap;

const RULE_ASSERTION: &str = "assert = { json-schema = \"schemas/default-label-format.json\" }";
const STALE_RULE_ASSERTION: &str =
    "assert = { require-label = { label = \"authored:*\", min = 99 } }";
const PROFILE_ASSET: &str = ".agents/skills/jit-manage/SKILL.md";
const EXECUTABLE_ASSET: &str = "scripts/ai-review.sh";
const PROFILE_RECORD: &str = ".jit/profiles/jit-dogfood.json";

fn read(harness: &TestHarness, path: &str) -> String {
    harness.storage.read_repo_file(path).unwrap().unwrap()
}

fn write(harness: &TestHarness, path: &str, content: &str) {
    if let Some(relative) = path.strip_prefix(".jit/") {
        harness.storage.add_data_file(relative, content);
    } else {
        harness.storage.add_worktree_file(path, content);
    }
}

fn profiled_harness(omit_profile_record: bool) -> TestHarness {
    let source = tempfile::tempdir().unwrap();
    let source_storage = jit::storage::JsonFileStorage::new(source.path().join(".jit"));
    let source_layout =
        jit::storage::discover_repository_layout(source.path(), source_storage.root()).unwrap();
    jit::commands::CommandExecutor::new(source_storage)
        .with_layout(source_layout)
        .initialize_fresh_repository(
            source.path(),
            &jit::hierarchy_templates::HierarchyTemplate::default(),
            Some("jit-dogfood"),
        )
        .unwrap();

    let harness = TestHarness::new();
    for path in repair_paths()
        .into_iter()
        .filter(|path| !omit_profile_record || path != PROFILE_RECORD)
    {
        write(
            &harness,
            &path,
            &std::fs::read_to_string(source.path().join(&path)).unwrap(),
        );
    }
    harness
}

fn capture(
    harness: &TestHarness,
    paths: impl IntoIterator<Item = String>,
) -> BTreeMap<VirtualPath, RepositoryEntry> {
    let layout = harness.storage.repository_layout();
    let paths = paths
        .into_iter()
        .map(|path| layout.classify_repository_relative(path).unwrap())
        .collect::<Vec<_>>();
    let (fixed, discovered): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .partition(|path| path.root_class() == RepositoryRootClass::Data);
    let mut spec = CaptureSpec::phase_one(
        fixed,
        CaptureBudget {
            max_paths: 256,
            max_listings: 0,
            max_bytes: 16 * 1024 * 1024,
            max_depth: 16,
        },
    )
    .unwrap();
    spec.discover_paths(discovered).unwrap();
    let mut session = harness
        .storage
        .open_mutation_session(layout.clone())
        .unwrap();
    session.capture(spec).unwrap().entries().clone()
}

fn repair_paths() -> Vec<String> {
    let package = jit::profile::jit_dogfood_package().unwrap();
    package
        .hashes()
        .targets
        .keys()
        .cloned()
        .chain(
            [
                PROFILE_RECORD,
                ".jit/config.toml",
                ".jit/gates.toml",
                ".jit/rules.toml",
                ".jit/reference/rules-and-gates.md",
                ".jit/schemas/default-label-format.json",
                ".jit/schemas/default-namespace-registry.json",
                ".jit/schemas/default-type-hierarchy-known.json",
                ".jit/index.json",
                ".jit/events.jsonl",
                "AGENTS.md",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .collect()
}

#[test]
fn test_harness_validate_fix_repairs_each_owned_class_and_preserves_authored_bytes() {
    let mut harness = profiled_harness(false);
    let baseline = [
        ".jit/rules.toml",
        ".jit/schemas/default-type-hierarchy-known.json",
        ".jit/reference/rules-and-gates.md",
        "AGENTS.md",
        PROFILE_ASSET,
    ]
    .into_iter()
    .map(|path| (path, read(&harness, path)))
    .collect::<BTreeMap<_, _>>();

    let authored_rules = format!(
        "# authored header remains byte-exact\n{}",
        baseline[".jit/rules.toml"]
            .replacen(
                "severity = \"error\"",
                "severity = \"error\" # authored policy",
                1
            )
            .replacen(RULE_ASSERTION, STALE_RULE_ASSERTION, 1)
    );
    let expected_rules = authored_rules.replacen(STALE_RULE_ASSERTION, RULE_ASSERTION, 1);
    write(&harness, ".jit/rules.toml", &authored_rules);
    write(
        &harness,
        ".jit/schemas/default-type-hierarchy-known.json",
        &(baseline[".jit/schemas/default-type-hierarchy-known.json"].clone() + " stale"),
    );
    write(
        &harness,
        ".jit/reference/rules-and-gates.md",
        "STALE FULL PROJECTION\n",
    );
    write(
        &harness,
        "AGENTS.md",
        &format!(
            "AUTHORED PREFIX\n{}\nAUTHORED SUFFIX\n",
            baseline["AGENTS.md"]
                .replace("_No items declared._", "STALE CONFIGURED REGION")
                .replace("## JIT workflow", "## STALE PROFILE REGION")
        ),
    );
    write(&harness, PROFILE_ASSET, "STALE PROFILE ASSET\n");
    write(
        &harness,
        ".jit/schemas/default-convention-only.json",
        "authored and unowned\n",
    );

    let first = format!("{:#}", harness.executor.validate_silent().unwrap_err());
    let second = format!("{:#}", harness.executor.validate_silent().unwrap_err());
    assert_eq!(first, second, "drift diagnosis must be deterministic");
    for path in [
        "rules.toml",
        "default-type-hierarchy-known.json",
        "rules-and-gates.md",
        "AGENTS.md",
        PROFILE_ASSET,
    ] {
        assert!(first.contains(path), "missing drift path {path}: {first}");
    }
    assert!(!first.contains("authored:*"));

    assert!(harness.executor.validate_with_fix(true, false).unwrap().0 >= 5);
    assert_eq!(read(&harness, ".jit/rules.toml"), expected_rules);
    assert_eq!(
        read(&harness, ".jit/schemas/default-type-hierarchy-known.json"),
        baseline[".jit/schemas/default-type-hierarchy-known.json"]
    );
    assert_eq!(
        read(&harness, ".jit/reference/rules-and-gates.md"),
        baseline[".jit/reference/rules-and-gates.md"]
    );
    assert_eq!(
        read(&harness, "AGENTS.md"),
        format!(
            "AUTHORED PREFIX\n{}\nAUTHORED SUFFIX\n",
            baseline["AGENTS.md"]
        )
    );
    assert_eq!(read(&harness, PROFILE_ASSET), baseline[PROFILE_ASSET]);
    assert_eq!(
        read(&harness, ".jit/schemas/default-convention-only.json"),
        "authored and unowned\n"
    );
    assert_eq!(
        harness.executor.validate_with_fix(true, false).unwrap().0,
        0
    );
}

#[test]
fn test_harness_validate_fix_repairs_mode_only_profile_drift() {
    let mut harness = profiled_harness(false);
    let executable = jit::profile::jit_dogfood_package()
        .unwrap()
        .manifest()
        .assets
        .iter()
        .filter(|asset| asset.executable)
        .map(|asset| asset.target.clone())
        .collect::<Vec<_>>();
    let baseline = executable
        .iter()
        .map(|path| (path.clone(), read(&harness, path)))
        .collect::<BTreeMap<_, _>>();
    let before = capture(&harness, executable.clone());
    for path in &executable {
        let virtual_path = harness
            .storage
            .repository_layout()
            .classify_repository_relative(path)
            .unwrap();
        assert!(matches!(
            before.get(&virtual_path),
            Some(RepositoryEntry::File {
                mode: FileMode::Regular,
                ..
            })
        ));
    }
    let diagnosis = format!("{:#}", harness.executor.validate_silent().unwrap_err());
    assert!(diagnosis.contains(EXECUTABLE_ASSET));

    assert_eq!(
        harness.executor.validate_with_fix(true, false).unwrap().0,
        executable.len()
    );
    let repaired = capture(&harness, executable.clone());
    for path in &executable {
        let virtual_path = harness
            .storage
            .repository_layout()
            .classify_repository_relative(path)
            .unwrap();
        assert_eq!(read(&harness, path), baseline[path]);
        assert!(matches!(
            repaired.get(&virtual_path),
            Some(RepositoryEntry::File {
                mode: FileMode::Executable,
                ..
            })
        ));
    }
    assert_eq!(
        harness.executor.validate_with_fix(true, false).unwrap().0,
        0
    );
}

#[test]
fn test_harness_validate_fix_rejects_ambiguous_ownership_transactionally() {
    let mut harness = profiled_harness(false);
    write(
        &harness,
        ".jit/rules.toml",
        &read(&harness, ".jit/rules.toml").replacen(RULE_ASSERTION, STALE_RULE_ASSERTION, 1),
    );
    write(
        &harness,
        "AGENTS.md",
        &read(&harness, "AGENTS.md").replacen(
            "<!-- jit:invariants:begin -->",
            "<!-- jit:invariants:begin -->\n<!-- jit:invariants:begin -->",
            1,
        ),
    );
    let before = capture(&harness, repair_paths());

    let error = harness.executor.validate_with_fix(true, false).unwrap_err();
    assert!(format!("{error:#}").contains("invariants"));
    assert_eq!(capture(&harness, repair_paths()), before);
}

#[test]
fn test_harness_validate_fix_profile_provenance_failures_are_zero_write() {
    for mismatch in [true, false] {
        let mut harness = if mismatch {
            profiled_harness(false)
        } else {
            profiled_harness(true)
        };
        write(&harness, PROFILE_ASSET, "STALE PROFILE ASSET\n");
        if mismatch {
            let mut record: serde_json::Value =
                serde_json::from_str(&read(&harness, PROFILE_RECORD)).unwrap();
            record["package_hash"] = serde_json::Value::String("0".repeat(64));
            write(
                &harness,
                PROFILE_RECORD,
                &serde_json::to_string_pretty(&record).unwrap(),
            );
        }
        let before = capture(&harness, repair_paths());

        let result = harness.executor.validate_with_fix(true, false);
        if mismatch {
            assert!(format!("{:#}", result.unwrap_err()).contains("does not match"));
        } else {
            assert_eq!(result.unwrap().0, 0);
        }
        assert_eq!(capture(&harness, repair_paths()), before);
    }
}
