//! Integration tests for repository-root discovery (issue ff9925af).
//!
//! `jit` must discover its `.jit/` repository the way git discovers `.git`:
//! by walking up from the current directory, stopping at the first ancestor
//! containing `.jit/` and never crossing a `.git` boundary. These tests drive
//! the real `jit` binary from various working directories inside isolated
//! `TempDir` fixtures.

use jit::test_cli::JitCli;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

/// `jit status --json` run from `dir` against `cli`'s environment.
fn jit_status(cli: &JitCli, dir: &Path) -> Output {
    cli.command(["status", "--json"])
        .current_dir(dir)
        .output()
        .expect("jit status failed to spawn")
}

fn status_error_message(output: &Output) -> String {
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status failure is one JSON document");
    envelope["error"]["message"]
        .as_str()
        .expect("status failure carries an error message")
        .to_owned()
}

// ---------------------------------------------------------------------------
// REQ-01: discovery from a nested subdirectory and from inside `.jit/` itself.
// ---------------------------------------------------------------------------

#[test]
fn test_status_discovers_repo_from_nested_subdirectory() {
    let cli = JitCli::initialised().unwrap();

    let nested = cli.root().join("crates/jit/src");
    fs::create_dir_all(&nested).unwrap();

    let out = jit_status(&cli, &nested);
    assert!(
        out.status.success(),
        "status from nested subdirectory failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_status_discovers_repo_from_inside_dot_jit() {
    let cli = JitCli::initialised().unwrap();

    let inside_dot_jit = cli.root().join(".jit/issues");
    assert!(
        inside_dot_jit.exists(),
        "jit init should have created .jit/issues"
    );

    let out = jit_status(&cli, &inside_dot_jit);
    assert!(
        out.status.success(),
        "status from inside .jit/issues failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_nested_profile_init_keeps_data_and_assets_in_child() {
    let parent = JitCli::initialised().unwrap();
    let parent_index = fs::read(parent.root().join(".jit/index.json")).unwrap();
    let parent_events = fs::read(parent.root().join(".jit/events.jsonl")).unwrap();

    let child = parent.root().join("child");
    fs::create_dir(&child).unwrap();
    let location = super::repository_package_at(&child, "jit-dogfood");
    let child_init = parent
        .command(["init", "--profile", &location, "--json"])
        .current_dir(&child)
        .output()
        .expect("nested init failed to spawn");

    assert!(
        child_init.status.success(),
        "nested init failed: stderr={}",
        String::from_utf8_lossy(&child_init.stderr)
    );
    assert!(child.join(".jit/index.json").is_file());
    assert!(child.join(".agents/skills/jit-manage/SKILL.md").is_file());
    assert_eq!(
        fs::read(parent.root().join(".jit/index.json")).unwrap(),
        parent_index
    );
    assert_eq!(
        fs::read(parent.root().join(".jit/events.jsonl")).unwrap(),
        parent_events
    );
    assert!(!parent.root().join(".agents").exists());
}

#[test]
fn test_explicit_non_ancestor_data_root_keeps_worktree_assets_at_cwd() {
    let parent = JitCli::initialised().unwrap();
    let parent_events = fs::read(parent.root().join(".jit/events.jsonl")).unwrap();
    let child = parent.root().join("child");
    fs::create_dir(&child).unwrap();
    let external = TempDir::new().unwrap();
    let data_root = external.path().join("jit-data");
    let location = super::repository_package_at(&child, "jit-dogfood");

    let output = parent
        .command_with_env(
            ["init", "--profile", &location, "--json"],
            [("JIT_DATA_DIR", &data_root)],
        )
        .current_dir(&child)
        .output()
        .expect("explicit-root init failed to spawn");

    assert!(
        output.status.success(),
        "explicit-root init failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(data_root.join("index.json").is_file());
    assert!(child.join(".agents/skills/jit-manage/SKILL.md").is_file());
    assert_eq!(
        fs::read(parent.root().join(".jit/events.jsonl")).unwrap(),
        parent_events
    );
    assert!(!parent.root().join(".agents").exists());
}

#[test]
fn test_relative_data_root_override_keeps_worktree_assets_at_discovered_root() {
    let repo = JitCli::initialised().unwrap();
    let child = repo.root().join("child");
    fs::create_dir(&child).unwrap();
    // Staged in the worktree the application targets, which the child's data
    // root override resolves to, not in the child the command runs from.
    let location = repo
        .root()
        .join(super::repository_package_at(repo.root(), "jit-dogfood"));

    let output = repo
        .command_with_env(
            [
                "profile",
                "apply",
                "--profile",
                &location.to_string_lossy(),
                "--json",
            ],
            [("JIT_DATA_DIR", "../.jit")],
        )
        .current_dir(&child)
        .output()
        .expect("relative-root profile apply failed to spawn");

    assert!(
        output.status.success(),
        "relative-root profile apply failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(repo.root().join(".jit/events.jsonl"))
            .unwrap()
            .contains("profile_lifecycle"),
        "the application is audited under the overridden data root"
    );
    assert!(repo
        .root()
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
    // An unrelated jit repository living above the boundary.
    let workspace = JitCli::initialised().unwrap();

    // A nested, separately version-controlled project with no `.jit` of its
    // own, and a subdirectory inside it.
    let repo_dir = workspace.root().join("repo");
    let sub_dir = repo_dir.join("sub");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::create_dir(repo_dir.join(".git")).unwrap();

    let out = jit_status(&workspace, &sub_dir);
    assert!(
        !out.status.success(),
        "status must not succeed by escaping into the unrelated outer .jit/"
    );
    let message = status_error_message(&out);
    assert!(
        message.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        message
    );
}

// ---------------------------------------------------------------------------
// REQ-03: the not-found error (with its `jit init` suggestion) is still
// emitted when no ancestor within the boundary has a `.jit/`.
// ---------------------------------------------------------------------------

#[test]
fn test_status_reports_not_found_with_init_suggestion_when_no_repo_in_boundary() {
    let cli = JitCli::uninitialised().unwrap();
    fs::create_dir(cli.root().join(".git")).unwrap();
    let sub_dir = cli.root().join("sub");
    fs::create_dir(&sub_dir).unwrap();

    let out = jit_status(&cli, &sub_dir);
    assert!(!out.status.success());
    let message = status_error_message(&out);
    assert!(
        message.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        message
    );
    assert!(
        message.contains("jit init"),
        "expected init suggestion, got: {}",
        message
    );
}

#[test]
fn test_status_reports_not_found_with_init_suggestion_when_no_repo_anywhere() {
    let cli = JitCli::uninitialised().unwrap();
    let sub_dir = cli.root().join("a/b/c");
    fs::create_dir_all(&sub_dir).unwrap();

    let out = jit_status(&cli, &sub_dir);
    assert!(!out.status.success());
    let message = status_error_message(&out);
    assert!(
        message.contains("JIT repository not found"),
        "expected not-found error, got: {}",
        message
    );
    assert!(
        message.contains("jit init"),
        "expected init suggestion, got: {}",
        message
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
    let ancestor_repo = JitCli::initialised().unwrap();
    let sub_dir = ancestor_repo.root().join("sub");
    fs::create_dir(&sub_dir).unwrap();

    // `override_repo` is a distinct repository pointed to explicitly via
    // JIT_DATA_DIR; it holds a marker issue that only exists there.
    let override_repo = JitCli::initialised().unwrap();
    override_repo
        .create_issue(["-t", "override-marker-issue"])
        .unwrap();

    let override_jit_dir = override_repo.root().join(".jit");

    let out = ancestor_repo
        .command_with_env(
            ["issue", "list", "--json"],
            [("JIT_DATA_DIR", &override_jit_dir)],
        )
        .current_dir(&sub_dir)
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

#[test]
fn test_hooks_install_targets_repository_selected_by_data_dir_override() {
    let process_repo = JitCli::uninitialised().unwrap();
    process_repo.init_git().unwrap();
    process_repo.run(["init"]).unwrap();
    let selected_repo = JitCli::uninitialised().unwrap();
    selected_repo.init_git().unwrap();
    selected_repo.run(["init"]).unwrap();

    let out = process_repo
        .command_with_env(
            ["hooks", "install", "--json"],
            [("JIT_DATA_DIR", selected_repo.root().join(".jit"))],
        )
        .output()
        .expect("hooks install failed to spawn");
    assert!(
        out.status.success(),
        "hooks install failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(selected_repo.root().join(".git/hooks/pre-commit").exists());
    assert!(!process_repo.root().join(".git/hooks/pre-commit").exists());
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
    let root = JitCli::uninitialised().unwrap();
    // `.git` bounds discovery to this fixture.
    fs::create_dir(root.root().join(".git")).unwrap();
    // A bare `.jit` with no index.json (a lone worktree.json mimics the
    // observed stray dir). This must be skipped, not adopted.
    let bare_jit = root.root().join(".jit");
    fs::create_dir(&bare_jit).unwrap();
    fs::write(bare_jit.join("worktree.json"), "{}").unwrap();

    let sub = root.root().join("a/b");
    fs::create_dir_all(&sub).unwrap();

    let out = jit_status(&root, &sub);
    assert!(!out.status.success());
    let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        envelope["error"]["code"],
        jit::output::ErrorCode::RepositoryNotFound.as_str(),
        "bare ancestor .jit must be reported as not found; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let message = status_error_message(&out);
    assert!(
        message.contains("JIT repository not found") && message.contains("jit init"),
        "expected not-found + init suggestion, got: {}",
        message
    );
}
