use super::{capture_or_retry, with_mutation_session, CommandExecutor, SessionStep};
use crate::profile::{
    build_profile_claims, jit_dogfood_package, ProfileApplicationStatus, ProfileApplyResult,
    ProfileListResult, ProfileOrigin, ProfilePackage, ProfilePackageSource, ProfilePlanResult,
    ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction, ProfileTargetChange,
};
use crate::repository_state::{
    apply_overlay, derive_materialization, AppliedProfileRecord, CaptureBudget, CaptureSpec,
    MaterializationPlan, MaterializationRequest, MutationContext, ProfileApplicationInput,
    ProfileTargetDisposition, RepositoryEntry, RepositoryImage, RepositoryRootClass,
    RootRelativePath, VirtualPath,
};
use crate::storage::{JsonFileStorage, RepositoryMutationSession};
use crate::validation::repository::RepositoryValidationFailure;
use anyhow::Result;
use std::collections::BTreeMap;

/// Profile application conflict detected before transaction preparation.
#[derive(Debug, thiserror::Error)]
pub enum ProfileApplyError {
    /// The installed record is not valid for the requested profile.
    #[error("installed profile record '{path}' conflicts with embedded package {id}@{version}")]
    InstalledRecordConflict {
        /// Repository-relative installed-record path.
        path: String,
        /// Requested profile id.
        id: String,
        /// Requested profile version.
        version: String,
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
    /// List the immutable profiles embedded in this binary.
    pub fn list_embedded_profiles(&self) -> Result<ProfileListResult> {
        let package = jit_dogfood_package()?;
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        let expected = expected_record(&package, &layout)?;
        let applied = self
            .read_installed_record(&package)?
            .is_some_and(|record| record == expected);
        let profiles = vec![ProfileSummary {
            id: metadata.id.to_string(),
            version: metadata.version.clone(),
            origin: package_origin(&package, &layout)?,
            jit: metadata.jit.clone(),
            applied,
        }];
        Ok(ProfileListResult {
            count: profiles.len(),
            profiles,
        })
    }

    /// Inspect one immutable embedded profile package.
    pub fn show_embedded_profile(&self, id: &str) -> Result<ProfileShowResult> {
        let package = embedded_profile(id)?;
        let layout = self.require_layout()?;
        Ok(ProfileShowResult {
            manifest: package.manifest().clone(),
            origin: package_origin(&package, &layout)?,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
            file_count: package.file_count(),
            byte_size: package.byte_size(),
            applied: self.read_installed_record(&package)?,
        })
    }

    /// Build the exact non-mutating target plan for one embedded profile.
    pub fn plan_embedded_profile(&self, id: &str) -> Result<ProfilePlanResult> {
        let package = embedded_profile(id)?;
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        let context = MutationContext::preview();
        with_mutation_session(self.storage(), &layout, "profile planning", |session| {
            let Some((plan, changes)) =
                self.prepare_embedded_profile(session, &package, &context)?
            else {
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

    /// Resolve and apply one embedded profile by stable ID.
    pub fn apply_profile(&self, id: &str) -> Result<ProfileApplyResult> {
        let package = embedded_profile(id)?;
        self.apply_embedded_profile(&package)
    }

    /// Validate an embedded profile identifier without reading or mutating a
    /// repository.
    pub fn validate_profile_id(&self, id: &str) -> Result<()> {
        embedded_profile(id).map(|_| ())
    }

    /// Apply one validated embedded profile package through the recovered session.
    ///
    /// Each attempt captures the whole-repository base under the held session guard,
    /// derives the exact profile-owned targets through the repository-state
    /// materialization dispatcher,
    /// finalizes one exact profile-application delta, validates that delta's proposed
    /// overlay, and publishes through `session.apply` with pre-journal revalidation.
    /// A no-op profile has an empty complete finalized delta: package targets and
    /// provenance are unchanged, and coupled default-rule/schema state is current.
    pub fn apply_embedded_profile(&self, package: &ProfilePackage) -> Result<ProfileApplyResult> {
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so the appended ProfileApplied event's id/timestamp stay stable.
        let context = MutationContext::production();
        with_mutation_session(self.storage(), &layout, "profile application", |session| {
            let Some((plan, _changes)) =
                self.prepare_embedded_profile(session, package, &context)?
            else {
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
    fn prepare_embedded_profile(
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
                ProfileTargetChange::new(repo_string(&target.path), action, target.mode)
            })
            .collect();
        Ok(Some((plan, changes)))
    }

    /// Read the installed provenance record through a recovered session capture.
    fn read_installed_record(
        &self,
        package: &ProfilePackage,
    ) -> Result<Option<AppliedProfileRecord>> {
        let metadata = &package.manifest().profile;
        let record_path = applied_record_path(metadata.id.as_str())?;
        let layout = self.require_layout()?;
        let budget = CaptureBudget {
            max_paths: 16,
            max_listings: 0,
            max_bytes: 4 * 1024 * 1024,
            max_depth: 4,
        };
        with_mutation_session(self.storage(), &layout, "profile record read", |session| {
            let Some(image) = capture_or_retry(
                session.capture(CaptureSpec::phase_one([record_path.clone()], budget)?),
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            Ok(SessionStep::Done(read_applied_record(
                &image,
                &record_path,
                metadata,
            )?))
        })
    }
}

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
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<ProfileOrigin> {
    let ProfilePackageSource::Directory(directory) = package.source() else {
        return Ok(ProfileOrigin::Embedded);
    };
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
    layout: &crate::repository_state::RepositoryLayout,
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
    layout: &crate::repository_state::RepositoryLayout,
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

/// Resolve one embedded profile package by stable id.
pub(super) fn embedded_profile(id: &str) -> Result<ProfilePackage> {
    let package = jit_dogfood_package()?;
    if package.manifest().profile.id.as_str() == id {
        Ok(package)
    } else {
        Err(crate::errors::NotFoundError::new(format!("Profile not found: {id}")).into())
    }
}

/// Repo-relative spelling of a canonical virtual path (`.jit/...` for Data).
pub(super) fn repo_string(path: &VirtualPath) -> String {
    let rel = match path.relative() {
        RootRelativePath::Root => String::new(),
        RootRelativePath::Descendant(text) => text.to_string(),
    };
    match path.root_class() {
        RepositoryRootClass::Data => format!(".jit/{rel}"),
        RepositoryRootClass::Worktree => rel,
    }
}

/// Read the captured installed record, or `None` when absent.
fn read_applied_record(
    base: &RepositoryImage,
    record_path: &VirtualPath,
    metadata: &crate::profile::ProfileMetadata,
) -> Result<Option<AppliedProfileRecord>> {
    match base.entry(record_path)? {
        RepositoryEntry::Absent => Ok(None),
        RepositoryEntry::File { bytes, .. } => {
            serde_json::from_slice::<AppliedProfileRecord>(bytes)
                .map(Some)
                .map_err(|_| {
                    ProfileApplyError::InstalledRecordConflict {
                        path: repo_string(record_path),
                        id: metadata.id.to_string(),
                        version: metadata.version.clone(),
                    }
                    .into()
                })
        }
        _ => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: repo_string(record_path),
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
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::repository_state::RootRelativePath;
    use crate::storage::{
        discover_repository_layout, IssueStore, RepositoryStateStore, RepositoryStateStoreError,
    };
    use include_dir::{include_dir, Dir};
    use std::collections::BTreeMap;
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

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-asset-only");

    /// A file-backed executor over a canonically initialized repository carrying
    /// its canonical layout.
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
            .initialize_fresh_repository(temp.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        let package = ProfilePackage::from_embedded_dir(&PACKAGE).unwrap();
        (temp, storage, executor, package)
    }

    /// The fixture package, written into the repository at `relative` and read
    /// back from there.
    fn package_read_from(temp: &TempDir, relative: &str) -> ProfilePackage {
        let root = crate::test_utils::write_package_tree(&PACKAGE, &temp.path().join(relative));
        ProfilePackage::from_directory(&root).expect("a valid package tree")
    }

    /// The provenance record a repository stores for the fixture package.
    fn stored_record(temp: &TempDir) -> AppliedProfileRecord {
        serde_json::from_slice(
            &fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn test_apply_embedded_profile_records_the_worktree_location_it_read_the_package_from() {
        let (temp, _storage, executor, _embedded) = fixture();
        let package = package_read_from(&temp, "vendor/profiles/planner");

        executor.apply_embedded_profile(&package).unwrap();

        // The stored location resolves, from the worktree root alone, back to
        // the directory whose bytes were applied — which is the whole point of
        // recording it.
        let ProfileOrigin::Directory(location) = stored_record(&temp).origin else {
            panic!("a package read from a directory must record that directory");
        };
        assert_eq!(
            ProfilePackage::from_directory(&temp.path().join(location.as_path()))
                .expect("the recorded location names a readable package")
                .hashes(),
            package.hashes()
        );
    }

    #[test]
    fn test_apply_embedded_profile_records_the_directory_read_rather_than_a_peer_copy() {
        // Two directories inside the worktree hold byte-identical packages.
        // Nothing but the directory a package was read through distinguishes
        // them, so the recorded location must follow that and only that.
        let (first_temp, _first_storage, first_executor, _embedded) = fixture();
        let (second_temp, _second_storage, second_executor, _embedded) = fixture();
        let first = package_read_from(&first_temp, "vendor/first");
        let second = package_read_from(&second_temp, "packages/second/tree");
        assert_eq!(first.hashes(), second.hashes());

        first_executor.apply_embedded_profile(&first).unwrap();
        second_executor.apply_embedded_profile(&second).unwrap();

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
    fn test_apply_embedded_profile_refuses_a_package_read_from_outside_the_worktree() {
        let (temp, storage, executor, _embedded) = fixture();
        let elsewhere = TempDir::new().unwrap();
        let root = crate::test_utils::write_package_tree(&PACKAGE, &elsewhere.path().join("pkg"));
        let package = ProfilePackage::from_directory(&root).expect("a valid package tree");

        let error = executor.apply_embedded_profile(&package).unwrap_err();

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
    fn test_apply_embedded_profile_refuses_a_package_read_from_the_data_root() {
        // The selected data root is not a worktree location, so a package
        // placed under it has no worktree-relative location to record.
        let (temp, storage, executor, _embedded) = fixture();
        let package = package_read_from(&temp, ".jit/vendored");

        let error = executor.apply_embedded_profile(&package).unwrap_err();

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

    #[test]
    fn test_profile_application_commits_targets_record_event_and_exact_no_op() {
        let (temp, storage, executor, package) = fixture();

        let applied = executor.apply_embedded_profile(&package).unwrap();
        assert_eq!(applied.status, ProfileApplicationStatus::Applied);
        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            package.source_bytes("assets/profile.txt").unwrap()
        );
        let record: AppliedProfileRecord = serde_json::from_slice(
            &fs::read(temp.path().join(".jit/profiles/planner-asset-only.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(record.id, "planner-asset-only");
        assert_eq!(record.origin, ProfileOrigin::Embedded);
        assert_eq!(storage.read_events().unwrap().len(), 1);

        let before = fs::read(temp.path().join(".jit/events.jsonl")).unwrap();
        let compact_record = serde_json::to_vec(&record).unwrap();
        fs::write(
            temp.path().join(".jit/profiles/planner-asset-only.json"),
            &compact_record,
        )
        .unwrap();
        let unchanged = executor.apply_embedded_profile(&package).unwrap();
        assert_eq!(unchanged.status, ProfileApplicationStatus::Unchanged);
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
            .prepare_embedded_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();
        let paths = plan
            .delta()
            .actions()
            .iter()
            .map(|action| repo_string(action.path()))
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
            .prepare_embedded_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();
        let second = executor
            .prepare_embedded_profile(session.as_mut(), &package, &context)
            .unwrap()
            .unwrap();

        assert_eq!(first.0.hash(), second.0.hash());
        assert_eq!(first.1, second.1);
    }

    #[test]
    fn test_profile_application_revalidates_locked_snapshot_before_any_write() {
        let (temp, storage, executor, package) = fixture();
        fs::write(temp.path().join(".jit/config.toml"), b"not = [valid").unwrap();

        assert!(executor.apply_embedded_profile(&package).is_err());
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

        let result =
            executor.prepare_embedded_profile(&mut session, &package, &MutationContext::preview());

        assert!(result.is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
    }

    #[test]
    fn test_profile_preparation_retries_when_final_proposed_closure_expands() {
        let (temp, _storage, executor, _package) = fixture();
        let package = jit_dogfood_package().unwrap();
        let layout = executor.require_layout().unwrap();
        let inner = executor.storage().open_mutation_session(layout).unwrap();
        let mut session = FinalClosureRaceSession {
            inner,
            captures: 0,
            config_path: temp.path().join(".jit/config.toml"),
            source_path: temp.path().join("RACE.md"),
        };

        let first = executor
            .prepare_embedded_profile(&mut session, &package, &MutationContext::preview())
            .unwrap();
        assert!(first.is_none(), "an expanded final closure must retry");

        let (plan, _) = executor
            .prepare_embedded_profile(&mut session, &package, &MutationContext::preview())
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

        executor.apply_embedded_profile(&package).unwrap();

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
    fn test_profile_application_does_not_mark_valid_unterminated_event_as_torn() {
        let (temp, storage, executor, package) = fixture();
        let prior_event = Event::draft_profile_applied(
            "prior".to_string(),
            "1.0.0".to_string(),
            ProfileOrigin::Embedded,
            "prior-package".to_string(),
            BTreeMap::new(),
            false,
        );
        fs::write(
            temp.path().join(".jit/events.jsonl"),
            serde_json::to_vec(&prior_event).unwrap(),
        )
        .unwrap();

        executor.apply_embedded_profile(&package).unwrap();

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
