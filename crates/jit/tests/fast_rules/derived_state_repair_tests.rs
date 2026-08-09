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

/// This repository's workflow package, assembled from its checkout, with the
/// directory holding the assembled tree.
///
/// What these fixtures need from it is what the checkout declares — the targets
/// an application owns and the executable bits it publishes — so the package is
/// assembled from the checkout rather than read from a copy of it.
fn shipped_workflow_package() -> (tempfile::TempDir, jit::profile::ProfilePackage) {
    jit::test_utils::temporary_repository_package("jit-dogfood")
}

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

/// A harness carrying the derived state of a profiled repository, with the
/// source checkout that repository was initialized in.
///
/// The harness is rooted at that checkout and the caller keeps it alive: the
/// applied-profile record names the worktree-relative directory its package was
/// read from, and repair reads the package back from there.
fn profiled_harness(omit_profile_record: bool) -> (tempfile::TempDir, TestHarness) {
    let source = tempfile::tempdir().unwrap();
    let source_storage = jit::storage::JsonFileStorage::new(source.path().join(".jit"));
    let source_layout =
        jit::storage::discover_repository_layout(source.path(), source_storage.root()).unwrap();
    let location = jit::test_utils::stage_repository_packages(source.path(), "jit-dogfood");
    jit::commands::CommandExecutor::new(source_storage)
        .with_layout(source_layout)
        .initialize_fresh_repository(
            source.path(),
            Some(&[jit::commands::ProfileSelector::path(&location)]),
        )
        .unwrap();

    let harness = TestHarness::rooted_at(source.path());
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
    (source, harness)
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
    let (_workspace, package) = shipped_workflow_package();
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
            let location = jit::test_utils::stage_repository_packages(source.path(), "jit-dogfood");
            jit::commands::CommandExecutor::new(storage.clone())
                .with_layout(layout.clone())
                .initialize_fresh_repository(
                    source.path(),
                    Some(&[jit::commands::ProfileSelector::path(&location)]),
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
            let (_workspace, package) = shipped_workflow_package();
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

/// The checked-in fixture tree every directory-recorded case below stages a
/// copy of inside the repository under test, which is the only place a fact
/// validation states about that package can have been read from.
fn directory_package() -> std::path::PathBuf {
    jit::test_utils::profile_package_fixture("planner-asset-only")
}

/// Worktree-relative directory the fixture package is written to and recorded
/// from.
const PACKAGE_DIRECTORY: &str = "packages/planner";

/// A filesystem repository holding the fixture package inside its own worktree.
///
/// The in-memory harness models a synthetic worktree root, so a record naming a
/// worktree-relative directory has nothing on disk to resolve against: reading
/// a package back from a recorded location is a property of a real repository
/// and is observed over one.
struct DirectoryPackageRepo {
    root: tempfile::TempDir,
    executor: jit::commands::CommandExecutor<jit::storage::JsonFileStorage>,
    package: jit::profile::ProfilePackage,
}

impl DirectoryPackageRepo {
    /// A repository whose applied-profile record names the package directory
    /// inside its worktree.
    fn applied() -> Self {
        let repo = Self::unapplied();
        repo.executor
            .initialize_profiled_repository(
                repo.root.path(),
                &[jit::commands::ProfileSelector::path(
                    repo.root.path().join(PACKAGE_DIRECTORY),
                )],
            )
            .expect("a package inside the worktree applies");
        repo
    }

    /// A repository holding the package tree and no applied-profile record.
    fn unapplied() -> Self {
        let root = tempfile::tempdir().expect("create repository root");
        let package =
            jit::profile::ProfilePackage::from_directory(&jit::test_utils::copy_package_tree(
                &directory_package(),
                &root.path().join(PACKAGE_DIRECTORY),
            ))
            .expect("a valid package tree");
        let storage = jit::storage::JsonFileStorage::new(root.path().join(".jit"));
        let layout = jit::storage::discover_repository_layout(root.path(), storage.root())
            .expect("a canonical repository layout");
        let executor = jit::commands::CommandExecutor::new(storage).with_layout(layout);
        Self {
            root,
            executor,
            package,
        }
    }

    /// The profile id the fixture package's own manifest declares.
    fn id(&self) -> &str {
        self.package.model().id.as_str()
    }

    /// The one target the fixture package owns, as an absolute path.
    fn target(&self) -> std::path::PathBuf {
        self.root.path().join(&self.asset().target)
    }

    /// The fixture package's single declared asset.
    fn asset(&self) -> &jit::profile::AssetDeclaration {
        self.package
            .model()
            .assets
            .first()
            .expect("the fixture package declares one asset")
    }

    /// The bytes the package at the recorded location publishes to its target.
    fn published_bytes(&self) -> String {
        std::fs::read_to_string(
            self.root
                .path()
                .join(PACKAGE_DIRECTORY)
                .join(&self.asset().source),
        )
        .expect("read the package's authored asset")
    }

    /// Rewrite the authored asset of the package at the recorded location, so
    /// the location holds a package the repository has never applied.
    fn rewrite_package(&mut self, bytes: &str) {
        let directory = self.root.path().join(PACKAGE_DIRECTORY);
        std::fs::write(directory.join(&self.asset().source), bytes)
            .expect("rewrite the package's authored asset");
        self.package =
            jit::profile::ProfilePackage::from_directory(&directory).expect("a valid package tree");
    }

    /// The applied-profile record path for `id`, as an absolute path.
    fn record_path(&self, id: &str) -> std::path::PathBuf {
        self.root.path().join(format!(".jit/profiles/{id}.json"))
    }

    /// Store the record the package now at the recorded location would write,
    /// so record and location agree on a package this repository never applied.
    fn record_the_current_package(&self) {
        let resolved =
            jit::profile::resolve_package(&self.package, &jit::profile::VariableInputs::default())
                .expect("the package resolves without variable inputs");
        let record = jit::repository_state::AppliedProfileRecord::new(
            self.id(),
            self.package.model().version.clone(),
            self.package.model().compatible_jit.clone(),
            jit::profile::ProfileOrigin::Directory(
                jit::repository_state::RootRelativePath::parse(PACKAGE_DIRECTORY)
                    .expect("a worktree-relative package location"),
            ),
            self.package.hashes().package.clone(),
            resolved.variables().clone(),
            std::collections::BTreeSet::new(),
        );
        std::fs::write(
            self.record_path(self.id()),
            record
                .to_bytes()
                .expect("encode the applied-profile record"),
        )
        .expect("store the applied-profile record");
    }

    /// Every repository byte, the package tree included, keyed by
    /// worktree-relative path.
    ///
    /// The event log, lock files, and the machine-local scratch area are working
    /// state rather than the repository content a refused repair is judged
    /// against.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        fn visit(
            root: &std::path::Path,
            current: &std::path::Path,
            entries: &mut BTreeMap<String, Vec<u8>>,
        ) {
            for child in std::fs::read_dir(current).expect("read repository directory") {
                let path = child.expect("read repository entry").path();
                if path.is_dir() {
                    visit(root, &path, entries);
                } else {
                    let relative = path
                        .strip_prefix(root)
                        .expect("repository-relative")
                        .to_string_lossy()
                        .replace('\\', "/");
                    entries.insert(relative, std::fs::read(&path).expect("read entry bytes"));
                }
            }
        }

        let mut entries = BTreeMap::new();
        visit(self.root.path(), self.root.path(), &mut entries);
        entries.retain(|path, _| {
            !path.starts_with(".jit/tmp/")
                && path != ".jit/events.jsonl"
                && !path.ends_with(".lock")
        });
        entries
    }
}

/// The repository-relative paths whose bytes differ between two snapshots.
fn changed_paths(
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

/// REQ-01: the record names a location that resolves, so repair recomputes the
/// expected record from the package read there and restores what that package
/// owns — deleted or edited.
#[test]
fn test_validate_fix_restores_a_deleted_or_edited_target_from_the_recorded_location() {
    for drift in ["deleted", "edited"] {
        let mut repo = DirectoryPackageRepo::applied();
        let published = repo.published_bytes();
        assert_eq!(
            std::fs::read_to_string(repo.target()).unwrap(),
            published,
            "application publishes the package's bytes"
        );
        if drift == "deleted" {
            std::fs::remove_file(repo.target()).unwrap();
        } else {
            std::fs::write(repo.target(), "AUTHORED OVER A PROFILE TARGET\n").unwrap();
        }
        assert!(
            repo.executor.validate_silent().is_err(),
            "{drift} profile-owned drift is a validation failure"
        );

        let (fixes, _messages) = repo.executor.validate_with_fix(true, false).unwrap();

        assert!(fixes > 0, "{drift} profile-owned drift is repairable");
        assert_eq!(
            std::fs::read_to_string(repo.target()).unwrap(),
            published,
            "{drift} target is restored from the package the record names"
        );
        repo.executor.validate_silent().unwrap();
    }
}

/// REQ-03: the expected targets are recomputed from the package read at the
/// recorded location, never derived from the digests the record stores.
///
/// The package at that location is rewritten and the record moved on with it,
/// so the stored digests describe the rewritten package exactly as well as they
/// described the applied one. Only the location holds the bytes that tell them
/// apart, and repair restoring those bytes is a live read of it — a repair that
/// derived its expectations from the record could not produce them.
#[test]
fn test_validate_fix_restores_the_bytes_the_recorded_location_holds_now() {
    let mut repo = DirectoryPackageRepo::applied();
    let originally_applied = repo.published_bytes();
    repo.rewrite_package("REWRITTEN PACKAGE ASSET\n");
    let rewritten = repo.published_bytes();
    assert_ne!(rewritten, originally_applied);
    repo.record_the_current_package();
    std::fs::remove_file(repo.target()).unwrap();

    repo.executor.validate_with_fix(true, false).unwrap();

    assert_eq!(
        std::fs::read_to_string(repo.target()).unwrap(),
        rewritten,
        "repair restores what the recorded location holds now"
    );
    repo.executor
        .validate_silent()
        .expect("the restored repository agrees with the package at its recorded location");
}

/// REQ-05: a stored record that disagrees with the package read from the
/// recorded location is a validation failure, not a repair.
///
/// The record is untouched here and the package alone moves on, so a repair
/// derived from the record's own digests would find nothing wrong. The failure
/// is therefore the recomputation, observed.
#[test]
fn test_validate_fails_when_the_recorded_location_disagrees_with_the_stored_record() {
    let mut repo = DirectoryPackageRepo::applied();
    repo.executor
        .validate_silent()
        .expect("applied state is coherent");
    repo.rewrite_package("A PACKAGE THIS REPOSITORY NEVER APPLIED\n");
    // Drift a profile-owned target too, so the refusal is observed against a
    // repository repair would otherwise change: the sibling REQ-01 case proves
    // this same drift is repairable when record and location agree.
    std::fs::remove_file(repo.target()).unwrap();
    let before = repo.snapshot();

    let diagnosis = format!("{:#}", repo.executor.validate_silent().unwrap_err());
    let repair = repo.executor.validate_with_fix(true, false);

    assert!(
        diagnosis.contains("does not match") && diagnosis.contains(repo.id()),
        "the disagreement must be reported over the recorded profile: {diagnosis}"
    );
    assert!(
        format!("{:#}", repair.unwrap_err()).contains("does not match"),
        "disagreement is a failure rather than a repair"
    );
    assert!(
        changed_paths(&before, &repo.snapshot()).is_empty(),
        "a repair that cannot account for the recorded package writes nothing: {:?}",
        changed_paths(&before, &repo.snapshot())
    );
}

/// REQ-02: the recorded location no longer resolves, so validation fails naming
/// the record and the path.
#[test]
fn test_validate_fails_when_the_recorded_location_no_longer_resolves() {
    let mut repo = DirectoryPackageRepo::applied();
    std::fs::remove_dir_all(repo.root.path().join(PACKAGE_DIRECTORY)).unwrap();
    // As above: a profile-owned target repair would restore had the location
    // resolved, so refusing to restore it is what the snapshot observes.
    std::fs::remove_file(repo.target()).unwrap();
    let before = repo.snapshot();

    let diagnosis = format!("{:#}", repo.executor.validate_silent().unwrap_err());
    let repair = repo.executor.validate_with_fix(true, false);

    for named in [
        format!(".jit/profiles/{}.json", repo.id()),
        PACKAGE_DIRECTORY.to_string(),
    ] {
        assert!(
            diagnosis.contains(&named),
            "an unresolvable location names {named}: {diagnosis}"
        );
    }
    assert!(format!("{:#}", repair.unwrap_err()).contains(PACKAGE_DIRECTORY));
    assert!(
        changed_paths(&before, &repo.snapshot()).is_empty(),
        "a repair that cannot obtain a recorded package writes nothing: {:?}",
        changed_paths(&before, &repo.snapshot())
    );
}

/// A record whose body declares a profile its own name does not is refused
/// before anything resolves.
///
/// A record states its profile twice — in the name application publishes it
/// under, and in the id inside it — and a repository that trusted whichever it
/// read first would repair one profile's targets while filing the provenance
/// for them under another profile's name. This is REQ-05's principle applied to
/// a record disagreeing with itself rather than with its package.
#[test]
fn test_validate_fails_when_a_record_declares_a_profile_its_name_does_not() {
    const IMPOSTOR: &str = "impostor";

    let mut repo = DirectoryPackageRepo::applied();
    let declared = repo.id().to_string();
    std::fs::rename(repo.record_path(&declared), repo.record_path(IMPOSTOR)).unwrap();
    // A profile-owned target repair would restore were the record trusted, so
    // refusing to restore it is what the snapshot observes; the REQ-01 case
    // proves this same drift is repairable from a record that agrees.
    std::fs::remove_file(repo.target()).unwrap();
    let before = repo.snapshot();

    let diagnosis = format!("{:#}", repo.executor.validate_silent().unwrap_err());
    let repair = repo.executor.validate_with_fix(true, false);

    for named in [
        format!(".jit/profiles/{IMPOSTOR}.json"),
        IMPOSTOR.to_string(),
        declared,
    ] {
        assert!(
            diagnosis.contains(&named),
            "a record disagreeing with its own name must name {named}: {diagnosis}"
        );
    }
    assert!(format!("{:#}", repair.unwrap_err()).contains(IMPOSTOR));
    assert!(
        changed_paths(&before, &repo.snapshot()).is_empty(),
        "a record that does not state which profile it records repairs nothing: {:?}",
        changed_paths(&before, &repo.snapshot())
    );
}

/// REQ-04: with no applied-profile record, a repository reads no package and
/// reports nothing about profiles — even while a readable package sits in its
/// worktree.
#[test]
fn test_validate_fix_without_an_applied_record_reads_no_package() {
    let mut repo = DirectoryPackageRepo::unapplied();
    repo.executor
        .initialize_fresh_repository(repo.root.path(), None)
        .unwrap();
    assert!(!repo.root.path().join(".jit/profiles").exists());
    let target = repo.target();
    assert!(
        !target.exists(),
        "an unapplied package owns nothing in this repository"
    );

    repo.executor.validate_silent().unwrap();
    let (_fixes, messages) = repo.executor.validate_with_fix(true, false).unwrap();

    assert!(
        !target.exists(),
        "no record names the package, so nothing claims its target"
    );
    let reported = messages.join("\n");
    assert!(
        !reported.contains(&repo.asset().target) && !reported.contains(repo.id()),
        "a repository with no record reports nothing about profiles: {reported}"
    );
}

#[test]
fn test_harness_validate_fix_repairs_each_owned_class_and_preserves_authored_bytes() {
    let (_source, mut harness) = profiled_harness(false);
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
    let (_source, mut harness) = profiled_harness(false);
    let (_workspace, package) = shipped_workflow_package();
    let executable = package
        .model()
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
    let (_source, mut harness) = profiled_harness(false);
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
        let (_source, mut harness) = if mismatch {
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
