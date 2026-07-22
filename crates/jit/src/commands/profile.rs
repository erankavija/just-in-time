use super::CommandExecutor;
use crate::profile::{
    build_profile_claims, jit_dogfood_package, AppliedProfileRecord, EmbeddedProfilePackage,
    ProfileApplicationStatus, ProfileApplyResult, ProfileListResult, ProfileOrigin,
    ProfilePlanResult, ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction,
    ProfileTargetChange,
};
use crate::repository_state::{
    apply_overlay, derive_profile_materializations, finalize_profile_application, CaptureBudget,
    CaptureSpec, FileMode, MutationContext, ProfileContribution, ProfileTargetContribution,
    RepositoryEntry, RepositoryImage, RepositoryRootClass, RootRelativePath, VirtualPath,
};
use crate::storage::{
    JsonFileStorage, RepositoryMutationSession, RepositoryStateStore, RepositoryStateStoreError,
};
use crate::validation::repository::RepositoryValidationFailure;
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

/// A prepared profile application derived over a captured base image: the exact
/// contribution to publish, its deterministic plan identity, whether it is a
/// complete no-op, and the per-target changes for the dry-run preview.
struct PreparedProfileApplication {
    contribution: ProfileContribution,
    delta_overlay: BTreeMap<VirtualPath, Option<Vec<u8>>>,
    plan_hash: String,
    is_no_op: bool,
    changes: Vec<ProfileTargetChange>,
}

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
}

impl CommandExecutor<JsonFileStorage> {
    /// List the immutable profiles embedded in this binary.
    pub fn list_embedded_profiles(&self) -> Result<ProfileListResult> {
        let package = jit_dogfood_package()?;
        let metadata = &package.manifest().profile;
        let applied = self
            .read_installed_record(&package)?
            .is_some_and(|record| record == expected_record(&package));
        let profiles = vec![ProfileSummary {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            origin: ProfileOrigin::Embedded,
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
        Ok(ProfileShowResult {
            manifest: package.manifest().clone(),
            origin: ProfileOrigin::Embedded,
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
        let mut session = self.storage().open_mutation_session(layout)?;
        let context = MutationContext::production();
        for _ in 0..8 {
            let Some(prepared) =
                self.prepare_embedded_profile(session.as_mut(), &package, &context)?
            else {
                continue;
            };
            return Ok(ProfilePlanResult {
                id: metadata.id.clone(),
                version: metadata.version.clone(),
                status: if prepared.is_no_op {
                    ProfilePlanStatus::Unchanged
                } else {
                    ProfilePlanStatus::WouldApply
                },
                plan_hash: prepared.plan_hash,
                targets: prepared.changes,
            });
        }
        Err(anyhow!(
            "profile planning did not converge after repeated capture conflicts"
        ))
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
    /// derives the exact profile-owned targets through
    /// [`repository_state::derive_profile_materializations`](crate::repository_state::derive_profile_materializations),
    /// finalizes one exact profile-application delta, validates that delta's proposed
    /// overlay, and publishes through `session.apply` with pre-journal revalidation.
    /// A no-op profile has an empty complete finalized delta: package targets and
    /// provenance are unchanged, and coupled default-rule/schema state is current.
    pub fn apply_embedded_profile(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<ProfileApplyResult> {
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        let mut session = self.storage().open_mutation_session(layout)?;
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so the appended ProfileApplied event's id/timestamp stay stable.
        let context = MutationContext::production();
        for _ in 0..8 {
            let Some(prepared) =
                self.prepare_embedded_profile(session.as_mut(), package, &context)?
            else {
                continue;
            };
            if prepared.is_no_op {
                return Ok(ProfileApplyResult {
                    id: metadata.id.clone(),
                    version: metadata.version.clone(),
                    status: ProfileApplicationStatus::Unchanged,
                    plan_hash: prepared.plan_hash,
                    transaction_id: None,
                    warnings: Vec::new(),
                });
            }
            let contribution = &prepared.contribution;

            let extra_paths = contribution.delta_paths()?;
            let base = match self.capture_proposed_base(
                session.as_mut(),
                &prepared.delta_overlay,
                &extra_paths,
                None,
            )? {
                None => continue,
                Some(base) => base,
            };
            let plan = finalize_profile_application(&base, contribution, &context)?;
            let proposed = apply_overlay(&base, prepared.delta_overlay.clone())
                .map_err(anyhow::Error::from)?;
            let validation = crate::validation::repository::validate_repository(&proposed)
                .map_err(ProfileApplyError::from)?;
            if validation.rule_report.has_errors() {
                return Err(ProfileApplyError::FinalValidationFindings {
                    error_count: validation.rule_report.error_count(),
                }
                .into());
            }

            match session.apply(&plan) {
                Ok(outcome) => {
                    return Ok(ProfileApplyResult {
                        id: metadata.id.clone(),
                        version: metadata.version.clone(),
                        status: ProfileApplicationStatus::Applied,
                        plan_hash: prepared.plan_hash,
                        transaction_id: Some(outcome.transaction_hash),
                        warnings: Vec::new(),
                    });
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "profile application did not converge after repeated capture conflicts"
        ))
    }

    /// Capture the base, derive the profile's canonical targets, and assemble one
    /// [`ProfileContribution`] plus its dry-run preview.
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
        package: &EmbeddedProfilePackage<'_>,
        context: &MutationContext,
    ) -> Result<Option<PreparedProfileApplication>> {
        let metadata = &package.manifest().profile;
        reject_reserved_application_targets(package.hashes().targets.keys().map(String::as_str))?;
        let record_path = VirtualPath::data(format!("profiles/{}.json", metadata.id))?;
        let profiles_dir = VirtualPath::data("profiles")?;
        let events_path = VirtualPath::data("events.jsonl")?;

        let mut content_paths = package
            .hashes()
            .targets
            .keys()
            .map(|target| super::repo_rel_virtual_path(target))
            .collect::<Result<Vec<_>>>()?;
        content_paths.push(record_path.clone());
        content_paths.push(profiles_dir.clone());
        content_paths.push(events_path.clone());

        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &content_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };

        let claims = build_profile_claims(package, &base)?;
        let derived = derive_profile_materializations(&base, claims)?;

        let record = expected_record(package);
        let record_matches = record_matches_in_image(&base, &record_path, &record)?;
        let ensure_profiles_dir = profile_dir_needs_creation(&base, &profiles_dir)?;

        let mut targets = Vec::new();
        let mut changes = Vec::new();
        let mut all_unchanged = true;
        for (path, (bytes, mode)) in &derived {
            let action = target_action(&base, path, bytes, *mode)?;
            if action != ProfileTargetAction::Unchanged {
                all_unchanged = false;
                targets.push(ProfileTargetContribution {
                    path: path.clone(),
                    bytes: bytes.clone(),
                    mode: *mode,
                });
            }
            changes.push(ProfileTargetChange::new(repo_string(path), action, *mode));
        }
        let direct_profile_changed = !all_unchanged || !record_matches;

        // This signal covers direct package/provenance changes for profiled init.
        // Standalone finalization audits any non-empty complete delta, including
        // coupled default-rule/schema repair discovered after this preparation.
        let contribution = ProfileContribution {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
            targets,
            record_path,
            record_bytes: record.to_bytes()?,
            record_changed: !record_matches,
            emit_event: direct_profile_changed,
            ensure_profiles_dir,
        };
        let extra_paths = contribution.delta_paths()?;
        let overrides = contribution.overlay_overrides()?;
        let probe = match self.capture_proposed_base(session, &overrides, &extra_paths, None)? {
            None => return Ok(None),
            Some(base) => base,
        };
        let preview = finalize_profile_application(&probe, &contribution, context)?;
        let is_no_op = preview.delta().actions().is_empty();
        let delta_overlay = super::validation_overlay(preview.delta());
        Ok(Some(PreparedProfileApplication {
            contribution,
            delta_overlay,
            plan_hash: profile_plan_hash(&derived),
            is_no_op,
            changes,
        }))
    }

    /// Read the installed provenance record through a recovered session capture.
    fn read_installed_record(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<Option<AppliedProfileRecord>> {
        let metadata = &package.manifest().profile;
        let record_path = VirtualPath::data(format!("profiles/{}.json", metadata.id))?;
        let layout = self.require_layout()?;
        let mut session = self.storage().open_mutation_session(layout)?;
        let budget = CaptureBudget {
            max_paths: 16,
            max_listings: 0,
            max_bytes: 4 * 1024 * 1024,
            max_depth: 4,
        };
        for _ in 0..8 {
            match session.capture(CaptureSpec::phase_one([record_path.clone()], budget)?) {
                Ok(image) => return read_applied_record(&image, &record_path, metadata),
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "profile record read did not converge after repeated capture conflicts"
        ))
    }
}

/// The expected provenance record for an embedded package.
pub(super) fn expected_record(package: &EmbeddedProfilePackage<'_>) -> AppliedProfileRecord {
    let metadata = &package.manifest().profile;
    AppliedProfileRecord {
        id: metadata.id.clone(),
        version: metadata.version.clone(),
        origin: ProfileOrigin::Embedded,
        package_hash: package.hashes().package.clone(),
        target_hashes: package.hashes().targets.clone(),
    }
}

/// Resolve one embedded profile package by stable id.
pub(super) fn embedded_profile(id: &str) -> Result<EmbeddedProfilePackage<'static>> {
    let package = jit_dogfood_package()?;
    if package.manifest().profile.id == id {
        Ok(package)
    } else {
        Err(crate::errors::NotFoundError::new(format!("Profile not found: {id}")).into())
    }
}

/// Repo-relative spelling of a canonical virtual path (`.jit/...` for Data).
fn repo_string(path: &VirtualPath) -> String {
    let rel = match path.relative() {
        RootRelativePath::Root => String::new(),
        RootRelativePath::Descendant(text) => text.clone(),
    };
    match path.root_class() {
        RepositoryRootClass::Data => format!(".jit/{rel}"),
        RepositoryRootClass::Worktree => rel,
    }
}

/// Classify one derived target against the captured base.
pub(super) fn target_action(
    base: &RepositoryImage,
    path: &VirtualPath,
    bytes: &[u8],
    mode: FileMode,
) -> Result<ProfileTargetAction> {
    Ok(match base.entry(path)? {
        RepositoryEntry::Absent => ProfileTargetAction::Create,
        RepositoryEntry::File {
            bytes: existing,
            mode: existing_mode,
            ..
        } if existing.as_slice() == bytes && *existing_mode == mode => {
            ProfileTargetAction::Unchanged
        }
        _ => ProfileTargetAction::Update,
    })
}

/// Deterministic plan identity over every derived target (path, bytes, mode). No
/// test pins its value; the schema only requires a `plan_hash` key.
pub(super) fn profile_plan_hash(derived: &BTreeMap<VirtualPath, (Vec<u8>, FileMode)>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"jit-profile-plan-v2\0");
    for (path, (bytes, mode)) in derived {
        let repo = repo_string(path);
        hasher.update((repo.len() as u64).to_be_bytes());
        hasher.update(repo.as_bytes());
        hasher.update(format!("{mode:?}").as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// Whether the captured installed record exactly matches `expected`.
///
/// Absent is a non-match; a parseable record that differs, or an unsupported
/// occupant, is a typed conflict aborting before any write.
pub(super) fn record_matches_in_image(
    base: &RepositoryImage,
    record_path: &VirtualPath,
    expected: &AppliedProfileRecord,
) -> Result<bool> {
    match base.entry(record_path)? {
        RepositoryEntry::Absent => Ok(false),
        RepositoryEntry::File { bytes, .. } => {
            match serde_json::from_slice::<AppliedProfileRecord>(bytes).ok() {
                Some(record) if &record == expected => Ok(true),
                _ => Err(ProfileApplyError::InstalledRecordConflict {
                    path: repo_string(record_path),
                    id: expected.id.clone(),
                    version: expected.version.clone(),
                }
                .into()),
            }
        }
        _ => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: repo_string(record_path),
        }
        .into()),
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
                        id: metadata.id.clone(),
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

/// Whether the `.jit/profiles` directory must be created, rejecting an unsupported
/// occupant.
pub(super) fn profile_dir_needs_creation(
    base: &RepositoryImage,
    profiles_dir: &VirtualPath,
) -> Result<bool> {
    match base.entry(profiles_dir)? {
        RepositoryEntry::Absent => Ok(true),
        RepositoryEntry::Directory { .. } => Ok(false),
        _ => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: repo_string(profiles_dir),
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
    use crate::storage::{discover_repository_layout, IssueStore};
    use include_dir::{include_dir, Dir};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-asset-only");

    /// A file-backed executor over a canonically initialized repository carrying
    /// its canonical layout.
    fn fixture() -> (
        TempDir,
        JsonFileStorage,
        CommandExecutor<JsonFileStorage>,
        EmbeddedProfilePackage<'static>,
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
        let package = EmbeddedProfilePackage::from_dir(&PACKAGE).unwrap();
        (temp, storage, executor, package)
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
    fn test_profile_application_revalidates_locked_snapshot_before_any_write() {
        let (temp, storage, executor, package) = fixture();
        fs::write(temp.path().join(".jit/config.toml"), b"not = [valid").unwrap();

        assert!(executor.apply_embedded_profile(&package).is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
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
