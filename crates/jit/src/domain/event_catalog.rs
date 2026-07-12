//! Catalog of the event-log tag vocabulary and each tag's association scope.
//!
//! [`Event`] defines the events appended to `.jit/events.jsonl`; its serde
//! declaration tags every record with a snake-case `type` field. This module
//! projects that vocabulary into a machine-consumable catalog ([`event_catalog`],
//! carried by `jit --schema` as its `events` array) and into the committed
//! markdown reference [`REFERENCE_PATH`] ([`render_event_reference`]).
//!
//! The catalog is derived from the type, never restated:
//!
//! - [`EventTag`] is the fieldless mirror of the `Event` variant set.
//!   [`Event::tag`] maps a record to its tag with a wildcard-free match, and
//!   [`Event::get_type`] returns [`EventTag::as_str`] — so a new `Event` variant
//!   fails to compile until it is given a tag here.
//! - [`EventTag::sample`] constructs one representative record per tag with a
//!   wildcard-free match, so a new tag fails to compile until it is sampled.
//! - Each row's `carries_issue_id` is read off the sample's serialized record —
//!   the same `serde_json` encoding `append_event` writes — rather than declared.
//! - A conformance test compares [`EventTag::ALL`] against the variant list
//!   schemars derives from the enum, so a tag missing from `ALL` fails the suite,
//!   and a golden test asserts the committed reference equals the projection
//!   (`@/inv/single-source-prose`).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{Assignee, Event, Priority, State};

/// Path of the committed markdown reference this module projects, relative to
/// the repository root.
pub const REFERENCE_PATH: &str = "docs/reference/events.md";

/// What an event is about: the state it records a change to.
///
/// The scope decides whether a record carries an `issue_id`: only
/// [`EventScope::Issue`] events name a single issue, so registry- and
/// repository-scoped records omit the field entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventScope {
    /// Records a change to one issue; the record carries that issue's `issue_id`.
    Issue,
    /// Records a change to a shared registry in `.jit/` (the gate registry).
    Registry,
    /// Records a change to the repository as a whole.
    Repository,
}

impl EventScope {
    /// The scope's snake-case name, as it appears in `jit --schema`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::EventScope;
    ///
    /// assert_eq!(EventScope::Issue.as_str(), "issue");
    /// assert_eq!(EventScope::Repository.as_str(), "repository");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            EventScope::Issue => "issue",
            EventScope::Registry => "registry",
            EventScope::Repository => "repository",
        }
    }
}

/// The event-log tag vocabulary: one variant per [`Event`] variant.
///
/// The tag is the value of a record's `type` field. [`Event::tag`] is the
/// wildcard-free mapping from a record to its tag, and [`Event::get_type`]
/// renders that tag as the string serde writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventTag {
    /// `issue_created`
    IssueCreated,
    /// `issue_claimed`
    IssueClaimed,
    /// `issue_state_changed`
    IssueStateChanged,
    /// `gate_passed`
    GatePassed,
    /// `gate_failed`
    GateFailed,
    /// `gate_added`
    GateAdded,
    /// `gate_removed`
    GateRemoved,
    /// `issue_completed`
    IssueCompleted,
    /// `issue_deleted`
    IssueDeleted,
    /// `issue_released`
    IssueReleased,
    /// `issue_updated`
    IssueUpdated,
    /// `document_archived`
    DocumentArchived,
    /// `dependency_reduced`
    DependencyReduced,
    /// `local_rule_bypassed`
    LocalRuleBypassed,
    /// `transition_blocked`
    TransitionBlocked,
    /// `graph_rule_bypassed`
    GraphRuleBypassed,
    /// `gate_definition_updated`
    GateDefinitionUpdated,
    /// `gate_definition_created`
    GateDefinitionCreated,
    /// `gate_definition_removed`
    GateDefinitionRemoved,
    /// `lifecycle_timestamps_backfilled`
    LifecycleTimestampsBackfilled,
}

impl EventTag {
    /// Every tag in the vocabulary, in [`Event`] declaration order.
    ///
    /// A conformance test compares this list against the variants schemars
    /// derives from the enum, so a tag left out of it fails the suite.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::EventTag;
    ///
    /// assert!(EventTag::ALL.contains(&EventTag::IssueCreated));
    /// ```
    pub const ALL: [EventTag; 20] = [
        EventTag::IssueCreated,
        EventTag::IssueClaimed,
        EventTag::IssueStateChanged,
        EventTag::GatePassed,
        EventTag::GateFailed,
        EventTag::GateAdded,
        EventTag::GateRemoved,
        EventTag::IssueCompleted,
        EventTag::IssueDeleted,
        EventTag::IssueReleased,
        EventTag::IssueUpdated,
        EventTag::DocumentArchived,
        EventTag::DependencyReduced,
        EventTag::LocalRuleBypassed,
        EventTag::TransitionBlocked,
        EventTag::GraphRuleBypassed,
        EventTag::GateDefinitionUpdated,
        EventTag::GateDefinitionCreated,
        EventTag::GateDefinitionRemoved,
        EventTag::LifecycleTimestampsBackfilled,
    ];

    /// The tag as serde writes it into a record's `type` field.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::EventTag;
    ///
    /// assert_eq!(EventTag::IssueStateChanged.as_str(), "issue_state_changed");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            EventTag::IssueCreated => "issue_created",
            EventTag::IssueClaimed => "issue_claimed",
            EventTag::IssueStateChanged => "issue_state_changed",
            EventTag::GatePassed => "gate_passed",
            EventTag::GateFailed => "gate_failed",
            EventTag::GateAdded => "gate_added",
            EventTag::GateRemoved => "gate_removed",
            EventTag::IssueCompleted => "issue_completed",
            EventTag::IssueDeleted => "issue_deleted",
            EventTag::IssueReleased => "issue_released",
            EventTag::IssueUpdated => "issue_updated",
            EventTag::DocumentArchived => "document_archived",
            EventTag::DependencyReduced => "dependency_reduced",
            EventTag::LocalRuleBypassed => "local_rule_bypassed",
            EventTag::TransitionBlocked => "transition_blocked",
            EventTag::GraphRuleBypassed => "graph_rule_bypassed",
            EventTag::GateDefinitionUpdated => "gate_definition_updated",
            EventTag::GateDefinitionCreated => "gate_definition_created",
            EventTag::GateDefinitionRemoved => "gate_definition_removed",
            EventTag::LifecycleTimestampsBackfilled => "lifecycle_timestamps_backfilled",
        }
    }

    /// The state this tag's records are about.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{EventScope, EventTag};
    ///
    /// assert_eq!(EventTag::GatePassed.scope(), EventScope::Issue);
    /// assert_eq!(EventTag::GateDefinitionCreated.scope(), EventScope::Registry);
    /// assert_eq!(EventTag::DocumentArchived.scope(), EventScope::Repository);
    /// ```
    pub fn scope(self) -> EventScope {
        match self {
            EventTag::IssueCreated
            | EventTag::IssueClaimed
            | EventTag::IssueStateChanged
            | EventTag::GatePassed
            | EventTag::GateFailed
            | EventTag::GateAdded
            | EventTag::GateRemoved
            | EventTag::IssueCompleted
            | EventTag::IssueDeleted
            | EventTag::IssueReleased
            | EventTag::IssueUpdated
            | EventTag::DependencyReduced
            | EventTag::LocalRuleBypassed
            | EventTag::TransitionBlocked
            | EventTag::GraphRuleBypassed => EventScope::Issue,
            EventTag::GateDefinitionUpdated
            | EventTag::GateDefinitionCreated
            | EventTag::GateDefinitionRemoved => EventScope::Registry,
            EventTag::DocumentArchived | EventTag::LifecycleTimestampsBackfilled => {
                EventScope::Repository
            }
        }
    }

    /// One-line description of what appends a record with this tag.
    fn description(self) -> &'static str {
        match self {
            EventTag::IssueCreated => "An issue was created.",
            EventTag::IssueClaimed => "An issue was claimed by an assignee.",
            EventTag::IssueStateChanged => {
                "An issue moved from one lifecycle state to another; the record carries \
                 both states."
            }
            EventTag::GatePassed => "A quality gate on the issue was recorded as passed.",
            EventTag::GateFailed => "A quality gate on the issue was recorded as failed.",
            EventTag::GateAdded => "A quality gate was added to the issue's required gates.",
            EventTag::GateRemoved => "A quality gate was removed from the issue's required gates.",
            EventTag::IssueCompleted => "An issue was completed.",
            EventTag::IssueDeleted => "An issue was permanently deleted.",
            EventTag::IssueReleased => {
                "An issue was released from its assignee; the record carries the former \
                 assignee and the reason."
            }
            EventTag::IssueUpdated => {
                "An issue's fields were updated; the record names the changed fields."
            }
            EventTag::DocumentArchived => {
                "`jit doc archive` moved a document into the archive; the record carries the \
                 source, the destination, the archive category, and the number of issues \
                 whose references it re-pointed."
            }
            EventTag::DependencyReduced => {
                "`jit validate --fix` removed the issue's redundant (transitively implied) \
                 dependency edges."
            }
            EventTag::LocalRuleBypassed => {
                "`--force` overrode an enforcing validation rule's error finding on an issue \
                 write; one record per bypassed rule."
            }
            EventTag::TransitionBlocked => {
                "An enforcing graph rule refused a state transition; the record carries the \
                 target state and the rule."
            }
            EventTag::GraphRuleBypassed => {
                "`--force` overrode an enforcing graph rule's error finding at a state \
                 transition; one record per bypassed rule."
            }
            EventTag::GateDefinitionUpdated => "`jit gate update` edited a registered gate.",
            EventTag::GateDefinitionCreated => "`jit gate define` registered a gate.",
            EventTag::GateDefinitionRemoved => "`jit gate remove` unregistered a gate.",
            EventTag::LifecycleTimestampsBackfilled => {
                "The one-time `jit migrate lifecycle-timestamps` backfill wrote derived \
                 lifecycle timestamps; the record carries the number of issues it updated."
            }
        }
    }

    /// A representative record with this tag.
    ///
    /// The catalog reads each row's record shape off this sample, so the shape it
    /// reports is the shape `serde_json` actually writes. The match is
    /// wildcard-free: a new tag fails to compile until it is sampled here.
    fn sample(self) -> Event {
        let id = "00000000-0000-0000-0000-000000000000".to_string();
        let issue_id = "00000000-0000-0000-0000-000000000001".to_string();
        let gate_key = "tests".to_string();
        let timestamp: DateTime<Utc> =
            DateTime::from_timestamp(0, 0).expect("the Unix epoch is a valid timestamp");
        let assignee: Assignee = "agent:worker-1"
            .parse()
            .expect("`agent:worker-1` is a well-formed assignee");

        match self {
            EventTag::IssueCreated => Event::IssueCreated {
                id,
                issue_id,
                timestamp,
                title: "Sample issue".to_string(),
                priority: Priority::Normal,
            },
            EventTag::IssueClaimed => Event::IssueClaimed {
                id,
                issue_id,
                timestamp,
                assignee,
            },
            EventTag::IssueStateChanged => Event::IssueStateChanged {
                id,
                issue_id,
                timestamp,
                from: State::Ready,
                to: State::InProgress,
            },
            EventTag::GatePassed => Event::GatePassed {
                id,
                issue_id,
                timestamp,
                gate_key,
                updated_by: Some(assignee),
            },
            EventTag::GateFailed => Event::GateFailed {
                id,
                issue_id,
                timestamp,
                gate_key,
                updated_by: Some(assignee),
            },
            EventTag::GateAdded => Event::GateAdded {
                id,
                issue_id,
                timestamp,
                gate_key,
            },
            EventTag::GateRemoved => Event::GateRemoved {
                id,
                issue_id,
                timestamp,
                gate_key,
            },
            EventTag::IssueCompleted => Event::IssueCompleted {
                id,
                issue_id,
                timestamp,
            },
            EventTag::IssueDeleted => Event::IssueDeleted {
                id,
                issue_id,
                timestamp,
            },
            EventTag::IssueReleased => Event::IssueReleased {
                id,
                issue_id,
                timestamp,
                assignee,
                reason: "stale lease".to_string(),
            },
            EventTag::IssueUpdated => Event::IssueUpdated {
                id,
                issue_id,
                timestamp,
                updated_by: "agent:worker-1".to_string(),
                fields: vec!["priority".to_string()],
            },
            EventTag::DocumentArchived => Event::DocumentArchived {
                id,
                timestamp,
                source: "dev/plan.md".to_string(),
                destination: "dev/archive/plan.md".to_string(),
                category: "plan".to_string(),
                issues_updated: 1,
            },
            EventTag::DependencyReduced => Event::DependencyReduced {
                id,
                issue_id,
                timestamp,
                old_count: 2,
                new_count: 1,
                removed_deps: vec!["00000000-0000-0000-0000-000000000002".to_string()],
            },
            EventTag::LocalRuleBypassed => Event::LocalRuleBypassed {
                id,
                issue_id,
                timestamp,
                rule: "type-required".to_string(),
            },
            EventTag::TransitionBlocked => Event::TransitionBlocked {
                id,
                issue_id,
                timestamp,
                target: State::Done,
                rule: "children-complete".to_string(),
            },
            EventTag::GraphRuleBypassed => Event::GraphRuleBypassed {
                id,
                issue_id,
                timestamp,
                target: State::Done,
                rule: "children-complete".to_string(),
            },
            EventTag::GateDefinitionUpdated => Event::GateDefinitionUpdated {
                id,
                timestamp,
                gate_key,
            },
            EventTag::GateDefinitionCreated => Event::GateDefinitionCreated {
                id,
                timestamp,
                gate_key,
            },
            EventTag::GateDefinitionRemoved => Event::GateDefinitionRemoved {
                id,
                timestamp,
                gate_key,
            },
            EventTag::LifecycleTimestampsBackfilled => Event::LifecycleTimestampsBackfilled {
                id,
                timestamp,
                issues_updated: 3,
            },
        }
    }
}

impl Event {
    /// The tag under which this record is stored.
    ///
    /// The wildcard-free match is what binds the catalog to the `Event` type: a
    /// new variant fails to compile until it is given a tag, and the tag then
    /// carries a scope, a sample, and a description with it.
    /// [`Event::get_type`] renders the returned tag as the string serde writes.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::{Event, EventTag};
    ///
    /// let event = Event::new_gate_definition_created("tests".to_string());
    /// assert_eq!(event.tag(), EventTag::GateDefinitionCreated);
    /// assert_eq!(event.get_type(), EventTag::GateDefinitionCreated.as_str());
    /// ```
    pub fn tag(&self) -> EventTag {
        match self {
            Event::IssueCreated { .. } => EventTag::IssueCreated,
            Event::IssueClaimed { .. } => EventTag::IssueClaimed,
            Event::IssueStateChanged { .. } => EventTag::IssueStateChanged,
            Event::GatePassed { .. } => EventTag::GatePassed,
            Event::GateFailed { .. } => EventTag::GateFailed,
            Event::GateAdded { .. } => EventTag::GateAdded,
            Event::GateRemoved { .. } => EventTag::GateRemoved,
            Event::IssueCompleted { .. } => EventTag::IssueCompleted,
            Event::IssueDeleted { .. } => EventTag::IssueDeleted,
            Event::IssueReleased { .. } => EventTag::IssueReleased,
            Event::IssueUpdated { .. } => EventTag::IssueUpdated,
            Event::DocumentArchived { .. } => EventTag::DocumentArchived,
            Event::DependencyReduced { .. } => EventTag::DependencyReduced,
            Event::LocalRuleBypassed { .. } => EventTag::LocalRuleBypassed,
            Event::TransitionBlocked { .. } => EventTag::TransitionBlocked,
            Event::GraphRuleBypassed { .. } => EventTag::GraphRuleBypassed,
            Event::GateDefinitionUpdated { .. } => EventTag::GateDefinitionUpdated,
            Event::GateDefinitionCreated { .. } => EventTag::GateDefinitionCreated,
            Event::GateDefinitionRemoved { .. } => EventTag::GateDefinitionRemoved,
            Event::LifecycleTimestampsBackfilled { .. } => EventTag::LifecycleTimestampsBackfilled,
        }
    }
}

/// One catalog row: an event tag, what it is about, and whether its records
/// carry an `issue_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventTagDoc {
    /// The record's `type` tag.
    pub tag: EventTag,
    /// The state the record is about.
    pub scope: EventScope,
    /// Whether records with this tag carry an `issue_id` field.
    pub carries_issue_id: bool,
    /// What appends a record with this tag.
    pub description: String,
}

/// Project the event-tag catalog: one row per [`Event`] variant.
///
/// Carried by `jit --schema` as its `events` array and rendered into
/// [`REFERENCE_PATH`] by [`render_event_reference`]. Every row is derived:
/// [`EventTag::as_str`] supplies the tag serde writes, [`EventTag::scope`] the
/// association scope, and the `issue_id` presence is read off the sample record's
/// serialized JSON object — the encoding the event log stores.
///
/// # Examples
///
/// ```
/// use jit::domain::{event_catalog, EventScope, EventTag};
///
/// let catalog = event_catalog();
/// let row = |tag| catalog.iter().find(|r| r.tag == tag).expect("tag is cataloged");
///
/// // Issue-scoped records name the issue they concern.
/// assert!(row(EventTag::IssueStateChanged).carries_issue_id);
/// // Repository- and registry-scoped records do not.
/// assert!(!row(EventTag::DocumentArchived).carries_issue_id);
/// assert_eq!(row(EventTag::GateDefinitionCreated).scope, EventScope::Registry);
/// ```
pub fn event_catalog() -> Vec<EventTagDoc> {
    EventTag::ALL
        .iter()
        .map(|&tag| EventTagDoc {
            tag,
            scope: tag.scope(),
            carries_issue_id: record_carries_issue_id(&tag.sample()),
            description: tag.description().to_string(),
        })
        .collect()
}

/// Whether a record's serialized JSON object has an `issue_id` field.
///
/// Serializes with `serde_json`, the encoding
/// [`append_event`](crate::storage::IssueStore::append_event) writes to
/// `events.jsonl`, so the answer is the on-disk record shape rather than a
/// declaration about it.
fn record_carries_issue_id(event: &Event) -> bool {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| value.as_object().map(|obj| obj.contains_key("issue_id")))
        .unwrap_or(false)
}

/// Render the event-log reference page ([`REFERENCE_PATH`]).
///
/// The page projects [`event_catalog`] into markdown, so the committed doc is
/// generated rather than hand-copied. A conformance test asserts the committed
/// file equals this output (`@/inv/single-source-prose`).
///
/// # Examples
///
/// ```
/// let page = jit::domain::render_event_reference();
///
/// assert!(page.contains("# Event Log Tags"));
/// assert!(page.contains("| `issue_state_changed` | issue | yes |"));
/// assert!(page.contains("| `document_archived` | repository | no |"));
/// ```
pub fn render_event_reference() -> String {
    let catalog = event_catalog();

    let no_issue = catalog
        .iter()
        .filter(|row| !row.carries_issue_id)
        .map(|row| format!("- `{}` ({})\n", row.tag.as_str(), row.scope.as_str()))
        .collect::<String>();

    let table = catalog
        .iter()
        .map(|row| {
            format!(
                "| `{}` | {} | {} | {} |",
                row.tag.as_str(),
                row.scope.as_str(),
                if row.carries_issue_id { "yes" } else { "no" },
                row.description,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "<!-- Generated from `jit::domain::event_catalog` — do not edit by hand. -->\n\
         \n\
         # Event Log Tags\n\
         \n\
         `.jit/events.jsonl` stores one JSON object per line. Every record is tagged by a\n\
         snake-case `type` field and carries its own `id` and `timestamp`; the remaining\n\
         fields are flat on the object and vary by tag. This reference is generated from\n\
         the `Event` type in `crates/jit/src/domain/types.rs` and is also served, as the\n\
         `events` array, by `jit --schema`.\n\
         \n\
         The **scope** is the state a record is about, and it decides the `issue_id` field:\n\
         an `issue`-scoped record names the one issue it concerns, while `registry`- and\n\
         `repository`-scoped records omit `issue_id` entirely, because they record a change\n\
         to shared state. The tags whose records carry no `issue_id` are exactly:\n\
         \n\
         {no_issue}\n\
         A `jit events query --issue-id <ID>` filter therefore never returns them.\n\
         \n\
         | Tag | Scope | `issue_id` | Emitted when |\n\
         | --- | --- | --- | --- |\n\
         {table}\n\
         \n\
         For the commands that read the log, see\n\
         [Event Log Commands](cli-commands.md#event-log-commands); for the file's place in\n\
         the `.jit/` layout, see [Storage Format](storage-format.md#event-log-format).\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REFERENCE_PATH)
    }

    /// Every string the derived schema of a fieldless enum admits: schemars
    /// renders each documented variant as its own `enum` subschema of a `oneOf`,
    /// so the accepted values are gathered from every `enum` array in the tree.
    fn schema_accepted_strings(schema: &serde_json::Value) -> BTreeSet<String> {
        match schema {
            serde_json::Value::Object(map) => map
                .iter()
                .flat_map(|(key, value)| match (key.as_str(), value) {
                    ("enum", serde_json::Value::Array(values)) => values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    _ => schema_accepted_strings(value),
                })
                .collect(),
            serde_json::Value::Array(values) => {
                values.iter().flat_map(schema_accepted_strings).collect()
            }
            _ => BTreeSet::new(),
        }
    }

    /// REQ-03 (enumeration): `EventTag::ALL` must list every tag. The expected
    /// set is the variant list schemars derives from the enum itself, so a tag
    /// added to `EventTag` — which a new `Event` variant forces, `Event::tag`
    /// being a wildcard-free match — but left out of `ALL` fails here.
    #[test]
    fn test_all_lists_every_tag_variant() {
        let schema = serde_json::to_value(schema_for!(EventTag)).expect("EventTag schema is JSON");
        let derived = schema_accepted_strings(&schema);
        assert!(
            !derived.is_empty(),
            "the derived EventTag schema must enumerate its variants"
        );

        let listed: BTreeSet<String> = EventTag::ALL
            .iter()
            .map(|tag| tag.as_str().to_string())
            .collect();

        assert_eq!(
            listed, derived,
            "EventTag::ALL must list every EventTag variant"
        );
    }

    /// REQ-03 (tag strings): `Event::get_type` — the accessor the event-type
    /// filter and the stored `type` field agree on — must return the tag's
    /// string for every tag's record.
    #[test]
    fn test_get_type_matches_tag_string_for_every_tag() {
        for tag in EventTag::ALL {
            let event = tag.sample();
            assert_eq!(event.get_type(), tag.as_str());
            assert_eq!(event.tag(), tag);
        }
    }

    /// The serialized `type` field of every sample is its tag: the catalog's tag
    /// column is the value `events.jsonl` actually stores.
    #[test]
    fn test_serialized_type_field_matches_tag_for_every_tag() {
        for tag in EventTag::ALL {
            let value = serde_json::to_value(tag.sample()).expect("sample event serializes");
            assert_eq!(
                value.get("type").and_then(|t| t.as_str()),
                Some(tag.as_str()),
            );
        }
    }

    /// REQ-01: every tag has a catalog row carrying its scope and `issue_id`
    /// presence, and the two agree — a record carries an `issue_id` exactly when
    /// it is issue-scoped.
    #[test]
    fn test_catalog_covers_every_tag_with_consistent_scope() {
        let catalog = event_catalog();
        assert_eq!(catalog.len(), EventTag::ALL.len());

        for (row, tag) in catalog.iter().zip(EventTag::ALL) {
            assert_eq!(row.tag, tag);
            assert_eq!(row.scope, tag.scope());
            assert!(
                !row.description.is_empty(),
                "{} has no description",
                row.tag.as_str()
            );
            assert_eq!(
                row.carries_issue_id,
                row.scope == EventScope::Issue,
                "`{}` must carry an issue_id exactly when it is issue-scoped",
                row.tag.as_str(),
            );
        }
    }

    /// REQ-01/REQ-03: the `issue_id` column agrees with `Event::get_issue_id`,
    /// the production accessor every consumer (the `--issue-id` filter, the
    /// lifecycle-timestamp derivation) reads an event's issue through: it returns
    /// an empty id exactly for the records that carry no `issue_id` field.
    #[test]
    fn test_carries_issue_id_matches_get_issue_id_accessor() {
        for row in event_catalog() {
            let event = row.tag.sample();
            assert_eq!(
                row.carries_issue_id,
                !event.get_issue_id().is_empty(),
                "`{}`: catalog and Event::get_issue_id disagree",
                row.tag.as_str(),
            );
        }
    }

    /// REQ-02: the records without an `issue_id` are exactly `document_archived`,
    /// the three gate-definition registry edits, and
    /// `lifecycle_timestamps_backfilled`. Asserted as a set equality, so a tag
    /// that joins or leaves the no-issue set fails here.
    #[test]
    fn test_no_issue_set_is_exactly_the_shared_state_events() {
        let no_issue: BTreeSet<&str> = event_catalog()
            .iter()
            .filter(|row| !row.carries_issue_id)
            .map(|row| row.tag.as_str())
            .collect();

        assert_eq!(
            no_issue,
            BTreeSet::from([
                "document_archived",
                "gate_definition_created",
                "gate_definition_removed",
                "gate_definition_updated",
                "lifecycle_timestamps_backfilled",
            ]),
        );
    }

    /// REQ-03 (projection freshness): the committed reference must equal the
    /// rendered catalog. Adding, removing, or re-scoping an event without
    /// refreshing the reference fails here.
    #[test]
    fn test_committed_reference_matches_projection() {
        let committed = std::fs::read_to_string(reference_path())
            .expect("committed events reference should exist");
        assert_eq!(
            committed,
            render_event_reference(),
            "{REFERENCE_PATH} is stale — regenerate it from `jit::domain::event_catalog` \
             (run: cargo test -p jit event_catalog -- --ignored regenerate)"
        );
    }

    /// Regenerate the committed reference from the catalog. Ignored by default;
    /// run explicitly after changing the `Event` type:
    ///   cargo test -p jit event_catalog -- --ignored regenerate
    ///
    /// Writes via the temp-file + atomic-rename pattern (`@/inv/atomic-writes`).
    #[test]
    #[ignore = "writes the committed reference; run explicitly to regenerate"]
    fn test_regenerate_reference_writes_committed_doc() {
        let path = reference_path();
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, render_event_reference()).expect("should write the events temp file");
        std::fs::rename(&tmp, &path).expect("should atomically replace the events reference");
    }

    /// The rendered page names every tag and its `issue_id` column.
    #[test]
    fn test_render_lists_every_tag() {
        let page = render_event_reference();
        for row in event_catalog() {
            assert!(
                page.contains(&format!(
                    "| `{}` | {} | {} |",
                    row.tag.as_str(),
                    row.scope.as_str(),
                    if row.carries_issue_id { "yes" } else { "no" },
                )),
                "rendered page is missing the row for `{}`",
                row.tag.as_str(),
            );
        }
    }
}
