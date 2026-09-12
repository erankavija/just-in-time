//! Regression coverage for document asset classification.

use super::doc_show_tests::{create_issue, initialized_repo, jit};
use std::fs;

const DEMANGLED_SYMBOL: &str = "memchr::arch::x86_64::memchr::count_raw::find_avx2";

#[test]
fn test_doc_add_links_a_report_containing_a_demangled_rust_symbol() {
    let repo = initialized_repo();
    let issue = create_issue(
        repo.path(),
        "Asset scan regression",
        &["type:bug".to_string()],
    );
    fs::write(
        repo.path().join("report.txt"),
        format!("# Attribution\n\nHot symbol: <{DEMANGLED_SYMBOL}>\n"),
    )
    .unwrap();

    for doc_type in ["report", "benchmark"] {
        let output = jit(
            repo.path(),
            &[
                "doc",
                "add",
                issue.short_id.as_str(),
                "report.txt",
                "--doc-type",
                doc_type,
            ],
        );
        assert!(
            output.status.success(),
            "doc add with {doc_type}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = jit(
        repo.path(),
        &["doc", "list", issue.short_id.as_str(), "--json"],
    );
    assert!(output.status.success(), "doc list failed");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let documents = listed["documents"].as_array().unwrap();
    assert!(!documents.is_empty());
    assert!(!listed.to_string().contains(DEMANGLED_SYMBOL));

    let assets = jit(
        repo.path(),
        &[
            "doc",
            "assets",
            "list",
            issue.short_id.as_str(),
            "report.txt",
            "--json",
        ],
    );
    assert!(assets.status.success());
    let assets: serde_json::Value = serde_json::from_slice(&assets.stdout).unwrap();
    assert!(assets["assets"].as_array().unwrap().is_empty());
}

#[test]
fn test_doc_add_still_refuses_an_escaping_relative_path_beside_the_symbol() {
    let repo = initialized_repo();
    let issue = create_issue(
        repo.path(),
        "Asset escape regression",
        &["type:bug".to_string()],
    );
    fs::write(
        repo.path().join("report.txt"),
        format!(
            "# Attribution\n\nHot symbol: <{DEMANGLED_SYMBOL}>\n\n[bad](../../../etc/passwd)\n"
        ),
    )
    .unwrap();

    let output = jit(
        repo.path(),
        &[
            "doc",
            "add",
            issue.short_id.as_str(),
            "report.txt",
            "--doc-type",
            "report",
        ],
    );
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

    let listed = jit(
        repo.path(),
        &["doc", "list", issue.short_id.as_str(), "--json"],
    );
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(!listed["documents"].as_array().unwrap().is_empty());
    assert!(!listed.to_string().contains(DEMANGLED_SYMBOL));
    assert!(!listed.to_string().contains("../../../etc/passwd"));

    let assets = jit(
        repo.path(),
        &[
            "doc",
            "assets",
            "list",
            issue.short_id.as_str(),
            "report.txt",
            "--json",
        ],
    );
    assert!(assets.status.success());
    let assets: serde_json::Value = serde_json::from_slice(&assets.stdout).unwrap();
    assert!(assets["assets"].as_array().unwrap().is_empty());
}
