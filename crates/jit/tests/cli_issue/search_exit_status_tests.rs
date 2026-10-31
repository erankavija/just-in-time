//! Search JSON failures report their registered error code.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Output;
use std::str::FromStr;
use tempfile::TempDir;

fn assert_json_error_reports_code(output: Output, expected_code: ErrorCode) {
    assert!(!output.status.success(), "search invocation should fail");
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("failed JSON search should emit an error envelope");
    let code = ErrorCode::from_str(
        json["error"]["code"]
            .as_str()
            .expect("error envelope should carry a string code"),
    )
    .expect("error envelope should carry a registered code");

    assert_eq!(code, expected_code);
}

#[test]
fn test_search_json_missing_ripgrep_reports_ripgrep_not_found() {
    let repo = JitCli::initialised().unwrap();
    let empty_path = TempDir::new().expect("empty PATH directory should be created");
    let output = repo
        .command_with_env(
            ["search", "needle", "--json"],
            [("PATH", empty_path.path().as_os_str())],
        )
        .output()
        .expect("search should run");

    assert_json_error_reports_code(output, ErrorCode::RipgrepNotFound);
}

#[test]
fn test_search_json_backend_failure_reports_search_failed() {
    let repo = JitCli::initialised().unwrap();
    let tools = TempDir::new().expect("fake tool directory should be created");
    let ripgrep = tools.path().join("rg");
    fs::write(&ripgrep, "#!/bin/sh\nexit 2\n").expect("fake ripgrep should be written");
    fs::set_permissions(&ripgrep, fs::Permissions::from_mode(0o755))
        .expect("fake ripgrep should be executable");

    let output = repo
        .command_with_env(
            ["search", "needle", "--json"],
            [("PATH", tools.path().as_os_str())],
        )
        .output()
        .expect("search should run");

    assert_json_error_reports_code(output, ErrorCode::SearchFailed);
}
