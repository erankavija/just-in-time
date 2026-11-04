//! Validation and status operations

use super::*;
use crate::domain::type_taxonomy::{detect_validation_issues, generate_fixes, ValidationFix};
use crate::domain::{EventBody, ReadinessCorrection};
use crate::repository_state::{
    MutationIntent, RepositoryAction, RepositoryDeclarations, RepositoryImage,
};
use anyhow::Context;

#[derive(Clone, Copy)]
enum BaseCapturePurpose<'a> {
    Validation,
    Mutation {
        precheck_target: Option<&'a IssueId>,
    },
    Repair,
    Archive,
}

/// A capture's parsed issues and, for validation, the JSON value each decoded from.
type ParsedIssues = (Result<Vec<Issue>>, Vec<serde_json::Value>);

struct CapturedRepairPlan {
    declarations: crate::repository_state::CapturedRepositoryDeclarations,
    plan: Option<crate::repository_state::MaterializationPlan>,
}

#[derive(Default)]
struct PlannedFixes {
    count: usize,
    messages: Vec<String>,
    intents: Vec<MutationIntent>,
    actions: Vec<RepositoryAction>,
}

impl PlannedFixes {
    fn merge(mut self, other: Self) -> Self {
        self.count += other.count;
        self.messages.extend(other.messages);
        self.intents.extend(other.intents);
        self.actions.extend(other.actions);
        self
    }

    fn skipped(class: &str, error: impl std::fmt::Display) -> Self {
        Self {
            messages: vec![format!("Skipped {class}: {error:#}")],
            ..Self::default()
        }
    }
}

/// Validation operations of a [`CommandExecutor`].
pub struct ValidationOps<'a, S: IssueStore> {
    exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The validation family.
    pub fn validation(&self) -> ValidationOps<'_, S> {
        ValidationOps { exec: self }
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> ValidationOps<'_, S> {
    /// Derive all repairs from one capture, publish once, then report the resulting repository.
    pub fn validate_with_fix(
        &self,
        dry_run: bool,
    ) -> Result<(
        usize,
        Vec<String>,
        crate::validation::ValidationReport,
        crate::output::DivergenceResponse,
    )> {
        use crate::repository_state::{finalize, MaterializationPlan, RepositoryDelta};
        let layout = self.exec.require_layout()?;
        let context = production_mutation_context();
        let (count, mut messages) = with_mutation_session(
            self.exec.storage(),
            &layout,
            "validation repair",
            |session| {
                let Some(captured) = self.capture_validation(session)? else {
                    return Ok(SessionStep::Retry);
                };
                let (Ok(original), Ok(_)) = (&captured.issues, &captured.index) else {
                    return Ok(SessionStep::Done((0, Vec::new())));
                };
                let image = &captured.image;
                let mut projected = original.to_vec();
                let declarations =
                    crate::repository_state::validation_declarations_from_image(image);
                let fixes = match &declarations {
                    Ok(declarations) => {
                        self.plan_hierarchy_fixes(declarations.config(), &mut projected, dry_run)?
                    }
                    Err(error) => {
                        PlannedFixes::skipped("hierarchy fixes and derived-state repair", error)
                    }
                };
                let mut fixes = fixes
                    .merge(plan_reduction_fixes(&mut projected)?)
                    .merge(plan_readiness_fixes(&mut projected));
                if let Ok(declarations) = declarations {
                    let repairs = match declarations.rules_load_error() {
                        Some(error) => PlannedFixes::skipped(
                            "derived-state repair",
                            format_args!("rules.toml could not be loaded: {error:#}"),
                        ),
                        None => plan_derived_state_fixes(image, declarations.borrowed()),
                    };
                    fixes = fixes.merge(repairs);
                }
                fixes.intents.extend(
                    projected
                        .into_iter()
                        .zip(original.iter())
                        .filter(|(issue, before)| issue != *before)
                        .map(|(issue, _)| MutationIntent::UpdateIssue {
                            issue: Box::new(issue),
                        }),
                );
                let issues = finalize(&layout, image, &context, &fixes.intents, Some(original))?;
                let mut actions = issues.delta().actions().to_vec();
                for action in fixes.actions {
                    if let RepositoryAction::AppendEvents { bytes, .. } = &action {
                        if let Some(RepositoryAction::AppendEvents {
                            bytes: existing, ..
                        }) = actions
                            .iter_mut()
                            .find(|action| matches!(action, RepositoryAction::AppendEvents { .. }))
                        {
                            existing.extend_from_slice(bytes);
                            continue;
                        }
                    }
                    actions.push(action);
                }
                let (count, messages) = (fixes.count, fixes.messages);
                let plan = MaterializationPlan::new(image, RepositoryDelta::new(&layout, actions)?);
                if dry_run || plan.delta().actions().is_empty() {
                    Ok(SessionStep::Done((count, messages)))
                } else {
                    Ok(SessionStep::Apply(plan, (count, messages)))
                }
            },
        )?;
        messages.push(if dry_run {
            format!("Dry run complete. {count} fixes would be applied.")
        } else if count > 0 {
            format!("Applied {count} fixes")
        } else {
            "No fixes applied".into()
        });
        let (report, divergences) = self.capture_validation_report()?;
        Ok((count, messages, report, divergences))
    }

    fn plan_hierarchy_fixes(
        &self,
        config: &crate::config::JitConfig,
        projected: &mut [Issue],
        dry_run: bool,
    ) -> Result<PlannedFixes> {
        let mut fixes = PlannedFixes::default();
        let hierarchy = match crate::domain::type_taxonomy::HierarchyConfig::from_declarations(
            &crate::config_manager::namespaces_from_config(config),
        ) {
            Ok(hierarchy) => hierarchy,
            Err(error) => {
                return Ok(PlannedFixes::skipped("hierarchy fixes", error));
            }
        };
        for issue in projected {
            let problems = detect_validation_issues(&hierarchy, &issue.id, &issue.labels);
            for fix in generate_fixes(&problems) {
                let ValidationFix::ReplaceType {
                    issue_id,
                    old_type,
                    new_type,
                } = fix;
                issue
                    .labels
                    .retain(|label| label != &label_utils::type_label(&old_type));
                issue.labels.push(label_utils::type_label(&new_type));
                if !dry_run {
                    fixes
                        .messages
                        .extend(self.exec.refuse_foreign_lease(&issue_id)?);
                }
                fixes.messages.push(format!(
                    "{} type '{old_type}' with '{new_type}' for issue {issue_id}",
                    if dry_run { "Would replace" } else { "Replaced" }
                ));
                fixes.count += 1;
                fixes.intents.push(MutationIntent::RecordEvent {
                    phase: 0,
                    event: Box::new(EventBody::draft_issue_updated(
                        issue_id,
                        "issue-update".into(),
                        vec!["labels".into()],
                    )),
                });
            }
        }
        Ok(fixes)
    }

    /// Validate the exact repository through the closed-image pipeline.
    ///
    /// Declaration and rule passes share one validation capture; document references
    /// resolve in separate bounded images under the same session. Every pass
    /// reads captured evidence.
    pub fn validate_silent(&self) -> Result<()> {
        let (failure, rule_report) = self.validate_repository_report()?.into_parts();
        if let Some(error) = failure {
            // Callers render only the outermost message; it carries every cause.
            let message = format!("{error:#}");
            return Err(error.context(message));
        }
        if rule_report.has_errors() {
            let messages = rule_report
                .findings
                .iter()
                .filter(|finding| finding.is_error())
                .map(|finding| format!("[{}] {}", finding.rule, finding.message))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow!(
                "Validation failed with {} rule error(s):\n{}",
                rule_report.error_count(),
                messages
            ));
        }
        Ok(())
    }
}

fn plan_reduction_fixes(projected: &mut [Issue]) -> Result<PlannedFixes> {
    let mut fixes = PlannedFixes::default();
    for intent in super::dependency::derive_reduced_graph_intents(projected, projected, None)? {
        match intent {
            MutationIntent::UpdateIssue { issue } => {
                if let Some(target) = projected.iter_mut().find(|target| target.id == issue.id) {
                    let removed = target.dependencies.len() - issue.dependencies.len();
                    fixes.count += removed;
                    fixes.messages.push(format!(
                        "Fixed {removed} redundant dependencies in issue {}",
                        issue.short_id()
                    ));
                    *target = *issue;
                }
            }
            event => fixes.intents.push(event),
        }
    }
    Ok(fixes)
}

fn plan_readiness_fixes(projected: &mut [Issue]) -> PlannedFixes {
    let mut fixes = PlannedFixes::default();
    loop {
        let resolved = crate::domain::queries::build_issue_map(projected);
        let corrections = projected
            .iter()
            .enumerate()
            .filter_map(|(index, issue)| {
                issue
                    .derive_readiness_correction(&resolved)
                    .map(|correction| (index, correction))
            })
            .collect::<Vec<_>>();
        if corrections.is_empty() {
            return fixes;
        }
        fixes.count += corrections.len();
        for (index, correction) in corrections {
            let issue = &mut projected[index];
            let from = issue.state;
            issue.state = match correction {
                ReadinessCorrection::Promote => State::Ready,
                ReadinessCorrection::Demote => State::Backlog,
            };
            fixes.messages.push(format!(
                "  → Transitioning {} to {}",
                issue.short_id(),
                issue.state.as_str()
            ));
            fixes.intents.push(MutationIntent::RecordEvent {
                phase: 4,
                event: Box::new(EventBody::draft_issue_state_changed(
                    issue.id.clone(),
                    from,
                    issue.state,
                )),
            });
        }
    }
}

fn plan_derived_state_fixes(
    image: &RepositoryImage,
    declarations: RepositoryDeclarations<'_>,
) -> PlannedFixes {
    use crate::repository_state::{derive_materialization, MaterializationRequest};
    match derive_materialization(
        image,
        MaterializationRequest::RepairDerivedState { declarations },
    ) {
        Ok(repair) => {
            let changed = repair
                .delta()
                .actions()
                .iter()
                .filter(|action| !matches!(action, RepositoryAction::CreateDirectory { .. }))
                .map(|action| action.path())
                .collect::<std::collections::BTreeSet<_>>();
            PlannedFixes {
                count: changed.len(),
                messages: changed
                    .into_iter()
                    .map(|path| format!("Repaired derived-state target {path:?}"))
                    .collect(),
                actions: repair.delta().actions().to_vec(),
                ..PlannedFixes::default()
            }
        }
        Err(error) => PlannedFixes::skipped("derived-state repair", error),
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> CommandExecutor<S> {
    /// One bounded capture round of the whole-repository validation base under an
    /// already-held session.
    ///
    /// Returns the captured PRE-overlay base (the caller applies any overlay for
    /// validation and reads it for its own delta), `Ok(None)` on a retryable
    /// read-set conflict, or an error otherwise. `overrides` scopes the closure to
    /// the proposed declarations (so a proposed init/profile state captures what
    /// its passes read); `extra_paths` discovers additional delta targets (init
    /// schema files, profile assets, directories) into the base so a
    /// subsequent `apply` finds each action's captured preimage; `start` is an
    /// image this session's round has already read, which the base grows from.
    /// Only rule-consumed plan documents enter here; other consuming passes
    /// extend their selected references through [`discover_document_references`].
    /// Sharing one held session lets a caller capture and apply under the same guard.
    pub(crate) fn capture_proposed_base(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        overrides: &std::collections::BTreeMap<
            crate::repository_state::VirtualPath,
            Option<Vec<u8>>,
        >,
        extra_paths: &[crate::repository_state::VirtualPath],
        start: Option<crate::repository_state::RepositoryImage>,
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        self.capture_proposed_base_inner(
            session,
            start,
            overrides,
            extra_paths,
            BaseCapturePurpose::Validation,
        )
        .map(|captured| captured.map(|(image, _)| image))
    }

    /// The base an issue mutation derives from: the registries, every issue,
    /// and the plan documents its transition rules read.
    pub(crate) fn capture_mutation_base(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        extra_paths: &[crate::repository_state::VirtualPath],
        precheck_target: Option<&IssueId>,
    ) -> Result<Option<MutationBase>> {
        self.capture_proposed_base_inner(
            session,
            None,
            &std::collections::BTreeMap::new(),
            extra_paths,
            BaseCapturePurpose::Mutation { precheck_target },
        )
        .and_then(|captured| {
            captured
                .map(|(image, (issues, _))| {
                    Ok(MutationBase {
                        image,
                        issues: issues?,
                    })
                })
                .transpose()
        })
    }

    /// Capture the resolved target and lease policy before taking the claims guard.
    /// Extend to the mutation base when `needs_base` holds.
    pub(crate) fn capture_mutation_preflight(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        issue_id: &IssueId,
        needs_base: impl FnOnce(&Issue) -> bool,
    ) -> Result<Option<MutationPreflight>> {
        use crate::repository_state::{CaptureSpec, VirtualPath};

        let record = VirtualPath::issue_record(issue_id);
        let spec = CaptureSpec::new(
            [VirtualPath::CONFIG, record.clone()],
            MUTATION_CAPTURE_BUDGET,
        )?;
        let Some(image) = super::capture_discovering(session, spec, |_, _| Ok(()))? else {
            return Ok(None);
        };
        let candidates = match image.file_bytes(&record)? {
            Some(_) => vec![issue_id.clone()],
            None => Vec::new(),
        };
        let issues = crate::repository_state::parse_issue_records(&candidates, |path| {
            image.file_bytes(path)
        })?
        .into_iter()
        .map(|(issue, _)| issue)
        .collect::<Vec<_>>();
        let target_id = super::require_issue_in_capture(&issues, issue_id)?;
        let target = issues
            .into_iter()
            .find(|issue| issue.id == target_id)
            .ok_or_else(|| crate::storage::IssueNotFoundError::new(target_id.clone()))?;
        if !needs_base(&target) {
            return Ok(Some(MutationPreflight {
                image,
                target,
                issues: None,
            }));
        }
        let grown = self.capture_proposed_base_inner(
            session,
            Some(image),
            &std::collections::BTreeMap::new(),
            &[],
            BaseCapturePurpose::Mutation {
                precheck_target: Some(issue_id),
            },
        )?;
        grown
            .map(|(image, (issues, _))| {
                Ok(MutationPreflight {
                    image,
                    target,
                    issues: Some(issues?),
                })
            })
            .transpose()
    }

    /// The base an archive plan reads: the event log's length and tail, with
    /// a listing budget for every directory below the archive roots.
    pub(crate) fn capture_archive_base(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        self.capture_proposed_base_inner(
            session,
            None,
            &std::collections::BTreeMap::new(),
            &[],
            BaseCapturePurpose::Archive,
        )
        .map(|captured| captured.map(|(image, _)| image))
    }

    /// One capture round of the base, from `start` when the caller has read
    /// part of it already.
    ///
    /// The round reads the registries, the event log's committed length and
    /// final-line tail, which determine the append offset, and the caller's
    /// paths (a `mutation` also parses issues strictly); then the closure those
    /// registries plan. Validation and mutation passes also read linked plan
    /// documents; a mutation's precheck target adds prompt files and gate-run
    /// results.
    /// Repair and archive omit these rule-evaluation inputs.
    fn capture_proposed_base_inner(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        start: Option<crate::repository_state::RepositoryImage>,
        overrides: &std::collections::BTreeMap<
            crate::repository_state::VirtualPath,
            Option<Vec<u8>>,
        >,
        extra_paths: &[crate::repository_state::VirtualPath],
        purpose: BaseCapturePurpose<'_>,
    ) -> Result<Option<(crate::repository_state::RepositoryImage, ParsedIssues)>> {
        use crate::repository_state::{
            validate_capture_closure, CaptureBudget, CaptureSpec, RepositoryImage, VirtualPath,
        };

        let mutation = matches!(purpose, BaseCapturePurpose::Mutation { .. });
        let precheck_target = match purpose {
            BaseCapturePurpose::Mutation { precheck_target } => precheck_target,
            _ => None,
        };
        // An archive rerun enumerates every directory below its existing
        // container destination, so it is allowed a listing per directory an
        // archive of the admitted size can hold rather than the handful a
        // registry scan needs.
        let budget = CaptureBudget {
            max_listings: if matches!(purpose, BaseCapturePurpose::Archive) {
                1 << 16
            } else {
                MUTATION_CAPTURE_BUDGET.max_listings
            },
            ..MUTATION_CAPTURE_BUDGET
        };
        let base = |spec: &mut CaptureSpec| -> Result<()> {
            spec.discover_paths(
                [
                    VirtualPath::CONFIG,
                    VirtualPath::INVARIANTS,
                    VirtualPath::RULES,
                    VirtualPath::GATES,
                    VirtualPath::TEMPLATES,
                    VirtualPath::INDEX,
                ]
                .into_iter()
                .chain(extra_paths.iter().cloned()),
            )?;
            spec.discover_listings([VirtualPath::ISSUES])?;
            spec.discover_tail(VirtualPath::EVENTS)?;
            Ok(())
        };
        let effective_virtual_path =
            |image: &RepositoryImage, path: &VirtualPath| -> Result<Option<Vec<u8>>> {
                match overrides.get(path) {
                    Some(value) => Ok(value.clone()),
                    None => super::image_virtual_bytes(image, path),
                }
            };
        let mut issues = None::<ParsedIssues>;
        let mut add_base = start.is_some();
        let mut discover = |image: &RepositoryImage, spec: &mut CaptureSpec| -> Result<()> {
            if add_base {
                add_base = false;
                base(spec)?;
                if spec != image.capture_spec() {
                    return Ok(());
                }
            }
            // Closure planning is best-effort: a malformed registry cannot fail
            // the capture, because validation itself is the authority that
            // reports it. A bad config/listing/rules yields a conservative closure
            // (its projection, item-kind, issue, and schema entries drop out),
            // and the subsequent validation report surfaces the parse
            // error as a finding.
            let config = effective_config(image, &effective_virtual_path).unwrap_or_default();
            let issue_ids =
                crate::repository_state::overlaid_issue_ids(image, overrides).unwrap_or_default();
            let rules_text = effective_virtual_path(image, &VirtualPath::RULES)?
                .map(String::from_utf8)
                .transpose()?;
            let schema_rules = rules_text.as_deref().filter(|content| {
                crate::declarations::rules::RuleSet::schema_requests(content).is_ok()
            });
            let closure =
                validate_capture_closure(image.layout(), &config, &issue_ids, schema_rules)?;
            let read = image.capture_spec();
            let closure_read = closure.paths.iter().all(|path| read.contains_path(path))
                && closure
                    .listings
                    .iter()
                    .all(|listing| read.listings().contains(listing));
            spec.discover_paths(closure.paths)?;
            spec.discover_listings(closure.listings)?;
            if !closure_read {
                return Ok(());
            }
            if issues.is_none() {
                // A mutation derives from exactly these issues, so they are
                // parsed once, with the strict reader, and handed back with the
                // image. Validation parses the overlaid set leniently and reports
                // defects as findings; it keeps each record's JSON value for
                // record fidelity. Repair derives from the declarations alone and
                // decodes no record.
                let (parsed, values) = if mutation {
                    (Ok(super::captured_active_issues(image)?), Vec::new())
                } else if matches!(purpose, BaseCapturePurpose::Repair) {
                    (Ok(Vec::new()), Vec::new())
                } else {
                    // Effective bytes: the override (create/replace or proposed
                    // absence) when present, otherwise the captured base bytes.
                    match crate::repository_state::parse_issue_records(&issue_ids, |path| {
                        match overrides.get(path) {
                            Some(value) => Ok(value.as_deref()),
                            None => image.file_bytes(path),
                        }
                    }) {
                        Ok(records) => {
                            let (issues, values) = records.into_iter().unzip();
                            (Ok(issues), values)
                        }
                        Err(error) if matches!(purpose, BaseCapturePurpose::Validation) => {
                            (Err(error.into()), Vec::new())
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                if matches!(
                    purpose,
                    BaseCapturePurpose::Validation | BaseCapturePurpose::Mutation { .. }
                ) {
                    discover_issue_closure(
                        image,
                        spec,
                        parsed.as_deref().unwrap_or_default(),
                        &config,
                        &effective_virtual_path,
                        precheck_target,
                    )?;
                }
                issues = Some((parsed, values));
            }
            Ok(())
        };
        let image = match start {
            Some(image) => super::grow_discovering(session, image, &mut discover)?,
            None => {
                let mut spec = CaptureSpec::new([], budget)?;
                base(&mut spec)?;
                super::capture_discovering(session, spec, &mut discover)?
            }
        };
        Ok(image.zip(issues))
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> ValidationOps<'_, S> {
    /// Capture the validation image and compute its whole-repository report.
    pub fn validate_repository_report(&self) -> Result<crate::validation::ValidationReport> {
        Ok(self.capture_validation_report()?.0)
    }

    /// Validate and derive advisory hierarchy divergences from one captured image.
    pub fn validate_repository_report_with_divergences(
        &self,
    ) -> Result<(
        crate::validation::ValidationReport,
        crate::output::DivergenceResponse,
    )> {
        self.capture_validation_report()
    }

    fn capture_validation_report(
        &self,
    ) -> Result<(
        crate::validation::ValidationReport,
        crate::output::DivergenceResponse,
    )> {
        let layout = self.exec.require_layout()?;
        with_mutation_session(
            self.exec.storage(),
            &layout,
            "repository validation",
            |session| {
                let Some(captured) = self.capture_validation(session)? else {
                    return Ok(SessionStep::Retry);
                };
                let issues = captured.issues.as_deref().unwrap_or_default();
                let divergences =
                    crate::repository_state::validation_declarations_from_image(&captured.image)
                        .map_err(anyhow::Error::from)
                        .and_then(|declarations| {
                            captured_divergences(issues, declarations.config())
                        })
                        .unwrap_or_else(|_| crate::output::DivergenceResponse::new(Vec::new()));
                let Some(document_references) =
                    capture_document_reference_validation(session, issues)?
                else {
                    return Ok(SessionStep::Retry);
                };
                let Some(record_fidelity) = capture_record_fidelity_validation(session, &captured)?
                else {
                    return Ok(SessionStep::Retry);
                };
                let report = crate::validation::report(
                    &captured,
                    &crate::validation::ReportInputs {
                        now: chrono::Utc::now(),
                        history: Some(crate::validation::HistoryVerdicts {
                            record_fidelity,
                            document_references,
                        }),
                    },
                );
                Ok(SessionStep::Done((report, divergences)))
            },
        )
    }

    /// Repair declaration-derived targets for `jit recover`, without decoding issue records.
    pub(crate) fn repair_derived_state(&self, dry_run: bool) -> Result<(usize, Vec<String>)> {
        use crate::repository_state::RepositoryAction;

        let layout = self.exec.require_layout()?;
        with_mutation_session(
            self.exec.storage(),
            &layout,
            "derived-state repair",
            |session| {
                let Some(derived) = self.capture_repair_plan(session)? else {
                    return Ok(SessionStep::Retry);
                };
                let mut captured = derived.map_err(|error| {
                    crate::errors::ValidationFailedError::new(format!("{error:#}"))
                })?;
                if let Some(error) = captured.declarations.take_rules_load_error() {
                    return Err(
                        crate::errors::ValidationFailedError::new(format!("{error:#}")).into(),
                    );
                }
                let plan = captured.plan.ok_or_else(|| {
                    anyhow!("derived-state repair has no plan for loadable declarations")
                })?;
                // One message per changed owned target. Include mode-only drift: a
                // target can have correct bytes but the wrong mode, and treating that
                // delta as empty would skip repair.
                let changed = plan
                    .delta()
                    .actions()
                    .iter()
                    .filter_map(|action| match action {
                        RepositoryAction::WriteFile { path, .. }
                        | RepositoryAction::DeleteFile { path, .. }
                        | RepositoryAction::SetMode { path, .. } => Some(path),
                        RepositoryAction::AppendEvents { .. } => Some(action.path()),
                        RepositoryAction::CreateDirectory { .. } => None,
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                let messages = changed
                    .into_iter()
                    .map(|path| format!("Repaired derived-state target {path:?}"))
                    .collect::<Vec<_>>();
                if messages.is_empty() {
                    return Ok(SessionStep::Done((0, Vec::new())));
                }
                if dry_run {
                    return Ok(SessionStep::Done((messages.len(), messages)));
                }
                Ok(SessionStep::Apply(plan, (messages.len(), messages)))
            },
        )
    }

    /// Capture the repository once and compute its report without the history
    /// verdicts, the report every selection but `Whole` reads.
    pub fn selection_report(&self) -> Result<crate::validation::ValidationReport> {
        let layout = self.exec.require_layout()?;
        with_mutation_session(
            self.exec.storage(),
            &layout,
            "repository validation",
            |session| {
                let Some(captured) = self.capture_validation(session)? else {
                    return Ok(SessionStep::Retry);
                };
                Ok(SessionStep::Done(crate::validation::report(
                    &captured,
                    &crate::validation::ReportInputs {
                        now: chrono::Utc::now(),
                        history: None,
                    },
                )))
            },
        )
    }

    /// Capture the whole-repository validation base with the issue records its
    /// closure planning parsed.
    pub(super) fn capture_validation(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    ) -> Result<Option<crate::validation::CapturedValidation<'static>>> {
        Ok(self
            .exec
            .capture_proposed_base_inner(
                session,
                None,
                &std::collections::BTreeMap::new(),
                &[],
                BaseCapturePurpose::Validation,
            )?
            .map(|(image, (issues, values))| {
                crate::validation::CapturedValidation::with_records(image, issues, values)
            }))
    }

    /// Capture the repository's own validation closure: its registries, its
    /// event log, and the paths they plan.
    fn capture_repair_base(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        Ok(self
            .exec
            .capture_proposed_base_inner(
                session,
                None,
                &std::collections::BTreeMap::new(),
                &[],
                BaseCapturePurpose::Repair,
            )?
            .map(|(image, _)| image))
    }

    /// Capture the repair base and derive its exact repair plan.
    ///
    /// Repair derives from the declarations alone, so it restores what the
    /// repository declares and never a profile-owned target. The inner `Err` is a
    /// declaration or materialization failure.
    fn capture_repair_plan(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    ) -> Result<Option<Result<CapturedRepairPlan>>> {
        use crate::repository_state::{derive_materialization, MaterializationRequest};

        let Some(image) = self.capture_repair_base(session)? else {
            return Ok(None);
        };
        let declarations = match crate::repository_state::validation_declarations_from_image(&image)
        {
            Ok(declarations) => declarations,
            Err(error) => {
                return Ok(Some(Err(
                    anyhow!(error).context("declaration validation pass")
                )))
            }
        };
        let plan = declarations
            .rules_loaded()
            .then(|| {
                derive_materialization(
                    &image,
                    MaterializationRequest::RepairDerivedState {
                        declarations: declarations.borrowed(),
                    },
                )
            })
            .transpose();
        Ok(Some(match plan {
            Ok(plan) => Ok(CapturedRepairPlan { declarations, plan }),
            Err(error) => Err(anyhow!(error).context("derived-materialization validation pass")),
        }))
    }
}

fn captured_divergences(
    issues: &[Issue],
    config: &JitConfig,
) -> Result<crate::output::DivergenceResponse> {
    use crate::domain::type_taxonomy::HierarchyConfig;
    use crate::output::{DivergenceResponse, DivergenceView};

    let namespaces = crate::config_manager::namespaces_from_config(config);
    let hierarchy = HierarchyConfig::from_declarations(&namespaces)?;
    let issue_refs = issues.iter().collect::<Vec<_>>();
    let divergences =
        crate::graph::hierarchy::detect_membership_divergences(&issue_refs, &hierarchy);
    let by_id = issues
        .iter()
        .map(|issue| (&issue.id, issue))
        .collect::<std::collections::HashMap<_, _>>();
    let divergences = divergences
        .into_iter()
        .map(|divergence| {
            let issue = by_id.get(&divergence.issue_id);
            DivergenceView {
                short_id: divergence.issue_id.short_id().to_string(),
                title: issue.map(|issue| issue.title.clone()).unwrap_or_default(),
                id: divergence.issue_id,
                label: divergence.label,
                namespace: divergence.namespace,
                value: divergence.value,
            }
        })
        .collect::<Vec<_>>();
    Ok(DivergenceResponse::new(divergences))
}

/// Effective-bytes byte source over a captured image and a proposed overlay.
type EffectiveVirtualPath<'a> = dyn Fn(
        &crate::repository_state::RepositoryImage,
        &crate::repository_state::VirtualPath,
    ) -> Result<Option<Vec<u8>>>
    + 'a;

/// Assemble the (possibly overlaid) configuration.
fn effective_config(
    image: &crate::repository_state::RepositoryImage,
    effective: &EffectiveVirtualPath<'_>,
) -> Result<JitConfig> {
    use crate::repository_state::VirtualPath;
    let config = effective(image, &VirtualPath::CONFIG)?;
    let templates = effective(image, &VirtualPath::TEMPLATES)?;
    let invariants = effective(image, &VirtualPath::INVARIANTS)?;
    Ok(crate::declarations::assemble_configuration(
        config.as_deref(),
        templates.as_deref(),
        invariants.as_deref(),
    )?)
}

/// Add what the captured `issues` name to `spec`: the plan documents, and a
/// precheck target's prompt files and, when one of its precheck gates consumes
/// run history, the runs its required gates' states name.
fn discover_issue_closure(
    image: &crate::repository_state::RepositoryImage,
    spec: &mut crate::repository_state::CaptureSpec,
    issues: &[Issue],
    config: &JitConfig,
    effective: &EffectiveVirtualPath<'_>,
    precheck_target: Option<&IssueId>,
) -> Result<()> {
    use crate::declarations::{GateChecker, GateMode, GateStage};

    let (issue, precheck) = match precheck_target {
        Some(raw_target) => {
            let target = super::require_issue_in_capture(issues, raw_target)?;
            let issue = issues
                .iter()
                .find(|issue| issue.id == target)
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(target.clone()))?;
            let gates = match effective(image, &crate::repository_state::VirtualPath::GATES)? {
                Some(bytes) => crate::declarations::parse_gate_registry(&bytes)?,
                None => crate::declarations::GateRegistry::default(),
            };
            let precheck = issue
                .gates_required
                .iter()
                .filter_map(|key| gates.gates.get(key))
                .filter(|gate| gate.stage == GateStage::Precheck)
                .cloned()
                .collect();
            (Some(issue), precheck)
        }
        None => (None, Vec::new()),
    };
    spec.discover_paths(plan_document_paths(issues, config)?)?;
    let automated = || {
        precheck
            .iter()
            .filter(|gate| gate.mode == GateMode::Auto)
            .filter_map(|gate| gate.checker.as_ref())
    };
    spec.discover_paths(
        automated()
            .filter_map(|checker| match checker {
                GateChecker::Exec {
                    pass_context: true,
                    prompt_file: Some(path),
                    ..
                } => Some(path),
                _ => None,
            })
            .map(|path| {
                image
                    .layout()
                    .classify_repository_relative(path)
                    .map_err(|_| anyhow!("prompt_file '{}' resolves outside the repository", path))
            })
            .collect::<Result<Vec<_>>>()?,
    )?;
    if let Some(issue) = issue.filter(|_| automated().any(super::checker_consumes_run_history)) {
        spec.discover_paths(
            super::named_gate_run_records(issue)
                .map(|(_, _, path)| path)
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(())
}

/// Capture budget for mutation bases and preflights.
pub(super) const MUTATION_CAPTURE_BUDGET: crate::repository_state::CaptureBudget =
    crate::repository_state::CaptureBudget {
        max_listings: 256,
        max_bytes: super::CAPTURE_MAX_BYTES,
        max_depth: 32,
    };

/// Capture budget for the record-fidelity pass, which lists one gate-run
/// directory per issue; the byte limit bounds its growth.
const RECORD_FIDELITY_CAPTURE_BUDGET: crate::repository_state::CaptureBudget =
    crate::repository_state::CaptureBudget {
        max_listings: 65_536,
        ..MUTATION_CAPTURE_BUDGET
    };

/// The base an issue mutation derives from, from
/// [`CommandExecutor::capture_mutation_base`].
pub(crate) struct MutationBase {
    /// Registries, issues, and the plan documents transition rules read.
    pub(crate) image: crate::repository_state::RepositoryImage,
    /// Every issue the listing in `image` states, sorted by id.
    pub(crate) issues: Vec<Issue>,
}

/// The lease inputs and target issue a mutation reads before taking the claims
/// guard, from [`CommandExecutor::capture_mutation_preflight`].
pub(crate) struct MutationPreflight {
    /// Configuration and candidate issue, or the full mutation base
    /// when `issues` is present.
    pub(crate) image: crate::repository_state::RepositoryImage,
    /// The issue the request names.
    pub(crate) target: Issue,
    /// Every listed issue, when the preflight grew to the full base.
    pub(crate) issues: Option<Vec<Issue>>,
}

/// Request only the references a consuming pass selects.
pub(super) fn discover_document_references<'a>(
    documents: impl IntoIterator<Item = &'a crate::domain::DocumentReference>,
    spec: &mut crate::repository_state::CaptureSpec,
) -> Result<()> {
    documents.into_iter().try_for_each(|document| {
        let requests = crate::document::DocumentReferenceRequests::for_reference(document)?;
        spec.discover_paths(requests.worktree().cloned())?;
        let (revision, path) = requests.pinned();
        Ok(spec.discover_pinned(revision, path)?)
    })
}

/// Resolve each reference without retaining the other references' bytes.
/// `None` restarts the enclosing capture after a read conflict.
fn capture_document_reference_validation(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    issues: &[Issue],
) -> Result<Option<std::result::Result<(), String>>> {
    use crate::repository_state::CaptureSpec;

    for issue in issues {
        for document in &issue.documents {
            let mut spec = CaptureSpec::new([], MUTATION_CAPTURE_BUDGET)?;
            discover_document_references(std::iter::once(document), &mut spec)?;
            let Some(image) = super::capture_or_retry(session.capture(spec))? else {
                return Ok(None);
            };
            if let Some(reason) =
                crate::document::resolve_document_reference(&image, document)?.unresolved()
            {
                return Ok(Some(Err(format!(
                    "Invalid document reference in issue '{}': {reason}",
                    issue.id
                ))));
            }
        }
    }
    Ok(Some(Ok(())))
}

/// Capture every gate-run, verdict and archive-evidence record, read the whole
/// event log, and judge the stored records against `captured`.
/// Whole-repository validation pays these reads; no other capture does.
fn capture_record_fidelity_validation(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    captured: &crate::validation::CapturedValidation<'_>,
) -> Result<Option<std::result::Result<(), String>>> {
    let spec = crate::repository_state::CaptureSpec::new([], RECORD_FIDELITY_CAPTURE_BUDGET)?;
    let Some(records) = super::capture_discovering(session, spec, |image, spec| {
        super::discover_gate_records(image, spec)?;
        discover_archive_evidence_records(image, spec)
    })?
    else {
        return Ok(None);
    };
    let events = session.read_history_log()?;
    let drift =
        crate::validation::record_fidelity::record_drift(captured, &records, events.as_deref())?;
    Ok(Some(match drift.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "{} stored record(s) do not reproduce what the writer would publish:\n  {}",
            drift.len(),
            drift.join("\n  ")
        )),
    }))
}

fn discover_archive_evidence_records(
    image: &crate::repository_state::RepositoryImage,
    spec: &mut crate::repository_state::CaptureSpec,
) -> Result<()> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let root = VirtualPath::ARCHIVE_EVIDENCE;
    spec.discover_paths([root.clone()])?;
    spec.discover_listing(root.clone())?;
    for (directory, listing) in image.directory_listings().iter().filter(|(path, _)| {
        path.root_class() == root.root_class()
            && (path == &&root
                || path.relative().as_path().parent() == Some(root.relative().as_path()))
    }) {
        let paths = listing
            .names()
            .iter()
            .filter(|name| directory == &root || name.ends_with(".json"))
            .map(|name| VirtualPath::data(directory.relative().as_path().join(name)))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        spec.discover_paths(paths)?;
    }
    spec.discover_listings(image.entries().iter().filter_map(|(path, entry)| {
        (path.root_class() == root.root_class()
            && path.relative().as_path().parent() == Some(root.relative().as_path())
            && matches!(entry, RepositoryEntry::Directory { .. }))
        .then_some(path.clone())
    }))?;
    Ok(())
}

/// Each breakable container's linked plan-document path.
fn plan_document_paths(
    issues: &[Issue],
    config: &JitConfig,
) -> Result<Vec<crate::repository_state::VirtualPath>> {
    let by_id = issues.iter().map(|issue| (&issue.id, issue)).collect();
    issues
        .iter()
        .filter_map(|issue| crate::templates::plan_document_ref(issue, &by_id, &config.templates))
        .map(|plan| Ok(crate::repository_state::VirtualPath::worktree(&plan.path)?))
        .collect()
}

/// Plan content for the image's complete issue set. Its closure holds every
/// path [`plan_document_paths`] derives. Invalid configuration yields no plans;
/// the validation report reports the failure.
pub(crate) fn plan_content_with_issues(
    image: &crate::repository_state::RepositoryImage,
    issues: &[Issue],
) -> Result<std::collections::HashMap<IssueId, String>> {
    let config = crate::repository_state::assemble_config(image).unwrap_or_default();
    crate::validation::repository::plan_content(
        &|path| super::image_repo_bytes(image, path),
        issues,
        issues,
        &config.templates,
    )
}

impl<S: IssueStore> ValidationOps<'_, S> {
    pub fn get_status(&self) -> Result<crate::output::StatusResponse> {
        let issues = self.exec.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);

        let backlog = issues.iter().filter(|i| i.state == State::Backlog).count();
        let ready = issues.iter().filter(|i| i.state == State::Ready).count();
        let in_progress = issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();
        let gated = issues.iter().filter(|i| i.state == State::Gated).count();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let rejected = issues.iter().filter(|i| i.state == State::Rejected).count();
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        Ok(crate::output::StatusResponse {
            backlog,
            ready,
            in_progress,
            done,
            rejected,
            blocked,
            gated,
            total: issues.len(),
        })
    }

    /// Validate that the branch still sits on top of `origin/main`.
    ///
    /// Compares the merge-base of HEAD and `origin/main` against the
    /// `origin/main` commit: they agree exactly when `origin/main` is an
    /// ancestor of HEAD. This is git branch drift, a concern separate from the
    /// membership-label-vs-DAG divergence that `jit query divergence` reports.
    ///
    /// # Returns
    /// Ok(()) when the branch is up-to-date, Err naming both commits and the
    /// rebase that reconciles them when it has drifted.
    pub fn validate_branch_drift(&self) -> Result<()> {
        use std::process::Command;

        let layout = self
            .exec
            .gates()
            .real_repo_layout()
            .context("Branch drift needs a repository on disk")?;
        let repository = layout.data_root();

        // Get merge-base between HEAD and origin/main
        let merge_base_output = Command::new("git")
            .args(["merge-base", "HEAD", "origin/main"])
            .current_dir(repository)
            .output()
            .context("Failed to execute git merge-base")?;

        if !merge_base_output.status.success() {
            let stderr = String::from_utf8_lossy(&merge_base_output.stderr);
            anyhow::bail!(
                "Failed to get merge-base with origin/main: {}",
                stderr.trim()
            );
        }

        let merge_base = String::from_utf8(merge_base_output.stdout)?
            .trim()
            .to_string();

        // Get current origin/main commit
        let main_commit_output = Command::new("git")
            .args(["rev-parse", "origin/main"])
            .current_dir(repository)
            .output()
            .context("Failed to execute git rev-parse")?;

        if !main_commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&main_commit_output.stderr);
            anyhow::bail!("Failed to get origin/main commit: {}", stderr.trim());
        }

        let main_commit = String::from_utf8(main_commit_output.stdout)?
            .trim()
            .to_string();

        // If merge-base != main commit, branch has diverged
        if merge_base != main_commit {
            anyhow::bail!(
                "Branch has diverged from origin/main\n\
                 Merge base: {}\n\
                 Main commit: {}\n\
                 Fix: git rebase origin/main",
                merge_base,
                main_commit
            );
        }

        Ok(())
    }

    /// Validate all active leases are consistent and not stale.
    ///
    /// Checks every lease record for:
    /// - Expired leases (TTL exceeded)
    /// - Leases referencing non-existent worktrees
    /// - Leases referencing non-existent issues
    ///
    /// # Returns
    /// Vector of invalid lease descriptions with fix suggestions
    pub fn validate_leases(&self) -> Result<Vec<String>> {
        use crate::storage::claim_coordinator::read_leases;
        use chrono::Utc;
        use std::collections::HashSet;

        let leases = read_leases(self.exec.require_worktree_paths()?)?;

        let mut invalid_leases = Vec::new();
        let now = Utc::now();
        let recorded_worktrees = if leases.is_empty() {
            HashSet::new()
        } else {
            recorded_worktree_ids(self.exec.require_worktree_paths()?)?
        };

        for lease in &leases {
            // Check if lease has expired
            if lease.is_expired(now) {
                if let Some(expires_at) = lease.expires_at {
                    let duration = now.signed_duration_since(expires_at);
                    invalid_leases.push(format!(
                        "Lease {} (Issue {}): Expired {} ago\n  Fix: jit claim release {}",
                        lease.lease_id,
                        lease.issue_id.short_id(),
                        format_duration(duration),
                        lease.issue_id.short_id()
                    ));
                    continue;
                }
            }

            // Check if worktree still exists
            if !recorded_worktrees.contains(&lease.worktree_id) {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Worktree {} no longer exists\n  Fix: {}",
                    lease.lease_id,
                    lease.issue_id.short_id(),
                    lease.worktree_id,
                    force_evict_remedy(lease)
                ));
                continue;
            }

            // Check if issue still exists (use storage layer for proper resolution)
            if self.exec.storage.load_issue(&lease.issue_id).is_err() {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Issue no longer exists\n  Fix: {}",
                    lease.lease_id,
                    lease.issue_id.short_id(),
                    force_evict_remedy(lease)
                ));
            }
        }

        Ok(invalid_leases)
    }
}

/// The command that evicts `lease`; it resolves the issue among stored leases,
/// so it also reaches a lease whose issue was deleted.
fn force_evict_remedy(lease: &crate::storage::claim_coordinator::Lease) -> String {
    format!(
        "jit claim force-evict {} --reason \"<reason>\"",
        lease.issue_id.short_id()
    )
}

/// Format duration in human-readable form
fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        if secs == 1 {
            "1 second".to_string()
        } else {
            format!("{} seconds", secs)
        }
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute".to_string()
        } else {
            format!("{} minutes", mins)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour".to_string()
        } else {
            format!("{} hours", hours)
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{} days", days)
        }
    }
}

/// The ids recorded by this repository's listed checkouts.
fn recorded_worktree_ids(
    paths: &crate::storage::worktree_paths::WorktreePaths,
) -> Result<std::collections::HashSet<crate::domain::identity::WorktreeId>> {
    use std::path::PathBuf;
    use std::process::Command;

    // Get all git worktrees
    let output = Command::new("git")
        .arg("-C")
        .arg(&paths.worktree_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to execute git worktree list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree list failed: {}", stderr);
    }

    let porcelain_output =
        String::from_utf8(output.stdout).context("Invalid UTF-8 in git worktree output")?;

    // A checkout that is gone or whose identity is unreadable records no id.
    Ok(porcelain_output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .filter_map(|worktree_path| paths.for_worktree_root(worktree_path).ok().flatten())
        .filter_map(|checkout| {
            crate::storage::worktree_identity::read_worktree_id(&checkout)
                .ok()
                .flatten()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{seed_issue, MemoryDeclarations};
    use crate::repository_state::VirtualPath;
    use crate::storage::JsonFileStorage;
    use crate::validation::ValidationReport;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    const DEFAULT_LABEL_ASSERTION: &str =
        "assert = { json-schema = \"schemas/default-label-format.json\" }";
    const STALE_LABEL_ASSERTION: &str =
        "assert = { require-label = { label = \"authored:*\", min = 99 } }";

    fn drift_default_assertion(rules: &str) -> (String, String) {
        let stale = format!(
            "# authored header remains byte-exact\n{}",
            rules
                .replacen(
                    "severity = \"error\"",
                    "severity = \"error\" # authored policy",
                    1,
                )
                .replacen(DEFAULT_LABEL_ASSERTION, STALE_LABEL_ASSERTION, 1)
        );
        let repaired = stale.replacen(STALE_LABEL_ASSERTION, DEFAULT_LABEL_ASSERTION, 1);
        (stale, repaired)
    }

    #[test]
    fn test_validate_silent_captures_archive_evidence_for_record_fidelity() {
        use crate::domain::artifact_plan::PlanTarget;
        use crate::repository_state::VirtualPath;
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config("").unwrap();
        for target in [
            PlanTarget::Document {
                path: "docs/plan.md".into(),
            },
            PlanTarget::Container {
                id: crate::test_utils::issue_id(1),
            },
        ] {
            let path = VirtualPath::archive_evidence_record(&target);
            let physical = executor.storage.root().join(path.relative().as_path());
            std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
            std::fs::write(&physical, b"not json").unwrap();
            let error = executor.validation().validate_silent().unwrap_err();
            assert!(
                format!("{error:#}").contains(&path.repository_relative()),
                "{error:#}"
            );
            std::fs::remove_file(physical).unwrap();
        }
    }

    #[test]
    fn test_validate_silent_reads_gate_runs_of_more_issues_than_the_mutation_budget_lists() {
        use crate::repository_state::VirtualPath;
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config("").unwrap();
        let issues = (0..=MUTATION_CAPTURE_BUDGET.max_listings as u128)
            .map(crate::test_utils::issue_id)
            .collect::<Vec<_>>();
        for issue in &issues {
            let directory = VirtualPath::gate_run_issue_directory(issue);
            std::fs::create_dir_all(executor.storage.root().join(directory.relative().as_path()))
                .unwrap();
        }
        let corrupt = VirtualPath::gate_run_record(
            issues.last().unwrap(),
            &crate::test_utils::gate_run_id(1),
        );
        std::fs::write(
            executor.storage.root().join(corrupt.relative().as_path()),
            b"not json",
        )
        .unwrap();

        let error = executor.validation().validate_silent().unwrap_err();

        assert!(
            format!("{error:#}").contains(&corrupt.repository_relative()),
            "{error:#}"
        );
    }

    #[test]
    fn test_validate_silent_file_backend_reports_missing_index_through_captured_image() {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        std::fs::create_dir_all(repo.path().join(".jit/issues")).unwrap();
        std::fs::write(repo.path().join(".jit/config.toml"), "").unwrap();
        std::fs::write(
            repo.path().join(".jit/gates.toml"),
            crate::declarations::serialize_gate_registry(
                &crate::declarations::GateRegistry::default(),
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::write(repo.path().join(".jit/events.jsonl"), "").unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);

        let error = executor.validation().validate_silent().unwrap_err();

        assert!(
            format!("{error:#}").contains("index.json"),
            "missing index must fail through captured-image validation: {error:#}"
        );
    }

    #[test]
    fn test_repair_derived_state_plans_without_decoding_issue_records() {
        let (repo, storage, taxonomy) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let jit_dir = repo.path().join(".jit");
        let config = format!(
            "{}\n\
            {}\n\
            [projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\ntarget = \"AGENTS.md\"\nstyle = \"id-anchor\"\n",
            taxonomy.config_fragment(),
            crate::test_utils::shipped_item_kinds_toml()
        );
        std::fs::write(jit_dir.join("config.toml"), config).unwrap();
        std::fs::write(
            repo.path().join("AGENTS.md"),
            "# Doc\n\n<!-- jit:invariants:begin -->\nSTALE\n<!-- jit:invariants:end -->\n",
        )
        .unwrap();
        // An undecodable record: planning that decoded it would fail.
        std::fs::write(
            jit_dir.join(format!("issues/{}.json", crate::test_utils::issue_id(1))),
            "{",
        )
        .unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);

        let (repaired, _) = executor.validation().repair_derived_state(true).unwrap();

        assert!(repaired >= 1, "the stale projection region must be planned");
    }

    #[test]
    fn test_validate_fix_repairs_stale_derived_projection_through_the_session() {
        let (repo, storage, taxonomy) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let jit_dir = repo.path().join(".jit");
        let config = format!(
            "{}\n\
            {}\n\
            [projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\ntarget = \"AGENTS.md\"\nstyle = \"id-anchor\"\n",
            taxonomy.config_fragment(),
            crate::test_utils::shipped_item_kinds_toml()
        );
        std::fs::write(jit_dir.join("config.toml"), config).unwrap();
        std::fs::write(
            repo.path().join(".jit/invariants.toml"),
            "[[invariants]]\nid = \"sample\"\nstatement = \"Stay acyclic.\"\nkind = \"advisory\"\n",
        )
        .unwrap();
        std::fs::write(
            repo.path().join("AGENTS.md"),
            "# Doc\n\n<!-- jit:invariants:begin -->\nSTALE\n<!-- jit:invariants:end -->\n",
        )
        .unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);

        let (fixes, _messages, _, _) = executor.validation().validate_with_fix(false).unwrap();
        assert!(fixes >= 1, "the stale projection region must be repaired");
        let agents = std::fs::read_to_string(repo.path().join("AGENTS.md")).unwrap();
        assert!(
            !agents.contains("STALE"),
            "the stale region must be rewritten: {agents}"
        );
        assert!(
            agents.contains("sample"),
            "the invariant must render into the repaired region: {agents}"
        );
        // A second --fix run is a no-op: the derived state is already coherent.
        let (again, _, _, _) = executor.validation().validate_with_fix(false).unwrap();
        assert_eq!(again, 0, "repair must be idempotent");
    }

    #[test]
    fn test_validate_fix_rejects_ambiguous_region_without_writing_other_repairs() {
        let repo = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));

        let rules_path = repo.path().join(".jit/rules.toml");
        let agents_path = repo.path().join("AGENTS.md");
        let rules = std::fs::read_to_string(&rules_path).unwrap();
        let agents = std::fs::read_to_string(&agents_path).unwrap();
        std::fs::write(&rules_path, drift_default_assertion(&rules).0).unwrap();
        std::fs::write(
            &agents_path,
            agents.replacen(
                "<!-- jit:invariants:begin -->",
                "<!-- jit:invariants:begin -->\n<!-- jit:invariants:begin -->",
                1,
            ),
        )
        .unwrap();
        let before_rules = std::fs::read(&rules_path).unwrap();
        let before_agents = std::fs::read(&agents_path).unwrap();

        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        let (_, _, report, _) = executor.validation().validate_with_fix(false).unwrap();
        let error = report.structural_failure.unwrap();
        assert!(format!("{error:#}").contains("invariants"), "{error:#}");
        assert_eq!(std::fs::read(&rules_path).unwrap(), before_rules);
        assert_eq!(std::fs::read(&agents_path).unwrap(), before_agents);
    }

    #[test]
    fn test_validate_fix_rejects_ambiguous_region_without_memory_writes() {
        use crate::commands::test_helpers::{memory_executor_declaring, seed_repo_file};
        use crate::storage::IssueStore;

        let (_source, storage) =
            crate::test_utils::profiled_in_memory_repository_fixture("jit-dogfood").unwrap();
        let rules =
            drift_default_assertion(&storage.read_repo_file(".jit/rules.toml").unwrap().unwrap()).0;
        let agents = storage
            .read_repo_file("AGENTS.md")
            .unwrap()
            .unwrap()
            .replacen(
                "<!-- jit:invariants:begin -->",
                "<!-- jit:invariants:begin -->\n<!-- jit:invariants:begin -->",
                1,
            );
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                rules: Some(&rules),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        seed_repo_file(&storage, "AGENTS.md", &agents);

        let (_, _, report, _) = executor.validation().validate_with_fix(false).unwrap();
        let error = report.structural_failure.unwrap();
        assert!(format!("{error:#}").contains("invariants"), "{error:#}");
        assert_eq!(
            storage.read_repo_file(".jit/rules.toml").unwrap().unwrap(),
            rules
        );
        assert_eq!(
            storage.read_repo_file("AGENTS.md").unwrap().unwrap(),
            agents
        );
    }

    /// REQ-01: repository validation reports a drifted profile-owned target as
    /// nothing. The target is an ordinary repository value, and git shows it.
    #[test]
    fn test_validate_silent_accepts_a_drifted_profile_owned_target() {
        let repo = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        executor
            .validation()
            .validate_silent()
            .expect("the coherent profiled repository validates");

        std::fs::write(
            repo.path().join(".agents/skills/jit-manage/SKILL.md"),
            "EDITED PROFILE ASSET\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_silent()
            .expect("a drifted profile-owned target is not a validation finding");
    }

    /// REQ-02: validation repair restores no profile-owned target, so a drifted
    /// one is left as the repository holds it.
    #[test]
    fn test_validate_fix_leaves_a_drifted_profile_owned_target_unrepaired() {
        let repo = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        let asset = repo.path().join(".agents/skills/jit-manage/SKILL.md");
        std::fs::write(&asset, "EDITED PROFILE ASSET\n").unwrap();

        let (repaired, _, _, _) = executor.validation().validate_with_fix(false).unwrap();

        assert_eq!(repaired, 0, "no profile-owned target is a repair action");
        assert_eq!(
            std::fs::read_to_string(&asset).unwrap(),
            "EDITED PROFILE ASSET\n"
        );
    }

    #[test]
    fn test_validate_fix_reports_malformed_records_without_publishing_repairs() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        let storage = crate::storage::InMemoryStorage::new();
        let executor = memory_executor(storage.clone());
        let path = format!(".jit/issues/{}.json", crate::test_utils::issue_id(1));
        seed_repo_file(&storage, &path, "{");
        let (count, _, report, _) = executor.validation().validate_with_fix(false).unwrap();
        assert_eq!(count, 0);
        assert!(report.structural_failure.is_some());
        assert_eq!(storage.read_repo_file(&path).unwrap().as_deref(), Some("{"));
    }

    #[test]
    fn test_validate_fix_skips_declaration_dependent_repairs_once_and_keeps_other_fixes() {
        use crate::commands::test_helpers::{
            memory_executor_declaring, seed_issue, seed_repo_file,
        };
        let storage = crate::storage::InMemoryStorage::new();
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                config: Some("[worktree]\nenforce_leases = \"off\"\n[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n"),
                ..MemoryDeclarations::default()
            },
        );
        seed_repo_file(&storage, ".jit/gates.toml", "not valid TOML");
        let mut leaf = crate::test_utils::fixture_issue("leaf".into(), String::new());
        leaf.state = State::Done;
        let mut middle = crate::test_utils::fixture_issue("middle".into(), String::new());
        middle.state = State::Done;
        middle.dependencies = vec![leaf.id.clone()];
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.state = State::Backlog;
        source.labels = vec!["type:taks".into()];
        source.dependencies = vec![middle.id.clone(), leaf.id.clone()];
        for issue in [&source, &middle, &leaf] {
            seed_issue(&storage, issue.clone());
        }

        let (_, messages, report, _) = executor.validation().validate_with_fix(false).unwrap();

        let skipped = messages
            .iter()
            .filter(|message| message.starts_with("Skipped"))
            .collect::<Vec<_>>();
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("hierarchy") && skipped[0].contains("derived-state"));
        assert!(skipped[0].contains("gates.toml"));
        let fixed = storage.load_issue(&source.id).unwrap();
        assert_eq!(fixed.labels, source.labels);
        assert_eq!(fixed.dependencies, vec![middle.id]);
        assert_eq!(fixed.state, State::Ready);
        assert!(report.structural_failure.is_some());
    }

    #[test]
    fn test_validate_fix_repairs_readiness_with_unloadable_rules_and_reports_the_finding() {
        use crate::commands::test_helpers::{memory_executor_declaring, seed_issue};
        let storage = crate::storage::InMemoryStorage::new();
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                rules: Some("not valid TOML"),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let mut issue = crate::test_utils::fixture_issue("unblocked".into(), String::new());
        issue.state = State::Backlog;
        seed_issue(&storage, issue.clone());

        let (count, messages, report, _) = executor.validation().validate_with_fix(false).unwrap();

        assert!(count > 0);
        assert_eq!(storage.load_issue(&issue.id).unwrap().state, State::Ready);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.reported.rule == "rules-file"));
        assert!(messages.iter().any(|message| {
            message.contains("Skipped derived-state repair") && message.contains("rules")
        }));
    }

    #[test]
    fn test_validate_fix_publishes_every_fix_class_once_and_dry_run_publishes_nothing() {
        use crate::commands::test_helpers::{
            memory_executor_declaring, seed_issue, seed_repo_file,
        };
        use crate::storage::{IssueStore, MutationPoint};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        #[derive(Default)]
        struct Publications(AtomicUsize);
        impl crate::storage::MutationInterruptionInjector for Publications {
            fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
                if *point == MutationPoint::BeforeCommitCheck {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            }
        }
        let (_source, storage) =
            crate::test_utils::profiled_in_memory_repository_fixture("jit-dogfood").unwrap();
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let mut leaf = crate::test_utils::fixture_issue("leaf".into(), String::new());
        leaf.state = State::Done;
        let mut middle = crate::test_utils::fixture_issue("middle".into(), String::new());
        middle.state = State::Done;
        middle.dependencies = vec![leaf.id.clone()];
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.labels = vec!["type:taks".into()];
        source.state = State::Backlog;
        source.dependencies = vec![middle.id.clone(), leaf.id.clone()];
        for issue in [&source, &middle, &leaf] {
            seed_issue(&storage, issue.clone());
        }
        let agents = storage.read_repo_file("AGENTS.md").unwrap().unwrap();
        seed_repo_file(
            &storage,
            "AGENTS.md",
            &agents.replacen(
                "<!-- jit:invariants:begin -->",
                "<!-- jit:invariants:begin -->\nSTALE",
                1,
            ),
        );
        let publications = Arc::new(Publications::default());
        let storage = storage.with_mutation_interruption_view(publications.clone());
        let executor =
            CommandExecutor::new(storage.clone()).with_layout(executor.require_layout().unwrap());
        let (preview, _, _, _) = executor.validation().validate_with_fix(true).unwrap();
        assert_eq!(publications.0.load(Ordering::SeqCst), 0);
        assert_eq!(storage.load_issue(&source.id).unwrap(), source);
        let (fixed, _, _, _) = executor.validation().validate_with_fix(false).unwrap();
        assert_eq!(fixed, preview);
        assert_eq!(publications.0.load(Ordering::SeqCst), 1);
        let fixed = storage.load_issue(&source.id).unwrap();
        assert!(fixed.labels.contains(&"type:task".into()));
        assert_eq!(fixed.dependencies, vec![middle.id]);
        assert_eq!(fixed.state, State::Ready);
        assert!(!storage
            .read_repo_file("AGENTS.md")
            .unwrap()
            .unwrap()
            .contains("STALE"));
    }

    #[test]
    fn test_validate_type_fix_is_noop_after_captured_repair() {
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        let config = "[worktree]\nenforce_leases = \"off\"\n\
             [type_hierarchy.types]\ntask = 4\n\
             [namespaces.type]\ndescription = \"Issue type\"\nunique = true\n";
        let executor = crate::commands::test_helpers::memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                config: Some(config),
                ..MemoryDeclarations::default()
            },
        );
        let id = executor
            .issues()
            .create_issue(
                "Type fix no-op".to_string(),
                String::new(),
                Priority::Normal,
                Vec::new(),
                vec!["type:taks".to_string()],
                None,
                None,
                true,
            )
            .unwrap()
            .issue
            .id;

        let (first, _, _, _) = executor.validation().validate_with_fix(false).unwrap();
        assert!(first > 0);
        let repaired = storage.load_issue(&id).unwrap();
        assert!(repaired.labels.iter().any(|label| label == "type:task"));
        assert!(!repaired.labels.iter().any(|label| label == "type:taks"));

        let (second, _, _, _) = executor.validation().validate_with_fix(false).unwrap();
        assert_eq!(second, 0, "a captured type repair must not repeat");
    }

    #[test]
    fn test_validate_fix_rederives_and_preserves_concurrent_type_correction() {
        use crate::commands::test_helpers::{memory_executor, with_apply_race, SessionRaceAction};
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        let executor = crate::commands::test_helpers::memory_executor_declaring(storage.clone(), MemoryDeclarations {
            config: Some("[worktree]\nenforce_leases = \"off\"\n[type_hierarchy.types]\ntask = 4\nstory = 3\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n"),
            ..MemoryDeclarations::default()
        });
        let id = executor
            .issues()
            .create_issue(
                "Type fix race".to_string(),
                String::new(),
                Priority::Normal,
                Vec::new(),
                vec!["type:taks".to_string()],
                None,
                None,
                true,
            )
            .unwrap()
            .issue
            .id;

        let mut issue = storage.load_issue(&id).unwrap();
        issue.labels = vec!["type:story".to_string()];
        let raced = with_apply_race(storage, SessionRaceAction::Save(Box::new(issue)));
        let executor = memory_executor(raced.clone());

        executor.validation().validate_with_fix(false).unwrap();

        assert_eq!(raced.load_issue(&id).unwrap().labels, ["type:story"]);
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(Duration::seconds(1)), "1 second");
        assert_eq!(format_duration(Duration::seconds(30)), "30 seconds");
        assert_eq!(format_duration(Duration::seconds(59)), "59 seconds");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(Duration::seconds(60)), "1 minute");
        assert_eq!(format_duration(Duration::seconds(90)), "1 minute");
        assert_eq!(format_duration(Duration::seconds(120)), "2 minutes");
        assert_eq!(format_duration(Duration::seconds(3599)), "59 minutes");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(Duration::seconds(3600)), "1 hour");
        assert_eq!(format_duration(Duration::seconds(3700)), "1 hour");
        assert_eq!(format_duration(Duration::seconds(7200)), "2 hours");
        assert_eq!(format_duration(Duration::seconds(86399)), "23 hours");
    }

    #[test]
    fn test_format_duration_days() {
        assert_eq!(format_duration(Duration::seconds(86400)), "1 day");
        assert_eq!(format_duration(Duration::seconds(90000)), "1 day");
        assert_eq!(format_duration(Duration::seconds(172800)), "2 days");
        assert_eq!(format_duration(Duration::seconds(604800)), "7 days");
    }

    // --- dangling-item-link pass (REQ-08 clause i, REQ-03) --------------------

    use crate::domain::Issue;
    use crate::storage::{InMemoryStorage, IssueStore};

    const REGISTRY_TOML: &str = "\
[[invariants]]
id = \"sample-invariant\"
statement = \"Every dependency edge stays acyclic.\"
kind = \"enforced\"
";

    fn issue_with_labels(title: &str, body: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.to_string(), body.to_string());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue
    }

    #[test]
    fn test_validate_silent_fails_on_dangling_link() {
        // REQ-03 via the validate PATH: a dangling link makes `validate_silent`
        // (the gate / `jit validate` core) return an error mentioning the rule.
        let target =
            issue_with_labels("target", "## Success Criteria\n\n- [hard] REQ-01: a\n", &[]);
        let short = target.short_id();
        let node = issue_with_labels("node", "", &[&format!("satisfies:{short}/BOGUS")]);
        // Wire a dependency so the two issues are not isolated nodes (which would
        // fail integrity before the rule pass).
        let mut node = node;
        node.dependencies = vec![target.id.clone()];
        let mut target = target;
        target.state = State::Ready;
        // Register the `satisfies` namespace so the dangling-link pass — not the
        // namespace-registry local rule — is the validation failure.
        let config = format!(
            "[namespaces.type]\ndescription = \"issue type\"\nunique = true\n\
             [namespaces.satisfies]\ndescription = \"satisfied item\"\nunique = false\n\
             [namespaces.enforces]\ndescription = \"enforced invariant\"\nunique = false\n\
             {}",
            crate::test_utils::shipped_item_kind_tables(&["invariant", "requirement"])
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(REGISTRY_TOML),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), target);
        seed_issue(exec.storage(), node);

        let err = exec.validation().validate_silent().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(crate::validation::report::BuiltInPass::DanglingItemLink.as_str())
                && msg.contains("BOGUS"),
            "expected dangling-link rule failure, got: {msg}"
        );
    }

    /// Shared bases exclude references; a consuming pass requests its own scope.
    #[test]
    fn test_capture_proposed_base_scopes_document_references_to_the_consuming_pass() {
        use crate::commands::test_helpers::{seed_issue, setup};
        use crate::repository_state::VirtualPath;
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        let issue = crate::test_utils::fixture_issue("Doc".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .documents()
            .add_document_reference(&issue_id, "dev/notes.md", None, None, None, true)
            .unwrap();
        executor
            .documents()
            .add_document_reference(&issue_id, "dev/pinned.md", Some("HEAD"), None, None, true)
            .unwrap();
        let document = VirtualPath::worktree("dev/notes.md").unwrap();
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();

        let validation = executor
            .capture_proposed_base(
                session.as_mut(),
                &std::collections::BTreeMap::new(),
                &[],
                None,
            )
            .unwrap()
            .expect("a settled capture");
        let mutation = executor
            .capture_mutation_base(session.as_mut(), &[], None)
            .unwrap()
            .expect("a settled capture")
            .image;

        assert!(!validation.capture_spec().contains_path(&document));
        assert!(validation.capture_spec().pinned().is_empty());
        let mut selected = executor.storage().load_issue(&issue_id).unwrap();
        selected.documents.truncate(1);
        let scoped =
            super::super::grow_discovering(session.as_mut(), validation.clone(), |_, spec| {
                discover_document_references(&selected.documents, spec)
            })
            .unwrap()
            .expect("a settled scoped capture");
        assert!(scoped.capture_spec().contains_path(&document));
        assert!(scoped
            .capture_spec()
            .pinned()
            .contains(&("HEAD".into(), "dev/notes.md".into())));
        assert!(!scoped
            .capture_spec()
            .pinned()
            .contains(&("HEAD".into(), "dev/pinned.md".into())));
        assert!(!mutation.capture_spec().contains_path(&document));
        assert!(mutation.capture_spec().pinned().is_empty());
    }

    #[test]
    fn test_capture_proposed_base_inner_reads_only_the_event_log_tail_for_every_purpose() {
        use crate::commands::test_helpers::{seed_gate_registry, seed_issue, setup};
        use crate::declarations::{GateChecker, GateDefinition, GateMode, GateRegistry, GateStage};
        use crate::repository_state::{ReadMode, VirtualPath};
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        let key: crate::domain::identity::GateKey = "whole-repository".parse().unwrap();
        let mut registry = GateRegistry::default();
        registry.gates.insert(
            key.clone(),
            GateDefinition {
                version: 1,
                key: key.clone(),
                title: "Whole repository".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::RepositoryValidation),
                priority: 100,
                reserved: std::collections::HashMap::new(),
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::test_utils::fixture_issue("Target".to_string(), String::new());
        issue.gates_required = vec![key];
        let target = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();

        for (purpose, expected) in [
            (BaseCapturePurpose::Validation, ReadMode::Tail),
            (
                BaseCapturePurpose::Mutation {
                    precheck_target: Some(&target),
                },
                ReadMode::Tail,
            ),
            (BaseCapturePurpose::Repair, ReadMode::Tail),
            (BaseCapturePurpose::Archive, ReadMode::Tail),
        ] {
            let (image, _) = executor
                .capture_proposed_base_inner(
                    session.as_mut(),
                    None,
                    &std::collections::BTreeMap::new(),
                    &[],
                    purpose,
                )
                .unwrap()
                .expect("a settled capture");
            assert_eq!(
                image.capture_spec().read_mode(&VirtualPath::EVENTS),
                Some(expected)
            );
        }
    }

    const CUSTOM_PLAN_TEMPLATE: &str = r#"
[[template]]
name = "custom-plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
doc = "receipts/large.log"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
labels = ["scaffold:{container.short_id}"]
depends_on = ["planning"]
"#;

    #[test]
    fn test_capture_repair_plan_ignores_document_larger_than_capture_budget() {
        use crate::commands::test_helpers::{
            memory_executor_declaring, seed_issue, seed_repo_file,
        };
        use crate::storage::{RepositoryMutationSession, RepositoryStateStore};

        let executor = memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some("[worktree]\nenforce_leases = \"off\"\n"),
                templates: Some(CUSTOM_PLAN_TEMPLATE),
                ..MemoryDeclarations::default()
            },
        );
        let mut issue = crate::test_utils::fixture_issue("Large receipt".into(), String::new());
        issue.labels = vec!["type:planning".into()];
        let mut document = crate::test_utils::document_reference("receipts/large.log");
        document.label = Some("plan".into());
        issue.documents.push(document);
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.labels = vec!["type:epic".into()];
        let mut breakdown = crate::test_utils::fixture_issue("Breakdown".into(), String::new());
        breakdown.labels = vec![
            "type:breakdown".into(),
            format!("scaffold:{}", container.short_id()),
        ];
        breakdown.dependencies.push(issue.id.clone());
        container.dependencies.push(breakdown.id.clone());
        seed_issue(executor.storage(), issue);
        seed_issue(executor.storage(), container);
        seed_issue(executor.storage(), breakdown);
        let max_bytes = 64 * 1024;
        seed_repo_file(
            executor.storage(),
            "receipts/large.log",
            &"x".repeat(max_bytes * 2),
        );
        seed_repo_file(
            executor.storage(),
            ".jit/schemas/default-label-format.json",
            "false",
        );
        let mut session = crate::commands::test_helpers::LimitedCaptureSession {
            inner: executor
                .storage()
                .open_mutation_session(executor.require_layout().unwrap())
                .unwrap(),
            max_bytes: max_bytes as u64,
        };
        let captured = executor
            .validation()
            .capture_repair_plan(&mut session)
            .unwrap()
            .expect("settled capture")
            .unwrap();
        let plan = captured.plan.unwrap();
        assert!(!plan.image().capture_spec().contains_path(
            &crate::repository_state::VirtualPath::worktree("receipts/large.log").unwrap()
        ));
        assert!(plan.image().capture_spec().pinned().is_empty());
        assert!(
            !plan.delta().actions().is_empty(),
            "the stale schema needs repair"
        );
        session.apply(&plan).unwrap();
        assert_ne!(
            executor
                .storage()
                .read_repo_file(".jit/schemas/default-label-format.json")
                .unwrap()
                .as_deref(),
            Some("false")
        );
    }

    #[test]
    fn test_capture_document_reference_validation_bounds_each_reference_independently() {
        use crate::commands::test_helpers::{seed_issue, seed_repo_file, setup};
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        let mut issue = crate::test_utils::fixture_issue("Receipts".into(), String::new());
        let max_bytes = 64 * 1024;
        let content = "x".repeat(max_bytes * 3 / 4);
        for path in ["receipts/first.log", "receipts/second.log"] {
            seed_repo_file(executor.storage(), path, &content);
            issue
                .documents
                .push(crate::test_utils::document_reference(path));
        }
        assert!(issue.documents.len() * content.len() > max_bytes);
        seed_issue(executor.storage(), issue.clone());
        let mut session = crate::commands::test_helpers::LimitedCaptureSession {
            inner: executor
                .storage()
                .open_mutation_session(executor.require_layout().unwrap())
                .unwrap(),
            max_bytes: max_bytes as u64,
        };
        capture_document_reference_validation(&mut session, &[issue])
            .unwrap()
            .expect("settled captures")
            .expect("every reference resolves within its own budget");
    }

    /// Preflight reads only the lease configuration and resolved issue.
    #[test]
    fn test_capture_mutation_preflight_reads_lease_inputs_and_target_only() {
        use crate::commands::test_helpers::{seed_issue, setup};
        use crate::repository_state::VirtualPath;
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        let target = crate::test_utils::fixture_issue("Target".to_string(), String::new());
        let other = crate::test_utils::fixture_issue("Other".to_string(), String::new());
        let (target_id, other_id) = (target.id.clone(), other.id.clone());
        seed_issue(executor.storage(), target);
        seed_issue(executor.storage(), other);
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();

        let preflight = executor
            .capture_mutation_preflight(session.as_mut(), &target_id, |_| false)
            .unwrap()
            .expect("a settled capture");

        let spec = preflight.image.capture_spec();
        assert_eq!(preflight.target.id, target_id);
        assert!(spec.contains_path(&VirtualPath::CONFIG));
        assert!(!spec.contains_path(&VirtualPath::GATES));
        assert!(!spec.contains_path(&VirtualPath::INVARIANTS));
        assert!(!spec.contains_path(&VirtualPath::EVENTS));
        assert!(!spec.contains_path(&VirtualPath::issue_record(&other_id)));
    }

    /// A session counting the capture rounds it opens.
    struct RoundCounter(Box<dyn crate::storage::RepositoryMutationSession>, usize);

    mod round_counter {
        use super::RoundCounter;
        use crate::repository_state::{CaptureSpec, MaterializationPlan, RepositoryImage};
        use crate::storage::{RepositoryApplyOutcome, RepositoryStateStoreError as Error};

        impl crate::storage::RepositoryMutationSession for RoundCounter {
            fn layout(&self) -> &crate::repository_state::RepositoryLayout {
                self.0.layout()
            }

            fn recovery_report(&self) -> &crate::storage::RecoveryDispatchReport {
                self.0.recovery_report()
            }

            fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, Error> {
                self.1 += 1;
                self.0.capture(spec)
            }

            fn extend(
                &mut self,
                image: RepositoryImage,
                spec: CaptureSpec,
            ) -> Result<RepositoryImage, Error> {
                self.0.extend(image, spec)
            }

            fn apply(
                &mut self,
                plan: &MaterializationPlan,
            ) -> Result<RepositoryApplyOutcome, Error> {
                self.0.apply(plan)
            }

            fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, Error> {
                self.0.read_history_log()
            }
        }
    }

    /// An executor holding a `Ready` target and another issue, the target's
    /// id, and a counter over a fresh session.
    fn counted_session() -> (
        CommandExecutor<crate::storage::InMemoryStorage>,
        IssueId,
        RoundCounter,
    ) {
        use crate::commands::test_helpers::{seed_issue, setup};
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        let mut issues = ["Target", "Other"]
            .map(|title| crate::test_utils::fixture_issue(title.to_string(), String::new()));
        issues[0].state = crate::domain::State::Ready;
        let target_id = issues[0].id.clone();
        issues
            .into_iter()
            .for_each(|issue| seed_issue(executor.storage(), issue));
        let layout = executor.require_layout().unwrap();
        let session = executor.storage().open_mutation_session(layout).unwrap();
        (executor, target_id, RoundCounter(session, 0))
    }

    /// The base grows through the session's one round: what the registries
    /// name is read beside them, never in a round of its own.
    #[test]
    fn test_capture_mutation_base_reads_in_one_round() {
        let (executor, target_id, mut counter) = counted_session();

        let base = executor
            .capture_mutation_base(&mut counter, &[], Some(&target_id))
            .unwrap()
            .expect("a settled capture");

        assert_eq!(base.issues.len(), 2);
        assert_eq!(counter.1, 1);
    }

    #[test]
    fn test_capture_mutation_preflight_reads_in_one_round() {
        let (executor, target_id, mut counter) = counted_session();

        executor
            .capture_mutation_preflight(&mut counter, &target_id, |_| false)
            .unwrap()
            .expect("a settled capture");

        assert_eq!(counter.1, 1);
    }

    /// A transition that runs prechecks grows its preflight to the full base
    /// within the preflight's one round.
    #[test]
    fn test_capture_mutation_preflight_precheck_transition_reads_base_in_one_round() {
        let (executor, target_id, mut counter) = counted_session();
        let start = crate::commands::CapturedLifecycleMutation::Claim {
            issue_id: target_id.clone(),
            assignee: "agent:worker".parse().unwrap(),
        };

        let preflight = executor
            .capture_mutation_preflight(&mut counter, &target_id, |target| {
                crate::commands::lifecycle_requires_prechecks(&start, target)
            })
            .unwrap()
            .expect("a settled capture");

        assert_eq!(preflight.issues.map(|issues| issues.len()), Some(2));
        assert_eq!(counter.1, 1);
    }

    fn seed_fixture_issue(
        executor: &mut crate::commands::CommandExecutor<JsonFileStorage>,
        title: &str,
        description: &str,
        labels: Vec<String>,
    ) {
        executor
            .issues()
            .create_issue(
                title.to_string(),
                description.to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                labels,
                None,
                None,
                false,
            )
            .unwrap();
    }

    fn data_path(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        path: &VirtualPath,
    ) -> PathBuf {
        executor.require_layout().unwrap().resolve(path).unwrap()
    }

    fn issue_path(id: &IssueId) -> VirtualPath {
        VirtualPath::issue_record(id)
    }

    /// Read a repo-relative fixture file's bytes, or `None` when absent.
    fn read_fixture(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        repo_rel: &str,
    ) -> Option<Vec<u8>> {
        let layout = executor.require_layout().unwrap();
        let path = layout
            .resolve(&layout.classify_repository_relative(repo_rel).unwrap())
            .unwrap();
        std::fs::read(path).ok()
    }

    /// Capture and validate the live fixture repository.
    fn validate_live(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
    ) -> std::result::Result<ValidationReport, ValidationReport> {
        split(executor.validation().validate_repository_report().unwrap())
    }

    fn capture_overlaid_image(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        overrides: &std::collections::BTreeMap<VirtualPath, Option<Vec<u8>>>,
        finish: impl Fn(&RepositoryImage) -> anyhow::Result<RepositoryImage>,
    ) -> RepositoryImage {
        with_mutation_session(
            executor.storage(),
            &executor.require_layout().unwrap(),
            "overlay fixture capture",
            |session| {
                let Some(base) = executor.capture_proposed_base(session, overrides, &[], None)?
                else {
                    return Ok(SessionStep::Retry);
                };
                let proposed = crate::repository_state::apply_overlay(&base, overrides.clone())?;
                Ok(SessionStep::Done(finish(&proposed)?))
            },
        )
        .unwrap()
    }

    /// Capture and validate the fixture overlaid with proposed final bytes/absence.
    fn validate_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<ValidationReport, ValidationReport> {
        let overrides = crate::commands::overrides_from_repo_changes(
            &executor.require_layout().unwrap(),
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        if overrides.is_empty() {
            return split(executor.validation().selection_report().unwrap());
        }
        let image = capture_overlaid_image(executor, &overrides, |proposed| Ok(proposed.clone()));
        split(crate::validation::report(
            &crate::validation::CapturedValidation::from_image(&image),
            &crate::validation::ReportInputs {
                now: Utc::now(),
                history: None,
            },
        ))
    }

    /// Validate a semantic proposal together with the exact coupled derived-state
    /// writes the canonical materializer would publish for that proposal.
    fn validate_materialized_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<ValidationReport, ValidationReport> {
        let layout = executor.require_layout().unwrap();
        let overrides = crate::commands::overrides_from_repo_changes(
            &layout,
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        let image = capture_overlaid_image(executor, &overrides, |proposed| {
            let declarations = crate::repository_state::declarations_from_image(proposed)?;
            let plan = crate::repository_state::derive_materialization(
                proposed,
                crate::repository_state::MaterializationRequest::RepairDerivedState {
                    declarations: declarations.borrowed(),
                },
            )?;
            Ok(crate::repository_state::apply_overlay(
                proposed,
                crate::repository_state::action_overrides(plan.delta().actions()),
            )?)
        });
        split(crate::validation::report(
            &crate::validation::CapturedValidation::from_image(&image),
            &crate::validation::ReportInputs {
                now: Utc::now(),
                history: None,
            },
        ))
    }

    /// `Err` when `report` carries a structural failure.
    fn split(report: ValidationReport) -> std::result::Result<ValidationReport, ValidationReport> {
        match report.structural_failure {
            Some(_) => Err(report),
            None => Ok(report),
        }
    }

    /// A report's structural failure with its causes.
    fn failure(report: &ValidationReport) -> String {
        format!("{:#}", report.structural_failure.as_ref().unwrap())
    }

    /// A string-valued overlay change (create/replace with UTF-8 text).
    fn put(path: impl Into<String>, text: &str) -> (String, Option<Vec<u8>>) {
        (path.into(), Some(text.as_bytes().to_vec()))
    }

    /// Seed one issue holding an attested gate, and name its run.
    fn seed_attested_gate_run(
        executor: &mut crate::commands::CommandExecutor<JsonFileStorage>,
    ) -> (
        crate::domain::identity::IssueId,
        crate::domain::identity::GateRunId,
    ) {
        seed_fixture_issue(
            executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let key: GateKey = "attested".parse().unwrap();
        executor
            .gates()
            .define_gate(
                &key,
                "Attested".to_string(),
                String::new(),
                crate::declarations::GateStage::Postcheck,
                crate::declarations::GateMode::Manual,
                None,
                100,
            )
            .unwrap();
        let issue = executor.storage().list_issues().unwrap().remove(0);
        executor.gates().add_gate(&issue.id, &key).unwrap();
        executor
            .gates()
            .pass_gate(&issue.id, &key, Some("human:test".to_string()), false)
            .unwrap();
        let run_id = executor
            .storage()
            .load_issue(&issue.id)
            .unwrap()
            .gates_status[&key]
            .run_id
            .clone()
            .unwrap();
        (issue.id, run_id)
    }

    #[test]
    fn test_live_and_identity_overlay_preserve_validation_results() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let plain = validate_live(&executor).unwrap();
        let planned = validate_overlaid::<&str>(&executor, []).unwrap();
        assert_eq!(plain.passes, planned.passes);
        assert_eq!(plain.rule_report(), planned.rule_report());
        assert_eq!(plain.passes.len(), 8);
    }

    #[test]
    fn test_validate_live_reports_a_gate_run_that_does_not_reproduce() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        let path = VirtualPath::gate_run_record(&issue_id, &run_id);
        let stored = read_fixture(&executor, &path.repository_relative()).unwrap();
        let mut raw: serde_json::Value = serde_json::from_slice(&stored).unwrap();
        raw["retired_key"] = serde_json::Value::Bool(true);
        raw["origin"] = serde_json::json!({ "kind": "not-a-recorded-origin" });
        std::fs::write(
            data_path(&executor, &path),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();

        let error = failure(&validate_live(&executor).unwrap_err());

        assert!(
            error.contains("record-fidelity") && error.contains(run_id.as_str()),
            "{error}"
        );
    }

    #[test]
    fn test_validate_live_reports_a_verdict_record_that_does_not_reproduce() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        let path = VirtualPath::gate_verdict_record(
            &"tests".parse().unwrap(),
            &crate::domain::repository_inputs::InputsDigest::try_from("d".repeat(64)).unwrap(),
        );
        let file = data_path(&executor, &path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let record = crate::domain::GateVerdictRecord { issue_id, run_id };
        std::fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();

        let error = failure(&validate_live(&executor).unwrap_err());

        assert!(
            error.contains("record-fidelity") && error.contains(&path.repository_relative()),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_validation_does_not_read_gate_run_records() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        std::fs::write(
            data_path(&executor, &VirtualPath::gate_run_record(&issue_id, &run_id)),
            "not json",
        )
        .unwrap();

        validate_overlaid::<&str>(&executor, []).unwrap();
    }

    #[test]
    fn test_overlay_config_ignores_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(data_path(&executor, &VirtualPath::CONFIG), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(VirtualPath::CONFIG.repository_relative(), "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n")],
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_templates_ignore_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(
            data_path(&executor, &VirtualPath::TEMPLATES),
            "not toml = [",
        )
        .unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(
                VirtualPath::TEMPLATES.repository_relative(),
                "templates = []\n"
            )]
        )
        .is_ok());
        assert!(validate_live(&executor).is_err());
    }

    #[test]
    fn test_overlay_rules_ignore_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let rules = read_fixture(&executor, &VirtualPath::RULES.repository_relative()).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::RULES), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [(VirtualPath::RULES.repository_relative(), Some(rules))]
        )
        .is_ok());
        assert!(validate_live(&executor)
            .unwrap()
            .rule_report()
            .findings
            .iter()
            .any(|finding| finding.rule == "rules-file"));
    }

    #[test]
    fn test_overlay_rules_and_referenced_schema_ignore_disagreeing_live_schema() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let schema_path = VirtualPath::schema("planned.json").unwrap();
        let rules = format!(
            "[[rules]]\nname = \"planned-schema\"\nseverity = \"error\"\nassert = {{ json-schema = {:?} }}\n",
            schema_path.relative().as_str(),
        );
        std::fs::create_dir_all(data_path(&executor, &VirtualPath::SCHEMAS)).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::RULES), &rules).unwrap();
        std::fs::write(data_path(&executor, &schema_path), "not json").unwrap();
        assert!(validate_materialized_overlaid(
            &executor,
            [
                put(VirtualPath::RULES.repository_relative(), &rules),
                put(schema_path.repository_relative(), "{}"),
            ],
        )
        .is_ok());
        assert!(validate_live(&executor)
            .unwrap()
            .rule_report()
            .findings
            .iter()
            .any(|finding| finding.rule == "rules-file"
                && finding.message.contains("not valid JSON")));
    }

    #[test]
    fn test_overlay_gate_registry_ignores_disagreeing_live_registry() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(data_path(&executor, &VirtualPath::GATES), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(VirtualPath::GATES.repository_relative(), "")]
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_index_ignores_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let index = read_fixture(&executor, &VirtualPath::INDEX.repository_relative()).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::INDEX), "not json").unwrap();
        assert!(validate_overlaid(
            &executor,
            [(VirtualPath::INDEX.repository_relative(), Some(index))],
        )
        .is_ok());
    }

    #[test]
    fn test_validate_live_reports_an_unparseable_event_log() {
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::EVENTS), "not jsonl\n").unwrap();

        let error = failure(&validate_live(&executor).unwrap_err());

        assert!(
            error.contains("record-fidelity")
                && error.contains(&VirtualPath::EVENTS.repository_relative())
                && error.contains("line 1"),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_records_dag_and_gate_integrity_judge_planned_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let id = executor.storage().list_issues().unwrap().remove(0).id;
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&executor, &issue_path(&id).repository_relative()).unwrap(),
        )
        .unwrap();
        issue.dependencies = vec![crate::test_utils::issue_id(
            0x424157f2ba6b7bb89e05938a9dbccee5,
        )];
        issue.gates_required = vec!["planned".parse().unwrap()];
        let bad = serde_json::to_vec_pretty(&issue).unwrap();
        let error = failure(
            &validate_overlaid(
                &executor,
                [
                    (issue_path(&id).repository_relative(), Some(bad)),
                    (
                        VirtualPath::GATES.repository_relative().to_string(),
                        Some(b"[[gates]]\nkey = \"planned\"\ntitle = \"p\"\ndescription = \"\"\nstage = \"postcheck\"\nmode = \"manual\"\n".to_vec()),
                    ),
                ],
            )
            .unwrap_err()
        );
        assert!(
            error.contains("repository-integrity")
                && error.contains(issue.dependencies[0].as_str()),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_namespace_and_hierarchy_judge_planned_config() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let report = validate_materialized_overlaid(
            &executor,
            [put(
                VirtualPath::CONFIG.repository_relative(),
                "[type_hierarchy.types]\nepic = 2\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
            )],
        )
        .unwrap();
        assert!(report.rule_report().has_errors());
        assert!(report
            .rule_report()
            .findings
            .iter()
            .any(|finding| finding.rule == "type-hierarchy-known"));
    }

    #[test]
    fn test_overlay_item_links_judge_planned_registry_deletion() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[namespaces.enforces]\ndescription = \"Item link\"\nunique = false\n{}",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        std::fs::write(
            data_path(&executor, &VirtualPath::CONFIG),
            config.as_bytes(),
        )
        .unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::INVARIANTS), invariants).unwrap();
        let id = &executor.storage().list_issues().unwrap().remove(0).id;
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&executor, &issue_path(id).repository_relative()).unwrap(),
        )
        .unwrap();
        issue
            .labels
            .push("enforces:@/invariant/planned".to_string());
        std::fs::write(
            data_path(&executor, &issue_path(id)),
            serde_json::to_vec_pretty(&issue).unwrap(),
        )
        .unwrap();

        executor.validation().validate_with_fix(false).unwrap();
        let live = validate_live(&executor).unwrap();
        assert!(!live.rule_report().has_errors(), "{:?}", live.rule_report());
        let planned = validate_materialized_overlaid(
            &executor,
            [(VirtualPath::INVARIANTS.repository_relative(), None)],
        )
        .unwrap();
        assert!(planned.rule_report().findings.iter().any(|finding| {
            finding.rule == crate::validation::report::BuiltInPass::DanglingItemLink.as_str()
        }));
    }

    #[test]
    fn test_overlay_projection_rejects_stale_planned_target_despite_live_state() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor.validation().validate_with_fix(false).unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n{}\n\
             [projection.invariants]\nkind = \"invariant\"\ntarget = \"INVARIANTS.md\"\nmode = \"separate-file\"\nstyle = \"full\"\n",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        let error = failure(
            &validate_overlaid(
                &executor,
                [
                    put(VirtualPath::CONFIG.repository_relative(), &config),
                    put(VirtualPath::INVARIANTS.repository_relative(), invariants),
                    put("INVARIANTS.md", "stale planned projection\n"),
                ],
            )
            .unwrap_err(),
        );
        assert!(
            error.contains("derived-materialization validation pass"),
            "{error}"
        );
    }
}
