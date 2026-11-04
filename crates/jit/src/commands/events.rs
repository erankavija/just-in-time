//! Event log operations

use super::*;
use crate::domain::event_catalog::EventTag;
use crate::domain::identity::IssueIdPrefix;

impl<S: IssueStore> QueryOps<'_, S> {
    /// Newest-first events matching `event_type` and the `issue` filter.
    pub fn query_events(
        &self,
        event_type: Option<EventTag>,
        issue: Option<IssueIdPrefix>,
        limit: usize,
    ) -> Result<Vec<Event>> {
        let events = self.exec.storage.read_events()?;

        let filtered: Vec<Event> = events
            .into_iter()
            .rev()
            .filter(|e| {
                if let Some(ref et) = event_type {
                    if e.body().tag() != *et {
                        return false;
                    }
                }
                if let Some(ref issue) = issue {
                    if !e.issue_id().is_some_and(|id| issue.matches(id)) {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect();

        Ok(filtered.into_iter().rev().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event_catalog::EventTag;
    use crate::domain::identity::IssueIdPrefix;
    use crate::domain::EventBody;
    use crate::domain::LinkedCheckoutWriteStance;
    use crate::storage::InMemoryStorage;

    fn seed_override_event(storage: &InMemoryStorage) -> Event {
        let event = EventTag::LinkedCheckoutWriteOverridden.sample();
        let bytes = crate::repository_state::serialize_event(&event)
            .expect("the override event serializes");
        let line = format!(
            "{}\n",
            String::from_utf8(bytes).expect("serialized event is UTF-8")
        );
        storage.add_data_file("events.jsonl", &line);
        event
    }

    /// REQ-03: an override-audit record in the log is read back and reported by
    /// the event query, and its event-type filter selects it by its stable tag.
    #[test]
    fn test_query_events_reports_linked_checkout_write_override_record() {
        let storage = InMemoryStorage::new();
        let expected = seed_override_event(&storage);
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let events = executor
            .queries()
            .query_events(Some(EventTag::LinkedCheckoutWriteOverridden), None, 10)
            .expect("the event query succeeds");

        assert_eq!(events, vec![expected]);
        let EventBody::LinkedCheckoutWriteOverridden {
            declared_stance,
            invocation_override,
            ..
        } = events[0].body()
        else {
            panic!("the event query reports the override record");
        };
        assert_eq!(*declared_stance, LinkedCheckoutWriteStance::Refuse);
        assert_eq!(*invocation_override, LinkedCheckoutWriteStance::Allow);
    }

    /// REQ-03: the record is repository-scoped, so the unfiltered query reports
    /// it while an issue-id filter never does.
    #[test]
    fn test_query_events_excludes_repository_scoped_override_record_from_issue_filter() {
        let storage = InMemoryStorage::new();
        let expected = seed_override_event(&storage);
        let executor = crate::commands::test_helpers::memory_executor(storage);
        assert_eq!(
            executor
                .queries()
                .query_events(None, None, 10)
                .expect("the event query succeeds"),
            vec![expected],
            "the unfiltered query reports the seeded record",
        );

        assert!(
            executor
                .queries()
                .query_events(
                    None,
                    Some(IssueIdPrefix::from(&crate::test_utils::issue_id(
                        0x100000000
                    ))),
                    10,
                )
                .expect("the event query succeeds")
                .is_empty(),
            "an issue-id filter reports no repository-scoped record",
        );
    }
}
