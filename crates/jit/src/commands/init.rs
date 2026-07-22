use super::CommandExecutor;
use crate::config::{slugify_project_name, JitConfig, ProjectName};
use crate::hierarchy_templates::HierarchyTemplate;
use crate::profile::{
    build_profile_claims, EmbeddedProfilePackage, ProfileApplicationStatus, ProfileApplyResult,
};
use crate::repository_state::{
    apply_overlay, derive_profile_materializations, finalize_initialization, GitattributesClaim,
    GitattributesStatus, InitializationScaffold, ProfileContribution, ProfileTargetContribution,
    RepositoryEntry, VirtualPath,
};
use crate::storage::{
    JsonFileStorage, RepositoryMutationSession, RepositoryStateStore, RepositoryStateStoreError,
};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Result of publishing a fresh repository scaffold.
#[derive(Debug)]
pub struct FreshInitResult {
    /// Canonical project identity derived from the repository directory.
    pub project_name: ProjectName,
    /// Applied profile result when initialization included one.
    pub profile: Option<ProfileApplyResult>,
    /// Outcome of the worktree `.gitattributes` merge-driver claim.
    pub gitattributes: GitattributesStatus,
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
        // Typed Git evidence is acquired once at the boundary (loop-invariant).
        let gitattributes = gitattributes_claim(&layout);
        let mut session = self.storage().open_mutation_session(layout)?;
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so a composed ProfileApplied event's id/timestamp stay stable.
        let context = crate::repository_state::MutationContext::production();
        for _ in 0..8 {
            let (config, project_name) =
                self.resolve_init_config(session.as_mut(), repo_dir, template)?;
            let neutral =
                InitializationScaffold::from_config(config.clone(), project_name.clone(), None)?;
            let profiled = match package.as_ref() {
                Some(package) => {
                    match self.compute_profile_contribution(session.as_mut(), package, &neutral)? {
                        // A retryable capture conflict: re-attempt the whole init.
                        None => continue,
                        Some(pair) => Some(pair),
                    }
                }
                None => None,
            };
            let (contribution, mut apply_result) = match profiled {
                Some((contribution, result)) => (Some(contribution), Some(result)),
                None => (None, None),
            };
            let scaffold = InitializationScaffold::from_config(config, project_name, contribution)?
                .with_gitattributes(gitattributes.clone());

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
                None,
            )? {
                None => continue,
                Some(base) => base,
            };
            let delta_overlay = super::validation_overlay(
                finalize_initialization(&probe, &scaffold, &context)?.delta(),
            );

            // Re-capture the base with the exact write set so the validation closure
            // and the delta's preimages come from one coherent image, then finalize,
            // validate, and publish under the same held session.
            let base = match self.capture_proposed_base(
                session.as_mut(),
                &delta_overlay,
                &extra_paths,
                None,
            )? {
                None => continue,
                Some(base) => base,
            };
            let plan = finalize_initialization(&base, &scaffold, &context)?;
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
                    let gitattributes = scaffold.gitattributes_status(&base)?;
                    let profile = apply_result.take().map(|mut result| {
                        if result.status == ProfileApplicationStatus::Applied {
                            result.transaction_id = Some(outcome.transaction_hash.clone());
                        }
                        result
                    });
                    return Ok(FreshInitResult {
                        project_name,
                        profile,
                        gitattributes,
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
    /// through the canonical `repository_state` derivation, carrying its asset bytes,
    /// provenance record, and audit-log image.
    ///
    /// The base is captured under the held session, overlaid with the neutral
    /// scaffold state init will publish (schemas always; an existing
    /// config/gates/rules preserved), so the profile merges over the proposed neutral
    /// repository. Returns `Ok(None)` on a retryable capture conflict so the caller
    /// re-attempts the whole initialization.
    fn compute_profile_contribution(
        &self,
        session: &mut (dyn RepositoryMutationSession + '_),
        package: &EmbeddedProfilePackage<'_>,
        neutral: &InitializationScaffold,
    ) -> Result<Option<(ProfileContribution, ProfileApplyResult)>> {
        super::profile::reject_reserved_application_targets(
            package.hashes().targets.keys().map(String::as_str),
        )?;
        let metadata = &package.manifest().profile;
        let record_path = VirtualPath::data(format!("profiles/{}.json", metadata.id))?;
        let profiles_dir = VirtualPath::data("profiles")?;
        let events_path = VirtualPath::data("events.jsonl")?;
        let neutral_files = neutral.neutral_files();

        // Capture the base broad enough for the merge, region composition, and
        // projection re-render: package content targets, the neutral scaffold files,
        // and the application-owned record/events/profiles paths.
        let mut content_paths = package
            .hashes()
            .targets
            .keys()
            .map(|target| super::repo_rel_virtual_path(target))
            .collect::<Result<Vec<_>>>()?;
        for (path, _) in &neutral_files {
            content_paths.push(super::repo_rel_virtual_path(path)?);
        }
        content_paths.push(record_path.clone());
        content_paths.push(profiles_dir.clone());
        content_paths.push(events_path.clone());
        let base =
            match self.capture_proposed_base(session, &BTreeMap::new(), &content_paths, None)? {
                None => return Ok(None),
                Some(base) => base,
            };

        // The neutral overlay reflects only the scaffold state init publishes:
        // schemas are always (re)written, but an existing config/gates/rules is
        // preserved (`IfAbsent`). Overlaying a present file with its default would
        // shadow the live declarations, so present non-schema neutral files fall
        // through to their captured bytes.
        let mut neutral_overrides: BTreeMap<VirtualPath, Option<Vec<u8>>> = BTreeMap::new();
        for (path, bytes) in neutral_files {
            let vpath = super::repo_rel_virtual_path(&path)?;
            let absent = matches!(base.entry(&vpath)?, RepositoryEntry::Absent);
            if path.starts_with(".jit/schemas/") || absent {
                neutral_overrides.insert(vpath, Some(bytes));
            }
        }
        let neutral_base = apply_overlay(&base, neutral_overrides)?;

        let claims = build_profile_claims(package, &neutral_base)?;
        let derived = derive_profile_materializations(&neutral_base, claims)?;

        let record = super::profile::expected_record(package);
        let record_matches =
            super::profile::record_matches_in_image(&neutral_base, &record_path, &record)?;
        let ensure_profiles_dir =
            super::profile::profile_dir_needs_creation(&neutral_base, &profiles_dir)?;

        let mut targets = Vec::new();
        let mut all_unchanged = true;
        for (path, (bytes, mode)) in &derived {
            if super::profile::target_action(&neutral_base, path, bytes, *mode)?
                != crate::profile::ProfileTargetAction::Unchanged
            {
                all_unchanged = false;
                targets.push(ProfileTargetContribution {
                    path: path.clone(),
                    bytes: bytes.clone(),
                    mode: *mode,
                });
            }
        }
        let profile_changed = !all_unchanged || !record_matches;

        // The finalizer composes the ProfileApplied audit append from the captured
        // events prefix (owning its id, timestamp, and torn-tail evidence); the
        // command only signals whether the profile changed.
        let contribution = ProfileContribution {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            package_hash: package.hashes().package.clone(),
            target_hashes: package.hashes().targets.clone(),
            targets,
            record_path,
            record_bytes: record.to_bytes()?,
            record_changed: !record_matches,
            emit_event: profile_changed,
            ensure_profiles_dir,
        };
        let apply_result = ProfileApplyResult {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            status: if profile_changed {
                ProfileApplicationStatus::Applied
            } else {
                ProfileApplicationStatus::Unchanged
            },
            plan_hash: super::profile::profile_plan_hash(&derived),
            transaction_id: None,
            warnings: Vec::new(),
        };
        Ok(Some((contribution, apply_result)))
    }
}

/// Resolve one embedded profile package by stable id.
fn embedded_profile(id: &str) -> Result<EmbeddedProfilePackage<'static>> {
    super::profile::embedded_profile(id)
}

/// Acquire typed Git evidence for the worktree `.gitattributes` merge-driver claim.
///
/// Eligible only when the worktree is inside a Git work tree AND the selected data
/// root is nested beneath it; the claim line is the Git-escaped worktree-relative
/// data-root path plus `/events.jsonl merge=union`. No Git, a disjoint data root,
/// or an unavailable Git is `NotApplicable`, and core init still succeeds
/// (`@/charter/D-4`).
fn gitattributes_claim(layout: &crate::repository_state::RepositoryLayout) -> GitattributesClaim {
    let worktree = layout.worktree_root();
    let Ok(relative) = layout.data_root().strip_prefix(worktree) else {
        return GitattributesClaim::NotApplicable;
    };
    if relative.as_os_str().is_empty() || !worktree_is_git_work_tree(worktree) {
        return GitattributesClaim::NotApplicable;
    }
    GitattributesClaim::Eligible {
        line: format!("{}/events.jsonl merge=union", git_escape_pattern(relative)),
    }
}

/// Whether Git identifies `worktree` as inside a work tree (Git-optional: an
/// absent or failing `git` is treated as "not a work tree").
fn worktree_is_git_work_tree(worktree: &Path) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "true")
}

/// Escape a worktree-relative data-root path for use as a `.gitattributes` pattern
/// prefix, normalizing separators and escaping the characters Git treats specially
/// in a pattern. The canonical `.jit` passes through unchanged.
fn git_escape_pattern(relative: &Path) -> String {
    let path = relative.to_string_lossy().replace('\\', "/");
    let mut escaped = String::with_capacity(path.len());
    for ch in path.chars() {
        if matches!(ch, ' ' | '#' | '!') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AppliedProfileRecord;
    use crate::storage::{discover_repository_layout, IssueStore};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    #[test]
    fn test_git_escape_pattern_default_and_special_chars() {
        assert_eq!(git_escape_pattern(&PathBuf::from(".jit")), ".jit");
        assert_eq!(git_escape_pattern(&PathBuf::from("da ta")), "da\\ ta");
        assert_eq!(git_escape_pattern(&PathBuf::from("a/b")), "a/b");
    }
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
        assert!(fs::read_to_string(repo.path().join(".jit/rules.toml"))
            .unwrap()
            .contains("name = \"namespace-unique-brackets\""));
        assert!(fs::read_to_string(
            repo.path()
                .join(".jit/schemas/default-type-hierarchy-known.json")
        )
        .unwrap()
        .contains("planning"));
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
    fn test_reinit_refreshes_default_rule_membership_header_and_type_schema() {
        const SQUAD_NAMESPACE: &str = "\
\n[namespaces.squad]\n\
description = \"Owning squad\"\n\
unique = true\n";

        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        executor_with_layout(&storage, repo.path())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();

        let config_path = repo.path().join(".jit/config.toml");
        let rules_path = repo.path().join(".jit/rules.toml");
        let config = fs::read_to_string(&config_path).unwrap().replacen(
            "task = 4 }",
            "task = 4, planning = 3 }",
            1,
        ) + SQUAD_NAMESPACE;
        fs::write(&config_path, &config).unwrap();
        let authored_rule = "\
\n[[rules]]\n\
name = \"custom-shape\"\n\
# authored content survives re-init\n\
severity = \"warn\"\n\
assert = { require-section = { heading = \"Goals\" } }\n";
        let stale_rules = fs::read_to_string(&rules_path).unwrap().replacen(
            crate::repository_state::rules_file_header(),
            "# stale\n\n",
            1,
        ) + authored_rule;
        fs::write(&rules_path, stale_rules).unwrap();

        executor_with_layout(&storage, repo.path())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();

        let refreshed = fs::read_to_string(&rules_path).unwrap();
        assert!(refreshed.starts_with(crate::repository_state::rules_file_header()));
        assert!(refreshed.contains("name = \"namespace-unique-squad\""));
        assert!(refreshed.contains("# authored content survives re-init"));
        assert!(fs::read_to_string(
            repo.path()
                .join(".jit/schemas/default-type-hierarchy-known.json")
        )
        .unwrap()
        .contains("planning"));

        fs::write(&config_path, config.replace(SQUAD_NAMESPACE, "")).unwrap();
        executor_with_layout(&storage, repo.path())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        let refreshed = fs::read_to_string(&rules_path).unwrap();
        assert!(!refreshed.contains("name = \"namespace-unique-squad\""));
        assert!(refreshed.contains("# authored content survives re-init"));
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
