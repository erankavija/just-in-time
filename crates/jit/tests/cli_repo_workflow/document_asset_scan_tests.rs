//! Regression coverage for document asset classification.

use jit::test_cli::JitCli;
use std::fs;

const DEMANGLED_SYMBOL: &str = "memchr::arch::x86_64::memchr::count_raw::find_avx2";

#[test]
fn test_doc_add_links_a_report_containing_a_demangled_rust_symbol() {
    let cli = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let issue = cli
        .create_issue(["--title", "Asset scan regression", "--label", "type:bug"])
        .unwrap()[..8]
        .to_owned();
    fs::write(
        cli.root().join("report.txt"),
        format!("# Attribution\n\nHot symbol: <{DEMANGLED_SYMBOL}>\n"),
    )
    .unwrap();

    for doc_type in ["report", "benchmark"] {
        let output = cli
            .command(["doc", "add", &issue, "report.txt", "--doc-type", doc_type])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "doc add with {doc_type}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = cli
        .command(["doc", "list", &issue, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "doc list failed");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let documents = listed["documents"].as_array().unwrap();
    assert!(!documents.is_empty());
    assert!(!listed.to_string().contains(DEMANGLED_SYMBOL));

    let assets = cli
        .command(["doc", "assets", "list", &issue, "report.txt", "--json"])
        .output()
        .unwrap();
    assert!(assets.status.success());
    let assets: serde_json::Value = serde_json::from_slice(&assets.stdout).unwrap();
    assert!(assets["assets"].as_array().unwrap().is_empty());
}

#[test]
fn test_doc_add_still_refuses_an_escaping_relative_path_beside_the_symbol() {
    let cli = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let issue = cli
        .create_issue(["--title", "Asset escape regression", "--label", "type:bug"])
        .unwrap()[..8]
        .to_owned();
    fs::write(
        cli.root().join("report.txt"),
        format!(
            "# Attribution\n\nHot symbol: <{DEMANGLED_SYMBOL}>\n\n[bad](../../../etc/passwd)\n"
        ),
    )
    .unwrap();

    let output = cli
        .command(["doc", "add", &issue, "report.txt", "--doc-type", "report"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_text.contains("Path escape detected"));

    let listed = cli
        .command(["doc", "list", &issue, "--json"])
        .output()
        .unwrap();
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(!listed["documents"].as_array().unwrap().is_empty());
    assert!(!listed.to_string().contains(DEMANGLED_SYMBOL));
    assert!(!listed.to_string().contains("../../../etc/passwd"));

    let assets = cli
        .command(["doc", "assets", "list", &issue, "report.txt", "--json"])
        .output()
        .unwrap();
    assert!(assets.status.success());
    let assets: serde_json::Value = serde_json::from_slice(&assets.stdout).unwrap();
    assert!(assets["assets"].as_array().unwrap().is_empty());
}
