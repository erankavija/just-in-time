//! REQ-02: bind every `command_exit_codes` row to the runtime behavior it
//! documents, so no mapping in the generated reference is hand-authored.
//!
//! Each row is bound one of two ways:
//!
//! - **Subprocess** (this file): run the real command into the documented
//!   condition and assert the process exit code. This covers the rows emitted by
//!   direct `std::process::exit` sites (findings signals) and the command families
//!   whose condition is reachable end-to-end.
//! - **Classifier** (`main.rs`, `exit_code_projection_tests`): run the typed error
//!   the condition raises through `error_to_exit_code` — the exact classifier CLI
//!   dispatch uses — and assert it lands on that row. This covers conditions that
//!   are impractical to provoke through the binary (a mid-batch write failure) and
//!   the gate-evaluation verdict rows.
//!
//! One row has no fixed code to observe: `serve --fg` is a pass-through, so it
//! is verified against the production function its dispatch calls
//! (`serve::foreground_exit_code`).
//!
//! `test_command_exit_codes_every_row_is_verified` keeps the set complete: adding
//! a row without a binding fails the build.

use jit::commands::serve::foreground_exit_code;
use jit::schema::CommandSchema;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
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
    let rules_path = temp.path().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).unwrap();
    rules.push_str(
        "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = false\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    );
    fs::write(rules_path, rules).unwrap();
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

/// `jit config validate` emits only {0, 1}: 0 on a valid config, 1 when a source
/// carries an invalid value. There is no warning outcome. This test drives both
/// live outcomes and asserts neither is `2`.
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

    // 1 is the only documented config-validate failure code.
    documented_row("config validate", Some(1), true);
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
        .args(["gate", "preset", "apply", "plan-review", "0000000000000000"])
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

/// `jit issue list` on a healthy repository exits 0 — the universal `*`/0 row.
#[test]
fn test_command_exit_codes_success_emits_0() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "list"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    documented_row("*", Some(0), false);
}

/// A too-short id prefix is a usage error — the universal `*`/2 row.
#[test]
fn test_command_exit_codes_invalid_argument_emits_2() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "show", "ab"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    documented_row("*", Some(2), false);
}

/// An unresolvable issue id is a not-found error — the universal `*`/3 row.
#[test]
fn test_command_exit_codes_not_found_emits_3() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "show", "0123456789abcdef"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    documented_row("*", Some(3), false);
}

/// `jit dep add` exits 4 when the edge would close a cycle — matching `dep add`/4.
#[test]
fn test_command_exit_codes_dep_add_cycle_emits_4() {
    let temp = setup();
    let first = create_issue(&temp, "First", &[]);
    let second = create_issue(&temp, "Second", &[]);

    // second -> first is fine.
    assert!(Command::new(jit_binary())
        .current_dir(&temp)
        .args(["dep", "add", &second, &first])
        .status()
        .unwrap()
        .success());

    // first -> second closes the cycle.
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["dep", "add", &first, &second])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("dep add", Some(4), false);
}

/// Build a dependency-blocked pair: `(dependency, dependent)`, where `dependent`
/// depends on the still-unmet `dependency`.
fn blocked_pair(temp: &TempDir) -> (String, String) {
    let dependency = create_issue(temp, "Dependency", &[]);
    let dependent = create_issue(temp, "Dependent", &[]);
    assert!(Command::new(jit_binary())
        .current_dir(temp)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap()
        .success());
    (dependency, dependent)
}

/// All three commands in the `issue update, issue claim, issue claim-next`/4 row
/// exit 4 on a blocked transition: `update` and `claim` on unmet dependencies,
/// `claim-next` on an unpassed precheck gate.
#[test]
fn test_command_exit_codes_blocked_transition_emits_4() {
    // `issue update` into `ready`, which the unmet dependency blocks (a dependency
    // must reach a terminal state before its dependent becomes ready).
    let temp = setup();
    let (_dependency, dependent) = blocked_pair(&temp);
    let update = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "update", &dependent, "--state", "ready"])
        .output()
        .unwrap();
    assert_eq!(
        update.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&update.stderr)
    );

    // `issue claim` on the same blocked issue.
    let claim = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "claim", &dependent, "agent:test"])
        .output()
        .unwrap();
    assert_eq!(
        claim.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&claim.stderr)
    );

    // `issue claim-next` picking up an issue whose precheck gate has not passed.
    let temp = setup();
    assert!(Command::new(jit_binary())
        .current_dir(&temp)
        .args([
            "gate",
            "define",
            "precheck-gate",
            "--title",
            "Precheck",
            "-d",
            "Precheck",
            "--stage",
            "precheck",
            "--mode",
            "manual",
        ])
        .status()
        .unwrap()
        .success());
    create_issue(&temp, "Precheck work", &["--gate", "precheck-gate"]);
    let claim_next = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["issue", "claim-next", "agent:test"])
        .output()
        .unwrap();
    assert_eq!(
        claim_next.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&claim_next.stderr)
    );

    documented_row(
        "issue update, issue claim, issue claim-next",
        Some(4),
        false,
    );
}

/// `jit gate define` exits 6 when the key is already registered — matching
/// `gate define`/6.
#[test]
fn test_command_exit_codes_gate_define_duplicate_emits_6() {
    let temp = setup();
    let define = || {
        Command::new(jit_binary())
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
            .output()
            .unwrap()
    };
    assert!(define().status.success());

    let duplicate = define();
    assert_eq!(
        duplicate.status.code(),
        Some(6),
        "stderr: {}",
        String::from_utf8_lossy(&duplicate.stderr)
    );
    documented_row("gate define", Some(6), false);
}

/// `jit issue delete` exits 2 when refused for missing operator confirmation
/// (`JIT_ALLOW_DELETION=1` not set) — matching `issue delete`/2 (jit:0daba57d).
#[test]
fn test_command_exit_codes_issue_delete_unconfirmed_emits_2() {
    let temp = setup();
    let id = create_issue(&temp, "Doomed", &[]);

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .env_remove("JIT_ALLOW_DELETION")
        .args(["issue", "delete", &id])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("issue delete", Some(2), false);
}

/// `jit issue batch-create` exits 2 when pre-validation rejects the file (here an
/// entry depending on a key the file never defines) — matching
/// `issue batch-create`/2. The companion `/10` row (a write that fails after some
/// issues were created) is pinned by the classifier test in `main.rs`, which runs
/// the `BatchWriteError` that path raises through `error_to_exit_code`.
#[test]
fn test_command_exit_codes_batch_create_prevalidation_emits_2() {
    let temp = setup();
    let batch = temp.path().join("batch.json");
    fs::write(
        &batch,
        r#"[{"key": "a", "title": "A", "depends_on": ["ghost"]}]"#,
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args([
            "issue",
            "batch-create",
            "--from-json",
            batch.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("issue batch-create", Some(2), false);
}

/// `jit snapshot export` exits 6 when the output path is already occupied —
/// matching `snapshot export`/6.
#[test]
fn test_command_exit_codes_snapshot_export_occupied_emits_6() {
    let temp = setup();
    create_issue(&temp, "Snapshot me", &[]);
    fs::create_dir(temp.path().join("taken")).unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["snapshot", "export", "--out", "taken", "--working-tree"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(6),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("snapshot export", Some(6), false);
}

/// A lease subcommand run outside a git repository exits 10 — matching `claim`/10.
/// `setup()` inits `.jit` without a git repository, which is exactly that case.
#[test]
fn test_command_exit_codes_claim_without_git_emits_10() {
    let temp = setup();
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["claim", "status"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(10),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    documented_row("claim", Some(10), false);
}

/// A downstream reader that closes the pipe mid-write (e.g. `| head`) makes jit
/// exit quietly with the SIGPIPE exit-status convention (`128 + 13 = 141`)
/// instead of panicking through std's `print!`/`println!` machinery — matching
/// `*`/141 (jit:6f881a85). Mirrors `jit --schema | head -c1`: `--schema` dumps
/// several hundred KB of JSON well past a pipe's kernel buffer (64KiB on
/// Linux), so the write reliably blocks and then fails once the read end
/// closes — reading only the first line, as `query all` with a handful of
/// issues does, races the child's own (near-instant, sub-buffer-size) exit
/// and does not reproduce reliably.
#[test]
fn test_command_exit_codes_broken_pipe_emits_141() {
    let temp = setup();

    let mut child = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("--schema")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let mut first_byte = [0u8; 1];
    stdout
        .read_exact(&mut first_byte)
        .expect("child should write at least one byte before the pipe closes");
    drop(stdout);

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();

    assert_eq!(
        status.code(),
        Some(141),
        "broken-pipe exit must use the SIGPIPE convention (128 + 13); stderr: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "broken-pipe exit must not print a panic banner; stderr: {stderr}"
    );
    documented_row("*", Some(141), true);
}

/// Guard: **every** row in the projection — exception and standard alike — must be
/// bound to runtime behavior, either by a subprocess test in this file or by the
/// classifier test in `main.rs` (`exit_code_projection_tests`), which runs the
/// typed error the row's condition raises through `error_to_exit_code` and asserts
/// it lands on that exact row. A row added without a binding fails here, so no
/// mapping in the reference is hand-authored.
#[test]
fn test_command_exit_codes_every_row_is_verified() {
    let verified: std::collections::HashSet<(String, Option<i32>)> = [
        // Pinned by the `main.rs` classifier test (typed error -> row).
        ("*", Some(1)),
        ("*", Some(5)),
        ("*", Some(10)),
        ("any command that writes an issue", Some(4)),
        ("issue batch-create", Some(10)),
        ("gate evaluate, gate evaluate-all", Some(4)),
        ("gate evaluate, gate evaluate-all", Some(10)),
        // Pinned by the subprocess tests in this file.
        ("*", Some(0)),
        ("*", Some(2)),
        ("*", Some(3)),
        ("dep add", Some(4)),
        // Pinned by test_apply_rejects_prospective_cycle_and_creates_nothing
        // (fast_docs_templates), which asserts the typed CycleDetected the
        // classifier maps to 4.
        ("apply", Some(4)),
        ("issue update, issue claim, issue claim-next", Some(4)),
        ("gate define", Some(6)),
        ("issue delete", Some(2)),
        ("issue batch-create", Some(2)),
        ("snapshot export", Some(6)),
        ("claim", Some(10)),
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
        ("serve, serve --stop, serve --status", Some(1)),
        ("*", Some(141)),
        // Pass-through: asserted against the production site it cites.
        ("serve --fg", None),
    ]
    .into_iter()
    .map(|(c, code)| (c.to_string(), code))
    .collect();

    let projected: std::collections::HashSet<(String, Option<i32>)> = CommandSchema::generate()
        .command_exit_codes
        .into_iter()
        .map(|c| (c.command, c.code))
        .collect();

    assert_eq!(
        projected, verified,
        "every projected row must have a verifying binding; \
         left = projection, right = verified set"
    );
}
