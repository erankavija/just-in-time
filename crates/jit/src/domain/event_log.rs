//! Pure parsing for the append-only event log.

use super::event_catalog::EventTag;
use super::Event;

/// Structural failure while reading a known event-log record.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    /// A newline-terminated line is not JSON.
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
    /// A record's tag is outside the current vocabulary.
    #[error("unknown event tag '{tag}' on line {line}")]
    UnknownTag {
        /// One-based physical line number.
        line: usize,
        /// The tag the record carries.
        tag: String,
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

/// Parse the event log, skipping blank lines.
///
/// A final line without a terminating newline is an uncommitted append, which
/// the next append overwrites, so it is ignored whatever it holds. Every other
/// line must be a well-formed record of the current vocabulary.
///
/// # Errors
///
/// Names the first line that is not such a record.
pub fn parse_known_events(contents: &[u8]) -> Result<Vec<Event>, EventLogError> {
    contents
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
        .filter(|(_, line)| line.ends_with(b"\n") && !line.trim_ascii().is_empty())
        .map(|(offset, record)| parse_record(offset + 1, record))
        .collect()
}

fn parse_record(line: usize, record: &[u8]) -> Result<Event, EventLogError> {
    let value: serde_json::Value = serde_json::from_slice(record)
        .map_err(|source| EventLogError::InvalidJson { line, source })?;
    let tag = value
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(serde_json::Value::as_str)
        .ok_or(EventLogError::MissingType { line })?;
    if !EventTag::ALL.iter().any(|known| known.as_str() == tag) {
        return Err(EventLogError::UnknownTag {
            line,
            tag: tag.to_owned(),
        });
    }
    serde_json::from_value(value)
        .map_err(|source| EventLogError::InvalidKnownEvent { line, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(event: &Event) -> String {
        serde_json::to_string(event).expect("a sample serializes")
    }

    #[test]
    fn test_parse_known_events_ignores_an_unterminated_final_line_and_rejects_a_malformed_middle_line(
    ) {
        let committed = EventTag::ALL[0].sample();
        let prefix = format!("{}\n", line(&committed));
        let complete_record = line(&EventTag::ALL[1].sample());
        let unterminated_final_lines = [
            b"{\"torn\":".as_slice(),
            complete_record.as_bytes(),
            b"{\"type\":\"retired_event\"}",
            // A write torn inside a multi-byte character.
            b"{\"title\":\"\xC3",
        ];
        for fragment in unterminated_final_lines {
            let log = [prefix.as_bytes(), fragment].concat();
            assert_eq!(
                parse_known_events(&log).unwrap(),
                std::slice::from_ref(&committed),
                "fragment {:?}",
                String::from_utf8_lossy(fragment)
            );
        }

        for fragment in [b"{\"torn\":".as_slice(), b"{\"title\":\"\xC3"] {
            let log = [fragment, b"\n", prefix.as_bytes()].concat();
            assert!(
                matches!(
                    parse_known_events(&log),
                    Err(EventLogError::InvalidJson { line: 1, .. })
                ),
                "fragment {:?}",
                String::from_utf8_lossy(fragment)
            );
        }
    }

    #[test]
    fn test_parse_known_events_skips_blank_lines_and_names_the_line_of_a_malformed_record() {
        let event = line(&EventTag::ALL[0].sample());
        let log = |bad: &str| format!("\n{event}\n  \n{bad}\n{event}\n").into_bytes();
        let line_of = |error: EventLogError| match error {
            EventLogError::InvalidJson { line, .. }
            | EventLogError::MissingType { line }
            | EventLogError::UnknownTag { line, .. }
            | EventLogError::InvalidKnownEvent { line, .. } => line,
        };

        assert_eq!(parse_known_events(log("").as_slice()).unwrap().len(), 2);
        for bad in [
            "{not-json}",
            "{}",
            "{\"type\":\"retired_event\"}",
            "{\"type\":\"issue_created\"}",
        ] {
            assert_eq!(
                parse_known_events(&log(bad)).map_err(line_of).unwrap_err(),
                4,
                "record {bad}"
            );
        }
    }

    /// REQ-03: the log reader retains every record of the current vocabulary,
    /// so a newly cataloged tag is read back rather than silently dropped.
    #[test]
    fn test_parse_known_events_retains_every_cataloged_tag() {
        let samples: Vec<Event> = EventTag::ALL.iter().map(|tag| tag.sample()).collect();
        let log = samples
            .iter()
            .map(|event| format!("{}\n", line(event)))
            .collect::<String>();

        assert_eq!(parse_known_events(log.as_bytes()).unwrap(), samples);
    }
}
