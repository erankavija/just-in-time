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
