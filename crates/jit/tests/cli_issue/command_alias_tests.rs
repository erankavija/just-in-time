use jit::test_cli::JitCli;

#[test]
fn test_dependency_alias_behaves_like_dep_add() {
    let temp = JitCli::initialised().unwrap();

    let dependent = temp
        .create_issue(["-t", "Dependent", "-d", "test"])
        .unwrap();
    let dependency = temp
        .create_issue(["-t", "Dependency", "-d", "test"])
        .unwrap();

    // Use the `dependency` alias instead of `dep`.
    let output = temp
        .command(["dependency", "add", &dependent, &dependency])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`jit dependency add` should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the edge was created via issue show.
    let show = temp
        .command(["issue", "show", &dependent, "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&show.stdout)).unwrap();
    let deps = json["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0]["id"], dependency);
}

#[test]
fn test_document_alias_behaves_like_doc_list() {
    let temp = JitCli::initialised().unwrap();

    let id = temp.create_issue(["-t", "Has docs", "-d", "test"]).unwrap();

    // `document list` should behave like `doc list`.
    let via_document = temp
        .command(["document", "list", &id, "--json"])
        .output()
        .unwrap();
    assert!(
        via_document.status.success(),
        "`jit document list` should succeed: {}",
        String::from_utf8_lossy(&via_document.stderr)
    );

    let via_doc = temp
        .command(["doc", "list", &id, "--json"])
        .output()
        .unwrap();
    assert!(via_doc.status.success());

    assert_eq!(via_document.stdout, via_doc.stdout);
}

#[test]
fn test_add_label_alias_on_issue_update() {
    let temp = JitCli::initialised().unwrap();

    let id = temp
        .create_issue(["-t", "Label target", "-d", "test"])
        .unwrap();

    let output = temp
        .command(["issue", "update", &id, "--add-label", "area:foo"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`--add-label` alias should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let show = temp
        .command(["issue", "show", &id, "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&show.stdout)).unwrap();
    let labels = json["labels"].as_array().unwrap();
    assert!(
        labels.iter().any(|l| l == "area:foo"),
        "expected label area:foo in {labels:?}"
    );
}

#[test]
fn test_issue_list_matches_query_all_json() {
    let temp = JitCli::initialised().unwrap();

    temp.create_issue(["-t", "Alpha", "-d", "test"]).unwrap();
    temp.create_issue(["-t", "Beta", "-d", "test"]).unwrap();
    temp.create_issue(["-t", "Gamma", "-d", "test"]).unwrap();

    let list = temp.command(["issue", "list", "--json"]).output().unwrap();
    assert!(
        list.status.success(),
        "`jit issue list --json` should succeed: {}",
        String::from_utf8_lossy(&list.stderr)
    );

    let query = temp.command(["query", "all", "--json"]).output().unwrap();
    assert!(query.status.success());

    let list_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&list.stdout)).unwrap();
    let query_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&query.stdout)).unwrap();

    assert_eq!(list_json["issues"], query_json["issues"]);
    assert_eq!(list_json["issues"].as_array().unwrap().len(), 3);
}
