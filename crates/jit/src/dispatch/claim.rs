//! `jit claim` and `jit recover` handlers.

use super::{CommandContext, Outcome};
use crate::cli::ClaimCommands;
use crate::commands::RecoveryFailedError;
use crate::domain::identity::{IssueId, IssueIdPrefix, LeaseId};
use crate::output::{
    ClaimAcquireResponse, ClaimForceEvictResponse, ClaimHeartbeatResponse, ClaimReleaseResponse,
    ClaimRenewResponse, LeaseListResponse, LeaseListing, OutputContext, RecoverResponse,
};
use crate::storage::{IssueStore, RepositoryStateStore, StorageWarning};

/// Human-mode warnings go to stderr after the work succeeded.
fn warn<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    warnings: &[StorageWarning],
) -> anyhow::Result<()> {
    let output = OutputContext::new(ctx.mode.quiet, ctx.mode.json);
    warnings
        .iter()
        .try_for_each(|warning| Ok(output.print_warning(warning)?))
}

fn resolve<S: IssueStore>(ctx: &CommandContext<'_, S>, requested: &str) -> anyhow::Result<IssueId> {
    ctx.executor
        .storage()
        .resolve_issue_id(&requested.parse::<IssueIdPrefix>()?)
}

/// A status filter naming no existing issue still filters, by its prefix.
fn resolve_filter<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    requested: &str,
) -> anyhow::Result<IssueIdPrefix> {
    let prefix = requested.parse::<IssueIdPrefix>()?;
    match ctx.executor.storage().resolve_issue_id(&prefix) {
        Ok(id) => Ok(IssueIdPrefix::from(&id)),
        Err(error) if error.is::<crate::storage::IssueNotFoundError>() => Ok(prefix),
        Err(error) => Err(error),
    }
}

fn acquire<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue_id: String,
    ttl: u64,
    agent_id: Option<String>,
    reason: Option<String>,
) -> anyhow::Result<Outcome<ClaimAcquireResponse>> {
    let full_id = resolve(ctx, &issue_id)?;
    let (lease_id, warnings) =
        ctx.executor
            .claims()
            .acquire(&full_id, ttl, agent_id.as_deref(), reason.as_deref())?;
    warn(ctx, &warnings)?;
    let message = format!("Acquired lease {lease_id} on issue {issue_id}");
    Ok(Outcome::Success {
        response: ClaimAcquireResponse {
            lease_id,
            issue_id,
            ttl_secs: ttl,
            warnings,
        },
        message: Some(message),
    })
}

fn release<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue_id: String,
) -> anyhow::Result<Outcome<ClaimReleaseResponse>> {
    let (released, warnings) = ctx
        .executor
        .claims()
        .release_by_issue(&resolve(ctx, &issue_id)?)?;
    warn(ctx, &warnings)?;
    let message = format!(
        "Released lease {} on issue {} (was held by {}) by {}",
        released.lease_id, released.issue_id, released.previous_owner, released.actor
    );
    Ok(Outcome::Success {
        response: ClaimReleaseResponse { released, warnings },
        message: Some(message),
    })
}

fn renew<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
    extension: u64,
) -> anyhow::Result<Outcome<ClaimRenewResponse>> {
    let (lease, warnings) = ctx.executor.claims().renew(&lease_id, extension)?;
    warn(ctx, &warnings)?;
    Ok(Outcome::Success {
        response: ClaimRenewResponse {
            lease,
            warnings,
            extension,
        },
        message: Some(format!("Renewed lease {lease_id} by {extension} seconds")),
    })
}

fn heartbeat<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
) -> anyhow::Result<Outcome<ClaimHeartbeatResponse>> {
    let warnings = ctx.executor.claims().heartbeat(&lease_id)?;
    warn(ctx, &warnings)?;
    Ok(Outcome::Success {
        message: Some(format!("Heartbeat sent for lease {lease_id}")),
        response: ClaimHeartbeatResponse { lease_id, warnings },
    })
}

fn status<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    issue: Option<String>,
    agent: Option<String>,
) -> anyhow::Result<Outcome<LeaseListResponse>> {
    let issue = issue.map(|id| resolve_filter(ctx, &id)).transpose()?;
    let leases = ctx
        .executor
        .claims()
        .status(issue.as_ref(), agent.as_deref())?;
    Ok(Outcome::Success {
        message: Some(format!("{} active lease(s)", leases.len())),
        response: LeaseListResponse::listing(leases, LeaseListing::Status),
    })
}

fn list<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<LeaseListResponse>> {
    let leases = ctx.executor.claims().list()?;
    Ok(Outcome::Success {
        message: Some(format!("{} lease(s) found", leases.len())),
        response: LeaseListResponse::listing(leases, LeaseListing::All),
    })
}

fn force_evict<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    lease_id: LeaseId,
    reason: String,
) -> anyhow::Result<Outcome<ClaimForceEvictResponse>> {
    let warnings = ctx.executor.claims().force_evict(&lease_id, &reason)?;
    warn(ctx, &warnings)?;
    Ok(Outcome::Success {
        message: Some(format!("Force-evicted lease {lease_id}")),
        response: ClaimForceEvictResponse {
            lease_id,
            reason,
            warnings,
        },
    })
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

/// `jit recover`, a leaf with no argument enum: its command is `()`.
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
        warn(ctx, &report.warnings)?;
        Ok(Outcome::Success {
            message: Some(format!(
                "Recovery: {} locks cleaned, {} leases evicted, {} leases reclaimed",
                report.stale_locks_cleaned,
                report.expired_leases_evicted,
                report.stale_leases_reclaimed
            )),
            response: RecoverResponse {
                success: true,
                report,
            },
        })
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
    use crate::output::HumanRender;

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
        let render = |listing| LeaseListResponse::listing(Vec::new(), listing).render_human();

        assert_ne!(render(LeaseListing::Status), render(LeaseListing::All));
    }
}
