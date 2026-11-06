//! Bind every `command_exit_codes` row, universal or derived from a leaf's
//! declared codes, to one case that runs a real subprocess in both public
//! invocation forms.
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
        cli.command_with_env(["issue", "create", "Queued"], [("JIT_LOCK_TIMEOUT", "0")]);
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

/// `gate evaluate-many` over `target` (`None`: the issue requiring the gate)
/// after the gate's checker is set to `checker_command`.
fn evaluate_many_output(checker_command: &str, target: Option<&str>, json: bool) -> Output {
    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, checker_command);
    let target = match target {
        Some(target) => target.to_owned(),
        None => id,
    };
    evaluate_many_in(&cli, &target, json)
}

fn evaluate_many_in(cli: &JitCli, target: &str, json: bool) -> Output {
    let output = output_in_form(
        cli,
        &[
            "gate",
            "evaluate-many",
            "projection-gate",
            target,
            "--force",
        ],
        json,
    );
    assert_batch_report(&output, json);
    output
}

fn assert_batch_report(output: &Output, json: bool) {
    if json {
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            envelope["error"]["details"]["results"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    } else {
        assert!(String::from_utf8_lossy(&output.stdout).contains("projection-gate"));
    }
}

fn invoke_gate_evaluate_many_parse_error(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, "true");
    fs::write(cli.root().join(".jit/gates.toml"), "gates = [").unwrap();
    vec![evaluate_many_in(&cli, &id, json)]
}

#[cfg(unix)]
fn invoke_gate_evaluate_many_permission_denied(json: bool) -> Vec<Output> {
    use std::os::unix::fs::PermissionsExt;

    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, "true");
    let registry = cli.root().join(".jit/gates.toml");
    let permissions = fs::metadata(&registry).unwrap().permissions();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o000)).unwrap();
    let output = evaluate_many_in(&cli, &id, json);
    fs::set_permissions(&registry, permissions).unwrap();
    vec![output]
}

#[cfg(unix)]
fn invoke_gate_evaluate_many_mutation_lock_timeout(json: bool) -> Vec<Output> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Duration;

    let cli = JitCli::initialised().unwrap();
    let id = setup_auto_gate_issue(&cli, "true");
    let record = cli.root().join(".jit/issues").join(format!("{id}.json"));
    let bytes = fs::read(&record).unwrap();
    fs::remove_file(&record).unwrap();
    nix::unistd::mkfifo(
        &record,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();

    // The FIFO read starts after bootstrap, before the gate's mutation session.
    let (ready, reader_started) = std::sync::mpsc::channel();
    let fifo = record.clone();
    let writer = std::thread::spawn(move || {
        ready
            .send(fs::OpenOptions::new().write(true).open(fifo).unwrap())
            .unwrap();
    });
    let mut command = cli.command_with_env(
        ["gate", "evaluate-many", "projection-gate", &id, "--force"],
        [("JIT_LOCK_TIMEOUT", "0")],
    );
    if json {
        command.arg("--json");
    }
    let child = std::thread::spawn(move || {
        assert_cmd::Command::from_std(command)
            .timeout(Duration::from_secs(5))
            .output()
            .unwrap()
    });
    let mut pipe = match reader_started.recv_timeout(Duration::from_secs(5)) {
        Ok(pipe) => pipe,
        Err(error) => {
            let _unblock = fs::OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_NONBLOCK)
                .open(&record)
                .unwrap();
            writer.join().unwrap();
            let output = child.join().unwrap();
            panic!("gate never read the fixture record: {error}; {output:?}");
        }
    };
    writer.join().unwrap();
    let _held = jit::storage::FileLocker::new(Duration::from_secs(1))
        .lock_exclusive(
            &cli.root()
                .join(".jit")
                .join(jit::repository_state::EVENTS_LOCK_FILE),
        )
        .unwrap();
    fs::remove_file(&record).unwrap();
    fs::write(&record, &bytes).unwrap();
    pipe.write_all(&bytes).unwrap();
    drop(pipe);
    let output = child.join().unwrap();
    assert_batch_report(&output, json);
    vec![output]
}

/// `gate evaluate-many` over a target whose checker ran and failed.
fn invoke_gate_evaluate_many_failure(json: bool) -> Vec<Output> {
    vec![evaluate_many_output("false", None, json)]
}

/// `gate evaluate-many` over a target whose checker cannot run.
fn invoke_gate_evaluate_many_runner_error(json: bool) -> Vec<Output> {
    vec![evaluate_many_output(
        "jit-projection-command-does-not-exist",
        None,
        json,
    )]
}

/// `gate evaluate-many` over an id that resolves to no issue.
fn invoke_gate_evaluate_many_missing_target(json: bool) -> Vec<Output> {
    vec![evaluate_many_output("true", Some("0000000000000000"), json)]
}

/// `gate evaluate-many` over a too-short id prefix.
fn invoke_gate_evaluate_many_invalid_target(json: bool) -> Vec<Output> {
    vec![evaluate_many_output("true", Some("ab"), json)]
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

/// `jit validate` on repository-integrity findings, error-severity rule
/// findings, branch drift and invalid leases: the one `validate` row covers all.
fn invoke_validate(json: bool) -> Vec<Output> {
    [
        invoke_validate_integrity,
        invoke_validate_rule_findings,
        invoke_validate_branch_drift,
        invoke_validate_leases,
    ]
    .iter()
    .flat_map(|invoke| invoke(json))
    .collect()
}

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
    let id = cli
        .create_issue([
            "--title",
            "An epic",
            "--label",
            "type:epic",
            "--label",
            "epic:auth",
        ])
        .unwrap();
    vec![
        output_in_form(&cli, &["validate"], json),
        output_in_form(&cli, &["validate", &id], json),
        output_in_form(&cli, &["validate", &id, "--explain"], json),
    ]
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
/// as a preset findings outcome.
fn invoke_gate_preset_apply_missing_target(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    vec![output_in_form(
        &cli,
        &["gate", "preset", "apply", "plan-review", "0000000000000000"],
        json,
    )]
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
/// `apply` and `dep add` closing a dependency cycle: they share one row.
fn invoke_apply_and_dep_add_cycle(json: bool) -> Vec<Output> {
    [invoke_apply_prospective_cycle, invoke_dep_add_cycle]
        .iter()
        .flat_map(|invoke| invoke(json))
        .collect()
}

fn invoke_dep_add_cycle(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let first = cli.create_issue(["--title", "First"]).unwrap();
    let second = cli.create_issue(["--title", "Second"]).unwrap();

    // second -> first is fine.
    cli.run(["dep", "add", &second, &first]).unwrap();

    // first -> second closes the cycle.
    let output = output_in_form(&cli, &["dep", "add", &first, &second], json);
    if json {
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["error"]["code"], "CYCLE_DETECTED");
        assert!(output.stderr.is_empty());
    }
    vec![output]
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

/// A gate key, a pack archive and a snapshot output that already exist.
fn invoke_occupied_target(json: bool) -> Vec<Output> {
    [
        invoke_gate_define_duplicate,
        invoke_profile_pack_occupied_output,
        invoke_snapshot_export_occupied,
    ]
    .iter()
    .flat_map(|invoke| invoke(json))
    .collect()
}

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

/// Git-required command families reject an initialized repository without Git.
fn invoke_commands_without_git(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    [
        ["claim", "status"],
        ["hooks", "install"],
        ["worktree", "info"],
        ["worktree", "list"],
    ]
    .into_iter()
    .map(|args| output_in_form(&cli, &args, json))
    .collect()
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

fn invoke_profile_conflict(command: &str, json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let package = jit::test_utils::profile_package_fixture("planner-asset-only");
    jit::test_utils::copy_package_tree(&package, &cli.root().join("packages/left"));
    jit::test_utils::copy_package_tree(&package, &cli.root().join("packages/right"));
    let manifest = cli.root().join("packages/right/manifest.toml");
    let declared = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        declared.replace("id = \"planner-asset-only\"", "id = \"rival\""),
    )
    .unwrap();
    fs::write(
        cli.root().join("packages/right/assets/profile.txt"),
        "different bytes\n",
    )
    .unwrap();
    let output = output_in_form(
        &cli,
        &[
            "profile",
            command,
            "--profile",
            "packages/left",
            "--profile",
            "packages/right",
        ],
        json,
    );
    if json {
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            envelope["error"]["code"],
            if command == "diff" {
                "VALIDATION_FAILED"
            } else {
                "PROFILE_CONFLICT"
            }
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        if command == "diff" {
            assert!(envelope["error"]["details"]["profiles"].is_array());
        }
    }
    vec![output]
}

fn invoke_profile_apply_conflict(json: bool) -> Vec<Output> {
    invoke_profile_conflict("apply", json)
}

fn invoke_profile_diff_conflict(json: bool) -> Vec<Output> {
    invoke_profile_conflict("diff", json)
}

fn pack_profile_fixture(cli: &JitCli) {
    let package = jit::test_utils::profile_package_fixture("planner-asset-only");
    jit::test_utils::copy_package_tree(&package, &cli.root().join("packages/profile"));
    cli.run([
        "profile",
        "pack",
        "--source",
        "packages/profile",
        "--output",
        "package.tar",
    ])
    .unwrap();
}

fn invoke_profile_pack_occupied_output(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    pack_profile_fixture(&cli);
    vec![output_in_form(
        &cli,
        &[
            "profile",
            "pack",
            "--source",
            "packages/profile",
            "--output",
            "package.tar",
        ],
        json,
    )]
}

fn invoke_profile_add_occupied_destination(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    pack_profile_fixture(&cli);
    vec![output_in_form(
        &cli,
        &[
            "profile",
            "add",
            "--archive",
            "package.tar",
            "--destination",
            "packages/profile",
        ],
        json,
    )]
}

fn invoke_retryable_reads(json: bool) -> Vec<Output> {
    let cli = JitCli::initialised().unwrap();
    let package = jit::test_utils::profile_package_fixture("planner-asset-only");
    jit::test_utils::copy_package_tree(&package, &cli.root().join("packages/profile"));
    let scratch = cli.root().join(".jit/tmp");
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("redo.record"), b"pending").unwrap();
    let manifest = cli.root().join("batch.json");
    fs::write(&manifest, "[]").unwrap();
    vec![
        output_in_form(&cli, &["archive", "candidates"], json),
        output_in_form(&cli, &["doc", "check-links", "--scope", "all"], json),
        output_in_form(&cli, &["graph", "roots"], json),
        output_in_form(&cli, &["invariant", "check"], json),
        output_in_form(
            &cli,
            &["profile", "diff", "--profile", "packages/profile"],
            json,
        ),
        output_in_form(&cli, &["query", "all"], json),
        output_in_form(&cli, &["issue", "list"], json),
        output_in_form(
            &cli,
            &[
                "issue",
                "batch-create",
                "--from-json",
                manifest.to_str().unwrap(),
                "--dry-run",
            ],
            json,
        ),
        output_in_form(&cli, &["item", "list"], json),
        output_in_form(&cli, &["label", "namespaces"], json),
        output_in_form(&cli, &["label", "values", "type"], json),
        output_in_form(&cli, &["config", "show-hierarchy"], json),
        output_in_form(&cli, &["validate"], json),
    ]
}

row_bindings! {
    test_command_exit_codes_success_emits_0: ("*", 0, false) => invoke_success;
    test_command_exit_codes_retryable_reads_emit_12: ("archive candidates, archive document, archive container, config show-hierarchy, doc list, doc show, doc dir, doc conformance, doc history, doc diff, doc check-links, graph deps, graph rdeps, graph roots, graph tree, graph export, rdeps, invariant check, profile diff, profile apply, profile pack, profile add, query all, query available, query blocked, query strategic, query closed, query count, query divergence, query, status, issue show, issue list, issue search, issue batch-create, issue update, issue claim-next, snapshot export, item list, item search, item show, item resolve, label namespaces, label values, validate", 12, false) => invoke_retryable_reads;
    test_command_exit_codes_malformed_stored_record_emits_1: ("*", 1, false) => invoke_malformed_stored_record;
    test_command_exit_codes_invalid_argument_emits_2: ("*", 2, false) => invoke_invalid_argument;
    test_command_exit_codes_not_found_emits_3: ("*", 3, false) => invoke_not_found;
    #[cfg(unix)]
    test_command_exit_codes_permission_denied_emits_5: ("*", 5, false) => invoke_permission_denied;
    test_command_exit_codes_unsupported_format_emits_10: ("*", 10, false) => invoke_unsupported_format;
    test_command_exit_codes_expired_lock_wait_emits_11: ("*", 11, false) => invoke_expired_lock_wait;
    test_command_exit_codes_broken_pipe_emits_141: ("*", 141, true) => invoke_broken_pipe;
    test_command_exit_codes_commands_without_git_emit_10: ("claim acquire, claim release, claim renew, claim heartbeat, claim status, claim list, claim force-evict, hooks install, worktree info, worktree list", 10, false) => invoke_commands_without_git;
    test_command_exit_codes_config_validate_invalid_value_emits_4: ("config validate", 4, true) => invoke_config_validate;
    test_command_exit_codes_apply_and_dep_add_cycle_emit_4: ("apply, dep add", 4, false) => invoke_apply_and_dep_add_cycle;
    test_command_exit_codes_doc_check_links_emits_4: ("doc check-links", 4, true) => invoke_doc_check_links;
    test_command_exit_codes_occupied_target_emits_6: ("gate define, profile pack, snapshot export", 6, false) => invoke_occupied_target;
    test_command_exit_codes_gate_preset_apply_missing_target_emits_3: ("gate preset apply", 3, true) => invoke_gate_preset_apply_missing_target;
    test_command_exit_codes_gate_status_all_emits_4: ("gate status-all", 4, true) => invoke_gate_status_all;
    test_command_exit_codes_gate_evaluate_failure_emits_4: ("gate evaluate, gate evaluate-all", 4, false) => invoke_gate_evaluate_failure;
    test_command_exit_codes_gate_evaluate_many_parse_error_emits_1: ("gate evaluate-many", 1, true) => invoke_gate_evaluate_many_parse_error;
    #[cfg(unix)]
    test_command_exit_codes_gate_evaluate_many_permission_denied_emits_5: ("gate evaluate-many", 5, true) => invoke_gate_evaluate_many_permission_denied;
    #[cfg(unix)]
    test_command_exit_codes_gate_evaluate_many_mutation_lock_timeout_emits_11: ("gate evaluate-many", 11, true) => invoke_gate_evaluate_many_mutation_lock_timeout;
    test_command_exit_codes_gate_evaluate_many_failure_emits_4: ("gate evaluate-many", 4, true) => invoke_gate_evaluate_many_failure;
    test_command_exit_codes_gate_evaluate_many_runner_error_emits_10: ("gate evaluate-many", 10, true) => invoke_gate_evaluate_many_runner_error;
    test_command_exit_codes_gate_evaluate_many_missing_target_emits_3: ("gate evaluate-many", 3, true) => invoke_gate_evaluate_many_missing_target;
    test_command_exit_codes_gate_evaluate_many_invalid_target_emits_2: ("gate evaluate-many", 2, true) => invoke_gate_evaluate_many_invalid_target;
    test_command_exit_codes_invariant_check_emits_4: ("invariant check", 4, true) => invoke_invariant_check;
    test_command_exit_codes_profile_diff_conflicting_findings_emit_4: ("profile diff", 4, true) => invoke_profile_diff_conflict;
    test_command_exit_codes_profile_apply_conflict_emits_4: ("profile apply", 4, false) => invoke_profile_apply_conflict;
    test_command_exit_codes_profile_add_occupied_destination_emits_4: ("profile add", 4, false) => invoke_profile_add_occupied_destination;
    test_command_exit_codes_enforcing_write_rule_emits_4: ("issue create", 4, false) => invoke_enforcing_write_rule;
    test_command_exit_codes_blocked_transition_emits_4: ("issue update, issue claim, issue claim-next", 4, false) => invoke_blocked_transition;
    test_command_exit_codes_issue_delete_unconfirmed_emits_2: ("issue delete", 2, false) => invoke_issue_delete_unconfirmed;
    test_command_exit_codes_search_missing_ripgrep_emits_1: ("search", 1, false) => invoke_search_missing_ripgrep;
    #[cfg(unix)]
    test_command_exit_codes_serve_foreground_exit_emits_1: ("serve", 1, false) => invoke_serve_foreground_exit;
    test_command_exit_codes_validate_findings_emits_4: ("validate", 4, true) => invoke_validate;
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
