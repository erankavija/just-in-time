//! `jit claim` and `jit recover` handlers.

use super::{resolve_issue, resolve_issue_filter, CommandContext, Outcome};
use crate::cli::ClaimCommands;
use crate::commands::RecoveryFailedError;
use crate::domain::identity::LeaseId;
use crate::output::{
    ClaimAcquireResponse, ClaimForceEvictResponse, ClaimHeartbeatResponse, ClaimReleaseResponse,
    ClaimRenewResponse, LeaseListResponse, RecoverResponse,
};
use crate::storage::{IssueStore, RepositoryStateStore};

fn acquire<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue_id: String,
    ttl: u64,
    agent_id: Option<String>,
    reason: Option<String>,
) -> anyhow::Result<Outcome<ClaimAcquireResponse>> {
    let full_id = resolve_issue(ctx.executor.storage(), &issue_id)?;
    let (lease_id, warnings) =
        ctx.executor
            .claims()
            .acquire(&full_id, ttl, agent_id.as_deref(), reason.as_deref())?;
    let message = format!("Acquired lease {lease_id} on issue {issue_id}");
    let shown = warnings.iter().map(ToString::to_string).collect::<Vec<_>>();
    let response = ClaimAcquireResponse {
        lease_id,
        issue_id,
        ttl_secs: ttl,
        warnings,
    };
    Ok(Outcome::success_with(response, message).with_warnings(shown))
}

fn release<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue_id: String,
) -> anyhow::Result<Outcome<ClaimReleaseResponse>> {
    let id = resolve_issue(ctx.executor.storage(), &issue_id)?;
    let (released, warnings) = ctx.executor.claims().release_by_issue(&id)?;
    let message = format!(
        "Released lease {} on issue {} (was held by {}) by {}",
        released.lease_id, released.issue_id, released.previous_owner, released.actor
    );
    let shown = warnings.iter().map(ToString::to_string).collect::<Vec<_>>();
    Ok(
        Outcome::success_with(ClaimReleaseResponse { released, warnings }, message)
            .with_warnings(shown),
    )
}

fn renew<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
    extension: u64,
) -> anyhow::Result<Outcome<ClaimRenewResponse>> {
    let (lease, warnings) = ctx.executor.claims().renew(&lease_id, extension)?;
    let shown = warnings.iter().map(ToString::to_string).collect::<Vec<_>>();
    let response = ClaimRenewResponse {
        lease,
        warnings,
        extension,
    };
    Ok(Outcome::success_with(
        response,
        format!("Renewed lease {lease_id} by {extension} seconds"),
    )
    .with_warnings(shown))
}

fn heartbeat<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
) -> anyhow::Result<Outcome<ClaimHeartbeatResponse>> {
    let warnings = ctx.executor.claims().heartbeat(&lease_id)?;
    let shown = warnings.iter().map(ToString::to_string).collect::<Vec<_>>();
    let message = format!("Heartbeat sent for lease {lease_id}");
    Ok(
        Outcome::success_with(ClaimHeartbeatResponse { lease_id, warnings }, message)
            .with_warnings(shown),
    )
}

fn status<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue: Option<String>,
    agent: Option<String>,
) -> anyhow::Result<Outcome<LeaseListResponse>> {
    let issue = issue
        .map(|id| resolve_issue_filter(ctx.executor.storage(), &id))
        .transpose()?;
    let leases = ctx
        .executor
        .claims()
        .status(issue.as_ref(), agent.as_deref())?;
    let message = format!("{} active lease(s)", leases.len());
    Ok(Outcome::success_with(
        LeaseListResponse::listing(leases, false),
        message,
    ))
}

fn list<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<LeaseListResponse>> {
    let leases = ctx.executor.claims().list()?;
    let message = format!("{} lease(s) found", leases.len());
    Ok(Outcome::success_with(
        LeaseListResponse::listing(leases, true),
        message,
    ))
}

fn force_evict<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
    reason: String,
) -> anyhow::Result<Outcome<ClaimForceEvictResponse>> {
    let warnings = ctx.executor.claims().force_evict(&lease_id, &reason)?;
    let shown = warnings.iter().map(ToString::to_string).collect::<Vec<_>>();
    let message = format!("Force-evicted lease {lease_id}");
    let response = ClaimForceEvictResponse {
        lease_id,
        reason,
        warnings,
    };
    Ok(Outcome::success_with(response, message).with_warnings(shown))
}

command_group! {
    command: ClaimCommands,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "claim_acquire" => ClaimCommands::Acquire { issue_id, ttl, agent_id, reason }
            => acquire(issue_id, ttl, agent_id, reason) -> ClaimAcquireResponse,
        "claim_release" => ClaimCommands::Release { issue_id }
            => release(issue_id) -> ClaimReleaseResponse,
        "claim_renew" => ClaimCommands::Renew { lease_id, extension }
            => renew(lease_id, extension) -> ClaimRenewResponse,
        "claim_heartbeat" => ClaimCommands::Heartbeat { lease_id }
            => heartbeat(lease_id) -> ClaimHeartbeatResponse,
        "claim_status" => ClaimCommands::Status { issue, agent }
            => status(issue, agent) -> LeaseListResponse,
        "claim_list" => ClaimCommands::List => list() -> LeaseListResponse,
        "claim_force-evict" => ClaimCommands::ForceEvict { lease_id, reason }
            => force_evict(lease_id, reason) -> ClaimForceEvictResponse,
    }
}

/// `jit recover`, whose unit variant is the single-leaf group's command.
pub mod recover {
    use super::*;

    /// Recovery failures keep the `recovery_failed` classification.
    fn recover<S: IssueStore + RepositoryStateStore>(
        ctx: &CommandContext<'_, S>,
    ) -> anyhow::Result<Outcome<RecoverResponse>> {
        let report = ctx
            .executor
            .claims()
            .recover()
            .map_err(RecoveryFailedError)?;
        let message = format!(
            "Recovery: {} locks cleaned, {} leases evicted, {} leases reclaimed",
            report.stale_locks_cleaned,
            report.expired_leases_evicted,
            report.stale_leases_reclaimed
        );
        let shown = report
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let response = RecoverResponse {
            success: true,
            report,
        };
        Ok(Outcome::success_with(response, message).with_warnings(shown))
    }

    command_group! {
        command: (),
        storage: [IssueStore + RepositoryStateStore],
        leaves: {
            "recover" => () => recover() -> RecoverResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{HumanRender, OutputMode};

    #[test]
    fn test_registered_output_schema_every_claim_leaf_has_a_typed_response() {
        let names = [
            ("claim_acquire", "ClaimAcquireResponse"),
            ("claim_release", "ClaimReleaseResponse"),
            ("claim_renew", "ClaimRenewResponse"),
            ("claim_heartbeat", "ClaimHeartbeatResponse"),
            ("claim_status", "LeaseListResponse"),
            ("claim_list", "LeaseListResponse"),
            ("claim_force-evict", "ClaimForceEvictResponse"),
            ("recover", "RecoverResponse"),
        ];

        for (path, name) in names {
            let registered = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(registered.success, name, "{path}");
        }
    }

    #[test]
    fn test_render_human_lease_listings_word_an_empty_result_per_command() {
        let render =
            |all| LeaseListResponse::listing(Vec::new(), all).render_human(OutputMode::default());

        assert_ne!(render(false), render(true));
    }
}
