//! The shared CLI process fixture drives the binary of this target.

use jit::test_cli::{seed_ambiguous_prefix, JitCli};
use std::path::Path;

#[test]
fn test_fixture_spawns_target_binary_over_owned_repository_and_reads_created_id() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    let id = cli.create_issue(["--title", "Fixture"]).unwrap();
    let shown = cli.run_json(["issue", "show", &id]).unwrap();

    assert_eq!(shown["id"], id);
    let root = cli.root().to_path_buf();
    drop(cli);
    assert!(!root.exists());
}

#[test]
fn test_commit_stages_the_whole_tree_into_head() {
    let cli = JitCli::initialised().unwrap();

    cli.init_git().unwrap().commit("base").unwrap();

    assert_eq!(
        cli.git(["log", "-1", "--format=%s"]).unwrap().trim(),
        "base"
    );
    assert!(cli.git(["status", "--porcelain"]).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn test_program_command_runs_named_program_in_repository_without_inherited_product_vars() {
    let cli = JitCli::uninitialised().unwrap();

    let output = cli
        .program_command("sh", ["-c", "pwd -P; env"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut lines = stdout.lines();
    assert_eq!(
        Path::new(lines.next().unwrap()),
        cli.root().canonicalize().unwrap()
    );
    assert!(lines.all(|line| !line.starts_with("JIT_")));
}

#[test]
fn test_linked_checkouts_share_base_commit_and_both_run_jit_leaving_runtime_state_ignored() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();

    let common_dir = |cli: &JitCli| {
        cli.root()
            .join(cli.git(["rev-parse", "--git-common-dir"]).unwrap().trim())
            .canonicalize()
            .unwrap()
    };
    assert_eq!(common_dir(&linked), common_dir(&primary));
    assert_ne!(linked.root(), primary.root());
    assert_eq!(
        linked.git(["rev-parse", "HEAD"]).unwrap(),
        primary.git(["rev-parse", "HEAD"]).unwrap()
    );
    let id = primary.create_issue(["--title", "Shared"]).unwrap();
    primary.commit("issue").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    assert_eq!(linked.run_json(["issue", "show", &id]).unwrap()["id"], id);
    let status = primary
        .git([
            "status",
            "--porcelain",
            "--ignored",
            "--untracked-files=all",
        ])
        .unwrap();
    let runtime: Vec<&str> = status
        .lines()
        .map(|line| {
            line.strip_prefix("!! ")
                .unwrap_or_else(|| panic!("{status}"))
        })
        .collect();
    assert!(
        runtime.iter().any(|path| path.starts_with(".jit/")),
        "{status}"
    );
    let ignored_in_linked = linked
        .git(
            ["check-ignore", "--"]
                .into_iter()
                .chain(runtime.iter().copied()),
        )
        .unwrap();
    assert_eq!(ignored_in_linked.lines().collect::<Vec<_>>(), runtime);
}

#[test]
fn test_seed_ambiguous_prefix_returns_prefix_of_at_least_two_ids() {
    let cli = JitCli::initialised().unwrap();

    let (prefix, ids) = seed_ambiguous_prefix(&cli).unwrap();

    assert!(ids.iter().filter(|id| id.starts_with(&prefix)).count() >= 2);
}
