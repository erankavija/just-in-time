//! Dependency graph operations

use super::*;
use crate::errors::{DependencyBatchRejectedError, RedundantDependencyError};
use crate::repository_state::{
    finalize, CaptureBudget, CaptureSpec, MutationContext, MutationIntent, RepositoryEntry,
    RepositoryImage, VirtualPath,
};
use crate::storage::{
    AmbiguousIdError, InvalidIdPrefixError, IssueNotFoundError, RepositoryStateStoreError,
    MIN_ID_PREFIX_LENGTH,
};
use std::collections::{BTreeSet, HashSet};

const DEPENDENCY_CAPTURE_BUDGET: CaptureBudget = CaptureBudget {
    max_paths: 1 << 16,
    max_listings: 1,
    max_bytes: 512 * 1024 * 1024,
    max_depth: 8,
};

#[derive(Clone)]
enum CapturedDependencyMutation {
    Single {
        issue_id: String,
        dependency_id: String,
        policy: RedundancyPolicy,
    },
    Batch {
        issue_id: String,
        dependency_ids: Vec<String>,
        policy: RedundancyPolicy,
    },
    ReduceAll {
        dry_run: bool,
    },
}

enum CapturedDependencyOutcome {
    Single {
        result: DependencyAddResult,
        warning: Option<String>,
    },
    Batch(DependenciesAddResult),
    Reduced {
        count: usize,
        messages: Vec<String>,
    },
}

struct DerivedDependencyMutation {
    outcome: CapturedDependencyOutcome,
    intents: Vec<MutationIntent>,
}

/// Result of adding multiple dependencies.
///
/// All-or-nothing (jit:c8518f2a): [`add_dependencies_with_policy`](CommandExecutor::add_dependencies_with_policy)
/// returns this only when every requested edge validated. A batch with any
/// rejected edge instead returns `Err(`[`DependencyBatchRejectedError`]`)` and
/// leaves the dependency set — and the event log — completely unchanged, so
/// there is no per-batch `errors` field to populate here.
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>, // (id, reason)
}

/// Result of removing multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Add a dependency to an issue, dropping any now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependency_with_policy`](Self::add_dependency_with_policy) with
    /// [`RedundancyPolicy::Reduce`]: the resulting graph is left acyclic and
    /// transitively reduced, which internal callers (templates, breakdown,
    /// batch-create) rely on. Returns `(result, warnings)` where `warnings`
    /// carries any lease warning.
    pub fn add_dependency(
        &self,
        issue_id: &str,
        dep_id: &str,
    ) -> Result<(DependencyAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_dependency_with_policy(issue_id, dep_id, RedundancyPolicy::Reduce)
    }

    /// Add a dependency under an explicit transitive-reduction [`RedundancyPolicy`].
    ///
    /// Cycle detection runs first (the existing write-time guard,
    /// @/inv/dag-acyclic). The candidate graph (the current issues plus the new
    /// edge) is then checked for transitive-reduction violations with the same
    /// [`DependencyGraph::find_redundant_edges`] property `jit validate` enforces:
    ///
    /// * [`RedundancyPolicy::Reject`] — if the edge shadows an existing direct
    ///   edge or is itself already reachable, the add fails with a
    ///   [`RedundantDependencyError`](crate::errors::RedundantDependencyError)
    ///   naming the offending edge pair; nothing is written.
    /// * [`RedundancyPolicy::Reduce`] — the edge is added and every now-redundant
    ///   edge is dropped in the same operation, leaving the graph reduced.
    ///
    /// A non-redundant edge is added normally under either policy.
    pub fn add_dependency_with_policy(
        &self,
        issue_id: &str,
        dep_id: &str,
        policy: RedundancyPolicy,
    ) -> Result<(DependencyAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::Single {
            issue_id: issue_id.to_string(),
            dependency_id: dep_id.to_string(),
            policy,
        })? {
            CapturedDependencyOutcome::Single { result, warning } => {
                Ok((result, warning.into_iter().collect()))
            }
            _ => unreachable!("single dependency request returns a single outcome"),
        }
    }

    /// Remove a dependency from an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let full_dep_id = self.storage.resolve_issue_id(dep_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_issue_id)? {
            warnings.push(warning);
        }

        let mut issue = self.storage.load_issue(&full_issue_id)?;
        let before = issue.dependencies.len();
        issue.dependencies.retain(|d| d != &full_dep_id);
        let removed = issue.dependencies.len() != before;

        // Only persist (and event-log) when an edge was actually removed: a no-op
        // removal (edge absent) must not bump `updated_at` or emit an event, the
        // same no-change contract as `update_issue`. Removing an edge can unblock
        // the issue, so the readiness check runs only on the real-change path.
        if removed {
            let event = Event::draft_issue_updated(
                full_issue_id.clone(),
                "dependency-remove".to_string(),
                vec!["dependencies".to_string()],
            );
            self.publish_ambient_issue_mutation(vec![issue], vec![(1, event)])?;
            self.auto_transition_to_ready(&full_issue_id)?;
        }

        Ok(warnings)
    }

    /// Add multiple dependencies to an issue, dropping now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependencies_with_policy`](Self::add_dependencies_with_policy) with
    /// [`RedundancyPolicy::Reduce`], preserving the eager-reduction behavior
    /// internal callers rely on.
    pub fn add_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_dependencies_with_policy(issue_id, dep_ids, RedundancyPolicy::Reduce)
    }

    /// Add multiple dependencies under an explicit [`RedundancyPolicy`], atomically.
    ///
    /// All-or-nothing (jit:c8518f2a): every requested edge is validated —
    /// id resolution, then cycle detection and (under
    /// [`RedundancyPolicy::Reject`]) the transitive-reduction check, all against
    /// the WOULD-BE final graph with every edge of this call applied at once —
    /// before anything is written. If any edge fails, `Err(`[`DependencyBatchRejectedError`]`)`
    /// names every rejected edge (REQ-02) and the dependency set, and the event
    /// log, are left completely unchanged (REQ-01, REQ-03). Only when every
    /// edge validates are the surviving edges applied and event-logged in one
    /// pass.
    ///
    /// Validating the full candidate graph up front (rather than one edge at a
    /// time against the graph as it stood before this call) also catches a
    /// violation that only emerges from the COMBINATION of two edges in the
    /// same call — e.g. two sibling edges that are each fine alone but jointly
    /// make one of them transitively redundant.
    pub fn add_dependencies_with_policy(
        &self,
        issue_id: &str,
        dep_ids: &[String],
        policy: RedundancyPolicy,
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::Batch {
            issue_id: issue_id.to_string(),
            dependency_ids: dep_ids.to_vec(),
            policy,
        })? {
            CapturedDependencyOutcome::Batch(result) => Ok(result),
            _ => unreachable!("batch dependency request returns a batch outcome"),
        }
    }

    /// Repair every transitive-reduction violation from one captured graph.
    pub(super) fn reduce_all_dependencies(&self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::ReduceAll {
            dry_run,
        })? {
            CapturedDependencyOutcome::Reduced { count, messages } => Ok((count, messages)),
            _ => unreachable!("reduction repair returns a reduction outcome"),
        }
    }

    /// Capture the complete active issue graph and publish one dependency mutation.
    ///
    /// The index is the sole discovery authority. The second phase captures every
    /// indexed record, the complete issues-directory listing, and the event log;
    /// semantic derivation reads only that closed image. A conflict restarts both
    /// phases in a fresh recovered session while reusing one mutation context.
    fn publish_captured_dependency_mutation(
        &self,
        request: CapturedDependencyMutation,
    ) -> Result<CapturedDependencyOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let layout = self.require_layout()?;
        let index_path = VirtualPath::data("index.json")?;
        let events_path = VirtualPath::data("events.jsonl")?;
        let issues_path = VirtualPath::data("issues")?;
        // Lease coordination precedes the repository guard and runs once. The
        // mutation request is then pinned to that captured full id, so retries
        // cannot retarget a short prefix to another issue.
        let (expected_source, warning) = match &request {
            CapturedDependencyMutation::Single { issue_id, .. }
            | CapturedDependencyMutation::Batch { issue_id, .. } => {
                let mut resolved = None;
                for _ in 0..8 {
                    let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                    if let Some((_, issues)) = capture_dependency_attempt(
                        preflight.as_mut(),
                        &index_path,
                        &events_path,
                        &issues_path,
                    )? {
                        resolved = Some(resolve_captured_issue_id(&issues, issue_id)?);
                        break;
                    }
                }
                let full_id = resolved.ok_or_else(|| {
                    anyhow!("dependency preflight did not converge after repeated conflicts")
                })?;
                let warning = self.require_active_lease(&full_id)?;
                (Some(full_id), warning)
            }
            CapturedDependencyMutation::ReduceAll { .. } => (None, None),
        };
        let request = match request {
            CapturedDependencyMutation::Single {
                dependency_id,
                policy,
                ..
            } => CapturedDependencyMutation::Single {
                issue_id: expected_source
                    .ok_or_else(|| anyhow!("single dependency preflight returned no source"))?,
                dependency_id,
                policy,
            },
            CapturedDependencyMutation::Batch {
                dependency_ids,
                policy,
                ..
            } => CapturedDependencyMutation::Batch {
                issue_id: expected_source
                    .ok_or_else(|| anyhow!("batch dependency preflight returned no source"))?,
                dependency_ids,
                policy,
            },
            CapturedDependencyMutation::ReduceAll { dry_run } => {
                CapturedDependencyMutation::ReduceAll { dry_run }
            }
        };
        let context = MutationContext::production();

        for _ in 0..8 {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some((image, issues)) = capture_dependency_attempt(
                session.as_mut(),
                &index_path,
                &events_path,
                &issues_path,
            )?
            else {
                continue;
            };
            let derived = derive_dependency_mutation(&issues, &request, warning.clone())?;
            if derived.intents.is_empty() {
                return Ok(derived.outcome);
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            match session.apply(&plan) {
                Ok(_) => return Ok(derived.outcome),
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "dependency mutation did not converge after repeated capture conflicts"
        ))
    }

    /// Remove multiple dependencies from an issue.
    ///
    /// This is what `jit dep rm <from> <id>...` calls. Each `dep_id` is
    /// matched directly against `issue`'s OWN stored `dependencies` (by exact
    /// id, or by normalized prefix once at least 4 characters are given) —
    /// NOT resolved through the repo-wide index first. A dependency whose
    /// target issue was deleted no longer resolves there, which previously
    /// made a dangling edge permanently unremovable via the CLI (jit:f847df3f);
    /// matching the raw stored id sidesteps that resolution step entirely and
    /// works whether the target is alive or gone. An exact stored id always
    /// wins; a prefix that matches more than one stored dependency is rejected
    /// as ambiguous rather than removing an arbitrary one.
    pub fn remove_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesRemoveResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate input
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }

        let full_issue_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_issue_id)?;

        let mut removed = Vec::new();
        let mut not_found = Vec::new();

        for dep_id in dep_ids {
            let normalized = dep_id.to_lowercase().replace('-', "");

            // An exact stored id is always unambiguous, even when it is also a
            // prefix of another stored id.
            let exact = issue
                .dependencies
                .iter()
                .find(|stored| stored.as_str() == dep_id.as_str())
                .cloned();

            let stored_match = if let Some(exact) = exact {
                Some(exact)
            } else {
                // No exact match: validate the target id the SAME way
                // `resolve_issue_id` validates `<from>`, so both id arguments of
                // `dep rm` are checked identically (jit:a05b87ae). A sub-4-char
                // prefix is a typed argument error (exit 2), not a silent
                // "not found" (exit 0).
                if normalized.len() < 4 {
                    return Err(crate::storage::InvalidIdPrefixError::new(dep_id.clone()).into());
                }

                // Match by normalized prefix, collecting ALL matches so an
                // ambiguous prefix is rejected the way `resolve_issue_id` rejects
                // it, rather than silently removing whichever edge happens to
                // appear first (jit:f847df3f).
                let matches: Vec<String> = issue
                    .dependencies
                    .iter()
                    .filter(|stored| {
                        stored
                            .to_lowercase()
                            .replace('-', "")
                            .starts_with(&normalized)
                    })
                    .cloned()
                    .collect();

                match matches.as_slice() {
                    [] => None,
                    [only] => Some(only.clone()),
                    _ => {
                        return Err(crate::storage::AmbiguousIdError::dependency(
                            dep_id.clone(),
                            matches,
                        )
                        .into());
                    }
                }
            };

            match stored_match {
                Some(full_dep_id) => {
                    issue.dependencies.retain(|d| d != &full_dep_id);
                    removed.push(dep_id.clone());
                }
                None => {
                    not_found.push(dep_id.clone());
                }
            }
        }

        // Only persist (and event-log) when at least one edge was actually
        // removed: a no-op `jit dep rm` (every target absent) must not bump
        // `updated_at` or emit an event, the same no-change contract as
        // `update_issue`. Removing edges can unblock the issue, so the readiness
        // check runs only on the real-change path.
        if !removed.is_empty() {
            let event = Event::draft_issue_updated(
                full_issue_id.clone(),
                "dependency-remove".to_string(),
                vec!["dependencies".to_string()],
            );
            self.publish_ambient_issue_mutation(vec![issue], vec![(1, event)])?;
            self.auto_transition_to_ready(&full_issue_id)?;
        }

        Ok(DependenciesRemoveResult { removed, not_found })
    }
}

fn capture_dependency_attempt(
    session: &mut dyn crate::storage::RepositoryMutationSession,
    index_path: &VirtualPath,
    events_path: &VirtualPath,
    issues_path: &VirtualPath,
) -> Result<Option<(RepositoryImage, Vec<Issue>)>> {
    let first = match session.capture(CaptureSpec::phase_one(
        [index_path.clone()],
        DEPENDENCY_CAPTURE_BUDGET,
    )?) {
        Ok(image) => image,
        Err(RepositoryStateStoreError::RetryableConflict { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let discovered = parse_captured_index(&first, index_path)?;
    let mut spec = CaptureSpec::phase_one(
        [index_path.clone(), events_path.clone()],
        DEPENDENCY_CAPTURE_BUDGET,
    )?;
    spec.discover_paths(
        discovered
            .all_ids
            .iter()
            .map(|id| VirtualPath::data(format!("issues/{id}.json")))
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    spec.discover_listing(issues_path.clone())?;
    let image = match session.capture(spec) {
        Ok(image) => image,
        Err(RepositoryStateStoreError::RetryableConflict { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let captured = parse_captured_index(&image, index_path)?;
    if captured.all_ids != discovered.all_ids || captured.deleted_ids != discovered.deleted_ids {
        return Ok(None);
    }
    let issues = parse_captured_issues(&image, issues_path, &captured.all_ids)?;
    Ok(Some((image, issues)))
}

fn parse_captured_index(
    image: &RepositoryImage,
    path: &VirtualPath,
) -> Result<crate::repository_state::RepositoryIndex> {
    let bytes = image
        .file_bytes(path)?
        .ok_or_else(|| anyhow!("index.json is absent during dependency mutation"))?;
    crate::storage::json::parse_repository_index(bytes)
}

fn parse_captured_issues(
    image: &RepositoryImage,
    issues_path: &VirtualPath,
    active_ids: &[String],
) -> Result<Vec<Issue>> {
    let listing = image
        .listing_fingerprints()
        .get(issues_path)
        .ok_or_else(|| anyhow!("complete issues listing is absent from dependency capture"))?;
    let mut issues = active_ids
        .iter()
        .map(|id| {
            let path = VirtualPath::data(format!("issues/{id}.json"))?;
            let bytes = match image.entry(&path)? {
                RepositoryEntry::File { bytes, .. } => bytes,
                RepositoryEntry::Absent => return Err(IssueNotFoundError::new(id).into()),
                _ => return Err(anyhow!("indexed issue {id} is not an ordinary file")),
            };
            let issue: Issue = serde_json::from_slice(bytes)
                .with_context(|| format!("failed to parse captured issue {id}"))?;
            if issue.id != *id {
                return Err(anyhow!(
                    "indexed issue {id} contains mismatched embedded id {}",
                    issue.id
                ));
            }
            Ok(issue)
        })
        .collect::<Result<Vec<_>>>()?;
    let indexed_files = active_ids
        .iter()
        .map(|id| format!("{id}.json"))
        .collect::<BTreeSet<_>>();
    let listed_files = listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .cloned()
        .collect::<BTreeSet<_>>();
    if indexed_files != listed_files {
        return Err(anyhow!(
            "issues directory membership does not match captured index"
        ));
    }
    issues.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(issues)
}

fn resolve_captured_issue_id(issues: &[Issue], partial_id: &str) -> Result<String> {
    let normalized = partial_id.to_lowercase().replace('-', "");
    if normalized.len() < MIN_ID_PREFIX_LENGTH {
        return Err(InvalidIdPrefixError::new(partial_id).into());
    }
    let matches = issues
        .iter()
        .filter(|issue| {
            issue
                .id
                .replace('-', "")
                .to_lowercase()
                .starts_with(&normalized)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(IssueNotFoundError::new(partial_id).into()),
        [issue] => Ok(issue.id.clone()),
        _ => Err(AmbiguousIdError::issue(
            partial_id,
            matches
                .iter()
                .map(|issue| format!("{} | {}", issue.short_id(), issue.title)),
        )
        .into()),
    }
}

fn captured_issue<'a>(issues: &'a [Issue], id: &str) -> Result<&'a Issue> {
    issues
        .iter()
        .find(|issue| issue.id == id)
        .ok_or_else(|| IssueNotFoundError::new(id).into())
}

fn derive_dependency_mutation(
    issues: &[Issue],
    request: &CapturedDependencyMutation,
    warning: Option<String>,
) -> Result<DerivedDependencyMutation> {
    match request {
        CapturedDependencyMutation::Single {
            issue_id,
            dependency_id,
            policy,
        } => derive_single_dependency_add(issues, issue_id, dependency_id, *policy, warning),
        CapturedDependencyMutation::Batch {
            issue_id,
            dependency_ids,
            policy,
        } => derive_batch_dependency_add(issues, issue_id, dependency_ids, *policy),
        CapturedDependencyMutation::ReduceAll { dry_run } => {
            derive_dependency_repair(issues, *dry_run)
        }
    }
}

fn derive_single_dependency_add(
    issues: &[Issue],
    issue_id: &str,
    dependency_id: &str,
    policy: RedundancyPolicy,
    warning: Option<String>,
) -> Result<DerivedDependencyMutation> {
    let full_issue_id = resolve_captured_issue_id(issues, issue_id)?;
    let full_dependency_id = resolve_captured_issue_id(issues, dependency_id)?;
    let graph = dependency_graph(issues);
    graph.validate_add_dependency(&full_issue_id, &full_dependency_id)?;
    let from = captured_issue(issues, &full_issue_id)?;
    if from.dependencies.contains(&full_dependency_id) {
        return Ok(DerivedDependencyMutation {
            outcome: CapturedDependencyOutcome::Single {
                result: DependencyAddResult::AlreadyExists,
                warning,
            },
            intents: Vec::new(),
        });
    }

    let candidate = candidate_issues(issues, &full_issue_id, [&full_dependency_id])?;
    let candidate_graph = dependency_graph(&candidate);
    let mut redundant = candidate_graph.find_redundant_edges();
    redundant.sort();
    if !redundant.is_empty() && policy == RedundancyPolicy::Reject {
        return Err(
            RedundantDependencyError::new((full_issue_id, full_dependency_id), redundant).into(),
        );
    }
    let reduced = candidate_graph.compute_transitive_reduction(&full_issue_id);
    let original = dependency_set(from);
    let result = if reduced == original {
        DependencyAddResult::Skipped {
            reason: "transitive (already reachable via other dependencies)".to_string(),
        }
    } else {
        DependencyAddResult::Added
    };
    let intents = if reduced == original {
        Vec::new()
    } else {
        derive_reduced_graph_intents(issues, &candidate, Some((&full_issue_id, &reduced)))?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Single { result, warning },
        intents,
    })
}

fn derive_batch_dependency_add(
    issues: &[Issue],
    issue_id: &str,
    dependency_ids: &[String],
    policy: RedundancyPolicy,
) -> Result<DerivedDependencyMutation> {
    let full_issue_id = resolve_captured_issue_id(issues, issue_id)?;
    let from_deps = dependency_set(captured_issue(issues, &full_issue_id)?);
    let mut already_exist = Vec::new();
    let mut new_edges = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();

    for (ordinal, dependency_id) in dependency_ids.iter().enumerate() {
        match resolve_captured_issue_id(issues, dependency_id) {
            Ok(full_id) if from_deps.contains(&full_id) || !seen.insert(full_id.clone()) => {
                already_exist.push(dependency_id.clone());
            }
            Ok(full_id) => new_edges.push((ordinal, dependency_id.clone(), full_id)),
            Err(error) => rejected.push((ordinal, dependency_id.clone(), error)),
        }
    }

    let candidate = candidate_issues(
        issues,
        &full_issue_id,
        new_edges.iter().map(|(_, _, full_id)| full_id),
    )?;
    let candidate_graph = dependency_graph(&candidate);
    let mut cycle_failed = HashSet::new();
    for (ordinal, text, full_id) in &new_edges {
        if let Err(error) = candidate_graph.validate_add_dependency(&full_issue_id, full_id) {
            cycle_failed.insert(*ordinal);
            rejected.push((*ordinal, text.clone(), error.into()));
        }
    }
    let valid_edges = new_edges
        .iter()
        .filter(|(ordinal, _, _)| !cycle_failed.contains(ordinal))
        .cloned()
        .collect::<Vec<_>>();
    let mut skipped = Vec::new();
    let mut reduced_from = from_deps.clone();
    let mut acyclic_candidate = issues.to_vec();

    if !valid_edges.is_empty() {
        acyclic_candidate = candidate_issues(
            issues,
            &full_issue_id,
            valid_edges.iter().map(|(_, _, full_id)| full_id),
        )?;
        let graph = dependency_graph(&acyclic_candidate);
        let mut redundant = graph.find_redundant_edges();
        redundant.sort();
        let self_redundant = redundant
            .iter()
            .filter(|(from, to)| {
                from == &full_issue_id && valid_edges.iter().any(|(_, _, full_id)| full_id == to)
            })
            .map(|(_, to)| to.clone())
            .collect::<HashSet<_>>();

        for (ordinal, text, full_id) in &valid_edges {
            if self_redundant.contains(full_id) {
                match policy {
                    RedundancyPolicy::Reject => rejected.push((
                        *ordinal,
                        text.clone(),
                        RedundantDependencyError::new(
                            (full_issue_id.clone(), full_id.clone()),
                            redundant.clone(),
                        )
                        .into(),
                    )),
                    RedundancyPolicy::Reduce => skipped.push((
                        text.clone(),
                        "transitive (already reachable via other dependencies)".to_string(),
                    )),
                }
            }
        }

        if policy == RedundancyPolicy::Reject {
            for (ordinal, text, full_id) in &valid_edges {
                if self_redundant.contains(full_id) {
                    continue;
                }
                let singleton = candidate_issues(issues, &full_issue_id, [full_id])?;
                let mut singleton_redundant = dependency_graph(&singleton).find_redundant_edges();
                singleton_redundant.sort();
                if !singleton_redundant.is_empty() {
                    rejected.push((
                        *ordinal,
                        text.clone(),
                        RedundantDependencyError::new(
                            (full_issue_id.clone(), full_id.clone()),
                            singleton_redundant,
                        )
                        .into(),
                    ));
                }
            }
        }
        reduced_from = graph.compute_transitive_reduction(&full_issue_id);
    }

    if !rejected.is_empty() {
        rejected.sort_by_key(|(ordinal, _, _)| *ordinal);
        return Err(DependencyBatchRejectedError::new(
            full_issue_id,
            rejected
                .into_iter()
                .map(|(_, dependency_id, error)| (dependency_id, error))
                .collect(),
        )
        .into());
    }
    let added_ids = reduced_from
        .difference(&from_deps)
        .cloned()
        .collect::<HashSet<_>>();
    let added = new_edges
        .iter()
        .filter(|(_, _, full_id)| added_ids.contains(full_id))
        .map(|(_, text, _)| text.clone())
        .collect();
    let intents = if reduced_from == from_deps {
        Vec::new()
    } else {
        derive_reduced_graph_intents(
            issues,
            &acyclic_candidate,
            Some((&full_issue_id, &reduced_from)),
        )?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Batch(DependenciesAddResult {
            added,
            already_exist,
            skipped,
        }),
        intents,
    })
}

fn derive_dependency_repair(issues: &[Issue], dry_run: bool) -> Result<DerivedDependencyMutation> {
    let graph = dependency_graph(issues);
    let reductions = issues
        .iter()
        .filter_map(|issue| {
            let reduced = graph.compute_transitive_reduction(&issue.id);
            let removed = issue.dependencies.len().saturating_sub(reduced.len());
            (removed > 0).then_some((issue, reduced, removed))
        })
        .collect::<Vec<_>>();
    let count = reductions.iter().map(|(_, _, count)| count).sum();
    let messages = if dry_run {
        Vec::new()
    } else {
        reductions
            .iter()
            .map(|(issue, _, fixed)| {
                format!(
                    "Fixed {} redundant {} in issue {}",
                    fixed,
                    if *fixed == 1 {
                        "dependency"
                    } else {
                        "dependencies"
                    },
                    &issue.id[..8.min(issue.id.len())]
                )
            })
            .collect()
    };
    let intents = if dry_run || reductions.is_empty() {
        Vec::new()
    } else {
        derive_reduced_graph_intents(issues, issues, None)?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Reduced { count, messages },
        intents,
    })
}

fn candidate_issues<'a>(
    issues: &[Issue],
    source_id: &str,
    dependencies: impl IntoIterator<Item = &'a String>,
) -> Result<Vec<Issue>> {
    let mut candidate = issues.to_vec();
    let source = candidate
        .iter_mut()
        .find(|issue| issue.id == source_id)
        .ok_or_else(|| IssueNotFoundError::new(source_id))?;
    source
        .dependencies
        .extend(dependencies.into_iter().cloned());
    source.dependencies.sort();
    source.dependencies.dedup();
    Ok(candidate)
}

fn dependency_graph(issues: &[Issue]) -> DependencyGraph<'_, Issue> {
    DependencyGraph::new(&issues.iter().collect::<Vec<_>>())
}

fn dependency_set(issue: &Issue) -> HashSet<String> {
    issue.dependencies.iter().cloned().collect()
}

fn derive_reduced_graph_intents(
    original: &[Issue],
    candidate: &[Issue],
    source: Option<(&str, &HashSet<String>)>,
) -> Result<Vec<MutationIntent>> {
    let graph = dependency_graph(candidate);
    let mut updates = Vec::new();
    let mut events = Vec::new();

    for candidate_issue in candidate {
        let original_issue = captured_issue(original, &candidate_issue.id)?;
        let original_deps = dependency_set(original_issue);
        let reduced = match source {
            Some((source_id, source_reduced)) if source_id == candidate_issue.id => {
                source_reduced.clone()
            }
            _ => graph.compute_transitive_reduction(&candidate_issue.id),
        };
        if reduced == original_deps {
            continue;
        }
        let mut issue = original_issue.clone();
        let mut removed = original_deps
            .difference(&reduced)
            .cloned()
            .collect::<Vec<_>>();
        removed.sort();
        issue.dependencies = reduced.iter().cloned().collect();
        issue.dependencies.sort();
        let issue_id = issue.id.clone();

        if source.is_some_and(|(source_id, _)| source_id == issue_id) {
            let added = reduced.difference(&original_deps);
            let any_unmet = added
                .map(|dependency_id| captured_issue(original, dependency_id))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|dependency| !is_dependency_met(dependency.state, dependency.archived_from));
            if issue.state == State::Ready && any_unmet {
                issue.state = State::Backlog;
                events.push(MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(Event::draft_issue_state_changed(
                        issue_id.clone(),
                        State::Ready,
                        State::Backlog,
                    )),
                });
            }
            events.push(MutationIntent::RecordEvent {
                phase: 2,
                event: Box::new(Event::draft_issue_updated(
                    issue_id,
                    "dependency-add".to_string(),
                    vec!["dependencies".to_string()],
                )),
            });
        } else {
            events.push(MutationIntent::RecordEvent {
                phase: 3,
                event: Box::new(Event::draft_dependency_reduced(
                    issue_id,
                    original_deps.len(),
                    reduced.len(),
                    removed,
                )),
            });
        }
        updates.push(MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        });
    }
    updates.extend(events);
    Ok(updates)
}

#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::storage::{InMemoryStorage, IssueStore, RepositoryStateStore};

    fn capture(
        storage: &InMemoryStorage,
    ) -> (
        Box<dyn crate::storage::RepositoryMutationSession>,
        RepositoryImage,
        Vec<Issue>,
    ) {
        let mut session = storage
            .open_mutation_session(storage.repository_layout())
            .unwrap();
        let (image, issues) = capture_dependency_attempt(
            session.as_mut(),
            &VirtualPath::data("index.json").unwrap(),
            &VirtualPath::data("events.jsonl").unwrap(),
            &VirtualPath::data("issues").unwrap(),
        )
        .unwrap()
        .unwrap();
        (session, image, issues)
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> &[u8] {
        plan.delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                crate::repository_state::RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &VirtualPath::data("events.jsonl").unwrap() =>
                {
                    Some(bytes.as_slice())
                }
                _ => None,
            })
            .unwrap()
    }

    fn seed_index(storage: &InMemoryStorage, ids: &[&str]) {
        storage.add_repo_file(
            ".jit/index.json",
            &serde_json::json!({
                "schema_version": 2,
                "all_ids": ids,
                "deleted_ids": []
            })
            .to_string(),
        );
    }

    #[test]
    fn test_dependency_capture_budget_is_fixed() {
        assert_eq!(DEPENDENCY_CAPTURE_BUDGET.max_paths, 1 << 16);
        assert_eq!(DEPENDENCY_CAPTURE_BUDGET.max_listings, 1);
    }

    #[test]
    fn test_dependency_retry_rederives_and_preserves_concurrent_issue_change() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.state = State::Ready;
        let source_id = source.id.clone();
        storage.save_issue(source).unwrap();
        let dependency = crate::domain::types::fixture_issue("dependency".into(), String::new());
        let dependency_id = dependency.id.clone();
        storage.save_issue(dependency).unwrap();
        seed_index(&storage, &[&source_id, &dependency_id]);
        let request = CapturedDependencyMutation::Single {
            issue_id: source_id.clone(),
            dependency_id: dependency_id.clone(),
            policy: RedundancyPolicy::Reduce,
        };
        let context = MutationContext::deterministic(
            [29; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-21T10:11:12Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let (mut first_session, first_image, first_issues) = capture(&storage);
        let first = derive_dependency_mutation(&first_issues, &request, None).unwrap();
        let first_plan =
            finalize(first_image.layout(), &first_image, &context, &first.intents).unwrap();

        let mut concurrent = storage.load_issue(&source_id).unwrap();
        concurrent.labels.push("owner:concurrent".into());
        storage.save_issue(concurrent).unwrap();
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let (mut retry_session, retry_image, retry_issues) = capture(&storage);
        let retry = derive_dependency_mutation(&retry_issues, &request, None).unwrap();
        let retry_plan =
            finalize(retry_image.layout(), &retry_image, &context, &retry.intents).unwrap();
        assert_eq!(event_bytes(&retry_plan), event_bytes(&first_plan));
        retry_session.apply(&retry_plan).unwrap();

        let updated = storage.load_issue(&source_id).unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert_eq!(updated.dependencies, vec![dependency_id]);
        assert_eq!(updated.state, State::Backlog);
    }

    #[test]
    fn test_dependency_capture_reports_indexed_missing_record_as_typed_error() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        let issue = crate::domain::types::fixture_issue("missing".into(), String::new());
        let id = issue.id.clone();
        storage.save_issue(issue).unwrap();
        seed_index(&storage, &[&id]);
        storage
            .repository_state()
            .entries
            .remove(&VirtualPath::data(format!("issues/{id}.json")).unwrap());

        let mut session = storage
            .open_mutation_session(storage.repository_layout())
            .unwrap();
        let error = capture_dependency_attempt(
            session.as_mut(),
            &VirtualPath::data("index.json").unwrap(),
            &VirtualPath::data("events.jsonl").unwrap(),
            &VirtualPath::data("issues").unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<IssueNotFoundError>().unwrap().id(), id);
    }
}
