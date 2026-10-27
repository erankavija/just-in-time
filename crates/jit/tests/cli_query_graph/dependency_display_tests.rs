use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_issue_show_enriched_dependencies() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    let dep1_id = cli
        .create_issue([
            "-t",
            "Dependency 1",
            "-d",
            "First dependency",
            "--priority",
            "high",
        ])
        .unwrap();
    let dep2_id = cli
        .create_issue([
            "-t",
            "Dependency 2",
            "-d",
            "Second dependency",
            "--priority",
            "normal",
        ])
        .unwrap();
    cli.run(["issue", "update", &dep1_id, "--state", "done"])
        .unwrap();
    let main_id = cli
        .create_issue(["-t", "Main Issue", "-d", "Has dependencies"])
        .unwrap();
    cli.run(["dep", "add", &main_id, &dep1_id, &dep2_id])
        .unwrap();

    let json = cli.run_json(["issue", "show", &main_id]).unwrap();

    let deps = json["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 2);

    // Each dependency should have: id, title, state, priority
    for dep in deps {
        assert!(dep["id"].is_string());
        assert!(dep["title"].is_string());
        assert!(dep["state"].is_string());
        assert!(dep["priority"].is_string());
    }

    let dep1 = deps.iter().find(|d| d["id"] == dep1_id).unwrap();
    assert_eq!(dep1["state"], "done");
    assert_eq!(dep1["title"], "Dependency 1");
    assert_eq!(dep1["priority"], "high");

    let dep2 = deps.iter().find(|d| d["id"] == dep2_id).unwrap();
    assert_eq!(dep2["state"], "ready");
    assert_eq!(dep2["title"], "Dependency 2");
    assert_eq!(dep2["priority"], "normal");

    let stdout = cli.run(["issue", "show", &main_id]).unwrap();

    // Should show dependencies with short hash, title, and state
    assert!(stdout.contains("Dependencies"));
    assert!(stdout.contains(&dep1_id[..8]));
    assert!(stdout.contains("Dependency 1"));
    assert!(stdout.contains("[done]"));
    assert!(stdout.contains(&dep2_id[..8]));
    assert!(stdout.contains("Dependency 2"));
    assert!(stdout.contains("[ready]"));

    // Should show summary
    assert!(stdout.contains("1/2 met") || stdout.contains("1 of 2 met"));
}

#[test]
fn test_issue_show_no_dependencies() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli
        .create_issue(["-t", "Standalone", "-d", "No deps"])
        .unwrap();

    let json = cli.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(json["dependencies"].as_array().unwrap().len(), 0);

    let stdout = cli.run(["issue", "show", &id]).unwrap();
    assert!(stdout.contains("Dependencies: None") || stdout.contains("Dependencies (0"));
}
