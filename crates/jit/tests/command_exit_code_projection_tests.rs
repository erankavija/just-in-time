//! REQ-02: bind every EXCEPTION row of the `command_exit_codes` projection to
//! the code that actually emits it.
//!
//! Exception rows are emitted by direct `std::process::exit` sites (a completed
//! run signalling findings) or by a pass-through, so most are observed by running
//! the binary. The two gate-evaluation rows route through `error_to_exit_code`
//! and are pinned by the classifier unit test in `main.rs`
//! (`exit_code_projection_tests`); the `serve` pass-through carries no fixed code
//! and is asserted structurally. [`every_exception_row_is_verified`] enforces
//! that this set stays complete: adding an exception row without a binding fails
//! the build.

use jit::schema::CommandSchema;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    let status = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    temp
}

fn create_issue(temp: &TempDir, title: &str, extra: &[&str]) -> String {
    let mut args = vec!["issue", "create", "--title", title, "--json"];
    args.extend_from_slice(extra);
    let output = Command::new(jit_binary())
        .current_dir(temp)
        .args(&args)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// Assert the projection carries an exception row for `command` at `code`
/// (`None` = pass-through).
fn assert_documented_exception(command: &str, code: Option<i32>) {
    let schema = CommandSchema::generate();
    let row = schema
        .command_exit_codes
        .iter()
        .find(|c| c.command == command && c.code == code)
        .unwrap_or_else(|| {
            panic!("command_exit_codes projection has no row for `{command}` code {code:?}")
        });
    assert!(
        row.exception,
        "row for `{command}` code {code:?} must be flagged as an exception"
    );
}

/// `jit validate` exits 4 on repository-integrity findings — matching `validate`/4.
#[test]
fn validate_integrity_exit_matches_projection() {
    let temp = setup();
    let id = create_issue(&temp, "Corruptible", &[]);

    let issue_path = temp
        .path()
        .join(".jit")
        .join("issues")
        .join(format!("{id}.json"));
    let mut issue: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue["dependencies"] = serde_json::json!(["nonexistent"]);
    fs::write(&issue_path, serde_json::to_string_pretty(&issue).unwrap()).unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("validate")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    assert_documented_exception("validate", Some(4));
}

/// Whole-repo `jit validate` exits 1 on an error-severity rule finding —
/// matching `validate`/1. A non-enforced error rule reports without diverting
/// through the integrity (exit-4) path.
#[test]
fn validate_rule_findings_exit_matches_projection() {
    let temp = setup();
    fs::write(
        temp.path().join(".jit/rules.toml"),
        "[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = false\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    )
    .unwrap();
    // An epic missing its `req:*` label violates the error-severity rule; the
    // `epic:auth` identity label keeps the finding to the rule under test.
    create_issue(
        &temp,
        "An epic",
        &["--label", "type:epic", "--label", "epic:auth"],
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("validate")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_documented_exception("validate", Some(1));
}

/// `jit gate status-all` exits 4 while any required gate is unpassed —
/// matching `gate status-all`/4.
#[test]
fn gate_status_all_exit_matches_projection() {
    let temp = setup();
    let status = Command::new(jit_binary())
        .current_dir(&temp)
        .args([
            "gate",
            "define",
            "--title",
            "Tests",
            "--description",
            "Tests",
            "tests",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let id = create_issue(&temp, "Gated", &["--gate", "tests"]);

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["gate", "status-all", &id])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    assert_documented_exception("gate status-all", Some(4));
}

/// `jit invariant check` exits 4 on enforcement drift (an `enforced-by` naming a
/// missing rule) — matching `invariant check`/4.
#[test]
fn invariant_check_drift_exit_matches_projection() {
    let temp = setup();
    fs::write(
        temp.path().join(".jit/rules.toml"),
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["invariant", "check"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    assert_documented_exception("invariant check", Some(4));
}

/// `jit config validate` exits 1 when a configuration source carries an invalid
/// value — matching `config validate`/1.
#[test]
fn config_validate_error_exit_matches_projection() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["config", "validate"])
        .env("JIT_WORKTREE_MODE", "definitely-not-a-mode")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_documented_exception("config validate", Some(1));
}

/// `jit doc check-links` exits 1 on a broken link and 2 on a risky-link warning
/// — matching the two `doc check-links` rows.
#[test]
fn doc_check_links_exits_match_projection() {
    // Broken link -> exit 1.
    let temp = setup();
    let id = create_issue(&temp, "Doc host", &[]);
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(
        temp.path().join("docs/broken.md"),
        "# Doc\n\nSee [missing](nonexistent.md).\n",
    )
    .unwrap();
    assert!(Command::new(jit_binary())
        .current_dir(&temp)
        .args(["doc", "add", &id, "docs/broken.md", "--label", "Test"])
        .status()
        .unwrap()
        .success());
    let broken = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert_eq!(broken.status.code(), Some(1));
    assert_documented_exception("doc check-links", Some(1));

    // Risky (deep relative) but valid link -> exit 2.
    let temp = setup();
    let id = create_issue(&temp, "Doc host", &[]);
    fs::create_dir_all(temp.path().join("docs/subdir")).unwrap();
    fs::create_dir_all(temp.path().join("assets")).unwrap();
    fs::write(
        temp.path().join("docs/subdir/risky.md"),
        "# Risky\n\n![Asset](../../assets/image.png)\n",
    )
    .unwrap();
    fs::write(temp.path().join("assets/image.png"), b"data").unwrap();
    assert!(Command::new(jit_binary())
        .current_dir(&temp)
        .args(["doc", "add", &id, "docs/subdir/risky.md", "--label", "Test"])
        .status()
        .unwrap()
        .success());
    let risky = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["doc", "check-links", "--scope", "all"])
        .output()
        .unwrap();
    assert_eq!(risky.status.code(), Some(2));
    assert_documented_exception("doc check-links", Some(2));
}

/// `jit gate preset apply` exits 1 when an issue fails to apply (a partial
/// batch) — matching `gate preset apply`/1. Applying a builtin preset to a
/// well-formed but nonexistent id fails that id and exits 1.
#[test]
fn gate_preset_apply_partial_failure_exit_matches_projection() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["gate", "preset", "apply", "minimal", "0000000000000000"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_documented_exception("gate preset apply", Some(1));
}

/// `jit serve` passes through the bundled dev-server child's own exit code, so
/// the projection models it as a pass-through (`code: None`) rather than a fixed
/// jit code. Starting a real server is impractical here; the runtime site is
/// `crates/jit/src/main.rs` (`status.code().unwrap_or(1)`), which carries no
/// fixed code to observe, so this row is asserted structurally.
#[test]
fn serve_passthrough_is_modeled_as_none() {
    assert_documented_exception("serve", None);
}

/// Guard: every exception row in the projection must be covered by a binding in
/// this file or by the `gate evaluate` classifier test in `main.rs`. Adding a new
/// exception row without a test fails here.
#[test]
fn every_exception_row_is_verified() {
    let verified: std::collections::HashSet<(String, Option<i32>)> = [
        // Pinned by main.rs `exit_code_projection_tests` (classifier).
        ("gate evaluate, gate evaluate-all", Some(4)),
        ("gate evaluate, gate evaluate-all", Some(10)),
        // Pinned by the subprocess tests above.
        ("validate", Some(4)),
        ("validate", Some(1)),
        ("gate status-all", Some(4)),
        ("invariant check", Some(4)),
        ("config validate", Some(1)),
        ("doc check-links", Some(1)),
        ("doc check-links", Some(2)),
        ("gate preset apply", Some(1)),
        // Pinned structurally (pass-through carries no fixed code).
        ("serve", None),
    ]
    .into_iter()
    .map(|(c, code)| (c.to_string(), code))
    .collect();

    let exceptions: std::collections::HashSet<(String, Option<i32>)> = CommandSchema::generate()
        .command_exit_codes
        .into_iter()
        .filter(|c| c.exception)
        .map(|c| (c.command, c.code))
        .collect();

    assert_eq!(
        exceptions, verified,
        "every exception row must have a verifying test; \
         left = projection, right = verified set"
    );
}
