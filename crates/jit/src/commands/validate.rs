//! Validation and status operations

use super::*;
use crate::domain::type_taxonomy::{
    detect_validation_issues, generate_fixes, ValidationFix, ValidationIssue,
};
use crate::domain::SHORT_ID_LENGTH;
use anyhow::Context;

/// Rule name carried by every finding the built-in dangling-item-link pass emits
/// (REQ-08 clause i, REQ-03). It is not a `.jit/rules.toml` rule; the pass runs
/// unconditionally, so the name is a stable constant for grouping and rendering.
pub const DANGLING_LINK_RULE: &str = "dangling-item-link";

/// Rule name carried by every finding the built-in enforcement-drift pass emits
/// (REQ-01/REQ-02). Like [`DANGLING_LINK_RULE`], it is not a `.jit/rules.toml`
/// rule: the pass runs as part of every validate path, gated only on the
/// presence of declared invariants, so the name is a stable constant for
/// grouping and rendering.
pub const ENFORCEMENT_DRIFT_RULE: &str = "enforcement-drift";

/// Rule name carried by the built-in advisory finding emitted while one or
/// more gate definitions still use [`GateChecker::ReviewPlaceholder`].
pub const REVIEW_PLACEHOLDER_RULE: &str = "review-placeholder";

struct CapturedRepairPlan {
    image: crate::repository_state::RepositoryImage,
    declarations: crate::repository_state::CapturedRepositoryDeclarations,
    plan: Option<crate::repository_state::MaterializationPlan>,
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> CommandExecutor<S> {
    /// Validate with optional fix mode.
    ///
    /// # Arguments
    ///
    /// * `fix` - If true, attempt to fix validation issues
    /// * `dry_run` - If true, show what would be fixed without applying changes
    ///
    /// # Returns
    ///
    /// Returns Ok with (count of fixes applied, messages), or Err if validation fails
    pub fn validate_with_fix(&mut self, fix: bool, dry_run: bool) -> Result<(usize, Vec<String>)> {
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
            // through the recovered session's transactional repair delta.
            let (repair_fixes, mut messages) = self.repair_derived_state(dry_run)?;
            total_fixes += repair_fixes;
            all_messages.append(&mut messages);

            // Add summary message
            if dry_run {
                all_messages.push(format!(
                    "\nDry run complete. {} fixes would be applied.",
                    total_fixes
                ));
            } else if total_fixes > 0 {
                all_messages.push(format!("\n✓ Applied {} fixes", total_fixes));
                // Re-run validation to verify fixes worked
                self.validate_silent()?;
                all_messages.push("✓ Repository is now valid".to_string());
            } else {
                all_messages.push("\n✓ No fixes needed".to_string());
            }

            return Ok((total_fixes, all_messages));
        }

        // Not in fix mode, just propagate the validation error
        validation_result?;
        Ok((0, vec![]))
    }

    /// Validate the exact repository through the closed-image pipeline.
    ///
    /// Every backend captures a bounded whole-repository image through the recovered
    /// mutation session and validates it with
    /// [`validate_repository`](crate::validation::repository::validate_repository),
    /// so every pass reads only image-projected content and no ambient filesystem or
    /// Git I/O. The same canonical repair plan supplies derived-state expectations
    /// for validation and `--fix`, including exactly proven installed-profile assets
    /// and regions.
    pub fn validate_silent(&self) -> Result<()> {
        let report = self.capture_validation_report()?.0?;
        if report.rule_report.has_errors() {
            let messages = report
                .rule_report
                .findings
                .iter()
                .filter(|finding| finding.is_error())
                .map(|finding| format!("[{}] {}", finding.rule, finding.message))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow!(
                "Validation failed with {} rule error(s):\n{}",
                report.rule_report.error_count(),
                messages
            ));
        }
        Ok(())
    }

    /// Capture the bounded whole-repository validation image through the recovered
    /// session (plan §2 two-phase capture, D14).
    ///
    /// Phase one reads the engine registries, the repository index, and the event
    /// log; phase two adds the complete validation closure
    /// ([`validate_capture_closure`](crate::repository_state::validate_capture_closure)):
    /// every issue record and the complete issues listing, referenced rule schemas,
    /// and every projection and item-kind source. Phase three adds every issue's
    /// document and pinned-document evidence plus each derived plan-document path,
    /// enumerated from the captured issue records — the command boundary owns the
    /// planning-node resolution the pure closure cannot express. A read-set change
    /// under the held session yields a typed retryable conflict, so the capture
    /// repeats both phases rather than grafting into a stale image.
    fn capture_validation_image(&self) -> Result<crate::repository_state::RepositoryImage> {
        self.capture_validation_image_with(&std::collections::BTreeMap::new())
    }

    /// Capture the whole-repository validation image, optionally over a proposed
    /// overlay.
    ///
    /// With an empty `overrides` this captures and validates the live repository.
    /// With a non-empty overlay (the projection of a proposed
    /// [`RepositoryDelta`](crate::repository_state::RepositoryDelta) to final
    /// bytes/absence) the closure is computed from the OVERLAID declarations — so
    /// it covers what the proposed state's passes read — the base is captured over
    /// that closure, and the overlay is applied
    /// ([`apply_overlay`](crate::repository_state::apply_overlay)) so
    /// `validate_repository` judges one coherent proposed repository under the same
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

    /// One bounded two-phase capture attempt of the whole-repository validation
    /// base under an already-held session.
    ///
    /// Returns the captured PRE-overlay base (the caller applies any overlay for
    /// validation and reads it for its own delta), `Ok(None)` on a retryable
    /// read-set conflict, or an error otherwise. `overrides` scopes the closure to
    /// the proposed declarations (so a proposed init/profile state captures what
    /// its passes read); `extra_paths` discovers additional delta targets (init
    /// schema files, profile assets, directories, provenance) into the base so a
    /// subsequent `apply` finds each action's captured preimage. Sharing one held
    /// session lets a caller capture the base and apply its delta under the same
    /// guard without a second, deadlock-prone session.
    pub(crate) fn capture_proposed_base(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        overrides: &std::collections::BTreeMap<
            crate::repository_state::VirtualPath,
            Option<Vec<u8>>,
        >,
        extra_paths: &[crate::repository_state::VirtualPath],
        precheck_target: Option<&str>,
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        self.capture_proposed_base_inner(
            session,
            overrides,
            extra_paths,
            &[],
            precheck_target,
            true,
            false,
        )
    }

    pub(crate) fn capture_proposed_base_without_documents(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        self.capture_proposed_base_inner(
            session,
            &std::collections::BTreeMap::new(),
            &[],
            &[],
            None,
            false,
            true,
        )
    }

    /// The whole-repository validation base plus a complete listing of each
    /// path in `extra_listings`.
    ///
    /// Derived-state repair lists `.jit/profiles/` because the applied-profile
    /// records it holds are the repository's own statement of which profile
    /// packages, if any, repair needs; an exact-path closure cannot express a
    /// question whose answer is the directory's occupants.
    fn capture_proposed_base_with_listings(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        extra_paths: &[crate::repository_state::VirtualPath],
        extra_listings: &[crate::repository_state::VirtualPath],
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        self.capture_proposed_base_inner(
            session,
            &std::collections::BTreeMap::new(),
            extra_paths,
            extra_listings,
            None,
            true,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_proposed_base_inner(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        overrides: &std::collections::BTreeMap<
            crate::repository_state::VirtualPath,
            Option<Vec<u8>>,
        >,
        extra_paths: &[crate::repository_state::VirtualPath],
        extra_listings: &[crate::repository_state::VirtualPath],
        precheck_target: Option<&str>,
        capture_documents: bool,
        archive_scan: bool,
    ) -> Result<Option<crate::repository_state::RepositoryImage>> {
        use crate::repository_state::{
            validate_capture_closure, CaptureBudget, CaptureSpec, VirtualPath,
        };

        // An archive citation scan may enumerate every directory below its
        // declared roots. A complete listing consumes one distinct captured
        // path, so archive capture binds that ceiling to the existing exact-path
        // closure limit rather than imposing a smaller repository-size cap.
        let max_paths = 1 << 16;
        let budget = CaptureBudget {
            max_paths,
            max_listings: if archive_scan { max_paths } else { 256 },
            max_bytes: 512 * 1024 * 1024,
            max_depth: 32,
        };
        let registries = || -> Result<Vec<VirtualPath>> {
            Ok(vec![
                VirtualPath::CONFIG,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::GATES,
                VirtualPath::TEMPLATES,
                VirtualPath::INDEX,
                VirtualPath::EVENTS,
            ])
        };
        // Effective bytes at a repo-relative path: the override (create/replace or
        // proposed absence) when present, otherwise the captured base bytes.
        let effective = |image: &crate::repository_state::RepositoryImage,
                         repo_rel: &str|
         -> Result<Option<Vec<u8>>> {
            let vpath = image.layout().classify_repository_relative(repo_rel)?;
            match overrides.get(&vpath) {
                Some(value) => Ok(value.clone()),
                None => super::image_repo_bytes(image, repo_rel),
            }
        };
        let Some(image_one) =
            capture_or_retry(session.capture(CaptureSpec::phase_one(registries()?, budget)?))?
        else {
            return Ok(None);
        };
        // Closure planning is best-effort: a malformed registry cannot fail the
        // capture, because validation itself is the authority that reports it.
        // A bad config/index/rules yields a conservative closure (its projection,
        // item-kind, issue, and schema entries drop out), and the subsequent
        // `validate_repository` pass surfaces the parse error as a finding.
        let config = effective_config(&image_one, &effective)
            .unwrap_or_else(|_| empty_materialization_config());
        let all_ids = effective_index_ids(&image_one, &effective).unwrap_or_default();
        let rules_text = effective(&image_one, ".jit/rules.toml")?
            .map(String::from_utf8)
            .transpose()?;
        let schema_rules = rules_text.as_deref().filter(|content| {
            crate::declarations::rules::RuleSet::schema_requests(content).is_ok()
        });
        let closure =
            validate_capture_closure(image_one.layout(), &config, &all_ids, schema_rules)?;

        let mut spec = CaptureSpec::phase_one(registries()?, budget)?;
        spec.discover_paths(closure.paths)?;
        for listing in &closure.listings {
            spec.discover_listing(listing.clone())?;
        }
        let mut phase_three = spec.clone();
        let image_two = match capture_or_retry(session.capture(spec))? {
            Some(image) if image.has_stable_overlap(&image_one) => image,
            Some(_) | None => return Ok(None),
        };

        let issues = effective_issues(&image_two, &all_ids, &effective)?;
        let mut full_config = config.clone();
        full_config.templates = effective_templates(&image_two, &config, &effective)?;
        let (worktree_docs, pinned) = if capture_documents {
            document_capture_closure(&issues, &full_config)?
        } else {
            (Vec::new(), Vec::new())
        };
        phase_three.discover_paths(worktree_docs)?;
        let mut capture_precheck_history = false;
        if let Some(raw_target) = precheck_target {
            let target = super::resolve_issue_from_capture(&issues, raw_target)?;
            let issue = issues
                .iter()
                .find(|issue| issue.id == target)
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(&target))?;
            let gates = match effective(&image_two, ".jit/gates.toml")? {
                Some(bytes) => crate::declarations::parse_gate_registry(&bytes)?,
                None => crate::declarations::GateRegistry::default(),
            };
            capture_precheck_history = issue
                .gates_required
                .iter()
                .filter_map(|key| gates.gates.get(key))
                .any(|gate| {
                    gate.stage == crate::declarations::GateStage::Precheck
                        && gate.mode == crate::declarations::GateMode::Auto
                        && gate
                            .checker
                            .as_ref()
                            .is_some_and(super::checker_consumes_run_history)
                });
            let prompt_paths = issue
                .gates_required
                .iter()
                .filter_map(|key| gates.gates.get(key))
                .filter(|gate| {
                    gate.stage == crate::declarations::GateStage::Precheck
                        && gate.mode == crate::declarations::GateMode::Auto
                })
                .filter_map(|gate| gate.checker.as_ref())
                .filter_map(|checker| match checker {
                    crate::declarations::GateChecker::Exec {
                        pass_context: true,
                        prompt_file: Some(path),
                        ..
                    } => Some(path),
                    _ => None,
                });
            phase_three.discover_paths(
                prompt_paths
                    .map(|path| {
                        image_two
                            .layout()
                            .classify_repository_relative(path)
                            .map_err(|_| {
                                anyhow!("prompt_file '{}' resolves outside the repository", path)
                            })
                    })
                    .collect::<Result<Vec<_>>>()?,
            )?;
            if capture_precheck_history {
                let gate_runs = VirtualPath::GATE_RUNS;
                phase_three.discover_paths([gate_runs.clone()])?;
                phase_three.discover_listing(gate_runs)?;
            }
        }
        phase_three.discover_paths(extra_paths.iter().cloned())?;
        for listing in extra_listings {
            phase_three.discover_listing(listing.clone())?;
        }
        for (revision, path) in pinned {
            phase_three.discover_pinned(revision, path)?;
        }
        let image_three = match capture_or_retry(session.capture(phase_three.clone()))? {
            Some(base) if base.has_stable_overlap(&image_two) => base,
            Some(_) | None => return Ok(None),
        };
        if !capture_precheck_history {
            return Ok(Some(image_three));
        }
        super::capture_gate_run_results(session, image_three)
    }

    /// Capture the validation image and return the full whole-repository report.
    ///
    /// The outer `Result` carries a capture/IO failure; the inner result is the
    /// validation outcome — `Ok(report)` when clean, or the
    /// [`RepositoryValidationFailure`](crate::validation::repository::RepositoryValidationFailure)
    /// carrying its structural error and partial report. Rendering callers (the
    /// `validate` command's non-fix path) consume the report's warnings and rule
    /// findings even on failure.
    pub fn validate_repository_report(
        &self,
    ) -> Result<
        std::result::Result<
            crate::validation::repository::RepositoryValidationReport,
            crate::validation::repository::RepositoryValidationFailure,
        >,
    > {
        Ok(self.capture_validation_report()?.0)
    }

    /// Validate and derive advisory hierarchy divergences from one captured image.
    pub fn validate_repository_report_with_divergences(
        &self,
    ) -> Result<(
        std::result::Result<
            crate::validation::repository::RepositoryValidationReport,
            crate::validation::repository::RepositoryValidationFailure,
        >,
        crate::output::DivergenceResponse,
    )> {
        self.capture_validation_report()
    }

    fn capture_validation_report(
        &self,
    ) -> Result<(
        std::result::Result<
            crate::validation::repository::RepositoryValidationReport,
            crate::validation::repository::RepositoryValidationFailure,
        >,
        crate::output::DivergenceResponse,
    )> {
        let layout = self.require_layout()?;
        let seed = repair_seed()?;
        with_mutation_session(
            self.storage(),
            &layout,
            "repository validation",
            |session| {
                let Some(derived) = self.capture_repair_plan(session, &seed)? else {
                    return Ok(SessionStep::Retry);
                };
                let captured = match derived {
                    Ok(derived) => derived,
                    Err(failure) => {
                        return Ok(SessionStep::Done((
                            Err(failure),
                            crate::output::DivergenceResponse {
                                count: 0,
                                divergences: Vec::new(),
                            },
                        )));
                    }
                };
                let divergences =
                    captured_divergences(&captured.image, captured.declarations.config())
                        .unwrap_or_else(|_| crate::output::DivergenceResponse {
                            count: 0,
                            divergences: Vec::new(),
                        });
                let report =
                    crate::validation::repository::validate_repository_with_materializations(
                        &captured.image,
                        &captured.declarations,
                        captured.plan.as_ref(),
                    );
                Ok(SessionStep::Done((report, divergences)))
            },
        )
    }

    /// Repair derived state (default rules/schemas plus every configured
    /// projection) through the recovered session (`jit validate --fix`).
    ///
    /// The complete owned-materialization set is derived from declared authority
    /// over one captured image (`MaterializationRequest::RepairDerivedState`) and
    /// applied through the same session; a coherent repository yields an empty
    /// delta (no action). Repair is ownership-safe by construction — `rules.toml`
    /// splices only the generated default spans and region projections splice only
    /// their managed region, so authored content is preserved. In `dry_run` the
    /// delta is derived but not applied. Returns the number of repaired targets and
    /// a per-target message. JSON and memory stores execute this same path.
    fn repair_derived_state(&self, dry_run: bool) -> Result<(usize, Vec<String>)> {
        use crate::repository_state::RepositoryAction;

        let layout = self.require_layout()?;
        let seed = repair_seed()?;
        with_mutation_session(self.storage(), &layout, "derived-state repair", |session| {
            let Some(derived) = self.capture_repair_plan(session, &seed)? else {
                return Ok(SessionStep::Retry);
            };
            let mut captured = derived.map_err(|failure| {
                crate::errors::ValidationFailedError::new(failure.to_string())
            })?;
            if let Some(error) = captured.declarations.take_rules_load_error() {
                return Err(crate::errors::ValidationFailedError::new(format!("{error:#}")).into());
            }
            let plan = captured.plan.ok_or_else(|| {
                anyhow!("derived-state repair has no plan for loadable declarations")
            })?;
            // One message per changed owned target. Include mode-only drift: an
            // executable profile asset can have correct bytes but still require a
            // transaction, and treating that delta as empty would skip repair.
            let changed = plan
                .delta()
                .actions()
                .iter()
                .filter_map(|action| match action {
                    RepositoryAction::WriteFile { path, .. }
                    | RepositoryAction::DeleteFile { path, .. }
                    | RepositoryAction::SetMode { path, .. } => Some(path),
                    RepositoryAction::CreateDirectory { .. } => None,
                })
                .collect::<std::collections::BTreeSet<_>>();
            let messages = changed
                .into_iter()
                .map(|path| format!("✓ Repaired derived-state target {path:?}"))
                .collect::<Vec<_>>();
            if messages.is_empty() {
                return Ok(SessionStep::Done((0, Vec::new())));
            }
            if dry_run {
                return Ok(SessionStep::Done((messages.len(), messages)));
            }
            Ok(SessionStep::Apply(plan, (messages.len(), messages)))
        })
    }

    /// Capture one complete validation closure and derive its exact repair plan.
    ///
    /// Which profile packages this repository needs, and whether it needs any,
    /// is decided by its own applied-profile records: the `.jit/profiles/`
    /// listing names them, so a repository that has applied nothing captures no
    /// package target and resolves no package. A recorded profile contributes
    /// repair claims only when its captured record exactly matches the resolved
    /// package identity and target hashes; what that profile then owns comes
    /// from the package the record proves, never from a target's filename or
    /// occupant.
    ///
    /// A record whose package cannot be obtained fails the whole capture rather
    /// than repairing the targets still accounted for: a repair that silently
    /// narrows what it restores is a breach of
    /// `@/invariant/derived-state-coherence`.
    fn capture_repair_plan(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        seed: &crate::repository_state::RepositorySeed,
    ) -> Result<
        Option<
            std::result::Result<
                CapturedRepairPlan,
                crate::validation::repository::RepositoryValidationFailure,
            >,
        >,
    > {
        use crate::repository_state::{
            derive_materialization, MaterializationRequest, VirtualPath,
        };

        let profiles_dir = VirtualPath::PROFILES;
        let listings = std::slice::from_ref(&profiles_dir);
        let Some(discovered) = self.capture_proposed_base_with_listings(session, &[], listings)?
        else {
            return Ok(None);
        };
        let recorded = recorded_profile_ids(&discovered, &profiles_dir)?;
        let packages = match resolve_recorded_packages(&recorded)? {
            Ok(packages) => packages,
            Err(failure) => return Ok(Some(Err(failure))),
        };

        let image = if packages.is_empty() {
            discovered
        } else {
            let layout = self.require_layout()?;
            let profile_paths = packages
                .iter()
                .flat_map(|(_, package)| package.hashes().targets.keys())
                .map(|path| {
                    layout
                        .classify_repository_relative(path)
                        .map_err(Into::into)
                })
                .chain(
                    packages
                        .iter()
                        .map(|(record_path, _)| Ok(record_path.clone())),
                )
                .collect::<Result<Vec<_>>>()?;
            let Some(image) =
                self.capture_proposed_base_with_listings(session, &profile_paths, listings)?
            else {
                return Ok(None);
            };
            // The closure was planned from the record set the first capture saw;
            // a concurrent application or removal restarts the attempt rather
            // than deriving repair over a stale answer.
            if recorded_profile_ids(&image, &profiles_dir)? != recorded {
                return Ok(None);
            }
            image
        };
        let profiles = match captured_profile_repair_claims(&image, &packages)? {
            Some(Ok(profiles)) => profiles,
            Some(Err(failure)) => return Ok(Some(Err(failure))),
            None => return Ok(None),
        };
        let declarations = match crate::repository_state::validation_declarations_from_image(&image)
        {
            Ok(declarations) => declarations,
            Err(error) => {
                return Ok(Some(Err(
                    crate::validation::repository::RepositoryValidationFailure::declaration(
                        error.into(),
                    ),
                )))
            }
        };
        let plan =
            if declarations.rules_loaded() {
                match derive_materialization(
                    &image,
                    MaterializationRequest::RepairDerivedState {
                        declarations: declarations.borrowed(),
                        profiles,
                        seed,
                    },
                ) {
                    Ok(plan) => Some(plan),
                    Err(error) => return Ok(Some(Err(
                        crate::validation::repository::RepositoryValidationFailure::materialization(
                            error.into(),
                        ),
                    ))),
                }
            } else {
                None
            };
        Ok(Some(Ok(CapturedRepairPlan {
            image,
            declarations,
            plan,
        })))
    }
}

/// Profile ids the repository's own applied-profile records name.
///
/// Application writes one record per applied profile at
/// `.jit/profiles/<id>.json`, so the listing of that directory is where the
/// repository states which packages it needs. A child whose name is not a
/// record name is not a record.
fn recorded_profile_ids(
    image: &crate::repository_state::RepositoryImage,
    profiles_dir: &crate::repository_state::VirtualPath,
) -> Result<std::collections::BTreeSet<String>> {
    Ok(image
        .listing_fingerprints()
        .get(profiles_dir)
        .ok_or_else(|| anyhow!("capture did not list {profiles_dir:?}"))?
        .children()
        .keys()
        .filter_map(|name| super::profile::record_name_profile_id(name))
        .map(str::to_string)
        .collect())
}

/// Resolve one package, and its record's canonical path, per recorded profile.
///
/// The first record whose package cannot be obtained is the whole answer:
/// repairing the profiles that did resolve would narrow what repair restores
/// without reporting it (`@/invariant/derived-state-coherence`).
#[allow(clippy::type_complexity)]
fn resolve_recorded_packages(
    recorded: &std::collections::BTreeSet<String>,
) -> Result<
    std::result::Result<
        Vec<(
            crate::repository_state::VirtualPath,
            crate::profile::ProfilePackage,
        )>,
        crate::validation::repository::RepositoryValidationFailure,
    >,
> {
    Ok(recorded
        .iter()
        .map(|id| {
            let record_path = super::profile::applied_record_path(id)?;
            Ok(super::profile::embedded_profile(id)
                .map(|package| (record_path.clone(), package))
                .map_err(|error| {
                    crate::validation::repository::RepositoryValidationFailure::materialization(
                        error.context(format!(
                            "applied profile record '{}' names profile '{id}', whose package cannot be obtained",
                            super::profile::repo_string(&record_path)
                        )),
                    )
                }))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .collect())
}

/// Repair claims for every recorded profile whose captured record proves it.
///
/// `Ok(None)` reports a listed record the capture no longer sees, which the
/// caller retries rather than repairing without it.
#[allow(clippy::type_complexity)]
fn captured_profile_repair_claims(
    image: &crate::repository_state::RepositoryImage,
    packages: &[(
        crate::repository_state::VirtualPath,
        crate::profile::ProfilePackage,
    )],
) -> Result<
    Option<
        std::result::Result<
            Vec<crate::repository_state::ProfileClaims>,
            crate::validation::repository::RepositoryValidationFailure,
        >,
    >,
> {
    use crate::repository_state::RepositoryEntry;
    use crate::validation::repository::RepositoryValidationFailure;

    let mut claims = Vec::with_capacity(packages.len());
    for (record_path, package) in packages {
        let metadata = &package.manifest().profile;
        match image.entry(record_path)? {
            RepositoryEntry::Absent => return Ok(None),
            RepositoryEntry::File { bytes, .. } => {
                let actual: crate::repository_state::AppliedProfileRecord =
                    match serde_json::from_slice(bytes)
                        .context("invalid applied profile provenance")
                    {
                        Ok(actual) => actual,
                        Err(error) => {
                            return Ok(Some(Err(RepositoryValidationFailure::materialization(
                                error,
                            ))))
                        }
                    };
                if actual != super::profile::expected_record(package, image.layout())? {
                    return Ok(Some(Err(RepositoryValidationFailure::materialization(
                        anyhow!(
                            "applied profile provenance for '{}@{}' does not match the resolvable embedded package",
                            metadata.id,
                            metadata.version
                        ),
                    ))));
                }
                match crate::profile::build_profile_repair_claims(package, image.layout()) {
                    Ok(built) => claims.push(built),
                    Err(error) => {
                        return Ok(Some(Err(RepositoryValidationFailure::materialization(
                            error.into(),
                        ))))
                    }
                }
            }
            _ => {
                return Ok(Some(Err(RepositoryValidationFailure::materialization(
                    anyhow!(
                        "applied profile provenance path {:?} is not a regular file",
                        record_path
                    ),
                ))))
            }
        }
    }
    Ok(Some(Ok(claims)))
}

fn captured_divergences(
    image: &crate::repository_state::RepositoryImage,
    config: &JitConfig,
) -> Result<crate::output::DivergenceResponse> {
    use crate::domain::type_taxonomy::HierarchyConfig;
    use crate::output::{DivergenceResponse, DivergenceView};

    let issues = captured_active_issues(image)?;
    let namespaces = crate::config_manager::namespaces_from_config(config);
    let hierarchy = HierarchyConfig::new(
        namespaces.declared_type_hierarchy(),
        namespaces.label_associations.unwrap_or_default(),
    )?;
    let issue_refs = issues.iter().collect::<Vec<_>>();
    let divergences =
        crate::graph::hierarchy::detect_membership_divergences(&issue_refs, &hierarchy);
    let by_id = issues
        .iter()
        .map(|issue| (issue.id.as_str(), issue))
        .collect::<std::collections::HashMap<_, _>>();
    let divergences = divergences
        .into_iter()
        .map(|divergence| {
            let issue = by_id.get(divergence.issue_id.as_str());
            DivergenceView {
                short_id: divergence.issue_id.chars().take(8).collect(),
                title: issue.map(|issue| issue.title.clone()).unwrap_or_default(),
                id: divergence.issue_id,
                label: divergence.label,
                namespace: divergence.namespace,
                value: divergence.value,
            }
        })
        .collect::<Vec<_>>();
    Ok(DivergenceResponse {
        count: divergences.len(),
        divergences,
    })
}

fn repair_seed() -> Result<crate::repository_state::RepositorySeed> {
    crate::repository_state::RepositorySeed::new(
        crate::repository_state::RepositorySeedKind::Command {
            name: "validate derived state".to_string(),
        },
        Default::default(),
        Default::default(),
    )
    .map_err(Into::into)
}

/// Effective-bytes byte source over a captured image and a proposed overlay.
type Effective<'a> =
    dyn Fn(&crate::repository_state::RepositoryImage, &str) -> Result<Option<Vec<u8>>> + 'a;

/// Parse the (possibly overlaid) configuration and invariant registry.
fn effective_config(
    image: &crate::repository_state::RepositoryImage,
    effective: &Effective<'_>,
) -> Result<JitConfig> {
    let config_bytes = effective(image, ".jit/config.toml")?
        .ok_or_else(|| anyhow!("captured image has no .jit/config.toml"))?;
    let declarations = crate::declarations::parse_configuration(&config_bytes)
        .context("invalid .jit/config.toml")?;
    let invariants = match effective(image, ".jit/invariants.toml")? {
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

/// Parse the (possibly overlaid) repository index's live issue ids.
fn effective_index_ids(
    image: &crate::repository_state::RepositoryImage,
    effective: &Effective<'_>,
) -> Result<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct IndexIds {
        #[serde(default)]
        all_ids: Vec<String>,
    }
    let bytes = effective(image, ".jit/index.json")?
        .ok_or_else(|| anyhow!("captured image has no .jit/index.json"))?;
    Ok(serde_json::from_slice::<IndexIds>(&bytes)?.all_ids)
}

/// Parse every ordinary issue record named by the index (possibly overlaid).
fn effective_issues(
    image: &crate::repository_state::RepositoryImage,
    all_ids: &[String],
    effective: &Effective<'_>,
) -> Result<Vec<Issue>> {
    all_ids
        .iter()
        .map(|id| {
            let bytes = effective(image, &format!(".jit/issues/{id}.json"))?
                .ok_or_else(|| anyhow!("captured image has no issue file for '{id}'"))?;
            Ok(serde_json::from_slice::<Issue>(&bytes)?)
        })
        .collect()
}

/// Parse the (possibly overlaid) template registry, needed to derive plan-document
/// paths for the phase-three document closure.
///
/// Best-effort like the rest of closure planning: a malformed `templates.toml`
/// yields an empty registry (no plan-document paths), and the subsequent
/// `validate_repository` pass surfaces the parse error as a validation failure.
fn effective_templates(
    image: &crate::repository_state::RepositoryImage,
    config: &JitConfig,
    effective: &Effective<'_>,
) -> Result<crate::templates::TemplateRegistry> {
    let hierarchy_types: Vec<&str> = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
        .unwrap_or_default();
    Ok(match effective(image, ".jit/templates.toml")? {
        Some(bytes) => String::from_utf8(bytes)
            .ok()
            .and_then(|text| {
                crate::templates::TemplateRegistry::from_toml_str(&text, &hierarchy_types).ok()
            })
            .unwrap_or_else(crate::templates::TemplateRegistry::empty),
        None => crate::templates::TemplateRegistry::empty(),
    })
}

/// The phase-three document closure: working-tree document/plan paths to capture,
/// plus pinned-document `(revision, path)` evidence requests.
type DocumentClosure = (
    Vec<crate::repository_state::VirtualPath>,
    Vec<(String, String)>,
);

/// Enumerate the phase-three document and plan-document capture closure.
///
/// Returns every working-tree document path to capture and every pinned-document
/// `(revision, path)` request: a document pinned to a commit is captured as
/// pinned evidence at that commit; an unpinned document is captured as its
/// working-tree entry plus HEAD evidence for the fallback. Each breakable
/// container's derived plan-document path (mirroring plan-content projection's
/// planning-node resolution) is added as a working-tree path so a container whose
/// criteria live in an external plan validates against image-projected content.
fn document_capture_closure(issues: &[Issue], config: &JitConfig) -> Result<DocumentClosure> {
    use crate::document::DocumentReferenceRequests;
    use crate::repository_state::VirtualPath;
    let mut worktree = Vec::new();
    let mut pinned = Vec::new();
    for issue in issues {
        for document in &issue.documents {
            let requests = DocumentReferenceRequests::for_reference(document)?;
            worktree.extend(requests.worktree().cloned());
            let (revision, path) = requests.pinned();
            pinned.push((revision.to_string(), path.to_string()));
        }
    }
    let templates = &config.templates;
    let breakable: std::collections::HashSet<String> =
        templates.breakable_types().into_iter().collect();
    let by_id: std::collections::HashMap<&str, &Issue> = issues
        .iter()
        .map(|issue| (issue.id.as_str(), issue))
        .collect();
    for issue in issues {
        let Some(issue_type) = crate::labels::type_label_value(&issue.labels)
            .filter(|issue_type| breakable.contains(*issue_type))
        else {
            continue;
        };
        let Some(template) = templates.template_for_container(issue_type) else {
            continue;
        };
        if template.plan_doc_location(&templates.roles).is_none() {
            continue;
        }
        let planning =
            crate::commands::find_planning_node(issue, template, &templates.roles, &by_id);
        let Some(path) = planning.and_then(crate::commands::planning_node_plan_path) else {
            continue;
        };
        worktree.push(VirtualPath::worktree(&path)?);
    }
    Ok((worktree, pinned))
}

/// Project the plan-document content map from a captured validation image — the
/// closed-read replacement for the retired ambient filesystem
/// reads.
///
/// Planning-node resolution runs against the full captured issue set (so a scoped
/// slice still finds a bracket's planning node), while content is emitted only for
/// `emit_for`. Each breakable container whose bracket planning node records an
/// external plan reference contributes its plan-document bytes, read from the
/// image's captured plan-document entries (enqueued by [`document_capture_closure`])
/// rather than the live filesystem. A not-yet-authored plan whose planning node is
/// not `done` is omitted (a freshly-applied bracket validates cleanly before its
/// plan exists); a missing plan whose planning node is `done` is an error.
pub(crate) fn plan_content_from_image(
    image: &crate::repository_state::RepositoryImage,
    emit_for: &[Issue],
) -> Result<std::collections::HashMap<String, String>> {
    let effective = |img: &crate::repository_state::RepositoryImage,
                     repo_rel: &str|
     -> Result<Option<Vec<u8>>> { super::image_repo_bytes(img, repo_rel) };
    let effective: &Effective<'_> = &effective;

    // Declaration parsing is best-effort, mirroring `capture_proposed_base`: a
    // missing or malformed config/index/issue set yields an empty projection (no
    // breakable containers, so no external plan docs), and `validate_repository`
    // is the authority that reports the parse error. Only a genuinely missing
    // required plan document (below) is a hard error.
    let mut config =
        effective_config(image, effective).unwrap_or_else(|_| empty_materialization_config());
    config.templates = effective_templates(image, &config, effective)?;
    let all_ids = effective_index_ids(image, effective).unwrap_or_default();
    let all_issues = effective_issues(image, &all_ids, effective).unwrap_or_default();

    let templates = &config.templates;
    let breakable: std::collections::HashSet<String> =
        templates.breakable_types().into_iter().collect();
    let by_id: std::collections::HashMap<&str, &Issue> = all_issues
        .iter()
        .map(|issue| (issue.id.as_str(), issue))
        .collect();

    let mut out = std::collections::HashMap::new();
    for issue in emit_for {
        let Some(issue_type) =
            label_utils::type_label_value(&issue.labels).filter(|t| breakable.contains(*t))
        else {
            continue;
        };
        let Some(template) = templates.template_for_container(issue_type) else {
            continue;
        };
        if template.plan_doc_location(&templates.roles).is_none() {
            continue;
        }
        let planning = find_planning_node(issue, template, &templates.roles, &by_id);
        let Some(plan_path) = planning.and_then(planning_node_plan_path) else {
            continue;
        };
        match super::image_repo_bytes(image, &plan_path)? {
            Some(bytes) => {
                out.insert(issue.id.clone(), String::from_utf8(bytes)?);
            }
            None if planning.is_none_or(|node| node.state != State::Done) => {}
            None => return Err(anyhow!("required plan document '{plan_path}' is missing")),
        }
    }
    Ok(out)
}

impl<S: IssueStore> CommandExecutor<S> {
    fn detect_and_fix_hierarchy_issues(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::config_manager::get_hierarchy_config;

        let config = get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;

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
                    ValidationIssue::InvalidMembershipReference {
                        issue_id,
                        label,
                        reason,
                        ..
                    } => {
                        messages.push(format!(
                            "  • Issue {} has invalid membership label '{}': {}",
                            issue_id, label, reason
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
                        self.apply_type_fix(issue_id, old_type, new_type)?;
                        messages.push(format!(
                            "✓ Replaced type '{}' with '{}' for issue {}",
                            old_type, new_type, issue_id
                        ));
                    }
                    fixes_applied += 1;
                }
            }
        }

        Ok((fixes_applied, messages))
    }

    fn apply_type_fix(&self, issue_id: &str, old_type: &str, new_type: &str) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Replace the type label
        let old_label = label_utils::type_label(old_type);
        let new_label = label_utils::type_label(new_type);

        self.publish_captured_field_update(CapturedFieldUpdate::label_edit(
            issue_id.to_string(),
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
    /// resolver [`resolve_link_label`](crate::commands::CommandExecutor::resolve_link_label),
    /// so resolution logic is not forked. A candidate that fails to resolve yields
    /// one [`GraphFinding`] attributed to the owning issue, naming the dangling
    /// qualified id; a resolvable candidate (and a legacy unqualified label, and a
    /// non-link namespace) yields nothing.
    ///
    /// The finding severity is [`Severity::Error`](crate::declarations::rules::Severity::Error)
    /// so a dangling link fails `jit validate`. The rule name is the constant
    /// [`DANGLING_LINK_RULE`]. This pass touches no `.jit/` ruleset: it runs
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
        let link_namespaces: BTreeSet<String> = self
            .item_kinds()?
            .iter()
            .flat_map(|kind| kind.link_namespaces().iter().cloned())
            .collect();

        let mut findings = Vec::new();
        for issue in issues {
            for label in &issue.labels {
                let Some((namespace, value)) = label.split_once(':') else {
                    continue;
                };
                // Only labels in a declared link-namespace whose value is a
                // qualified id are link references; everything else (legacy
                // unqualified labels, non-link namespaces) is left alone.
                if !link_namespaces.contains(namespace) {
                    continue;
                }
                if !crate::domain::item::is_qualified_reference(value) {
                    continue;
                }
                // Resolve through the single generic resolver; an `Err` means the
                // qualified id is dangling and is reported as a finding.
                if self.resolve_link_label(label).is_err() {
                    findings.push(GraphFinding::for_issue(
                        issue.id.clone(),
                        Finding {
                            rule: DANGLING_LINK_RULE.to_string(),
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

    /// Built-in validate pass: report enforcement drift between the invariant
    /// registry and the declared rules/gates (REQ-01/REQ-02).
    ///
    /// This runs as part of every validate path — NOT behind an opt-in
    /// `.jit/rules.toml` rule — mirroring
    /// [`dangling_link_findings`](Self::dangling_link_findings). It is gated only
    /// on the presence of declared invariants: when `.jit/invariants.toml` is
    /// absent or empty the pass returns no findings (graceful degradation), so a
    /// repository that declares no invariants — including the live repo — is
    /// totally unaffected and `jit validate` surfaces nothing new.
    ///
    /// When invariants ARE declared, drift is reported as unattributed
    /// [`GraphFinding`]s (drift pertains to the project's declarations, not a
    /// single issue) via
    /// [`compute_drift_findings`](crate::commands::CommandExecutor::compute_drift_findings),
    /// the SAME tolerant drift computation shared with
    /// [`check_invariants`](crate::commands::CommandExecutor::check_invariants). The
    /// sole direction is **declared-but-unenforced** — an invariant whose
    /// `enforced-by` names a missing/unloadable rule or gate — emitted at
    /// [`Severity::Error`](crate::declarations::rules::Severity::Error), so it FAILS
    /// `jit validate` (the broken binding is a real defect). An unclaimed rule or
    /// gate is NOT drift (the enforced-but-undeclared direction was removed in
    /// REQ-05).
    ///
    /// The rule name on every finding is [`ENFORCEMENT_DRIFT_RULE`]. A `rules.toml`
    /// (or gate registry) load failure is tolerated, NOT propagated as an `Err`:
    /// [`compute_drift_findings`](crate::commands::CommandExecutor::compute_drift_findings)
    /// resolves it defensively to
    /// [`SourceState::Unloadable`](crate::validation::drift::SourceState::Unloadable),
    /// so it surfaces as a declared-but-unenforced finding rather than aborting
    /// validation.
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
                    rule: ENFORCEMENT_DRIFT_RULE.to_string(),
                    // A broken binding (declared-but-unenforced) is a real defect
                    // -> fails validate. It is the only drift direction (REQ-05).
                    severity: Severity::Error,
                    message: f.message(),
                })
            })
            .collect();
        Ok(findings)
    }

    /// Compute the enforcement-drift findings, tolerating an unloadable rule set /
    /// gate registry (REQ-01 "missing OR unloadable").
    ///
    /// This is the SINGLE drift computation shared by the built-in validate pass
    /// ([`enforcement_drift_findings`](Self::enforcement_drift_findings), which
    /// assigns per-direction severity) and the
    /// [`check_invariants`](crate::commands::CommandExecutor::check_invariants)
    /// command (which serializes the raw findings), so both surfaces report
    /// IDENTICALLY. Returns an empty list when no invariants are declared (the
    /// pass is dormant — a repo without `.jit/invariants.toml` is unaffected).
    ///
    /// Each enforcement SOURCE is loaded defensively: a parse/load failure is NOT
    /// propagated (it would crash the run) but treated as
    /// [`SourceState::Unloadable`](crate::validation::drift::SourceState::Unloadable),
    /// so a binding into it surfaces as a declared-but-unenforced finding flagged
    /// `unloadable` instead of an error. Only `cached_config` (the invariant
    /// registry itself) can still error.
    pub fn compute_drift_findings(&self) -> Result<Vec<crate::validation::drift::DriftFinding>> {
        use crate::validation::drift::{enforcement_drift_tolerant, SourceState};
        use std::collections::BTreeSet;

        let config = self.cached_config()?;
        let invariants = &config.invariants.invariants;
        // Gated on declared invariants: a repo with none sees no drift findings.
        if invariants.is_empty() {
            return Ok(Vec::new());
        }

        // Defensive (tolerant) loads: a failure -> unloadable, not an error.
        let rule_names_owned = self.loadable_rule_names();
        let gate_keys_owned: Option<Vec<String>> = self
            .storage
            .load_gate_registry()
            .ok()
            .map(|reg| reg.gates.keys().cloned().collect());

        let rule_set: Option<BTreeSet<&str>> = rule_names_owned
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());
        let gate_set: Option<BTreeSet<&str>> = gate_keys_owned
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());

        let rules_state = match &rule_set {
            Some(set) => SourceState::Loaded(set),
            None => SourceState::Unloadable,
        };
        let gates_state = match &gate_set {
            Some(set) => SourceState::Loaded(set),
            None => SourceState::Unloadable,
        };

        Ok(enforcement_drift_tolerant(
            invariants,
            rules_state,
            gates_state,
        ))
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
        let rules_path = self.storage.root().join("rules.toml");
        if rules_path.exists() {
            // Present: parse it directly (do NOT go through the cached
            // `effective_rules`, which `?`-errors). A parse failure -> unloadable.
            self.config_manager
                .load()
                .ok()
                .and_then(|config| {
                    crate::storage::ruleset_store::load_ruleset(self.storage.root(), &config).ok()
                })
                .map(|set| set.rules.into_iter().map(|r| r.name).collect())
        } else {
            // Absent: the in-memory defaults are loadable. A namespace-registry
            // failure (rare) reads as unloadable rather than crashing.
            self.cached_namespaces().ok().map(|namespaces| {
                crate::repository_state::default_ruleset(namespaces)
                    .rules
                    .into_iter()
                    .map(|r| r.name)
                    .collect()
            })
        }
    }

    /// Run the repository-integrity checks ONLY, without evaluating declarative
    /// graph rules.
    ///
    /// This is the structural half of [`CommandExecutor::validate_silent`]:
    /// broken dependency references, invalid gate references, document
    /// references, DAG acyclicity, isolated nodes, transitive reduction, and
    /// claims-index consistency. Label validity and type-hierarchy checks are NO
    /// LONGER here — they were migrated to default rules and are evaluated by the
    /// local/graph rule engine in `validate_silent` (see the NOTE in that
    /// method). It deliberately
    /// excludes the `RuleScope::Graph` declarative rules so a caller can render those
    /// as structured findings (e.g. whole-repo `jit validate --json`) and decide
    /// the exit status AFTER output, rather than aborting before the rule report
    /// is built.
    ///
    /// Returns `Ok(())` when the repository is structurally sound, or an `Err`
    /// describing the first integrity violation found.
    pub fn validate_integrity_silent(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;

        // Build lookup map of valid issue IDs
        let valid_ids: std::collections::HashSet<String> =
            issues.iter().map(|i| i.id.clone()).collect();

        // Check for broken dependency references
        for issue in &issues {
            for dep in &issue.dependencies {
                if !valid_ids.contains(dep) {
                    return Err(anyhow!(
                        "Invalid dependency: issue '{}' depends on '{}' which does not exist",
                        issue.id,
                        dep
                    ));
                }
            }
        }

        // Check for invalid gate references
        let registry = self.storage.load_gate_registry()?;
        for issue in &issues {
            for gate_key in &issue.gates_required {
                if !registry.gates.contains_key(gate_key) {
                    return Err(anyhow!(
                        "Gate '{}' required by issue '{}' is not defined in registry",
                        gate_key,
                        issue.id
                    ));
                }
            }
        }

        // NOTE: label format, namespace registry, namespace value/pattern/
        // unique/required, type-label requirement, and unknown-type detection are
        // NO LONGER checked here. They are now default rules (see
        // `repository_state::default_rules`) evaluated by `validate_silent` via
        // `local_rules_error_message` (a0f0f342 migration).

        // Validate document references (git integration)
        self.validate_document_references(&issues)?;

        // Validate DAG (no cycles)
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        graph.validate_dag()?;

        // Validate no isolated nodes (nodes outside the main DAG)
        // Exception: A single issue in the repository is not considered isolated
        if issues.len() > 1 {
            let isolated = graph.get_isolated_nodes();
            if !isolated.is_empty() {
                let isolated_ids: Vec<String> = isolated
                    .iter()
                    .map(|i| format!("'{}' ({})", &i.id[..8.min(i.id.len())], i.title))
                    .collect();
                return Err(anyhow!(
                    "Found {} isolated issue(s) not connected to the dependency graph:\n  {}\n\
                     Isolated issues have no dependencies and are not dependencies of any other issue.\n\
                     Either add dependencies with 'jit dep add' or delete these issues.",
                    isolated.len(),
                    isolated_ids.join("\n  ")
                ));
            }
        }

        // Validate transitive reduction (no redundant dependencies)
        self.validate_transitive_reduction(&graph, &issues)?;

        // Validate claims index (if worktree mode is active and not in test mode)
        if std::env::var("JIT_TEST_MODE").is_err() {
            let index_issues = validate_claims_index()
                .unwrap_or_else(|e| vec![format!("Failed to validate claims index: {}", e)]);
            if !index_issues.is_empty() {
                return Err(anyhow!(
                    "Claims index validation failed:\n  {}",
                    index_issues.join("\n  ")
                ));
            }
        }

        Ok(())
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
        let ruleset = self.effective_rules()?;

        let graph_rules: Vec<&crate::declarations::rules::Rule> = ruleset
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph)
            .collect();

        // The repo HierarchyConfig is injected into `type-hierarchy` rules at
        // evaluation time (D1); it is no longer stored in the parsed rule. Build
        // it from the same namespace registry the default rules derive from.
        let namespaces = self.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
        let hierarchy = crate::repository_state::hierarchy_config(namespaces);
        let repo_format = self.repo_content_format()?;

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

    /// Plan-document content for `emit_for`, image-projected when a captured
    /// session is available (the closed-read replacement for the ambient
    /// the plan-content projection).
    ///
    /// This captures the whole-repository validation image
    /// ([`capture_validation_image`](Self::capture_validation_image), whose
    /// closure enqueues every derived plan-document path) and projects the
    /// plan-document content map through [`plan_content_from_image`]. Planning-node
    /// resolution runs against the full captured issue set, so a scoped slice still
    /// resolves a bracket's planning node, and the plan bytes are read from the
    /// closed image rather than the live filesystem.
    ///
    pub(crate) fn image_plan_content(
        &self,
        emit_for: &[Issue],
    ) -> Result<std::collections::HashMap<String, String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let image = self.capture_validation_image()?;
        plan_content_from_image(&image, emit_for)
    }

    /// Run the declarative rule set (`.jit/rules.toml`) as a per-issue or
    /// whole-repo report.
    ///
    /// When `id` is `Some`, the issue is resolved (partial ids accepted) and its
    /// matching local rules are evaluated, then graph rules are evaluated across
    /// the whole store and filtered to those that pertain to this issue using
    /// structured attribution (each graph finding carries its issue id), plus any
    /// `config-error` finding for a graph rule whose selector applies to this
    /// issue — so a malformed graph rule is reported, never silently passed. When
    /// `id` is `None`, local rules run for every issue and graph rules run across
    /// the store, with every graph finding reported. The result is a pure
    /// [`RuleReport`](crate::validation::report::RuleReport); rendering and exit
    /// codes are the caller's concern.
    ///
    /// # Errors
    ///
    /// Returns an error if `.jit/rules.toml` is malformed, the issue id cannot be
    /// resolved, or a matching local rule's schema fails to compile (a
    /// misconfigured rule never silently disables enforcement).
    pub fn run_rules(&self, id: Option<&str>) -> Result<crate::validation::report::RuleReport>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::validation::report::{ReportedFinding, RuleReport};

        // If the rule SOURCE is unloadable, do NOT crash the whole report: the
        // enforcement-drift pass still reports a declared-but-unenforced finding
        // for a binding into the unloadable source (REQ-01), and the parse problem
        // itself is surfaced as a config-error finding so validation still fails.
        // This mirrors how a SEMANTIC config-error (a rule that parses but is
        // rejected by its evaluator) is already reported as a finding rather than
        // an early `?`.
        let ruleset = match self.effective_rules() {
            Ok(set) => set,
            Err(e) => {
                let mut findings: Vec<ReportedFinding> = Vec::new();
                // The drift pass loads the ruleset tolerantly on its own.
                for gf in self.enforcement_drift_findings()? {
                    findings.push(ReportedFinding::new(gf.issue_id.clone(), &gf.finding));
                }
                if id.is_none() {
                    findings.extend(self.review_placeholder_findings()?);
                }
                // Surface the unparseable ruleset as an error-severity finding so
                // the report still fails (config-error prefix keeps it grouped).
                findings.push(ReportedFinding::new(
                    None,
                    &crate::validation::engine::Finding {
                        rule: "rules-file".to_string(),
                        severity: crate::declarations::rules::Severity::Error,
                        message: format!("config error: {e}"),
                    },
                ));
                return Ok(RuleReport { findings });
            }
        };
        let repo_format = self.repo_content_format()?;
        let issues = self.storage.list_issues()?;

        let mut findings: Vec<ReportedFinding> = Vec::new();

        match id {
            Some(partial) => {
                // `load_issue` keys on the FULL id, so the caller's id — which may
                // be a short-id prefix, as every other command accepts — is put
                // through the store's prefix resolver first.
                let full_id = self.storage.resolve_issue_id(partial)?;
                let issue = self.storage.load_issue(&full_id)?;

                // Local rules for this issue only.
                let evaluation = crate::validation::evaluate_local(&issue, ruleset, repo_format)
                    .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
                findings.extend(
                    evaluation
                        .findings()
                        .into_iter()
                        .map(|f| ReportedFinding::new(Some(issue.id.clone()), f)),
                );

                // Graph rules across the store. Keep, by EXACT structural
                // attribution (not substring matching): findings attributed to
                // this issue, plus any config-error for a graph rule whose
                // selector applies to this issue (a malformed graph rule must be
                // reported here, never silently dropped).
                let applicable_rules: std::collections::HashSet<String> = ruleset
                    .matching_rules(&issue)
                    .into_iter()
                    .map(|r| r.name.clone())
                    .collect();
                let mut graph_findings = self.evaluate_graph_rules(&issues)?;
                // Built-in dangling-item-link pass (REQ-08 clause i, REQ-03):
                // attributed to issues, so per-issue filtering below keeps only
                // this issue's dangling links.
                graph_findings.extend(self.dangling_link_findings(&issues)?);
                // The enforcement-drift pass is intentionally NOT folded into the
                // single-issue report: drift is project-scoped (unattributed), not
                // a property of one issue. It surfaces in the whole-repo report
                // (the `None` arm below) and gates `validate_silent` / cargo-ci.
                findings.extend(graph_findings.iter().filter_map(|gf| {
                    let pertains = gf.issue_id.as_deref() == Some(issue.id.as_str())
                        || (gf.is_config_error() && applicable_rules.contains(&gf.finding.rule));
                    pertains.then(|| ReportedFinding::new(Some(issue.id.clone()), &gf.finding))
                }));
            }
            None => {
                // Local rules for every issue.
                for issue in &issues {
                    let evaluation = crate::validation::evaluate_local(issue, ruleset, repo_format)
                        .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
                    findings.extend(
                        evaluation
                            .findings()
                            .into_iter()
                            .map(|f| ReportedFinding::new(Some(issue.id.clone()), f)),
                    );
                }

                // Graph rules across the store; every finding is reported,
                // carrying its structured issue attribution (None for config
                // errors).
                let mut graph_findings = self.evaluate_graph_rules(&issues)?;
                // Built-in dangling-item-link pass (REQ-08 clause i, REQ-03):
                // every dangling link surfaces in the whole-repo report.
                graph_findings.extend(self.dangling_link_findings(&issues)?);
                // Built-in enforcement-drift pass (REQ-01/REQ-02): project-scoped
                // (unattributed) drift surfaces in the whole-repo report. Gated on
                // declared invariants, so a repo without any is unaffected.
                graph_findings.extend(self.enforcement_drift_findings()?);
                findings.extend(
                    graph_findings
                        .iter()
                        .map(|gf| ReportedFinding::new(gf.issue_id.clone(), &gf.finding)),
                );
                findings.extend(self.review_placeholder_findings()?);
            }
        }

        Ok(RuleReport { findings })
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

        let registry = self.storage.load_gate_registry()?;
        let mut keys: Vec<&str> = registry
            .gates
            .iter()
            .filter_map(|(key, gate)| {
                matches!(gate.checker, Some(GateChecker::ReviewPlaceholder)).then_some(key.as_str())
            })
            .collect();
        keys.sort_unstable();

        Ok(if keys.is_empty() {
            Vec::new()
        } else {
            vec![ReportedFinding::new(
                None,
                &Finding {
                    rule: REVIEW_PLACEHOLDER_RULE.to_string(),
                    severity: Severity::Warn,
                    message: format!(
                        "WARNING: passing external-review placeholder still configured for gate(s): {}. Replace each placeholder with a real review checker before relying on these gates.",
                        keys.join(", ")
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
    /// Returns an error if `.jit/rules.toml` is malformed, the container id cannot
    /// be resolved, or a matching local rule's schema fails to compile.
    pub fn validate_scope(
        &self,
        container_id: &str,
    ) -> Result<crate::validation::report::RuleReport>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::rules::{RuleScope, Severity};
        use crate::validation::report::{ReportedFinding, RuleReport};

        let ruleset = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;

        // Resolve the container (partial ids accepted) and build the slice.
        let container_full_id = self.storage.resolve_issue_id(container_id)?;
        let all = self.storage.list_issues()?;

        // The breakdown boundary type is template-driven: resolve the container's
        // `type:` label, look up its graph template, and read that template's
        // breakdown-node type. No applicable template (a non-bracketed container)
        // → no boundary, so the scope walk is the full dependency closure (see
        // `bracket_scope_ids`).
        let container_type = all
            .iter()
            .find(|i| i.id == container_full_id)
            .and_then(|c| label_utils::type_label_value(&c.labels).map(str::to_string));
        let templates = &self.cached_config()?.templates;
        let breakdown_type = container_type
            .as_deref()
            .and_then(|ty| templates.template_for_container(ty))
            .and_then(|t| t.breakdown_type(&templates.roles))
            .map(str::to_string);

        let scope_ids = crate::domain::queries::bracket_scope_ids(
            &container_full_id,
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
            let namespaces = self.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
            let hierarchy = crate::repository_state::hierarchy_config(namespaces);
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

    /// Build the `--explain` report for one issue: EVERY rule in the ruleset,
    /// paired with whether its selector matched the issue and, for matched rules,
    /// whether they passed and their messages.
    ///
    /// A rule whose selector excludes the issue is reported as skipped: its
    /// [`RuleOutcome`](crate::validation::report::RuleOutcome) carries `matched =
    /// false` and a `skip_reason` naming the excluding selector dimension(s) (the
    /// state dimension is called out explicitly, e.g. "state predicate did not
    /// match (issue is 'in_progress', wants 'done')"). Local rules that match are
    /// evaluated against the issue; matching graph rules are evaluated across the
    /// whole store and attributed to this issue by EXACT structural attribution
    /// (each finding carries its issue id), and any `config-error` finding for an
    /// applicable graph rule is also surfaced — so a malformed graph rule is
    /// reported as a FAILED outcome, never shown as passing. Widening the report
    /// to include non-matching rules does not change which rules EXECUTE: only
    /// matched rules are evaluated.
    ///
    /// # Errors
    ///
    /// Returns an error if `.jit/rules.toml` is malformed, the issue id cannot be
    /// resolved, or a matching local rule's schema fails to compile.
    pub fn explain_rules(&self, id: &str) -> Result<crate::validation::report::ExplainReport>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::rules::RuleScope;
        use crate::validation::report::{ExplainReport, RuleOutcome};

        let ruleset = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        // `load_issue` keys on the FULL id; resolve a short-id prefix first.
        let full_id = self.storage.resolve_issue_id(id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let issues = self.storage.list_issues()?;

        // Local findings for this issue, grouped by rule name.
        let local_eval = crate::validation::evaluate_local(&issue, ruleset, repo_format)
            .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
        let local_messages = group_messages(
            local_eval
                .findings()
                .into_iter()
                .map(|f| (f.rule.clone(), f.message.clone())),
        );

        // Graph findings pertaining to this issue, grouped by rule name, using
        // EXACT structural attribution: a finding attributed to this issue, or a
        // config-error (which carries no issue id but must be surfaced so a
        // malformed graph rule fails rather than silently passes). The
        // selector-based "applies to this issue" decision is made per-rule below;
        // including all config-errors here is safe because these messages are
        // consumed only by the matched arm (non-matching rules are reported as
        // skipped with no messages).
        let graph_findings = self.evaluate_graph_rules(&issues)?;
        let graph_messages = group_messages(graph_findings.iter().filter_map(|gf| {
            let pertains =
                gf.issue_id.as_deref() == Some(issue.id.as_str()) || gf.is_config_error();
            pertains.then(|| (gf.finding.rule.clone(), gf.finding.message.clone()))
        }));

        // EVERY rule in the ruleset becomes one outcome, not just the matching
        // ones: a rule excluded by its selector is reported with `matched =
        // false` and a `skip_reason` naming the dimension(s) that excluded the
        // issue, so `--explain` can show "the state predicate did not match".
        // Matching rules keep their PASS/FAIL semantics. This widens only the
        // REPORT; which rules EXECUTE is still decided by `matching_rules`
        // (here, the per-rule `when.matches` check selecting the matched arm).
        let outcomes: Vec<RuleOutcome> = ruleset
            .rules
            .iter()
            .map(|rule| {
                match rule.when.match_failure(&issue) {
                    // Selector excluded the issue: report it as skipped.
                    Some(skip_reason) => RuleOutcome {
                        rule: rule.name.clone(),
                        scope: rule.scope,
                        severity: rule.severity,
                        selector: render_selector(&rule.when),
                        matched: false,
                        skip_reason: Some(skip_reason),
                        passed: true,
                        messages: Vec::new(),
                    },
                    // Selector matched: PASS/FAIL from the evaluated findings.
                    None => {
                        let messages = match rule.scope {
                            RuleScope::Local => {
                                local_messages.get(&rule.name).cloned().unwrap_or_default()
                            }
                            RuleScope::Graph => {
                                graph_messages.get(&rule.name).cloned().unwrap_or_default()
                            }
                        };
                        RuleOutcome {
                            rule: rule.name.clone(),
                            scope: rule.scope,
                            severity: rule.severity,
                            selector: render_selector(&rule.when),
                            matched: true,
                            skip_reason: None,
                            passed: messages.is_empty(),
                            messages,
                        }
                    }
                }
            })
            .collect();

        Ok(ExplainReport {
            issue_id: issue.id,
            outcomes,
        })
    }

    fn validate_document_references(&self, issues: &[Issue]) -> Result<()> {
        use git2::Repository;

        // Try to open git repository
        let layout = self.require_layout()?;
        let repo = match Repository::open(layout.worktree_root()) {
            Ok(r) => r,
            Err(_) => {
                // If not a git repo, skip document validation
                return Ok(());
            }
        };

        // Check if repository has any commits (HEAD exists)
        let has_commits = repo.head().is_ok();

        for issue in issues {
            for doc in &issue.documents {
                // Validate commit hash if specified
                if let Some(ref commit_hash) = doc.commit {
                    // Use revparse to resolve short hashes
                    let resolved_oid = repo
                        .revparse_single(commit_hash)
                        .and_then(|obj| obj.peel_to_commit())
                        .map(|commit| commit.id());

                    if resolved_oid.is_err() {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': commit '{}' not found for '{}'",
                            issue.id,
                            commit_hash,
                            doc.path
                        ));
                    }

                    // Validate file exists at the specified commit
                    if self
                        .check_file_exists_in_git(&repo, &doc.path, commit_hash)
                        .is_err()
                    {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': file '{}' not found at commit {}",
                            issue.id,
                            doc.path,
                            commit_hash
                        ));
                    }
                } else {
                    // No commit specified - check working tree or HEAD
                    if has_commits {
                        // Repository has commits - validate against HEAD, falling back to
                        // working tree so that newly added (not yet committed) files pass.
                        let in_git = self
                            .check_file_exists_in_git(&repo, &doc.path, "HEAD")
                            .is_ok();
                        let in_working_tree = std::path::Path::new(&doc.path).exists();
                        if !in_git && !in_working_tree {
                            return Err(anyhow!(
                                "Invalid document reference in issue '{}': file '{}' not found at HEAD or in working tree",
                                issue.id,
                                doc.path
                            ));
                        }
                    } else {
                        // Repository has no commits - check working tree only
                        let path = std::path::Path::new(&doc.path);
                        if !path.exists() {
                            return Err(anyhow!(
                                "Invalid document reference in issue '{}': file '{}' not found in working tree (repository has no commits yet)",
                                issue.id,
                                doc.path
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn check_file_exists_in_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<()> {
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;

        // Try to find the file in the tree
        tree.get_path(std::path::Path::new(path))?;

        Ok(())
    }

    pub fn get_status(&self) -> Result<StatusSummary> {
        let issues = self.storage.list_issues()?;
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
            open: backlog, // Keep 'open' field name for backward compatibility
            ready,
            in_progress,
            done,
            rejected,
            blocked,
            gated,
            total: issues.len(),
        })
    }

    /// Validate transitive reduction of dependency graph.
    ///
    /// Checks that no issue has redundant dependencies - dependencies that are
    /// already reachable through other dependency paths.
    fn validate_transitive_reduction(
        &self,
        graph: &DependencyGraph<Issue>,
        issues: &[Issue],
    ) -> Result<()> {
        use std::collections::HashSet;

        for issue in issues {
            if issue.dependencies.is_empty() {
                continue;
            }

            // Compute minimal dependency set
            let reduced = graph.compute_transitive_reduction(&issue.id);
            let reduced_set: HashSet<_> = reduced.iter().collect();

            // Find redundant edges (in current but not in reduction)
            for dep_id in &issue.dependencies {
                if !reduced_set.contains(dep_id) {
                    // This edge is redundant - find the transitive path
                    let path = graph.find_shortest_path(&issue.id, dep_id);
                    let path_str = if path.is_empty() {
                        "unknown path".to_string()
                    } else {
                        path.iter()
                            .map(|id| &id[..8.min(id.len())])
                            .collect::<Vec<_>>()
                            .join(" → ")
                    };

                    return Err(anyhow!(
                        "Transitive reduction violation: Issue {} has redundant dependency on {} \
                         (already reachable via: {}). Run 'jit validate --fix' to remove redundant edges.",
                        issue.short_id(),
                        dep_id.chars().take(SHORT_ID_LENGTH).collect::<String>(),
                        path_str
                    ));
                }
            }
        }

        Ok(())
    }

    /// Fix transitive reduction violations for all issues.
    ///
    /// Returns count of redundant edges fixed (or that would be fixed if dry_run).
    fn fix_all_transitive_reductions(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.reduce_all_dependencies(dry_run)
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
    fn check_pending_transitions(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)>
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
            let issues = self.storage.list_issues()?;
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
                        ReadinessCorrection::Promote => self.auto_transition_to_ready(issue_id)?,
                        ReadinessCorrection::Demote => self.auto_transition_to_backlog(issue_id)?,
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

        // Get merge-base between HEAD and origin/main
        let merge_base_output = Command::new("git")
            .args(["merge-base", "HEAD", "origin/main"])
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
    /// Checks claims.index.json for:
    /// - Expired leases (TTL exceeded)
    /// - Leases referencing non-existent worktrees
    /// - Leases referencing non-existent issues
    ///
    /// # Returns
    /// Vector of invalid lease descriptions with fix suggestions
    pub fn validate_leases(&self) -> Result<Vec<String>> {
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;
        use chrono::Utc;

        let paths = WorktreePaths::detect().context("Failed to detect worktree paths")?;

        // Load the active-lease index through storage (an absent index yields an
        // empty one, so there are no leases to validate).
        let index = ClaimsIndex::load(&paths)?;

        let mut invalid_leases = Vec::new();
        let now = Utc::now();

        for lease in &index.leases {
            // Check if lease has expired
            if let Some(expires_at) = lease.expires_at {
                if expires_at < now {
                    let duration = now.signed_duration_since(expires_at);
                    invalid_leases.push(format!(
                        "Lease {} (Issue {}): Expired {} ago\n  Fix: jit claim release {}",
                        lease.lease_id,
                        &lease.issue_id[..8.min(lease.issue_id.len())],
                        format_duration(duration),
                        &lease.issue_id[..8.min(lease.issue_id.len())]
                    ));
                    continue;
                }
            }

            // Check if worktree still exists
            if !check_worktree_exists(&lease.worktree_id)? {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Worktree {} no longer exists\n  Fix: jit claim force-evict {}",
                    lease.lease_id,
                    &lease.issue_id[..8.min(lease.issue_id.len())],
                    lease.worktree_id,
                    lease.lease_id
                ));
                continue;
            }

            // Check if issue still exists (use storage layer for proper resolution)
            if self.storage.load_issue(&lease.issue_id).is_err() {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Issue no longer exists\n  Fix: jit claim release {}",
                    lease.lease_id,
                    &lease.issue_id[..8.min(lease.issue_id.len())],
                    &lease.issue_id[..8.min(lease.issue_id.len())]
                ));
            }
        }

        Ok(invalid_leases)
    }
}

/// Find `container`'s bracket planning node `P`, if one has been applied.
///
/// A breakable container is bracketed by a breakdown node `B` carrying the
/// container-specific label declared by the template's breakdown node and
/// depending on a planning node `P` of the template's planning type. This
/// walks that relationship — the declared container label on `B`, then `B`'s
/// planning-typed dependency — and returns `P`, or `None` when no bracket has
/// been applied (no matching `B`, or its planning dependency is absent). The
/// lookup is over the WHOLE store (`by_id`) so it stays consistent when the
/// caller's slice bounds out the bracket infrastructure.
///
/// Domain-agnostic: the planning type and container label are read from the
/// container's template (selected by the repository's role bindings), not
/// hardcoded.
pub(crate) fn find_planning_node<'a>(
    container: &Issue,
    template: &crate::templates::GraphTemplate,
    roles: &crate::templates::RoleBindings,
    by_id: &std::collections::HashMap<&str, &'a Issue>,
) -> Option<&'a Issue> {
    let planning_type = template.planning_type(roles)?;
    let bracket_label =
        crate::commands::template_expand::declared_container_label(template, roles, container)
            .ok()?;
    let planning_type_label = label_utils::type_label(planning_type);

    // The breakdown node carries the template-declared container label and
    // depends on the planning node; find it, then return that dependency.
    by_id
        .values()
        .filter(|b| b.labels.contains(&bracket_label))
        .find_map(|b| {
            b.dependencies.iter().find_map(|dep_id| {
                by_id
                    .get(dep_id.as_str())
                    .copied()
                    .filter(|p| p.labels.contains(&planning_type_label))
            })
        })
}

/// The document-reference label that marks a planning node's plan document.
///
/// A planning node records WHERE its container's plan lives as a
/// [`DocumentReference`](crate::domain::DocumentReference) carrying this label.
/// That reference is the validation-time source of truth for the plan-doc
/// location: `jit validate` reads the plan from this reference's `path`, so a
/// plan that is moved/archived and re-linked keeps validating from its new
/// location. The graph template's
/// [`plan_doc_location`](crate::templates::GraphTemplate::plan_doc_location)
/// resolves into the planning node's description as an instruction naming where
/// to author the plan; applying a template attaches no document reference, and
/// this one is created when the plan is authored and linked.
const PLAN_DOC_LABEL: &str = "plan";

/// The repo-root-relative path of `planning`'s recorded plan document, if it has
/// one.
///
/// Reads the planning node's [`PLAN_DOC_LABEL`]-labeled
/// [`DocumentReference`](crate::domain::DocumentReference) — the validation-time
/// source of truth for the plan-doc location. Returns `None` when the node
/// records no such reference (the plan is inline, in the container's body).
pub(crate) fn planning_node_plan_path(planning: &Issue) -> Option<String> {
    planning
        .documents
        .iter()
        .find(|d| d.label.as_deref() == Some(PLAN_DOC_LABEL))
        .map(|d| d.path.clone())
}

/// Group `(rule_name, message)` pairs into a map of rule name to its messages,
/// preserving first-seen order within each rule.
fn group_messages(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> std::collections::HashMap<String, Vec<String>> {
    pairs.into_iter().fold(
        std::collections::HashMap::new(),
        |mut acc, (rule, message)| {
            acc.entry(rule).or_default().push(message);
            acc
        },
    )
}

/// Render a rule selector as a compact, human-readable string for `--explain`.
///
/// An empty selector (matches everything) renders as `"*"`; otherwise the
/// present dimensions are joined with `", "` (e.g. `"type=epic, state=ready"`).
fn render_selector(selector: &crate::declarations::rules::Selector) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &selector.type_ {
        parts.push(format!("type={t}"));
    }
    if let Some(l) = &selector.label {
        parts.push(format!("label={l}"));
    }
    if let Some(s) = &selector.state {
        parts.push(format!("state={}", s.tokens().join("|")));
    }
    if let Some(d) = &selector.has_doc_type {
        parts.push(format!("has_doc_type={d}"));
    }
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join(", ")
    }
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

/// Check if a worktree with the given ID still exists
fn check_worktree_exists(worktree_id: &str) -> Result<bool> {
    use std::path::PathBuf;
    use std::process::Command;

    // Get all git worktrees
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to execute git worktree list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree list failed: {}", stderr);
    }

    let porcelain_output =
        String::from_utf8(output.stdout).context("Invalid UTF-8 in git worktree output")?;

    // Parse worktree paths
    let worktree_paths = porcelain_output
        .lines()
        .filter(|line| line.starts_with("worktree "))
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    // Check each worktree for matching ID. A missing or unreadable identity
    // file is tolerated (skip it), matching the prior inline behavior.
    for worktree_path in worktree_paths {
        if let Some(id) =
            crate::storage::worktree_identity::read_worktree_id(&worktree_path).unwrap_or(None)
        {
            if id == worktree_id {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Validate claims index consistency
///
/// Checks for structural corruption:
/// - Duplicate leases for the same issue (invariant violation)
/// - Schema version mismatches (incompatibility)
/// - Sequence gaps (data loss indicator)
///
/// Note: Does NOT check for expired leases - those are normal state handled by
/// evict_expired(). Use validate_leases() for expiration checks.
///
/// Returns vector of corruption issues found (empty if structurally valid)
pub fn validate_claims_index() -> Result<Vec<String>> {
    use crate::storage::worktree_paths::WorktreePaths;

    let paths = WorktreePaths::detect().context("Failed to detect worktree paths")?;
    validate_claims_index_with_paths(&paths)
}

/// Validate claims-index consistency for an explicitly selected repository.
pub(crate) fn validate_claims_index_at(repository_root: &std::path::Path) -> Result<Vec<String>> {
    use crate::storage::worktree_paths::WorktreePaths;

    let paths =
        WorktreePaths::detect_from(repository_root).context("Failed to detect worktree paths")?;
    validate_claims_index_with_paths(&paths)
}

fn validate_claims_index_with_paths(
    paths: &crate::storage::worktree_paths::WorktreePaths,
) -> Result<Vec<String>> {
    use crate::storage::claim_coordinator::ClaimsIndex;
    use std::collections::HashSet;

    // Load the active-lease index through storage (an absent index yields an
    // empty one, i.e. no claims coordination active).
    let index = ClaimsIndex::load(paths)?;

    let mut issues = Vec::new();

    // Check schema version
    if index.schema_version != 1 {
        issues.push(format!(
            "Invalid schema version: expected 1, found {}",
            index.schema_version
        ));
    }

    // Check for duplicate leases (same issue claimed twice)
    let mut seen_issues = HashSet::new();
    for lease in &index.leases {
        if !seen_issues.insert(&lease.issue_id) {
            issues.push(format!(
                "Duplicate lease detected for issue {}: Multiple leases exist for the same issue",
                &lease.issue_id[..8.min(lease.issue_id.len())]
            ));
        }
    }

    // Note: Expired leases are NOT considered corruption - they are normal state
    // evicted by `jit recover`. Use validate_leases() to check expiration.

    // Report sequence gaps (if any were detected during rebuild)
    if !index.sequence_gaps.is_empty() {
        issues.push(format!(
            "Sequence gaps detected in claims log: missing sequences {:?}",
            index.sequence_gaps
        ));
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// An in-memory repository seeded from a canonical file-backed `jit init`,
    /// applying `profile` when one is named.
    fn memory_fixture(profile: Option<&str>) -> crate::storage::InMemoryStorage {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        use crate::test_taxonomy::test_taxonomy;

        const PROFILE_TABLES: &str = r#"
[item_kinds.invariant]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = ["enforces"]
scope = "project"
source = { toml = ".jit/invariants.toml", table = "invariants", id-field = "id", text-field = "statement" }
source-of-truth = "registry-first"
aliases = ["inv"]

[item_kinds.rule]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = ["enforces"]
scope = "project"
source = { toml = ".jit/rules.toml", table = "rules", id-field = "name", text-field = "description" }
source-of-truth = "registry-first"

[item_kinds.gate]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = ["enforces"]
scope = "project"
source = { toml = ".jit/gates.toml", table = "gates", id-field = "key", text-field = "description" }
source-of-truth = "registry-first"

[projection.invariants]
kind = "invariant"
mode = "region"
target = "AGENTS.md"
style = "id-anchor"

[projection.rules-and-gates]
kind = ["rule", "gate"]
mode = "separate-file"
target = ".jit/reference/rules-and-gates.md"
style = "full"
"#;

        fn seed_tree(
            storage: &crate::storage::InMemoryStorage,
            root: &std::path::Path,
            current: &std::path::Path,
        ) {
            for entry in std::fs::read_dir(current).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    seed_tree(storage, root, &path);
                } else {
                    seed_repo_file(
                        storage,
                        path.strip_prefix(root).unwrap().to_str().unwrap(),
                        &std::fs::read_to_string(&path).unwrap(),
                    );
                }
            }
        }

        let taxonomy = test_taxonomy();
        let source = tempfile::tempdir().unwrap();
        let source_storage = JsonFileStorage::new(source.path().join(".jit"));
        std::fs::create_dir_all(source_storage.root()).unwrap();
        let config = if profile.is_some() {
            format!("{}\n{PROFILE_TABLES}", taxonomy.config_fragment())
        } else {
            taxonomy.config_fragment()
        };
        std::fs::write(source_storage.root().join("config.toml"), config).unwrap();
        let source_layout =
            crate::storage::discover_repository_layout(source.path(), source_storage.root())
                .unwrap();
        CommandExecutor::new(source_storage)
            .with_layout(source_layout)
            .initialize_fresh_repository(source.path(), &taxonomy.hierarchy_template(), profile)
            .unwrap();

        let storage = crate::storage::InMemoryStorage::new();
        seed_tree(&storage, source.path(), source.path());
        // The text-only fixture seeder does not retain executable bits. Exercise
        // the same repair path once to restore those modes before returning.
        memory_executor(storage.clone())
            .validate_with_fix(true, false)
            .unwrap();
        storage
    }

    // Note: validate_leases() and validate_branch_drift() require git repository setup
    // and are integration-tested through manual testing and real usage.
    // Unit tests focus on pure functions like format_duration().

    #[test]
    fn test_find_planning_node_resolves_template_declared_container_label() {
        let template_toml = r#"
[[template]]
name = "custom-plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
labels = ["scaffold:{container.short_id}"]
depends_on = ["planning"]
"#;
        let registry =
            crate::templates::TemplateRegistry::from_toml_str(template_toml, &[] as &[&str])
                .unwrap();
        let template = registry.get("custom-plan").unwrap();

        let mut container =
            crate::domain::types::fixture_issue("container".to_string(), String::new());
        container.id = "container-12345678".to_string();

        let mut planning =
            crate::domain::types::fixture_issue("planning".to_string(), String::new());
        planning.id = "planning-12345678".to_string();
        planning.labels = vec!["type:planning".to_string()];

        let mut breakdown =
            crate::domain::types::fixture_issue("breakdown".to_string(), String::new());
        breakdown.id = "breakdown-12345678".to_string();
        breakdown.labels = vec![
            "type:breakdown".to_string(),
            format!("scaffold:{}", container.short_id()),
        ];
        breakdown.dependencies = vec![planning.id.clone()];

        let by_id = std::collections::HashMap::from([
            (planning.id.as_str(), &planning),
            (breakdown.id.as_str(), &breakdown),
        ]);
        let resolved = find_planning_node(&container, template, &registry.roles, &by_id).unwrap();

        assert_eq!(resolved.id, planning.id);
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

        let error = executor.validate_silent().unwrap_err();

        assert!(
            format!("{error:#}").contains("index.json"),
            "missing index must fail through captured-image validation: {error:#}"
        );
    }

    #[test]
    fn test_validate_fix_repairs_stale_derived_projection_through_the_session() {
        let (repo, storage, taxonomy) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let jit_dir = repo.path().join(".jit");
        let config = format!(
            "{}\n\
            [item_kinds.invariant]\nsection = \"success_criteria\"\nid-pattern = \"[a-z-]+\"\nmarkers = []\nlink-namespaces = []\nscope = \"project\"\nsource-of-truth = \"registry-first\"\nsource = {{ toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }}\n\
            [projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\ntarget = \"AGENTS.md\"\nstyle = \"id-anchor\"\n",
            taxonomy.config_fragment()
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
        let mut executor = CommandExecutor::new(storage).with_layout(layout);

        let (fixes, _messages) = executor.validate_with_fix(true, false).unwrap();
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
        let (again, _) = executor.validate_with_fix(true, false).unwrap();
        assert_eq!(again, 0, "repair must be idempotent");
    }

    #[test]
    fn test_validate_fix_repairs_every_owned_materialization_and_preserves_unowned_files() {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .initialize_fresh_repository(
                repo.path(),
                &crate::test_taxonomy::test_taxonomy().hierarchy_template(),
                Some("jit-dogfood"),
            )
            .unwrap();

        let owned = [
            ".jit/rules.toml",
            ".jit/schemas/default-type-hierarchy-known.json",
            ".jit/reference/rules-and-gates.md",
            "AGENTS.md",
            ".agents/skills/jit-manage/SKILL.md",
        ];
        let mut baseline = owned
            .iter()
            .map(|path| (*path, std::fs::read(repo.path().join(path)).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let unowned = repo.path().join(".jit/schemas/default-unowned.json");
        std::fs::write(&unowned, b"{}\n").unwrap();

        let rules = String::from_utf8(baseline[".jit/rules.toml"].clone()).unwrap();
        let (stale_rules, repaired_rules) = drift_default_assertion(&rules);
        std::fs::write(repo.path().join(".jit/rules.toml"), stale_rules).unwrap();
        baseline.insert(".jit/rules.toml", repaired_rules.into_bytes());
        let mut schema = baseline[".jit/schemas/default-type-hierarchy-known.json"].clone();
        schema.push(b' ');
        std::fs::write(
            repo.path()
                .join(".jit/schemas/default-type-hierarchy-known.json"),
            schema,
        )
        .unwrap();
        std::fs::write(
            repo.path().join(".jit/reference/rules-and-gates.md"),
            b"STALE\n",
        )
        .unwrap();
        let agents = String::from_utf8(baseline["AGENTS.md"].clone()).unwrap();
        std::fs::write(
            repo.path().join("AGENTS.md"),
            agents
                .replace("_No invariants declared._", "STALE INVARIANTS")
                .replace("## JIT workflow", "## STALE workflow"),
        )
        .unwrap();
        std::fs::write(
            repo.path().join(".agents/skills/jit-manage/SKILL.md"),
            b"STALE\n",
        )
        .unwrap();

        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let mut executor = CommandExecutor::new(storage).with_layout(layout);
        let first_error = format!("{:#}", executor.validate_silent().unwrap_err());
        let second_error = format!("{:#}", executor.validate_silent().unwrap_err());
        assert_eq!(
            first_error, second_error,
            "drift diagnostics must be stable"
        );
        for path in owned {
            assert!(
                first_error.contains(path.trim_start_matches(".jit/")),
                "diagnostics must identify {path}: {first_error}"
            );
        }

        let (fixes, _) = executor.validate_with_fix(true, false).unwrap();
        assert!(fixes >= baseline.len());
        for (path, bytes) in baseline {
            assert_eq!(
                std::fs::read(repo.path().join(path)).unwrap(),
                bytes,
                "{path}"
            );
        }
        assert_eq!(std::fs::read(&unowned).unwrap(), b"{}\n");
        assert_eq!(executor.validate_with_fix(true, false).unwrap().0, 0);
    }

    #[test]
    fn test_validate_fix_rejects_ambiguous_region_without_writing_other_repairs() {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .initialize_fresh_repository(
                repo.path(),
                &crate::test_taxonomy::test_taxonomy().hierarchy_template(),
                Some("jit-dogfood"),
            )
            .unwrap();

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
        let mut executor = CommandExecutor::new(storage).with_layout(layout);
        let error = executor.validate_with_fix(true, false).unwrap_err();
        assert!(format!("{error:#}").contains("invariants"), "{error:#}");
        assert_eq!(std::fs::read(&rules_path).unwrap(), before_rules);
        assert_eq!(std::fs::read(&agents_path).unwrap(), before_agents);
    }

    #[test]
    fn test_validate_fix_repairs_owned_materializations_in_memory() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        use crate::storage::IssueStore;

        let storage = memory_fixture(Some("jit-dogfood"));
        let mut executor = memory_executor(storage.clone());
        executor.validate_silent().unwrap();

        let owned = [
            ".jit/rules.toml",
            ".jit/schemas/default-type-hierarchy-known.json",
            ".jit/reference/rules-and-gates.md",
            "AGENTS.md",
            ".agents/skills/jit-manage/SKILL.md",
        ];
        let mut baseline = owned
            .iter()
            .map(|path| {
                (
                    *path,
                    storage.read_repo_file(path).unwrap().expect("owned file"),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        seed_repo_file(&storage, ".jit/schemas/default-unowned.json", "{}\n");
        let (stale_rules, repaired_rules) = drift_default_assertion(&baseline[".jit/rules.toml"]);
        seed_repo_file(&storage, ".jit/rules.toml", &stale_rules);
        baseline.insert(".jit/rules.toml", repaired_rules);
        seed_repo_file(
            &storage,
            ".jit/schemas/default-type-hierarchy-known.json",
            &(baseline[".jit/schemas/default-type-hierarchy-known.json"].clone() + " "),
        );
        seed_repo_file(&storage, ".jit/reference/rules-and-gates.md", "STALE\n");
        seed_repo_file(
            &storage,
            "AGENTS.md",
            &baseline["AGENTS.md"]
                .replace("_No invariants declared._", "STALE INVARIANTS")
                .replace("## JIT workflow", "## STALE workflow"),
        );
        seed_repo_file(&storage, ".agents/skills/jit-manage/SKILL.md", "STALE\n");

        let error = format!("{:#}", executor.validate_silent().unwrap_err());
        for path in owned {
            assert!(
                error.contains(path.trim_start_matches(".jit/")),
                "diagnostics must identify {path}: {error}"
            );
        }
        let (fixes, _) = executor.validate_with_fix(true, false).unwrap();
        assert!(fixes >= baseline.len());
        for (path, content) in baseline {
            assert_eq!(
                storage.read_repo_file(path).unwrap().unwrap(),
                content,
                "{path}"
            );
        }
        assert_eq!(
            storage
                .read_repo_file(".jit/schemas/default-unowned.json")
                .unwrap()
                .unwrap(),
            "{}\n"
        );
        assert_eq!(executor.validate_with_fix(true, false).unwrap().0, 0);
    }

    #[test]
    fn test_validate_fix_rejects_ambiguous_region_without_memory_writes() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        use crate::storage::IssueStore;

        let storage = memory_fixture(Some("jit-dogfood"));
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
        seed_repo_file(&storage, ".jit/rules.toml", &rules);
        seed_repo_file(&storage, "AGENTS.md", &agents);

        let mut executor = memory_executor(storage.clone());
        let error = executor.validate_with_fix(true, false).unwrap_err();
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

    /// A syntactically valid applied-profile provenance record naming `id`.
    fn applied_record_json(id: &str) -> String {
        serde_json::to_string_pretty(&crate::repository_state::AppliedProfileRecord::new(
            id,
            "1.0.0",
            crate::profile::ProfileOrigin::Embedded,
            "0".repeat(64),
            std::collections::BTreeMap::new(),
        ))
        .unwrap()
    }

    /// REQ-06 case 1 / REQ-01: no applied-profile record. Validation and repair
    /// settle without any profile package being loaded — a loaded package
    /// contributes its targets to the closure, so a repair closure equal to the
    /// plain validation closure is that absence, observed.
    #[test]
    fn test_capture_repair_plan_without_applied_record_loads_no_profile_package() {
        use crate::commands::test_helpers::memory_executor;
        use crate::storage::RepositoryStateStore;

        let executor = memory_executor(memory_fixture(None));
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();

        let plain = executor
            .capture_proposed_base(
                session.as_mut(),
                &std::collections::BTreeMap::new(),
                &[],
                None,
            )
            .unwrap()
            .expect("a settled capture");
        let captured = executor
            .capture_repair_plan(session.as_mut(), &repair_seed().unwrap())
            .unwrap()
            .expect("a settled capture")
            .expect("a repository that has applied no profile validates");

        assert_eq!(
            captured
                .image
                .capture_spec()
                .paths()
                .collect::<std::collections::BTreeSet<_>>(),
            plain
                .capture_spec()
                .paths()
                .collect::<std::collections::BTreeSet<_>>(),
            "repair must read no path plain validation does not"
        );
        assert!(
            captured.plan.is_some(),
            "repair still derives a plan for the repository's own declarations"
        );
    }

    /// REQ-06 case 2 / REQ-02: a record whose package resolves. The package's
    /// targets enter the capture closure, and a drifted one is a repair action.
    #[test]
    fn test_capture_repair_plan_with_recorded_profile_captures_and_repairs_its_targets() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        use crate::storage::RepositoryStateStore;

        const ASSET: &str = ".agents/skills/jit-manage/SKILL.md";

        let storage = memory_fixture(Some("jit-dogfood"));
        seed_repo_file(&storage, ASSET, "STALE PROFILE ASSET\n");
        let executor = memory_executor(storage);
        let layout = executor.require_layout().unwrap();
        let asset = layout.classify_repository_relative(ASSET).unwrap();
        let mut session = executor
            .storage()
            .open_mutation_session(layout.clone())
            .unwrap();

        let captured = executor
            .capture_repair_plan(session.as_mut(), &repair_seed().unwrap())
            .unwrap()
            .expect("a settled capture")
            .expect("a record whose package resolves validates");

        assert!(
            captured.image.capture_spec().contains_path(&asset),
            "a recorded profile's targets must be captured"
        );
        assert!(
            captured
                .plan
                .expect("loadable declarations derive a plan")
                .delta()
                .actions()
                .iter()
                .any(|action| action.path() == &asset),
            "the drifted profile-owned target must be a repair action"
        );
    }

    /// REQ-05: repository validation with nothing to do with profiles is
    /// unaffected by which profile case the repository is in. One repository,
    /// one non-profile drift, the record present and then absent.
    #[test]
    fn test_validate_diagnoses_non_profile_drift_identically_across_record_states() {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        executor
            .initialize_fresh_repository(
                repo.path(),
                &crate::test_taxonomy::test_taxonomy().hierarchy_template(),
                Some("jit-dogfood"),
            )
            .unwrap();
        let rules_path = repo.path().join(".jit/rules.toml");
        let rules = std::fs::read_to_string(&rules_path).unwrap();
        std::fs::write(&rules_path, drift_default_assertion(&rules).0).unwrap();

        let with_record = format!("{:#}", executor.validate_silent().unwrap_err());
        std::fs::remove_file(repo.path().join(".jit/profiles/jit-dogfood.json")).unwrap();
        let without_record = format!("{:#}", executor.validate_silent().unwrap_err());

        assert!(
            with_record.contains("rules.toml"),
            "the non-profile drift must be diagnosed: {with_record}"
        );
        assert_eq!(
            with_record, without_record,
            "the profile record must not change a non-profile diagnosis"
        );

        std::fs::write(
            repo.path().join(".jit/profiles/absent-workflow.json"),
            applied_record_json("absent-workflow"),
        )
        .unwrap();
        let unobtainable = format!("{:#}", executor.validate_silent().unwrap_err());
        assert!(
            unobtainable.contains("absent-workflow"),
            "an unobtainable recorded package is the reported failure: {unobtainable}"
        );
    }

    /// A stored record whose origin names a directory it cannot address is a
    /// validation failure rather than a field validation reads past: the
    /// location is how a later run finds the package again, so validation that
    /// ignored an absent or malformed one would report a repository as coherent
    /// while its record addressed nothing.
    #[test]
    fn test_validate_fails_on_an_applied_record_whose_directory_location_is_absent_or_malformed() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};

        /// The `jit-dogfood` record with `origin` replaced, as stored text.
        fn stored_record(origin: serde_json::Value) -> String {
            let mut record: serde_json::Value =
                serde_json::from_str(&applied_record_json("jit-dogfood")).unwrap();
            record["origin"] = origin;
            serde_json::to_string_pretty(&record).unwrap()
        }

        fn diagnosis(record: &str) -> String {
            let storage = memory_fixture(None);
            seed_repo_file(&storage, ".jit/profiles/jit-dogfood.json", record);
            format!(
                "{:#}",
                memory_executor(storage).validate_silent().unwrap_err()
            )
        }

        // The control is the same record with an origin that does address its
        // bytes: its package resolves, so validation gets past provenance and
        // fails on the hash mismatch this synthetic record carries instead.
        let control = diagnosis(&stored_record(serde_json::json!({ "source": "embedded" })));
        assert!(
            !control.contains("invalid applied profile provenance"),
            "the control record must read as valid provenance: {control}"
        );

        for origin in [
            serde_json::json!({ "source": "directory" }),
            serde_json::json!({ "source": "directory", "location": "../outside" }),
            serde_json::json!({ "source": "directory", "location": "/absolute" }),
        ] {
            let error = diagnosis(&stored_record(origin.clone()));
            assert!(
                error.contains("invalid applied profile provenance"),
                "{origin} must fail validation: {error}"
            );
        }
    }

    /// REQ-04: a well-formed applied-profile record naming a profile whose
    /// package cannot be obtained. Repair reports that condition and restores
    /// nothing — narrowing `--fix` to the targets it can still account for
    /// would weaken `@/invariant/derived-state-coherence` without saying so.
    /// The control half proves the drift is one repair would otherwise fix.
    #[test]
    fn test_validate_fix_fails_when_a_recorded_profile_package_is_unobtainable() {
        use crate::commands::test_helpers::{memory_executor, seed_repo_file};
        use crate::storage::IssueStore;

        let storage = memory_fixture(None);
        let (stale_rules, repaired_rules) =
            drift_default_assertion(&storage.read_repo_file(".jit/rules.toml").unwrap().unwrap());
        seed_repo_file(&storage, ".jit/rules.toml", &stale_rules);
        seed_repo_file(
            &storage,
            ".jit/profiles/absent-workflow.json",
            &applied_record_json("absent-workflow"),
        );

        let mut executor = memory_executor(storage.clone());
        let error = format!("{:#}", executor.validate_with_fix(true, false).unwrap_err());

        assert!(
            error.contains("absent-workflow") && error.contains("cannot be obtained"),
            "the failure must name the record and its unobtainable package: {error}"
        );
        assert_eq!(
            storage.read_repo_file(".jit/rules.toml").unwrap().unwrap(),
            stale_rules,
            "a repair that cannot account for a recorded profile must restore nothing"
        );

        let control = memory_fixture(None);
        seed_repo_file(&control, ".jit/rules.toml", &stale_rules);
        let mut control_executor = memory_executor(control.clone());
        assert!(control_executor.validate_with_fix(true, false).unwrap().0 > 0);
        assert_eq!(
            control.read_repo_file(".jit/rules.toml").unwrap().unwrap(),
            repaired_rules,
            "the same drift without the record is repairable, so the assertion above has force"
        );
    }

    #[test]
    fn test_validate_type_fix_is_noop_after_captured_repair() {
        use crate::commands::test_helpers::memory_executor;
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        let config = "[worktree]\nenforce_leases = \"off\"\n\
             [type_hierarchy.types]\ntask = 4\n\
             [namespaces.type]\ndescription = \"Issue type\"\nunique = true\n";
        storage.add_data_file("config.toml", config);
        // The type-repair path reads the declared hierarchy through a
        // `ConfigManager` rooted at the store, so the fixture declares it there
        // as well as in the memory image.
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        let mut executor = memory_executor(storage.clone());
        let id = executor
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
            .0;

        let (first, _) = executor.validate_with_fix(true, false).unwrap();
        assert!(first > 0);
        let repaired = storage.load_issue(&id).unwrap();
        assert!(repaired.labels.iter().any(|label| label == "type:task"));
        assert!(!repaired.labels.iter().any(|label| label == "type:taks"));

        let (second, _) = executor.validate_with_fix(true, false).unwrap();
        assert_eq!(second, 0, "a captured type repair must not repeat");
    }

    #[test]
    fn test_apply_type_fix_preserves_concurrent_type_correction() {
        use crate::commands::test_helpers::{memory_executor, with_open_race, OpenRaceAction};
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        let executor = memory_executor(storage.clone());
        let id = executor
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
            .0;

        let mut issue = storage.load_issue(&id).unwrap();
        issue.labels = vec!["type:story".to_string()];
        let raced = with_open_race(storage, 2, OpenRaceAction::Save(Box::new(issue)));
        let executor = memory_executor(raced.clone());

        executor.apply_type_fix(&id, "taks", "task").unwrap();

        assert_eq!(raced.load_issue(&id).unwrap().labels, ["type:story"]);
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(Duration::seconds(1)), "1 second");
        assert_eq!(format_duration(Duration::seconds(30)), "30 seconds");
        assert_eq!(format_duration(Duration::seconds(59)), "59 seconds");
    }

    #[test]
    fn test_render_selector_state_single() {
        use crate::declarations::rules::{Selector, StatePredicate};
        let sel = Selector {
            type_: Some("epic".to_string()),
            state: Some(StatePredicate::single("in_progress")),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), "type=epic, state=in_progress");
    }

    #[test]
    fn test_render_selector_state_list_joins_with_pipe() {
        use crate::declarations::rules::{Selector, StatePredicate};
        let sel = Selector {
            state: Some(StatePredicate::list(["ready", "in_progress"])),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), "state=ready|in_progress");
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

    /// The complete `[item_kinds]` table `jit init` authors. The engine bakes in no
    /// kinds, so the dangling-link pass (which owns a label's namespace only when a
    /// declared kind claims it) needs the table to recognize the test labels.
    const CANONICAL_ITEM_KINDS: &str = "\
[item_kinds.requirement]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = [\"[hard]\"]
link-namespaces = [\"satisfies\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.decision]
section = \"decisions\"
id-pattern = \"D-[0-9]+\"
markers = []
link-namespaces = [\"per\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.risk]
section = \"risks\"
id-pattern = \"RISK-[0-9]+\"
markers = []
link-namespaces = [\"mitigates\", \"resolves\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.invariant]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }
source-of-truth = \"registry-first\"
";

    /// Build an executor over an in-memory `.jit` carrying a `.jit/invariants.toml`
    /// (so the registry-first `invariant` kind resolves `enforces:@/invariant/<id>`) and the
    /// canonical `[item_kinds]` table, seeded with `issues`.
    fn dangling_exec(issues: Vec<Issue>) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("config.toml"), CANONICAL_ITEM_KINDS).unwrap();
        storage.add_data_file("config.toml", CANONICAL_ITEM_KINDS);
        // The registry-first `invariant` kind reads its toml through the storage
        // boundary at the descriptor path, so seed the in-memory repo-file map (not
        // the real fs) at `.jit/invariants.toml`.
        storage.add_data_file("invariants.toml", REGISTRY_TOML);
        for issue in issues {
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }
        finish_dangling_executor(storage)
    }

    /// Like [`dangling_exec`] but its `config.toml` ALSO registers the link
    /// namespaces used by these tests (`satisfies`/`enforces`) so the default
    /// `namespace-registry` local rule does not fire on the synthetic labels —
    /// isolating the dangling-item-link pass as the validation failure.
    fn dangling_exec_with_namespaces(issues: Vec<Issue>) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();
        // Invariant registry through the storage boundary (descriptor path); the
        // `config.toml` is parsed from the real `.jit` root by `cached_config`.
        storage.add_data_file("invariants.toml", REGISTRY_TOML);
        let config = format!(
            "[namespaces.type]\ndescription = \"issue type\"\nunique = true\n\
             [namespaces.satisfies]\ndescription = \"satisfied item\"\nunique = false\n\
             [namespaces.enforces]\ndescription = \"enforced invariant\"\nunique = false\n\
             {CANONICAL_ITEM_KINDS}"
        );
        std::fs::write(storage.root().join("config.toml"), &config).unwrap();
        storage.add_data_file("config.toml", &config);
        for issue in issues {
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }
        finish_dangling_executor(storage)
    }

    fn finish_dangling_executor(storage: InMemoryStorage) -> CommandExecutor<InMemoryStorage> {
        let mut ids = storage
            .list_issues()
            .unwrap()
            .into_iter()
            .map(|issue| issue.id)
            .collect::<Vec<_>>();
        ids.sort();
        storage.add_data_file(
            "index.json",
            &serde_json::json!({"schema_version": 2, "all_ids": ids, "deleted_ids": []})
                .to_string(),
        );
        let layout = storage.repository_layout();
        CommandExecutor::new(storage).with_layout(layout)
    }

    fn issue_with_labels(title: &str, body: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::domain::types::fixture_issue(title.to_string(), body.to_string());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue
    }

    /// `[item_kinds]` declarations for `rule` and `gate` (jit:d30695e4), both
    /// declaring `link-namespaces = ["enforces"]` like the live repo's config, so
    /// an `enforces:@/rule/<name>` / `enforces:@/gate/<key>` label is recognized
    /// by the dangling-link pass alongside [`CANONICAL_ITEM_KINDS`]'s invariant.
    const RULE_AND_GATE_ITEM_KINDS: &str = "\
[item_kinds.rule]
section = \"success_criteria\"
id-pattern = \"[a-z][a-z0-9-]*\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/rules.toml\", table = \"rules\", id-field = \"name\", text-field = \"name\" }
source-of-truth = \"registry-first\"

[item_kinds.gate]
section = \"success_criteria\"
id-pattern = \"[a-z][a-z0-9-]*\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/gates.toml\", table = \"gates\", id-field = \"key\", text-field = \"description\" }
source-of-truth = \"registry-first\"
";

    const ONE_RULE: &str = "\
[[rules]]
name = \"label-format\"
";

    const ONE_GATE: &str = "\
[[gates]]
key = \"cargo-ci\"
description = \"Full Rust CI pipeline must pass.\"
";

    /// Like [`dangling_exec`] but ALSO declares `rule` and `gate` item kinds and
    /// seeds `.jit/rules.toml` / `.jit/gates.toml`, exercising the dangling-link
    /// pass for `enforces:@/rule/<name>` and `enforces:@/gate/<key>` labels.
    fn dangling_exec_with_rules_and_gates(issues: Vec<Issue>) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();
        let config = format!("{CANONICAL_ITEM_KINDS}\n{RULE_AND_GATE_ITEM_KINDS}");
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        storage.add_data_file("invariants.toml", REGISTRY_TOML);
        storage.add_data_file("rules.toml", ONE_RULE);
        storage.add_data_file("gates.toml", ONE_GATE);
        for issue in issues {
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }
        CommandExecutor::new(storage)
    }

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
            &["enforces:@/rule/label-format", "enforces:@/gate/cargo-ci"],
        );
        let exec = dangling_exec_with_rules_and_gates(vec![node]);
        let issues = exec.storage().list_issues().unwrap();
        assert!(
            exec.dangling_link_findings(&issues).unwrap().is_empty(),
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
        let exec = dangling_exec_with_rules_and_gates(vec![node]);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
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
        let exec = dangling_exec(vec![target, node]);

        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].issue_id.as_deref(), Some(node_id.as_str()));
        assert_eq!(findings[0].finding.rule, DANGLING_LINK_RULE);
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
        let exec = dangling_exec(vec![target, node]);

        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
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
        let exec = dangling_exec(vec![target, node]);
        let issues = exec.storage().list_issues().unwrap();
        assert!(
            exec.dangling_link_findings(&issues).unwrap().is_empty(),
            "resolvable explicit-kind links must produce no finding"
        );

        // An unresolvable explicit form is still classified as qualified and
        // reported as dangling, never silently ignored as unqualified.
        let bad = issue_with_labels("bad", "", &["enforces:@/invariant/missing-invariant"]);
        let exec = dangling_exec(vec![bad]);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
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
        let exec = dangling_exec(vec![node]);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("dangling item link"));
        // The dangling classification comes from `resolve_link_label` erroring on
        // the SAME value. Assert that error's chain carries the three PARSED
        // components in distinct phrasing, proving the named-project value was
        // structurally routed to (project, kind, self-id) rather than mis-split.
        let err = exec
            .resolve_link_label("enforces:@acme/invariant/sample-invariant")
            .unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("project 'acme'"), "got: {chain}");
        assert!(chain.contains("kind 'invariant'"), "got: {chain}");
        assert!(chain.contains("self-id 'sample-invariant'"), "got: {chain}");
    }

    #[test]
    fn test_dangling_link_findings_unqualified_and_non_link_ns_ignored() {
        // A legacy unqualified label (`satisfies:REQ-01`) and a non-link namespace
        // (`type:task`) are NOT link references and produce no finding.
        let node = issue_with_labels("node", "", &["satisfies:REQ-01", "type:task", "epic:foo"]);
        let exec = dangling_exec(vec![node]);
        let issues = exec.storage().list_issues().unwrap();
        assert!(exec.dangling_link_findings(&issues).unwrap().is_empty());
    }

    #[test]
    fn test_dangling_invariant_link_reports_finding() {
        // REQ-03 for the project-scope invariant kind: `enforces:@/invariant/missing-invariant`
        // is a registered link namespace with a qualified-but-unresolvable id.
        let node = issue_with_labels("node", "", &["enforces:@/invariant/missing-invariant"]);
        let exec = dangling_exec(vec![node]);
        let issues = exec.storage().list_issues().unwrap();
        let findings = exec.dangling_link_findings(&issues).unwrap();
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
        // Register the `satisfies` namespace so the dangling-link pass — not the
        // namespace-registry local rule — is the validation failure.
        let exec = dangling_exec_with_namespaces(vec![target, node]);

        std::env::set_var("JIT_TEST_MODE", "1");
        let err = exec.validate_silent().unwrap_err();
        std::env::remove_var("JIT_TEST_MODE");
        let msg = err.to_string();
        assert!(
            msg.contains(DANGLING_LINK_RULE) && msg.contains("BOGUS"),
            "expected dangling-link rule failure, got: {msg}"
        );
    }

    #[test]
    fn test_run_rules_whole_repo_surfaces_dangling_link() {
        // REQ-03 via the structured report `jit validate [--json]` consumes:
        // `run_rules(None)` reports the dangling link as an error finding (which
        // serializes for `--json`).
        let target =
            issue_with_labels("target", "## Success Criteria\n\n- [hard] REQ-01: a\n", &[]);
        let short = target.short_id();
        let mut node = issue_with_labels("node", "", &[&format!("satisfies:{short}/BOGUS")]);
        node.dependencies = vec![target.id.clone()];
        let exec = dangling_exec(vec![target, node]);

        let report = exec.run_rules(None).unwrap();
        assert!(report.has_errors());
        let dangling: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.rule == DANGLING_LINK_RULE)
            .collect();
        assert_eq!(dangling.len(), 1);
        assert!(dangling[0].message.contains("BOGUS"));
        // The finding serializes (used by `jit validate --json`).
        let value = serde_json::to_value(&report.findings).unwrap();
        assert!(value.to_string().contains(DANGLING_LINK_RULE));
    }

    /// `jit validate <id>` takes the same short-id prefix every other command
    /// takes. `load_issue` keys on the full id, so `run_rules` must resolve the
    /// prefix first; without that it reports the issue as not found.
    #[test]
    fn test_run_rules_accepts_short_id_prefix() {
        let target =
            issue_with_labels("target", "## Success Criteria\n\n- [hard] REQ-01: a\n", &[]);
        let full_id = target.id.clone();
        let short_id = target.short_id();
        let exec = dangling_exec(vec![target]);

        let by_short = exec
            .run_rules(Some(&short_id))
            .expect("short-id prefix must resolve");
        let by_full = exec
            .run_rules(Some(&full_id))
            .expect("full id must resolve");

        assert_eq!(
            by_short.findings.len(),
            by_full.findings.len(),
            "the short-id prefix must select the same issue as the full id"
        );
    }

    /// The same resolution holds for `jit validate --explain <id>`.
    #[test]
    fn test_explain_rules_accepts_short_id_prefix() {
        let target =
            issue_with_labels("target", "## Success Criteria\n\n- [hard] REQ-01: a\n", &[]);
        let full_id = target.id.clone();
        let short_id = target.short_id();
        let exec = dangling_exec(vec![target]);

        let by_short = exec
            .explain_rules(&short_id)
            .expect("short-id prefix must resolve");
        let by_full = exec.explain_rules(&full_id).expect("full id must resolve");

        assert_eq!(by_short.issue_id, by_full.issue_id);
        assert_eq!(by_short.outcomes.len(), by_full.outcomes.len());
    }
}
