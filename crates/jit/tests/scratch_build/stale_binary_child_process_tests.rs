//! REQ-02 regression for jit:7446af34: a checker SCRIPT that itself shells
//! out to `jit` (e.g. `scripts/jit-validate.sh`'s `exec jit validate "$@"`)
//! resolves that `jit` from `PATH`, independently of the evaluator process.
//! The evaluator's own `check_gate` guard (REQ-01) only covers the
//! evaluator's OWN binary — without a self-check inside the checker's
//! process tree too, a stale PATH `jit` spawned BY the checker could still
//! silently produce the verdict the evaluator persists. This is the exact
//! incident that motivated the whole feature (jit-validate.sh reported a
//! real fix as "not found" using a stale binary, and `code-review` treated
//! that as a genuine finding).
//!
//! `gate_execution::execute_gate_checker_with_context` sets `JIT_GATE_RUN=1`
//! on every checker's environment (inherited by anything the checker spawns);
//! `main`'s startup dispatch self-checks whenever that variable is present.
//!
//! Reproducing "evaluator fresh, child stale" genuinely needs TWO different
//! binaries — the evaluator's own build commit is fixed at compile time, so a
//! single binary cannot be both fresh (as evaluator) and stale (as child)
//! against the same repository. This file builds a second `jit` binary via
//! `build.rs`'s `JIT_BUILD_GIT_HASH`/`JIT_BUILD_GIT_DIRTY` override env vars
//! (an existing, intentional escape hatch — see `crates/jit/build.rs`), so
//! the "stale child" is compiled from the SAME source tree, just told it was
//! built from an older, real commit. The first process builds under
//! `target/jit-stale-child-test-cache`; later processes verify and reuse its
//! provenance-keyed artifact without invoking Cargo. It is never written to
//! the shared `target/debug/jit` path that `CARGO_BIN_EXE_jit` and every other
//! test rely on.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};
use tempfile::TempDir;

use jit::storage::FileLocker;

const STALE_CHILD_CACHE: &str = "jit-stale-child-test-cache";
const FIXTURE_MARKER_VERSION: u32 = 5;
const FIXTURE_LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const PINNED_NEXTEST_VERSION: &str = "0.9.133";
const SETUP_MODE: &str = "JIT_STALE_FIXTURE_SETUP";
const SETUP_RECEIPT: &str = "JIT_STALE_FIXTURE_RECEIPT";
const SETUP_RECEIPT_SHA256: &str = "JIT_STALE_FIXTURE_RECEIPT_SHA256";
const REUSE_OBSERVATION_DIR: &str = "JIT_STALE_FIXTURE_REUSE_OBSERVATION_DIR";
const NEXTEST_SEMANTIC_TESTS: [&str; 6] = [
    "stale_binary_child_process_tests::test_checker_child_stale_binary_fails_gate_run_visibly",
    "stale_binary_child_process_tests::test_checker_child_metadata_only_change_does_not_refuse",
    "stale_binary_child_process_tests::test_non_gate_context_child_invocation_stays_unchecked",
    "stale_binary_child_process_tests::test_gate_context_early_paths_refuse_stale_binary",
    "stale_binary_json_exit_tests::test_gate_evaluate_stale_binary_exits_10_in_text_and_json_modes",
    "stale_binary_json_exit_tests::test_gate_evaluate_metadata_only_commit_does_not_refuse_stale_binary",
];

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct VerifiedArtifactMarker {
    version: u32,
    source_sha256: String,
    built_from: String,
    short_commit: String,
    artifact_sha256: String,
    cargo_build_invocations: u32,
    reuse_observations: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FixtureUse {
    Built,
    Reused,
    PreparedReuse,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ScratchShape {
    BuildInputChange,
    MetadataOnly,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ScopedFixtureObservation {
    test: String,
    outcome: FixtureUse,
    source_sha256: String,
    built_from: String,
    cargo_build_invocations: u32,
    scratch_shape: Option<ScratchShape>,
    scratch_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PreparedFixtureReceipt {
    version: u32,
    nextest_run_id: String,
    artifact: PathBuf,
    marker: VerifiedArtifactMarker,
    source_fingerprint: FileFingerprint,
    artifact_fingerprint: FileFingerprint,
    build_input_baseline: PathBuf,
    metadata_baseline: PathBuf,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FileFingerprint {
    length: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

pub(super) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

/// Resolve a real, well-in-the-past commit in the workspace's own history
/// (`HEAD~8`), used as the "stale child" binary's fake build commit. `None`
/// when the workspace has fewer than 9 commits or git is unavailable.
pub(super) fn ancestor_commit(workspace_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD~8"])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Return the verified stale-child artifact shared by all six nested-build
/// tests. The outer test binary's `jit` executable fingerprints the Cargo
/// inputs that produced this test run, while `ancestor` fingerprints the
/// provenance injected into the child. Together they form the cache key.
///
/// Every process takes the advisory lock before inspecting the marker. A valid
/// marker proves the keyed artifact's bytes and reported provenance, so that
/// process records a reuse without invoking Cargo. The first process builds,
/// verifies, and publishes both artifact and marker while still holding the
/// lock. Returns `None` only when the nested Cargo command cannot run or fails.
pub(super) fn build_stale_child_binary(workspace_root: &Path, ancestor: &str) -> Option<PathBuf> {
    if std::env::var_os("NEXTEST").is_some() && std::env::var_os(SETUP_MODE).is_none() {
        return Some(consume_prepared_stale_child(workspace_root, ancestor));
    }
    let target_dir = workspace_root.join("target").join(STALE_CHILD_CACHE);
    build_stale_child_binary_with_runner(
        workspace_root,
        ancestor,
        &target_dir,
        run_nested_cargo,
        binary_reports_provenance,
    )
}

fn consume_prepared_stale_child(workspace_root: &Path, ancestor: &str) -> PathBuf {
    let receipt = read_prepared_receipt()
        .unwrap_or_else(|error| panic!("nextest stale-binary fixture setup is invalid: {error}"));
    let run_id = std::env::var("NEXTEST_RUN_ID")
        .expect("pinned nextest should identify the setup/test invocation");
    assert_eq!(
        receipt.nextest_run_id, run_id,
        "stale-binary fixture receipt must belong to this nextest invocation"
    );
    assert_eq!(receipt.marker.version, FIXTURE_MARKER_VERSION);
    assert_eq!(receipt.marker.built_from, ancestor);
    assert_eq!(receipt.marker.cargo_build_invocations, 1);
    assert_eq!(
        regular_file_fingerprint(Path::new(jit_binary()))
            .expect("the outer jit binary should remain an ordinary file"),
        receipt.source_fingerprint,
        "the source binary verified by setup must remain unchanged"
    );
    assert!(
        receipt
            .artifact
            .starts_with(workspace_root.join("target").join(STALE_CHILD_CACHE)),
        "prepared artifact must remain inside the shared stale-child cache: {}",
        receipt.artifact.display()
    );
    assert_eq!(
        regular_file_fingerprint(&receipt.artifact)
            .expect("the prepared stale child should remain an ordinary file"),
        receipt.artifact_fingerprint,
        "the stale child verified by setup must remain unchanged"
    );
    let marker_path = receipt
        .artifact
        .parent()
        .expect("prepared artifact should have a cache directory")
        .join("verified-artifact.json");
    assert_eq!(
        read_marker(&marker_path).as_ref(),
        Some(&receipt.marker),
        "the setup-verified marker must remain unchanged"
    );

    record_scoped_fixture_observation(&receipt.marker, FixtureUse::PreparedReuse);
    receipt.artifact
}

fn read_prepared_receipt() -> Result<PreparedFixtureReceipt, String> {
    let path = std::env::var_os(SETUP_RECEIPT)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{SETUP_RECEIPT} is not set"))?;
    regular_file_fingerprint(&path).map_err(|error| {
        format!(
            "receipt {} is not an ordinary file: {error}",
            path.display()
        )
    })?;
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let expected_sha256 = std::env::var(SETUP_RECEIPT_SHA256)
        .map_err(|_| format!("{SETUP_RECEIPT_SHA256} is not set"))?;
    if sha256_bytes(&bytes) != expected_sha256 {
        return Err(format!(
            "receipt digest does not match setup output: {}",
            path.display()
        ));
    }
    let receipt: PreparedFixtureReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    (receipt.version == FIXTURE_MARKER_VERSION)
        .then_some(receipt)
        .ok_or_else(|| format!("unsupported receipt version in {}", path.display()))
}

fn prepare_nextest_fixture() {
    assert_eq!(std::env::var("NEXTEST").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("NEXTEST_VERSION").as_deref(),
        Ok(PINNED_NEXTEST_VERSION),
        "the setup helper must run under the pinned nextest reporter contract"
    );
    let run_id = std::env::var("NEXTEST_RUN_ID")
        .expect("nextest setup should expose one invocation identity");
    let nextest_env = PathBuf::from(
        std::env::var_os("NEXTEST_ENV")
            .expect("nextest setup should expose its test-environment output file"),
    );
    let workspace_root = workspace_root();
    let ancestor = ancestor_commit(&workspace_root)
        .expect("workspace should have enough history for the stale-binary fixture");
    let artifact = build_stale_child_binary(&workspace_root, &ancestor)
        .expect("nextest setup should prepare one verified stale child");
    let artifact_dir = artifact
        .parent()
        .expect("prepared artifact should have a provenance-keyed directory");
    let marker = read_marker(&artifact_dir.join("verified-artifact.json"))
        .expect("prepared artifact should retain its verified marker");
    assert_eq!(marker.cargo_build_invocations, 1);
    assert_eq!(
        sha256_file(Path::new(jit_binary())).expect("setup source binary should hash"),
        marker.source_sha256,
        "the setup cache key must describe the current outer jit binary"
    );
    assert_eq!(
        sha256_file(&artifact).expect("setup artifact should hash"),
        marker.artifact_sha256,
        "the setup receipt must describe the published artifact bytes"
    );
    assert!(binary_reports_provenance(
        &artifact,
        &ancestor,
        &marker.short_commit
    ));

    let target_dir = workspace_root.join("target").join(STALE_CHILD_CACHE);
    let _lock = FileLocker::new(FIXTURE_LOCK_TIMEOUT)
        .lock_exclusive(&target_dir.join("fixture.lock"))
        .expect("stale-child setup lock should be acquirable");
    let build_input_baseline = prepare_scratch_baseline(
        &workspace_root,
        &target_dir,
        &ancestor,
        ScratchShape::BuildInputChange,
    );
    let metadata_baseline = prepare_scratch_baseline(
        &workspace_root,
        &target_dir,
        &ancestor,
        ScratchShape::MetadataOnly,
    );

    let observation_dir = match std::env::var_os(REUSE_OBSERVATION_DIR) {
        Some(path) => {
            let path = PathBuf::from(path);
            assert_eq!(
                fs::read_dir(&path)
                    .expect("injected observation directory should be readable")
                    .count(),
                0,
                "injected observation directory must start empty"
            );
            path
        }
        None => {
            let parent = target_dir.join("observations");
            fs::create_dir_all(&parent)
                .expect("stale fixture observation parent should be creatable");
            let path = parent.join(&run_id);
            fs::create_dir(&path)
                .expect("nextest run observation directory should publish without replacement");
            path
        }
    };

    let receipts = target_dir.join("receipts");
    fs::create_dir_all(&receipts).expect("stale fixture receipt directory should be creatable");
    let receipt_path = receipts.join(format!("{run_id}.json"));
    let receipt = PreparedFixtureReceipt {
        version: FIXTURE_MARKER_VERSION,
        nextest_run_id: run_id,
        source_fingerprint: regular_file_fingerprint(Path::new(jit_binary()))
            .expect("setup source binary should remain an ordinary file"),
        artifact_fingerprint: regular_file_fingerprint(&artifact)
            .expect("setup artifact should remain an ordinary file"),
        artifact,
        marker,
        build_input_baseline,
        metadata_baseline,
    };
    let receipt_bytes =
        serde_json::to_vec_pretty(&receipt).expect("fixture receipt should serialize");
    let receipt_sha256 = sha256_bytes(&receipt_bytes);
    let staging = receipts.join(format!(
        ".{}.json.tmp.{}",
        receipt.nextest_run_id,
        std::process::id()
    ));
    fs::write(&staging, &receipt_bytes).expect("fixture receipt staging should be writable");
    assert_eq!(
        serde_json::from_reader::<_, PreparedFixtureReceipt>(
            File::open(&staging).expect("fixture receipt staging should open")
        )
        .expect("fixture receipt staging should remain valid JSON"),
        receipt
    );
    publish_staged_file_noreplace(&staging, &receipt_path)
        .expect("fixture receipt should publish atomically without replacement");

    let setup_observation = observation_dir.join("setup-receipt.json");
    let setup_staging =
        observation_dir.join(format!(".setup-receipt.json.tmp.{}", std::process::id()));
    fs::write(&setup_staging, &receipt_bytes)
        .expect("setup observation staging should be writable");
    publish_staged_file_noreplace(&setup_staging, &setup_observation)
        .expect("setup observation should publish without replacement");

    append_nextest_env(&nextest_env, SETUP_RECEIPT, &receipt_path);
    append_nextest_env_value(&nextest_env, SETUP_RECEIPT_SHA256, &receipt_sha256);
    append_nextest_env(
        &nextest_env,
        "JIT_STALE_FIXTURE_BUILT_FROM",
        Path::new(&receipt.marker.built_from),
    );
    append_nextest_env(&nextest_env, REUSE_OBSERVATION_DIR, &observation_dir);
}

fn append_nextest_env(path: &Path, key: &str, value: &Path) {
    append_nextest_env_value(path, key, &value.to_string_lossy());
}

fn append_nextest_env_value(path: &Path, key: &str, value: &str) {
    use std::io::Write;

    assert!(
        !value.contains('\n') && !value.contains('\r'),
        "nextest environment values must fit on one line"
    );
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("nextest environment output should be appendable");
    writeln!(file, "{key}={value}").expect("nextest environment output should be writable");
}

fn scratch_shape_change(shape: ScratchShape) -> (&'static str, &'static [u8]) {
    match shape {
        ScratchShape::BuildInputChange => (
            "crates/jit/src/main.rs",
            b"\n// build-input change for stale-binary coverage\n",
        ),
        ScratchShape::MetadataOnly => (
            "docs/stale-binary-metadata.md",
            b"metadata-only change for stale-binary coverage\n",
        ),
    }
}

fn prepare_scratch_baseline(
    workspace_root: &Path,
    target_dir: &Path,
    ancestor: &str,
    shape: ScratchShape,
) -> PathBuf {
    let repository = TempDir::new_in(target_dir)
        .expect("scratch baseline repository staging should be creatable");
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(repository.path())
            .status()
            .is_ok_and(|status| status.success())
    };
    let (changed_path, change) = scratch_shape_change(shape);
    let cloned = Command::new("git")
        .args(["clone", "--shared", "--no-checkout", "-q"])
        .arg(workspace_root)
        .arg(repository.path())
        .status()
        .expect("shared scratch baseline clone should launch");
    assert!(
        cloned.success(),
        "shared scratch baseline clone should succeed"
    );
    assert!(run(&["sparse-checkout", "set", "--no-cone", changed_path]));
    assert!(run(&["checkout", "-q", "-b", "fixture", ancestor]));
    let path = repository.path().join(changed_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("scratch shape parent should be creatable");
    }
    fs::write(&path, change).expect("scratch shape change should be writable");
    assert!(run(&["add", changed_path]));
    assert!(run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "-m",
        "advance past the build commit",
    ]));
    remove_inherited_git_topology(repository.path());
    assert!(verify_scratch_repository(
        repository.path(),
        ancestor,
        shape
    ));
    repository.keep()
}

fn verify_scratch_repository(repository: &Path, ancestor: &str, shape: ScratchShape) -> bool {
    if !fs::symlink_metadata(repository).is_ok_and(|metadata| metadata.file_type().is_dir())
        || !fs::symlink_metadata(repository.join(".git"))
            .is_ok_and(|metadata| metadata.file_type().is_dir())
    {
        return false;
    }
    let topology_is_isolated = git_refs(repository, "refs/heads")
        .is_some_and(|refs| refs == ["refs/heads/fixture"])
        && git_refs(repository, "refs/remotes").is_some_and(|refs| refs.is_empty());
    let parent = Command::new("git")
        .args(["rev-parse", "HEAD^"])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    let changed = Command::new("git")
        .args(["diff", "--name-only", "HEAD^", "HEAD"])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    let clean = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| output.stdout.is_empty());
    let (changed_path, change) = scratch_shape_change(shape);
    topology_is_isolated
        && parent.as_deref() == Some(ancestor)
        && changed.as_deref() == Some(changed_path)
        && fs::read(repository.join(changed_path)).ok().as_deref() == Some(change)
        && clean
}

fn remove_inherited_git_topology(repository: &Path) {
    let remote = Command::new("git")
        .args(["remote", "remove", "origin"])
        .current_dir(repository)
        .status()
        .expect("prepared scratch origin removal should launch");
    assert!(
        remote.success(),
        "prepared scratch origin should be removable"
    );

    git_refs(repository, "refs/heads")
        .expect("prepared scratch local heads should be readable")
        .into_iter()
        .filter(|reference| reference != "refs/heads/fixture")
        .for_each(|reference| {
            let deleted = Command::new("git")
                .args(["update-ref", "-d", &reference])
                .current_dir(repository)
                .status()
                .expect("inherited scratch head removal should launch");
            assert!(
                deleted.success(),
                "inherited scratch head should be removable: {reference}"
            );
        });
}

fn git_refs(repository: &Path, namespace: &str) -> Option<Vec<String>> {
    Command::new("git")
        .args(["for-each-ref", "--format=%(refname)", namespace])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
}

fn assert_main_branch_source_does_not_leak_into_prepared_scratch() {
    let source = TempDir::new().expect("main-branch regression needs a source repository");
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(source.path())
            .status()
            .is_ok_and(|status| status.success())
    };
    assert!(run(&["init", "-q", "-b", "main"]));
    let changed_path = source.path().join("crates/jit/src/main.rs");
    fs::create_dir_all(changed_path.parent().unwrap()).unwrap();
    fs::write(&changed_path, b"fn main() {}\n").unwrap();
    assert!(run(&["add", "."]));
    assert!(run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "-m",
        "fixture ancestor",
    ]));
    let ancestor = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(source.path())
        .output()
        .expect("fixture ancestor should resolve");
    assert!(ancestor.status.success());
    let ancestor = String::from_utf8_lossy(&ancestor.stdout).trim().to_owned();
    fs::write(
        source.path().join("main-only.txt"),
        b"integrated main head\n",
    )
    .unwrap();
    assert!(run(&["add", "."]));
    assert!(run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "-m",
        "main advances past fixture ancestor",
    ]));

    let target = TempDir::new().expect("main-branch regression needs a target directory");
    let baseline = prepare_scratch_baseline(
        source.path(),
        target.path(),
        &ancestor,
        ScratchShape::BuildInputChange,
    );
    assert_eq!(
        git_refs(&baseline, "refs/heads").unwrap(),
        vec!["refs/heads/fixture"],
        "a source main head must not survive in the prepared baseline"
    );
    assert!(
        git_refs(&baseline, "refs/remotes").unwrap().is_empty(),
        "prepared baseline must not retain source remote-tracking refs"
    );
    let consumer = clone_scratch_baseline(&baseline, &ancestor, ScratchShape::BuildInputChange);
    assert!(
        git_refs(consumer.path(), "refs/remotes")
            .unwrap()
            .is_empty(),
        "a prepared consumer must not synthesize origin/main"
    );
    let mut init = Command::new(jit_binary());
    init.current_dir(consumer.path())
        .arg("init")
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY");
    let output = init
        .output()
        .expect("real jit init should launch in the prepared main-branch consumer");
    assert!(
        output.status.success(),
        "isolated prepared consumer must admit real jit init: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn build_stale_child_binary_with_runner<RunCargo, VerifyBinary>(
    workspace_root: &Path,
    ancestor: &str,
    target_dir: &Path,
    run_cargo: RunCargo,
    verify_binary: VerifyBinary,
) -> Option<PathBuf>
where
    RunCargo: FnOnce(&Path, &Path, &str, &str) -> Option<PathBuf>,
    VerifyBinary: Fn(&Path, &str, &str) -> bool,
{
    let short = Command::new("git")
        .args(["rev-parse", "--short=8", ancestor])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    let source_sha256 = sha256_file(Path::new(jit_binary()))
        .expect("the Cargo-built outer jit binary should be hashable");
    let cache_key = fixture_cache_key(ancestor, &source_sha256);
    fs::create_dir_all(target_dir).expect("stale-child cache directory should be creatable");
    let _lock = FileLocker::new(FIXTURE_LOCK_TIMEOUT)
        .lock_exclusive(&target_dir.join("fixture.lock"))
        .expect("stale-child fixture lock should be acquirable");

    let artifact_dir = target_dir.join("artifacts").join(cache_key);
    let artifact = artifact_dir.join(format!("jit{}", std::env::consts::EXE_SUFFIX));
    let marker_path = artifact_dir.join("verified-artifact.json");
    let build_count_path = artifact_dir.join("cargo-build-invocations");

    if let Some(mut marker) = read_verified_marker(
        &marker_path,
        &build_count_path,
        &artifact,
        &source_sha256,
        ancestor,
        &short,
    ) {
        marker.reuse_observations = marker
            .reuse_observations
            .checked_add(1)
            .expect("stale-child fixture reuse count should remain representable");
        let expected_reuse_observations = marker.reuse_observations;
        write_marker(&marker_path, &marker);
        let observed = read_marker(&marker_path)
            .expect("the reuse observation should read back from its marker");
        assert_eq!(observed.cargo_build_invocations, 1);
        assert_eq!(observed.reuse_observations, expected_reuse_observations);
        record_scoped_fixture_observation(&marker, FixtureUse::Reused);
        return Some(artifact);
    }

    assert!(
        !artifact.exists() && !marker_path.exists() && !build_count_path.exists(),
        "invalid occupied stale-child cache must fail without deleting or rebuilding: {}",
        artifact_dir.display()
    );

    let cargo_binary = run_cargo(workspace_root, target_dir, ancestor, &short)?;
    if !verify_binary(&cargo_binary, ancestor, &short) {
        return None;
    }

    fs::create_dir_all(&artifact_dir)
        .expect("provenance-keyed stale-child artifact directory should be creatable");
    let staging = artifact_dir.join(format!("artifact.tmp.{}", std::process::id()));
    let cargo_sha256 = sha256_file(&cargo_binary).expect("Cargo child output should hash");
    fs::copy(&cargo_binary, &staging)
        .expect("verified Cargo output should copy into the fixture cache");
    assert_eq!(
        sha256_file(&staging).expect("staged child artifact should hash"),
        cargo_sha256,
        "staged child artifact must match Cargo output before publication"
    );
    assert!(verify_binary(&staging, ancestor, &short));
    publish_staged_file_noreplace(&staging, &artifact)
        .expect("verified stale-child artifact should publish atomically without replacement");

    let cargo_build_invocations = record_cargo_build_invocation(&build_count_path);
    assert_eq!(
        cargo_build_invocations, 1,
        "one provenance key must publish exactly one successful nested Cargo build"
    );
    let artifact_sha256 = sha256_file(&artifact).expect("published child artifact should hash");
    assert!(verify_binary(&artifact, ancestor, &short));
    let marker = VerifiedArtifactMarker {
        version: FIXTURE_MARKER_VERSION,
        source_sha256,
        built_from: ancestor.to_string(),
        short_commit: short,
        artifact_sha256,
        cargo_build_invocations,
        reuse_observations: 0,
    };
    write_marker(&marker_path, &marker);
    let observed = read_marker(&marker_path).expect("the built artifact marker should read back");
    assert_eq!(observed.cargo_build_invocations, 1);
    assert_eq!(observed.reuse_observations, 0);
    record_scoped_fixture_observation(&marker, FixtureUse::Built);
    Some(artifact)
}

fn run_nested_cargo(
    workspace_root: &Path,
    target_dir: &Path,
    ancestor: &str,
    short: &str,
) -> Option<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "-p", "jit", "--bin", "jit"])
        .current_dir(workspace_root)
        .env("JIT_BUILD_GIT_HASH", ancestor)
        .env("JIT_BUILD_GIT_SHORT_HASH", short)
        .env("JIT_BUILD_GIT_DIRTY", "false")
        .env("CARGO_TARGET_DIR", target_dir)
        .status()
        .ok()?;
    status.success().then(|| {
        target_dir
            .join("debug")
            .join(format!("jit{}", std::env::consts::EXE_SUFFIX))
    })
}

fn fixture_cache_key(ancestor: &str, source_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FIXTURE_MARKER_VERSION.to_le_bytes());
    hasher.update(ancestor.as_bytes());
    hasher.update([0]);
    hasher.update(source_sha256.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn read_verified_marker(
    marker_path: &Path,
    build_count_path: &Path,
    artifact: &Path,
    source_sha256: &str,
    ancestor: &str,
    short: &str,
) -> Option<VerifiedArtifactMarker> {
    let marker = read_marker(marker_path)?;
    (marker.version == FIXTURE_MARKER_VERSION
        && marker.source_sha256 == source_sha256
        && marker.built_from == ancestor
        && marker.short_commit == short
        && marker.cargo_build_invocations == 1
        && read_build_count(build_count_path) == Some(marker.cargo_build_invocations)
        && sha256_file(artifact).ok().as_deref() == Some(marker.artifact_sha256.as_str()))
    .then_some(marker)
}

fn read_marker(path: &Path) -> Option<VerifiedArtifactMarker> {
    serde_json::from_reader(BufReader::new(File::open(path).ok()?)).ok()
}

fn record_cargo_build_invocation(path: &Path) -> u32 {
    let count = read_build_count(path).unwrap_or(0) + 1;
    let staging = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&staging, format!("{count}\n"))
        .expect("nested Cargo build count staging file should be writable under its lock");
    assert_eq!(read_build_count(&staging), Some(count));
    publish_staged_file_noreplace(&staging, path)
        .expect("nested Cargo build count should publish without replacement");
    count
}

fn read_build_count(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn binary_reports_provenance(binary: &Path, ancestor: &str, short: &str) -> bool {
    let output = Command::new(binary)
        .args(["version", "--json"])
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY")
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .is_some_and(|version| {
            version["git_commit"].as_str() == Some(ancestor)
                && version["git_short_commit"].as_str() == Some(short)
                && version["git_dirty"].as_bool() == Some(false)
        })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn regular_file_fingerprint(path: &Path) -> std::io::Result<FileFingerprint> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not an ordinary file",
            path.display()
        )));
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    Ok(FileFingerprint {
        length: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn write_marker(path: &Path, marker: &VerifiedArtifactMarker) {
    let staging = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(marker).expect("fixture marker should serialize");
    fs::write(&staging, bytes).expect("fixture marker staging file should be writable");
    assert_eq!(read_marker(&staging).as_ref(), Some(marker));
    if path.exists() {
        fs::rename(staging, path).expect("fixture marker replacement should be atomic");
    } else {
        publish_staged_file_noreplace(&staging, path)
            .expect("new fixture marker should publish atomically without replacement");
    }
}

fn publish_staged_file_noreplace(staging: &Path, destination: &Path) -> std::io::Result<()> {
    if let Err(error) = fs::hard_link(staging, destination) {
        // Only the unpublished staging name is cleanup-eligible. An occupied
        // destination is never removed or overwritten.
        let _ = fs::remove_file(staging);
        return Err(error);
    }
    // `destination` now names the fully written staging inode. Removing this
    // second name cannot create a gap at the published destination.
    fs::remove_file(staging)
}

fn record_scoped_fixture_observation(marker: &VerifiedArtifactMarker, outcome: FixtureUse) {
    if std::env::var_os(SETUP_MODE).is_some() {
        return;
    }
    let Some(directory) = std::env::var_os(REUSE_OBSERVATION_DIR).map(PathBuf::from) else {
        return;
    };
    let test = current_semantic_test();
    let index = NEXTEST_SEMANTIC_TESTS
        .iter()
        .position(|candidate| *candidate == test)
        .expect("the current semantic test should have a stable fixture index");
    let staging = directory.join(format!(".fixture-{index}.json.tmp"));
    let destination = directory.join(format!("fixture-{index}.json"));
    let observation = ScopedFixtureObservation {
        test: test.to_string(),
        outcome,
        source_sha256: marker.source_sha256.clone(),
        built_from: marker.built_from.clone(),
        cargo_build_invocations: marker.cargo_build_invocations,
        scratch_shape: None,
        scratch_root: None,
    };
    let content = serde_json::to_vec_pretty(&observation)
        .expect("scoped fixture observation should serialize");
    fs::write(&staging, &content)
        .expect("scoped fixture observation staging file should be writable");
    assert_eq!(
        serde_json::from_reader::<_, ScopedFixtureObservation>(
            File::open(&staging).expect("scoped fixture observation should read back")
        )
        .expect("scoped fixture observation should remain valid JSON"),
        observation
    );
    publish_staged_file_noreplace(&staging, &destination)
        .expect("each nextest semantic test should publish one unique fixture observation");
}

fn current_semantic_test() -> &'static str {
    let arguments = std::env::args().collect::<Vec<_>>();
    NEXTEST_SEMANTIC_TESTS
        .iter()
        .copied()
        .find(|test| arguments.iter().any(|argument| argument == test))
        .unwrap_or_else(|| {
            panic!("nextest fixture caller has no semantic-test identity in argv: {arguments:?}")
        })
}

fn record_scoped_scratch_observation(shape: ScratchShape, root: &Path) {
    let Some(directory) = std::env::var_os(REUSE_OBSERVATION_DIR).map(PathBuf::from) else {
        return;
    };
    let test = current_semantic_test();
    let index = NEXTEST_SEMANTIC_TESTS
        .iter()
        .position(|candidate| *candidate == test)
        .expect("the current semantic test should have a stable fixture index");
    let destination = directory.join(format!("fixture-{index}.json"));
    let mut observation: ScopedFixtureObservation = serde_json::from_reader(
        File::open(&destination).expect("artifact observation should precede scratch setup"),
    )
    .expect("artifact observation should remain valid JSON");
    assert!(
        observation.scratch_root.is_none(),
        "each semantic test should consume exactly one prepared scratch repository"
    );
    observation.scratch_shape = Some(shape);
    observation.scratch_root = Some(root.to_path_buf());
    let staging = directory.join(format!(".fixture-{index}.scratch.json.tmp"));
    fs::write(
        &staging,
        serde_json::to_vec_pretty(&observation).expect("scratch observation should serialize"),
    )
    .expect("scratch observation staging file should be writable");
    assert_eq!(
        serde_json::from_reader::<_, ScopedFixtureObservation>(
            File::open(&staging).expect("scratch observation staging should open")
        )
        .expect("scratch observation staging should remain valid JSON"),
        observation
    );
    fs::rename(staging, destination)
        .expect("per-test scratch observation replacement should be atomic");
}

fn assert_nextest_configuration_prepares_stale_fixture_with_supported_setup_script() {
    let workspace_root = workspace_root();
    let config: toml::Value = fs::read_to_string(workspace_root.join(".config/nextest.toml"))
        .expect("nextest configuration should be readable")
        .parse()
        .expect("nextest configuration should remain valid TOML");

    let experimental = config["experimental"]
        .as_array()
        .expect("nextest experimental features should be declared")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<BTreeSet<_>>();
    assert!(
        experimental.contains("setup-scripts"),
        "the pinned runner requires an explicit setup-scripts opt-in"
    );

    let setup = &config["scripts"]["setup"]["stale-binary-fixture"];
    let command = &setup["command"];
    assert_eq!(command["relative-to"].as_str(), Some("workspace-root"));
    let command_line = command["command-line"]
        .as_str()
        .expect("stale fixture setup should name one workspace-relative helper");
    let helper = workspace_root.join(command_line);
    assert!(
        helper.is_file(),
        "configured stale fixture setup helper should exist: {}",
        helper.display()
    );
    let self_test = Command::new(&helper)
        .arg("--self-test")
        .current_dir(&workspace_root)
        .output()
        .expect("stale fixture setup helper self-test should launch");
    assert!(
        self_test.status.success()
            && String::from_utf8_lossy(&self_test.stdout).contains("self-test: PASS"),
        "stale fixture setup helper self-test should pass: stdout={} stderr={}",
        String::from_utf8_lossy(&self_test.stdout),
        String::from_utf8_lossy(&self_test.stderr)
    );

    let rules = config["profile"]["default"]["scripts"]
        .as_array()
        .expect("the default profile should attach setup scripts to test filtersets");
    assert!(
        rules.iter().any(|rule| {
            rule["setup"].as_str() == Some("stale-binary-fixture")
                && NEXTEST_SEMANTIC_TESTS.iter().all(|test| {
                    rule["filter"]
                        .as_str()
                        .is_some_and(|filter| filter.contains(test))
                })
        }),
        "one supported setup rule must cover all six semantic tests"
    );
}

#[test]
fn test_stale_binary_fixture_no_replace_publication_preserves_occupied_destination() {
    let directory = TempDir::new().unwrap();
    let staging = directory.path().join("artifact.tmp");
    let destination = directory.path().join("jit");
    fs::write(&staging, b"replacement").unwrap();
    fs::write(&destination, b"occupied").unwrap();

    let error = publish_staged_file_noreplace(&staging, &destination).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&destination).unwrap(), b"occupied");
    assert!(!staging.exists());
}

#[test]
fn test_stale_binary_fixture_marker_replacement_leaves_one_complete_file() {
    let directory = TempDir::new().unwrap();
    let marker_path = directory.path().join("verified-artifact.json");
    let marker = |reuse_observations| VerifiedArtifactMarker {
        version: FIXTURE_MARKER_VERSION,
        source_sha256: "source".to_string(),
        built_from: "full".to_string(),
        short_commit: "short".to_string(),
        artifact_sha256: "artifact".to_string(),
        cargo_build_invocations: 1,
        reuse_observations,
    };

    write_marker(&marker_path, &marker(0));
    write_marker(&marker_path, &marker(1));

    assert_eq!(read_marker(&marker_path).unwrap().reuse_observations, 1);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn test_stale_binary_fixture_failed_cargo_attempt_leaves_cache_retryable() {
    const FAKE_CHILD: &[u8] = b"verified fake child";

    let workspace_root = workspace_root();
    let ancestor = ancestor_commit(&workspace_root)
        .expect("workspace should have enough history for the stale-binary fixture");
    let directory = TempDir::new().expect("failure-injection cache needs a directory");
    let target_dir = directory.path().join(STALE_CHILD_CACHE);
    let artifacts_root = target_dir.join("artifacts");
    let artifact_directories = || match fs::read_dir(&artifacts_root) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .expect("fixture artifact entry should remain readable")
                    .path()
            })
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!(
            "fixture artifacts root {} should remain readable: {error}",
            artifacts_root.display()
        ),
    };
    let verify_fake =
        |binary: &Path, _: &str, _: &str| fs::read(binary).is_ok_and(|bytes| bytes == FAKE_CHILD);

    let failed = build_stale_child_binary_with_runner(
        &workspace_root,
        &ancestor,
        &target_dir,
        |_, _, _, _| None,
        verify_fake,
    );

    assert!(failed.is_none());
    let directories_before_retry = artifact_directories();
    assert!(
        directories_before_retry.is_empty(),
        "failed Cargo must leave no published artifact directories before retry: {directories_before_retry:?}"
    );

    let artifact = build_stale_child_binary_with_runner(
        &workspace_root,
        &ancestor,
        &target_dir,
        |_, target_dir, _, _| {
            let cargo_binary = target_dir.join("fake-cargo-output");
            fs::write(&cargo_binary, FAKE_CHILD).unwrap();
            Some(cargo_binary)
        },
        verify_fake,
    )
    .expect("a later caller should retry and publish one verified artifact");

    let artifact_directories = artifact_directories();
    assert_eq!(
        artifact_directories.len(),
        1,
        "successful retry must publish exactly one provenance-key directory: {artifact_directories:?}"
    );
    let artifact_dir = &artifact_directories[0];
    assert_eq!(artifact.parent(), Some(artifact_dir.as_path()));
    assert_eq!(fs::read(&artifact).unwrap(), FAKE_CHILD);
    assert_eq!(
        read_build_count(&artifact_dir.join("cargo-build-invocations")),
        Some(1)
    );
    assert_eq!(
        read_marker(&artifact_dir.join("verified-artifact.json"))
            .unwrap()
            .cargo_build_invocations,
        1
    );
}

/// REQ-03: while Cargo remains the outer suite runner, this regression invokes
/// the pinned nextest runner over exactly the six stale-binary semantic tests.
/// Nextest sets `NEXTEST` in each process it launches, so the test returns when
/// the outer suite itself is nextest and cannot recursively spawn another run.
#[test]
fn test_stale_binary_fixture_runs_six_semantic_tests_under_pinned_nextest() {
    assert_nextest_configuration_prepares_stale_fixture_with_supported_setup_script();
    if std::env::var(SETUP_MODE).as_deref() == Ok("1") {
        prepare_nextest_fixture();
        return;
    }
    if std::env::var_os("NEXTEST").is_some() {
        return;
    }
    assert_main_branch_source_does_not_leak_into_prepared_scratch();

    let version = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .expect("the pinned cargo-nextest runner should be installed");
    let version_stdout = String::from_utf8_lossy(&version.stdout);
    assert!(
        version.status.success()
            && version_stdout.starts_with(&format!("cargo-nextest {PINNED_NEXTEST_VERSION} ")),
        "expected cargo-nextest {PINNED_NEXTEST_VERSION}; stdout={} stderr={}",
        version_stdout,
        String::from_utf8_lossy(&version.stderr)
    );

    let workspace_root = workspace_root();
    let reuse_observations = TempDir::new().expect("nextest reuse observations need a directory");

    let filter = "test(checker_child) | test(non_gate_context) | \
                  test(gate_context_early) | test(test_gate_evaluate_)";
    let run = Command::new("cargo")
        .current_dir(&workspace_root)
        .env(REUSE_OBSERVATION_DIR, reuse_observations.path())
        .args([
            "nextest",
            "run",
            "-p",
            "jit",
            "--test",
            "scratch_build",
            "--test-threads",
            "6",
            "-E",
            filter,
        ])
        .output()
        .expect("the pinned nextest stale-binary run should launch");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success(),
        "pinned nextest stale-binary run failed:\n{report}"
    );
    assert!(
        report.contains("6 tests run: 6 passed"),
        "pinned nextest must independently report all six semantic tests:\n{report}"
    );
    let setup_receipt_path = reuse_observations.path().join("setup-receipt.json");
    let setup_receipt: PreparedFixtureReceipt = serde_json::from_reader(
        File::open(&setup_receipt_path).expect("setup build observation should open"),
    )
    .expect("setup build observation should remain valid JSON");
    assert_eq!(
        setup_receipt.marker.cargo_build_invocations, 1,
        "setup must prove exactly one successful nested Cargo build"
    );

    let mut observations = fs::read_dir(reuse_observations.path())
        .expect("nextest fixture observation directory should remain readable")
        .filter_map(|entry| {
            let path = entry
                .expect("fixture observation entry should be readable")
                .path();
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fixture-"))
                .then(|| {
                    serde_json::from_reader::<_, ScopedFixtureObservation>(
                        File::open(&path).expect("fixture observation should open"),
                    )
                    .unwrap_or_else(|error| {
                        panic!("invalid fixture observation {}: {error}", path.display())
                    })
                })
        })
        .collect::<Vec<_>>();
    observations.sort_by(|left, right| left.test.cmp(&right.test));
    eprintln!("stale-binary nextest fixture observations: {observations:#?}");

    let expected_tests = NEXTEST_SEMANTIC_TESTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let observed_tests = observations
        .iter()
        .map(|observation| observation.test.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed_tests, expected_tests,
        "all six selected nextest semantic tests must identify their fixture outcome"
    );
    assert_eq!(
        observations.len(),
        6,
        "each selected nextest semantic test must publish exactly one fixture observation"
    );
    assert_eq!(
        observations
            .iter()
            .filter(|observation| observation.outcome == FixtureUse::PreparedReuse)
            .count(),
        6,
        "all semantic identities must truthfully report prepared reuse: {observations:#?}"
    );
    assert!(
        observations
            .iter()
            .all(|observation| observation.cargo_build_invocations == 1),
        "every observed artifact must prove exactly one nested Cargo build: {observations:#?}"
    );
    assert_eq!(
        observations
            .iter()
            .map(|observation| (&observation.source_sha256, &observation.built_from))
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "all six nextest tests must use one provenance-keyed artifact: {observations:#?}"
    );
    assert_eq!(
        observations
            .iter()
            .filter_map(|observation| observation.scratch_shape)
            .collect::<BTreeSet<_>>()
            .len(),
        2,
        "the setup must supply exactly the build-input and metadata-only scratch shapes: {observations:#?}"
    );
    assert_eq!(
        observations
            .iter()
            .filter_map(|observation| observation.scratch_root.as_ref())
            .collect::<BTreeSet<_>>()
            .len(),
        6,
        "every semantic identity must receive a unique mutable scratch root: {observations:#?}"
    );
}

/// Build a scratch git repository whose `HEAD` is one commit past `ancestor`:
/// `ancestor` is a known commit here (fetched from the real workspace, along
/// with its full history), but no longer at `HEAD`. Entirely inside the
/// disposable scratch repo — no ref in the real workspace is read, moved, or
/// written. Returns `None` (skip) if any local git step fails.
pub(super) fn scratch_repo_stale_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
    if std::env::var_os("NEXTEST").is_some() && std::env::var_os(SETUP_MODE).is_none() {
        return Some(clone_prepared_scratch(
            workspace_root,
            ancestor,
            ScratchShape::BuildInputChange,
        ));
    }
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "crates/jit/src/main.rs",
        b"\n// build-input change for stale-binary coverage\n",
    )
}

pub(super) fn scratch_repo_metadata_only_for(
    workspace_root: &Path,
    ancestor: &str,
) -> Option<TempDir> {
    if std::env::var_os("NEXTEST").is_some() && std::env::var_os(SETUP_MODE).is_none() {
        return Some(clone_prepared_scratch(
            workspace_root,
            ancestor,
            ScratchShape::MetadataOnly,
        ));
    }
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "docs/stale-binary-metadata.md",
        b"metadata-only change for stale-binary coverage\n",
    )
}

fn clone_prepared_scratch(workspace_root: &Path, ancestor: &str, shape: ScratchShape) -> TempDir {
    let receipt = read_prepared_receipt()
        .unwrap_or_else(|error| panic!("nextest stale-binary scratch setup is invalid: {error}"));
    assert_eq!(receipt.marker.built_from, ancestor);
    let baseline = match shape {
        ScratchShape::BuildInputChange => &receipt.build_input_baseline,
        ScratchShape::MetadataOnly => &receipt.metadata_baseline,
    };
    assert!(
        baseline.starts_with(workspace_root.join("target").join(STALE_CHILD_CACHE)),
        "prepared scratch baseline must remain inside the stale-child cache: {}",
        baseline.display()
    );
    assert!(
        verify_scratch_repository(baseline, ancestor, shape),
        "prepared scratch baseline must remain verified after setup"
    );
    let scratch = clone_scratch_baseline(baseline, ancestor, shape);
    scrub_stale_repository_data(scratch.path());
    record_scoped_scratch_observation(shape, scratch.path());
    scratch
}

fn clone_scratch_baseline(baseline: &Path, ancestor: &str, shape: ScratchShape) -> TempDir {
    let scratch = TempDir::new().expect("prepared scratch clone needs a unique directory");
    let status = Command::new("git")
        .args(["clone", "--shared", "--no-checkout", "-q"])
        .arg(baseline)
        .arg(scratch.path())
        .status()
        .expect("prepared scratch baseline clone should launch");
    assert!(status.success(), "prepared scratch baseline should clone");
    let (changed_path, change) = scratch_shape_change(shape);
    let sparse = Command::new("git")
        .args(["sparse-checkout", "set", "--no-cone", changed_path])
        .current_dir(scratch.path())
        .status()
        .expect("prepared scratch sparse checkout should launch");
    assert!(sparse.success());
    let checkout = Command::new("git")
        .args(["checkout", "-q", "fixture"])
        .current_dir(scratch.path())
        .status()
        .expect("prepared scratch checkout should launch");
    assert!(checkout.success());
    let remote = Command::new("git")
        .args(["remote", "remove", "origin"])
        .current_dir(scratch.path())
        .status()
        .expect("prepared scratch consumer origin removal should launch");
    assert!(remote.success());
    let parent = Command::new("git")
        .args(["rev-parse", "HEAD^"])
        .current_dir(scratch.path())
        .output()
        .expect("prepared scratch clone ancestry should be readable");
    assert!(parent.status.success());
    assert_eq!(String::from_utf8_lossy(&parent.stdout).trim(), ancestor);
    assert_eq!(
        fs::read(scratch.path().join(changed_path))
            .expect("prepared scratch clone should preserve its shape"),
        change
    );
    assert!(verify_scratch_repository(scratch.path(), ancestor, shape));
    scratch
}

fn scrub_stale_repository_data(root: &Path) {
    [".jit", ".claude"].iter().for_each(|name| {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(&path)
                .unwrap_or_else(|error| panic!("cannot remove {}: {error}", path.display())),
            Ok(_) => fs::remove_file(&path)
                .unwrap_or_else(|error| panic!("cannot remove {}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot inspect {}: {error}", path.display()),
        }
        assert!(
            fs::symlink_metadata(&path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "stale repository data must be absent: {}",
            path.display()
        );
    });
}

fn scratch_repo_advanced_with_change(
    workspace_root: &Path,
    ancestor: &str,
    changed_path: &str,
    change: &[u8],
) -> Option<TempDir> {
    let temp = TempDir::new().ok()?;
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !run(&["init", "-q"]) {
        return None;
    }
    if !run(&["fetch", "-q", workspace_root.to_str()?, ancestor]) {
        return None;
    }
    if !run(&["checkout", "-q", "FETCH_HEAD"]) {
        return None;
    }
    let path = temp.path().join(changed_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::write(path, change).ok()?;
    if !run(&["add", changed_path]) {
        return None;
    }
    if !run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "-m",
        "advance past the build commit",
    ]) {
        return None;
    }
    // The checked-out ANCESTOR predates the repository's move to canonical
    // `.agents/skills` doc links (`.claude` was a user-local convenience symlink
    // briefly git-tracked by mistake). A session-backed `jit init` captures the
    // whole-repository closure, whose no-follow discipline must not traverse that
    // symlink, so scrub the stale repository data from the disposable scratch tree:
    // the `jit init` below then runs fresh and exercises the stale-binary refusal
    // without any `.claude` dependency (jit:49adf23b).
    let _ = Command::new("rm")
        .args(["-rf", ".jit", ".claude"])
        .current_dir(temp.path())
        .status();
    Some(temp)
}

/// `jit init` + one automated gate (key `g`, checker `exec jit validate` —
/// the same shape as `scripts/jit-validate.sh`) + one issue requiring it, all
/// inside `repo_root`, using the real (fresh) `jit` binary. Returns the issue
/// id.
fn setup_gated_issue_with_validate_checker(repo_root: &Path) -> String {
    let status = Command::new(jit_binary())
        .current_dir(repo_root)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(jit_binary())
        .current_dir(repo_root)
        .args([
            "gate",
            "define",
            "g",
            "--title",
            "G",
            "--description",
            "G",
            "--mode",
            "auto",
            "--checker-command",
            "exec jit validate",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(repo_root)
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let created: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = created["id"].as_str().unwrap().to_string();

    let status = Command::new(jit_binary())
        .current_dir(repo_root)
        .args(["gate", "add", &issue_id, "g"])
        .status()
        .unwrap();
    assert!(status.success());

    issue_id
}

/// REQ-02: the evaluator (the real, fresh test binary, invoked by its exact
/// path — the identity predicate finds its build commit unknown in this
/// scratch repo, so it stays silent, REQ-03, and proceeds to spawn the
/// checker) is not itself stale relative to the scratch repo. The checker
/// script's OWN `jit validate` invocation resolves, via `PATH`, to a
/// SEPARATELY BUILT binary whose build commit IS known in the scratch repo
/// but no longer at `HEAD` — a genuinely stale child. The gate run must FAIL
/// (not silently pass), and the stale-binary refusal must be visible in the
/// persisted run record.
#[cfg(unix)]
#[test]
fn test_checker_child_stale_binary_fails_gate_run_visibly() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(child_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale child binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_stale_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };

    let issue_id = setup_gated_issue_with_validate_checker(scratch.path());

    // A scratch "bin" dir whose only `jit` is the stale child binary,
    // prepended to PATH for the evaluator's own process (and, by
    // inheritance, whatever checker subprocess it spawns). The evaluator
    // itself is invoked by its exact `CARGO_BIN_EXE_jit` path below, so this
    // PATH override affects only the checker's nested `jit validate` lookup.
    let bin_dir = TempDir::new().unwrap();
    std::os::unix::fs::symlink(&child_binary, bin_dir.path().join("jit")).unwrap();
    let real_path = std::env::var("PATH").unwrap_or_default();
    let overridden_path = format!("{}:{}", bin_dir.path().display(), real_path);

    let evaluate_output = Command::new(jit_binary())
        .current_dir(scratch.path())
        .env("PATH", &overridden_path)
        .args(["gate", "evaluate", &issue_id, "g"])
        .output()
        .unwrap();

    // The gate run FAILS (the checker ran — from the evaluator's point of
    // view — and returned a nonzero exit): GATE_FAILED, exit 4. NOT exit 10
    // (that's the evaluator's own direct-refusal case, which never spawns a
    // checker at all) and NOT exit 0 (a persisted stale "passed" verdict,
    // the failure mode this whole feature exists to prevent).
    assert_eq!(
        evaluate_output.status.code(),
        Some(4),
        "gate run should fail (checker ran, returned nonzero) rather than either \
         silently passing or the evaluator refusing outright; stdout={} stderr={}",
        String::from_utf8_lossy(&evaluate_output.stdout),
        String::from_utf8_lossy(&evaluate_output.stderr)
    );

    // The persisted run record — not just the evaluator's own stderr — must
    // show WHY: the stale-binary refusal, not a misleading downstream error
    // (REQ-02's literal wording: visible in the run record).
    let status_output = Command::new(jit_binary())
        .current_dir(scratch.path())
        .args(["gate", "status", &issue_id, "g", "--json"])
        .output()
        .unwrap();
    assert!(status_output.status.success());
    let run: serde_json::Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(run["status"], "failed");
    assert_eq!(
        run["exit_code"], 10,
        "the child's own exit code (10) must be the recorded exit code"
    );
    let stderr = run["stderr"].as_str().unwrap_or_default();
    assert!(
        stderr.contains("predates the tree under review"),
        "the persisted run's stderr must carry the child's stale-binary refusal, \
         not a misleading downstream error: {stderr}"
    );
}

/// The same child-process path stays usable when the repository advanced only
/// through a metadata change. This covers the startup self-check separately
/// from the evaluator-side guard above.
#[cfg(unix)]
#[test]
fn test_checker_child_metadata_only_change_does_not_refuse() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(child_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale child binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_metadata_only_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };
    let issue_id = setup_gated_issue_with_validate_checker(scratch.path());

    let bin_dir = TempDir::new().unwrap();
    std::os::unix::fs::symlink(&child_binary, bin_dir.path().join("jit")).unwrap();
    let real_path = std::env::var("PATH").unwrap_or_default();
    let overridden_path = format!("{}:{}", bin_dir.path().display(), real_path);

    let evaluate_output = Command::new(jit_binary())
        .current_dir(scratch.path())
        .env("PATH", &overridden_path)
        .args(["gate", "evaluate", &issue_id, "g"])
        .output()
        .unwrap();
    assert_eq!(
        evaluate_output.status.code(),
        Some(0),
        "metadata-only changes must not make the child stale: stdout={} stderr={}",
        String::from_utf8_lossy(&evaluate_output.stdout),
        String::from_utf8_lossy(&evaluate_output.stderr)
    );
}

/// REQ-02/03 counterpart: without `JIT_GATE_RUN` (i.e. NOT running inside a
/// gate checker's process tree), the exact same objectively-stale binary and
/// repository combination is left completely unchecked — an ordinary,
/// non-gate-context `jit` invocation is unaffected by this feature.
#[cfg(unix)]
#[test]
fn test_non_gate_context_child_invocation_stays_unchecked() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(child_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale child binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_stale_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };

    // `jit init`, run by the (objectively stale, relative to `scratch`)
    // child binary directly — NOT as a gate checker's child. The gate-context
    // variables are scrubbed explicitly: when this test suite itself executes
    // under a `cargo-ci` gate evaluation, the whole process tree inherits
    // JIT_GATE_RUN=1 from the evaluator, which would otherwise turn this
    // deliberately context-free invocation into a self-checking one.
    let output = Command::new(&child_binary)
        .current_dir(scratch.path())
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY")
        .arg("init")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "an ordinary (non-gate-context) invocation of an objectively stale binary \
         must succeed unchecked; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // A second ordinary command, past init, for good measure: still no
    // gate context, still unchecked.
    let output = Command::new(&child_binary)
        .current_dir(scratch.path())
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY")
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// REQ-02: every pre-dispatch output path (`--schema`, `version`, and Clap's
/// own `--help` auto-exit) is guarded. A checker script can consume any of
/// these outputs to inform its verdict, so under gate context a stale binary
/// must refuse before serving them — `run()`'s `stale_gate_child_precheck`
/// is its first statement, ahead of `Cli::parse` and every early return.
#[cfg(unix)]
#[test]
fn test_gate_context_early_paths_refuse_stale_binary() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(child_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale child binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_stale_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };

    // `.jit/` must exist for the precheck's discovery to name a real
    // repository root; initialize it context-free (unchecked, per the
    // non-gate-context test above).
    let output = Command::new(&child_binary)
        .current_dir(scratch.path())
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY")
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success());

    for args in [&["--schema"][..], &["version"][..], &["--help"][..]] {
        let output = Command::new(&child_binary)
            .current_dir(scratch.path())
            .env("JIT_GATE_RUN", "1")
            .env_remove("JIT_ISSUE_ID")
            .env_remove("JIT_GATE_KEY")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(10),
            "{args:?} under gate context must refuse from a stale binary; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("predates the tree under review"),
            "{args:?}: refusal must be the stale-binary one: {stderr}"
        );
    }
}
