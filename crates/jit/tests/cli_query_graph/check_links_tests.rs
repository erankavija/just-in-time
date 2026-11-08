//! Integration tests for jit doc check-links command
//!
//! Tests follow acceptance criteria from issue fb6e2e31:
//! 1. Validate all documents in scope
//! 2. Check asset existence (working tree or git)
//! 3. Validate internal doc links resolve
//! 4. Detect broken relative vs root-relative links
//! 5. Shared issue scope filtering
//! 6. Exit codes (0=valid or warnings only, 4=broken links)
//! 7. JSON output support

use jit::test_cli::JitCli;
use jit::test_harness::git::GitStandIn;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// Serve and count loopback requests without external network access.
fn spawn_http_server(status: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock HTTP server");
    let addr = listener.local_addr().expect("mock server local addr");
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // Drain the request head (a HEAD request carries no body), then
            // reply with the configured status and close.
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = stream.write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            );
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), requests)
}

#[test]
fn test_check_all_valid_links_succeeds() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    // Create issue with document that has valid assets
    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Test description"])
        .unwrap();

    // Create a markdown document with valid local asset
    let doc_path = "docs/test-doc.md";
    let asset_path = "docs/assets/diagram.png";

    fs::create_dir_all(cli.root().join("docs/assets")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "# Test Doc\n\n![Diagram](assets/diagram.png)\n",
    )
    .unwrap();
    fs::write(cli.root().join(asset_path), b"fake-png-data").unwrap();

    // Add document with scanned assets
    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();
    let stdout = cli.run(["doc", "check-links", "--scope", "all"]).unwrap();
    assert!(stdout.contains("All documents valid"));
}

#[test]
fn test_missing_assets_fails() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Test description"])
        .unwrap();

    // Create document referencing missing asset
    let doc_path = "docs/broken-doc.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "# Broken Doc\n\n![Missing](assets/missing.png)\n",
    )
    .unwrap();

    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();
    let output = cli
        .command(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Errors found"));
    assert!(stdout.contains("missing"));
}

#[test]
fn test_broken_internal_doc_links_fails() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Test description"])
        .unwrap();

    // Create document with broken link to another doc
    let doc_path = "docs/doc-with-broken-link.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "# Doc\n\nSee [missing doc](nonexistent.md) for details.\n",
    )
    .unwrap();

    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();
    let output = cli
        .command(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Errors found"));
    assert!(stdout.contains("broken"));
}

#[test]
fn test_check_links_broken_link_json_is_validation_failed_envelope_exit_4_with_report_details() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Test description"])
        .unwrap();
    let doc_path = "docs/doc-with-broken-link.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "# Doc\n\nSee [missing doc](nonexistent.md) for details.\n",
    )
    .unwrap();
    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();

    let output = cli
        .command(["doc", "check-links", "--scope", "all", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "VALIDATION_FAILED");
    let details = &envelope["error"]["details"];
    assert_eq!(details["valid"], false);
    assert!(!details["errors"].as_array().unwrap().is_empty());
}

#[test]
fn test_risky_relative_links_warns() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Test description"])
        .unwrap();

    // Create document with risky (but valid) relative path
    let doc_path = "docs/subdir/risky-doc.md";
    fs::create_dir_all(cli.root().join("docs/subdir")).unwrap();
    fs::create_dir_all(cli.root().join("assets")).unwrap();

    // Deep relative path that exists but is fragile
    fs::write(
        cli.root().join(doc_path),
        "# Risky\n\n![Asset](../../assets/image.png)\n",
    )
    .unwrap();
    fs::write(cli.root().join("assets/image.png"), b"data").unwrap();

    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();

    // Warnings alone are reported and do not fail the check.
    let output = cli
        .command(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Warnings"));
    assert!(stdout.contains("risky") || stdout.contains("relative"));
    let json = cli
        .run_json(["doc", "check-links", "--scope", "all"])
        .unwrap();
    assert!(!json["link_warnings"].as_array().unwrap().is_empty());
    assert!(json["errors"].as_array().unwrap().is_empty());
}

#[test]
fn test_scope_filtering_all() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    // Create multiple issues with documents
    let issue1 = cli
        .create_issue(["--title", "Issue 1", "--description", "Description"])
        .unwrap();
    let issue2 = cli
        .create_issue(["--title", "Issue 2", "--description", "Description"])
        .unwrap();

    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/doc1.md"), "# Doc 1").unwrap();
    fs::write(cli.root().join("docs/doc2.md"), "# Doc 2").unwrap();

    cli.run(["doc", "add", &issue1, "docs/doc1.md", "--label", "Doc1"])
        .unwrap();
    cli.run(["doc", "add", &issue2, "docs/doc2.md", "--label", "Doc2"])
        .unwrap();

    // Scope all should check both documents
    let stdout = cli.run(["doc", "check-links", "--scope", "all"]).unwrap();
    assert!(stdout.contains("2 document(s)"));
}

#[test]
fn test_scope_filtering_issue_id() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue1 = cli
        .create_issue(["--title", "Issue 1", "--description", "Description"])
        .unwrap();
    let issue2 = cli
        .create_issue(["--title", "Issue 2", "--description", "Description"])
        .unwrap();

    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/doc1.md"), "# Doc 1").unwrap();
    fs::write(cli.root().join("docs/doc2.md"), "# Doc 2").unwrap();

    cli.run(["doc", "add", &issue1, "docs/doc1.md", "--label", "Doc1"])
        .unwrap();
    cli.run(["doc", "add", &issue2, "docs/doc2.md", "--label", "Doc2"])
        .unwrap();

    // Scope issue:ID should check only that issue's documents
    let scope = format!("issue:{}", &issue1[..8]);
    let stdout = cli.run(["doc", "check-links", "--scope", &scope]).unwrap();
    assert!(stdout.contains("1 document(s)"));
}

#[test]
fn test_check_links_label_scope_checks_only_matching_issue_documents() {
    let cli = JitCli::initialised().unwrap();
    let selected = cli
        .create_issue(["Selected", "--label", "epic:auth"])
        .unwrap();
    let excluded = cli
        .create_issue(["Excluded", "--label", "epic:billing"])
        .unwrap();
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/selected.md"), "# Valid").unwrap();
    fs::write(cli.root().join("docs/excluded.md"), "[Broken](missing.md)").unwrap();
    cli.run(["doc", "add", &selected, "docs/selected.md"])
        .unwrap();
    cli.run(["doc", "add", &excluded, "docs/excluded.md"])
        .unwrap();

    let report = cli
        .run_json(["doc", "check-links", "--scope", "label:epic:auth"])
        .unwrap();

    assert_eq!(report["summary"]["total_documents"], 1);
    assert_eq!(report["valid"], true);
}

#[test]
fn test_json_output_structure() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Description"])
        .unwrap();

    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/test.md"), "# Test").unwrap();

    cli.run(["doc", "add", &issue_id, "docs/test.md", "--label", "Test"])
        .unwrap();

    // JSON output should have expected structure
    let json = cli
        .run_json(["doc", "check-links", "--scope", "all"])
        .unwrap();

    // Validate JSON structure
    // success field removed
    assert!(json["valid"].is_boolean());
    assert!(json["errors"].is_array());
    assert!(json["link_warnings"].is_array());
    assert!(json["summary"].is_object());
    assert!(json["summary"]["total_documents"].is_number());
}

#[test]
fn test_invalid_scope_format() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let output = cli
        .command(["doc", "check-links", "--scope", "invalid", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_empty_repository_no_documents() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    // No documents should exit cleanly
    let stdout = cli.run(["doc", "check-links", "--scope", "all"]).unwrap();
    assert!(stdout.contains("No documents found") || stdout.contains("0 document"));
}

#[test]
fn test_external_urls_are_validated() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Description"])
        .unwrap();

    // Document with a reachable external URL, served by a local loopback mock so
    // the test is deterministic and offline-safe (no dependency on httpbin.org).
    let (base, _) = spawn_http_server("200 OK");
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join("docs/external.md"),
        format!("# External\n\n![Remote]({base}/status/200)\n"),
    )
    .unwrap();

    cli.run([
        "doc",
        "add",
        &issue_id,
        "docs/external.md",
        "--label",
        "Test",
    ])
    .unwrap();

    // A reachable external URL should validate successfully (no warnings/errors).
    let output = cli
        .command(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("valid"));
}

#[test]
fn test_check_document_links_probes_shared_url_once_and_reports_each_document() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Shared URL", "--description", "Description"])
        .unwrap();
    let (base, requests) = spawn_http_server("404 Not Found");
    let url = format!("{base}/missing.png");
    let documents = ["first.md", "second.md"];
    for document in documents {
        fs::write(cli.root().join(document), format!("![Remote]({url})\n")).unwrap();
        cli.run(["doc", "add", &issue_id, document]).unwrap();
    }

    let report = check_links_report(&cli);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    for document in documents {
        let errors = errors_for_document(&report, document);
        assert_eq!(errors.len(), 1, "{report}");
        assert_eq!(errors[0]["type"], "unreachable_url");
        assert_eq!(errors[0]["asset"], url);
    }
}

#[test]
fn test_missing_document_file() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Description"])
        .unwrap();

    // Create and add document
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/temp.md"), "# Temp").unwrap();

    cli.run(["doc", "add", &issue_id, "docs/temp.md", "--label", "Test"])
        .unwrap();

    // Delete the document file
    fs::remove_file(cli.root().join("docs/temp.md")).unwrap();

    // Check should report missing document
    let output = cli
        .command(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("missing_document") || stdout.contains("not found"));
}

#[test]
fn test_directory_link_target_is_navigation_not_a_missing_asset_or_scan_error() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Directory link", "--description", "Description"])
        .unwrap();
    fs::create_dir_all(cli.root().join("dev/studies")).unwrap();
    fs::create_dir_all(cli.root().join("dev/notes")).unwrap();
    fs::write(
        cli.root().join("dev/notes/guide.md"),
        "See [the studies area](../studies).\n",
    )
    .unwrap();

    cli.run(["doc", "add", &issue_id, "dev/notes/guide.md"])
        .unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    assert!(
        errors_for_document(&report, "dev/notes/guide.md")
            .iter()
            .all(|error| error["type"] != "missing_asset" && error["type"] != "broken_link"),
        "an existing directory is navigation, not a missing asset: {report}"
    );
    assert!(
        report["link_warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .all(|warning| warning["type"] != "scan_error"),
        "a directory target must not make link scanning fail: {report}"
    );
}

#[test]
fn test_validate_reports_direct_directory_reference_as_unsupported_artifact_type() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Directory reference",
            "--description",
            "Description",
        ])
        .unwrap();
    fs::create_dir_all(cli.root().join("dev/studies")).unwrap();

    cli.run(["doc", "add", &issue_id, "dev/studies"]).unwrap();

    let output = cli.command(["validate"]).output().unwrap();
    assert!(!output.status.success());
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("unsupported artifact type"));
    assert!(report.contains("dev/studies"));
    assert!(!report.contains("not found"));
}

#[test]
fn test_pinned_directory_link_target_is_navigation_at_its_commit() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned directory link",
            "--description",
            "Description",
        ])
        .unwrap();
    fs::create_dir_all(cli.root().join("docs/studies")).unwrap();
    fs::write(cli.root().join("docs/studies/.keep"), "tracked directory\n").unwrap();
    fs::write(
        cli.root().join("docs/report.md"),
        "See [the studies area](studies).\n",
    )
    .unwrap();
    cli.commit("add pinned directory navigation").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, "docs/report.md", "--commit", &pin])
        .unwrap();

    fs::remove_dir_all(cli.root().join("docs/studies")).unwrap();
    fs::write(cli.root().join("docs/studies"), "now a file\n").unwrap();
    cli.commit("replace navigation directory").unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    assert!(
        errors_for_document(&report, "docs/report.md").is_empty(),
        "the pinned directory target must remain navigation: {report}"
    );
    assert!(
        report["link_warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .all(|warning| warning["type"] != "scan_error"),
        "the pinned directory target must not produce a scan error: {report}"
    );
}

#[test]
fn test_pinned_directory_reference_is_unsupported_at_its_commit() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned directory reference",
            "--description",
            "Description",
        ])
        .unwrap();
    fs::create_dir_all(cli.root().join("dev/studies")).unwrap();
    fs::write(cli.root().join("dev/studies/.keep"), "tracked directory\n").unwrap();
    cli.commit("add pinned directory reference").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, "dev/studies", "--commit", &pin])
        .unwrap();

    fs::remove_dir_all(cli.root().join("dev/studies")).unwrap();
    fs::write(cli.root().join("dev/studies"), "now a file\n").unwrap();
    cli.commit("replace pinned directory reference").unwrap();

    let output = cli.command(["validate"]).output().unwrap();
    assert!(!output.status.success());
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("unsupported artifact type"));
    assert!(report.contains("dev/studies"));
    assert!(!report.contains("not found"));
}

#[cfg(unix)]
#[test]
fn test_pinned_symlink_target_is_unsupported_at_its_commit() {
    use std::os::unix::fs::symlink;

    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned symlink target",
            "--description",
            "Description",
        ])
        .unwrap();
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join("docs/target.md"), "Pinned target\n").unwrap();
    fs::write(
        cli.root().join("docs/report.md"),
        "See [the linked target](linked.md).\n",
    )
    .unwrap();
    symlink("target.md", cli.root().join("docs/linked.md")).unwrap();
    cli.commit("add pinned symlink target").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, "docs/report.md", "--commit", &pin])
        .unwrap();

    fs::remove_file(cli.root().join("docs/linked.md")).unwrap();
    fs::write(
        cli.root().join("docs/linked.md"),
        "This ordinary working-tree file must not be used.\n",
    )
    .unwrap();
    cli.commit("replace symlink with regular file").unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, "docs/report.md");
    assert!(
        errors
            .iter()
            .any(|error| error["type"] == "unsupported_asset"),
        "the pinned symlink must be diagnosed as unsupported: {report}"
    );
    assert!(
        errors.iter().any(|error| {
            error["type"] == "broken_link"
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("unsupported artifact type"))
        }),
        "the pinned symlink must not resolve through the current regular file: {report}"
    );
}

#[test]
fn test_git_versioned_asset_exists() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Test issue", "--description", "Description"])
        .unwrap();

    // Create document with asset, commit it
    fs::create_dir_all(cli.root().join("docs/assets")).unwrap();
    let doc_path = "docs/versioned.md";
    let asset_path = "docs/assets/old.png";

    fs::write(
        cli.root().join(doc_path),
        "# Versioned\n\n![Old](assets/old.png)\n",
    )
    .unwrap();
    fs::write(cli.root().join(asset_path), b"old-data").unwrap();

    // Commit to git
    cli.commit("Add document with asset").unwrap();

    // Add document reference
    cli.run(["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .unwrap();

    // Remove asset from working tree
    fs::remove_file(cli.root().join(asset_path)).unwrap();

    // Check should pass because asset exists in git
    let stdout = cli.run(["doc", "check-links", "--scope", "all"]).unwrap();
    assert!(stdout.contains("valid"));
}

#[test]
fn test_check_document_links_without_git_counts_only_working_tree_assets() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Local asset without Git"])
        .unwrap();
    let doc_path = "docs/local.md";
    let asset_path = "docs/assets/local.png";
    fs::create_dir_all(cli.root().join("docs/assets")).unwrap();
    fs::write(cli.root().join(doc_path), "![Local](assets/local.png)\n").unwrap();
    fs::write(cli.root().join(asset_path), b"image").unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();
    let present = check_links_report(&cli);
    assert!(
        errors_for_document(&present, doc_path).is_empty(),
        "{present}"
    );

    fs::remove_file(cli.root().join(asset_path)).unwrap();
    let missing = check_links_report(&cli);
    assert!(
        errors_for_document(&missing, doc_path)
            .iter()
            .any(|error| error["type"] == "missing_asset" && error["resolved"] == asset_path),
        "{missing}"
    );
}

#[test]
fn test_check_document_links_reports_head_gitlink_missing_with_or_without_target_object() {
    [false, true].into_iter().for_each(|target_available| {
        let cli = JitCli::uninitialised().unwrap();
        cli.init_git().unwrap().run(["init"]).unwrap();
        let issue_id = cli.create_issue(["--title", "Submodule asset"]).unwrap();
        let doc_path = "docs/submodule.md";
        let asset_path = "docs/assets/module";
        fs::create_dir_all(cli.root().join("docs/assets")).unwrap();
        fs::write(
            cli.root().join(doc_path),
            "# Submodule\n\n![Module](assets/module)\n",
        )
        .unwrap();
        fs::write(cli.root().join(asset_path), b"placeholder").unwrap();
        cli.commit("record original asset").unwrap();
        let available_target = cli.git(["rev-parse", "HEAD"]).unwrap();
        cli.run(["doc", "add", &issue_id, doc_path]).unwrap();
        fs::remove_file(cli.root().join(asset_path)).unwrap();
        let target = if target_available {
            available_target.trim()
        } else {
            "1111111111111111111111111111111111111111"
        };
        let cacheinfo = format!("160000,{target},{asset_path}");
        cli.git(["update-index", "--add", "--cacheinfo", &cacheinfo])
            .unwrap();
        cli.git(["commit", "-q", "-m", "record submodule gitlink"])
            .unwrap();
        let report = check_links_report(&cli);
        assert!(report["errors"].is_array(), "{report}");
        assert!(
            errors_for_document(&report, doc_path)
                .iter()
                .any(|error| error["type"] == "missing_asset" && error["resolved"] == asset_path),
            "gitlink target_available={target_available} must be missing: {report}"
        );
    });
}

#[test]
fn test_check_document_links_batches_head_asset_presence_into_one_cat_file_process() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();
    let issue_id = cli.create_issue(["--title", "Versioned assets"]).unwrap();
    let doc_path = "docs/batched.md";
    let assets = ["one.png", "two.png", "three.png"];
    fs::create_dir_all(cli.root().join("docs/assets")).unwrap();
    let document = assets
        .iter()
        .map(|asset| format!("![{asset}](assets/{asset})"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(cli.root().join(doc_path), document).unwrap();
    assets.iter().for_each(|asset| {
        fs::write(cli.root().join("docs/assets").join(asset), b"image").unwrap();
    });
    cli.commit("add document and assets").unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();
    assets.iter().for_each(|asset| {
        fs::remove_file(cli.root().join("docs/assets").join(asset)).unwrap();
    });
    let stand_in = GitStandIn::new();
    let output = cli
        .command_with_env(
            ["doc", "check-links", "--scope", "all", "--json"],
            [("PATH", stand_in.search_path())],
        )
        .output()
        .unwrap();
    // Count document-reading commit resolution and the single HEAD asset-presence batch.
    let cat_file_calls = stand_in
        .invocations()
        .into_iter()
        .filter(|call| call.args() == ["cat-file", "--batch-check", "-Z"])
        .collect::<Vec<_>>();
    assert_eq!(cat_file_calls.len(), 2, "{cat_file_calls:?}");
    assert!(
        output.status.success(),
        "committed assets must remain present: {output:?}"
    );
}

/// The link report of `doc check-links --json`: the failure envelope's
/// `error.details` when a link is broken, the success payload otherwise.
fn check_links_report(cli: &JitCli) -> serde_json::Value {
    let output = cli
        .command(["doc", "check-links", "--scope", "all", "--json"])
        .output()
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    envelope
        .pointer("/error/details")
        .cloned()
        .unwrap_or(envelope)
}

/// Every error a report attributes to one document, whatever its type.
fn errors_for_document<'a>(
    report: &'a serde_json::Value,
    document: &str,
) -> Vec<&'a serde_json::Value> {
    report["errors"]
        .as_array()
        .expect("errors array")
        .iter()
        .filter(|error| error["document"].as_str() == Some(document))
        .collect()
}

#[test]
fn test_check_document_links_resolves_pinned_document_deleted_from_working_tree() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue(["--title", "Pinned document", "--description", "Description"])
        .unwrap();
    let doc_path = "docs/pinned.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Pinned\n\nBody.\n").unwrap();
    cli.commit("add pinned document").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, doc_path, "--commit", &pin])
        .unwrap();

    fs::remove_file(cli.root().join(doc_path)).unwrap();
    cli.commit("delete pinned document").unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, doc_path);
    assert!(
        errors.is_empty(),
        "a reference pinned to a commit holding its file must resolve, got: {errors:?}"
    );
    assert_eq!(
        errors.is_empty(),
        cli.run(["validate"]).is_ok(),
        "link checking and whole-repository validation must classify the pinned reference alike"
    );
}

/// A pinned reference's assets and internal links name the same version the
/// reference does, so a document whose whole neighbourhood left the working tree
/// still resolves.
#[test]
fn test_check_document_links_resolves_pinned_assets_and_links_at_the_pin() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned document with neighbours",
            "--description",
            "Description",
        ])
        .unwrap();
    let doc_path = "docs/report.md";
    let asset_path = "docs/diagram.png";
    let link_target = "docs/appendix.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "# Report\n\n![Diagram](diagram.png)\n\nSee [the appendix](appendix.md).\n",
    )
    .unwrap();
    fs::write(cli.root().join(asset_path), b"png-bytes").unwrap();
    fs::write(cli.root().join(link_target), "# Appendix\n").unwrap();
    cli.commit("add report with asset and appendix").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, doc_path, "--commit", &pin])
        .unwrap();

    fs::remove_file(cli.root().join(doc_path)).unwrap();
    fs::remove_file(cli.root().join(asset_path)).unwrap();
    fs::remove_file(cli.root().join(link_target)).unwrap();
    cli.commit("delete the report and its neighbours").unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, doc_path);
    assert!(
        errors.is_empty(),
        "a pinned document's assets and links resolve at its pin, got: {errors:?}"
    );
    assert_eq!(
        errors.is_empty(),
        cli.run(["validate"]).is_ok(),
        "link checking and whole-repository validation must classify the pinned reference alike"
    );
}

#[test]
fn test_check_document_links_rejects_pinned_document_absent_at_its_commit() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    fs::write(cli.root().join("README.md"), "# Repo\n").unwrap();
    cli.commit("initial commit").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    let issue_id = cli
        .create_issue([
            "--title",
            "Reference pinned before its file",
            "--description",
            "Description",
        ])
        .unwrap();
    let doc_path = "docs/later.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Later\n").unwrap();
    cli.commit("add the document after the pinned commit")
        .unwrap();

    // The file is in the working tree, but not at the commit the reference names.
    cli.run(["doc", "add", &issue_id, doc_path, "--commit", &pin])
        .unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, doc_path);
    assert!(
        !errors.is_empty(),
        "a reference pinned to a commit without its file must not resolve"
    );
    assert_eq!(
        errors.is_empty(),
        cli.run(["validate"]).is_ok(),
        "link checking and whole-repository validation must classify the pinned reference alike"
    );
}

/// Without Git a pin has no object database to resolve against, so the reference
/// is reported unresolved — the same answer whole-repository validation gives.
#[test]
fn test_check_document_links_reports_pinned_document_without_git() {
    let cli = JitCli::initialised().unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned without git",
            "--description",
            "Description",
        ])
        .unwrap();
    let doc_path = "docs/pinned.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Pinned\n").unwrap();

    cli.run([
        "doc",
        "add",
        &issue_id,
        doc_path,
        "--commit",
        "0123456789abcdef0123456789abcdef01234567",
    ])
    .unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, doc_path);
    assert!(
        !errors.is_empty(),
        "an unresolvable pin must be reported even though the file is in the working tree"
    );
    assert_eq!(
        errors.is_empty(),
        cli.run(["validate"]).is_ok(),
        "link checking and whole-repository validation must classify the pinned reference alike"
    );
}

/// A link target that names no repository path is read as absent, rather than
/// failing the check that would have reported it.
#[test]
fn test_check_document_links_reports_pinned_link_target_outside_the_repository() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Pinned document linking outward",
            "--description",
            "Description",
        ])
        .unwrap();
    let doc_path = "notes.md";
    fs::write(
        cli.root().join(doc_path),
        "# Notes\n\nSee [the parent](../) for context.\n",
    )
    .unwrap();
    cli.commit("add notes").unwrap();
    let pin = cli.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    cli.run(["doc", "add", &issue_id, doc_path, "--commit", &pin])
        .unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    assert!(
        report["errors"].is_array(),
        "the check completes and reports, rather than failing"
    );
    assert!(
        errors_for_document(&report, doc_path)
            .iter()
            .any(|error| error["type"] == "broken_link"),
        "a target outside the repository is reported broken"
    );
}

/// An unpinned reference keeps resolving from the working tree when Git is
/// absent, so link checking stays usable without Git.
#[test]
fn test_check_document_links_resolves_unpinned_document_without_git() {
    let cli = JitCli::initialised().unwrap();

    let issue_id = cli
        .create_issue([
            "--title",
            "Unpinned without git",
            "--description",
            "Description",
        ])
        .unwrap();
    let doc_path = "docs/current.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Current\n").unwrap();

    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();

    let report: serde_json::Value = check_links_report(&cli);
    let errors = errors_for_document(&report, doc_path);
    assert!(
        errors.is_empty(),
        "an unpinned reference resolves from the working tree, got: {errors:?}"
    );
}
