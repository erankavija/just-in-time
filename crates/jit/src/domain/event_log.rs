//! Pure parsing for the append-only event log.

use super::{Event, EventTag};

/// Structural failure while reading a known event-log record.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    /// A line is not JSON and lacks the immediately following profile marker
    /// that certifies it as an isolated torn tail.
    #[error("invalid event JSON on line {line}: {source}")]
    InvalidJson {
        /// One-based physical line number.
        line: usize,
        /// JSON parser failure.
        #[source]
        source: serde_json::Error,
    },
    /// A syntactically valid record lacks its tag.
    #[error("event line {line} is missing a string type")]
    MissingType {
        /// One-based physical line number.
        line: usize,
    },
    /// A current-vocabulary record does not match its typed event shape.
    #[error("invalid known event on line {line}: {source}")]
    InvalidKnownEvent {
        /// One-based physical line number.
        line: usize,
        /// Typed deserialization failure.
        #[source]
        source: serde_json::Error,
    },
}

/// Parse current-vocabulary events while retaining retired/unknown records.
///
/// Invalid JSON is rejected except for exactly one physical line immediately
/// followed by a valid [`Event::ProfileLifecycle`] whose
/// `isolated_torn_tail` flag is true. That marker is written in the same
/// transaction as the preserved prefix, making the exception explicit and
/// auditable rather than broadly accepting malformed history.
pub fn parse_known_events(contents: &str) -> Result<Vec<Event>, EventLogError> {
    let lines = contents.lines().collect::<Vec<_>>();
    let mut events = Vec::new();
    for (offset, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) if certifies_preceding_torn_tail(lines.get(offset + 1)) => continue,
            Err(source) => {
                return Err(EventLogError::InvalidJson {
                    line: offset + 1,
                    source,
                })
            }
        };
        let Some(tag) = value
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(serde_json::Value::as_str)
        else {
            return Err(EventLogError::MissingType { line: offset + 1 });
        };
        if EventTag::ALL.iter().any(|known| known.as_str() == tag) {
            events.push(serde_json::from_value(value).map_err(|source| {
                EventLogError::InvalidKnownEvent {
                    line: offset + 1,
                    source,
                }
            })?);
        }
    }
    Ok(events)
}

fn certifies_preceding_torn_tail(next_line: Option<&&str>) -> bool {
    next_line
        .and_then(|line| serde_json::from_str::<Event>(line).ok())
        .is_some_and(|event| {
            matches!(
                event,
                Event::ProfileLifecycle {
                    isolated_torn_tail: true,
                    ..
                }
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(isolated_torn_tail: bool) -> Event {
        Event::ProfileLifecycle {
            id: String::new(),
            timestamp: chrono::DateTime::UNIX_EPOCH,
            operation: crate::domain::ProfileLifecycleOperation::Apply,
            profiles: vec![crate::domain::ProfileLifecycleProfile {
                id: "example"
                    .try_into()
                    .expect("the profile marker has a canonical identifier"),
                status: crate::domain::ProfileLifecycleStatus::Installed,
                variables: Vec::new(),
            }],
            isolated_torn_tail,
        }
    }

    #[test]
    fn test_parser_accepts_only_torn_line_certified_by_immediate_profile_marker() {
        let certified_marker = marker(true);
        let certified = format!(
            "{{\"torn\":\n{}\n",
            serde_json::to_string(&certified_marker).unwrap()
        );
        assert_eq!(
            parse_known_events(&certified).unwrap(),
            vec![certified_marker]
        );

        let unmarked_marker = marker(false);
        let unmarked = format!(
            "{{\"torn\":\n{}\n",
            serde_json::to_string(&unmarked_marker).unwrap()
        );
        assert!(matches!(
            parse_known_events(&unmarked),
            Err(EventLogError::InvalidJson { line: 1, .. })
        ));
        let separated_marker = marker(true);
        let separated = format!(
            "{{\"torn\":\n\n{}\n",
            serde_json::to_string(&separated_marker).unwrap()
        );
        assert!(matches!(
            parse_known_events(&separated),
            Err(EventLogError::InvalidJson { line: 1, .. })
        ));
    }
}
