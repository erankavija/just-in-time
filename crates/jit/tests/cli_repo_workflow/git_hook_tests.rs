//! The installed pre-commit hook, run by real commits against the built binary.

use jit::test_cli::{jit_binary, JitCli};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Output};

const SYNC_SUBJECT: &str = "chore(jit): sync tracker";

/// `PATH` with the built binary first, as the hook and `land.sh` need it.
fn search_path() -> OsString {
    let bin = jit_binary().parent().unwrap().to_path_buf();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited))).unwrap()
}

fn on_path(cli: &JitCli, program: impl Into<OsString>, args: &[&str]) -> Command {
    let mut command = cli.program_command(program.into(), args);
    command
        .env("PATH", search_path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    command
}

/// A repository with the hook installed on `feature`, one commit behind `main`.
fn diverged_repository(worktree_section: &str) -> JitCli {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    cli.run(["init"]).unwrap();
    let config = cli.root().join(".jit/config.toml");
    let mut contents = std::fs::read_to_string(&config).unwrap();
    contents.push_str(worktree_section);
    std::fs::write(&config, contents).unwrap();
    cli.commit("base").unwrap();
    cli.git(["checkout", "-q", "-b", "feature"]).unwrap();
    cli.git(["checkout", "-q", "main"]).unwrap();
    std::fs::write(cli.root().join("main-only.txt"), "x").unwrap();
    cli.commit("main advances").unwrap();
    cli.git(["checkout", "-q", "feature"]).unwrap();
    cli.run(["hooks", "install"]).unwrap();
    cli
}

fn commit_of_gates(cli: &JitCli) -> Output {
    let gates = cli.root().join(".jit/gates.toml");
    let mut contents = std::fs::read_to_string(&gates).unwrap();
    contents.push_str("# edit\n");
    std::fs::write(&gates, contents).unwrap();
    cli.git(["add", ".jit/gates.toml"]).unwrap();
    on_path(cli, "git", &["commit", "-q", "-m", "edit gates"])
        .output()
        .unwrap()
}

#[test]
fn test_pre_commit_hook_divergence_check_follows_the_mode_jit_config_reports() {
    for (section, blocked) in [
        ("", false),
        ("\n[worktree]\nenforce_leases = \"off\"\n", false),
        ("\n[worktree]\nenforce_leases = \"warn\"\n", true),
        ("\n[worktree]\nenforce_leases = \"strict\"\n", true),
        ("\n[worktree]\n", true),
    ] {
        let cli = diverged_repository(section);
        let reported = cli
            .run(["config", "get", "worktree.enforce_leases"])
            .unwrap();
        assert_eq!(reported.trim() != "off", blocked, "{section:?}");

        let output = commit_of_gates(&cli);

        assert_eq!(!output.status.success(), blocked, "{section:?}");
    }
}

fn land_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.agents/skills/jit-execution-lead/scripts/land.sh")
}

fn jit_as(cli: &JitCli, agent: &str, args: &[&str]) {
    let output = cli
        .command_with_env(args, [("JIT_AGENT_ID", agent)])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_land_sync_commit_passes_the_real_hook_with_others_records_and_a_closed_own_issue() {
    const LEAD: &str = "agent:lead";
    const OTHER: &str = "agent:other";
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    cli.run(["init"]).unwrap();
    std::fs::write(
        cli.root().join(".gitignore"),
        ".jit-bootstrap.lock\n.jit/.*.lock\n.jit/worktree.json\n",
    )
    .unwrap();
    let config = cli.root().join(".jit/config.toml");
    let mut contents = std::fs::read_to_string(&config).unwrap();
    contents.push_str("\n[worktree]\nenforce_leases = \"strict\"\n");
    std::fs::write(&config, contents).unwrap();
    let others = cli.create_issue(["--title", "Other's"]).unwrap();
    let own = cli.create_issue(["--title", "Own"]).unwrap();
    cli.commit("base").unwrap();
    cli.git(["checkout", "-q", "-b", "side"]).unwrap();
    std::fs::write(cli.root().join("side.txt"), "side\n").unwrap();
    cli.commit("side").unwrap();
    cli.git(["checkout", "-q", "main"]).unwrap();
    cli.run(["hooks", "install"]).unwrap();
    jit_as(&cli, OTHER, &["claim", "acquire", &others, "--ttl", "600"]);
    jit_as(
        &cli,
        OTHER,
        &["issue", "update", &others, "--priority", "high"],
    );
    jit_as(&cli, LEAD, &["claim", "acquire", &own, "--ttl", "600"]);
    jit_as(&cli, LEAD, &["issue", "update", &own, "--state", "done"]);
    jit_as(&cli, LEAD, &["claim", "release", &own]);

    let output = on_path(&cli, "bash", &[land_script().to_str().unwrap(), "side"])
        .env("JIT_AGENT_ID", LEAD)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        cli.git(["log", "-1", "--format=%s", "HEAD^1"])
            .unwrap()
            .trim(),
        SYNC_SUBJECT
    );
    let synced = cli
        .git(["show", "--name-only", "--format=", "HEAD^1"])
        .unwrap();
    assert!(synced.contains(&others), "{synced}");
    assert!(synced.contains(&own), "{synced}");
}
