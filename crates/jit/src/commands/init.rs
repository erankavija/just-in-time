use super::CommandExecutor;
use crate::config::{slugify_project_name, JitConfig, ProjectName};
use crate::config_manager::ConfigManager;
use crate::domain::Event;
use crate::hierarchy_templates::HierarchyTemplate;
use crate::profile::{
    append_profile_event_image, plan_profile_application_against, AppliedProfileRecord,
    EmbeddedProfilePackage, ProfileApplicationStatus, ProfileApplicationWarning,
    ProfileApplyResult, ProfileOrigin, ProjectedFileMode, RepositorySnapshot, SnapshotEntry,
    SnapshotFile,
};
use crate::storage::{
    FileTransactionKernel, FileTransactionPlan, GateRegistry, IssueStore, JsonFileStorage,
    RecoveryCoordinator, RecoveryRequiredError, RecoveryState, TransactionAction,
};
use crate::validation::repository::{
    validate_repository, FilesystemRepositoryView, OverlayRepositoryView, RepositoryView,
};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// Result of publishing a fresh repository scaffold.
#[derive(Debug)]
pub struct FreshInitResult {
    /// Canonical project identity derived from the repository directory.
    pub project_name: ProjectName,
    /// Applied profile result when initialization included one.
    pub profile: Option<ProfileApplyResult>,
    /// Non-fatal transaction cleanup diagnostics.
    pub warnings: Vec<String>,
}

/// Pure byte image shared by ordinary and profiled fresh initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitScaffold {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeSet<String>,
    project_name: ProjectName,
}

impl InitScaffold {
    /// Generate every neutral repository byte without touching the filesystem.
    pub(crate) fn generate(repo_dir: &Path, template: &HierarchyTemplate) -> Result<Self> {
        let basename = repo_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let project_name: ProjectName = slugify_project_name(basename).parse()?;
        let config = crate::storage::config_store::render_repo_config(
            &template.generate_config_toml(),
            &project_name,
        );
        let parsed: JitConfig =
            toml::from_str(&config).context("Failed to parse generated init configuration")?;
        let namespaces = ConfigManager::new(repo_dir.join(".jit")).namespaces_from_config(&parsed);
        let rules = crate::validation::serialize::serialize_ruleset(
            &crate::validation::defaults::default_ruleset(&namespaces),
        );

        let mut files = BTreeMap::from([
            (
                ".jit/index.json".to_string(),
                crate::storage::json::fresh_index_bytes()?,
            ),
            (
                ".jit/gates.toml".to_string(),
                crate::storage::gate_store::serialize_gate_registry(&GateRegistry::default())?
                    .into_bytes(),
            ),
            (".jit/events.jsonl".to_string(), Vec::new()),
            (".jit/config.toml".to_string(), config.into_bytes()),
            (".jit/rules.toml".to_string(), rules.rules_toml.into_bytes()),
        ]);
        files.extend(rules.schema_files.into_iter().map(|schema| {
            (
                format!(".jit/schemas/{}", schema.name),
                schema.content.into_bytes(),
            )
        }));

        Ok(Self {
            files,
            directories: BTreeSet::from([".jit/issues".to_string()]),
            project_name,
        })
    }

    fn overlay(&self) -> impl Iterator<Item = (PathBuf, Option<Vec<u8>>)> + '_ {
        self.files
            .iter()
            .map(|(path, bytes)| (PathBuf::from(path), Some(bytes.clone())))
    }
}

struct FreshProfilePlan {
    apply_result: ProfileApplyResult,
    changed_files: BTreeMap<String, (Vec<u8>, ProjectedFileMode)>,
}

impl CommandExecutor<JsonFileStorage> {
    /// Publish a fresh neutral or profiled repository in one recoverable
    /// fresh-root transaction.
    ///
    /// The caller must use this only while the selected storage root is absent.
    /// No plain mutating initialization is invoked before profile planning:
    /// neutral bytes are generated in memory, profile bytes are merged and
    /// validated over that image, and the complete result is published once.
    pub fn initialize_fresh_repository(
        &self,
        repo_dir: &Path,
        template: &HierarchyTemplate,
        profile_id: Option<&str>,
    ) -> Result<FreshInitResult> {
        let _recovery_session = RecoveryCoordinator::recover_before_services(&self.storage)?;
        if self.storage.root().exists() {
            anyhow::bail!(
                "another initializer published the data directory: {}",
                self.storage.root().display()
            );
        }
        let kernel = FileTransactionKernel::new(self.storage.open_repository_capability()?)?;
        self.initialize_fresh_repository_with_kernel(repo_dir, template, profile_id, &kernel)
    }

    fn initialize_fresh_repository_with_kernel(
        &self,
        repo_dir: &Path,
        template: &HierarchyTemplate,
        profile_id: Option<&str>,
        kernel: &FileTransactionKernel,
    ) -> Result<FreshInitResult> {
        let bootstrap_guard = self.storage.acquire_bootstrap_write_lock()?;
        let scaffold = InitScaffold::generate(repo_dir, template)?;

        let validation_base: Arc<dyn RepositoryView> = Arc::new(
            FilesystemRepositoryView::from_jit_root(self.storage.root())?,
        );
        let neutral_view = OverlayRepositoryView::new(validation_base.clone(), scaffold.overlay())?;
        let neutral_validation = validate_repository(&neutral_view)?;
        if neutral_validation.rule_report.has_errors() {
            anyhow::bail!(
                "fresh repository scaffold produced {} validation error finding(s)",
                neutral_validation.rule_report.error_count()
            );
        }

        let profile = profile_id
            .map(|id| {
                let package = super::profile::embedded_profile(id)?;
                self.prepare_fresh_profile(&scaffold, validation_base.clone(), &package)
            })
            .transpose()?;

        let mut files = scaffold
            .files
            .iter()
            .map(|(path, bytes)| (path.clone(), (bytes.clone(), ProjectedFileMode::Regular)))
            .collect::<BTreeMap<_, _>>();
        if let Some(profile) = &profile {
            files.extend(profile.changed_files.clone());
        }

        let mut actions = scaffold
            .directories
            .iter()
            .map(|path| TransactionAction::CreateDirectory {
                path: super::profile::transaction_path(self.storage.root(), path),
                unix_mode: Some(0o755),
            })
            .collect::<Vec<_>>();
        if profile.is_some() {
            actions.push(TransactionAction::CreateDirectory {
                path: super::profile::transaction_path(self.storage.root(), ".jit/profiles"),
                unix_mode: Some(0o755),
            });
        }
        actions.extend(files.into_iter().map(|(path, (contents, mode))| {
            TransactionAction::WriteFile {
                path: super::profile::transaction_path(self.storage.root(), &path),
                contents,
                unix_mode: super::profile::unix_mode(mode),
            }
        }));

        let transaction_id = format!("init-{}", Uuid::new_v4());
        let outcome = kernel.execute(
            &bootstrap_guard,
            FileTransactionPlan {
                transaction_id: transaction_id.clone(),
                actions,
            },
        );
        let mut warnings = Vec::new();
        match outcome {
            Ok(_) => {}
            Err(error) => {
                let Some(recovery) = error.downcast_ref::<RecoveryRequiredError>() else {
                    return Err(error);
                };
                if recovery.state != RecoveryState::Committed {
                    return Err(error);
                }
                warnings.push(format!(
                    "transaction {transaction_id} committed but cleanup is pending: {:#}",
                    recovery.source
                ));
            }
        }

        let profile = profile.map(|mut profile| {
            if !warnings.is_empty() {
                profile.apply_result.warnings.push(
                    ProfileApplicationWarning::TransactionCleanupPending {
                        transaction_id: transaction_id.clone(),
                        reason: warnings.join("; "),
                    },
                );
            }
            profile.apply_result.transaction_id = Some(transaction_id);
            profile.apply_result
        });

        Ok(FreshInitResult {
            project_name: scaffold.project_name,
            profile,
            warnings,
        })
    }

    fn prepare_fresh_profile(
        &self,
        scaffold: &InitScaffold,
        filesystem: Arc<dyn RepositoryView>,
        package: &EmbeddedProfilePackage<'_>,
    ) -> Result<FreshProfilePlan> {
        super::profile::reject_reserved_application_targets(
            package.hashes().targets.keys().map(String::as_str),
        )?;
        let metadata = &package.manifest().profile;
        let record_path = format!(".jit/profiles/{}.json", metadata.id);
        let captured = self.storage.capture_profile_snapshot(
            package
                .hashes()
                .targets
                .keys()
                .map(String::as_str)
                .chain([record_path.as_str(), ".jit/profiles"]),
        )?;
        let mut entries = captured.entries().clone();
        entries.extend(
            scaffold
                .directories
                .iter()
                .cloned()
                .map(|path| (PathBuf::from(path), SnapshotEntry::Directory)),
        );
        entries.extend(scaffold.files.iter().map(|(path, bytes)| {
            (
                PathBuf::from(path),
                SnapshotEntry::File(SnapshotFile {
                    bytes: bytes.clone(),
                    mode: ProjectedFileMode::Regular,
                }),
            )
        }));
        let snapshot = RepositorySnapshot::new(captured.root(), entries)?;
        let neutral_view: Arc<dyn RepositoryView> =
            Arc::new(OverlayRepositoryView::new(filesystem, scaffold.overlay())?);
        let plan = plan_profile_application_against(package, &snapshot, neutral_view)?;

        let record = AppliedProfileRecord {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            origin: ProfileOrigin::Embedded,
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
        };
        let event = Event::new_profile_applied(
            metadata.id.clone(),
            metadata.version.clone(),
            ProfileOrigin::Embedded,
            package.hashes().package.clone(),
            package.hashes().targets.clone(),
            false,
        );
        let events = append_profile_event_image(&[], &event)?;

        let mut final_overlay = scaffold.overlay().collect::<BTreeMap<_, _>>();
        final_overlay.extend(plan.overlay_changes());
        final_overlay.insert(PathBuf::from(&record_path), Some(record.to_bytes()?));
        final_overlay.insert(PathBuf::from(".jit/events.jsonl"), Some(events.clone()));
        let final_view = OverlayRepositoryView::new(
            Arc::new(FilesystemRepositoryView::from_jit_root(
                self.storage.root(),
            )?),
            final_overlay.clone(),
        )?;
        let validation = validate_repository(&final_view)?;
        if validation.rule_report.has_errors() {
            anyhow::bail!(
                "fresh profiled repository produced {} validation error finding(s)",
                validation.rule_report.error_count()
            );
        }

        let mut changed_files = plan
            .targets
            .values()
            .filter(|target| target.action != crate::profile::PlannedTargetAction::NoOp)
            .map(|target| (target.path.clone(), (target.bytes.clone(), target.mode)))
            .collect::<BTreeMap<_, _>>();
        changed_files.insert(
            record_path,
            (record.to_bytes()?, ProjectedFileMode::Regular),
        );
        changed_files.insert(
            ".jit/events.jsonl".to_string(),
            (events, ProjectedFileMode::Regular),
        );

        Ok(FreshProfilePlan {
            apply_result: ProfileApplyResult {
                id: metadata.id.clone(),
                version: metadata.version.clone(),
                status: ProfileApplicationStatus::Applied,
                plan_hash: plan.identity.plan_hash,
                transaction_id: None,
                warnings: Vec::new(),
            },
            changed_files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{IssueStore, TransactionFailureInjector, TransactionFailurePoint};
    use std::fs;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    struct FirstPublishFailure {
        publish_pending: AtomicBool,
        reverse_pending: AtomicBool,
    }

    struct TerminalCleanupFailure(AtomicBool);

    impl TransactionFailureInjector for TerminalCleanupFailure {
        fn check(&self, point: &TransactionFailurePoint) -> io::Result<()> {
            if matches!(point, TransactionFailurePoint::CleanupTerminalResidue)
                && self.0.swap(false, Ordering::SeqCst)
            {
                Err(io::Error::other("injected terminal cleanup residue"))
            } else {
                Ok(())
            }
        }
    }

    impl FirstPublishFailure {
        fn new(fail_reverse: bool) -> Self {
            Self {
                publish_pending: AtomicBool::new(true),
                reverse_pending: AtomicBool::new(fail_reverse),
            }
        }
    }

    impl TransactionFailureInjector for FirstPublishFailure {
        fn check(&self, point: &TransactionFailurePoint) -> io::Result<()> {
            let fail = match point {
                TransactionFailurePoint::AfterPublish { .. } => {
                    self.publish_pending.swap(false, Ordering::SeqCst)
                }
                TransactionFailurePoint::ReverseAction { .. } => {
                    self.reverse_pending.swap(false, Ordering::SeqCst)
                }
                _ => false,
            };
            if fail {
                Err(io::Error::other(format!("injected {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    fn first_publish_kernel(
        storage: &JsonFileStorage,
        fail_reverse: bool,
    ) -> FileTransactionKernel {
        FileTransactionKernel::with_injector(
            storage.open_repository_capability().unwrap(),
            Arc::new(FirstPublishFailure::new(fail_reverse)),
        )
        .unwrap()
    }

    fn terminal_cleanup_kernel(storage: &JsonFileStorage) -> FileTransactionKernel {
        FileTransactionKernel::with_injector(
            storage.open_repository_capability().unwrap(),
            Arc::new(TerminalCleanupFailure(AtomicBool::new(true))),
        )
        .unwrap()
    }

    #[test]
    fn test_pure_scaffold_matches_established_plain_init_bytes() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = CommandExecutor::new(storage.clone());
        let template = HierarchyTemplate::default();
        let scaffold = InitScaffold::generate(repo.path(), &template).unwrap();

        storage.init().unwrap();
        executor
            .seed_project_config(repo.path(), &template.generate_config_toml())
            .unwrap();
        executor.scaffold_default_rules().unwrap();

        for (path, expected) in scaffold.files {
            assert_eq!(
                fs::read(repo.path().join(path)).unwrap(),
                expected,
                "plain init byte contract drifted"
            );
        }
    }

    #[test]
    fn test_fresh_profile_init_publishes_complete_valid_repo_without_git() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = CommandExecutor::new(storage.clone());

        let result = executor
            .initialize_fresh_repository(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
            )
            .unwrap();

        assert_eq!(
            result.profile.unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert!(repo.path().join(".jit/index.json").is_file());
        assert!(repo.path().join(".jit/profiles/jit-dogfood.json").is_file());
        assert!(repo
            .path()
            .join(".agents/skills/jit-manage/SKILL.md")
            .is_file());
        validate_repository(&FilesystemRepositoryView::new(repo.path())).unwrap();
    }

    #[test]
    fn test_fresh_profile_init_rolls_back_to_no_jit_after_publication_failure() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = CommandExecutor::new(storage.clone());
        let kernel = first_publish_kernel(&storage, false);

        assert!(executor
            .initialize_fresh_repository_with_kernel(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
                &kernel,
            )
            .is_err());

        assert!(!repo.path().join(".jit").exists());
        assert!(!repo.path().join(".agents").exists());
        assert!(!repo.path().join(".jit-bootstrap").exists());
    }

    #[test]
    fn test_fresh_profile_init_recovers_prepared_bootstrap_before_retry() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = CommandExecutor::new(storage.clone());
        let kernel = first_publish_kernel(&storage, true);
        let error = executor
            .initialize_fresh_repository_with_kernel(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
                &kernel,
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::Prepared
        );
        assert!(repo.path().join(".jit-bootstrap").exists());

        let result = executor
            .initialize_fresh_repository(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
            )
            .unwrap();

        assert_eq!(
            result.profile.unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert!(!repo.path().join(".jit-bootstrap").exists());
        validate_repository(&FilesystemRepositoryView::new(repo.path())).unwrap();
    }

    #[test]
    fn test_fresh_profile_init_reports_committed_cleanup_and_recovery_cleans_it() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = CommandExecutor::new(storage.clone());
        let kernel = terminal_cleanup_kernel(&storage);

        let result = executor
            .initialize_fresh_repository_with_kernel(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
                &kernel,
            )
            .unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.profile.unwrap().warnings.len(), 1);
        assert!(repo.path().join(".jit-bootstrap").exists());
        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(session.report().recovered_count(), 1);
        drop(session);
        assert!(!repo.path().join(".jit-bootstrap").exists());
        validate_repository(&FilesystemRepositoryView::new(repo.path())).unwrap();
    }

    #[test]
    fn test_concurrent_fresh_profile_init_publishes_one_coherent_repository() {
        let repo = Arc::new(TempDir::new().unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let repo = Arc::clone(&repo);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let storage = JsonFileStorage::new(repo.path().join(".jit"));
                    let executor = CommandExecutor::new(storage);
                    barrier.wait();
                    executor.initialize_fresh_repository(
                        repo.path(),
                        &HierarchyTemplate::default(),
                        Some("jit-dogfood"),
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        validate_repository(&FilesystemRepositoryView::new(repo.path())).unwrap();
        let events = fs::read_to_string(repo.path().join(".jit/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 1);
    }
}
