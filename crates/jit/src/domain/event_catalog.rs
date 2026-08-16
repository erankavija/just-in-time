//! Catalog of the event-log tag vocabulary and each tag's association scope.
//!
//! [`Event`] defines the events appended to `.jit/events.jsonl`; its serde
//! declaration tags every record with a snake-case `type` field. This module
//! projects that vocabulary into a machine-consumable catalog ([`event_catalog`],
//! carried by `jit --schema` as its `events` array) and into the committed
//! markdown reference rendered by [`render_event_reference`].
//!
//! The catalog is derived from the type, never restated:
//!
//! - [`EventTag`] is the fieldless mirror of the `Event` variant set.
//!   [`Event::tag`] maps a record to its tag with a wildcard-free match, and
//!   [`Event::get_type`] returns [`EventTag::as_str`] — so a new `Event` variant
//!   fails to compile until it is given a tag here.
//! - [`EventTag::sample`] constructs one representative record per tag with a
//!   wildcard-free match, so a new tag fails to compile until it is sampled.
//! - A row's `carries_issue_id` follows from its [`EventScope`]: only
//!   issue-scoped records name an issue. Tests hold that against reality from
//!   both sides — against the serialized sample (the same `serde_json` encoding
//!   the repository-state finalizer publishes to `events.jsonl`) and against [`Event::get_issue_id`],
//!   the accessor every consumer reads an event's issue through.
//! - Conformance tests compare the catalog against the variant lists schemars
//!   derives from the enums: every serde `type` tag of [`Event`] must be
//!   cataloged, and every [`EventTag`] must appear in [`EventTag::ALL`]. A golden
//!   test asserts the committed reference equals the projection
//!   (`@/inv/single-source-prose`).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{Assignee, Event, LinkedCheckoutWriteStance, Priority, State};

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    /// Path of the committed markdown reference this module projects, relative to
    /// the repository root.
    pub const REFERENCE_PATH: &str = "docs/reference/events.md";

    /// The command that renders [`REFERENCE_PATH`] from this catalog, named in the
    /// conformance test's message so a stale reference carries its own repair.
    pub const REFERENCE_GENERATOR: &str = "./scripts/generate-events-reference.sh";
}

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
    /// `artifact_archive_executed`
    ArtifactArchiveExecuted,
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
    /// `profile_lifecycle`
    ProfileLifecycle,
    /// `linked_checkout_write_overridden`
    LinkedCheckoutWriteOverridden,
}

impl EventTag {
    /// Every tag in the vocabulary, in [`Event`] declaration order.
    ///
    /// A conformance test compares this list against the variants schemars
    /// derives from the enum, so a tag left out of it fails the suite.
    pub const ALL: [EventTag; 22] = [
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
        EventTag::ArtifactArchiveExecuted,
        EventTag::DependencyReduced,
        EventTag::LocalRuleBypassed,
        EventTag::TransitionBlocked,
        EventTag::GraphRuleBypassed,
        EventTag::GateDefinitionUpdated,
        EventTag::GateDefinitionCreated,
        EventTag::GateDefinitionRemoved,
        EventTag::LifecycleTimestampsBackfilled,
        EventTag::ProfileLifecycle,
        EventTag::LinkedCheckoutWriteOverridden,
    ];

    /// The tag as serde writes it into a record's `type` field.
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
            EventTag::ArtifactArchiveExecuted => "artifact_archive_executed",
            EventTag::DependencyReduced => "dependency_reduced",
            EventTag::LocalRuleBypassed => "local_rule_bypassed",
            EventTag::TransitionBlocked => "transition_blocked",
            EventTag::GraphRuleBypassed => "graph_rule_bypassed",
            EventTag::GateDefinitionUpdated => "gate_definition_updated",
            EventTag::GateDefinitionCreated => "gate_definition_created",
            EventTag::GateDefinitionRemoved => "gate_definition_removed",
            EventTag::LifecycleTimestampsBackfilled => "lifecycle_timestamps_backfilled",
            EventTag::ProfileLifecycle => "profile_lifecycle",
            EventTag::LinkedCheckoutWriteOverridden => "linked_checkout_write_overridden",
        }
    }

    /// The state this tag's records are about.
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
            EventTag::ArtifactArchiveExecuted
            | EventTag::LifecycleTimestampsBackfilled
            | EventTag::ProfileLifecycle
            | EventTag::LinkedCheckoutWriteOverridden => EventScope::Repository,
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
            EventTag::ArtifactArchiveExecuted => {
                "`jit archive ... --execute` durably recorded publications, exact reference \
                 changes, and identity-guarded planned deletions before deletion attempts."
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
            EventTag::ProfileLifecycle => {
                "A profile lifecycle operation reached one durable transaction commit point; \
                 the record summarizes per-profile actions and variable source kinds without \
                 resolved values or rendered content."
            }
            EventTag::LinkedCheckoutWriteOverridden => {
                "An explicit per-invocation override permitted a state-mutating command inside a \
                 linked checkout that the declared write stance refuses; the record names the \
                 checkout, the refusing stance, and the permitting override."
            }
        }
    }

    /// A representative record with this tag.
    ///
    /// The record shape here is the shape `serde_json` actually writes, so the
    /// catalog's claims are tested against these samples rather than against a
    /// restatement of them. The match is wildcard-free: a new tag fails to compile
    /// until it is sampled here.
    pub fn sample(self) -> Event {
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
            EventTag::ArtifactArchiveExecuted => Event::ArtifactArchiveExecuted {
                id,
                timestamp,
                target: crate::domain::artifact_plan::PlanTarget::Document {
                    path: "dev/plan.md".to_string(),
                },
                destination_root: "dev/archive".to_string(),
                publications: Vec::new(),
                reference_changes: Vec::new(),
                planned_deletions: Vec::new(),
                reconciling: false,
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
            EventTag::ProfileLifecycle => Event::ProfileLifecycle {
                id,
                timestamp,
                operation: crate::domain::ProfileLifecycleOperation::Apply,
                profiles: vec![crate::domain::ProfileLifecycleProfile {
                    id: "example"
                        .try_into()
                        .expect("sample profile id is canonical"),
                    status: crate::domain::ProfileLifecycleStatus::Installed,
                    variables: vec![crate::domain::ProfileLifecycleVariable {
                        name: "NAME"
                            .try_into()
                            .expect("sample variable name is canonical"),
                        source: crate::profile::VariableSource::Set,
                    }],
                }],
                isolated_torn_tail: false,
            },
            EventTag::LinkedCheckoutWriteOverridden => Event::LinkedCheckoutWriteOverridden {
                id,
                timestamp,
                checkout: std::path::PathBuf::from("/repo/.worktrees/feature"),
                declared_stance: LinkedCheckoutWriteStance::Refuse,
                invocation_override: LinkedCheckoutWriteStance::Allow,
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
            Event::ArtifactArchiveExecuted { .. } => EventTag::ArtifactArchiveExecuted,
            Event::DependencyReduced { .. } => EventTag::DependencyReduced,
            Event::LocalRuleBypassed { .. } => EventTag::LocalRuleBypassed,
            Event::TransitionBlocked { .. } => EventTag::TransitionBlocked,
            Event::GraphRuleBypassed { .. } => EventTag::GraphRuleBypassed,
            Event::GateDefinitionUpdated { .. } => EventTag::GateDefinitionUpdated,
            Event::GateDefinitionCreated { .. } => EventTag::GateDefinitionCreated,
            Event::GateDefinitionRemoved { .. } => EventTag::GateDefinitionRemoved,
            Event::LifecycleTimestampsBackfilled { .. } => EventTag::LifecycleTimestampsBackfilled,
            Event::ProfileLifecycle { .. } => EventTag::ProfileLifecycle,
            Event::LinkedCheckoutWriteOverridden { .. } => EventTag::LinkedCheckoutWriteOverridden,
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
    /// Whether records with this tag carry an `issue_id` field. True exactly for
    /// [`EventScope::Issue`] tags; tests hold this against the serialized record
    /// and against [`Event::get_issue_id`].
    pub carries_issue_id: bool,
    /// What appends a record with this tag.
    pub description: String,
}

/// Project the event-tag catalog: one row per [`Event`] variant.
///
/// Carried by `jit --schema` as its `events` array and rendered into the
/// committed reference by [`render_event_reference`]. Every row is derived:
/// [`EventTag::as_str`] supplies the tag serde writes, [`EventTag::scope`] the
/// association scope, and the `issue_id` presence is read off the sample record's
/// serialized JSON object — the encoding the event log stores.
pub fn event_catalog() -> Vec<EventTagDoc> {
    EventTag::ALL
        .iter()
        .map(|&tag| EventTagDoc {
            tag,
            scope: tag.scope(),
            carries_issue_id: tag.scope() == EventScope::Issue,
            description: tag.description().to_string(),
        })
        .collect()
}

/// Render the event-log reference page.
///
/// The page projects [`event_catalog`] into markdown, so the committed doc is
/// generated rather than hand-copied. A conformance test asserts the committed
/// file equals this output (`@/inv/single-source-prose`).
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
            .join(test_support::REFERENCE_PATH)
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

    /// REQ-03: every serde `type` tag the `Event` enum admits is cataloged.
    ///
    /// `Event::tag()` being wildcard-free forces a new variant to be *given* a
    /// tag, but nothing stops it being given an existing one — serde would then
    /// write a `type` string absent from the catalog while the code still
    /// compiles. This reads the tags off the schema schemars derives from `Event`
    /// itself (each variant contributes its `type` const), so the catalog is bound
    /// to the serde encoding rather than to `EventTag`'s own variant list.
    #[test]
    fn test_event_variant_tags_are_all_cataloged() {
        let schema = serde_json::to_value(schema_for!(Event)).expect("Event schema is JSON");
        let variants = schema
            .get("oneOf")
            .and_then(|one_of| one_of.as_array())
            .expect("an internally-tagged enum derives a oneOf of its variants");

        let serde_tags: BTreeSet<String> = variants
            .iter()
            .map(|variant| {
                let accepted = schema_accepted_strings(&variant["properties"]["type"]);
                assert_eq!(
                    accepted.len(),
                    1,
                    "each variant pins exactly one `type` string, got {accepted:?}"
                );
                accepted.into_iter().next().expect("checked above")
            })
            .collect();

        assert_eq!(
            serde_tags.len(),
            variants.len(),
            "two Event variants serialize to the same `type` tag"
        );

        let cataloged: BTreeSet<String> = event_catalog()
            .iter()
            .map(|row| row.tag.as_str().to_string())
            .collect();

        assert_eq!(
            serde_tags, cataloged,
            "every `type` tag Event serializes must be cataloged; \
             left = serde, right = catalog"
        );
    }

    /// REQ-01/REQ-03: the `issue_id` column agrees with the on-disk record — the
    /// same `serde_json` encoding the repository-state finalizer publishes to
    /// `events.jsonl`. The
    /// column follows from the tag's scope, so this is what holds that scope
    /// against the record it claims to describe.
    #[test]
    fn test_carries_issue_id_matches_serialized_record() {
        for row in event_catalog() {
            let record =
                serde_json::to_value(row.tag.sample()).expect("an event record serializes to JSON");
            let object = record.as_object().expect("a record is a JSON object");
            assert_eq!(
                row.carries_issue_id,
                object.contains_key("issue_id"),
                "`{}`: catalog and serialized record disagree on issue_id",
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

    /// REQ-02: repository- and registry-scoped records, including the linked
    /// checkout override audit record, omit `issue_id`. Asserted as a set
    /// equality, so a tag that joins or leaves the no-issue set fails here.
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
                "artifact_archive_executed",
                "gate_definition_created",
                "gate_definition_removed",
                "gate_definition_updated",
                "lifecycle_timestamps_backfilled",
                "linked_checkout_write_overridden",
                "profile_lifecycle",
            ]),
        );
    }

    /// REQ-03: every cataloged record survives a serialization round trip with
    /// its tag intact — the property the event log depends on to read back what
    /// it wrote.
    #[test]
    fn test_sample_round_trips_through_serialization_for_every_tag() {
        for tag in EventTag::ALL {
            let event = tag.sample();
            let encoded = serde_json::to_string(&event).expect("a sample event serializes");
            let decoded: Event = serde_json::from_str(&encoded).expect("a serialized event parses");
            assert_eq!(decoded, event, "`{}` did not round-trip", tag.as_str());
            assert_eq!(decoded.tag(), tag);
        }
    }

    /// REQ-01: the override record names the linked checkout, the declared
    /// stance that would have refused the invocation, and the per-invocation
    /// override that permitted it. Asserted against the serialized record —
    /// the form `events.jsonl` stores — because the three names are the whole
    /// point of the variant.
    #[test]
    fn test_linked_checkout_write_overridden_names_checkout_stance_and_override() {
        let event = EventTag::LinkedCheckoutWriteOverridden.sample();
        let Event::LinkedCheckoutWriteOverridden {
            checkout,
            declared_stance,
            invocation_override,
            ..
        } = &event
        else {
            panic!("the tag's sample is the override record");
        };
        assert_eq!(*declared_stance, LinkedCheckoutWriteStance::Refuse);
        assert_eq!(*invocation_override, LinkedCheckoutWriteStance::Allow);

        let record = serde_json::to_value(&event).expect("the override record serializes");
        let object = record.as_object().expect("a record is a JSON object");
        assert_eq!(
            object.get("type").and_then(serde_json::Value::as_str),
            Some("linked_checkout_write_overridden"),
        );
        assert_eq!(
            object.get("checkout").and_then(serde_json::Value::as_str),
            checkout.to_str(),
            "the record names the linked checkout",
        );
        assert_eq!(
            object
                .get("declared_stance")
                .and_then(serde_json::Value::as_str),
            Some("refuse"),
            "the record names the stance that would have refused",
        );
        assert_eq!(
            object
                .get("invocation_override")
                .and_then(serde_json::Value::as_str),
            Some("allow"),
            "the record names the override that permitted the invocation",
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
            "{} is stale — regenerate it from `jit::domain::event_catalog` (run: {})",
            test_support::REFERENCE_PATH,
            test_support::REFERENCE_GENERATOR,
        );
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
