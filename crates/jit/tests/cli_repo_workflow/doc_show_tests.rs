//! Integration tests for the read-only `jit doc` commands
//!
//! Tests for:
//! - doc show without git
//! - doc show with git
//! - doc show with --at commit
//! - doc dir, the canonical artifact directory resolver
//!
//! The repository configuration and area fixtures below are crate-visible: the
//! sibling `artifact_conformance_cli_tests` module drives the same repository
//! shape and resolves directories through the same `doc dir` run
//! (`@/invariant/shared-test-contracts`).
use jit::domain::identity::IssueId;
use jit::test_cli::JitCli;
use jit::test_utils::declaration_dir;
use std::collections::HashSet;
use std::fs;

#[test]
fn test_doc_show_without_git() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();
    let short_id = issue_id.short_id();
    fs::write(
        cli.root().join("design.md"),
        "# Design Document\n\nThis is a test document.",
    )
    .unwrap();
    cli.run([
        "doc",
        "add",
        short_id,
        "design.md",
        "--label",
        "Design Doc",
        "--doc-type",
        "design",
    ])
    .unwrap();

    let document = cli
        .run_json(["doc", "show", short_id, "design.md"])
        .unwrap();
    assert!(document["content"]
        .as_str()
        .unwrap()
        .contains("# Design Document"));
    assert_eq!(document["commit"], "working-tree");

    cli.run([
        "doc",
        "add",
        short_id,
        "design.md",
        "--commit",
        "HEAD",
        "--skip-scan",
    ])
    .unwrap();
    let output = cli
        .command(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "a pinned read requires git");
}

#[test]
fn test_doc_show_with_git() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    let issue_id = cli
        .create_issue(["--title", "Test Issue"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();
    let short_id = issue_id.short_id();
    fs::write(
        cli.root().join("design.md"),
        "# Design Document\n\nCommitted content.",
    )
    .unwrap();
    cli.git(["add", "design.md"]).unwrap();
    cli.git(["commit", "-q", "-m", "Add design doc"]).unwrap();
    cli.run(["doc", "add", short_id, "design.md"]).unwrap();

    let output = cli
        .command(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Design Document"));
    assert!(stdout.contains("Committed content"));
}

/// The configuration of this suite's repository: the shared taxonomy plus its
/// own documentation registry. The tests exercise both vocabularies, so the
/// repository declares them rather than inheriting any.
pub(crate) fn declared_config() -> String {
    format!(
        "{}\n[documentation]\ndevelopment_root = \"{DEVELOPMENT_ROOT}\"\n\
         managed_paths = [\"{}\", \"{}\"]\n\
         issue_scoped_areas = [\"{}\"]\n",
        jit::test_taxonomy::test_taxonomy().config_fragment(),
        declared_area(),
        undeclared_area(),
        declared_area(),
    )
}

/// The development root this suite's repository declares.
const DEVELOPMENT_ROOT: &str = "workspace";

/// The issue-scoped area this suite's repository declares.
pub(crate) fn declared_area() -> &'static str {
    "workspace/drafts"
}

/// A development area this suite's repository manages while leaving it outside
/// the issue-scoped convention, so rejecting it can only come from the
/// registry rather than from the path being unknown.
pub(crate) fn undeclared_area() -> &'static str {
    "workspace/notes"
}

/// A type the shared taxonomy maps to a membership namespace, with that
/// namespace. An issue carrying `type:<type>` and `<namespace>:<value>` names
/// a single membership value.
pub(crate) fn membership_type_and_namespace() -> (String, String) {
    let mut associations = jit::test_taxonomy::test_taxonomy()
        .label_associations
        .into_iter()
        .collect::<Vec<_>>();
    associations.sort();
    associations
        .into_iter()
        .next()
        .expect("the shared taxonomy maps a type to a membership namespace")
}

/// Every string leaf in `payload`, so a rejection can be asserted to carry a
/// diagnostic naming the offending input without pinning the envelope's member
/// names.
fn string_leaves(payload: &serde_json::Value) -> Vec<&str> {
    match payload {
        serde_json::Value::String(text) => vec![text.as_str()],
        serde_json::Value::Array(items) => items.iter().flat_map(string_leaves).collect(),
        serde_json::Value::Object(members) => members.values().flat_map(string_leaves).collect(),
        _ => Vec::new(),
    }
}

/// Assert the `--json` counterpart of a `doc dir` rejection: rerunning `args`
/// with `--json` fails and prints a parseable diagnostic on stdout naming each
/// string in `named`.
fn assert_json_rejection_agrees(cli: &JitCli, args: &[&str], named: &[&str], shape: &str) {
    let json_args = args.iter().copied().chain(["--json"]).collect::<Vec<_>>();
    let output = cli.command(json_args).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "{shape} must not resolve a directory under --json"
    );

    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!("{shape} prints a machine-readable rejection on stdout ({error}): {stdout}")
    });
    let diagnostic = string_leaves(&payload);
    named.iter().for_each(|expected| {
        assert!(
            diagnostic.iter().any(|text| text.contains(expected)),
            "the {shape} rejection carries a diagnostic naming {expected}: {payload}"
        );
    });
}

#[test]
fn test_doc_dir_names_the_directory_the_membership_precedence_resolves_inside_a_declared_area() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let area = declared_area();
    let (issue_type, namespace) = membership_type_and_namespace();
    let membership = "artifact-layout";

    let member = cli
        .create_issue([
            "--title",
            "Membership-labelled issue",
            "--label",
            &format!("type:{issue_type}"),
            "--label",
            &format!("{namespace}:{membership}"),
        ])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();
    let bare = cli
        .create_issue(["--title", "Issue naming no membership value"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();

    let member_directory = cli
        .run(["doc", "dir", member.short_id(), area])
        .unwrap()
        .trim()
        .to_string();
    let bare_directory = cli
        .run(["doc", "dir", bare.short_id(), area])
        .unwrap()
        .trim()
        .to_string();

    [(&member, &member_directory), (&bare, &bare_directory)]
        .iter()
        .for_each(|(issue, directory)| {
            let (parent, name) = directory
                .rsplit_once('/')
                .unwrap_or_else(|| panic!("{directory} sits inside an area"));
            assert_eq!(parent, area, "{directory} sits directly in the named area");
            assert!(
                name.starts_with(issue.short_id()),
                "{directory} is named for the issue that owns it"
            );
            assert!(
                !cli.root().join(directory).exists(),
                "{directory} is named, not created"
            );
        });

    // The two issues differ only in their membership labels, so the precedence
    // the resolver applies is what separates the two directory names.
    assert_eq!(
        bare_directory,
        format!("{area}/{}", bare.short_id()),
        "an issue naming no membership value owns the bare short-id directory"
    );
    assert!(
        member_directory.ends_with(&format!("-{membership}")),
        "a single resolved membership value suffixes the short id: {member_directory}"
    );
}

#[test]
fn test_doc_dir_rejects_an_area_the_configured_registry_does_not_declare() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let area = declared_area();
    let issue = cli
        .create_issue(["--title", "Undeclared area"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();

    // The same repository resolves a declared area, so a rejection below is the
    // registry talking rather than the command being absent.
    let declared_directory = cli
        .run(["doc", "dir", issue.short_id(), area])
        .unwrap()
        .trim()
        .to_string();

    [
        (
            "a development area outside the convention",
            undeclared_area().to_string(),
        ),
        ("a path inside a declared area", declared_directory.clone()),
        (
            "a sibling whose name starts with a declared area",
            format!("{area}-elsewhere"),
        ),
    ]
    .iter()
    .for_each(|(shape, undeclared)| {
        let args = ["doc", "dir", issue.short_id(), undeclared.as_str()];
        let output = cli.command(args).output().unwrap();
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

        // The same rejection under --json, so a machine caller reads the
        // registry's refusal instead of an unparseable line.
        assert_json_rejection_agrees(&cli, &args, &[undeclared.as_str(), area], shape);
    });
}

#[test]
fn test_doc_dir_resolves_the_area_registry_from_repository_configuration() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let declared = declared_area();
    let authored = undeclared_area();
    let issue = cli
        .create_issue(["--title", "Configured registry"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();

    let accepts = |area: &str| {
        cli.command(["doc", "dir", issue.short_id(), area])
            .status()
            .unwrap()
            .success()
    };

    assert!(
        accepts(declared) && !accepts(authored),
        "the configured registry declares {declared} and not {authored}"
    );

    // Replace the declared registry with one naming the other area. Nothing but
    // configuration changes, so a flip in what resolves is the command reading
    // the repository rather than a vocabulary compiled into the binary.
    let config_path = cli.root().join(".jit").join("config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    let declaration = "issue_scoped_areas = [";
    let start = config
        .find(declaration)
        .expect("the authored configuration declares the registry");
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
        accepts(authored) && !accepts(declared),
        "an authored registry replaces the configured one"
    );
    assert!(
        cli.run(["doc", "dir", issue.short_id(), authored])
            .unwrap()
            .trim()
            .to_string()
            .starts_with(&format!("{authored}/")),
        "the resolved directory sits in the area the authored registry declares"
    );
}

#[test]
fn test_doc_dir_json_output_is_a_flat_object_naming_the_issue_the_area_and_the_directory() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let area = declared_area();
    let (issue_type, namespace) = membership_type_and_namespace();
    let issue = cli
        .create_issue([
            "--title",
            "Machine-readable directory",
            "--label",
            &format!("type:{issue_type}"),
            "--label",
            &format!("{namespace}:artifact-layout"),
        ])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();

    let printed = cli
        .run(["doc", "dir", issue.short_id(), area])
        .unwrap()
        .trim()
        .to_string();
    let output = cli
        .command(["doc", "dir", issue.short_id(), area, "--json"])
        .output()
        .unwrap();
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
        ("the issue's full identifier", issue.as_str()),
        ("the issue's short identifier", issue.short_id()),
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
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let area = declared_area();
    let known = cli
        .create_issue(["--title", "Known issue"])
        .unwrap()
        .parse::<IssueId>()
        .unwrap();
    let unknown = "deadbeef";
    assert_ne!(
        known.short_id(),
        unknown,
        "the identifier names no issue here"
    );

    // The same area resolves for an issue that exists, so the rejection below
    // is the identifier and not the area.
    cli.run(["doc", "dir", known.short_id(), area])
        .unwrap()
        .trim()
        .to_string();

    let args = ["doc", "dir", unknown, area];
    let output = cli.command(args).output().unwrap();
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

    // The same rejection under --json, so a machine caller reads the failed
    // lookup instead of an unparseable line.
    assert_json_rejection_agrees(&cli, &args, &[unknown], "an identifier naming no issue");
}

fn failure_of(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("failure envelope on stdout")
}

#[test]
fn test_doc_show_missing_document_reports_document_not_found() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();

    let output = cli
        .command([
            "doc",
            "show",
            issue_id.as_str(),
            "docs/missing.md",
            "--json",
        ])
        .output()
        .unwrap();

    let code = jit::output::ErrorCode::DocumentNotFound;
    assert_eq!(failure_of(&output)["error"]["code"], code.as_str());
    assert_eq!(output.status.code(), Some(code.exit_code() as i32));
}

#[cfg(unix)]
#[test]
fn test_doc_show_document_outside_repository_reports_invalid_argument() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "# outside").unwrap();
    let link = cli.root().join("escape");
    fs::create_dir(&link).unwrap();
    fs::write(link.join("secret.md"), "# inside").unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "escape/secret.md"])
        .unwrap();
    fs::remove_dir_all(&link).unwrap();
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();

    let output = cli
        .command([
            "doc",
            "show",
            issue_id.as_str(),
            "escape/secret.md",
            "--json",
        ])
        .output()
        .unwrap();

    let code = jit::output::ErrorCode::InvalidArgument;
    assert_eq!(failure_of(&output)["error"]["code"], code.as_str());
    assert_eq!(output.status.code(), Some(code.exit_code() as i32));
}
