use super::{with_mutation_session, CommandExecutor, SessionStep};
use crate::domain::identity::ProfileId;
use crate::domain::{ProfileLifecycleOperation, ProfileOrigin};
use crate::profile::profile_plan_entry;
use crate::profile::{
    build_profile_claims_from_resolved, capture_package_tree, pack_captured_package_archive,
    read_package_archive, resolve_package, CapturedPackageTree, EngineVersion, PackageCaptureError,
    ProfileAddEntry, ProfileAddResult, ProfileApplicationStatus, ProfileApplyResult,
    ProfileComposedApplyResult, ProfileGraphError, ProfilePackEntry, ProfilePackResult,
    ProfilePackage, ProfilePackageError, ProfilePackageSource, ProfilePlanResult, ProfileShowEntry,
    ProfileShowResult, ProfileTargetAction, ProfileTargetChange, ProfileVariableAssignment,
    ProfileVariableName, ResolvedProfileContent, ResolvedProfileGraph, SelectionObservation,
    VariableInputs, MAX_PROFILE_PACKAGE_ARCHIVE_BYTES,
};
use crate::repository_state::{
    apply_overlay, classify_repository_export, derive_materialization,
    finalize_package_tree_capture, CaptureBudget, CapturedTreeFile, FileMode, MaterializationPlan,
    MaterializationRequest, MutationContext, PackageTreeCapture, ProfileApplicationInput,
    ProfileContributionClaim, RepositoryAction, RepositoryEntry, RepositoryExportDestination,
    RepositoryExportIntent, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryStateError, RootRelativePath, TreeFileDisposition, TreeFileOutcome, VirtualPath,
};
use crate::storage::{IssueStore, RepositoryMutationSession, RepositoryStateStore, WorktreePaths};
use crate::validation::repository::RepositoryValidationFailure;
use anyhow::Result;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Project dependency-first unique durable results back onto ordered root
/// observations. A repeated selector observes the publication's first result,
/// then reports an unchanged second observation without attempting another
/// publication.
pub(super) fn selection_profile_results(
    applied: Vec<ProfileApplyResult>,
    roots: &[ProfilePackage],
) -> Result<ProfileComposedApplyResult> {
    selection_observations(applied, roots).map(ProfileComposedApplyResult::new)
}

/// Project one aggregate selection's per-package answers onto the ordered root
/// observations the caller made.
///
/// A closure member the caller did not name is still part of what the selection
/// settles, so it is reported once, before the roots. A repeated root selector
/// observes the publication its first occurrence accounts for, so its later
/// observations report what [`SelectionObservation::observe_without_publishing`]
/// leaves — one meaning of "this observation publishes nothing", stated once
/// per answer type rather than per projection.
///
/// A rehearsal and the run it precedes project through this one function, so
/// their answers stay comparable entry for entry rather than drifting into two
/// shapes.
fn selection_observations<Observation: Clone + SelectionObservation>(
    closure: Vec<Observation>,
    roots: &[ProfilePackage],
) -> Result<Vec<Observation>> {
    let root_ids = roots
        .iter()
        .map(|root| root.model().id.as_str())
        .collect::<BTreeSet<_>>();
    let by_id = closure
        .iter()
        .map(|observation| (observation.profile_id(), observation))
        .collect::<BTreeMap<_, _>>();
    let mut observations = closure
        .iter()
        .filter(|observation| !root_ids.contains(observation.profile_id()))
        .cloned()
        .collect::<Vec<_>>();
    let mut occurrences = BTreeMap::<&str, usize>::new();
    for root in roots {
        let id = root.model().id.as_str();
        let Some(source) = by_id.get(id) else {
            anyhow::bail!("selected root '{id}' has no aggregate selection answer");
        };
        let occurrence = occurrences.entry(id).or_default();
        let first = *occurrence == 0;
        *occurrence += 1;
        let mut observation = (*source).clone();
        if !first {
            observation.observe_without_publishing();
        }
        observations.push(observation);
    }
    Ok(observations)
}

/// What a prepared selection does with a decision it cannot publish.
///
/// The two answers are the whole difference between a rehearsal and a
/// difference report: both derive the same decision from the same session, and
/// only one of them is allowed to carry a conflict back to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileConflictResponse {
    /// Fail the preparation, exactly where the publication it precedes would.
    Refuse,
    /// Return the decision, conflicts included, for a caller that publishes
    /// nothing.
    Report,
}

/// One ordered profile selection from the command boundary: the package
/// directory, resolved relative to the repository worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileSelector {
    /// A package directory, resolved relative to the repository worktree.
    Path(PathBuf),
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

impl FromStr for ProfileSelector {
    type Err = ProfileSelectorError;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        if selector.is_empty() {
            Err(ProfileSelectorError::EmptyPath)
        } else if selector.starts_with("id:") || selector.starts_with("path:") {
            Err(ProfileSelectorError::TaggedSelector {
                selector: selector.to_string(),
            })
        } else {
            Ok(Self::path(selector))
        }
    }
}

impl ProfileSelector {
    /// Construct a selector from a worktree-relative or absolute spelling.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }
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
    /// A package whose live sources are drawn from repository content could
    /// not be assembled for an archive.
    #[error("profile package location '{location}' cannot be captured: {source}")]
    UncapturableLocation {
        /// Location as the caller supplied it.
        location: String,
        /// Capture failure.
        source: PackageCaptureError,
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
    #[error("final profile application validation failed: {0}")]
    FinalValidation(#[from] RepositoryValidationFailure),
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

/// Project package-tree materialization outcomes into the profile family's one
/// public target-decision vocabulary.
fn tree_target_changes(outcomes: &[TreeFileOutcome]) -> Vec<ProfileTargetChange> {
    outcomes
        .iter()
        .map(|outcome| {
            let action = match outcome.disposition {
                TreeFileDisposition::Unchanged => ProfileTargetAction::Unchanged,
                TreeFileDisposition::Create => ProfileTargetAction::Create,
                TreeFileDisposition::Update => ProfileTargetAction::Update,
                TreeFileDisposition::Remove => ProfileTargetAction::Remove,
            };
            ProfileTargetChange::new(
                outcome.path.repository_relative(),
                action,
                outcome.mode,
                None,
            )
        })
        .collect()
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
/// [`CommandExecutor::select_profiles`].
///
/// Building it reads the packages the selectors name, resolves their
/// dependency graph through [`ResolvedProfileGraph::resolve`], validates the
/// variable inputs against the closure's declarations, and resolves and
/// composes every closure member. Plan, apply, diff and initialization take
/// this value, so an invocation runs each of those stages once.
#[derive(Clone)]
pub struct ProfileSelection {
    roots: Vec<ProfilePackage>,
    graph: ResolvedProfileGraph,
    variable_inputs: VariableInputs,
    members: Vec<SelectedProfile>,
    contribution_context: Vec<ProfileContributionClaim>,
}

impl ProfileSelection {
    /// The selected packages, one per selector occurrence, in occurrence order.
    pub fn roots(&self) -> &[ProfilePackage] {
        &self.roots
    }

    /// The resolved package graph the closure was taken from.
    pub fn graph(&self) -> &ResolvedProfileGraph {
        &self.graph
    }

    /// The validated variable inputs every member was resolved with.
    pub fn variable_inputs(&self) -> &VariableInputs {
        &self.variable_inputs
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
    resolved: ResolvedProfileContent,
    input: ProfileApplicationInput,
}

impl SelectedProfile {
    /// The package as it was read.
    pub fn package(&self) -> &ProfilePackage {
        &self.package
    }

    /// The package's content with the selection's variables substituted.
    pub fn resolved(&self) -> &ResolvedProfileContent {
        &self.resolved
    }
}

impl<S: IssueStore + RepositoryStateStore> CommandExecutor<S> {
    /// Read the package directory one profile command acts on, using the id
    /// declared by that package.
    pub fn resolve_profile_package(&self, selector: &ProfileSelector) -> Result<ProfilePackage> {
        let layout = self.require_layout()?;
        let ProfileSelector::Path(location) = selector;
        supplied_package(location, &layout)
    }

    /// Resolve every selector in occurrence order before any package is applied.
    ///
    /// A repeated selector reads its package once.
    pub fn resolve_profile_selectors(
        &self,
        selectors: &[ProfileSelector],
    ) -> Result<Vec<ProfilePackage>> {
        #[cfg(test)]
        crate::profile::selection_probe::record(
            crate::profile::selection_probe::SelectionStage::SelectorResolution,
        );
        let layout = self.require_layout()?;
        let mut read = BTreeMap::<PathBuf, ProfilePackage>::new();
        selectors
            .iter()
            .map(|selector| {
                let ProfileSelector::Path(location) = selector;
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
        let layout = self.require_layout()?;
        let source = worktree_directory(&layout, invocation_dir, package_source)?;
        let package = capture_package_tree(&source, &layout).map_err(|source_error| {
            ProfileResolutionError::UncapturableLocation {
                location: package_source.display().to_string(),
                source: source_error,
            }
        })?;
        let archive = pack_captured_package_archive(&package)?;
        let archive_bytes = archive.len() as u64;
        let warnings =
            self.publish_package_archive(&layout, invocation_dir, output, archive, publish)?;
        Ok((
            ProfilePackResult::new(vec![ProfilePackEntry {
                id: package.model().id.to_string(),
                version: package.model().version.clone(),
                origin: ProfileOrigin::Directory(source.relative().clone()),
                package_hash: package.hashes().package.clone(),
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
    /// Every [`PackageArchiveError`] the read reports; an archive that cannot
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
        let layout = self.require_layout()?;
        let destination = worktree_directory(&layout, invocation_dir, destination)?;
        let tree = read_package_archive(&read_archive_bytes(invocation_dir, archive)?)?;
        let (status, outcomes) =
            self.publish_package_tree(&layout, &destination, &tree, publish)?;
        Ok(ProfileAddResult::new(vec![ProfileAddEntry {
            id: tree.model().id.to_string(),
            version: tree.model().version.clone(),
            origin: ProfileOrigin::Directory(destination.relative().clone()),
            package_hash: tree.hashes().package.clone(),
            archive: archive.display().to_string(),
            destination: destination.repository_relative(),
            file_count: tree.file_count(),
            byte_size: tree.byte_size(),
            targets: tree_target_changes(&outcomes),
            status,
        }]))
    }

    /// Publish one validated package tree at `destination`.
    ///
    /// A package tree reaches the worktree through one redo record. The
    /// destination must be absent, which is enforced against the same captured
    /// image the plan is derived from rather than by a separate look at the
    /// filesystem. An occupied destination is refused here; one that becomes
    /// occupied afterwards is refused at apply, because the plan creates it
    /// with an absent preimage.
    fn publish_package_tree(
        &self,
        layout: &RepositoryLayout,
        destination: &VirtualPath,
        tree: &CapturedPackageTree,
        publish: bool,
    ) -> Result<(ProfileApplicationStatus, Vec<TreeFileOutcome>)> {
        let capture = PackageTreeCapture::new(
            destination.clone(),
            tree.files()
                .iter()
                .map(|(relative, file)| {
                    Ok(CapturedTreeFile {
                        relative: RootRelativePath::parse(relative)?,
                        bytes: file.bytes.clone(),
                        mode: file.mode,
                    })
                })
                .collect::<Result<Vec<_>, RepositoryLayoutError>>()?,
        )?;
        let budget = package_tree_capture_budget(destination);

        with_mutation_session(self.storage(), layout, "profile package tree", |session| {
            // The destination's whole subtree must be read before the delta is
            // derived, because a file it holds that the tree no longer declares
            // is one a republication removes.
            let Some(image) = super::capture_discovering(
                session,
                capture.capture_spec(budget)?,
                |image, spec| Ok(capture.expand_destination_closure(image, spec)?),
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            if !matches!(image.entry(destination)?, RepositoryEntry::Absent) {
                return Err(ProfileApplyError::PackageDestinationOccupied {
                    path: destination.repository_relative(),
                }
                .into());
            }

            let (plan, outcomes) = finalize_package_tree_capture(&image, &capture)?;
            let status = if plan.delta().actions().is_empty() {
                ProfileApplicationStatus::Unchanged
            } else if !publish {
                ProfileApplicationStatus::WouldApply
            } else {
                ProfileApplicationStatus::Applied
            };
            if publish {
                Ok(SessionStep::Apply(plan, (status, outcomes)))
            } else {
                Ok(SessionStep::Done((status, outcomes)))
            }
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
                    self.publish_repository_export(layout, &intent, budget)
                } else {
                    self.plan_repository_export(layout, &intent, budget)
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
        Ok(ProfileShowEntry {
            id: package.model().id.to_string(),
            version: package.model().version.clone(),
            manifest: package.model().clone(),
            origin: package_origin(package, layout)?,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
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
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileSelection> {
        let roots = self.resolve_profile_selectors(selectors)?;
        self.build_profile_selection(roots, options)
    }

    fn build_profile_selection(
        &self,
        roots: Vec<ProfilePackage>,
        options: &ProfileVariableOptions,
    ) -> Result<ProfileSelection> {
        let graph = self.resolve_profile_graph(&roots)?;
        let closure = graph.selected_packages();
        let variable_inputs = load_profile_variable_inputs(&closure, options)?;
        let layout = self.require_layout()?;
        let members = closure
            .into_iter()
            .map(|package| {
                let targets = package
                    .hashes()
                    .targets
                    .keys()
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
                    resolved,
                    input,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let contribution_context = members
            .iter()
            .flat_map(|member| member.input.claims.contributions.iter().cloned())
            .collect::<Vec<_>>();
        let members = members
            .into_iter()
            .map(|member| SelectedProfile {
                input: member
                    .input
                    .with_contribution_context(&contribution_context),
                ..member
            })
            .collect();
        Ok(ProfileSelection {
            roots,
            graph,
            variable_inputs,
            members,
            contribution_context,
        })
    }

    /// Rehearse applying `selection`, refusing a decision the application
    /// would refuse.
    pub fn plan_profiles(&self, selection: &ProfileSelection) -> Result<ProfilePlanResult> {
        self.preview_profile_selection(selection, ProfileConflictResponse::Refuse)
    }

    /// Resolve and apply profiles by selector with no variable inputs.
    pub fn apply_profile(
        &self,
        selectors: &[ProfileSelector],
    ) -> Result<ProfileComposedApplyResult> {
        self.apply_profiles(&self.select_profiles(selectors, &ProfileVariableOptions::default())?)
    }

    /// Apply `selection`'s complete closure through one held session.
    ///
    /// Selector occurrence order remains a result concern only.
    pub fn apply_profiles(
        &self,
        selection: &ProfileSelection,
    ) -> Result<ProfileComposedApplyResult> {
        if selection.members.is_empty() {
            return Ok(ProfileComposedApplyResult::new(Vec::new()));
        }
        let applied = self.publish_profile_selection(selection)?;
        selection_profile_results(applied.items, &selection.roots)
    }

    /// Report what applying a selection would do to this repository, without
    /// publishing any of it.
    ///
    /// The report is the decision itself: the selection is planned through
    /// the same preparation a publication runs, and every target and registry
    /// declaration each participating profile decided is then stated — the
    /// values it would create or update and the ones it cannot publish, each
    /// named beside the packages that define it. A conflict is reported rather
    /// than refused, which is the whole difference from the rehearsal the
    /// lifecycle commands run, so an adopter can read a conflicting decision
    /// before it decides whether to publish one.
    ///
    /// A package that is not installed is reported as readily as one that is:
    /// the selection is resolved from the location its selector names, so an
    /// adopter inspects a package before applying it rather than after.
    ///
    /// Nothing is written. The capture runs under the same session guard the
    /// publication would, so the answer describes the repository the
    /// publication would act on.
    ///
    /// # Errors
    ///
    /// A repository that cannot be read.
    pub fn diff_profiles(&self, selection: &ProfileSelection) -> Result<ProfilePlanResult> {
        self.preview_profile_selection(selection, ProfileConflictResponse::Report)
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

    /// Resolve the selected packages' dependency closure, then settle every
    /// dependency, incompatibility, and engine range claim before a caller
    /// prepares a publication.
    ///
    /// Only the selection's own closure is a candidate: a package this
    /// repository applied earlier takes no part unless the selection reaches it.
    pub fn resolve_profile_graph(
        &self,
        selected: &[ProfilePackage],
    ) -> Result<ResolvedProfileGraph> {
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
        ResolvedProfileGraph::resolve(packages, selected_ids, &engine).map_err(profile_graph_error)
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
                    .resolve_dependency_package_from(&declaring, &dependency.id)
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
        let ProfilePackageSource::Directory(directory) = declaring.source();
        let not_found = || -> anyhow::Error {
            crate::errors::NotFoundError::new(format!("Profile not found: {dependency}")).into()
        };
        let Some(location) = directory
            .parent()
            .map(|parent| parent.join(dependency.as_str()))
        else {
            return Err(not_found());
        };
        match supplied_package(&location, &self.require_layout()?) {
            Ok(package) if package.model().id == *dependency => Ok(package),
            Ok(_) => Err(not_found()),
            Err(error) if is_unreadable_location(&error) => Err(not_found()),
            Err(error) => Err(error),
        }
    }

    /// Apply one validated package together with the packages it depends on,
    /// with no variable inputs.
    ///
    /// The whole closure is resolved and ordered first, so a cycle or an
    /// unresolvable dependency fails before a plan is published.
    pub fn apply_profile_package(
        &self,
        package: &ProfilePackage,
    ) -> Result<ProfileComposedApplyResult> {
        self.apply_profiles(
            &self.build_profile_selection(
                vec![package.clone()],
                &ProfileVariableOptions::default(),
            )?,
        )
    }

    /// Publish the selection's dependency-first unique closure through one
    /// mutation session and one `session.apply` call at most.
    fn publish_profile_selection(
        &self,
        selection: &ProfileSelection,
    ) -> Result<ProfileComposedApplyResult> {
        let layout = self.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_session(self.storage(), &layout, "profile application", |session| {
            let Some(plan) = self.prepare_profile_selection(
                session,
                selection,
                &context,
                ProfileLifecycleOperation::Apply,
                ProfileConflictResponse::Refuse,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            let results = |changed: &dyn Fn(&ProfilePackage) -> bool| {
                selection
                    .members
                    .iter()
                    .map(|member| {
                        applied_profile_result(
                            &plan,
                            &member.package,
                            &layout,
                            if changed(&member.package) {
                                ProfileApplicationStatus::Applied
                            } else {
                                ProfileApplicationStatus::Unchanged
                            },
                        )
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(ProfileComposedApplyResult::new)
            };
            if plan.delta().actions().is_empty() {
                return Ok(SessionStep::Done(results(&|_| false)?));
            }
            let overrides = super::validation_overlay(plan.image(), plan.delta())?;
            let proposed = apply_overlay(plan.image(), overrides).map_err(anyhow::Error::from)?;
            let validation = crate::validation::repository::validate_repository(&proposed)
                .map_err(ProfileApplyError::from)?;
            if validation.rule_report.has_errors() {
                return Err(ProfileApplyError::FinalValidationFindings {
                    error_count: validation.rule_report.error_count(),
                }
                .into());
            }
            let applied =
                results(&|package| plan.applied_profiles().contains(&package.model().id))?;
            Ok(SessionStep::Apply(plan, applied))
        })
    }

    /// Preview `selection` through the same preparation durable publication
    /// runs, deciding what a conflict does.
    ///
    /// A rehearsal and a difference report reach this one preview with
    /// different answers to that question and nothing else, so their entries
    /// stay comparable entry for entry.
    fn preview_profile_selection(
        &self,
        selection: &ProfileSelection,
        response: ProfileConflictResponse,
    ) -> Result<ProfilePlanResult> {
        if selection.members.is_empty() {
            return Ok(ProfilePlanResult::new(Vec::new()));
        }
        let layout = self.require_layout()?;
        let context = MutationContext::preview();
        with_mutation_session(self.storage(), &layout, "profile planning", |session| {
            let Some(plan) = self.prepare_profile_selection(
                session,
                selection,
                &context,
                ProfileLifecycleOperation::Apply,
                response,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            let rehearsed = selection
                .members
                .iter()
                .map(|member| {
                    Ok(profile_plan_entry(
                        &plan,
                        &member.package.model().id,
                        &member.package.model().version,
                        package_origin(&member.package, &layout)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let plans = selection_observations(rehearsed, &selection.roots)?;
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
    ) -> Result<
        Option<(
            MaterializationPlan,
            Vec<crate::profile::ProfileTargetChange>,
        )>,
    > {
        let layout = self.require_layout()?;
        let selection = self
            .build_profile_selection(vec![package.clone()], &ProfileVariableOptions::default())?;
        let plan = self.prepare_profile_selection(
            session,
            &selection,
            context,
            ProfileLifecycleOperation::Apply,
            ProfileConflictResponse::Refuse,
        )?;
        let origin = package_origin(package, &layout)?;
        Ok(plan.map(|plan| {
            let changes =
                profile_plan_entry(&plan, &package.model().id, &package.model().version, origin)
                    .targets;
            (plan, changes)
        }))
    }

    /// Capture and close the complete selection once. A retry restarts from a
    /// fresh held session image; no member of the selection is published until
    /// the final aggregate plan reaches `session.apply`.
    ///
    /// `response` decides what a decision that cannot be published does here.
    /// Every route that publishes or rehearses a publication refuses it, and
    /// the difference report is the sole reader that carries it through, so a
    /// conflicting target or declaration reaches an adopter as a report entry
    /// exactly once and as a refusal everywhere else.
    fn prepare_profile_selection(
        &self,
        session: &mut (dyn RepositoryMutationSession + '_),
        selection: &ProfileSelection,
        context: &MutationContext,
        operation: ProfileLifecycleOperation,
        response: ProfileConflictResponse,
    ) -> Result<Option<MaterializationPlan>> {
        let contribution_context = selection.contribution_context();
        let inputs = selection.application_inputs();
        let events_path = VirtualPath::EVENTS;

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
        )?);
        content_paths.push(events_path.clone());

        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &content_paths, None)? {
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
        let Some(base) =
            self.capture_proposed_base(session, &BTreeMap::new(), &expanded_paths, Some(base))?
        else {
            return Ok(None);
        };
        let preview = derive_materialization(
            &base,
            MaterializationRequest::ApplyProfileSelection {
                profiles: &inputs,
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
        let overrides = super::validation_overlay(preview.image(), preview.delta())?;
        let Some(probe) =
            self.capture_proposed_base(session, &overrides, &extra_paths, Some(base))?
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
            MaterializationRequest::ApplyProfileSelection {
                profiles: &inputs,
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
        if response == ProfileConflictResponse::Refuse {
            crate::profile::ensure_publishable_targets(&plan)
                .map_err(RepositoryStateError::from)?;
            ensure_committed_write_targets(&plan, &self.require_layout()?)?;
        }
        Ok(Some(plan))
    }
}

/// Refuse `plan` when a path its delta writes differs from `HEAD`.
///
/// Each root is classified on its own: a relocated data root can sit outside
/// the checkout holding the worktree targets, or in another one.
fn ensure_committed_write_targets(
    plan: &MaterializationPlan,
    layout: &RepositoryLayout,
) -> Result<()> {
    let paths = [
        (RepositoryRootClass::Worktree, layout.worktree_root()),
        (RepositoryRootClass::Data, layout.data_root()),
    ]
    .into_iter()
    .map(|(class, root)| uncommitted_write_targets(plan, class, root))
    .collect::<Result<Vec<_>>>()?
    .into_iter()
    .flatten()
    .collect::<BTreeSet<_>>();
    if paths.is_empty() {
        return Ok(());
    }
    Err(ProfileApplyError::UncommittedTargets {
        paths: paths.into_iter().collect(),
    }
    .into())
}

/// Checkout-relative dirty paths among the ones `plan` writes under `root`,
/// empty when no git checkout holds `root`.
///
/// An event-log append is left out because it keeps every uncommitted line.
fn uncommitted_write_targets(
    plan: &MaterializationPlan,
    class: RepositoryRootClass,
    root: &Path,
) -> Result<Vec<String>> {
    let written = plan
        .delta()
        .actions()
        .iter()
        .filter(|action| !matches!(action, RepositoryAction::AppendEvents { .. }))
        .map(RepositoryAction::path)
        .filter(|path| path.root_class() == class)
        .map(|path| path.relative().as_path())
        .collect::<BTreeSet<_>>();
    if written.is_empty() {
        return Ok(Vec::new());
    }
    let paths = WorktreePaths::detect_from(root)?;
    let Some(checkout) = paths.git() else {
        return Ok(Vec::new());
    };
    // Git reports paths from its physical top level.
    let physical_root = root.canonicalize()?;
    let offset = physical_root.strip_prefix(&paths.worktree_root)?;
    Ok(checkout
        .dirty_paths()?
        .into_iter()
        .filter(|dirty| {
            dirty
                .strip_prefix(offset)
                .is_ok_and(|relative| written.contains(relative))
        })
        .map(|dirty| dirty.display().to_string())
        .collect())
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

/// How many package-model byte budgets one package-tree publication may span.
///
/// One is the tree being published. One more is what the destination already
/// holds, which a previous publication left there as a package tree bounded the
/// same way. The rest is headroom for content an adopter added beside it, so a
/// republication is refused for what the destination holds rather than for the
/// shape of the package arriving at it.
const PACKAGE_TREE_MODEL_BUDGETS: usize = 8;

/// Bounds for publishing one package tree at `destination`.
///
/// Every bound here is derived from what the package model constrains. That
/// matters because the model bounds a package's file count
/// ([`MAX_PROFILE_PACKAGE_FILES`]) and its total size
/// ([`MAX_PROFILE_PACKAGE_BYTES`]) and says nothing about how deep or how long
/// its paths are, so a bound assuming a path shape refuses packages the model
/// admits — an archive `jit profile pack` writes that `jit profile add` will
/// not take.
///
/// One property of the model carries all three. The manifest declares every
/// file the package holds besides itself, by name, and no source may be
/// declared twice, so the package's declared path lengths sum to less than the
/// manifest's own size — which is itself part of the same byte budget its
/// content is. From that:
///
/// * **Listings.** A republication takes one per directory beneath the
///   destination. Each directory the package needs is a proper prefix of a
///   declared path ending at a separator, so the package contributes fewer of
///   them than its declared paths have bytes.
/// * **Depth.** A path's component count is below its own byte length, so no
///   declared path reaches a package byte budget's worth of components below
///   the destination the caller named.
/// * **Bytes.** What actually binds a publication: the tree's content is at
///   most one package byte budget, and a republication also reads what the
///   destination already holds.
///
/// Nothing bounds how many paths the capture declares, because the same
/// property already bounds them: the closure is the declared files plus their
/// directory prefixes, so it stays under `MAX_PROFILE_PACKAGE_FILES` plus a
/// package byte budget. A budget stating a number that large refuses no package
/// the model admits, and any smaller number refuses packages it does admit.
///
/// [`MAX_PROFILE_PACKAGE_FILES`]: crate::profile::MAX_PROFILE_PACKAGE_FILES
/// [`MAX_PROFILE_PACKAGE_BYTES`]: crate::profile::MAX_PROFILE_PACKAGE_BYTES
fn package_tree_capture_budget(destination: &VirtualPath) -> CaptureBudget {
    // Saturating throughout: a bound that wrapped would be smaller than the
    // model demands, which is the failure this function exists to prevent. The
    // only term this build cannot compute at compile time is the destination's
    // own depth, which the caller supplies.
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
        .map_err(|source| {
            anyhow::anyhow!(
                "cannot read profile package archive '{}': {source}",
                archive.display()
            )
        })?
        .take(MAX_PROFILE_PACKAGE_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
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
/// The location is taken from the source the package recorded while reading
/// itself, never from a path supplied beside it. A directory that resolves
/// outside the worktree is refused, because a report names a package by a
/// location that does not depend on machine state. The selected data root is
/// not a worktree location even when it nests inside one, so a package placed
/// under it is refused by the same rule.
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

/// Project one aggregate publication decision into the same per-profile facts
/// its rehearsal reports.
pub(super) fn applied_profile_result(
    plan: &MaterializationPlan,
    package: &ProfilePackage,
    layout: &RepositoryLayout,
    status: ProfileApplicationStatus,
) -> Result<ProfileApplyResult> {
    let preview = profile_plan_entry(
        plan,
        &package.model().id,
        &package.model().version,
        package_origin(package, layout)?,
    );
    Ok(ProfileApplyResult {
        id: preview.id,
        version: preview.version,
        origin: preview.origin,
        status,
        targets: preview.targets,
        contributions: preview.contributions,
    })
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
        target_hashes: resolved.target_hashes()?,
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
    use crate::profile::{
        ProfileContributionChange, ProfilePlanEntry, ProfilePlanStatus, ProfileTargetAction,
    };
    use crate::repository_state::{
        Contribution, ContributionConflictOwner, ContributionIdentity, MapEntryTarget,
        ProfileTargetConflict, ProfileTargetConflictEntry, ProfileTargetSubject,
        RepositoryStateError, RootRelativePath, ScalarTarget, SetStringTarget,
    };
    use crate::storage::JsonFileStorage;
    use crate::storage::{
        discover_repository_layout, IssueStore, MutationPoint, RedoPoint, RepositoryStateStore,
    };
    use crate::test_utils::{file_executor, setup_test_repo, setup_test_repo_with_config};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Observe actual publications without coupling the test to
    /// preparatory graph or capture sessions. This apply edge runs
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

    impl crate::storage::MutationInterruptionInjector for PublicationCounter {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            if point == &MutationPoint::Redo(RedoPoint::BeforeApply) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

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
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfileComposedApplyResult> {
        executor
            .select_profiles(selectors, options)
            .and_then(|selection| executor.apply_profiles(&selection))
    }

    /// Rehearse `selectors` the way `jit profile apply --dry-run` does.
    fn plan_selected(
        executor: &CommandExecutor<JsonFileStorage>,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfilePlanResult> {
        executor
            .select_profiles(selectors, options)
            .and_then(|selection| executor.plan_profiles(&selection))
    }

    /// Report `selectors` the way `jit profile diff` does.
    fn diff_selected(
        executor: &CommandExecutor<JsonFileStorage>,
        selectors: &[ProfileSelector],
        options: &ProfileVariableOptions,
    ) -> Result<ProfilePlanResult> {
        executor
            .select_profiles(selectors, options)
            .and_then(|selection| executor.diff_profiles(&selection))
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
            ProfileTargetSubject::File(path) => path.repository_relative(),
            ProfileTargetSubject::Contribution(identity) => {
                panic!("this refusal is about a target, not the declaration '{identity}'")
            }
        }
    }

    /// The one declaration `error` refused, beside every owner defining it.
    ///
    /// A contested declaration reaches an adopter as a refused decision of the
    /// same shape a contested target does, so a test reads it out of the one
    /// refusal rather than out of a second error family
    /// (`@/invariant/semantic-test-assertions`). Every profile that declares it
    /// decides it, so a selection carrying two of them refuses the same
    /// declaration once per profile and states one answer.
    fn refused_declaration(
        error: &anyhow::Error,
    ) -> (ContributionIdentity, Vec<ContributionConflictOwner>) {
        let declarations = refused_targets(error)
            .iter()
            .filter_map(|entry| match (&entry.subject, &entry.conflict) {
                (
                    ProfileTargetSubject::Contribution(identity),
                    ProfileTargetConflict::Contested { owners },
                ) => Some((identity.clone(), owners.clone())),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            declarations.len(),
            1,
            "this selection refuses one declaration: {error:#}"
        );
        declarations
            .into_iter()
            .next()
            .expect("the refused declaration")
    }

    #[test]
    fn test_resolve_profile_package_reads_the_package_a_supplied_location_holds() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
    fn test_apply_profile_package_resolves_a_declared_dependency_beside_the_declaring_package() {
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
        let applied = executor.apply_profile_package(&workflow).unwrap();

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

        executor
            .apply_profile_package(&workflow)
            .unwrap_or_else(|error| panic!("{error:#}"));

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
            "packages/workflow".parse::<ProfileSelector>(),
            Ok(ProfileSelector::path("packages/workflow"))
        );
    }

    #[test]
    fn test_profile_selector_parse_refuses_a_retired_tag_naming_the_bare_path_form() {
        for selector in ["id:workflow", "path:packages/workflow", "path:"] {
            let error = selector
                .parse::<ProfileSelector>()
                .expect_err("a tagged selector is refused");
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
            "".parse::<ProfileSelector>(),
            Err(ProfileSelectorError::EmptyPath)
        );
    }

    #[test]
    fn test_resolve_profile_package_path_selector_uses_the_directory_package_id() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, "vendor/supplied");
        let selector = ProfileSelector::path(temp.path().join("vendor/supplied"));
        let resolved = executor.resolve_profile_package(&selector).unwrap();
        assert_eq!(resolved.model().id, package.model().id);
    }

    #[test]
    fn test_resolve_profile_package_reports_a_supplied_location_holding_no_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();

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
    fn test_apply_profile_package_reports_the_worktree_location_it_read_the_package_from() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, "vendor/profiles/planner");

        let applied = executor.apply_profile_package(&package).unwrap();

        // The reported location resolves, from the worktree root alone, back to
        // the directory whose bytes were applied.
        let ProfileOrigin::Directory(location) = &applied.requested().unwrap().origin;
        assert_eq!(
            ProfilePackage::from_directory(&temp.path().join(location.as_path()))
                .expect("the reported location names a readable package")
                .hashes(),
            package.hashes()
        );
    }

    #[test]
    fn test_apply_profile_package_reports_the_directory_read_rather_than_a_peer_copy() {
        // Two directories inside the worktree hold byte-identical packages.
        // Nothing but the directory a package was read through distinguishes
        // them, so the reported location must follow that and only that.
        let (first_temp, _) = setup_test_repo().unwrap();
        let first_executor = file_executor(first_temp.path()).unwrap();
        let (second_temp, _) = setup_test_repo().unwrap();
        let second_executor = file_executor(second_temp.path()).unwrap();
        let first = package_read_from(&first_temp, "vendor/first");
        let second = package_read_from(&second_temp, "packages/second/tree");
        assert_eq!(first.hashes(), second.hashes());

        let first_applied = first_executor.apply_profile_package(&first).unwrap();
        let second_applied = second_executor.apply_profile_package(&second).unwrap();

        assert_eq!(
            first_applied.requested().unwrap().origin,
            ProfileOrigin::Directory(RootRelativePath::parse("vendor/first").unwrap())
        );
        assert_eq!(
            second_applied.requested().unwrap().origin,
            ProfileOrigin::Directory(RootRelativePath::parse("packages/second/tree").unwrap())
        );
    }

    #[test]
    fn test_apply_profile_package_refuses_a_package_read_from_outside_the_worktree() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
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
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_refuses_a_package_read_from_the_data_root() {
        // The selected data root is not a worktree location, so a package
        // placed under it has no worktree-relative location to report.
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
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
                |contribution| contribution.registry_path().repository_relative()
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
    fn test_apply_profile_package_repairs_a_generated_schema_outside_the_package_targets() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
    fn test_apply_profile_package_jit_dogfood_composes_what_applying_both_packages_composes() {
        // Two bare repositories, one per route: the explicit one has both
        // packages applied to it in turn, and the resolved one names only the
        // workflow package and receives the default as its declared
        // dependency. They start identical, so the configurations they end with
        // may differ only by the route that reached them.
        let (explicit_temp, explicit_executor) =
            setup_test_repo_with_config(COMPOSITION_CONFIG).unwrap();
        let explicit_default = shipped_package_in(explicit_temp.path(), "jit-default");
        let explicit_dogfood = shipped_package_in(explicit_temp.path(), "jit-dogfood");
        let (resolved_temp, resolved_executor) =
            setup_test_repo_with_config(COMPOSITION_CONFIG).unwrap();
        shipped_package_in(resolved_temp.path(), "jit-default");
        let resolved_dogfood = shipped_package_in(resolved_temp.path(), "jit-dogfood");

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
                .items
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

        let applied = executor.apply_profile_package(&package).unwrap();
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            package.source_bytes("assets/profile.txt").unwrap()
        );
        assert!(!temp.path().join(".jit/profiles").exists());
        assert_eq!(storage.read_events().unwrap().len(), 1);

        let before = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let unchanged = executor.apply_profile_package(&package).unwrap();
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
    fn test_apply_profile_package_rejects_each_reserved_application_target() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();

        for (index, target) in [".jit/events.jsonl", ".jit/profiles", ".git/config"]
            .into_iter()
            .enumerate()
        {
            let package = package_publishing(
                &temp,
                &format!("packages/reserved-{index}"),
                &format!("reserved-{index}"),
                target,
                "reserved target content\n",
            );
            let error = executor.apply_profile_package(&package).unwrap_err();

            assert!(
                matches!(
                    error.downcast_ref::<ProfileApplyError>(),
                    Some(ProfileApplyError::ReservedApplicationTarget { path }) if path == target
                ),
                "target '{target}' should be refused as application-owned state: {error:#}"
            );
        }

        assert!(storage.read_events().unwrap().is_empty());
        assert!(!temp.path().join(".jit/profiles").exists());
    }

    #[test]
    fn test_apply_profile_package_reapplies_changed_content() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        executor.apply_profile_package(&package).unwrap();
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
        assert_eq!(
            fs::read_to_string(unowned).unwrap(),
            "repository-authored\n",
            "a package reapply does not delete unowned repository content"
        );
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
        let selection = executor
            .select_profiles(&[package_selector(&package)], &options)
            .unwrap();
        let inputs = selection.variable_inputs();
        assert_eq!(
            inputs.values_file[&"NAME".try_into().unwrap()],
            "values-file-value"
        );
        assert_eq!(
            inputs.command_line.last().unwrap().value,
            "resolved-value-last"
        );

        executor.apply_profiles(&selection).unwrap();

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

        let first = executor
            .prepare_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();
        let second = executor
            .prepare_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();

        assert_eq!(first.0.delta(), second.0.delta());
        assert_eq!(first.1, second.1);
    }

    #[test]
    fn test_profile_plan_collection_preserves_selector_occurrences_without_events() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_read_from(&temp, FIXTURE_LOCATION);
        let before = storage.read_events().unwrap();
        let selector = ProfileSelector::path(FIXTURE_LOCATION);

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

        assert!(executor.apply_profile_package(&package).is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_application_preserves_and_certifies_torn_event_tail_end_to_end() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        let torn = b"{\"torn\":";
        fs::write(temp.path().join(".jit/events.jsonl"), torn).unwrap();

        executor.apply_profile_package(&package).unwrap();

        let image = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        assert!(image.starts_with(b"{\"torn\":\n"));
        assert_eq!(&image[..torn.len()], torn);
        let events = storage.read_events().unwrap();
        assert!(matches!(
            events
                .iter()
                .map(Event::body)
                .collect::<Vec<_>>()
                .as_slice(),
            [EventBody::ProfileLifecycle {
                isolated_torn_tail: true,
                ..
            }]
        ));
    }

    #[test]
    fn test_apply_profile_package_applies_a_declared_dependency_before_the_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

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
        assert_eq!(applied.requested().unwrap().id, "workflow");
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
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        let left = package_declaring(&temp, "vendor/left", "left", &[]);
        let right = package_declaring(&temp, "vendor/right", "right", &[]);
        let publications = PublicationCounter::new();
        let storage = JsonFileStorage::with_mutation_interruptions(
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
        executor
            .apply_profile(&[package_selector(&left), package_selector(&right)])
            .unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let publications = PublicationCounter::new();
        let storage = JsonFileStorage::with_mutation_interruptions(
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
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());

        assert!(executor.apply_profile(&selectors).is_err());

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

        let retried = CommandExecutor::new(recovered.clone())
            .with_layout(discover_repository_layout(temp.path(), recovered.root()).unwrap())
            .apply_profile(&selectors)
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

        let applied = executor
            .apply_profile(&[package_selector(&package), package_selector(&package)])
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
    fn test_apply_profile_package_applies_a_transitive_dependency_chain_deepest_first() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/vocabulary", "vocabulary", &[]);
        package_declaring(&temp, "vendor/base", "base", &["vocabulary"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

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
    fn test_apply_profile_package_applies_a_shared_dependency_once() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/shared", "shared", &[]);
        package_declaring(&temp, "vendor/left", "left", &["shared"]);
        package_declaring(&temp, "vendor/right", "right", &["shared"]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["left", "right"]);

        let applied = executor.apply_profile_package(&dependant).unwrap();

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
        executor.apply_profile_package(&package).unwrap();

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
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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

        let (_, owners) = refused_declaration(&error);
        assert_eq!(
            owners,
            vec![
                ContributionConflictOwner::Package(ProfileId::try_from("base").unwrap()),
                ContributionConflictOwner::Package(ProfileId::try_from("workflow").unwrap()),
            ]
        );
        for path in ["docs/base.txt", "docs/workflow.txt"] {
            assert!(
                !temp.path().join(path).exists(),
                "the conflicting closure published {path}"
            );
        }
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
                        ContributionConflictOwner::Package(ProfileId::try_from("base").unwrap()),
                        ContributionConflictOwner::Package(
                            ProfileId::try_from("workflow").unwrap()
                        ),
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
    fn test_apply_profile_package_merges_an_identical_restatement_of_an_applied_target() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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

        // A restatement of what is already there is not a conflict, and it
        // publishes nothing: the repository already holds its content.
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read_to_string(temp.path().join(target)).unwrap(),
            "identical\n"
        );
    }

    #[test]
    fn test_apply_profile_package_recontributes_its_own_semantic_identity() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
    }

    #[test]
    fn test_apply_profile_package_keeps_distinct_identities_in_one_registry_independent() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
    }

    #[test]
    fn test_apply_profile_package_rejects_a_dependency_cycle_before_applying_anything() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
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
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_package_reports_a_dependency_that_cannot_be_resolved() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        // Nothing sits beside the package under that name, so no route
        // resolves the dependency.
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
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_apply_profile_rejects_a_dependency_version_mismatch_before_publication() {
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
    fn test_apply_profile_excludes_a_previously_applied_package_from_incompatibility_checks() {
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

        // `base` is in the repository but not in the selection, so the
        // incompatibility it would answer has no party to hold against.
        executor
            .apply_profile(&[package_selector(&candidate)])
            .unwrap();

        assert!(temp.path().join("docs/candidate.txt").is_file());
        assert_eq!(applied_event_ids(&storage), vec!["base", "candidate"]);
    }

    #[test]
    fn test_apply_profile_package_reports_no_work_for_an_applied_unchanged_closure() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);
        executor.apply_profile_package(&dependant).unwrap();
        let events = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();

        let reapplied = executor.apply_profile_package(&dependant).unwrap();

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
    fn test_apply_profile_package_refuses_a_dependency_only_a_previous_application_supplied() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let dependency = package_declaring(&temp, "vendor/base", "base", &[]);
        executor.apply_profile_package(&dependency).unwrap();
        // A dependant elsewhere in the worktree, with no `base` beside it. The
        // earlier application of `base` is not a candidate for the selection.
        let dependant = package_declaring(&temp, "elsewhere/workflow", "workflow", &["base"]);
        assert!(!temp.path().join("elsewhere/base").exists());

        let error = executor.apply_profile_package(&dependant).unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ProfileDependencyError>(),
                Some(ProfileDependencyError::UnresolvableDependency { package, dependency, .. })
                    if package == "workflow" && dependency == "base"
            ),
            "the refusal must name both packages: {error:#}"
        );
        assert!(!temp.path().join("docs/workflow.txt").exists());
        assert_eq!(applied_event_ids(&storage), vec!["base"]);
    }

    #[test]
    fn test_resolve_profile_closure_reads_a_dependency_from_beside_the_declaring_package() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        // The package beside the declaring package is the one its declaration
        // means.
        let beside = package_declaring(&temp, "vendor/base", "base", &[]);
        let dependant = package_declaring(&temp, "vendor/workflow", "workflow", &["base"]);

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
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
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
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let package = package_read_from(&temp, FIXTURE_LOCATION);

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
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let storage = executor.storage().clone();
        let package = package_read_from(&temp, FIXTURE_LOCATION);
        let prior_event = crate::test_utils::stamped_event(
            EventBody::draft_profile_lifecycle(
                ProfileLifecycleOperation::Apply,
                vec![crate::domain::ProfileLifecycleProfile {
                    id: "prior".try_into().unwrap(),
                    version: "1.0.0".to_string(),
                    status: ProfileLifecycleStatus::Installed,
                    variables: Default::default(),
                }],
            ),
            String::new(),
            chrono::DateTime::UNIX_EPOCH,
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
            events.last().unwrap().body(),
            EventBody::ProfileLifecycle {
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
    fn reported(report: &ProfilePlanResult, id: &str) -> ProfilePlanEntry {
        report
            .items
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("the report names profile {id}: {report:?}"))
            .clone()
    }

    /// The targets `entry` decided `action` about, by repository-relative name.
    fn decided_targets(entry: &ProfilePlanEntry, action: ProfileTargetAction) -> BTreeSet<String> {
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
        assert_eq!(entry.status, ProfilePlanStatus::WouldConflict);
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
        assert_eq!(entry.status, ProfilePlanStatus::WouldApply);
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
    fn declared(entry: &ProfilePlanEntry, identity: &str) -> ProfileContributionChange {
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
        assert_eq!(entry.status, ProfilePlanStatus::WouldConflict);
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
                    ProfileTargetSubject::Contribution(identity) =>
                        Some((identity.to_string(), refused.conflict.message())),
                    ProfileTargetSubject::File(_) => None,
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

    /// A package reapplied with a changed definition of its own declaration
    /// is not a conflict party with itself: the repository's current value
    /// never contests the package being applied.
    #[test]
    fn test_apply_profile_package_reapplies_a_changed_contribution_without_a_conflict() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let namespace = "reviewed";
        executor
            .apply_profile_package(&package_contributing(
                &temp,
                "vendor/workflow",
                "workflow",
                namespace,
                "The first wording.",
            ))
            .unwrap();
        let changed = package_contributing(
            &temp,
            "vendor/workflow",
            "workflow",
            namespace,
            "The second wording.",
        );

        let applied = executor.apply_profile_package(&changed);

        assert!(applied.is_ok(), "{:#}", applied.unwrap_err());
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
    ) -> (ProfileSelector, usize) {
        package_declaring(temp, "vendor/base", "base", &[]);
        let root = package_declaring(temp, "vendor/workflow", "workflow", &["base"]);
        let closure = executor.resolve_profile_closure(&root).unwrap().len();
        (package_selector(&root), closure)
    }

    #[test]
    fn test_select_profiles_validates_equivalent_directory_spellings_once() {
        let (temp, _) = setup_test_repo().unwrap();
        let executor = file_executor(temp.path()).unwrap();
        let (absolute, closure) = dependent_selection(&temp, &executor);
        let selectors = vec![
            ProfileSelector::path("vendor/workflow"),
            absolute,
            ProfileSelector::path("vendor/./workflow/"),
        ];
        #[cfg(unix)]
        let selectors = {
            std::os::unix::fs::symlink("vendor", temp.path().join("alias")).unwrap();
            let mut selectors = selectors;
            selectors.push(ProfileSelector::path("alias/workflow"));
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
                    executor.select_profiles(
                        &[ProfileSelector::path(location)],
                        &ProfileVariableOptions::default(),
                    )
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

        let result = executor.select_profiles(
            &[selector, ProfileSelector::path("vendor/alias")],
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

        let result = executor.select_profiles(
            &[ProfileSelector::path("outside/vendor/workflow")],
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
