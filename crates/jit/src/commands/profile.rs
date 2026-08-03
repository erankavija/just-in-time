use super::{capture_or_retry, with_mutation_session, CommandExecutor, SessionStep};
use crate::profile::{
    build_profile_claims, ProfileApplicationStatus, ProfileApplyResult, ProfileComposedApplyResult,
    ProfileId, ProfileListResult, ProfileOrigin, ProfilePackage, ProfilePackageError,
    ProfilePackageSource, ProfilePlanResult, ProfilePlanStatus, ProfileShowResult, ProfileSummary,
    ProfileTargetAction, ProfileTargetChange,
};
use crate::repository_state::{
    apply_overlay, derive_materialization, AppliedProfileRecord, CaptureBudget, CaptureSpec,
    MaterializationPlan, MaterializationRequest, MutationContext, ProfileApplicationInput,
    ProfileTargetDisposition, RepositoryEntry, RepositoryImage, RepositoryLayout,
    RepositoryRootClass, VirtualPath,
};
use crate::storage::{JsonFileStorage, RepositoryMutationSession};
use crate::validation::repository::RepositoryValidationFailure;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Failure resolving which package bytes a command reads.
///
/// Resolution takes the first route that answers — a location the caller
/// supplied, then the location this repository's applied-profile record names —
/// and each variant names the route that failed together with what it
/// addressed. A recorded location that no longer resolves is one of these
/// rather than an absent profile: a deleted directory is a repository whose
/// record outlived its package, not a repository that never applied one.
#[derive(Debug, thiserror::Error)]
pub enum ProfileResolutionError {
    /// A supplied location holds no readable package.
    #[error("profile package location '{location}' cannot be read: {source}")]
    UnreadableLocation {
        /// Location as the caller supplied it.
        location: String,
        /// Package-reader failure.
        source: ProfilePackageError,
    },
    /// A supplied location holds a package declaring another profile.
    #[error("profile package location '{location}' declares profile '{found}', not '{requested}'")]
    UnexpectedProfileAtLocation {
        /// Location as the caller supplied it.
        location: String,
        /// Profile the caller asked for.
        requested: String,
        /// Profile the package at that location declares.
        found: String,
    },
    /// A recorded location no longer holds a readable package.
    #[error(
        "applied profile record '{record}' names package location '{location}', \
         which no longer holds a readable package: {source}"
    )]
    UnresolvableRecordedLocation {
        /// Repository-relative applied-record path.
        record: String,
        /// Worktree-relative location the record names.
        location: String,
        /// Package-reader failure.
        source: ProfilePackageError,
    },
}

/// Failure composing the set of packages one application applies.
///
/// Both variants are raised while the closure is still being built, before the
/// first package is applied, so a repository never carries part of a set it
/// could not compose.
#[derive(Debug, thiserror::Error)]
pub enum ProfileDependencyError {
    /// A declared dependency resolves through no route.
    ///
    /// The declaring package is named beside it because an adopter asked for
    /// that one: a bare failure over the dependency would name a package they
    /// never installed by name.
    #[error(
        "profile '{package}' declares a dependency on profile '{dependency}', \
         which cannot be resolved: {cause:#}"
    )]
    UnresolvableDependency {
        /// Package whose manifest declares the dependency.
        package: String,
        /// Dependency that manifest names.
        dependency: String,
        /// Why every resolution route refused it.
        cause: anyhow::Error,
    },
    /// The declared dependencies close a cycle, which has no application order.
    #[error("profile dependency cycle: {}", .cycle.join(" -> "))]
    DependencyCycle {
        /// The cycle as a closed path: the first id repeats as the last.
        cycle: Vec<String>,
    },
}

/// Profile application conflict detected before transaction preparation.
#[derive(Debug, thiserror::Error)]
pub enum ProfileApplyError {
    /// The installed record is not readable provenance for its own profile.
    #[error("installed profile record '{path}' is not a readable record for profile '{id}'")]
    InstalledRecordConflict {
        /// Repository-relative installed-record path.
        path: String,
        /// Profile the record's own name identifies.
        id: String,
    },
    /// The installed-record directory has an unsafe occupant.
    #[error("profile metadata path '{path}' has unsupported filesystem state")]
    UnsupportedMetadataPath {
        /// Repository-relative path.
        path: String,
    },
    /// Final package, provenance, and event overlay failed validation.
    #[error("final profile application validation failed: {0}")]
    FinalValidation(#[from] RepositoryValidationFailure),
    /// Final overlay produced error-severity rule findings.
    #[error("final profile application validation produced {error_count} error finding(s)")]
    FinalValidationFindings {
        /// Number of blocking findings.
        error_count: usize,
    },
    /// Package targets overlap application-owned audit or provenance state.
    #[error("profile package target '{path}' is reserved for application state")]
    ReservedApplicationTarget {
        /// Conflicting repository-relative package target.
        path: String,
    },
    /// A package's bytes were read from outside the repository worktree.
    #[error("profile package directory '{path}' is not inside the repository worktree")]
    PackageOutsideWorktree {
        /// Resolved package directory the bytes were read from.
        path: String,
    },
}

impl CommandExecutor<JsonFileStorage> {
    /// Read the package one profile command acts on.
    ///
    /// The routes are tried in a fixed order and the first that answers wins:
    /// `location` when the caller supplies one, then the location this
    /// repository's applied-profile record for `id` names. A supplied location
    /// answers the first application, when the repository has obtained a
    /// package and recorded nothing yet; the record answers every run after it,
    /// so a caller need not remember where the bytes came from. A repository
    /// that has neither is not carrying that profile, which is a
    /// [`NotFoundError`](crate::errors::NotFoundError).
    ///
    /// A supplied location must hold a package declaring `id`, and a recorded
    /// location that no longer holds a readable package is a
    /// [`ProfileResolutionError`] naming the record and the location rather
    /// than an absent profile.
    pub fn resolve_profile_package(
        &self,
        id: &str,
        location: Option<&Path>,
    ) -> Result<ProfilePackage> {
        match location {
            Some(location) => supplied_package(location, id),
            None => match self.read_applied_profile_record(id)? {
                Some(record) => {
                    recorded_package(&record, &applied_record_path(id)?, &self.require_layout()?)
                }
                None => Err(
                    crate::errors::NotFoundError::new(format!("Profile not found: {id}")).into(),
                ),
            },
        }
    }

    /// List the profiles this repository's own applied-profile records name.
    ///
    /// The `.jit/profiles/` listing is where a repository states which profiles
    /// it carries, and each record's package is read from the location that
    /// record names, so the answer describes this repository rather than the
    /// running binary. A record whose package cannot be resolved fails the
    /// enumeration ([`ProfileResolutionError`]) instead of dropping the profile
    /// from the answer.
    pub fn list_recorded_profiles(&self) -> Result<ProfileListResult> {
        let layout = self.require_layout()?;
        let profiles_dir = VirtualPath::PROFILES;
        with_mutation_session(self.storage(), &layout, "profile enumeration", |session| {
            let Some((image, recorded)) = capture_applied_records(session, &profiles_dir)? else {
                return Ok(SessionStep::Retry);
            };
            let Some(profiles) = recorded
                .iter()
                .map(|id| recorded_summary(&image, id, &layout))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .collect::<Option<Vec<_>>>()
            else {
                return Ok(SessionStep::Retry);
            };
            Ok(SessionStep::Done(ProfileListResult {
                count: profiles.len(),
                profiles,
            }))
        })
    }

    /// Inspect one resolved profile package.
    pub fn show_profile(&self, id: &str, location: Option<&Path>) -> Result<ProfileShowResult> {
        let package = self.resolve_profile_package(id, location)?;
        let layout = self.require_layout()?;
        Ok(ProfileShowResult {
            manifest: package.manifest().clone(),
            origin: package_origin(&package, &layout)?,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
            file_count: package.file_count(),
            byte_size: package.byte_size(),
            applied: self.read_applied_profile_record(id)?,
        })
    }

    /// Build the exact non-mutating target plan for one resolved profile.
    pub fn plan_profile(&self, id: &str, location: Option<&Path>) -> Result<ProfilePlanResult> {
        let package = self.resolve_profile_package(id, location)?;
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        let context = MutationContext::preview();
        with_mutation_session(self.storage(), &layout, "profile planning", |session| {
            let Some((plan, changes)) = self.prepare_profile(session, &package, &context)? else {
                return Ok(SessionStep::Retry);
            };
            Ok(SessionStep::Done(ProfilePlanResult {
                id: metadata.id.to_string(),
                version: metadata.version.clone(),
                status: if plan.delta().actions().is_empty() {
                    ProfilePlanStatus::Unchanged
                } else {
                    ProfilePlanStatus::WouldApply
                },
                plan_hash: plan.hash().to_string(),
                targets: changes,
            }))
        })
    }

    /// Resolve and apply one profile by stable ID, with the packages it depends
    /// on.
    pub fn apply_profile(
        &self,
        id: &str,
        location: Option<&Path>,
    ) -> Result<ProfileComposedApplyResult> {
        let package = self.resolve_profile_package(id, location)?;
        self.apply_profile_package(&package)
    }

    /// Resolve the complete set of packages applying `package` applies, in the
    /// order it applies them.
    ///
    /// A package's declared dependencies are resolved transitively, so the
    /// answer closes over everything reachable from `package`, and ordered so
    /// each package follows everything it depends on. `package` itself is
    /// therefore last. A dependency reached by two packages is resolved and
    /// applied once.
    ///
    /// Nothing here touches the repository: a
    /// [`ProfileDependencyError::DependencyCycle`] or an unresolvable
    /// dependency is raised over the whole closure before its first package is
    /// applied.
    pub fn resolve_profile_closure(&self, package: &ProfilePackage) -> Result<Vec<ProfilePackage>> {
        let root = package.manifest().profile.id.to_string();
        let mut resolved = BTreeMap::from([(root.clone(), package.clone())]);
        let mut adjacency: Vec<(String, Vec<String>)> = Vec::new();
        let mut pending = std::collections::VecDeque::from([root]);

        while let Some(id) = pending.pop_front() {
            let declaring = resolved
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("package '{id}' left the closure being built"))?
                .clone();
            let declared: Vec<String> = declaring
                .manifest()
                .dependencies
                .iter()
                .map(ProfileId::to_string)
                .collect();
            for dependency in &declared {
                if resolved.contains_key(dependency) {
                    continue;
                }
                let package = self
                    .resolve_dependency_package(&declaring, dependency)
                    .map_err(|cause| ProfileDependencyError::UnresolvableDependency {
                        package: id.clone(),
                        dependency: dependency.clone(),
                        cause,
                    })?;
                resolved.insert(dependency.clone(), package);
                pending.push_back(dependency.clone());
            }
            adjacency.push((id, declared));
        }

        crate::graph::keyed_topological_order(&adjacency)
            .map_err(|cycle| ProfileDependencyError::DependencyCycle { cycle })?
            .into_iter()
            .map(|id| {
                resolved
                    .remove(&id)
                    .ok_or_else(|| anyhow::anyhow!("ordered package '{id}' left the closure"))
            })
            .collect()
    }

    /// Read the package one declared dependency names.
    ///
    /// A package read from a directory states where its dependencies are by
    /// where it sits: the dependency is looked for beside it, in a directory
    /// named by the dependency's own id. That is the location the caller named
    /// for the declaring package, carried to what that package declares, and it
    /// is what lets one obtained directory of packages apply as a set before
    /// any of them has a record.
    ///
    /// A directory that is not there, or that holds a package declaring another
    /// profile, is not that dependency, so resolution continues through the
    /// route every other command takes
    /// ([`resolve_profile_package`](Self::resolve_profile_package) without a
    /// location): this repository's own applied-profile record. A directory
    /// that is there and cannot be read as a package is reported rather than
    /// passed over, because falling through would answer with a package the
    /// adopter did not put there.
    fn resolve_dependency_package(
        &self,
        declaring: &ProfilePackage,
        dependency: &str,
    ) -> Result<ProfilePackage> {
        let ProfilePackageSource::Directory(directory) = declaring.source();
        let Some(location) = directory.parent().map(|parent| parent.join(dependency)) else {
            return self.resolve_profile_package(dependency, None);
        };
        match ProfilePackage::from_directory(&location) {
            Ok(package) if package.manifest().profile.id.as_str() == dependency => Ok(package),
            Ok(_) | Err(ProfilePackageError::UnreadableDirectory { .. }) => {
                self.resolve_profile_package(dependency, None)
            }
            Err(source) => Err(ProfileResolutionError::UnreadableLocation {
                location: location.display().to_string(),
                source,
            }
            .into()),
        }
    }

    /// Prove one profile selection resolves without mutating a repository.
    ///
    /// Initialization runs this before it publishes anything, so an
    /// unresolvable id or location fails before a repository is created. What a
    /// selection resolves to is the whole set applying it applies, so an
    /// unresolvable dependency and a dependency cycle fail here too rather than
    /// at the publication the check exists to precede.
    pub fn validate_profile_selection(&self, id: &str, location: Option<&Path>) -> Result<()> {
        let package = self.resolve_profile_package(id, location)?;
        self.resolve_profile_closure(&package).map(drop)
    }

    /// Apply one validated package together with the packages it depends on.
    ///
    /// The whole closure is resolved and ordered first
    /// ([`resolve_profile_closure`](Self::resolve_profile_closure)), so a cycle
    /// or an unresolvable dependency fails before any package is applied. Each
    /// package is then applied in that order through one application of its
    /// own, which is what gives every applied package its own provenance record
    /// and its own audit event. A package whose targets and provenance are
    /// already exact reports no work, so re-applying a set that is already
    /// applied publishes nothing.
    pub fn apply_profile_package(
        &self,
        package: &ProfilePackage,
    ) -> Result<ProfileComposedApplyResult> {
        self.resolve_profile_closure(package)?
            .iter()
            .map(|package| self.apply_one_profile_package(package))
            .collect::<Result<Vec<_>>>()
            .map(ProfileComposedApplyResult::new)
    }

    /// Apply one validated profile package through the recovered session.
    ///
    /// Each attempt captures the whole-repository base under the held session guard,
    /// derives the exact profile-owned targets through the repository-state
    /// materialization dispatcher,
    /// finalizes one exact profile-application delta, validates that delta's proposed
    /// overlay, and publishes through `session.apply` with pre-journal revalidation.
    /// A no-op profile has an empty complete finalized delta: package targets and
    /// provenance are unchanged, and coupled default-rule/schema state is current.
    ///
    /// This applies exactly the package it is handed; the packages that package
    /// depends on are applied by
    /// [`apply_profile_package`](Self::apply_profile_package), which orders them
    /// against it.
    pub(super) fn apply_one_profile_package(
        &self,
        package: &ProfilePackage,
    ) -> Result<ProfileApplyResult> {
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so the appended ProfileApplied event's id/timestamp stay stable.
        let context = MutationContext::production();
        with_mutation_session(self.storage(), &layout, "profile application", |session| {
            let Some((plan, _changes)) = self.prepare_profile(session, package, &context)? else {
                return Ok(SessionStep::Retry);
            };
            if plan.delta().actions().is_empty() {
                return Ok(SessionStep::Done(ProfileApplyResult {
                    id: metadata.id.to_string(),
                    version: metadata.version.clone(),
                    status: ProfileApplicationStatus::Unchanged,
                    plan_hash: plan.hash().to_string(),
                    transaction_id: None,
                    warnings: Vec::new(),
                }));
            }
            let proposed = apply_overlay(plan.image(), super::validation_overlay(plan.delta()))
                .map_err(anyhow::Error::from)?;
            let validation = crate::validation::repository::validate_repository(&proposed)
                .map_err(ProfileApplyError::from)?;
            if validation.rule_report.has_errors() {
                return Err(ProfileApplyError::FinalValidationFindings {
                    error_count: validation.rule_report.error_count(),
                }
                .into());
            }

            let result = ProfileApplyResult {
                id: metadata.id.to_string(),
                version: metadata.version.clone(),
                status: ProfileApplicationStatus::Applied,
                plan_hash: plan.hash().to_string(),
                // The applied transaction hash is the plan hash by construction.
                transaction_id: Some(plan.hash().to_string()),
                warnings: Vec::new(),
            };
            Ok(SessionStep::Apply(plan, result))
        })
    }

    /// Capture the base and derive one complete canonical profile plan.
    ///
    /// Returns `Ok(None)` on a retryable capture conflict so the caller re-attempts.
    /// The profile package is parsed into canonical claims
    /// ([`build_profile_claims`](crate::profile::build_profile_claims)) and composed
    /// into exact target bytes by `repository_state`; finalizing the complete probe
    /// delta decides whether the operation is a no-op and supplies the validation
    /// overlay reused by application.
    fn prepare_profile(
        &self,
        session: &mut (dyn RepositoryMutationSession + '_),
        package: &ProfilePackage,
        context: &MutationContext,
    ) -> Result<Option<(MaterializationPlan, Vec<ProfileTargetChange>)>> {
        let metadata = &package.manifest().profile;
        reject_reserved_application_targets(package.hashes().targets.keys().map(String::as_str))?;
        let record_path = applied_record_path(metadata.id.as_str())?;
        let profiles_dir = VirtualPath::PROFILES;
        let events_path = VirtualPath::EVENTS;
        let layout = self.require_layout()?;

        let mut content_paths = package
            .hashes()
            .targets
            .keys()
            .map(|target| {
                layout
                    .classify_repository_relative(target)
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        content_paths.extend(
            content_paths
                .clone()
                .into_iter()
                .map(|path| path.ancestor_directories())
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten(),
        );
        content_paths.push(record_path.clone());
        content_paths.push(profiles_dir.clone());
        content_paths.push(events_path.clone());

        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &content_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };

        let input = profile_application_input(package, base.layout(), record_path.clone())?;
        let mut expanded_paths = content_paths.clone();
        expanded_paths.extend(crate::repository_state::profile_capture_closure(
            &base,
            &input.claims,
        )?);
        // A package that re-derives the default rules reaches the schemas they
        // reference, and reaching a schema that is absent means proving its
        // absence against the directory listing, so the directory is discovered
        // whether or not the package names a target under it.
        if input.owns_default_rule_authority() {
            expanded_paths.push(VirtualPath::SCHEMAS);
        }
        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &expanded_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };
        let input = profile_application_input(package, base.layout(), record_path.clone())?;
        let preview = derive_materialization(
            &base,
            MaterializationRequest::ApplyProfile {
                profile: input,
                context,
            },
        )?;
        let mut extra_paths = expanded_paths;
        extra_paths.extend(
            preview
                .delta()
                .actions()
                .iter()
                .map(|action| action.path().clone()),
        );
        let overrides = super::validation_overlay(preview.delta());
        let probe = match self.capture_proposed_base(session, &overrides, &extra_paths, None)? {
            None => return Ok(None),
            Some(base) => base,
        };
        let input = profile_application_input(package, probe.layout(), record_path)?;
        let final_closure =
            crate::repository_state::profile_capture_closure(&probe, &input.claims)?;
        if final_closure
            .iter()
            .any(|path| !probe.capture_spec().contains_path(path))
        {
            return Ok(None);
        }
        let plan = derive_materialization(
            &probe,
            MaterializationRequest::ApplyProfile {
                profile: input,
                context,
            },
        )?;
        if plan.delta().actions().iter().any(|action| {
            !probe
                .capture_spec()
                .paths()
                .any(|path| path == action.path())
        }) {
            return Ok(None);
        }
        let changes = plan
            .profile_targets()
            .iter()
            .map(|target| {
                let action = match target.disposition {
                    ProfileTargetDisposition::Unchanged => ProfileTargetAction::Unchanged,
                    ProfileTargetDisposition::Create => ProfileTargetAction::Create,
                    ProfileTargetDisposition::Update => ProfileTargetAction::Update,
                };
                ProfileTargetChange::new(target.path.repository_relative(), action, target.mode)
            })
            .collect();
        Ok(Some((plan, changes)))
    }

    /// Read the applied-profile record this repository holds for `id`, through
    /// a recovered session capture.
    fn read_applied_profile_record(&self, id: &str) -> Result<Option<AppliedProfileRecord>> {
        let record_path = applied_record_path(id)?;
        let layout = self.require_layout()?;
        with_mutation_session(self.storage(), &layout, "profile record read", |session| {
            let Some(image) = capture_or_retry(session.capture(CaptureSpec::phase_one(
                [record_path.clone()],
                RECORD_CAPTURE_BUDGET,
            )?))?
            else {
                return Ok(SessionStep::Retry);
            };
            Ok(SessionStep::Done(read_applied_record(
                &image,
                &record_path,
                id,
            )?))
        })
    }
}

/// Bounds for the applied-record captures.
///
/// One record per applied profile, each a small JSON document directly under
/// `.jit/profiles/`, plus the listing that names them.
const RECORD_CAPTURE_BUDGET: CaptureBudget = CaptureBudget {
    max_paths: 256,
    max_listings: 1,
    max_bytes: 4 * 1024 * 1024,
    max_depth: 4,
};

/// Canonical applied-profile provenance path for one profile id.
///
/// Application publishes the record here and validation reads it back from the
/// same construction, so the record's own name identifies the profile it
/// records.
pub(super) fn applied_record_path(id: &str) -> Result<VirtualPath> {
    VirtualPath::data(format!("profiles/{id}.json")).map_err(Into::into)
}

/// The profile id an occupant of `.jit/profiles/` names, or `None` when the
/// name is not a provenance-record name.
pub(super) fn record_name_profile_id(name: &str) -> Option<&str> {
    name.strip_suffix(".json").filter(|id| !id.is_empty())
}

/// Read this repository's applied-profile records through a held session.
///
/// The `.jit/profiles/` listing states which packages the repository carries,
/// so the records it names are read in the same capture that re-reads that
/// listing: the returned image holds every named record's bytes, and the ids
/// are the profiles those records name.
///
/// `Ok(None)` reports a record set that moved while it was being read — a
/// concurrent application or removal — which the caller retries rather than
/// answering over an inventory no single repository state ever held.
///
/// Enumeration and derived-state repair both begin here, because a record is
/// what names the location its package is read from: neither can resolve a
/// package before reading the records.
pub(super) fn capture_applied_records(
    session: &mut (dyn RepositoryMutationSession + '_),
    profiles_dir: &VirtualPath,
) -> Result<Option<(RepositoryImage, BTreeSet<String>)>> {
    let mut spec = CaptureSpec::phase_one([], RECORD_CAPTURE_BUDGET)?;
    spec.discover_listing(profiles_dir.clone())?;
    let Some(listed) = capture_or_retry(session.capture(spec.clone()))? else {
        return Ok(None);
    };
    let recorded = recorded_profile_ids(&listed, profiles_dir)?;
    spec.discover_paths(
        recorded
            .iter()
            .map(|id| applied_record_path(id))
            .collect::<Result<Vec<_>>>()?,
    )?;
    let Some(image) = capture_or_retry(session.capture(spec))? else {
        return Ok(None);
    };
    if recorded_profile_ids(&image, profiles_dir)? != recorded {
        return Ok(None);
    }
    Ok(Some((image, recorded)))
}

/// Profile ids the repository's own applied-profile records name.
///
/// Application writes one record per applied profile at
/// `.jit/profiles/<id>.json`, so the listing of that directory is where the
/// repository states which packages it carries. A child whose name is not a
/// record name is not a record.
pub(super) fn recorded_profile_ids(
    image: &RepositoryImage,
    profiles_dir: &VirtualPath,
) -> Result<BTreeSet<String>> {
    Ok(image
        .listing_fingerprints()
        .get(profiles_dir)
        .ok_or_else(|| anyhow::anyhow!("capture did not list {profiles_dir:?}"))?
        .children()
        .keys()
        .filter_map(|name| record_name_profile_id(name))
        .map(str::to_string)
        .collect())
}

/// Read the package at a location the caller supplied.
///
/// The package there must declare `id`: the record an application writes is
/// named by the package's own identity, so admitting a package declaring
/// something else would apply a profile the caller did not ask for and record
/// it under the name it did not name.
fn supplied_package(location: &Path, id: &str) -> Result<ProfilePackage> {
    let package = ProfilePackage::from_directory(location).map_err(|source| {
        ProfileResolutionError::UnreadableLocation {
            location: location.display().to_string(),
            source,
        }
    })?;
    let found = package.manifest().profile.id.as_str();
    if found == id {
        Ok(package)
    } else {
        Err(ProfileResolutionError::UnexpectedProfileAtLocation {
            location: location.display().to_string(),
            requested: id.to_string(),
            found: found.to_string(),
        }
        .into())
    }
}

/// Read the package one applied-profile record names.
///
/// The record's origin is the whole answer: the worktree-relative location it
/// names is resolved against this repository's worktree root. A failure there
/// names the record, so a repository whose package moved or was deleted reports
/// the record that outlived it rather than an absent profile.
///
/// Validation and repair resolve a recorded profile through this same route,
/// which is why it is reachable from the whole command layer rather than
/// private to the profile commands.
pub(super) fn recorded_package(
    record: &AppliedProfileRecord,
    record_path: &VirtualPath,
    layout: &RepositoryLayout,
) -> Result<ProfilePackage> {
    let ProfileOrigin::Directory(location) = &record.origin;
    ProfilePackage::from_directory(&layout.worktree_root().join(location.as_path())).map_err(
        |source| {
            ProfileResolutionError::UnresolvableRecordedLocation {
                record: record_path.repository_relative(),
                location: location.as_path().display().to_string(),
                source,
            }
            .into()
        },
    )
}

/// Summarize one recorded profile from the package its record resolves to.
///
/// Every reported fact but `applied` comes from the resolved package, so the
/// answer states what the recorded location holds now. `applied` is the
/// comparison between the two: the stored record against the record that
/// package would write today.
///
/// `Ok(None)` reports a record the listing named that the capture no longer
/// holds, which the caller retries rather than reporting without it.
fn recorded_summary(
    image: &RepositoryImage,
    id: &str,
    layout: &RepositoryLayout,
) -> Result<Option<ProfileSummary>> {
    let record_path = applied_record_path(id)?;
    let Some(record) = read_applied_record(image, &record_path, id)? else {
        return Ok(None);
    };
    let package = recorded_package(&record, &record_path, layout)?;
    let metadata = &package.manifest().profile;
    Ok(Some(ProfileSummary {
        id: metadata.id.to_string(),
        version: metadata.version.clone(),
        origin: package_origin(&package, layout)?,
        jit: metadata.jit.clone(),
        applied: record == expected_record(&package, layout)?,
    }))
}

/// The provenance origin a package's own bytes came through.
///
/// The location is taken from the source the package recorded while reading
/// itself, never from a path supplied beside it, so a record names the
/// directory whose bytes it addresses. A directory that resolves outside the
/// worktree is refused rather than recorded: the record is worktree-relative
/// repository state, so a location it cannot express would make re-reading the
/// package depend on machine state. The selected data root is not a worktree
/// location even when it nests inside one, so a package placed under it is
/// refused by the same rule.
pub(super) fn package_origin(
    package: &ProfilePackage,
    layout: &RepositoryLayout,
) -> Result<ProfileOrigin> {
    let ProfilePackageSource::Directory(directory) = package.source();
    layout
        .classify_and_canonicalize(directory)
        .ok()
        .filter(|path| path.root_class() == RepositoryRootClass::Worktree)
        .map(|path| ProfileOrigin::Directory(path.relative().clone()))
        .ok_or_else(|| {
            ProfileApplyError::PackageOutsideWorktree {
                path: directory.display().to_string(),
            }
            .into()
        })
}

/// The expected provenance record for a package read through this repository.
pub(super) fn expected_record(
    package: &ProfilePackage,
    layout: &RepositoryLayout,
) -> Result<AppliedProfileRecord> {
    let metadata = &package.manifest().profile;
    Ok(AppliedProfileRecord::new(
        metadata.id.to_string(),
        metadata.version.clone(),
        package_origin(package, layout)?,
        package.hashes().package.clone(),
        package.hashes().targets.clone(),
    ))
}

/// Convert an immutable package into neutral claims plus provenance metadata.
fn profile_application_input(
    package: &ProfilePackage,
    layout: &RepositoryLayout,
    record_path: VirtualPath,
) -> Result<ProfileApplicationInput> {
    let metadata = &package.manifest().profile;
    Ok(ProfileApplicationInput {
        id: metadata.id.to_string(),
        version: metadata.version.clone(),
        package_hash: package.hashes().package.clone(),
        target_hashes: package.hashes().targets.clone(),
        origin: package_origin(package, layout)?,
        claims: build_profile_claims(package, layout)?,
        record_path,
    })
}

/// Read the captured installed record, or `None` when absent.
fn read_applied_record(
    base: &RepositoryImage,
    record_path: &VirtualPath,
    id: &str,
) -> Result<Option<AppliedProfileRecord>> {
    match base.entry(record_path)? {
        RepositoryEntry::Absent => Ok(None),
        RepositoryEntry::File { bytes, .. } => {
            serde_json::from_slice::<AppliedProfileRecord>(bytes)
                .map(Some)
                .map_err(|_| {
                    ProfileApplyError::InstalledRecordConflict {
                        path: record_path.repository_relative(),
                        id: id.to_string(),
                    }
                    .into()
                })
        }
        _ => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: record_path.repository_relative(),
        }
        .into()),
    }
}

pub(super) fn reject_reserved_application_targets<'a>(
    targets: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    if let Some(path) = targets.into_iter().find(|path| {
        *path == ".jit/events.jsonl"
            || *path == ".jit/profiles"
            || path.starts_with(".jit/profiles/")
            || *path == ".jit/tmp"
            || path.starts_with(".jit/tmp/")
            || *path == ".git"
            || path.starts_with(".git/")
    }) {
        return Err(ProfileApplyError::ReservedApplicationTarget {
            path: path.to_string(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Event;
    use crate::repository_state::{
        Contribution, InitializationError, MapEntryTarget, ProducerError, ProfileConflictOccupant,
        ProfilePackageId, ProfileTargetConflictError, RepositoryStateError, RootRelativePath,
        ScalarTarget, SetStringTarget,
    };
    use crate::storage::{
        discover_repository_layout, IssueStore, RepositoryStateStore, RepositoryStateStoreError,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    struct RecaptureRaceSession {
        inner: Box<dyn RepositoryMutationSession>,
        captures: usize,
        config_path: std::path::PathBuf,
    }

    struct FinalClosureRaceSession {
        inner: Box<dyn RepositoryMutationSession>,
        captures: usize,
        config_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
    }

    impl RepositoryMutationSession for RecaptureRaceSession {
        fn layout(&self) -> &crate::repository_state::RepositoryLayout {
            self.inner.layout()
        }

        fn recovery_report(&self) -> &crate::storage::RecoveryDispatchReport {
            self.inner.recovery_report()
        }

        fn capture(
            &mut self,
            spec: CaptureSpec,
        ) -> std::result::Result<RepositoryImage, RepositoryStateStoreError> {
            self.captures += 1;
            if self.captures == 4 {
                fs::write(&self.config_path, b"not = [valid").unwrap();
            }
            self.inner.capture(spec)
        }

        fn apply(
            &mut self,
            plan: &MaterializationPlan,
        ) -> std::result::Result<crate::storage::RepositoryApplyOutcome, RepositoryStateStoreError>
        {
            self.inner.apply(plan)
        }
    }

    impl RepositoryMutationSession for FinalClosureRaceSession {
        fn layout(&self) -> &crate::repository_state::RepositoryLayout {
            self.inner.layout()
        }

        fn recovery_report(&self) -> &crate::storage::RecoveryDispatchReport {
            self.inner.recovery_report()
        }

        fn capture(
            &mut self,
            spec: CaptureSpec,
        ) -> std::result::Result<RepositoryImage, RepositoryStateStoreError> {
            self.captures += 1;
            if self.captures == 7 {
                let mut config = fs::read_to_string(&self.config_path).unwrap();
                config.push_str(
                    "\n[item_kinds.race]\nsection = \"race\"\nid-pattern = \"R-[0-9]+\"\n\
                     markers = []\nlink-namespaces = []\nscope = \"project\"\n\
                     source-of-truth = \"markdown-first\"\nsource = \"RACE.md\"\n\
                     [projection.race]\nkind = \"race\"\nmode = \"separate-file\"\n\
                     target = \".agents/skills/jit-manage/SKILL.md\"\nstyle = \"id-anchor\"\n",
                );
                fs::write(&self.config_path, config).unwrap();
                fs::write(&self.source_path, "## Race\n\n- **R-1** — recaptured\n").unwrap();
            }
            self.inner.capture(spec)
        }

        fn apply(
            &mut self,
            plan: &MaterializationPlan,
        ) -> std::result::Result<crate::storage::RepositoryApplyOutcome, RepositoryStateStoreError>
        {
            self.inner.apply(plan)
        }
    }

    /// The checked-in fixture tree every case below stages a copy of.
    fn fixture_package_tree() -> std::path::PathBuf {
        crate::test_utils::profile_package_fixture("planner-asset-only")
    }

    /// Worktree-relative location [`fixture`] stages its package at.
    ///
    /// A package is applied from inside the repository it is applied to,
    /// because the applied-profile record names its worktree-relative location.
    const FIXTURE_LOCATION: &str = "packages/planner";

    /// A file-backed executor over a canonically initialized repository carrying
    /// its canonical layout, with the fixture package staged inside its
    /// worktree.
    fn fixture() -> (
        TempDir,
        JsonFileStorage,
        CommandExecutor<JsonFileStorage>,
        ProfilePackage,
    ) {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        let initializer = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        initializer
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        (temp, storage, executor, package)
    }

    /// The fixture package, written into the repository at `relative` and read
    /// back from there.
    fn package_read_from(temp: &TempDir, relative: &str) -> ProfilePackage {
        let root = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &temp.path().join(relative),
        );
        ProfilePackage::from_directory(&root).expect("a valid package tree")
    }

    /// The provenance record a repository stores for the fixture package.
    fn stored_record(temp: &TempDir) -> AppliedProfileRecord {
        record_for(temp, &fixture_id())
    }

    /// Rewrite one authored source of the package tree at `relative`, and read
    /// the package back from there.
    ///
    /// Two copies of one fixture are byte-identical and therefore
    /// indistinguishable in everything a resolution reports, so a test that
    /// asks which copy was read edits one of them first.
    fn repackage(temp: &TempDir, relative: &str, source: &str, bytes: &str) -> ProfilePackage {
        let root = temp.path().join(relative);
        fs::write(root.join(source), bytes).unwrap();
        ProfilePackage::from_directory(&root).expect("a valid package tree")
    }

    /// The fixture package's id, which its manifest declares.
    fn fixture_id() -> String {
        ProfilePackage::from_directory(&fixture_package_tree())
            .expect("the checked-in fixture tree is a valid package")
            .manifest()
            .profile
            .id
            .to_string()
    }

    /// The fixture package tree written at `relative`, rewritten to declare
    /// `id`, an asset target of its own, and a dependency on each of
    /// `dependencies`, and read back from there.
    ///
    /// No package this repository ships declares a dependency, so a composition
    /// scenario is authored here rather than borrowed from one. The asset
    /// target follows the id so two authored packages publish different files
    /// and their applications are distinguishable.
    fn package_declaring(
        temp: &TempDir,
        relative: &str,
        id: &str,
        dependencies: &[&str],
    ) -> ProfilePackage {
        crate::test_utils::write_package_declaring(
            &fixture_package_tree(),
            &temp.path().join(relative),
            id,
            dependencies,
        )
    }

    /// The profile ids this repository's audit log records as applied, in the
    /// order it recorded them.
    fn applied_event_ids(storage: &JsonFileStorage) -> Vec<String> {
        storage
            .read_events()
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                Event::ProfileApplied { profile_id, .. } => Some(profile_id),
                _ => None,
            })
            .collect()
    }

    /// The provenance record this repository stores for `id`.
    fn record_for(temp: &TempDir, id: &str) -> AppliedProfileRecord {
        serde_json::from_slice(
            &fs::read(temp.path().join(format!(".jit/profiles/{id}.json"))).unwrap(),
        )
        .unwrap()
    }

    /// The fixture package tree written at `relative`, rewritten to declare
    /// `id`, to publish `content` at asset target `target`, and to carry
    /// `contributions` verbatim, and read back from there.
    ///
    /// Every package this repository authors publishes targets of its own, so
    /// two packages claiming one target — which is what a conflict between
    /// packages is — are authored from the same fixture tree the other
    /// scenarios read (`@/invariant/shared-test-contracts`).
    fn authored_package(
        temp: &TempDir,
        relative: &str,
        id: &str,
        target: &str,
        content: &str,
        contributions: &str,
    ) -> ProfilePackage {
        let tree = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &temp.path().join(relative),
        );
        let manifest_path = tree.join(crate::profile::MANIFEST_FILE_NAME);
        let authored = fs::read_to_string(&manifest_path).expect("read the package manifest");
        let source = ProfilePackage::parse_manifest(authored.as_bytes())
            .expect("the source package manifest parses");
        let asset = source
            .assets
            .first()
            .expect("the fixture package declares an asset");
        fs::write(tree.join(&asset.source), content).expect("write the package asset content");
        let rewritten = authored
            .replace(
                &format!("id = \"{}\"", source.profile.id),
                &format!("id = \"{id}\""),
            )
            .replace(
                &format!("target = \"{}\"", asset.target),
                &format!("target = \"{target}\""),
            );
        fs::write(&manifest_path, format!("{rewritten}{contributions}"))
            .expect("write the rewritten package manifest");
        ProfilePackage::from_directory(&tree).expect("a valid package tree")
    }

    /// A package declaring `id` and publishing `content` at asset target
    /// `target`.
    fn package_publishing(
        temp: &TempDir,
        relative: &str,
        id: &str,
        target: &str,
        content: &str,
    ) -> ProfilePackage {
        authored_package(temp, relative, id, target, content, "")
    }

    /// A package declaring `id`, publishing an asset target of its own, and
    /// contributing the label namespace `namespace` described as `description`.
    fn package_contributing(
        temp: &TempDir,
        relative: &str,
        id: &str,
        namespace: &str,
        description: &str,
    ) -> ProfilePackage {
        authored_package(
            temp,
            relative,
            id,
            &format!("docs/{id}.txt"),
            id,
            &namespace_contribution(namespace, description),
        )
    }

    /// A manifest fragment declaring the label namespace `namespace` as
    /// `description`.
    fn namespace_contribution(namespace: &str, description: &str) -> String {
        format!(
            "\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\n\
             identity = \"{namespace}\"\n\
             value = {{ description = \"{description}\", unique = false }}\n"
        )
    }

    /// The asset-target conflict `error` reports, whichever variant carries it.
    ///
    /// Composition raises one conflict value; the profile-application producer
    /// re-wraps it under initialization on its way out. A test asserting what a
    /// conflict reports reads that value rather than the variant that carried
    /// it (`@/invariant/semantic-test-assertions`).
    fn target_conflict(error: &anyhow::Error) -> &ProfileTargetConflictError {
        match error.downcast_ref::<RepositoryStateError>() {
            Some(
                RepositoryStateError::ProfileTargetConflict(conflict)
                | RepositoryStateError::Initialization(InitializationError::ProfileTargetConflict(
                    conflict,
                )),
            ) => conflict,
            _ => panic!("a colliding asset target fails as a target conflict: {error:#}"),
        }
    }

    /// Store `record` as this repository's applied-profile record for its id.
    fn store_record(temp: &TempDir, record: &AppliedProfileRecord) {
        fs::create_dir_all(temp.path().join(".jit/profiles")).unwrap();
        fs::write(
            temp.path()
                .join(format!(".jit/profiles/{}.json", record.id)),
            record.to_bytes().unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_resolve_profile_package_reads_the_package_a_supplied_location_holds() {
        let (temp, _storage, executor, _package) = fixture();
        let supplied = package_read_from(&temp, "vendor/supplied");

        let resolved = executor
            .resolve_profile_package(&fixture_id(), Some(&temp.path().join("vendor/supplied")))
            .unwrap();

        assert_eq!(resolved.hashes(), supplied.hashes());
        assert_eq!(
            package_origin(&resolved, &executor.require_layout().unwrap()).unwrap(),
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/supplied").unwrap())
        );
    }

    #[test]
    fn test_resolve_profile_package_prefers_a_supplied_location_over_the_recorded_one() {
        let (temp, _storage, executor, _package) = fixture();
        let recorded = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&recorded).unwrap();
        // A second copy the record does not name, distinguishable from the
        // recorded one by its bytes alone.
        package_read_from(&temp, "vendor/supplied");
        let supplied = repackage(
            &temp,
            "vendor/supplied",
            "assets/profile.txt",
            "supplied bytes\n",
        );
        assert_ne!(supplied.hashes(), recorded.hashes());

        let resolved = executor
            .resolve_profile_package(&fixture_id(), Some(&temp.path().join("vendor/supplied")))
            .unwrap();

        assert_eq!(resolved.hashes(), supplied.hashes());
    }

    #[test]
    fn test_resolve_profile_package_reads_the_location_the_record_names() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();

        // Nothing names the location at the call site: the repository's own
        // record is the only statement of where the bytes are. Rewriting the
        // package there afterwards proves the answer is a read of that
        // location rather than a replay of what the record stored.
        let rewritten = repackage(
            &temp,
            "vendor/recorded",
            "assets/profile.txt",
            "rewritten bytes\n",
        );
        assert_ne!(rewritten.hashes(), applied.hashes());

        let resolved = executor
            .resolve_profile_package(&fixture_id(), None)
            .unwrap();

        assert_eq!(resolved.hashes(), rewritten.hashes());
    }

    #[test]
    fn test_resolve_profile_package_reads_the_recorded_package_not_another_declaring_that_id() {
        let (temp, _storage, executor, _package) = fixture();
        let (_workspace, authored) = crate::test_utils::temporary_repository_package("jit-dogfood");
        let id = authored.manifest().profile.id.to_string();

        // A directory package declaring the id this repository also authors, so
        // nothing but the record decides which of the two answers.
        package_read_from(&temp, "vendor/dogfood");
        let manifest = fs::read_to_string(temp.path().join("vendor/dogfood/manifest.toml"))
            .unwrap()
            .replace(
                &format!("id = \"{}\"", fixture_id()),
                &format!("id = \"{id}\""),
            );
        let recorded = repackage(&temp, "vendor/dogfood", "manifest.toml", &manifest);
        assert_eq!(recorded.manifest().profile.id.as_str(), id);
        assert_ne!(recorded.hashes(), authored.hashes());
        store_record(
            &temp,
            &AppliedProfileRecord::new(
                id.clone(),
                recorded.manifest().profile.version.clone(),
                ProfileOrigin::Directory(RootRelativePath::parse("vendor/dogfood").unwrap()),
                recorded.hashes().package.clone(),
                recorded.hashes().targets.clone(),
            ),
        );

        let resolved = executor.resolve_profile_package(&id, None).unwrap();

        assert_eq!(resolved.hashes(), recorded.hashes());
    }

    #[test]
    fn test_resolve_profile_package_reports_not_found_without_a_location_or_a_record() {
        let (temp, _storage, executor, _package) = fixture();
        assert!(!temp.path().join(".jit/profiles").exists());
        // The ids this repository authors are named beside an id it does not,
        // because a resolution with no supplied or recorded package must
        // report profile-not-found for each of them.
        for id in ["jit-default", "jit-dogfood", "no-such-profile"] {
            let error = executor.resolve_profile_package(id, None).unwrap_err();

            assert!(
                error
                    .downcast_ref::<crate::errors::NotFoundError>()
                    .is_some(),
                "resolving '{id}' with no location and no record must report \
                 profile-not-found: {error:#}"
            );
        }
    }

    #[test]
    fn test_apply_profile_package_resolves_a_declared_dependency_beside_the_declaring_package() {
        let (temp, _storage, executor, _fixture) = fixture();
        let config_path = temp.path().join(".jit/config.toml");
        let scaffolded = fs::read_to_string(&config_path).unwrap();
        let mut bare = scaffolded.parse::<toml_edit::DocumentMut>().unwrap();
        bare.as_table_mut()
            .retain(|key, _| matches!(key, "version" | "project"));
        fs::write(&config_path, bare.to_string()).unwrap();

        crate::test_utils::assemble_repository_package(
            "jit-default",
            &temp.path().join("vendor/jit-default"),
        )
        .expect("this repository's jit-default package assembles");
        let workflow = package_declaring(&temp, "vendor/workflow", "workflow", &["jit-default"]);
        let applied = executor.apply_profile_package(&workflow).unwrap();

        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["jit-default", "workflow"]
        );
        assert_eq!(
            record_for(&temp, "jit-default").origin,
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/jit-default").unwrap())
        );
    }

    #[test]
    fn test_apply_workflow_profile_to_bare_repository_contributes_hierarchy_rules() {
        let (temp, _storage, executor, _fixture) = fixture();
        let config_path = temp.path().join(".jit/config.toml");
        let scaffolded = fs::read_to_string(&config_path).unwrap();
        let mut bare = scaffolded.parse::<toml_edit::DocumentMut>().unwrap();
        bare.as_table_mut()
            .retain(|key, _| matches!(key, "version" | "project"));
        fs::write(&config_path, bare.to_string()).unwrap();
        let rules_path = temp.path().join(".jit/rules.toml");
        let mut rules = fs::read_to_string(&rules_path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        rules["rules"]
            .as_array_of_tables_mut()
            .unwrap()
            .retain(|rule| {
                !matches!(
                    rule["name"].as_str(),
                    Some("orphan-leaf" | "strategic-consistency")
                )
            });
        fs::write(&rules_path, rules.to_string()).unwrap();
        // The workflow package and the package it depends on, side by side
        // inside the worktree, which is where a declared dependency is
        // looked for.
        shipped_package_in(temp.path(), "jit-default");
        let workflow = shipped_package_in(temp.path(), "jit-dogfood");

        executor
            .apply_profile_package(&workflow)
            .unwrap_or_else(|error| panic!("{error:#}"));

        let rules: toml::Value =
            toml::from_str(&fs::read_to_string(temp.path().join(".jit/rules.toml")).unwrap())
                .unwrap();
        let rules = rules["rules"].as_array().unwrap();
        for expected in ["orphan-leaf", "strategic-consistency"] {
            assert!(
                rules.iter().any(|rule| {
                    rule["name"].as_str() == Some(expected)
                        && rule["origin"].as_str() == Some("jit-dogfood")
                }),
                "workflow profile did not contribute {expected}: {rules:?}"
            );
        }
    }

    #[test]
    fn test_resolve_profile_package_reports_a_recorded_location_that_no_longer_resolves() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();
        fs::remove_dir_all(temp.path().join("vendor/recorded")).unwrap();

        let error = executor
            .resolve_profile_package(&fixture_id(), None)
            .unwrap_err();

        // The record outlived its package: naming the record and the location
        // is what distinguishes that from a profile this repository never
        // applied, which is what an absent-profile answer would claim.
        let message = format!("{error:#}");
        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableRecordedLocation { record, location, .. })
                    if record == ".jit/profiles/planner-asset-only.json"
                        && location == "vendor/recorded"
            ),
            "{message}"
        );
        assert!(
            error
                .downcast_ref::<crate::errors::NotFoundError>()
                .is_none(),
            "{message}"
        );
    }

    #[test]
    fn test_resolve_profile_package_refuses_a_supplied_location_declaring_another_profile() {
        let (temp, _storage, executor, _package) = fixture();
        package_read_from(&temp, "vendor/supplied");

        let error = executor
            .resolve_profile_package("jit-dogfood", Some(&temp.path().join("vendor/supplied")))
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnexpectedProfileAtLocation { requested, found, .. })
                    if requested == "jit-dogfood" && *found == fixture_id()
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_resolve_profile_package_reports_a_supplied_location_holding_no_package() {
        let (temp, _storage, executor, _package) = fixture();

        let error = executor
            .resolve_profile_package(&fixture_id(), Some(&temp.path().join("vendor/absent")))
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnreadableLocation { location, .. })
                    if location.ends_with("vendor/absent")
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_list_recorded_profiles_names_no_profile_until_a_record_does() {
        let (temp, _storage, executor, _package) = fixture();

        let before = executor.list_recorded_profiles().unwrap();
        assert_eq!(before.count, 0);
        assert!(before.profiles.is_empty());

        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();

        let after = executor.list_recorded_profiles().unwrap();
        assert_eq!(after.count, after.profiles.len());
        assert_eq!(
            after
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec![fixture_id().as_str()]
        );
        assert_eq!(
            after.profiles[0].origin,
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/recorded").unwrap())
        );
        assert!(after.profiles[0].applied);
    }

    #[test]
    fn test_list_recorded_profiles_resolves_each_record_from_the_location_it_names() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();
        let recorded_version = stored_record(&temp).version;

        // The package at the recorded location moves on without the repository.
        // What enumeration reports is what reading that location now yields, so
        // the new version is reported and the stored record no longer describes
        // it.
        let manifest = fs::read_to_string(temp.path().join("vendor/recorded/manifest.toml"))
            .unwrap()
            .replace(
                &format!("version = \"{recorded_version}\""),
                "version = \"2.0.0\"",
            );
        let rewritten = repackage(&temp, "vendor/recorded", "manifest.toml", &manifest);
        assert_ne!(rewritten.manifest().profile.version, recorded_version);

        let listed = executor.list_recorded_profiles().unwrap();

        assert_eq!(listed.count, 1);
        assert_eq!(
            listed.profiles[0].version,
            rewritten.manifest().profile.version
        );
        assert!(
            !listed.profiles[0].applied,
            "a record that no longer describes the package at its location is not applied state"
        );
    }

    #[test]
    fn test_list_recorded_profiles_reports_a_recorded_location_that_no_longer_resolves() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();
        fs::remove_dir_all(temp.path().join("vendor/recorded")).unwrap();

        let error = executor.list_recorded_profiles().unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableRecordedLocation { record, location, .. })
                    if record == ".jit/profiles/planner-asset-only.json"
                        && location == "vendor/recorded"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_apply_profile_package_records_the_worktree_location_it_read_the_package_from() {
        let (temp, _storage, executor, _package) = fixture();
        let package = package_read_from(&temp, "vendor/profiles/planner");

        executor.apply_profile_package(&package).unwrap();

        // The stored location resolves, from the worktree root alone, back to
        // the directory whose bytes were applied — which is the whole point of
        // recording it.
        let ProfileOrigin::Directory(location) = stored_record(&temp).origin;
        assert_eq!(
            ProfilePackage::from_directory(&temp.path().join(location.as_path()))
                .expect("the recorded location names a readable package")
                .hashes(),
            package.hashes()
        );
    }

    #[test]
    fn test_apply_profile_package_records_the_directory_read_rather_than_a_peer_copy() {
        // Two directories inside the worktree hold byte-identical packages.
        // Nothing but the directory a package was read through distinguishes
        // them, so the recorded location must follow that and only that.
        let (first_temp, _first_storage, first_executor, _package) = fixture();
        let (second_temp, _second_storage, second_executor, _package) = fixture();
        let first = package_read_from(&first_temp, "vendor/first");
        let second = package_read_from(&second_temp, "packages/second/tree");
        assert_eq!(first.hashes(), second.hashes());

        first_executor.apply_profile_package(&first).unwrap();
        second_executor.apply_profile_package(&second).unwrap();

        assert_eq!(
            stored_record(&first_temp).origin,
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/first").unwrap())
        );
        assert_eq!(
            stored_record(&second_temp).origin,
            ProfileOrigin::Directory(RootRelativePath::parse("packages/second/tree").unwrap())
        );
    }

    #[test]
    fn test_apply_profile_package_refuses_a_package_read_from_outside_the_worktree() {
        let (temp, storage, executor, _package) = fixture();
        let elsewhere = TempDir::new().unwrap();
        let root = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &elsewhere.path().join("pkg"),
        );
        let package = ProfilePackage::from_directory(&root).expect("a valid package tree");

        let error = executor.apply_profile_package(&package).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::PackageOutsideWorktree { path })
                    if Path::new(path) == fs::canonicalize(&root).unwrap()
            ),
            "the refusal must name the package directory: {error:#}"
        );
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_refuses_a_package_read_from_the_data_root() {
        // The selected data root is not a worktree location, so a package
        // placed under it has no worktree-relative location to record.
        let (temp, storage, executor, _package) = fixture();
        let package = package_read_from(&temp, ".jit/vendored");

        let error = executor.apply_profile_package(&package).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::PackageOutsideWorktree { path })
                    if Path::new(path) == temp.path().join(".jit/vendored")
            ),
            "the refusal must name the package directory: {error:#}"
        );
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    /// This repository's generic vocabulary package, assembled from its
    /// checkout, with the directory holding the assembled tree.
    fn assembled_default_package() -> (TempDir, ProfilePackage) {
        crate::test_utils::temporary_repository_package("jit-default")
    }

    /// This repository's `id` package, assembled from its checkout into
    /// `worktree`, which is where a package has to sit to be applied to the
    /// repository rooted there.
    fn shipped_package_in(worktree: &Path, id: &str) -> ProfilePackage {
        crate::test_utils::assemble_repository_package(id, &worktree.join("profiles").join(id))
            .expect("this repository's package assembles")
    }

    /// The project name every repository built by
    /// [`bare_repository_beside_shipped_packages`] carries, so two of them
    /// differ in what was applied to them rather than in where they sit.
    const COMPOSITION_PROJECT_NAME: &str = "composition";

    /// A repository declaring nothing, with the two shipped package trees
    /// beside each other inside its worktree.
    ///
    /// The configuration is reduced to what an initialization derives from the
    /// repository itself — the schema version and the project name — so what an
    /// application writes afterwards is what the applied packages declare and
    /// nothing that survived the reduction. The package directories sit side by
    /// side because that is where an adopter puts an obtained set, and where a
    /// package's declared dependency is looked for.
    ///
    /// Answers with the repository, an executor over it, and the `jit-default`
    /// and `jit-dogfood` packages read back from the trees inside it.
    fn bare_repository_beside_shipped_packages() -> (
        TempDir,
        CommandExecutor<JsonFileStorage>,
        ProfilePackage,
        ProfilePackage,
    ) {
        let (temp, storage, _executor, _fixture_package) = fixture();
        let config_path = temp.path().join(".jit/config.toml");
        let mut bare = fs::read_to_string(&config_path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        bare.as_table_mut()
            .retain(|key, _| matches!(key, "version" | "project"));
        bare["project"]["name"] = toml_edit::value(COMPOSITION_PROJECT_NAME);
        fs::write(&config_path, bare.to_string()).unwrap();

        let default = shipped_package_in(temp.path(), "jit-default");
        let dogfood = shipped_package_in(temp.path(), "jit-dogfood");
        // Re-discovered over the reduced configuration, as a fresh process
        // would read it.
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        (temp, executor, default, dogfood)
    }

    /// The configuration a repository carries, parsed rather than read as
    /// bytes: key order and formatting are not what a comparison is about.
    fn published_configuration(temp: &TempDir) -> serde_json::Value {
        toml_edit::de::from_str(&fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap())
            .unwrap()
    }

    /// The map identities a package declares, each with the configuration
    /// table it declares it in and the value it declares, in manifest order.
    fn declared_map_entries(
        package: &ProfilePackage,
    ) -> Vec<(MapEntryTarget, &str, &serde_json::Value)> {
        package
            .manifest()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::MapEntry {
                    target,
                    identity,
                    value,
                } => Some((*target, identity.as_str(), value)),
                _ => None,
            })
            .collect()
    }

    /// The repository-relative file a project-scope item-kind declaration reads
    /// its items from, or `None` for a kind that needs no source.
    fn item_kind_source(declaration: &serde_json::Value) -> Option<&str> {
        let source = declaration.get("source")?;
        source
            .get("toml")
            .and_then(serde_json::Value::as_str)
            .or_else(|| source.as_str())
    }

    /// The item-kind declarations one package contributes, by kind name.
    fn declared_item_kinds(package: &ProfilePackage) -> BTreeMap<&str, &serde_json::Value> {
        declared_map_entries(package)
            .into_iter()
            .filter(|(target, ..)| *target == MapEntryTarget::ItemKinds)
            .map(|(_, identity, value)| (identity, value))
            .collect()
    }

    #[test]
    fn test_from_directory_reads_jit_default_with_a_stable_content_address() {
        let (_workspace, package) = assembled_default_package();
        let (_repeated_workspace, repeated) = assembled_default_package();
        let (_workflow_workspace, workflow) =
            crate::test_utils::temporary_repository_package("jit-dogfood");

        // The address is over content: re-reading the same declarations
        // reproduces it, and the workflow package next to it does not share it.
        assert_eq!(package.hashes(), repeated.hashes());
        assert_ne!(package.hashes().package, workflow.hashes().package);
        assert!(!package.hashes().package.is_empty());
        // One repository target, because every declaration writes the
        // configuration registry and the package publishes no file.
        assert_eq!(
            package.hashes().targets.keys().collect::<Vec<_>>(),
            vec![&".jit/config.toml".to_string()]
        );
    }

    #[test]
    fn test_jit_default_carries_no_asset_and_no_workflow_registry_content() {
        let (_workspace, package) = assembled_default_package();
        let manifest = package.manifest();

        // The manifest is the whole package: no file is published with it, so
        // no gate prompt, checker, or template body arrives either.
        assert_eq!(package.file_count(), 1);
        assert!(manifest.assets.is_empty());
        assert!(manifest.regions.is_empty());
        // Gates, rules, and templates are the registries a sequencing opinion
        // reaches a repository through; every declaration here writes the
        // configuration registry instead.
        assert!(manifest
            .contributions
            .iter()
            .all(|contribution| contribution.registry_path() == ".jit/config.toml"));
    }

    #[test]
    fn test_jit_default_declares_every_kind_an_initialized_repository_can_carry() {
        let (temp, _storage, _executor, _fixture_package) = fixture();
        let (_workspace, package) = assembled_default_package();
        let kinds = declared_item_kinds(&package);

        assert!(
            kinds.values().any(|kind| item_kind_source(kind).is_some()),
            "the rule is vacuous unless some declared kind names a source"
        );
        for (name, declaration) in &kinds {
            let Some(source) = item_kind_source(declaration) else {
                continue;
            };
            // The package publishes no file at all, so every source it names is
            // one initialization already wrote.
            assert!(
                package
                    .manifest()
                    .assets
                    .iter()
                    .all(|asset| asset.target != source),
                "package cannot provide '{source}' for kind '{name}'"
            );
            assert!(
                temp.path().join(source).is_file(),
                "kind '{name}' names source '{source}', which an initialized repository does not carry"
            );
        }

        // Nothing is left out: every registry initialization writes and can hold
        // addressable items is claimed by one of these kinds.
        let declared_sources: BTreeSet<&str> = kinds
            .values()
            .filter_map(|kind| item_kind_source(kind))
            .collect();
        for registry in [".jit/invariants.toml", ".jit/rules.toml", ".jit/gates.toml"] {
            assert!(
                temp.path().join(registry).is_file(),
                "initialization must carry {registry}"
            );
            assert!(
                declared_sources.contains(registry),
                "no declared kind reads {registry}"
            );
        }
    }

    #[test]
    fn test_apply_profile_package_repairs_a_generated_schema_outside_the_package_targets() {
        let (temp, _storage, executor, _fixture_package) = fixture();
        let package = shipped_package_in(temp.path(), "jit-default");
        // The package writes the configuration, so applying it re-derives the
        // default rules and the schemas they reference — targets it names none
        // of, and whose drift it therefore has to reach without being told.
        assert!(package
            .hashes()
            .targets
            .keys()
            .all(|target| !target.starts_with(".jit/schemas/")));
        let schemas = temp.path().join(".jit/schemas");
        let grammar = schemas.join("default-label-format.json");
        let generated = fs::read(&grammar).unwrap();
        fs::remove_file(&grammar).unwrap();

        let applied = executor.apply_profile_package(&package).unwrap();

        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        // The projection the repository already had is restored, and the
        // registry the package declared brings its own along, though the
        // package names neither as a target.
        assert_eq!(fs::read(&grammar).unwrap(), generated);
        assert!(schemas.join("default-namespace-registry.json").is_file());
    }

    #[test]
    fn test_apply_profile_package_jit_default_writes_every_declaration_it_contributes() {
        // An initialization writes the structural minimum, so what the applied
        // package declares is the whole of the repository's vocabulary. Each
        // assertion below reads the manifest rather than a copy of its values,
        // so the repository is compared against the package's own declaration.
        let (temp, storage, _executor, _fixture_package) = fixture();
        let config_path = temp.path().join(".jit/config.toml");
        let initialized: serde_json::Value =
            toml_edit::de::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let table_names = |config: &serde_json::Value| {
            config
                .as_object()
                .expect("configuration is a table")
                .keys()
                .cloned()
                .collect::<BTreeSet<String>>()
        };
        assert_eq!(
            table_names(&initialized),
            ["project", "version"]
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
            "an initialization derives only these from the repository"
        );

        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        let package = shipped_package_in(temp.path(), "jit-default");
        let applied = executor.apply_profile_package(&package).unwrap();
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );

        let produced: serde_json::Value =
            toml_edit::de::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            table_names(&produced),
            [
                "documentation",
                "item_kinds",
                "namespaces",
                "project",
                "type_hierarchy",
                "validation",
                "version",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>(),
            "the tables the package's contributions target, and nothing else"
        );

        // Every declaration the manifest carries reaches the table it targets,
        // under the identity and value the manifest gave it.
        package
            .manifest()
            .contributions
            .iter()
            .for_each(|contribution| match contribution {
                Contribution::MapEntry {
                    target,
                    identity,
                    value,
                } => {
                    let (table, key) = match target {
                        MapEntryTarget::TypeHierarchyTypes => ("type_hierarchy", "types"),
                        MapEntryTarget::LabelAssociations => {
                            ("type_hierarchy", "label_associations")
                        }
                        MapEntryTarget::Namespaces => ("namespaces", ""),
                        MapEntryTarget::ItemKinds => ("item_kinds", ""),
                    };
                    let entries = if key.is_empty() {
                        &produced[table]
                    } else {
                        &produced[table][key]
                    };
                    assert_eq!(
                        &entries[identity], value,
                        "[{table}] carries the declared {identity}"
                    );
                }
                Contribution::Scalar { target, value } => {
                    let (table, key) = match target {
                        ScalarTarget::DocumentationDevelopmentRoot => {
                            ("documentation", "development_root")
                        }
                        ScalarTarget::DocumentationArchiveRoot => ("documentation", "archive_root"),
                        ScalarTarget::ValidationStrictness => ("validation", "strictness"),
                        ScalarTarget::ValidationDefaultType => ("validation", "default_type"),
                    };
                    assert_eq!(
                        produced[table][key].as_str(),
                        Some(value.as_str()),
                        "[{table}].{key} carries the declared value"
                    );
                }
                Contribution::SetString { target, value } => {
                    let (table, key) = match target {
                        SetStringTarget::StrategicTypes => ("type_hierarchy", "strategic_types"),
                        SetStringTarget::DocumentationManagedPaths => {
                            ("documentation", "managed_paths")
                        }
                        SetStringTarget::DocumentationPermanentPaths => {
                            ("documentation", "permanent_paths")
                        }
                        SetStringTarget::DocumentationIssueScopedAreas => {
                            ("documentation", "issue_scoped_areas")
                        }
                    };
                    assert!(
                        produced[table][key]
                            .as_array()
                            .is_some_and(|declared| declared
                                .iter()
                                .any(|entry| entry.as_str() == Some(value.as_str()))),
                        "[{table}].{key} carries the declared {value}"
                    );
                }
                Contribution::KeyedArray { .. } | Contribution::Projection { .. } => {
                    panic!("the default package contributes no registry entry or projection")
                }
            });
    }

    #[test]
    fn test_jit_dogfood_declares_default_dependency_and_only_additive_contributions() {
        let (_default_workspace, default) = assembled_default_package();
        let (_workflow_workspace, dogfood) =
            crate::test_utils::temporary_repository_package("jit-dogfood");

        assert_eq!(
            dogfood
                .manifest()
                .dependencies
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["jit-default"]
        );

        // A type name, a namespace and an item kind are each a map identity in
        // one of these tables, so what the two packages declare in them is what
        // they may not both declare.
        let declared_vocabulary = |package: &ProfilePackage| {
            declared_map_entries(package)
                .into_iter()
                .filter(|(target, ..)| {
                    matches!(
                        target,
                        MapEntryTarget::TypeHierarchyTypes
                            | MapEntryTarget::Namespaces
                            | MapEntryTarget::ItemKinds
                    )
                })
                .map(|(target, identity, _)| format!("{target:?}:{identity}"))
                .collect::<BTreeSet<_>>()
        };
        let default_vocabulary = declared_vocabulary(&default);
        let dogfood_vocabulary = declared_vocabulary(&dogfood);

        assert!(
            !dogfood_vocabulary.is_empty(),
            "the disjointness is vacuous unless the workflow package declares vocabulary"
        );
        assert!(
            default_vocabulary.is_disjoint(&dogfood_vocabulary),
            "the workflow package must not restate default vocabulary"
        );
        assert!(
            declared_map_entries(&dogfood)
                .iter()
                .all(|(target, ..)| *target != MapEntryTarget::ItemKinds),
            "the workflow package must not declare item kinds"
        );
    }

    #[test]
    fn test_apply_profile_package_jit_dogfood_composes_what_applying_both_packages_composes() {
        // Two bare repositories, one per route: the explicit one has both
        // packages applied to it in turn, and the resolved one names only the
        // workflow package and receives the default as its declared
        // dependency. They start identical, so the configurations they end with
        // may differ only by the route that reached them.
        let (explicit_temp, explicit_executor, explicit_default, explicit_dogfood) =
            bare_repository_beside_shipped_packages();
        let (resolved_temp, resolved_executor, _, resolved_dogfood) =
            bare_repository_beside_shipped_packages();

        let explicit_default_result = explicit_executor.apply_profile_package(&explicit_default);
        let explicit_dogfood_result = explicit_executor.apply_profile_package(&explicit_dogfood);
        let resolved_result = resolved_executor.apply_profile_package(&resolved_dogfood);

        // A contribution restating an identity under another value is refused
        // as a conflict, so an application that answers at all reported none.
        let conflicted = [
            ("the default package by itself", &explicit_default_result),
            ("the workflow package over it", &explicit_dogfood_result),
            ("the workflow package alone", &resolved_result),
        ]
        .into_iter()
        .filter_map(|(route, result)| {
            result
                .as_ref()
                .err()
                .map(|error| format!("{route}: {error:#}"))
        })
        .collect::<Vec<_>>();
        assert_eq!(
            conflicted,
            Vec::<String>::new(),
            "each entry is a route whose application reported a conflict"
        );
        let explicit_default_result = explicit_default_result.unwrap();
        let explicit_dogfood_result = explicit_dogfood_result.unwrap();
        let resolved_result = resolved_result.unwrap();

        // The two routes reach the composition differently. The explicit one
        // applies the dependency in its own right, so the workflow package's
        // own closure then finds it already applied; the resolved one applies
        // both under the single application of the package naming the other.
        let published_by = |result: &ProfileComposedApplyResult| {
            result
                .profiles
                .iter()
                .map(|profile| (profile.id.clone(), profile.status))
                .collect::<Vec<_>>()
        };
        let default_id = explicit_default.manifest().profile.id.to_string();
        let dogfood_id = explicit_dogfood.manifest().profile.id.to_string();
        assert_eq!(
            published_by(&explicit_default_result),
            vec![(default_id.clone(), ProfileApplicationStatus::Applied)]
        );
        assert_eq!(
            published_by(&explicit_dogfood_result),
            vec![
                (default_id.clone(), ProfileApplicationStatus::Unchanged),
                (dogfood_id.clone(), ProfileApplicationStatus::Applied),
            ]
        );
        assert_eq!(
            published_by(&resolved_result),
            vec![
                (default_id, ProfileApplicationStatus::Applied),
                (dogfood_id, ProfileApplicationStatus::Applied),
            ]
        );

        assert_eq!(
            published_configuration(&explicit_temp),
            published_configuration(&resolved_temp),
            "naming the workflow package alone must produce what applying both packages produces"
        );

        // Not two repositories agreeing on nothing: every identity either
        // package declares reached the configuration the comparison is over,
        // carrying the value that package declares for it.
        let composed = published_configuration(&resolved_temp);
        let composed_table = |target: MapEntryTarget| match target {
            MapEntryTarget::TypeHierarchyTypes => &composed["type_hierarchy"]["types"],
            MapEntryTarget::LabelAssociations => &composed["type_hierarchy"]["label_associations"],
            MapEntryTarget::Namespaces => &composed["namespaces"],
            MapEntryTarget::ItemKinds => &composed["item_kinds"],
        };
        let divergent = [&explicit_default, &resolved_dogfood]
            .into_iter()
            .flat_map(declared_map_entries)
            .filter(|(target, identity, value)| {
                composed_table(*target).get(identity) != Some(*value)
            })
            .map(|(target, identity, _)| format!("{target:?}:{identity}"))
            .collect::<Vec<_>>();
        assert_eq!(
            divergent,
            Vec::<String>::new(),
            "each entry is a declared identity the composition lacks or carries another value for"
        );
    }

    #[test]
    fn test_jit_default_states_the_coordination_guidance_without_its_engine_defaults() {
        let (_workspace, package) = assembled_default_package();
        let manifest = std::str::from_utf8(
            package
                .source_bytes(crate::profile::MANIFEST_FILE_NAME)
                .expect("the package carries its manifest"),
        )
        .expect("the packaged manifest is UTF-8");

        // The scaffold renders two engine constants into its coordination
        // guidance. A package is bytes and projects nothing, so the packaged
        // guidance names those keys and leaves their values to the reference
        // that does project them.
        for key in ["default_ttl_secs", "stale_threshold_secs"] {
            let stated: Vec<&str> = manifest.lines().filter(|line| line.contains(key)).collect();
            assert!(!stated.is_empty(), "packaged guidance must name '{key}'");
            assert!(
                stated
                    .iter()
                    .all(|line| !line.contains(|character: char| character.is_ascii_digit())),
                "packaged guidance restates '{key}' with a value: {stated:?}"
            );
        }
    }

    #[test]
    fn test_profile_application_commits_targets_record_event_and_exact_no_op() {
        let (temp, storage, executor, package) = fixture();

        let applied = executor.apply_profile_package(&package).unwrap();
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            package.source_bytes("assets/profile.txt").unwrap()
        );
        let record: AppliedProfileRecord = serde_json::from_slice(
            &fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(record.id, "planner-asset-only");
        assert_eq!(
            record.origin,
            ProfileOrigin::Directory(RootRelativePath::parse(FIXTURE_LOCATION).unwrap())
        );
        assert_eq!(storage.read_events().unwrap().len(), 1);

        let before = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let compact_record = serde_json::to_vec(&record).unwrap();
        fs::write(
            temp.path().join(".jit/profiles/planner-asset-only.json"),
            &compact_record,
        )
        .unwrap();
        let unchanged = executor.apply_profile_package(&package).unwrap();
        assert_eq!(
            unchanged.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            before
        );
        assert_eq!(
            fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap(),
            compact_record
        );
    }

    #[test]
    fn test_profile_preparation_returns_one_complete_canonical_plan() {
        let (_temp, _storage, executor, package) = fixture();
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let context = MutationContext::deterministic(
            [7; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-22T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let (plan, changes) = executor
            .prepare_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();
        let paths = plan
            .delta()
            .actions()
            .iter()
            .map(|action| action.path().repository_relative())
            .collect::<Vec<_>>();

        assert!(paths.iter().any(|path| path == "docs/profile.txt"));
        assert!(paths
            .iter()
            .any(|path| path == ".jit/profiles/planner-asset-only.json"));
        assert!(paths.iter().any(|path| path == ".jit/events.jsonl"));
        assert_eq!(plan.hash().len(), 64);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_profile_dry_run_hash_is_stable_across_repeated_plans() {
        let (_temp, _storage, executor, package) = fixture();
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let context = MutationContext::preview();

        let first = executor
            .prepare_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();
        let second = executor
            .prepare_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();

        assert_eq!(first.0.hash(), second.0.hash());
        assert_eq!(first.1, second.1);
    }

    #[test]
    fn test_profile_application_revalidates_locked_snapshot_before_any_write() {
        let (temp, storage, executor, package) = fixture();
        fs::write(temp.path().join(".jit/config.toml"), b"not = [valid").unwrap();

        assert!(executor.apply_profile_package(&package).is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_preparation_rederives_from_recaptured_repository() {
        let (temp, _storage, executor, package) = fixture();
        let layout = executor.require_layout().unwrap();
        let inner = executor.storage().open_mutation_session(layout).unwrap();
        let mut session = RecaptureRaceSession {
            inner,
            captures: 0,
            config_path: temp.path().join(".jit/config.toml"),
        };

        let result = executor.prepare_profile(&mut session, &package, &MutationContext::preview());

        assert!(result.is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
    }

    #[test]
    fn test_profile_preparation_retries_when_final_proposed_closure_expands() {
        let (temp, _storage, executor, _package) = fixture();
        // The workflow package's projections name kinds its declared dependency
        // carries, so the dependency is applied first — the order the resolved
        // closure would apply them in.
        executor
            .apply_profile_package(&shipped_package_in(temp.path(), "jit-default"))
            .unwrap();
        let package = shipped_package_in(temp.path(), "jit-dogfood");
        let layout = executor.require_layout().unwrap();
        let inner = executor.storage().open_mutation_session(layout).unwrap();
        let mut session = FinalClosureRaceSession {
            inner,
            captures: 0,
            config_path: temp.path().join(".jit/config.toml"),
            source_path: temp.path().join("RACE.md"),
        };

        let first = executor
            .prepare_profile(&mut session, &package, &MutationContext::preview())
            .unwrap();
        assert!(first.is_none(), "an expanded final closure must retry");

        let (plan, _) = executor
            .prepare_profile(&mut session, &package, &MutationContext::preview())
            .unwrap()
            .expect("the repeated capture includes the new source");
        assert!(plan
            .image()
            .capture_spec()
            .contains_path(&VirtualPath::worktree("RACE.md").unwrap()));
        assert!(!temp.path().join(".jit/profiles/jit-dogfood.json").exists());
    }

    #[test]
    fn test_profile_application_preserves_and_certifies_torn_event_tail_end_to_end() {
        let (temp, storage, executor, package) = fixture();
        let torn = b"{\"torn\":";
        fs::write(temp.path().join(".jit/events.jsonl"), torn).unwrap();

        executor.apply_profile_package(&package).unwrap();

        let image = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        assert!(image.starts_with(b"{\"torn\":\n"));
        assert_eq!(&image[..torn.len()], torn);
        let events = storage.read_events().unwrap();
        assert!(matches!(
            events.as_slice(),
            [Event::ProfileApplied {
                isolated_torn_tail: true,
                ..
            }]
        ));
    }

    #[test]
    fn test_apply_profile_package_applies_a_declared_dependency_before_the_package() {
        let (temp, storage, executor, _fixture) = fixture();
        let dependency = package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

        // The set is the answer, in the order it was applied: the dependency,
        // then the package that declared it.
        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "workflow"]
        );
        assert_eq!(applied.count, applied.profiles.len());
        assert_eq!(applied.requested().unwrap().id, "workflow");
        assert!(applied
            .profiles
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Applied));
        // What each package publishes is in the repository, and the audit log
        // states the same order.
        assert!(temp.path().join("docs/base.txt").is_file());
        assert!(temp.path().join("docs/workflow.txt").is_file());
        assert_eq!(applied_event_ids(&storage), vec!["base", "workflow"]);
        // One record per package, each addressing its own package's bytes.
        assert_eq!(
            record_for(&temp, "base").package_hash,
            dependency.hashes().package
        );
        assert_eq!(
            record_for(&temp, "workflow").package_hash,
            dependant.hashes().package
        );
        assert_ne!(dependency.hashes().package, dependant.hashes().package);
    }

    #[test]
    fn test_apply_profile_package_applies_a_transitive_dependency_chain_deepest_first() {
        let (temp, storage, executor, _fixture) = fixture();
        package_declaring(&temp, "vendor/vocabulary", "vocabulary", &[]);
        package_declaring(&temp, "vendor/base", "base", &["vocabulary"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

        // `workflow` never names `vocabulary`; reaching it is what "transitively"
        // means, and it is applied before the package that reaches it.
        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["vocabulary", "base", "workflow"]
        );
        assert_eq!(
            applied_event_ids(&storage),
            vec!["vocabulary", "base", "workflow"]
        );
        for id in ["vocabulary", "base", "workflow"] {
            assert_eq!(record_for(&temp, id).id, id);
        }
    }

    #[test]
    fn test_apply_profile_package_applies_a_shared_dependency_once() {
        let (temp, storage, executor, _fixture) = fixture();
        package_declaring(&temp, "vendor/shared", "shared", &[]);
        package_declaring(&temp, "vendor/left", "left", &["shared"]);
        package_declaring(&temp, "vendor/right", "right", &["shared"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["left", "right"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

        let order: Vec<&str> = applied
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect();
        assert_eq!(
            order.iter().filter(|id| **id == "shared").count(),
            1,
            "a dependency two packages declare is applied once: {order:?}"
        );
        let position = |id: &str| order.iter().position(|candidate| *candidate == id).unwrap();
        assert!(position("shared") < position("left"));
        assert!(position("shared") < position("right"));
        assert!(position("left") < position("workflow"));
        assert!(position("right") < position("workflow"));
        assert_eq!(
            applied_event_ids(&storage)
                .iter()
                .filter(|id| *id == "shared")
                .count(),
            1
        );
    }

    #[test]
    fn test_apply_profile_package_names_both_packages_of_a_conflicting_asset_target() {
        let (temp, _storage, executor, _fixture) = fixture();
        let target = "docs/shared.txt";
        let occupant = package_publishing(&temp, "vendor/base", "base", target, "first\n");
        let candidate =
            package_publishing(&temp, "vendor/workflow", "workflow", target, "second\n");
        executor.apply_profile_package(&occupant).unwrap();

        let error = executor.apply_profile_package(&candidate).unwrap_err();

        let conflict = target_conflict(&error);
        assert_eq!(
            conflict.occupant,
            ProfileConflictOccupant::Package(ProfilePackageId::new(
                occupant.manifest().profile.id.to_string()
            ))
        );
        assert_eq!(
            conflict.candidate,
            ProfilePackageId::new(candidate.manifest().profile.id.to_string())
        );
        assert_eq!(conflict.path.repository_relative(), target);
        // The occupant's bytes stand: a conflict applies nothing.
        assert_eq!(
            fs::read_to_string(temp.path().join(target)).unwrap(),
            "first\n"
        );
    }

    #[test]
    fn test_apply_profile_package_names_both_packages_of_a_conflicting_contribution() {
        let (temp, _storage, executor, _fixture) = fixture();
        let namespace = "shared-namespace";
        let occupant = package_contributing(
            &temp,
            "vendor/base",
            "base",
            namespace,
            "What the occupant means by it.",
        );
        let candidate = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "Another meaning entirely.",
        );
        executor.apply_profile_package(&occupant).unwrap();

        let error = executor.apply_profile_package(&candidate).unwrap_err();

        let Some(RepositoryStateError::Producer(ProducerError::ProfileContributionConflict {
            identity,
            candidate: conflicting,
            occupant: held_by,
            ..
        })) = error.downcast_ref::<RepositoryStateError>()
        else {
            panic!("a colliding contribution fails as a contribution conflict: {error:#}");
        };
        assert_eq!(identity, namespace);
        assert_eq!(
            *held_by,
            ProfileConflictOccupant::Package(ProfilePackageId::new(
                occupant.manifest().profile.id.to_string()
            ))
        );
        assert_eq!(
            *conflicting,
            ProfilePackageId::new(candidate.manifest().profile.id.to_string())
        );
    }

    #[test]
    fn test_apply_profile_package_reports_the_repository_as_the_occupant_it_authored() {
        let (temp, _storage, executor, _fixture) = fixture();
        let target = "docs/shared.txt";
        // A package is applied first, so what distinguishes the answer is which
        // targets a record claims rather than whether any record exists.
        let applied = package_publishing(
            &temp,
            "vendor/applied",
            "applied",
            "docs/applied.txt",
            "applied\n",
        );
        executor.apply_profile_package(&applied).unwrap();
        let authored = temp.path().join(target);
        fs::create_dir_all(authored.parent().unwrap()).unwrap();
        fs::write(&authored, "authored here\n").unwrap();
        let candidate = package_publishing(
            &temp,
            "vendor/workflow",
            "workflow",
            target,
            "package bytes\n",
        );

        let error = executor.apply_profile_package(&candidate).unwrap_err();

        let conflict = target_conflict(&error);
        assert_eq!(conflict.occupant, ProfileConflictOccupant::Repository);
        assert_eq!(conflict.path.repository_relative(), target);
        assert_eq!(fs::read_to_string(&authored).unwrap(), "authored here\n");
    }

    #[test]
    fn test_apply_profile_package_merges_an_identical_restatement_of_an_applied_target() {
        let (temp, _storage, executor, _fixture) = fixture();
        let target = "docs/shared.txt";
        let namespace = "shared-namespace";
        let contribution = namespace_contribution(namespace, "One meaning.");
        let stated = authored_package(
            &temp,
            "vendor/stated",
            "stated",
            target,
            "identical\n",
            &contribution,
        );
        let restated = authored_package(
            &temp,
            "vendor/restated",
            "restated",
            target,
            "identical\n",
            &contribution,
        );
        executor.apply_profile_package(&stated).unwrap();

        let applied = executor.apply_profile_package(&restated).unwrap();

        // A restatement of what is already there is not a conflict: neither
        // package overrides the other, and both records stand.
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read_to_string(temp.path().join(target)).unwrap(),
            "identical\n"
        );
        assert_eq!(record_for(&temp, "stated").id, "stated");
        assert_eq!(record_for(&temp, "restated").id, "restated");
    }

    #[test]
    fn test_apply_profile_package_rejects_a_dependency_cycle_before_applying_anything() {
        let (temp, storage, executor, _fixture) = fixture();
        package_declaring(&temp, "vendor/base", "base", &["workflow"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let error = executor.apply_profile_package(&dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::DependencyCycle { cycle })
                    if cycle.first() == cycle.last()
                        && cycle.iter().any(|id| id == "base")
                        && cycle.iter().any(|id| id == "workflow")
            ),
            "the refusal must name the cycle: {error:#}"
        );
        // Neither package of the cycle reached the repository.
        assert!(!temp.path().join("docs/base.txt").exists());
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_reports_a_dependency_that_cannot_be_resolved() {
        let (temp, storage, executor, _fixture) = fixture();
        // Nothing sits beside the package under that name and this repository
        // has no record for it, so no route resolves the dependency.
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["absent-base"]);

        let error = executor.apply_profile_package(&dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::UnresolvableDependency { package, dependency, .. })
                    if package == "workflow" && dependency == "absent-base"
            ),
            "the refusal must name both packages: {error:#}"
        );
        // Naming both is what distinguishes this from an absent profile the
        // adopter asked for by name.
        let message = format!("{error:#}");
        assert!(message.contains("workflow"), "{message}");
        assert!(message.contains("absent-base"), "{message}");
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_reports_no_work_for_an_applied_unchanged_closure() {
        let (temp, storage, executor, _fixture) = fixture();
        package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);
        executor.apply_profile_package(&dependant).unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let records = ["base", "workflow"].map(|id| record_for(&temp, id));

        let reapplied = executor.apply_profile_package(&dependant).unwrap();

        // No work anywhere in the set: the dependency is unchanged too, not
        // only the package that was named.
        assert!(
            reapplied
                .profiles
                .iter()
                .all(|profile| profile.status == ProfileApplicationStatus::Unchanged),
            "{:?}",
            reapplied.profiles
        );
        assert!(reapplied
            .profiles
            .iter()
            .all(|profile| profile.transaction_id.is_none()));
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events
        );
        assert_eq!(applied_event_ids(&storage), vec!["base", "workflow"]);
        assert_eq!(
            ["base", "workflow"].map(|id| record_for(&temp, id)),
            records
        );
    }

    #[test]
    fn test_apply_profile_package_resolves_a_dependency_from_the_record_when_none_sits_beside_it() {
        let (temp, _storage, executor, _fixture) = fixture();
        let dependency = package_declaring(&temp, "vendor/base", "base", &[]);
        executor.apply_profile_package(&dependency).unwrap();
        // A dependant elsewhere in the worktree, with no `base` beside it: what
        // answers is this repository's own record for the dependency.
        let dependant = package_declaring(&temp, "elsewhere/workflow", "workflow", &["base"]);
        assert!(!temp.path().join("elsewhere/base").exists());

        let applied = executor.apply_profile_package(&dependant).unwrap();

        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("base", ProfileApplicationStatus::Unchanged),
                ("workflow", ProfileApplicationStatus::Applied),
            ]
        );
        assert_eq!(
            record_for(&temp, "base").package_hash,
            dependency.hashes().package
        );
    }

    #[test]
    fn test_resolve_profile_closure_reads_a_dependency_from_beside_the_declaring_package() {
        let (temp, _storage, executor, _fixture) = fixture();
        // Two copies of the dependency, distinguishable by their bytes alone.
        // The one beside the declaring package is the one its declaration means.
        package_declaring(&temp, "elsewhere/base", "base", &[]);
        let recorded = repackage(&temp, "elsewhere/base", "assets/profile.txt", "elsewhere\n");
        executor.apply_profile_package(&recorded).unwrap();
        let beside = package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);
        assert_ne!(beside.hashes(), recorded.hashes());

        let closure = executor.resolve_profile_closure(&dependant).unwrap();

        assert_eq!(
            closure
                .iter()
                .map(|package| package.hashes())
                .collect::<Vec<_>>(),
            vec![beside.hashes(), dependant.hashes()]
        );
    }

    #[test]
    fn test_resolve_profile_closure_reports_an_unreadable_package_beside_the_declaring_one() {
        let (temp, _storage, executor, _fixture) = fixture();
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);
        // A directory that is there under the dependency's name and holds no
        // readable package: passing over it would answer with bytes the adopter
        // did not put there.
        fs::create_dir_all(temp.path().join("vendor/base")).unwrap();
        fs::write(
            temp.path().join("vendor/base/manifest.toml"),
            b"not = [valid",
        )
        .unwrap();

        let error = executor.resolve_profile_closure(&dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::UnresolvableDependency { package, dependency, .. })
                    if package == "workflow" && dependency == "base"
            ),
            "{error:#}"
        );
        assert!(format!("{error:#}").contains("vendor/base"), "{error:#}");
    }

    #[test]
    fn test_resolve_profile_closure_passes_over_a_neighbour_declaring_another_profile() {
        let (temp, _storage, executor, _fixture) = fixture();
        // The directory beside the declarant is named for the dependency but
        // holds a different package, so it is not that dependency and nothing
        // else answers either.
        package_declaring(&temp, "vendor/base", "impostor", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let error = executor.resolve_profile_closure(&dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::UnresolvableDependency { dependency, .. })
                    if dependency == "base"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_resolve_profile_closure_of_a_package_declaring_nothing_is_that_package() {
        let (_temp, _storage, executor, package) = fixture();

        let closure = executor.resolve_profile_closure(&package).unwrap();

        assert_eq!(
            closure
                .iter()
                .map(|package| package.hashes())
                .collect::<Vec<_>>(),
            vec![package.hashes()]
        );
    }

    #[test]
    fn test_profile_application_does_not_mark_valid_unterminated_event_as_torn() {
        let (temp, storage, executor, package) = fixture();
        let prior_event = Event::draft_profile_applied(
            "prior".to_string(),
            "1.0.0".to_string(),
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/prior").unwrap()),
            "prior-package".to_string(),
            BTreeMap::new(),
            false,
        );
        fs::write(
            temp.path().join(".jit/events.jsonl"),
            serde_json::to_vec(&prior_event).unwrap(),
        )
        .unwrap();

        executor.apply_profile_package(&package).unwrap();

        let events = storage.read_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events.last().unwrap(),
            Event::ProfileApplied {
                isolated_torn_tail: false,
                ..
            }
        ));
    }
}
