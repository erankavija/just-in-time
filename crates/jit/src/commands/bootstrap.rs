//! Repository opening for one CLI invocation: data-root selection, the
//! linked-checkout write decision, startup recovery and store validation, in
//! the order the invocation's correctness depends on.

use super::{CommandExecutor, DispatchMutationAnnotation};
use crate::config_manager::ConfigManager;
use crate::domain::LinkedCheckoutWriteStance;
use crate::errors::{LinkedCheckoutWriteRefusedError, RecoveryFailedError};
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
/// A linked-checkout write refusal precedes every lock and write, and startup
/// recovery precedes validation. A non-exempt command fails here when the
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

    // Discovery is mandatory: startup reports discovery and wiring failures
    // before dispatch. The worktree root always comes from this boundary, never
    // inferred from the storage parent (`@/charter/D-4`).
    let worktree_paths = WorktreePaths::detect_for_data_root(&data_root, &non_git_worktree_root)
        .context("failed to discover repository worktree layout")?;
    let layout = discover_repository_layout(&worktree_paths.worktree_root, &data_root)
        .context("failed to construct repository layout")?;

    // Decided strictly before the storage handle exists, so a refusal appends
    // nothing and leaves no residue for recovery to converge.
    let write_override = linked_checkout_write_decision(
        invocation.opens_mutation_session,
        &worktree_paths,
        &layout,
        invocation.write_override,
    )?;

    let storage = JsonFileStorage::new(&data_root);
    storage.configure_worktree_paths(&worktree_paths);
    // Session opening is the one recovery boundary. Claim-coordinating commands
    // open their operation session only after coordination, so their startup
    // open repairs residue without inverting coordinator -> repository.
    let recovery_failure = |error: anyhow::Error| {
        if invocation.is_recover && is_leftover_record_failure(&error) {
            RecoveryFailedError(error).into()
        } else {
            error
        }
    };
    let (retained_session, recovered_records) = match (
        invocation.opens_mutation_session,
        invocation.coordinates_claims_first,
    ) {
        (false, _) => (None, 0),
        (true, true) => {
            let session = RepositoryStateStore::open_mutation_session(&storage, layout.clone())
                .map_err(|error| recovery_failure(error.into()))?;
            (None, session.recovery_report().recovered_count())
        }
        (true, false) => {
            let guard = storage
                .open_and_retain_mutation_session(layout.clone())
                .map_err(recovery_failure)?;
            let recovered = guard.recovery_report().recovered_count();
            (Some(guard), recovered)
        }
    };
    let executor = CommandExecutor::new(storage.clone())
        .with_layout(layout.clone())
        .with_worktree_paths(worktree_paths.clone())
        .with_recovered_records(recovered_records);
    let write_override_annotation = write_override.map(DispatchMutationAnnotation::install);

    if !invocation.is_validation_exempt {
        storage.validate()?;
    }

    Ok(OpenedRepository {
        write_override_annotation,
        retained_session,
        executor,
        data_root,
        worktree_paths,
        layout,
        recovered_records,
    })
}

/// The data root and the worktree root used outside Git.
///
/// Discovery walks up like git's: it stops at the first ancestor holding
/// `.jit/` and never crosses a `.git` boundary. Absent a match it falls back to
/// `<current_dir>/.jit`, so validation's "repository not found" names a
/// sensible path.
fn select_data_root(invocation: &RepositoryInvocation) -> (PathBuf, PathBuf) {
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
    layout: &RepositoryLayout,
    invocation_override: Option<LinkedCheckoutWriteStance>,
) -> Result<Option<MutationContextAnnotation>> {
    if !opens_mutation_session || !worktree_paths.holds_linked_store() {
        return Ok(None);
    }
    let declared = ConfigManager::new(layout.clone()).get_write_policy()?;
    let effective = LinkedCheckoutWriteStance::resolve(declared, invocation_override);
    match effective {
        LinkedCheckoutWriteStance::Refuse => Err(LinkedCheckoutWriteRefusedError::new(
            worktree_paths.worktree_root.clone(),
            effective,
        )
        .into()),
        LinkedCheckoutWriteStance::Allow => Ok((declared == LinkedCheckoutWriteStance::Refuse)
            .then(
                || MutationContextAnnotation::LinkedCheckoutWriteOverridden {
                    checkout: worktree_paths.worktree_root.clone(),
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
            is_init: false,
            opens_mutation_session: false,
            coordinates_claims_first: false,
            is_validation_exempt: false,
            is_recover: false,
            write_override: None,
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
                &repository_layout_for_data_root(jit_dir),
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
            &repository_layout_for_data_root(refusing.path()),
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
                    &repository_layout_for_data_root(jit_dir),
                    Some(LinkedCheckoutWriteStance::Allow)
                )
                .expect("the invocation is permitted")
                .is_none(),
                "{reason}"
            );
        }
    }
}
