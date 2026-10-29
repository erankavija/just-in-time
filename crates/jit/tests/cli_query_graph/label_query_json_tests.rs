//! Integration tests for label query JSON output

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use jit::test_taxonomy::{membership_label, setup_test_repo_with_taxonomy, TaxonomyRepo};
use std::str::FromStr;

#[test]
fn test_query_label_json_exact_match() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);
    let label = membership_label(&taxonomy, 1, "v1.0");
    cli.create_issue(["-t", "Milestone Task", "--label", &label])
        .unwrap();

    let json = cli
        .run_json(["query", "all", "--label", &label, "--full"])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert!(json["issues"].is_array());
    assert_eq!(json["issues"].as_array().unwrap().len(), 1);

    let issue = &json["issues"][0];
    assert_eq!(issue["title"], "Milestone Task");
    assert!(issue["labels"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(label)));
}

#[test]
fn test_query_label_json_invalid_pattern() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Query with invalid pattern (no colon)
    let output = cli
        .command(["query", "all", "--label", "invalidlabel", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("machine-readable failure is one JSON envelope");
    let code = ErrorCode::from_str(json["error"]["code"].as_str().unwrap())
        .expect("failure code is registered");
    assert_eq!(code, ErrorCode::InvalidLabelPattern);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid label pattern"));
    assert!(json["error"]["suggestions"][0]
        .as_str()
        .is_some_and(|suggestion| suggestion.contains("namespace:value")));
}

#[test]
fn test_query_label_text_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Test Task", "--label", "milestone:v1.0"])
        .unwrap();

    let stdout = cli
        .run(["query", "all", "--label", "milestone:v1.0"])
        .unwrap();

    assert!(stdout.contains("All issues (filtered):"));
    assert!(stdout.contains("Test Task"));
    assert!(stdout.contains("Total: 1"));
}
