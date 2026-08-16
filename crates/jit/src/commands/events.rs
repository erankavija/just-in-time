//! Event log operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn tail_events(&self, n: usize) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;
        let start = events.len().saturating_sub(n);
        Ok(events[start..].to_vec())
    }

    pub fn query_events(
        &self,
        event_type: Option<String>,
        issue_id: Option<String>,
        limit: usize,
    ) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;

        let filtered: Vec<Event> = events
            .into_iter()
            .rev()
            .filter(|e| {
                if let Some(ref et) = event_type {
                    if e.get_type() != et {
                        return false;
                    }
                }
                if let Some(ref iid) = issue_id {
                    if e.get_issue_id() != iid {
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
    use crate::domain::{EventTag, LinkedCheckoutWriteStance};
    use crate::storage::InMemoryStorage;

    fn seeded_override_executor() -> (CommandExecutor<InMemoryStorage>, Event) {
        let storage = InMemoryStorage::new();
        let event = EventTag::LinkedCheckoutWriteOverridden.sample();
        let bytes = crate::repository_state::serialize_event(&event)
            .expect("the override event serializes");
        let line = format!(
            "{}\n",
            String::from_utf8(bytes).expect("serialized event is UTF-8")
        );
        storage.add_data_file("events.jsonl", &line);
        (CommandExecutor::new(storage), event)
    }

    /// REQ-03: an override-audit record in the log is read back and reported by
    /// the event query, and its event-type filter selects it by its stable tag.
    #[test]
    fn test_query_events_reports_linked_checkout_write_override_record() {
        let (executor, expected) = seeded_override_executor();
        let events = executor
            .query_events(
                Some("linked_checkout_write_overridden".to_string()),
                None,
                10,
            )
            .expect("the event query succeeds");

        assert_eq!(events, vec![expected]);
        let Event::LinkedCheckoutWriteOverridden {
            declared_stance,
            invocation_override,
            ..
        } = &events[0]
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
        let (executor, expected) = seeded_override_executor();
        assert_eq!(
            executor
                .query_events(None, None, 10)
                .expect("the event query succeeds"),
            vec![expected],
            "the unfiltered query reports the seeded record",
        );

        assert!(
            executor
                .query_events(
                    None,
                    Some("00000000-0000-0000-0000-000000000001".to_string()),
                    10,
                )
                .expect("the event query succeeds")
                .is_empty(),
            "an issue-id filter reports no repository-scoped record",
        );
    }
}
