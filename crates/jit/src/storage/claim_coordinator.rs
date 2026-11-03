//! Claim coordination for multi-agent parallel work
//!
//! This module implements atomic, race-free claim acquisition using file locks
//! to ensure only one agent can hold a lease on an issue at a time.
//!
//! # Design
//!
//! - Uses exclusive file locks (`fs4` crate) for atomicity
//! - Claims stored in append-only JSONL log for audit trail
//! - Claims index provides derived view of active leases
//! - Monotonic time semantics for expiration (immune to NTP)
//!
//! See design doc: `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` - "Claim Acquisition Algorithm"

use crate::storage::clock::{Clock, SystemClock};
use crate::storage::StorageWarning;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::lock::FileLocker;
use super::worktree_paths::WorktreePaths;
use crate::domain::identity::{IssueId, LeaseId, WorktreeId};
use crate::domain::Assignee;
use crate::errors;

/// A lease granting exclusive write access to an issue
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Lease {
    /// Unique lease identifier (UUID)
    pub lease_id: LeaseId,
    /// Issue being claimed
    pub issue_id: IssueId,
    /// Agent holding the lease
    #[serde(deserialize_with = "deserialize_agent_id")]
    #[schemars(with = "String")]
    pub agent_id: Assignee,
    /// Worktree where lease was acquired
    pub worktree_id: WorktreeId,
    /// Git branch (informational)
    pub branch: Option<String>,
    /// Time-to-live in seconds (0 = indefinite)
    pub ttl_secs: u64,
    /// When the lease was acquired
    pub acquired_at: DateTime<Utc>,
    /// When the lease expires (None if TTL=0)
    pub expires_at: Option<DateTime<Utc>>,
    /// Last heartbeat timestamp (for indefinite leases)
    pub last_beat: DateTime<Utc>,
    /// Whether the lease is stale (computed during index rebuild for TTL=0 leases)
    #[serde(default)]
    pub stale: bool,
}

fn deserialize_agent_id<'de, D>(deserializer: D) -> std::result::Result<Assignee, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    raw.parse()
        .map_err(|error| serde::de::Error::custom(format!("agent_id: {error}")))
}

impl Lease {
    /// Finite leases expire strictly after `expires_at`; indefinite leases and
    /// leases without an expiry never expire.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        if self.ttl_secs > 0 {
            if let Some(expires_at) = self.expires_at {
                return now > expires_at;
            }
        }
        false
    }

    /// Return whether this lease is stale at `now`: an indefinite lease whose
    /// last heartbeat is older than
    /// [`CLAIM_STALE_THRESHOLD_SECS`](crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS).
    /// Finite leases use expiry instead and are never stale.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.ttl_secs == 0
            && now.signed_duration_since(self.last_beat).num_seconds()
                > crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64
    }

    /// Active leases are neither expired nor stale at `now`.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        !self.is_expired(now) && !self.is_stale(now)
    }
}

/// Claim operation types for audit log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ClaimOp {
    /// Acquire a new lease
    Acquire {
        #[serde(flatten)]
        lease: Lease,
    },
    /// Renew an existing lease
    Renew {
        lease_id: LeaseId,
        new_expires_at: Option<DateTime<Utc>>,
        new_last_beat: DateTime<Utc>,
    },
    /// Heartbeat for indefinite leases (updates last_beat only)
    Heartbeat {
        lease_id: LeaseId,
        new_last_beat: DateTime<Utc>,
    },
    /// Release a lease explicitly
    Release {
        lease_id: LeaseId,
        released_at: DateTime<Utc>,
    },
    /// Automatically evict expired lease
    AutoEvict {
        lease_id: LeaseId,
        evicted_at: DateTime<Utc>,
        reason: String,
    },
    /// Force evict a lease (admin operation)
    ForceEvict {
        lease_id: LeaseId,
        evicted_at: DateTime<Utc>,
        reason: String,
    },
}

/// Claim operation with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimLogEntry {
    /// Schema version for future compatibility
    pub schema_version: u32,
    /// Sequence number for total ordering
    pub seq: u64,
    /// Timestamp of operation
    pub timestamp: DateTime<Utc>,
    /// The operation
    #[serde(flatten)]
    pub operation: ClaimOp,
}

/// Index of active leases (derived from log)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaimsIndex {
    /// Schema version
    pub schema_version: u32,
    /// When this index was generated
    pub generated_at: DateTime<Utc>,
    /// Last sequence number processed
    pub last_seq: u64,
    /// Active leases by issue ID
    pub leases: Vec<Lease>,
    /// Detected sequence gaps (for diagnostics)
    #[serde(default)]
    pub sequence_gaps: Vec<u64>,
}

/// Policy limits applied when acquiring an indefinite claim lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimAcquireLimits {
    max_indefinite_per_agent: u32,
    max_indefinite_per_repo: u32,
}

impl ClaimAcquireLimits {
    /// Build acquisition limits for one agent and for the repository overall.
    pub const fn new(max_indefinite_per_agent: u32, max_indefinite_per_repo: u32) -> Self {
        Self {
            max_indefinite_per_agent,
            max_indefinite_per_repo,
        }
    }
}

impl ClaimsIndex {
    /// Load the stored index without locking, returning an empty index if absent.
    /// Lease readers use [`read_leases`] or its active variants.
    pub fn load(paths: &WorktreePaths) -> Result<ClaimsIndex> {
        let index_path = paths.shared_jit.join("claims.index.json");

        if !index_path.exists() {
            return Ok(ClaimsIndex {
                schema_version: 1,
                generated_at: Utc::now(),
                last_seq: 0,
                leases: Vec::new(),
                sequence_gaps: Vec::new(),
            });
        }

        let content = fs::read_to_string(&index_path).context("Failed to read claims index")?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse claims index at {}", index_path.display()))
    }

    /// Non-fatal warnings derived from the index's diagnostic state.
    ///
    /// Currently this surfaces any sequence gaps detected while rebuilding the
    /// index from the claims log as [`StorageWarning::SequenceGap`] values. The
    /// storage layer collects these instead of printing them; the output layer
    /// decides whether and how to render them.
    pub fn warnings(&self) -> Vec<StorageWarning> {
        self.sequence_gaps
            .iter()
            .map(|&missing| StorageWarning::SequenceGap { missing })
            .collect()
    }

    /// The first active lease on `issue_id` held by anyone but `current_agent`; a
    /// writer with no identity holds none, so every active lease conflicts.
    pub(crate) fn conflicting_lease(
        &self,
        issue_id: &IssueId,
        current_agent: Option<&Assignee>,
        now: DateTime<Utc>,
    ) -> Option<&Lease> {
        self.leases.iter().find(|lease| {
            &lease.issue_id == issue_id
                && lease.is_active(now)
                && current_agent.is_none_or(|agent| &lease.agent_id != agent)
        })
    }

    /// Find active lease for an issue
    pub fn find_lease(&self, issue_id: &IssueId) -> Option<&Lease> {
        self.leases.iter().find(|l| &l.issue_id == issue_id)
    }

    /// Add a lease to the index
    pub fn add_lease(&mut self, lease: Lease) {
        // Remove any existing lease for this issue
        self.leases.retain(|l| l.issue_id != lease.issue_id);
        self.leases.push(lease);
    }

    /// Remove a lease from the index
    pub fn remove_lease(&mut self, lease_id: &LeaseId) {
        self.leases.retain(|l| &l.lease_id != lease_id);
    }

    /// Find a lease by lease_id
    pub fn find_lease_by_id(&self, lease_id: &LeaseId) -> Option<&Lease> {
        self.leases.iter().find(|l| &l.lease_id == lease_id)
    }
}

/// Read every stored lease without locking or evicting, including inactive leases.
pub fn read_leases(paths: &WorktreePaths) -> Result<Vec<Lease>> {
    Ok(ClaimsIndex::load(paths)?.leases)
}

/// Read active leases without locking or evicting.
pub fn read_active_leases(paths: &WorktreePaths, now: DateTime<Utc>) -> Result<Vec<Lease>> {
    Ok(read_leases(paths)?
        .into_iter()
        .filter(|lease| lease.is_active(now))
        .collect())
}

/// Read an issue's active lease without locking or evicting.
pub fn read_active_lease(
    paths: &WorktreePaths,
    issue_id: &IssueId,
    now: DateTime<Utc>,
) -> Result<Option<Lease>> {
    Ok(read_leases(paths)?
        .into_iter()
        .find(|lease| &lease.issue_id == issue_id && lease.is_active(now)))
}

/// Claim coordinator for atomic lease operations
pub struct ClaimCoordinator {
    paths: WorktreePaths,
    locker: FileLocker,
    worktree_id: WorktreeId,
    agent_id: Assignee,
    /// Whether to fsync the audit log and index on every write. Defaults to
    /// `true` for crash durability in production. Tests that exercise pure
    /// index/rebuild logic (not crash recovery) disable it via
    /// [`ClaimCoordinator::with_fsync`]: an fsync costs ~10ms and property
    /// tests do hundreds of writes, so leaving it on makes them minutes long.
    fsync: bool,
    /// Source of "now" for lease expiry and eviction. Defaults to
    /// [`SystemClock`] (the real system clock) in production; tests inject a
    /// deterministic clock so TTL boundaries are exercised without real delays.
    clock: Arc<dyn Clock>,
}

/// Stable read view of claims held across one repository mutation attempt.
///
/// The private guards enforce and retain the coordinator -> repository order;
/// callers may inspect the snapshot but cannot mutate the control plane through
/// this value.
pub(crate) struct ClaimsMutationGuard {
    index: ClaimsIndex,
    now: DateTime<Utc>,
    _order: crate::storage::guard_order::CoordinationOrderGuard,
    _lock: crate::storage::lock::LockGuard,
}

/// What a [`LeaseLock::apply`] operation does to its issue's lease.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaseChange {
    /// Leave the lease as it is.
    Keep,
    /// Store this lease: a new lease when none is current, otherwise the current
    /// lease with a new expiry or heartbeat.
    Put(Lease),
    /// Remove the current lease, recording the reason in the claims log.
    Remove(String),
}

/// The held claims lock: the one sequence through which every lease changes.
///
/// It holds the coordination-order marker and `claims.lock` until dropped.
/// [`load`](Self::load) and [`list`](Self::list) read the log-replayed lease
/// state, expired and stale leases included; [`apply`](Self::apply) is the only
/// writer.
pub struct LeaseLock<'a> {
    coordinator: &'a ClaimCoordinator,
    index: ClaimsIndex,
    _lock: crate::storage::lock::LockGuard,
    _order: crate::storage::guard_order::CoordinationOrderGuard,
}

impl LeaseLock<'_> {
    /// The issue's stored lease, expired or stale included.
    pub fn load(&self, issue_id: &IssueId) -> Result<Option<Lease>> {
        Ok(self.index.find_lease(issue_id).cloned())
    }

    /// Every stored lease, expired and stale included.
    pub fn list(&self) -> Result<Vec<Lease>> {
        Ok(self.index.leases.clone())
    }

    /// `lease-id-scan-bridge`: find a lease by id until renew, heartbeat and
    /// force-evict are addressed by issue.
    fn find_by_lease_id(&self, lease_id: &LeaseId) -> Result<Lease> {
        self.list()?
            .into_iter()
            .find(|lease| &lease.lease_id == lease_id)
            .ok_or_else(|| lease_not_found(lease_id))
    }

    /// Change the issue's lease through `op(current, now)`.
    ///
    /// An expired lease is evicted first, so `op` sees it as absent. The
    /// eviction and the change are appended to the claims log in one write and
    /// published to the index before this returns, even when `op` fails; `op`'s
    /// error is then returned. Returns the issue's lease after the change.
    ///
    /// # Errors
    ///
    /// `op`'s error; a `Put` for another issue or one that replaces a different
    /// current lease or changes more than its expiry and heartbeat; or an I/O
    /// failure persisting the change.
    pub fn apply(
        &mut self,
        issue_id: &IssueId,
        op: impl FnOnce(Option<&Lease>, DateTime<Utc>) -> Result<LeaseChange>,
    ) -> Result<Option<Lease>> {
        let now = self.coordinator.clock.now();
        let (expired, current) = match self.index.find_lease(issue_id).cloned() {
            Some(lease) if lease.is_expired(now) => (Some(lease), None),
            stored => (None, stored),
        };
        let outcome = op(current.as_ref(), now)
            .and_then(|change| resolve_change(issue_id, current.as_ref(), change, now));
        let (change_record, next) = match &outcome {
            Ok((record, next)) => (record.clone(), next.clone()),
            Err(_) => (None, current),
        };
        let records = expired
            .map(|lease| ClaimOp::AutoEvict {
                reason: format!(
                    "Lease expired at {}",
                    lease
                        .expires_at
                        .map_or_else(String::new, |at| at.to_string())
                ),
                lease_id: lease.lease_id,
                evicted_at: now,
            })
            .into_iter()
            .chain(change_record)
            .collect::<Vec<_>>();
        self.publish(issue_id, &records, next, now)?;
        outcome.map(|(_, next)| next)
    }

    /// Append `records` in one write, then publish the issue's resulting lease
    /// to the index. The log is what locked reads replay, so its append commits.
    fn publish(
        &mut self,
        issue_id: &IssueId,
        records: &[ClaimOp],
        next: Option<Lease>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let first_seq = self.index.last_seq + 1;
        let lines = records
            .iter()
            .zip(first_seq..)
            .map(|(operation, seq)| {
                serde_json::to_string(&ClaimLogEntry {
                    schema_version: 1,
                    seq,
                    timestamp: now,
                    operation: operation.clone(),
                })
                .map(|json| json + "\n")
            })
            .collect::<serde_json::Result<String>>()?;
        self.coordinator.append_claim_log(lines.as_bytes())?;

        self.index
            .leases
            .retain(|lease| &lease.issue_id != issue_id);
        self.index.leases.extend(next);
        self.index.last_seq = first_seq + records.len() as u64 - 1;
        self.index.generated_at = now;
        self.coordinator.write_index_atomic(&self.index)
    }
}

/// The claims-log record and resulting lease for `change` over `current`.
fn resolve_change(
    issue_id: &IssueId,
    current: Option<&Lease>,
    change: LeaseChange,
    now: DateTime<Utc>,
) -> Result<(Option<ClaimOp>, Option<Lease>)> {
    match change {
        LeaseChange::Keep => Ok((None, current.cloned())),
        LeaseChange::Remove(reason) => Ok((
            current.map(|lease| ClaimOp::ForceEvict {
                lease_id: lease.lease_id.clone(),
                evicted_at: now,
                reason,
            }),
            None,
        )),
        LeaseChange::Put(lease) => {
            if &lease.issue_id != issue_id {
                bail!(
                    "lease {} is for issue {}, not {}",
                    lease.lease_id,
                    lease.issue_id,
                    issue_id
                );
            }
            let record = match current {
                None => ClaimOp::Acquire {
                    lease: lease.clone(),
                },
                Some(current) if current.lease_id != lease.lease_id => bail!(
                    "lease {} would replace lease {} on issue {}",
                    lease.lease_id,
                    current.lease_id,
                    issue_id
                ),
                Some(current)
                    if Lease {
                        expires_at: current.expires_at,
                        last_beat: current.last_beat,
                        stale: current.stale,
                        ..lease.clone()
                    } != *current =>
                {
                    bail!(
                        "lease {} may change only its expiry and heartbeat",
                        lease.lease_id
                    )
                }
                Some(current) if current.expires_at != lease.expires_at => ClaimOp::Renew {
                    lease_id: lease.lease_id.clone(),
                    new_expires_at: lease.expires_at,
                    new_last_beat: lease.last_beat,
                },
                Some(_) => ClaimOp::Heartbeat {
                    lease_id: lease.lease_id.clone(),
                    new_last_beat: lease.last_beat,
                },
            };
            Ok((Some(record), Some(lease)))
        }
    }
}

fn lease_not_found(lease_id: &LeaseId) -> anyhow::Error {
    crate::errors::LeaseNotFoundError::new(&errors::lease_not_found(lease_id.as_str())).into()
}

impl ClaimsMutationGuard {
    /// Return the first active lease on `issue_id` not held by `current_agent`.
    pub(crate) fn conflicting_lease(
        &self,
        issue_id: &IssueId,
        current_agent: Option<&Assignee>,
    ) -> Option<Lease> {
        self.index
            .conflicting_lease(issue_id, current_agent, self.now)
            .cloned()
    }

    /// Non-fatal diagnostics observed in the held control-plane snapshot.
    pub(crate) fn warnings(&self) -> Vec<StorageWarning> {
        self.index.warnings()
    }
}

impl ClaimCoordinator {
    /// Create a new claim coordinator
    ///
    /// # Arguments
    ///
    /// * `paths` - Worktree paths (for control plane location)
    /// * `locker` - File locker for atomic operations
    /// * `worktree_id` - Current worktree ID
    /// * `agent_id` - Current agent ID
    pub fn new(
        paths: WorktreePaths,
        locker: FileLocker,
        worktree_id: WorktreeId,
        agent_id: Assignee,
    ) -> Self {
        Self {
            paths,
            locker,
            worktree_id,
            agent_id,
            fsync: true,
            clock: Arc::new(SystemClock),
        }
    }

    /// Override the clock used for lease expiry and eviction (see the `clock`
    /// field). Test-only: production always uses [`SystemClock`].
    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Override the fsync-on-write behavior (see the `fsync` field).
    ///
    /// Set `false` only in tests that verify in-memory/rebuild invariants
    /// rather than crash durability; it removes the per-write fsync that
    /// otherwise dominates I/O-heavy property tests.
    #[must_use]
    pub fn with_fsync(mut self, fsync: bool) -> Self {
        self.fsync = fsync;
        self
    }

    /// Initialize control plane directories
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(self.paths.shared_jit.join("locks"))
            .context("Failed to create locks directory")?;
        Ok(())
    }

    /// Lock claim evidence for one repository mutation attempt.
    ///
    /// The returned inspection guard holds the mandatory coordination-order
    /// marker and the claims lock until it is dropped. It deliberately performs
    /// no reconciliation or eviction: writers own those mutations, while this
    /// read path only needs a stable control-plane snapshot across repository
    /// capture and apply.
    pub(crate) fn lock_claims_for_repository_mutation(&self) -> Result<ClaimsMutationGuard> {
        let (order, lock) = self.enter_claims_lock()?;
        let now = self.clock.now();
        let index = self.rebuild_index_from_log_at(now, true)?;
        Ok(ClaimsMutationGuard {
            index,
            now,
            _order: order,
            _lock: lock,
        })
    }

    /// Enter coordination, then take `claims.lock`.
    fn enter_claims_lock(
        &self,
    ) -> Result<(
        crate::storage::guard_order::CoordinationOrderGuard,
        crate::storage::lock::LockGuard,
    )> {
        let order = crate::storage::guard_order::CoordinationOrderGuard::enter()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let lock_dir = self.paths.shared_jit.join("locks");
        fs::create_dir_all(&lock_dir)?;
        let lock = self.locker.lock_exclusive_with_metadata(
            &lock_dir.join("claims.lock"),
            &self.agent_id.to_string(),
        )?;
        Ok((order, lock))
    }

    /// Take the [`LeaseLock`], replaying the claims log under it.
    ///
    /// # Errors
    ///
    /// Fails while a repository mutation session is held on this thread, when
    /// the lock wait expires, or when the claims log cannot be replayed.
    pub fn lock(&self) -> Result<LeaseLock<'_>> {
        let (order, lock) = self.enter_claims_lock()?;
        let index = self.rebuild_index_from_log_at(self.clock.now(), true)?;
        Ok(LeaseLock {
            coordinator: self,
            index,
            _lock: lock,
            _order: order,
        })
    }

    /// Acquire a claim and run `repository_sync` with the resulting lease under
    /// the held [`LeaseLock`], in the mandatory order coordinator → (repository
    /// session: bootstrap → repository → events).
    ///
    /// This is the sole claim path that mutates repository-owned issue/event
    /// state. The caller's own lease on the issue (same agent and worktree) is a
    /// convergent retry: it is returned unchanged, even when stale, and no second
    /// acquire is logged. The indefinite-lease limits count active leases only.
    pub fn acquire_claim_synchronized<T>(
        &self,
        issue_id: &IssueId,
        ttl_secs: u64,
        reason: Option<&str>,
        limits: ClaimAcquireLimits,
        repository_sync: impl FnOnce(&Lease) -> Result<T>,
    ) -> Result<(Lease, T)> {
        if ttl_secs == 0 && matches!(reason, None | Some("")) {
            bail!(
                "Indefinite leases (TTL=0) require --reason flag.\n\
                 Example: jit claim acquire {} --ttl 0 --reason \"Manual oversight\"",
                issue_id
            );
        }
        let lease_id = LeaseId::try_from(Uuid::new_v4().to_string())?;
        let branch = self.get_current_branch().ok();

        let mut lock = self.lock()?;
        let now = self.clock.now();
        let indefinite = lock
            .list()?
            .into_iter()
            .filter(|lease| lease.ttl_secs == 0 && lease.is_active(now))
            .collect::<Vec<_>>();
        let agent_indefinite = indefinite
            .iter()
            .filter(|lease| lease.agent_id == self.agent_id)
            .count() as u32;
        let repo_indefinite = indefinite.len() as u32;

        let lease = lock
            .apply(issue_id, |current, now| match current {
                Some(existing)
                    if existing.agent_id == self.agent_id
                        && existing.worktree_id == self.worktree_id =>
                {
                    Ok(LeaseChange::Keep)
                }
                Some(existing) => {
                    let expires_info = match existing.expires_at {
                        Some(expires_at) if existing.ttl_secs > 0 => {
                            format!("until {expires_at}")
                        }
                        _ => format!("(indefinite lease, last beat: {})", existing.last_beat),
                    };
                    bail!(
                        "{}",
                        errors::already_claimed(
                            issue_id,
                            &existing.agent_id.to_string(),
                            &expires_info
                        )
                    )
                }
                None => {
                    if ttl_secs == 0 && agent_indefinite >= limits.max_indefinite_per_agent {
                        bail!(
                            "Exceeded per-agent limit for indefinite leases.\n\
                             Agent {} already has {} indefinite lease(s) (max: {}).\n\
                             Release an existing indefinite lease or use a finite TTL.",
                            self.agent_id,
                            agent_indefinite,
                            limits.max_indefinite_per_agent
                        );
                    }
                    if ttl_secs == 0 && repo_indefinite >= limits.max_indefinite_per_repo {
                        bail!(
                            "Exceeded per-repository limit for indefinite leases.\n\
                             Repository has {} indefinite lease(s) (max: {}).\n\
                             Wait for leases to be released or use a finite TTL.",
                            repo_indefinite,
                            limits.max_indefinite_per_repo
                        );
                    }
                    Ok(LeaseChange::Put(Lease {
                        lease_id,
                        issue_id: issue_id.clone(),
                        agent_id: self.agent_id.clone(),
                        worktree_id: self.worktree_id.clone(),
                        branch,
                        ttl_secs,
                        acquired_at: now,
                        expires_at: (ttl_secs > 0)
                            .then(|| now + Duration::seconds(ttl_secs as i64)),
                        last_beat: now,
                        stale: false,
                    }))
                }
            })?
            .ok_or_else(|| anyhow::anyhow!("acquire left issue {issue_id} without a lease"))?;

        let synced = repository_sync(&lease)?;
        Ok((lease, synced))
    }

    /// Refresh `last_beat` on the caller's lease without changing its expiry.
    ///
    /// # Errors
    ///
    /// The lease does not exist (or has expired), or another agent owns it.
    pub fn heartbeat(&self, lease_id: &LeaseId) -> Result<Lease> {
        let mut lock = self.lock()?;
        let listed = lock.find_by_lease_id(lease_id)?;
        lock.apply(&listed.issue_id, |current, now| {
            if listed.agent_id != self.agent_id {
                bail!(
                    "Cannot heartbeat lease {} owned by {} (you are {})",
                    lease_id,
                    listed.agent_id,
                    self.agent_id
                );
            }
            let current = current
                .filter(|lease| &lease.lease_id == lease_id)
                .ok_or_else(|| lease_not_found(lease_id))?;
            Ok(LeaseChange::Put(Lease {
                last_beat: now,
                stale: false,
                ..current.clone()
            }))
        })?
        .ok_or_else(|| lease_not_found(lease_id))
    }

    /// Append `lines` to the claims log in one write.
    fn append_claim_log(&self, lines: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.shared_jit.join("claims.jsonl"))?;
        file.write_all(lines)?;
        if self.fsync {
            file.sync_all()?;
        }
        Ok(())
    }

    /// Load claims index (or create empty if missing)
    pub fn load_claims_index(&self) -> Result<ClaimsIndex> {
        ClaimsIndex::load(&self.paths)
    }

    /// Write claims index atomically (temp + rename)
    pub fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
        let index_path = self.paths.shared_jit.join("claims.index.json");
        let temp_path = index_path.with_extension("tmp");

        let json = serde_json::to_string_pretty(index)?;
        fs::write(&temp_path, json)?;

        // Fsync for durability
        if self.fsync {
            let file = fs::File::open(&temp_path)?;
            file.sync_all()?;
            drop(file);
        }

        // Atomic rename
        fs::rename(temp_path, index_path)?;

        Ok(())
    }

    /// Evict expired leases from index
    pub fn evict_expired(&self, index: &mut ClaimsIndex) -> Result<()> {
        let now = self.clock.now();
        self.evict_expired_at(index, now)
    }

    fn evict_expired_at(&self, index: &mut ClaimsIndex, now: DateTime<Utc>) -> Result<()> {
        let expired: Vec<_> = index
            .leases
            .iter()
            .filter(|l| l.is_expired(now))
            .cloned()
            .collect();

        for lease in expired {
            // Log eviction
            let op = ClaimOp::AutoEvict {
                lease_id: lease.lease_id.clone(),
                evicted_at: now,
                reason: format!("Lease expired at {}", lease.expires_at.unwrap()),
            };
            self.append_claim_op(&op)?;

            // Remove from index
            index.remove_lease(&lease.lease_id);
        }

        Ok(())
    }

    /// Append a claim operation to the audit log
    fn append_claim_op(&self, op: &ClaimOp) -> Result<()> {
        let log_path = self.paths.shared_jit.join("claims.jsonl");

        // Get next sequence number
        let seq = self.get_next_seq(&log_path)?;

        let entry = ClaimLogEntry {
            schema_version: 1,
            seq,
            timestamp: self.clock.now(),
            operation: op.clone(),
        };

        self.append_claim_log(format!("{}\n", serde_json::to_string(&entry)?).as_bytes())
    }

    /// Get next sequence number from log
    fn get_next_seq(&self, log_path: &PathBuf) -> Result<u64> {
        if !log_path.exists() {
            return Ok(1);
        }

        let content = fs::read_to_string(log_path)?;
        let max_seq = content
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<ClaimLogEntry>(line)
                    .ok()
                    .map(|e| e.seq)
            })
            .max()
            .unwrap_or(0);

        Ok(max_seq + 1)
    }

    /// Get current git branch
    fn get_current_branch(&self) -> Result<String> {
        use std::process::Command;

        let output = Command::new("git")
            .arg("-C")
            .arg(&self.paths.worktree_root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .context("Failed to get current branch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "{}",
                errors::git_command_failed("git rev-parse --abbrev-ref HEAD", &stderr)
            );
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Renew the caller's lease: a finite lease expires `extension_secs` from
    /// now, an indefinite one gets a heartbeat.
    ///
    /// # Errors
    ///
    /// The lease is owned by another agent, or does not exist or has expired.
    /// An expired lease's eviction is persisted before either error.
    pub fn renew_lease(&self, lease_id: &LeaseId, extension_secs: u64) -> Result<Lease> {
        let mut lock = self.lock()?;
        let listed = lock.find_by_lease_id(lease_id)?;
        lock.apply(&listed.issue_id, |current, now| {
            if listed.agent_id != self.agent_id {
                bail!(
                    "{}",
                    errors::not_owner(
                        &format!("lease {}", lease_id),
                        &listed.agent_id.to_string(),
                        &self.agent_id.to_string()
                    )
                );
            }
            let current = current
                .filter(|lease| &lease.lease_id == lease_id)
                .ok_or_else(|| lease_not_found(lease_id))?;
            // A finite lease gains a new expiry; an indefinite one a heartbeat.
            let (expires_at, last_beat) = if current.ttl_secs > 0 {
                (
                    Some(now + Duration::seconds(extension_secs as i64)),
                    current.last_beat,
                )
            } else {
                (None, now)
            };
            Ok(LeaseChange::Put(Lease {
                expires_at,
                last_beat,
                stale: false,
                ..current.clone()
            }))
        })?
        .ok_or_else(|| lease_not_found(lease_id))
    }

    /// Remove a lease whoever owns it (admin operation). A lease that has
    /// already expired is evicted as expired.
    ///
    /// # Errors
    ///
    /// - Lease not found
    pub fn force_evict_lease(&self, lease_id: &LeaseId, reason: &str) -> Result<()> {
        let mut lock = self.lock()?;
        let listed = lock.find_by_lease_id(lease_id)?;
        lock.apply(&listed.issue_id, |current, _| {
            Ok(match current {
                Some(lease) if &lease.lease_id == lease_id => {
                    LeaseChange::Remove(reason.to_string())
                }
                _ => LeaseChange::Keep,
            })
        })
        .map(drop)
    }

    /// Rebuild claims index from JSONL log
    ///
    /// Reconstructs the index by replaying all operations from the append-only log.
    /// This is used for recovery from corruption or to ensure consistency.
    ///
    /// # Algorithm
    ///
    /// 1. Read all entries from claims.jsonl in sequence order
    /// 2. Apply operations to build active leases map:
    ///    - Acquire: add lease
    ///    - Renew: update expires_at and last_beat
    ///    - Release/AutoEvict/ForceEvict: remove lease
    /// 3. Filter out expired finite leases
    /// 4. Track highest sequence number
    ///
    /// # Returns
    ///
    /// A new ClaimsIndex with all active, non-expired leases
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be read or parsed
    pub fn rebuild_index_from_log(&self) -> Result<ClaimsIndex> {
        self.rebuild_index_from_log_at(self.clock.now(), false)
    }

    fn rebuild_index_from_log_at(
        &self,
        now: DateTime<Utc>,
        include_expired: bool,
    ) -> Result<ClaimsIndex> {
        use std::collections::HashMap;
        use std::io::{BufRead, BufReader};

        let log_path = self.paths.shared_jit.join("claims.jsonl");
        let mut active: HashMap<LeaseId, Lease> = HashMap::new();
        let mut max_seq = 0u64;
        let mut expected_seq = 1u64;
        let mut sequence_gaps = Vec::new();

        if log_path.exists() {
            let file = fs::File::open(&log_path).context("Failed to open claims log")?;
            let reader = BufReader::new(file);

            for (index, line) in reader.lines().enumerate() {
                let line = line.context("Failed to read line from claims log")?;
                if line.trim().is_empty() {
                    continue;
                }

                let entry: ClaimLogEntry = serde_json::from_str(&line).with_context(|| {
                    format!(
                        "Failed to parse claim log entry at {}:{}",
                        log_path.display(),
                        index + 1
                    )
                })?;

                // Check for sequence gaps
                if entry.seq != expected_seq {
                    // Detect missing sequences. These are surfaced as
                    // `StorageWarning::SequenceGap` via `ClaimsIndex::warnings()`;
                    // storage must not render diagnostics itself.
                    sequence_gaps.extend(expected_seq..entry.seq);
                }
                expected_seq = entry.seq + 1;
                max_seq = max_seq.max(entry.seq);

                match entry.operation {
                    ClaimOp::Acquire { lease } => {
                        active.insert(lease.lease_id.clone(), lease);
                    }
                    ClaimOp::Renew {
                        lease_id,
                        new_expires_at,
                        new_last_beat,
                    } => {
                        if let Some(lease) = active.get_mut(&lease_id) {
                            lease.expires_at = new_expires_at;
                            lease.last_beat = new_last_beat;
                        }
                    }
                    ClaimOp::Heartbeat {
                        lease_id,
                        new_last_beat,
                    } => {
                        if let Some(lease) = active.get_mut(&lease_id) {
                            lease.last_beat = new_last_beat;
                        }
                    }
                    ClaimOp::Release { lease_id, .. }
                    | ClaimOp::AutoEvict { lease_id, .. }
                    | ClaimOp::ForceEvict { lease_id, .. } => {
                        active.remove(&lease_id);
                    }
                }
            }
        }

        // Expired leases leave the rebuilt index; stale ones stay, flagged below.
        if !include_expired {
            active.retain(|_, lease| !lease.is_expired(now));
        }

        let leases: Vec<Lease> = active
            .into_values()
            .map(|lease| Lease {
                stale: lease.is_stale(now),
                ..lease
            })
            .collect();

        Ok(ClaimsIndex {
            schema_version: 1,
            generated_at: now,
            last_seq: max_seq,
            leases,
            sequence_gaps,
        })
    }

    /// Verify index consistency
    ///
    /// Returns `(true, _)` if the index is valid and `(false, _)` if it needs
    /// rebuilding, paired with any non-fatal [`StorageWarning`]s describing the
    /// inconsistency (e.g. a [`StorageWarning::DuplicateLease`]). The caller's
    /// output layer decides whether and how to render the warnings.
    ///
    /// Note: Does NOT check for expired leases - those are handled by evict_expired().
    /// Expired leases are normal state, not corruption. Treating them as corruption
    /// would trigger unnecessary full index rebuilds.
    pub fn verify_index_consistency(&self) -> Result<(bool, Vec<StorageWarning>)> {
        use std::collections::HashSet;

        let index_path = self.paths.shared_jit.join("claims.index.json");
        if !index_path.exists() {
            let log_has_entries = self
                .paths
                .shared_jit
                .join("claims.jsonl")
                .try_exists()
                .context("Failed to inspect claims log")?
                && fs::read_to_string(self.paths.shared_jit.join("claims.jsonl"))
                    .context("Failed to read claims log")?
                    .lines()
                    .any(|line| !line.trim().is_empty());
            return Ok((!log_has_entries, Vec::new()));
        }

        // Try to parse index
        let index: ClaimsIndex = match fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(idx) => idx,
            None => return Ok((false, Vec::new())), // Corrupted JSON
        };

        // Check for duplicates (actual corruption)
        let mut seen_issues = HashSet::new();
        for lease in &index.leases {
            if !seen_issues.insert(&lease.issue_id) {
                return Ok((
                    false,
                    vec![StorageWarning::DuplicateLease {
                        issue_id: lease.issue_id.clone(),
                    }],
                ));
            }
        }

        // All checks passed
        Ok((true, Vec::new()))
    }
}

/// Acquires a claim, treating an expired lock wait as "this caller has not
/// reached the critical section yet" rather than as the coordinator's answer.
///
/// The concurrency tests below are about what the coordinator decides once
/// callers contend: that distinct issues are every one of them grantable, and
/// that a single issue is granted once. Neither is a statement about elapsed
/// time. A [`LockTimeout`](super::lock::LockTimeout) reports only that this
/// caller was still queued when its wait ran out — a fact about the host's
/// scheduler, not about coordination — so waiting again is what puts the
/// question back to the coordinator. Load on the machine therefore changes how
/// many attempts a caller makes and nothing else about the result.
///
/// The retry ends when nothing at all is happening. `claimants` counts the
/// callers this coordinator has answered, and any answer — a claim granted or a
/// claim refused — is a critical section entered and left, so a rising count is
/// proof the lock is still changing hands. That count is the progress the
/// [`ProgressWatch`] measures: while it moves this caller waits, however long
/// the machine takes about it, and a stall of
/// [`CONTENTION_STALL_LIMIT`](super::contention_probe::CONTENTION_STALL_LIMIT)
/// means the lock is held by something that is not going to release it, which
/// this caller says and fails on.
///
/// That bound measures the whole set of claimants for progress rather than
/// racing this caller against a clock. Load slows claimants down without
/// stopping them, so a stall of that length with claimants still queued is a
/// stuck lock — the one shape of defect this loop could otherwise wait out
/// forever.
#[cfg(test)]
fn acquire_claim_when_reached(
    coordinator: &ClaimCoordinator,
    issue_id: &IssueId,
    ttl_secs: u64,
    claimants: &Claimants,
) -> Result<Lease> {
    let mut watch = super::contention_probe::ProgressWatch::new();
    let mut answered = claimants.answered_count();
    loop {
        match coordinator
            .acquire_claim_synchronized(
                issue_id,
                ttl_secs,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease)
        {
            Err(error) if super::lock::is_lock_timeout(&error) => {
                claimants.contention.record_refusal();
                let now_answered = claimants.answered_count();
                let progressed = now_answered != answered;
                answered = now_answered;
                watch.observe(progressed, |stalled_for| {
                    format!(
                        "no claimant was answered in {stalled_for:?} while this one \
                         waited for the claims lock, so nothing is releasing it; \
                         the last wait ended with: {error}"
                    )
                });
            }
            decided => {
                claimants.record_answer();
                return decided;
            }
        }
    }
}

/// The claimants contending for one coordinator's claims lock, and what they
/// have achieved against it.
///
/// Shared between the threads of one test so each can see the others making
/// progress. The [`Contenders`](super::contention_probe::Contenders) record
/// beneath it holds both halves: which claimants have had a wait for the lock
/// expire, so a test can establish that contention happened rather than assume
/// a schedule produced it, and how many the coordinator has decided about. A
/// claimant that dies without an answer stops contributing progress — which is
/// what it should look like to everyone waiting behind it.
#[cfg(test)]
#[derive(Debug, Default)]
struct Claimants {
    contention: super::contention_probe::Contenders,
}

#[cfg(test)]
impl Claimants {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// How many callers the coordinator has decided about, granted and refused
    /// alike: each entered the critical section and left it.
    fn answered_count(&self) -> usize {
        self.contention.admitted_count()
    }

    fn record_answer(&self) {
        self.contention.record_admission();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration as StdDuration;
    use tempfile::TempDir;

    fn make_lease(ttl_secs: u64, expires_at: Option<DateTime<Utc>>) -> Lease {
        let now = Utc::now();
        Lease {
            lease_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            issue_id: crate::test_utils::issue_id(8),
            agent_id: "agent:1".parse().unwrap(),
            worktree_id: "wt:abc123".parse().unwrap(),
            branch: Some("main".to_string()),
            ttl_secs,
            acquired_at: now,
            expires_at,
            last_beat: now,
            stale: false,
        }
    }

    #[test]
    fn test_lease_is_active_at_expiry_and_heartbeat_boundaries() {
        let now = Utc::now();
        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let cases = [
            (600, Some(now - Duration::nanoseconds(1)), now, false),
            (600, Some(now), now, true),
            (600, Some(now + Duration::nanoseconds(1)), now, true),
            (600, None, now, true),
            (600, Some(now), now - Duration::seconds(threshold + 1), true),
            (0, None, now - Duration::seconds(threshold), true),
            (0, None, now - Duration::seconds(threshold + 1), false),
            (0, Some(now - Duration::seconds(1)), now, true),
        ];
        for (ttl_secs, expires_at, last_beat, expected) in cases {
            for stale in [false, true] {
                let lease = Lease {
                    last_beat,
                    stale,
                    ..make_lease(ttl_secs, expires_at)
                };
                assert_eq!(lease.is_active(now), expected, "{lease:?}");
            }
        }
    }

    fn stored_read_leases(coordinator: &ClaimCoordinator, now: DateTime<Utc>) -> Vec<Lease> {
        let leases = vec![
            make_lease(600, Some(now + Duration::seconds(600))),
            Lease {
                issue_id: crate::test_utils::issue_id(9),
                ..make_lease(600, Some(now - Duration::seconds(1)))
            },
            Lease {
                issue_id: crate::test_utils::issue_id(10),
                last_beat: now
                    - Duration::seconds(
                        crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64 + 1,
                    ),
                ..make_lease(0, None)
            },
        ];
        coordinator
            .write_index_atomic(&ClaimsIndex {
                leases: leases.clone(),
                ..ClaimsIndex::default()
            })
            .unwrap();
        leases
    }

    #[test]
    fn test_read_leases_returns_every_stored_lease_without_locking_or_writing() {
        let temp = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp);
        let leases = stored_read_leases(&coordinator, Utc::now());
        let index_path = coordinator.paths.shared_jit.join("claims.index.json");
        let before = fs::read(&index_path).unwrap();
        let _lock = hold_claims_lock(&temp);

        assert_eq!(read_leases(&coordinator.paths).unwrap(), leases);
        assert_eq!(fs::read(index_path).unwrap(), before);
    }

    #[test]
    fn test_read_active_leases_filters_stored_leases_without_locking() {
        let temp = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp);
        let now = Utc::now();
        let leases = stored_read_leases(&coordinator, now);
        let _lock = hold_claims_lock(&temp);

        assert_eq!(
            read_active_leases(&coordinator.paths, now).unwrap(),
            leases
                .into_iter()
                .filter(|lease| lease.is_active(now))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_read_active_lease_selects_only_the_requested_active_issue_without_locking() {
        let temp = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp);
        let now = Utc::now();
        let leases = stored_read_leases(&coordinator, now);
        let _lock = hold_claims_lock(&temp);

        for lease in &leases {
            assert_eq!(
                read_active_lease(&coordinator.paths, &lease.issue_id, now).unwrap(),
                lease.is_active(now).then(|| lease.clone())
            );
        }
        assert_eq!(
            read_active_lease(&coordinator.paths, &crate::test_utils::issue_id(11), now).unwrap(),
            None
        );
    }

    #[test]
    fn test_read_leases_missing_index_is_empty_and_corrupt_index_is_an_error() {
        let temp = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(temp.path());
        assert!(read_leases(&paths).unwrap().is_empty());
        fs::create_dir_all(&paths.shared_jit).unwrap();
        fs::write(paths.shared_jit.join("claims.index.json"), "invalid json").unwrap();
        assert!(read_leases(&paths)
            .unwrap_err()
            .to_string()
            .contains("Failed to parse claims index"));
    }

    fn setup_coordinator(temp_dir: &TempDir) -> ClaimCoordinator {
        coordinator_with_lock_wait(
            temp_dir,
            StdDuration::from_secs(crate::runtime_defaults::LOCK_TIMEOUT_SECS),
        )
    }

    /// A coordinator whose contended lock waits expire after `lock_wait`.
    fn coordinator_with_lock_wait(temp_dir: &TempDir, lock_wait: StdDuration) -> ClaimCoordinator {
        let paths = crate::test_utils::primary_checkout_paths(temp_dir.path());

        let coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(lock_wait),
            "wt:test123".parse().unwrap(),
            "agent:test".parse().unwrap(),
        );

        coordinator.init().unwrap();
        coordinator
    }

    /// Shorter than any critical section, so a caller that meets the claims lock
    /// held is refused it.
    const EXPIRING_LOCK_WAIT: StdDuration = StdDuration::from_millis(1);

    /// Takes the coordinator's claims lock, so every claimant that asks for it
    /// while the returned guard lives is refused it.
    ///
    /// Nothing else holds the lock when a test calls this, so the take needs no
    /// wait at all and an occupied lock is a defect in the test rather than
    /// contention to sit out.
    fn hold_claims_lock(temp_dir: &TempDir) -> crate::storage::lock::LockGuard {
        let lock_path = temp_dir.path().join(".git/jit/locks/claims.lock");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        FileLocker::new(EXPIRING_LOCK_WAIT)
            .try_lock_exclusive(&lock_path)
            .unwrap()
            .expect("no claimant has started yet, so the claims lock is free")
    }

    #[test]
    fn test_acquire_claim_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(9),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(lease.issue_id, crate::test_utils::issue_id(9));
        assert_eq!(lease.agent_id.to_string(), "agent:test");
        assert_eq!(lease.worktree_id.as_str(), "wt:test123");
        assert_eq!(lease.ttl_secs, 600);
        assert!(lease.expires_at.is_some());
    }

    #[test]
    fn test_acquire_synchronized_recognizes_a_lease_logged_without_its_index() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(10),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        // Simulate a crash where the durable log advanced but the derived index was
        // never published.
        std::fs::remove_file(temp_dir.path().join(".git/jit/claims.index.json")).unwrap();
        // The retry reconciles the index from the append-only log, recognizes the
        // own lease, and never appends a duplicate acquire.
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(10),
                600,
                None,
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(lease.issue_id, crate::test_utils::issue_id(10));
        assert_eq!(coordinator.lock().unwrap().list().unwrap(), vec![lease]);
        let log = std::fs::read_to_string(temp_dir.path().join(".git/jit/claims.jsonl")).unwrap();
        assert_eq!(
            log.lines().count(),
            1,
            "reconcile must recognize the durable lease, not re-acquire"
        );
    }

    #[test]
    fn test_acquire_synchronized_recognizes_a_logged_lease_missing_from_a_stale_index() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(11),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let stale_index =
            std::fs::read(temp_dir.path().join(".git/jit/claims.index.json")).unwrap();
        let (durable, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(12),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        std::fs::write(
            temp_dir.path().join(".git/jit/claims.index.json"),
            stale_index,
        )
        .unwrap();

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(12),
                600,
                None,
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(lease.lease_id, durable.lease_id);
        assert_eq!(coordinator.lock().unwrap().list().unwrap().len(), 2);
        let log = std::fs::read_to_string(temp_dir.path().join(".git/jit/claims.jsonl")).unwrap();
        assert_eq!(log.lines().count(), 2, "retry must not append a duplicate");
    }

    #[test]
    fn test_acquire_synchronized_rejects_another_agents_lease() {
        let temp_dir = TempDir::new().unwrap();
        let owner = setup_coordinator(&temp_dir);
        owner
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(13),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease)
            .unwrap();
        // A different agent cannot acquire the same issue.
        let locker = FileLocker::new(StdDuration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let other = ClaimCoordinator::new(
            crate::test_utils::primary_checkout_paths(temp_dir.path()),
            locker,
            "wt:other".parse().unwrap(),
            "agent:other".parse().unwrap(),
        );
        let result = other.acquire_claim_synchronized(
            &crate::test_utils::issue_id(13),
            600,
            None,
            ClaimAcquireLimits::new(100, 100),
            |_| Ok(()),
        );
        assert!(result.is_err(), "another agent's lease blocks acquisition");
    }

    #[test]
    fn test_acquire_synchronized_is_not_blocked_by_an_expired_lease_of_another_issue() {
        use crate::storage::clock::FixedClock;
        use std::cell::Cell;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(14),
                1,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        clock.set(base + Duration::seconds(2));
        let repository_sync_ran = Cell::new(false);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(15),
                600,
                None,
                ClaimAcquireLimits::new(100, 100),
                |_| {
                    repository_sync_ran.set(true);
                    Ok(())
                },
            )
            .expect("an expired lease on another issue must not block this one");

        assert_eq!(lease.issue_id, crate::test_utils::issue_id(15));
        assert!(repository_sync_ran.get());
        assert!(read_leases(&coordinator.paths).unwrap().contains(&lease));
    }

    #[test]
    fn test_acquire_claim_writes_log_under_shared_jit_not_local_jit() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(16),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // The append-only claim log lives in the shared control plane
        // (.git/jit/claims.jsonl), never in the per-worktree data plane
        // (.jit/claims.jsonl) — `.gitattributes` carries no merge-driver
        // entry for the latter because jit never writes it there.
        assert!(temp_dir.path().join(".git/jit/claims.jsonl").exists());
        assert!(!temp_dir.path().join(".jit/claims.jsonl").exists());
    }

    #[test]
    fn test_acquire_claim_indefinite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(17),
                0,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(lease.ttl_secs, 0);
        assert!(lease.expires_at.is_none());
        assert_eq!(lease.last_beat, lease.acquired_at);
    }

    #[test]
    fn test_concurrent_claim_attempts_serialize() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = Arc::new(temp_dir.path().to_path_buf());

        let barrier = Arc::new(Barrier::new(20)); // Increased from 5 to 20
        let successes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let claimants = Claimants::new();

        let handles: Vec<_> = (0..20) // Increased from 5 to 20
            .map(|i| {
                let temp_path = Arc::clone(&temp_path);
                let barrier = Arc::clone(&barrier);
                let successes = Arc::clone(&successes);
                let claimants = Arc::clone(&claimants);

                thread::spawn(move || {
                    let paths = crate::test_utils::primary_checkout_paths(temp_path.as_path());

                    let locker = FileLocker::new(StdDuration::from_secs(10));
                    let coordinator = ClaimCoordinator::new(
                        paths,
                        locker,
                        WorktreeId::try_from(format!("wt:thread{}", i)).unwrap(),
                        format!("agent:thread{}", i).parse().unwrap(),
                    );

                    coordinator.init().unwrap();

                    // Synchronize start
                    barrier.wait();

                    // All try to claim same issue
                    if let Ok(lease) = acquire_claim_when_reached(
                        &coordinator,
                        &crate::test_utils::issue_id(19),
                        600,
                        &claimants,
                    ) {
                        successes.lock().unwrap().push(lease);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Exactly one should succeed
        let successes = successes.lock().unwrap();
        assert_eq!(
            successes.len(),
            1,
            "Exactly one thread should acquire the claim"
        );
    }

    /// An expired wait is the caller's report that it never reached the
    /// coordinator, and the coordinator answers the same question once the lock
    /// is free. This is the condition the two tests below hold their claimants
    /// in, and the one an arbitrarily busy host produces on its own.
    #[test]
    fn test_acquire_claim_reports_an_expired_wait_while_the_claims_lock_is_held() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = coordinator_with_lock_wait(&temp_dir, EXPIRING_LOCK_WAIT);
        let held = hold_claims_lock(&temp_dir);

        let while_held = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(20),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease)
            .expect_err("the lock is held, so this caller cannot reach the coordinator");

        assert!(
            crate::storage::lock::is_lock_timeout(&while_held),
            "a caller still queued for the lock reports an expired wait rather \
             than an outcome the coordinator decided: {while_held}"
        );
        drop(held);
        assert!(
            coordinator
                .acquire_claim_synchronized(
                    &crate::test_utils::issue_id(20),
                    600,
                    Some("test lease"),
                    ClaimAcquireLimits::new(100, 100),
                    |_| Ok(())
                )
                .map(|(lease, ())| lease)
                .is_ok(),
            "the same call is granted once the lock is free, so the expired wait \
             said nothing about the issue"
        );
    }

    /// Coordination decides who gets a claim; the lock wait only decides how
    /// long a caller queues for the chance to be decided about. These two tests
    /// pin that separation by holding the claims lock until every claimant's
    /// wait for it has expired, so each one meets the condition an arbitrarily
    /// busy host produces and no realistic timeout can rule out. Both once
    /// failed here for want of time rather than for want of coordination;
    /// whoever puts a bound back into the deciding path will see them fail
    /// again.
    #[test]
    fn test_acquire_claim_grants_every_distinct_issue_when_every_lock_wait_expires() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = Arc::new(coordinator_with_lock_wait(&temp_dir, EXPIRING_LOCK_WAIT));
        let issue_ids: Vec<IssueId> = (0..8).map(crate::test_utils::issue_id).collect();
        let claimants = Claimants::new();
        let held = hold_claims_lock(&temp_dir);

        let contenders = issue_ids
            .iter()
            .map(|issue_id| {
                let coordinator = Arc::clone(&coordinator);
                let claimants = Arc::clone(&claimants);
                let issue_id = issue_id.clone();
                thread::spawn(move || {
                    acquire_claim_when_reached(&coordinator, &issue_id, 600, &claimants)
                })
            })
            .collect::<Vec<_>>();

        claimants.contention.await_refusals(issue_ids.len());
        drop(held);

        let granted = contenders
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter_map(|outcome| outcome.ok())
            .map(|lease| lease.issue_id)
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(
            granted,
            issue_ids
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "claims on distinct issues do not compete, so each one is granted \
             however often its holder was refused the lock"
        );
    }

    #[test]
    fn test_acquire_claim_grants_exactly_one_claimant_when_every_lock_wait_expires() {
        let temp_dir = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(temp_dir.path());
        setup_coordinator(&temp_dir);
        let claimant_count = 8;
        let claimants = Claimants::new();
        let held = hold_claims_lock(&temp_dir);

        let contenders = (0..claimant_count)
            .map(|i| {
                let coordinator = ClaimCoordinator::new(
                    paths.clone(),
                    FileLocker::new(EXPIRING_LOCK_WAIT),
                    format!("wt:claimant{i}").parse().unwrap(),
                    format!("agent:claimant{i}").parse().unwrap(),
                );
                let claimants = Arc::clone(&claimants);
                thread::spawn(move || {
                    acquire_claim_when_reached(
                        &coordinator,
                        &crate::test_utils::issue_id(21),
                        600,
                        &claimants,
                    )
                })
            })
            .collect::<Vec<_>>();

        claimants.contention.await_refusals(claimant_count);
        drop(held);

        let outcomes = contenders
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        let granted = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(
            granted, 1,
            "one claimant holds the issue and the rest are refused it"
        );
        assert!(
            outcomes
                .iter()
                .filter_map(|outcome| outcome.as_ref().err())
                .all(|error| error.to_string().contains("already claimed")),
            "every refusal states the issue was already claimed, rather than \
             reporting that the caller ran out of time to ask"
        );
    }

    #[test]
    fn test_claims_log_persisted() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let issue_id = crate::test_utils::issue_id(16);
        coordinator
            .acquire_claim_synchronized(
                &issue_id,
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        assert!(log_path.exists());

        let content = fs::read_to_string(log_path).unwrap();
        assert!(content.contains(issue_id.as_str()));
        assert!(content.contains("acquire"));
    }

    #[test]
    fn test_claim_log_round_trips_existing_lines_byte_for_byte() {
        let header = r#"{"schema_version":1,"seq":1,"timestamp":"2026-01-06T20:00:00Z","op":"#;
        let lease_id = "b53beeba-32f2-4671-a407-2d94e4d13d41";
        let issue_id = crate::test_utils::issue_id(1);
        let fields = [
            format!(
                r#""acquire","lease_id":"{lease_id}","issue_id":"{issue_id}","agent_id":"agent:valid","worktree_id":"wt:main","branch":null,"ttl_secs":0,"acquired_at":"2026-01-06T20:00:00Z","expires_at":null,"last_beat":"2026-01-06T20:00:00Z","stale":false}}"#
            ),
            format!(
                r#""renew","lease_id":"{lease_id}","new_expires_at":null,"new_last_beat":"2026-01-06T20:00:00Z"}}"#
            ),
            format!(
                r#""heartbeat","lease_id":"{lease_id}","new_last_beat":"2026-01-06T20:00:00Z"}}"#
            ),
            format!(r#""release","lease_id":"{lease_id}","released_at":"2026-01-06T20:00:00Z"}}"#),
            format!(
                r#""auto-evict","lease_id":"{lease_id}","evicted_at":"2026-01-06T20:00:00Z","reason":"expired"}}"#
            ),
            format!(
                r#""force-evict","lease_id":"{lease_id}","evicted_at":"2026-01-06T20:00:00Z","reason":"admin"}}"#
            ),
        ];
        for fields in fields {
            let line = format!("{header}{fields}");
            let decoded: ClaimLogEntry = serde_json::from_str(&line).unwrap();
            assert_eq!(serde_json::to_string(&decoded).unwrap(), line);
        }
    }

    /// One acquire entry of the claims log, encoded as a JSON object.
    fn acquire_entry_value() -> serde_json::Value {
        let now = Utc::now();
        serde_json::to_value(ClaimLogEntry {
            schema_version: 1,
            seq: 1,
            timestamp: now,
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
                    issue_id: crate::test_utils::issue_id(7),
                    agent_id: "agent:valid".parse().unwrap(),
                    worktree_id: "wt:main".parse().unwrap(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: now,
                    expires_at: None,
                    last_beat: now,
                    stale: false,
                },
            },
        })
        .unwrap()
    }

    #[test]
    fn test_claim_log_decode_rejects_invalid_agent_id_and_names_field() {
        let mut line = acquire_entry_value();
        line["agent_id"] = serde_json::Value::String("invalid".to_string());
        let line = serde_json::to_string(&line).unwrap();

        let error = serde_json::from_str::<ClaimLogEntry>(&line).unwrap_err();

        assert!(
            error.to_string().contains("agent_id"),
            "the claims-log decode error should name the invalid field, got: {error}"
        );
    }

    #[test]
    fn test_rebuild_index_from_log_rejects_a_non_canonical_issue_id_naming_the_record() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let mut entry = acquire_entry_value();
        let non_canonical = crate::test_utils::issue_id(0xabcd).as_str().to_uppercase();
        entry["issue_id"] = serde_json::Value::String(non_canonical.clone());
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        fs::write(&log_path, format!("{entry}\n")).unwrap();

        let error = format!("{:#}", coordinator.rebuild_index_from_log().unwrap_err());

        assert!(
            error.contains(&format!("{}:1", log_path.display())) && error.contains(&non_canonical),
            "{error}"
        );
    }

    #[test]
    fn test_claims_index_updated() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(22),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        let index_path = temp_dir.path().join(".git/jit/claims.index.json");
        assert!(index_path.exists());

        let index = coordinator.load_claims_index().unwrap();
        assert_eq!(index.leases.len(), 1);
        assert_eq!(index.leases[0].issue_id, crate::test_utils::issue_id(22));
    }

    #[test]
    fn test_acquire_claim_synchronized_takes_over_an_expired_lease_of_another_agent() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let mut coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let issue_id = crate::test_utils::issue_id(23);
        coordinator.agent_id = "agent:old".parse().unwrap();
        let (expired, ()) = acquire(&coordinator, &issue_id, 1, ClaimAcquireLimits::new(1, 1));
        clock.set(base + Duration::seconds(2));
        coordinator.agent_id = "agent:test".parse().unwrap();

        let (lease, ()) = acquire(&coordinator, &issue_id, 600, ClaimAcquireLimits::new(1, 1));

        assert_ne!(lease.lease_id, expired.lease_id);
        assert_eq!(read_leases(&coordinator.paths).unwrap(), vec![lease]);
        assert_evicted_once(&temp_dir, &coordinator, &expired);
    }

    fn acquire(
        coordinator: &ClaimCoordinator,
        issue_id: &IssueId,
        ttl_secs: u64,
        limits: ClaimAcquireLimits,
    ) -> (Lease, ()) {
        coordinator
            .acquire_claim_synchronized(issue_id, ttl_secs, Some("test lease"), limits, |_| Ok(()))
            .unwrap()
    }

    /// `lease` is gone from the store, and the claims log evicted it exactly once.
    fn assert_evicted_once(temp_dir: &TempDir, coordinator: &ClaimCoordinator, lease: &Lease) {
        assert!(read_leases(&coordinator.paths)
            .unwrap()
            .iter()
            .all(|stored| stored.lease_id != lease.lease_id));
        let evictions = logged_operations(temp_dir)
            .into_iter()
            .filter(|op| {
                matches!(op, ClaimOp::AutoEvict { lease_id, .. } if *lease_id == lease.lease_id)
            })
            .count();
        assert_eq!(evictions, 1, "an expired lease is evicted exactly once");
    }

    /// A coordinator for another agent over the same control plane and clock.
    fn coordinator_for(
        temp_dir: &TempDir,
        agent: &str,
        clock: Arc<crate::storage::clock::FixedClock>,
    ) -> ClaimCoordinator {
        ClaimCoordinator::new(
            crate::test_utils::primary_checkout_paths(temp_dir.path()),
            FileLocker::new(StdDuration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            format!("wt:{}", agent.replace(':', "-")).parse().unwrap(),
            agent.parse().unwrap(),
        )
        .with_clock(clock)
    }

    #[test]
    fn test_renew_lease_failing_as_not_found_keeps_the_eviction_of_the_expired_lease() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let (expired, ()) = acquire(
            &coordinator,
            &crate::test_utils::issue_id(24),
            1,
            ClaimAcquireLimits::new(1, 1),
        );
        clock.set(base + Duration::seconds(2));

        for _ in 0..2 {
            let error = coordinator.renew_lease(&expired.lease_id, 600).unwrap_err();
            assert!(error.to_string().contains("not found"), "{error}");
        }

        assert_evicted_once(&temp_dir, &coordinator, &expired);
    }

    #[test]
    fn test_renew_lease_failing_as_not_owned_keeps_the_eviction_of_the_expired_lease() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let owner = coordinator_for(&temp_dir, "agent:owner", clock.clone());
        let (expired, ()) = acquire(
            &owner,
            &crate::test_utils::issue_id(24),
            1,
            ClaimAcquireLimits::new(1, 1),
        );
        clock.set(base + Duration::seconds(2));
        let stranger = coordinator_for(&temp_dir, "agent:stranger", clock);

        let error = stranger.renew_lease(&expired.lease_id, 600).unwrap_err();

        assert!(
            error.to_string().contains("owned by agent:owner"),
            "{error}"
        );
        assert_evicted_once(&temp_dir, &owner, &expired);
    }

    #[test]
    fn test_acquire_claim_synchronized_failing_after_eviction_keeps_the_eviction() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let issue_id = crate::test_utils::issue_id(25);
        let owner = coordinator_for(&temp_dir, "agent:owner", clock.clone());
        let (expired, ()) = acquire(&owner, &issue_id, 1, ClaimAcquireLimits::new(1, 1));
        clock.set(base + Duration::seconds(2));
        let at_limit = setup_coordinator(&temp_dir).with_clock(clock);

        let error = at_limit
            .acquire_claim_synchronized(
                &issue_id,
                0,
                Some("test lease"),
                ClaimAcquireLimits::new(0, 10),
                |_| Ok(()),
            )
            .unwrap_err();

        assert!(error.to_string().contains("per-agent limit"), "{error}");
        assert_evicted_once(&temp_dir, &owner, &expired);
    }

    #[test]
    fn test_acquire_claim_synchronized_limits_count_only_active_indefinite_leases() {
        use crate::storage::clock::FixedClock;

        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let limits = ClaimAcquireLimits::new(1, 1);
        acquire(&coordinator, &crate::test_utils::issue_id(26), 0, limits);
        let blocked = |issue| {
            coordinator
                .acquire_claim_synchronized(
                    &crate::test_utils::issue_id(issue),
                    0,
                    Some("test lease"),
                    limits,
                    |_| Ok(()),
                )
                .is_err()
        };
        assert!(blocked(27), "an active indefinite lease counts");

        clock.set(base + Duration::seconds(threshold + 1));

        assert!(!blocked(27), "a stale indefinite lease does not count");
    }

    fn logged_operations(temp_dir: &TempDir) -> Vec<ClaimOp> {
        fs::read_to_string(temp_dir.path().join(".git/jit/claims.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<ClaimLogEntry>(line)
                    .unwrap()
                    .operation
            })
            .collect()
    }

    #[test]
    fn test_renew_finite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire a finite lease
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(26),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let original_expiry = lease.expires_at.unwrap();

        // Wait a bit
        thread::sleep(StdDuration::from_millis(100));

        // Renew the lease
        let renewed = coordinator.renew_lease(&lease.lease_id, 600).unwrap();

        // Expiry should be extended
        assert!(renewed.expires_at.unwrap() > original_expiry);
        assert_eq!(renewed.issue_id, lease.issue_id);
        assert_eq!(renewed.agent_id, lease.agent_id);
    }

    #[test]
    fn test_renew_indefinite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire an indefinite lease
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(27),
                0,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let original_heartbeat = lease.last_beat;

        // Wait a bit
        thread::sleep(StdDuration::from_millis(100));

        // Renew (heartbeat) the lease
        let renewed = coordinator.renew_lease(&lease.lease_id, 0).unwrap();

        // Heartbeat should be updated
        assert!(renewed.last_beat > original_heartbeat);
        assert_eq!(renewed.ttl_secs, 0);
        assert!(renewed.expires_at.is_none());
    }

    #[test]
    fn test_renew_nonexistent_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let result = coordinator.renew_lease(
            &"00000000-0000-4000-8000-000000000007".parse().unwrap(),
            600,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_renew_lease_not_owned() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator1 = setup_coordinator(&temp_dir);

        // Agent 1 acquires lease
        let (lease, ()) = coordinator1
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(28),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Create coordinator for different agent
        let paths = crate::test_utils::primary_checkout_paths(temp_dir.path());
        let coordinator2 = ClaimCoordinator::new(
            paths,
            FileLocker::new(StdDuration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            "wt:other".parse().unwrap(),
            "agent:other".parse().unwrap(),
        );

        // Agent 2 tries to renew - should fail
        let result = coordinator2.renew_lease(&lease.lease_id, 600);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owned by"));
    }

    #[test]
    fn test_force_evict_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator1 = setup_coordinator(&temp_dir);

        // Agent 1 acquires lease
        let (lease, ()) = coordinator1
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(31),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Create admin coordinator
        let paths = crate::test_utils::primary_checkout_paths(temp_dir.path());
        let admin_coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(StdDuration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            "wt:admin".parse().unwrap(),
            "admin:alice".parse().unwrap(),
        );

        // Admin force evicts (no ownership check)
        admin_coordinator
            .force_evict_lease(&lease.lease_id, "stale after crash")
            .unwrap();

        // Verify lease is gone
        let index = coordinator1.load_claims_index().unwrap();
        assert!(index.find_lease(&crate::test_utils::issue_id(31)).is_none());

        // Should be able to claim again
        coordinator1
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(31),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
    }

    #[test]
    fn test_force_evict_nonexistent_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let result = coordinator.force_evict_lease(
            &"00000000-0000-4000-8000-000000000007".parse().unwrap(),
            "test",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_operations_logged_to_audit_trail() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Perform various operations
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(32),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator.renew_lease(&lease.lease_id, 600).unwrap();
        coordinator
            .force_evict_lease(&lease.lease_id, "test release")
            .unwrap();

        // Check audit log
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        let content = fs::read_to_string(log_path).unwrap();

        assert!(content.contains("acquire"));
        assert!(content.contains("renew"));
        assert!(content.contains("force-evict"));

        let lines: Vec<_> = content.lines().collect();
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_rebuild_index_from_empty_log() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let index = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(index.schema_version, 1);
        assert_eq!(index.last_seq, 0);
        assert_eq!(index.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_with_acquire_operations() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create some leases
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(33),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(34),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Rebuild from log
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(rebuilt.leases.len(), 2);
        assert!(rebuilt
            .leases
            .iter()
            .any(|l| l.issue_id == crate::test_utils::issue_id(33)));
        assert!(rebuilt
            .leases
            .iter()
            .any(|l| l.issue_id == crate::test_utils::issue_id(34)));
        assert_eq!(rebuilt.last_seq, 2);
    }

    #[test]
    fn test_rebuild_index_with_force_eviction() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire and release
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(35),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .force_evict_lease(&lease.lease_id, "test release")
            .unwrap();

        // Rebuild should show no active leases
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_filters_expired_leases() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create lease that expires immediately
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(36),
                1,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Wait for expiry
        thread::sleep(StdDuration::from_millis(1100));

        // Rebuild should filter out expired
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_with_renew_operation() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(37),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let original_expiry = lease.expires_at.unwrap();

        // Renew
        thread::sleep(StdDuration::from_millis(100));
        coordinator.renew_lease(&lease.lease_id, 600).unwrap();

        // Rebuild and verify updated expiry
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 1);
        let renewed = &rebuilt.leases[0];
        assert!(renewed.expires_at.unwrap() > original_expiry);
    }

    #[test]
    fn test_rebuild_index_with_evict_operations() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(38),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Force evict
        coordinator
            .force_evict_lease(&lease.lease_id, "test eviction")
            .unwrap();

        // Rebuild should show no active leases
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_preserves_indefinite_leases() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create indefinite lease
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(39),
                0,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        // Rebuild
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(rebuilt.leases.len(), 1);
        let preserved = &rebuilt.leases[0];
        assert_eq!(preserved.ttl_secs, 0);
        assert!(preserved.expires_at.is_none());
        assert_eq!(preserved.lease_id, lease.lease_id);
    }

    #[test]
    fn test_rebuild_index_sequence_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Multiple operations
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(40),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(41),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(42),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        // Should track last sequence number
        assert_eq!(rebuilt.last_seq, 3);
    }

    #[test]
    fn test_rebuild_detects_sequence_gaps() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create log with gap: sequences 1, 2, 4 (missing 3)
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();

        let entry1 = ClaimLogEntry {
            schema_version: 1,
            seq: 1,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
                    issue_id: crate::test_utils::issue_id(7),
                    agent_id: "agent:test".parse().unwrap(),
                    worktree_id: "wt:main".parse().unwrap(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                    stale: false,
                },
            },
        };

        let entry2 = ClaimLogEntry {
            schema_version: 1,
            seq: 2,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "00000000-0000-4000-8000-000000000003".parse().unwrap(),
                    issue_id: crate::test_utils::issue_id(43),
                    agent_id: "agent:test".parse().unwrap(),
                    worktree_id: "wt:main".parse().unwrap(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                    stale: false,
                },
            },
        };

        // Skip seq 3, jump to 4
        let entry4 = ClaimLogEntry {
            schema_version: 1,
            seq: 4,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "00000000-0000-4000-8000-000000000004".parse().unwrap(),
                    issue_id: crate::test_utils::issue_id(44),
                    agent_id: "agent:test".parse().unwrap(),
                    worktree_id: "wt:main".parse().unwrap(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                    stale: false,
                },
            },
        };

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();

        use std::io::Write;
        writeln!(file, "{}", serde_json::to_string(&entry1).unwrap()).unwrap();
        writeln!(file, "{}", serde_json::to_string(&entry2).unwrap()).unwrap();
        writeln!(file, "{}", serde_json::to_string(&entry4).unwrap()).unwrap();
        drop(file);

        // Should succeed but detect gap
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        // Should still work despite gap
        assert_eq!(rebuilt.leases.len(), 3);
        assert_eq!(rebuilt.last_seq, 4);
        assert!(
            rebuilt.sequence_gaps.contains(&3),
            "Should detect missing sequence 3"
        );
        // The gap must also surface as a typed warning for the output layer.
        assert!(
            rebuilt
                .warnings()
                .contains(&StorageWarning::SequenceGap { missing: 3 }),
            "Sequence gap must surface as a SequenceGap warning, got {:?}",
            rebuilt.warnings()
        );
    }

    #[test]
    fn test_verify_index_consistency_reports_duplicate_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Write an index with two active leases for the same issue (corruption).
        let make_lease = |lease_id: LeaseId| Lease {
            lease_id,
            issue_id: crate::test_utils::issue_id(45),
            agent_id: "agent:test".parse().unwrap(),
            worktree_id: "wt:main".parse().unwrap(),
            branch: None,
            ttl_secs: 600,
            acquired_at: Utc::now(),
            expires_at: Some(Utc::now() + Duration::seconds(600)),
            last_beat: Utc::now(),
            stale: false,
        };
        let index = ClaimsIndex {
            schema_version: 1,
            generated_at: Utc::now(),
            last_seq: 2,
            leases: vec![
                make_lease("00000000-0000-4000-8000-000000000005".parse().unwrap()),
                make_lease("00000000-0000-4000-8000-000000000006".parse().unwrap()),
            ],
            sequence_gaps: Vec::new(),
        };
        coordinator.write_index_atomic(&index).unwrap();

        let (consistent, warnings) = coordinator.verify_index_consistency().unwrap();
        assert!(
            !consistent,
            "Duplicate lease must mark the index inconsistent"
        );
        assert_eq!(
            warnings,
            vec![StorageWarning::DuplicateLease {
                issue_id: crate::test_utils::issue_id(45)
            }],
            "Duplicate lease must surface a DuplicateLease warning"
        );
    }

    #[test]
    fn test_lock_claims_for_repository_mutation_excludes_stale_indefinite_lease() {
        use crate::storage::clock::FixedClock;

        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let issue_id = crate::test_utils::issue_id(46);
        coordinator
            .acquire_claim_synchronized(
                &issue_id,
                0,
                Some("probe"),
                ClaimAcquireLimits::new(1, 1),
                |_| Ok(()),
            )
            .unwrap();

        let is_enforced_after = |elapsed_secs: i64| {
            clock.set(base + Duration::seconds(elapsed_secs));
            coordinator
                .lock_claims_for_repository_mutation()
                .unwrap()
                .conflicting_lease(&issue_id, Some(&"agent:stranger".parse().unwrap()))
                .is_some()
        };
        assert!(is_enforced_after(threshold));
        assert!(!is_enforced_after(threshold + 1));
    }

    #[test]
    fn test_indefinite_lease_per_agent_limit() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Default limit is 2 indefinite leases per agent
        // First two should succeed
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(7),
                0,
                Some("Manual task 1"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(43),
                0,
                Some("Manual task 2"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .unwrap();

        // Third should fail - exceeds per-agent limit
        let result = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(44),
                0,
                Some("Manual task 3"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("per-agent limit") || err.contains("indefinite"),
            "Expected per-agent limit error, got: {}",
            err
        );

        // But finite TTL should still work
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(47),
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
    }

    #[test]
    fn test_indefinite_lease_requires_reason() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // TTL=0 without reason should fail
        let result = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(7),
                0,
                None,
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("reason") || err.contains("--reason"),
            "Expected reason required error, got: {}",
            err
        );

        // TTL=0 with reason should succeed
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(7),
                0,
                Some("Manual oversight"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .unwrap();
    }

    #[test]
    fn test_heartbeat_operation_updates_last_beat() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire indefinite lease
        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(7),
                0,
                Some("Long task"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .unwrap();
        let original_beat = lease.last_beat;

        // Wait a bit
        std::thread::sleep(StdDuration::from_millis(50));

        // Send heartbeat
        let updated = coordinator.heartbeat(&lease.lease_id).unwrap();

        // last_beat should be updated
        assert!(
            updated.last_beat > original_beat,
            "Heartbeat should update last_beat"
        );
        // expires_at should remain None
        assert!(
            updated.expires_at.is_none(),
            "Indefinite lease should not get expiry"
        );
    }

    #[test]
    fn test_heartbeat_logs_to_audit_trail() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let (lease, ()) = coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(7),
                0,
                Some("Manual"),
                ClaimAcquireLimits::new(2, 10),
                |_| Ok(()),
            )
            .unwrap();

        coordinator.heartbeat(&lease.lease_id).unwrap();

        // Check audit log contains heartbeat operation
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        let content = fs::read_to_string(log_path).unwrap();
        assert!(
            content.contains("heartbeat"),
            "Audit log should contain heartbeat operation"
        );
    }

    #[test]
    fn test_claims_mutation_guard_replays_log_and_holds_lock_without_repair() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let issue_id = crate::test_utils::issue_id(46);
        coordinator
            .acquire_claim_synchronized(
                &issue_id,
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let index_path = temp_dir.path().join(".git/jit/claims.index.json");
        fs::remove_file(&index_path).unwrap();

        let guard = coordinator.lock_claims_for_repository_mutation().unwrap();

        let stranger: Assignee = "agent:stranger".parse().unwrap();
        assert!(guard
            .conflicting_lease(&issue_id, Some(&stranger))
            .is_some());
        assert!(
            !index_path.exists(),
            "inspection must not repair derived state"
        );
        let lock_path = temp_dir.path().join(".git/jit/locks/claims.lock");
        let contender = FileLocker::new(StdDuration::from_millis(10));
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_none());
        drop(guard);
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_some());
    }

    #[test]
    fn test_claims_mutation_guard_lease_blocks_only_its_own_issue() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let leased = crate::test_utils::issue_id(48);
        let other = crate::test_utils::issue_id(49);
        coordinator
            .acquire_claim_synchronized(
                &leased,
                600,
                Some("test lease"),
                ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let guard = coordinator.lock_claims_for_repository_mutation().unwrap();
        let stranger: Assignee = "agent:stranger".parse().unwrap();

        assert!(guard.conflicting_lease(&leased, Some(&stranger)).is_some());
        assert!(guard.conflicting_lease(&other, Some(&stranger)).is_none());
    }

    /// A coordinator on a fixed clock with a finite lease on issue 60 that has
    /// expired and an indefinite lease on issue 61 that has gone stale, both
    /// known only to the claims log.
    fn lease_lock_fixture(
        temp_dir: &TempDir,
    ) -> (
        ClaimCoordinator,
        Arc<crate::storage::clock::FixedClock>,
        Lease,
        Lease,
    ) {
        use crate::storage::clock::FixedClock;

        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(temp_dir).with_clock(clock.clone());
        let limits = ClaimAcquireLimits::new(1, 1);
        let (expired, ()) = acquire(&coordinator, &crate::test_utils::issue_id(60), 1, limits);
        let (stale, ()) = acquire(&coordinator, &crate::test_utils::issue_id(61), 0, limits);
        fs::remove_file(coordinator.paths.shared_jit.join("claims.index.json")).unwrap();
        clock.set(base + Duration::seconds(threshold + 1));
        (coordinator, clock, expired, stale)
    }

    fn claims_log_bytes(coordinator: &ClaimCoordinator) -> Vec<u8> {
        fs::read(coordinator.paths.shared_jit.join("claims.jsonl")).unwrap()
    }

    #[test]
    fn test_lease_lock_load_and_list_read_every_logged_lease_including_inactive_ones() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, expired, stale) = lease_lock_fixture(&temp_dir);

        let lock = coordinator.lock().unwrap();

        let mut listed = lock.list().unwrap();
        listed.sort_by_key(|lease| lease.issue_id.clone());
        let ids = |leases: &[Lease]| {
            leases
                .iter()
                .map(|lease| lease.lease_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&listed), ids(&[expired.clone(), stale.clone()]));
        assert_eq!(
            lock.load(&expired.issue_id).unwrap().map(|l| l.lease_id),
            Some(expired.lease_id)
        );
        assert_eq!(lock.load(&crate::test_utils::issue_id(62)).unwrap(), None);
    }

    #[test]
    fn test_lease_lock_holds_the_claims_lock_until_dropped() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let lock_path = temp_dir.path().join(".git/jit/locks/claims.lock");
        let contender = FileLocker::new(EXPIRING_LOCK_WAIT);

        let lock = coordinator.lock().unwrap();
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_none());
        drop(lock);
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_some());
    }

    #[test]
    fn test_lock_fails_inside_a_repository_order_guard() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let _session = crate::storage::guard_order::RepositoryOrderGuard::enter();

        let error = coordinator.lock().err().expect("reverse order is refused");

        assert!(
            error.to_string().contains("repository mutation session"),
            "{error}"
        );
    }

    #[test]
    fn test_lease_lock_apply_keep_returns_the_current_lease_and_writes_nothing() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, _expired, stale) = lease_lock_fixture(&temp_dir);
        let log_before = claims_log_bytes(&coordinator);
        let mut lock = coordinator.lock().unwrap();

        let kept = lock
            .apply(&stale.issue_id, |current, _| {
                assert_eq!(current.map(|l| &l.lease_id), Some(&stale.lease_id));
                Ok(LeaseChange::Keep)
            })
            .unwrap();

        assert_eq!(kept.map(|l| l.lease_id), Some(stale.lease_id));
        assert_eq!(claims_log_bytes(&coordinator), log_before);
        assert!(!coordinator
            .paths
            .shared_jit
            .join("claims.index.json")
            .exists());
    }

    #[test]
    fn test_lease_lock_apply_put_publishes_the_lease_and_logs_its_record() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, _expired, stale) = lease_lock_fixture(&temp_dir);
        let fresh = Lease {
            lease_id: "00000000-0000-4000-8000-000000000063".parse().unwrap(),
            issue_id: crate::test_utils::issue_id(63),
            ..stale.clone()
        };
        let mut lock = coordinator.lock().unwrap();

        let created = lock
            .apply(&fresh.issue_id, |_, _| Ok(LeaseChange::Put(fresh.clone())))
            .unwrap();
        let beaten = lock
            .apply(&stale.issue_id, |current, now| {
                Ok(LeaseChange::Put(Lease {
                    last_beat: now,
                    ..current.unwrap().clone()
                }))
            })
            .unwrap()
            .unwrap();

        assert_eq!(created, Some(fresh.clone()));
        let stored = read_leases(&coordinator.paths).unwrap();
        assert!(stored.contains(&fresh) && stored.contains(&beaten));
        assert_eq!(lock.load(&stale.issue_id).unwrap(), Some(beaten.clone()));
        let logged = logged_operations(&temp_dir);
        assert!(matches!(
            &logged[logged.len() - 2..],
            [ClaimOp::Acquire { lease }, ClaimOp::Heartbeat { lease_id, new_last_beat }]
                if *lease == fresh && *lease_id == beaten.lease_id
                    && *new_last_beat == beaten.last_beat
        ));
    }

    #[test]
    fn test_lease_lock_apply_put_logs_a_new_expiry_as_a_renewal() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let (lease, ()) = acquire(
            &coordinator,
            &crate::test_utils::issue_id(64),
            600,
            ClaimAcquireLimits::new(1, 1),
        );
        let mut lock = coordinator.lock().unwrap();

        let renewed = lock
            .apply(&lease.issue_id, |current, now| {
                Ok(LeaseChange::Put(Lease {
                    expires_at: Some(now + Duration::seconds(1200)),
                    ..current.unwrap().clone()
                }))
            })
            .unwrap()
            .unwrap();

        assert!(matches!(
            logged_operations(&temp_dir).last(),
            Some(ClaimOp::Renew { lease_id, new_expires_at, .. })
                if *lease_id == lease.lease_id && *new_expires_at == renewed.expires_at
        ));
    }

    #[test]
    fn test_lease_lock_apply_remove_unpublishes_the_lease_and_logs_the_reason() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, _expired, stale) = lease_lock_fixture(&temp_dir);
        let mut lock = coordinator.lock().unwrap();

        let after = lock
            .apply(&stale.issue_id, |_, _| {
                Ok(LeaseChange::Remove("operator request".to_string()))
            })
            .unwrap();

        assert_eq!(after, None);
        assert_eq!(lock.load(&stale.issue_id).unwrap(), None);
        assert!(read_leases(&coordinator.paths)
            .unwrap()
            .iter()
            .all(|lease| lease.lease_id != stale.lease_id));
        assert!(matches!(
            logged_operations(&temp_dir).last(),
            Some(ClaimOp::ForceEvict { lease_id, reason, .. })
                if *lease_id == stale.lease_id && reason == "operator request"
        ));
    }

    #[test]
    fn test_lease_lock_apply_evicts_an_expired_lease_before_op_and_persists_it_when_op_fails() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, expired, _stale) = lease_lock_fixture(&temp_dir);
        let mut lock = coordinator.lock().unwrap();

        let error = lock
            .apply(&expired.issue_id, |current, _| {
                assert_eq!(current, None, "op never sees an expired lease");
                bail!("op refused")
            })
            .unwrap_err();
        let retried = lock.apply(&expired.issue_id, |_, _| Ok(LeaseChange::Keep));
        drop(lock);

        assert_eq!(error.to_string(), "op refused");
        assert_eq!(retried.unwrap(), None);
        assert_eq!(
            coordinator.lock().unwrap().load(&expired.issue_id).unwrap(),
            None
        );
        assert_evicted_once(&temp_dir, &coordinator, &expired);
    }

    #[test]
    fn test_lease_lock_apply_rejects_a_put_for_another_issue_or_lease_and_writes_nothing() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, _expired, stale) = lease_lock_fixture(&temp_dir);
        let log_before = claims_log_bytes(&coordinator);
        let mut lock = coordinator.lock().unwrap();
        let other_issue = crate::test_utils::issue_id(65);
        let replacement = Lease {
            lease_id: "00000000-0000-4000-8000-000000000065".parse().unwrap(),
            ..stale.clone()
        };
        let rebranched = Lease {
            branch: Some("elsewhere".to_string()),
            ..stale.clone()
        };

        for (issue_id, put) in [
            (&other_issue, stale.clone()),
            (&stale.issue_id, replacement),
            (&stale.issue_id, rebranched),
        ] {
            assert!(lock
                .apply(issue_id, |_, _| Ok(LeaseChange::Put(put)))
                .is_err());
        }

        assert_eq!(claims_log_bytes(&coordinator), log_before);
        assert_eq!(
            lock.load(&stale.issue_id).unwrap().map(|l| l.lease_id),
            Some(stale.lease_id)
        );
    }
}

// Include property-based tests
#[cfg(test)]
#[path = "claim_coordinator_proptests.rs"]
mod proptests;
