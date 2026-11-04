//! A test-only command group exercising the handler contract end to end.

use super::{registered_output_schema, CommandContext, HandlerResult, Outcome};
use crate::output::{ErrorCode, ExitCode, HumanRender, OutputMode, RunStatus};
use crate::schema::CommandSchema;
use crate::storage::IssueStore;
use crate::test_harness::TestHarness;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

pub(super) enum TestCommand {
    Count { label: String },
    Flag { reason: String },
    Undeclared { code: ErrorCode },
    Fail,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct CountResponse {
    label: String,
    issues: usize,
}

impl HumanRender for CountResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}: {}\n", self.label, self.issues)
    }
}

#[derive(Serialize, JsonSchema)]
pub(super) struct FlagReport {
    reason: String,
}

impl HumanRender for FlagReport {
    fn render_human(&self, _: OutputMode) -> String {
        format!("flagged: {}\n", self.reason)
    }
}

const FINDINGS_CODE: ErrorCode = ErrorCode::ValidationFailed;

fn count<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    label: String,
) -> HandlerResult<CountResponse> {
    let issues = ctx.executor.storage().list_issues()?.len();
    Ok(CountResponse { label, issues }.into())
}

fn flag<S: IssueStore>(
    _ctx: &CommandContext<'_, S>,
    reason: String,
) -> HandlerResult<CountResponse> {
    Ok(Outcome::findings(
        FINDINGS_CODE,
        format!("flagged because {reason}"),
        FlagReport { reason },
    ))
}

fn undeclared<S: IssueStore>(
    _ctx: &CommandContext<'_, S>,
    code: ErrorCode,
) -> HandlerResult<CountResponse> {
    Ok(Outcome::findings(
        code,
        "undeclared",
        FlagReport {
            reason: String::new(),
        },
    ))
}

fn fail<S: IssueStore>(_ctx: &CommandContext<'_, S>) -> HandlerResult<FlagReport> {
    anyhow::bail!("handler failed")
}

command_group! {
    command: TestCommand,
    storage: [IssueStore],
    leaves: {
        "test_count" => TestCommand::Count { label } => count(label) -> CountResponse,
        "test_flag" => TestCommand::Flag { reason } => flag(reason)
            findings ValidationFailed "flagged" -> CountResponse,
        "test_undeclared" => TestCommand::Undeclared { code } => undeclared(code) -> CountResponse,
        "test_fail" => TestCommand::Fail => fail() -> FlagReport,
    }
}

/// What one dispatch wrote and returned.
struct Run {
    status: anyhow::Result<RunStatus>,
    out: String,
    err: String,
}

fn run(harness: &TestHarness, command: TestCommand, json: bool) -> Run {
    let ctx = CommandContext {
        executor: &harness.executor,
        mode: OutputMode { json, quiet: false },
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = dispatch(command, &ctx, &mut out, &mut err);
    Run {
        status,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn flag_command() -> TestCommand {
    TestCommand::Flag {
        reason: "drift".to_owned(),
    }
}

fn count_command() -> TestCommand {
    TestCommand::Count {
        label: "open".to_owned(),
    }
}

#[test]
fn test_dispatch_success_json_writes_handler_response_as_success_envelope() {
    let harness = TestHarness::new();
    harness.create_issue("one");
    let run = run(&harness, count_command(), true);

    let printed: Value = serde_json::from_str(&run.out).unwrap();
    let expected = CountResponse {
        label: "open".to_owned(),
        issues: harness.storage.list_issues().unwrap().len(),
    };
    assert_eq!(printed, serde_json::to_value(expected).unwrap());
    assert_eq!(run.status.unwrap(), ExitCode::Success.into());
    assert!(run.err.is_empty());
}

#[test]
fn test_dispatch_success_human_writes_response_rendering() {
    let harness = TestHarness::new();
    let run = run(&harness, count_command(), false);

    let expected = CountResponse {
        label: "open".to_owned(),
        issues: harness.storage.list_issues().unwrap().len(),
    };
    assert_eq!(run.out, expected.render_human(OutputMode::default()));
    assert_eq!(run.status.unwrap(), ExitCode::Success.into());
    assert!(run.err.is_empty());
}

#[test]
fn test_dispatch_findings_json_writes_failure_envelope_with_report_details() {
    let run = run(&TestHarness::new(), flag_command(), true);

    let printed: Value = serde_json::from_str(&run.out).unwrap();
    let report = FlagReport {
        reason: "drift".to_owned(),
    };
    assert_eq!(
        printed["error"]["code"],
        serde_json::to_value(FINDINGS_CODE).unwrap()
    );
    assert_eq!(
        printed["error"]["details"],
        serde_json::to_value(report).unwrap()
    );
    assert_eq!(run.status.unwrap(), FINDINGS_CODE.exit_code().into());
    assert!(run.err.is_empty());
}

#[test]
fn test_dispatch_findings_human_writes_report_and_unprefixed_message_line() {
    let run = run(&TestHarness::new(), flag_command(), false);

    let report = FlagReport {
        reason: "drift".to_owned(),
    };
    assert_eq!(run.out, report.render_human(OutputMode::default()));
    assert_eq!(run.err, "flagged because drift\n");
    assert_eq!(run.status.unwrap(), FINDINGS_CODE.exit_code().into());
}

#[test]
fn test_dispatch_findings_under_undeclared_code_fails_without_writing() {
    let command = TestCommand::Undeclared {
        code: ErrorCode::GateFailed,
    };
    let run = run(&TestHarness::new(), command, true);

    assert!(run
        .status
        .unwrap_err()
        .to_string()
        .contains("test_undeclared"));
    assert!(run.out.is_empty() && run.err.is_empty());
}

#[test]
fn test_dispatch_findings_under_universal_code_need_no_declaration() {
    let code = ErrorCode::LockTimeout;
    let run = run(&TestHarness::new(), TestCommand::Undeclared { code }, true);

    assert_eq!(run.status.unwrap(), code.exit_code().into());
}

#[test]
fn test_dispatch_handler_error_propagates_without_writing() {
    let run = run(&TestHarness::new(), TestCommand::Fail, true);

    assert!(run.status.is_err());
    assert!(run.out.is_empty() && run.err.is_empty());
}

#[test]
fn test_registered_group_dispatches_and_resolves_its_leaf_schemas_through_registry() {
    fn described<T: JsonSchema>() -> (String, Value) {
        (
            T::schema_name(),
            serde_json::to_value(schemars::schema_for!(T)).unwrap(),
        )
    }
    let harness = TestHarness::new();
    assert_eq!(
        run(&harness, count_command(), true).status.unwrap(),
        ExitCode::Success.into()
    );

    let leaves = [
        ("test_count", described::<CountResponse>()),
        ("test_flag", described::<CountResponse>()),
        ("test_fail", described::<FlagReport>()),
    ];
    for (path, (name, schema)) in leaves {
        let registered = registered_output_schema(path).unwrap();
        assert_eq!(registered.success, name, "{path}");
        assert_eq!(registered.success_schema, Some(schema), "{path}");

        let resolved = CommandSchema::get_output_schema_for_command(path).unwrap();
        assert_eq!(resolved.success, name, "{path}");
        assert_eq!(resolved.success_schema, registered.success_schema, "{path}");
    }
}

#[test]
fn test_registered_output_schema_unregistered_leaf_returns_none() {
    assert!(registered_output_schema("test_absent").is_none());
}

#[test]
fn test_output_schema_test_leaves_are_distinct_from_every_real_command_path() {
    fn leaf_paths(command: &clap::Command, path: &str, paths: &mut Vec<String>) {
        paths.push(path.to_owned());
        for sub in command.get_subcommands() {
            leaf_paths(sub, &format!("{path}_{}", sub.get_name()), paths);
        }
    }
    let cli = <crate::cli::Cli as clap::CommandFactory>::command();
    let mut paths = Vec::new();
    for top in cli.get_subcommands() {
        leaf_paths(top, top.get_name(), &mut paths);
    }
    let shadowed: Vec<_> = paths
        .iter()
        .filter(|path| {
            output_schema(path).is_some() || super::test_forms::output_schema(path).is_some()
        })
        .collect();
    assert!(shadowed.is_empty(), "{shadowed:?}");
}
