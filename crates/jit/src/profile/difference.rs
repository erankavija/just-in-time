//! What a profile selection would do to a repository, read from the decision a
//! publication would act on.
//!
//! A prepared materialization plan carries one decision per target each
//! participating profile takes part in: the values it would create or update,
//! the recorded claims it would retain or remove, and the targets it cannot
//! publish at all. [`profile_target_decisions`] is the sole reader of that
//! decision, and it has exactly two callers: the difference report, which
//! states the decision without publishing any of it, and
//! [`ensure_publishable_targets`], which refuses a publication the decision
//! says cannot be made. A reported outcome and a published outcome therefore
//! cannot disagree — there is nothing for them to disagree about.
//!
//! The refusal carries every conflicting target rather than the first one,
//! because a selection is refused as a whole and an adopter resolving one
//! conflict per run cannot see how much work the selection actually is.

use crate::profile::{
    ProfileId, ProfilePlanEntry, ProfilePlanStatus, ProfileTargetAction, ProfileTargetChange,
};
use crate::repository_state::{
    MaterializationPlan, ProfileTargetConflictEntry, ProfileTargetConflictsError,
    ProfileTargetDisposition, ProfileTargetMaterialization,
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

/// Project one profile's decisions into the public target vocabulary.
pub(crate) fn profile_target_changes(
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
                target
                    .owners
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                match &target.disposition {
                    ProfileTargetDisposition::Conflict(conflict) => Some(conflict.message()),
                    _ => None,
                },
            )
        })
        .collect()
}

/// Build one profile's preview entry from the decision the plan carries.
///
/// A rehearsal of a publication and a difference report both answer through
/// this projection, so the entry an adopter inspects before applying is the
/// entry the run itself would report.
pub(crate) fn profile_plan_entry(
    plan: &MaterializationPlan,
    id: &ProfileId,
    version: &str,
) -> ProfilePlanEntry {
    let targets = profile_target_changes(plan, id);
    let status = if targets
        .iter()
        .any(|target| target.action == ProfileTargetAction::Conflict)
    {
        ProfilePlanStatus::WouldConflict
    } else if plan.applied_profiles().contains(id) {
        ProfilePlanStatus::WouldApply
    } else {
        ProfilePlanStatus::Unchanged
    };
    ProfilePlanEntry {
        id: id.to_string(),
        version: version.to_string(),
        status,
        plan_hash: plan.hash().to_string(),
        targets,
    }
}

/// Refuse a plan whose profile decisions include a target no publication may
/// make.
///
/// Every mutating profile route — application, reconfiguration, upgrade, their
/// rehearsals, and a profiled initialization — passes through this refusal, so
/// a conflicting target stops a publication regardless of which command asked
/// for it. The difference report is the one caller that reads the same decision
/// without refusing it.
///
/// # Errors
///
/// Returns [`ProfileTargetConflictsError`] naming every profile, target, and
/// reason the plan refused.
pub(crate) fn ensure_publishable_targets(
    plan: &MaterializationPlan,
) -> Result<(), ProfileTargetConflictsError> {
    let owners = plan
        .profile_targets()
        .iter()
        .map(|target| &target.owner)
        .collect::<BTreeSet<_>>();
    let conflicts = owners
        .into_iter()
        .flat_map(|owner| {
            profile_target_decisions(plan, owner)
                .into_iter()
                .filter_map(|target| match &target.disposition {
                    ProfileTargetDisposition::Conflict(conflict) => {
                        Some(ProfileTargetConflictEntry {
                            owner: owner.clone(),
                            target: target.path.clone(),
                            conflict: conflict.clone(),
                        })
                    }
                    _ => None,
                })
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
        ProfileTargetDisposition::Retain => ProfileTargetAction::Retain,
        ProfileTargetDisposition::Remove => ProfileTargetAction::Remove,
        ProfileTargetDisposition::Conflict(_) => ProfileTargetAction::Conflict,
    }
}
