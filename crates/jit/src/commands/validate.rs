//! Validation and status operations

use super::*;
use crate::domain::identity::GateKey;
use crate::domain::type_taxonomy::{
    detect_validation_issues, generate_fixes, ValidationFix, ValidationIssue,
};
use crate::validation::report::{BuiltInPass, RuleSelectionFault};
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
type ParsedIssues = (Vec<Issue>, Vec<serde_json::Value>);

struct CapturedRepairPlan {
    declarations: crate::repository_state::CapturedRepositoryDeclarations,
    plan: Option<crate::repository_state::MaterializationPlan>,
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
    /// Validate with optional fix mode.
    ///
    /// # Arguments
    ///
    /// * `fix` - If true, attempt to fix validation issues
    /// * `dry_run` - If true, show what would be fixed without applying changes
    ///
    /// # Returns
    ///
    /// Returns Ok with (count of fixes applied, messages), or Err if validation fails.
    /// In fix mode a repository that still fails validation with nothing
    /// repaired is an Err naming the findings `--fix` cannot repair.
    pub fn validate_with_fix(&self, fix: bool, dry_run: bool) -> Result<(usize, Vec<String>)> {
        // First run standard validation
        let validation_result = self.validate_silent();

        if validation_result.is_ok() && !fix {
            return Ok((0, vec![]));
        }

        // If fix mode is enabled, detect and fix issues
        if fix {
            let mut total_fixes = 0;
            let mut all_messages = Vec::new();

            // Fix type hierarchy issues
            let (hierarchy_fixes, mut messages) = self.detect_and_fix_hierarchy_issues(dry_run)?;
            total_fixes += hierarchy_fixes;
            all_messages.append(&mut messages);

            // Fix transitive reduction violations (both dry-run and actual fix)
            let (reduction_fixes, mut messages) = self.fix_all_transitive_reductions(dry_run)?;
            total_fixes += reduction_fixes;
            all_messages.append(&mut messages);

            // Fix pending state transitions
            let (transition_fixes, mut messages) = self.check_pending_transitions(dry_run)?;
            total_fixes += transition_fixes;
            all_messages.append(&mut messages);

            // Repair derived state (default rules/schemas + configured projections)
            // through one repair delta in one redo record.
            let (repair_fixes, mut messages) = self.repair_derived_state(dry_run)?;
            total_fixes += repair_fixes;
            all_messages.append(&mut messages);

            if total_fixes == 0 {
                validation_result.map_err(|error| {
                    crate::errors::ValidationFailedError::new(format!(
                        "--fix has no automatic repair for these findings: {error}"
                    ))
                })?;
            }

            // Add summary message
            if dry_run {
                all_messages.push(format!(
                    "\nDry run complete. {} fixes would be applied.",
                    total_fixes
                ));
            } else if total_fixes > 0 {
                all_messages.push(format!("\nApplied {} fixes", total_fixes));
                // Re-run validation to verify fixes worked
                self.validate_silent()?;
                all_messages.push("Repository is now valid".to_string());
            } else {
                all_messages.push("\nNo fixes needed".to_string());
            }

            return Ok((total_fixes, all_messages));
        }

        // Not in fix mode, just propagate the validation error
        validation_result?;
        Ok((0, vec![]))
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

impl<S: IssueStore + crate::storage::RepositoryStateStore> CommandExecutor<S> {
    /// Capture the declaration and rule-validation image through a mutation
    /// session, in the one round
    /// [`capture_proposed_base_inner`](Self::capture_proposed_base_inner) reads.
    fn capture_validation_image(&self) -> Result<crate::repository_state::RepositoryImage> {
        self.capture_validation_image_with(&std::collections::BTreeMap::new())
    }

    /// Capture the declaration and rule-validation image, optionally over a
    /// proposed overlay. Document-reference auditing is a separate capture pass.
    ///
    /// With an empty `overrides` this captures and validates the live repository.
    /// With a non-empty overlay (the projection of a proposed
    /// [`RepositoryDelta`](crate::repository_state::RepositoryDelta) to final
    /// bytes/absence) the closure is computed from the OVERLAID declarations — so
    /// it covers what the proposed state's passes read — the base is captured over
    /// that closure, and the overlay is applied
    /// ([`apply_overlay`](crate::repository_state::apply_overlay)) so
    /// the validation report judges one coherent proposed repository under the same
    /// closed-read discipline.
    pub(crate) fn capture_validation_image_with(
        &self,
        overrides: &std::collections::BTreeMap<
            crate::repository_state::VirtualPath,
            Option<Vec<u8>>,
        >,
    ) -> Result<crate::repository_state::RepositoryImage> {
        use crate::repository_state::apply_overlay;

        let layout = self.require_layout()?;
        with_mutation_session(
            self.storage(),
            &layout,
            "validation capture",
            |session| match self.capture_proposed_base(session, overrides, &[], None)? {
                None => Ok(SessionStep::Retry),
                Some(base) if overrides.is_empty() => Ok(SessionStep::Done(base)),
                Some(base) => Ok(SessionStep::Done(apply_overlay(&base, overrides.clone())?)),
            },
        )
    }

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
        .map(|captured| captured.map(|(image, (issues, _))| MutationBase { image, issues }))
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
        Ok(grown.map(|(image, (issues, _))| MutationPreflight {
            image,
            target,
            issues: Some(issues),
        }))
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
        let mut discover = |image: &RepositoryImage, spec: &mut CaptureSpec| -> Result<()> {
            base(spec)?;
            if spec != image.capture_spec() {
                return Ok(());
            }
            // Closure planning is best-effort: a malformed registry cannot fail
            // the capture, because validation itself is the authority that
            // reports it. A bad config/listing/rules yields a conservative closure
            // (its projection, item-kind, issue, and schema entries drop out),
            // and the subsequent validation report surfaces the parse
            // error as a finding.
            let config = effective_config(image, &effective_virtual_path)
                .unwrap_or_else(|_| empty_materialization_config());
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
                    (super::captured_active_issues(image)?, Vec::new())
                } else if matches!(purpose, BaseCapturePurpose::Repair) {
                    (Vec::new(), Vec::new())
                } else {
                    // Effective bytes: the override (create/replace or proposed
                    // absence) when present, otherwise the captured base bytes.
                    crate::repository_state::parse_issue_records(
                        &issue_ids,
                        |path| match overrides.get(path) {
                            Some(value) => Ok(value.as_deref()),
                            None => image.file_bytes(path),
                        },
                    )?
                    .into_iter()
                    .unzip()
                };
                if matches!(
                    purpose,
                    BaseCapturePurpose::Validation | BaseCapturePurpose::Mutation { .. }
                ) {
                    discover_issue_closure(
                        image,
                        spec,
                        &parsed,
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

    /// Repair derived state (default rules/schemas plus every configured
    /// projection) through one mutation session (`jit validate --fix`).
    ///
    /// The complete owned-materialization set is derived from declared authority
    /// over one captured image (`MaterializationRequest::RepairDerivedState`) and
    /// applied through the same session; a coherent repository yields an empty
    /// delta (no action). Repair is ownership-safe by construction — `rules.toml`
    /// splices only the generated default spans and region projections splice only
    /// their managed region, so authored content is preserved. In `dry_run` the
    /// delta is derived but not applied. Returns the number of repaired targets and
    /// a per-target message. JSON and memory stores execute this same path.
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
    fn capture_validation(
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

/// Parse the (possibly overlaid) configuration and invariant registry.
fn effective_config(
    image: &crate::repository_state::RepositoryImage,
    effective: &EffectiveVirtualPath<'_>,
) -> Result<JitConfig> {
    let config_bytes = effective(image, &crate::repository_state::VirtualPath::CONFIG)?
        .ok_or_else(|| anyhow!("captured image has no .jit/config.toml"))?;
    let declarations = crate::declarations::parse_configuration(&config_bytes)
        .context("invalid .jit/config.toml")?;
    let invariants = match effective(image, &crate::repository_state::VirtualPath::INVARIANTS)? {
        Some(bytes) => crate::declarations::invariants::InvariantRegistry::from_toml_str(
            &String::from_utf8(bytes)?,
        )?,
        None => crate::declarations::invariants::InvariantRegistry::empty(),
    };
    Ok(declarations.materialization_config(invariants))
}

/// Canonical empty declaration view for best-effort closure planning after a
/// malformed config; the validation pass remains responsible for the finding.
fn empty_materialization_config() -> JitConfig {
    crate::declarations::parse_configuration(b"")
        .expect("empty configuration parses")
        .materialization_config(crate::declarations::invariants::InvariantRegistry::empty())
}

/// Parse the (possibly overlaid) template registry, needed to derive plan-document
/// paths.
///
/// Best-effort like the rest of closure planning: a malformed `templates.toml`
/// yields an empty registry (no plan-document paths), and the subsequent
/// validation report surfaces the parse error as a validation failure.
fn effective_templates(
    image: &crate::repository_state::RepositoryImage,
    config: &JitConfig,
    effective: &EffectiveVirtualPath<'_>,
) -> Result<crate::templates::TemplateRegistry> {
    let hierarchy_types: Vec<&str> = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
        .unwrap_or_default();
    Ok(
        match effective(image, &crate::repository_state::VirtualPath::TEMPLATES)? {
            Some(bytes) => String::from_utf8(bytes)
                .ok()
                .and_then(|text| {
                    crate::templates::TemplateRegistry::from_toml_str(&text, &hierarchy_types).ok()
                })
                .unwrap_or_else(crate::templates::TemplateRegistry::empty),
            None => crate::templates::TemplateRegistry::empty(),
        },
    )
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

    let mut full_config = config.clone();
    full_config.templates = effective_templates(image, config, effective)?;
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
    spec.discover_paths(plan_document_paths(issues, &full_config)?)?;
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

/// Plan-document content for `emit_for` from a captured image, whose closure
/// holds every path [`plan_document_paths`] derives; `all_issues` is the image's
/// complete issue set. A missing or malformed configuration yields no plan
/// documents, and the validation report reports it.
pub(crate) fn plan_content_with_issues(
    image: &crate::repository_state::RepositoryImage,
    all_issues: &[Issue],
    emit_for: &[Issue],
) -> Result<std::collections::HashMap<IssueId, String>> {
    let effective_virtual_path =
        |img: &crate::repository_state::RepositoryImage,
         path: &crate::repository_state::VirtualPath|
         -> Result<Option<Vec<u8>>> { super::image_virtual_bytes(img, path) };
    let effective_virtual_path: &EffectiveVirtualPath<'_> = &effective_virtual_path;
    let config = effective_config(image, effective_virtual_path)
        .unwrap_or_else(|_| empty_materialization_config());
    let templates = effective_templates(image, &config, effective_virtual_path)?;
    crate::validation::repository::plan_content(
        &|path| super::image_repo_bytes(image, path),
        all_issues,
        emit_for,
        &templates,
    )
}

/// Evaluate one configured graph rule against one issue from a closed image.
///
/// The selected issue is the sole firing subject. The complete captured issue
/// set is retained as graph and resolution context, and all rule/configuration
/// lookup plus selected plan-document projection comes from the same image.
/// This is the shared evaluation boundary for the live CLI and native gate checker.
///
/// A rule that is missing, disabled, non-graph, or inapplicable to the issue is
/// a configuration fault, returned as the inner `Err` so each caller chooses its
/// reporting: the CLI an error finding, the gate checker an `error` run.
pub(super) fn rule_validation_outcome(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &IssueId,
    rule_name: &str,
) -> Result<std::result::Result<crate::validation::report::RuleReport, RuleSelectionFault>> {
    use crate::validation::report::{ReportedFinding, RuleReport};

    let issues = super::captured_active_issues(image)?;
    let issue = issues
        .iter()
        .find(|issue| &issue.id == issue_id)
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id.clone()))?;
    let declarations = crate::repository_state::declarations_from_image(image)?;
    let config = crate::repository_state::assemble_config(image)?;
    let repo_format = config.content_format();
    let rule = match crate::validation::graph::select_graph_rule(
        &declarations.rules.rules,
        rule_name,
        issue,
    ) {
        Ok(rule) => rule,
        Err(message) => {
            return Ok(Err(RuleSelectionFault {
                issue_id: issue.id.clone(),
                rule: rule_name.to_string(),
                message,
            }));
        }
    };

    let plan_source_ids =
        crate::validation::graph::graph_rule_plan_source_ids(rule, issue, &issues);
    let plan_sources = plan_source_ids
        .iter()
        .filter_map(|source_id| {
            issues
                .iter()
                .find(|candidate| &candidate.id == source_id)
                .cloned()
        })
        .collect::<Vec<_>>();
    let plan_content = plan_content_with_issues(image, &issues, &plan_sources)?;
    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let hierarchy = crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces)?;
    let context = crate::validation::graph::GraphEvaluationContext {
        graph_context: &issues,
        resolution_index: &issues,
        hierarchy: &hierarchy,
        repo_default_format: repo_format,
        now: chrono::Utc::now(),
        plan_content: &plan_content,
    };
    let findings =
        crate::validation::graph::evaluate_graph_rule(rule, std::slice::from_ref(issue), &context);
    Ok(Ok(RuleReport {
        findings: findings
            .into_iter()
            .map(|finding| ReportedFinding::new(finding.issue_id, &finding.finding))
            .collect(),
    }))
}

impl<S: IssueStore> ValidationOps<'_, S> {
    fn detect_and_fix_hierarchy_issues(&self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::config_manager::get_hierarchy_config;

        let config = get_hierarchy_config(&self.exec.storage)?;
        let issues = self.exec.storage.list_issues()?;

        // Collect all validation issues
        let mut all_validation_issues = Vec::new();

        for issue in &issues {
            let validation_issues = detect_validation_issues(&config, &issue.id, &issue.labels);
            all_validation_issues.extend(validation_issues);
        }

        if all_validation_issues.is_empty() {
            return Ok((0, vec![]));
        }

        // Generate fixes
        let fixes = generate_fixes(&all_validation_issues);

        if fixes.is_empty() {
            // We found issues but can't auto-fix them
            let mut messages = vec![format!(
                "Found {} validation issues but no automatic fixes available:",
                all_validation_issues.len()
            )];
            for issue in &all_validation_issues {
                match issue {
                    ValidationIssue::UnknownType {
                        issue_id,
                        unknown_type,
                        ..
                    } => {
                        messages.push(format!(
                            "  • Issue {} has unknown type '{}'",
                            issue_id, unknown_type
                        ));
                    }
                }
            }
            return Ok((0, messages));
        }

        // Apply or preview fixes
        let mut fixes_applied = 0;
        let mut messages = Vec::new();

        for fix in &fixes {
            match fix {
                ValidationFix::ReplaceType {
                    issue_id,
                    old_type,
                    new_type,
                } => {
                    if dry_run {
                        messages.push(format!(
                            "Would replace type '{}' with '{}' for issue {}",
                            old_type, new_type, issue_id
                        ));
                    } else {
                        messages.extend(self.exec.refuse_foreign_lease(issue_id)?);
                        self.apply_type_fix(issue_id, old_type, new_type)?;
                        messages.push(format!(
                            "Replaced type '{}' with '{}' for issue {}",
                            old_type, new_type, issue_id
                        ));
                    }
                    fixes_applied += 1;
                }
            }
        }

        Ok((fixes_applied, messages))
    }

    fn apply_type_fix(&self, issue_id: &IssueId, old_type: &str, new_type: &str) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Replace the type label
        let old_label = label_utils::type_label(old_type);
        let new_label = label_utils::type_label(new_type);

        self.exec
            .publish_captured_field_update(CapturedFieldUpdate::label_edit(
                issue_id.clone(),
                CapturedLabelEdit::ReplaceExact {
                    old: old_label,
                    new: new_label,
                },
            ))
            .map(|_| ())
    }

    // Note: apply_dependency_reversal is removed - we don't reverse dependencies
    // Type hierarchy is orthogonal to DAG structure

    /// Built-in validate pass: report every node→item link label whose qualified
    /// id cannot be resolved as a finding, rather than silently dropping it
    /// (REQ-08 clause i, REQ-03).
    ///
    /// For each issue, every label is inspected. A label is a candidate link
    /// reference when its namespace is a `link-namespace` of SOME configured item
    /// kind (the set is derived generically from
    /// [`ItemKind::link_namespaces`](crate::domain::item::ItemKind::link_namespaces),
    /// never a hardcoded list of names) AND its value is a qualified id
    /// `<scope>/<self-id>`. Each candidate is resolved through the SINGLE generic
    /// resolver [`resolve_link_label`](crate::commands::ItemOps::resolve_link_label),
    /// so resolution logic is not forked. A candidate that fails to resolve yields
    /// one [`GraphFinding`] attributed to the owning issue, naming the dangling
    /// qualified id. A link-namespace or coverage-credit-namespace label with a
    /// bare self-id yields the
    /// [`unqualified_link_message`](crate::domain::item::unqualified_link_message)
    /// finding; a resolvable candidate and any other namespace yield nothing.
    ///
    /// The finding severity is [`Severity::Error`](crate::declarations::rules::Severity::Error)
    /// so a dangling link fails `jit validate`. The rule name is the constant
    /// [`BuiltInPass::DanglingItemLink`]. This pass touches no `.jit/` ruleset: it runs
    /// unconditionally as part of the validate path.
    pub fn dangling_link_findings(
        &self,
        issues: &[Issue],
    ) -> Result<Vec<crate::validation::graph::GraphFinding>> {
        use crate::declarations::rules::Severity;
        use crate::validation::engine::Finding;
        use crate::validation::graph::GraphFinding;
        use std::collections::BTreeSet;

        // Derive the link-namespace set generically from the configured kinds — no
        // kind-name literal, just the namespaces each kind declares.
        let kinds = self.exec.items().item_kinds()?;
        // An unloadable rule set is reported by the rules pass; it credits nothing.
        let credit_namespaces = self
            .exec
            .effective_rules()
            .map(|set| crate::validation::graph::coverage_credit_namespaces(&set.rules))
            .unwrap_or_default();
        let link_namespaces: BTreeSet<String> = kinds
            .iter()
            .flat_map(|kind| kind.link_namespaces().iter().cloned())
            .collect();

        let mut findings = Vec::new();
        for issue in issues {
            for label in &issue.labels {
                let Some((namespace, value)) = label.split_once(':') else {
                    continue;
                };
                if let Some(message) = crate::domain::item::unqualified_link_message(
                    &issue.id,
                    namespace,
                    value,
                    &kinds,
                    &credit_namespaces,
                ) {
                    findings.push(GraphFinding::for_issue(
                        issue.id.clone(),
                        Finding {
                            rule: BuiltInPass::DanglingItemLink.as_str().to_owned(),
                            severity: Severity::Error,
                            message,
                        },
                    ));
                    continue;
                }
                if !link_namespaces.contains(namespace) {
                    continue;
                }
                if !crate::domain::item::is_qualified_reference(value) {
                    continue;
                }
                // Resolve through the single generic resolver; an `Err` means the
                // qualified id is dangling and is reported as a finding.
                if self.exec.items().resolve_link_label(label).is_err() {
                    findings.push(GraphFinding::for_issue(
                        issue.id.clone(),
                        Finding {
                            rule: BuiltInPass::DanglingItemLink.as_str().to_owned(),
                            severity: Severity::Error,
                            message: format!(
                                "issue {} has a dangling item link '{label}': \
                                 the qualified id '{value}' resolves to no addressable item",
                                issue.short_id()
                            ),
                        },
                    ));
                }
            }
        }
        Ok(findings)
    }

    /// Report unresolved invariant enforcement bindings as unattributed errors.
    /// An unloadable rule set remains reportable; gate load errors propagate.
    pub fn enforcement_drift_findings(
        &self,
    ) -> Result<Vec<crate::validation::graph::GraphFinding>> {
        use crate::declarations::rules::Severity;
        use crate::validation::engine::Finding;
        use crate::validation::graph::GraphFinding;

        let findings = self
            .compute_drift_findings()?
            .into_iter()
            .map(|f| {
                GraphFinding::unattributed(Finding {
                    rule: BuiltInPass::EnforcementDrift.as_str().to_owned(),
                    // A broken binding (declared-but-unenforced) is a real defect
                    // -> fails validate. It is the only drift direction (REQ-05).
                    severity: Severity::Error,
                    message: f.message(),
                })
            })
            .collect();
        Ok(findings)
    }

    /// Compute drift with tolerant rule loading and strict gate loading.
    /// Returns no findings when no invariants are declared.
    pub fn compute_drift_findings(&self) -> Result<Vec<crate::validation::drift::DriftFinding>> {
        use crate::validation::drift::{enforcement_drift, SourceState};
        use std::collections::BTreeSet;

        let config = self.exec.cached_config()?;
        let invariants = &config.invariants.invariants;
        // Gated on declared invariants: a repo with none sees no drift findings.
        if invariants.is_empty() {
            return Ok(Vec::new());
        }

        let rule_names_owned = self.loadable_rule_names();
        let gate_set: BTreeSet<GateKey> = self
            .exec
            .storage
            .load_gate_registry()?
            .gates
            .into_keys()
            .collect();

        let rule_set: Option<BTreeSet<&str>> = rule_names_owned
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());

        let rules_state = match &rule_set {
            Some(set) => SourceState::Loaded(set),
            None => SourceState::Unloadable,
        };
        Ok(enforcement_drift(invariants, rules_state, &gate_set))
    }

    /// Defensively resolve the names of every LOADABLE rule, returning `None` when
    /// the rule SOURCE is unloadable.
    ///
    /// Mirrors [`effective_rules`](crate::commands::CommandExecutor::effective_rules)
    /// resolution but tolerantly: a present-and-parseable `.jit/rules.toml` yields
    /// its rule names; an ABSENT file yields the in-memory default rule set's names
    /// (still loadable); a present-but-MALFORMED file (or an unresolvable namespace
    /// registry) yields `None` (unloadable). Used only by the enforcement-drift
    /// pass, which must not crash when the ruleset fails to parse.
    pub fn loadable_rule_names(&self) -> Option<Vec<String>> {
        let layout = self.exec.require_layout().ok()?;
        let rules_path = layout
            .resolve(&crate::repository_state::VirtualPath::RULES)
            .ok()?;
        if rules_path.exists() {
            // Present: parse it directly (do NOT go through the cached
            // `effective_rules`, which `?`-errors). A parse failure -> unloadable.
            self.exec
                .config_manager
                .load()
                .ok()
                .and_then(|config| {
                    crate::storage::ruleset_store::load_ruleset(&layout, &config).ok()
                })
                .map(|set| set.rules.into_iter().map(|r| r.name).collect())
        } else {
            // Absent: the in-memory defaults are loadable. A namespace-registry
            // failure (rare) reads as unloadable rather than crashing.
            self.exec.cached_namespaces().ok().map(|namespaces| {
                crate::repository_state::default_ruleset(namespaces)
                    .rules
                    .into_iter()
                    .map(|r| r.name)
                    .collect()
            })
        }
    }

    /// Evaluate every `RuleScope::Graph` rule from `.jit/rules.toml` over the supplied
    /// issue set, returning one
    /// [`GraphFinding`](crate::validation::graph::GraphFinding) per violation
    /// (including `config-error` findings for malformed rules). Each finding
    /// carries the issue it pertains to (or `None` for a config-error), so
    /// per-issue reporting can attribute findings exactly rather than by matching
    /// substrings in the message.
    ///
    /// The ruleset is loaded via [`CommandExecutor::rules`](crate::commands::CommandExecutor::rules);
    /// a genuine `rules.toml` parse/load failure is surfaced as an `Err` rather
    /// than silently disabling enforcement. A missing `rules.toml` yields no
    /// findings. This method performs no filesystem writes; it reads the cached
    /// ruleset and the issues passed in.
    pub fn evaluate_graph_rules(
        &self,
        issues: &[Issue],
    ) -> Result<Vec<crate::validation::graph::GraphFinding>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::rules::RuleScope;

        // Surface a misconfigured rules.toml instead of swallowing it.
        let ruleset = self.exec.effective_rules()?;

        let graph_rules: Vec<&crate::declarations::rules::Rule> = ruleset
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph)
            .collect();

        // The repo HierarchyConfig is injected into `type-hierarchy` rules at
        // evaluation time (D1); it is no longer stored in the parsed rule. Build
        // it from the same namespace registry as the rule declarations.
        let namespaces = self.exec.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
        let hierarchy =
            crate::domain::type_taxonomy::HierarchyConfig::from_declarations(namespaces)?;
        let repo_format = self.exec.cached_config()?.content_format();

        // Resolve any external plan documents from the captured validation image
        // so a container whose criteria live in an external file is validated
        // against the FILE content; the engine itself reads only the injected map
        // (stays pure) and the plan bytes come from the closed image, not the
        // live filesystem.
        let plan_content = self.image_plan_content(issues)?;

        // Inject the wall-clock instant at the boundary so `gate-recency` rules
        // are deterministic and the graph engine stays pure (CC-5b).
        Ok(crate::validation::graph::evaluate_graph(
            &graph_rules,
            issues,
            &hierarchy,
            repo_format,
            chrono::Utc::now(),
            &plan_content,
        ))
    }

    /// Plan-document content for `emit_for` from the whole-repository validation
    /// image, resolving planning nodes over every captured issue so a scoped slice
    /// still finds its bracket's planning node.
    pub(crate) fn image_plan_content(
        &self,
        emit_for: &[Issue],
    ) -> Result<std::collections::HashMap<IssueId, String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let image = self.exec.capture_validation_image()?;
        // Undecodable issue records yield no plan content; the validation report
        // reports them.
        let all_issues = super::captured_active_issues(&image).unwrap_or_default();
        plan_content_with_issues(&image, &all_issues, emit_for)
    }

    /// Report configured passing review placeholders as project-scoped warnings.
    ///
    /// The registry keys are sorted before rendering so the finding is stable
    /// across hash-map iteration order. This pass is advisory: placeholders make
    /// repository validation noisy and truthful without making an otherwise
    /// valid repository unusable before its external reviewer is configured.
    pub fn review_placeholder_findings(
        &self,
    ) -> Result<Vec<crate::validation::report::ReportedFinding>> {
        use crate::declarations::rules::Severity;
        use crate::declarations::GateChecker;
        use crate::validation::engine::Finding;
        use crate::validation::report::ReportedFinding;

        let registry = self.exec.storage.load_gate_registry()?;
        let mut keys: Vec<&GateKey> = registry
            .gates
            .iter()
            .filter_map(|(key, gate)| {
                matches!(gate.checker, Some(GateChecker::ReviewPlaceholder)).then_some(key)
            })
            .collect();
        keys.sort_unstable();

        Ok(if keys.is_empty() {
            Vec::new()
        } else {
            vec![ReportedFinding::new(
                None,
                &Finding {
                    rule: BuiltInPass::ReviewPlaceholder.as_str().to_owned(),
                    severity: Severity::Warn,
                    message: format!(
                        "WARNING: passing external-review placeholder still configured for gate(s): {}. Replace each placeholder with a real review checker before relying on these gates.",
                        keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                    ),
                },
            )]
        })
    }

    /// Run the declarative rule set over a **container bracket subtree** for use
    /// as a deterministic gate checker (`jit validate --scope <id>`, T2/D14).
    ///
    /// The scope slice is the container's transitive dependency closure
    /// **including** the `type:breakdown` node `B` but **bounded** so the walk
    /// stops at `B` (it never pulls in `P` / upstream beyond the breakdown gate).
    /// See [`bracket_scope_ids`](crate::domain::queries::bracket_scope_ids) for
    /// the precise membership rule. For each
    /// in-slice issue, the rules whose `when` selector matches it are evaluated —
    /// so a rule keyed on `type:breakdown` (the coverage-preview instance, D13)
    /// fires because `B` is in scope. This decides *whose* rules run; it is
    /// orthogonal to `child-type-exclude`, which governs only the coverage walk's
    /// candidate set (D14) and is a separate, coverage-rule-internal concern.
    ///
    /// Repo-wide rule kinds (`label-uniqueness`, repo-wide `label-reference`,
    /// `type-hierarchy`) are EXCLUDED here exactly as they are excluded from
    /// transition-time enforcement (the CC-2a `is_repo_wide_at_transition`
    /// filter, R2): they need the whole repository, not a slice, so they remain a
    /// whole-repo `jit validate` concern and never participate in a `--scope`
    /// gate.
    ///
    /// Both local and graph rules participate: local rules are evaluated per
    /// in-slice issue; graph rules are evaluated over the slice and attributed
    /// findings (plus any `config-error` for an applicable rule) are reported. The
    /// result is a pure [`RuleReport`](crate::validation::report::RuleReport);
    /// the caller decides the process exit code (4 / `ValidationFailed` on any
    /// error-severity finding, 0 when clean).
    ///
    /// # Errors
    ///
    /// Returns an error if `.jit/rules.toml` is malformed, the container is not
    /// found, or a matching local rule's schema fails to compile.
    pub fn validate_scope(
        &self,
        container_full_id: &IssueId,
    ) -> Result<crate::validation::report::RuleReport>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::rules::{RuleScope, Severity};
        use crate::validation::report::{ReportedFinding, RuleReport};

        let ruleset = self.exec.effective_rules()?;
        let repo_format = self.exec.cached_config()?.content_format();
        let all = self.exec.storage.list_issues()?;

        // The breakdown boundary type is template-driven: resolve the container's
        // `type:` label, look up its graph template, and read that template's
        // breakdown-node type. No applicable template (a non-bracketed container)
        // → no boundary, so the scope walk is the full dependency closure (see
        // `bracket_scope_ids`).
        let container = all
            .iter()
            .find(|i| &i.id == container_full_id)
            .ok_or_else(|| crate::storage::IssueNotFoundError::new(container_full_id.clone()))?;
        let container_type = label_utils::type_label_value(&container.labels).map(str::to_string);
        let templates = &self.exec.cached_config()?.templates;
        let breakdown_type = container_type
            .as_deref()
            .and_then(|ty| templates.template_for_container(ty))
            .and_then(|t| t.breakdown_type(&templates.roles))
            .map(str::to_string);

        let scope_ids = crate::domain::queries::bracket_scope_ids(
            container_full_id,
            &all,
            breakdown_type.as_deref(),
        );
        // Keep the full issue set alive: it is the resolution index against which a
        // `container-from-label` pointer resolves, so a valid pointer to a
        // container outside this slice does not read as dangling (ef0065ad).
        let slice: Vec<Issue> = all
            .iter()
            .filter(|i| scope_ids.contains(&i.id))
            .cloned()
            .collect();

        let mut findings: Vec<ReportedFinding> = Vec::new();

        // Local rules: evaluate each in-slice issue against its matching local
        // rules. (`evaluate_local` itself selects only `RuleScope::Local`,
        // non-`off` rules whose selector matches.)
        for issue in &slice {
            let evaluation = crate::validation::evaluate_local(issue, ruleset, repo_format)
                .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
            findings.extend(
                evaluation
                    .findings()
                    .into_iter()
                    .map(|f| ReportedFinding::new(Some(issue.id.clone()), f)),
            );
        }

        // Graph rules: select those whose `when` matches SOME in-slice issue,
        // minus the repo-wide kinds (R2 / CC-2a), then evaluate over the slice.
        // This mirrors transition-time graph-rule select-then-slice
        // precision, but membership here is the bracket subtree, not a
        // transition neighborhood.
        let graph_rules: Vec<&crate::declarations::rules::Rule> = ruleset
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
            .filter(|rule| !rule.assert.is_repo_wide_at_transition())
            .filter(|rule| slice.iter().any(|issue| rule.when.matches(issue)))
            .collect();

        if !graph_rules.is_empty() {
            let namespaces = self.exec.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
            let hierarchy =
                crate::domain::type_taxonomy::HierarchyConfig::from_declarations(namespaces)?;
            // Resolve external plan docs for the in-scope issues from the captured
            // validation image so a container whose criteria live in an external
            // file validates against the FILE (closed-read, no live filesystem).
            let plan_content = self.image_plan_content(&slice)?;
            let graph_findings = crate::validation::graph::evaluate_graph_scoped(
                &graph_rules,
                &slice,
                &all,
                &hierarchy,
                repo_format,
                chrono::Utc::now(),
                &plan_content,
            );
            findings.extend(
                graph_findings
                    .iter()
                    .map(|gf| ReportedFinding::new(gf.issue_id.clone(), &gf.finding)),
            );
        }

        // Built-in dangling-item-link pass over the in-slice issues (REQ-08
        // clause i, REQ-03): a dangling link on a bracket-subtree node fails the
        // `--scope` gate exactly like a ruleset error-severity finding.
        findings.extend(
            self.dangling_link_findings(&slice)?
                .iter()
                .map(|gf| ReportedFinding::new(gf.issue_id.clone(), &gf.finding)),
        );

        // Built-in enforcement-drift pass (REQ-01/REQ-02): a project-scoped,
        // declaration-consistency check the scope gate also enforces. Gated on
        // declared invariants, so a repo without any is unaffected.
        findings.extend(
            self.enforcement_drift_findings()?
                .iter()
                .map(|gf| ReportedFinding::new(gf.issue_id.clone(), &gf.finding)),
        );

        Ok(RuleReport { findings })
    }

    pub fn get_status(&self) -> Result<StatusSummary> {
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

        Ok(StatusSummary {
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

    /// Fix transitive reduction violations for all issues.
    ///
    /// Returns count of redundant edges fixed (or that would be fixed if dry_run).
    fn fix_all_transitive_reductions(&self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.exec.reduce_all_dependencies(dry_run)
    }

    /// Reconcile stored readiness with the readiness the dependency graph derives.
    ///
    /// Stored state can drift from the graph in both directions: after a worktree
    /// merge a backlog issue may have every dependency in a terminal state without
    /// ever auto-transitioning to ready, and an issue can hold `Ready` while a
    /// dependency that blocks it is unmet. Both are violations of
    /// `@/invariant/derived-state-coherence` and both are repaired here, through
    /// the one derivation [`Issue::derive_readiness_correction`] owns.
    ///
    /// Uses multiple passes to handle cascading transitions (e.g., when tasks reach a
    /// terminal state, stories become ready, then epics that depend on those stories
    /// also become ready).
    ///
    /// # Arguments
    ///
    /// * `dry_run` - If true, report what would be fixed without applying changes
    ///
    /// # Returns
    ///
    /// Count of issues transitioned and informational messages
    fn check_pending_transitions(&self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::domain::ReadinessCorrection;

        let mut total_fixed = 0;
        let mut messages = Vec::new();
        let max_passes = 10; // Safety limit to prevent infinite loops

        // Keep checking until no more transitions found (cascading transitions)
        // In dry-run mode, only do one pass since we don't actually change state
        let num_passes = if dry_run { 1 } else { max_passes };

        for _pass in 0..num_passes {
            let issues = self.exec.storage.list_issues()?;
            let resolved = crate::domain::queries::build_issue_map(&issues);

            // Every issue whose stored readiness disagrees with the graph, with the
            // correction that restores agreement.
            let corrections = issues
                .iter()
                .filter_map(|issue| {
                    issue
                        .derive_readiness_correction(&resolved)
                        .map(|correction| (issue.id.clone(), issue.short_id(), correction))
                })
                .collect::<Vec<_>>();

            for (issue_id, short_id, correction) in &corrections {
                messages.push(match correction {
                    ReadinessCorrection::Promote => {
                        format!("  → Transitioning {short_id} to ready (dependencies terminal)")
                    }
                    ReadinessCorrection::Demote => {
                        format!("  → Transitioning {short_id} to backlog (dependencies unmet)")
                    }
                });

                if !dry_run {
                    match correction {
                        ReadinessCorrection::Promote => {
                            self.exec.auto_transition_to_ready(issue_id)?
                        }
                        ReadinessCorrection::Demote => {
                            self.exec.auto_transition_to_backlog(issue_id)?
                        }
                    };
                }
            }

            total_fixed += corrections.len();

            // If no fixes this pass, we're done
            if corrections.is_empty() {
                break;
            }
        }

        Ok((total_fixed, messages))
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
    use crate::storage::JsonFileStorage;
    use chrono::Duration;

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
    fn test_evaluate_graph_rules_preserves_invalid_hierarchy_error() {
        let (repo, storage, _) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let config = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        let executor = crate::test_utils::file_executor(repo.path()).unwrap();

        let error = executor
            .validation()
            .evaluate_graph_rules(&[])
            .expect_err("invalid hierarchy must fail graph-rule evaluation");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
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

        let (fixes, _messages) = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
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
        let (again, _) = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
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
        let error = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap_err();
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

        let error = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap_err();
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

        let (repaired, _) = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();

        assert_eq!(repaired, 0, "no profile-owned target is a repair action");
        assert_eq!(
            std::fs::read_to_string(&asset).unwrap(),
            "EDITED PROFILE ASSET\n"
        );
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

        let (first, _) = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        assert!(first > 0);
        let repaired = storage.load_issue(&id).unwrap();
        assert!(repaired.labels.iter().any(|label| label == "type:task"));
        assert!(!repaired.labels.iter().any(|label| label == "type:taks"));

        let (second, _) = executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        assert_eq!(second, 0, "a captured type repair must not repeat");
    }

    #[test]
    fn test_apply_type_fix_preserves_concurrent_type_correction() {
        use crate::commands::test_helpers::{memory_executor, with_apply_race, SessionRaceAction};
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        let executor = memory_executor(storage.clone());
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

        executor
            .validation()
            .apply_type_fix(&id, "taks", "task")
            .unwrap();

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

    const ONE_RULE: &str = "\
[[rules]]
name = \"require-success-criteria\"
assert = { require-section = { heading = \"Success Criteria\" } }
";

    const ONE_GATE: &str = "\
[[gates]]
version = 1
key = \"cargo-ci\"
title = \"Cargo CI\"
description = \"Full Rust CI pipeline must pass.\"
stage = \"postcheck\"
mode = \"manual\"
priority = 100
";

    #[test]
    fn test_dangling_link_findings_resolves_enforces_rule_and_gate_links() {
        // REQ-01 (jit:d30695e4): `rule` and `gate` declaring
        // `link-namespaces = ["enforces"]` means the SAME generic dangling-link
        // pass that already resolves `enforces:@/<invariant-id>` above now also
        // resolves an `enforces:@/rule/<name>` and `enforces:@/gate/<key>` label —
        // no new resolution code, just the kind-config declaration.
        let node = issue_with_labels(
            "node",
            "",
            &[
                "enforces:@/rule/require-success-criteria",
                "enforces:@/gate/cargo-ci",
            ],
        );
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant", "rule", "gate"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                rules: Some(ONE_RULE),
                invariants: Some(REGISTRY_TOML),
                gates: Some(ONE_GATE),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), node);
        let issues = exec.storage().list_issues().unwrap();
        assert!(
            exec.validation()
                .dangling_link_findings(&issues)
                .unwrap()
                .is_empty(),
            "enforces: rule/gate links must resolve, not dangle"
        );
    }

    #[test]
    fn test_dangling_link_findings_reports_unresolvable_enforces_rule_and_gate_links() {
        // The negative case: an `enforces:` link to a rule/gate self-id that does
        // not exist in the registry is qualified but unresolvable, so it is
        // reported as dangling rather than silently ignored.
        let node = issue_with_labels(
            "node",
            "",
            &[
                "enforces:@/rule/no-such-rule",
                "enforces:@/gate/no-such-gate",
            ],
        );
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant", "rule", "gate"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                rules: Some(ONE_RULE),
                invariants: Some(REGISTRY_TOML),
                gates: Some(ONE_GATE),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), node);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings
            .iter()
            .any(|f| f.finding.message.contains("no-such-rule")));
        assert!(findings
            .iter()
            .any(|f| f.finding.message.contains("no-such-gate")));
    }

    #[test]
    fn test_dangling_link_findings_reports_unresolvable_qualified_id() {
        // REQ-03: a node carrying `satisfies:<scope>/BOGUS` (a registered link
        // namespace, qualified, but unresolvable) yields one error-severity
        // finding naming the node and the dangling qualified id.
        let target = issue_with_labels(
            "target",
            "## Success Criteria\n\n- [hard] REQ-01: real one\n",
            &[],
        );
        let short = target.short_id();
        let node = issue_with_labels("node", "", &[&format!("satisfies:{short}/BOGUS")]);
        let node_id = node.id.clone();
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant", "requirement"]);
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

        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].issue_id.as_ref(), Some(&node_id));
        assert_eq!(
            findings[0].finding.rule,
            BuiltInPass::DanglingItemLink.as_str()
        );
        assert_eq!(
            findings[0].finding.severity,
            crate::declarations::rules::Severity::Error
        );
        assert!(findings[0].finding.message.contains("BOGUS"));
        assert!(findings[0].finding.message.contains("dangling item link"));
    }

    #[test]
    fn test_dangling_link_findings_resolvable_link_no_finding() {
        // A resolvable qualified link produces no finding, across all four kinds:
        // requirement/decision/risk (issue-scope) and invariant (project-scope).
        let target = issue_with_labels(
            "target",
            "## Success Criteria\n\n- [hard] REQ-01: a\n\n\
             ## Decisions\n\n- D-01: a\n\n\
             ## Risks\n\n- RISK-01: a\n",
            &[],
        );
        let short = target.short_id();
        let node = issue_with_labels(
            "node",
            "",
            &[
                &format!("satisfies:{short}/REQ-01"),
                &format!("per:{short}/D-01"),
                &format!("mitigates:{short}/RISK-01"),
                "enforces:@/invariant/sample-invariant",
            ],
        );
        let config = crate::test_utils::shipped_item_kind_tables(&[
            "invariant",
            "requirement",
            "decision",
            "risk",
        ]);
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

        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert!(
            findings.is_empty(),
            "resolvable links must produce no finding, got: {findings:?}"
        );
    }

    #[test]
    fn test_dangling_link_findings_classifies_explicit_kind_forms() {
        // REQ-03: the explicit `@/<kind>/<self-id>` and
        // `@/issue/<short>/<kind>/<self-id>` forms are classified as qualified
        // references — a resolvable one yields no finding.
        let target =
            issue_with_labels("target", "## Success Criteria\n\n- [hard] REQ-01: a\n", &[]);
        let short = target.short_id();
        let node = issue_with_labels(
            "node",
            "",
            &[
                "enforces:@/invariant/sample-invariant",
                &format!("satisfies:@/issue/{short}/requirement/REQ-01"),
            ],
        );
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant", "requirement"]);
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
        let issues = exec.storage().list_issues().unwrap();
        assert!(
            exec.validation()
                .dangling_link_findings(&issues)
                .unwrap()
                .is_empty(),
            "resolvable explicit-kind links must produce no finding"
        );

        // An unresolvable explicit form is still classified as qualified and
        // reported as dangling, never silently ignored as unqualified.
        let bad = issue_with_labels("bad", "", &["enforces:@/invariant/missing-invariant"]);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(REGISTRY_TOML),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), bad);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("missing-invariant"));
    }

    #[test]
    fn test_dangling_link_findings_named_project_form_is_dangling_not_panic() {
        // Audit (jit:7a2bbe4f) REQ-02: a named-project `@<name>/<kind>/<self-id>`
        // link value is a qualified reference (`is_qualified_reference` sees the
        // `/`), and since named-project RESOLUTION is not yet wired
        // (task a1b6b3da), it resolves through the SAME path as any other
        // unresolvable qualified id: a dangling-link finding, never a panic or
        // a silently-ignored label.
        let node = issue_with_labels("node", "", &["enforces:@acme/invariant/sample-invariant"]);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(REGISTRY_TOML),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), node);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("dangling item link"));
        // The dangling classification comes from `resolve_link_label` erroring on
        // the SAME value. Assert that error's chain carries the three PARSED
        // components in distinct phrasing, proving the named-project value was
        // structurally routed to (project, kind, self-id) rather than mis-split.
        let err = exec
            .items()
            .resolve_link_label("enforces:@acme/invariant/sample-invariant")
            .unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("project 'acme'"), "got: {chain}");
        assert!(chain.contains("kind 'invariant'"), "got: {chain}");
        assert!(chain.contains("self-id 'sample-invariant'"), "got: {chain}");
    }

    #[test]
    fn test_dangling_link_findings_reports_unqualified_link_and_ignores_non_link_ns() {
        // An unqualified link label is reported once; non-link namespaces are not.
        let node = issue_with_labels("node", "", &["satisfies:REQ-01", "type:task", "epic:foo"]);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant", "requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(REGISTRY_TOML),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), node);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].finding.message.contains("'satisfies:REQ-01'"));
    }

    #[test]
    fn test_dangling_invariant_link_reports_finding() {
        // REQ-03 for the project-scope invariant kind: `enforces:@/invariant/missing-invariant`
        // is a registered link namespace with a qualified-but-unresolvable id.
        let node = issue_with_labels("node", "", &["enforces:@/invariant/missing-invariant"]);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(REGISTRY_TOML),
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), node);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.validation().dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("missing-invariant"));
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
            msg.contains(BuiltInPass::DanglingItemLink.as_str()) && msg.contains("BOGUS"),
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
}
