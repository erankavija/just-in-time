//! Repository opening for one CLI invocation: data-root selection, the
//! linked-checkout write decision, startup recovery and store validation, in
//! the order the invocation's correctness depends on.

use super::{CommandExecutor, DispatchMutationAnnotation};
use crate::domain::LinkedCheckoutWriteStance;
use crate::errors::{
    InvalidWorktreeRootError, LinkedCheckoutWriteRefusedError, RecoveryFailedError,
    WorktreeRootSource,
};
use crate::repository_state::{MutationContextAnnotation, RepositoryLayout};
use crate::storage::discovery::{discover_jit_dir, discover_recovery_jit_dir};
use crate::storage::repository_state_store::is_leftover_record_failure;
use crate::storage::{
    discover_repository_layout, JsonFileStorage, RepositoryStateStore,
    RetainedMutationSessionGuard, WorktreePaths,
};
use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

/// The invocation facts [`open_repository`] reads.
#[derive(Debug, Clone)]
pub struct RepositoryInvocation {
    pub current_dir: PathBuf,
    /// `JIT_DATA_DIR`, resolved against `current_dir`; it outranks every
    /// other selection.
    pub data_dir_override: Option<PathBuf>,
    /// `JIT_WORKTREE_ROOT`, captured once at the invocation boundary.
    pub worktree_root_override: Option<PathBuf>,
    /// `jit init` targets `<current_dir>/.jit` without searching upward.
    pub is_init: bool,
    /// The command opens a repository mutation session (recovery dispatch).
    pub opens_mutation_session: bool,
    /// The command opens its own session after claim coordination, so startup
    /// recovers through an unretained session.
    pub coordinates_claims_first: bool,
    /// The command runs over a store that may not be initialized.
    pub is_validation_exempt: bool,
    /// The command is `jit recover`, whose own work is applying the leftover
    /// redo record.
    pub is_recover: bool,
    pub write_override: Option<LinkedCheckoutWriteStance>,
}

/// A repository opened for dispatch.
///
/// Field order is drop order: the write-override annotation is uninstalled
/// before the retained session is released.
#[must_use = "dropping the repository releases its retained session"]
pub struct OpenedRepository {
    /// Installed when an explicit override is the only reason this invocation
    /// may write inside a linked checkout.
    pub write_override_annotation: Option<DispatchMutationAnnotation>,
    pub retained_session: Option<RetainedMutationSessionGuard>,
    pub executor: CommandExecutor<JsonFileStorage>,
    pub data_root: PathBuf,
    pub worktree_paths: WorktreePaths,
    pub layout: RepositoryLayout,
    /// Leftover redo records startup recovery applied.
    pub recovered_records: usize,
}

/// Open the repository `invocation` selects and return it ready for dispatch.
///
/// Committed recovery precedes root selection and policy for new work. A clean
/// store's write refusal precedes mutation locks and writes. A non-exempt command fails here when the
/// selected store is absent or uninitialized.
///
/// # Errors
///
/// Returns [`LinkedCheckoutWriteRefusedError`] when the effective stance
/// refuses the invocation, and [`RecoveryFailedError`] when `jit recover`
/// cannot read or apply the leftover redo record; otherwise the
/// layout-discovery, session-open or validation failure.
pub fn open_repository(invocation: RepositoryInvocation) -> Result<OpenedRepository> {
    let (data_root, non_git_worktree_root) = select_data_root(&invocation);

    let mut detected = None;
    let mut recovered_records = 0;
    crate::storage::repository_state_store::retry_optimistic_reads(
        "repository root selection",
        || {
            let attempt = (|| -> Result<OpenedRepository> {
                let storage = JsonFileStorage::new(&data_root);
                let recovery_failure = |error: anyhow::Error| {
                    if invocation.is_recover && is_leftover_record_failure(&error) {
                        RecoveryFailedError(error).into()
                    } else {
                        error
                    }
                };
                if invocation.opens_mutation_session {
                    recovered_records += storage
                        .recover_pending_records()
                        .map_err(|error| recovery_failure(error.into()))?
                        .recovered_count();
                }
                let observation =
                    crate::storage::repository_state_store::ConfigurationObservation::capture(
                        &data_root,
                    )?;
                let base = match &detected {
                    Some(paths) => paths,
                    None => detected.insert(
                        WorktreePaths::detect_for_data_root(&data_root, &non_git_worktree_root)
                            .context("failed to discover repository worktree layout")?,
                    ),
                };
                let guard_policy = invocation.opens_mutation_session && base.holds_linked_store();
                let worktree = observation
                    .worktree(invocation.worktree_root_override.is_none(), guard_policy)?;
                let selection = invocation
                    .worktree_root_override
                    .as_ref()
                    .map(|path| (WorktreeRootSource::Environment, path))
                    .or_else(|| {
                        worktree
                            .root
                            .as_ref()
                            .map(|path| (WorktreeRootSource::Configuration, path))
                    });
                let (worktree_paths, layout) = match selection {
                    Some((setting, supplied)) => {
                        let layout = validate_worktree_root(&data_root, setting, supplied)?;
                        (base.with_worktree_root(layout.worktree_root())?, layout)
                    }
                    None => (
                        base.clone(),
                        discover_repository_layout(&base.worktree_root, &data_root)
                            .context("failed to construct repository layout")?,
                    ),
                };
                let declared = worktree.write_policy();
                let write_override = linked_checkout_write_decision(
                    invocation.opens_mutation_session,
                    &worktree_paths,
                    declared,
                    invocation.write_override,
                )?;
                storage.configure_worktree_paths(&worktree_paths);
                storage.bind_selection(
                    observation,
                    invocation.worktree_root_override.is_some(),
                    guard_policy,
                )?;
                let retained_session = match (
                    invocation.opens_mutation_session,
                    invocation.coordinates_claims_first,
                ) {
                    (false, _) => None,
                    (true, true) => {
                        let session =
                            RepositoryStateStore::open_mutation_session(&storage, layout.clone())
                                .map_err(|error| recovery_failure(error.into()))?;
                        recovered_records += session.recovery_report().recovered_count();
                        None
                    }
                    (true, false) => {
                        let guard = storage
                            .open_and_retain_mutation_session(layout.clone())
                            .map_err(recovery_failure)?;
                        recovered_records += guard.recovery_report().recovered_count();
                        Some(guard)
                    }
                };
                let executor = CommandExecutor::new(storage.clone())
                    .with_layout(layout.clone())?
                    .with_worktree_paths(worktree_paths.clone())
                    .with_recovered_records(recovered_records);
                let write_override_annotation =
                    write_override.map(DispatchMutationAnnotation::install);
                if !invocation.is_validation_exempt {
                    storage.validate()?;
                }
                Ok(OpenedRepository {
                    write_override_annotation,
                    retained_session,
                    executor,
                    data_root: data_root.clone(),
                    worktree_paths,
                    layout,
                    recovered_records,
                })
            })();
            if attempt.as_ref().err().is_some_and(|error| {
                error.chain().any(|cause| {
                    matches!(
                        cause.downcast_ref::<crate::storage::RepositoryStateStoreError>(),
                        Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
                    )
                })
            }) {
                Ok(None)
            } else {
                Ok(Some(attempt))
            }
        },
    )
}

/// The data root and the worktree root used outside Git.
///
/// Discovery walks up like git's: it stops at the first ancestor holding
/// `.jit/` and never crosses a `.git` boundary. Absent a match it falls back to
/// `<current_dir>/.jit`, so validation's "repository not found" names a
/// sensible path.
pub(super) fn select_data_root(invocation: &RepositoryInvocation) -> (PathBuf, PathBuf) {
    let current_dir = &invocation.current_dir;
    let discovered_root = |discovered: Option<PathBuf>| {
        discovered.map_or_else(
            || (current_dir.join(".jit"), current_dir.clone()),
            |discovered| {
                let worktree_root = discovered
                    .parent()
                    .map_or_else(|| current_dir.clone(), Path::to_path_buf);
                (discovered, worktree_root)
            },
        )
    };
    match &invocation.data_dir_override {
        Some(custom_dir) => {
            let target = normalize_absolute_path(&current_dir.join(custom_dir));
            let worktree_root = discover_jit_dir(current_dir)
                .filter(|candidate| candidate == &target)
                .and_then(|candidate| candidate.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| current_dir.clone());
            (target, worktree_root)
        }
        None if invocation.is_init => (current_dir.join(".jit"), current_dir.clone()),
        None if invocation.opens_mutation_session => {
            discovered_root(discover_recovery_jit_dir(current_dir))
        }
        None => discovered_root(discover_jit_dir(current_dir)),
    }
}

/// Validate an explicitly supplied root without selecting a default or probing Git.
pub(super) fn validate_worktree_root(
    data_root: &Path,
    setting: WorktreeRootSource,
    supplied: &Path,
) -> Result<RepositoryLayout> {
    let resolved = normalize_absolute_path(&data_root.join(supplied));
    let invalid = |reason: String| InvalidWorktreeRootError {
        setting,
        supplied: supplied.to_path_buf(),
        resolved: resolved.clone(),
        reason,
    };
    if supplied.as_os_str().is_empty() {
        return Err(invalid("path is empty".into()).into());
    }
    crate::storage::repository_state_store::ensure_symlink_free_ancestry(
        &data_root.join(supplied),
        false,
    )
    .map_err(|error| invalid(error.to_string()))?;
    crate::storage::capability_fs::open_absolute_dir_nofollow(&resolved)
        .map_err(|error| invalid(error.to_string()))?;
    discover_repository_layout(&resolved, data_root)
        .map_err(|error| invalid(error.to_string()).into())
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    path.components()
        .fold(PathBuf::new(), |mut normalized, component| {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                other => normalized.push(other.as_os_str()),
            }
            normalized
        })
}

/// Decide whether this invocation may write inside the checkout it selected, and
/// what that permission owes the audit trail.
///
/// This is the whole linked-checkout write policy, evaluated once. It governs
/// exactly the invocations that open a repository mutation session over a store
/// held by a linked non-primary checkout according to the selected-root worktree
/// authority; every other invocation is outside the policy and answers `None`
/// without reading a declaration. The governed ones take one of three outcomes:
///
/// - the effective stance refuses, and the invocation is refused;
/// - the repository's own declaration permits it, and nothing is owed;
/// - an explicit per-invocation override turned the declared refusal into an
///   allowance, which is the one shape warranting an audit record — the returned
///   annotation's fields name the stance that *would have refused*.
///
/// Precedence between the two stances is never re-decided here: it comes from
/// [`LinkedCheckoutWriteStance::resolve`].
///
/// # Errors
///
/// Returns [`LinkedCheckoutWriteRefusedError`] when the effective stance refuses
/// the invocation, or an error when the repository's declared stance cannot be
/// read.
fn linked_checkout_write_decision(
    opens_mutation_session: bool,
    worktree_paths: &WorktreePaths,
    declared: LinkedCheckoutWriteStance,
    invocation_override: Option<LinkedCheckoutWriteStance>,
) -> Result<Option<MutationContextAnnotation>> {
    if !opens_mutation_session || !worktree_paths.holds_linked_store() {
        return Ok(None);
    }
    let effective = LinkedCheckoutWriteStance::resolve(declared, invocation_override);
    match effective {
        LinkedCheckoutWriteStance::Refuse => Err(LinkedCheckoutWriteRefusedError::new(
            worktree_paths.checkout_top_level().to_path_buf(),
            effective,
        )
        .into()),
        LinkedCheckoutWriteStance::Allow => Ok((declared == LinkedCheckoutWriteStance::Refuse)
            .then(
                || MutationContextAnnotation::LinkedCheckoutWriteOverridden {
                    checkout: worktree_paths.checkout_top_level().to_path_buf(),
                    declared_stance: declared,
                    invocation_override: effective,
                },
            )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RepositoryNotFoundError;
    use crate::test_utils::setup_test_repo;

    fn invocation(current_dir: &Path) -> RepositoryInvocation {
        RepositoryInvocation {
            current_dir: current_dir.to_path_buf(),
            data_dir_override: None,
            worktree_root_override: None,
            is_init: false,
            opens_mutation_session: false,
            coordinates_claims_first: false,
            is_validation_exempt: false,
            is_recover: false,
            write_override: None,
        }
    }

    #[test]
    fn test_open_repository_selects_configured_root_relative_to_data() {
        let (temp, _) = setup_test_repo().unwrap();
        let code = temp.path().join("code");
        std::fs::create_dir(&code).unwrap();
        std::fs::write(
            temp.path().join(".jit/config.toml"),
            "[worktree]\nroot = '../code'\n",
        )
        .unwrap();
        let opened = open_repository(invocation(temp.path())).unwrap();
        assert_eq!(opened.layout.worktree_root(), code);
    }

    #[test]
    fn test_open_repository_environment_overrides_config_with_absolute_or_data_relative_root() {
        let (temp, _) = setup_test_repo().unwrap();
        let code = temp.path().join("code");
        std::fs::create_dir(&code).unwrap();
        std::fs::write(
            temp.path().join(".jit/config.toml"),
            "[worktree]\nroot = 'missing'\n",
        )
        .unwrap();
        for selected in [PathBuf::from("../code"), code.clone()] {
            let opened = open_repository(RepositoryInvocation {
                worktree_root_override: Some(selected),
                ..invocation(temp.path())
            })
            .unwrap();
            assert_eq!(opened.layout.worktree_root(), code);
        }
    }

    #[test]
    fn test_open_repository_explicit_invalid_roots_report_source_and_path() {
        let (temp, _) = setup_test_repo().unwrap();
        let data = temp.path().join(".jit");
        std::fs::write(temp.path().join("file"), b"not a directory").unwrap();
        std::fs::create_dir(data.join("child")).unwrap();
        for selected in [
            PathBuf::new(),
            PathBuf::from("missing"),
            PathBuf::from("../file"),
            PathBuf::from("."),
            PathBuf::from("child"),
        ] {
            for source in [
                WorktreeRootSource::Environment,
                WorktreeRootSource::Configuration,
            ] {
                std::fs::write(
                    data.join("config.toml"),
                    if source == WorktreeRootSource::Configuration {
                        format!("[worktree]\nroot = {:?}\n", selected.to_string_lossy())
                    } else {
                        String::new()
                    },
                )
                .unwrap();
                let error = open_repository(RepositoryInvocation {
                    worktree_root_override: (source == WorktreeRootSource::Environment)
                        .then_some(selected.clone()),
                    ..invocation(temp.path())
                })
                .err()
                .unwrap();
                let invalid = error.downcast_ref::<InvalidWorktreeRootError>().unwrap();
                assert_eq!(invalid.setting, source);
                assert_eq!(invalid.supplied, selected);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_open_repository_explicit_root_rejects_symlinked_ancestors() {
        let (temp, _) = setup_test_repo().unwrap();
        let target = temp.path().join("target");
        std::fs::create_dir_all(target.join("child")).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        for root in [alias.clone(), alias.join("child"), alias.join("../target")] {
            let error = open_repository(RepositoryInvocation {
                worktree_root_override: Some(root),
                ..invocation(temp.path())
            })
            .err()
            .unwrap();
            assert!(error.downcast_ref::<InvalidWorktreeRootError>().is_some());
        }
    }

    #[test]
    fn test_open_repository_projects_only_required_root_fields() {
        let (temp, _) = setup_test_repo().unwrap();
        let code = temp.path().join("code");
        std::fs::create_dir(&code).unwrap();
        let config = temp.path().join(".jit/config.toml");
        std::fs::write(&config, "[project]\nname = 'INVALID'\n[worktree]\nroot = '../code'\nenforce_leases = 'invalid'\nwrite_policy = 'invalid'\n").unwrap();
        assert_eq!(
            open_repository(invocation(temp.path()))
                .unwrap()
                .layout
                .worktree_root(),
            code
        );
        for bytes in ["broken = [", "[worktree]\nroot = 42\n"] {
            std::fs::write(&config, bytes).unwrap();
            let opened = open_repository(RepositoryInvocation {
                worktree_root_override: Some(code.clone()),
                ..invocation(temp.path())
            })
            .unwrap();
            assert_eq!(opened.layout.worktree_root(), code);
            assert!(open_repository(invocation(temp.path())).is_err());
        }
    }

    #[test]
    fn test_open_repository_requires_original_linked_policy_even_with_override() {
        let (primary, _) = setup_test_repo().unwrap();
        let git = crate::test_harness::git::GitRepository::init(primary.path());
        git.commit("base data");
        let outer = tempfile::tempdir().unwrap();
        let linked = outer.path().join("linked");
        git.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let data = linked.join(".jit");
        for bytes in ["[worktree]\nwrite_policy = 'invalid'\n", "broken = ["] {
            std::fs::write(data.join("config.toml"), bytes).unwrap();
            let before = std::fs::read(data.join("events.jsonl")).unwrap();
            let error = open_repository(RepositoryInvocation {
                worktree_root_override: Some(linked.clone()),
                opens_mutation_session: true,
                write_override: Some(LinkedCheckoutWriteStance::Allow),
                ..invocation(&linked)
            })
            .err()
            .unwrap();
            assert!(
                error
                    .downcast_ref::<crate::declarations::ConfigurationAssemblyError>()
                    .is_some(),
                "{error:#}"
            );
            assert_eq!(std::fs::read(data.join("events.jsonl")).unwrap(), before);
            assert_eq!(
                std::fs::read_to_string(data.join("config.toml")).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn test_open_repository_data_only_recovery_precedes_invalid_new_root_selection() {
        use crate::storage::{commit_redo_record, DataRoot, PutFile, RedoRecord};
        let (temp, _) = setup_test_repo().unwrap();
        let data = temp.path().join(".jit");
        let record = RedoRecord::new(
            Vec::new(),
            vec![PutFile {
                path: crate::repository_state::VirtualPath::CONFIG,
                bytes: b"[worktree]\nroot = 'missing'\n".to_vec(),
                mode: crate::repository_state::FileMode::Regular,
            }],
            Vec::new(),
            None,
        )
        .unwrap();
        commit_redo_record(&DataRoot::open(&data).unwrap(), &record).unwrap();
        let error = open_repository(RepositoryInvocation {
            opens_mutation_session: true,
            ..invocation(temp.path())
        })
        .err()
        .unwrap();
        assert!(error.downcast_ref::<InvalidWorktreeRootError>().is_some());
        assert!(!data
            .join(crate::storage::RECORD_SCRATCH_DIR)
            .join(crate::storage::RECORD_FILE_NAME)
            .exists());
        assert_eq!(
            std::fs::read(data.join("config.toml")).unwrap(),
            record.puts()[0].bytes
        );
    }

    #[test]
    fn test_open_repository_replays_original_audit_before_recovered_write_policy() {
        use crate::repository_state::{FileMode, VirtualPath};
        use crate::storage::{commit_redo_record, DataRoot, EventAppend, PutFile, RedoRecord};
        use crate::test_harness::git::GitRepository;
        for next_policy in [
            LinkedCheckoutWriteStance::Refuse,
            LinkedCheckoutWriteStance::Allow,
        ] {
            for partial_config in [false, true] {
                for override_stance in [None, Some(LinkedCheckoutWriteStance::Allow)] {
                    let old_policy = match next_policy {
                        LinkedCheckoutWriteStance::Refuse => LinkedCheckoutWriteStance::Allow,
                        LinkedCheckoutWriteStance::Allow => LinkedCheckoutWriteStance::Refuse,
                    };
                    let (primary, _) = setup_test_repo().unwrap();
                    let git = GitRepository::init(primary.path());
                    git.commit("base data");
                    let outer = tempfile::tempdir().unwrap();
                    let linked = outer.path().join("linked");
                    git.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
                    let data = linked.join(".jit");
                    let old_code = outer.path().join("old-code");
                    let next_code = outer.path().join("next-code");
                    std::fs::create_dir(&old_code).unwrap();
                    std::fs::create_dir(&next_code).unwrap();
                    let config = |root: &str, policy: LinkedCheckoutWriteStance| {
                        format!("[worktree]\nroot = '../../{root}'\nwrite_policy = '{}'\nenforce_leases = 'off'\n", policy.as_token())
                    };
                    std::fs::write(data.join("config.toml"), config("old-code", old_policy))
                        .unwrap();
                    let new_config = config("next-code", next_policy).into_bytes();
                    let before = std::fs::read(data.join("events.jsonl")).unwrap();
                    let mut audit = serde_json::to_vec(
                        &crate::domain::event_catalog::EventTag::LinkedCheckoutWriteOverridden
                            .sample(),
                    )
                    .unwrap();
                    audit.push(b'\n');
                    let record = RedoRecord::for_layout(
                        &discover_repository_layout(&old_code, &data).unwrap(),
                        Vec::new(),
                        vec![
                            PutFile {
                                path: VirtualPath::CONFIG,
                                bytes: new_config.clone(),
                                mode: FileMode::Regular,
                            },
                            PutFile {
                                path: VirtualPath::worktree("committed.md").unwrap(),
                                bytes: b"original target".to_vec(),
                                mode: FileMode::Regular,
                            },
                        ],
                        Vec::new(),
                        Some(EventAppend {
                            offset: before.len() as u64,
                            bytes: audit.clone(),
                        }),
                    )
                    .unwrap();
                    commit_redo_record(&DataRoot::open(&data).unwrap(), &record).unwrap();
                    if partial_config {
                        std::fs::write(data.join("config.toml"), &new_config).unwrap();
                    }
                    let opened = open_repository(RepositoryInvocation {
                        current_dir: linked.clone(),
                        opens_mutation_session: true,
                        write_override: override_stance,
                        ..invocation(&linked)
                    });
                    if next_policy == LinkedCheckoutWriteStance::Refuse && override_stance.is_none()
                    {
                        assert!(opened
                            .err()
                            .unwrap()
                            .downcast_ref::<LinkedCheckoutWriteRefusedError>()
                            .is_some());
                    } else {
                        let opened = opened.unwrap();
                        assert_eq!(opened.layout.worktree_root(), next_code);
                        assert_eq!(opened.recovered_records, 1);
                    }
                    assert_eq!(
                        std::fs::read(data.join("events.jsonl")).unwrap(),
                        [before, audit].concat(),
                        "recovery preserves exactly the original audit bytes"
                    );
                    assert_eq!(
                        std::fs::read(old_code.join("committed.md")).unwrap(),
                        b"original target"
                    );
                    assert!(!next_code.join("committed.md").exists());
                    assert!(!data
                        .join(crate::storage::RECORD_SCRATCH_DIR)
                        .join(crate::storage::RECORD_FILE_NAME)
                        .exists());
                }
            }
        }
    }

    #[test]
    fn test_open_repository_validates_except_for_exempt_commands() {
        let absent = tempfile::TempDir::new().expect("create a temporary directory");
        let over_absent_store = RepositoryInvocation {
            data_dir_override: Some(absent.path().join(".jit")),
            ..invocation(absent.path())
        };
        let error = open_repository(over_absent_store.clone())
            .err()
            .expect("a non-exempt command validates the selected store");
        assert!(
            error.downcast_ref::<RepositoryNotFoundError>().is_some(),
            "validation reports the absent store: {error:#}"
        );
        assert!(
            open_repository(RepositoryInvocation {
                is_validation_exempt: true,
                ..over_absent_store
            })
            .is_ok(),
            "an exempt command opens without validating"
        );

        let (repo, _storage) = setup_test_repo().expect("create an initialized repository");
        let mutating = RepositoryInvocation {
            opens_mutation_session: true,
            ..invocation(repo.path())
        };
        let opened = open_repository(mutating.clone()).expect("open for a mutating command");
        assert!(
            opened.retained_session.is_some(),
            "a mutating command dispatches inside the retained session"
        );
        drop(opened);
        let claiming = open_repository(RepositoryInvocation {
            coordinates_claims_first: true,
            ..mutating
        })
        .expect("open for a claim-coordinating command");
        assert!(
            claiming.retained_session.is_none(),
            "a claim-coordinating command opens its session after coordination"
        );
    }

    #[test]
    fn test_open_repository_reports_an_unreadable_leftover_record_as_recovery_failure_only_for_recover(
    ) {
        let (repo, _storage) = setup_test_repo().expect("create an initialized repository");
        let scratch = repo
            .path()
            .join(".jit")
            .join(crate::storage::RECORD_SCRATCH_DIR);
        std::fs::create_dir_all(&scratch).expect("create the record scratch directory");
        std::fs::write(scratch.join(crate::storage::RECORD_FILE_NAME), b"no header")
            .expect("write an unreadable leftover record");
        let mutating = |coordinates_claims_first| RepositoryInvocation {
            opens_mutation_session: true,
            coordinates_claims_first,
            ..invocation(repo.path())
        };

        for coordinates_claims_first in [false, true] {
            let other = open_repository(mutating(coordinates_claims_first))
                .err()
                .expect("an unreadable leftover record fails the session open");
            assert!(
                is_leftover_record_failure(&other)
                    && other.downcast_ref::<RecoveryFailedError>().is_none(),
                "another command reports the session-open failure itself: {other:#}"
            );
        }
        let recover = open_repository(RepositoryInvocation {
            is_recover: true,
            ..mutating(true)
        })
        .err()
        .expect("an unreadable leftover record fails jit recover");
        let failure = recover
            .downcast_ref::<RecoveryFailedError>()
            .unwrap_or_else(|| panic!("jit recover reports a recovery failure: {recover:#}"));
        assert!(
            is_leftover_record_failure(&failure.0),
            "the recovery failure carries the session-open failure"
        );
    }
}

#[cfg(test)]
mod linked_checkout_write_decision_tests {
    //! The dispatch-time linked-checkout write policy: which invocations it
    //! refuses, which it permits, and which of the permitted ones owe an audit
    //! record.

    use super::linked_checkout_write_decision;
    use crate::domain::LinkedCheckoutWriteStance;
    use crate::errors::LinkedCheckoutWriteRefusedError;
    use crate::repository_state::MutationContextAnnotation;
    use crate::storage::worktree_paths::WorktreePaths;
    use crate::test_utils::repository_layout_for_data_root;

    /// A repository whose `[worktree]` section declares `stance`, or which
    /// declares no stance at all when `stance` is `None`.
    fn repository_declaring(stance: Option<LinkedCheckoutWriteStance>) -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().expect("create a temporary repository");
        let declaration = stance
            .map(|stance| format!("[worktree]\nwrite_policy = \"{}\"\n", stance.as_token()))
            .unwrap_or_default();
        std::fs::write(temp.path().join("config.toml"), declaration)
            .expect("write the repository configuration");
        temp
    }

    fn declared_policy(data: &std::path::Path) -> LinkedCheckoutWriteStance {
        crate::config_manager::write_policy_from_config(
            &crate::storage::configuration_store::load_configuration(
                &repository_layout_for_data_root(data),
            )
            .unwrap(),
        )
    }

    /// Paths describing either a linked non-primary checkout or the primary one.
    fn checkout_paths(linked: bool) -> WorktreePaths {
        if linked {
            crate::test_utils::linked_checkout_paths("/repo", "/repo/.worktrees/feature")
        } else {
            crate::test_utils::primary_checkout_paths("/repo")
        }
    }

    #[test]
    fn test_linked_checkout_write_decision_refuses_every_governed_invocation_the_stance_refuses() {
        let refusing = repository_declaring(Some(LinkedCheckoutWriteStance::Refuse));
        let undeclared = repository_declaring(None);
        let allowing = repository_declaring(Some(LinkedCheckoutWriteStance::Allow));
        let linked = checkout_paths(true);

        let cases: [(&str, &std::path::Path, Option<LinkedCheckoutWriteStance>); 3] = [
            ("a declared refusal refuses", refusing.path(), None),
            (
                "an undeclared stance refuses, because refusing is what a repository that never \
                 considered the question gets",
                undeclared.path(),
                None,
            ),
            (
                "an invocation whose own stance refuses outranks a declaration that would have \
                 permitted it",
                allowing.path(),
                Some(LinkedCheckoutWriteStance::Refuse),
            ),
        ];

        for (reason, jit_dir, invocation_override) in cases {
            let error = linked_checkout_write_decision(
                true,
                &linked,
                declared_policy(jit_dir),
                invocation_override,
            )
            .expect_err(reason);
            let refusal = error
                .downcast_ref::<LinkedCheckoutWriteRefusedError>()
                .unwrap_or_else(|| panic!("the refusal is typed, not a message: {error:#}"));
            assert_eq!(
                refusal.checkout(),
                linked.worktree_root,
                "the refusal names the checkout the worktree authority selected"
            );
            assert_eq!(
                refusal.stance(),
                LinkedCheckoutWriteStance::Refuse,
                "the refusal names the stance that refused it"
            );
        }
    }

    #[test]
    fn test_linked_checkout_write_decision_installs_a_record_only_when_an_override_outranks_a_refusal(
    ) {
        let refusing = repository_declaring(Some(LinkedCheckoutWriteStance::Refuse));
        let annotation = linked_checkout_write_decision(
            true,
            &checkout_paths(true),
            declared_policy(refusing.path()),
            Some(LinkedCheckoutWriteStance::Allow),
        )
        .expect("an override outranking a refusal permits the invocation")
        .expect("an override outranking a refusal warrants an audit record");

        let MutationContextAnnotation::LinkedCheckoutWriteOverridden {
            checkout,
            declared_stance,
            invocation_override,
        } = annotation;
        assert_eq!(
            checkout,
            checkout_paths(true).worktree_root,
            "the record names the checkout the worktree authority selected"
        );
        assert_eq!(
            (declared_stance, invocation_override),
            (
                LinkedCheckoutWriteStance::Refuse,
                LinkedCheckoutWriteStance::Allow
            ),
            "the record pairs the declaration that would have refused with the stance that permitted"
        );
    }

    #[test]
    fn test_linked_checkout_write_decision_permits_without_a_record_where_no_refusal_was_outranked()
    {
        let refusing = repository_declaring(Some(LinkedCheckoutWriteStance::Refuse));
        let allowing = repository_declaring(Some(LinkedCheckoutWriteStance::Allow));

        let cases: [(&str, bool, &WorktreePaths, &std::path::Path); 3] = [
            (
                "an invocation that opens no mutation session is outside the policy, so it is \
                 neither refused nor audited",
                false,
                &checkout_paths(true),
                refusing.path(),
            ),
            (
                "the primary checkout is never governed by the linked-checkout stance",
                true,
                &checkout_paths(false),
                refusing.path(),
            ),
            (
                "a repository declaring the allowing stance permits by its own declaration, so \
                 an override there outranks nothing",
                true,
                &checkout_paths(true),
                allowing.path(),
            ),
        ];

        for (reason, opens_mutation_session, worktree_paths, jit_dir) in cases {
            assert!(
                linked_checkout_write_decision(
                    opens_mutation_session,
                    worktree_paths,
                    declared_policy(jit_dir),
                    Some(LinkedCheckoutWriteStance::Allow)
                )
                .expect("the invocation is permitted")
                .is_none(),
                "{reason}"
            );
        }
    }
}
