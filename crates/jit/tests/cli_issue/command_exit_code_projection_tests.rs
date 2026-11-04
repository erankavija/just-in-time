//! REQ-02: bind every `command_exit_codes` row to the runtime behavior it
//! documents, so no mapping in the generated reference is hand-authored.
//!
//! Each projected row is bound here by one case that runs a real subprocess in
//! both public invocation forms. The classifier tests in `output::failure`
//! remain useful unit coverage, but cannot substitute for a command-level
//! binding because they do not exercise parsing, dispatch, or `--json` output
//! selection.
//!
//! `test_command_exit_codes_every_row_is_verified` compares the projection with
//! the rows the binding cases declare: a row without a case, or with two, fails.

use jit::schema::CommandSchema;
use jit::storage::worktree_paths::WorktreePaths;
use jit::test_cli::{jit_binary, JitCli};
use jit::test_lease_store::{corrupt_lease, LeaseCorruption};
use std::fs;
use std::io::Read;
use std::process::{Output, Stdio};
use tempfile::TempDir;

/// Define the auto gate `projection-gate` over `checker_command` and create an
/// issue requiring it.
fn setup_auto_gate_issue(cli: &JitCli, checker_command: &str) -> String {
    cli.run([
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
    .unwrap();
    cli.create_issue(["--title", "Gated projection", "--gate", "projection-gate"])
        .unwrap()
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

fn setup_prospective_cycle(cli: &JitCli) -> (String, String) {
    fs::write(cli.root().join(".jit/config.toml"), CYCLIC_APPLY_CONFIG).unwrap();
    fs::write(
        cli.root().join(".jit/templates.toml"),
        CYCLIC_APPLY_TEMPLATE,
    )
    .unwrap();
    let upstream = cli
        .create_issue(["--title", "Upstream", "--type", "task"])
        .unwrap();
    let container = cli
        .create_issue(["--title", "Container", "--type", "epic"])
        .unwrap();
    cli.run(["dep", "add", &container, &upstream]).unwrap();
    (container, upstream)
}

/// The projection's exit status for `command`, asserting its exception
/// classification.
fn documented_exit_code(command: &str, code: i32, exception: bool) -> i32 {
    let schema = CommandSchema::generate();
    let row = schema
        .command_exit_codes
        .iter()
        .find(|c| c.command == command && c.code == code)
        .unwrap_or_else(|| {
            panic!("command_exit_codes projection has no row for `{command}` code {code}")
        });
    assert_eq!(
        row.exception, exception,
        "row for `{command}` code {code} has wrong exception flag"
    );
    row.code
}

/// What a binding case observes in one invocation form: every outcome the row's
/// condition produces, in a fixed order.
type Invoke = fn(bool) -> Vec<Output>;

/// Run `args` in `cli`, with `--json` in the machine-readable form.
fn output_in_form(cli: &JitCli, args: &[&str], json: bool) -> Output {
    let mut command = cli.command(args);
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}

/// Bind one projected row to both public invocation forms.
///
/// The expected status comes from the generated projection, not a second
/// hand-authored numeric assertion. Running the same documented condition with
/// and without `--json` also makes a disagreement between the two forms fail.
fn assert_row_status_in_both_forms(command: &str, code: i32, exception: bool, invoke: Invoke) {
    let expected = documented_exit_code(command, code, exception);
    let (plain, json) = (invoke(false), invoke(true));
    assert_eq!(
        plain.len(),
        json.len(),
        "`{command}` observes the same outcomes in both forms"
    );
    plain
        .iter()
        .zip(&json)
        .enumerate()
        .for_each(|(index, (plain, json))| {
            let (plain_status, json_status) = (plain.status.code(), json.status.code());
            assert!(
                plain_status.is_some(),
                "plain `{command}` outcome {index} must exit with a status"
            );
            assert_eq!(
                plain_status,
                Some(expected),
                "plain `{command}` outcome {index} must match its projected row; stderr: {}",
                String::from_utf8_lossy(&plain.stderr)
            );
            assert_eq!(
                json_status,
                Some(expected),
                "machine-readable `{command}` outcome {index} must match its projected row; \
                 stderr: {}",
                String::from_utf8_lossy(&json.stderr)
            );
            assert_eq!(
                plain_status, json_status,
                "plain and machine-readable `{command}` outcome {index} statuses must agree"
            );
        });
}

/// Declare one binding case per projected row: each entry is a test that binds
/// its row in both invocation forms, and the same entry lists the row in
/// [`BOUND_ROWS`], which the completeness check compares with the projection.
macro_rules! row_bindings {
    ($($(#[$meta:meta])* $test:ident: ($command:expr, $code:expr, $exception:expr) => $invoke:path;)*) => {
        const BOUND_ROWS: &[(&str, i32)] = &[$($(#[$meta])* ($command, $code),)*];
        $($(#[$meta])* #[test]
        fn $test() {
            assert_row_status_in_both_forms($command, $code, $exception, $invoke);
        })*
    };
}

/// `*`/0: a healthy `issue list`, and a risky (deep relative) but valid link that
/// `doc check-links` reports as a warning only.
fn invoke_success(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let listed = output_in_form(&cli, &["issue", "list"], json);

    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doc host"]).unwrap();
    fs::create_dir_all(cli.root().join("docs/subdir")).unwrap();
    fs::create_dir_all(cli.root().join("assets")).unwrap();
    fs::write(
        cli.root().join("docs/subdir/risky.md"),
        "# Risky\n\n![Asset](../../assets/image.png)\n",
    )
    .unwrap();
    fs::write(cli.root().join("assets/image.png"), b"data").unwrap();
    cli.run(["doc", "add", &id, "docs/subdir/risky.md", "--label", "Test"])
        .unwrap();
    let warned = output_in_form(&cli, &["doc", "check-links", "--scope", "all"], json);
    vec![listed, warned]
}

/// `*`/1: a malformed stored record is a parse failure no typed classifier maps
/// to a more specific status.
fn invoke_malformed_stored_record(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/index.json"), b"{").unwrap();
    vec![output_in_form(&cli, &["query", "all"], json)]
}

/// `*`/2: a missing template and a too-short id prefix are invalid invocations.
fn invoke_invalid_argument(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let container = cli.create_issue(["--title", "Container"]).unwrap();
    vec![
        output_in_form(
            &cli,
            &["apply", "definitely-missing-template", container.as_str()],
            json,
        ),
        output_in_form(&cli, &["issue", "show", "ab"], json),
    ]
}

/// `*`/3: an id that resolves to nothing, in each namespace that resolves one.
fn invoke_not_found(json: bool) -> Vec<Output> {
    const MISSING: &str = "0000000000000000";
    let cli = JitCli::initialised().unwrap();
    let existing = cli.create_issue(["--title", "Existing issue"]).unwrap();
    vec![
        output_in_form(&cli, &["issue", "show", "0123456789abcdef"], json),
        output_in_form(&cli, &["dep", "add", &existing, MISSING], json),
        output_in_form(&cli, &["issue", "claim", MISSING, "agent:parity"], json),
        output_in_form(&cli, &["gate", "add", MISSING, "cargo-ci"], json),
    ]
}

/// `*`/5: an unreadable index exercises the filesystem-permission row through
/// the top-level query path (rather than a handler-owned mutation response).
#[cfg(unix)]
fn invoke_permission_denied(json: bool) -> Vec<Output> {
    use std::os::unix::fs::PermissionsExt;

    let cli = JitCli::initialised().unwrap();
    let index = cli.root().join(".jit/index.json");
    let mut denied = fs::metadata(&index).unwrap().permissions();
    denied.set_mode(0o000);
    fs::set_permissions(&index, denied).unwrap();

    let output = output_in_form(&cli, &["query", "all"], json);

    let mut restored = fs::metadata(&index).unwrap().permissions();
    restored.set_mode(0o755);
    fs::set_permissions(&index, restored).unwrap();
    vec![output]
}

/// `*`/10: a repository format other than the binary's supported one fails
/// before command dispatch.
fn invoke_unsupported_format(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let index_path = cli.root().join(".jit/index.json");
    let mut index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    index["schema_version"] = serde_json::json!(9999);
    fs::write(index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();
    vec![output_in_form(&cli, &["query", "all"], json)]
}

/// `*`/11: a mutation queued behind another holder of the repository lock chain,
/// with a wait shorter than the hold.
fn invoke_expired_lock_wait(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let _held = jit::storage::FileLocker::new(std::time::Duration::from_secs(5))
        .lock_exclusive(&cli.root().join(".jit-bootstrap.lock"))
        .unwrap();
    let mut command =
        cli.command_with_env(["issue", "create", "Queued"], [("JIT_LOCK_TIMEOUT", "1")]);
    if json {
        command.arg("--json");
    }
    vec![command.output().unwrap()]
}

/// `*`/141: a downstream reader that closes the pipe mid-write (e.g. `| head`)
/// makes jit exit quietly with the SIGPIPE convention instead of panicking
/// through std's `print!` machinery (jit:6f881a85). A shown issue with a 2 MiB
/// description exceeds the maximum pipe capacity in both forms, so the writer is
/// still active after the reader closes. The file-based update avoids argv's
/// per-argument size limit.
fn invoke_broken_pipe(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Pipe payload"]).unwrap();
    let description_path = cli.root().join("large-description.txt");
    fs::write(&description_path, "x".repeat(2 * 1024 * 1024)).unwrap();
    cli.run([
        "issue",
        "update",
        &id,
        "--description-file",
        description_path.to_str().unwrap(),
    ])
    .unwrap();

    let mut command = cli.command(["issue", "show", &id]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
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

    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    let text = String::from_utf8_lossy(&stderr);
    assert!(
        !text.contains("panicked") && !text.contains("RUST_BACKTRACE"),
        "broken-pipe exit must not print a panic banner; stderr: {text}"
    );
    vec![Output {
        status: child.wait().unwrap(),
        stdout: Vec::new(),
        stderr,
    }]
}

/// An enforcing validation rule rejects an issue create before publication,
/// exercising the shared write-validation row through a real writer.
fn invoke_enforcing_write_rule(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let rules_path = cli.root().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).unwrap();
    rules.push_str(
        "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = true\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    );
    fs::write(rules_path, rules).unwrap();
    vec![output_in_form(
        &cli,
        &[
            "issue",
            "create",
            "--title",
            "Unqualified epic",
            "--type",
            "epic",
        ],
        json,
    )]
}

/// A checker that ran and failed is the gate-evaluation verdict-failure row.
fn invoke_gate_evaluate_failure(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, "false");
    vec![output_in_form(
        &cli,
        &["gate", "evaluate", &id, "projection-gate", "--force"],
        json,
    )]
}

/// A checker that cannot run to a verdict is the gate-evaluation runner-error
/// row. The unique command is intentionally absent from the test environment.
fn invoke_gate_evaluate_runner_error(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, "jit-projection-command-does-not-exist");
    vec![output_in_form(
        &cli,
        &["gate", "evaluate", &id, "projection-gate", "--force"],
        json,
    )]
}

/// The prospective-cycle guard rejects template application before it mutates.
fn invoke_apply_prospective_cycle(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let (container, upstream) = setup_prospective_cycle(&cli);
    let upstream_binding = format!("upstream={upstream}");
    vec![output_in_form(
        &cli,
        &["apply", "cyclic", &container, "--anchor", &upstream_binding],
        json,
    )]
}

/// `jit validate` on repository-integrity findings.
fn invoke_validate_integrity(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Corruptible"]).unwrap();

    let issue_path = cli
        .root()
        .join(".jit")
        .join("issues")
        .join(format!("{id}.json"));
    let mut issue: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue["dependencies"] = serde_json::json!(["00000000-0000-4000-8000-00000000dead"]);
    fs::write(&issue_path, serde_json::to_string_pretty(&issue).unwrap()).unwrap();

    vec![output_in_form(&cli, &["validate"], json)]
}

/// Whole-repo `jit validate` on an error-severity rule finding. A non-enforced
/// error rule reports without diverting through the integrity (exit-4) path.
fn invoke_validate_rule_findings(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let rules_path = cli.root().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).unwrap();
    rules.push_str(
        "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = false\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    );
    fs::write(rules_path, rules).unwrap();
    // An epic missing its `req:*` label violates the error-severity rule; the
    // `epic:auth` identity label keeps the finding to the rule under test.
    cli.create_issue([
        "--title",
        "An epic",
        "--label",
        "type:epic",
        "--label",
        "epic:auth",
    ])
    .unwrap();
    vec![output_in_form(&cli, &["validate"], json)]
}

/// `jit validate --branch-drift`; with no git upstream the check cannot succeed.
fn invoke_validate_branch_drift(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    vec![output_in_form(&cli, &["validate", "--branch-drift"], json)]
}

/// `jit validate --leases` over a malformed lease record, so lease validation
/// cannot produce a clean result.
fn invoke_validate_leases(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    corrupt_lease(
        &paths.shared_jit,
        &jit::test_utils::issue_id(1),
        LeaseCorruption::MalformedJson,
    )
    .unwrap();
    vec![output_in_form(&cli, &["validate", "--leases"], json)]
}

/// `jit gate status-all` while a required gate is unpassed.
fn invoke_gate_status_all(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "--title",
        "Tests",
        "--description",
        "Tests",
        "tests",
    ])
    .unwrap();
    let id = cli
        .create_issue(["--title", "Gated", "--gate", "tests"])
        .unwrap();
    vec![output_in_form(&cli, &["gate", "status-all", &id], json)]
}

/// `jit invariant check` on enforcement drift (an `enforced-by` naming a missing
/// rule).
fn invoke_invariant_check(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    fs::write(
        cli.root().join(".jit/rules.toml"),
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    )
    .unwrap();
    fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    )
    .unwrap();
    vec![output_in_form(&cli, &["invariant", "check"], json)]
}

/// `jit config validate` on an invalid configuration.
fn invoke_config_validate(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), "not = valid = toml").unwrap();
    vec![output_in_form(&cli, &["config", "validate"], json)]
}

/// `jit item` over an unreadable config (list, search) and over a value that is
/// not a qualified id (show, resolve).
fn invoke_item_failures(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/config.toml"), "not = valid = toml").unwrap();
    let mut outputs = vec![
        output_in_form(&cli, &["item", "list"], json),
        output_in_form(&cli, &["item", "search", "survey"], json),
    ];
    fs::write(cli.root().join(".jit/config.toml"), "").unwrap();
    outputs.push(output_in_form(
        &cli,
        &["item", "show", "not-a-qualified-address"],
        json,
    ));
    outputs.push(output_in_form(
        &cli,
        &["item", "resolve", "not-a-qualified-address"],
        json,
    ));
    outputs
}

/// `jit doc check-links` on a broken link.
fn invoke_doc_check_links(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doc host"]).unwrap();
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join("docs/broken.md"),
        "# Doc\n\nSee [missing](nonexistent.md).\n",
    )
    .unwrap();
    cli.run(["doc", "add", &id, "docs/broken.md", "--label", "Test"])
        .unwrap();
    vec![output_in_form(
        &cli,
        &["doc", "check-links", "--scope", "all"],
        json,
    )]
}

/// `jit gate preset apply` on a well-formed but nonexistent id, which fails
/// through the ordinary not-found classifier.
fn invoke_gate_preset_apply_missing_target(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    vec![output_in_form(
        &cli,
        &["gate", "preset", "apply", "plan-review", "0000000000000000"],
        json,
    )]
}

/// `jit init` over a `--values-file` path that cannot be read.
fn invoke_init_missing_values_file(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    vec![output_in_form(
        &cli,
        &["init", "--values-file", "missing-values.toml"],
        json,
    )]
}

/// `jit serve --status` (like the daemon start and `--stop`) classifies a
/// malformed PID file as `PARSE_ERROR`, whose registered status is 1.
fn invoke_serve_daemon_error(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    // A malformed PID file makes `server_status` (via `read_pid_file`) error, so
    // the `--status` arm routes the typed parser failure through the shared
    // classifier.
    fs::write(cli.root().join(".jit/server.pid.json"), "not json").unwrap();
    vec![output_in_form(&cli, &["serve", "--status"], json)]
}

/// Copy the real CLI beside a deterministic fake `jit-server` child.
///
/// `find_server_binary` prefers a sibling of the current executable, so this
/// fixture drives the public foreground dispatch without starting a long-lived
/// server or relying on PATH.
#[cfg(unix)]
fn jit_with_exiting_server(cli: &JitCli, exit_code: i32) -> std::path::PathBuf {
    let bin_dir = cli.root().join("serve-fg-bin");
    fs::create_dir(&bin_dir).unwrap();

    let jit_copy = bin_dir.join("jit");
    jit::test_utils::write_executable(&jit_copy, fs::read(jit_binary()).unwrap()).unwrap();

    jit::test_utils::write_executable(
        &bin_dir.join("jit-server"),
        format!("#!/bin/sh\nexit {exit_code}\n"),
    )
    .unwrap();

    jit_copy
}

/// `jit serve --fg` reports a failing server child as `SERVER_EXITED`, carrying
/// the child's status in `error.details`.
#[cfg(unix)]
fn invoke_serve_foreground_exit(json: bool) -> Vec<Output> {
    const CHILD_EXIT: i32 = 42;

    let cli = JitCli::initialised().unwrap();
    let jit = jit_with_exiting_server(&cli, CHILD_EXIT);
    let mut command = cli.program_command(&jit, ["serve", "--fg", "--port", "0"]);
    if json {
        command.arg("--json");
    }
    let output = command.output().unwrap();
    if json {
        let response: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("serve --fg --json result");
        assert_eq!(response["error"]["code"], "SERVER_EXITED");
        assert_eq!(response["error"]["details"]["exit_code"], CHILD_EXIT);
    }
    vec![output]
}

/// `jit dep add` when the edge would close a cycle.
fn invoke_dep_add_cycle(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let first = cli.create_issue(["--title", "First"]).unwrap();
    let second = cli.create_issue(["--title", "Second"]).unwrap();

    // second -> first is fine.
    cli.run(["dep", "add", &second, &first]).unwrap();

    // first -> second closes the cycle.
    vec![output_in_form(&cli, &["dep", "add", &first, &second], json)]
}

/// Build a dependency-blocked pair: `(dependency, dependent)`, where `dependent`
/// depends on the still-unmet `dependency`.
fn blocked_pair(cli: &JitCli) -> (String, String) {
    let dependency = cli.create_issue(["--title", "Dependency"]).unwrap();
    let dependent = cli.create_issue(["--title", "Dependent"]).unwrap();
    cli.run(["dep", "add", &dependent, &dependency]).unwrap();
    (dependency, dependent)
}

/// All three commands of the row exit 4 on a blocked transition: `update` and
/// `claim` on unmet dependencies, `claim-next` on an unpassed precheck gate.
fn invoke_blocked_transition(json: bool) -> Vec<Output> {
    // `issue update` into `ready`, which the unmet dependency blocks (a dependency
    // must reach a terminal state before its dependent becomes ready).
    let cli = JitCli::initialised().unwrap();
    let (_dependency, dependent) = blocked_pair(&cli);
    let update = output_in_form(
        &cli,
        &["issue", "update", &dependent, "--state", "ready"],
        json,
    );
    let claim = output_in_form(&cli, &["issue", "claim", &dependent, "agent:test"], json);

    let cli = JitCli::initialised().unwrap();
    cli.run([
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
    .unwrap();
    cli.create_issue(["--title", "Precheck work", "--gate", "precheck-gate"])
        .unwrap();
    let claim_next = output_in_form(&cli, &["issue", "claim-next", "agent:test"], json);
    vec![update, claim, claim_next]
}

/// `jit gate define` when the key is already registered.
fn invoke_gate_define_duplicate(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let define = [
        "gate",
        "define",
        "--title",
        "Tests",
        "--description",
        "Tests",
        "tests",
    ];
    assert!(output_in_form(&cli, &define, false).status.success());
    vec![output_in_form(&cli, &define, json)]
}

/// `jit issue delete` refused for missing operator confirmation
/// (`JIT_ALLOW_DELETION=1` not set; jit:0daba57d).
fn invoke_issue_delete_unconfirmed(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doomed"]).unwrap();
    vec![output_in_form(&cli, &["issue", "delete", &id], json)]
}

/// `jit issue batch-create` when pre-validation rejects the file (an entry
/// depending on a key the file never defines).
fn invoke_batch_create_prevalidation(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let batch = cli.root().join("batch.json");
    fs::write(
        &batch,
        r#"[{"key": "a", "title": "A", "depends_on": ["ghost"]}]"#,
    )
    .unwrap();
    vec![output_in_form(
        &cli,
        &[
            "issue",
            "batch-create",
            "--from-json",
            batch.to_str().unwrap(),
        ],
        json,
    )]
}

/// `jit snapshot export` when the output path is already occupied.
fn invoke_snapshot_export_occupied(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["--title", "Snapshot me"]).unwrap();
    fs::create_dir(cli.root().join("taken")).unwrap();
    vec![output_in_form(
        &cli,
        &["snapshot", "export", "--out", "taken", "--working-tree"],
        json,
    )]
}

/// A lease subcommand run outside a git repository. `JitCli::initialised()`
/// inits `.jit` without a git repository, which is exactly that case.
fn invoke_claim_without_git(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    vec![output_in_form(&cli, &["claim", "status"], json)]
}

/// `search`/1: ripgrep is not on `PATH`, so the search classifies to
/// `RIPGREP_NOT_FOUND`, which exits 1.
fn invoke_search_missing_ripgrep(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let empty_path = TempDir::new().unwrap();
    let mut command = cli.command_with_env(
        ["search", "needle"],
        [("PATH", empty_path.path().as_os_str())],
    );
    if json {
        command.arg("--json");
    }
    vec![command.output().unwrap()]
}

/// `search`/10: an executable `rg` the kernel cannot run (no shebang, so
/// `ENOEXEC`) passes the `PATH` lookup and fails at spawn, an I/O failure that
/// classifies to `IO_ERROR`, which exits 10.
#[cfg(unix)]
fn invoke_search_unspawnable_ripgrep(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let tools = TempDir::new().unwrap();
    jit::test_utils::write_executable(&tools.path().join("rg"), "not a program\n").unwrap();
    let mut command =
        cli.command_with_env(["search", "needle"], [("PATH", tools.path().as_os_str())]);
    if json {
        command.arg("--json");
    }
    vec![command.output().unwrap()]
}

row_bindings! {
    test_command_exit_codes_success_emits_0: ("*", 0, false) => invoke_success;
    test_command_exit_codes_malformed_stored_record_emits_1: ("*", 1, false) => invoke_malformed_stored_record;
    test_command_exit_codes_invalid_argument_emits_2: ("*", 2, false) => invoke_invalid_argument;
    test_command_exit_codes_not_found_emits_3: ("*", 3, false) => invoke_not_found;
    #[cfg(unix)]
    test_command_exit_codes_permission_denied_emits_5: ("*", 5, false) => invoke_permission_denied;
    test_command_exit_codes_unsupported_format_emits_10: ("*", 10, false) => invoke_unsupported_format;
    test_command_exit_codes_expired_lock_wait_emits_11: ("*", 11, false) => invoke_expired_lock_wait;
    test_command_exit_codes_broken_pipe_emits_141: ("*", 141, true) => invoke_broken_pipe;
    test_command_exit_codes_enforcing_write_rule_emits_4: ("any command that writes an issue", 4, false) => invoke_enforcing_write_rule;
    test_command_exit_codes_gate_evaluate_failure_emits_4: ("gate evaluate, gate evaluate-all, gate evaluate-many", 4, true) => invoke_gate_evaluate_failure;
    test_command_exit_codes_gate_evaluate_runner_error_emits_10: ("gate evaluate, gate evaluate-all, gate evaluate-many", 10, true) => invoke_gate_evaluate_runner_error;
    test_command_exit_codes_apply_prospective_cycle_emits_4: ("apply", 4, false) => invoke_apply_prospective_cycle;
    test_command_exit_codes_validate_integrity_emits_4: ("validate", 4, true) => invoke_validate_integrity;
    test_command_exit_codes_validate_rule_findings_emits_1: ("validate", 1, true) => invoke_validate_rule_findings;
    test_command_exit_codes_validate_branch_drift_emits_1: ("validate --branch-drift", 1, true) => invoke_validate_branch_drift;
    test_command_exit_codes_validate_leases_emits_1: ("validate --leases", 1, true) => invoke_validate_leases;
    test_command_exit_codes_gate_status_all_emits_4: ("gate status-all", 4, true) => invoke_gate_status_all;
    test_command_exit_codes_invariant_check_emits_4: ("invariant check", 4, true) => invoke_invariant_check;
    test_command_exit_codes_item_failures_emit_2: ("item list, item search, item show, item resolve", 2, false) => invoke_item_failures;
    test_command_exit_codes_config_validate_invalid_value_emits_4: ("config validate", 4, false) => invoke_config_validate;
    test_command_exit_codes_doc_check_links_emits_1: ("doc check-links", 1, true) => invoke_doc_check_links;
    test_command_exit_codes_init_missing_values_file_emits_2: ("init", 2, false) => invoke_init_missing_values_file;
    test_command_exit_codes_gate_preset_apply_missing_target_emits_3: ("gate preset apply", 3, false) => invoke_gate_preset_apply_missing_target;
    test_command_exit_codes_serve_daemon_error_emits_1: ("serve, serve --stop, serve --status", 1, false) => invoke_serve_daemon_error;
    #[cfg(unix)]
    test_command_exit_codes_serve_foreground_exit_emits_1: ("serve --fg", 1, false) => invoke_serve_foreground_exit;
    test_command_exit_codes_dep_add_cycle_emits_4: ("dep add", 4, false) => invoke_dep_add_cycle;
    test_command_exit_codes_blocked_transition_emits_4: ("issue update, issue claim, issue claim-next", 4, false) => invoke_blocked_transition;
    test_command_exit_codes_gate_define_duplicate_emits_6: ("gate define", 6, false) => invoke_gate_define_duplicate;
    test_command_exit_codes_issue_delete_unconfirmed_emits_2: ("issue delete", 2, false) => invoke_issue_delete_unconfirmed;
    test_command_exit_codes_batch_create_prevalidation_emits_2: ("issue batch-create", 2, false) => invoke_batch_create_prevalidation;
    test_command_exit_codes_snapshot_export_occupied_emits_6: ("snapshot export", 6, false) => invoke_snapshot_export_occupied;
    test_command_exit_codes_claim_without_git_emits_10: ("claim", 10, false) => invoke_claim_without_git;
    test_command_exit_codes_search_missing_ripgrep_emits_1: ("search", 1, false) => invoke_search_missing_ripgrep;
    #[cfg(unix)]
    test_command_exit_codes_search_unspawnable_ripgrep_emits_10: ("search", 10, false) => invoke_search_unspawnable_ripgrep;
}

/// Guard: **every** projected row, exception and standard alike, has exactly one
/// binding case in [`row_bindings!`]. A row added without a binding, or bound
/// twice, fails here. Unix-only: the `*`/5 and `serve --fg` rows are bound by
/// cases that need Unix permissions and shell scripts.
#[cfg(unix)]
#[test]
fn test_command_exit_codes_every_row_is_verified() {
    let mut bound: Vec<(String, i32)> = BOUND_ROWS
        .iter()
        .map(|(command, code)| (command.to_string(), *code))
        .collect();
    let mut projected: Vec<(String, i32)> = CommandSchema::generate()
        .command_exit_codes
        .into_iter()
        .map(|c| (c.command, c.code))
        .collect();
    bound.sort();
    projected.sort();

    assert_eq!(
        projected, bound,
        "every projected row must have exactly one binding case; \
         left = projection, right = bound rows"
    );
}
