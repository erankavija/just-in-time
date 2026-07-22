//! Integration tests for repository-root discovery (issue ff9925af).
//!
//! `jit` must discover its `.jit/` repository the way git discovers `.git`:
//! by walking up from the current directory, stopping at the first ancestor
//! containing `.jit/` and never crossing a `.git` boundary. These tests drive
//! the real `jit` binary from various working directories inside isolated
//! `TempDir` fixtures.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Build a sanitized `jit` command anchored at `dir`, with `JIT_DATA_DIR`
/// removed so ambient environment state never leaks into a test.
fn jit_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new(jit_binary());
    cmd.current_dir(dir)
        .env_remove("JIT_DATA_DIR")
        .env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY");
    cmd
}

fn jit_init(dir: &Path) -> Output {
    jit_cmd(dir)
        .arg("init")
        .output()
        .expect("jit init failed to spawn")
}

fn jit_status(dir: &Path) -> Output {
    jit_cmd(dir)
        .args(["status", "--json"])
        .output()
        .expect("jit status failed to spawn")
}

// ---------------------------------------------------------------------------
// REQ-01: discovery from a nested subdirectory and from inside `.jit/` itself.
// ---------------------------------------------------------------------------

#[test]
fn test_status_discovers_repo_from_nested_subdirectory() {
    let temp = TempDir::new().unwrap();
    let init_out = jit_init(temp.path());
    assert!(init_out.status.success(), "init failed: {:?}", init_out);

    let nested = temp.path().join("crates/jit/src");
    fs::create_dir_all(&nested).unwrap();

    let out = jit_status(&nested);
    assert!(
        out.status.success(),
        "status from nested subdirectory failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_status_discovers_repo_from_inside_dot_jit() {
    let temp = TempDir::new().unwrap();
    let init_out = jit_init(temp.path());
    assert!(init_out.status.success(), "init failed: {:?}", init_out);

    let inside_dot_jit = temp.path().join(".jit/issues");
    assert!(
        inside_dot_jit.exists(),
        "jit init should have created .jit/issues"
    );

    let out = jit_status(&inside_dot_jit);
    assert!(
        out.status.success(),
        "status from inside .jit/issues failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_status_gate_child_discovers_repo_from_inside_dot_jit() {
    let temp = TempDir::new().unwrap();
    let init_out = jit_init(temp.path());
    assert!(init_out.status.success(), "init failed: {:?}", init_out);

    let inside_dot_jit = temp.path().join(".jit/issues");
    let out = jit_cmd(&inside_dot_jit)
        .env("JIT_GATE_RUN", "1")
        .args(["status", "--json"])
        .output()
        .expect("jit status failed to spawn");
    assert!(
        out.status.success(),
        "gate child status from inside .jit/issues failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_nested_profile_init_keeps_data_and_assets_in_child() {
    let parent = TempDir::new().unwrap();
    let parent_init = jit_init(parent.path());
    assert!(
        parent_init.status.success(),
        "parent init failed: {parent_init:?}"
    );
    let parent_index = fs::read(parent.path().join(".jit/index.json")).unwrap();
    let parent_events = fs::read(parent.path().join(".jit/events.jsonl")).unwrap();

    let child = parent.path().join("child");
    fs::create_dir(&child).unwrap();
    let child_init = jit_cmd(&child)
        .args(["init", "--profile", "jit-dogfood", "--json"])
        .output()
        .expect("nested init failed to spawn");

    assert!(
        child_init.status.success(),
        "nested init failed: stderr={}",
        String::from_utf8_lossy(&child_init.stderr)
    );
    assert!(child.join(".jit/index.json").is_file());
    assert!(child.join(".jit/profiles/jit-dogfood.json").is_file());
    assert!(child.join(".agents/skills/jit-manage/SKILL.md").is_file());
    assert_eq!(
        fs::read(parent.path().join(".jit/index.json")).unwrap(),
        parent_index
    );
    assert_eq!(
        fs::read(parent.path().join(".jit/events.jsonl")).unwrap(),
        parent_events
    );
    assert!(!parent
        .path()
        .join(".jit/profiles/jit-dogfood.json")
        .exists());
    assert!(!parent.path().join(".agents").exists());
}

#[test]
fn test_explicit_non_ancestor_data_root_keeps_worktree_assets_at_cwd() {
    let parent = TempDir::new().unwrap();
    assert!(jit_init(parent.path()).status.success());
    let parent_events = fs::read(parent.path().join(".jit/events.jsonl")).unwrap();
    let child = parent.path().join("child");
    fs::create_dir(&child).unwrap();
    let external = TempDir::new().unwrap();
    let data_root = external.path().join("jit-data");

    let output = Command::new(jit_binary())
        .current_dir(&child)
        .env("JIT_DATA_DIR", &data_root)
        .args(["init", "--profile", "jit-dogfood", "--json"])
        .output()
        .expect("explicit-root init failed to spawn");

    assert!(
        output.status.success(),
        "explicit-root init failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(data_root.join("index.json").is_file());
    assert!(data_root.join("profiles/jit-dogfood.json").is_file());
    assert!(child.join(".agents/skills/jit-manage/SKILL.md").is_file());
    assert_eq!(
        fs::read(parent.path().join(".jit/events.jsonl")).unwrap(),
        parent_events
    );
    assert!(!parent.path().join(".agents").exists());
}

#[test]
fn test_relative_data_root_override_keeps_worktree_assets_at_discovered_root() {
    let repo = TempDir::new().unwrap();
    assert!(jit_init(repo.path()).status.success());
    let child = repo.path().join("child");
    fs::create_dir(&child).unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&child)
        .env("JIT_DATA_DIR", "../.jit")
        .args(["profile", "apply", "jit-dogfood", "--json"])
        .output()
        .expect("relative-root profile apply failed to spawn");

    assert!(
        output.status.success(),
        "relative-root profile apply failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(repo.path().join(".jit/profiles/jit-dogfood.json").is_file());
    assert!(repo
        .path()
        .join(".agents/skills/jit-manage/SKILL.md")
        .is_file());
    assert!(!child.join(".agents").exists());
}

// ---------------------------------------------------------------------------
// REQ-02: discovery halts at a `.git` boundary rather than escaping into an
// unrelated ancestor's `.jit/`.
// ---------------------------------------------------------------------------

#[test]
fn test_status_does_not_cross_git_boundary_into_unrelated_ancestor_repo() {
    let workspace = TempDir::new().unwrap();

    // An unrelated jit repository living above the boundary.
    let outer_init = jit_init(workspace.path());
    assert!(outer_init.status.success());

    // A nested, separately version-controlled project with no `.jit` of its
    // own, and a subdirectory inside it.
    let repo_dir = workspace.path().join("repo");
    let sub_dir = repo_dir.join("sub");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::create_dir(repo_dir.join(".git")).unwrap();

    let out = jit_status(&sub_dir);
    assert!(
        !out.status.success(),
        "status must not succeed by escaping into the unrelated outer .jit/"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// REQ-03: the not-found error (with its `jit init` suggestion) is still
// emitted when no ancestor within the boundary has a `.jit/`.
// ---------------------------------------------------------------------------

#[test]
fn test_status_reports_not_found_with_init_suggestion_when_no_repo_in_boundary() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let sub_dir = temp.path().join("sub");
    fs::create_dir(&sub_dir).unwrap();

    let out = jit_status(&sub_dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        stderr
    );
    assert!(
        stderr.contains("jit init"),
        "expected init suggestion, got: {}",
        stderr
    );
}

#[test]
fn test_status_reports_not_found_with_init_suggestion_when_no_repo_anywhere() {
    let temp = TempDir::new().unwrap();
    let sub_dir = temp.path().join("a/b/c");
    fs::create_dir_all(&sub_dir).unwrap();

    let out = jit_status(&sub_dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        stderr
    );
    assert!(
        stderr.contains("jit init"),
        "expected init suggestion, got: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// REQ-04: an explicit JIT_DATA_DIR override takes precedence over ancestor
// discovery.
// ---------------------------------------------------------------------------

#[test]
fn test_jit_data_dir_override_takes_precedence_over_ancestor_discovery() {
    // `ancestor_repo` is a real repository that ancestor discovery *would*
    // find from `ancestor_repo/sub` if the override were not honored.
    let ancestor_repo = TempDir::new().unwrap();
    let ancestor_init = jit_init(ancestor_repo.path());
    assert!(ancestor_init.status.success());
    let sub_dir = ancestor_repo.path().join("sub");
    fs::create_dir(&sub_dir).unwrap();

    // `override_repo` is a distinct repository pointed to explicitly via
    // JIT_DATA_DIR; it holds a marker issue that only exists there.
    let override_repo = TempDir::new().unwrap();
    let override_init = jit_init(override_repo.path());
    assert!(override_init.status.success());
    let create_out = jit_cmd(override_repo.path())
        .args(["issue", "create", "-t", "override-marker-issue"])
        .output()
        .expect("issue create failed to spawn");
    assert!(
        create_out.status.success(),
        "issue create failed: {:?}",
        create_out
    );

    let override_jit_dir = override_repo.path().join(".jit");

    let out = Command::new(jit_binary())
        .current_dir(&sub_dir)
        .env("JIT_DATA_DIR", &override_jit_dir)
        .args(["issue", "list", "--json"])
        .output()
        .expect("issue list failed to spawn");
    assert!(
        out.status.success(),
        "issue list with JIT_DATA_DIR override failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("override-marker-issue"),
        "expected the overridden repository's issue to be listed, got: {}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// Regression: a bare `.jit` (no index.json) in a shared ancestor — e.g. stray
// litter under a persistent CI temp root — must not be bound. Before discovery
// required an initialized repo, the upward walk resolved to such a directory
// and `jit` failed with GenericError (exit 1, "missing index.json") instead of
// the correct NotFound (exit 3). The `.git` marker bounds the walk within the
// fixture so the assertion is independent of the real filesystem above it.
// ---------------------------------------------------------------------------

#[test]
fn test_bare_ancestor_jit_is_skipped_not_bound() {
    let root = TempDir::new().unwrap();
    // `.git` bounds discovery to this fixture.
    fs::create_dir(root.path().join(".git")).unwrap();
    // A bare `.jit` with no index.json (a lone worktree.json mimics the
    // observed stray dir). This must be skipped, not adopted.
    let bare_jit = root.path().join(".jit");
    fs::create_dir(&bare_jit).unwrap();
    fs::write(bare_jit.join("worktree.json"), "{}").unwrap();

    let sub = root.path().join("a/b");
    fs::create_dir_all(&sub).unwrap();

    let out = jit_status(&sub);
    assert!(!out.status.success());
    assert_eq!(
        out.status.code(),
        Some(3),
        "bare ancestor .jit must yield NotFound (3), not GenericError (1); stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("JIT repository not found") && stderr.contains("jit init"),
        "expected not-found + init suggestion, got: {}",
        stderr
    );
}
