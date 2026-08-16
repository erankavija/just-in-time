//! Pure comparison of the records two repository stores hold.
//!
//! A repository's issue records and event history live in one store per
//! checkout. When two checkouts' stores disagree, the disagreement takes three
//! shapes: a record only one side holds (in either direction), and a record both
//! sides hold under one identity but with different values. [`compare_stores`]
//! names each of them as a [`StoreDivergence`] over [`StoreRecords`] read from
//! each side.
//!
//! This module is pure: it reads no filesystem, resolves no checkout, and takes
//! its two sides as already-loaded slices of domain records. The caller decides
//! which stores those slices came from and which side is which.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::types::{Event, Issue};

/// The kind of record a [`StoreDivergence`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DivergentRecord {
    /// An issue record, identified by its issue id.
    Issue,
    /// An event-log record, identified by its event id.
    Event,
}

impl DivergentRecord {
    /// Return the same token the serialized form carries.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Event => "event",
        }
    }
}

/// How one record differs between the two compared stores.
///
/// The three variants partition every disagreement: a record is held by one
/// side alone (in one direction or the other), or by both sides with values
/// that do not match. A record both sides hold with equal values is agreement
/// and yields no finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceClass {
    /// Held by the inspected checkout's store, absent from the reference store.
    LocalOnly,
    /// Held by the reference store, absent from the inspected checkout's store.
    ReferenceOnly,
    /// Held under one identity by both stores, with values that differ.
    Conflicting,
}

impl DivergenceClass {
    /// Return the same token the serialized form carries.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::ReferenceOnly => "reference_only",
            Self::Conflicting => "conflicting",
        }
    }
}

/// One record whose presence or value differs between two stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoreDivergence {
    /// Which kind of record diverged.
    pub record: DivergentRecord,
    /// How the two stores disagree about it.
    pub class: DivergenceClass,
    /// The record's identity, the key the two stores were matched on.
    pub id: String,
}

/// The records one store holds, as one side of a [`compare_stores`] call.
#[derive(Debug, Clone, Copy)]
pub struct StoreRecords<'a> {
    /// Every issue record the store holds.
    pub issues: &'a [Issue],
    /// Every event record the store's log holds.
    pub events: &'a [Event],
}

/// Name every record the two stores disagree about.
///
/// Issues are matched on issue id and events on event id. A record only `local`
/// holds is [`DivergenceClass::LocalOnly`], a record only `reference` holds is
/// [`DivergenceClass::ReferenceOnly`], and one both hold under the same identity
/// with differing values is [`DivergenceClass::Conflicting`]. Two stores holding
/// equal records yield an empty result, so an empty result is agreement.
///
/// Findings are ordered by record kind (issues, then events) and by identity
/// within each kind, so repeated comparisons of unchanged stores report the same
/// sequence. An identity a store repeats is compared as the whole run of records
/// carrying it, so a repetition one side does not match is itself a conflict.
pub fn compare_stores(
    local: StoreRecords<'_>,
    reference: StoreRecords<'_>,
) -> Vec<StoreDivergence> {
    compare_records(
        DivergentRecord::Issue,
        local.issues,
        reference.issues,
        |issue| issue.id.as_str(),
    )
    .chain(compare_records(
        DivergentRecord::Event,
        local.events,
        reference.events,
        Event::id,
    ))
    .collect()
}

/// Name the disagreements between two sides' records of one kind.
///
/// Both sides are grouped by identity so a repeated identity compares as the run
/// of records holding it rather than silently collapsing to one.
fn compare_records<'a, T: PartialEq>(
    record: DivergentRecord,
    local: &'a [T],
    reference: &'a [T],
    identity: impl Fn(&'a T) -> &'a str + Copy,
) -> impl Iterator<Item = StoreDivergence> + 'a {
    let local = group_by_identity(local, identity);
    let reference = group_by_identity(reference, identity);

    let identities: BTreeSet<&str> = local.keys().chain(reference.keys()).copied().collect();

    identities
        .into_iter()
        .filter_map(move |id| {
            let class = match (local.get(id), reference.get(id)) {
                (Some(_), None) => DivergenceClass::LocalOnly,
                (None, Some(_)) => DivergenceClass::ReferenceOnly,
                (Some(here), Some(there)) if here != there => DivergenceClass::Conflicting,
                _ => return None,
            };
            Some(StoreDivergence {
                record,
                class,
                id: id.to_string(),
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// Group one side's records by the identity the two stores are matched on.
fn group_by_identity<'a, T>(
    records: &'a [T],
    identity: impl Fn(&'a T) -> &'a str,
) -> BTreeMap<&'a str, Vec<&'a T>> {
    records.iter().fold(BTreeMap::new(), |mut grouped, record| {
        grouped.entry(identity(record)).or_default().push(record);
        grouped
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{fixture_issue, Priority};

    /// An event carrying `id`, associated with `issue_id`.
    fn event(id: &str, issue_id: &str, title: &str) -> Event {
        Event::IssueCreated {
            id: id.to_string(),
            issue_id: issue_id.to_string(),
            timestamp: chrono::DateTime::UNIX_EPOCH,
            title: title.to_string(),
            priority: Priority::Normal,
        }
    }

    /// One side of a comparison over the given records.
    fn side<'a>(issues: &'a [Issue], events: &'a [Event]) -> StoreRecords<'a> {
        StoreRecords { issues, events }
    }

    /// The findings of `kind`, as (class, id) pairs.
    fn findings_of(
        divergences: &[StoreDivergence],
        kind: DivergentRecord,
    ) -> Vec<(DivergenceClass, &str)> {
        divergences
            .iter()
            .filter(|divergence| divergence.record == kind)
            .map(|divergence| (divergence.class, divergence.id.as_str()))
            .collect()
    }

    #[test]
    fn test_compare_stores_names_issue_records_held_by_one_store_in_each_direction() {
        let shared = fixture_issue("Shared".to_string(), "Both stores hold it".to_string());
        let here = fixture_issue("Here".to_string(), "Only the local store".to_string());
        let there = fixture_issue("There".to_string(), "Only the reference store".to_string());

        let local = [shared.clone(), here.clone()];
        let reference = [shared.clone(), there.clone()];

        let divergences = compare_stores(side(&local, &[]), side(&reference, &[]));

        let issues = findings_of(&divergences, DivergentRecord::Issue);
        assert!(
            issues.contains(&(DivergenceClass::LocalOnly, here.id.as_str())),
            "the record only the local store holds is reported as local-only: {divergences:?}"
        );
        assert!(
            issues.contains(&(DivergenceClass::ReferenceOnly, there.id.as_str())),
            "the record only the reference store holds is reported as reference-only: {divergences:?}"
        );
        assert!(
            !issues.iter().any(|(_, id)| *id == shared.id),
            "a record both stores hold identically is no divergence: {divergences:?}"
        );
    }

    #[test]
    fn test_compare_stores_names_event_records_held_by_one_log_in_each_direction() {
        let shared = event("shared-event", "issue-a", "Both logs hold it");
        let here = event("local-event", "issue-b", "Only the local log");
        let there = event("reference-event", "issue-c", "Only the reference log");

        let local = [shared.clone(), here.clone()];
        let reference = [shared.clone(), there.clone()];

        let divergences = compare_stores(side(&[], &local), side(&[], &reference));

        let events = findings_of(&divergences, DivergentRecord::Event);
        assert!(
            events.contains(&(DivergenceClass::LocalOnly, here.id())),
            "the event only the local log holds is reported as local-only: {divergences:?}"
        );
        assert!(
            events.contains(&(DivergenceClass::ReferenceOnly, there.id())),
            "the event only the reference log holds is reported as reference-only: {divergences:?}"
        );
        assert!(
            !events.iter().any(|(_, id)| *id == shared.id()),
            "an event both logs hold identically is no divergence: {divergences:?}"
        );
    }

    #[test]
    fn test_compare_stores_separates_conflicting_shared_records_from_one_sided_records() {
        let mut here = fixture_issue("Same id".to_string(), "The local value".to_string());
        let mut there = here.clone();
        there.description = "The reference value".to_string();
        here.title = "Diverged locally".to_string();

        let local_event = event("shared-event", "issue-a", "The local value");
        let reference_event = event("shared-event", "issue-a", "The reference value");

        let one_sided = fixture_issue("One sided".to_string(), "Local store only".to_string());

        let local = [here.clone(), one_sided.clone()];
        let reference = [there.clone()];

        let divergences = compare_stores(
            side(&local, std::slice::from_ref(&local_event)),
            side(&reference, std::slice::from_ref(&reference_event)),
        );

        let issues = findings_of(&divergences, DivergentRecord::Issue);
        assert!(
            issues.contains(&(DivergenceClass::Conflicting, here.id.as_str())),
            "an id both stores hold with different values is a conflict: {divergences:?}"
        );
        assert!(
            issues.contains(&(DivergenceClass::LocalOnly, one_sided.id.as_str())),
            "a one-sided record keeps its own class beside a conflict: {divergences:?}"
        );
        assert_eq!(
            findings_of(&divergences, DivergentRecord::Event),
            vec![(DivergenceClass::Conflicting, local_event.id())],
            "an event id both logs hold with different values is a conflict, not a one-sided record"
        );
        assert!(
            divergences
                .iter()
                .all(|divergence| divergence.class != DivergenceClass::ReferenceOnly),
            "a conflicting record is not also reported as held by one side: {divergences:?}"
        );
    }

    #[test]
    fn test_compare_stores_reports_nothing_for_stores_holding_equal_records() {
        let issues = [
            fixture_issue("First".to_string(), "Held by both".to_string()),
            fixture_issue("Second".to_string(), "Held by both".to_string()),
        ];
        let events = [
            event("event-1", "issue-a", "Held by both"),
            event("event-2", "issue-b", "Held by both"),
        ];

        assert_eq!(
            compare_stores(side(&issues, &events), side(&issues, &events)),
            Vec::new(),
            "stores holding equal records agree, so the comparison is empty"
        );
    }

    #[test]
    fn test_compare_stores_orders_findings_by_record_kind_and_identity() {
        let issues = [
            fixture_issue("B".to_string(), "Local only".to_string()),
            fixture_issue("A".to_string(), "Local only".to_string()),
        ];
        let events = [
            event("event-b", "issue-a", "Local only"),
            event("event-a", "issue-b", "Local only"),
        ];

        let divergences = compare_stores(side(&issues, &events), side(&[], &[]));

        let kinds: Vec<DivergentRecord> = divergences
            .iter()
            .map(|divergence| divergence.record)
            .collect();
        assert_eq!(
            kinds,
            vec![
                DivergentRecord::Issue,
                DivergentRecord::Issue,
                DivergentRecord::Event,
                DivergentRecord::Event
            ],
            "issue findings precede event findings"
        );

        let event_ids: Vec<&str> = findings_of(&divergences, DivergentRecord::Event)
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        assert!(
            event_ids.windows(2).all(|pair| pair[0] <= pair[1]),
            "findings of one kind are ordered by identity: {event_ids:?}"
        );
    }

    #[test]
    fn test_compare_stores_reports_an_unmatched_repeated_identity_as_a_conflict() {
        let repeated = event("event-1", "issue-a", "Recorded twice locally");
        let local = [repeated.clone(), repeated.clone()];
        let reference = [repeated.clone()];

        assert_eq!(
            findings_of(
                &compare_stores(side(&[], &local), side(&[], &reference)),
                DivergentRecord::Event
            ),
            vec![(DivergenceClass::Conflicting, repeated.id())],
            "one log holding an identity twice disagrees with a log holding it once"
        );
    }

    #[test]
    fn test_as_str_matches_the_serialized_token_for_every_variant() {
        for record in [DivergentRecord::Issue, DivergentRecord::Event] {
            let serialized = serde_json::to_string(&record).expect("record serialization");
            assert_eq!(record.as_str(), serialized.trim_matches('"'));
        }

        for class in [
            DivergenceClass::LocalOnly,
            DivergenceClass::ReferenceOnly,
            DivergenceClass::Conflicting,
        ] {
            let serialized = serde_json::to_string(&class).expect("class serialization");
            assert_eq!(class.as_str(), serialized.trim_matches('"'));
        }
    }
}
