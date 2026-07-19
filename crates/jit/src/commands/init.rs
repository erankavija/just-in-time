use super::CommandExecutor;
use crate::config::{slugify_project_name, JitConfig, ProjectName};
use crate::domain::Event;
use crate::hierarchy_templates::HierarchyTemplate;
use crate::profile::{
    append_profile_event_image, plan_profile_application_against, EmbeddedProfilePackage,
    PlannedTargetAction, ProfileApplicationStatus, ProfileApplyResult, ProfileOrigin,
    ProjectedFileMode, RepositorySnapshot, SnapshotEntry, SnapshotFile,
};
use crate::repository_state::{
    apply_overlay, finalize_initialization, InitializationScaffold, ProfileContribution,
    ProfileTargetContribution, VirtualPath,
};
use crate::storage::{
    IssueStore, JsonFileStorage, RepositoryStateStore, RepositoryStateStoreError,
};
use crate::validation::repository::{
    FilesystemRepositoryView, OverlayRepositoryView, RepositoryView,
};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Result of publishing a fresh repository scaffold.
#[derive(Debug)]
pub struct FreshInitResult {
    /// Canonical project identity derived from the repository directory.
    pub project_name: ProjectName,
    /// Applied profile result when initialization included one.
    pub profile: Option<ProfileApplyResult>,
    /// Non-fatal diagnostics. The recovered session owns transaction recovery, so
    /// this is empty in normal operation and retained only for output stability.
    pub warnings: Vec<String>,
}

impl CommandExecutor<JsonFileStorage> {
    /// Atomically complete neutral initialization and apply one profile.
    ///
    /// An absent data directory is published through the recovered session's
    /// staged-root machinery; an existing partial repository fills only its missing
    /// neutral scaffold state and the profile's changes in one delta.
    pub fn initialize_profiled_repository(
        &self,
        repo_dir: &Path,
        template: &HierarchyTemplate,
        profile_id: &str,
    ) -> Result<FreshInitResult> {
        self.run_initialization(repo_dir, template, Some(profile_id))
    }

    /// Publish a fresh neutral or profiled repository through the recovered
    /// session.
    ///
    /// The neutral scaffold and any profile bytes are captured and validated over
    /// one held session guard, then published as one exact
    /// [`RepositoryDelta`](crate::repository_state::RepositoryDelta); nothing is
    /// mutated before publication.
    pub fn initialize_fresh_repository(
        &self,
        repo_dir: &Path,
        template: &HierarchyTemplate,
        profile_id: Option<&str>,
    ) -> Result<FreshInitResult> {
        self.run_initialization(repo_dir, template, profile_id)
    }

    /// Capture the base under one recovered session, validate the proposed
    /// scaffold/profile overlay, and publish the complete initialization delta.
    fn run_initialization(
        &self,
        repo_dir: &Path,
        template: &HierarchyTemplate,
        profile_id: Option<&str>,
    ) -> Result<FreshInitResult> {
        let package = profile_id.map(embedded_profile).transpose()?;
        let layout = self.require_layout()?;
        let mut session = self.storage().open_mutation_session(layout)?;
        for _ in 0..8 {
            let (config, project_name) =
                self.resolve_init_config(session.as_mut(), repo_dir, template)?;
            let neutral =
                InitializationScaffold::from_config(config.clone(), project_name.clone(), None)?;
            let profiled = package
                .as_ref()
                .map(|package| self.compute_profile_contribution(package, &neutral))
                .transpose()?;
            let (contribution, mut apply_result) = match profiled {
                Some((contribution, result)) => (Some(contribution), Some(result)),
                None => (None, None),
            };
            let scaffold = InitializationScaffold::from_config(config, project_name, contribution)?;

            let extra_paths = scaffold.delta_paths()?;
            // Probe capture: the deliberately over-inclusive scaffold overlay yields
            // a base good enough to finalize the exact delta. That delta's overlay
            // is the AUTHORITATIVE proposed state — only the files init writes — so a
            // preserved `IfAbsent` file (e.g. an existing `rules.toml` referencing a
            // custom schema) is not shadowed by its neutral default in the closure.
            let probe_overrides = scaffold.overlay_overrides()?;
            let probe = match self.capture_proposed_base(
                session.as_mut(),
                &probe_overrides,
                &extra_paths,
            )? {
                None => continue,
                Some(base) => base,
            };
            let delta_overlay =
                super::validation_overlay(finalize_initialization(&probe, &scaffold)?.delta());

            // Re-capture the base with the exact write set so the validation closure
            // and the delta's preimages come from one coherent image, then finalize,
            // validate, and publish under the same held session.
            let base =
                match self.capture_proposed_base(session.as_mut(), &delta_overlay, &extra_paths)? {
                    None => continue,
                    Some(base) => base,
                };
            let plan = finalize_initialization(&base, &scaffold)?;
            let proposed = apply_overlay(&base, delta_overlay)?;
            let validation = crate::validation::repository::validate_repository(&proposed)?;
            if validation.rule_report.has_errors() {
                anyhow::bail!(
                    "repository initialization produced {} validation error finding(s)",
                    validation.rule_report.error_count()
                );
            }

            match session.apply(&plan) {
                Ok(outcome) => {
                    let project_name = scaffold.project_name().clone();
                    let profile = apply_result.take().map(|mut result| {
                        if result.status == ProfileApplicationStatus::Applied {
                            result.transaction_id = Some(outcome.transaction_hash.clone());
                        }
                        result
                    });
                    return Ok(FreshInitResult {
                        project_name,
                        profile,
                        warnings: Vec::new(),
                    });
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "repository initialization did not converge after repeated capture conflicts"
        ))
    }

    /// Resolve the effective configuration bytes and project identity: an existing
    /// `config.toml` is preserved (its authored project name kept), otherwise the
    /// template body is rendered under a slug of the repository directory name.
    fn resolve_init_config(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        repo_dir: &Path,
        template: &HierarchyTemplate,
    ) -> Result<(String, ProjectName)> {
        use crate::repository_state::{CaptureBudget, CaptureSpec};
        let budget = CaptureBudget {
            max_paths: 16,
            max_listings: 0,
            max_bytes: 64 * 1024 * 1024,
            max_depth: 6,
        };
        let spec = CaptureSpec::phase_one([VirtualPath::data("config.toml")?], budget)?;
        let image = session.capture(spec)?;
        let generated_name: ProjectName = slugify_project_name(
            repo_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
        )
        .parse()?;
        match super::image_repo_bytes(&image, ".jit/config.toml")? {
            Some(bytes) => {
                let config =
                    String::from_utf8(bytes).context("existing .jit/config.toml is not UTF-8")?;
                let parsed: JitConfig = toml::from_str(&config)
                    .context("Failed to parse existing init configuration")?;
                let project_name = parsed
                    .project
                    .and_then(|project| project.name)
                    .unwrap_or(generated_name);
                Ok((config, project_name))
            }
            None => Ok((
                crate::repository_state::render_repo_config(
                    &template.generate_config_toml(),
                    &generated_name,
                ),
                generated_name,
            )),
        }
    }

    /// Compute one embedded profile's contribution to the initialization delta
    /// through the transitional profile planner (increment-6 deletion target),
    /// carrying its asset bytes, provenance record, and audit-log image.
    fn compute_profile_contribution(
        &self,
        package: &EmbeddedProfilePackage<'_>,
        neutral: &InitializationScaffold,
    ) -> Result<(ProfileContribution, ProfileApplyResult)> {
        super::profile::reject_reserved_application_targets(
            package.hashes().targets.keys().map(String::as_str),
        )?;
        let metadata = &package.manifest().profile;
        let record_path = format!(".jit/profiles/{}.json", metadata.id);
        let neutral_files = neutral.neutral_files();
        let captured = self.storage.capture_profile_snapshot(
            package
                .hashes()
                .targets
                .keys()
                .map(String::as_str)
                .chain([record_path.as_str(), ".jit/profiles", ".jit/events.jsonl"])
                .chain(neutral_files.iter().map(|(path, _)| path.as_str())),
        )?;
        // The neutral overlay must reflect only the scaffold state init will
        // actually publish: schemas are always (re)written, but an existing
        // config/gates/rules is preserved (`IfAbsent`). Overlaying a present file
        // with its empty default would shadow the live declarations and make the
        // profile planner see spurious drift, so present non-schema neutral files
        // fall through to their captured on-disk bytes.
        let overlay_files: Vec<(String, Vec<u8>)> = neutral_files
            .into_iter()
            .filter(|(path, _)| path.starts_with(".jit/schemas/") || captured.entry(path).is_none())
            .collect();
        let mut entries = captured.entries().clone();
        entries.insert(PathBuf::from(".jit/issues"), SnapshotEntry::Directory);
        for (path, bytes) in &overlay_files {
            entries.insert(
                PathBuf::from(path),
                SnapshotEntry::File(SnapshotFile {
                    bytes: bytes.clone(),
                    mode: ProjectedFileMode::Regular,
                }),
            );
        }
        let snapshot = RepositorySnapshot::new(captured.root(), entries)?;
        super::profile::ensure_profile_directory(&snapshot)?;
        let filesystem: Arc<dyn RepositoryView> = Arc::new(
            FilesystemRepositoryView::from_jit_root(self.storage.root())?,
        );
        let overlay = overlay_files
            .iter()
            .map(|(path, bytes)| (PathBuf::from(path), Some(bytes.clone())))
            .collect::<Vec<_>>();
        let neutral_view: Arc<dyn RepositoryView> =
            Arc::new(OverlayRepositoryView::new(filesystem, overlay)?);
        let plan = plan_profile_application_against(package, &snapshot, neutral_view)?;

        let record = super::profile::expected_record(package);
        let record_matches =
            super::profile::inspect_installed_record(&snapshot, &record_path, &record)?;
        let profile_changed = !plan.is_no_op() || !record_matches;
        let prior_events = snapshot
            .file(".jit/events.jsonl")
            .map_or(&[][..], |file| file.bytes.as_slice());
        let events = if profile_changed {
            let isolated_torn_tail =
                super::profile::has_malformed_unterminated_event_tail(prior_events);
            let event = Event::new_profile_applied(
                metadata.id.clone(),
                metadata.version.clone(),
                ProfileOrigin::Embedded,
                package.hashes().package.clone(),
                package.hashes().targets.clone(),
                isolated_torn_tail,
            );
            append_profile_event_image(prior_events, &event)?
        } else {
            prior_events.to_vec()
        };

        let targets = plan
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
        let contribution = ProfileContribution {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            package_hash: package.hashes().package.clone(),
            targets,
            record_path: VirtualPath::data(format!("profiles/{}.json", metadata.id))?,
            record_bytes: record.to_bytes()?,
            record_changed: !record_matches,
            events_bytes: events,
            events_changed: profile_changed,
            ensure_profiles_dir: snapshot.entry(".jit/profiles").is_none(),
        };
        let apply_result = ProfileApplyResult {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            status: if profile_changed {
                ProfileApplicationStatus::Applied
            } else {
                ProfileApplicationStatus::Unchanged
            },
            plan_hash: plan.identity.plan_hash,
            transaction_id: None,
            warnings: Vec::new(),
        };
        Ok((contribution, apply_result))
    }
}

/// Resolve one embedded profile package by stable id.
fn embedded_profile(id: &str) -> Result<EmbeddedProfilePackage<'static>> {
    super::profile::embedded_profile(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AppliedProfileRecord;
    use crate::storage::discover_repository_layout;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    /// A file-backed executor carrying the canonical layout for `worktree`.
    fn executor_with_layout(
        storage: &JsonFileStorage,
        worktree: &Path,
    ) -> CommandExecutor<JsonFileStorage> {
        let layout = discover_repository_layout(worktree, storage.root()).unwrap();
        CommandExecutor::new(storage.clone()).with_layout(layout)
    }

    /// Assert the published repository validates cleanly through the closed-image
    /// pipeline.
    fn assert_repo_valid(worktree: &Path) {
        let data = worktree.join(".jit");
        let layout = discover_repository_layout(worktree, &data).unwrap();
        CommandExecutor::new(JsonFileStorage::new(&data))
            .with_layout(layout)
            .validate_repository_report()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn test_fresh_init_publishes_complete_valid_repo_without_git() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());

        let result = executor
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();

        assert!(result.profile.is_none());
        for path in [
            "index.json",
            "gates.toml",
            "events.jsonl",
            "config.toml",
            "rules.toml",
        ] {
            assert!(
                repo.path().join(".jit").join(path).is_file(),
                "missing {path}"
            );
        }
        assert!(repo.path().join(".jit/issues").is_dir());
        assert_repo_valid(repo.path());
    }

    #[test]
    fn test_fresh_profile_init_publishes_complete_valid_repo_without_git() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());

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
        assert_eq!(
            fs::read_to_string(repo.path().join(".jit/events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_repo_valid(repo.path());
    }

    #[test]
    fn test_reinit_profiled_over_existing_root_is_idempotent_unchanged() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());

        executor
            .initialize_fresh_repository(
                repo.path(),
                &HierarchyTemplate::default(),
                Some("jit-dogfood"),
            )
            .unwrap();
        let compact_record = {
            let record: AppliedProfileRecord = serde_json::from_slice(
                &fs::read(repo.path().join(".jit/profiles/jit-dogfood.json")).unwrap(),
            )
            .unwrap();
            serde_json::to_vec(&record).unwrap()
        };
        fs::write(
            repo.path().join(".jit/profiles/jit-dogfood.json"),
            &compact_record,
        )
        .unwrap();
        let before_events = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

        // A second `jit init` is a fresh process that re-discovers the layout over
        // the now-published root; reuse of the first executor's absent-root layout
        // would be a test artifact, not the real re-init path.
        let reinit = executor_with_layout(&storage, repo.path());
        let again = reinit
            .initialize_profiled_repository(
                repo.path(),
                &HierarchyTemplate::default(),
                "jit-dogfood",
            )
            .unwrap();

        assert_eq!(
            again.profile.unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
            before_events
        );
        assert_eq!(
            fs::read(repo.path().join(".jit/profiles/jit-dogfood.json")).unwrap(),
            compact_record
        );
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
                    let executor = executor_with_layout(&storage, repo.path());
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
        assert_repo_valid(repo.path());
        let events = fs::read_to_string(repo.path().join(".jit/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 1);
    }
}
