use super::*;
use std::process::Command;
use tempfile::TempDir;

fn real_git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
}

fn committed_repository() -> (TempDir, GitRepository, String) {
    let dir = TempDir::new().unwrap();
    let repository = GitRepository::init(dir.path());
    repository.write("docs/a.md", "first a");
    let commit = repository.commit("first");
    (dir, repository, commit)
}

#[test]
fn test_git_repository_commit_returns_the_new_head_commit_id() {
    let (_dir, repository, first) = committed_repository();
    repository.write("docs/a.md", "second a");
    repository.write("docs/nested/b.md", "second b");
    let second = repository.commit("second");

    let head =
        |dir: &Path| String::from_utf8(real_git(dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let parent =
        String::from_utf8(real_git(repository.root(), &["rev-parse", "HEAD^"]).stdout).unwrap();
    assert_eq!(head(repository.root()).trim(), second);
    assert_eq!(parent.trim(), first);
}

#[cfg(unix)]
#[test]
fn test_git_stand_in_records_each_invocation_arguments_in_order() {
    let (_dir, repository, _commit) = committed_repository();
    let stand_in = GitStandIn::new();
    let run = |args: &[&str]| {
        Command::new(stand_in.program())
            .args(args)
            .output()
            .unwrap()
    };
    let root = repository.root().to_str().unwrap();
    run(&["-C", root, "log", "--format=%s and more", "-1"]);
    run(&["--version"]);
    run(&["-C", root, "", "status"]);

    let invocations = stand_in.invocations();
    let recorded: Vec<(Option<&Path>, Vec<&str>)> = invocations
        .iter()
        .map(|invocation| {
            (
                invocation.directory(),
                invocation.args().iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        recorded,
        vec![
            (
                Some(repository.root()),
                vec!["log", "--format=%s and more", "-1"]
            ),
            (None, vec!["--version"]),
            (Some(repository.root()), vec!["", "status"]),
        ]
    );
}

#[cfg(unix)]
#[test]
fn test_git_stand_in_returns_the_real_git_output_and_exit_status() {
    let (_dir, repository, _commit) = committed_repository();
    let stand_in = GitStandIn::new();
    let root = repository.root();

    [
        vec!["rev-parse", "HEAD"],
        vec!["rev-parse", "--verify", "--quiet", "refs/heads/absent"],
    ]
    .iter()
    .for_each(|args| {
        let through_stand_in = Command::new(stand_in.program())
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        let direct = real_git(root, args);
        assert_eq!(through_stand_in.status.code(), direct.status.code());
        assert_eq!(through_stand_in.stdout, direct.stdout);
        assert_eq!(through_stand_in.stderr, direct.stderr);
    });
}

#[cfg(unix)]
#[test]
fn test_git_stand_in_on_a_child_search_path_records_the_child_git_process() {
    let (_dir, repository, commit) = committed_repository();
    let stand_in = GitStandIn::new();

    let output = Command::new("git")
        .env("PATH", stand_in.search_path())
        .arg("-C")
        .arg(repository.root())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), commit);
    let invocations = stand_in.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].args(), ["rev-parse", "HEAD"]);
}

#[cfg(unix)]
#[test]
fn test_git_stand_in_records_concurrent_invocations_once_each() {
    let stand_in = GitStandIn::new();
    let markers: Vec<String> = (0..16).map(|index| format!("marker {index}")).collect();

    markers
        .iter()
        .map(|marker| {
            Command::new(stand_in.program())
                .args(["--version", marker])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .for_each(|mut child| {
            child.wait().unwrap();
        });

    let mut recorded: Vec<String> = stand_in
        .invocations()
        .iter()
        .map(|invocation| invocation.args().join("\n"))
        .collect();
    recorded.sort();
    let mut expected: Vec<String> = markers
        .iter()
        .map(|marker| format!("--version\n{marker}"))
        .collect();
    expected.sort();
    assert_eq!(recorded, expected);
}
