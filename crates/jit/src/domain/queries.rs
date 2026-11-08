//! Pure query operations on issue collections.
//!
//! This module provides domain-level query functions that operate on slices of issues
//! without requiring storage access. These are pure functions that can be used
//! independently of the CLI orchestration layer.

use crate::domain::identity::GateKey;
use crate::domain::identity::IssueId;
use crate::domain::identity::IssueIdPrefix;
use crate::domain::{
    is_dependency_met, Assignee, GateStatus, Issue, Priority, ReadinessCorrection, State,
};
use crate::graph::{DependencyGraph, Direction};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Optional restrictions on all issues; every supplied filter must match.
#[derive(Debug, Default, Clone)]
pub struct AllFilter<'a> {
    pub state: Option<State>,
    pub assignee: Option<Assignee>,
    pub priority: Option<Priority>,
    pub labels: &'a [String],
}

/// Return issues admitted by every filter, preserving input order.
pub fn query_all(issues: &[Issue], filter: AllFilter<'_>) -> Vec<Issue> {
    issues
        .iter()
        .filter(|issue| {
            filter.state.is_none_or(|state| issue.state == state)
                && filter.assignee.as_ref().is_none_or(|assignee| {
                    issue
                        .assignee
                        .as_ref()
                        .is_some_and(|value| value == assignee)
                })
                && filter
                    .priority
                    .is_none_or(|priority| issue.priority == priority)
                && filter
                    .labels
                    .iter()
                    .all(|pattern| crate::labels::matches_pattern(&issue.labels, pattern))
        })
        .cloned()
        .collect()
}

/// Optional restrictions on available work; label patterns are AND-combined.
#[derive(Debug, Default, Clone, Copy)]
pub struct AvailableFilter<'a> {
    pub priority: Option<Priority>,
    pub labels: &'a [String],
}

/// Return matching ready issues in priority order, preserving input order for ties.
pub fn query_available(issues: &[Issue], filter: AvailableFilter<'_>) -> Vec<Issue> {
    let ready = query_ready(issues);
    let mut matches = query_by_labels(&ready, filter.labels);
    if let Some(priority) = filter.priority {
        matches = query_by_priority(&matches, priority);
    }
    matches.sort_by_key(|issue| Reverse(issue.priority));
    matches
}

/// Build a HashMap of issue ID to issue reference for dependency resolution.
///
/// This is a common pattern used throughout query and validation code to enable
/// efficient lookups when checking if issues are blocked by dependencies.
pub fn build_issue_map(issues: &[Issue]) -> HashMap<IssueId, &Issue> {
    issues.iter().map(|i| (i.id.clone(), i)).collect()
}

/// The dependencies of `issue` that [`is_dependency_met`] rejects, resolved
/// against `resolved_issues`.
///
/// This is the population [`Issue::is_blocked`] answers over, so every surface
/// that explains a blocked issue names the same dependencies the predicate
/// counts. A dependency id that resolves to no issue blocks the issue too, but
/// carries no record to return; whole-repository validation reports such a
/// dangling id as its own defect.
pub fn unmet_dependencies<'a>(
    issue: &Issue,
    resolved_issues: &HashMap<IssueId, &'a Issue>,
) -> Vec<&'a Issue> {
    issue
        .dependencies
        .iter()
        .filter_map(|dependency_id| resolved_issues.get(dependency_id).copied())
        .filter(|dependency| !is_dependency_met(dependency.state, dependency.archived_from))
        .collect()
}

/// Query issues that are ready to be worked on.
///
/// Returns issues that are:
/// - In `Ready` state
/// - Unassigned
/// - Free of unmet dependencies (gates gate completion, not start)
pub fn query_ready(issues: &[Issue]) -> Vec<Issue> {
    let resolved = build_issue_map(issues);

    issues
        .iter()
        .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
        .cloned()
        .collect()
}

/// Positions of a record set's issues by id and in id order: the one id and
/// prefix resolution over a record set. Reads take the records it was built
/// from and count as keyed lookups.
#[derive(Debug, Clone, Default)]
pub struct IdIndex {
    positions: HashMap<IssueId, usize>,
    /// Positions in id order, which is also normalised-id order, so an id
    /// prefix resolves by binary search.
    ordered: Vec<usize>,
}

impl IdIndex {
    pub fn new(records: &[Issue]) -> Self {
        let mut ordered = (0..records.len()).collect::<Vec<_>>();
        ordered.sort_by(|&left, &right| records[left].id.cmp(&records[right].id));
        Self {
            positions: records
                .iter()
                .enumerate()
                .map(|(position, issue)| (issue.id.clone(), position))
                .collect(),
            ordered,
        }
    }

    pub fn contains(&self, id: &IssueId) -> bool {
        note_index_lookup();
        self.positions.contains_key(id)
    }

    /// The record of `records` with `id`.
    pub fn get<'r>(&self, records: &'r [Issue], id: &IssueId) -> Option<&'r Issue> {
        note_index_lookup();
        self.positions.get(id).map(|&position| &records[position])
    }

    /// The records of `records` `prefix` names, in id order.
    pub fn named<'r>(&self, records: &'r [Issue], prefix: &IssueIdPrefix) -> Vec<&'r Issue> {
        note_index_lookup();
        let wanted = prefix.normalised().as_bytes();
        let start = self.ordered.partition_point(|&position| {
            records[position]
                .id
                .as_str()
                .bytes()
                .filter(|byte| *byte != b'-')
                .take(wanted.len())
                .lt(wanted.iter().copied())
        });
        self.ordered[start..]
            .iter()
            .map(|&position| &records[position])
            .take_while(|issue| prefix.matches(&issue.id))
            .collect()
    }
}

/// A parsed record set with its [`IdIndex`], read only through counted
/// lookups and passes.
#[derive(Debug, Clone)]
pub struct IndexedIssues {
    records: std::sync::Arc<[Issue]>,
    ids: IdIndex,
}

impl IndexedIssues {
    pub fn new(records: impl Into<std::sync::Arc<[Issue]>>) -> Self {
        let records = records.into();
        Self {
            ids: IdIndex::new(&records),
            records,
        }
    }

    pub fn get(&self, id: &IssueId) -> Option<&Issue> {
        self.ids.get(&self.records, id)
    }

    /// The records `prefix` names, in id order.
    pub fn named(&self, prefix: &IssueIdPrefix) -> Vec<&Issue> {
        self.ids.named(&self.records, prefix)
    }

    /// Every record, in parse order.
    pub fn records(&self) -> &[Issue] {
        note_repository_pass(self.records.len());
        &self.records
    }
}

/// Captured issues keyed by id over the session's [`DependencyGraph`], whose
/// reverse index answers dependents, and by label, so a derivation over a few
/// issues reads their neighbours and label holders without scanning the issue
/// list. Records replaced since capture shadow the captured ones.
pub struct IssueIndex<'a> {
    captured: &'a [Issue],
    ids: IdIndex,
    graph: DependencyGraph<'a, Issue>,
    replaced: HashMap<IssueId, Issue>,
    /// Ids absent from the capture, in insertion order.
    added: Vec<IssueId>,
    /// Dependents gained by replaced records' dependency lists, which the
    /// graph, built over the capture, lacks.
    relinked: HashMap<IssueId, BTreeSet<IssueId>>,
    /// Captured positions by label `(namespace, value)`, built on first use.
    labels: std::cell::OnceCell<BTreeMap<(String, String), BTreeSet<usize>>>,
    /// Holders gained by replaced records' labels, which `labels` lacks.
    relabelled: BTreeMap<(String, String), BTreeSet<IssueId>>,
}

/// `label` split as `(namespace, value)`.
fn label_parts(label: &str) -> Option<(&str, &str)> {
    label.split_once(':')
}

impl<'a> IssueIndex<'a> {
    pub fn new(captured: &'a [Issue]) -> Self {
        Self {
            captured,
            ids: IdIndex::new(captured),
            graph: DependencyGraph::new(&captured.iter().collect::<Vec<_>>()),
            replaced: HashMap::new(),
            added: Vec::new(),
            relinked: HashMap::new(),
            labels: std::cell::OnceCell::new(),
            relabelled: BTreeMap::new(),
        }
    }

    /// Every issue: the captured ones in capture order, then the added ones.
    pub fn issues(&self) -> impl Iterator<Item = &Issue> + '_ {
        note_repository_pass(self.captured.len() + self.added.len());
        self.captured
            .iter()
            .map(|issue| self.replaced.get(&issue.id).unwrap_or(issue))
            .chain(self.added.iter().filter_map(|id| self.replaced.get(id)))
    }

    pub fn get(&self, id: &IssueId) -> Option<&Issue> {
        self.replaced.get(id).or_else(|| self.captured_record(id))
    }

    fn captured_record(&self, id: &IssueId) -> Option<&'a Issue> {
        self.ids.get(self.captured, id)
    }

    /// Insert `issue`, or replace the record with its id.
    pub fn replace(&mut self, issue: Issue) {
        let captured = self.captured_record(&issue.id);
        issue
            .dependencies
            .iter()
            .filter(|id| captured.is_none_or(|captured| !captured.dependencies.contains(id)))
            .for_each(|id| {
                self.relinked
                    .entry(id.clone())
                    .or_default()
                    .insert(issue.id.clone());
            });
        issue
            .labels
            .iter()
            .filter(|label| captured.is_none_or(|captured| !captured.labels.contains(label)))
            .filter_map(|label| label_parts(label))
            .for_each(|(namespace, value)| {
                self.relabelled
                    .entry((namespace.to_string(), value.to_string()))
                    .or_default()
                    .insert(issue.id.clone());
            });
        if captured.is_none() && !self.replaced.contains_key(&issue.id) {
            self.added.push(issue.id.clone());
        }
        self.replaced.insert(issue.id.clone(), issue);
    }

    /// This index as it stands.
    pub fn view(&self) -> IssueView<'_> {
        self.with(&[])
    }

    /// This index with `replaced` records, the last for an id winning, in place
    /// of or beside its own.
    pub fn with<'v>(&'v self, replaced: &'v [Issue]) -> IssueView<'v> {
        IssueView {
            index: self,
            replaced,
        }
    }

    /// Ids that have depended on `id` in some record: a superset, since
    /// [`IssueView::dependents`] rechecks each current record.
    fn candidate_dependents(&self, id: &IssueId) -> impl Iterator<Item = &IssueId> + '_ {
        self.graph
            .get_dependents(id)
            .into_iter()
            .map(|issue| &issue.id)
            .chain(self.relinked.get(id).into_iter().flatten())
    }

    /// Ids that have carried `namespace:value` (any value in `namespace` when
    /// `value` is `None`) in some record: a superset, since
    /// [`IssueView::labelled`] rechecks each current record.
    fn candidate_holders(&self, namespace: &str, value: Option<&str>) -> Vec<&IssueId> {
        let captured = self.labels.get_or_init(|| {
            self.captured.iter().enumerate().fold(
                BTreeMap::new(),
                |mut holders, (position, issue)| {
                    issue
                        .labels
                        .iter()
                        .filter_map(|label| label_parts(label))
                        .for_each(|(namespace, value)| {
                            holders
                                .entry((namespace.to_string(), value.to_string()))
                                .or_default()
                                .insert(position);
                        });
                    holders
                },
            )
        });
        let in_scope = |key_namespace: &str, key_value: &str| {
            key_namespace == namespace && value.is_none_or(|value| key_value == value)
        };
        let start = (namespace.to_string(), value.unwrap_or_default().to_string());
        let relabelled = self
            .relabelled
            .range(start.clone()..)
            .take_while(|((key_namespace, key_value), _)| in_scope(key_namespace, key_value))
            .flat_map(|(_, ids)| ids);
        captured
            .range(start..)
            .take_while(|((key_namespace, key_value), _)| in_scope(key_namespace, key_value))
            .flat_map(|(_, positions)| {
                positions
                    .iter()
                    .map(|&position| &self.captured[position].id)
            })
            .chain(relabelled)
            .collect()
    }
}

/// An [`IssueIndex`] read through a few replaced records, which cost each read
/// at most a pass over them.
#[derive(Clone, Copy)]
pub struct IssueView<'a> {
    index: &'a IssueIndex<'a>,
    replaced: &'a [Issue],
}

impl<'a> IssueView<'a> {
    pub fn index(&self) -> &'a IssueIndex<'a> {
        self.index
    }

    pub fn replaced(&self) -> &'a [Issue] {
        self.replaced
    }

    pub fn get(&self, id: &IssueId) -> Option<&'a Issue> {
        self.replaced
            .iter()
            .rev()
            .find(|issue| &issue.id == id)
            .or_else(|| self.index.get(id))
    }

    /// Every issue: the index's in its order, then the replaced records it
    /// lacks.
    pub fn issues(&self) -> impl Iterator<Item = &'a Issue> + 'a {
        let view = *self;
        let added = self
            .replaced
            .iter()
            .enumerate()
            .filter(move |(position, issue)| {
                !view.index.ids.contains(&issue.id)
                    && !view.index.replaced.contains_key(&issue.id)
                    && !view.replaced[position + 1..]
                        .iter()
                        .any(|later| later.id == issue.id)
            })
            .map(|(_, issue)| issue);
        self.index
            .issues()
            .filter_map(move |issue| view.get(&issue.id))
            .chain(added)
    }

    /// The ids of the issues whose dependency list names `id`.
    pub fn dependents(&self, id: &IssueId) -> BTreeSet<&'a IssueId> {
        note_index_lookup();
        let view = *self;
        let depends = move |issue: &Issue| issue.dependencies.contains(id);
        self.index
            .candidate_dependents(id)
            .filter(|dependent| view.get(dependent).is_some_and(depends))
            .chain(
                self.replaced
                    .iter()
                    .map(|issue| &issue.id)
                    .filter(|replaced| view.get(replaced).is_some_and(depends)),
            )
            .collect()
    }

    /// The issues `prefix` names, in id order.
    pub fn resolve_prefix(&self, prefix: &IssueIdPrefix) -> Vec<&'a Issue> {
        let index = self.index;
        index
            .ids
            .named(index.captured, prefix)
            .into_iter()
            .map(|issue| &issue.id)
            .chain(index.added.iter().filter(|id| prefix.matches(id)))
            .chain(
                self.replaced
                    .iter()
                    .map(|issue| &issue.id)
                    .filter(|id| prefix.matches(id)),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// The issues `start` reaches along dependency edges in `direction`, each
    /// once and without `start`; a reached issue `admits` rejects is dropped
    /// and not walked through.
    pub fn reach<'s>(
        &self,
        start: &'s IssueId,
        direction: Direction,
        admits: impl Fn(&Issue) -> bool,
    ) -> Vec<&'a Issue>
    where
        'a: 's,
    {
        crate::graph::traverse_with(
            start,
            0,
            |id| -> Vec<&'s IssueId> {
                match direction {
                    Direction::Dependencies => self
                        .get(id)
                        .map(|issue| issue.dependencies.iter().collect())
                        .unwrap_or_default(),
                    Direction::Dependents => self.dependents(id).into_iter().collect(),
                }
            },
            |id| self.get(id),
            |issue| admits(issue),
        )
    }

    /// The issues carrying `namespace:value`, or any label in `namespace` when
    /// `value` is `None`, in id order.
    pub fn labelled(&self, namespace: &str, value: Option<&str>) -> Vec<&'a Issue> {
        note_index_lookup();
        let carries = |issue: &Issue| {
            issue
                .labels
                .iter()
                .filter_map(|label| label_parts(label))
                .any(|(key_namespace, key_value)| {
                    key_namespace == namespace && value.is_none_or(|value| key_value == value)
                })
        };
        self.index
            .candidate_holders(namespace, value)
            .into_iter()
            .chain(self.replaced.iter().map(|issue| &issue.id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.get(id))
            .filter(|issue| carries(issue))
            .collect()
    }

    /// `issue`'s dependencies that resolve to an issue, keyed by id.
    pub fn resolve_dependencies(&self, issue: &Issue) -> HashMap<IssueId, &'a Issue> {
        issue
            .dependencies
            .iter()
            .filter_map(|id| self.get(id).map(|dependency| (id.clone(), dependency)))
            .collect()
    }
}

/// Repository reads on the current thread: keyed [`IssueIndex`] lookups, and
/// the records visited by passes over the repository, parsed issue records
/// included.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RepositoryReads {
    pub(crate) lookups: usize,
    pub(crate) scanned: usize,
}

#[cfg(test)]
impl std::ops::Sub for RepositoryReads {
    type Output = Self;

    fn sub(self, earlier: Self) -> Self {
        Self {
            lookups: self.lookups - earlier.lookups,
            scanned: self.scanned - earlier.scanned,
        }
    }
}

#[cfg(test)]
thread_local! {
    static REPOSITORY_READS: std::cell::Cell<RepositoryReads> = const {
        std::cell::Cell::new(RepositoryReads { lookups: 0, scanned: 0 })
    };
}

/// `run`'s result and the repository reads it made on this thread.
#[cfg(test)]
pub(crate) fn count_repository_reads<T>(run: impl FnOnce() -> T) -> (T, RepositoryReads) {
    let before = REPOSITORY_READS.with(std::cell::Cell::get);
    let value = run();
    (value, REPOSITORY_READS.with(std::cell::Cell::get) - before)
}

fn note_index_lookup() {
    #[cfg(test)]
    REPOSITORY_READS.with(|reads| {
        reads.set(RepositoryReads {
            lookups: reads.get().lookups + 1,
            ..reads.get()
        })
    });
}

/// Note a pass visiting `records` records of the repository.
pub(crate) fn note_repository_pass(records: usize) {
    #[cfg(test)]
    REPOSITORY_READS.with(|reads| {
        reads.set(RepositoryReads {
            scanned: reads.get().scanned + records,
            ..reads.get()
        })
    });
    #[cfg(not(test))]
    let _ = records;
}

/// The issues [`derive_readiness_corrections`] re-derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessScope {
    /// Issues a mutation can affect: new issues, issues whose dependency list
    /// changed, and direct dependents of an issue whose effective terminal state
    /// changed or which was removed.
    Affected,
    /// Every issue after the mutation.
    All,
}

/// The readiness corrections the dependency graph demands of `after`, the
/// issues following a mutation of `before`, for the issues `scope` admits,
/// ordered by issue id.
///
/// A correction moves an issue only between `Backlog` and `Ready`, which meets
/// no dependency and unmeets none, so one pass over `after` is complete.
pub fn derive_readiness_corrections(
    before: &[Issue],
    after: &[Issue],
    scope: ReadinessScope,
) -> Vec<(IssueId, ReadinessCorrection)> {
    derive_indexed_readiness_corrections(before, IssueIndex::new(after).view(), scope)
}

/// [`derive_readiness_corrections`] over an indexed `after`.
pub fn derive_indexed_readiness_corrections(
    before: &[Issue],
    after: IssueView<'_>,
    scope: ReadinessScope,
) -> Vec<(IssueId, ReadinessCorrection)> {
    match scope {
        ReadinessScope::All => correct_readiness(
            after,
            after
                .issues()
                .map(|issue| issue.id.clone())
                .collect::<BTreeSet<_>>(),
        ),
        ReadinessScope::Affected => {
            let prior = build_issue_map(before);
            let current = after
                .issues()
                .map(|issue| (&issue.id, prior.get(&issue.id).copied()));
            let removed = before
                .iter()
                .filter(|issue| after.get(&issue.id).is_none())
                .map(|issue| (&issue.id, Some(issue)));
            derive_changed_readiness_corrections(after, current.chain(removed))
        }
    }
}

/// The readiness corrections `after` demands once each `(id, prior)` change
/// replaced `prior`, the record it held (`None` for a new issue), with
/// `after`'s record for `id` (none for a removed issue), ordered by issue id.
///
/// Each change re-derives its issue when it is new or its dependency list
/// changed, and its direct dependents when its effective terminal state changed
/// or it was removed, so the work is bounded by the changes' neighbourhoods.
pub fn derive_changed_readiness_corrections<'p>(
    after: IssueView<'_>,
    changes: impl IntoIterator<Item = (&'p IssueId, Option<&'p Issue>)>,
) -> Vec<(IssueId, ReadinessCorrection)> {
    let admitted = changes
        .into_iter()
        .flat_map(|(id, prior)| {
            let current = after.get(id);
            let rederives_issue = current.is_some_and(|current| {
                prior.is_none_or(|prior| prior.dependencies != current.dependencies)
            });
            let rederives_dependents = prior.is_some_and(|prior| {
                current.is_none_or(|current| {
                    prior.effective_terminal_state() != current.effective_terminal_state()
                })
            });
            rederives_issue.then(|| id.clone()).into_iter().chain(
                rederives_dependents
                    .then(|| after.dependents(id))
                    .into_iter()
                    .flatten()
                    .cloned(),
            )
        })
        .collect::<BTreeSet<_>>();
    correct_readiness(after, admitted)
}

fn correct_readiness(
    after: IssueView<'_>,
    admitted: BTreeSet<IssueId>,
) -> Vec<(IssueId, ReadinessCorrection)> {
    admitted
        .into_iter()
        .filter_map(|id| {
            let issue = after.get(&id)?;
            issue
                .derive_readiness_correction(&after.resolve_dependencies(issue))
                .map(|correction| (id, correction))
        })
        .collect()
}

/// A typed reason explaining why an issue is blocked.
///
/// Produced by [`query_blocked`]: the CLI matches on these variants to build its
/// presentation form without re-parsing text. The [`Display`](std::fmt::Display)
/// implementation renders the human reason string (`dependency:<id>
/// (<title>:<state>)` and `gate:<key> (<status>)`) with wire tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingReason {
    /// An unmet dependency: the issue waits on `id` (titled `title`), which is in
    /// `state`, a state that [`is_dependency_met`] rejects (anything short of an
    /// effective terminal state — `Done`, `Rejected`, or `Archived` retired from
    /// one of those).
    Dependency {
        /// Id of the depended-on issue.
        id: IssueId,
        /// Title of the depended-on issue.
        title: String,
        /// Current state of the depended-on issue.
        state: State,
    },
    /// A required gate that is not yet `Passed`. `status` is the gate's current
    /// status, defaulting to [`GateStatus::Pending`] when it has never been
    /// evaluated.
    Gate {
        /// The required gate key.
        key: GateKey,
        /// Current gate status.
        status: GateStatus,
    },
}

impl std::fmt::Display for BlockingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockingReason::Dependency { id, title, state } => {
                write!(f, "dependency:{} ({}:{})", id, title, state.as_str())
            }
            BlockingReason::Gate { key, status } => {
                write!(f, "gate:{} ({})", key, status)
            }
        }
    }
}

/// Query blocked issues with typed reasons for being blocked.
///
/// Returns the issues [`Issue::is_blocked`] holds back, each with the list of
/// [`BlockingReason`]s explaining it: every unmet dependency (by
/// [`is_dependency_met`]) plus every required gate that has not passed.
pub fn query_blocked(issues: &[Issue]) -> Vec<(Issue, Vec<BlockingReason>)> {
    let resolved = build_issue_map(issues);

    issues
        .iter()
        .filter(|issue| issue.is_blocked(&resolved))
        .map(|issue| {
            // Unmet dependencies, by the one predicate every surface shares.
            let dep_reasons = unmet_dependencies(issue, &resolved).into_iter().map(|dep| {
                BlockingReason::Dependency {
                    id: dep.id.clone(),
                    title: dep.title.clone(),
                    state: dep.state,
                }
            });

            // Required gates that have not passed (absent status reads as Pending).
            let gate_reasons = issue.gates_required.iter().filter_map(|gate_key| {
                let gate_state = issue.gates_status.get(gate_key);
                let is_passed = gate_state
                    .map(|gs| gs.status == GateStatus::Passed)
                    .unwrap_or(false);

                (!is_passed).then(|| BlockingReason::Gate {
                    key: gate_key.clone(),
                    status: gate_state
                        .map(|gs| gs.status)
                        .unwrap_or(GateStatus::Pending),
                })
            });

            (issue.clone(), dep_reasons.chain(gate_reasons).collect())
        })
        .collect()
}

/// Query issues by assignee.
pub fn query_by_assignee(issues: &[Issue], assignee: &Assignee) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.assignee.as_ref() == Some(assignee))
        .cloned()
        .collect()
}

/// Query issues by state.
pub fn query_by_state(issues: &[Issue], state: State) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.state == state)
        .cloned()
        .collect()
}

/// Query issues by priority.
pub fn query_by_priority(issues: &[Issue], priority: Priority) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.priority == priority)
        .cloned()
        .collect()
}

/// Query issues by label pattern.
///
/// Pattern format: `namespace:value` or `namespace:*` for wildcard matching.
pub fn query_by_label(issues: &[Issue], pattern: &str) -> Vec<Issue> {
    use crate::labels;

    issues
        .iter()
        .filter(|issue| labels::matches_pattern(&issue.labels, pattern))
        .cloned()
        .collect()
}

/// Query issues matching every given label pattern (AND-combined).
///
/// Each pattern uses the same `namespace:value` / `namespace:*` wildcard
/// syntax as [`query_by_label`]; an issue is kept only when it matches every
/// pattern in `patterns`. An empty `patterns` slice matches every issue.
pub fn query_by_labels(issues: &[Issue], patterns: &[String]) -> Vec<Issue> {
    use crate::labels;

    issues
        .iter()
        .filter(|issue| {
            patterns
                .iter()
                .all(|pattern| labels::matches_pattern(&issue.labels, pattern))
        })
        .cloned()
        .collect()
}

/// Query strategic issues (those with strategic type labels).
///
/// Strategic types are defined in configuration (e.g., milestone, epic).
/// This function takes the list of strategic type names as a parameter.
pub fn query_strategic(issues: &[Issue], strategic_types: &[String]) -> Vec<Issue> {
    use crate::labels;

    if strategic_types.is_empty() {
        return Vec::new();
    }

    issues
        .iter()
        .filter(|issue| {
            strategic_types.iter().any(|type_value| {
                labels::matches_pattern(&issue.labels, &labels::type_label(type_value))
            })
        })
        .cloned()
        .collect()
}

/// Query closed issues (effectively terminal: `Done`, `Rejected`, or `Archived`
/// retired from one of those — [`Issue::is_effectively_terminal`]).
pub fn query_closed(issues: &[Issue]) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.is_effectively_terminal())
        .cloned()
        .collect()
}

/// Count issues by state, enumerating **every** [`State`] variant.
///
/// The result carries one `(state, count)` pair per variant in
/// [`State::ALL`] order, so a state with no issues is present with count `0`
/// (never omitted). This keeps the aggregation shape stable and complete
/// regardless of which states happen to be populated, and derives the state
/// list from the enum rather than hardcoding it (@/inv/domain-agnostic).
pub fn count_by_state(issues: &[Issue]) -> Vec<(State, usize)> {
    State::ALL
        .into_iter()
        .map(|state| (state, issues.iter().filter(|i| i.state == state).count()))
        .collect()
}

/// Compute the bracket-scope membership for `jit validate --scope <container>`:
/// the container's transitive dependency closure, **including** any breakdown
/// boundary node `B` reached, but **bounded** so the walk does not descend
/// through `B` into its dependencies (`P` / upstream). The container id itself is
/// always a member.
///
/// The breakdown boundary type is supplied by the caller, not baked in: `B` is
/// the node carrying `type:<breakdown_type>`, where `breakdown_type` is the
/// caller's configured breakdown-node type (e.g. `"breakdown"`, or any custom
/// name) — derived by the caller from the applicable graph template's breakdown
/// node. The label string is built at runtime from that name, so the engine stays
/// domain-agnostic.
///
/// When `breakdown_type` is `None` — i.e. no applicable graph template, so the
/// container is not bracketed — there is no breakdown boundary: the walk is the
/// full transitive dependency closure of the container, with no halt.
///
/// This is a pure graph walk over dependency edges (no I/O), resolving each id
/// through `resolve`: a walk from the
/// container that adds each dependency it reaches and continues descending,
/// except that (when a boundary type is configured) it never enqueues the
/// dependencies OF a boundary node — so `B` is collected as the boundary but
/// everything strictly upstream of it is excluded. The container is matched by
/// its full id (the caller resolves partial ids before calling). A missing
/// container yields just that id.
///
/// Membership here decides whose `when` rules are evaluated (D14); it is
/// independent of `child-type-exclude`, which governs only the coverage rule's
/// candidate traversal.
///
/// # Examples
///
/// ```rust
/// use jit::domain::queries::bracket_scope_ids;
/// use jit::domain::Issue;
/// use jit::test_utils::issue_id;
///
/// // A spine `C -> I -> B -> P`: container, an impl task, a breakdown node,
/// // and the plan upstream of the breakdown. Edges point dependent -> dependency.
/// let mut c = Issue::draft(issue_id(1), "container".to_string(), String::new());
/// c.labels = vec!["type:epic".to_string()];
/// c.dependencies = vec![issue_id(2)];
///
/// let mut i = Issue::draft(issue_id(2), "impl".to_string(), String::new());
/// i.dependencies = vec![issue_id(3)];
///
/// let mut b = Issue::draft(issue_id(3), "breakdown".to_string(), String::new());
/// b.labels = vec!["type:breakdown".to_string()];
/// b.dependencies = vec![issue_id(4)];
///
/// let p = Issue::draft(issue_id(4), "plan".to_string(), String::new());
///
/// let issues = jit::domain::queries::IndexedIssues::new(vec![c, i, b, p]);
/// let resolve = |id: &_| issues.get(id);
///
/// // The walk halts AT the breakdown node: C, I and B are in scope, P is not.
/// let ids = bracket_scope_ids(&issue_id(1), resolve, Some("breakdown"));
/// assert!(ids.contains(&issue_id(1)) && ids.contains(&issue_id(2)) && ids.contains(&issue_id(3)));
/// assert!(!ids.contains(&issue_id(4)));
///
/// // With no configured boundary the walk descends the full closure.
/// let full = bracket_scope_ids(&issue_id(1), resolve, None);
/// assert!(full.contains(&issue_id(4)));
/// ```
pub fn bracket_scope_ids<'a>(
    container_id: &IssueId,
    resolve: impl Fn(&IssueId) -> Option<&'a Issue>,
    breakdown_type: Option<&str>,
) -> std::collections::HashSet<IssueId> {
    // The boundary label, built from the caller-supplied breakdown-node type
    // (e.g. `type:breakdown`). Absent when the container is not bracketed (no
    // applicable graph template), in which case nothing halts the walk and the
    // result is the full dependency closure.
    let breakdown_label = breakdown_type.map(crate::labels::type_label);
    let is_breakdown = |issue: &Issue| {
        breakdown_label
            .as_ref()
            .is_some_and(|label| issue.labels.contains(label))
    };
    let Some(container) = resolve(container_id) else {
        return std::collections::HashSet::from([container_id.clone()]);
    };
    // Halt the walk AT a breakdown node: it is in scope, but its own
    // dependencies (the plan `P` / upstream) are out of scope. A dangling edge
    // resolves to no issue and is skipped; the integrity check owns reporting
    // it.
    crate::graph::traverse_with(
        &container.id,
        0,
        |id| match resolve(id) {
            Some(issue) if issue.id == container.id || !is_breakdown(issue) => {
                issue.dependencies.iter().collect()
            }
            _ => Vec::new(),
        },
        &resolve,
        |_| true,
    )
    .into_iter()
    .map(|issue| issue.id.clone())
    .chain(std::iter::once(container_id.clone()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity::IssueId;
    use crate::test_utils::issue_id;

    /// [`bracket_scope_ids`] of `container` over `issues`.
    fn scope_of(
        container: &IssueId,
        issues: &[Issue],
        breakdown_type: Option<&str>,
    ) -> std::collections::HashSet<IssueId> {
        let issues = IndexedIssues::new(issues.to_vec());
        bracket_scope_ids(container, |id| issues.get(id), breakdown_type)
    }

    use proptest::prelude::*;

    fn all_filter_issues() -> Vec<Issue> {
        (0..16)
            .map(|bits| {
                let mut issue = state_issue(
                    bits,
                    if bits & 1 == 0 {
                        State::Ready
                    } else {
                        State::Done
                    },
                );
                issue.assignee = Some(
                    if bits & 2 == 0 { "agent:a" } else { "agent:b" }
                        .parse()
                        .unwrap(),
                );
                issue.priority = if bits & 4 == 0 {
                    Priority::High
                } else {
                    Priority::Low
                };
                issue.labels = vec![if bits & 8 == 0 {
                    "component:api"
                } else {
                    "component:web"
                }
                .into()];
                issue
            })
            .collect()
    }

    fn assert_query_all_intersects(
        filter: AllFilter<'_>,
        first: impl Fn(&[Issue]) -> Vec<Issue>,
        second: impl Fn(&[Issue]) -> Vec<Issue>,
    ) {
        let issues = all_filter_issues();
        assert_eq!(
            ids_of(&query_all(&issues, filter)),
            ids_of(&second(&first(&issues))),
        );
    }

    #[test]
    fn test_query_all_composes_state_and_assignee() {
        let assignee: Assignee = "agent:a".parse().unwrap();
        assert_query_all_intersects(
            AllFilter {
                state: Some(State::Ready),
                assignee: Some(assignee.clone()),
                ..AllFilter::default()
            },
            |issues| query_by_state(issues, State::Ready),
            |issues| query_by_assignee(issues, &assignee),
        );
    }

    #[test]
    fn test_query_all_composes_state_and_priority() {
        assert_query_all_intersects(
            AllFilter {
                state: Some(State::Ready),
                priority: Some(Priority::High),
                ..AllFilter::default()
            },
            |issues| query_by_state(issues, State::Ready),
            |issues| query_by_priority(issues, Priority::High),
        );
    }

    #[test]
    fn test_query_all_composes_state_and_labels() {
        let labels = ["component:api".into()];
        assert_query_all_intersects(
            AllFilter {
                state: Some(State::Ready),
                labels: &labels,
                ..AllFilter::default()
            },
            |issues| query_by_state(issues, State::Ready),
            |issues| query_by_labels(issues, &labels),
        );
    }

    #[test]
    fn test_query_all_composes_assignee_and_priority() {
        let assignee: Assignee = "agent:a".parse().unwrap();
        assert_query_all_intersects(
            AllFilter {
                assignee: Some(assignee.clone()),
                priority: Some(Priority::High),
                ..AllFilter::default()
            },
            |issues| query_by_assignee(issues, &assignee),
            |issues| query_by_priority(issues, Priority::High),
        );
    }

    #[test]
    fn test_query_all_composes_assignee_and_labels() {
        let labels = ["component:api".into()];
        let assignee: Assignee = "agent:a".parse().unwrap();
        assert_query_all_intersects(
            AllFilter {
                assignee: Some(assignee.clone()),
                labels: &labels,
                ..AllFilter::default()
            },
            |issues| query_by_assignee(issues, &assignee),
            |issues| query_by_labels(issues, &labels),
        );
    }

    #[test]
    fn test_query_all_composes_priority_and_labels() {
        let labels = ["component:api".into()];
        assert_query_all_intersects(
            AllFilter {
                priority: Some(Priority::High),
                labels: &labels,
                ..AllFilter::default()
            },
            |issues| query_by_priority(issues, Priority::High),
            |issues| query_by_labels(issues, &labels),
        );
    }

    proptest! {
        #[test]
        fn test_query_all_returns_exactly_issues_admitted_by_every_filter(
            entries in proptest::collection::vec(
                (0..State::ALL.len(), 0usize..3, 0..Priority::ALL.len(), 0u8..8), 0..40,
            ),
            state_index in proptest::option::of(0..State::ALL.len()),
            assignee_index in proptest::option::of(0usize..3),
            priority_index in proptest::option::of(0..Priority::ALL.len()),
            label_indexes in proptest::collection::vec(0usize..5, 0..4),
        ) {
            let issue_labels = ["component:api", "component:web", "epic:auth"];
            let patterns = ["component:api", "component:*", "epic:auth", "epic:*", "component:absent"];
            let assignees = ["agent:a", "agent:b", "agent:absent"];
            let issues: Vec<_> = entries.into_iter().enumerate().map(|(index, (s, a, p, labels))| {
                let mut issue = state_issue(index, State::ALL[s]);
                issue.assignee = (a < 2).then(|| assignees[a].parse().unwrap());
                issue.priority = Priority::ALL[p];
                issue.labels = issue_labels.iter().enumerate()
                    .filter(|(bit, _)| labels & (1 << bit) != 0)
                    .map(|(_, label)| (*label).to_string()).collect();
                issue
            }).collect();
            let state = state_index.map(|index| State::ALL[index]);
            let assignee = assignee_index.map(|index| assignees[index].parse::<Assignee>().unwrap());
            let priority = priority_index.map(|index| Priority::ALL[index]);
            let labels: Vec<_> = label_indexes.into_iter().map(|index| patterns[index].into()).collect();
            let admitted = [
                state.map(|state| query_by_state(&issues, state)),
                assignee.as_ref().map(|assignee| query_by_assignee(&issues, assignee)),
                priority.map(|priority| query_by_priority(&issues, priority)),
                Some(query_by_labels(&issues, &labels)),
            ];
            let expected: Vec<_> = issues.iter()
                .filter(|issue| admitted.iter().flatten()
                    .all(|matches| matches.iter().any(|candidate| candidate.id == issue.id)))
                .map(|issue| &issue.id).collect();
            let actual = query_all(&issues, AllFilter { state, assignee, priority, labels: &labels });
            prop_assert_eq!(ids_of(&actual), expected);
        }
    }

    fn available_issue(index: usize, priority: Priority, labels: &[&str]) -> Issue {
        let mut issue = labeled_issue(&format!("Issue {index}"), labels);
        issue.id = issue_id(index as u128);
        issue.state = State::Ready;
        issue.priority = priority;
        issue
    }

    #[test]
    fn test_query_available_composes_each_priority_and_label_filter() {
        let priorities = [
            Priority::Critical,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ];
        let issues: Vec<_> = priorities
            .iter()
            .enumerate()
            .flat_map(|(index, &priority)| {
                [
                    available_issue(index * 2, priority, &["component:api", "epic:auth"]),
                    available_issue(index * 2 + 1, priority, &["component:web"]),
                ]
            })
            .collect();
        for priority in std::iter::once(None).chain(priorities.into_iter().map(Some)) {
            for patterns in [
                vec![],
                vec!["component:api"],
                vec!["component:*"],
                vec!["component:*", "epic:auth"],
                vec!["component:api", "component:api"],
                vec!["component:absent"],
            ] {
                let labels: Vec<_> = patterns.into_iter().map(String::from).collect();
                let ready = query_ready(&issues);
                let labeled = query_by_labels(&ready, &labels);
                let expected =
                    priority.map_or_else(|| labeled.clone(), |p| query_by_priority(&labeled, p));
                let actual = query_available(
                    &issues,
                    AvailableFilter {
                        priority,
                        labels: &labels,
                    },
                );
                assert_eq!(
                    actual.iter().map(|i| &i.id).collect::<Vec<_>>(),
                    expected.iter().map(|i| &i.id).collect::<Vec<_>>(),
                    "priority={priority:?}, labels={labels:?}"
                );
            }
        }
    }

    #[test]
    fn test_query_available_preserves_input_order_within_priority_ties() {
        let issues = vec![
            available_issue(9, Priority::High, &[]),
            available_issue(7, Priority::Low, &[]),
            available_issue(4, Priority::Critical, &[]),
            available_issue(2, Priority::High, &[]),
        ];
        let actual = query_available(&issues, AvailableFilter::default());
        assert_eq!(
            actual.iter().map(|i| &i.id).collect::<Vec<_>>(),
            [&issues[2].id, &issues[0].id, &issues[3].id, &issues[1].id]
        );
    }

    proptest! {
        #[test]
        fn test_query_available_stably_orders_the_filtered_ready_set(
            entries in proptest::collection::vec((0usize..4, any::<bool>(), any::<bool>()), 0..40),
            priority_index in proptest::option::of(0usize..4),
            require_label in any::<bool>(),
        ) {
            let priorities = [Priority::Critical, Priority::High, Priority::Normal, Priority::Low];
            let issues: Vec<_> = entries.iter().enumerate().map(|(index, &(rank, ready, labeled))| {
                let mut issue = available_issue(index, priorities[rank], if labeled { &["component:api"] } else { &[] });
                if !ready { issue.state = State::Backlog; }
                issue
            }).collect();
            let labels = if require_label { vec!["component:api".to_string()] } else { vec![] };
            let priority = priority_index.map(|index| priorities[index]);
            let ready = query_ready(&issues);
            let labeled = query_by_labels(&ready, &labels);
            // Concatenating priority bands preserves input ties without repeating the sort.
            let expected: Vec<_> = priorities.into_iter()
                .filter(|p| priority.is_none_or(|selected| selected == *p))
                .flat_map(|p| query_by_priority(&labeled, p).into_iter().map(|i| i.id))
                .collect();
            let actual = query_available(&issues, AvailableFilter { priority, labels: &labels });
            prop_assert_eq!(actual.into_iter().map(|i| i.id).collect::<Vec<_>>(), expected);
        }
    }

    #[test]
    fn test_build_issue_map() {
        // Create test issues
        let issue1 = crate::test_utils::fixture_issue("Task 1".to_string(), String::new());
        let issue2 = crate::test_utils::fixture_issue("Task 2".to_string(), String::new());
        let issue3 = crate::test_utils::fixture_issue("Task 3".to_string(), String::new());

        let issues = vec![issue1.clone(), issue2.clone(), issue3.clone()];

        // Build the map
        let map = build_issue_map(&issues);

        // Verify all issues are in the map
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&issue1.id).unwrap().id, issue1.id);
        assert_eq!(map.get(&issue2.id).unwrap().id, issue2.id);
        assert_eq!(map.get(&issue3.id).unwrap().id, issue3.id);

        // Verify we can look up by ID
        assert_eq!(map.get(&issue1.id).unwrap().title, "Task 1");
        assert_eq!(map.get(&issue2.id).unwrap().title, "Task 2");
        assert_eq!(map.get(&issue3.id).unwrap().title, "Task 3");
    }

    #[test]
    fn test_build_issue_map_empty() {
        let issues: Vec<Issue> = vec![];
        let map = build_issue_map(&issues);
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_build_issue_map_single() {
        let issue = crate::test_utils::fixture_issue("Single task".to_string(), String::new());
        let issues = vec![issue.clone()];
        let map = build_issue_map(&issues);

        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&issue.id).unwrap().title, "Single task");
    }

    fn labeled_issue(title: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.to_string(), String::new());
        issue.labels = labels.iter().map(|l| l.to_string()).collect();
        issue
    }

    #[test]
    fn test_query_by_labels_ands_multiple_patterns() {
        let both = labeled_issue("Both", &["epic:auth", "component:api"]);
        let epic_only = labeled_issue("EpicOnly", &["epic:auth"]);
        let issues = vec![both.clone(), epic_only];

        let result = query_by_labels(
            &issues,
            &["epic:auth".to_string(), "component:api".to_string()],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Both");
    }

    #[test]
    fn test_query_by_labels_supports_wildcards() {
        let issues = vec![
            labeled_issue("A", &["epic:auth", "component:api"]),
            labeled_issue("B", &["epic:billing", "component:api"]),
            labeled_issue("C", &["epic:auth", "component:web"]),
        ];

        // Wildcard on one pattern, exact match on the other: AND still applies.
        let result = query_by_labels(
            &issues,
            &["epic:*".to_string(), "component:api".to_string()],
        );
        let titles: Vec<&str> = result.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "B"]);
    }

    #[test]
    fn test_query_by_labels_empty_patterns_matches_all() {
        let issues = vec![labeled_issue("A", &["epic:auth"]), labeled_issue("B", &[])];

        assert_eq!(query_by_labels(&issues, &[]).len(), 2);
    }

    #[test]
    fn test_query_by_labels_no_match_returns_empty() {
        let issues = vec![labeled_issue("A", &["epic:auth", "component:api"])];

        let result = query_by_labels(
            &issues,
            &["epic:auth".to_string(), "component:web".to_string()],
        );

        assert!(result.is_empty());
    }

    #[test]
    fn test_query_by_labels_single_pattern_matches_query_by_label() {
        let issues = vec![
            labeled_issue("A", &["epic:auth"]),
            labeled_issue("B", &["epic:billing"]),
        ];

        let single = query_by_label(&issues, "epic:auth");
        let via_labels = query_by_labels(&issues, &["epic:auth".to_string()]);

        assert_eq!(single, via_labels);
    }

    /// Build an issue with the given id, type label, and dependency ids.
    fn scope_issue(id: IssueId, type_: &str, deps: &[IssueId]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(format!("issue {id}"), String::new());
        issue.id = id;
        issue.labels = vec![crate::labels::type_label(type_)];
        issue.dependencies = deps.to_vec();
        issue
    }

    /// A bracket spine `C -> impl1 -> impl2 -> B -> P` plus an upstream `U`
    /// that `P` depends on. Edges point from dependent to dependency (an issue
    /// depends on the ones before it close).
    fn bracket_spine() -> Vec<Issue> {
        vec![
            scope_issue(issue_id(200), "epic", &[issue_id(201)]),
            scope_issue(issue_id(201), "task", &[issue_id(202)]),
            scope_issue(issue_id(202), "task", &[issue_id(203)]),
            scope_issue(issue_id(203), "breakdown", &[issue_id(204)]),
            scope_issue(issue_id(204), "planning", &[issue_id(205)]),
            scope_issue(issue_id(205), "epic", &[]),
        ]
    }

    #[test]
    fn test_bracket_scope_includes_container_impl_and_breakdown() {
        let issues = bracket_spine();
        let ids = scope_of(&issue_id(200), &issues, Some("breakdown"));
        assert!(ids.contains(&issue_id(200)), "container is in scope");
        assert!(ids.contains(&issue_id(201)), "impl source is in scope");
        assert!(ids.contains(&issue_id(202)), "impl sink is in scope");
        assert!(
            ids.contains(&issue_id(203)),
            "breakdown node B is in scope (its rule fires)"
        );
    }

    #[test]
    fn test_bracket_scope_stops_at_breakdown_excludes_plan_and_upstream() {
        let issues = bracket_spine();
        let ids = scope_of(&issue_id(200), &issues, Some("breakdown"));
        assert!(
            !ids.contains(&issue_id(204)),
            "walk halts at B; plan P is out of scope"
        );
        assert!(
            !ids.contains(&issue_id(205)),
            "upstream beyond B is out of scope"
        );
        assert_eq!(ids.len(), 4, "exactly C + 2 impl + B");
    }

    #[test]
    fn test_bracket_scope_boundary_type_is_caller_supplied() {
        // The boundary is whatever breakdown-node type the caller passes — here a
        // CUSTOM type `synthesis`, not the literal "breakdown". The walk must
        // halt at the `type:synthesis` node and exclude its upstream.
        let issues = vec![
            scope_issue(issue_id(200), "goal", &[issue_id(201)]),
            scope_issue(issue_id(201), "task", &[issue_id(206)]),
            scope_issue(issue_id(206), "synthesis", &[issue_id(204)]),
            scope_issue(issue_id(204), "planning", &[issue_id(205)]),
            scope_issue(issue_id(205), "goal", &[]),
        ];
        let ids = scope_of(&issue_id(200), &issues, Some("synthesis"));
        assert!(ids.contains(&issue_id(200)), "container is in scope");
        assert!(ids.contains(&issue_id(201)), "impl is in scope");
        assert!(
            ids.contains(&issue_id(206)),
            "the custom-named boundary node is in scope"
        );
        assert!(
            !ids.contains(&issue_id(204)),
            "walk halts at the synthesis boundary; plan is out of scope"
        );
        assert!(
            !ids.contains(&issue_id(205)),
            "upstream beyond the synthesis boundary is out of scope"
        );
        assert_eq!(ids.len(), 3, "exactly C + impl + synthesis boundary");
    }

    #[test]
    fn test_bracket_scope_no_boundary_walks_full_closure() {
        // With no boundary type (breakdown_type = None — a non-bracketed
        // container) there is no bracket boundary, so the walk is the full
        // transitive closure: even a node that WOULD be a boundary under a
        // configured type does not halt it.
        let issues = bracket_spine();
        let ids = scope_of(&issue_id(200), &issues, None);
        assert!(
            ids.contains(&issue_id(203))
                && ids.contains(&issue_id(204))
                && ids.contains(&issue_id(205)),
            "without a configured boundary the walk descends the whole closure: {ids:?}"
        );
        assert_eq!(ids.len(), 6, "all six spine nodes are reachable");
    }

    #[test]
    fn test_bracket_scope_lone_container_is_just_itself() {
        let issues = vec![scope_issue(issue_id(200), "epic", &[])];
        let ids = scope_of(&issue_id(200), &issues, Some("breakdown"));
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&issue_id(200)));
    }

    #[test]
    fn test_query_blocked_dependency_reason_is_typed() {
        let mut dep = crate::test_utils::fixture_issue("Upstream".to_string(), String::new());
        dep.id = crate::test_utils::issue_id(63);
        dep.state = State::InProgress;

        let mut blocked = crate::test_utils::fixture_issue("Downstream".to_string(), String::new());
        blocked.id = crate::test_utils::issue_id(64);
        blocked.state = State::Backlog;
        blocked.dependencies = vec![issue_id(63)];

        let issues = vec![dep, blocked];
        let result = query_blocked(&issues);

        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].1,
            vec![BlockingReason::Dependency {
                id: issue_id(63),
                title: "Upstream".to_string(),
                state: State::InProgress,
            }]
        );
        // Display renders the canonical reason string.
        assert_eq!(
            result[0].1[0].to_string(),
            format!("dependency:{} (Upstream:in_progress)", issue_id(63))
        );
    }

    #[test]
    fn test_query_blocked_omits_done_dependency_from_reasons() {
        let done = state_issue(0, State::Done);
        let open = state_issue(1, State::InProgress);
        let blocked = dependent_of(2, &[&done, &open]);
        let reason = BlockingReason::Dependency {
            id: open.id.clone(),
            title: open.title.clone(),
            state: State::InProgress,
        };

        let result = query_blocked(&[done, open, blocked]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, [reason]);
    }

    #[test]
    fn test_query_blocked_omits_rejected_dependency_from_reasons() {
        // A Rejected dependency is met: only the in_progress one is a reason.
        let mut rejected = crate::test_utils::fixture_issue("Abandoned".to_string(), String::new());
        rejected.id = crate::test_utils::issue_id(63);
        rejected.state = State::Rejected;

        let mut pending = crate::test_utils::fixture_issue("Upstream".to_string(), String::new());
        pending.id = crate::test_utils::issue_id(65);
        pending.state = State::InProgress;

        let mut blocked = crate::test_utils::fixture_issue("Downstream".to_string(), String::new());
        blocked.id = crate::test_utils::issue_id(64);
        blocked.state = State::Backlog;
        blocked.dependencies = vec![issue_id(63), issue_id(65)];

        let issues = vec![rejected, pending, blocked];
        let result = query_blocked(&issues);

        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].1,
            vec![BlockingReason::Dependency {
                id: issue_id(65),
                title: "Upstream".to_string(),
                state: State::InProgress,
            }]
        );
    }

    #[test]
    fn test_query_blocked_gate_reason_is_typed_and_defaults_pending() {
        use crate::domain::{GateState, GateStatus};

        // A Backlog issue with an unmet dependency keeps it blocked; a required
        // gate with no recorded status contributes a Pending gate reason.
        let mut dep = crate::test_utils::fixture_issue("Upstream".to_string(), String::new());
        dep.id = crate::test_utils::issue_id(63);
        dep.state = State::InProgress;

        let mut blocked = crate::test_utils::fixture_issue("Downstream".to_string(), String::new());
        blocked.id = crate::test_utils::issue_id(64);
        blocked.state = State::Backlog;
        blocked.dependencies = vec![issue_id(63)];
        blocked.gates_required = vec!["never-run".parse().unwrap(), "failed".parse().unwrap()];
        blocked.gates_status.insert(
            "failed".parse().unwrap(),
            GateState {
                status: GateStatus::Failed,
                updated_by: None,
                updated_at: chrono::Utc::now(),
                run_id: None,
            },
        );

        let issues = vec![dep, blocked];
        let result = query_blocked(&issues);

        assert_eq!(result.len(), 1);
        let reasons = &result[0].1;
        assert!(reasons.contains(&BlockingReason::Gate {
            key: "never-run".parse().unwrap(),
            status: GateStatus::Pending,
        }));
        assert!(reasons.contains(&BlockingReason::Gate {
            key: "failed".parse().unwrap(),
            status: GateStatus::Failed,
        }));
        assert!(reasons
            .iter()
            .any(|r| r.to_string() == format!("gate:never-run ({})", GateStatus::Pending)));
        assert!(reasons
            .iter()
            .any(|r| r.to_string() == format!("gate:failed ({})", GateStatus::Failed)));
    }

    #[test]
    fn test_bracket_scope_ignores_dangling_dependency() {
        // Container depends on an id absent from the store; the walk must not
        // panic and must not include the missing id.
        let issues = vec![scope_issue(issue_id(200), "epic", &[issue_id(207)])];
        let ids = scope_of(&issue_id(200), &issues, Some("breakdown"));
        assert_eq!(ids, std::collections::HashSet::from([issue_id(200)]));
    }

    fn ids_of(issues: &[Issue]) -> Vec<&IssueId> {
        issues.iter().map(|i| &i.id).collect()
    }

    fn state_issue(index: usize, state: State) -> Issue {
        let mut issue = labeled_issue(&format!("Issue {index}"), &[]);
        issue.id = issue_id(index as u128);
        issue.state = state;
        issue
    }

    #[test]
    fn test_queries_over_no_issues_return_empty() {
        let patterns = ["epic:auth".to_string()];
        assert!(query_ready(&[]).is_empty());
        assert!(query_blocked(&[]).is_empty());
        assert!(query_closed(&[]).is_empty());
        assert!(query_by_state(&[], State::Ready).is_empty());
        assert!(query_by_priority(&[], Priority::High).is_empty());
        assert!(query_by_assignee(&[], &"agent:a".parse().unwrap()).is_empty());
        assert!(query_by_label(&[], "epic:auth").is_empty());
        assert!(query_by_labels(&[], &patterns).is_empty());
        assert!(query_strategic(&[], &["epic".to_string()]).is_empty());
        assert!(query_available(&[], AvailableFilter::default()).is_empty());
    }

    #[test]
    fn test_query_ready_includes_ready_issue_with_unpassed_required_gates() {
        let mut issue = state_issue(0, State::Ready);
        issue.gates_required = vec!["tests".parse().unwrap()];
        assert_eq!(
            ids_of(&query_ready(std::slice::from_ref(&issue))),
            [&issue.id]
        );
    }

    #[test]
    fn test_query_ready_requires_ready_state_unassigned_and_met_dependencies() {
        let mut done_dep = state_issue(100, State::Done);
        done_dep.id = crate::test_utils::issue_id(66);
        let mut open_dep = state_issue(101, State::InProgress);
        open_dep.id = crate::test_utils::issue_id(67);
        let mut issues: Vec<Issue> = State::ALL
            .into_iter()
            .enumerate()
            .map(|(index, state)| state_issue(index, state))
            .collect();
        let mut assigned = state_issue(50, State::Ready);
        assigned.assignee = Some("agent:a".parse().unwrap());
        let mut after_done = state_issue(51, State::Ready);
        after_done.dependencies = vec![issue_id(66)];
        let mut after_open = state_issue(52, State::Ready);
        after_open.dependencies = vec![issue_id(67)];
        issues.extend([done_dep, open_dep, assigned, after_done, after_open]);

        let ready_ids = ids_of(&query_ready(&issues))
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        let ready_state_index = State::ALL
            .into_iter()
            .position(|state| state == State::Ready)
            .unwrap();
        assert_eq!(
            ready_ids,
            [issue_id(ready_state_index as u128), issue_id(51)]
        );
    }

    proptest! {
        #[test]
        fn test_query_by_state_selects_exactly_issues_in_that_state(
            indexes in proptest::collection::vec(0..State::ALL.len(), 0..30),
        ) {
            let issues: Vec<_> = indexes.iter().enumerate()
                .map(|(i, &s)| state_issue(i, State::ALL[s])).collect();
            let total: usize = State::ALL.into_iter().map(|state| {
                let selected = query_by_state(&issues, state);
                prop_assert!(selected.iter().all(|i| i.state == state));
                Ok(selected.len())
            }).collect::<Result<Vec<_>, TestCaseError>>()?.into_iter().sum();
            prop_assert_eq!(total, issues.len());
        }

        #[test]
        fn test_query_by_priority_selects_exactly_issues_at_that_priority(
            indexes in proptest::collection::vec(0..Priority::ALL.len(), 0..30),
        ) {
            let issues: Vec<_> = indexes.iter().enumerate().map(|(i, &p)| {
                let mut issue = state_issue(i, State::Backlog);
                issue.priority = Priority::ALL[p];
                issue
            }).collect();
            let total: usize = Priority::ALL.into_iter().map(|priority| {
                let selected = query_by_priority(&issues, priority);
                prop_assert!(selected.iter().all(|i| i.priority == priority));
                Ok(selected.len())
            }).collect::<Result<Vec<_>, TestCaseError>>()?.into_iter().sum();
            prop_assert_eq!(total, issues.len());
        }
    }

    #[test]
    fn test_query_by_assignee_matches_exact_assignee_only() {
        let assigned = |index: usize, assignee: Option<Assignee>| {
            let mut issue = state_issue(index, State::Ready);
            issue.assignee = assignee;
            issue
        };
        let issues = vec![
            assigned(0, Some("agent:a".parse().unwrap())),
            assigned(1, Some("agent:b".parse().unwrap())),
            assigned(2, Some("agent:a".parse().unwrap())),
            assigned(3, None),
        ];

        let assignee_a: Assignee = "agent:a".parse().unwrap();
        let assignee_b: Assignee = "agent:b".parse().unwrap();
        let assignee_c: Assignee = "agent:c".parse().unwrap();
        assert_eq!(
            ids_of(&query_by_assignee(&issues, &assignee_a)),
            [&issues[0].id, &issues[2].id]
        );
        assert_eq!(
            ids_of(&query_by_assignee(&issues, &assignee_b)),
            [&issues[1].id]
        );
        assert!(query_by_assignee(&issues, &assignee_c).is_empty());
    }

    #[test]
    fn test_query_closed_selects_effectively_terminal_issues() {
        let archived = |index: usize, from: Option<State>| {
            let mut issue = state_issue(index, State::Archived);
            issue.archived_from = from;
            issue
        };
        let issues = vec![
            state_issue(0, State::Done),
            state_issue(1, State::Rejected),
            state_issue(2, State::Backlog),
            state_issue(3, State::InProgress),
            archived(4, Some(State::Done)),
            archived(5, Some(State::Backlog)),
            archived(6, None),
        ];

        assert_eq!(
            ids_of(&query_closed(&issues)),
            [&issues[0].id, &issues[1].id, &issues[4].id]
        );
    }

    #[test]
    fn test_query_by_label_matches_whole_label_exactly() {
        let issues = vec![
            labeled_issue("Mixed", &["component:AuthService", "epic:auth"]),
            labeled_issue("Rule", &["enforces:@/rule/label-format"]),
            labeled_issue("Other rule", &["enforces:@/rule/other-rule"]),
        ];
        let titles = |pattern: &str| {
            query_by_label(&issues, pattern)
                .into_iter()
                .map(|i| i.title)
                .collect::<Vec<_>>()
        };

        assert_eq!(titles("component:AuthService"), ["Mixed"]);
        assert!(titles("component:authservice").is_empty());
        assert!(titles("component:Auth").is_empty());
        assert_eq!(titles("epic:auth"), ["Mixed"]);
        assert_eq!(titles("enforces:@/rule/label-format"), ["Rule"]);
        assert!(titles("enforces:@/rule").is_empty());
    }

    #[test]
    fn test_query_by_label_namespace_wildcard_matches_any_value_in_that_namespace() {
        let issues = vec![
            labeled_issue("A", &["epic:auth"]),
            labeled_issue("B", &["epic:billing", "component:api"]),
            labeled_issue("C", &["component:api"]),
            labeled_issue("D", &["epicx:auth"]),
            labeled_issue("E", &[]),
        ];
        let titles = |pattern: &str| {
            query_by_label(&issues, pattern)
                .into_iter()
                .map(|i| i.title)
                .collect::<Vec<_>>()
        };

        assert_eq!(titles("epic:*"), ["A", "B"]);
        assert_eq!(titles("component:*"), ["B", "C"]);
        assert!(titles("milestone:*").is_empty());
    }

    #[test]
    fn test_query_strategic_selects_issues_of_any_declared_strategic_type() {
        let issues = vec![
            labeled_issue("Milestone", &["type:milestone"]),
            labeled_issue("Epic", &["type:epic", "epic:auth"]),
            labeled_issue("Task", &["type:task"]),
            labeled_issue("Untyped", &[]),
        ];
        let titles = |strategic: &[&str]| {
            let types: Vec<String> = strategic.iter().map(|t| t.to_string()).collect();
            query_strategic(&issues, &types)
                .into_iter()
                .map(|i| i.title)
                .collect::<Vec<_>>()
        };

        assert_eq!(titles(&["milestone", "epic"]), ["Milestone", "Epic"]);
        assert_eq!(titles(&["epic"]), ["Epic"]);
        assert!(titles(&[]).is_empty());
        assert!(titles(&["story"]).is_empty());
    }

    #[test]
    fn test_query_strategic_ignores_membership_label_without_strategic_type() {
        let issues = vec![
            labeled_issue("Member", &["epic:auth", "type:task"]),
            labeled_issue("Container", &["type:epic"]),
        ];

        let result = query_strategic(&issues, &["epic".to_string()]);

        assert_eq!(
            result.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["Container"]
        );
    }

    fn dependent_of(index: usize, dependencies: &[&Issue]) -> Issue {
        let mut issue = state_issue(index, State::Backlog);
        issue.dependencies = dependencies.iter().map(|issue| issue.id.clone()).collect();
        issue
    }

    fn affected_corrections(
        before: &[Issue],
        after: &[Issue],
    ) -> Vec<(IssueId, ReadinessCorrection)> {
        derive_readiness_corrections(before, after, ReadinessScope::Affected)
    }

    fn with_state(issue: &Issue, state: State) -> Issue {
        Issue {
            state,
            ..issue.clone()
        }
    }

    #[test]
    fn test_derive_readiness_corrections_promotes_a_dependent_whose_blocker_reaches_done_or_rejected(
    ) {
        for terminal in [State::Done, State::Rejected] {
            let blocker = state_issue(0, State::InProgress);
            let dependent = dependent_of(1, &[&blocker]);
            let before = [blocker.clone(), dependent.clone()];
            let after = [with_state(&blocker, terminal), dependent.clone()];
            assert_eq!(
                affected_corrections(&before, &after),
                [(dependent.id.clone(), ReadinessCorrection::Promote)],
                "{terminal:?}"
            );
        }
    }

    #[test]
    fn test_derive_readiness_corrections_demotes_a_dependent_whose_blocker_leaves_a_terminal_state()
    {
        let blocker = state_issue(0, State::Done);
        let dependent = with_state(&dependent_of(1, &[&blocker]), State::Ready);
        let before = [blocker.clone(), dependent.clone()];
        let after = [with_state(&blocker, State::InProgress), dependent.clone()];
        assert_eq!(
            affected_corrections(&before, &after),
            [(dependent.id, ReadinessCorrection::Demote)]
        );
    }

    #[test]
    fn test_derive_readiness_corrections_demotes_the_source_of_an_added_edge() {
        let blocker = state_issue(0, State::InProgress);
        let source = state_issue(1, State::Ready);
        let before = [blocker.clone(), source.clone()];
        let after = [
            blocker.clone(),
            Issue {
                dependencies: vec![blocker.id.clone()],
                ..source.clone()
            },
        ];
        assert_eq!(
            affected_corrections(&before, &after),
            [(source.id, ReadinessCorrection::Demote)]
        );
    }

    #[test]
    fn test_derive_readiness_corrections_promotes_the_source_of_a_removed_edge() {
        let blocker = state_issue(0, State::InProgress);
        let source = dependent_of(1, &[&blocker]);
        let before = [blocker.clone(), source.clone()];
        let after = [blocker, dependent_of(1, &[])];
        assert_eq!(
            affected_corrections(&before, &after),
            [(source.id, ReadinessCorrection::Promote)]
        );
    }

    #[test]
    fn test_derive_readiness_corrections_promotes_the_dependent_of_a_deleted_blocker() {
        let blocker = state_issue(0, State::InProgress);
        let dependent = dependent_of(1, &[&blocker]);
        let before = [blocker, dependent.clone()];
        let after = [dependent_of(1, &[])];
        assert_eq!(
            affected_corrections(&before, &after),
            [(dependent.id, ReadinessCorrection::Promote)]
        );
    }

    #[test]
    fn test_derive_readiness_corrections_leaves_dependents_outside_backlog_and_ready() {
        let blocker = state_issue(0, State::InProgress);
        let dependents = [
            (State::InProgress, None),
            (State::Gated, None),
            (State::Done, None),
            (State::Rejected, None),
            (State::Archived, Some(State::Backlog)),
            (State::Archived, Some(State::Done)),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (state, archived_from))| Issue {
            archived_from,
            ..with_state(&dependent_of(index + 1, &[&blocker]), state)
        })
        .collect::<Vec<_>>();
        let before = std::iter::once(blocker.clone())
            .chain(dependents.iter().cloned())
            .collect::<Vec<_>>();
        let after = std::iter::once(with_state(&blocker, State::Done))
            .chain(dependents)
            .collect::<Vec<_>>();
        assert!(affected_corrections(&before, &after).is_empty());
    }

    #[test]
    fn test_derive_readiness_corrections_affected_scope_excludes_drift_the_all_scope_reports() {
        let drift = state_issue(0, State::Backlog);
        let issues = [drift.clone()];
        assert!(affected_corrections(&issues, &issues).is_empty());
        assert_eq!(
            derive_readiness_corrections(&issues, &issues, ReadinessScope::All),
            [(drift.id, ReadinessCorrection::Promote)]
        );
    }

    /// A repository whose stored readiness agrees with its acyclic graph: each
    /// issue depends only on lower-indexed issues.
    fn coherent_issues(nodes: &[(usize, usize, Vec<usize>)]) -> Vec<Issue> {
        let issues = nodes
            .iter()
            .enumerate()
            .map(|(index, (state, archived_from, dependencies))| {
                let mut issue = state_issue(index, State::ALL[*state]);
                issue.archived_from =
                    (issue.state == State::Archived).then_some(State::ALL[*archived_from]);
                issue.dependencies = dependencies
                    .iter()
                    .filter(|_| index > 0)
                    .map(|dependency| issue_id((dependency % index.max(1)) as u128))
                    .collect();
                issue.dependencies.sort();
                issue.dependencies.dedup();
                issue
            })
            .collect::<Vec<_>>();
        let resolved = build_issue_map(&issues);
        let blocked = issues
            .iter()
            .map(|issue| issue.is_blocked(&resolved))
            .collect::<Vec<_>>();
        issues
            .into_iter()
            .zip(blocked)
            .map(|(mut issue, blocked)| {
                if matches!(issue.state, State::Backlog | State::Ready) {
                    issue.state = if blocked {
                        State::Backlog
                    } else {
                        State::Ready
                    };
                }
                issue
            })
            .collect()
    }

    /// Apply one edit to `issues`, returning the edited set and the issues the
    /// edit can reach: the edited source of an edge edit, or the direct
    /// dependents of a changed or deleted issue.
    fn edit_issues(
        mut issues: Vec<Issue>,
        (kind, first, second, state, cleans): (u8, usize, usize, usize, bool),
    ) -> (Vec<Issue>, Vec<IssueId>) {
        let count = issues.len();
        let target = issues[first % count].id.clone();
        let dependents = issues
            .iter()
            .filter(|issue| issue.dependencies.contains(&target))
            .map(|issue| issue.id.clone())
            .collect::<Vec<_>>();
        match kind {
            0 => {
                let resolved_blocked = {
                    let resolved = build_issue_map(&issues);
                    issues[first % count].is_blocked(&resolved)
                };
                let issue = &mut issues[first % count];
                let previous = issue.state;
                issue.state = match State::ALL[state] {
                    State::Backlog | State::Ready if resolved_blocked => State::Backlog,
                    State::Backlog | State::Ready => State::Ready,
                    other => other,
                };
                issue.archived_from = (issue.state == State::Archived).then_some(previous);
                (issues, dependents)
            }
            1 => {
                let dependency = issues[second % count].id.clone();
                if second % count < first % count {
                    let issue = &mut issues[first % count];
                    issue.dependencies.push(dependency);
                    issue.dependencies.sort();
                    issue.dependencies.dedup();
                }
                (issues, vec![target])
            }
            2 => {
                let issue = &mut issues[first % count];
                if !issue.dependencies.is_empty() {
                    let removed = second % issue.dependencies.len();
                    issue.dependencies.remove(removed);
                }
                (issues, vec![target])
            }
            _ => {
                issues.retain(|issue| issue.id != target);
                if cleans {
                    issues
                        .iter_mut()
                        .for_each(|issue| issue.dependencies.retain(|id| id != &target));
                }
                (issues, dependents)
            }
        }
    }

    /// The ids `start` reaches in `issues` along `direction`, by fixpoint.
    fn reference_closure<'a>(
        issues: &'a [Issue],
        start: &IssueId,
        direction: Direction,
    ) -> BTreeSet<&'a IssueId> {
        let linked = |from: &IssueId, to: &Issue| match direction {
            Direction::Dependencies => issues
                .iter()
                .any(|issue| &issue.id == from && issue.dependencies.contains(&to.id)),
            Direction::Dependents => to.dependencies.contains(from),
        };
        let mut reached: BTreeSet<&'a IssueId> = BTreeSet::new();
        loop {
            let next = issues
                .iter()
                .filter(|issue| &issue.id != start)
                .filter(|issue| {
                    linked(start, issue) || reached.iter().any(|from| linked(from, issue))
                })
                .map(|issue| &issue.id)
                .collect::<BTreeSet<_>>();
            if next == reached {
                return reached;
            }
            reached = next;
        }
    }

    proptest! {
        #[test]
        fn test_issue_index_read_through_replacements_agrees_with_the_records_it_stands_for(
            nodes in proptest::collection::vec(
                (
                    0..State::ALL.len(),
                    0..State::ALL.len(),
                    proptest::collection::vec(0usize..16, 0..4),
                ),
                1..12,
            ),
            edits in proptest::collection::vec(
                (0u8..3, 0usize..16, 0usize..16, 0..State::ALL.len(), any::<bool>()),
                1..4,
            ),
            relabels in proptest::collection::vec(0usize..4, 12),
        ) {
            let label = |n: usize| format!("ns{}:v{}", n % 2, n % 3);
            let before = coherent_issues(&nodes)
                .into_iter()
                .enumerate()
                .map(|(index, issue)| Issue { labels: vec![label(index)], ..issue })
                .collect::<Vec<_>>();
            let after = edits
                .into_iter()
                .fold(before.clone(), |issues, edit| edit_issues(issues, edit).0)
                .into_iter()
                .zip(&relabels)
                .map(|(issue, &relabel)| match relabel {
                    0 => Issue { labels: vec![label(relabel + 4)], ..issue },
                    _ => issue,
                })
                .collect::<Vec<_>>();
            let prior = build_issue_map(&before);
            let replaced = after
                .iter()
                .filter(|issue| prior.get(&issue.id) != Some(issue))
                .cloned()
                .collect::<Vec<_>>();
            // Each replaced id first takes a stale record, depending on every
            // other issue and carrying another label, that its final record
            // supersedes.
            let history = replaced
                .iter()
                .flat_map(|issue| {
                    let stale = Issue {
                        dependencies: before
                            .iter()
                            .map(|other| other.id.clone())
                            .filter(|id| id != &issue.id)
                            .collect(),
                        labels: vec![label(5)],
                        ..issue.clone()
                    };
                    [stale, issue.clone()]
                })
                .collect::<Vec<_>>();
            let base = IssueIndex::new(&before);
            let mut owned = IssueIndex::new(&before);
            history
                .iter()
                .for_each(|issue| owned.replace(issue.clone()));
            let expected =
                derive_readiness_corrections(&before, &after, ReadinessScope::Affected);

            for view in [base.with(&history), owned.view()] {
                prop_assert_eq!(view.issues().collect::<Vec<_>>(), after.iter().collect::<Vec<_>>());
                for target in &before {
                    let dependents = after
                        .iter()
                        .filter(|issue| issue.dependencies.contains(&target.id))
                        .map(|issue| &issue.id)
                        .collect::<BTreeSet<_>>();
                    prop_assert_eq!(view.dependents(&target.id), dependents);
                    for direction in [Direction::Dependencies, Direction::Dependents] {
                        let reached = view
                            .reach(&target.id, direction, |_| true)
                            .into_iter()
                            .map(|issue| &issue.id)
                            .collect::<BTreeSet<_>>();
                        prop_assert_eq!(reached, reference_closure(&after, &target.id, direction));
                    }
                    for (namespace, value) in [("ns0", None), ("ns1", Some("v1")), ("ns0", Some("v2"))] {
                        let carriers = after
                            .iter()
                            .filter(|issue| {
                                issue.labels.iter().any(|label| {
                                    label.split_once(':').is_some_and(|(key, held)| {
                                        key == namespace && value.is_none_or(|value| held == value)
                                    })
                                })
                            })
                            .map(|issue| &issue.id)
                            .collect::<BTreeSet<_>>();
                        prop_assert_eq!(
                            view.labelled(namespace, value)
                                .into_iter()
                                .map(|issue| &issue.id)
                                .collect::<BTreeSet<_>>(),
                            carriers
                        );
                    }
                    let prefix = IssueIdPrefix::try_from(&target.id.as_str()[..10]).unwrap();
                    let named = after
                        .iter()
                        .filter(|issue| prefix.matches(&issue.id))
                        .map(|issue| &issue.id)
                        .collect::<BTreeSet<_>>();
                    prop_assert_eq!(
                        view.resolve_prefix(&prefix)
                            .into_iter()
                            .map(|issue| &issue.id)
                            .collect::<BTreeSet<_>>(),
                        named
                    );
                }
                let changes = replaced
                    .iter()
                    .map(|issue| (&issue.id, prior.get(&issue.id).copied()));
                prop_assert_eq!(
                    &derive_changed_readiness_corrections(view, changes),
                    &expected
                );
            }
        }

        #[test]
        fn test_derive_readiness_corrections_affected_scope_restores_coherence_after_one_edit(
            nodes in proptest::collection::vec(
                (
                    0..State::ALL.len(),
                    0..State::ALL.len(),
                    proptest::collection::vec(0usize..16, 0..4),
                ),
                1..12,
            ),
            edit in (0u8..4, 0usize..16, 0usize..16, 0..State::ALL.len(), any::<bool>()),
        ) {
            let before = coherent_issues(&nodes);
            let (after, reachable) = edit_issues(before.clone(), edit);

            let corrections = affected_corrections(&before, &after);
            prop_assert!(corrections.iter().all(|(id, _)| reachable.contains(id)));

            let corrected = after
                .into_iter()
                .map(|issue| match corrections.iter().find(|(id, _)| id == &issue.id) {
                    Some((_, ReadinessCorrection::Promote)) => with_state(&issue, State::Ready),
                    Some((_, ReadinessCorrection::Demote)) => with_state(&issue, State::Backlog),
                    None => issue,
                })
                .collect::<Vec<_>>();
            prop_assert!(
                derive_readiness_corrections(&corrected, &corrected, ReadinessScope::All)
                    .is_empty()
            );
        }
    }
}
