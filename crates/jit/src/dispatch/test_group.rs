//! A test-only command group exercising the handler contract end to end.

use super::{CommandContext, Outcome};
use crate::output::{ErrorCode, ExitCode, HumanRender, OutputMode};
use crate::storage::IssueStore;
use crate::test_harness::TestHarness;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

pub(super) enum TestCommand {
    Count { label: String },
    Flag { reason: String },
    Fail,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct CountResponse {
    label: String,
    issues: usize,
}

impl HumanRender for CountResponse {
    fn render_human(&self) -> String {
        format!("{}: {}\n", self.label, self.issues)
    }
}

#[derive(Serialize, JsonSchema)]
pub(super) struct FlagReport {
    reason: String,
}

impl HumanRender for FlagReport {
    fn render_human(&self) -> String {
        format!("flagged: {}\n", self.reason)
    }
}

const FINDINGS_CODE: ErrorCode = ErrorCode::ValidationFailed;

fn count<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    label: String,
) -> anyhow::Result<Outcome<CountResponse>> {
    let issues = ctx.executor.storage().list_issues()?.len();
    Ok(Outcome::Success(CountResponse { label, issues }))
}

fn flag<S: IssueStore>(
    _ctx: &CommandContext<'_, S>,
    reason: String,
) -> anyhow::Result<Outcome<CountResponse>> {
    Ok(Outcome::Findings {
        code: FINDINGS_CODE,
        message: format!("flagged because {reason}"),
        report: Box::new(FlagReport { reason }),
    })
}

fn fail<S: IssueStore>(_ctx: &CommandContext<'_, S>) -> anyhow::Result<Outcome<FlagReport>> {
    anyhow::bail!("handler failed")
}

command_group! {
    command: TestCommand,
    storage: [IssueStore],
    leaves: {
        "test_count" => TestCommand::Count { label } => count(label) -> CountResponse,
        "test_flag" => TestCommand::Flag { reason } => flag(reason) -> CountResponse,
        "test_fail" => TestCommand::Fail => fail() -> FlagReport,
    }
}

/// What one dispatch wrote and returned.
struct Run {
    status: anyhow::Result<ExitCode>,
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
    assert_eq!(run.status.unwrap(), ExitCode::Success);
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
    assert_eq!(run.out, expected.render_human());
    assert_eq!(run.status.unwrap(), ExitCode::Success);
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
    assert_eq!(run.status.unwrap(), FINDINGS_CODE.exit_code());
    assert!(run.err.is_empty());
}

#[test]
fn test_dispatch_findings_human_writes_report_and_one_error_diagnostic() {
    let run = run(&TestHarness::new(), flag_command(), false);

    let report = FlagReport {
        reason: "drift".to_owned(),
    };
    assert_eq!(run.out, report.render_human());
    assert_eq!(run.err.lines().count(), 1);
    assert!(run.err.starts_with("Error: ") && run.err.contains("drift"));
    assert_eq!(run.status.unwrap(), FINDINGS_CODE.exit_code());
}

#[test]
fn test_dispatch_handler_error_propagates_without_writing() {
    let run = run(&TestHarness::new(), TestCommand::Fail, true);

    assert!(run.status.is_err());
    assert!(run.out.is_empty() && run.err.is_empty());
}

#[test]
fn test_output_schema_returns_each_registered_leaf_response_schema() {
    fn described<T: JsonSchema>() -> (String, Value) {
        (
            T::schema_name(),
            serde_json::to_value(schemars::schema_for!(T)).unwrap(),
        )
    }
    let leaves = [
        ("test_count", described::<CountResponse>()),
        ("test_flag", described::<CountResponse>()),
        ("test_fail", described::<FlagReport>()),
    ];
    for (path, (name, schema)) in leaves {
        let registered = output_schema(path).unwrap();
        assert_eq!(registered.success, name, "{path}");
        assert_eq!(registered.success_schema, Some(schema), "{path}");
    }
}

#[test]
fn test_output_schema_unregistered_leaf_returns_none() {
    assert!(output_schema("test_absent").is_none());
}
