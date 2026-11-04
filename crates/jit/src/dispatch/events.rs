//! `jit events` handlers.

use super::{count_message, resolve_issue_filter, CommandContext, HandlerResult, Outcome};
use crate::cli::EventCommands;
use crate::domain::event_catalog::EventTag;
use crate::output::EventListResponse;
use crate::storage::IssueStore;

fn tail<S: IssueStore>(ctx: &CommandContext<'_, S>, n: usize) -> HandlerResult<EventListResponse> {
    let events = ctx.executor.queries().query_events(None, None, n)?;
    let message = count_message(events.len(), "", " event(s)");
    Ok(Outcome::success_with(
        EventListResponse::new(events),
        message,
    ))
}

fn query<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    event_type: Option<EventTag>,
    issue_id: Option<String>,
    limit: usize,
) -> HandlerResult<EventListResponse> {
    let issue = issue_id
        .map(|id| resolve_issue_filter(ctx.executor, &id))
        .transpose()?;
    let events = ctx
        .executor
        .queries()
        .query_events(event_type, issue, limit)?;
    let message = count_message(events.len(), "", " event(s)");
    Ok(Outcome::success_with(
        EventListResponse::new(events),
        message,
    ))
}

command_group! {
    command: EventCommands,
    storage: [IssueStore],
    leaves: {
        "events_tail" => EventCommands::Tail { n } => tail(n) -> EventListResponse,
        "events_query" => EventCommands::Query { event_type, issue_id, limit }
            => query(event_type, issue_id, limit) -> EventListResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::memory_executor;
    use crate::output::{ExitCode, OutputMode};
    use crate::storage::InMemoryStorage;

    fn seeded_storage() -> InMemoryStorage {
        let storage = InMemoryStorage::new();
        let event = EventTag::LinkedCheckoutWriteOverridden.sample();
        let bytes =
            crate::repository_state::serialize_event(&event).expect("the sample event serializes");
        storage.add_data_file(
            "events.jsonl",
            &format!("{}\n", String::from_utf8(bytes).expect("UTF-8 event")),
        );
        storage
    }

    fn context_for<'a>(
        executor: &'a crate::commands::CommandExecutor<InMemoryStorage>,
        json: bool,
    ) -> CommandContext<'a, InMemoryStorage> {
        CommandContext {
            executor,
            mode: OutputMode { json, quiet: false },
        }
    }

    #[test]
    fn test_dispatch_tail_json_wraps_events_in_counted_envelope_with_message() {
        let executor = memory_executor(seeded_storage());
        let ctx = context_for(&executor, true);
        let mut out = Vec::new();

        let status = dispatch(
            EventCommands::Tail { n: 10 },
            &ctx,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let events = printed["events"].as_array().unwrap();
        assert_eq!(printed["count"], events.len());
        assert_eq!(
            printed["message"],
            count_message(events.len(), "", " event(s)")
        );
        assert_eq!(status, ExitCode::Success.into());
    }

    #[test]
    fn test_dispatch_query_human_prints_one_serialized_event_per_line() {
        let executor = memory_executor(seeded_storage());
        let ctx = context_for(&executor, false);
        let mut out = Vec::new();

        dispatch(
            EventCommands::Query {
                event_type: None,
                issue_id: None,
                limit: 50,
            },
            &ctx,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(!lines.is_empty());
        assert!(lines
            .iter()
            .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok()));
    }

    #[test]
    fn test_registered_output_schema_events_leaves_are_named_event_list_response() {
        for path in ["events_tail", "events_query"] {
            let registered = crate::dispatch::registered_output_schema(path).unwrap();

            assert_eq!(registered.success, "EventListResponse");
        }
    }
}
