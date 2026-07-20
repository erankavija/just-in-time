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
//! built from an older, real commit. The build is cached under
//! `target/jit-stale-child-test-cache` (first run pays a full dependency
//! build, ~20-30s; later runs are incremental) and is never written to the
//! shared `target/debug/jit` path that `CARGO_BIN_EXE_jit` and every other
//! test rely on.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

/// Resolve a real, well-in-the-past commit in the workspace's own history
/// (`HEAD~8`), used as the "stale child" binary's fake build commit. `None`
/// when the workspace has fewer than 9 commits or git is unavailable.
fn ancestor_commit(workspace_root: &Path) -> Option<String> {
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

/// Build a second `jit` binary that reports `ancestor` as its OWN build
/// commit (clean, not dirty) via `build.rs`'s env-var override — a real,
/// separately-compiled binary, not a mock. Cached under a stable directory so
/// repeat test runs are incremental. Returns its path, or `None` (skip) when
/// `cargo` is unavailable or the build fails.
fn build_stale_child_binary(workspace_root: &Path, ancestor: &str) -> Option<PathBuf> {
    let short = Command::new("git")
        .args(["rev-parse", "--short=8", ancestor])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    let target_dir = workspace_root
        .join("target")
        .join("jit-stale-child-test-cache");
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
    let binary = target_dir.join("debug").join("jit");
    binary.is_file().then_some(binary)
}

/// Build a scratch git repository whose `HEAD` is one commit past `ancestor`:
/// `ancestor` is a known commit here (fetched from the real workspace, along
/// with its full history), but no longer at `HEAD`. Entirely inside the
/// disposable scratch repo — no ref in the real workspace is read, moved, or
/// written. Returns `None` (skip) if any local git step fails.
fn scratch_repo_stale_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
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
    if !run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "--allow-empty",
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
