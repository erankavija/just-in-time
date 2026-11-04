//! What a profile selection would do to a repository, read from the decision a
//! publication would act on.
//!
//! A prepared materialization plan carries one decision per target and one per
//! semantic identity each participating profile takes part in: the values it
//! would create or update, the declarations it would publish, and the targets
//! and declarations it cannot publish at all. [`profile_target_decisions`] and
//! [`profile_contribution_decisions`] are the sole readers of that decision, and
//! each has exactly two callers: the difference report, which states the
//! decision without publishing any of it, and [`ensure_publishable_targets`],
//! which refuses a publication the decision says cannot be made. A reported
//! outcome and a published outcome therefore cannot disagree — there is nothing
//! for them to disagree about.
//!
//! Both halves of the decision are read the same way because contributions
//! compose by identity: a package claims a semantic declaration inside a
//! registry rather than the registry file, so what publishing it would do and
//! whether it can be published at all are answered per identity and reported
//! beside the file targets rather than folded into them.
//!
//! The refusal carries every conflicting decision rather than the first one,
//! because a selection is refused as a whole and an adopter resolving one
//! conflict per run cannot see how much work the selection actually is.

use crate::domain::identity::ProfileId;
use crate::domain::relative_path::RootRelativePath;
use crate::profile::{
    ProfileApplicationStatus, ProfileApplyResult, ProfileContributionChange, ProfileRunMode,
    ProfileTargetAction, ProfileTargetChange,
};
use crate::repository_state::{
    MaterializationPlan, ProfileContributionMaterialization, ProfileTargetConflictEntry,
    ProfileTargetConflictsError, ProfileTargetDisposition, ProfileTargetMaterialization,
    ProfileTargetSubject,
};
use std::collections::BTreeSet;

/// Every target `owner` decided in a prepared plan, in canonical target order.
///
/// One plan composes every member of its selection, so the answer is scoped to
/// the profile the caller is asking about: it carries the decisions that
/// profile made and no others. A target more than one selected profile
/// contributes is decided by each of them and therefore answers under each —
/// attributing it to one owner would misreport the others.
pub(crate) fn profile_target_decisions<'plan>(
    plan: &'plan MaterializationPlan,
    owner: &ProfileId,
) -> Vec<&'plan ProfileTargetMaterialization> {
    plan.profile_targets()
        .iter()
        .filter(|target| &target.owner == owner)
        .collect()
}

/// Every declaration `owner` decided in a prepared plan, in canonical identity
/// order. Scoped to one profile for the same reason its targets are.
pub(crate) fn profile_contribution_decisions<'plan>(
    plan: &'plan MaterializationPlan,
    owner: &ProfileId,
) -> Vec<&'plan ProfileContributionMaterialization> {
    plan.profile_contributions()
        .iter()
        .filter(|contribution| &contribution.owner == owner)
        .collect()
}

/// Project one profile's target decisions into the public target vocabulary.
fn profile_target_changes(
    plan: &MaterializationPlan,
    owner: &ProfileId,
) -> Vec<ProfileTargetChange> {
    profile_target_decisions(plan, owner)
        .into_iter()
        .map(|target| {
            ProfileTargetChange::new(
                target.path.repository_relative(),
                target_action(&target.disposition),
                target.mode,
                conflict_reason(&target.disposition),
            )
        })
        .collect()
}

/// Project one profile's declaration decisions into the public vocabulary.
fn profile_contribution_changes(
    plan: &MaterializationPlan,
    owner: &ProfileId,
) -> Vec<ProfileContributionChange> {
    profile_contribution_decisions(plan, owner)
        .into_iter()
        .map(|contribution| ProfileContributionChange {
            identity: contribution.identity.to_string(),
            action: target_action(&contribution.disposition),
            reason: conflict_reason(&contribution.disposition),
        })
        .collect()
}

/// Build one profile's answer from the decision the plan carries.
///
/// Every run mode answers through this projection, so the entry an adopter
/// inspects before applying is the entry the run itself reports.
pub(crate) fn profile_run_entry(
    plan: &MaterializationPlan,
    id: &ProfileId,
    version: &semver::Version,
    origin: RootRelativePath,
    mode: ProfileRunMode,
) -> ProfileApplyResult {
    let targets = profile_target_changes(plan, id);
    let contributions = profile_contribution_changes(plan, id);
    let conflicted = targets
        .iter()
        .map(|target| target.action)
        .chain(contributions.iter().map(|contribution| contribution.action))
        .any(|action| action == ProfileTargetAction::Conflict);
    let changed = !plan.delta().actions().is_empty() && plan.applied_profiles().contains(id);
    let status = match (conflicted, changed, mode) {
        (true, _, _) => ProfileApplicationStatus::WouldConflict,
        (false, true, ProfileRunMode::Publish) => ProfileApplicationStatus::Applied,
        (false, true, _) => ProfileApplicationStatus::WouldApply,
        (false, false, _) => ProfileApplicationStatus::Unchanged,
    };
    ProfileApplyResult {
        id: id.clone(),
        version: version.clone(),
        origin,
        status,
        targets,
        contributions,
        refusal: None,
    }
}

/// Refuse a plan whose profile decisions include a target or a declaration no
/// publication may make.
///
/// Every mutating profile route — application, its rehearsal, and a profiled
/// initialization — passes through this refusal, so
/// a conflicting decision stops a publication regardless of which command asked
/// for it, and regardless of whether it was about a file or a declaration. The
/// difference report is the one caller that reads the same decision without
/// refusing it.
///
/// # Errors
///
/// Returns [`ProfileTargetConflictsError`] naming every profile, target,
/// declaration, and reason the plan refused.
pub(crate) fn ensure_publishable_targets(
    plan: &MaterializationPlan,
) -> Result<(), ProfileTargetConflictsError> {
    let owners = plan
        .profile_targets()
        .iter()
        .map(|target| &target.owner)
        .chain(
            plan.profile_contributions()
                .iter()
                .map(|contribution| &contribution.owner),
        )
        .collect::<BTreeSet<_>>();
    let conflicts = owners
        .into_iter()
        .flat_map(|owner| {
            let targets = profile_target_decisions(plan, owner)
                .into_iter()
                .filter_map(|target| {
                    refused(&target.disposition).map(|conflict| ProfileTargetConflictEntry {
                        owner: owner.clone(),
                        subject: ProfileTargetSubject::File {
                            path: target.path.clone(),
                        },
                        conflict,
                    })
                })
                .collect::<Vec<_>>();
            let contributions = profile_contribution_decisions(plan, owner)
                .into_iter()
                .filter_map(|contribution| {
                    refused(&contribution.disposition).map(|conflict| ProfileTargetConflictEntry {
                        owner: owner.clone(),
                        subject: ProfileTargetSubject::Contribution {
                            identity: contribution.identity.clone(),
                        },
                        conflict,
                    })
                })
                .collect::<Vec<_>>();
            targets.into_iter().chain(contributions)
        })
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(ProfileTargetConflictsError::new(conflicts))
    }
}

/// Name the public action for one decided disposition.
fn target_action(disposition: &ProfileTargetDisposition) -> ProfileTargetAction {
    match disposition {
        ProfileTargetDisposition::Unchanged => ProfileTargetAction::Unchanged,
        ProfileTargetDisposition::Create => ProfileTargetAction::Create,
        ProfileTargetDisposition::Update => ProfileTargetAction::Update,
        ProfileTargetDisposition::Remove => ProfileTargetAction::Remove,
        ProfileTargetDisposition::Conflict(_) => ProfileTargetAction::Conflict,
    }
}

/// The sentence a decision that cannot be published states, absent otherwise.
fn conflict_reason(disposition: &ProfileTargetDisposition) -> Option<String> {
    refused(disposition).map(|conflict| conflict.message())
}

/// The reason one decision cannot be published, absent when it can.
fn refused(
    disposition: &ProfileTargetDisposition,
) -> Option<crate::repository_state::ProfileTargetConflict> {
    match disposition {
        ProfileTargetDisposition::Conflict(conflict) => Some(conflict.clone()),
        _ => None,
    }
}
