use super::{
    profile::{ProfileOps, ProfileSelection},
    with_mutation_session, SessionStep,
};
use crate::config::{slugify_project_name, ProjectName};
use crate::profile::{ProfileComposedApplyResult, ProfileRunMode};
use crate::repository_state::{
    apply_overlay, derive_materialization, ExpectedPreimage, GitattributesClaim,
    GitattributesStatus, InitializationScaffold, MaterializationPlan, MaterializationRequest,
    RepositoryAction, VirtualPath,
};
use crate::storage::worktree_identity::WorktreeIdentity;
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::{Context, Result};
use std::path::Path;

/// Result of publishing a fresh repository scaffold.
#[derive(Debug)]
pub struct FreshInitResult {
    /// Applied profile results when initialization included a profile, one per
    /// package of its dependency closure.
    pub profile: Option<ProfileComposedApplyResult>,
    /// Outcome of the worktree `.gitattributes` merge-driver claim.
    pub gitattributes: GitattributesStatus,
    /// Files whose canonical plan preimage was absent.
    pub created_paths: Vec<String>,
    /// Files whose canonical plan preimage was an existing file.
    pub modified_paths: Vec<String>,
}

impl<S: IssueStore + RepositoryStateStore> ProfileOps<'_, S> {
    /// Publish a fresh neutral or profiled repository through one mutation
    /// session.
    ///
    /// The neutral scaffold and any profile bytes are captured and validated over
    /// one held session guard, then published as one exact
    /// [`RepositoryDelta`](crate::repository_state::RepositoryDelta); nothing is
    /// mutated before publication.
    ///
    /// The selection was built before this runs, so an unresolvable selector,
    /// dependency, or variable input fails before a repository is created.
    /// The complete dependency-first closure and neutral scaffold publish in
    /// one delta with one aggregate lifecycle event.
    pub fn initialize_fresh_repository(
        &self,
        repo_dir: &Path,
        selection: Option<&ProfileSelection>,
    ) -> Result<FreshInitResult> {
        let members = selection.map(ProfileSelection::members).unwrap_or_default();
        let contribution_context = selection
            .map(ProfileSelection::contribution_context)
            .unwrap_or_default();
        let profiles = selection
            .map(ProfileSelection::application_inputs)
            .unwrap_or_default();
        let layout = self.exec.require_layout()?;
        // Typed Git evidence is acquired once at the boundary (loop-invariant).
        let gitattributes = gitattributes_claim(&layout);
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so a composed lifecycle event's id/timestamp stay stable.
        let context = super::production_mutation_context();
        let mut result = with_mutation_session(
            self.exec.storage(),
            &layout,
            "repository initialization",
            |session| {
                let (config, project_name, config_image) =
                    self.resolve_init_config(&mut *session, repo_dir)?;
                let scaffold = InitializationScaffold::from_config(
                    config.clone(),
                    project_name.clone(),
                    None,
                )?
                .with_gitattributes(gitattributes.clone())
                .with_profiles(profiles.clone());

                let mut extra_paths = scaffold
                    .delta_paths()?
                    .into_iter()
                    .filter(|path| path != &VirtualPath::EVENTS)
                    .collect::<Vec<_>>();
                extra_paths.extend(crate::repository_state::profile_contribution_target_paths(
                    contribution_context,
                ));
                // Probe capture: the deliberately over-inclusive scaffold overlay yields
                // a base good enough to finalize the exact delta. That delta's overlay
                // is the AUTHORITATIVE proposed state — only the files init writes — so a
                // preserved `IfAbsent` file (e.g. an existing `rules.toml` referencing a
                // custom schema) is not shadowed by its neutral default in the closure.
                let probe_overrides = scaffold.overlay_overrides()?;
                let Some(mut probe) = self.exec.capture_proposed_base(
                    &mut *session,
                    &probe_overrides,
                    &extra_paths,
                    Some(config_image),
                )?
                else {
                    return Ok(SessionStep::Retry);
                };
                if scaffold.has_profiles() {
                    let closure = scaffold.profile_capture_closure(&probe)?;
                    let mut spec = probe.capture_spec().clone();
                    spec.discover_paths(closure.iter().cloned())?;
                    extra_paths.extend(closure);
                    let Some(expanded) = super::capture_or_retry(session.extend(probe, spec))?
                    else {
                        return Ok(SessionStep::Retry);
                    };
                    probe = expanded;
                }
                let delta_overlay = crate::repository_state::action_overrides(
                    derive_materialization(
                        &probe,
                        &context,
                        MaterializationRequest::Initialize {
                            scaffold: &scaffold,
                        },
                    )?
                    .delta()
                    .actions(),
                );

                // Grow the probe by the exact write set's validation closure, then
                // finalize, validate, and publish under the same held session.
                let Some(base) = self.exec.capture_proposed_base(
                    &mut *session,
                    &delta_overlay,
                    &extra_paths,
                    Some(probe),
                )?
                else {
                    return Ok(SessionStep::Retry);
                };
                let final_profile_closure = scaffold.profile_capture_closure(&base)?;
                if final_profile_closure
                    .iter()
                    .any(|path| !base.capture_spec().contains_path(path))
                {
                    return Ok(SessionStep::Retry);
                }
                let plan = derive_materialization(
                    &base,
                    &context,
                    MaterializationRequest::Initialize {
                        scaffold: &scaffold,
                    },
                )?;
                if plan.delta().actions().iter().any(|action| {
                    !base
                        .capture_spec()
                        .paths()
                        .any(|path| path == action.path())
                }) {
                    return Ok(SessionStep::Retry);
                }
                // A profiled initialization publishes through the same profile
                // decision every lifecycle command does, so it refuses the same
                // unpublishable targets rather than writing part of a selection
                // it cannot complete.
                crate::profile::ensure_publishable_targets(&plan)
                    .map_err(crate::repository_state::RepositoryStateError::from)?;
                let proposed = apply_overlay(
                    &base,
                    crate::repository_state::action_overrides(plan.delta().actions()),
                )?;
                let validation = crate::validation::report(
                    &crate::validation::CapturedValidation::from_image(&proposed),
                    &crate::validation::ReportInputs {
                        now: chrono::Utc::now(),
                        history: None,
                    },
                );
                let (failure, rule_report) = validation.into_parts();
                if let Some(error) = failure {
                    return Err(init_validation_error(error));
                }
                if rule_report.has_errors() {
                    anyhow::bail!(
                        "repository initialization produced {} validation error finding(s)",
                        rule_report.error_count()
                    );
                }
                let gitattributes = scaffold.gitattributes_status(&base)?;
                let (created_paths, modified_paths) = init_response_paths(&plan, gitattributes)?;

                let profile = if members.is_empty() {
                    None
                } else {
                    Some(ProfileComposedApplyResult::new(
                        members
                            .iter()
                            .map(|member| {
                                super::profile::profile_result(
                                    &plan,
                                    member.package(),
                                    &layout,
                                    ProfileRunMode::Publish,
                                )
                            })
                            .collect::<Result<Vec<_>>>()?,
                    ))
                };
                Ok(SessionStep::Apply(
                    plan,
                    FreshInitResult {
                        profile,
                        gitattributes,
                        created_paths,
                        modified_paths,
                    },
                ))
            },
        )?;

        result.profile = result
            .profile
            .take()
            .map(|applied| {
                super::profile::selection_profile_results(
                    applied.items,
                    selection.map(ProfileSelection::roots).unwrap_or_default(),
                )
            })
            .transpose()?;
        Ok(result)
    }

    /// Resolve the effective configuration bytes and project identity: an existing
    /// `config.toml` is preserved (its authored project name kept), otherwise the
    /// structural minimum is rendered under a slug of the repository directory
    /// name. Returns the round's image for the base to grow from.
    fn resolve_init_config(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        repo_dir: &Path,
    ) -> Result<(
        String,
        ProjectName,
        crate::repository_state::RepositoryImage,
    )> {
        let spec = crate::repository_state::CaptureSpec::new(
            [VirtualPath::CONFIG],
            super::validate::MUTATION_CAPTURE_BUDGET,
        )?;
        let image = session.capture(spec)?;
        let generated_name: ProjectName = slugify_project_name(
            repo_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
        )
        .parse()?;
        let (config, project_name) = match super::image_virtual_bytes(
            &image,
            &crate::repository_state::VirtualPath::CONFIG,
        )? {
            Some(bytes) => {
                let config =
                    String::from_utf8(bytes).context("existing .jit/config.toml is not UTF-8")?;
                let parsed = crate::declarations::assemble_configuration(
                    Some(config.as_bytes()),
                    None,
                    None,
                )
                .context("Failed to parse existing init configuration")?;
                let project_name = parsed
                    .project
                    .and_then(|project| project.name)
                    .unwrap_or(generated_name);
                (config, project_name)
            }
            None => (
                crate::repository_state::render_repo_config(
                    &crate::repository_state::structural_minimum_config(),
                    &generated_name,
                ),
                generated_name,
            ),
        };
        Ok((config, project_name, image))
    }
}

impl<S: IssueStore> ProfileOps<'_, S> {
    /// Create or refresh machine-local worktree identity after repository
    /// scaffold publication.
    ///
    /// Called after the scaffold is published, so optional Git host state
    /// remains outside repository publication.
    pub fn initialize_worktree_identity(
        &self,
        paths: &crate::storage::worktree_paths::WorktreePaths,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;

        if !paths.is_git_repository() {
            return Ok((None, Vec::new()));
        }

        // Get git branch name
        let branch = std::process::Command::new("git")
            .arg("-C")
            .arg(&paths.worktree_root)
            .args(["branch", "--show-current"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "main".to_string())
            .trim()
            .to_string();

        // Create or update worktree identity
        // This handles copied files in git worktrees automatically
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        Ok((Some(identity), warnings))
    }
}

/// Project the stable public init report from the exact finalized plan.
///
/// The response intentionally reports only the user-facing scaffold files plus
/// the worktree `.gitattributes` claim. Derived schemas, projections, and
/// other materialization refreshes remain internal publication details. Every
/// reported creation or modification is nevertheless proven by the finalized
/// action's expected preimage; no ambient filesystem probe participates.
fn init_response_paths(
    plan: &MaterializationPlan,
    gitattributes: GitattributesStatus,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut created = [
        VirtualPath::INDEX,
        VirtualPath::GATES,
        VirtualPath::INVARIANTS,
        VirtualPath::EVENTS,
        VirtualPath::CONFIG,
        VirtualPath::RULES,
    ]
    .into_iter()
    .filter_map(|path| {
        plan.delta()
            .actions()
            .iter()
            .find(|action| action.path() == &path)
            // Each of these paths is a file, so an absent preimage means it is created.
            .is_some_and(|action| *action.expected() == ExpectedPreimage::Absent)
            .then(|| path.repository_relative())
    })
    .collect::<Vec<_>>();
    let mut modified = Vec::new();

    let attributes_path = VirtualPath::GITATTRIBUTES;
    let attributes_action = plan
        .delta()
        .actions()
        .iter()
        .find(|action| action.path() == &attributes_path);
    match gitattributes {
        GitattributesStatus::Created => match attributes_action {
            Some(RepositoryAction::WriteFile {
                expected: ExpectedPreimage::Absent,
                ..
            }) => created.push(".gitattributes".to_string()),
            _ => anyhow::bail!(
                "initialization report status {gitattributes:?} disagrees with finalized .gitattributes action"
            ),
        },
        GitattributesStatus::Modified => match attributes_action {
            Some(RepositoryAction::WriteFile {
                expected: ExpectedPreimage::File { .. },
                ..
            }) => modified.push(".gitattributes".to_string()),
            _ => anyhow::bail!(
                "initialization report status {gitattributes:?} disagrees with finalized .gitattributes action"
            ),
        },
        GitattributesStatus::NotApplicable | GitattributesStatus::Unchanged => {}
    }

    Ok((created, modified))
}

/// Preserve the startup-facing unsupported-format error while deriving it from the
/// same closed proposed image that init validates. Other structural failures keep
/// their original validation context.
fn init_validation_error(error: anyhow::Error) -> anyhow::Error {
    let unsupported = error.chain().find_map(|cause| {
        cause
            .downcast_ref::<crate::repository_state::RepositoryIndexError>()
            .and_then(|error| match error {
                crate::repository_state::RepositoryIndexError::UnsupportedVersion {
                    found,
                    supported,
                } => Some((*found, *supported)),
                _ => None,
            })
    });
    unsupported.map_or(error, |(found, supported)| {
        crate::storage::RepositoryFormatUnsupportedError::new(found, supported).into()
    })
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
        line: format!("{} merge=union", git_events_pattern(relative)),
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

/// Render the worktree-relative events path as a literal `.gitattributes` pattern.
/// The canonical `.jit/events.jsonl` passes through unchanged; patterns containing
/// whitespace or a quote use Git's C-style quoting layer.
fn git_events_pattern(relative: &Path) -> String {
    let path = relative.join(
        crate::repository_state::VirtualPath::EVENTS
            .relative()
            .as_path(),
    );
    let path = path.to_string_lossy();
    let mut escaped = String::with_capacity(path.len());
    for ch in path.chars() {
        if ch == std::path::MAIN_SEPARATOR {
            escaped.push('/');
            continue;
        }
        if matches!(ch, '#' | '!' | '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    if !path.chars().any(|ch| ch == '"' || ch.is_whitespace()) {
        return escaped;
    }
    let mut quoted = escaped.chars().fold(String::from("\""), |mut quoted, ch| {
        if matches!(ch, '\\' | '"') {
            quoted.push('\\');
        }
        quoted.push(ch);
        quoted
    });
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::profile::ProfileApplicationStatus;
    use crate::profile::ProfilePackage;
    use crate::repository_state::{Contribution, MapEntryTarget};
    use crate::storage::{discover_repository_layout, IssueStore, JsonFileStorage};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    /// Count each apply's own pre-commit check, the one boundary every
    /// publication crosses on both the redo-record and the absent-root path,
    /// and no read-only capture.
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
        fn check(&self, point: &crate::storage::MutationPoint) -> std::io::Result<()> {
            if point == &crate::storage::MutationPoint::BeforeCommitCheck {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    #[test]
    fn test_git_events_pattern_preserves_default_and_quotes_lexical_specials() {
        assert_eq!(
            git_events_pattern(&PathBuf::from(".jit")),
            ".jit/events.jsonl"
        );
        assert_eq!(
            git_events_pattern(&PathBuf::from("da ta")),
            "\"da ta/events.jsonl\""
        );
        assert_eq!(
            git_events_pattern(&PathBuf::from("#data")),
            "\\#data/events.jsonl"
        );
        assert_eq!(
            git_events_pattern(&PathBuf::from("!data")),
            "\\!data/events.jsonl"
        );
        assert_eq!(
            git_events_pattern(&PathBuf::from("\"data")),
            "\"\\\"data/events.jsonl\""
        );
        assert_eq!(
            git_events_pattern(&PathBuf::from("a/b")),
            "a/b/events.jsonl"
        );
    }

    #[test]
    fn test_git_events_pattern_escapes_globs_and_literal_backslash() {
        assert_eq!(
            git_events_pattern(&PathBuf::from("glob*/query?/open[close]")),
            "glob\\*/query\\?/open\\[close\\]/events.jsonl"
        );
        #[cfg(unix)]
        assert_eq!(
            git_events_pattern(&PathBuf::from(r"back\slash")),
            r"back\\slash/events.jsonl"
        );
        #[cfg(windows)]
        assert_eq!(
            git_events_pattern(&PathBuf::from(r"back\slash")),
            "back/slash/events.jsonl"
        );
    }
    use crate::test_utils::file_executor;
    use std::thread;
    use tempfile::TempDir;

    impl CommandExecutor<JsonFileStorage> {
        /// Initialize `repo_dir` the way `jit init --profile` does: the
        /// selection is built before initialization opens its session.
        fn initialize_selected(
            &self,
            repo_dir: &Path,
            selectors: &[PathBuf],
        ) -> Result<FreshInitResult> {
            let selection = self
                .profiles()
                .select_profiles(selectors, &Default::default())?;
            self.profiles()
                .initialize_fresh_repository(repo_dir, Some(&selection))
        }
    }

    #[test]
    fn test_fresh_init_publishes_complete_valid_repo_without_git() {
        let repo = TempDir::new().unwrap();
        let executor = file_executor(repo.path()).unwrap();

        let result = executor
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        assert!(result.profile.is_none());
        for path in [
            "index.json",
            "gates.toml",
            "invariants.toml",
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
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    #[test]
    fn test_fresh_init_over_memory_store_writes_the_files_the_json_backend_writes() {
        let repo = TempDir::new().unwrap();
        let json = file_executor(repo.path()).unwrap();
        json.profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        let memory = crate::storage::InMemoryStorage::rooted_at(repo.path());
        let layout = memory.repository_layout();
        let memory_executor = CommandExecutor::new(memory.clone())
            .with_layout(layout)
            .unwrap();
        memory_executor
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        for path in [
            ".jit/config.toml",
            ".jit/index.json",
            ".jit/gates.toml",
            ".jit/invariants.toml",
            ".jit/events.jsonl",
            ".jit/rules.toml",
        ] {
            let expected = json.storage().read_repo_file(path).unwrap();
            assert!(expected.is_some(), "JSON backend wrote no {path}");
            assert_eq!(memory.read_repo_file(path).unwrap(), expected, "{path}");
        }
    }

    /// Write `config_toml` as the repository's configuration before it is
    /// initialized, which initialization preserves.
    fn declare_before_init(repo: &Path, config_toml: &str) {
        fs::create_dir_all(repo.join(".jit")).unwrap();
        fs::write(repo.join(".jit/config.toml"), config_toml).unwrap();
    }

    #[test]
    fn test_fresh_init_creates_the_invariant_registry_present_and_empty() {
        let repo = TempDir::new().unwrap();
        declare_before_init(
            repo.path(),
            &crate::test_utils::shipped_item_kind_tables(&["invariant"]),
        );
        let executor = file_executor(repo.path()).unwrap();

        let result = executor
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        // Present and empty, like the gate registry beside it: the file exists,
        // parses, and declares nothing.
        let registry = fs::read_to_string(repo.path().join(".jit/invariants.toml")).unwrap();
        assert!(
            crate::declarations::invariants::InvariantRegistry::from_toml_str(&registry)
                .unwrap()
                .invariants
                .is_empty()
        );
        assert!(!registry.is_empty());
        assert!(result
            .created_paths
            .contains(&".jit/invariants.toml".to_string()));
        // The kind resolves against that registry with nothing declared in it and
        // no package applied.
        assert!(executor
            .items()
            .list_items(Some("invariant"))
            .unwrap()
            .items
            .is_empty());
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    #[test]
    fn test_fresh_init_registry_resolves_an_authored_invariant_without_a_package() {
        let repo = TempDir::new().unwrap();
        declare_before_init(
            repo.path(),
            &crate::test_utils::shipped_item_kind_tables(&["invariant"]),
        );
        let executor = file_executor(repo.path()).unwrap();
        executor
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        // The adopter authors into the registry initialization gave them; nothing
        // else is installed first.
        let authored = crate::declarations::invariants::InvariantRegistry {
            invariants: vec![crate::declarations::invariants::Invariant {
                id: "authored-here".to_string(),
                statement: "Authored straight into the scaffolded registry.".to_string(),
                kind: crate::declarations::invariants::InvariantKind::Advisory,
                enforced_by: None,
            }],
        };
        fs::write(
            repo.path().join(".jit/invariants.toml"),
            crate::declarations::invariants::serialize_invariant_registry(&authored).unwrap(),
        )
        .unwrap();

        let resolved = file_executor(repo.path())
            .unwrap()
            .items()
            .show_item("@/invariant/authored-here")
            .unwrap();
        assert_eq!(resolved.item.self_id, "authored-here");
        assert!(resolved.issue_full_id.is_none());
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    /// The checked-in fixture tree every composition case below stages copies
    /// of, each under a rewritten manifest declaring its own id.
    fn composition_package() -> std::path::PathBuf {
        crate::test_utils::profile_package_fixture("planner-asset-only")
    }

    fn composition_package_with_namespace(
        repository: &TempDir,
        location: &str,
        id: &str,
        dependencies: &[&str],
        description: &str,
    ) -> ProfilePackage {
        let directory = repository.path().join(location);
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &directory,
            id,
            dependencies,
        );
        let manifest = directory.join(crate::profile::MANIFEST_FILE_NAME);
        let authored = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            format!(
                "{authored}\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\nidentity = \"init-shared\"\nvalue = {{ description = \"{description}\", unique = false }}\n"
            ),
        )
        .unwrap();
        ProfilePackage::from_directory(&directory).unwrap()
    }

    #[test]
    fn test_fresh_profile_init_applies_the_packages_the_named_one_depends_on() {
        let repo = TempDir::new().unwrap();
        // Two packages beside each other inside the worktree the repository is
        // created in, which is where an adopter puts an obtained set.
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/base"),
            "base",
            &[],
        );
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["base"],
        );

        let result = file_executor(repo.path())
            .unwrap()
            .initialize_selected(repo.path(), &[repo.path().join("packages/workflow")])
            .unwrap();

        // One initialization, both packages, the dependency first.
        let profile = result
            .profile
            .expect("a profiled initialization reports it");
        assert_eq!(
            profile
                .items
                .iter()
                .map(|applied| (applied.id.as_str(), applied.status))
                .collect::<Vec<_>>(),
            vec![
                ("base", ProfileApplicationStatus::Applied),
                ("workflow", ProfileApplicationStatus::Applied),
            ]
        );
        assert!(repo.path().join("docs/base.txt").is_file());
        assert!(repo.path().join("docs/workflow.txt").is_file());
    }

    #[test]
    fn test_profiled_init_publishes_its_complete_closure_through_one_publication() {
        let repo = TempDir::new().unwrap();
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/base"),
            "base",
            &[],
        );
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["base"],
        );
        let publications = PublicationCounter::new();
        let storage = JsonFileStorage::with_mutation_interruptions(
            repo.path().join(".jit"),
            publications.clone(),
        );

        let result = CommandExecutor::new(storage.clone())
            .with_layout(discover_repository_layout(repo.path(), storage.root()).unwrap())
            .unwrap()
            .initialize_selected(repo.path(), &[repo.path().join("packages/workflow")])
            .unwrap();

        assert_eq!(publications.count(), 1);
        assert_eq!(
            result
                .profile
                .expect("profiled init reports its closure")
                .items
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "workflow"]
        );
    }

    #[test]
    fn test_fresh_profile_init_refuses_a_conflicting_closure_before_scaffolding() {
        let repo = TempDir::new().unwrap();
        composition_package_with_namespace(
            &repo,
            "packages/base",
            "base",
            &[],
            "Base initialization definition.",
        );
        composition_package_with_namespace(
            &repo,
            "packages/workflow",
            "workflow",
            &[],
            "Workflow initialization definition.",
        );

        let error = file_executor(repo.path())
            .unwrap()
            .initialize_selected(
                repo.path(),
                &[
                    repo.path().join("packages/base"),
                    repo.path().join("packages/workflow"),
                ],
            )
            .unwrap_err();

        assert!(format!("{error:#}").contains("init-shared"));
        assert!(
            !repo.path().join(".jit").exists(),
            "a conflicting closure must fail before scaffold publication"
        );
    }

    #[test]
    fn test_fresh_profile_init_preserves_repeated_root_occurrences_in_order() {
        let repo = TempDir::new().unwrap();
        let location = repo.path().join("packages/workflow");
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &location,
            "workflow",
            &[],
        );

        let result = file_executor(repo.path())
            .unwrap()
            .initialize_selected(
                repo.path(),
                &[PathBuf::from(&location), PathBuf::from(&location)],
            )
            .unwrap();

        let profiles = result
            .profile
            .expect("profiled init reports its roots")
            .items;
        assert_eq!(
            profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.status))
                .collect::<Vec<_>>(),
            vec![
                ("workflow", ProfileApplicationStatus::Applied),
                ("workflow", ProfileApplicationStatus::Unchanged),
            ],
            "each selector occurrence remains an ordered root result"
        );
        assert_eq!(
            fs::read_to_string(repo.path().join(".jit/events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1,
            "repeated roots do not duplicate the durable application"
        );
    }

    #[test]
    fn test_fresh_profile_init_applies_every_package_of_the_selected_closure() {
        let repo = TempDir::new().unwrap();
        let executor = file_executor(repo.path()).unwrap();
        // The set initialization has to apply is the selected package's whole
        // dependency closure. It is named from the staged package itself, so
        // the two sides reach it by different routes.
        let location = crate::test_utils::stage_repository_packages(repo.path(), "jit-dogfood");
        let workflow = crate::profile::ProfilePackage::from_directory(&location).unwrap();
        let closure = crate::test_utils::selected_closure(&executor, &workflow).unwrap();

        let result = executor
            .initialize_selected(repo.path(), &[PathBuf::from(&location)])
            .unwrap();

        let ids = closure
            .iter()
            .map(|package| package.model().id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            result
                .profile
                .expect("a profiled initialization reports it")
                .items
                .iter()
                .map(|applied| applied.id.as_str())
                .collect::<Vec<_>>(),
            ids,
            "initialization applies the whole set the named profile resolves to"
        );
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    #[test]
    fn test_fresh_profile_init_applies_a_declared_dependency_under_a_retained_session() {
        let repo = TempDir::new().unwrap();
        // The dependency is named and no location is supplied for it, so it is
        // resolved from the directory beside the package declaring it — the
        // shape an obtained set of packages arrives in.
        let dependency = crate::test_utils::capture_repository_package(
            "jit-default",
            &repo.path().join("packages/jit-default"),
        )
        .expect("this repository's jit-default package captures");
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["jit-default"],
        );
        let executor = file_executor(repo.path()).unwrap();
        // The shape every `jit init` runs in: startup recovers and retains its
        // session, and initialization publishes and applies inside it.
        let retained = executor
            .storage()
            .open_and_retain_mutation_session(executor.require_layout().unwrap())
            .unwrap();

        let result = executor
            .initialize_selected(repo.path(), &[repo.path().join("packages/workflow")])
            .unwrap();

        assert_eq!(
            result
                .profile
                .expect("a profiled initialization reports it")
                .items
                .iter()
                .map(|applied| (applied.id.as_str(), applied.status))
                .collect::<Vec<_>>(),
            vec![
                ("jit-default", ProfileApplicationStatus::Applied),
                ("workflow", ProfileApplicationStatus::Applied),
            ]
        );
        // Both packages reached the published repository: the dependency's own
        // vocabulary and the named package's asset.
        assert!(repo.path().join("docs/workflow.txt").is_file());
        let published = fs::read(repo.path().join(".jit/config.toml")).unwrap();
        let types = crate::declarations::assemble_configuration(Some(&published), None, None)
            .unwrap()
            .type_hierarchy
            .expect("the published configuration declares a type hierarchy")
            .types;
        let missing = dependency
            .model()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::MapEntry {
                    target: MapEntryTarget::TypeHierarchyTypes,
                    identity,
                    ..
                } => Some(identity.as_str()),
                _ => None,
            })
            .filter(|identity| !types.contains_key(*identity))
            .collect::<Vec<_>>();
        assert_eq!(
            missing,
            Vec::<&str>::new(),
            "each entry is a type the dependency declares that the published configuration lacks"
        );
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
        drop(retained);
    }

    #[test]
    fn test_fresh_profile_init_creates_no_repository_when_a_dependency_cannot_be_resolved() {
        let repo = TempDir::new().unwrap();
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["absent-base"],
        );

        let error = file_executor(repo.path())
            .unwrap()
            .initialize_selected(repo.path(), &[repo.path().join("packages/workflow")])
            .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("workflow"), "{message}");
        assert!(message.contains("absent-base"), "{message}");
        assert!(
            !repo.path().join(".jit").exists(),
            "the closure is resolved before a repository is created"
        );
    }

    #[test]
    fn test_fresh_profile_init_publishes_complete_valid_repo_without_git() {
        let repo = TempDir::new().unwrap();
        let executor = file_executor(repo.path()).unwrap();

        let result = executor
            .initialize_selected(
                repo.path(),
                &[crate::test_utils::stage_repository_packages(
                    repo.path(),
                    "jit-dogfood",
                )],
            )
            .unwrap();

        let applied = result
            .profile
            .expect("a profiled initialization reports it");
        assert_eq!(
            applied.requested().unwrap().status,
            ProfileApplicationStatus::Applied
        );
        assert!(repo.path().join(".jit/index.json").is_file());
        assert!(repo
            .path()
            .join(".agents/skills/jit-manage/SKILL.md")
            .is_file());
        assert_eq!(
            fs::read_to_string(repo.path().join(".jit/events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1,
            "initialization appends one aggregate lifecycle event"
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
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    #[test]
    fn test_reinit_profiled_over_existing_root_is_idempotent_unchanged() {
        let repo = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
        )
        .unwrap();
        let before_events = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

        // A second `jit init` is a fresh process that re-discovers the layout over
        // the now-published root; reuse of the first executor's absent-root layout
        // would be a test artifact, not the real re-init path.
        let reinit = file_executor(repo.path()).unwrap();
        let again = reinit
            .initialize_selected(
                repo.path(),
                &[PathBuf::from(format!(
                    "{}/jit-dogfood",
                    crate::test_utils::PROFILE_PACKAGE_SOURCES
                ))],
            )
            .unwrap();

        assert_eq!(
            again.profile.unwrap().requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
            before_events
        );
    }

    #[test]
    fn test_single_profile_reinit_attributes_coupled_schema_repair() {
        let repo =
            crate::test_utils::profiled_repository_fixture("jit-default", "packages").unwrap();
        let schema = repo
            .path()
            .join(".jit/schemas/default-namespace-registry.json");
        let expected_schema = fs::read(&schema).unwrap();
        fs::remove_file(&schema).unwrap();
        let events_path = repo.path().join(".jit/events.jsonl");
        let events_before = fs::read_to_string(&events_path).unwrap();

        let repaired = file_executor(repo.path())
            .unwrap()
            .initialize_selected(repo.path(), &[PathBuf::from("packages/jit-default")])
            .unwrap();

        assert_eq!(
            repaired.profile.unwrap().requested().unwrap().status,
            ProfileApplicationStatus::Applied,
            "the profile whose default-rule authority repaired the schema is applied"
        );
        assert_eq!(fs::read(&schema).unwrap(), expected_schema);
        assert_eq!(
            fs::read_to_string(&events_path).unwrap().lines().count(),
            events_before.lines().count() + 1,
            "a coupled-only repair keeps the current per-profile audit event"
        );

        let again = file_executor(repo.path())
            .unwrap()
            .initialize_selected(repo.path(), &[PathBuf::from("packages/jit-default")])
            .unwrap();
        assert_eq!(
            again.profile.unwrap().requested().unwrap().status,
            ProfileApplicationStatus::Unchanged
        );
        assert_eq!(
            fs::read_to_string(events_path).unwrap().lines().count(),
            events_before.lines().count() + 1
        );
    }

    #[test]
    fn test_profiled_init_detects_projection_source_outside_final_capture() {
        let repo = TempDir::new().unwrap();
        let executor = file_executor(repo.path()).unwrap();
        executor
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        let config_path = repo.path().join(".jit/config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        let project_name =
            crate::declarations::assemble_configuration(Some(config.as_bytes()), None, None)
                .unwrap()
                .project
                .and_then(|project| project.name)
                .unwrap();
        // `race` is a custom kind; the default package does not ship it.
        config.push_str(
            "\n[item_kinds.race]\nsection = \"race\"\nid-pattern = \"R-[0-9]+\"\n\
             markers = []\nlink-namespaces = []\nscope = \"project\"\n\
             source-of-truth = \"markdown-first\"\nsource = \"RACE.md\"\n\
             [projection.race]\nkind = \"race\"\nmode = \"separate-file\"\n\
             target = \"RACE.generated.md\"\nstyle = \"id-anchor\"\n",
        );
        // The package's own projections and templates name the kinds and the
        // container type its dependency declares, and this case composes the
        // package alone, so the repository declares them itself.
        config.push_str("\n[type_hierarchy]\ntypes = { epic = 2 }\n");
        config.push_str(&crate::test_utils::shipped_item_kind_tables(&[
            "invariant",
            "rule",
            "gate",
        ]));
        fs::write(
            repo.path().join("RACE.md"),
            "## Race\n\n- **R-1** — recaptured\n",
        )
        .unwrap();

        // Read from inside the worktree it is prepared against, as an adopter
        // stages it.
        let package = crate::test_utils::capture_repository_package(
            "jit-dogfood",
            &repo.path().join("profiles/jit-dogfood"),
        )
        .unwrap();
        let profile = super::super::profile::profile_application_input(
            &crate::profile::resolve_package(&package, &Default::default()).unwrap(),
            &executor.require_layout().unwrap(),
        )
        .unwrap();
        let proposed_config = config.as_bytes().to_vec();
        let scaffold =
            InitializationScaffold::from_config(config, project_name, Some(profile)).unwrap();
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let image = executor
            .capture_proposed_base(
                session.as_mut(),
                &Default::default(),
                &scaffold
                    .delta_paths()
                    .unwrap()
                    .into_iter()
                    .filter(|path| path != &VirtualPath::EVENTS)
                    .collect::<Vec<_>>(),
                None,
            )
            .unwrap()
            .unwrap();
        let proposed = apply_overlay(
            &image,
            [(
                VirtualPath::data("config.toml").unwrap(),
                Some(proposed_config),
            )],
        )
        .unwrap();
        let source = VirtualPath::worktree("RACE.md").unwrap();

        assert!(!proposed.capture_spec().contains_path(&source));
        assert!(scaffold
            .profile_capture_closure(&proposed)
            .unwrap()
            .contains(&source));
    }

    #[test]
    fn test_reinit_refreshes_default_rule_membership_and_type_schema() {
        const SQUAD_NAMESPACE: &str = "\
\n[namespaces.squad]\n\
description = \"Owning squad\"\n\
unique = true\n";

        let repo = TempDir::new().unwrap();
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        // Re-initialization refreshes what the repository's own registry
        // generates, so the repository declares that registry first.
        declare_before_init(repo.path(), &taxonomy.config_fragment());
        file_executor(repo.path())
            .unwrap()
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        let config_path = repo.path().join(".jit/config.toml");
        let rules_path = repo.path().join(".jit/rules.toml");
        let leaf_type = taxonomy.type_at_level(4);
        let config = fs::read_to_string(&config_path).unwrap().replacen(
            &format!("{leaf_type} = 4 }}"),
            &format!("{leaf_type} = 4, planning = 3 }}"),
            1,
        ) + SQUAD_NAMESPACE;
        fs::write(&config_path, &config).unwrap();
        let authored_rule = "\
\n[[rules]]\n\
name = \"custom-shape\"\n\
# authored content survives re-init\n\
severity = \"warn\"\n\
assert = { require-section = { heading = \"Goals\" } }\n";
        let authored_rules = fs::read_to_string(&rules_path).unwrap().replacen(
            crate::repository_state::rules_file_header(),
            "# authored header survives re-init\n\n",
            1,
        ) + authored_rule;
        fs::write(&rules_path, authored_rules).unwrap();

        file_executor(repo.path())
            .unwrap()
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        let refreshed = fs::read_to_string(&rules_path).unwrap();
        assert!(refreshed.starts_with("# authored header survives re-init\n\n"));
        assert!(refreshed.contains("name = \"namespace-unique-squad\""));
        assert!(refreshed.contains("# authored content survives re-init"));
        assert!(fs::read_to_string(
            repo.path()
                .join(".jit/schemas/default-type-hierarchy-known.json")
        )
        .unwrap()
        .contains("planning"));

        fs::write(&config_path, config.replace(SQUAD_NAMESPACE, "")).unwrap();
        file_executor(repo.path())
            .unwrap()
            .profiles()
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();
        let refreshed = fs::read_to_string(&rules_path).unwrap();
        assert!(!refreshed.contains("name = \"namespace-unique-squad\""));
        assert!(refreshed.contains("# authored content survives re-init"));
    }

    #[test]
    fn test_concurrent_fresh_profile_init_publishes_one_coherent_repository() {
        let repo = Arc::new(TempDir::new().unwrap());
        // Staged once, before either initialization starts: the race under test
        // is between two initializations, not between two stagings.
        let location = Arc::new(crate::test_utils::stage_repository_packages(
            repo.path(),
            "jit-dogfood",
        ));
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let repo = Arc::clone(&repo);
                let location = Arc::clone(&location);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let executor = file_executor(repo.path()).unwrap();
                    barrier.wait();
                    executor.initialize_selected(repo.path(), &[PathBuf::from(location.as_path())])
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        let winner = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect::<Vec<_>>();
        assert_eq!(winner.len(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        file_executor(repo.path())
            .unwrap()
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
        let events = fs::read_to_string(repo.path().join(".jit/events.jsonl")).unwrap();
        assert_eq!(
            events.lines().count(),
            1,
            "only one aggregate lifecycle event reached the event log"
        );
    }

    #[test]
    fn test_profiled_init_runs_each_selection_stage_once_per_invocation() {
        let repo = TempDir::new().unwrap();
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/base"),
            "base",
            &[],
        );
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["base"],
        );
        let executor = file_executor(repo.path()).unwrap();
        let location = repo.path().join("packages/workflow");
        let closure = crate::test_utils::selected_closure(
            &executor,
            &ProfilePackage::from_directory(&location).unwrap(),
        )
        .unwrap()
        .len();
        let selectors = [location];

        let (initialized, stages) = crate::profile::selection_probe::observe(|| {
            executor.initialize_selected(repo.path(), &selectors)
        });

        initialized.unwrap();
        assert_eq!(
            stages,
            crate::profile::selection_probe::once_per_invocation(closure)
        );
    }
}
