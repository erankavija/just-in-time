use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit = jit_binary();
    Command::new(jit)
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

/// Create an issue and return its ID (last whitespace token of stdout).
fn create_issue(temp: &TempDir, title: &str) -> String {
    let jit = jit_binary();
    let output = Command::new(jit)
        .args(["issue", "create", "-t", title, "-d", "test"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

#[test]
fn test_dependency_alias_behaves_like_dep_add() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let dependent = create_issue(&temp, "Dependent");
    let dependency = create_issue(&temp, "Dependency");

    // Use the `dependency` alias instead of `dep`.
    let output = Command::new(jit)
        .args(["dependency", "add", &dependent, &dependency])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`jit dependency add` should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the edge was created via issue show.
    let show = Command::new(jit)
        .args(["issue", "show", &dependent, "--json"])
        .current_dir(temp.path())
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
    let temp = setup_test_repo();
    let jit = jit_binary();

    let id = create_issue(&temp, "Has docs");

    // `document list` should behave like `doc list`.
    let via_document = Command::new(jit)
        .args(["document", "list", &id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        via_document.status.success(),
        "`jit document list` should succeed: {}",
        String::from_utf8_lossy(&via_document.stderr)
    );

    let via_doc = Command::new(jit)
        .args(["doc", "list", &id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(via_doc.status.success());

    assert_eq!(via_document.stdout, via_doc.stdout);
}

#[test]
fn test_add_label_alias_on_issue_update() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let id = create_issue(&temp, "Label target");

    let output = Command::new(jit)
        .args(["issue", "update", &id, "--add-label", "area:foo"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`--add-label` alias should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let show = Command::new(jit)
        .args(["issue", "show", &id, "--json"])
        .current_dir(temp.path())
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
    let temp = setup_test_repo();
    let jit = jit_binary();

    create_issue(&temp, "Alpha");
    create_issue(&temp, "Beta");
    create_issue(&temp, "Gamma");

    let list = Command::new(jit)
        .args(["issue", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "`jit issue list --json` should succeed: {}",
        String::from_utf8_lossy(&list.stderr)
    );

    let query = Command::new(jit)
        .args(["query", "all", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(query.status.success());

    let list_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&list.stdout)).unwrap();
    let query_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&query.stdout)).unwrap();

    assert_eq!(list_json["count"], query_json["count"]);
    assert_eq!(list_json["issues"], query_json["issues"]);
    assert_eq!(list_json["count"], 3);
}
