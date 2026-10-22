//! The shared CLI process fixture drives the binary of this target.

use jit::test_cli::JitCli;

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
