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
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

use jit::storage::FileLocker;

const STALE_CHILD_CACHE: &str = "jit-stale-child-test-cache";
const FIXTURE_MARKER_VERSION: u32 = 3;
const FIXTURE_LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const PINNED_NEXTEST_VERSION: &str = "0.9.133";
const REUSE_OBSERVATION_DIR: &str = "JIT_STALE_FIXTURE_REUSE_OBSERVATION_DIR";

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
    let target_dir = workspace_root.join("target").join(STALE_CHILD_CACHE);
    fs::create_dir_all(&target_dir).expect("stale-child cache directory should be creatable");
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
        record_scoped_reuse_observation();
        return Some(artifact);
    }

    assert!(
        !artifact.exists() && !marker_path.exists() && !build_count_path.exists(),
        "invalid occupied stale-child cache must fail without deleting or rebuilding: {}",
        artifact_dir.display()
    );

    fs::create_dir_all(&artifact_dir)
        .expect("provenance-keyed stale-child artifact directory should be creatable");
    let cargo_build_invocations = record_cargo_build_invocation(&build_count_path);
    assert_eq!(
        cargo_build_invocations, 1,
        "one provenance key must invoke nested Cargo exactly once"
    );
    let status = Command::new("cargo")
        .args(["build", "-p", "jit", "--bin", "jit"])
        .current_dir(workspace_root)
        .env("JIT_BUILD_GIT_HASH", ancestor)
        .env("JIT_BUILD_GIT_SHORT_HASH", &short)
        .env("JIT_BUILD_GIT_DIRTY", "false")
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let cargo_binary = target_dir
        .join("debug")
        .join(format!("jit{}", std::env::consts::EXE_SUFFIX));
    if !binary_reports_provenance(&cargo_binary, ancestor, &short) {
        return None;
    }

    let staging = artifact_dir.join(format!("artifact.tmp.{}", std::process::id()));
    let cargo_sha256 = sha256_file(&cargo_binary).expect("Cargo child output should hash");
    fs::copy(&cargo_binary, &staging)
        .expect("verified Cargo output should copy into the fixture cache");
    assert_eq!(
        sha256_file(&staging).expect("staged child artifact should hash"),
        cargo_sha256,
        "staged child artifact must match Cargo output before publication"
    );
    assert!(binary_reports_provenance(&staging, ancestor, &short));
    publish_staged_file_noreplace(&staging, &artifact)
        .expect("verified stale-child artifact should publish atomically without replacement");

    let artifact_sha256 = sha256_file(&artifact).expect("published child artifact should hash");
    assert!(binary_reports_provenance(&artifact, ancestor, &short));
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
    Some(artifact)
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

fn record_scoped_reuse_observation() {
    let Some(directory) = std::env::var_os(REUSE_OBSERVATION_DIR).map(PathBuf::from) else {
        return;
    };
    let process = std::process::id();
    let staging = directory.join(format!(".reuse-{process}.tmp"));
    let observation = directory.join(format!("reuse-{process}"));
    let content = b"verified artifact reused without Cargo\n";
    fs::write(&staging, content).expect("scoped reuse observation staging file should be writable");
    assert_eq!(
        fs::read(&staging).expect("scoped reuse observation should read back"),
        content
    );
    publish_staged_file_noreplace(&staging, &observation)
        .expect("each nextest process should publish one unique reuse observation");
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

/// REQ-03: while Cargo remains the outer suite runner, this regression invokes
/// the pinned nextest runner over exactly the six stale-binary semantic tests.
/// Nextest sets `NEXTEST` in each process it launches, so the test returns when
/// the outer suite itself is nextest and cannot recursively spawn another run.
#[test]
fn test_stale_binary_fixture_runs_six_semantic_tests_under_pinned_nextest() {
    if std::env::var_os("NEXTEST").is_some() {
        return;
    }

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
    let ancestor = ancestor_commit(&workspace_root)
        .expect("workspace should have enough history for the stale-binary fixture");
    build_stale_child_binary(&workspace_root, &ancestor)
        .expect("the cargo-test process should prewarm the verified child artifact");
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
            "2",
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
    assert_eq!(
        fs::read_dir(reuse_observations.path())
            .expect("nextest reuse observation directory should remain readable")
            .count(),
        6,
        "each nextest process must record one verified reuse without Cargo"
    );
}

/// Build a scratch git repository whose `HEAD` is one commit past `ancestor`:
/// `ancestor` is a known commit here (fetched from the real workspace, along
/// with its full history), but no longer at `HEAD`. Entirely inside the
/// disposable scratch repo — no ref in the real workspace is read, moved, or
/// written. Returns `None` (skip) if any local git step fails.
fn scratch_repo_stale_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "crates/jit/src/main.rs",
        b"\n// build-input change for stale-binary coverage\n",
    )
}

fn scratch_repo_metadata_only_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "docs/stale-binary-metadata.md",
        b"metadata-only change for stale-binary coverage\n",
    )
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
