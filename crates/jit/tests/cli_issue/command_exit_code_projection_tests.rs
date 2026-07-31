//! REQ-02: bind every `command_exit_codes` row to the runtime behavior it
//! documents, so no mapping in the generated reference is hand-authored.
//!
//! Each projected fixed-code row is bound here by a real subprocess in both
//! public invocation forms. The classifier tests in `main.rs` remain useful
//! unit coverage, but cannot substitute for a command-level binding because
//! they do not exercise parsing, dispatch, or `--json` output selection.
//!
//! One row has no fixed projected code: `serve --fg` is a pass-through. Its
//! public plain and JSON forms are exercised with a deterministic child exit,
//! while `serve::foreground_exit_code` separately covers the full helper-level
//! pass-through range and signal-termination fallback.
//!
//! `test_command_exit_codes_every_row_is_verified` keeps the set complete: adding
//! a row without a binding fails the build.

use jit::commands::serve::foreground_exit_code;
use jit::schema::CommandSchema;
use std::fs;
use std::io::Read;
use std::process::{Command, Output, Stdio};
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

fn setup_auto_gate_issue(temp: &TempDir, checker_command: &str) -> String {
    let definition = Command::new(jit_binary())
        .current_dir(temp)
        .args([
            "gate",
            "define",
            "projection-gate",
            "--title",
            "Projection gate",
            "--description",
            "Projection gate",
            "--mode",
            "auto",
            "--checker-command",
            checker_command,
            "--timeout",
            "10",
        ])
        .output()
        .unwrap();
    assert!(
        definition.status.success(),
        "gate definition failed: {}",
        String::from_utf8_lossy(&definition.stderr)
    );
    create_issue(temp, "Gated projection", &["--gate", "projection-gate"])
}

const CYCLIC_APPLY_CONFIG: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

const CYCLIC_APPLY_TEMPLATE: &str = r#"
[[template]]
name        = "cyclic"
applies_to  = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.anchors]]
  name = "upstream"
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = "Plan {container.title}."
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  description = "Break down {container.title}."
  labels      = ["brackets:{container.short_id}"]
  depends_on  = ["planning"]
  [[template.anchor_edges]]
  from = "upstream"
  to   = "breakdown"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#;

fn setup_prospective_cycle(temp: &TempDir) -> (String, String) {
    fs::write(temp.path().join(".jit/config.toml"), CYCLIC_APPLY_CONFIG).unwrap();
    fs::write(
        temp.path().join(".jit/templates.toml"),
        CYCLIC_APPLY_TEMPLATE,
    )
    .unwrap();
    let upstream = create_issue(temp, "Upstream", &["--type", "task"]);
    let container = create_issue(temp, "Container", &["--type", "epic"]);
    let edge = Command::new(jit_binary())
        .current_dir(temp)
        .args(["dep", "add", &container, &upstream])
        .output()
        .unwrap();
    assert!(
        edge.status.success(),
        "cycle fixture edge setup failed: {}",
        String::from_utf8_lossy(&edge.stderr)
    );
    (container, upstream)
}

/// Return the projection's exit status for `command` and assert its documented
/// exception classification. `None` denotes a pass-through row.
fn documented_exit_code(command: &str, code: Option<i32>, exception: bool) -> Option<i32> {
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
    row.code
}

/// Bind a subprocess-reachable projection row to both public invocation forms.
///
/// The expected status comes from the generated projection, not a second
/// hand-authored numeric assertion. Running the same documented condition with
/// and without `--json` also makes a disagreement between the two forms fail.
fn assert_projected_exit_status_in_both_forms<F>(
    command: &str,
    code: i32,
    exception: bool,
    invoke: F,
) where
    F: Fn(bool) -> Output,
{
    let expected = documented_exit_code(command, Some(code), exception)
        .expect("subprocess bindings must name a fixed-code projection row");
    let plain = invoke(false);
    let json = invoke(true);
    let plain_status = plain.status.code();
    let json_status = json.status.code();

    assert_eq!(
        plain_status,
        Some(expected),
        "plain `{command}` status must match its projected row; stderr: {}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_eq!(
        json_status,
        Some(expected),
        "machine-readable `{command}` status must match its projected row; stderr: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert_eq!(
        plain_status, json_status,
        "plain and machine-readable `{command}` statuses must agree"
    );
}

/// A missing template is an invalid invocation, covering the universal
/// invalid-argument row through the public `apply` command.
#[test]
fn test_command_exit_codes_unknown_apply_template_emits_2() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("*", 2, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args([
            "apply",
            "definitely-missing-template",
            "0000000000000000",
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// An unreadable index exercises the universal filesystem-permission row through
/// the top-level query path (rather than a handler-owned mutation response).
#[test]
#[cfg(unix)]
fn test_command_exit_codes_permission_denied_emits_5() {
    use std::os::unix::fs::PermissionsExt;

    assert_projected_exit_status_in_both_forms("*", 5, false, |json| {
        let temp = setup();
        let index = temp.path().join(".jit/index.json");
        let mut denied = fs::metadata(&index).unwrap().permissions();
        denied.set_mode(0o000);
        fs::set_permissions(&index, denied).unwrap();

        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["query", "all"]);
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();

        let mut restored = fs::metadata(&index).unwrap().permissions();
        restored.set_mode(0o755);
        fs::set_permissions(&index, restored).unwrap();
        output
    });
}

/// A repository index newer than the binary's supported format reaches the
/// universal external-error row before command dispatch.
#[test]
fn test_command_exit_codes_too_new_format_emits_10() {
    assert_projected_exit_status_in_both_forms("*", 10, false, |json| {
        let temp = setup();
        let index_path = temp.path().join(".jit/index.json");
        let mut index: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
        index["schema_version"] = serde_json::json!(9999);
        fs::write(index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();

        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["query", "all"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// An enforcing validation rule rejects an issue create before publication,
/// exercising the shared write-validation row through a real writer.
#[test]
fn test_command_exit_codes_enforcing_write_rule_emits_4() {
    assert_projected_exit_status_in_both_forms(
        "any command that writes an issue",
        4,
        false,
        |json| {
            let temp = setup();
            let rules_path = temp.path().join(".jit/rules.toml");
            let mut rules = fs::read_to_string(&rules_path).unwrap();
            rules.push_str(
                "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
                 severity = \"error\"\nenforce = true\n\
                 assert = { require-label = { label = \"req:*\", min = 1 } }\n",
            );
            fs::write(rules_path, rules).unwrap();

            let mut command = Command::new(jit_binary());
            command.current_dir(&temp).args([
                "issue",
                "create",
                "--title",
                "Unqualified epic",
                "--type",
                "epic",
            ]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );
}

/// A checker that ran and failed is the gate-evaluation verdict-failure row.
#[test]
fn test_command_exit_codes_gate_evaluate_failure_emits_4() {
    let temp = setup();
    let id = setup_auto_gate_issue(&temp, "false");
    assert_projected_exit_status_in_both_forms(
        "gate evaluate, gate evaluate-all",
        4,
        true,
        |json| {
            let mut command = Command::new(jit_binary());
            command.current_dir(&temp).args([
                "gate",
                "evaluate",
                &id,
                "projection-gate",
                "--force",
            ]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );
}

/// A checker that cannot run to a verdict is the gate-evaluation runner-error
/// row. The unique command is intentionally absent from the test environment.
#[test]
fn test_command_exit_codes_gate_evaluate_runner_error_emits_10() {
    let temp = setup();
    let id = setup_auto_gate_issue(&temp, "jit-projection-command-does-not-exist");
    assert_projected_exit_status_in_both_forms(
        "gate evaluate, gate evaluate-all",
        10,
        true,
        |json| {
            let mut command = Command::new(jit_binary());
            command.current_dir(&temp).args([
                "gate",
                "evaluate",
                &id,
                "projection-gate",
                "--force",
            ]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );
}

/// The prospective-cycle guard rejects template application before it mutates,
/// binding the `apply`/4 row to both public invocation forms.
#[test]
fn test_command_exit_codes_apply_prospective_cycle_emits_4() {
    let temp = setup();
    let (container, upstream) = setup_prospective_cycle(&temp);
    let upstream_binding = format!("upstream={upstream}");
    assert_projected_exit_status_in_both_forms("apply", 4, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args([
            "apply",
            "cyclic",
            &container,
            "--anchor",
            &upstream_binding,
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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

    assert_projected_exit_status_in_both_forms("validate", 4, true, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).arg("validate");
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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

    assert_projected_exit_status_in_both_forms("validate", 1, true, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).arg("validate");
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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

    assert_projected_exit_status_in_both_forms("gate status-all", 4, true, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["gate", "status-all", &id]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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

    assert_projected_exit_status_in_both_forms("invariant check", 4, true, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["invariant", "check"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit config validate` classifies an invalid configuration as validation failed.
#[test]
fn test_command_exit_codes_config_validate_invalid_value_emits_4() {
    let temp = setup();

    assert_projected_exit_status_in_both_forms("config validate", 4, false, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["config", "validate"])
            .env("JIT_WORKTREE_MODE", "definitely-not-a-mode");
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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
    assert_projected_exit_status_in_both_forms("doc check-links", 1, true, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["doc", "check-links", "--scope", "all"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });

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
    assert_projected_exit_status_in_both_forms("doc check-links", 2, true, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["doc", "check-links", "--scope", "all"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit gate preset apply` exits 3 when a preset target is missing — matching
/// `gate preset apply`/3. Applying a builtin preset to a well-formed but
/// nonexistent id fails that id through the ordinary not-found classifier.
#[test]
fn test_command_exit_codes_gate_preset_apply_missing_target_emits_3() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("gate preset apply", 3, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args([
            "gate",
            "preset",
            "apply",
            "plan-review",
            "0000000000000000",
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit serve --status` (like the daemon start and `--stop`) classifies a
/// malformed PID file as `PARSE_ERROR`, whose registered status is 1, matching
/// the `serve, serve --stop, serve --status`/1 row.
#[test]
fn test_command_exit_codes_serve_daemon_error_emits_1() {
    let temp = setup();
    // A malformed PID file makes `server_status` (via `read_pid_file`) error, so
    // the `--status` arm routes the typed parser failure through the shared
    // classifier.
    fs::write(temp.path().join(".jit/server.pid.json"), "not json").unwrap();

    assert_projected_exit_status_in_both_forms(
        "serve, serve --stop, serve --status",
        1,
        false,
        |json| {
            let mut command = Command::new(jit_binary());
            command.current_dir(&temp).args(["serve", "--status"]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );
}

/// `jit serve --fg` passes the inline dev-server child's own exit code through,
/// so the projection models it as a pass-through (`code: None`).
///
/// The mapping is owned by `serve::foreground_exit_code`, which the `serve --fg`
/// dispatch in `crates/jit/src/main.rs` calls to compute the code it exits with.
/// This helper-level check complements the public dual-form test below: the
/// child's code is returned verbatim, and a signal-terminated child (no code)
/// reports `1`.
#[test]
fn test_command_exit_codes_serve_foreground_is_passthrough() {
    assert_eq!(documented_exit_code("serve --fg", None, true), None);

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

/// Copy the real CLI beside a deterministic fake `jit-server` child.
///
/// `find_server_binary` prefers a sibling of the current executable, so this
/// fixture drives the public foreground dispatch without starting a long-lived
/// server or relying on PATH. The arbitrary child code is deliberately outside
/// the documented taxonomy: observing it proves pass-through behavior.
#[cfg(unix)]
fn jit_with_exiting_server(temp: &TempDir, exit_code: i32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = temp.path().join("serve-fg-bin");
    fs::create_dir(&bin_dir).unwrap();

    let jit_copy = bin_dir.join("jit");
    fs::copy(jit_binary(), &jit_copy).unwrap();
    let mut jit_permissions = fs::metadata(&jit_copy).unwrap().permissions();
    jit_permissions.set_mode(0o755);
    fs::set_permissions(&jit_copy, jit_permissions).unwrap();

    let server = bin_dir.join("jit-server");
    fs::write(&server, format!("#!/bin/sh\nexit {exit_code}\n")).unwrap();
    let mut server_permissions = fs::metadata(&server).unwrap().permissions();
    server_permissions.set_mode(0o755);
    fs::set_permissions(&server, server_permissions).unwrap();

    jit_copy
}

/// Bind the `serve --fg` pass-through row through both public invocation forms.
#[cfg(unix)]
#[test]
fn test_command_exit_codes_serve_foreground_passthrough_is_dual_form() {
    const CHILD_EXIT: i32 = 42;

    let temp = setup();
    let jit = jit_with_exiting_server(&temp, CHILD_EXIT);
    assert_eq!(documented_exit_code("serve --fg", None, true), None);

    let invoke = |json| {
        let mut command = Command::new(&jit);
        command
            .current_dir(&temp)
            .args(["serve", "--fg", "--port", "0"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };

    let plain = invoke(false);
    let json = invoke(true);
    assert_eq!(
        plain.status.code(),
        Some(CHILD_EXIT),
        "plain stderr: {}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_eq!(
        json.status.code(),
        Some(CHILD_EXIT),
        "JSON stderr: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert_eq!(
        plain.status.code(),
        json.status.code(),
        "plain and --json serve --fg statuses must agree"
    );

    let response: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("serve --fg --json result");
    assert_eq!(response["status"], "exited");
    assert_eq!(response["exit_code"], CHILD_EXIT);
}

/// `jit validate --branch-drift` exits 1 when the drift check reports drift or
/// cannot run, matching the projected `validate --branch-drift` row.
#[test]
fn test_command_exit_codes_validate_branch_drift_emits_1() {
    let temp = setup();
    // No git upstream here, so the branch-drift check cannot succeed.
    assert_projected_exit_status_in_both_forms("validate --branch-drift", 1, true, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["validate", "--branch-drift"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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

    assert_projected_exit_status_in_both_forms("validate --leases", 1, true, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["validate", "--leases"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit issue list` on a healthy repository exits 0 — the universal `*`/0 row.
#[test]
fn test_command_exit_codes_success_emits_0() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("*", 0, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["issue", "list"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// A too-short id prefix is a usage error — the universal `*`/2 row.
#[test]
fn test_command_exit_codes_invalid_argument_emits_2() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("*", 2, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["issue", "show", "ab"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// An unresolvable issue id is a not-found error — the universal `*`/3 row.
#[test]
fn test_command_exit_codes_not_found_emits_3() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("*", 3, false, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["issue", "show", "0123456789abcdef"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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
    assert_projected_exit_status_in_both_forms("dep add", 4, false, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["dep", "add", &first, &second]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
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
    assert_projected_exit_status_in_both_forms(
        "issue update, issue claim, issue claim-next",
        4,
        false,
        |json| {
            let mut command = Command::new(jit_binary());
            command
                .current_dir(&temp)
                .args(["issue", "update", &dependent, "--state", "ready"]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );

    // `issue claim` on the same blocked issue.
    assert_projected_exit_status_in_both_forms(
        "issue update, issue claim, issue claim-next",
        4,
        false,
        |json| {
            let mut command = Command::new(jit_binary());
            command
                .current_dir(&temp)
                .args(["issue", "claim", &dependent, "agent:test"]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
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
    assert_projected_exit_status_in_both_forms(
        "issue update, issue claim, issue claim-next",
        4,
        false,
        |json| {
            let mut command = Command::new(jit_binary());
            command
                .current_dir(&temp)
                .args(["issue", "claim-next", "agent:test"]);
            if json {
                command.arg("--json");
            }
            command.output().unwrap()
        },
    );
}

/// `jit gate define` exits 6 when the key is already registered — matching
/// `gate define`/6.
#[test]
fn test_command_exit_codes_gate_define_duplicate_emits_6() {
    let temp = setup();
    let define = |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args([
            "gate",
            "define",
            "--title",
            "Tests",
            "--description",
            "Tests",
            "tests",
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    assert!(define(false).status.success());
    assert_projected_exit_status_in_both_forms("gate define", 6, false, define);
}

/// `jit issue delete` exits 2 when refused for missing operator confirmation
/// (`JIT_ALLOW_DELETION=1` not set) — matching `issue delete`/2 (jit:0daba57d).
#[test]
fn test_command_exit_codes_issue_delete_unconfirmed_emits_2() {
    let temp = setup();
    let id = create_issue(&temp, "Doomed", &[]);

    assert_projected_exit_status_in_both_forms("issue delete", 2, false, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .env_remove("JIT_ALLOW_DELETION")
            .args(["issue", "delete", &id]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit issue batch-create` exits 2 when pre-validation rejects the file (here an
/// entry depending on a key the file never defines) — matching
/// `issue batch-create`/2.
#[test]
fn test_command_exit_codes_batch_create_prevalidation_emits_2() {
    let temp = setup();
    let batch = temp.path().join("batch.json");
    fs::write(
        &batch,
        r#"[{"key": "a", "title": "A", "depends_on": ["ghost"]}]"#,
    )
    .unwrap();

    assert_projected_exit_status_in_both_forms("issue batch-create", 2, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args([
            "issue",
            "batch-create",
            "--from-json",
            batch.to_str().unwrap(),
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// `jit snapshot export` exits 6 when the output path is already occupied —
/// matching `snapshot export`/6.
#[test]
fn test_command_exit_codes_snapshot_export_occupied_emits_6() {
    let temp = setup();
    create_issue(&temp, "Snapshot me", &[]);
    fs::create_dir(temp.path().join("taken")).unwrap();

    assert_projected_exit_status_in_both_forms("snapshot export", 6, false, |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["snapshot", "export", "--out", "taken", "--working-tree"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// A lease subcommand run outside a git repository exits 10 — matching `claim`/10.
/// `setup()` inits `.jit` without a git repository, which is exactly that case.
#[test]
fn test_command_exit_codes_claim_without_git_emits_10() {
    let temp = setup();
    assert_projected_exit_status_in_both_forms("claim", 10, false, |json| {
        let mut command = Command::new(jit_binary());
        command.current_dir(&temp).args(["claim", "status"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    });
}

/// A downstream reader that closes the pipe mid-write (e.g. `| head`) makes jit
/// exit quietly with the SIGPIPE exit-status convention (`128 + 13 = 141`)
/// instead of panicking through std's `print!`/`println!` machinery — matching
/// `*`/141 (jit:6f881a85). A shown issue with a 2 MiB description exceeds the
/// maximum pipe capacity in both its plain and `--json` forms, so the writer is
/// still active after the reader closes. The file-based update avoids argv's
/// per-argument size limit.
#[test]
fn test_command_exit_codes_broken_pipe_emits_141() {
    let temp = setup();
    let id = create_issue(&temp, "Pipe payload", &[]);
    let description_path = temp.path().join("large-description.txt");
    fs::write(&description_path, "x".repeat(2 * 1024 * 1024)).unwrap();
    assert!(Command::new(jit_binary())
        .current_dir(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description-file",
            description_path.to_str().unwrap(),
        ])
        .status()
        .unwrap()
        .success());
    let expected = documented_exit_code("*", Some(141), true)
        .expect("broken-pipe row must have a fixed status");
    let broken_pipe_status = |json| {
        let mut command = Command::new(jit_binary());
        command
            .current_dir(&temp)
            .args(["issue", "show", &id])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if json {
            command.arg("--json");
        }
        let mut child = command.spawn().unwrap();
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
        (child.wait().unwrap().code(), stderr)
    };

    let (plain_status, plain_stderr) = broken_pipe_status(false);
    let (json_status, json_stderr) = broken_pipe_status(true);
    for (form, status, stderr) in [
        ("plain", plain_status, plain_stderr),
        ("machine-readable", json_status, json_stderr),
    ] {
        assert_eq!(
            status,
            Some(expected),
            "{form} broken-pipe exit must match its projected row; stderr: {stderr}"
        );
        assert!(
            !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
            "{form} broken-pipe exit must not print a panic banner; stderr: {stderr}"
        );
    }
    assert_eq!(
        plain_status, json_status,
        "plain and machine-readable broken-pipe statuses must agree"
    );
}

/// Guard: **every** row in the projection — exception and standard alike — must be
/// bound to both public invocation forms by a subprocess test in this file.
/// A row added without a binding fails here, so no mapping in the reference is
/// hand-authored or left with a machine-readable blind spot.
#[test]
fn test_command_exit_codes_every_row_is_verified() {
    let verified: std::collections::HashSet<(String, Option<i32>)> = [
        // Pinned by the subprocess tests in this file.
        ("*", Some(1)),
        ("*", Some(5)),
        ("*", Some(10)),
        ("any command that writes an issue", Some(4)),
        ("gate evaluate, gate evaluate-all", Some(4)),
        ("gate evaluate, gate evaluate-all", Some(10)),
        ("*", Some(0)),
        ("*", Some(2)),
        ("*", Some(3)),
        ("dep add", Some(4)),
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
        ("config validate", Some(4)),
        ("doc check-links", Some(1)),
        ("doc check-links", Some(2)),
        ("gate preset apply", Some(3)),
        ("serve, serve --stop, serve --status", Some(1)),
        ("*", Some(141)),
        // Pass-through: both public forms use a deterministic child exit;
        // the sibling helper-level test covers the complete code range.
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
