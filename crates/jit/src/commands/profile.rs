use super::{capture_or_retry, with_mutation_session, CommandExecutor, SessionStep};
use crate::domain::ProfileLifecycleOperation;
use crate::profile::{
    build_profile_claims_from_resolved, resolve_package, resolve_package_from_record,
    EngineVersion, ProfileApplicationStatus, ProfileApplyResult, ProfileComposedApplyResult,
    ProfileGraphError, ProfileId, ProfileListResult, ProfileOrigin, ProfilePackage,
    ProfilePackageError, ProfilePackageSource, ProfilePlanEntry, ProfilePlanResult,
    ProfilePlanStatus, ProfileShowEntry, ProfileShowResult, ProfileSummary, ProfileTargetAction,
    ProfileTargetChange, ProfileVariableAssignment, ProfileVariableName, RecordedValueAuthority,
    ResolvedProfileContent, ResolvedProfileGraph, ResolvedVariables, VariableInputs,
};
use crate::repository_state::{
    apply_overlay, derive_materialization, AppliedProfileRecord, CaptureBudget, CaptureSpec,
    MaterializationPlan, MaterializationRequest, MutationContext, ProfileApplicationInput,
    ProfileContributionClaim, ProfileTargetDisposition, RepositoryEntry, RepositoryImage,
    RepositoryLayout, RepositoryRootClass, VirtualPath,
};
use crate::storage::{JsonFileStorage, RepositoryMutationSession};
use crate::validation::repository::RepositoryValidationFailure;
use anyhow::Result;
use semver::Version;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Project dependency-first unique durable results back onto ordered root
/// observations. A repeated selector observes the transaction's first result,
/// then reports an unchanged second observation without attempting another
/// publication.
fn selection_profile_results(
    applied: Vec<ProfileApplyResult>,
    roots: &[ProfilePackage],
) -> Result<ProfileComposedApplyResult> {
    let root_ids = roots
        .iter()
        .map(|root| root.model().id.as_str())
        .collect::<BTreeSet<_>>();
    let applied_by_id = applied
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect::<BTreeMap<_, _>>();
    let mut results = applied
        .iter()
        .filter(|result| !root_ids.contains(result.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut occurrences = BTreeMap::<&str, usize>::new();
    for root in roots {
        let id = root.model().id.as_str();
        let Some(source) = applied_by_id.get(id) else {
            anyhow::bail!("selected root '{id}' has no aggregate application result");
        };
        let occurrence = occurrences.entry(id).or_default();
        let first = *occurrence == 0;
        *occurrence += 1;
        let mut result = (*source).clone();
        if !first {
            result.status = ProfileApplicationStatus::Unchanged;
            result.transaction_id = None;
            result.warnings.clear();
        }
        results.push(result);
    }
    Ok(ProfileComposedApplyResult::new(results))
}

/// One ordered profile selection from the command boundary.
///
/// The tag is part of the value rather than inferred from filesystem state:
/// `id:NAME` reads the recorded package named by `NAME`, while `path:DIR`
/// reads the package directory `DIR` relative to the worktree. This keeps one
/// occurrence stream lossless and prevents a local directory from silently
/// replacing an explicitly selected recorded package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileSelector {
    /// A profile id resolved through this repository's applied records.
    Id(ProfileId),
    /// A package directory, resolved relative to the repository worktree.
    Path(PathBuf),
}

/// A malformed `--profile` selector.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileSelectorError {
    /// The selector does not have one of the supported tags.
    #[error("invalid profile selector '{selector}': expected id:ID or path:DIR")]
    InvalidShape {
        /// User-supplied selector.
        selector: String,
    },
    /// The selector has a supported tag but no value.
    #[error("invalid profile selector '{selector}': the value after '{kind}:' is empty")]
    EmptyValue {
        /// User-supplied selector.
        selector: String,
        /// Supported selector tag.
        kind: String,
    },
    /// The id portion does not satisfy the package id contract.
    #[error("invalid profile selector '{selector}': {error}")]
    InvalidId {
        /// User-supplied selector.
        selector: String,
        /// Canonical profile-id validation failure.
        error: String,
    },
}

impl FromStr for ProfileSelector {
    type Err = ProfileSelectorError;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        let (kind, value) =
            selector
                .split_once(':')
                .ok_or_else(|| ProfileSelectorError::InvalidShape {
                    selector: selector.to_string(),
                })?;
        if value.is_empty() {
            return Err(ProfileSelectorError::EmptyValue {
                selector: selector.to_string(),
                kind: match kind {
                    "id" | "path" => kind.to_string(),
                    _ => {
                        return Err(ProfileSelectorError::InvalidShape {
                            selector: selector.to_string(),
                        })
                    }
                },
            });
        }
        match kind {
            "id" => ProfileId::try_from(value.to_string())
                .map(ProfileSelector::Id)
                .map_err(|error| ProfileSelectorError::InvalidId {
                    selector: selector.to_string(),
                    error,
                }),
            "path" => Ok(ProfileSelector::Path(PathBuf::from(value))),
            _ => Err(ProfileSelectorError::InvalidShape {
                selector: selector.to_string(),
            }),
        }
    }
}

impl ProfileSelector {
    /// Construct an id selector from a validated profile id.
    pub fn id(id: impl AsRef<str>) -> Result<Self, ProfileSelectorError> {
        format!("id:{}", id.as_ref()).parse()
    }

    /// Construct a path selector from a worktree-relative or absolute spelling.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }
}

/// Failure resolving which package bytes a command reads.
///
/// Resolution names the route a selector addressed together with what failed.
/// A recorded location that no longer resolves is one of these rather than
/// an absent profile: a deleted directory is a repository whose record
/// outlived its package, not a repository that never applied one.
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
    /// A path selector attempts to use the name of an explicitly selected id.
    #[error(
        "profile package directory '{location}' declares profile '{id}', which shadows a selected recorded profile with the same id"
    )]
    PathShadowsSelectedId {
        /// Path selector that declared the duplicate id.
        location: String,
        /// Profile id that the path would shadow.
        id: String,
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
    /// A converted shipped-v1 record preserves an embedded origin, but this
    /// binary deliberately has no embedded-package discovery surface.
    #[error(
        "applied profile record '{record}' has embedded provenance, which is unavailable for package resolution"
    )]
    EmbeddedProvenanceUnavailable {
        /// Repository-relative applied-record path.
        record: String,
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
    /// A dependency is present but outside the version range its declarer
    /// requires.
    #[error(
        "profile '{package}' requires dependency '{dependency}' at '{required}', but the participating package provides '{found}'"
    )]
    DependencyVersionMismatch {
        /// Package whose manifest declares the dependency.
        package: String,
        /// Dependency whose version did not satisfy the requirement.
        dependency: String,
        /// Authored dependency range.
        required: String,
        /// Resolved dependency version.
        found: String,
    },
    /// The declared dependencies close a cycle, which has no application order.
    #[error("profile dependency cycle: {}", .cycle.join(" -> "))]
    DependencyCycle {
        /// The cycle as a closed path: the first id repeats as the last.
        cycle: Vec<String>,
    },
    /// The running engine is outside a participating package's compatibility
    /// range.
    #[error(
        "profile '{package}' requires compatible JIT '{required}', but the running engine is '{actual}'"
    )]
    IncompatibleEngine {
        /// Package whose compatibility range rejected the engine.
        package: String,
        /// Authored engine range.
        required: String,
        /// Running engine version.
        actual: String,
    },
    /// Two participating packages declare an incompatibility that applies to
    /// the other package's version.
    #[error(
        "profile '{package}' is incompatible with profile '{other}' (requirement '{requirement}', other version '{other_version}')"
    )]
    IncompatiblePackages {
        /// Package declaring the incompatibility.
        package: String,
        /// Participating package named by the declaration.
        other: String,
        /// Authored incompatible-package range.
        requirement: String,
        /// Version of the other package.
        other_version: String,
    },
    /// Two different package images claim one profile identity during graph
    /// construction.
    #[error(
        "profile id '{id}' resolves to different package images '{first_hash}' and '{second_hash}'"
    )]
    ConflictingPackageIdentity {
        /// Profile identity with multiple images.
        id: String,
        /// First package digest.
        first_hash: String,
        /// Second package digest.
        second_hash: String,
    },
}

/// Profile application conflict detected before transaction preparation.
#[derive(Debug, thiserror::Error)]
pub enum ProfileApplyError {
    /// Reconfiguration selected package bytes that no longer match the
    /// installed package provenance.
    #[error(
        "profile '{id}' cannot be reconfigured because its installed package identity changed (installed version '{installed_version}' hash '{installed_hash}', found version '{found_version}' hash '{found_hash}')"
    )]
    ReconfigurationPackageChanged {
        /// Stable profile identity.
        id: String,
        /// Version recorded when the package was installed.
        installed_version: String,
        /// Package digest recorded when the package was installed.
        installed_hash: String,
        /// Version currently supplied by the selected package.
        found_version: String,
        /// Package digest currently supplied by the selected package.
        found_hash: String,
    },
    /// An upgrade replacement did not advance the installed semantic version.
    #[error(
        "profile '{id}' cannot be upgraded from version '{installed_version}' to non-newer version '{candidate_version}'"
    )]
    UpgradeVersionNotNewer {
        /// Stable profile identity.
        id: String,
        /// Version recorded when the package was installed.
        installed_version: String,
        /// Replacement package version.
        candidate_version: String,
    },
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

/// Read-only command options for the package's non-secret input channels.
///
/// The command boundary turns these filesystem and process inputs into the
/// pure [`VariableInputs`] value consumed by profile resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileVariableOptions {
    /// Optional TOML file containing exactly one `[variables]` string table.
    pub values_file: Option<PathBuf>,
    /// Repeated typed assignments in command-line occurrence order.
    pub assignments: Vec<ProfileVariableAssignment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileValuesFile {
    variables: BTreeMap<ProfileVariableName, String>,
}

/// Capture profile variable inputs from the command boundary.
pub(super) fn load_profile_variable_inputs(
    packages: &[ProfilePackage],
    values_file: Option<&Path>,
    assignments: &[ProfileVariableAssignment],
) -> Result<VariableInputs> {
    let values_file = values_file
        .map(fs::read_to_string)
        .transpose()?
        .map(|text| toml::from_str::<ProfileValuesFile>(&text))
        .transpose()?
        .map(|file| file.variables)
        .unwrap_or_default();
    let declarations = packages
        .iter()
        .flat_map(|package| package.model().variables.iter())
        .collect::<Vec<_>>();
    let names = declarations
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<BTreeSet<_>>();
    let environment_names = declarations
        .iter()
        .filter_map(|declaration| declaration.env.clone())
        .collect::<BTreeSet<_>>();
    let mut inputs = VariableInputs {
        values_file,
        environment: BTreeMap::new(),
        command_line: assignments.to_vec(),
    };
    inputs.validate_against_names(&names, &environment_names)?;

    for declaration in declarations {
        let Some(env_name) = declaration.env.as_ref() else {
            continue;
        };
        if let Some(value) = std::env::var_os(env_name.as_str()) {
            let value = value.into_string().map_err(|_| {
                anyhow::anyhow!(
                    "environment variable '{env_name}' for profile variable '{}' is not valid UTF-8",
                    declaration.name
                )
            })?;
            inputs.environment.insert(env_name.clone(), value);
        }
    }
    Ok(inputs)
}

pub(super) fn validate_variable_inputs(
    packages: &[ProfilePackage],
    inputs: &VariableInputs,
) -> Result<()> {
    let names = packages
        .iter()
        .flat_map(|package| package.model().variables.iter())
        .map(|declaration| declaration.name.clone())
        .collect::<BTreeSet<_>>();
    let environment_names = packages
        .iter()
        .flat_map(|package| package.model().variables.iter())
        .filter_map(|declaration| declaration.env.clone())
        .collect::<BTreeSet<_>>();
    inputs.validate_against_names(&names, &environment_names)?;
    Ok(())
}

impl CommandExecutor<JsonFileStorage> {
    /// Read the package one profile command acts on.
    ///
    /// An `id:` selector reads the location named by this repository's applied
    /// profile record. A `path:` selector reads a package directory in the
    /// worktree and uses the id declared by that package. A recorded location
    /// that no longer holds a readable package is a [`ProfileResolutionError`]
    /// naming the record and location rather than an absent profile.
    pub fn resolve_profile_package(&self, selector: &ProfileSelector) -> Result<ProfilePackage> {
        let layout = self.require_layout()?;
        match selector {
            ProfileSelector::Path(location) => supplied_package(location, &layout),
            ProfileSelector::Id(id) => match self.read_applied_profile_record(id.as_str())? {
                Some(record) => {
                    recorded_package(&record, &applied_record_path(id.as_str())?, &layout)
                }
                None => Err(
                    crate::errors::NotFoundError::new(format!("Profile not found: {id}")).into(),
                ),
            },
        }
    }

    /// Resolve every selector in occurrence order before any package is applied.
    ///
    /// A path that declares the id of an explicitly selected recorded package is
    /// rejected before application, so selector order cannot make one source
    /// silently win over the other.
    pub fn resolve_profile_selectors(
        &self,
        selectors: &[ProfileSelector],
    ) -> Result<Vec<ProfilePackage>> {
        let resolved = selectors
            .iter()
            .map(|selector| self.resolve_profile_package(selector))
            .collect::<Result<Vec<_>>>()?;
        let selected_id_sources = resolved
            .iter()
            .zip(selectors)
            .filter_map(|(package, selector)| match selector {
                ProfileSelector::Id(_) => {
                    Some((package.model().id.to_string(), package.source().clone()))
                }
                ProfileSelector::Path(_) => None,
            })
            .collect::<Vec<_>>();

        resolved
            .into_iter()
            .zip(selectors)
            .map(|(package, selector)| {
                if let ProfileSelector::Path(location) = selector {
                    let shadows = selected_id_sources.iter().any(|(id, source)| {
                        id == package.model().id.as_str() && source != package.source()
                    });
                    if shadows {
                        return Err(ProfileResolutionError::PathShadowsSelectedId {
                            location: location.display().to_string(),
                            id: package.model().id.to_string(),
                        }
                        .into());
                    }
                }
                Ok(package)
            })
            .collect()
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

    /// Inspect every selected profile package in selector occurrence order.
    pub fn show_profiles(&self, selectors: &[ProfileSelector]) -> Result<ProfileShowResult> {
        let layout = self.require_layout()?;
        let profiles = self
            .resolve_profile_selectors(selectors)?
            .iter()
            .map(|package| self.show_profile_entry(package, &layout))
            .collect::<Result<Vec<_>>>()?;
        Ok(ProfileShowResult::new(profiles))
    }

    /// Build one package inspection entry after selector-level ambiguity checks
    /// have already run for the complete request.
    fn show_profile_entry(
        &self,
        package: &ProfilePackage,
        layout: &RepositoryLayout,
    ) -> Result<ProfileShowEntry> {
        let id = package.model().id.as_str();
        Ok(ProfileShowEntry {
            manifest: package.model().clone(),
            origin: package_origin(package, layout)?,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
            file_count: package.file_count(),
            byte_size: package.byte_size(),
            applied: self.read_applied_profile_record(id)?,
        })
    }

    /// Build the exact non-mutating target plan for one resolved profile.
    pub fn plan_profiles(&self, selectors: &[ProfileSelector]) -> Result<ProfilePlanResult> {
        self.plan_profiles_with_inputs(selectors, &VariableInputs::default())
    }

    /// Build plans from already captured, command-bound variable inputs.
    pub fn plan_profiles_with_inputs(
        &self,
        selectors: &[ProfileSelector],
        inputs: &VariableInputs,
    ) -> Result<ProfilePlanResult> {
        // Validate the complete request first so dry-run uses the same
        // selector-level ambiguity and confinement rules as show/apply before
        // constructing any individual preview.
        let selected = self.resolve_profile_selectors(selectors)?;
        if !selected.is_empty() {
            let packages = self.resolve_profile_graph(&selected)?.selected_packages();
            validate_variable_inputs(&packages, inputs)?;
        }
        selectors
            .iter()
            .map(|selector| {
                let package = self.resolve_profile_package(selector)?;
                self.plan_profile_package_with_inputs(&package, inputs)
            })
            .collect::<Result<Vec<_>>>()
            .map(ProfilePlanResult::new)
    }

    /// Build plans after loading the values file and declared environment
    /// variables at the command boundary.
    pub fn plan_profiles_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfilePlanResult> {
        let selected = self.resolve_profile_selectors(selectors)?;
        let packages = if selected.is_empty() {
            Vec::new()
        } else {
            self.resolve_profile_graph(&selected)?.selected_packages()
        };
        let inputs = load_profile_variable_inputs(
            &packages,
            options.values_file.as_deref(),
            &options.assignments,
        )?;
        self.plan_profiles_with_inputs(selectors, &inputs)
    }

    /// Build the exact non-mutating target plan for one resolved profile.
    pub fn plan_profile(&self, selector: &ProfileSelector) -> Result<ProfilePlanEntry> {
        self.plan_profile_with_inputs(selector, &VariableInputs::default())
    }

    /// Build one exact plan with resolved variables.
    pub fn plan_profile_with_inputs(
        &self,
        selector: &ProfileSelector,
        inputs: &VariableInputs,
    ) -> Result<ProfilePlanEntry> {
        let package = self.resolve_profile_package(selector)?;
        validate_variable_inputs(std::slice::from_ref(&package), inputs)?;
        self.plan_profile_package_with_inputs(&package, inputs)
    }

    fn plan_profile_package_with_inputs(
        &self,
        package: &ProfilePackage,
        inputs: &VariableInputs,
    ) -> Result<ProfilePlanEntry> {
        let resolved = resolve_package(
            package,
            &inputs.for_declarations(&package.model().variables),
        )?;
        let metadata = package.model();
        let layout = self.require_layout()?;
        let context = MutationContext::preview();
        let contribution_context =
            self.profile_contribution_candidates(std::slice::from_ref(package), inputs)?;
        with_mutation_session(self.storage(), &layout, "profile planning", |session| {
            let Some(plan) = self.prepare_profile_selection(
                session,
                std::slice::from_ref(package),
                std::slice::from_ref(&resolved),
                &contribution_context,
                &context,
                ProfileLifecycleOperation::Apply,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            let changes = profile_target_changes(&plan);
            Ok(SessionStep::Done(ProfilePlanEntry {
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
        selectors: &[ProfileSelector],
    ) -> Result<ProfileComposedApplyResult> {
        self.apply_profile_with_inputs(selectors, &VariableInputs::default())
    }

    /// Resolve and apply profiles with already captured variable inputs.
    ///
    /// One complete selected closure is planned and published through the held
    /// session. Selector occurrence order remains a result concern only.
    pub fn apply_profile_with_inputs(
        &self,
        selectors: &[ProfileSelector],
        inputs: &VariableInputs,
    ) -> Result<ProfileComposedApplyResult> {
        let selected = self.resolve_profile_selectors(selectors)?;
        if selected.is_empty() {
            return Ok(ProfileComposedApplyResult::new(Vec::new()));
        }
        let packages = self
            .resolve_profile_graph_for_mutation(&selected)?
            .selected_packages();
        validate_variable_inputs(&packages, inputs)?;
        let contribution_context = self.profile_contribution_candidates(&packages, inputs)?;
        let applied = self.apply_profile_selection(&packages, inputs, &contribution_context)?;
        selection_profile_results(applied.profiles, &selected)
    }

    /// Resolve the semantic candidates that every selected package contributes.
    ///
    /// The candidate vector is carried into the one aggregate materialization so
    /// each affected record derives its shared ownership from the same selection.
    pub(super) fn profile_contribution_candidates(
        &self,
        packages: &[ProfilePackage],
        inputs: &VariableInputs,
    ) -> Result<Vec<ProfileContributionClaim>> {
        let resolved = packages
            .iter()
            .map(|package| {
                resolve_package(
                    package,
                    &inputs.for_declarations(&package.model().variables),
                )
                .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        self.profile_contribution_candidates_from_resolved(&resolved)
    }

    /// Build semantic ownership candidates from content whose variables have
    /// already been resolved by one lifecycle operation.
    fn profile_contribution_candidates_from_resolved(
        &self,
        resolved: &[ResolvedProfileContent],
    ) -> Result<Vec<ProfileContributionClaim>> {
        let layout = self.require_layout()?;
        resolved
            .iter()
            .map(|resolved| {
                build_profile_claims_from_resolved(resolved, &layout, false)
                    .map(|claims| claims.contributions)
                    .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()
            .map(|claims| claims.into_iter().flatten().collect())
    }

    /// Resolve and apply profiles after loading command-bound variable inputs.
    pub fn apply_profile_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        let selected = self.resolve_profile_selectors(selectors)?;
        let packages = if selected.is_empty() {
            Vec::new()
        } else {
            self.resolve_profile_graph_for_mutation(&selected)?
                .selected_packages()
        };
        let inputs = load_profile_variable_inputs(
            &packages,
            options.values_file.as_deref(),
            &options.assignments,
        )?;
        self.apply_profile_with_inputs(selectors, &inputs)
    }

    /// Re-render already installed packages from their recorded values and
    /// newly supplied inputs without changing package identity.
    pub fn reconfigure_profiles_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        let (selected, packages, resolved) = self.lifecycle_selection_from_sources(
            selectors,
            options,
            RecordedValueAuthority::Exact,
        )?;
        if selected.is_empty() {
            return Ok(ProfileComposedApplyResult::new(Vec::new()));
        }
        let contribution_context = self.profile_contribution_candidates_from_resolved(&resolved)?;
        let applied = self.apply_resolved_profile_selection(
            &packages,
            &resolved,
            &contribution_context,
            ProfileLifecycleOperation::Reconfigure,
        )?;
        selection_profile_results(applied.profiles, &selected)
    }

    /// Preview the exact reconfiguration aggregate without publishing files,
    /// provenance, or an audit event.
    pub fn plan_reconfigure_profiles_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfilePlanResult> {
        let (selected, packages, resolved) = self.lifecycle_selection_from_sources(
            selectors,
            options,
            RecordedValueAuthority::Exact,
        )?;
        if selected.is_empty() {
            return Ok(ProfilePlanResult::new(Vec::new()));
        }
        let contribution_context = self.profile_contribution_candidates_from_resolved(&resolved)?;
        self.plan_resolved_profile_selection(
            &selected,
            &packages,
            &resolved,
            &contribution_context,
            ProfileLifecycleOperation::Reconfigure,
        )
    }

    /// Replace selected installed packages with newer package versions while
    /// retaining stored values for declarations that survive the replacement.
    pub fn upgrade_profiles_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        let (selected, packages, resolved) = self.lifecycle_selection_from_sources(
            selectors,
            options,
            RecordedValueAuthority::Superseded,
        )?;
        if selected.is_empty() {
            return Ok(ProfileComposedApplyResult::new(Vec::new()));
        }
        let contribution_context = self.profile_contribution_candidates_from_resolved(&resolved)?;
        let applied = self.apply_resolved_profile_selection(
            &packages,
            &resolved,
            &contribution_context,
            ProfileLifecycleOperation::Upgrade,
        )?;
        selection_profile_results(applied.profiles, &selected)
    }

    /// Preview the exact upgrade aggregate without publishing files,
    /// provenance, or an audit event.
    pub fn plan_upgrade_profiles_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfilePlanResult> {
        let (selected, packages, resolved) = self.lifecycle_selection_from_sources(
            selectors,
            options,
            RecordedValueAuthority::Superseded,
        )?;
        if selected.is_empty() {
            return Ok(ProfilePlanResult::new(Vec::new()));
        }
        let contribution_context = self.profile_contribution_candidates_from_resolved(&resolved)?;
        self.plan_resolved_profile_selection(
            &selected,
            &packages,
            &resolved,
            &contribution_context,
            ProfileLifecycleOperation::Upgrade,
        )
    }

    /// Resolve and render one lifecycle selection from the records the
    /// repository already holds.
    ///
    /// `authority` is the whole difference between the two commands. Under
    /// [`RecordedValueAuthority::Exact`] every closure member must still be the
    /// package its record describes, so a replay cannot switch identity. Under
    /// [`RecordedValueAuthority::Superseded`] a selected member must be
    /// installed and its replacement must advance the version, while a
    /// dependency the closure reaches for the first time is resolved as an
    /// ordinary first application.
    ///
    /// The graph is settled over every applied package that survives the
    /// selection before any member is rendered, so a range or incompatibility
    /// another profile depends on refuses the selection here rather than at the
    /// publication this precedes.
    fn lifecycle_selection_from_sources(
        &self,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
        authority: RecordedValueAuthority,
    ) -> Result<(
        Vec<ProfilePackage>,
        Vec<ProfilePackage>,
        Vec<ResolvedProfileContent>,
    )> {
        let selected = self.resolve_profile_selectors(selectors)?;
        if selected.is_empty() {
            return Ok((selected, Vec::new(), Vec::new()));
        }
        let selected_ids = selected
            .iter()
            .map(|package| package.model().id.clone())
            .collect::<BTreeSet<_>>();
        let packages = self
            .resolve_profile_graph_for_mutation(&selected)?
            .selected_packages();
        let inputs = load_profile_variable_inputs(
            &packages,
            options.values_file.as_deref(),
            &options.assignments,
        )?;
        let resolved = packages
            .iter()
            .map(|package| {
                let id = package.model().id.as_str();
                let package_inputs = inputs.for_declarations(&package.model().variables);
                let missing_record = || ProfileApplyError::InstalledRecordConflict {
                    path: applied_record_path(id)
                        .map(|path| path.repository_relative())
                        .unwrap_or_else(|_| id.to_string()),
                    id: id.to_string(),
                };
                match (self.read_applied_profile_record(id)?, authority) {
                    (Some(record), RecordedValueAuthority::Exact) => {
                        ensure_reconfiguration_package_identity(package, &record)?;
                        resolve_package_from_record(
                            package,
                            &record.variables,
                            &package_inputs,
                            authority,
                        )
                        .map_err(Into::into)
                    }
                    (Some(record), RecordedValueAuthority::Superseded) => {
                        ensure_upgrade_version_is_newer_when_replaced(package, &record)?;
                        resolve_package_from_record(
                            package,
                            &record.variables,
                            &package_inputs,
                            authority,
                        )
                        .map_err(Into::into)
                    }
                    (None, RecordedValueAuthority::Exact) => Err(missing_record().into()),
                    (None, RecordedValueAuthority::Superseded) => {
                        if selected_ids.contains(&package.model().id) {
                            return Err(missing_record().into());
                        }
                        resolve_package(package, &package_inputs).map_err(Into::into)
                    }
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((selected, packages, resolved))
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
        self.resolve_profile_graph(std::slice::from_ref(package))
            .map(|graph| graph.selected_packages())
    }

    /// Resolve selected packages together with every package already recorded
    /// as applied, then settle all dependency, incompatibility, and engine
    /// range claims before a caller prepares a publication.
    pub fn resolve_profile_graph(
        &self,
        selected: &[ProfilePackage],
    ) -> Result<ResolvedProfileGraph> {
        let applied = self.resolve_applied_profile_packages()?;
        self.resolve_profile_graph_with_applied(selected, applied)
    }

    /// Mutating profile application may retain authenticated embedded
    /// provenance as ownership evidence without trying to rediscover a package
    /// that no longer exists. Ordinary graph reads never take this route.
    fn resolve_profile_graph_with_applied(
        &self,
        selected: &[ProfilePackage],
        applied: BTreeMap<ProfileId, ProfilePackage>,
    ) -> Result<ResolvedProfileGraph> {
        let mut packages = BTreeMap::new();
        let mut selected_ids = BTreeSet::new();
        let mut pending = VecDeque::new();

        for package in selected {
            selected_ids.insert(package.model().id.clone());
            insert_candidate_package(&mut packages, package.clone())?;
        }
        pending.extend(selected_ids.iter().cloned());

        // Load the selected closure first. A package explicitly obtained beside
        // a selector is the candidate for that dependency; an applied record is
        // the fallback when no such sibling exists.
        self.load_package_dependencies(&mut packages, &applied, &mut pending)?;

        // Applied packages that were not selected still participate in every
        // graph decision. A selected closure candidate with the same id is the
        // deliberate replacement; two different candidates discovered for one
        // id never win by occurrence order.
        for (id, package) in applied {
            if packages.contains_key(&id) {
                continue;
            }
            insert_candidate_package(&mut packages, package)?;
            pending.push_back(id);
        }
        self.load_package_dependencies(&mut packages, &BTreeMap::new(), &mut pending)?;

        let engine = EngineVersion::running()
            .map_err(|source| anyhow::anyhow!("running JIT engine version is invalid: {source}"))?;
        ResolvedProfileGraph::resolve(packages, selected_ids, &engine).map_err(profile_graph_error)
    }

    /// Load every package the repository's applied records currently name.
    fn resolve_applied_profile_packages(&self) -> Result<BTreeMap<ProfileId, ProfilePackage>> {
        self.resolve_applied_profile_packages_except(&BTreeSet::new())
    }

    /// Resolve current records for one mutating application while excluding
    /// provenance-only Embedded records. The application image still supplies
    /// their ownership claims; this does not synthesize configuration from them.
    fn resolve_applied_profile_packages_except(
        &self,
        excluded: &BTreeSet<String>,
    ) -> Result<BTreeMap<ProfileId, ProfilePackage>> {
        let layout = self.require_layout()?;
        with_mutation_session(self.storage(), &layout, "profile graph read", |session| {
            let Some((image, ids)) = capture_applied_records(session, &VirtualPath::PROFILES)?
            else {
                return Ok(SessionStep::Retry);
            };
            let mut packages = BTreeMap::new();
            for id in ids {
                if excluded.contains(&id) {
                    continue;
                }
                let record_path = applied_record_path(&id)?;
                let record = read_applied_record(&image, &record_path, &id)?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "applied profile record '{}' disappeared while resolving the profile graph",
                        record_path.repository_relative()
                    )
                })?;
                let package = recorded_package(&record, &record_path, &layout)?;
                if package.model().id.as_str() != id {
                    return Err(anyhow::anyhow!(
                        "applied profile record '{id}' names package '{}'",
                        package.model().id
                    ));
                }
                insert_candidate_package(&mut packages, package)?;
            }
            Ok(SessionStep::Done(packages))
        })
    }

    fn provenance_only_profile_ids_for_mutation(&self) -> Result<BTreeSet<String>> {
        let layout = self.require_layout()?;
        with_mutation_session(
            self.storage(),
            &layout,
            "profile application provenance",
            |session| {
                let Some((image, ids)) = capture_applied_records(session, &VirtualPath::PROFILES)?
                else {
                    return Ok(SessionStep::Retry);
                };
                let provenance_only = ids
                    .into_iter()
                    .map(|id| {
                        let path = applied_record_path(&id)?;
                        let RepositoryEntry::File { bytes, .. } = image.entry(&path)? else {
                            return Ok(None);
                        };
                        let embedded_current =
                            match serde_json::from_slice::<AppliedProfileRecord>(bytes) {
                                Ok(record) => {
                                    record.id.as_str() == id
                                        && matches!(record.origin, ProfileOrigin::Embedded)
                                }
                                Err(_) => false,
                            };
                        Ok((embedded_current
                            || crate::repository_state::is_shipped_v1_candidate(bytes))
                        .then_some(id))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect::<BTreeSet<_>>();
                Ok(SessionStep::Done(provenance_only))
            },
        )
    }

    fn resolve_profile_closure_for_mutation(
        &self,
        package: &ProfilePackage,
    ) -> Result<Vec<ProfilePackage>> {
        self.resolve_profile_graph_for_mutation(std::slice::from_ref(package))
            .map(|graph| graph.selected_packages())
    }

    /// Resolve a package graph for a pending repository mutation.
    ///
    /// Current embedded records and the named shipped-v1 boundary remain
    /// ownership evidence in the materialization image, but do not describe
    /// rediscoverable packages. Ordinary graph readers intentionally continue
    /// through [`Self::resolve_profile_graph`] and reject those records.
    pub(crate) fn resolve_profile_graph_for_mutation(
        &self,
        selected: &[ProfilePackage],
    ) -> Result<ResolvedProfileGraph> {
        let provenance_only = self.provenance_only_profile_ids_for_mutation()?;
        let applied = self.resolve_applied_profile_packages_except(&provenance_only)?;
        self.resolve_profile_graph_with_applied(selected, applied)
    }

    /// Extend `packages` with dependencies in canonical id order.
    fn load_package_dependencies(
        &self,
        packages: &mut BTreeMap<ProfileId, ProfilePackage>,
        applied: &BTreeMap<ProfileId, ProfilePackage>,
        pending: &mut VecDeque<ProfileId>,
    ) -> Result<()> {
        while let Some(id) = pending.pop_front() {
            let declaring = packages
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("profile '{id}' left the graph being built"))?;
            let mut dependencies = declaring.model().dependencies.clone();
            dependencies.sort_by(|left, right| {
                left.id
                    .cmp(&right.id)
                    .then_with(|| left.version.cmp(&right.version))
            });
            for dependency in dependencies {
                if packages.contains_key(&dependency.id) {
                    continue;
                }
                let package = self
                    .resolve_dependency_package_from(&declaring, dependency.id.as_str(), applied)
                    .map_err(|cause| ProfileDependencyError::UnresolvableDependency {
                        package: id.to_string(),
                        dependency: dependency.id.to_string(),
                        cause,
                    })?;
                let dependency_id = package.model().id.clone();
                insert_candidate_package(packages, package)?;
                pending.push_back(dependency_id);
            }
        }
        Ok(())
    }

    /// Resolve a dependency from a sibling package, then from the coherent
    /// applied-package map, and finally from the record route for callers that
    /// are resolving an individual closure.
    ///
    /// A package read from a directory states where its dependencies are by
    /// where it sits: the dependency is looked for beside it, in a directory
    /// named by the dependency's own id. A directory that is not there, or that
    /// holds a package declaring another profile, is not that dependency, so
    /// resolution continues through the applied-profile record route. A
    /// directory that is there and cannot be read as a package is reported
    /// rather than passed over, because falling through would answer with a
    /// package the adopter did not put there.
    fn resolve_dependency_package_from(
        &self,
        declaring: &ProfilePackage,
        dependency: &str,
        applied: &BTreeMap<ProfileId, ProfilePackage>,
    ) -> Result<ProfilePackage> {
        let ProfilePackageSource::Directory(directory) = declaring.source();
        let fallback = || -> Result<ProfilePackage> {
            if let Some(package) = applied
                .iter()
                .find_map(|(id, package)| (id.as_str() == dependency).then_some(package))
            {
                Ok(package.clone())
            } else {
                Ok(self.resolve_profile_package(&ProfileSelector::id(dependency)?)?)
            }
        };
        let Some(location) = directory.parent().map(|parent| parent.join(dependency)) else {
            return fallback();
        };
        match supplied_package(&location, &self.require_layout()?) {
            Ok(package) if package.model().id.as_str() == dependency => Ok(package),
            Ok(_) => fallback(),
            Err(error) if is_unreadable_location(&error) => fallback(),
            Err(error) => Err(error),
        }
    }

    /// Prove one profile selection resolves without mutating a repository.
    ///
    /// Initialization runs this before it publishes anything, so an
    /// unresolvable id or location fails before a repository is created. What a
    /// selection resolves to is the whole set applying it applies, so an
    /// unresolvable dependency and a dependency cycle fail here too rather than
    /// at the publication the check exists to precede.
    pub fn validate_profile_selection(&self, selectors: &[ProfileSelector]) -> Result<()> {
        let selected = self.resolve_profile_selectors(selectors)?;
        if selected.is_empty() {
            Ok(())
        } else {
            self.resolve_profile_graph(&selected).map(drop)
        }
    }

    /// Prove a pending profile mutation can resolve without accepting embedded
    /// provenance as a package source. Strict validation remains available
    /// through [`Self::validate_profile_selection`].
    pub fn validate_profile_selection_for_mutation(
        &self,
        selectors: &[ProfileSelector],
    ) -> Result<()> {
        let selected = self.resolve_profile_selectors(selectors)?;
        if selected.is_empty() {
            Ok(())
        } else {
            self.resolve_profile_graph_for_mutation(&selected).map(drop)
        }
    }

    /// Apply one validated package together with the packages it depends on.
    ///
    /// The whole closure is resolved and ordered first
    /// ([`resolve_profile_closure`](Self::resolve_profile_closure)), so a cycle
    /// or an unresolvable dependency fails before a plan is published. The
    /// dependency-first closure enters the aggregate application seam once.
    pub fn apply_profile_package(
        &self,
        package: &ProfilePackage,
    ) -> Result<ProfileComposedApplyResult> {
        self.apply_profile_package_with_inputs(package, &VariableInputs::default())
    }

    /// Apply one package closure with already captured variable inputs.
    ///
    /// The closure is one aggregate application. Its scoped context gives every
    /// affected record complete shared ownership evidence on the first run.
    pub fn apply_profile_package_with_inputs(
        &self,
        package: &ProfilePackage,
        inputs: &VariableInputs,
    ) -> Result<ProfileComposedApplyResult> {
        let packages = self.resolve_profile_closure_for_mutation(package)?;
        validate_variable_inputs(&packages, inputs)?;
        let contribution_context = self.profile_contribution_candidates(&packages, inputs)?;
        self.apply_profile_selection(&packages, inputs, &contribution_context)
    }

    /// Publish the complete dependency-first unique closure through one
    /// recovered session and one `session.apply` call at most.
    pub(super) fn apply_profile_selection(
        &self,
        packages: &[ProfilePackage],
        inputs: &VariableInputs,
        contribution_context: &[ProfileContributionClaim],
    ) -> Result<ProfileComposedApplyResult> {
        let resolved = packages
            .iter()
            .map(|package| {
                resolve_package(
                    package,
                    &inputs.for_declarations(&package.model().variables),
                )
                .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        self.apply_resolved_profile_selection(
            packages,
            &resolved,
            contribution_context,
            ProfileLifecycleOperation::Apply,
        )
    }

    /// Publish already resolved package candidates through the one aggregate
    /// profile-selection transaction.
    fn apply_resolved_profile_selection(
        &self,
        packages: &[ProfilePackage],
        resolved: &[ResolvedProfileContent],
        contribution_context: &[ProfileContributionClaim],
        operation: ProfileLifecycleOperation,
    ) -> Result<ProfileComposedApplyResult> {
        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_session(self.storage(), &layout, "profile application", |session| {
            let Some(plan) = self.prepare_profile_selection(
                session,
                packages,
                resolved,
                contribution_context,
                &context,
                operation,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            if plan.delta().actions().is_empty() {
                return Ok(SessionStep::Done(ProfileComposedApplyResult::new(
                    packages
                        .iter()
                        .map(|package| ProfileApplyResult {
                            id: package.model().id.to_string(),
                            version: package.model().version.clone(),
                            status: ProfileApplicationStatus::Unchanged,
                            plan_hash: plan.hash().to_string(),
                            transaction_id: None,
                            warnings: Vec::new(),
                        })
                        .collect(),
                )));
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

            let results = packages
                .iter()
                .map(|package| {
                    let changed = plan.applied_profiles().contains(&package.model().id);
                    Ok(ProfileApplyResult {
                        id: package.model().id.to_string(),
                        version: package.model().version.clone(),
                        status: if changed {
                            ProfileApplicationStatus::Applied
                        } else {
                            ProfileApplicationStatus::Unchanged
                        },
                        plan_hash: plan.hash().to_string(),
                        transaction_id: changed.then(|| plan.hash().to_string()),
                        warnings: Vec::new(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SessionStep::Apply(
                plan,
                ProfileComposedApplyResult::new(results),
            ))
        })
    }

    /// Preview an aggregate lifecycle selection through the same planner used
    /// by durable publication.
    fn plan_resolved_profile_selection(
        &self,
        selected: &[ProfilePackage],
        packages: &[ProfilePackage],
        resolved: &[ResolvedProfileContent],
        contribution_context: &[ProfileContributionClaim],
        operation: ProfileLifecycleOperation,
    ) -> Result<ProfilePlanResult> {
        let layout = self.require_layout()?;
        let context = MutationContext::preview();
        with_mutation_session(self.storage(), &layout, "profile planning", |session| {
            let Some(plan) = self.prepare_profile_selection(
                session,
                packages,
                resolved,
                contribution_context,
                &context,
                operation,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            let targets = profile_target_changes(&plan);
            let plans = selected
                .iter()
                .map(|package| ProfilePlanEntry {
                    id: package.model().id.to_string(),
                    version: package.model().version.clone(),
                    status: if plan.delta().actions().is_empty() {
                        ProfilePlanStatus::Unchanged
                    } else {
                        ProfilePlanStatus::WouldApply
                    },
                    plan_hash: plan.hash().to_string(),
                    targets: targets.clone(),
                })
                .collect();
            Ok(SessionStep::Done(ProfilePlanResult::new(plans)))
        })
    }

    /// Test-only single-member view of the aggregate preparation path.
    #[cfg(test)]
    fn prepare_profile(
        &self,
        session: &mut (dyn RepositoryMutationSession + '_),
        package: &ProfilePackage,
        context: &MutationContext,
    ) -> Result<Option<(MaterializationPlan, Vec<ProfileTargetChange>)>> {
        let resolved = resolve_package(package, &VariableInputs::default())?;
        let contribution_context = self.profile_contribution_candidates(
            std::slice::from_ref(package),
            &VariableInputs::default(),
        )?;
        let plan = self.prepare_profile_selection(
            session,
            std::slice::from_ref(package),
            std::slice::from_ref(&resolved),
            &contribution_context,
            context,
            ProfileLifecycleOperation::Apply,
        )?;
        Ok(plan.map(|plan| {
            let changes = profile_target_changes(&plan);
            (plan, changes)
        }))
    }

    /// Capture and close the complete selection once. A retry restarts from a
    /// fresh held session image; no member of the selection is published until
    /// the final aggregate plan reaches `session.apply`.
    fn prepare_profile_selection(
        &self,
        session: &mut (dyn RepositoryMutationSession + '_),
        packages: &[ProfilePackage],
        resolved: &[ResolvedProfileContent],
        contribution_context: &[ProfileContributionClaim],
        context: &MutationContext,
        operation: ProfileLifecycleOperation,
    ) -> Result<Option<MaterializationPlan>> {
        if packages.len() != resolved.len() {
            anyhow::bail!("profile selection packages and resolved content differ in length");
        }
        for package in packages {
            reject_reserved_application_targets(
                package.hashes().targets.keys().map(String::as_str),
            )?;
        }
        let profiles_dir = VirtualPath::PROFILES;
        let events_path = VirtualPath::EVENTS;
        let layout = self.require_layout()?;

        let mut content_paths = packages
            .iter()
            .flat_map(|package| package.hashes().targets.keys())
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
        content_paths.extend(
            packages
                .iter()
                .map(|package| applied_record_path(package.model().id.as_str()))
                .collect::<Result<Vec<_>>>()?,
        );
        content_paths.extend(crate::repository_state::profile_contribution_target_paths(
            contribution_context,
        )?);
        content_paths.push(profiles_dir.clone());
        content_paths.push(events_path.clone());

        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &content_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };

        // Candidate detection is intentionally shallow: the exact five-field
        // decoder runs below only after this same session has captured every
        // pinned historical unit it must authenticate.
        let mut migration_paths = Vec::new();
        let has_candidate = recorded_profile_ids(&base, &profiles_dir)?
            .into_iter()
            .map(|id| applied_record_path(&id))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|path| {
                Ok(matches!(
                    base.entry(&path)?,
                    RepositoryEntry::File { bytes, .. }
                        if crate::repository_state::is_shipped_v1_candidate(bytes)
                ))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .any(|candidate| candidate);
        if has_candidate {
            migration_paths = crate::repository_state::shipped_v1_migration_paths()?
                .into_iter()
                .map(|path| layout.classify_repository_relative(&path))
                .collect::<Result<Vec<_>, _>>()?;
        }
        let mut authenticated_paths = content_paths.clone();
        let base = if migration_paths.is_empty() {
            base
        } else {
            authenticated_paths.extend(migration_paths);
            match self.capture_proposed_base(
                session,
                &BTreeMap::new(),
                &authenticated_paths,
                None,
            )? {
                None => return Ok(None),
                Some(base) => base,
            }
        };
        // This is the sole exact legacy decode/conversion for this operation.
        // The authenticated rewrite map stays immutable through closure,
        // preview, and final derivation; final delta preimages revalidate the
        // raw record bytes before publication.
        let migrations = crate::repository_state::migrate_shipped_v1_records(&base)?;
        let inputs = profile_selection_inputs(
            packages,
            resolved,
            base.layout(),
            contribution_context,
            &migrations,
        )?;
        let migration_base = migration_overlay(&base, &migrations)?;
        let contribution_base = apply_overlay(
            &migration_base,
            crate::repository_state::profile_contribution_overrides(
                &migration_base,
                contribution_context,
            )?,
        )?;
        let mut expanded_paths = authenticated_paths;
        for input in &inputs {
            expanded_paths.extend(crate::repository_state::profile_capture_closure(
                &contribution_base,
                input,
            )?);
        }
        // A package that re-derives the default rules reaches the schemas they
        // reference, and reaching a schema that is absent means proving its
        // absence against the directory listing, so the directory is discovered
        // whether or not the package names a target under it.
        if inputs
            .iter()
            .any(ProfileApplicationInput::owns_default_rule_authority)
        {
            expanded_paths.push(VirtualPath::SCHEMAS);
        }
        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &expanded_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };
        let inputs = profile_selection_inputs(
            packages,
            resolved,
            base.layout(),
            contribution_context,
            &migrations,
        )?;
        let preview = derive_materialization(
            &base,
            MaterializationRequest::ApplyProfileSelection {
                profiles: inputs,
                context,
                operation,
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
        let inputs = profile_selection_inputs(
            packages,
            resolved,
            probe.layout(),
            contribution_context,
            &migrations,
        )?;
        let migration_probe = migration_overlay(&probe, &migrations)?;
        let contribution_probe = apply_overlay(
            &migration_probe,
            crate::repository_state::profile_contribution_overrides(
                &migration_probe,
                contribution_context,
            )?,
        )?;
        for input in &inputs {
            let final_closure =
                crate::repository_state::profile_capture_closure(&contribution_probe, input)?;
            if final_closure
                .iter()
                .any(|path| !probe.capture_spec().contains_path(path))
            {
                return Ok(None);
            }
        }
        let plan = derive_materialization(
            &probe,
            MaterializationRequest::ApplyProfileSelection {
                profiles: inputs,
                context,
                operation,
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
        Ok(Some(plan))
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

/// Project a prepared plan's owned targets into the public dry-run vocabulary.
///
/// Every profile command that reports targets reads them from the plan it
/// prepared, so what an adopter is shown and what the transaction would publish
/// come from one derivation.
fn profile_target_changes(plan: &MaterializationPlan) -> Vec<ProfileTargetChange> {
    plan.profile_targets()
        .iter()
        .map(|target| {
            let action = match target.disposition {
                ProfileTargetDisposition::Unchanged => ProfileTargetAction::Unchanged,
                ProfileTargetDisposition::Create => ProfileTargetAction::Create,
                ProfileTargetDisposition::Update => ProfileTargetAction::Update,
            };
            ProfileTargetChange::new(target.path.repository_relative(), action, target.mode)
        })
        .collect()
}

/// Insert one package image without allowing selector or dependency traversal
/// order to choose between different images for one semantic profile id.
fn insert_candidate_package(
    packages: &mut BTreeMap<ProfileId, ProfilePackage>,
    package: ProfilePackage,
) -> Result<()> {
    let id = package.model().id.clone();
    if let Some(existing) = packages.get(&id) {
        if existing.hashes().package != package.hashes().package {
            let (first_hash, second_hash) = if existing.hashes().package < package.hashes().package
            {
                (
                    existing.hashes().package.clone(),
                    package.hashes().package.clone(),
                )
            } else {
                (
                    package.hashes().package.clone(),
                    existing.hashes().package.clone(),
                )
            };
            return Err(ProfileDependencyError::ConflictingPackageIdentity {
                id: id.to_string(),
                first_hash,
                second_hash,
            }
            .into());
        }
        if package_source_key(&package) < package_source_key(existing) {
            packages.insert(id, package);
        }
        return Ok(());
    }
    packages.insert(id, package);
    Ok(())
}

/// Refuse a reconfiguration that would replace package provenance instead of
/// replaying the installed package with different values.
fn ensure_reconfiguration_package_identity(
    package: &ProfilePackage,
    record: &AppliedProfileRecord,
) -> Result<()> {
    let metadata = package.model();
    let unchanged_identity = record.id == metadata.id
        && record.version == metadata.version
        && record.compatible_jit == metadata.compatible_jit
        && record.package_hash == package.hashes().package;
    unchanged_identity.then_some(()).ok_or_else(|| {
        ProfileApplyError::ReconfigurationPackageChanged {
            id: metadata.id.to_string(),
            installed_version: record.version.clone(),
            installed_hash: record.package_hash.clone(),
            found_version: metadata.version.clone(),
            found_hash: package.hashes().package.clone(),
        }
        .into()
    })
}

/// Require a changed replacement package to advance semantic version. An exact
/// package replay is a valid no-op upgrade rehearsal or invocation.
fn ensure_upgrade_version_is_newer_when_replaced(
    package: &ProfilePackage,
    record: &AppliedProfileRecord,
) -> Result<()> {
    let metadata = package.model();
    if record.package_hash == package.hashes().package && record.version == metadata.version {
        return Ok(());
    }
    let installed = Version::parse(&record.version).map_err(|_| {
        anyhow::anyhow!(
            "installed profile '{}' records invalid version '{}'",
            record.id,
            record.version
        )
    })?;
    let candidate = Version::parse(&metadata.version).map_err(|_| {
        anyhow::anyhow!(
            "replacement profile '{}' declares invalid version '{}'",
            metadata.id,
            metadata.version
        )
    })?;
    (candidate > installed).then_some(()).ok_or_else(|| {
        ProfileApplyError::UpgradeVersionNotNewer {
            id: metadata.id.to_string(),
            installed_version: record.version.clone(),
            candidate_version: metadata.version.clone(),
        }
        .into()
    })
}

/// Stable source spelling used only to choose between byte-identical package
/// copies. Package bytes and graph claims remain the semantic identity.
fn package_source_key(package: &ProfilePackage) -> String {
    match package.source() {
        ProfilePackageSource::Directory(path) => path.to_string_lossy().into_owned(),
    }
}

/// Convert the pure graph's typed refusal into the command-layer dependency
/// error vocabulary, retaining every package named by the failed claim.
fn profile_graph_error(error: ProfileGraphError) -> anyhow::Error {
    match error {
        ProfileGraphError::MissingDependency {
            package,
            dependency,
        } => ProfileDependencyError::UnresolvableDependency {
            package: package.clone(),
            dependency: dependency.clone(),
            cause: anyhow::anyhow!("profile graph is missing participating package '{dependency}'"),
        }
        .into(),
        ProfileGraphError::DependencyVersionMismatch {
            package,
            dependency,
            required,
            found,
        } => ProfileDependencyError::DependencyVersionMismatch {
            package,
            dependency,
            required,
            found,
        }
        .into(),
        ProfileGraphError::DependencyCycle { cycle } => {
            ProfileDependencyError::DependencyCycle { cycle }.into()
        }
        ProfileGraphError::IncompatibleEngine {
            package,
            required,
            actual,
        } => ProfileDependencyError::IncompatibleEngine {
            package,
            required,
            actual,
        }
        .into(),
        ProfileGraphError::IncompatiblePackages {
            package,
            other,
            requirement,
            other_version,
        } => ProfileDependencyError::IncompatiblePackages {
            package,
            other,
            requirement,
            other_version,
        }
        .into(),
        other => anyhow::anyhow!(other),
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
/// The package there supplies the id for a `path:` selector. The caller may
/// combine that selector with `id:` selectors, but a path package may not
/// shadow a selected recorded id.
fn supplied_package(location: &Path, layout: &RepositoryLayout) -> Result<ProfilePackage> {
    let requested = if location.is_absolute() {
        location.to_path_buf()
    } else {
        layout.worktree_root().join(location)
    };
    let package = match fs::canonicalize(&requested) {
        Ok(_) => {
            ensure_worktree_package_directory(&requested, layout)?;
            ProfilePackage::from_directory(&requested)
        }
        Err(_) => ProfilePackage::from_directory(&requested),
    }
    .map_err(|source| ProfileResolutionError::UnreadableLocation {
        location: location.display().to_string(),
        source,
    })?;
    Ok(package)
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
    let ProfileOrigin::Directory(location) = &record.origin else {
        return Err(ProfileResolutionError::EmbeddedProvenanceUnavailable {
            record: record_path.repository_relative(),
        }
        .into());
    };
    let requested = layout.worktree_root().join(location.as_path());
    let package = match fs::canonicalize(&requested) {
        Ok(_) => {
            ensure_worktree_package_directory(&requested, layout)?;
            ProfilePackage::from_directory(&requested)
        }
        Err(_) => ProfilePackage::from_directory(&requested),
    };
    package.map_err(|source| {
        ProfileResolutionError::UnresolvableRecordedLocation {
            record: record_path.repository_relative(),
            location: location.as_path().display().to_string(),
            source,
        }
        .into()
    })
}

/// Reuse the repository layout's canonical root classification before reading a
/// package directory. This is deliberately the same worktree/data-root
/// boundary used for every persisted package origin.
fn ensure_worktree_package_directory(
    directory: &Path,
    layout: &RepositoryLayout,
) -> Result<(), ProfileApplyError> {
    let path = layout.classify_and_canonicalize(directory).map_err(|_| {
        ProfileApplyError::PackageOutsideWorktree {
            path: directory.display().to_string(),
        }
    })?;
    if path.root_class() != RepositoryRootClass::Worktree {
        return Err(ProfileApplyError::PackageOutsideWorktree {
            path: directory.display().to_string(),
        });
    }
    Ok(())
}

fn is_unreadable_location(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ProfileResolutionError>()
        .is_some_and(|error| {
            matches!(
                error,
                ProfileResolutionError::UnreadableLocation {
                    source: ProfilePackageError::UnreadableDirectory { .. },
                    ..
                }
            )
        })
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
    let metadata = package.model();
    Ok(Some(ProfileSummary {
        id: metadata.id.to_string(),
        version: metadata.version.clone(),
        origin: package_origin(&package, layout)?,
        jit: metadata.compatible_jit.clone(),
        applied: record.matches_package_provenance(&expected_record(
            &package,
            layout,
            &record.variables,
        )?),
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
///
/// Resolved variables are validated against the current package without
/// consulting ambient input. Ownership claims remain repository-state
/// provenance rather than immutable package identity, so the package check
/// deliberately excludes them.
pub(super) fn expected_record(
    package: &ProfilePackage,
    layout: &RepositoryLayout,
    variables: &ResolvedVariables,
) -> Result<AppliedProfileRecord> {
    let metadata = package.model();
    let _resolved = resolve_package_from_record(
        package,
        variables,
        &VariableInputs::default(),
        RecordedValueAuthority::Exact,
    )?;
    Ok(AppliedProfileRecord::new(
        metadata.id.clone(),
        metadata.version.clone(),
        metadata.compatible_jit.clone(),
        package_origin(package, layout)?,
        package.hashes().package.clone(),
        variables.clone(),
        BTreeSet::new(),
    ))
}

/// Build one canonical aggregate input collection. The authenticated migration
/// map belongs to its first dependency-first member only, so one selection
/// converts each shipped-v1 record once while every later member plans against
/// the same proposed current-format image.
fn profile_selection_inputs(
    packages: &[ProfilePackage],
    resolved: &[ResolvedProfileContent],
    layout: &RepositoryLayout,
    contribution_context: &[ProfileContributionClaim],
    migrations: &BTreeMap<VirtualPath, AppliedProfileRecord>,
) -> Result<Vec<ProfileApplicationInput>> {
    packages
        .iter()
        .zip(resolved)
        .enumerate()
        .map(|(index, (package, resolved))| {
            let record_path = applied_record_path(package.model().id.as_str())?;
            let input = profile_application_input(package, resolved, layout, record_path)?
                .with_contribution_context(contribution_context);
            Ok(if index == 0 {
                input.with_shipped_v1_migrations(migrations.clone())
            } else {
                input
            })
        })
        .collect()
}

/// Apply authenticated migration bytes to an in-memory image for later
/// aggregate members. The durable migration actions remain in the final plan.
fn migration_overlay(
    base: &RepositoryImage,
    migrations: &BTreeMap<VirtualPath, AppliedProfileRecord>,
) -> Result<RepositoryImage> {
    apply_overlay(
        base,
        migrations
            .iter()
            .map(|(path, record)| Ok((path.clone(), Some(record.to_bytes()?))))
            .collect::<Result<Vec<_>, serde_json::Error>>()?,
    )
    .map_err(Into::into)
}

/// Convert an immutable package into neutral claims plus provenance metadata.
fn profile_application_input(
    package: &ProfilePackage,
    resolved: &ResolvedProfileContent,
    layout: &RepositoryLayout,
    record_path: VirtualPath,
) -> Result<ProfileApplicationInput> {
    let metadata = resolved.model();
    let claims = build_profile_claims_from_resolved(resolved, layout, false)?;
    Ok(ProfileApplicationInput {
        id: metadata.id.clone(),
        version: metadata.version.clone(),
        compatible_jit: metadata.compatible_jit.clone(),
        package_hash: package.hashes().package.clone(),
        variables: resolved.variables().clone(),
        target_hashes: resolved.target_hashes()?,
        origin: package_origin(package, layout)?,
        contribution_context: claims.contributions.clone(),
        claims,
        shipped_v1_migrations: BTreeMap::new(),
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
                .and_then(|record| {
                    (record.id.as_str() == id).then_some(record).ok_or_else(|| {
                        serde_json::Error::io(std::io::Error::other(
                            "record id does not match its canonical path",
                        ))
                    })
                })
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
    use crate::domain::{Event, ProfileLifecycleOperation, ProfileLifecycleStatus};
    use crate::repository_state::{
        Contribution, ContributionCompositionConflict, ContributionConflictOwner,
        InitializationError, MapEntryTarget, ProfileConflictOccupant, ProfilePackageId,
        ProfileTargetConflictError, ProfileThreeWayConflictError, RepositoryStateError,
        RootRelativePath, ScalarTarget, SetStringTarget,
    };
    use crate::storage::{
        discover_repository_layout, IssueStore, RepositoryStateStore, RepositoryStateStoreError,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
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

    /// Observe actual recoverable publications without coupling the test to
    /// preparatory graph or capture sessions. This transaction-kernel edge runs
    /// once for each delta that reaches `session.apply`.
    struct PublicationCounter(AtomicUsize);

    impl PublicationCounter {
        fn new() -> Arc<Self> {
            Arc::new(Self(AtomicUsize::new(0)))
        }

        fn count(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl crate::storage::TransactionFailureInjector for PublicationCounter {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
            if point == &crate::storage::TransactionFailurePoint::RepositoryBeforeControlCreation {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    /// Fail one named kernel boundary, then let the next recovered session run.
    struct FailOnce {
        point: crate::storage::TransactionFailurePoint,
        fired: AtomicBool,
    }

    impl FailOnce {
        fn at(point: crate::storage::TransactionFailurePoint) -> Arc<Self> {
            Arc::new(Self {
                point,
                fired: AtomicBool::new(false),
            })
        }
    }

    impl crate::storage::TransactionFailureInjector for FailOnce {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
            if point == &self.point && !self.fired.swap(true, Ordering::SeqCst) {
                return Err(std::io::Error::other(format!("injected {point:?}")));
            }
            Ok(())
        }
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
            .model()
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

    /// A v2 package with explicit dependency, incompatibility, and engine
    /// range declarations for graph-resolution tests.
    fn package_v2(
        temp: &TempDir,
        relative: &str,
        id: &str,
        version: &str,
        compatible_jit: &str,
        dependencies: &[(&str, &str)],
        incompatibilities: &[(&str, &str)],
    ) -> ProfilePackage {
        let tree = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &temp.path().join(relative),
        );
        let source = ProfilePackage::parse_manifest(
            &fs::read(tree.join(crate::profile::MANIFEST_FILE_NAME)).unwrap(),
        )
        .expect("the source package manifest parses");
        let asset = source
            .assets
            .first()
            .expect("the fixture declares an asset");
        let mut manifest = format!(
            "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"{version}\"\ncompatible-jit = \"{compatible_jit}\"\n"
        );
        for (dependency, requirement) in dependencies {
            manifest.push_str(&format!(
                "\n[[dependency]]\nid = \"{dependency}\"\nversion = \"{requirement}\"\n"
            ));
        }
        for (incompatible, requirement) in incompatibilities {
            manifest.push_str(&format!(
                "\n[[incompatibility]]\nid = \"{incompatible}\"\nversion = \"{requirement}\"\n"
            ));
        }
        manifest.push_str(&format!(
            "\n[[asset]]\nsource = \"{}\"\ntarget = \"docs/{id}.txt\"\n",
            asset.source
        ));
        fs::write(tree.join(crate::profile::MANIFEST_FILE_NAME), manifest)
            .expect("write the v2 graph manifest");
        ProfilePackage::from_directory(&tree).expect("the v2 graph package is valid")
    }

    /// Select a package through the worktree location it was read from.
    fn package_selector(package: &ProfilePackage) -> ProfileSelector {
        let ProfilePackageSource::Directory(path) = package.source();
        ProfileSelector::path(path.clone())
    }

    /// The profile ids this repository's audit log records as applied, in the
    /// order it recorded them.
    fn applied_event_ids(storage: &JsonFileStorage) -> Vec<String> {
        storage
            .read_events()
            .unwrap()
            .into_iter()
            .flat_map(|event| match event {
                Event::ProfileLifecycle { profiles, .. } => profiles
                    .into_iter()
                    .map(|profile| profile.id.to_string())
                    .collect(),
                _ => Vec::new(),
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
                &format!("id = \"{}\"", source.id),
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

    /// Rewrite an installed test package to stop contributing every asset.
    fn package_without_assets(temp: &TempDir, relative: &str, id: &str) -> ProfilePackage {
        fs::remove_file(temp.path().join(relative).join("assets/profile.txt"))
            .expect("remove the former asset source");
        repackage(
            temp,
            relative,
            crate::profile::MANIFEST_FILE_NAME,
            &format!(
                "[profile]\nmanifest-version = 1\nid = \"{id}\"\nversion = \"2.0.0\"\njit = \">=0.2.0, <2.0.0\"\n"
            ),
        )
    }

    /// A package tree at `relative` holding `manifest` and exactly the named
    /// `sources`, read back from there.
    ///
    /// A lifecycle scenario turns on exactly what a manifest declares — its
    /// version, its variables, which assets a variable reaches — and a package
    /// admits no file its manifest does not declare, so these cases author the
    /// whole tree they mean instead of staging a fixture whose own asset the
    /// authored manifest would have to keep declaring. The one builder is
    /// shared by every case below (`@/invariant/shared-test-contracts`).
    fn authored_manifest_package(
        temp: &TempDir,
        relative: &str,
        manifest: &str,
        sources: &[(&str, &str)],
    ) -> ProfilePackage {
        let tree = temp.path().join(relative);
        if tree.exists() {
            fs::remove_dir_all(&tree).expect("clear a re-authored package tree");
        }
        fs::create_dir_all(&tree).expect("create the package root");
        fs::write(tree.join(crate::profile::MANIFEST_FILE_NAME), manifest)
            .expect("write the authored manifest");
        for (source, content) in sources {
            let path = tree.join(source);
            fs::create_dir_all(path.parent().expect("a package source has a directory"))
                .expect("create the package source directory");
            fs::write(path, content).expect("write the package source");
        }
        ProfilePackage::from_directory(&tree).expect("a valid authored package tree")
    }

    /// Profile id every lifecycle case below reconfigures or upgrades.
    const LIFECYCLE_ID: &str = "lifecycle-package";

    /// Worktree location the lifecycle package is staged and re-read at.
    const LIFECYCLE_LOCATION: &str = "packages/lifecycle";

    /// The lifecycle package at `version`, declaring one variable that reaches
    /// one asset and leaves the other alone.
    ///
    /// Separating a target the variable feeds from a target it does not is what
    /// makes "republish what the value affects" observable as distinct from
    /// "republish everything this package owns".
    fn lifecycle_package(temp: &TempDir, version: &str) -> ProfilePackage {
        authored_manifest_package(
            temp,
            LIFECYCLE_LOCATION,
            &format!(
                "[profile]\nmanifest-version = 2\nid = \"{LIFECYCLE_ID}\"\n\
                 version = \"{version}\"\ncompatible-jit = \"*\"\n\n\
                 [[variable]]\nname = \"GREETING\"\ndefault = \"authored\"\n\n\
                 [[asset]]\nsource = \"assets/templated.txt\"\n\
                 target = \"docs/templated.txt\"\ntemplate = true\n\n\
                 [[asset]]\nsource = \"assets/fixed.txt\"\ntarget = \"docs/fixed.txt\"\n"
            ),
            &[
                ("assets/templated.txt", "greeting={{jit:var:GREETING}}\n"),
                ("assets/fixed.txt", "no variable reaches this\n"),
            ],
        )
    }

    /// Command options carrying repeated `--set NAME=VALUE` assignments.
    fn supplied_values(assignments: &[(&str, &str)]) -> ProfileVariableOptions {
        ProfileVariableOptions {
            values_file: None,
            assignments: assignments
                .iter()
                .map(|(name, value)| {
                    ProfileVariableAssignment::new(
                        (*name).try_into().expect("a canonical variable name"),
                        *value,
                    )
                })
                .collect(),
        }
    }

    /// Apply `package` through the ordinary command entry point.
    fn apply_package(
        executor: &CommandExecutor<JsonFileStorage>,
        package: &ProfilePackage,
        options: &ProfileVariableOptions,
    ) -> ProfileComposedApplyResult {
        executor
            .apply_profile_from_sources(&[package_selector(package)], options)
            .expect("the package applies")
    }

    /// Select an installed profile the way a lifecycle command addresses it:
    /// through the record that names where its package is read from.
    fn installed_selector(id: &str) -> Vec<ProfileSelector> {
        vec![ProfileSelector::id(id).expect("a canonical profile id")]
    }

    /// The fingerprint a record carries for the asset it published at `target`.
    ///
    /// This is the base of the next three-way decision, so a target that was
    /// left alone must carry the same one it carried before.
    fn recorded_asset_fingerprint(record: &AppliedProfileRecord, target: &str) -> String {
        record
            .claims
            .iter()
            .find_map(|claim| match &claim.identity {
                crate::repository_state::AppliedProfileClaimIdentity::Asset { target: claimed } => {
                    (claimed.path.as_path() == Path::new(target))
                        .then(|| claim.base_fingerprint.as_str().to_string())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the record claims an asset at {target}"))
    }

    /// Every file this repository holds, keyed by its path below the worktree.
    ///
    /// A rehearsal writes nothing, which is a statement about the whole
    /// repository rather than about the targets a plan happens to name.
    fn repository_files(temp: &TempDir) -> BTreeMap<PathBuf, Vec<u8>> {
        fn collect(directory: &Path, root: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(directory).expect("read a repository directory") {
                let path = entry.expect("a readable directory entry").path();
                if path.is_dir() {
                    collect(&path, root, files);
                } else {
                    files.insert(
                        path.strip_prefix(root)
                            .expect("every file sits below the worktree")
                            .to_path_buf(),
                        fs::read(&path).expect("read a repository file"),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        collect(temp.path(), temp.path(), &mut files);
        files
    }

    /// The lifecycle operation and per-profile statuses of the last audited
    /// profile event.
    fn last_lifecycle_event(
        storage: &JsonFileStorage,
    ) -> (
        ProfileLifecycleOperation,
        Vec<(String, ProfileLifecycleStatus)>,
    ) {
        storage
            .read_events()
            .expect("the audit log is readable")
            .into_iter()
            .filter_map(|event| match event {
                Event::ProfileLifecycle {
                    operation,
                    profiles,
                    ..
                } => Some((
                    operation,
                    profiles
                        .into_iter()
                        .map(|profile| (profile.id.to_string(), profile.status))
                        .collect(),
                )),
                _ => None,
            })
            .next_back()
            .expect("a lifecycle mutation audits one profile event")
    }

    /// Number of profile lifecycle events this repository has audited.
    fn lifecycle_event_count(storage: &JsonFileStorage) -> usize {
        storage
            .read_events()
            .expect("the audit log is readable")
            .into_iter()
            .filter(|event| matches!(event, Event::ProfileLifecycle { .. }))
            .count()
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

    fn package_declaring_contribution(
        temp: &TempDir,
        relative: &str,
        id: &str,
        dependencies: &[&str],
        namespace: &str,
        description: &str,
    ) -> ProfilePackage {
        package_declaring(temp, relative, id, dependencies);
        let directory = temp.path().join(relative);
        let manifest = directory.join(crate::profile::MANIFEST_FILE_NAME);
        let authored = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            format!(
                "{authored}{}",
                namespace_contribution(namespace, description)
            ),
        )
        .unwrap();
        ProfilePackage::from_directory(&directory).unwrap()
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

    /// The safe-change conflict carried by real profile application.
    fn three_way_conflict(error: &anyhow::Error) -> &ProfileThreeWayConflictError {
        match error.downcast_ref::<RepositoryStateError>() {
            Some(RepositoryStateError::Initialization(
                InitializationError::ProfileThreeWayConflict(conflict),
            )) => conflict,
            _ => panic!("a concurrent profile edit fails as a three-way conflict: {error:#}"),
        }
    }

    /// The semantic-composition conflict carried by real profile application.
    fn contribution_conflict(error: &anyhow::Error) -> &ContributionCompositionConflict {
        match error.downcast_ref::<RepositoryStateError>() {
            Some(
                RepositoryStateError::ContributionComposition(conflict)
                | RepositoryStateError::Initialization(InitializationError::ContributionComposition(
                    conflict,
                )),
            ) => conflict,
            _ => panic!("a colliding contribution fails as a semantic conflict: {error:#}"),
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

    /// Encode the exact historical five-field wire only to prove ordinary
    /// readers do not accept it. The migration boundary owns all legacy input.
    fn shipped_v1_record(record: &AppliedProfileRecord) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": record.id,
            "version": record.version,
            "origin": record.origin,
            "package_hash": record.package_hash,
            "target_hashes": {},
        }))
        .unwrap()
    }

    fn exact_shipped_dogfood_v1_record() -> Vec<u8> {
        let evidence: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../repository_state/shipped_v1_dogfood_evidence.json"
        ))
        .expect("pinned evidence is JSON");
        let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "id": "jit-dogfood",
            "version": "1.0.0",
            "origin": { "source": "embedded" },
            "package_hash": "43829e7e032e5e9ec40776103b1996f15e7291664c8b11e403c20b7f54af905c",
            "target_hashes": evidence["target_hashes"],
        }))
        .expect("v1 fixture serializes");
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn test_resolve_profile_package_reads_the_package_a_supplied_location_holds() {
        let (temp, _storage, executor, _package) = fixture();
        let supplied = package_read_from(&temp, "vendor/supplied");

        let selector = ProfileSelector::path(temp.path().join("vendor/supplied"));
        let resolved = executor.resolve_profile_package(&selector).unwrap();

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

        let selector = ProfileSelector::path(temp.path().join("vendor/supplied"));
        let resolved = executor.resolve_profile_package(&selector).unwrap();

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

        let selector = ProfileSelector::id(fixture_id()).unwrap();
        let resolved = executor.resolve_profile_package(&selector).unwrap();

        assert_eq!(resolved.hashes(), rewritten.hashes());
    }

    #[test]
    fn test_resolve_profile_package_refuses_embedded_provenance_without_restoring_discovery() {
        let (temp, _storage, executor, _package) = fixture();
        let id = fixture_id();
        store_record(
            &temp,
            &AppliedProfileRecord::new(
                id.clone()
                    .try_into()
                    .expect("fixture profile id is canonical"),
                "1.0.0",
                ">=1.0.0, <2.0.0",
                ProfileOrigin::Embedded,
                "a".repeat(64),
                ResolvedVariables::default(),
                BTreeSet::new(),
            ),
        );

        let error = executor
            .resolve_profile_package(&ProfileSelector::id(&id).unwrap())
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ProfileResolutionError>(),
            Some(ProfileResolutionError::EmbeddedProvenanceUnavailable { record })
                if record == &format!(".jit/profiles/{id}.json")
        ));
    }

    #[test]
    fn test_apply_profile_package_keeps_embedded_provenance_as_ownership_only_on_later_mutations() {
        let (temp, _storage, executor, _package) = fixture();
        store_record(
            &temp,
            &AppliedProfileRecord::new(
                "jit-dogfood"
                    .try_into()
                    .expect("fixture profile id is canonical"),
                "1.0.0",
                ">=1.0.0, <2.0.0",
                ProfileOrigin::Embedded,
                "a".repeat(64),
                ResolvedVariables::default(),
                BTreeSet::new(),
            ),
        );
        let later =
            package_publishing(&temp, "vendor/later", "later", "notes/later.txt", "later\n");

        let applied = executor
            .apply_profile_package(&later)
            .expect("a later mutation does not resolve Embedded provenance as configuration");
        assert_eq!(applied.profiles.len(), 1);
        assert!(temp.path().join("notes/later.txt").is_file());
    }

    #[test]
    fn test_apply_profile_from_sources_keeps_embedded_provenance_out_of_mutating_graph_resolution()
    {
        let (temp, _storage, executor, _package) = fixture();
        store_record(
            &temp,
            &AppliedProfileRecord::new(
                "jit-dogfood"
                    .try_into()
                    .expect("fixture profile id is canonical"),
                "1.0.0",
                ">=1.0.0, <2.0.0",
                ProfileOrigin::Embedded,
                "a".repeat(64),
                ResolvedVariables::default(),
                BTreeSet::new(),
            ),
        );

        let result = executor
            .apply_profile_from_sources(
                &[ProfileSelector::path(FIXTURE_LOCATION)],
                &ProfileVariableOptions::default(),
            )
            .expect("a mutation treats Embedded provenance as ownership only");
        assert_eq!(result.profiles.len(), 1);
    }

    #[test]
    fn test_apply_profile_package_authenticates_a_shipped_v1_record_once_before_publication() {
        let (temp, storage, executor, _package) = fixture();
        let raw_v1 = exact_shipped_dogfood_v1_record();
        fs::create_dir_all(temp.path().join(".jit/profiles")).unwrap();
        let record_path = temp.path().join(".jit/profiles/jit-dogfood.json");
        fs::write(&record_path, &raw_v1).unwrap();
        let later =
            package_publishing(&temp, "vendor/later", "later", "notes/later.txt", "later\n");

        crate::repository_state::reset_shipped_v1_conversion_count();
        let error = executor.apply_profile_package(&later).unwrap_err();

        assert!(error.to_string().contains("shipped-v1 migration"));
        assert_eq!(crate::repository_state::shipped_v1_conversion_count(), 1);
        assert_eq!(fs::read(&record_path).unwrap(), raw_v1);
        assert!(!temp.path().join("notes/later.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_retains_an_adopted_managed_region_across_reapplication() {
        let (temp, _storage, executor, _package) = fixture();
        let package_root = temp.path().join("vendor/managed-region");
        fs::create_dir_all(package_root.join("assets")).unwrap();
        fs::write(
            package_root.join("manifest.toml"),
            r#"
[profile]
manifest-version = 2
id = "managed-region"
version = "1.0.0"
compatible-jit = "*"

[[region]]
source = "assets/guidance.md"
target = "AGENTS.md"
region-id = "guidance"
placement = "append"
"#,
        )
        .unwrap();
        fs::write(
            package_root.join("assets/guidance.md"),
            b"Repository guidance.\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("AGENTS.md"),
            b"Repository policy.\n\n<!-- jit:guidance:begin -->\nRepository guidance.\n<!-- jit:guidance:end -->\n",
        )
        .unwrap();
        let package = ProfilePackage::from_directory(&package_root).unwrap();

        executor.apply_profile_package(&package).unwrap();
        let first = record_for(&temp, "managed-region");
        assert!(first.claims.iter().any(|claim| {
            matches!(
                claim.identity,
                crate::repository_state::AppliedProfileClaimIdentity::ManagedRegion { .. }
            ) && claim.retain_if_unowned
        }));

        executor.apply_profile_package(&package).unwrap();
        let second = record_for(&temp, "managed-region");
        assert!(second.claims.iter().any(|claim| {
            matches!(
                claim.identity,
                crate::repository_state::AppliedProfileClaimIdentity::ManagedRegion { .. }
            ) && claim.retain_if_unowned
        }));
    }

    #[test]
    fn test_ordinary_profile_readers_reject_shipped_v1_records() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();
        let id = applied.model().id.to_string();
        let record_path = temp.path().join(format!(".jit/profiles/{id}.json"));
        fs::write(&record_path, shipped_v1_record(&stored_record(&temp))).unwrap();
        let selector = ProfileSelector::id(&id).unwrap();

        assert!(executor.resolve_profile_package(&selector).is_err());
        assert!(executor.list_recorded_profiles().is_err());
        assert!(executor
            .show_profiles(std::slice::from_ref(&selector))
            .is_err());
        assert!(executor.plan_profiles(&[selector]).is_err());
    }

    #[test]
    fn test_resolve_profile_package_reads_the_recorded_package_not_another_declaring_that_id() {
        let (temp, _storage, executor, _package) = fixture();
        let (_workspace, authored) = crate::test_utils::temporary_repository_package("jit-dogfood");
        let id = authored.model().id.to_string();

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
        assert_eq!(recorded.model().id.as_str(), id);
        assert_ne!(recorded.hashes(), authored.hashes());
        store_record(
            &temp,
            &AppliedProfileRecord::new(
                id.clone()
                    .try_into()
                    .expect("fixture profile id is canonical"),
                recorded.model().version.clone(),
                recorded.model().compatible_jit.clone(),
                ProfileOrigin::Directory(RootRelativePath::parse("vendor/dogfood").unwrap()),
                recorded.hashes().package.clone(),
                ResolvedVariables::default(),
                BTreeSet::new(),
            ),
        );

        let selector = ProfileSelector::id(&id).unwrap();
        let resolved = executor.resolve_profile_package(&selector).unwrap();

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
            let selector = ProfileSelector::id(id).unwrap();
            let error = executor.resolve_profile_package(&selector).unwrap_err();

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
    fn test_apply_profile_selection_recovers_an_obsolete_default_schema_atomically() {
        let (temp, executor, _default, dogfood) = bare_repository_beside_shipped_packages();
        executor.apply_profile_package(&dogfood).unwrap();

        let stale_schema = temp
            .path()
            .join(".jit/schemas/default-obsolete-profile-selection.json");
        fs::write(&stale_schema, "{}").unwrap();
        let rules_path = temp.path().join(".jit/rules.toml");
        let rules = fs::read_to_string(&rules_path).unwrap();
        fs::write(
            &rules_path,
            format!(
                "{rules}\n[[rules]]\nname = \"obsolete-profile-selection\"\norigin = \"default\"\nassert = {{ json-schema = \"schemas/default-obsolete-profile-selection.json\" }}\n"
            ),
        )
        .unwrap();
        let events_before = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(
            temp.path().join(".jit"),
            FailOnce::at(
                crate::storage::TransactionFailurePoint::RepositoryAfterAction { action: 0 },
            ),
        );
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());

        assert!(executor.apply_profile_package(&dogfood).is_err());

        let recovered = JsonFileStorage::new(temp.path().join(".jit"));
        let layout = discover_repository_layout(temp.path(), recovered.root()).unwrap();
        drop(recovered.open_mutation_session(layout).unwrap());
        assert!(stale_schema.exists());
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events_before
        );

        let repaired = CommandExecutor::new(recovered.clone())
            .with_layout(discover_repository_layout(temp.path(), recovered.root()).unwrap())
            .apply_profile_package(&dogfood)
            .unwrap();
        assert!(!stale_schema.exists());
        assert_eq!(
            repaired
                .profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("jit-default", ProfileApplicationStatus::Applied),
                ("jit-dogfood", ProfileApplicationStatus::Unchanged),
            ]
        );
    }

    #[test]
    fn test_resolve_profile_package_reports_a_recorded_location_that_no_longer_resolves() {
        let (temp, _storage, executor, _package) = fixture();
        let applied = package_read_from(&temp, "vendor/recorded");
        executor.apply_profile_package(&applied).unwrap();
        fs::remove_dir_all(temp.path().join("vendor/recorded")).unwrap();

        let selector = ProfileSelector::id(fixture_id()).unwrap();
        let error = executor.resolve_profile_package(&selector).unwrap_err();

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
    fn test_resolve_profile_package_path_selector_uses_the_directory_package_id() {
        let (temp, _storage, executor, _package) = fixture();
        let package = package_read_from(&temp, "vendor/supplied");
        let selector = ProfileSelector::path(temp.path().join("vendor/supplied"));
        let resolved = executor.resolve_profile_package(&selector).unwrap();
        assert_eq!(resolved.model().id, package.model().id);
    }

    #[test]
    fn test_resolve_profile_package_reports_a_supplied_location_holding_no_package() {
        let (temp, _storage, executor, _package) = fixture();

        let selector = ProfileSelector::path(temp.path().join("vendor/absent"));
        let error = executor.resolve_profile_package(&selector).unwrap_err();

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
        assert_ne!(rewritten.model().version, recorded_version);

        let listed = executor.list_recorded_profiles().unwrap();

        assert_eq!(listed.count, 1);
        assert_eq!(listed.profiles[0].version, rewritten.model().version);
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
        let location = match stored_record(&temp).origin {
            ProfileOrigin::Directory(location) => location,
            ProfileOrigin::Embedded => panic!("fixture records a directory package"),
        };
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
            .model()
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
        let manifest = package.model();

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
                    .model()
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
            .model()
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
                .model()
                .dependencies
                .iter()
                .map(|dependency| dependency.id.to_string())
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
        let default_id = explicit_default.model().id.to_string();
        let dogfood_id = explicit_dogfood.model().id.to_string();
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
        assert_eq!(record.id.as_str(), "planner-asset-only");
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
    fn test_apply_profile_package_reapplies_changed_content_and_rewrites_its_record() {
        let (temp, _storage, executor, package) = fixture();
        executor.apply_profile_package(&package).unwrap();
        let previous = stored_record(&temp);
        let unowned = temp.path().join("docs/repository-note.txt");
        fs::write(&unowned, "repository-authored\n").unwrap();
        let replacement = repackage(
            &temp,
            FIXTURE_LOCATION,
            "assets/profile.txt",
            "replacement profile content\n",
        );

        let applied = executor.apply_profile_package(&replacement).unwrap();

        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/profile.txt")).unwrap(),
            "replacement profile content\n"
        );
        let rewritten = stored_record(&temp);
        assert_ne!(rewritten.package_hash, previous.package_hash);
        assert_eq!(rewritten.package_hash, replacement.hashes().package);
        assert_eq!(
            fs::read_to_string(unowned).unwrap(),
            "repository-authored\n",
            "a package reapply does not delete unowned repository content"
        );
    }

    #[test]
    fn test_apply_profile_package_removes_only_unchanged_sole_asset_claims() {
        let (temp, _storage, executor, package) = fixture();
        executor.apply_profile_package(&package).unwrap();
        let replacement = package_without_assets(&temp, FIXTURE_LOCATION, "planner-asset-only");

        let applied = executor.apply_profile_package(&replacement).unwrap();

        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert!(
            !temp.path().join("docs/profile.txt").exists(),
            "an unchanged, solely-owned, unretained former asset is removable"
        );
        assert!(stored_record(&temp).claims.is_empty());
    }

    #[test]
    fn test_apply_profile_package_retains_stopped_assets_that_are_changed_shared_or_adopted() {
        let retained_cases = [
            ("changed", "repository edit\n", false),
            ("adopted", "Portable profile fixture.\n", true),
        ];
        for (name, current, adopt_before_apply) in retained_cases {
            let (temp, _storage, executor, package) = fixture();
            let target = temp.path().join("docs/profile.txt");
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if adopt_before_apply {
                fs::write(&target, current).unwrap();
            }
            executor.apply_profile_package(&package).unwrap();
            if !adopt_before_apply {
                fs::write(&target, current).unwrap();
            }
            let replacement = package_without_assets(&temp, FIXTURE_LOCATION, "planner-asset-only");

            executor.apply_profile_package(&replacement).unwrap();

            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                current,
                "{name} former content must survive"
            );
            assert!(
                stored_record(&temp).claims.is_empty(),
                "{name} content is no longer owned by the replacement package"
            );
        }

        let (temp, _storage, executor, _fixture) = fixture();
        let target = "docs/shared.txt";
        let base = package_publishing(&temp, "vendor/base", "base", target, "shared\n");
        let survivor = package_publishing(&temp, "vendor/survivor", "survivor", target, "shared\n");
        executor.apply_profile_package(&base).unwrap();
        executor.apply_profile_package(&survivor).unwrap();
        let replacement = package_without_assets(&temp, "vendor/base", "base");

        executor.apply_profile_package(&replacement).unwrap();

        assert_eq!(
            fs::read_to_string(temp.path().join(target)).unwrap(),
            "shared\n"
        );
        assert!(record_for(&temp, "base").claims.is_empty());
        assert!(record_for(&temp, "survivor")
            .claims
            .iter()
            .any(|claim| matches!(
                claim.identity,
                crate::repository_state::AppliedProfileClaimIdentity::Asset { .. }
            )));
    }

    #[test]
    fn test_apply_profile_package_rejects_concurrent_target_edits_without_publishing() {
        let (temp, _storage, executor, package) = fixture();
        executor.apply_profile_package(&package).unwrap();
        let record_before =
            fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap();
        let target = temp.path().join("docs/profile.txt");
        fs::write(&target, "repository edit\n").unwrap();
        let replacement = repackage(
            &temp,
            FIXTURE_LOCATION,
            "assets/profile.txt",
            "replacement profile content\n",
        );

        let error = executor.apply_profile_package(&replacement).unwrap_err();

        let conflict = three_way_conflict(&error);
        assert_eq!(conflict.target.repository_relative(), "docs/profile.txt");
        assert_eq!(conflict.owner.as_str(), "planner-asset-only");
        assert!(matches!(
            conflict.base,
            crate::profile::ThreeWayValue::Present(_)
        ));
        assert!(matches!(
            conflict.current,
            crate::profile::ThreeWayValue::Present(_)
        ));
        assert!(matches!(
            conflict.candidate,
            crate::profile::ThreeWayValue::Present(_)
        ));
        let message = error.to_string();
        for expected in [
            "docs/profile.txt",
            "planner-asset-only",
            "base",
            "current",
            "candidate",
        ] {
            assert!(
                message.contains(expected),
                "conflict omits '{expected}': {message}"
            );
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "repository edit\n");
        assert_eq!(
            fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap(),
            record_before
        );
    }

    #[test]
    fn test_profile_application_substitutes_values_without_auditing_resolved_content() {
        let (temp, storage, executor, _package) = fixture();
        let package_root = temp.path().join(FIXTURE_LOCATION);
        fs::write(
            package_root.join(crate::profile::MANIFEST_FILE_NAME),
            br#"
[profile]
manifest-version = 2
id = "planner-variable"
version = "1.0.0"
compatible-jit = "*"

[[variable]]
name = "NAME"
default = "default"

[[asset]]
source = "assets/profile.txt"
target = "docs/profile.txt"
template = true
"#,
        )
        .unwrap();
        let resolved_value = "resolved-value";
        let values_file = temp.path().join("profile-values.toml");
        fs::write(&values_file, "[variables]\nNAME = \"values-file-value\"\n").unwrap();
        fs::write(
            package_root.join("assets/profile.txt"),
            b"profile={{jit:var:NAME}}\n",
        )
        .unwrap();
        let package = ProfilePackage::from_directory(&package_root).unwrap();
        let inputs = load_profile_variable_inputs(
            std::slice::from_ref(&package),
            Some(&values_file),
            &[
                ProfileVariableAssignment::new("NAME".try_into().unwrap(), resolved_value),
                ProfileVariableAssignment::new(
                    "NAME".try_into().unwrap(),
                    format!("{resolved_value}-last"),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            inputs.values_file[&"NAME".try_into().unwrap()],
            "values-file-value"
        );
        assert_eq!(
            inputs.command_line.last().unwrap().value,
            "resolved-value-last"
        );

        executor
            .apply_profile_package_with_inputs(&package, &inputs)
            .unwrap();

        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            b"profile=resolved-value-last\n"
        );
        let event_bytes = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let record_bytes =
            fs::read(temp.path().join(".jit/profiles/planner-variable.json")).unwrap();
        let record: AppliedProfileRecord = serde_json::from_slice(&record_bytes).unwrap();
        assert!(record
            .claims
            .iter()
            .all(|claim| claim.base_fingerprint.as_str().len() == 64));
        let events = storage.read_events().unwrap();
        assert!(matches!(
            events.as_slice(),
            [Event::ProfileLifecycle { profiles, .. }]
                if profiles.len() == 1
                    && profiles[0].variables[0].name.as_ref() == "NAME"
                    && profiles[0].variables[0].source == crate::profile::VariableSource::Set
        ));
        assert!(!event_bytes
            .windows(resolved_value.len())
            .any(|window| window == resolved_value.as_bytes()));
        assert!(record_bytes
            .windows(resolved_value.len())
            .any(|window| window == resolved_value.as_bytes()));
        assert_eq!(
            record.variables.values()[&"NAME".try_into().unwrap()],
            "resolved-value-last"
        );
        assert_eq!(
            record.variables.sources()[&"NAME".try_into().unwrap()],
            crate::profile::VariableSource::Set
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
    fn test_profile_plan_collection_preserves_selector_occurrences_without_events() {
        let (temp, storage, executor, _package) = fixture();
        let before = storage.read_events().unwrap();
        let selector = ProfileSelector::path(FIXTURE_LOCATION);

        let result = executor
            .plan_profiles(&[selector.clone(), selector])
            .unwrap();

        assert_eq!(result.count, 2);
        assert_eq!(
            result
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["planner-asset-only", "planner-asset-only"]
        );
        assert_eq!(storage.read_events().unwrap(), before);
        assert!(!temp
            .path()
            .join(".jit/profiles/planner-asset-only.json")
            .exists());
        assert!(!temp.path().join("docs/profile.txt").exists());
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
            [Event::ProfileLifecycle {
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
        assert!(matches!(
            storage.read_events().unwrap().as_slice(),
            [Event::ProfileLifecycle {
                operation: ProfileLifecycleOperation::Apply,
                profiles,
                converted_records,
                ..
            }]
                if converted_records.is_empty()
                    && profiles.len() == 2
                    && profiles.iter().all(|profile| profile.status
                        == ProfileLifecycleStatus::Installed)
        ));
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
    fn test_apply_profile_selection_publishes_multiple_packages_through_one_transaction() {
        let temp = TempDir::new().unwrap();
        let bootstrap = JsonFileStorage::new(temp.path().join(".jit"));
        CommandExecutor::new(bootstrap.clone())
            .with_layout(discover_repository_layout(temp.path(), bootstrap.root()).unwrap())
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        let publications = PublicationCounter::new();
        let storage = JsonFileStorage::with_repository_state_failures(
            temp.path().join(".jit"),
            publications.clone(),
        );
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());

        let applied = executor
            .apply_profile(&[package_selector(&left), package_selector(&right)])
            .unwrap();

        assert_eq!(
            publications.count(),
            1,
            "one selection must reach the recoverable publication boundary once"
        );
        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("left", ProfileApplicationStatus::Applied),
                ("right", ProfileApplicationStatus::Applied),
            ]
        );
        for id in ["left", "right"] {
            assert!(temp
                .path()
                .join(format!(".jit/profiles/{id}.json"))
                .is_file());
            assert!(temp.path().join(format!("docs/{id}.txt")).is_file());
        }
        assert_eq!(applied_event_ids(&storage), vec!["left", "right"]);
    }

    #[test]
    fn test_apply_profile_selection_is_a_complete_noop_when_every_member_is_present() {
        let (temp, _storage, executor, _fixture) = fixture();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        executor
            .apply_profile(&[package_selector(&left), package_selector(&right)])
            .unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let records = ["left", "right"].map(|id| record_for(&temp, id));
        let publications = PublicationCounter::new();
        let storage = JsonFileStorage::with_repository_state_failures(
            temp.path().join(".jit"),
            publications.clone(),
        );
        let executor = CommandExecutor::new(storage).with_layout(
            discover_repository_layout(temp.path(), temp.path().join(".jit")).unwrap(),
        );

        let reapplied = executor
            .apply_profile(&[package_selector(&left), package_selector(&right)])
            .unwrap();

        assert_eq!(publications.count(), 0);
        assert!(reapplied
            .profiles
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Unchanged));
        assert!(reapplied
            .profiles
            .iter()
            .all(|profile| profile.transaction_id.is_none()));
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events
        );
        assert_eq!(["left", "right"].map(|id| record_for(&temp, id)), records);
    }

    #[test]
    fn test_apply_profile_selection_recovers_target_record_and_audit_together_after_failure() {
        let temp = TempDir::new().unwrap();
        let bootstrap = JsonFileStorage::new(temp.path().join(".jit"));
        CommandExecutor::new(bootstrap.clone())
            .with_layout(discover_repository_layout(temp.path(), bootstrap.root()).unwrap())
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        let original_events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(
            temp.path().join(".jit"),
            FailOnce::at(
                crate::storage::TransactionFailurePoint::RepositoryAfterAction { action: 0 },
            ),
        );
        let selectors = [package_selector(&left), package_selector(&right)];
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());

        assert!(executor.apply_profile(&selectors).is_err());

        let recovered = JsonFileStorage::new(temp.path().join(".jit"));
        let layout = discover_repository_layout(temp.path(), recovered.root()).unwrap();
        drop(recovered.open_mutation_session(layout).unwrap());
        for id in ["left", "right"] {
            assert!(!temp.path().join(format!("docs/{id}.txt")).exists());
            assert!(!temp
                .path()
                .join(format!(".jit/profiles/{id}.json"))
                .exists());
        }
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            original_events
        );

        let retried = CommandExecutor::new(recovered.clone())
            .with_layout(discover_repository_layout(temp.path(), recovered.root()).unwrap())
            .apply_profile(&selectors)
            .unwrap();
        assert!(retried
            .profiles
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Applied));
        assert_eq!(applied_event_ids(&recovered), vec!["left", "right"]);
    }

    #[test]
    fn test_apply_profile_selection_keeps_duplicate_root_observation_without_duplicate_publication()
    {
        let (temp, _storage, executor, _fixture) = fixture();
        let package = package_declaring(&temp, "vendor/duplicate", "duplicate", &[]);

        let applied = executor
            .apply_profile(&[package_selector(&package), package_selector(&package)])
            .unwrap();

        assert_eq!(
            applied
                .profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("duplicate", ProfileApplicationStatus::Applied),
                ("duplicate", ProfileApplicationStatus::Unchanged),
            ]
        );
        assert_eq!(
            applied_event_ids(&JsonFileStorage::new(temp.path().join(".jit"))),
            vec!["duplicate"]
        );
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
            assert_eq!(record_for(&temp, id).id.as_str(), id);
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
    fn test_apply_profile_package_records_shared_owners_for_its_dependency_closure() {
        let (temp, _storage, executor, _fixture) = fixture();
        let namespace = "closure-shared";
        let base = package_declaring_contribution(
            &temp,
            "vendor/base",
            "base",
            &[],
            namespace,
            "Shared closure definition.",
        );
        let workflow = package_declaring_contribution(
            &temp,
            "vendor/workflow",
            "workflow",
            &["base"],
            namespace,
            "Shared closure definition.",
        );

        executor.apply_profile_package(&workflow).unwrap();

        let shared_claims = [base, workflow]
            .into_iter()
            .map(|package| {
                let record = record_for(&temp, package.model().id.as_str());
                let semantic = record
                    .claims
                    .into_iter()
                    .filter(|claim| {
                        matches!(
                            claim.identity,
                            crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(semantic.len(), 1);
                semantic.into_iter().next().unwrap()
            })
            .collect::<Vec<_>>();
        assert!(shared_claims.iter().all(|claim| matches!(
            claim.identity,
            crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
        )));
        assert_eq!(shared_claims[0], shared_claims[1]);
    }

    #[test]
    fn test_effective_configuration_loads_registry_not_record_claims() {
        let (temp, storage, executor, _fixture) = fixture();
        let namespace = "registry-derived";
        let package = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "Published definition.",
        );
        executor.apply_profile_package(&package).unwrap();
        let record = record_for(&temp, "workflow");
        assert!(record.claims.iter().any(|claim| matches!(
            claim.identity,
            crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
        )));

        let config_path = temp.path().join(".jit/config.toml");
        let registry = fs::read_to_string(&config_path)
            .unwrap()
            .replace("Published definition.", "Current registry definition.");
        fs::write(config_path, registry).unwrap();
        let reloaded = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());

        assert_eq!(
            reloaded
                .cached_config()
                .unwrap()
                .namespaces
                .as_ref()
                .unwrap()[namespace]
                .description,
            "Current registry definition."
        );
    }

    #[test]
    fn test_apply_profile_package_preflights_a_conflicting_dependency_closure() {
        let (temp, _storage, executor, _fixture) = fixture();
        let namespace = "closure-conflict";
        package_declaring_contribution(
            &temp,
            "vendor/base",
            "base",
            &[],
            namespace,
            "Base definition.",
        );
        let workflow = package_declaring_contribution(
            &temp,
            "vendor/workflow",
            "workflow",
            &["base"],
            namespace,
            "Workflow definition.",
        );

        let error = executor.apply_profile_package(&workflow).unwrap_err();

        let conflict = contribution_conflict(&error);
        assert_eq!(
            conflict.owners,
            vec![
                ContributionConflictOwner::Package(ProfilePackageId::new("base")),
                ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
            ]
        );
        for path in [
            ".jit/profiles/base.json",
            ".jit/profiles/workflow.json",
            "docs/base.txt",
            "docs/workflow.txt",
        ] {
            assert!(
                !temp.path().join(path).exists(),
                "the conflicting closure published {path}"
            );
        }
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
                occupant.model().id.to_string()
            ))
        );
        assert_eq!(
            conflict.candidate,
            ProfilePackageId::new(candidate.model().id.to_string())
        );
        assert_eq!(conflict.path.repository_relative(), target);
        // The occupant's bytes stand: a conflict applies nothing.
        assert_eq!(
            fs::read_to_string(temp.path().join(target)).unwrap(),
            "first\n"
        );
    }

    #[test]
    fn test_apply_profile_package_reports_semantic_conflicts_without_an_order_winner() {
        let namespace = "shared-namespace";
        let expected_owners = vec![
            ContributionConflictOwner::Package(ProfilePackageId::new("base")),
            ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
        ];

        for (first_id, first_description, second_id, second_description) in [
            (
                "base",
                "The base meaning.",
                "workflow",
                "The workflow meaning.",
            ),
            (
                "workflow",
                "The workflow meaning.",
                "base",
                "The base meaning.",
            ),
        ] {
            let (temp, _storage, executor, _fixture) = fixture();
            let first = package_contributing(
                &temp,
                &format!("vendor/{first_id}"),
                first_id,
                namespace,
                first_description,
            );
            let second = package_contributing(
                &temp,
                &format!("vendor/{second_id}"),
                second_id,
                namespace,
                second_description,
            );
            executor.apply_profile_package(&first).unwrap();

            let error = executor.apply_profile_package(&second).unwrap_err();

            let conflict = contribution_conflict(&error);
            assert_eq!(conflict.owners, expected_owners);
            assert_eq!(
                conflict.identity.to_string(),
                Contribution::MapEntry {
                    target: MapEntryTarget::Namespaces,
                    identity: namespace.to_string(),
                    value: serde_json::json!({ "description": first_description, "unique": false }),
                }
                .semantic_identity()
                .to_string()
            );
            assert!(
                !temp
                    .path()
                    .join(format!(".jit/profiles/{second_id}.json"))
                    .exists(),
                "the conflicting candidate must not gain a record"
            );
            assert!(
                fs::read_to_string(temp.path().join(".jit/config.toml"))
                    .unwrap()
                    .contains(first_description),
                "the first definition remains the repository state"
            );
        }
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
        assert_eq!(record_for(&temp, "stated").id.as_str(), "stated");
        let record = record_for(&temp, "restated");
        assert_eq!(record.id.as_str(), "restated");
        assert_eq!(
            record
                .claims
                .iter()
                .filter(|claim| matches!(
                    claim.identity,
                    crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn test_apply_profile_package_recontributes_its_own_semantic_identity() {
        let (temp, _storage, executor, _fixture) = fixture();
        let package = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            "self-recontribution",
            "One package owns this definition.",
        );
        executor.apply_profile_package(&package).unwrap();

        let reapplied = executor.apply_profile_package(&package).unwrap();

        assert_eq!(
            reapplied.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        let record = record_for(&temp, "workflow");
        assert_eq!(
            record
                .claims
                .iter()
                .filter(|claim| matches!(
                    claim.identity,
                    crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn test_apply_profile_package_keeps_distinct_identities_in_one_registry_independent() {
        let (temp, _storage, executor, _fixture) = fixture();
        let first = package_contributing(
            &temp,
            "vendor/base",
            "base",
            "first-identity",
            "First registry definition.",
        );
        let second = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            "second-identity",
            "Second registry definition.",
        );
        executor.apply_profile_package(&first).unwrap();
        executor.apply_profile_package(&second).unwrap();

        let config = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
        assert!(config.contains("First registry definition."));
        assert!(config.contains("Second registry definition."));
        let record = record_for(&temp, "workflow");
        assert_eq!(
            record
                .claims
                .iter()
                .filter(|claim| matches!(
                    claim.identity,
                    crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn test_apply_profile_package_reports_repository_and_package_semantic_owners() {
        let (temp, _storage, executor, _fixture) = fixture();
        let namespace = "repository-owned";
        let config_path = temp.path().join(".jit/config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        config.push_str(
            "\n[namespaces.repository-owned]\ndescription = \"Repository definition.\"\nunique = false\n",
        );
        fs::write(config_path, config).unwrap();
        let candidate = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "Package definition.",
        );

        let error = executor.apply_profile_package(&candidate).unwrap_err();

        let conflict = contribution_conflict(&error);
        assert_eq!(
            conflict.owners,
            vec![
                ContributionConflictOwner::Repository,
                ContributionConflictOwner::Package(ProfilePackageId::new("workflow")),
            ]
        );
        assert!(
            !temp.path().join(".jit/profiles/workflow.json").exists(),
            "a rejected repository conflict must not publish provenance"
        );
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
    fn test_apply_profile_rejects_a_dependency_version_mismatch_before_publication() {
        let (temp, storage, executor, _fixture) = fixture();
        package_v2(
            &temp,
            "vendor/base",
            "base",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        let workflow = package_v2(
            &temp,
            "vendor/workflow",
            "workflow",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[("base", ">=2.0.0")],
            &[],
        );

        let error = executor
            .apply_profile(&[package_selector(&workflow)])
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::DependencyVersionMismatch {
                    package,
                    dependency,
                    ..
                }) if package == "workflow" && dependency == "base"
            ),
            "the dependency and declaring package must be named: {error:#}"
        );
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_rejects_a_package_outside_the_running_engine_range() {
        let (temp, storage, executor, _fixture) = fixture();
        let package = package_v2(
            &temp,
            "vendor/future",
            "future",
            "1.0.0",
            ">=2.0.0",
            &[],
            &[],
        );

        let error = executor
            .apply_profile(&[package_selector(&package)])
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::IncompatibleEngine { package, .. })
                    if package == "future"
            ),
            "the incompatible package must be named: {error:#}"
        );
        assert!(!temp.path().join("docs/future.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_rejects_incompatibilities_symmetrically_and_independently_of_selector_order(
    ) {
        let (temp, storage, executor, _fixture) = fixture();
        let left = package_v2(
            &temp,
            "vendor/left",
            "left",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[("right", "*")],
        );
        let right = package_v2(
            &temp,
            "vendor/right",
            "right",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        let left_selector = package_selector(&left);
        let right_selector = package_selector(&right);

        let first = executor
            .apply_profile(&[left_selector.clone(), right_selector.clone()])
            .unwrap_err();
        let second = executor
            .apply_profile(&[right_selector, left_selector])
            .unwrap_err();

        assert_eq!(format!("{first:#}"), format!("{second:#}"));
        let message = format!("{first:#}");
        assert!(message.contains("left"), "{message}");
        assert!(message.contains("right"), "{message}");
        assert!(!temp.path().join("docs/left.txt").exists());
        assert!(!temp.path().join("docs/right.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_includes_an_unselected_applied_profile_in_incompatibility_checks() {
        let (temp, storage, executor, _fixture) = fixture();
        let base = package_v2(
            &temp,
            "vendor/base",
            "base",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        executor.apply_profile(&[package_selector(&base)]).unwrap();
        let candidate = package_v2(
            &temp,
            "vendor/candidate",
            "candidate",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[("base", "*")],
        );

        let error = executor
            .apply_profile(&[package_selector(&candidate)])
            .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("candidate"), "{message}");
        assert!(message.contains("base"), "{message}");
        assert!(!temp.path().join("docs/candidate.txt").exists());
        assert_eq!(storage.read_events().unwrap().len(), 1);
    }

    #[test]
    fn test_apply_profile_includes_an_unselected_applied_profile_in_engine_range_checks() {
        let (temp, storage, executor, _fixture) = fixture();
        let applied = package_v2(
            &temp,
            "vendor/applied",
            "applied",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        executor
            .apply_profile(&[package_selector(&applied)])
            .unwrap();
        let manifest = fs::read_to_string(temp.path().join("vendor/applied/manifest.toml"))
            .unwrap()
            .replace(
                "compatible-jit = \">=0.2.0, <2.0.0\"",
                "compatible-jit = \">=2.0.0\"",
            );
        fs::write(temp.path().join("vendor/applied/manifest.toml"), manifest).unwrap();
        let candidate = package_v2(
            &temp,
            "vendor/candidate",
            "candidate",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );

        let error = executor
            .apply_profile(&[package_selector(&candidate)])
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::IncompatibleEngine { package, .. })
                    if package == "applied"
            ),
            "the unselected applied package must be named: {error:#}"
        );
        assert!(!temp.path().join("docs/candidate.txt").exists());
        assert_eq!(storage.read_events().unwrap().len(), 1);
    }

    #[test]
    fn test_apply_profile_includes_an_unselected_applied_profile_in_dependency_checks() {
        let (temp, storage, executor, _fixture) = fixture();
        let applied = package_v2(
            &temp,
            "vendor/applied",
            "applied",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        executor
            .apply_profile(&[package_selector(&applied)])
            .unwrap();
        let manifest = format!(
            "{}\n[[dependency]]\nid = \"missing-base\"\nversion = \"*\"\n",
            fs::read_to_string(temp.path().join("vendor/applied/manifest.toml")).unwrap()
        );
        fs::write(temp.path().join("vendor/applied/manifest.toml"), manifest).unwrap();
        let candidate = package_v2(
            &temp,
            "vendor/candidate",
            "candidate",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );

        let error = executor
            .apply_profile(&[package_selector(&candidate)])
            .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("applied"), "{message}");
        assert!(message.contains("missing-base"), "{message}");
        assert!(!temp.path().join("docs/candidate.txt").exists());
        assert_eq!(storage.read_events().unwrap().len(), 1);
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
        let prior_event = Event::ProfileApplied {
            id: String::new(),
            timestamp: chrono::DateTime::UNIX_EPOCH,
            profile_id: "prior".to_string(),
            version: "1.0.0".to_string(),
            origin: ProfileOrigin::Directory(RootRelativePath::parse("vendor/prior").unwrap()),
            package_hash: "prior-package".to_string(),
            target_hashes: BTreeMap::new(),
            isolated_torn_tail: false,
        };
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
            Event::ProfileLifecycle {
                isolated_torn_tail: false,
                ..
            }
        ));
    }

    /// Assert a rehearsal's per-target decisions against what the real run did.
    ///
    /// A rehearsal is only worth running if its decisions are the ones the
    /// publication makes, so every target it would create or update must have
    /// changed and every target it called unchanged must not have.
    fn assert_planned_targets_match_published(
        before: &BTreeMap<PathBuf, Vec<u8>>,
        after: &BTreeMap<PathBuf, Vec<u8>>,
        planned: &ProfilePlanResult,
    ) {
        assert!(
            planned
                .profiles
                .iter()
                .flat_map(|plan| &plan.targets)
                .any(|target| target.action != ProfileTargetAction::Unchanged),
            "a rehearsal that decides nothing proves nothing about the run it precedes"
        );
        for target in planned.profiles.iter().flat_map(|plan| &plan.targets) {
            let path = PathBuf::from(&target.path);
            match target.action {
                ProfileTargetAction::Unchanged => assert_eq!(
                    before.get(&path),
                    after.get(&path),
                    "the rehearsal called {} unchanged",
                    target.path
                ),
                ProfileTargetAction::Create | ProfileTargetAction::Update => assert_ne!(
                    before.get(&path),
                    after.get(&path),
                    "the rehearsal said it would publish {}",
                    target.path
                ),
            }
        }
    }

    #[test]
    fn test_reconfigure_profiles_from_sources_republishes_only_the_targets_the_changed_value_feeds()
    {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        let untouched = temp.path().join("docs/fixed.txt");
        let untouched_before = fs::read(&untouched).unwrap();
        let record_before = record_for(&temp, LIFECYCLE_ID);
        let events_before = lifecycle_event_count(&storage);

        let reconfigured = executor
            .reconfigure_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[("GREETING", "supplied")]),
            )
            .unwrap();

        assert_eq!(
            reconfigured.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert!(
            fs::read_to_string(temp.path().join("docs/templated.txt"))
                .unwrap()
                .contains("supplied"),
            "the target the supplied value feeds carries that value"
        );
        assert_eq!(
            fs::read(&untouched).unwrap(),
            untouched_before,
            "a target no supplied value reaches keeps its exact bytes"
        );
        let record_after = record_for(&temp, LIFECYCLE_ID);
        assert_eq!(
            recorded_asset_fingerprint(&record_after, "docs/fixed.txt"),
            recorded_asset_fingerprint(&record_before, "docs/fixed.txt"),
            "a target no supplied value reaches keeps the base its next decision reads"
        );
        assert_ne!(
            recorded_asset_fingerprint(&record_after, "docs/templated.txt"),
            recorded_asset_fingerprint(&record_before, "docs/templated.txt"),
            "the republished target records the base it was published at"
        );
        assert_eq!(
            record_after.package_hash, record_before.package_hash,
            "reconfiguration replays the installed package rather than replacing it"
        );
        assert_eq!(lifecycle_event_count(&storage), events_before + 1);
    }

    #[test]
    fn test_reconfigure_profiles_from_sources_publishes_nothing_when_no_value_is_supplied() {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let reconfigured = executor
            .reconfigure_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[]),
            )
            .unwrap();

        assert_eq!(
            reconfigured.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged,
            "replaying the record with nothing supplied changes nothing"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "a complete no-op writes no file"
        );
        assert_eq!(
            lifecycle_event_count(&storage),
            events_before,
            "a complete no-op audits no lifecycle event"
        );
    }

    #[test]
    fn test_reconfigure_profiles_from_sources_reports_an_edited_target_as_a_conflict_without_publishing(
    ) {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        fs::write(
            temp.path().join("docs/templated.txt"),
            "greeting=edited in place\n",
        )
        .unwrap();
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let error = executor
            .reconfigure_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[("GREETING", "supplied")]),
            )
            .unwrap_err();

        let conflict = three_way_conflict(&error);
        assert_eq!(conflict.target.repository_relative(), "docs/templated.txt");
        assert_eq!(conflict.owner.as_str(), LIFECYCLE_ID);
        assert_ne!(
            conflict.current, conflict.base,
            "the conflict reports current diverging from the recorded base"
        );
        assert_ne!(
            conflict.current, conflict.candidate,
            "the conflict reports current diverging from the resolved candidate"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "a conflicting decision publishes nothing"
        );
        assert_eq!(lifecycle_event_count(&storage), events_before);
    }

    #[test]
    fn test_upgrade_profiles_from_sources_updates_owned_targets_and_retains_shared_content() {
        let (temp, _storage, executor, _fixture) = fixture();
        let shared_body = "one body, two owners\n";
        let alpha = authored_manifest_package(
            &temp,
            "packages/alpha",
            "[profile]\nmanifest-version = 2\nid = \"alpha\"\nversion = \"1.0.0\"\n\
             compatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/own.txt\"\ntarget = \"docs/alpha-own.txt\"\n\n\
             [[asset]]\nsource = \"assets/shared.txt\"\ntarget = \"docs/shared.txt\"\n",
            &[
                ("assets/own.txt", "alpha own v1\n"),
                ("assets/shared.txt", shared_body),
            ],
        );
        let beta = authored_manifest_package(
            &temp,
            "packages/beta",
            "[profile]\nmanifest-version = 2\nid = \"beta\"\nversion = \"1.0.0\"\n\
             compatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/shared.txt\"\ntarget = \"docs/shared.txt\"\n",
            &[("assets/shared.txt", shared_body)],
        );
        apply_package(&executor, &alpha, &supplied_values(&[]));
        apply_package(&executor, &beta, &supplied_values(&[]));
        authored_manifest_package(
            &temp,
            "packages/alpha",
            "[profile]\nmanifest-version = 2\nid = \"alpha\"\nversion = \"2.0.0\"\n\
             compatible-jit = \"*\"\n\n\
             [[asset]]\nsource = \"assets/own.txt\"\ntarget = \"docs/alpha-own.txt\"\n",
            &[("assets/own.txt", "alpha own v2\n")],
        );

        let upgraded = executor
            .upgrade_profiles_from_sources(&installed_selector("alpha"), &supplied_values(&[]))
            .unwrap();

        assert_eq!(
            upgraded.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/alpha-own.txt")).unwrap(),
            "alpha own v2\n",
            "an unchanged target this package solely owns takes the new version's content"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/shared.txt")).unwrap(),
            shared_body,
            "content a surviving owner still claims outlives the owner that stopped claiming it"
        );
        assert_eq!(record_for(&temp, "alpha").version, "2.0.0");
        assert_eq!(
            record_for(&temp, "beta").version,
            "1.0.0",
            "upgrading one profile leaves every other applied record where it was"
        );
    }

    #[test]
    fn test_upgrade_profiles_from_sources_refuses_a_version_a_surviving_profile_depends_on() {
        let (temp, storage, executor, _fixture) = fixture();
        let alpha = package_v2(&temp, "packages/alpha", "alpha", "1.0.0", "*", &[], &[]);
        let beta = package_v2(
            &temp,
            "packages/beta",
            "beta",
            "1.0.0",
            "*",
            &[("alpha", "^1.0.0")],
            &[],
        );
        apply_package(&executor, &alpha, &supplied_values(&[]));
        apply_package(&executor, &beta, &supplied_values(&[]));
        package_v2(&temp, "packages/alpha", "alpha", "2.0.0", "*", &[], &[]);
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let error = executor
            .upgrade_profiles_from_sources(&installed_selector("alpha"), &supplied_values(&[]))
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::DependencyVersionMismatch {
                    package,
                    dependency,
                    found,
                    ..
                }) if package == "beta" && dependency == "alpha" && found == "2.0.0"
            ),
            "the range a surviving profile depends on refuses the replacement: {error:#}"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "a refused upgrade publishes nothing"
        );
        assert_eq!(lifecycle_event_count(&storage), events_before);
    }

    #[test]
    fn test_upgrade_profiles_from_sources_reports_a_divergent_target_as_a_conflict_without_publishing(
    ) {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        fs::write(
            temp.path().join("docs/fixed.txt"),
            "edited by the adopter\n",
        )
        .unwrap();
        authored_manifest_package(
            &temp,
            LIFECYCLE_LOCATION,
            &format!(
                "[profile]\nmanifest-version = 2\nid = \"{LIFECYCLE_ID}\"\n\
                 version = \"2.0.0\"\ncompatible-jit = \"*\"\n\n\
                 [[variable]]\nname = \"GREETING\"\ndefault = \"authored\"\n\n\
                 [[asset]]\nsource = \"assets/templated.txt\"\n\
                 target = \"docs/templated.txt\"\ntemplate = true\n\n\
                 [[asset]]\nsource = \"assets/fixed.txt\"\ntarget = \"docs/fixed.txt\"\n"
            ),
            &[
                ("assets/templated.txt", "greeting={{jit:var:GREETING}}\n"),
                ("assets/fixed.txt", "the new version's fixed content\n"),
            ],
        );
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let error = executor
            .upgrade_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &supplied_values(&[]))
            .unwrap_err();

        let conflict = three_way_conflict(&error);
        assert_eq!(conflict.target.repository_relative(), "docs/fixed.txt");
        assert_eq!(conflict.owner.as_str(), LIFECYCLE_ID);
        assert_ne!(conflict.current, conflict.base);
        assert_ne!(conflict.current, conflict.candidate);
        assert_eq!(
            repository_files(&temp),
            before,
            "a conflicting upgrade publishes nothing"
        );
        assert_eq!(lifecycle_event_count(&storage), events_before);
    }

    #[test]
    fn test_upgrade_profiles_from_sources_carries_supplied_values_and_defaults_new_declarations() {
        let (temp, _storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(
            &executor,
            &package,
            &supplied_values(&[("GREETING", "carried")]),
        );
        authored_manifest_package(
            &temp,
            LIFECYCLE_LOCATION,
            &format!(
                "[profile]\nmanifest-version = 2\nid = \"{LIFECYCLE_ID}\"\n\
                 version = \"2.0.0\"\ncompatible-jit = \"*\"\n\n\
                 [[variable]]\nname = \"GREETING\"\ndefault = \"authored\"\n\n\
                 [[variable]]\nname = \"FAREWELL\"\ndefault = \"new default\"\n\n\
                 [[asset]]\nsource = \"assets/templated.txt\"\n\
                 target = \"docs/templated.txt\"\ntemplate = true\n"
            ),
            &[(
                "assets/templated.txt",
                "greeting={{jit:var:GREETING}} farewell={{jit:var:FAREWELL}}\n",
            )],
        );

        executor
            .upgrade_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &supplied_values(&[]))
            .unwrap();

        assert_eq!(
            fs::read_to_string(temp.path().join("docs/templated.txt")).unwrap(),
            "greeting=carried farewell=new default\n",
            "a stored supplied value survives the replacement while a newly declared \
             variable takes the new package's default"
        );
    }

    #[test]
    fn test_plan_reconfigure_profiles_from_sources_reports_the_decisions_the_run_publishes() {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);
        let options = supplied_values(&[("GREETING", "supplied")]);

        let planned = executor
            .plan_reconfigure_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &options)
            .unwrap();

        assert_eq!(
            repository_files(&temp),
            before,
            "a rehearsal writes no file and leaves every applied record byte-identical"
        );
        assert_eq!(
            lifecycle_event_count(&storage),
            events_before,
            "a rehearsal audits nothing"
        );
        assert_eq!(planned.count, planned.profiles.len());
        assert_eq!(
            planned.profiles[0].status,
            ProfilePlanStatus::WouldApply,
            "a rehearsal over changed values reports work to publish"
        );

        executor
            .reconfigure_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &options)
            .unwrap();

        assert_planned_targets_match_published(&before, &repository_files(&temp), &planned);
    }

    #[test]
    fn test_plan_upgrade_profiles_from_sources_reports_the_decisions_the_run_publishes() {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        authored_manifest_package(
            &temp,
            LIFECYCLE_LOCATION,
            &format!(
                "[profile]\nmanifest-version = 2\nid = \"{LIFECYCLE_ID}\"\n\
                 version = \"2.0.0\"\ncompatible-jit = \"*\"\n\n\
                 [[variable]]\nname = \"GREETING\"\ndefault = \"authored\"\n\n\
                 [[asset]]\nsource = \"assets/templated.txt\"\n\
                 target = \"docs/templated.txt\"\ntemplate = true\n\n\
                 [[asset]]\nsource = \"assets/fixed.txt\"\ntarget = \"docs/fixed.txt\"\n"
            ),
            &[
                ("assets/templated.txt", "greeting={{jit:var:GREETING}}\n"),
                ("assets/fixed.txt", "the new version's fixed content\n"),
            ],
        );
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let planned = executor
            .plan_upgrade_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[]),
            )
            .unwrap();

        assert_eq!(
            repository_files(&temp),
            before,
            "a rehearsal writes no file and leaves every applied record byte-identical"
        );
        assert_eq!(lifecycle_event_count(&storage), events_before);
        assert_eq!(planned.count, planned.profiles.len());
        assert_eq!(planned.profiles[0].status, ProfilePlanStatus::WouldApply);

        executor
            .upgrade_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &supplied_values(&[]))
            .unwrap();

        assert_planned_targets_match_published(&before, &repository_files(&temp), &planned);
    }

    #[test]
    fn test_profile_lifecycle_run_audits_one_event_naming_its_operation_and_profile_status() {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        let after_apply = lifecycle_event_count(&storage);

        executor
            .reconfigure_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[("GREETING", "supplied")]),
            )
            .unwrap();

        assert_eq!(lifecycle_event_count(&storage), after_apply + 1);
        assert_eq!(
            last_lifecycle_event(&storage),
            (
                ProfileLifecycleOperation::Reconfigure,
                vec![(
                    LIFECYCLE_ID.to_string(),
                    ProfileLifecycleStatus::Reconfigured
                )]
            )
        );

        lifecycle_package(&temp, "2.0.0");
        executor
            .upgrade_profiles_from_sources(&installed_selector(LIFECYCLE_ID), &supplied_values(&[]))
            .unwrap();

        assert_eq!(lifecycle_event_count(&storage), after_apply + 2);
        assert_eq!(
            last_lifecycle_event(&storage),
            (
                ProfileLifecycleOperation::Upgrade,
                vec![(LIFECYCLE_ID.to_string(), ProfileLifecycleStatus::Upgraded)]
            )
        );
    }

    #[test]
    fn test_reconfigure_profiles_from_sources_refuses_a_package_that_replaced_its_installed_identity(
    ) {
        let (temp, storage, executor, _fixture) = fixture();
        let package = lifecycle_package(&temp, "1.0.0");
        apply_package(&executor, &package, &supplied_values(&[]));
        lifecycle_package(&temp, "2.0.0");
        let before = repository_files(&temp);
        let events_before = lifecycle_event_count(&storage);

        let error = executor
            .reconfigure_profiles_from_sources(
                &installed_selector(LIFECYCLE_ID),
                &supplied_values(&[("GREETING", "supplied")]),
            )
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::ReconfigurationPackageChanged { id, .. })
                    if id == LIFECYCLE_ID
            ),
            "reconfiguration never switches package identity: {error:#}"
        );
        assert_eq!(repository_files(&temp), before);
        assert_eq!(lifecycle_event_count(&storage), events_before);
    }
}
