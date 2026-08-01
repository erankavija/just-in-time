use crate::harness::TestHarness;
use jit::repository_state::{
    assemble_config, repair_target_paths, validate_capture_closure, CaptureBudget, CaptureSpec,
    FileMode, RepositoryDeclarations, RepositoryEntry, RepositoryLayout, RepositoryRootClass,
    VirtualPath,
};
use jit::storage::{IssueStore, RepositoryStateStore};
use std::collections::BTreeMap;

const RULE_ASSERTION: &str = "assert = { json-schema = \"schemas/default-label-format.json\" }";
const STALE_RULE_ASSERTION: &str =
    "assert = { require-label = { label = \"authored:*\", min = 99 } }";
const PROFILE_ASSET: &str = ".agents/skills/jit-manage/SKILL.md";
const EXECUTABLE_ASSET: &str = "contrib/gates/ai-review.sh";
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

/// Render a captured `VirtualPath` back to the `write`/`read`-facing spelling
/// this file's fixtures use: `.jit/`-prefixed for the data root, bare for the
/// worktree.
fn repo_relative(path: &VirtualPath) -> String {
    let relative = path
        .relative()
        .as_path()
        .to_str()
        .expect("canonical paths are UTF-8")
        .to_string();
    match path.root_class() {
        RepositoryRootClass::Data => format!(".jit/{relative}"),
        RepositoryRootClass::Worktree => relative,
    }
}

/// Capture the complete image `repair_target_paths` needs from a jit-dogfood
/// repository: the engine registries, the validation-derived
/// projection/schema/kind closure those registries imply, and — because this
/// repository's applied-profile record names the profile — that profile's own
/// target paths plus the record itself. This is the exact closure production
/// repair captures for a repository carrying that record
/// (`CommandExecutor::capture_repair_plan`), assembled here from public
/// `repository_state` producers only.
fn repository_image(
    storage: &impl RepositoryStateStore,
    layout: &RepositoryLayout,
) -> jit::repository_state::RepositoryImage {
    let budget = CaptureBudget {
        max_paths: 512,
        max_listings: 16,
        max_bytes: 32 * 1024 * 1024,
        max_depth: 16,
    };
    let registries = [
        "config.toml",
        "invariants.toml",
        "rules.toml",
        "gates.toml",
        "templates.toml",
        "index.json",
        "events.jsonl",
    ]
    .into_iter()
    .map(|path| VirtualPath::data(path).unwrap())
    .collect::<Vec<_>>();
    let mut session = storage.open_mutation_session(layout.clone()).unwrap();
    let image_one = session
        .capture(CaptureSpec::phase_one(registries.clone(), budget).unwrap())
        .unwrap();

    let config = assemble_config(&image_one).unwrap();
    let rules_content = image_one
        .file_bytes(&VirtualPath::data("rules.toml").unwrap())
        .unwrap()
        .map(|bytes| std::str::from_utf8(bytes).unwrap().to_string());
    let closure = validate_capture_closure(layout, &config, &[], rules_content.as_deref()).unwrap();

    let mut spec = CaptureSpec::phase_one(registries, budget).unwrap();
    spec.discover_paths(closure.paths).unwrap();
    for listing in &closure.listings {
        spec.discover_listing(listing.clone()).unwrap();
    }
    let package = jit::profile::jit_dogfood_package().unwrap();
    spec.discover_paths(
        package
            .hashes()
            .targets
            .keys()
            .map(|path| layout.classify_repository_relative(path).unwrap()),
    )
    .unwrap();
    spec.discover_paths([layout.classify_repository_relative(PROFILE_RECORD).unwrap()])
        .unwrap();

    session.capture(spec).unwrap()
}

/// `repair_target_paths` computed over a freshly initialized, coherent
/// jit-dogfood repository, as `write`/`read`-facing repo-relative strings.
/// Computed once and cached: every call site needs the identical set, and this
/// repository is otherwise self-contained (does not depend on any test's own,
/// possibly already-drifted, harness).
///
/// The `profiles` argument passed to `repair_target_paths` here uses
/// `build_profile_claims` (install semantics), not the
/// `build_profile_repair_claims` production repair itself passes: a repair
/// claim carries no contributions at all, because a registry-merge target
/// (config.toml/gates.toml/templates.toml) is install-time only — a mismatched
/// value is an unresolvable conflict, never something repair rewrites (see
/// `crate::profile::apply_claims::build_claims`). Using install semantics here
/// means this fixture's target set additionally covers those registries, so
/// `profiled_harness` can seed a genuinely coherent repository; it does not
/// change what production repair itself would touch.
fn repair_target_path_strings() -> Vec<String> {
    static PATHS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    PATHS
        .get_or_init(|| {
            let source = tempfile::tempdir().unwrap();
            let storage = jit::storage::JsonFileStorage::new(source.path().join(".jit"));
            let layout =
                jit::storage::discover_repository_layout(source.path(), storage.root()).unwrap();
            jit::commands::CommandExecutor::new(storage.clone())
                .with_layout(layout.clone())
                .initialize_fresh_repository(
                    source.path(),
                    &jit::hierarchy_templates::HierarchyTemplate::default(),
                    Some("jit-dogfood"),
                )
                .unwrap();

            let image = repository_image(&storage, &layout);
            let record_path = layout.classify_repository_relative(PROFILE_RECORD).unwrap();
            let config_bytes = image
                .file_bytes(&VirtualPath::data("config.toml").unwrap())
                .unwrap()
                .unwrap();
            let configuration = jit::declarations::parse_configuration(config_bytes).unwrap();
            let gates = jit::declarations::GateRegistry::default();
            let rules = jit::declarations::rules::RuleSet::empty();
            let declarations = RepositoryDeclarations {
                configuration: &configuration,
                gates: &gates,
                rules: &rules,
            };
            let package = jit::profile::jit_dogfood_package().unwrap();
            let profiles = match image.entry(&record_path).unwrap() {
                RepositoryEntry::Absent => Vec::new(),
                _ => vec![jit::profile::build_profile_claims(&package, image.layout()).unwrap()],
            };

            let mut paths: Vec<String> = repair_target_paths(&image, declarations, profiles)
                .unwrap()
                .into_iter()
                .map(|path| repo_relative(&path))
                .collect();
            paths.sort();
            paths
        })
        .clone()
}

/// The complete fixture-seeding path set: [`repair_target_path_strings`] plus
/// the three explicit fixture-input paths repair reads but never writes
/// (`index.json` and `events.jsonl` are command-only ledgers; the
/// applied-profile record is provenance repair only reads, never a repair write
/// target).
fn repair_paths() -> Vec<String> {
    let mut paths = repair_target_path_strings();
    // Beyond the profile-owned targets, the harness repository needs the state
    // initialization owns and the profile never claims: the index, the event log,
    // and the invariant registry the configured `invariant` kind reads.
    paths.extend(
        [
            PROFILE_RECORD,
            ".jit/index.json",
            ".jit/events.jsonl",
            ".jit/invariants.toml",
        ]
        .into_iter()
        .map(str::to_string),
    );
    paths.sort();
    paths.dedup();
    paths
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
