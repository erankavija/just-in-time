use super::{
    profile::validate_variable_inputs, with_mutation_session, CommandExecutor, SessionStep,
};
use crate::config::{slugify_project_name, ProjectName};
use crate::profile::{
    build_profile_claims_from_resolved, resolve_package, ProfileApplicationStatus,
    ProfileApplyResult, ProfileComposedApplyResult, ProfilePackage, VariableInputs,
};
use crate::repository_state::{
    apply_overlay, derive_materialization, ExpectedPreimage, GitattributesClaim,
    GitattributesStatus, InitializationScaffold, MaterializationPlan, MaterializationRequest,
    ProfileApplicationInput, RepositoryAction, VirtualPath,
};
use crate::storage::JsonFileStorage;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Keep initialization's dependency work set unique while retaining one result
/// for every selected root occurrence. A repeated root is an idempotent second
/// observation of the same durable application, not a second publication.
fn init_profile_results(
    applied: &[ProfileApplyResult],
    roots: &[ProfilePackage],
) -> Result<ProfileComposedApplyResult> {
    let root_ids = roots
        .iter()
        .map(|root| root.model().id.as_str())
        .collect::<HashSet<_>>();
    let applied_by_id = applied
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect::<HashMap<_, _>>();
    let mut results = applied
        .iter()
        .filter(|result| !root_ids.contains(result.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut occurrences = HashMap::<&str, usize>::new();
    for root in roots {
        let id = root.model().id.as_str();
        let Some(source) = applied_by_id.get(id) else {
            anyhow::bail!("initialized root '{id}' has no applied result");
        };
        let occurrence = occurrences.entry(id).or_default();
        let first = *occurrence == 0;
        *occurrence += 1;
        let mut result = (*source).clone();
        if !first {
            result.status = ProfileApplicationStatus::Unchanged;
            result.transaction_id = None;
            result.warnings.clear();
            result.targets.clear();
            result.contributions.clear();
        }
        results.push(result);
    }
    Ok(ProfileComposedApplyResult::new(results))
}

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

impl CommandExecutor<JsonFileStorage> {
    /// Atomically complete neutral initialization and apply the selected profiles.
    ///
    /// An absent data directory is published through the recovered session's
    /// staged-root machinery; an existing partial repository fills only its missing
    /// neutral scaffold state and the profile's changes in one delta.
    pub fn initialize_profiled_repository(
        &self,
        repo_dir: &Path,
        selectors: &[super::profile::ProfileSelector],
    ) -> Result<FreshInitResult> {
        self.run_initialization(repo_dir, selectors, &VariableInputs::default())
    }

    /// Initialize and apply profiles after loading command-bound variable
    /// inputs.
    pub fn initialize_profiled_repository_from_sources(
        &self,
        repo_dir: &Path,
        selectors: &[super::profile::ProfileSelector],
        options: &super::profile::ProfileVariableOptions,
    ) -> Result<FreshInitResult> {
        self.initialize_from_sources(repo_dir, selectors, options)
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
        selectors: Option<&[super::profile::ProfileSelector]>,
    ) -> Result<FreshInitResult> {
        self.run_initialization(
            repo_dir,
            selectors.unwrap_or_default(),
            &VariableInputs::default(),
        )
    }

    /// Initialize a repository after loading command-bound variable inputs.
    pub fn initialize_fresh_repository_from_sources(
        &self,
        repo_dir: &Path,
        selectors: Option<&[super::profile::ProfileSelector]>,
        options: &super::profile::ProfileVariableOptions,
    ) -> Result<FreshInitResult> {
        self.initialize_from_sources(repo_dir, selectors.unwrap_or_default(), options)
    }

    fn initialize_from_sources(
        &self,
        repo_dir: &Path,
        selectors: &[super::profile::ProfileSelector],
        options: &super::profile::ProfileVariableOptions,
    ) -> Result<FreshInitResult> {
        let selected = self.resolve_profile_selectors(selectors)?;
        let packages = if selected.is_empty() {
            Vec::new()
        } else {
            self.resolve_profile_graph_for_mutation(&selected)?
                .selected_packages()
        };
        let inputs = super::profile::load_profile_variable_inputs(
            &packages,
            options.values_file.as_deref(),
            &options.assignments,
        )?;
        self.run_initialization(repo_dir, selectors, &inputs)
    }

    /// Capture the base under one recovered session, validate the proposed
    /// scaffold/profile overlay, and publish the complete initialization delta.
    ///
    /// A selected profile is resolved into its complete dependency closure
    /// before anything is published, so an unresolvable id, an unresolvable
    /// dependency, or a dependency cycle fails before a repository is created.
    /// The first package of that closure — the one nothing else in it depends
    /// on — is published together with the scaffold, because a package needs a
    /// repository to be applied to; the rest follow in closure order through
    /// the ordinary application and one aggregate lifecycle event.
    fn run_initialization(
        &self,
        repo_dir: &Path,
        selectors: &[super::profile::ProfileSelector],
        variable_inputs: &VariableInputs,
    ) -> Result<FreshInitResult> {
        let (roots, packages) = if selectors.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let selected = self.resolve_profile_selectors(selectors)?;
            let packages = self
                .resolve_profile_graph_for_mutation(&selected)?
                .selected_packages();
            (selected, packages)
        };
        validate_variable_inputs(&packages, variable_inputs)?;
        let contribution_context =
            self.profile_contribution_candidates(&packages, variable_inputs)?;
        let layout = self.require_layout()?;
        // Typed Git evidence is acquired once at the boundary (loop-invariant).
        let gitattributes = gitattributes_claim(&layout);
        // One MutationContext per operation, reused across probe/final finalize and
        // every retry so a composed lifecycle event's id/timestamp stay stable.
        let context = crate::repository_state::MutationContext::production();
        let mut result = with_mutation_session(
            self.storage(),
            &layout,
            "repository initialization",
            |session| {
                let (config, project_name) = self.resolve_init_config(&mut *session, repo_dir)?;
                let profiles = packages
                    .iter()
                    .map(|package| {
                        self.profile_input_with_inputs(package, variable_inputs)
                            .map(|input| input.with_contribution_context(&contribution_context))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let scaffold = InitializationScaffold::from_config(
                    config.clone(),
                    project_name.clone(),
                    None,
                )?
                .with_gitattributes(gitattributes.clone())
                .with_profiles(profiles);

                let mut extra_paths = scaffold.delta_paths()?;
                extra_paths.extend(crate::repository_state::profile_contribution_target_paths(
                    &contribution_context,
                )?);
                // Probe capture: the deliberately over-inclusive scaffold overlay yields
                // a base good enough to finalize the exact delta. That delta's overlay
                // is the AUTHORITATIVE proposed state — only the files init writes — so a
                // preserved `IfAbsent` file (e.g. an existing `rules.toml` referencing a
                // custom schema) is not shadowed by its neutral default in the closure.
                let probe_overrides = scaffold.overlay_overrides()?;
                let Some(mut probe) = self.capture_proposed_base(
                    &mut *session,
                    &probe_overrides,
                    &extra_paths,
                    None,
                )?
                else {
                    return Ok(SessionStep::Retry);
                };
                if scaffold.has_profiles() {
                    extra_paths.extend(scaffold.profile_capture_closure(&probe)?);
                    let Some(expanded) = self.capture_proposed_base(
                        &mut *session,
                        &probe_overrides,
                        &extra_paths,
                        None,
                    )?
                    else {
                        return Ok(SessionStep::Retry);
                    };
                    if !expanded.has_stable_overlap(&probe) {
                        return Ok(SessionStep::Retry);
                    }
                    probe = expanded;
                }
                let delta_overlay = super::validation_overlay(
                    derive_materialization(
                        &probe,
                        MaterializationRequest::Initialize {
                            scaffold: &scaffold,
                            context: &context,
                        },
                    )?
                    .delta(),
                );

                // Re-capture the base with the exact write set so the validation closure
                // and the delta's preimages come from one coherent image, then finalize,
                // validate, and publish under the same held session.
                let Some(base) =
                    self.capture_proposed_base(&mut *session, &delta_overlay, &extra_paths, None)?
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
                    MaterializationRequest::Initialize {
                        scaffold: &scaffold,
                        context: &context,
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
                let proposed = apply_overlay(&base, super::validation_overlay(plan.delta()))?;
                let validation = crate::validation::repository::validate_repository(&proposed)
                    .map_err(init_validation_error)?;
                if validation.rule_report.has_errors() {
                    anyhow::bail!(
                        "repository initialization produced {} validation error finding(s)",
                        validation.rule_report.error_count()
                    );
                }
                let gitattributes = scaffold.gitattributes_status(&base)?;
                let (created_paths, modified_paths) = init_response_paths(&plan, gitattributes)?;

                let profile = if packages.is_empty() {
                    None
                } else {
                    Some(ProfileComposedApplyResult::new(
                        packages
                            .iter()
                            .map(|package| {
                                let changed = plan.applied_profiles().contains(&package.model().id);
                                super::profile::applied_profile_result(
                                    &plan,
                                    package,
                                    &layout,
                                    if changed {
                                        ProfileApplicationStatus::Applied
                                    } else {
                                        ProfileApplicationStatus::Unchanged
                                    },
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
            .map(|applied| init_profile_results(&applied.profiles, &roots))
            .transpose()?;
        Ok(result)
    }

    /// Resolve the effective configuration bytes and project identity: an existing
    /// `config.toml` is preserved (its authored project name kept), otherwise the
    /// structural minimum is rendered under a slug of the repository directory
    /// name.
    fn resolve_init_config(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        repo_dir: &Path,
    ) -> Result<(String, ProjectName)> {
        use crate::repository_state::{CaptureBudget, CaptureSpec};
        let budget = CaptureBudget {
            max_listings: 0,
            max_bytes: 64 * 1024 * 1024,
            max_depth: 6,
        };
        let spec = CaptureSpec::phase_one([VirtualPath::CONFIG], budget)?;
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
                let parsed = crate::declarations::parse_configuration(config.as_bytes())
                    .context("Failed to parse existing init configuration")?;
                let project_name = parsed.project_name().cloned().unwrap_or(generated_name);
                Ok((config, project_name))
            }
            None => Ok((
                crate::repository_state::render_repo_config(
                    &crate::repository_state::structural_minimum_config(),
                    &generated_name,
                ),
                generated_name,
            )),
        }
    }

    /// Capture the neutral proposed base and convert one resolved package into
    /// complete neutral claims and provenance metadata.
    ///
    /// The base is captured under the held session, overlaid with the neutral
    /// scaffold state init will publish (schemas always; an existing
    /// config/gates/rules preserved), so the profile merges over the proposed neutral
    /// repository. Returns `Ok(None)` on a retryable capture conflict so the caller
    /// re-attempts the whole initialization.
    #[cfg(test)]
    fn profile_input(&self, package: &ProfilePackage) -> Result<ProfileApplicationInput> {
        self.profile_input_with_inputs(package, &VariableInputs::default())
    }

    fn profile_input_with_inputs(
        &self,
        package: &ProfilePackage,
        variable_inputs: &VariableInputs,
    ) -> Result<ProfileApplicationInput> {
        super::profile::reject_reserved_application_targets(
            package.hashes().targets.keys().map(String::as_str),
        )?;
        let metadata = package.model();
        let record_path = VirtualPath::data(format!("profiles/{}.json", metadata.id))?;
        let layout = self.require_layout()?;
        let resolved = resolve_package(
            package,
            &variable_inputs.for_declarations(&package.model().variables),
        )?;
        let claims = build_profile_claims_from_resolved(&resolved, &layout, false)?;
        Ok(ProfileApplicationInput {
            id: metadata.id.clone(),
            version: metadata.version.clone(),
            compatible_jit: metadata.compatible_jit.clone(),
            package_hash: package.hashes().package.clone(),
            variables: resolved.variables().clone(),
            target_hashes: resolved.target_hashes()?,
            origin: super::profile::package_origin(package, &layout)?,
            contribution_context: claims.contributions.clone(),
            claims,
            record_path,
        })
    }
}

/// Project the stable public init report from the exact finalized plan.
///
/// The response intentionally reports only the user-facing scaffold files plus
/// the worktree `.gitattributes` claim. Derived schemas, projections, and
/// other materialization refreshes remain internal transaction details. Every
/// reported creation or modification is nevertheless proven by the finalized
/// action's expected preimage; no ambient filesystem probe participates.
fn init_response_paths(
    plan: &MaterializationPlan,
    gitattributes: GitattributesStatus,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut created = [
        (".jit/index.json", "index.json"),
        (".jit/gates.toml", "gates.toml"),
        (".jit/invariants.toml", "invariants.toml"),
        (".jit/events.jsonl", "events.jsonl"),
        (".jit/config.toml", "config.toml"),
        (".jit/rules.toml", "rules.toml"),
    ]
    .into_iter()
    .map(|(reported, relative)| {
        let path = VirtualPath::data(relative)?;
        Ok(plan
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &path)
            .is_some_and(|action| {
                matches!(
                    action,
                    RepositoryAction::WriteFile {
                        expected: ExpectedPreimage::Absent,
                        ..
                    }
                )
            })
            .then(|| reported.to_string()))
    })
    .collect::<Result<Vec<_>>>()?
    .into_iter()
    .flatten()
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

/// Preserve the startup-facing too-new-format error while deriving it from the
/// same closed proposed image that init validates. Other structural failures keep
/// their original validation context.
fn init_validation_error(
    failure: crate::validation::repository::RepositoryValidationFailure,
) -> anyhow::Error {
    let error = failure.into_error();
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
        crate::storage::RepositoryFormatTooNewError::new(found, supported).into()
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
    let path = relative.to_string_lossy();
    let mut escaped = String::with_capacity(path.len() + "/events.jsonl".len());
    for ch in path.chars().chain("/events.jsonl".chars()) {
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
    use crate::commands::{ProfileSelector, ProfileVariableOptions};
    use crate::profile::ProfileOrigin;
    use crate::repository_state::{AppliedProfileRecord, Contribution, MapEntryTarget};
    use crate::storage::{discover_repository_layout, IssueStore, RepositoryStateStore};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    /// Count only the recoverable publication boundary, not read-only captures.
    struct PublicationCounter(AtomicUsize);

    impl PublicationCounter {
        fn new() -> Arc<Self> {
            Arc::new(Self(AtomicUsize::new(0)))
        }

        fn count(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl crate::storage::TransactionFailureInjector for PublicationCounter {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
            if point == &crate::storage::TransactionFailurePoint::RepositoryBeforeControlCreation {
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
        assert_repo_valid(repo.path());
    }

    /// The `invariant` item kind, declared by the repository itself.
    ///
    /// A kind is a declaration like any other, so a repository that applies no
    /// package obtains one by writing it. This is the smallest declaration that
    /// makes `@/invariant/<id>` resolve against `.jit/invariants.toml`.
    const INVARIANT_KIND: &str = "\n[item_kinds.invariant]\n\
section = \"success_criteria\"\n\
id-pattern = \"[a-z][a-z0-9-]*\"\n\
markers = []\n\
link-namespaces = []\n\
scope = \"project\"\n\
source = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }\n\
source-of-truth = \"registry-first\"\n";

    /// The `rule` and `gate` item kinds, declared by the repository itself, for
    /// a case whose package contributes projections naming them.
    const RULE_AND_GATE_KINDS: &str = "\n[item_kinds.rule]\n\
section = \"success_criteria\"\n\
id-pattern = \"[a-z][a-z0-9-]*\"\n\
markers = []\n\
link-namespaces = []\n\
scope = \"project\"\n\
source = { toml = \".jit/rules.toml\", table = \"rules\", id-field = \"name\", text-field = \"description\" }\n\
source-of-truth = \"registry-first\"\n\
\n[item_kinds.gate]\n\
section = \"success_criteria\"\n\
id-pattern = \"[a-z][a-z0-9-]*\"\n\
markers = []\n\
link-namespaces = []\n\
scope = \"project\"\n\
source = { toml = \".jit/gates.toml\", table = \"gates\", id-field = \"key\", text-field = \"description\" }\n\
source-of-truth = \"registry-first\"\n";

    /// Write `config_toml` as the repository's configuration before it is
    /// initialized, which initialization preserves.
    fn declare_before_init(repo: &Path, config_toml: &str) {
        fs::create_dir_all(repo.join(".jit")).unwrap();
        fs::write(repo.join(".jit/config.toml"), config_toml).unwrap();
    }

    #[test]
    fn test_fresh_init_creates_the_invariant_registry_present_and_empty() {
        let repo = TempDir::new().unwrap();
        declare_before_init(repo.path(), INVARIANT_KIND);
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());

        let result = executor
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
            .list_items(Some("invariant"))
            .unwrap()
            .items
            .is_empty());
        assert_repo_valid(repo.path());
    }

    #[test]
    fn test_fresh_init_registry_resolves_an_authored_invariant_without_a_package() {
        let repo = TempDir::new().unwrap();
        declare_before_init(repo.path(), INVARIANT_KIND);
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());
        executor
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

        let resolved = executor_with_layout(&storage, repo.path())
            .show_item("@/invariant/authored-here")
            .unwrap();
        assert_eq!(resolved.item.self_id, "authored-here");
        assert!(resolved.issue_full_id.is_none());
        assert_repo_valid(repo.path());
    }

    /// The checked-in fixture tree every composition case below stages copies
    /// of, each under a rewritten manifest declaring its own id.
    fn composition_package() -> std::path::PathBuf {
        crate::test_utils::profile_package_fixture("planner-asset-only")
    }

    fn write_embedded_record(repo: &TempDir, id: &str) {
        let record = AppliedProfileRecord::new(
            id.try_into().expect("fixture profile id is canonical"),
            "1.0.0",
            "*",
            ProfileOrigin::Embedded,
            "a".repeat(64),
            Default::default(),
            Default::default(),
        );
        fs::create_dir_all(repo.path().join(".jit/profiles")).unwrap();
        fs::write(
            repo.path().join(format!(".jit/profiles/{id}.json")),
            record.to_bytes().unwrap(),
        )
        .unwrap();
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
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
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

        let result = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
            )
            .unwrap();

        // One initialization, both packages, the dependency first.
        let profile = result
            .profile
            .expect("a profiled initialization reports it");
        assert_eq!(
            profile
                .profiles
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
        assert!(repo.path().join(".jit/profiles/base.json").is_file());
        assert!(repo.path().join(".jit/profiles/workflow.json").is_file());
    }

    #[test]
    fn test_profiled_init_publishes_its_complete_closure_through_one_transaction() {
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
        let storage = JsonFileStorage::with_repository_state_failures(
            repo.path().join(".jit"),
            publications.clone(),
        );

        let result = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
            )
            .unwrap();

        assert_eq!(publications.count(), 1);
        assert_eq!(
            result
                .profile
                .expect("profiled init reports its closure")
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "workflow"]
        );
    }

    #[test]
    fn test_profiled_init_from_sources_keeps_embedded_provenance_out_of_mutating_graph_resolution()
    {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        executor_with_layout(&storage, repo.path())
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();
        write_embedded_record(&repo, "jit-dogfood");
        let location = repo.path().join("packages/later");
        crate::test_utils::write_package_declaring(&composition_package(), &location, "later", &[]);

        let result = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository_from_sources(
                repo.path(),
                &[ProfileSelector::path(&location)],
                &ProfileVariableOptions::default(),
            )
            .expect("profiled init treats Embedded provenance as ownership only");

        assert_eq!(
            result
                .profile
                .expect("profiled init reports the package")
                .profiles
                .len(),
            1
        );
        assert!(repo.path().join(".jit/profiles/later.json").is_file());
    }

    #[test]
    fn test_fresh_profile_init_records_shared_owners_across_its_closure() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        composition_package_with_namespace(
            &repo,
            "packages/base",
            "base",
            &[],
            "Shared initialization definition.",
        );
        composition_package_with_namespace(
            &repo,
            "packages/workflow",
            "workflow",
            &["base"],
            "Shared initialization definition.",
        );

        executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
            )
            .unwrap();

        let shared_claims = ["base", "workflow"]
            .into_iter()
            .map(|id| {
                let record: AppliedProfileRecord = serde_json::from_slice(
                    &fs::read(repo.path().join(format!(".jit/profiles/{id}.json"))).unwrap(),
                )
                .unwrap();
                let semantic = record
                    .claims
                    .into_iter()
                    .filter(|claim| {
                        matches!(
                            claim.identity,
                            crate::repository_state::AppliedProfileClaimIdentity::Semantic { .. }
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(semantic.len(), 1);
                semantic.into_iter().next().unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(shared_claims[0], shared_claims[1]);
    }

    #[test]
    fn test_fresh_profile_init_refuses_a_conflicting_closure_before_scaffolding() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
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
            &["base"],
            "Workflow initialization definition.",
        );

        let error = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
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
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let location = repo.path().join("packages/workflow");
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &location,
            "workflow",
            &[],
        );

        let result = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[
                    ProfileSelector::path(&location),
                    ProfileSelector::path(&location),
                ],
            )
            .unwrap();

        let profiles = result
            .profile
            .expect("profiled init reports its roots")
            .profiles;
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
    fn test_fresh_profile_init_records_every_package_of_the_selected_closure() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());
        // The set initialization has to apply is the selected package's whole
        // dependency closure. It is named from the staged package itself, so
        // the two sides reach it by different routes.
        let location = crate::test_utils::stage_repository_packages(repo.path(), "jit-dogfood");
        let workflow = crate::profile::ProfilePackage::from_directory(&location).unwrap();
        let closure = executor.resolve_profile_closure(&workflow).unwrap();

        let result = executor
            .initialize_fresh_repository(repo.path(), Some(&[ProfileSelector::path(&location)]))
            .unwrap();

        let ids = closure
            .iter()
            .map(|package| package.model().id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            result
                .profile
                .expect("a profiled initialization reports it")
                .profiles
                .iter()
                .map(|applied| applied.id.as_str())
                .collect::<Vec<_>>(),
            ids,
            "initialization applies the whole set the named profile resolves to"
        );
        let unrecorded = ids
            .iter()
            .filter(|id| {
                !repo
                    .path()
                    .join(format!(".jit/profiles/{id}.json"))
                    .is_file()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            unrecorded,
            Vec::<&&str>::new(),
            "each entry is an applied package the created repository has no record of"
        );
        assert_repo_valid(repo.path());
    }

    #[test]
    fn test_fresh_profile_init_applies_a_declared_dependency_under_a_retained_session() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
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
        let layout = discover_repository_layout(repo.path(), storage.root()).unwrap();
        // The shape every `jit init` runs in: startup recovers and retains its
        // session, and initialization publishes and applies inside it.
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();

        let result = CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
            )
            .unwrap();

        assert_eq!(
            result
                .profile
                .expect("a profiled initialization reports it")
                .profiles
                .iter()
                .map(|applied| (applied.id.as_str(), applied.status))
                .collect::<Vec<_>>(),
            vec![
                ("jit-default", ProfileApplicationStatus::Applied),
                ("workflow", ProfileApplicationStatus::Applied),
            ]
        );
        // Both packages reached the published repository: the dependency's own
        // vocabulary, the named package's asset, and a provenance record each.
        let record: AppliedProfileRecord = serde_json::from_slice(
            &fs::read(repo.path().join(".jit/profiles/jit-default.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record.origin,
            ProfileOrigin::Directory(
                crate::repository_state::RootRelativePath::parse("packages/jit-default").unwrap()
            )
        );
        assert!(repo.path().join(".jit/profiles/workflow.json").is_file());
        assert!(repo.path().join("docs/workflow.txt").is_file());
        let published = fs::read(repo.path().join(".jit/config.toml")).unwrap();
        let types = crate::declarations::parse_configuration(&published)
            .unwrap()
            .hierarchy
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
        assert_repo_valid(repo.path());
        drop(retained);
    }

    #[test]
    fn test_fresh_profile_init_creates_no_repository_when_a_dependency_cannot_be_resolved() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        crate::test_utils::write_package_declaring(
            &composition_package(),
            &repo.path().join("packages/workflow"),
            "workflow",
            &["absent-base"],
        );

        let error = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::path(repo.path().join("packages/workflow"))],
            )
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
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());

        let result = executor
            .initialize_fresh_repository(
                repo.path(),
                Some(&[ProfileSelector::path(
                    crate::test_utils::stage_repository_packages(repo.path(), "jit-dogfood"),
                )]),
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
        assert_repo_valid(repo.path());
    }

    #[test]
    fn test_reinit_profiled_over_existing_root_is_idempotent_unchanged() {
        let repo = crate::test_utils::profiled_repository_fixture(
            "jit-dogfood",
            crate::test_utils::PROFILE_PACKAGE_SOURCES,
            None,
        )
        .unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
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
                &[ProfileSelector::id("jit-dogfood").unwrap()],
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
        assert_eq!(
            fs::read(repo.path().join(".jit/profiles/jit-dogfood.json")).unwrap(),
            compact_record
        );
    }

    #[test]
    fn test_single_profile_reinit_attributes_coupled_schema_repair() {
        let repo = crate::test_utils::profiled_repository_fixture("jit-default", "packages", None)
            .unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let schema = repo
            .path()
            .join(".jit/schemas/default-namespace-registry.json");
        let expected_schema = fs::read(&schema).unwrap();
        fs::remove_file(&schema).unwrap();
        let events_path = repo.path().join(".jit/events.jsonl");
        let events_before = fs::read_to_string(&events_path).unwrap();

        let repaired = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::id("jit-default").unwrap()],
            )
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

        let again = executor_with_layout(&storage, repo.path())
            .initialize_profiled_repository(
                repo.path(),
                &[ProfileSelector::id("jit-default").unwrap()],
            )
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
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let executor = executor_with_layout(&storage, repo.path());
        executor
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();

        let config_path = repo.path().join(".jit/config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        let project_name = crate::declarations::parse_configuration(config.as_bytes())
            .unwrap()
            .project_name()
            .cloned()
            .unwrap();
        config.push_str(
            "\n[item_kinds.race]\nsection = \"race\"\nid-pattern = \"R-[0-9]+\"\n\
             markers = []\nlink-namespaces = []\nscope = \"project\"\n\
             source-of-truth = \"markdown-first\"\nsource = \"RACE.md\"\n\
             [projection.race]\nkind = \"race\"\nmode = \"separate-file\"\n\
             target = \"RACE.generated.md\"\nstyle = \"id-anchor\"\n",
        );
        // The package's own projections name the kinds its dependency declares,
        // and this case composes the package alone, so the repository declares
        // those kinds itself.
        config.push_str(INVARIANT_KIND);
        config.push_str(RULE_AND_GATE_KINDS);
        fs::write(
            repo.path().join("RACE.md"),
            "## Race\n\n- **R-1** — recaptured\n",
        )
        .unwrap();

        // Read from inside the worktree it is prepared against, because the
        // preparation records the package's worktree-relative location.
        let package = crate::test_utils::capture_repository_package(
            "jit-dogfood",
            &repo.path().join("profiles/jit-dogfood"),
        )
        .unwrap();
        let profile = executor.profile_input(&package).unwrap();
        let proposed_config = config.as_bytes().to_vec();
        let scaffold =
            InitializationScaffold::from_config(config, project_name, Some(profile)).unwrap();
        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage().open_mutation_session(layout).unwrap();
        let image = executor
            .capture_proposed_base(
                session.as_mut(),
                &Default::default(),
                &scaffold.delta_paths().unwrap(),
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
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        executor_with_layout(&storage, repo.path())
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

        executor_with_layout(&storage, repo.path())
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
        executor_with_layout(&storage, repo.path())
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
                    let storage = JsonFileStorage::new(repo.path().join(".jit"));
                    let executor = executor_with_layout(&storage, repo.path());
                    barrier.wait();
                    executor.initialize_fresh_repository(
                        repo.path(),
                        Some(&[ProfileSelector::path(location.as_path())]),
                    )
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
        assert_repo_valid(repo.path());
        let events = fs::read_to_string(repo.path().join(".jit/events.jsonl")).unwrap();
        assert_eq!(
            events.lines().count(),
            1,
            "only one aggregate lifecycle event reached the event log"
        );
    }
}
