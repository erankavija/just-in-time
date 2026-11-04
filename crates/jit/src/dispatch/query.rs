//! `jit query` and `jit status` handlers.

use super::issue::{list as list_all, ISSUE_LISTING};
use super::{CommandContext, HandlerResult, Outcome};
use crate::cli::{CountDimension, IssueFilterArgs, QueryArgs, QueryCommands};
use crate::domain::Issue;
use crate::errors::InvalidArgumentError;
use crate::output::{
    BlockedListing, DivergenceResponse, ExitCode, IssueListing, QuerySubset, StateRollup,
    SubsetListing,
};
use crate::storage::IssueStore;

fn found(count: usize) -> String {
    format!("Found {count} issue(s)")
}

fn subset(subset: QuerySubset, issues: Vec<Issue>, full: bool) -> HandlerResult<SubsetListing> {
    let message = found(issues.len());
    Ok(Outcome::success_with(
        SubsetListing::new(subset, issues, full),
        message,
    ))
}

fn available<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<SubsetListing> {
    let issues = ctx.executor.queries().query_available(filters.parse()?)?;
    subset(QuerySubset::Available, issues, full)
}

fn strategic<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<SubsetListing> {
    let issues = ctx
        .executor
        .queries()
        .query_strategic_filtered(filters.parse()?)?;
    subset(QuerySubset::Strategic, issues, full)
}

fn closed<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<SubsetListing> {
    let issues = ctx
        .executor
        .queries()
        .query_closed_filtered(filters.parse()?)?;
    subset(QuerySubset::Closed, issues, full)
}

fn blocked<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<BlockedListing> {
    let blocked = ctx
        .executor
        .queries()
        .query_blocked_filtered(filters.parse()?)?;
    let message = found(blocked.len());
    Ok(Outcome::success_with(
        BlockedListing::new(&blocked, full),
        message,
    ))
}

fn count<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    by: CountDimension,
    filters: IssueFilterArgs,
) -> HandlerResult<StateRollup> {
    let filters = filters.parse()?;
    let rollup = match by {
        CountDimension::State => ctx.executor.queries().query_count_by_state(filters)?,
    };
    let message = format!(
        "{}/{} done ({}%)",
        rollup.extras.done, rollup.extras.total, rollup.extras.percent
    );
    Ok(Outcome::success_with(rollup, message))
}

fn divergence<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<DivergenceResponse> {
    let report = ctx.executor.graph().detect_divergences()?;
    let message = format!("{} divergence(s)", report.count());
    Ok(Outcome::success_with(report, message))
}

command_group! {
    command: QueryCommands,
    storage: [IssueStore],
    leaves: {
        "query_all" => QueryCommands::All { filters, full } => list_all(filters, full) -> IssueListing
            as union [crate::output::IssueListResponse, crate::output::IssueListFullResponse]
            ISSUE_LISTING,
        "query_available" => QueryCommands::Available { filters, full } => available(filters, full) -> SubsetListing
            as union [crate::output::IssueListResponse, crate::output::IssueListFullResponse]
            ISSUE_LISTING,
        "query_blocked" => QueryCommands::Blocked { filters, full } => blocked(filters, full) -> BlockedListing
            as union [crate::output::BlockedListResponse, crate::output::BlockedFullListResponse]
            "Default shape: MinimalBlockedIssue entries (lean fields plus blocked_reasons strings). With --full: BlockedIssue entries (lean fields plus structured blocked_reasons). Neither shape carries gate-list fields; a blocking gate appears only as a reason entry.",
        "query_strategic" => QueryCommands::Strategic { filters, full } => strategic(filters, full) -> SubsetListing
            as union [crate::output::IssueListResponse, crate::output::IssueListFullResponse]
            ISSUE_LISTING,
        "query_closed" => QueryCommands::Closed { filters, full } => closed(filters, full) -> SubsetListing
            as union [crate::output::IssueListResponse, crate::output::IssueListFullResponse]
            ISSUE_LISTING,
        "query_count" => QueryCommands::Count { by, filters } => count(by, filters) -> StateRollup,
        "query_divergence" => QueryCommands::Divergence => divergence() -> DivergenceResponse,
    }
}

/// Bare `jit query`: the `query all` listing under the parent's own filters.
pub mod bare {
    use super::*;

    command_group! {
        command: (IssueFilterArgs, bool),
        storage: [IssueStore],
        leaves: {
            "query" => (filters, full) => list_all(filters, full) -> IssueListing
                as union [crate::output::IssueListResponse, crate::output::IssueListFullResponse]
                ISSUE_LISTING,
        }
    }
}

/// `jit status`.
pub mod status {
    use super::*;
    use crate::output::StatusResponse;

    fn summary<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<StatusResponse> {
        let status = ctx.executor.validation().get_status()?;
        let message = format!(
            "{} backlog, {} ready, {} in_progress, {} done",
            status.backlog, status.ready, status.in_progress, status.done
        );
        Ok(Outcome::success_with(status, message))
    }

    command_group! {
        command: (),
        storage: [IssueStore],
        leaves: {
            "status" => () => summary() -> StatusResponse,
        }
    }
}

/// Reject parent-level filters placed before a subcommand, which would
/// otherwise be dropped silently.
fn reject_parent_filters(
    subcommand: &QueryCommands,
    parent: &IssueFilterArgs,
    full: bool,
) -> anyhow::Result<()> {
    let offending: Vec<&str> = [
        parent.state.as_ref().map(|_| "--state"),
        parent.assignee.as_ref().map(|_| "--assignee"),
        parent.priority.as_ref().map(|_| "--priority"),
        (!parent.labels.is_empty()).then_some("--label"),
        full.then_some("--full"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if offending.is_empty() {
        return Ok(());
    }
    let sub = match subcommand {
        QueryCommands::All { .. } => "all",
        QueryCommands::Available { .. } => "available",
        QueryCommands::Blocked { .. } => "blocked",
        QueryCommands::Strategic { .. } => "strategic",
        QueryCommands::Closed { .. } => "closed",
        QueryCommands::Count { .. } => "count",
        QueryCommands::Divergence => "divergence",
    };
    let flags = offending.join(" ");
    Err(InvalidArgumentError::new(format!(
        "filter(s) {flags} were given before the `{sub}` subcommand, where they \
         are ignored. Put them after the subcommand (e.g. `jit query {sub} {flags}`), \
         or drop the subcommand to use the bare form (e.g. `jit query {flags}`)."
    ))
    .into())
}

/// Run `jit query`: its subcommand, or the bare listing when none is given.
pub fn run<S: IssueStore>(
    args: QueryArgs,
    ctx: &CommandContext<'_, S>,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
) -> anyhow::Result<ExitCode> {
    let QueryArgs {
        filters,
        subcommand,
        full,
    } = args;
    match subcommand {
        None => bare::dispatch((filters, full), ctx, out, err),
        Some(subcommand) => {
            reject_parent_filters(&subcommand, &filters, full)?;
            dispatch(subcommand, ctx, out, err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::registered_output_schema;
    use crate::output::OutputMode;
    use crate::test_harness::TestHarness;
    use serde_json::Value;

    fn emitted(
        harness: &TestHarness,
        args: QueryArgs,
        json: bool,
    ) -> (anyhow::Result<ExitCode>, String) {
        let ctx = CommandContext {
            executor: &harness.executor,
            mode: OutputMode { json, quiet: false },
        };
        let mut out = Vec::new();
        let status = run(args, &ctx, &mut out, &mut Vec::new());
        (status, String::from_utf8(out).unwrap())
    }

    fn args(subcommand: Option<QueryCommands>, filters: IssueFilterArgs, full: bool) -> QueryArgs {
        QueryArgs {
            filters,
            subcommand,
            full,
        }
    }

    fn available_leaf() -> QueryCommands {
        QueryCommands::Available {
            filters: IssueFilterArgs::default(),
            full: false,
        }
    }

    #[test]
    fn test_run_parent_filter_before_subcommand_fails_naming_flags_and_subcommand() {
        let parent = IssueFilterArgs {
            state: Some("ready".to_owned()),
            labels: vec!["epic:x".to_owned()],
            ..IssueFilterArgs::default()
        };

        let (status, out) = emitted(
            &TestHarness::new(),
            args(Some(available_leaf()), parent, true),
            true,
        );

        let error = status.unwrap_err();
        let invalid = error.downcast_ref::<InvalidArgumentError>().unwrap();
        let message = invalid.to_string();
        assert!(message.contains("--state --label --full") && message.contains("`available`"));
        assert!(out.is_empty());
    }

    #[test]
    fn test_run_bare_query_lists_like_query_all_under_the_parent_filters() {
        let harness = TestHarness::new();
        harness.create_issue("one");
        let bare = || IssueFilterArgs::default();
        let all = QueryCommands::All {
            filters: bare(),
            full: false,
        };

        let (_, bare_out) = emitted(&harness, args(None, bare(), false), true);
        let (_, all_out) = emitted(&harness, args(Some(all), bare(), false), true);

        assert_eq!(bare_out, all_out);
        let printed: Value = serde_json::from_str(&bare_out).unwrap();
        assert_eq!(printed["count"], 1);
    }

    #[test]
    fn test_run_available_json_carries_found_message_and_issue_list() {
        let harness = TestHarness::new();
        harness.create_ready_issue("ready one");

        let (status, out) = emitted(
            &harness,
            args(Some(available_leaf()), IssueFilterArgs::default(), false),
            true,
        );

        assert_eq!(status.unwrap(), ExitCode::Success);
        let printed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(printed["message"], "Found 1 issue(s)");
        assert_eq!(printed["issues"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_registered_output_schema_types_every_query_and_status_leaf() {
        for (path, name) in [
            ("query", "IssueListResponse"),
            ("query_all", "IssueListResponse"),
            ("query_available", "IssueListResponse"),
            ("query_strategic", "IssueListResponse"),
            ("query_closed", "IssueListResponse"),
            ("query_blocked", "BlockedListResponse"),
            ("query_count", "StateRollup"),
            ("query_divergence", "DivergenceResponse"),
            ("status", "StatusResponse"),
        ] {
            let registered = registered_output_schema(path).unwrap();

            assert_eq!(registered.success, name, "{path}");
            assert!(registered.success_schema.is_some(), "{path}");
        }
    }
}
