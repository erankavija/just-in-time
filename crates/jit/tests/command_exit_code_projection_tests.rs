//! REQ-02: bind every `command_exit_codes` row emitted by a direct exit site to
//! the code that actually emits it.
//!
//! Findings-signal and pass-through rows are emitted by direct
//! `std::process::exit` sites, so they are observed by running the binary. The
//! two gate-evaluation exception rows route through `error_to_exit_code` and are
//! pinned by the classifier unit test in `main.rs` (`exit_code_projection_tests`).
//! `serve --fg` has no fixed code to observe, so it is verified against the
//! production function its dispatch calls (`serve::foreground_exit_code`); the
//! reserved `config validate` `2` is unreachable by construction (the handler
//! defines no warning condition) and is asserted to be documented as reserved.
//! `test_command_exit_codes_every_exception_row_is_verified` keeps the set
//! complete: adding an exception row without a binding fails the build.

use jit::commands::serve::foreground_exit_code;
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

/// Assert the projection carries a row for `command` at `code` (`None` =
/// pass-through) with the expected `exception` flag, and return its condition so
/// callers can assert on the documented text.
fn documented_row(command: &str, code: Option<i32>, exception: bool) -> String {
    let schema = CommandSchema::generate();
    let row = schema
        .command_exit_codes
        .iter()
        .find(|c| c.command == command && c.code == code)
        .unwrap_or_else(|| {
            panic!("command_exit_codes projection has no row for `{command}` code {code:?}")
        });
    assert_eq!(
        row.exception, exception,
        "row for `{command}` code {code:?} has wrong exception flag"
    );
    row.condition.clone()
}

/// `jit validate` exits 4 on repository-integrity findings — matching `validate`/4.
#[test]
fn test_command_exit_codes_validate_integrity_emits_4() {
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
    documented_row("validate", Some(4), true);
}

/// Whole-repo `jit validate` exits 1 on an error-severity rule finding —
/// matching `validate`/1. A non-enforced error rule reports without diverting
/// through the integrity (exit-4) path.
#[test]
fn test_command_exit_codes_validate_rule_findings_emits_1() {
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
    documented_row("validate", Some(1), true);
}

/// `jit gate status-all` exits 4 while any required gate is unpassed —
/// matching `gate status-all`/4.
#[test]
fn test_command_exit_codes_gate_status_all_emits_4() {
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
    documented_row("gate status-all", Some(4), true);
}

/// `jit invariant check` exits 4 on enforcement drift (an `enforced-by` naming a
/// missing rule) — matching `invariant check`/4.
#[test]
fn test_command_exit_codes_invariant_check_emits_4() {
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
    documented_row("invariant check", Some(4), true);
}

/// `jit config validate` emits only {0, 1} in practice: 0 on a valid config, 1
/// when a source carries an invalid value. The projection documents `2` as
/// RESERVED — the handler has an `exit(2)` warnings branch, but no warning
/// condition is defined, so `2` is never produced. This test drives both live
/// outcomes, asserts neither is `2`, and asserts the reserved row is documented.
#[test]
fn test_command_exit_codes_config_validate_emits_only_0_and_1() {
    let temp = setup();

    // Valid configuration -> 0.
    let valid = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["config", "validate"])
        .output()
        .unwrap();
    assert_eq!(
        valid.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&valid.stderr)
    );

    // Invalid environment-variable value -> 1.
    let invalid = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["config", "validate"])
        .env("JIT_WORKTREE_MODE", "definitely-not-a-mode")
        .output()
        .unwrap();
    assert_eq!(
        invalid.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&invalid.stderr)
    );

    // Neither live outcome is 2.
    assert_ne!(valid.status.code(), Some(2));
    assert_ne!(invalid.status.code(), Some(2));

    // 1 is emitted; 2 is documented as reserved (present, exception-flagged).
    documented_row("config validate", Some(1), true);
    let reserved = documented_row("config validate", Some(2), true);
    assert!(
        reserved.to_lowercase().contains("reserved"),
        "config validate 2 must be documented as reserved, got: {reserved}"
    );
}

/// `jit doc check-links` exits 1 on a broken link and 2 on a risky-link warning
/// — matching the two `doc check-links` rows.
#[test]
fn test_command_exit_codes_doc_check_links_emits_1_and_2() {
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
    documented_row("doc check-links", Some(1), true);

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
    documented_row("doc check-links", Some(2), true);
}

/// `jit gate preset apply` exits 1 when an issue fails to apply (a partial
/// batch) — matching `gate preset apply`/1. Applying a builtin preset to a
/// well-formed but nonexistent id fails that id and exits 1.
#[test]
fn test_command_exit_codes_gate_preset_apply_emits_1() {
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
    documented_row("gate preset apply", Some(1), true);
}

/// `jit serve --status` (like the daemon start and `--stop`) exits 1 on an
/// error — here a malformed PID file — matching the `serve, serve --stop, serve
/// --status`/1 row (standard taxonomy, not an exception).
#[test]
fn test_command_exit_codes_serve_daemon_error_emits_1() {
    let temp = setup();
    // A malformed PID file makes `server_status` (via `read_pid_file`) error, so
    // the `--status` arm hits its `exit(1)` site.
    fs::write(temp.path().join(".jit/server.pid.json"), "not json").unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["serve", "--status"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("serve, serve --stop, serve --status", Some(1), false);
}

/// `jit serve --fg` passes the inline dev-server child's own exit code through,
/// so the projection models it as a pass-through (`code: None`).
///
/// The mapping is owned by `serve::foreground_exit_code`, which the `serve --fg`
/// dispatch in `crates/jit/src/main.rs` calls to compute the code it exits with.
/// Exercising that production function verifies the documented pass-through
/// against the runtime path: the child's code is returned verbatim, and a
/// signal-terminated child (no code) reports `1`.
#[test]
fn test_command_exit_codes_serve_foreground_is_passthrough() {
    documented_row("serve --fg", None, true);

    for child_code in [0, 1, 2, 3, 4, 10, 101, 127] {
        assert_eq!(
            foreground_exit_code(Some(child_code)),
            child_code,
            "serve --fg must pass the child's exit code {child_code} through verbatim"
        );
    }
    assert_eq!(
        foreground_exit_code(None),
        1,
        "a signal-terminated child carries no code, so serve --fg reports 1"
    );
}

/// `jit validate --branch-drift` exits 1 when the drift check reports drift or
/// cannot run, matching the projected `validate --branch-drift` row.
#[test]
fn test_command_exit_codes_validate_branch_drift_emits_1() {
    let temp = setup();
    // No git upstream here, so the branch-drift check cannot succeed.
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["validate", "--branch-drift"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "validate --branch-drift must exit 1 when the check fails; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("validate --branch-drift", Some(1), true);
}

/// `jit validate --leases` exits 1 when lease validation finds invalid leases or
/// cannot run, matching the projected `validate --leases` row.
#[test]
fn test_command_exit_codes_validate_leases_emits_1() {
    let temp = setup();
    // Corrupt the claims index so lease validation cannot produce a clean result.
    let shared = temp.path().join(".git/jit");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("claims.index.json"), "not json").unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["validate", "--leases"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "validate --leases must exit 1 when lease validation fails; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("validate --leases", Some(1), true);
}

/// Guard: every exception row in the projection must be covered by a binding in
/// this file or by the `gate evaluate` classifier test in `main.rs`. Adding a new
/// exception row without a test fails here.
#[test]
fn test_command_exit_codes_every_exception_row_is_verified() {
    let verified: std::collections::HashSet<(String, Option<i32>)> = [
        // Pinned by main.rs `exit_code_projection_tests` (classifier).
        ("gate evaluate, gate evaluate-all", Some(4)),
        ("gate evaluate, gate evaluate-all", Some(10)),
        // Pinned by the subprocess tests above.
        ("validate", Some(4)),
        ("validate", Some(1)),
        ("validate --branch-drift", Some(1)),
        ("validate --leases", Some(1)),
        ("gate status-all", Some(4)),
        ("invariant check", Some(4)),
        ("config validate", Some(1)),
        ("doc check-links", Some(1)),
        ("doc check-links", Some(2)),
        ("gate preset apply", Some(1)),
        // Reserved / pass-through: asserted structurally against cited sites.
        ("config validate", Some(2)),
        ("serve --fg", None),
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
