//! Integration tests for the read-only `jit doc` commands
//!
//! Tests for:
//! - doc show without git (filesystem fallback)
//! - doc show with git
//! - doc show with --at commit
//! - doc dir, the canonical artifact directory resolver

use assert_cmd::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

#[test]
fn test_doc_show_without_git() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path();

    // Initialize jit (without git)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .arg("init")
        .assert()
        .success();

    // Create an issue
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["issue", "create", "--title", "Test Issue", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = json["id"].as_str().unwrap();
    let short_id = &issue_id[..8];

    // Create a document file
    let doc_path = temp_path.join("design.md");
    fs::write(&doc_path, "# Design Document\n\nThis is a test document.").unwrap();

    // Add document reference to issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args([
            "doc",
            "add",
            short_id,
            "design.md",
            "--label",
            "Design Doc",
            "--doc-type",
            "design",
        ])
        .assert()
        .success();

    // Show document without git - should read from filesystem
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "doc show should work without git: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Design Document"));
    assert!(stdout.contains("This is a test document"));
}

#[test]
fn test_doc_show_with_git() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path();

    // Initialize git repo
    Command::new("git")
        .current_dir(temp_path)
        .args(["init"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["config", "user.name", "Test User"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["config", "user.email", "test@example.com"])
        .assert()
        .success();

    // Initialize jit
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .arg("init")
        .assert()
        .success();

    // Create an issue
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["issue", "create", "--title", "Test Issue", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = json["id"].as_str().unwrap();
    let short_id = &issue_id[..8];

    // Create and commit a document
    let doc_path = temp_path.join("design.md");
    fs::write(&doc_path, "# Design Document\n\nCommitted content.").unwrap();

    Command::new("git")
        .current_dir(temp_path)
        .args(["add", "design.md"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["commit", "-m", "Add design doc"])
        .assert()
        .success();

    // Add document reference
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "add", short_id, "design.md"])
        .assert()
        .success();

    // Show document - should read from git
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Design Document"));
    assert!(stdout.contains("Committed content"));
}

/// Run the built binary in `repo` and hand back the whole outcome, so a case can
/// assert on exit status, stdout, and stderr together.
fn jit(repo: &Path, args: &[&str]) -> Output {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}

/// An initialized repository with no Git history. `doc dir` derives a path from
/// configuration and the issue record alone, so Git has nothing to contribute.
fn initialized_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let init = jit(temp.path(), &["init"]);
    assert!(
        init.status.success(),
        "jit init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    temp
}

/// The identifiers a created issue answers to.
struct CreatedIssue {
    id: String,
    short_id: String,
}

fn create_issue(repo: &Path, title: &str, labels: &[String]) -> CreatedIssue {
    let mut args = vec!["issue", "create", "--title", title, "--json"];
    args.extend(labels.iter().flat_map(|label| ["--label", label.as_str()]));
    let output = jit(repo, &args);
    assert!(
        output.status.success(),
        "issue create {title}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    CreatedIssue {
        id: created["id"].as_str().unwrap().to_string(),
        short_id: created["short_id"].as_str().unwrap().to_string(),
    }
}

/// An area a fresh repository declares issue-scoped. `jit init` scaffolds the
/// shipped policy into `.jit/config.toml`, so reading the policy states which
/// areas that repository declares without restating the registry here.
fn declared_area() -> &'static str {
    jit::config::SHIPPED_DOCUMENTATION_POLICY
        .issue_scoped_areas
        .first()
        .copied()
        .expect("the shipped policy declares at least one issue-scoped area")
}

/// A development area the shipped policy manages but leaves outside the
/// convention. A real area rather than an invented path, so rejecting it can
/// only come from the registry.
fn undeclared_area() -> &'static str {
    jit::config::SHIPPED_DOCUMENTATION_POLICY
        .managed_paths
        .iter()
        .copied()
        .find(|path| {
            !jit::config::SHIPPED_DOCUMENTATION_POLICY
                .issue_scoped_areas
                .contains(path)
        })
        .expect("the shipped policy manages an area outside the convention")
}

/// A type the scaffolded hierarchy maps to a membership namespace, with that
/// namespace. `jit init` applies this template, so an issue carrying
/// `type:<type>` and `<namespace>:<value>` names a single membership value.
fn membership_type_and_namespace() -> (String, String) {
    let mut associations = jit::hierarchy_templates::HierarchyTemplate::default()
        .label_associations
        .into_iter()
        .collect::<Vec<_>>();
    associations.sort();
    associations
        .into_iter()
        .next()
        .expect("the scaffolded hierarchy maps a type to a membership namespace")
}

/// The directory `doc dir` names for `id` in `area`, requiring the run to
/// succeed.
fn resolve_directory(repo: &Path, id: &str, area: &str) -> String {
    let output = jit(repo, &["doc", "dir", id, area]);
    assert!(
        output.status.success(),
        "doc dir {id} {area}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn test_doc_dir_names_the_directory_the_membership_precedence_resolves_inside_a_declared_area() {
    let repo = initialized_repo();
    let area = declared_area();
    let (issue_type, namespace) = membership_type_and_namespace();
    let membership = "artifact-layout";

    let member = create_issue(
        repo.path(),
        "Membership-labelled issue",
        &[
            format!("type:{issue_type}"),
            format!("{namespace}:{membership}"),
        ],
    );
    let bare = create_issue(repo.path(), "Issue naming no membership value", &[]);

    let member_directory = resolve_directory(repo.path(), &member.short_id, area);
    let bare_directory = resolve_directory(repo.path(), &bare.short_id, area);

    [(&member, &member_directory), (&bare, &bare_directory)]
        .iter()
        .for_each(|(issue, directory)| {
            let (parent, name) = directory
                .rsplit_once('/')
                .unwrap_or_else(|| panic!("{directory} sits inside an area"));
            assert_eq!(parent, area, "{directory} sits directly in the named area");
            assert!(
                name.starts_with(&issue.short_id),
                "{directory} is named for the issue that owns it"
            );
            assert!(
                !repo.path().join(directory).exists(),
                "{directory} is named, not created"
            );
        });

    // The two issues differ only in their membership labels, so the precedence
    // the resolver applies is what separates the two directory names.
    assert_eq!(
        bare_directory,
        format!("{area}/{}", bare.short_id),
        "an issue naming no membership value owns the bare short-id directory"
    );
    assert!(
        member_directory.ends_with(&format!("-{membership}")),
        "a single resolved membership value suffixes the short id: {member_directory}"
    );
}

#[test]
fn test_doc_dir_rejects_an_area_the_configured_registry_does_not_declare() {
    let repo = initialized_repo();
    let area = declared_area();
    let issue = create_issue(repo.path(), "Undeclared area", &[]);

    // The same repository resolves a declared area, so a rejection below is the
    // registry talking rather than the command being absent.
    let declared_directory = resolve_directory(repo.path(), &issue.short_id, area);

    [
        (
            "a development area outside the convention",
            undeclared_area().to_string(),
        ),
        (
            "a path inside a declared area",
            declared_directory.clone(),
        ),
        (
            "a sibling whose name starts with a declared area",
            format!("{area}-elsewhere"),
        ),
    ]
    .iter()
    .for_each(|(shape, undeclared)| {
        let output = jit(repo.path(), &["doc", "dir", &issue.short_id, undeclared]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "{shape} must not resolve a directory: {undeclared}"
        );
        assert!(
            stderr.contains(undeclared.as_str()),
            "the error for {shape} names the offending area: {stderr}"
        );
        assert!(
            stderr.contains(area),
            "the error for {shape} names the registry it was matched against: {stderr}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).trim().is_empty(),
            "{shape} prints no path"
        );
    });
}

#[test]
fn test_doc_dir_resolves_the_area_registry_from_repository_configuration() {
    let repo = initialized_repo();
    let scaffolded = declared_area();
    let authored = undeclared_area();
    let issue = create_issue(repo.path(), "Configured registry", &[]);

    let accepts = |area: &str| {
        jit(repo.path(), &["doc", "dir", &issue.short_id, area])
            .status
            .success()
    };

    assert!(
        accepts(scaffolded) && !accepts(authored),
        "the scaffolded registry declares {scaffolded} and not {authored}"
    );

    // Replace the declared registry with one naming the other area. Nothing but
    // configuration changes, so a flip in what resolves is the command reading
    // the repository rather than a vocabulary compiled into the binary.
    let config_path = repo.path().join(".jit").join("config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    let declaration = "issue_scoped_areas = [";
    let start = config
        .find(declaration)
        .expect("the scaffolded configuration declares the registry");
    let end = start
        + config[start..]
            .find(']')
            .expect("the registry declaration is closed")
        + 1;
    fs::write(
        &config_path,
        format!(
            "{}{declaration}\"{authored}\"]{}",
            &config[..start],
            &config[end..]
        ),
    )
    .unwrap();

    assert!(
        accepts(authored) && !accepts(scaffolded),
        "an authored registry replaces the scaffolded one"
    );
    assert!(
        resolve_directory(repo.path(), &issue.short_id, authored)
            .starts_with(&format!("{authored}/")),
        "the resolved directory sits in the area the authored registry declares"
    );
}

#[test]
fn test_doc_dir_json_output_is_a_flat_object_naming_the_issue_the_area_and_the_directory() {
    let repo = initialized_repo();
    let area = declared_area();
    let (issue_type, namespace) = membership_type_and_namespace();
    let issue = create_issue(
        repo.path(),
        "Machine-readable directory",
        &[
            format!("type:{issue_type}"),
            format!("{namespace}:artifact-layout"),
        ],
    );

    let printed = resolve_directory(repo.path(), &issue.short_id, area);
    let output = jit(repo.path(), &["doc", "dir", &issue.short_id, area, "--json"]);
    assert!(
        output.status.success(),
        "doc dir --json: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let object = payload
        .as_object()
        .unwrap_or_else(|| panic!("a named object, not a bare value: {payload}"));

    // One value, so every member is a scalar: no count, no wrapped collection.
    assert!(
        object.values().all(serde_json::Value::is_string),
        "a flat named object rather than the list envelope: {payload}"
    );

    let values = object
        .values()
        .filter_map(serde_json::Value::as_str)
        .collect::<HashSet<_>>();
    [
        ("the issue's full identifier", issue.id.as_str()),
        ("the issue's short identifier", issue.short_id.as_str()),
        ("the area the caller named", area),
        ("the directory the same run prints", printed.as_str()),
    ]
    .iter()
    .for_each(|(member, expected)| {
        assert!(
            values.contains(expected),
            "the object carries {member} ({expected}): {payload}"
        );
    });
}

#[test]
fn test_doc_dir_rejects_an_unknown_issue_identifier() {
    let repo = initialized_repo();
    let area = declared_area();
    let known = create_issue(repo.path(), "Known issue", &[]);
    let unknown = "deadbeef";
    assert_ne!(known.short_id, unknown, "the identifier names no issue here");

    // The same area resolves for an issue that exists, so the rejection below
    // is the identifier and not the area.
    resolve_directory(repo.path(), &known.short_id, area);

    let output = jit(repo.path(), &["doc", "dir", unknown, area]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "an identifier naming no issue must not resolve a directory"
    );
    assert!(
        stderr.contains(unknown),
        "the error names the identifier that resolved to no issue: {stderr}"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "no path is printed for an issue that does not exist"
    );
}
