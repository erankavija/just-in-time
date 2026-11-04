//! Integration tests for label query JSON output

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;
use std::str::FromStr;

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
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid label pattern"));
    assert!(json["error"]["suggestions"][0]
        .as_str()
        .is_some_and(|suggestion| suggestion.contains("namespace:value")));
}
