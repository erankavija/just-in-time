//! Integration tests for snapshot export CLI

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
use jit::test_harness::git::GitStandIn;

#[test]
fn test_snapshot_export_help() {
    let cli = JitCli::uninitialised().unwrap();
    let mut cmd = cli.command(["snapshot", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot export commands"))
        .stdout(predicate::str::contains("export"));
}

#[test]
fn test_snapshot_export_subcommand_help() {
    let cli = JitCli::uninitialised().unwrap();
    let mut cmd = cli.command(["snapshot", "export", "--help"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Archive a complete snapshot"))
        .stdout(predicate::str::contains("--scope"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--out"));
}

#[test]
fn test_snapshot_export_requires_init() {
    let cli = JitCli::uninitialised().unwrap();

    let mut cmd = cli.command(["snapshot", "export"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_snapshot_export_all_directory() {
    let temp = JitCli::initialised().unwrap();

    // Create a test issue
    temp.command(["issue", "create", "--title", "Test Issue"])
        .assert()
        .success();

    // Export snapshot
    let mut cmd = temp.command(["snapshot", "export", "--out", "test-snapshot"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot exported"))
        .stdout(predicate::str::contains("1 issues"));

    // Verify snapshot structure
    let snapshot_dir = temp.root().join("test-snapshot");
    assert!(snapshot_dir.exists());
    assert!(snapshot_dir.join("manifest.json").exists());
    assert!(snapshot_dir.join("README.md").exists());
    assert!(snapshot_dir.join("checksums.txt").exists());
    assert!(snapshot_dir.join(".jit").exists());
    assert!(snapshot_dir.join(".jit/issues").exists());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(snapshot_dir.join("manifest.json")).unwrap()).unwrap();
    let issues = &manifest["issues"];
    let documents = &manifest["documents"];
    assert_eq!(
        issues["count"].as_u64().unwrap(),
        issues["files"].as_array().unwrap().len() as u64
    );
    assert!(issues["states"].is_object());
    assert_eq!(
        documents["count"].as_u64().unwrap(),
        documents["items"].as_array().unwrap().len() as u64
    );
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_git_mode_reads_all_documents_and_assets_through_one_object_reader() {
    let temp = JitCli::initialised().unwrap();
    temp.init_git().unwrap();

    fs::create_dir_all(temp.root().join("docs")).unwrap();
    fs::write(
        temp.root().join("docs/one.md"),
        "# One\n\n![One](one.png)\n",
    )
    .unwrap();
    fs::write(temp.root().join("docs/one.png"), b"one image").unwrap();
    fs::write(
        temp.root().join("docs/two.md"),
        "# Two\n\n![Two](two.png)\n",
    )
    .unwrap();
    fs::write(temp.root().join("docs/two.png"), b"two image").unwrap();

    let issue = temp.create_issue(["--title", "Snapshot source"]).unwrap();
    for path in ["docs/one.md", "docs/two.md"] {
        temp.command(["doc", "add", &issue, path])
            .assert()
            .success();
    }
    temp.commit("snapshot source").unwrap();

    let paths = ["docs/one.md", "docs/one.png", "docs/two.md", "docs/two.png"];
    let expected_blobs = paths
        .iter()
        .map(|path| {
            (
                *path,
                temp.git(["rev-parse", &format!("HEAD:{path}")])
                    .unwrap()
                    .trim()
                    .to_owned(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let expected_commit = temp.git(["rev-parse", "HEAD"]).unwrap().trim().to_owned();

    let stand_in = GitStandIn::new();
    let mut command = temp.command([
        "snapshot", "export", "--at", "HEAD", "--force", "--out", "snapshot", "--json",
    ]);
    command.env("PATH", stand_in.search_path());
    command.assert().success();

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.root().join("snapshot/manifest.json")).unwrap())
            .unwrap();
    let documents = manifest["documents"]["items"].as_array().unwrap();
    assert_eq!(documents.len(), 2);
    let captured = documents
        .iter()
        .flat_map(|document| {
            std::iter::once(document)
                .chain(document["assets"].as_array().unwrap().iter())
                .map(|file| {
                    (
                        file["path"].as_str().unwrap().to_owned(),
                        file["source"].clone(),
                    )
                })
        })
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(captured.len(), paths.len());
    paths.iter().for_each(|path| {
        assert_eq!(
            captured[*path]["blob_sha"].as_str(),
            expected_blobs.get(path).map(String::as_str),
            "incorrect Git blob id for {path}"
        );
        assert_eq!(
            captured[*path]["commit"].as_str(),
            Some(expected_commit.as_str()),
            "incorrect Git commit for {path}"
        );
    });
    assert_eq!(
        stand_in
            .invocations()
            .iter()
            .filter(|invocation| invocation.args() == ["cat-file", "--batch", "-Z"])
            .count(),
        1
    );
}

#[test]
fn test_snapshot_export_git_mode_fails_with_not_found_for_a_missing_document() {
    let temp = JitCli::initialised().unwrap();
    temp.init_git().unwrap();
    let issue = temp
        .create_issue(["--title", "Missing snapshot document"])
        .unwrap();
    let issue_path = fs::read_dir(temp.root().join(".jit/issues"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.file_stem().is_some_and(|stem| stem == issue.as_str()))
        .unwrap();
    let mut issue_record: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).unwrap()).unwrap();
    issue_record["documents"] = serde_json::json!([{
        "path": "docs/missing.md",
        "commit": null,
        "label": null,
        "doc_type": null,
        "format": "markdown",
        "assets": []
    }]);
    fs::write(
        &issue_path,
        serde_json::to_vec_pretty(&issue_record).unwrap(),
    )
    .unwrap();
    temp.commit("missing snapshot document").unwrap();

    let output = temp
        .command([
            "snapshot", "export", "--at", "HEAD", "--force", "--out", "snapshot",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Path 'docs/missing.md' not found in commit"));
}

#[test]
fn test_snapshot_export_git_mode_fails_with_not_found_for_a_missing_asset() {
    let temp = JitCli::initialised().unwrap();
    temp.init_git().unwrap();
    fs::create_dir_all(temp.root().join("docs")).unwrap();
    fs::write(
        temp.root().join("docs/present.md"),
        "# Present\n\n![Missing](missing.png)\n",
    )
    .unwrap();
    let issue = temp
        .create_issue(["--title", "Missing snapshot asset"])
        .unwrap();
    let issue_path = fs::read_dir(temp.root().join(".jit/issues"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.file_stem().is_some_and(|stem| stem == issue.as_str()))
        .unwrap();
    let mut issue_record: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).unwrap()).unwrap();
    issue_record["documents"] = serde_json::json!([{
        "path": "docs/present.md",
        "commit": null,
        "label": null,
        "doc_type": null,
        "format": "markdown",
        "assets": []
    }]);
    fs::write(
        &issue_path,
        serde_json::to_vec_pretty(&issue_record).unwrap(),
    )
    .unwrap();
    temp.commit("missing snapshot asset").unwrap();

    let output = temp
        .command([
            "snapshot", "export", "--at", "HEAD", "--force", "--out", "snapshot",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Path 'docs/missing.png' not found in commit"));
}

#[test]
fn test_snapshot_export_tar_format() {
    let temp = JitCli::initialised().unwrap();

    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export as tar
    let mut cmd = temp.command(["snapshot", "export", "--format", "tar", "--out", "test.tar"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot exported"))
        .stdout(predicate::str::contains("Archive"));

    // Verify tar file exists
    let tar_file = temp.root().join("test.tar");
    assert!(tar_file.exists());
    assert!(tar_file.metadata().unwrap().len() > 0);
}

#[test]
fn test_snapshot_export_label_scope() {
    let temp = JitCli::initialised().unwrap();

    // Create issues with different labels
    let epic_id = temp
        .create_issue(["--title", "Epic Issue", "--label", "epic:auth"])
        .unwrap();

    let auth_id = temp
        .create_issue(["--title", "Auth Task", "--label", "epic:auth"])
        .unwrap();

    let other_id = temp
        .create_issue(["--title", "Other Task", "--label", "epic:billing"])
        .unwrap();

    // Create dependencies to connect the issues
    temp.command(["dep", "add", &auth_id, &epic_id])
        .assert()
        .success();

    temp.command(["dep", "add", &other_id, &epic_id])
        .assert()
        .success();

    // Export only epic:auth scope
    let mut cmd = temp.command([
        "snapshot",
        "export",
        "--scope",
        "label:epic:auth",
        "--out",
        "auth-snapshot",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("2 issues"));

    let snapshot_dir = temp.root().join("auth-snapshot");
    assert!(snapshot_dir.exists());

    // Verify manifest shows 2 issues
    let manifest_content = fs::read_to_string(snapshot_dir.join("manifest.json")).unwrap();
    assert!(manifest_content.contains("\"count\": 2"));
    assert!(manifest_content.contains("\"scope\": \"label:epic:auth\""));
}

#[test]
fn test_snapshot_export_default_naming() {
    let temp = JitCli::initialised().unwrap();

    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export without --out (should use timestamp-based name)
    let mut cmd = temp.command(["snapshot", "export"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("snapshot-"));
}

#[test]
fn test_snapshot_export_json_output() {
    let temp = JitCli::initialised().unwrap();

    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export with --json
    let mut cmd = temp.command(["snapshot", "export", "--out", "test-json", "--json"]);

    let output = cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("\"path\""));
    let response: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert!(response["path"].is_string());
    assert!(response["message"]
        .as_str()
        .unwrap()
        .starts_with("Exported 1 issues to "));
    assert!(output.get_output().stderr.is_empty());
}

#[test]
fn test_snapshot_export_invalid_scope() {
    let temp = JitCli::initialised().unwrap();

    // Try invalid scope format
    let mut cmd = temp.command(["snapshot", "export", "--scope", "invalid"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Invalid scope"));
}

#[test]
fn test_snapshot_export_output_exists() {
    let temp = JitCli::initialised().unwrap();

    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // First export
    temp.command(["snapshot", "export", "--out", "existing"])
        .assert()
        .success();

    // Try to export to same location
    let mut cmd = temp.command(["snapshot", "export", "--out", "existing"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_snapshot_export_nested_data_and_external_destinations() {
    let outer = TempDir::new().unwrap();
    let repo_cli = JitCli::with_repository(tempfile::tempdir_in(outer.path()).unwrap());
    repo_cli.run(["init"]).unwrap();
    let repo = repo_cli.root().to_path_buf();
    repo_cli
        .command(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::create_dir(repo.join(".jit/exports")).unwrap();

    repo_cli
        .command(["snapshot", "export", "--out", ".jit/exports/snapshot"])
        .assert()
        .success();
    assert!(repo.join(".jit/exports/snapshot/manifest.json").is_file());

    #[cfg(target_os = "linux")]
    {
        let external_directory = outer.path().join("external-snapshot");
        repo_cli
            .command([
                "snapshot",
                "export",
                "--out",
                external_directory.to_str().unwrap(),
            ])
            .assert()
            .success();
        assert!(external_directory.join("manifest.json").is_file());
    }

    let external = outer.path().join("external.tar");
    repo_cli
        .command([
            "snapshot",
            "export",
            "--format",
            "tar",
            "--out",
            external.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(external.is_file());
    let original = fs::read(&external).unwrap();
    repo_cli
        .command([
            "snapshot",
            "export",
            "--format",
            "tar",
            "--out",
            external.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(fs::read(&external).unwrap(), original);
    assert!(fs::read_dir(outer.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_repository_symlink_alias() {
    use std::os::unix::fs::symlink;

    let outer = TempDir::new().unwrap();
    let repo_cli = JitCli::with_repository(tempfile::tempdir_in(outer.path()).unwrap());
    repo_cli.run(["init"]).unwrap();
    let repo = repo_cli.root().to_path_buf();
    let external = outer.path().join("external");
    fs::create_dir(&external).unwrap();
    repo_cli
        .command(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    symlink(&external, repo.join("alias")).unwrap();

    repo_cli
        .command(["snapshot", "export", "--out", "alias/snapshot"])
        .assert()
        .failure();
    assert!(!external.join("snapshot").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_external_symlink_alias_into_repository() {
    use std::os::unix::fs::symlink;

    let outer = TempDir::new().unwrap();
    let repo_cli = JitCli::with_repository(tempfile::tempdir_in(outer.path()).unwrap());
    repo_cli.run(["init"]).unwrap();
    let repo = repo_cli.root().to_path_buf();
    repo_cli
        .command(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    let alias = outer.path().join("alias");
    symlink(&repo, &alias).unwrap();

    repo_cli
        .command([
            "snapshot",
            "export",
            "--out",
            alias.join("snapshot").to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(!repo.join("snapshot").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_symlinked_document_source() {
    use std::os::unix::fs::symlink;

    let temp = JitCli::initialised().unwrap();
    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::create_dir(temp.root().join("docs")).unwrap();
    fs::write(temp.root().join("real.md"), "secret").unwrap();
    symlink("../real.md", temp.root().join("docs/link.md")).unwrap();
    let issue_path = fs::read_dir(temp.root().join(".jit/issues"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let mut issue: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).unwrap()).unwrap();
    issue["documents"] = serde_json::json!([{
        "path": "docs/link.md",
        "commit": null,
        "label": null,
        "doc_type": null,
        "format": "markdown",
        "assets": []
    }]);
    fs::write(&issue_path, serde_json::to_vec_pretty(&issue).unwrap()).unwrap();

    temp.command([
        "snapshot",
        "export",
        "--working-tree",
        "--force",
        "--out",
        "snapshot",
    ])
    .assert()
    .success()
    .stderr(predicate::str::contains("Warning"));
    assert!(!temp.root().join("snapshot/docs/link.md").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_symlinked_data_source_without_output() {
    use std::os::unix::fs::symlink;

    let temp = JitCli::initialised().unwrap();
    temp.command(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::remove_file(temp.root().join(".jit/config.toml")).unwrap();
    fs::write(temp.root().join("outside.toml"), "secret = true").unwrap();
    symlink("../outside.toml", temp.root().join(".jit/config.toml")).unwrap();

    temp.command([
        "snapshot",
        "export",
        "--force",
        "--working-tree",
        "--out",
        "snapshot",
    ])
    .assert()
    .failure();
    assert!(!temp.root().join("snapshot").exists());
}
