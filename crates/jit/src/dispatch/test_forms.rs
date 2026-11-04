//! Test-only groups proving each `command_group!` form through its own handler.

use super::{registered_output_schema, CommandContext, Dispatched, Outcome};
use crate::output::{ExitCode, HumanRender, OutputMode};
use crate::schema::OutputSchema;
use crate::storage::{InMemoryStorage, IssueStore};
use crate::test_harness::TestHarness;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize, JsonSchema)]
struct Echo {
    word: String,
}

impl HumanRender for Echo {
    fn render_human(&self, mode: OutputMode) -> String {
        let header = if mode.quiet { "" } else { "echo\n" };
        format!("{header}{}\n", self.word)
    }
}

#[derive(Serialize, JsonSchema)]
struct Count {
    issues: usize,
}

impl HumanRender for Count {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}\n", self.issues)
    }
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
enum Shape {
    Echo(Echo),
    Count(Count),
}

impl HumanRender for Shape {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Echo(echo) => echo.render_human(mode),
            Self::Count(count) => count.render_human(mode),
        }
    }
}

#[derive(clap::Args)]
struct EchoArgs {
    word: String,
}

fn echo_of(args: EchoArgs) -> Outcome<Echo> {
    Outcome::success(Echo { word: args.word }).with_warnings(["careful"])
}

fn mode(json: bool, quiet: bool) -> OutputMode {
    OutputMode { json, quiet }
}

/// What one dispatch wrote.
struct Run {
    out: String,
    err: String,
}

fn captured(
    call: impl FnOnce(&mut Vec<u8>, &mut Vec<u8>) -> anyhow::Result<ExitCode>,
) -> (Run, ExitCode) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = call(&mut out, &mut err).unwrap();
    (
        Run {
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        },
        status,
    )
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(OutputSchema::of::<T>().success_schema).unwrap()
}

fn registered_schema(path: &str) -> Value {
    serde_json::to_value(registered_output_schema(path).unwrap().success_schema).unwrap()
}

mod free {
    use super::*;

    pub(super) enum Command {
        Say { word: String },
    }

    fn say(word: String) -> anyhow::Result<Outcome<Echo>> {
        Ok(echo_of(EchoArgs { word }))
    }

    fn ping() -> anyhow::Result<Outcome<Count>> {
        Ok(Outcome::success_with(Count { issues: 0 }, "pong"))
    }

    fn echo(args: EchoArgs) -> anyhow::Result<Outcome<Echo>> {
        Ok(echo_of(args))
    }

    command_group! {
        command: Command,
        leaves: {
            "test_free_say" => Command::Say { word } => say(word) -> Echo,
        }
    }

    pub(super) mod single_args {
        use super::*;
        command_group! {
            command: EchoArgs,
            leaves: { "test_free_args" => args => echo(args) -> Echo }
        }
    }

    pub(super) mod single_unit {
        use super::*;
        command_group! {
            command: (),
            leaves: { "test_free_unit" => () => ping() -> Count }
        }
    }
}

mod with_executor {
    use super::*;

    fn echo<S: IssueStore>(
        _: &CommandContext<'_, S>,
        args: EchoArgs,
    ) -> anyhow::Result<Outcome<Echo>> {
        Ok(echo_of(args))
    }

    fn ping<S: IssueStore>(ctx: &CommandContext<'_, S>) -> anyhow::Result<Outcome<Count>> {
        let issues = ctx.executor.storage().list_issues()?.len();
        Ok(Outcome::success(Count { issues }))
    }

    pub(super) mod single_args {
        use super::*;
        command_group! {
            command: EchoArgs,
            storage: [IssueStore],
            leaves: { "test_exec_args" => args => echo(args) -> Echo }
        }
    }

    pub(super) mod single_unit {
        use super::*;
        command_group! {
            command: (),
            storage: [IssueStore],
            leaves: { "test_exec_unit" => () => ping() -> Count }
        }
    }
}

mod concrete {
    use super::*;

    fn count(ctx: &CommandContext<'_, InMemoryStorage>) -> anyhow::Result<Outcome<Count>> {
        let issues = ctx.executor.storage().list_issues()?.len();
        Ok(Outcome::success(Count { issues }))
    }

    command_group! {
        command: (),
        storage: InMemoryStorage,
        leaves: { "test_concrete_count" => () => count() -> Count }
    }
}

mod union {
    use super::*;

    pub(super) enum Command {
        Echo(String),
        Count,
    }

    fn pick(_: OutputMode, command: Command) -> Shape {
        match command {
            Command::Echo(word) => Shape::Echo(Echo { word }),
            Command::Count => Shape::Count(Count { issues: 0 }),
        }
    }

    fn shape(selection: Selection) -> anyhow::Result<Outcome<Shape>> {
        Ok(Outcome::success(pick(OutputMode::default(), selection.0)))
    }

    pub(super) struct Selection(pub Command);

    command_group! {
        command: Selection,
        leaves: {
            "test_union_shape" => selection => shape(selection) -> Shape
                as union [Echo, Count] "an Echo or a Count",
        }
    }
}

mod split {
    use super::*;

    pub(super) enum Command {
        Owned,
        Remaining(u8),
    }

    fn owned() -> anyhow::Result<Outcome<Echo>> {
        Ok(Outcome::success(Echo {
            word: "owned".to_owned(),
        }))
    }

    command_group! {
        command: Command,
        leaves: { "test_split_owned" => Command::Owned => owned() -> Echo },
        unmigrated: Command::Remaining(_)
    }
}

pub(super) fn output_schema(cmd_path: &str) -> Option<OutputSchema> {
    free::output_schema(cmd_path)
        .or_else(|| free::single_args::output_schema(cmd_path))
        .or_else(|| free::single_unit::output_schema(cmd_path))
        .or_else(|| with_executor::single_args::output_schema(cmd_path))
        .or_else(|| with_executor::single_unit::output_schema(cmd_path))
        .or_else(|| concrete::output_schema(cmd_path))
        .or_else(|| union::output_schema(cmd_path))
        .or_else(|| split::output_schema(cmd_path))
}

#[test]
fn test_executor_free_form_dispatches_with_output_mode_and_registers_schema() {
    let command = || free::Command::Say {
        word: "hi".to_owned(),
    };

    let (json, _) = captured(|out, err| free::dispatch(command(), mode(true, false), out, err));
    let (human, status) =
        captured(|out, err| free::dispatch(command(), mode(false, false), out, err));
    let (quiet, _) = captured(|out, err| free::dispatch(command(), mode(false, true), out, err));

    assert_eq!(
        serde_json::from_str::<Value>(&json.out).unwrap(),
        json!({ "word": "hi", "warnings": ["careful"] })
    );
    assert!(json.err.is_empty());
    assert_eq!(status, ExitCode::Success);
    assert_eq!(human.out, "echo\nhi\n");
    assert_eq!(human.err, "Warning: careful\n");
    assert_eq!(quiet.out, "hi\n");
    assert!(quiet.err.is_empty());
    assert_eq!(registered_schema("test_free_say"), schema_of::<Echo>());
}

#[test]
fn test_single_leaf_form_registers_args_and_unit_variants_with_and_without_an_executor() {
    let harness = TestHarness::new();
    harness.create_issue("one");
    let ctx = CommandContext {
        executor: &harness.executor,
        mode: mode(true, false),
    };
    let args = || EchoArgs {
        word: "w".to_owned(),
    };

    let (free_args, _) =
        captured(|o, e| free::single_args::dispatch(args(), mode(true, false), o, e));
    let (free_unit, _) = captured(|o, e| free::single_unit::dispatch((), mode(true, false), o, e));
    let (exec_args, _) = captured(|o, e| with_executor::single_args::dispatch(args(), &ctx, o, e));
    let (exec_unit, _) = captured(|o, e| with_executor::single_unit::dispatch((), &ctx, o, e));

    let printed = |run: &Run| serde_json::from_str::<Value>(&run.out).unwrap();
    assert_eq!(
        printed(&free_args),
        json!({ "word": "w", "warnings": ["careful"] })
    );
    assert_eq!(
        printed(&exec_args),
        json!({ "word": "w", "warnings": ["careful"] })
    );
    assert_eq!(
        printed(&free_unit),
        json!({ "issues": 0, "message": "pong" })
    );
    assert_eq!(printed(&exec_unit), json!({ "issues": 1 }));
    assert_eq!(registered_schema("test_free_args"), schema_of::<Echo>());
    assert_eq!(registered_schema("test_exec_args"), schema_of::<Echo>());
    assert_eq!(registered_schema("test_free_unit"), schema_of::<Count>());
    assert_eq!(registered_schema("test_exec_unit"), schema_of::<Count>());
}

#[test]
fn test_concrete_store_form_dispatches_over_its_named_store_and_registers_schema() {
    let harness = TestHarness::new();
    harness.create_issue("one");
    harness.create_issue("two");
    let ctx = CommandContext {
        executor: &harness.executor,
        mode: mode(true, false),
    };

    let (run, status) = captured(|out, err| concrete::dispatch((), &ctx, out, err));

    assert_eq!(
        serde_json::from_str::<Value>(&run.out).unwrap(),
        json!({ "issues": 2 })
    );
    assert_eq!(status, ExitCode::Success);
    assert_eq!(
        registered_schema("test_concrete_count"),
        schema_of::<Count>()
    );
}

#[test]
fn test_union_leaf_emits_each_shape_and_registers_one_of_over_the_member_types() {
    let (echo, _) = captured(|out, err| {
        union::dispatch(
            union::Selection(union::Command::Echo("e".to_owned())),
            mode(true, false),
            out,
            err,
        )
    });
    let (count, _) = captured(|out, err| {
        union::dispatch(
            union::Selection(union::Command::Count),
            mode(true, false),
            out,
            err,
        )
    });

    assert_eq!(
        serde_json::from_str::<Value>(&echo.out).unwrap(),
        json!({ "word": "e" })
    );
    assert_eq!(
        serde_json::from_str::<Value>(&count.out).unwrap(),
        json!({ "issues": 0 })
    );
    let registered = registered_output_schema("test_union_shape").unwrap();
    assert_eq!(registered.success, "Echo");
    assert_eq!(
        registered.success_schema,
        Some(json!({
            "oneOf": [schema_of::<Echo>(), schema_of::<Count>()],
            "description": "an Echo or a Count",
        }))
    );
}

#[test]
fn test_unmigrated_form_handles_owned_variants_and_returns_the_rest_unchanged() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let owned = split::dispatch(split::Command::Owned, mode(true, false), &mut out, &mut err);
    let remaining = split::dispatch(
        split::Command::Remaining(7),
        mode(true, false),
        &mut Vec::new(),
        &mut Vec::new(),
    );

    assert!(matches!(owned, Ok(Dispatched::Handled(ExitCode::Success))));
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap(),
        json!({ "word": "owned" })
    );
    assert!(matches!(
        remaining,
        Ok(Dispatched::Unhandled(split::Command::Remaining(7)))
    ));
    assert_eq!(registered_schema("test_split_owned"), schema_of::<Echo>());
}
