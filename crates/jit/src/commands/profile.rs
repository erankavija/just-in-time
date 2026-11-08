use super::{with_mutation_session, with_read_session, CommandExecutor, SessionStep};
use crate::domain::identity::ProfileId;
use crate::domain::relative_path::RootRelativePath;
use crate::domain::ProfileLifecycleOperation;
use crate::profile::{
    build_profile_claims_from_resolved, capture_package_tree, pack_package_archive,
    read_package_archive, resolve_package, EngineVersion, PackageCaptureError, ProfileAddEntry,
    ProfileAddResult, ProfileApplicationStatus, ProfileApplyResult, ProfileComposedApplyResult,
    ProfileGraphError, ProfilePackEntry, ProfilePackResult, ProfilePackage, ProfilePackageError,
    ProfileShowEntry, ProfileShowResult, ProfileTargetAction, ProfileTargetChange,
    ProfileVariableAssignment, ProfileVariableName, ResolvedProfileContent, ResolvedProfileGraph,
    VariableInputs, MAX_PROFILE_PACKAGE_ARCHIVE_BYTES,
};
use crate::profile::{profile_run_entry, ProfileRunMode};
use crate::repository_state::{
    apply_overlay, classify_repository_export, derive_materialization, drop_overridden_claims,
    finalize_package_tree_capture, CaptureBudget, CapturedTreeFile, FileMode, MaterializationPlan,
    MaterializationRequest, MutationContext, PackageTreeCapture, ProfileApplicationInput,
    ProfileContributionClaim, RepositoryAction, RepositoryEntry, RepositoryExportDestination,
    RepositoryExportIntent, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryStateError, VirtualPath,
};
use crate::storage::{IssueStore, RepositoryReadSession, RepositoryStateStore, WorktreePaths};
use anyhow::Result;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// Project one run's dependency-first per-package answers onto the ordered root
/// observations the caller made.
///
/// A closure member the caller did not name is reported once, before the roots.
/// A repeated root selector observes the publication its first occurrence
/// accounts for ([`ProfileApplyResult::observed_again`]).
pub(super) fn selection_profile_results(
    closure: Vec<ProfileApplyResult>,
    roots: &[ProfilePackage],
) -> Result<ProfileComposedApplyResult> {
    let root_ids = roots
        .iter()
        .map(|root| root.model().id.as_str())
        .collect::<BTreeSet<_>>();
    let by_id = closure
        .iter()
        .map(|observation| (observation.id.as_str(), observation))
        .collect::<BTreeMap<_, _>>();
    let mut observations = closure
        .iter()
        .filter(|observation| !root_ids.contains(observation.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut observed = BTreeSet::new();
    for root in roots {
        let id = root.model().id.as_str();
        let Some(source) = by_id.get(id) else {
            anyhow::bail!("selected root '{id}' has no aggregate selection answer");
        };
        let observation = (*source).clone();
        observations.push(if observed.insert(id) {
            observation
        } else {
            observation.observed_again()
        });
    }
    Ok(ProfileComposedApplyResult::new(observations))
}

/// A malformed `--profile` selector.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileSelectorError {
    /// The selector spells a retired `id:` or `path:` tag.
    #[error(
        "invalid profile selector '{selector}': selectors are bare package directory paths; pass the directory itself, such as packages/<PROFILE_ID>"
    )]
    TaggedSelector {
        /// User-supplied selector.
        selector: String,
    },
    /// The selector names no directory.
    #[error("invalid profile selector: the package directory path is empty")]
    EmptyPath,
}

/// Parse one `--profile` value into the package directory it names, resolved
/// later relative to the repository worktree.
pub fn parse_profile_selector(selector: &str) -> Result<PathBuf, ProfileSelectorError> {
    if selector.is_empty() {
        Err(ProfileSelectorError::EmptyPath)
    } else if selector.starts_with("id:") || selector.starts_with("path:") {
        Err(ProfileSelectorError::TaggedSelector {
            selector: selector.to_string(),
        })
    } else {
        Ok(PathBuf::from(selector))
    }
}

/// Parse the ordered `--profile` selector stream of a profile command.
pub fn parse_profile_selectors(values: &[String]) -> Result<Vec<PathBuf>> {
    values
        .iter()
        .map(|value| {
            parse_profile_selector(value)
                .map_err(|error| crate::errors::InvalidArgumentError::new(error.to_string()).into())
        })
        .collect()
}

/// Parse the ordered `--set NAME=VALUE` assignments of a profile command.
pub fn parse_profile_variable_assignments(
    values: &[String],
) -> Result<Vec<ProfileVariableAssignment>> {
    values
        .iter()
        .map(|assignment| {
            let (name, value) = assignment.split_once('=').ok_or_else(|| {
                crate::errors::InvalidArgumentError::new(format!(
                    "invalid profile variable assignment '{assignment}'; expected NAME=VALUE"
                ))
            })?;
            let name = ProfileVariableName::try_from(name.to_string())
                .map_err(crate::errors::InvalidArgumentError::new)?;
            Ok(ProfileVariableAssignment::new(name, value))
        })
        .collect()
}

/// A profile no package location supplies.
#[derive(Debug, thiserror::Error)]
#[error("Profile not found: {profile}")]
pub struct ProfileNotFoundError {
    /// The profile that was looked for.
    pub profile: ProfileId,
}

/// Failure resolving which package bytes a command reads.
///
/// Resolution names the route a selector addressed together with what failed.
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
    /// A supplied archive could not be opened or read.
    #[error("cannot read profile package archive '{location}': {source}")]
    UnreadableArchive {
        /// Location as the caller supplied it.
        location: String,
        /// Archive-reader failure.
        #[source]
        source: std::io::Error,
    },
    /// A package whose live sources are drawn from repository content could
    /// not be assembled for an archive.
    #[error("profile package location '{location}' cannot be captured: {source}")]
    UncapturableLocation {
        /// Location as the caller supplied it.
        location: String,
        /// Capture failure.
        source: PackageCaptureError,
    },
    /// A declared dependency could not be loaded. The declaring package is
    /// named because the adopter never asked for the dependency by name.
    #[error(
        "profile '{package}' declares a dependency on profile '{dependency}', \
         which cannot be resolved: {cause:#}"
    )]
    UnresolvableDependency {
        /// Package whose manifest declares the dependency.
        package: String,
        /// Dependency that manifest names.
        dependency: String,
        /// Why loading it failed.
        cause: anyhow::Error,
    },
}

/// How an adopter clears [`ProfileApplyError::UncommittedTargets`].
pub const UNCOMMITTED_TARGETS_RESOLUTION: &str =
    "commit or discard those changes and the apply can proceed";

/// Profile application conflict detected before anything is published.
#[derive(Debug, thiserror::Error)]
pub enum ProfileApplyError {
    /// Paths the publication would write differ from `HEAD`.
    #[error(
        "profile application would overwrite uncommitted changes in {}; {UNCOMMITTED_TARGETS_RESOLUTION}",
        .paths.join(", ")
    )]
    UncommittedTargets {
        /// Each differing path, relative to the checkout holding it.
        paths: Vec<String>,
    },
    /// Final package and event overlay failed validation.
    #[error("final profile application validation failed: {0:#}")]
    FinalValidation(anyhow::Error),
    /// Final overlay produced error-severity rule findings.
    #[error("final profile application validation produced {error_count} error finding(s)")]
    FinalValidationFindings {
        /// Number of blocking findings.
        error_count: usize,
    },
    /// Package targets overlap application-owned audit or scratch state.
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
    /// An archived package's destination already holds something.
    #[error("profile package destination '{path}' already exists")]
    PackageDestinationOccupied {
        /// Repository-relative destination the add would have published at.
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

/// Capture the variable inputs `options` names for the closure `packages`
/// forms, validated once against that closure's declarations.
fn load_profile_variable_inputs(
    packages: &[ProfilePackage],
    options: &ProfileVariableOptions,
) -> Result<VariableInputs> {
    let values_file = options
        .values_file
        .as_deref()
        .map(|path| {
            fs::read_to_string(path).map_err(|error| {
                crate::errors::InvalidArgumentError::new(format!(
                    "cannot read profile values file '{}': {error}",
                    path.display()
                ))
            })
        })
        .transpose()?
        .map(|text| toml::from_str::<ProfileValuesFile>(&text))
        .transpose()?
        .map(|file| file.variables)
        .unwrap_or_default();
    let declarations = packages
        .iter()
        .flat_map(|package| package.model().variables.iter())
        .collect::<Vec<_>>();
    let mut inputs = VariableInputs {
        values_file,
        environment: BTreeMap::new(),
        command_line: options.assignments.clone(),
    };
    inputs.validate_against(declarations.iter().copied())?;

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

/// One profile selection, built once per invocation by
/// [`ProfileOps::select_profiles`].
///
/// Building it reads the packages the selectors name, resolves their
/// dependency graph through [`ResolvedProfileGraph::resolve`], validates the
/// variable inputs against the closure's declarations, and resolves and
/// composes every closure member. Plan, apply, diff and initialization take
/// this value, so an invocation runs each of those stages once.
#[derive(Clone)]
pub struct ProfileSelection {
    roots: Vec<ProfilePackage>,
    members: Vec<SelectedProfile>,
    contribution_context: Vec<ProfileContributionClaim>,
}

impl ProfileSelection {
    /// The selected packages, one per selector occurrence, in occurrence order.
    pub fn roots(&self) -> &[ProfilePackage] {
        &self.roots
    }

    /// The selected packages' dependency closure, dependency first.
    pub fn members(&self) -> &[SelectedProfile] {
        &self.members
    }

    /// Every member's composition input, in closure order.
    pub(super) fn application_inputs(&self) -> Vec<ProfileApplicationInput> {
        self.members
            .iter()
            .map(|member| member.input.clone())
            .collect()
    }

    /// The semantic contributions of every member, which each input composes
    /// its shared identities against.
    pub(super) fn contribution_context(&self) -> &[ProfileContributionClaim] {
        &self.contribution_context
    }
}

/// One member of a [`ProfileSelection`]'s closure.
#[derive(Clone)]
pub struct SelectedProfile {
    package: ProfilePackage,
    targets: Vec<VirtualPath>,
    input: ProfileApplicationInput,
}

impl SelectedProfile {
    /// The package as it was read.
    pub fn package(&self) -> &ProfilePackage {
        &self.package
    }
}

/// Profile-package and repository-initialization operations of a [`CommandExecutor`].
pub struct ProfileOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The profile-package and repository-initialization family.
    pub fn profiles(&self) -> ProfileOps<'_, S> {
        ProfileOps { exec: self }
    }
}

impl<S: IssueStore + RepositoryStateStore> ProfileOps<'_, S> {
    /// Resolve every selector in occurrence order before any package is applied.
    ///
    /// A repeated selector reads its package once.
    fn resolve_profile_selectors(&self, selectors: &[PathBuf]) -> Result<Vec<ProfilePackage>> {
        #[cfg(test)]
        crate::profile::selection_probe::record(
            crate::profile::selection_probe::SelectionStage::SelectorResolution,
        );
        let layout = self.exec.require_layout()?;
        let mut read = BTreeMap::<PathBuf, ProfilePackage>::new();
        selectors
            .iter()
            .map(|location| {
                let directory = supplied_package_directory(location, &layout)?;
                if let Some(package) = read.get(&directory) {
                    return Ok(package.clone());
                }
                let package = read_supplied_package(&directory, location)?;
                read.insert(directory, package.clone());
                Ok(package)
            })
            .collect()
    }

    /// Pack the package at `package_source` into one portable archive at
    /// `output`.
    ///
    /// The package directory is a worktree path resolved the way every other
    /// path argument is. The package is captured before it is archived, so live
    /// assets are drawn from their declared repository targets. A symbolic link, an
    /// entry of another kind, or content the manifest does not declare is
    /// refused before an archive exists. No resolved variable value and
    /// nothing outside the package or its declared repository authorities
    /// reaches the archive.
    ///
    /// The output path is the caller's, and it is published no-replace through
    /// the same classification `jit snapshot export` uses — into the repository
    /// through one redo record when it lands inside a repository
    /// root, and through the shared external publisher when it does not. An
    /// occupied output path is refused either way.
    ///
    /// The archive carries the package's identity digest so an add can verify
    /// it. That is integrity: it detects a truncated or corrupted archive, and
    /// says nothing about who produced one.
    ///
    /// # Errors
    ///
    /// A package directory that is not worktree content or does not read as a
    /// package; an archive that cannot be built; and an occupied or
    /// unpublishable output path.
    pub fn pack_profile_package(
        &self,
        invocation_dir: &Path,
        package_source: &Path,
        output: &Path,
    ) -> Result<(ProfilePackResult, Vec<String>)> {
        self.pack_profile_package_inner(invocation_dir, package_source, output, true)
    }

    /// Rehearse packing a package without writing the archive.
    pub fn plan_profile_pack(
        &self,
        invocation_dir: &Path,
        package_source: &Path,
        output: &Path,
    ) -> Result<(ProfilePackResult, Vec<String>)> {
        self.pack_profile_package_inner(invocation_dir, package_source, output, false)
    }

    fn pack_profile_package_inner(
        &self,
        invocation_dir: &Path,
        package_source: &Path,
        output: &Path,
        publish: bool,
    ) -> Result<(ProfilePackResult, Vec<String>)> {
        let layout = self.exec.require_layout()?;
        let source = worktree_directory(&layout, invocation_dir, package_source)?;
        let package = capture_package_tree(&source, &layout).map_err(|source_error| {
            ProfileResolutionError::UncapturableLocation {
                location: package_source.display().to_string(),
                source: source_error,
            }
        })?;
        let archive = pack_package_archive(&package)?;
        let archive_bytes = archive.len() as u64;
        let warnings =
            self.publish_package_archive(&layout, invocation_dir, output, archive, publish)?;
        Ok((
            ProfilePackResult::new(vec![ProfilePackEntry {
                id: package.model().id.clone(),
                version: package.model().version.clone(),
                origin: source.relative().clone(),
                package_hash: package.package_hash().clone(),
                source: source.repository_relative(),
                archive: output.display().to_string(),
                file_count: package.file_count(),
                byte_size: package.byte_size(),
                archive_bytes,
                targets: vec![ProfileTargetChange::new(
                    output.display().to_string(),
                    ProfileTargetAction::Create,
                    FileMode::Regular,
                    None,
                )],
                status: if publish {
                    ProfileApplicationStatus::Applied
                } else {
                    ProfileApplicationStatus::WouldApply
                },
            }]),
            warnings,
        ))
    }

    /// Place the package one archive carries at `destination`.
    ///
    /// The archive is read as hostile input: its bytes are bounded before they
    /// are held, every entry is validated as a safe relative path of an
    /// admissible kind, the extracted content is validated as a package, and
    /// its identity is recomputed and held against the digest the archive
    /// carries. Nothing is written until all of that has passed, so a refused
    /// or interrupted add leaves no partially extracted package behind.
    ///
    /// The destination is a worktree path and must be absent: an add publishes a package that arrived from
    /// outside, never over one already there.
    ///
    /// # Errors
    ///
    /// Every [`crate::profile::PackageArchiveError`] the read reports; an archive that cannot
    /// be read or is larger than a package archive may be; a destination that
    /// is not worktree content or is occupied; and the publication failures of
    /// the shared mutation session.
    pub fn add_profile_package(
        &self,
        invocation_dir: &Path,
        archive: &Path,
        destination: &Path,
    ) -> Result<ProfileAddResult> {
        self.add_profile_package_inner(invocation_dir, archive, destination, true)
    }

    /// Rehearse adding an archive without publishing its package tree.
    pub fn plan_profile_add(
        &self,
        invocation_dir: &Path,
        archive: &Path,
        destination: &Path,
    ) -> Result<ProfileAddResult> {
        self.add_profile_package_inner(invocation_dir, archive, destination, false)
    }

    fn add_profile_package_inner(
        &self,
        invocation_dir: &Path,
        archive: &Path,
        destination: &Path,
        publish: bool,
    ) -> Result<ProfileAddResult> {
        let layout = self.exec.require_layout()?;
        let destination = worktree_directory(&layout, invocation_dir, destination)?;
        let tree = read_package_archive(&read_archive_bytes(invocation_dir, archive)?)?;
        let targets = self.publish_package_tree(&layout, &destination, &tree, publish)?;
        Ok(ProfileAddResult::new(vec![ProfileAddEntry {
            id: tree.model().id.clone(),
            version: tree.model().version.clone(),
            origin: destination.relative().clone(),
            package_hash: tree.package_hash().clone(),
            archive: archive.display().to_string(),
            destination: destination.repository_relative(),
            file_count: tree.file_count(),
            byte_size: tree.byte_size(),
            targets,
            status: if publish {
                ProfileApplicationStatus::Applied
            } else {
                ProfileApplicationStatus::WouldApply
            },
        }]))
    }

    /// Publish one validated package tree at `destination`, answering every path
    /// it creates.
    ///
    /// The destination must be absent, which is enforced against the same
    /// captured image the plan is derived from. One that becomes occupied
    /// afterwards is refused at apply, because the plan creates it with an absent
    /// preimage. Unless `publish`, the plan is derived through the read driver
    /// and only answered.
    fn publish_package_tree(
        &self,
        layout: &RepositoryLayout,
        destination: &VirtualPath,
        tree: &ProfilePackage,
        publish: bool,
    ) -> Result<Vec<ProfileTargetChange>> {
        let capture = PackageTreeCapture::new(
            destination.clone(),
            tree.files()
                .iter()
                .map(|(relative, bytes)| {
                    Ok(CapturedTreeFile {
                        relative: RootRelativePath::parse(relative)?,
                        bytes: bytes.clone(),
                        mode: tree.model().file_mode(relative),
                    })
                })
                .collect::<Result<Vec<_>, RepositoryLayoutError>>()?,
        )?;
        let budget = package_tree_capture_budget(destination);
        let context = super::production_mutation_context();

        let derive = |session: &mut dyn RepositoryReadSession| -> Result<Option<_>> {
            let Some(image) =
                super::capture_or_retry(session.capture(capture.capture_spec(budget)?))?
            else {
                return Ok(None);
            };
            if !matches!(image.entry(destination)?, RepositoryEntry::Absent) {
                return Err(ProfileApplyError::PackageDestinationOccupied {
                    path: destination.repository_relative(),
                }
                .into());
            }
            let plan = finalize_package_tree_capture(&image, &context, &capture)?;
            let targets = plan
                .delta()
                .file_writes()
                .filter_map(|action| match action {
                    RepositoryAction::WriteFile { path, mode, .. } => {
                        Some(ProfileTargetChange::new(
                            path.repository_relative(),
                            ProfileTargetAction::Create,
                            *mode,
                            None,
                        ))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            Ok(Some((plan, targets)))
        };
        let store = self.exec.storage();
        let operation = "profile package tree";
        if !publish {
            return with_read_session(store, layout, operation, |session| {
                Ok(derive(session)?.map(|(_, targets)| targets))
            });
        }
        with_mutation_session(store, layout, operation, |session| {
            Ok(match derive(session)? {
                Some((plan, targets)) => SessionStep::Apply(plan, targets),
                None => SessionStep::Retry,
            })
        })
    }

    /// Publish one built archive at the output path the caller named.
    ///
    /// The destination is classified exactly as a snapshot export's is, so an
    /// archive written inside a repository root goes through the mutation
    /// session's redo record and one written outside goes through the shared external
    /// publisher. Both refuse an occupied path.
    fn publish_package_archive(
        &self,
        layout: &RepositoryLayout,
        invocation_dir: &Path,
        output: &Path,
        archive: Vec<u8>,
        publish: bool,
    ) -> Result<Vec<String>> {
        match classify_repository_export(layout, invocation_dir, output)? {
            RepositoryExportDestination::Repository(target) => {
                let intent = RepositoryExportIntent::new_absent_file(target, archive);
                // The export reads exactly two paths — the output path and its
                // parent — plus that parent's listing, whatever the path's
                // depth: it enumerates no ancestors, so nothing here scales
                // with depth. What occupies the output path is read to decide
                // it is occupied, and an occupant larger than a package archive
                // may be exhausts the byte budget: that is a refusal carrying a
                // less specific message than the occupied-path one, never a
                // publication.
                //
                // `max_depth` therefore bounds only how deep an operator may
                // name `--output`, and no package's own paths reach it: the
                // archive is one file, wherever the operator puts it.
                let budget = CaptureBudget {
                    max_listings: 1,
                    max_bytes: crate::profile::MAX_PROFILE_PACKAGE_ARCHIVE_BYTES as u64,
                    max_depth: 128,
                };
                let result = if publish {
                    self.exec.publish_repository_export(layout, &intent, budget)
                } else {
                    self.exec.plan_repository_export(layout, &intent, budget)
                };
                super::map_occupied_export_error(result, output)?;
                Ok(Vec::new())
            }
            RepositoryExportDestination::External(path) => {
                if !publish {
                    crate::storage::external_publish::preflight_external_file_noreplace(&path)?;
                    return Ok(Vec::new());
                }
                // Staging is not publication: the archive lands in a temporary
                // file outside both repository roots so the shared no-replace
                // publisher can rename it into place, and nothing at the output
                // path is touched until that publisher commits.
                use std::io::Write as _;
                let mut staged = tempfile::NamedTempFile::new()?;
                staged.write_all(&archive)?;
                staged.as_file().sync_all()?;
                Ok(
                    crate::storage::external_publish::publish_external_file_noreplace(
                        &path,
                        staged.path(),
                    )?
                    .warnings,
                )
            }
        }
    }

    /// Inspect every selected profile package in selector occurrence order.
    pub fn show_profiles(&self, selectors: &[PathBuf]) -> Result<ProfileShowResult> {
        let layout = self.exec.require_layout()?;
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
        Ok(ProfileShowEntry {
            id: package.model().id.clone(),
            version: package.model().version.clone(),
            manifest: package.model().clone(),
            origin: package_origin(package, layout)?,
            package_hash: package.package_hash().clone(),
            targets: package.model().targets(),
            file_count: package.file_count(),
            byte_size: package.byte_size(),
        })
    }

    /// Build the selection one invocation acts on.
    ///
    /// # Errors
    ///
    /// An unreadable selector, an unsatisfiable dependency graph, an
    /// unreadable values file, a variable input the closure does not declare,
    /// and a package that cannot be resolved or composed.
    pub fn select_profiles(
        &self,
        selectors: &[PathBuf],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileSelection> {
        let roots = self.resolve_profile_selectors(selectors)?;
        let graph = self.resolve_profile_graph(&roots)?;
        let closure = graph.selected_packages();
        let variable_inputs = load_profile_variable_inputs(&closure, options)?;
        let layout = self.exec.require_layout()?;
        let members = closure
            .into_iter()
            .map(|package| {
                let targets = package
                    .model()
                    .targets()
                    .iter()
                    .map(|target| {
                        layout
                            .classify_repository_relative(target)
                            .map_err(Into::into)
                    })
                    .collect::<Result<Vec<_>>>()?;
                reject_reserved_application_targets(targets.iter())?;
                let resolved = resolve_package(
                    &package,
                    &variable_inputs.for_declarations(&package.model().variables),
                )?;
                let input = profile_application_input(&resolved, &layout)?;
                Ok(SelectedProfile {
                    package,
                    targets,
                    input,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let claims = drop_overridden_claims(
            members
                .iter()
                .map(|member: &SelectedProfile| member.input.claims.clone())
                .collect(),
            &graph.transitive_dependencies(),
        );
        let contribution_context = claims
            .iter()
            .flat_map(|claims| claims.contributions.iter().cloned())
            .collect::<Vec<_>>();
        let members = members
            .into_iter()
            .zip(claims)
            .map(|(member, claims)| SelectedProfile {
                input: ProfileApplicationInput {
                    claims,
                    ..member.input
                }
                .with_contribution_context(&contribution_context),
                ..member
            })
            .collect();
        Ok(ProfileSelection {
            roots,
            members,
            contribution_context,
        })
    }

    /// Rehearse applying `selection`: a read-only run that fails exactly where
    /// the application would, final validation included.
    pub fn plan_profiles(
        &self,
        selection: &ProfileSelection,
    ) -> Result<ProfileComposedApplyResult> {
        self.run_profile_selection(selection, ProfileRunMode::Rehearse)
    }

    /// Apply `selection`'s complete closure through one held session.
    ///
    /// Selector occurrence order remains a result concern only.
    pub fn apply_profiles(
        &self,
        selection: &ProfileSelection,
    ) -> Result<ProfileComposedApplyResult> {
        self.run_profile_selection(selection, ProfileRunMode::Publish)
    }

    /// Report what applying a selection would do to this repository, conflicts
    /// included, without publishing any of it.
    ///
    /// A package that is not installed is reported as readily as one that is,
    /// so an adopter inspects a package before applying it. The report derives
    /// from the capture the publication would, admitted as one repository
    /// version.
    ///
    /// # Errors
    ///
    /// A repository that cannot be read.
    pub fn diff_profiles(
        &self,
        selection: &ProfileSelection,
    ) -> Result<ProfileComposedApplyResult> {
        self.run_profile_selection(selection, ProfileRunMode::Report)
    }

    /// Derive `selection`'s dependency-first unique closure from one capture:
    /// a publication's through one mutation session and one `session.apply`
    /// call at most, a preview's through the read driver.
    fn run_profile_selection(
        &self,
        selection: &ProfileSelection,
        mode: ProfileRunMode,
    ) -> Result<ProfileComposedApplyResult> {
        if selection.members.is_empty() {
            return Ok(ProfileComposedApplyResult::new(Vec::new()));
        }
        let layout = self.exec.require_layout()?;
        let context = match mode {
            ProfileRunMode::Publish => super::production_mutation_context(),
            ProfileRunMode::Rehearse | ProfileRunMode::Report => MutationContext::preview(),
        };
        let label = match mode {
            ProfileRunMode::Publish => "profile application",
            ProfileRunMode::Rehearse | ProfileRunMode::Report => "profile planning",
        };
        let derive = |session: &mut dyn RepositoryReadSession| -> Result<Option<_>> {
            let Some(plan) = self.prepare_profile_selection(session, selection, &context, mode)?
            else {
                return Ok(None);
            };
            // A plan its decisions already refuse never reaches final validation
            // in the publication this reports on, and its overlay omits what
            // they refuse.
            let refusal = match mode {
                ProfileRunMode::Report
                    if crate::profile::ensure_publishable_targets(&plan).is_ok() =>
                {
                    final_validation_refusal(&plan)?.map(|refusal| refusal.to_string())
                }
                _ => None,
            };
            let entries = selection
                .members
                .iter()
                .map(|member| {
                    let entry = profile_result(&plan, &member.package, &layout, mode)?;
                    Ok(match &refusal {
                        Some(refusal) => entry.refused_by(refusal),
                        None => entry,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some((plan, entries)))
        };
        let closure = match mode {
            ProfileRunMode::Publish => {
                with_mutation_session(self.exec.storage(), &layout, label, |session| {
                    Ok(match derive(session)? {
                        None => SessionStep::Retry,
                        Some((plan, entries)) if !plan.delta().actions().is_empty() => {
                            SessionStep::Apply(plan, entries)
                        }
                        Some((_, entries)) => SessionStep::Done(entries),
                    })
                })
            }
            ProfileRunMode::Rehearse | ProfileRunMode::Report => {
                with_read_session(self.exec.storage(), &layout, label, |session| {
                    Ok(derive(session)?.map(|(_, entries)| entries))
                })
            }
        }?;
        selection_profile_results(closure, &selection.roots)
    }

    /// Resolve the selected packages' dependency closure, then settle every
    /// dependency, incompatibility, and engine range claim before a caller
    /// prepares a publication.
    ///
    /// Only the selection's own closure is a candidate: a package this
    /// repository applied earlier takes no part unless the selection reaches it.
    fn resolve_profile_graph(&self, selected: &[ProfilePackage]) -> Result<ResolvedProfileGraph> {
        #[cfg(test)]
        crate::profile::selection_probe::record(
            crate::profile::selection_probe::SelectionStage::GraphResolution,
        );
        let mut packages = BTreeMap::new();
        let mut selected_ids = BTreeSet::new();
        for package in selected {
            selected_ids.insert(package.model().id.clone());
            insert_candidate_package(&mut packages, package.clone())?;
        }
        let mut pending = selected_ids.iter().cloned().collect::<VecDeque<_>>();
        self.load_package_dependencies(&mut packages, &mut pending)?;

        let engine = EngineVersion::running()
            .map_err(|source| anyhow::anyhow!("running JIT engine version is invalid: {source}"))?;
        Ok(ResolvedProfileGraph::resolve(
            packages,
            selected_ids,
            &engine,
        )?)
    }

    /// Extend `packages` with dependencies in canonical id order.
    fn load_package_dependencies(
        &self,
        packages: &mut BTreeMap<ProfileId, ProfilePackage>,
        pending: &mut VecDeque<ProfileId>,
    ) -> Result<()> {
        while let Some(id) = pending.pop_front() {
            let declaring = packages
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("profile '{id}' left the graph being built"))?;
            let mut dependencies = declaring.model().dependencies.clone();
            dependencies.sort_by_cached_key(|dependency| {
                (dependency.id.clone(), dependency.version.to_string())
            });
            for dependency in dependencies {
                if packages.contains_key(&dependency.id) {
                    continue;
                }
                let package = self
                    .resolve_dependency_package_from(&declaring, &dependency.id)
                    .map_err(|cause| ProfileResolutionError::UnresolvableDependency {
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

    /// Resolve a dependency from a sibling package.
    ///
    /// A package read from a directory states where its dependencies are by
    /// where it sits: the dependency is looked for beside it, in a directory
    /// named by the dependency's own id. A directory that is not there, or that
    /// holds a package declaring another profile, is not that dependency, so
    /// the dependency is not found. A directory that is there and cannot be
    /// read as a package is reported rather than passed over, because its
    /// failure is the actionable answer.
    fn resolve_dependency_package_from(
        &self,
        declaring: &ProfilePackage,
        dependency: &ProfileId,
    ) -> Result<ProfilePackage> {
        let not_found = || -> anyhow::Error {
            ProfileNotFoundError {
                profile: dependency.clone(),
            }
            .into()
        };
        let Some(location) = declaring
            .directory()
            .and_then(Path::parent)
            .map(|parent| parent.join(dependency.as_str()))
        else {
            return Err(not_found());
        };
        match supplied_package(&location, &self.exec.require_layout()?) {
            Ok(package) if package.model().id == *dependency => Ok(package),
            Ok(_) => Err(not_found()),
            Err(error) if is_unreadable_location(&error) => Err(not_found()),
            Err(error) => Err(error),
        }
    }

    /// Capture and close the complete selection once. A retry restarts from a
    /// fresh session image; no member of the selection is published until
    /// the final aggregate plan reaches `session.apply`.
    ///
    /// `mode` decides what a decision that cannot be published does here. A
    /// publication and its rehearsal refuse it, final validation included, and
    /// the difference report alone carries it through.
    fn prepare_profile_selection(
        &self,
        session: &mut (dyn RepositoryReadSession + '_),
        selection: &ProfileSelection,
        context: &MutationContext,
        mode: ProfileRunMode,
    ) -> Result<Option<MaterializationPlan>> {
        let contribution_context = selection.contribution_context();
        let inputs = selection.application_inputs();

        let mut content_paths = selection
            .members
            .iter()
            .flat_map(|member| member.targets.iter().cloned())
            .collect::<Vec<_>>();
        content_paths.extend(
            content_paths
                .clone()
                .into_iter()
                .map(|path| path.ancestor_directories())
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten(),
        );
        content_paths.extend(crate::repository_state::profile_contribution_target_paths(
            contribution_context,
        ));

        let base = match self.exec.capture_proposed_base(
            session,
            &BTreeMap::new(),
            &content_paths,
            None,
        )? {
            None => return Ok(None),
            Some(base) => base,
        };

        let contribution_base = apply_overlay(
            &base,
            crate::repository_state::profile_contribution_overrides(&base, contribution_context)?,
        )?;
        let mut expanded_paths = content_paths;
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
        let Some(base) = self.exec.capture_proposed_base(
            session,
            &BTreeMap::new(),
            &expanded_paths,
            Some(base),
        )?
        else {
            return Ok(None);
        };
        let preview = derive_materialization(
            &base,
            context,
            MaterializationRequest::ApplyProfileSelection {
                profiles: &inputs,
                operation: ProfileLifecycleOperation::Apply,
            },
        )?;
        let mut extra_paths = expanded_paths;
        extra_paths.extend(
            preview
                .delta()
                .actions()
                .iter()
                .map(|action| action.path().clone())
                .filter(|path| path != &VirtualPath::EVENTS),
        );
        let overrides = crate::repository_state::action_overrides(preview.delta().actions());
        let Some(probe) =
            self.exec
                .capture_proposed_base(session, &overrides, &extra_paths, Some(base))?
        else {
            return Ok(None);
        };
        let contribution_probe = apply_overlay(
            &probe,
            crate::repository_state::profile_contribution_overrides(&probe, contribution_context)?,
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
            context,
            MaterializationRequest::ApplyProfileSelection {
                profiles: &inputs,
                operation: ProfileLifecycleOperation::Apply,
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
        if mode != ProfileRunMode::Report {
            crate::profile::ensure_publishable_targets(&plan)
                .map_err(RepositoryStateError::from)?;
            ensure_committed_write_targets(
                &plan,
                &self.exec.require_layout()?,
                self.exec.require_worktree_paths()?,
            )?;
            if let Some(refusal) = final_validation_refusal(&plan)? {
                return Err(refusal.into());
            }
        }
        Ok(Some(plan))
    }
}

/// Why final validation refuses the repository `plan` would publish, if it does.
fn final_validation_refusal(plan: &MaterializationPlan) -> Result<Option<ProfileApplyError>> {
    if plan.delta().actions().is_empty() {
        return Ok(None);
    }
    let overrides = crate::repository_state::action_overrides(plan.delta().actions());
    let proposed = apply_overlay(plan.image(), overrides)?;
    let validation = crate::validation::report(
        &crate::validation::CapturedValidation::from_image(&proposed),
        &crate::validation::ReportInputs {
            now: chrono::Utc::now(),
            history: None,
        },
    );
    Ok(match validation.into_parts() {
        (Some(error), _) => Some(ProfileApplyError::FinalValidation(error)),
        (None, rule_report) if rule_report.has_errors() => {
            Some(ProfileApplyError::FinalValidationFindings {
                error_count: rule_report.error_count(),
            })
        }
        (None, _) => None,
    })
}

/// Refuse `plan` when a path its delta writes differs from `HEAD`.
///
/// A path outside the detected checkout proceeds.
fn ensure_committed_write_targets(
    plan: &MaterializationPlan,
    layout: &RepositoryLayout,
    paths: &WorktreePaths,
) -> Result<()> {
    let written = plan
        .delta()
        .file_writes()
        .map(RepositoryAction::path)
        .collect::<Vec<_>>();
    if written.is_empty() {
        return Ok(());
    }
    // Git reports paths from its physical top level.
    let worktree_root = layout.worktree_root().canonicalize()?;
    let data_root = layout.data_root().canonicalize()?;
    let written = written
        .into_iter()
        .map(|path| {
            match path.root_class() {
                RepositoryRootClass::Worktree => &worktree_root,
                RepositoryRootClass::Data => &data_root,
            }
            .join(path.relative().as_path())
        })
        .collect::<BTreeSet<_>>();
    let mut uncommitted = BTreeSet::new();
    let mut checked = BTreeSet::new();
    let checkouts = [
        paths
            .write_checkout()
            .map(|checkout| (checkout, paths.checkout_top_level())),
        paths.code_checkout().zip(paths.code_checkout_top_level()),
    ];
    for (checkout, top) in checkouts.into_iter().flatten() {
        if !checked.insert(top.to_path_buf()) {
            continue;
        }
        for dirty in checkout.dirty_paths()? {
            if written.contains(&top.join(&dirty)) {
                uncommitted.insert(dirty.display().to_string());
            }
        }
    }
    if uncommitted.is_empty() {
        return Ok(());
    }
    Err(ProfileApplyError::UncommittedTargets {
        paths: uncommitted.into_iter().collect(),
    }
    .into())
}

/// Insert one package image without allowing selector or dependency traversal
/// order to choose between different images for one semantic profile id.
fn insert_candidate_package(
    packages: &mut BTreeMap<ProfileId, ProfilePackage>,
    package: ProfilePackage,
) -> Result<()> {
    let id = package.model().id.clone();
    if let Some(existing) = packages.get(&id) {
        if existing.package_hash() != package.package_hash() {
            let (first_hash, second_hash) = if existing.package_hash() < package.package_hash() {
                (
                    existing.package_hash().clone(),
                    package.package_hash().clone(),
                )
            } else {
                (
                    package.package_hash().clone(),
                    existing.package_hash().clone(),
                )
            };
            return Err(ProfileGraphError::ConflictingPackageIdentity {
                id: id.to_string(),
                first_hash,
                second_hash,
            }
            .into());
        }
        // A stable choice between byte-identical copies; package bytes and graph
        // claims remain the semantic identity.
        if package.directory() < existing.directory() {
            packages.insert(id, package);
        }
        return Ok(());
    }
    packages.insert(id, package);
    Ok(())
}

/// How many package-model byte budgets one package-tree capture may read.
///
/// A publication writes only over absent paths, so this bounds what an occupied
/// destination holds at the tree's paths: past it, an add is refused for the
/// budget rather than for the occupant.
const PACKAGE_TREE_MODEL_BUDGETS: usize = 8;

/// Bounds for publishing one package tree at `destination`.
///
/// Each bound derives from what the package model constrains: the model bounds
/// a package's file count and total size and leaves its paths' depth and length
/// free, so a bound assuming a path shape would refuse an archive `jit profile
/// pack` writes. The manifest names every file besides itself and no source
/// twice, so the declared path lengths sum below the package byte budget. That
/// bounds the directories the tree can need ([`CaptureBudget::max_listings`])
/// and the components of any one path ([`CaptureBudget::max_depth`]).
fn package_tree_capture_budget(destination: &VirtualPath) -> CaptureBudget {
    // Saturating throughout: a bound that wrapped would be smaller than the
    // model demands.
    const READ_BYTES: usize =
        PACKAGE_TREE_MODEL_BUDGETS.saturating_mul(crate::profile::MAX_PROFILE_PACKAGE_BYTES);

    CaptureBudget {
        max_listings: READ_BYTES.saturating_add(1),
        max_bytes: READ_BYTES as u64,
        max_depth: destination
            .relative()
            .depth()
            .saturating_add(crate::profile::MAX_PROFILE_PACKAGE_BYTES),
    }
}

/// Resolve and classify a selector before a cached package can satisfy it.
fn supplied_package_directory(location: &Path, layout: &RepositoryLayout) -> Result<PathBuf> {
    let requested = if location.is_absolute() {
        location.to_path_buf()
    } else {
        layout.worktree_root().join(location)
    };
    let directory = ProfilePackage::resolve_directory(&requested).map_err(|source| {
        ProfileResolutionError::UnreadableLocation {
            location: location.display().to_string(),
            source,
        }
    })?;
    ensure_worktree_package_directory(&requested, layout)?;
    ensure_worktree_package_directory(&directory, layout)?;
    Ok(directory)
}

fn read_supplied_package(directory: &Path, location: &Path) -> Result<ProfilePackage> {
    ProfilePackage::from_directory(directory)
        .map_err(|source| ProfileResolutionError::UnreadableLocation {
            location: location.display().to_string(),
            source,
        })
        .map_err(Into::into)
}

/// Read the package at a location the caller supplied.
fn supplied_package(location: &Path, layout: &RepositoryLayout) -> Result<ProfilePackage> {
    read_supplied_package(&supplied_package_directory(location, layout)?, location)
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

/// Resolve one caller-supplied package directory to worktree content.
///
/// A package directory is repository material an adopter reads and commits, so
/// the separate data root and anything outside the repository are refused —
/// the same rule [`package_origin`] applies to the directory a package's bytes
/// were read from, applied here to a directory the caller only named.
/// Resolution goes through the shared export classifier, so a relative path
/// means what the invoking shell means by it.
fn worktree_directory(
    layout: &RepositoryLayout,
    invocation_dir: &Path,
    requested: &Path,
) -> Result<VirtualPath> {
    let outside = || ProfileApplyError::PackageOutsideWorktree {
        path: requested.display().to_string(),
    };
    match classify_repository_export(layout, invocation_dir, requested)? {
        RepositoryExportDestination::Repository(path)
            if path.root_class() == RepositoryRootClass::Worktree =>
        {
            Ok(path)
        }
        RepositoryExportDestination::Repository(_) | RepositoryExportDestination::External(_) => {
            Err(outside().into())
        }
    }
}

/// Read one archive the caller named, bounded before its bytes are held.
///
/// The archive is external input an adopter obtained over a channel they chose,
/// so the path is the caller's to name and is resolved against the invocation
/// directory like any other path argument; the worktree boundary that governs
/// where a package may be *published* says nothing about where an archive may
/// be read from. Nothing else is opened: no directory is searched for it, and
/// no other location is consulted when it is absent.
///
/// The read stops one byte past what a package archive may occupy, so an
/// enormous file is refused rather than held.
fn read_archive_bytes(invocation_dir: &Path, archive: &Path) -> Result<Vec<u8>> {
    use std::io::Read as _;

    let path = if archive.is_absolute() {
        archive.to_path_buf()
    } else {
        invocation_dir.join(archive)
    };
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| {
            file.take(MAX_PROFILE_PACKAGE_ARCHIVE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|source| ProfileResolutionError::UnreadableArchive {
            location: archive.display().to_string(),
            source,
        })?;
    if bytes.len() > MAX_PROFILE_PACKAGE_ARCHIVE_BYTES {
        anyhow::bail!(
            "profile package archive '{}' is larger than {MAX_PROFILE_PACKAGE_ARCHIVE_BYTES} bytes",
            archive.display()
        );
    }
    Ok(bytes)
}

/// The worktree-relative origin a package's own bytes came through.
///
/// The location is taken from the directory the package recorded while reading
/// itself, never from a path supplied beside it. A directory that resolves
/// outside the worktree is refused, because a report names a package by a
/// location that does not depend on machine state. The selected data root is
/// not a worktree location even when it nests inside one, so a package placed
/// under it is refused by the same rule.
fn package_origin(package: &ProfilePackage, layout: &RepositoryLayout) -> Result<RootRelativePath> {
    let Some(directory) = package.directory() else {
        anyhow::bail!(
            "profile '{}' was not read from a directory, so it names no origin",
            package.model().id
        );
    };
    layout
        .classify_and_canonicalize(directory)
        .ok()
        .filter(|path| path.root_class() == RepositoryRootClass::Worktree)
        .map(|path| path.relative().clone())
        .ok_or_else(|| {
            ProfileApplyError::PackageOutsideWorktree {
                path: directory.display().to_string(),
            }
            .into()
        })
}

/// One member's answer to a run, from the decision `plan` carries.
pub(super) fn profile_result(
    plan: &MaterializationPlan,
    package: &ProfilePackage,
    layout: &RepositoryLayout,
    mode: ProfileRunMode,
) -> Result<ProfileApplyResult> {
    Ok(profile_run_entry(
        plan,
        &package.model().id,
        &package.model().version,
        package_origin(package, layout)?,
        mode,
    ))
}

/// Convert resolved package content into neutral claims.
pub(super) fn profile_application_input(
    resolved: &ResolvedProfileContent,
    layout: &RepositoryLayout,
) -> Result<ProfileApplicationInput> {
    let metadata = resolved.model();
    let claims = build_profile_claims_from_resolved(resolved, layout)?;
    Ok(ProfileApplicationInput {
        id: metadata.id.clone(),
        version: metadata.version.clone(),
        variables: resolved.variables().clone(),
        targets: metadata.targets(),
        contribution_context: claims.contributions.clone(),
        claims,
    })
}

fn reject_reserved_application_targets<'a>(
    targets: impl IntoIterator<Item = &'a VirtualPath>,
) -> Result<()> {
    if let Some(path) = targets
        .into_iter()
        .find(|path| path.is_reserved_application_target())
    {
        return Err(ProfileApplyError::ReservedApplicationTarget {
            path: path.repository_relative(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Event, EventBody, ProfileLifecycleOperation, ProfileLifecycleStatus};
    use crate::profile::selection_probe;
    use crate::profile::{ProfileApplyResult, ProfileContributionChange, ProfileTargetAction};
    use crate::repository_state::{
        Contribution, MapEntryTarget, ProfileTargetConflict, ProfileTargetConflictEntry,
        ProfileTargetSubject, RepositoryStateError, ScalarTarget, SetStringTarget,
    };
    use crate::storage::JsonFileStorage;
    use crate::storage::{
        discover_repository_layout, IssueStore, MutationPoint, RepositoryStateStore,
    };
    use crate::test_utils::{
        apply_package, apply_selected_packages, file_executor, selected_closure, setup_test_repo,
        setup_unsynced_test_repo_with_config,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Fail one named session moment, then let the next mutation session run.
    struct FailOnce {
        point: MutationPoint,
        fired: AtomicBool,
    }

    impl FailOnce {
        fn at(point: MutationPoint) -> Arc<Self> {
            Arc::new(Self {
                point,
                fired: AtomicBool::new(false),
            })
        }
    }

    impl crate::storage::MutationInterruptionInjector for FailOnce {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            if point == &self.point && !self.fired.swap(true, Ordering::SeqCst) {
                return Err(std::io::Error::other(format!("injected {point:?}")));
            }
            Ok(())
        }
    }

    /// The checked-in fixture tree every case below stages a copy of.
    fn fixture_package_tree() -> std::path::PathBuf {
        crate::test_utils::profile_package_fixture("planner-asset-only")
    }

    /// Worktree-relative location the cases stage the fixture package at.
    ///
    /// A package is applied from inside the repository it is applied to,
    /// because application refuses a package outside the worktree.
    const FIXTURE_LOCATION: &str = "packages/planner";

    /// The fixture package, written into the repository at `relative` and read
    /// back from there.
    fn package_read_from(temp: &TempDir, relative: &str) -> ProfilePackage {
        let root = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &temp.path().join(relative),
        );
        ProfilePackage::from_directory(&root).expect("a valid package tree")
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

    /// Apply `selectors` the way `jit profile apply` does.
    fn apply_selected(
        executor: &CommandExecutor<JsonFileStorage>,
        selectors: &[PathBuf],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        executor
            .profiles()
            .select_profiles(selectors, options)
            .and_then(|selection| executor.profiles().apply_profiles(&selection))
    }

    /// Resolve `selector` to the one package it names.
    fn resolved_selector(
        executor: &CommandExecutor<JsonFileStorage>,
        selector: &PathBuf,
    ) -> Result<ProfilePackage> {
        executor
            .profiles()
            .resolve_profile_selectors(std::slice::from_ref(selector))
            .map(|mut packages| packages.remove(0))
    }

    /// The canonical plan `package`'s closure prepares in `session`, and what
    /// `package` decides in it.
    fn prepared_plan(
        executor: &CommandExecutor<JsonFileStorage>,
        session: &mut (dyn RepositoryReadSession + '_),
        package: &ProfilePackage,
        context: &MutationContext,
    ) -> (MaterializationPlan, Vec<ProfileTargetChange>) {
        let selection = executor
            .profiles()
            .select_profiles(
                &[package_selector(package)],
                &ProfileVariableOptions::default(),
            )
            .unwrap();
        let plan = executor
            .profiles()
            .prepare_profile_selection(session, &selection, context, ProfileRunMode::Rehearse)
            .unwrap()
            .unwrap();
        let changes = profile_result(
            &plan,
            package,
            &executor.require_layout().unwrap(),
            ProfileRunMode::Rehearse,
        )
        .unwrap()
        .targets;
        (plan, changes)
    }

    /// Rehearse `selectors` the way `jit profile apply --dry-run` does.
    fn plan_selected(
        executor: &CommandExecutor<JsonFileStorage>,
        selectors: &[PathBuf],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        executor
            .profiles()
            .select_profiles(selectors, options)
            .and_then(|selection| executor.profiles().plan_profiles(&selection))
    }

    /// Report `selectors` the way `jit profile diff` does.
    fn diff_selected(
        executor: &CommandExecutor<JsonFileStorage>,
        selectors: &[PathBuf],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        executor
            .profiles()
            .select_profiles(selectors, options)
            .and_then(|selection| executor.profiles().diff_profiles(&selection))
    }

    /// Select a package through the worktree location it was read from.
    fn package_selector(package: &ProfilePackage) -> PathBuf {
        package
            .directory()
            .expect("the fixture package was read from a directory")
            .to_path_buf()
    }

    /// The profile ids this repository's audit log records as applied, in the
    /// order it recorded them.
    fn applied_event_ids(storage: &JsonFileStorage) -> Vec<String> {
        storage
            .read_events()
            .unwrap()
            .into_iter()
            .flat_map(|event| match event.body() {
                EventBody::ProfileLifecycle { profiles, .. } => profiles
                    .iter()
                    .map(|profile| profile.id.to_string())
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
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

    /// Every file this repository holds, keyed by its path below the worktree.
    ///
    /// A rehearsal writes nothing, which is a statement about the whole
    /// repository rather than about the targets a plan happens to name. The
    /// coordination locks a held session creates are machine-local rather than
    /// repository content, so holding a session is not a write.
    fn repository_files(temp: &TempDir) -> BTreeMap<PathBuf, Vec<u8>> {
        fn collect(directory: &Path, root: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(directory).expect("read a repository directory") {
                let path = entry.expect("a readable directory entry").path();
                if path.is_dir() {
                    collect(&path, root, files);
                } else if path.extension().is_none_or(|extension| extension != "lock") {
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

    /// Every target `error` refused, in the order the plan decided them.
    ///
    /// A selection is refused as a whole and carries every target that refused
    /// it, so a test reads that set rather than the variant that carried it
    /// (`@/invariant/semantic-test-assertions`).
    fn refused_targets(error: &anyhow::Error) -> &[ProfileTargetConflictEntry] {
        match error.downcast_ref::<RepositoryStateError>() {
            Some(RepositoryStateError::ProfileTargetConflicts(refusal)) => &refusal.conflicts,
            _ => panic!("an unpublishable profile target fails as a target conflict: {error:#}"),
        }
    }

    /// The repository target one refused decision names.
    fn refused_path(entry: &ProfileTargetConflictEntry) -> String {
        match &entry.subject {
            ProfileTargetSubject::File { path } => path.repository_relative(),
            ProfileTargetSubject::Contribution { identity } => {
                panic!("this refusal is about a target, not the declaration '{identity}'")
            }
        }
    }

    #[test]
    fn test_resolve_profile_selectors_reads_the_package_a_supplied_location_holds() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let supplied = package_read_from(&temp, "vendor/supplied");

        let selector = temp.path().join("vendor/supplied");
        let resolved = resolved_selector(&executor, &selector).unwrap();

        assert_eq!(resolved.package_hash(), supplied.package_hash());
        assert_eq!(
            package_origin(&resolved, &executor.require_layout().unwrap()).unwrap(),
            RootRelativePath::parse("vendor/supplied").unwrap()
        );
    }

    #[test]
    fn test_apply_profiles_resolves_a_declared_dependency_beside_the_declaring_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let config_path = temp.path().join(".jit/config.toml");
        let scaffolded = fs::read_to_string(&config_path).unwrap();
        let mut bare = scaffolded.parse::<toml_edit::DocumentMut>().unwrap();
        bare.as_table_mut()
            .retain(|key, _| matches!(key, "version" | "project"));
        fs::write(&config_path, bare.to_string()).unwrap();

        crate::test_utils::capture_repository_package(
            "jit-default",
            &temp.path().join("vendor/jit-default"),
        )
        .expect("this repository's jit-default package captures");
        let workflow = package_declaring(&temp, "vendor/workflow", "workflow", &["jit-default"]);
        let applied = apply_package(&executor, &workflow).unwrap();

        assert_eq!(
            applied
                .items
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["jit-default", "workflow"]
        );
    }

    #[test]
    fn test_apply_workflow_profile_to_bare_repository_contributes_hierarchy_rules() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
            .retain(|rule| rule["assert"].get("type-hierarchy").is_none());
        fs::write(&rules_path, rules.to_string()).unwrap();
        // The workflow package and the package it depends on, side by side
        // inside the worktree, which is where a declared dependency is
        // looked for.
        shipped_package_in(temp.path(), "jit-default");
        let workflow = shipped_package_in(temp.path(), "jit-dogfood");

        apply_package(&executor, &workflow).unwrap_or_else(|error| panic!("{error:#}"));

        let rules: toml::Value =
            toml::from_str(&fs::read_to_string(temp.path().join(".jit/rules.toml")).unwrap())
                .unwrap();
        let rules = rules["rules"].as_array().unwrap();
        for expected in crate::declarations::rules::TypeHierarchyKind::ALL {
            assert!(
                rules.iter().any(|rule| {
                    rule.get("assert")
                        .and_then(|assert| assert.get("type-hierarchy"))
                        .and_then(|hierarchy| hierarchy.get("kind"))
                        .and_then(toml::Value::as_str)
                        == Some(expected.as_str())
                        && rule["origin"].as_str() == Some("jit-dogfood")
                }),
                "workflow profile did not contribute {expected}: {rules:?}"
            );
        }
    }

    #[test]
    fn test_apply_profile_selection_recovers_an_obsolete_default_schema_atomically() {
        let temp = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let dogfood = ProfilePackage::from_directory(&temp.path().join("profiles/jit-dogfood"))
            .expect("the verified baseline retains its selected package");

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
        let storage = JsonFileStorage::with_mutation_interruptions(
            temp.path().join(".jit"),
            FailOnce::at(MutationPoint::BeforeCommitCheck),
        );
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap())
            .unwrap();

        assert!(apply_package(&executor, &dogfood).is_err());

        let recovered = JsonFileStorage::new(temp.path().join(".jit"));
        let layout = discover_repository_layout(temp.path(), recovered.root()).unwrap();
        drop(recovered.open_mutation_session(layout).unwrap());
        assert!(stale_schema.exists());
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events_before
        );

        let repaired = apply_package(
            &CommandExecutor::new(recovered.clone())
                .with_layout(discover_repository_layout(temp.path(), recovered.root()).unwrap())
                .unwrap(),
            &dogfood,
        )
        .unwrap();
        assert!(!stale_schema.exists());
        assert_eq!(
            repaired
                .items
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
    fn test_profile_selector_parse_reads_a_bare_directory_path() {
        assert_eq!(
            parse_profile_selector("packages/workflow"),
            Ok(PathBuf::from("packages/workflow"))
        );
    }

    #[test]
    fn test_profile_selector_parse_refuses_a_retired_tag_naming_the_bare_path_form() {
        for selector in ["id:workflow", "path:packages/workflow", "path:"] {
            let error = parse_profile_selector(selector).expect_err("a tagged selector is refused");
            assert!(
                matches!(&error, ProfileSelectorError::TaggedSelector { selector: given } if given == selector),
                "{error}"
            );
            assert!(
                error.to_string().contains("bare package directory path"),
                "{error}"
            );
        }
    }

    #[test]
    fn test_profile_selector_parse_refuses_an_empty_path() {
        assert_eq!(
            parse_profile_selector(""),
            Err(ProfileSelectorError::EmptyPath)
        );
    }

    #[test]
    fn test_resolve_profile_selectors_path_selector_uses_the_directory_package_id() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, "vendor/supplied");
        let selector = temp.path().join("vendor/supplied");
        let resolved = resolved_selector(&executor, &selector).unwrap();
        assert_eq!(resolved.model().id, package.model().id);
    }

    #[test]
    fn test_resolve_profile_selectors_reports_a_supplied_location_holding_no_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();

        let selector = temp.path().join("vendor/absent");
        let error = resolved_selector(&executor, &selector).unwrap_err();

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
    fn test_apply_profiles_judges_only_paths_inside_the_data_roots_checkout() {
        let temp = TempDir::new().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path().join("repo"));
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let storage = JsonFileStorage::new(repository.root().join("state"));
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(&outside, storage.root()).unwrap())
            .unwrap()
            .with_worktree_paths(
                crate::storage::WorktreePaths::detect_for_data_root(storage.root(), &outside)
                    .unwrap(),
            );
        executor
            .profiles()
            .initialize_fresh_repository(&outside, None)
            .unwrap();
        repository.commit("track the store");
        let package = authored_package(
            &temp,
            "outside/packages/first",
            "first",
            "docs/first.txt",
            "rendered\n",
            "\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\nidentity = \"shared\"\nvalue = { description = \"shared\", unique = false }\n",
        );
        let outside_target = outside.join("docs/first.txt");
        fs::create_dir(outside.join("docs")).unwrap();
        fs::write(&outside_target, "a local edit\n").unwrap();
        let config = repository.root().join("state/config.toml");
        fs::write(
            &config,
            format!("{}\n# a local edit\n", fs::read_to_string(&config).unwrap()),
        )
        .unwrap();

        let error = apply_package(&executor, &package).unwrap_err();

        assert!(matches!(
            error.downcast_ref::<ProfileApplyError>(),
            Some(ProfileApplyError::UncommittedTargets { paths })
                if paths.as_slice() == ["state/config.toml"]
        ));

        repository.git(&["checkout", "--", "state/config.toml"]);
        apply_package(&executor, &package).unwrap();

        assert_eq!(fs::read_to_string(outside_target).unwrap(), "rendered\n");
    }

    #[test]
    fn test_apply_profiles_refuses_dirty_target_in_selected_code_subdirectory() {
        let temp = TempDir::new().unwrap();
        let data = crate::test_harness::git::GitRepository::init(temp.path().join("data"));
        let code = crate::test_harness::git::GitRepository::init(temp.path().join("code"));
        let sub = code.root().join("sub");
        fs::create_dir(&sub).unwrap();
        let storage = JsonFileStorage::new(data.root().join("state"));
        let paths =
            crate::storage::WorktreePaths::detect_for_data_root(storage.root(), data.root())
                .unwrap()
                .with_worktree_root(&sub)
                .unwrap();
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(&sub, storage.root()).unwrap())
            .unwrap()
            .with_worktree_paths(paths);
        executor
            .profiles()
            .initialize_fresh_repository(&sub, None)
            .unwrap();
        data.commit("track data");
        fs::create_dir(sub.join("docs")).unwrap();
        fs::write(
            sub.join("docs/first.txt"),
            "tracked
",
        )
        .unwrap();
        code.commit("track target");
        fs::write(
            sub.join("docs/first.txt"),
            "local edit
",
        )
        .unwrap();
        let package = authored_package(
            &temp,
            "code/sub/packages/first",
            "first",
            "docs/first.txt",
            "rendered
",
            "",
        );
        let error = apply_package(&executor, &package).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::UncommittedTargets { .. })
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_apply_profiles_reports_the_worktree_location_it_read_the_package_from() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, "vendor/profiles/planner");

        let applied = apply_package(&executor, &package).unwrap();

        // The reported location resolves, from the worktree root alone, back to
        // the directory whose bytes were applied.
        let location = &applied.requested().unwrap().origin;
        assert_eq!(
            ProfilePackage::from_directory(&temp.path().join(location.as_path()))
                .expect("the reported location names a readable package")
                .package_hash(),
            package.package_hash()
        );
    }

    #[test]
    fn test_apply_profiles_reports_the_directory_read_rather_than_a_peer_copy() {
        // Two directories inside the worktree hold byte-identical packages.
        // Nothing but the directory a package was read through distinguishes
        // them, so the reported location must follow that and only that.
        let (first_temp, _) = setup_test_repo().unwrap();
        let first_executor = file_executor(first_temp.path()).unwrap();
        let (second_temp, _) = setup_test_repo().unwrap();
        let second_executor = file_executor(second_temp.path()).unwrap();
        let first = package_read_from(&first_temp, "vendor/first");
        let second = package_read_from(&second_temp, "packages/second/tree");
        assert_eq!(first.package_hash(), second.package_hash());

        let first_applied = apply_package(&first_executor, &first).unwrap();
        let second_applied = apply_package(&second_executor, &second).unwrap();

        assert_eq!(
            first_applied.requested().unwrap().origin,
            RootRelativePath::parse("vendor/first").unwrap()
        );
        assert_eq!(
            second_applied.requested().unwrap().origin,
            RootRelativePath::parse("packages/second/tree").unwrap()
        );
    }

    #[test]
    fn test_apply_profiles_refuses_a_package_read_from_outside_the_worktree() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let elsewhere = TempDir::new().unwrap();
        let root = crate::test_utils::copy_package_tree(
            &fixture_package_tree(),
            &elsewhere.path().join("pkg"),
        );
        let package = ProfilePackage::from_directory(&root).expect("a valid package tree");

        let error = apply_package(&executor, &package).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::PackageOutsideWorktree { path })
                    if Path::new(path) == fs::canonicalize(&root).unwrap()
            ),
            "the refusal must name the package directory: {error:#}"
        );
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profiles_refuses_a_package_read_from_the_data_root() {
        // The selected data root is not a worktree location, so a package
        // placed under it has no worktree-relative location to report.
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_read_from(&temp, ".jit/vendored");

        let error = apply_package(&executor, &package).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileApplyError>(),
                Some(ProfileApplyError::PackageOutsideWorktree { path })
                    if Path::new(path) == temp.path().join(".jit/vendored")
            ),
            "the refusal must name the package directory: {error:#}"
        );
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    /// This repository's `id` package, assembled from its checkout into
    /// `worktree`, which is where a package has to sit to be applied to the
    /// repository rooted there.
    fn shipped_package_in(worktree: &Path, id: &str) -> ProfilePackage {
        crate::test_utils::capture_repository_package(id, &worktree.join("profiles").join(id))
            .expect("this repository's package captures")
    }

    /// A configuration declaring only the project, whose name both routes of a
    /// composition share, so what an application writes afterwards is what the
    /// applied packages declare and the repositories differ only in that.
    const COMPOSITION_CONFIG: &str = "[project]\nname = \"composition\"\n";

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
        let (_workspace, package) = crate::test_utils::temporary_repository_package("jit-default");
        let (_repeated_workspace, repeated) =
            crate::test_utils::temporary_repository_package("jit-default");
        let (_workflow_workspace, workflow) =
            crate::test_utils::temporary_repository_package("jit-dogfood");

        // The address is over content: re-reading the same declarations
        // reproduces it, and the workflow package next to it does not share it.
        assert_eq!(package.package_hash(), repeated.package_hash());
        assert_ne!(package.package_hash(), workflow.package_hash());
        assert!(!package.package_hash().is_empty());
        // One repository target, because every declaration writes the
        // configuration registry and the package publishes no file.
        assert_eq!(
            package.model().targets().into_iter().collect::<Vec<_>>(),
            vec![".jit/config.toml".to_string()]
        );
    }

    #[test]
    fn test_jit_default_carries_no_asset_and_no_workflow_registry_content() {
        let (_workspace, package) = crate::test_utils::temporary_repository_package("jit-default");
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
            .all(
                |contribution| contribution.registry().path().repository_relative()
                    == ".jit/config.toml"
            ));
    }

    #[test]
    fn test_jit_default_declares_every_kind_an_initialized_repository_can_carry() {
        let (temp, _) = setup_test_repo().unwrap();
        let (_workspace, package) = crate::test_utils::temporary_repository_package("jit-default");
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
    fn test_apply_profiles_repairs_a_generated_schema_outside_the_package_targets() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = shipped_package_in(temp.path(), "jit-default");
        // The package writes the configuration, so applying it re-derives the
        // default rules and the schemas they reference — targets it names none
        // of, and whose drift it therefore has to reach without being told.
        assert!(package
            .model()
            .targets()
            .iter()
            .all(|target| !target.starts_with(".jit/schemas/")));
        let schemas = temp.path().join(".jit/schemas");
        let grammar = schemas.join("default-label-format.json");
        let generated = fs::read(&grammar).unwrap();
        fs::remove_file(&grammar).unwrap();

        let applied = apply_package(&executor, &package).unwrap();

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
    fn test_apply_profiles_jit_default_writes_every_declaration_it_contributes() {
        // An initialization writes the structural minimum, so what the applied
        // package declares is the whole of the repository's vocabulary. Each
        // assertion below reads the manifest rather than a copy of its values,
        // so the repository is compared against the package's own declaration.
        let (temp, storage) = setup_test_repo().unwrap();
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
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap())
            .unwrap();
        let package = shipped_package_in(temp.path(), "jit-default");
        let applied = apply_package(&executor, &package).unwrap();
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
        let (_default_workspace, default) =
            crate::test_utils::temporary_repository_package("jit-default");
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
    fn test_apply_profiles_jit_dogfood_composes_what_applying_both_packages_composes() {
        // Two bare repositories, one per route: the explicit one has both
        // packages applied to it in turn, and the resolved one names only the
        // workflow package and receives the default as its declared
        // dependency. They start identical, so the configurations they end with
        // may differ only by the route that reached them. The routes share
        // nothing, so they run concurrently, and publish unsynced: the property
        // is the composition, not crash durability.
        let (explicit, resolved) = std::thread::scope(|scope| {
            let explicit = scope.spawn(|| {
                let (temp, executor) =
                    setup_unsynced_test_repo_with_config(COMPOSITION_CONFIG).unwrap();
                let default = shipped_package_in(temp.path(), "jit-default");
                let dogfood = shipped_package_in(temp.path(), "jit-dogfood");
                let default_result = apply_package(&executor, &default);
                let dogfood_result = apply_package(&executor, &dogfood);
                (temp, default, dogfood, default_result, dogfood_result)
            });
            let resolved = scope.spawn(|| {
                let (temp, executor) =
                    setup_unsynced_test_repo_with_config(COMPOSITION_CONFIG).unwrap();
                shipped_package_in(temp.path(), "jit-default");
                let dogfood = shipped_package_in(temp.path(), "jit-dogfood");
                let result = apply_package(&executor, &dogfood);
                (temp, dogfood, result)
            });
            (
                crate::test_utils::join_reraising(explicit),
                crate::test_utils::join_reraising(resolved),
            )
        });
        let (
            explicit_temp,
            explicit_default,
            explicit_dogfood,
            explicit_default_result,
            explicit_dogfood_result,
        ) = explicit;
        let (resolved_temp, resolved_dogfood, resolved_result) = resolved;

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
                .items
                .iter()
                .map(|profile| (profile.id.clone(), profile.status))
                .collect::<Vec<_>>()
        };
        let default_id = explicit_default.model().id.clone();
        let dogfood_id = explicit_dogfood.model().id.clone();
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
        let (_workspace, package) = crate::test_utils::temporary_repository_package("jit-default");
        let manifest = std::str::from_utf8(
            package
                .source_bytes(crate::profile::MANIFEST_FILE_NAME)
                .expect("the package carries its manifest"),
        )
        .expect("the packaged manifest is UTF-8");

        // The claim timing defaults are engine constants. A package is bytes
        // and projects nothing, so the packaged guidance points at the
        // reference that projects them and offers no configuration key for them.
        assert!(manifest.contains(crate::runtime_defaults::test_support::REFERENCE_PATH));
        for key in [
            "default_ttl_secs",
            "lease_renewal_threshold_pct",
            "stale_threshold_secs",
        ] {
            assert!(
                !manifest.contains(key),
                "packaged guidance offers the removed key '{key}'"
            );
        }
    }

    #[test]
    fn test_profile_application_commits_targets_and_event_and_reapplies_as_exact_no_op() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_read_from(&temp, FIXTURE_LOCATION);

        let applied = apply_package(&executor, &package).unwrap();
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            package.source_bytes("assets/profile.txt").unwrap()
        );
        assert_eq!(storage.read_events().unwrap().len(), 1);

        let before = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let unchanged = apply_package(&executor, &package).unwrap();
        assert_eq!(
            unchanged.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            before
        );
    }

    #[test]
    fn test_apply_profiles_rejects_each_reserved_application_target() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();

        for (index, target) in [".jit/events.jsonl", ".git/config"].into_iter().enumerate() {
            let package = package_publishing(
                &temp,
                &format!("packages/reserved-{index}"),
                &format!("reserved-{index}"),
                target,
                "reserved target content\n",
            );
            let error = apply_package(&executor, &package).unwrap_err();

            assert!(
                matches!(
                    error.downcast_ref::<ProfileApplyError>(),
                    Some(ProfileApplyError::ReservedApplicationTarget { path }) if path == target
                ),
                "target '{target}' should be refused as application-owned state: {error:#}"
            );
        }

        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_application_audits_the_values_it_substituted() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_read_from(&temp, FIXTURE_LOCATION);
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
        let options = ProfileVariableOptions {
            values_file: Some(values_file),
            assignments: vec![
                ProfileVariableAssignment::new("NAME".try_into().unwrap(), resolved_value),
                ProfileVariableAssignment::new(
                    "NAME".try_into().unwrap(),
                    format!("{resolved_value}-last"),
                ),
            ],
        };
        let inputs =
            load_profile_variable_inputs(std::slice::from_ref(&package), &options).unwrap();
        assert_eq!(
            inputs.values_file[&"NAME".try_into().unwrap()],
            "values-file-value"
        );
        assert_eq!(
            inputs.command_line.last().unwrap().value,
            "resolved-value-last"
        );

        apply_selected(&executor, &[package_selector(&package)], &options).unwrap();

        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            b"profile=resolved-value-last\n"
        );
        let events = storage.read_events().unwrap();
        let name: crate::profile::ProfileVariableName = "NAME".try_into().unwrap();
        assert!(matches!(
            events.iter().map(Event::body).collect::<Vec<_>>().as_slice(),
            [EventBody::ProfileLifecycle { profiles, .. }]
                if profiles.len() == 1
                    && profiles[0].variables.get(&name).map(String::as_str)
                        == Some("resolved-value-last")
        ));
    }

    #[test]
    fn test_profile_preparation_returns_one_complete_canonical_plan() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let context = MutationContext::deterministic(
            [7; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-22T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let (plan, changes) = prepared_plan(&executor, session.as_mut(), &package, &context);
        let paths = plan
            .delta()
            .actions()
            .iter()
            .map(|action| action.path().repository_relative())
            .collect::<Vec<_>>();

        assert!(paths.iter().any(|path| path == "docs/profile.txt"));
        assert!(paths.iter().any(|path| path == ".jit/events.jsonl"));
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_profile_dry_run_plan_is_stable_across_repeated_plans() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let context = MutationContext::preview();

        let first = prepared_plan(&executor, session.as_mut(), &package, &context);
        let second = prepared_plan(&executor, session.as_mut(), &package, &context);

        assert_eq!(first.0.delta(), second.0.delta());
        assert_eq!(first.1, second.1);
    }

    #[test]
    fn test_plan_profiles_completes_beside_a_held_mutation_session() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        package_read_from(&temp, FIXTURE_LOCATION);
        let selectors = [PathBuf::from(FIXTURE_LOCATION)];
        let options = ProfileVariableOptions::default();
        let alone = plan_selected(&executor, &selectors, &options).unwrap();

        let beside = crate::commands::test_helpers::read_beside_held_mutation_session(
            executor.storage(),
            &executor.require_layout().unwrap(),
            || plan_selected(&executor, &selectors, &options).unwrap(),
        );

        assert!(!beside.items.is_empty());
        assert_eq!(beside, alone);
    }

    #[test]
    fn test_plan_profile_add_completes_beside_a_held_mutation_session() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        let archive = temp.path().join("planner.jitpkg");
        fs::write(&archive, pack_package_archive(&package).unwrap()).unwrap();
        let destination = Path::new("packages/added");

        let planned = crate::commands::test_helpers::read_beside_held_mutation_session(
            executor.storage(),
            &executor.require_layout().unwrap(),
            || {
                executor
                    .profiles()
                    .plan_profile_add(temp.path(), &archive, destination)
                    .unwrap()
            },
        );

        assert_eq!(planned.items[0].targets.len(), package.file_count());
        assert!(!temp.path().join(destination).exists());
    }

    #[test]
    fn test_profile_plan_collection_preserves_selector_occurrences_without_events() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_read_from(&temp, FIXTURE_LOCATION);
        let before = storage.read_events().unwrap();
        let selector = PathBuf::from(FIXTURE_LOCATION);

        let result = plan_selected(
            &executor,
            &[selector.clone(), selector],
            &ProfileVariableOptions::default(),
        )
        .unwrap();

        assert_eq!(result.items.len(), 2);
        assert_eq!(
            result
                .items
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["planner-asset-only", "planner-asset-only"]
        );
        assert_eq!(storage.read_events().unwrap(), before);
        assert!(!temp.path().join("docs/profile.txt").exists());
    }

    #[test]
    fn test_profile_application_revalidates_locked_snapshot_before_any_write() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        fs::write(temp.path().join(".jit/config.toml"), b"not = [valid").unwrap();

        assert!(apply_package(&executor, &package).is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_application_overwrites_an_unterminated_event_tail_end_to_end() {
        let complete_record = serde_json::to_vec(&crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("prior".parse().unwrap()),
            String::new(),
            chrono::DateTime::UNIX_EPOCH,
        ))
        .unwrap();
        for tail in [b"{\"partial\":".as_slice(), &complete_record] {
            let (temp, _) = setup_test_repo().unwrap();
            let executor = file_executor(temp.path()).unwrap();
            let package = package_read_from(&temp, FIXTURE_LOCATION);
            let log = temp.path().join(".jit/events.jsonl");
            fs::write(&log, tail).unwrap();

            apply_package(&executor, &package).unwrap();

            assert!(fs::read(&log).unwrap().ends_with(b"\n"));
            let events = executor.storage().read_events().unwrap();
            assert!(
                matches!(
                    events
                        .iter()
                        .map(Event::body)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    [EventBody::ProfileLifecycle { .. }]
                ),
                "{events:?}"
            );
        }
    }

    #[test]
    fn test_apply_profiles_applies_a_declared_dependency_before_the_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = apply_package(&executor, &dependant).unwrap();

        // The set is the answer, in the order it was applied: the dependency,
        // then the package that declared it.
        assert_eq!(
            applied
                .items
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "workflow"]
        );
        assert_eq!(applied.requested().unwrap().id.as_str(), "workflow");
        assert!(applied
            .items
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Applied));
        // What each package publishes is in the repository, and the audit log
        // states the same order.
        assert!(temp.path().join("docs/base.txt").is_file());
        assert!(temp.path().join("docs/workflow.txt").is_file());
        assert_eq!(applied_event_ids(&storage), vec!["base", "workflow"]);
        assert!(matches!(
            storage.read_events().unwrap().iter().map(Event::body).collect::<Vec<_>>().as_slice(),
            [EventBody::ProfileLifecycle {
                operation: ProfileLifecycleOperation::Apply,
                profiles,
                ..
            }]
                if profiles.len() == 2
                    && profiles.iter().all(|profile| profile.status
                        == ProfileLifecycleStatus::Installed)
        ));
    }

    #[test]
    fn test_apply_profile_selection_publishes_multiple_packages_through_one_publication() {
        let temp = TempDir::new().unwrap();
        let bootstrap = JsonFileStorage::new(temp.path().join(".jit"));
        CommandExecutor::new(bootstrap.clone())
            .with_layout(discover_repository_layout(temp.path(), bootstrap.root()).unwrap())
            .unwrap()
            .profiles()
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        let publications = crate::commands::test_helpers::PublicationCounter::new();
        let storage = JsonFileStorage::with_mutation_interruptions(
            temp.path().join(".jit"),
            publications.clone(),
        );
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap())
            .unwrap();

        let applied = apply_selected_packages(
            &executor,
            &[package_selector(&left), package_selector(&right)],
        )
        .unwrap();

        assert_eq!(
            publications.count(),
            1,
            "one selection must reach the recoverable publication boundary once"
        );
        assert_eq!(
            applied
                .items
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("left", ProfileApplicationStatus::Applied),
                ("right", ProfileApplicationStatus::Applied),
            ]
        );
        for id in ["left", "right"] {
            assert!(temp.path().join(format!("docs/{id}.txt")).is_file());
        }
        assert_eq!(applied_event_ids(&storage), vec!["left", "right"]);
    }

    #[test]
    fn test_apply_profile_selection_is_a_complete_noop_when_every_member_is_present() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        apply_selected_packages(
            &executor,
            &[package_selector(&left), package_selector(&right)],
        )
        .unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let publications = crate::commands::test_helpers::PublicationCounter::new();
        let storage = JsonFileStorage::with_mutation_interruptions(
            temp.path().join(".jit"),
            publications.clone(),
        );
        let executor = CommandExecutor::new(storage)
            .with_layout(discover_repository_layout(temp.path(), temp.path().join(".jit")).unwrap())
            .unwrap();

        let reapplied = apply_selected_packages(
            &executor,
            &[package_selector(&left), package_selector(&right)],
        )
        .unwrap();

        assert_eq!(publications.count(), 0);
        assert!(reapplied
            .items
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Unchanged));
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events
        );
    }

    #[test]
    fn test_apply_profile_selection_recovers_targets_and_audit_together_after_failure() {
        let temp = TempDir::new().unwrap();
        let bootstrap = JsonFileStorage::new(temp.path().join(".jit"));
        CommandExecutor::new(bootstrap.clone())
            .with_layout(discover_repository_layout(temp.path(), bootstrap.root()).unwrap())
            .unwrap()
            .profiles()
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        let original_events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let storage = JsonFileStorage::with_mutation_interruptions(
            temp.path().join(".jit"),
            FailOnce::at(MutationPoint::BeforeCommitCheck),
        );
        let selectors = [package_selector(&left), package_selector(&right)];
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap())
            .unwrap();

        assert!(apply_selected_packages(&executor, &selectors).is_err());

        let recovered = JsonFileStorage::new(temp.path().join(".jit"));
        let layout = discover_repository_layout(temp.path(), recovered.root()).unwrap();
        drop(recovered.open_mutation_session(layout).unwrap());
        for id in ["left", "right"] {
            assert!(!temp.path().join(format!("docs/{id}.txt")).exists());
        }
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            original_events
        );

        let retried = apply_selected_packages(
            &CommandExecutor::new(recovered.clone())
                .with_layout(discover_repository_layout(temp.path(), recovered.root()).unwrap())
                .unwrap(),
            &selectors,
        )
        .unwrap();
        assert!(retried
            .items
            .iter()
            .all(|profile| profile.status == ProfileApplicationStatus::Applied));
        assert_eq!(applied_event_ids(&recovered), vec!["left", "right"]);
    }

    #[test]
    fn test_apply_profile_selection_keeps_duplicate_root_observation_without_duplicate_publication()
    {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_declaring(&temp, "vendor/duplicate", "duplicate", &[]);

        let applied = apply_selected_packages(
            &executor,
            &[package_selector(&package), package_selector(&package)],
        )
        .unwrap();

        assert_eq!(
            applied
                .items
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
    fn test_apply_profiles_applies_a_transitive_dependency_chain_deepest_first() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/vocabulary", "vocabulary", &[]);
        package_declaring(&temp, "vendor/base", "base", &["vocabulary"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = apply_package(&executor, &dependant).unwrap();

        // `workflow` never names `vocabulary`; reaching it is what "transitively"
        // means, and it is applied before the package that reaches it.
        assert_eq!(
            applied
                .items
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["vocabulary", "base", "workflow"]
        );
        assert_eq!(
            applied_event_ids(&storage),
            vec!["vocabulary", "base", "workflow"]
        );
    }

    #[test]
    fn test_apply_profiles_applies_a_shared_dependency_once() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/shared", "shared", &[]);
        package_declaring(&temp, "vendor/left", "left", &["shared"]);
        package_declaring(&temp, "vendor/right", "right", &["shared"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["left", "right"]);

        let applied = apply_package(&executor, &dependant).unwrap();

        let order: Vec<&str> = applied
            .items
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
    fn test_effective_configuration_loads_an_authored_edit_of_a_published_registry() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let namespace = "registry-derived";
        let package = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "Published definition.",
        );
        apply_package(&executor, &package).unwrap();

        let config_path = temp.path().join(".jit/config.toml");
        let registry = fs::read_to_string(&config_path)
            .unwrap()
            .replace("Published definition.", "Current registry definition.");
        fs::write(config_path, registry).unwrap();
        let reloaded = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap())
            .unwrap();

        assert_eq!(
            reloaded
                .captured_declarations()
                .unwrap()
                .config()
                .namespaces
                .as_ref()
                .unwrap()[namespace]
                .description,
            "Current registry definition."
        );
    }

    #[test]
    fn test_apply_profiles_publishes_the_dependents_value_of_a_shared_declaration() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let namespace = "closure-override";
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

        apply_package(&executor, &workflow).unwrap();

        let config = fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap();
        assert!(config.contains("Workflow definition."));
        assert!(!config.contains("Base definition."));
    }

    /// A managed region reaches publication through the same override a
    /// declaration does.
    #[test]
    fn test_apply_profiles_publishes_the_dependents_managed_region() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let manifest = |id: &str, dependency: &str| {
            format!(
                "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n{dependency}\n[[region]]\nsource = \"regions/guide.md\"\ntarget = \"docs/guide.md\"\nregion-id = \"guidance\"\nplacement = \"append\"\n"
            )
        };
        authored_manifest_package(
            &temp,
            "vendor/base",
            &manifest("base", ""),
            &[("regions/guide.md", "Base guidance.\n")],
        );
        let workflow = authored_manifest_package(
            &temp,
            "vendor/workflow",
            &manifest(
                "workflow",
                "\n[[dependency]]\nid = \"base\"\nversion = \"*\"\n",
            ),
            &[("regions/guide.md", "Workflow guidance.\n")],
        );

        apply_package(&executor, &workflow).unwrap();

        let guide = fs::read_to_string(temp.path().join("docs/guide.md")).unwrap();
        assert!(guide.contains("Workflow guidance."));
        assert!(!guide.contains("Base guidance."));
    }

    #[test]
    fn test_apply_profiles_names_both_packages_of_a_conflicting_asset_target() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let target = "docs/shared.txt";
        let base = package_publishing(&temp, "vendor/base", "base", target, "first\n");
        let workflow = package_publishing(&temp, "vendor/workflow", "workflow", target, "second\n");

        let error = apply_selected(
            &executor,
            &[package_selector(&base), package_selector(&workflow)],
            &supplied_values(&[]),
        )
        .unwrap_err();

        let refused = refused_targets(&error);
        assert_eq!(
            refused.len(),
            2,
            "each declaring package decides the target"
        );
        for entry in refused {
            assert_eq!(refused_path(entry), target);
            assert_eq!(
                entry.conflict,
                ProfileTargetConflict::Contested {
                    owners: vec![
                        ProfileId::try_from("base").unwrap(),
                        ProfileId::try_from("workflow").unwrap(),
                    ]
                }
            );
        }
        assert!(
            !temp.path().join(target).exists(),
            "a conflict applies nothing"
        );
    }

    #[test]
    fn test_apply_profiles_rejects_a_dependency_cycle_before_applying_anything() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/base", "base", &["workflow"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let error = apply_package(&executor, &dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileGraphError>(),
                Some(ProfileGraphError::DependencyCycle { cycle })
                    if cycle.first() == cycle.last()
                        && cycle.iter().any(|id| id == "base")
                        && cycle.iter().any(|id| id == "workflow")
            ),
            "the refusal must name the cycle: {error:#}"
        );
        // Neither package of the cycle reached the repository.
        assert!(!temp.path().join("docs/base.txt").exists());
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profiles_reports_a_dependency_that_cannot_be_resolved() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        // Nothing sits beside the package under that name, so no route
        // resolves the dependency.
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["absent-base"]);

        let error = apply_package(&executor, &dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableDependency { package, dependency, .. })
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
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profiles_rejects_a_dependency_version_mismatch_before_publication() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
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

        let error = apply_selected_packages(&executor, &[package_selector(&workflow)]).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileGraphError>(),
                Some(ProfileGraphError::DependencyVersionMismatch {
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
    fn test_apply_profiles_rejects_a_package_outside_the_running_engine_range() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_v2(
            &temp,
            "vendor/future",
            "future",
            "1.0.0",
            ">=2.0.0",
            &[],
            &[],
        );

        let error = apply_selected_packages(&executor, &[package_selector(&package)]).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileGraphError>(),
                Some(ProfileGraphError::IncompatibleEngine { package, .. })
                    if package == "future"
            ),
            "the incompatible package must be named: {error:#}"
        );
        assert!(!temp.path().join("docs/future.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profiles_rejects_incompatibilities_symmetrically_and_independently_of_selector_order(
    ) {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
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

        let first =
            apply_selected_packages(&executor, &[left_selector.clone(), right_selector.clone()])
                .unwrap_err();
        let second =
            apply_selected_packages(&executor, &[right_selector, left_selector]).unwrap_err();

        assert_eq!(format!("{first:#}"), format!("{second:#}"));
        let message = format!("{first:#}");
        assert!(message.contains("left"), "{message}");
        assert!(message.contains("right"), "{message}");
        assert!(!temp.path().join("docs/left.txt").exists());
        assert!(!temp.path().join("docs/right.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profiles_excludes_a_previously_applied_package_from_incompatibility_checks() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let base = package_v2(
            &temp,
            "vendor/base",
            "base",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[],
        );
        apply_selected_packages(&executor, &[package_selector(&base)]).unwrap();
        let candidate = package_v2(
            &temp,
            "vendor/candidate",
            "candidate",
            "1.0.0",
            ">=0.2.0, <2.0.0",
            &[],
            &[("base", "*")],
        );

        // `base` is in the repository but not in the selection, so the
        // incompatibility it would answer has no party to hold against.
        apply_selected_packages(&executor, &[package_selector(&candidate)]).unwrap();

        assert!(temp.path().join("docs/candidate.txt").is_file());
        assert_eq!(applied_event_ids(&storage), vec!["base", "candidate"]);
    }

    #[test]
    fn test_apply_profiles_reports_no_work_for_an_applied_unchanged_closure() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);
        apply_package(&executor, &dependant).unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();

        let reapplied = apply_package(&executor, &dependant).unwrap();

        // No work anywhere in the set: the dependency is unchanged too, not
        // only the package that was named.
        assert!(
            reapplied
                .items
                .iter()
                .all(|profile| profile.status == ProfileApplicationStatus::Unchanged),
            "{:?}",
            reapplied.items
        );
        assert_eq!(
            fs::read(temp.path().join(".jit/events.jsonl")).unwrap(),
            events
        );
        assert_eq!(applied_event_ids(&storage), vec!["base", "workflow"]);
    }

    #[test]
    fn test_apply_profiles_refuses_a_dependency_only_a_previous_application_supplied() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let dependency = package_declaring(&temp, "vendor/base", "base", &[]);
        apply_package(&executor, &dependency).unwrap();
        // A dependant elsewhere in the worktree, with no `base` beside it. The
        // earlier application of `base` is not a candidate for the selection.
        let dependant = package_declaring(&temp, "elsewhere/workflow", "workflow", &["base"]);
        assert!(!temp.path().join("elsewhere/base").exists());

        let error = apply_package(&executor, &dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableDependency { package, dependency, .. })
                    if package == "workflow" && dependency == "base"
            ),
            "the refusal must name both packages: {error:#}"
        );
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert_eq!(applied_event_ids(&storage), vec!["base"]);
    }

    #[test]
    fn test_select_profiles_closure_reads_a_dependency_from_beside_the_declaring_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        // The package beside the declaring package is the one its declaration
        // means.
        let beside = package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let closure = selected_closure(&executor, &dependant).unwrap();

        assert_eq!(
            closure
                .iter()
                .map(|package| package.package_hash())
                .collect::<Vec<_>>(),
            vec![beside.package_hash(), dependant.package_hash()]
        );
    }

    #[test]
    fn test_select_profiles_closure_reports_an_unreadable_package_beside_the_declaring_one() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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

        let error = selected_closure(&executor, &dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableDependency { package, dependency, .. })
                    if package == "workflow" && dependency == "base"
            ),
            "{error:#}"
        );
        assert!(format!("{error:#}").contains("vendor/base"), "{error:#}");
    }

    #[test]
    fn test_select_profiles_closure_passes_over_a_neighbour_declaring_another_profile() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        // The directory beside the declarant is named for the dependency but
        // holds a different package, so it is not that dependency and nothing
        // else answers either.
        package_declaring(&temp, "vendor/base", "impostor", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let error = selected_closure(&executor, &dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileResolutionError>(),
                Some(ProfileResolutionError::UnresolvableDependency { dependency, .. })
                    if dependency == "base"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_select_profiles_closure_of_a_package_declaring_nothing_is_that_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);

        let closure = selected_closure(&executor, &package).unwrap();

        assert_eq!(
            closure
                .iter()
                .map(|package| package.package_hash())
                .collect::<Vec<_>>(),
            vec![package.package_hash()]
        );
    }

    /// Assert a rehearsal's per-target decisions against what the real run did.
    ///
    /// A rehearsal is only worth running if its decisions are the ones the
    /// publication makes, so every target it would create or update must have
    /// changed and every target it called unchanged must not have.
    fn assert_planned_targets_match_published(
        before: &BTreeMap<PathBuf, Vec<u8>>,
        after: &BTreeMap<PathBuf, Vec<u8>>,
        planned: &ProfileComposedApplyResult,
    ) {
        assert!(
            planned
                .items
                .iter()
                .flat_map(|plan| &plan.targets)
                .any(|target| target.action != ProfileTargetAction::Unchanged),
            "a rehearsal that decides nothing proves nothing about the run it precedes"
        );
        for target in planned.items.iter().flat_map(|plan| &plan.targets) {
            let path = PathBuf::from(&target.path);
            match target.action {
                ProfileTargetAction::Unchanged => assert_eq!(
                    before.get(&path),
                    after.get(&path),
                    "the rehearsal called {} unchanged",
                    target.path
                ),
                ProfileTargetAction::Create
                | ProfileTargetAction::Update
                | ProfileTargetAction::Remove => assert_ne!(
                    before.get(&path),
                    after.get(&path),
                    "the rehearsal said it would publish {}",
                    target.path
                ),
                ProfileTargetAction::Conflict => panic!(
                    "a rehearsal refuses an unpublishable decision rather than reporting {}",
                    target.path
                ),
            }
        }
    }

    // =======================================================================
    // jit:96268a98 — `jit profile diff` states what a selection would change
    // before anything is published.
    // =======================================================================

    /// A package publishing one file per named target, authored so a case can
    /// state exactly how many targets a selection decides.
    fn package_publishing_each(
        temp: &TempDir,
        relative: &str,
        id: &str,
        targets: &[(&str, &str)],
    ) -> ProfilePackage {
        let manifest = targets
            .iter()
            .enumerate()
            .map(|(index, (target, _))| {
                format!("\n[[asset]]\nsource = \"assets/{index}.txt\"\ntarget = \"{target}\"\n")
            })
            .collect::<String>();
        let sources = targets
            .iter()
            .enumerate()
            .map(|(index, (_, content))| (format!("assets/{index}.txt"), (*content).to_string()))
            .collect::<Vec<_>>();
        authored_manifest_package(
            temp,
            relative,
            &format!(
                "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\n\
                 compatible-jit = \"*\"\n{manifest}"
            ),
            &sources
                .iter()
                .map(|(source, content)| (source.as_str(), content.as_str()))
                .collect::<Vec<_>>(),
        )
    }

    /// The one entry a report carries for `id`.
    fn reported(report: &ProfileComposedApplyResult, id: &str) -> ProfileApplyResult {
        report
            .items
            .iter()
            .find(|entry| entry.id.as_str() == id)
            .unwrap_or_else(|| panic!("the report names profile {id}: {report:?}"))
            .clone()
    }

    /// The targets `entry` decided `action` about, by repository-relative name.
    fn decided_targets(
        entry: &ProfileApplyResult,
        action: ProfileTargetAction,
    ) -> BTreeSet<String> {
        entry
            .decided(action)
            .map(|target| target.path.clone())
            .collect()
    }

    /// The report states every target the selection cannot publish, not the
    /// first one it reached, and the publication it precedes refuses exactly
    /// that set — while the report itself writes nothing (REQ-01, REQ-03).
    #[test]
    fn test_diff_profiles_states_every_unpublishable_target_the_apply_refuses() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let targets = ["docs/first.txt", "docs/second.txt", "docs/third.txt"];
        let package = package_publishing_each(
            &temp,
            "packages/collider",
            "collider",
            &targets.map(|target| (target, "the package's own bytes\n")),
        );
        let other = package_publishing_each(
            &temp,
            "packages/other",
            "other",
            &targets.map(|target| (target, "the other package's bytes\n")),
        );
        let selectors = [package_selector(&package), package_selector(&other)];
        let before = repository_files(&temp);

        let report = diff_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("a conflicting selection is reported, not refused");

        let entry = reported(&report, "collider");
        assert_eq!(entry.status, ProfileApplicationStatus::WouldConflict);
        assert_eq!(
            decided_targets(&entry, ProfileTargetAction::Conflict),
            targets.map(str::to_string).into_iter().collect(),
            "the report states every target the selection cannot publish"
        );
        assert!(
            entry
                .decided(ProfileTargetAction::Conflict)
                .all(|target| target
                    .reason
                    .as_ref()
                    .is_some_and(|reason| reason.contains("other"))),
            "each refusal names the other package defining the target: {entry:?}"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "the report publishes nothing"
        );

        let error = apply_selected(&executor, &selectors, &supplied_values(&[]))
            .expect_err("the publication refuses what the report reported");
        assert_eq!(
            entry
                .decided(ProfileTargetAction::Conflict)
                .map(|target| (
                    target.path.clone(),
                    target
                        .reason
                        .clone()
                        .expect("a refused target says why it was refused")
                ))
                .collect::<BTreeMap<_, _>>(),
            refused_targets(&error)
                .iter()
                .map(|refused| (refused_path(refused), refused.conflict.message()))
                .collect::<BTreeMap<_, _>>(),
            "the report and the refusal describe each target in one vocabulary"
        );
    }

    /// A selection this repository has not applied is reported from the
    /// location it was selected at, so an adopter inspects a package before
    /// applying it (REQ-04).
    #[test]
    fn test_diff_profiles_reports_a_package_this_repository_has_not_applied() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_publishing_each(
            &temp,
            "packages/unapplied",
            "unapplied",
            &[("docs/unapplied.txt", "the package's own bytes\n")],
        );
        let before = repository_files(&temp);

        let report = diff_selected(
            &executor,
            &[package_selector(&package)],
            &supplied_values(&[]),
        )
        .expect("an unapplied package is reported from its location");

        let entry = reported(&report, "unapplied");
        assert_eq!(entry.status, ProfileApplicationStatus::WouldApply);
        assert!(
            decided_targets(&entry, ProfileTargetAction::Create).contains("docs/unapplied.txt"),
            "the report states the target the package would publish: {entry:?}"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "the report publishes nothing"
        );
    }

    /// The report states the decision the publication then makes, target for
    /// target, because both read one decision (REQ-03).
    #[test]
    fn test_diff_profiles_decides_the_targets_the_apply_it_precedes_publishes() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let fixture_package = package_read_from(&temp, FIXTURE_LOCATION);
        let selectors = [package_selector(&fixture_package)];

        let report = diff_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("the selection is reportable");

        let before = repository_files(&temp);
        apply_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("the reported selection applies");
        let after = repository_files(&temp);

        assert_planned_targets_match_published(&before, &after, &report);
    }

    /// The canonical identity of the label-namespace declaration named
    /// `namespace`, which is what a report names a declaration by.
    ///
    /// An identity is the registry, declaration target, and local name; the
    /// value is deliberately not part of it, so any definition of the namespace
    /// answers with the same identity.
    fn namespace_identity(namespace: &str) -> String {
        Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: namespace.to_string(),
            value: serde_json::json!({}),
        }
        .semantic_identity()
        .to_string()
    }

    /// The decision `entry` states for the declaration `identity`.
    fn declared(entry: &ProfileApplyResult, identity: &str) -> ProfileContributionChange {
        entry
            .contributions
            .iter()
            .find(|declaration| declaration.identity == identity)
            .unwrap_or_else(|| panic!("the report states the declaration {identity}: {entry:?}"))
            .clone()
    }

    /// A declaration two selected packages define differently is stated by the
    /// report as the decision it is — named by its semantic identity, beside the
    /// packages defining it — and the publication it precedes refuses that
    /// declaration for that same reason (REQ-01, REQ-02, REQ-03).
    #[test]
    fn test_diff_profiles_states_a_contested_declaration_the_apply_refuses() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let namespace = "contested-vocabulary";
        let base =
            package_contributing(&temp, "vendor/base", "base", namespace, "The base meaning.");
        let candidate = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "The workflow meaning.",
        );
        let selectors = [package_selector(&base), package_selector(&candidate)];
        let before = repository_files(&temp);

        let report = diff_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("a contested declaration is reported, not refused");

        let entry = reported(&report, "workflow");
        assert_eq!(entry.status, ProfileApplicationStatus::WouldConflict);
        let contested = declared(&entry, &namespace_identity(namespace));
        assert_eq!(contested.action, ProfileTargetAction::Conflict);
        assert!(
            contested
                .reason
                .as_ref()
                .is_some_and(|reason| reason.contains("base")),
            "the report names the other package defining the declaration: {contested:?}"
        );
        assert_eq!(
            repository_files(&temp),
            before,
            "the report publishes nothing"
        );

        let error = apply_selected(&executor, &selectors, &supplied_values(&[]))
            .expect_err("the publication refuses what the report reported");
        assert_eq!(
            entry
                .decided_contributions(ProfileTargetAction::Conflict)
                .map(|declaration| (
                    declaration.identity.clone(),
                    declaration
                        .reason
                        .clone()
                        .expect("a refused declaration says why it was refused")
                ))
                .collect::<BTreeMap<_, _>>(),
            refused_targets(&error)
                .iter()
                .filter_map(|refused| match &refused.subject {
                    ProfileTargetSubject::Contribution { identity } =>
                        Some((identity.to_string(), refused.conflict.message())),
                    ProfileTargetSubject::File { .. } => None,
                })
                .collect::<BTreeMap<_, _>>(),
            "the report and the refusal describe each declaration in one vocabulary"
        );
    }

    /// The report states what the selection would do to each declaration it
    /// carries: a declaration the registry lacks is created, and after the
    /// publication the same declaration is unchanged (REQ-01, REQ-03).
    #[test]
    fn test_diff_profiles_decides_the_declarations_the_apply_it_precedes_publishes() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let namespace = "published-vocabulary";
        let description = "The workflow meaning.";
        let package =
            package_contributing(&temp, "vendor/workflow", "workflow", namespace, description);
        let selectors = [package_selector(&package)];
        let identity = namespace_identity(namespace);

        let planned = diff_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("an unapplied declaration is reportable");

        let created = declared(&reported(&planned, "workflow"), &identity);
        assert_eq!(created.action, ProfileTargetAction::Create);

        apply_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("the reported selection applies");

        assert!(
            fs::read_to_string(temp.path().join(".jit/config.toml"))
                .unwrap()
                .contains(description),
            "the publication created the declaration the report stated"
        );
        let republished = diff_selected(&executor, &selectors, &supplied_values(&[]))
            .expect("an applied declaration is reportable");
        let published = declared(&reported(&republished, "workflow"), &identity);
        assert_eq!(published.action, ProfileTargetAction::Unchanged);
    }

    /// A selection final validation refuses, as it does one leaving a configured
    /// projection stale, is refused by the rehearsal and stated by the report in
    /// the words the application refuses in.
    #[test]
    fn test_profile_previews_report_the_final_validation_refusal_apply_raises() {
        let (temp, _) = setup_test_repo().unwrap();
        let config = temp.path().join(".jit/config.toml");
        let authored = fs::read_to_string(&config).unwrap();
        fs::write(
            &config,
            format!("{authored}{}", crate::test_utils::GATE_PROJECTION_CONFIG),
        )
        .unwrap();
        let executor = file_executor(temp.path()).unwrap();
        executor.items().project_render(None).unwrap();
        let package = authored_package(
            &temp,
            "vendor/workflow",
            "workflow",
            "docs/workflow.txt",
            "workflow",
            crate::test_utils::NEW_GATE_CONTRIBUTION,
        );
        let selectors = [package_selector(&package)];
        let options = supplied_values(&[]);
        let before = repository_files(&temp);

        let refusal = apply_selected(&executor, &selectors, &options)
            .expect_err("the package leaves the projection stale")
            .to_string();
        let rehearsal = plan_selected(&executor, &selectors, &options)
            .expect_err("a rehearsal fails where its publication would");
        let report = diff_selected(&executor, &selectors, &options)
            .expect("the report states the refusal rather than raising it");

        assert!(refusal.contains("docs/gates.md"), "{refusal}");
        assert!(matches!(
            rehearsal.downcast_ref::<ProfileApplyError>(),
            Some(ProfileApplyError::FinalValidation(_))
        ));
        assert_eq!(rehearsal.to_string(), refusal);
        let entry = reported(&report, "workflow");
        assert_eq!(entry.status, ProfileApplicationStatus::WouldConflict);
        assert_eq!(entry.refusal.as_deref(), Some(refusal.as_str()));
        assert_eq!(repository_files(&temp), before, "nothing is written");
    }

    /// Every file an application changes, the ones it derives included, appears
    /// in the report before it and the result after it.
    #[test]
    fn test_profile_reports_list_every_file_the_application_changes() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_contributing(&temp, "vendor/workflow", "workflow", "added", "Added.");
        let selectors = [package_selector(&package)];
        let options = supplied_values(&[]);
        let before = repository_files(&temp);

        let report = diff_selected(&executor, &selectors, &options).unwrap();
        let applied = apply_selected(&executor, &selectors, &options).unwrap();

        let after = repository_files(&temp);
        let changed = after
            .iter()
            .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
            .map(|(path, _)| path.to_string_lossy().into_owned())
            .filter(|path| path != ".jit/events.jsonl")
            .collect::<BTreeSet<_>>();
        assert!(
            changed.contains(".jit/schemas/default-namespace-registry.json"),
            "the application derives a schema: {changed:?}"
        );
        let reported_paths = report
            .items
            .iter()
            .flat_map(|entry| &entry.targets)
            .map(|target| target.path.clone())
            .collect::<BTreeSet<_>>();
        let result_paths = applied
            .items
            .iter()
            .flat_map(|entry| &entry.targets)
            .map(|target| target.path.clone())
            .collect::<BTreeSet<_>>();
        assert!(changed.is_subset(&reported_paths), "{reported_paths:?}");
        assert!(changed.is_subset(&result_paths), "{result_paths:?}");
    }

    /// A derived file the application deletes is reported as a removal.
    #[test]
    fn test_diff_profiles_reports_an_obsolete_default_schema_as_removed() {
        let temp = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let dogfood =
            ProfilePackage::from_directory(&temp.path().join("profiles/jit-dogfood")).unwrap();
        let schema = ".jit/schemas/default-obsolete-profile-selection.json";
        fs::write(temp.path().join(schema), "{}").unwrap();
        let rules_path = temp.path().join(".jit/rules.toml");
        let rules = fs::read_to_string(&rules_path).unwrap();
        fs::write(
            &rules_path,
            format!(
                "{rules}\n[[rules]]\nname = \"obsolete-profile-selection\"\norigin = \"default\"\n\
                 assert = {{ json-schema = \"schemas/default-obsolete-profile-selection.json\" }}\n"
            ),
        )
        .unwrap();
        let executor = file_executor(temp.path()).unwrap();

        let report = diff_selected(
            &executor,
            &[package_selector(&dogfood)],
            &supplied_values(&[]),
        )
        .unwrap();

        assert!(
            report
                .items
                .iter()
                .any(|entry| decided_targets(entry, ProfileTargetAction::Remove).contains(schema)),
            "{report:#?}"
        );
    }

    /// Every dimension of the publication budget clears what the package model
    /// can demand of it.
    ///
    /// The model bounds a package's file count and total size and says nothing
    /// about how deep or how long its paths are, so what a publication can be
    /// asked for is stated against those constants. The manifest names every
    /// file the package holds and no source twice, so the package's declared
    /// path lengths sum below its byte budget: it can need no more directories
    /// than that, and no path of it can hold more components than that.
    #[test]
    fn test_package_tree_capture_budget_clears_what_the_package_model_can_demand() {
        let destination = VirtualPath::worktree("packages/deep").unwrap();
        let budget = package_tree_capture_budget(&destination);
        let model_bytes = crate::profile::MAX_PROFILE_PACKAGE_BYTES;

        assert!(
            budget.max_listings > model_bytes,
            "a package can need a listing per directory its declared paths can name"
        );
        assert!(
            budget.max_depth >= destination.relative().depth() + model_bytes,
            "a package can declare a path with a component per byte its paths can hold"
        );
        assert!(
            budget.max_bytes >= model_bytes as u64,
            "a publication reads the whole tree the package carries"
        );
    }

    /// A package the model admits whose cost is driven by its directories rather
    /// than by its files declares a capture the publication budget accepts.
    ///
    /// This is the shape that used to be refused: a bound reading the model's
    /// file count and assuming a path depth to go with it refuses an archive
    /// `pack` wrote, leaving an operator holding a file nothing takes.
    #[test]
    fn test_package_tree_capture_spec_admits_a_file_count_of_unshared_directory_chains() {
        /// Directory levels each file sits below, sharing none with another.
        const DEPTH: usize = 64;

        let destination = VirtualPath::worktree("packages/deep").unwrap();
        let files = crate::profile::MAX_PROFILE_PACKAGE_FILES;
        let tree = PackageTreeCapture::new(
            destination.clone(),
            (0..files)
                .map(|index| {
                    Ok(CapturedTreeFile {
                        relative: RootRelativePath::parse(
                            (0..DEPTH)
                                .map(|level| format!("d{level}-{index}/"))
                                .chain(std::iter::once(format!("source-{index}")))
                                .collect::<String>(),
                        )?,
                        bytes: Vec::new(),
                        mode: FileMode::Regular,
                    })
                })
                .collect::<Result<Vec<_>, RepositoryLayoutError>>()
                .expect("every declared path is a safe relative path"),
        )
        .expect("the destination is worktree content");

        let spec = tree
            .capture_spec(package_tree_capture_budget(&destination))
            .expect("the publication budget admits a package the model admits");

        assert!(
            spec.paths().count() > files * DEPTH,
            "the package under test must declare a closure driven by its directories"
        );
    }

    /// A root package depending on one sibling, selected by its directory,
    /// with the size of the closure that selector reaches.
    fn dependent_selection(
        temp: &TempDir,
        executor: &CommandExecutor<JsonFileStorage>,
    ) -> (PathBuf, usize) {
        package_declaring(temp, "vendor/base", "base", &[]);
        let root = package_declaring(temp, "vendor/workflow", "workflow", &["base"]);
        let closure = selected_closure(executor, &root).unwrap().len();
        (package_selector(&root), closure)
    }

    #[test]
    fn test_select_profiles_validates_equivalent_directory_spellings_once() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (absolute, closure) = dependent_selection(&temp, &executor);
        let selectors = vec![
            PathBuf::from("vendor/workflow"),
            absolute,
            PathBuf::from("vendor/./workflow/"),
        ];
        #[cfg(unix)]
        let selectors = {
            std::os::unix::fs::symlink("vendor", temp.path().join("alias")).unwrap();
            let mut selectors = selectors;
            selectors.push(PathBuf::from("alias/workflow"));
            selectors
        };
        let (planned, stages) = selection_probe::observe(|| {
            plan_selected(&executor, &selectors, &ProfileVariableOptions::default())
        });
        assert_eq!(planned.unwrap().count(), closure + selectors.len() - 1);
        assert_eq!(stages, selection_probe::once_per_invocation(closure));
    }

    #[test]
    fn test_select_profiles_rejects_parent_traversal_before_model_validation() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        dependent_selection(&temp, &executor);

        for spelling in ["vendor/../vendor/workflow", "vendor/workflow/../workflow"] {
            for location in [PathBuf::from(spelling), temp.path().join(spelling)] {
                let (result, stages) = selection_probe::observe(|| {
                    executor
                        .profiles()
                        .select_profiles(&[location], &ProfileVariableOptions::default())
                });

                let error = result.err().expect("parent traversal must remain rejected");
                assert!(matches!(
                    error.downcast_ref::<ProfileApplyError>(),
                    Some(ProfileApplyError::PackageOutsideWorktree { .. })
                ));
                assert!(!stages
                    .contains_key(&selection_probe::SelectionStage::ModelReferenceValidation));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_select_profiles_refuses_a_symlink_alias_of_a_cached_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (selector, _) = dependent_selection(&temp, &executor);
        std::os::unix::fs::symlink("workflow", temp.path().join("vendor/alias")).unwrap();

        let result = executor.profiles().select_profiles(
            &[selector, PathBuf::from("vendor/alias")],
            &ProfileVariableOptions::default(),
        );

        let error = result
            .err()
            .expect("a cached package cannot admit a symlink selector");
        assert!(matches!(
            error.downcast_ref::<ProfileResolutionError>(),
            Some(ProfileResolutionError::UnreadableLocation {
                source: ProfilePackageError::SymlinkedSource { .. },
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_select_profiles_refuses_a_directory_resolving_outside_the_worktree() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let outside = TempDir::new().unwrap();
        package_declaring(&outside, "vendor/workflow", "workflow", &[]);
        std::os::unix::fs::symlink(outside.path(), temp.path().join("outside")).unwrap();

        let result = executor.profiles().select_profiles(
            &[PathBuf::from("outside/vendor/workflow")],
            &ProfileVariableOptions::default(),
        );

        let error = result
            .err()
            .expect("the resolved package must remain in the worktree");
        assert!(matches!(
            error.downcast_ref::<ProfileApplyError>(),
            Some(ProfileApplyError::PackageOutsideWorktree { .. })
        ));
    }

    #[test]
    fn test_plan_profiles_runs_each_selection_stage_once_per_invocation() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (selector, closure) = dependent_selection(&temp, &executor);

        let (planned, stages) = selection_probe::observe(|| {
            plan_selected(&executor, &[selector], &ProfileVariableOptions::default())
        });

        planned.unwrap();
        assert_eq!(stages, selection_probe::once_per_invocation(closure));
    }

    #[test]
    fn test_apply_profiles_runs_each_selection_stage_once_per_invocation() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (selector, closure) = dependent_selection(&temp, &executor);

        let (applied, stages) = selection_probe::observe(|| {
            apply_selected(&executor, &[selector], &ProfileVariableOptions::default())
        });

        applied.unwrap();
        assert_eq!(stages, selection_probe::once_per_invocation(closure));
    }

    #[test]
    fn test_diff_profiles_runs_each_selection_stage_once_per_invocation() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (selector, closure) = dependent_selection(&temp, &executor);

        let (difference, stages) = selection_probe::observe(|| {
            diff_selected(&executor, &[selector], &ProfileVariableOptions::default())
        });

        difference.unwrap();
        assert_eq!(stages, selection_probe::once_per_invocation(closure));
    }
}
