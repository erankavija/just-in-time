use super::CommandExecutor;
use crate::domain::Event;
use crate::profile::{
    append_profile_event_image, jit_dogfood_package, plan_profile_application_against,
    AppliedProfileRecord, EmbeddedProfilePackage, PlannedTargetAction, ProfileApplicationPlan,
    ProfileApplicationStatus, ProfileApplyResult, ProfileListResult, ProfileOrigin,
    ProfilePlanResult, ProfilePlanStatus, ProfileShowResult, ProfileSummary, ProfileTargetAction,
    ProfileTargetChange, RepositorySnapshot, SnapshotEntry,
};
use crate::repository_state::{
    apply_overlay, finalize_profile_application, ProfileContribution, ProfileTargetContribution,
    VirtualPath,
};
use crate::storage::{
    IssueStore, JsonFileStorage, RepositoryStateStore, RepositoryStateStoreError,
};
use crate::validation::repository::{
    FilesystemRepositoryView, OverlayRepositoryView, RepositoryValidationFailure, RepositoryView,
};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::Arc;

struct PreparedProfileApplication {
    record: AppliedProfileRecord,
    record_matches: bool,
    snapshot: RepositorySnapshot,
    next_events: Vec<u8>,
    plan: ProfileApplicationPlan,
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
            .inspect_applied_record(&package)?
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
            applied: self.inspect_applied_record(&package)?,
        })
    }

    /// Build the exact non-mutating target plan for one embedded profile.
    pub fn plan_embedded_profile(&self, id: &str) -> Result<ProfilePlanResult> {
        let package = embedded_profile(id)?;
        let prepared = self.prepare_embedded_profile(&package)?;
        Ok(profile_plan_result(&package, &prepared))
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
    /// Each attempt rebuilds the plan inputs, captures the whole-repository base
    /// under the held session guard, finalizes one exact profile-application delta,
    /// validates that delta's proposed overlay, and publishes through
    /// `session.apply` with pre-journal revalidation. A no-op profile (plan no-op
    /// and provenance already recorded) publishes nothing.
    pub fn apply_embedded_profile(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<ProfileApplyResult> {
        let metadata = &package.manifest().profile;
        let layout = self.require_layout()?;
        let mut session = self.storage().open_mutation_session(layout)?;
        for _ in 0..8 {
            let prepared = self.prepare_embedded_profile(package)?;
            if prepared.plan.is_no_op() && prepared.record_matches {
                return Ok(ProfileApplyResult {
                    id: metadata.id.clone(),
                    version: metadata.version.clone(),
                    status: ProfileApplicationStatus::Unchanged,
                    plan_hash: prepared.plan.identity.plan_hash,
                    transaction_id: None,
                    warnings: Vec::new(),
                });
            }
            ensure_profile_directory(&prepared.snapshot)?;
            let contribution = self.profile_contribution(package, &prepared)?;

            // Probe capture yields a base good enough to finalize the exact delta;
            // that delta's overlay is the authoritative proposed state for both the
            // capture closure and validation, so a preserved file is never shadowed.
            let extra_paths = contribution.delta_paths()?;
            let probe_overrides = contribution.overlay_overrides()?;
            let probe = match self.capture_proposed_base(
                session.as_mut(),
                &probe_overrides,
                &extra_paths,
            )? {
                None => continue,
                Some(base) => base,
            };
            let delta_overlay = super::validation_overlay(
                finalize_profile_application(&probe, &contribution)?.delta(),
            );

            let base =
                match self.capture_proposed_base(session.as_mut(), &delta_overlay, &extra_paths)? {
                    None => continue,
                    Some(base) => base,
                };
            let plan = finalize_profile_application(&base, &contribution)?;
            let proposed = apply_overlay(&base, delta_overlay).map_err(anyhow::Error::from)?;
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
                        plan_hash: prepared.plan.identity.plan_hash,
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

    /// Convert one prepared profile application into its canonical contribution:
    /// changed asset targets, provenance record, appended audit log, and the
    /// directory requirement. Asset bytes and the `ProfileApplied` event still come
    /// from the transitional planner/helpers (increments 6/7).
    fn profile_contribution(
        &self,
        package: &EmbeddedProfilePackage<'_>,
        prepared: &PreparedProfileApplication,
    ) -> Result<ProfileContribution> {
        let metadata = &package.manifest().profile;
        let targets = prepared
            .plan
            .targets
            .values()
            .filter(|target| target.action != PlannedTargetAction::NoOp)
            .map(|target| {
                Ok(ProfileTargetContribution {
                    path: super::repo_rel_virtual_path(&target.path)?,
                    bytes: target.bytes.clone(),
                    mode: super::file_mode(target.mode),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ProfileContribution {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            package_hash: package.hashes().package.clone(),
            targets,
            record_path: VirtualPath::data(format!("profiles/{}.json", metadata.id))?,
            record_bytes: prepared.record.to_bytes()?,
            record_changed: !prepared.record_matches,
            events_bytes: prepared.next_events.clone(),
            // The apply path is reached only when the profile changed, and it always
            // appends exactly one `ProfileApplied` event to the captured prefix.
            events_changed: true,
            ensure_profiles_dir: prepared.snapshot.entry(".jit/profiles").is_none(),
        })
    }

    fn prepare_embedded_profile(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<PreparedProfileApplication> {
        let metadata = &package.manifest().profile;
        let record_path = format!(".jit/profiles/{}.json", metadata.id);
        reject_reserved_application_targets(package.hashes().targets.keys().map(String::as_str))?;
        let snapshot_paths = package.hashes().targets.keys().map(String::as_str).chain([
            record_path.as_str(),
            ".jit/profiles",
            ".jit/events.jsonl",
        ]);
        let snapshot = self.storage.capture_profile_snapshot(snapshot_paths)?;
        let record = AppliedProfileRecord {
            ..expected_record(package)
        };
        let record_matches = inspect_installed_record(&snapshot, &record_path, &record)?;
        let prior_events = snapshot
            .file(".jit/events.jsonl")
            .map_or(&[][..], |file| file.bytes.as_slice());
        let isolated_torn_tail = has_malformed_unterminated_event_tail(prior_events);
        let event = Event::new_profile_applied(
            metadata.id.clone(),
            metadata.version.clone(),
            ProfileOrigin::Embedded,
            package.hashes().package.clone(),
            package.hashes().targets.clone(),
            isolated_torn_tail,
        );
        let next_events = append_profile_event_image(prior_events, &event)?;
        let validation_base: Arc<dyn RepositoryView> = Arc::new(
            FilesystemRepositoryView::from_jit_root(self.storage.root())?,
        );
        let planning_validation_base: Arc<dyn RepositoryView> = if isolated_torn_tail {
            Arc::new(OverlayRepositoryView::new(
                validation_base.clone(),
                [(
                    PathBuf::from(".jit/events.jsonl"),
                    Some(next_events.clone()),
                )],
            )?)
        } else {
            validation_base
        };
        let plan = plan_profile_application_against(package, &snapshot, planning_validation_base)?;
        Ok(PreparedProfileApplication {
            record,
            record_matches,
            snapshot,
            next_events,
            plan,
        })
    }

    fn inspect_applied_record(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<Option<AppliedProfileRecord>> {
        let metadata = &package.manifest().profile;
        let record_path = format!(".jit/profiles/{}.json", metadata.id);
        let snapshot = self
            .storage
            .capture_profile_snapshot([record_path.as_str()])?;
        match snapshot.entry(&record_path) {
            None => Ok(None),
            Some(SnapshotEntry::File(file)) => {
                let record =
                    serde_json::from_slice::<AppliedProfileRecord>(&file.bytes).map_err(|_| {
                        ProfileApplyError::InstalledRecordConflict {
                            path: record_path.clone(),
                            id: metadata.id.clone(),
                            version: metadata.version.clone(),
                        }
                    })?;
                Ok(Some(record))
            }
            Some(_) => Err(ProfileApplyError::UnsupportedMetadataPath { path: record_path }.into()),
        }
    }
}

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

pub(super) fn embedded_profile(id: &str) -> Result<EmbeddedProfilePackage<'static>> {
    let package = jit_dogfood_package()?;
    if package.manifest().profile.id == id {
        Ok(package)
    } else {
        Err(crate::errors::NotFoundError::new(format!("Profile not found: {id}")).into())
    }
}

fn profile_plan_result(
    package: &EmbeddedProfilePackage<'_>,
    prepared: &PreparedProfileApplication,
) -> ProfilePlanResult {
    let targets = prepared
        .plan
        .targets
        .values()
        .map(|target| {
            let action = match target.action {
                PlannedTargetAction::Create => ProfileTargetAction::Create,
                PlannedTargetAction::Update => ProfileTargetAction::Update,
                PlannedTargetAction::NoOp => ProfileTargetAction::Unchanged,
            };
            ProfileTargetChange::new(target.path.clone(), action, target.mode)
        })
        .collect();
    ProfilePlanResult {
        id: package.manifest().profile.id.clone(),
        version: package.manifest().profile.version.clone(),
        status: if prepared.plan.is_no_op() && prepared.record_matches {
            ProfilePlanStatus::Unchanged
        } else {
            ProfilePlanStatus::WouldApply
        },
        plan_hash: prepared.plan.identity.plan_hash.clone(),
        targets,
    }
}

pub(super) fn has_malformed_unterminated_event_tail(events: &[u8]) -> bool {
    if events.is_empty() || events.ends_with(b"\n") {
        return false;
    }
    let final_line = events
        .rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap_or(events);
    serde_json::from_slice::<serde_json::Value>(final_line).is_err()
}

pub(super) fn inspect_installed_record(
    snapshot: &crate::profile::RepositorySnapshot,
    path: &str,
    expected: &AppliedProfileRecord,
) -> Result<bool> {
    match snapshot.entry(path) {
        None => Ok(false),
        Some(SnapshotEntry::File(file)) => {
            let parsed = serde_json::from_slice::<AppliedProfileRecord>(&file.bytes).ok();
            if parsed.as_ref() == Some(expected) {
                Ok(true)
            } else {
                Err(ProfileApplyError::InstalledRecordConflict {
                    path: path.to_string(),
                    id: expected.id.clone(),
                    version: expected.version.clone(),
                }
                .into())
            }
        }
        Some(_) => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: path.to_string(),
        }
        .into()),
    }
}

pub(super) fn ensure_profile_directory(
    snapshot: &crate::profile::RepositorySnapshot,
) -> Result<()> {
    match snapshot.entry(".jit/profiles") {
        None | Some(SnapshotEntry::Directory) => Ok(()),
        Some(_) => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: ".jit/profiles".to_string(),
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
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::storage::discover_repository_layout;
    use include_dir::{include_dir, Dir};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-asset-only");

    /// A file-backed executor over an initialized, config-seeded repository carrying
    /// its canonical layout.
    fn fixture() -> (
        TempDir,
        JsonFileStorage,
        CommandExecutor<JsonFileStorage>,
        EmbeddedProfilePackage<'static>,
    ) {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(temp.path(), storage.root()).unwrap());
        executor
            .seed_project_config(
                temp.path(),
                &HierarchyTemplate::default().generate_config_toml(),
            )
            .unwrap();
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
        let prior_event = Event::new_profile_applied(
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
