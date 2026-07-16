use super::CommandExecutor;
use crate::domain::Event;
use crate::profile::{
    append_profile_event_image, plan_profile_application_against, AppliedProfileRecord,
    EmbeddedProfilePackage, PlannedTargetAction, ProfileApplicationStatus,
    ProfileApplicationWarning, ProfileApplyResult, ProfileOrigin, ProjectedFileMode, SnapshotEntry,
};
use crate::storage::{
    FileTransactionKernel, FileTransactionPlan, IssueStore, JsonFileStorage, RecoveryRequiredError,
    RecoveryState, TransactionAction,
};
use crate::validation::repository::{
    validate_repository, FilesystemRepositoryView, OverlayRepositoryView,
    RepositoryValidationFailure,
};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

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
    /// Apply one validated embedded profile package under a single write lock.
    pub fn apply_embedded_profile(
        &self,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<ProfileApplyResult> {
        let kernel = FileTransactionKernel::new(self.storage.open_repository_capability()?)?;
        self.apply_embedded_profile_with_kernel(package, &kernel)
    }

    /// Test seam for deterministic transaction interruption and cleanup failure.
    #[doc(hidden)]
    pub fn apply_embedded_profile_with_kernel(
        &self,
        package: &EmbeddedProfilePackage<'_>,
        kernel: &FileTransactionKernel,
    ) -> Result<ProfileApplyResult> {
        let repo_guard = self.storage.acquire_repo_write_lock()?;
        let _events_guard = self.storage.acquire_events_write_lock()?;

        // Rebuild all plan inputs after serialization. Nothing computed before
        // this boundary is trusted for publication.
        let metadata = &package.manifest().profile;
        let record_path = format!(".jit/profiles/{}.json", metadata.id);
        reject_reserved_application_targets(package.hashes().targets.keys().map(String::as_str))?;
        let snapshot_paths = package.hashes().targets.keys().map(String::as_str).chain([
            record_path.as_str(),
            ".jit/profiles",
            ".jit/events.jsonl",
        ]);
        let snapshot = self.storage.capture_profile_snapshot(snapshot_paths)?;
        let validation_base = Arc::new(FilesystemRepositoryView::from_jit_root(
            self.storage.root(),
        )?);
        let plan = plan_profile_application_against(package, &snapshot, validation_base.clone())?;
        let record = AppliedProfileRecord {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            origin: ProfileOrigin::Embedded,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
        };
        let record_matches = inspect_installed_record(&snapshot, &record_path, &record)?;

        if plan.is_no_op() && record_matches {
            return Ok(ProfileApplyResult {
                id: metadata.id.clone(),
                version: metadata.version.clone(),
                status: ProfileApplicationStatus::Unchanged,
                plan_hash: plan.identity.plan_hash,
                transaction_id: None,
                warnings: Vec::new(),
            });
        }

        ensure_profile_directory(&snapshot)?;
        let event = Event::new_profile_applied(
            metadata.id.clone(),
            metadata.version.clone(),
            ProfileOrigin::Embedded,
            package.hashes().package.clone(),
            package.hashes().targets.clone(),
        );
        let prior_events = snapshot
            .file(".jit/events.jsonl")
            .map_or(&[][..], |file| file.bytes.as_slice());
        let next_events = append_profile_event_image(prior_events, &event)?;
        let record_bytes = record.to_bytes()?;

        let mut overlay = plan.overlay_changes();
        overlay.insert(PathBuf::from(&record_path), Some(record_bytes.clone()));
        overlay.insert(
            PathBuf::from(".jit/events.jsonl"),
            Some(next_events.clone()),
        );
        let final_view = OverlayRepositoryView::new(validation_base, overlay)?;
        let validation = validate_repository(&final_view).map_err(ProfileApplyError::from)?;
        if validation.rule_report.has_errors() {
            return Err(ProfileApplyError::FinalValidationFindings {
                error_count: validation.rule_report.error_count(),
            }
            .into());
        }

        let mut actions = plan
            .targets
            .values()
            .filter(|target| target.action != PlannedTargetAction::NoOp)
            .map(|target| TransactionAction::WriteFile {
                path: transaction_path(self.storage.root(), &target.path),
                contents: target.bytes.clone(),
                unix_mode: unix_mode(target.mode),
            })
            .collect::<Vec<_>>();
        if snapshot.entry(".jit/profiles").is_none() {
            actions.push(TransactionAction::CreateDirectory {
                path: transaction_path(self.storage.root(), ".jit/profiles"),
                unix_mode: Some(0o755),
            });
        }
        if !record_matches {
            actions.push(TransactionAction::WriteFile {
                path: transaction_path(self.storage.root(), &record_path),
                contents: record_bytes,
                unix_mode: Some(0o644),
            });
        }
        actions.push(TransactionAction::WriteFile {
            path: transaction_path(self.storage.root(), ".jit/events.jsonl"),
            contents: next_events,
            unix_mode: Some(0o644),
        });

        let transaction_id = format!("profile-{}-{}", metadata.id, Uuid::new_v4());
        let outcome = kernel.execute(
            &repo_guard,
            FileTransactionPlan {
                transaction_id: transaction_id.clone(),
                actions,
            },
        );
        let warnings = match outcome {
            Ok(_) => Vec::new(),
            Err(error) => {
                let Some(recovery) = error.downcast_ref::<RecoveryRequiredError>() else {
                    return Err(error);
                };
                if recovery.state != RecoveryState::Committed {
                    return Err(error);
                }
                vec![ProfileApplicationWarning::TransactionCleanupPending {
                    transaction_id: transaction_id.clone(),
                    reason: format!("{:#}", recovery.source),
                }]
            }
        };

        Ok(ProfileApplyResult {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            status: ProfileApplicationStatus::Applied,
            plan_hash: plan.identity.plan_hash,
            transaction_id: Some(transaction_id),
            warnings,
        })
    }
}

fn inspect_installed_record(
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

fn ensure_profile_directory(snapshot: &crate::profile::RepositorySnapshot) -> Result<()> {
    match snapshot.entry(".jit/profiles") {
        None | Some(SnapshotEntry::Directory) => Ok(()),
        Some(_) => Err(ProfileApplyError::UnsupportedMetadataPath {
            path: ".jit/profiles".to_string(),
        }
        .into()),
    }
}

fn reject_reserved_application_targets<'a>(
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

fn unix_mode(mode: ProjectedFileMode) -> Option<u32> {
    Some(match mode {
        ProjectedFileMode::Regular => 0o644,
        ProjectedFileMode::Executable => 0o755,
    })
}

fn transaction_path(storage_root: &Path, virtual_path: &str) -> String {
    let storage_name = storage_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".jit");
    virtual_path.strip_prefix(".jit").map_or_else(
        || virtual_path.to_string(),
        |suffix| format!("{storage_name}{suffix}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::storage::{
        RecoveryCoordinator, TransactionFailureInjector, TransactionFailurePoint,
    };
    use include_dir::{include_dir, Dir};
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::sync::Arc;
    use tempfile::TempDir;

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-asset-only");

    struct SelectedFailures(HashSet<TransactionFailurePoint>);

    impl TransactionFailureInjector for SelectedFailures {
        fn check(&self, point: &TransactionFailurePoint) -> io::Result<()> {
            if self.0.contains(point) {
                Err(io::Error::other(format!("injected {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    fn fixture() -> (TempDir, JsonFileStorage, EmbeddedProfilePackage<'static>) {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        CommandExecutor::new(storage.clone())
            .seed_project_config(
                temp.path(),
                &HierarchyTemplate::default().generate_config_toml(),
            )
            .unwrap();
        let package = EmbeddedProfilePackage::from_dir(&PACKAGE).unwrap();
        (temp, storage, package)
    }

    fn kernel(
        storage: &JsonFileStorage,
        failures: impl IntoIterator<Item = TransactionFailurePoint>,
    ) -> FileTransactionKernel {
        FileTransactionKernel::with_injector(
            storage.open_repository_capability().unwrap(),
            Arc::new(SelectedFailures(failures.into_iter().collect())),
        )
        .unwrap()
    }

    #[test]
    fn test_profile_application_commits_targets_record_event_and_exact_no_op() {
        let (temp, storage, package) = fixture();
        let executor = CommandExecutor::new(storage.clone());

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
    fn test_profile_application_rolls_back_handled_publication_failure() {
        let (temp, storage, package) = fixture();
        let executor = CommandExecutor::new(storage.clone());
        let kernel = kernel(
            &storage,
            [TransactionFailurePoint::AfterPublish { action: 1 }],
        );

        assert!(executor
            .apply_embedded_profile_with_kernel(&package, &kernel)
            .is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp
            .path()
            .join(".jit/profiles/planner-asset-only.json")
            .exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_application_reports_committed_cleanup_and_recovery_cleans_it() {
        let (temp, storage, package) = fixture();
        let executor = CommandExecutor::new(storage.clone());
        let kernel = kernel(&storage, [TransactionFailurePoint::CleanupTerminalResidue]);

        let applied = executor
            .apply_embedded_profile_with_kernel(&package, &kernel)
            .unwrap();
        assert_eq!(applied.status, ProfileApplicationStatus::Applied);
        assert_eq!(applied.warnings.len(), 1);
        assert!(temp.path().join("docs/profile.txt").exists());
        drop(executor);

        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(session.report().recovered_count(), 1);
        drop(session);
        assert_eq!(storage.read_events().unwrap().len(), 1);
    }

    #[test]
    fn test_profile_application_prepared_interruption_recovers_all_old_state() {
        let (temp, storage, package) = fixture();
        let executor = CommandExecutor::new(storage.clone());
        let kernel = kernel(
            &storage,
            [
                TransactionFailurePoint::AfterPublish { action: 1 },
                TransactionFailurePoint::ReverseAction { action: 1 },
            ],
        );

        let error = executor
            .apply_embedded_profile_with_kernel(&package, &kernel)
            .unwrap_err();
        let recovery = error.downcast_ref::<RecoveryRequiredError>().unwrap();
        assert_eq!(recovery.state, RecoveryState::Prepared);
        drop(executor);

        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(session.report().recovered_count(), 1);
        drop(session);
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp
            .path()
            .join(".jit/profiles/planner-asset-only.json")
            .exists());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_profile_application_revalidates_locked_snapshot_before_any_write() {
        let (temp, storage, package) = fixture();
        fs::write(temp.path().join(".jit/config.toml"), b"not = [valid").unwrap();
        let executor = CommandExecutor::new(storage.clone());

        assert!(executor.apply_embedded_profile(&package).is_err());
        assert!(!temp.path().join("docs/profile.txt").exists());
        assert!(!temp.path().join(".jit/profiles").exists());
        assert!(storage.read_events().unwrap().is_empty());
    }
}
