//! Claim coordination for multi-agent parallel work
//!
//! This module implements atomic, race-free claim acquisition using file locks
//! to ensure only one agent can hold a lease on an issue at a time.
//!
//! # Design
//!
//! - Uses exclusive file locks (`fs4` crate) for atomicity
//! - Each issue's lease is one record under `leases/`, changed only under the lock
//! - Claim operations append their history to `claims.jsonl`, which no command reads
//! - Monotonic time semantics for expiration (immune to NTP)
//!
//! See design doc: `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` - "Claim Acquisition Algorithm"

use crate::storage::clock::{Clock, SystemClock};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use super::lock::FileLocker;
use super::worktree_paths::WorktreePaths;
use crate::domain::identity::{IssueId, IssueIdPrefix, LeaseId, WorktreeId};
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
    /// Recovery reclaims an indefinite lease whose heartbeat lapsed
    Reclaim {
        lease_id: LeaseId,
        reclaimed_at: DateTime<Utc>,
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

/// Leases [`ClaimCoordinator::evict_lapsed`] removed, by cause.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LapsedLeases {
    /// Finite leases evicted past their expiry.
    pub expired: usize,
    /// Indefinite leases reclaimed for a stale heartbeat.
    pub reclaimed: usize,
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

/// Read every stored lease without locking or evicting, including inactive
/// leases, ordered by issue.
pub fn read_leases(paths: &WorktreePaths) -> Result<Vec<Lease>> {
    read_lease_records(&paths.shared_jit)
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
    Ok(read_lease_record(&paths.shared_jit, issue_id)?.filter(|lease| lease.is_active(now)))
}

/// The issue's lease record, or `None` when it has none.
fn read_lease_record(shared_jit: &Path, issue_id: &IssueId) -> Result<Option<Lease>> {
    let path = WorktreePaths::lease_path(shared_jit, issue_id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read lease record at {}", path.display()))
        }
    };
    let lease: Lease = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse lease record at {}", path.display()))?;
    if &lease.issue_id != issue_id {
        bail!(
            "Lease record at {} holds a lease for issue {}",
            path.display(),
            lease.issue_id
        );
    }
    Ok(Some(lease))
}

/// Every lease record, ordered by issue. A record removed while the directory
/// is read is skipped.
fn read_lease_records(shared_jit: &Path) -> Result<Vec<Lease>> {
    lease_record_issues(shared_jit)?
        .iter()
        .filter_map(|issue_id| read_lease_record(shared_jit, issue_id).transpose())
        .collect()
}

/// The issues named by lease record file names, sorted, without reading any
/// record. Names other than a lease record's, such as an interrupted
/// operation's stage file, are skipped.
fn lease_record_issues(shared_jit: &Path) -> Result<Vec<IssueId>> {
    let dir = WorktreePaths::leases_dir(shared_jit);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to list lease records in {}", dir.display()))
        }
    };
    let mut issues = entries
        .map(|entry| entry.map(|entry| WorktreePaths::lease_record_issue(&entry.file_name())))
        .filter_map(Result::transpose)
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("Failed to list lease records in {}", dir.display()))?;
    issues.sort();
    Ok(issues)
}

/// One step of [`LeaseLock::apply`]'s publication, named so an interruption
/// matrix can stop the operation at it. A point names the state before its step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeasePoint {
    /// The change is resolved, before the stage file of a stored lease is written.
    BeforeStage,
    /// Before the rename or unlink that commits the lease record.
    BeforeCommit,
    /// The record is committed, before `claims.seq` advances.
    BeforeSequence,
    /// Before the history lines are appended.
    BeforeHistory,
    /// The history lines are appended: the operation is complete.
    AfterHistory,
}

/// Test seam for interrupting one named step of a lease operation.
pub(crate) trait LeaseInterruptionInjector: Send + Sync {
    /// Interrupt the operation at one point, or let it proceed.
    fn check(&self, point: &LeasePoint) -> std::io::Result<()>;
}

/// Production injector: every point proceeds.
struct NoLeaseInterruptions;

impl LeaseInterruptionInjector for NoLeaseInterruptions {
    fn check(&self, _point: &LeasePoint) -> std::io::Result<()> {
        Ok(())
    }
}

/// Claim coordinator for atomic lease operations
#[derive(Clone)]
pub struct ClaimCoordinator {
    paths: WorktreePaths,
    locker: FileLocker,
    worktree_id: WorktreeId,
    agent_id: Assignee,
    /// Whether to fsync lease records, the sequence and the history log on
    /// every write. Defaults to `true` for crash durability in production.
    /// Tests that exercise pure lease logic (not crash recovery) disable it via
    /// [`ClaimCoordinator::with_fsync`]: an fsync costs ~10ms and property
    /// tests do hundreds of writes, so leaving it on makes them minutes long.
    fsync: bool,
    /// Source of "now" for lease expiry and eviction. Defaults to
    /// [`SystemClock`] (the real system clock) in production; tests inject a
    /// deterministic clock so TTL boundaries are exercised without real delays.
    clock: Arc<dyn Clock>,
    interruptions: Arc<dyn LeaseInterruptionInjector>,
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
    /// Remove the current lease as reclaimed for a stale heartbeat, recording
    /// the reason in the claims log.
    Reclaim(String),
}

/// The held claims lock: the one sequence through which every lease changes.
///
/// It holds the coordination-order marker and `claims.lock` until dropped.
/// [`load`](Self::load) and [`list`](Self::list) read lease records, expired
/// and stale leases included; [`apply`](Self::apply) is the only writer.
pub struct LeaseLock {
    coordinator: ClaimCoordinator,
    _lock: crate::storage::lock::LockGuard,
    _order: crate::storage::guard_order::CoordinationOrderGuard,
}

impl LeaseLock {
    /// The issue's stored lease, expired or stale included.
    pub fn load(&self, issue_id: &IssueId) -> Result<Option<Lease>> {
        read_lease_record(&self.coordinator.paths.shared_jit, issue_id)
    }

    /// Every stored lease, expired and stale included, ordered by issue.
    pub fn list(&self) -> Result<Vec<Lease>> {
        read_lease_records(&self.coordinator.paths.shared_jit)
    }

    /// The issue among the stored lease records that `prefix` names, read
    /// from record names only, so a lease whose issue was deleted resolves.
    ///
    /// # Errors
    ///
    /// [`LeaseNotFoundError`](errors::LeaseNotFoundError) when no record
    /// matches, or [`AmbiguousIdError`](crate::storage::AmbiguousIdError) when
    /// several do.
    fn resolve_leased_issue(&self, prefix: &IssueIdPrefix) -> Result<IssueId> {
        let issues = lease_record_issues(&self.coordinator.paths.shared_jit)?;
        crate::storage::resolve_unique_issue_id(prefix, issues.iter(), |id| Some(id.to_string()))
            .map_err(
                |error| match error.is::<crate::storage::IssueNotFoundError>() {
                    true => no_active_lease(prefix),
                    false => error,
                },
            )
    }

    /// Change the issue's lease through `op(current, now)`.
    ///
    /// An expired lease is evicted first, so `op` sees it as absent. The
    /// eviction and the change are published to the issue's lease record and
    /// appended to the claims log in one write before this returns, even when
    /// `op` fails; `op`'s error is then returned. Returns the issue's lease
    /// after the change.
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
        let (expired, current) = match self.load(issue_id)? {
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
        self.publish(issue_id, &records, next.as_ref(), now)?;
        outcome.map(|(_, next)| next)
    }

    /// Publish the issue's resulting lease record, which commits the change,
    /// then take `records`' sequence numbers from `claims.seq`, then append
    /// them to the claims log in one write. An interruption after the commit
    /// loses only history lines.
    fn publish(
        &mut self,
        issue_id: &IssueId,
        records: &[ClaimOp],
        next: Option<&Lease>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        self.coordinator.publish_lease_record(issue_id, next)?;
        self.coordinator
            .check_interruption(LeasePoint::BeforeSequence)?;
        let first_seq = self.coordinator.take_claim_seqs(records.len() as u64)?;
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
        self.coordinator
            .check_interruption(LeasePoint::BeforeHistory)?;
        self.coordinator.append_claim_log(lines.as_bytes())?;
        self.coordinator
            .check_interruption(LeasePoint::AfterHistory)
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
        LeaseChange::Reclaim(reason) => Ok((
            current.map(|lease| ClaimOp::Reclaim {
                lease_id: lease.lease_id.clone(),
                reclaimed_at: now,
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

/// The lease-not-found failure naming `issue`.
pub(crate) fn no_active_lease(issue: impl std::fmt::Display) -> anyhow::Error {
    errors::LeaseNotFoundError::new(&errors::no_active_lease(issue)).into()
}

/// The lease check a repository mutation reads through a [`LeaseLock`] held
/// across its session; it writes nothing.
pub(crate) struct ClaimsMutationGuard {
    lock: LeaseLock,
    now: DateTime<Utc>,
}

impl ClaimsMutationGuard {
    /// Hold `lock` for the mutation, judging activity at the coordinator's now.
    pub(crate) fn hold(lock: LeaseLock) -> Self {
        let now = lock.coordinator.clock.now();
        Self { lock, now }
    }

    /// The issue's active lease when held by anyone but `current_agent`; a
    /// writer with no identity holds none, so every active lease conflicts.
    pub(crate) fn conflicting_lease(
        &self,
        issue_id: &IssueId,
        current_agent: Option<&Assignee>,
    ) -> Result<Option<Lease>> {
        Ok(self.lock.load(issue_id)?.filter(|lease| {
            lease.is_active(self.now) && current_agent.is_none_or(|agent| &lease.agent_id != agent)
        }))
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
            interruptions: Arc::new(NoLeaseInterruptions),
        }
    }

    /// Check every [`LeasePoint`] of this coordinator's lease operations
    /// against `injector`.
    #[cfg(test)]
    pub(crate) fn with_interruptions(
        mut self,
        injector: Arc<dyn LeaseInterruptionInjector>,
    ) -> Self {
        self.interruptions = injector;
        self
    }

    /// Put one point to the injector, naming it in the interruption it reports.
    fn check_interruption(&self, point: LeasePoint) -> Result<()> {
        self.interruptions
            .check(&point)
            .with_context(|| format!("lease operation interrupted at {point:?}"))
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
    /// Set `false` only in tests that verify lease invariants
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

    /// Take the [`LeaseLock`].
    ///
    /// # Errors
    ///
    /// Fails while a repository mutation session is held on this thread or
    /// when the lock wait expires.
    pub fn lock(&self) -> Result<LeaseLock> {
        let (order, lock) = self.enter_claims_lock()?;
        Ok(LeaseLock {
            coordinator: self.clone(),
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
        let branch = self
            .paths
            .write_checkout()
            .and_then(|checkout| checkout.head().ok().flatten())
            .map(|head| head.branch_label().to_owned());

        let mut lock = self.lock()?;
        let now = self.clock.now();
        // Only an indefinite lease has limits, so only it reads other leases.
        let indefinite = if ttl_secs == 0 {
            lock.list()?
                .into_iter()
                .filter(|lease| lease.ttl_secs == 0 && lease.is_active(now))
                .collect()
        } else {
            Vec::new()
        };
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
                    }))
                }
            })?
            .ok_or_else(|| anyhow::anyhow!("acquire left issue {issue_id} without a lease"))?;

        let synced = repository_sync(&lease)?;
        Ok((lease, synced))
    }

    /// Refresh `last_beat` on the caller's lease on `issue_id` without
    /// changing its expiry.
    ///
    /// # Errors
    ///
    /// The issue has no unexpired lease, or another agent owns it. An expired
    /// lease's eviction is persisted before the error.
    pub fn heartbeat(&self, issue_id: &IssueId) -> Result<Lease> {
        self.change_own_lease(issue_id, |current, now| Lease {
            last_beat: now,
            ..current.clone()
        })
    }

    /// Put `change(current, now)` as the issue's lease when the caller owns it.
    fn change_own_lease(
        &self,
        issue_id: &IssueId,
        change: impl FnOnce(&Lease, DateTime<Utc>) -> Lease,
    ) -> Result<Lease> {
        self.lock()?
            .apply(issue_id, |current, now| match current {
                None => Err(no_active_lease(issue_id)),
                Some(current) if current.agent_id != self.agent_id => bail!(
                    "{}",
                    errors::not_owner(
                        &format!("the lease on issue {issue_id}"),
                        &current.agent_id.to_string(),
                        &self.agent_id.to_string()
                    )
                ),
                Some(current) => Ok(LeaseChange::Put(change(current, now))),
            })?
            .ok_or_else(|| no_active_lease(issue_id))
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

    /// Publish `lease` as the issue's lease record, or remove the record for
    /// `None`; the rename or unlink is the commit point.
    fn publish_lease_record(&self, issue_id: &IssueId, lease: Option<&Lease>) -> Result<()> {
        let dir = WorktreePaths::leases_dir(&self.paths.shared_jit);
        let path = WorktreePaths::lease_path(&self.paths.shared_jit, issue_id);
        match lease {
            Some(lease) => {
                if !dir.try_exists()? {
                    fs::create_dir_all(&dir).context("Failed to create lease directory")?;
                    self.sync_directory(&self.paths.shared_jit)?;
                }
                let stage = WorktreePaths::lease_stage_path(&self.paths.shared_jit, issue_id);
                self.check_interruption(LeasePoint::BeforeStage)?;
                self.write_stage(&stage, &serde_json::to_vec_pretty(lease)?)?;
                self.check_interruption(LeasePoint::BeforeCommit)?;
                self.rename_into_place(&stage, &path)
            }
            None => {
                self.check_interruption(LeasePoint::BeforeCommit)?;
                match fs::remove_file(&path) {
                    Ok(()) => self.sync_directory(&dir),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error).with_context(|| {
                        format!("Failed to remove lease record {}", path.display())
                    }),
                }
            }
        }
    }

    /// Advance `claims.seq` by `count` and return the first number taken.
    fn take_claim_seqs(&self, count: u64) -> Result<u64> {
        let path = self.paths.shared_jit.join("claims.seq");
        let last = match fs::read_to_string(&path) {
            Ok(text) => text
                .trim()
                .parse::<u64>()
                .with_context(|| format!("Failed to parse claim sequence at {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to read claim sequence at {}", path.display())
                })
            }
        };
        self.replace_file(
            &self.paths.shared_jit.join("claims.seq.stage"),
            &path,
            (last + count).to_string().as_bytes(),
        )?;
        Ok(last + 1)
    }

    /// Replace `target` with `content` through `stage`.
    fn replace_file(&self, stage: &Path, target: &Path, content: &[u8]) -> Result<()> {
        self.write_stage(stage, content)?;
        self.rename_into_place(stage, target)
    }

    /// Write `content` to `stage`, synced.
    fn write_stage(&self, stage: &Path, content: &[u8]) -> Result<()> {
        if self.fsync {
            super::atomic_write::write_file_synced(stage, content)
        } else {
            fs::write(stage, content).with_context(|| format!("writing {}", stage.display()))
        }
    }

    /// Rename `stage` over `target`, then sync the directory.
    fn rename_into_place(&self, stage: &Path, target: &Path) -> Result<()> {
        fs::rename(stage, target)
            .with_context(|| format!("renaming {} to {}", stage.display(), target.display()))?;
        target
            .parent()
            .map_or(Ok(()), |parent| self.sync_directory(parent))
    }

    fn sync_directory(&self, dir: &Path) -> Result<()> {
        if self.fsync {
            super::atomic_write::sync_directory(dir)
        } else {
            Ok(())
        }
    }

    /// Under one [`LeaseLock`], evict expired leases and reclaim stale
    /// indefinite ones ([`Lease::is_stale`], the rule lease listings report),
    /// each through [`LeaseLock::apply`], which logs one record per lease.
    pub fn evict_lapsed(&self) -> Result<LapsedLeases> {
        let mut lock = self.lock()?;
        lock.list()?
            .into_iter()
            .try_fold(LapsedLeases::default(), |mut lapsed, listed| {
                lock.apply(&listed.issue_id, |current, now| {
                    Ok(match current {
                        // `apply` evicted the listed lease as expired.
                        None => {
                            lapsed.expired += 1;
                            LeaseChange::Keep
                        }
                        Some(lease) if lease.is_stale(now) => {
                            lapsed.reclaimed += 1;
                            LeaseChange::Reclaim(format!("No heartbeat since {}", lease.last_beat))
                        }
                        Some(_) => LeaseChange::Keep,
                    })
                })?;
                Ok(lapsed)
            })
    }

    /// Renew the caller's lease on `issue_id`: a finite lease expires
    /// `extension_secs` from now, an indefinite one gets a heartbeat.
    ///
    /// # Errors
    ///
    /// The issue has no unexpired lease, or another agent owns it. An expired
    /// lease's eviction is persisted before the error.
    pub fn renew_lease(&self, issue_id: &IssueId, extension_secs: u64) -> Result<Lease> {
        self.change_own_lease(issue_id, |current, now| match current.ttl_secs {
            0 => Lease {
                last_beat: now,
                ..current.clone()
            },
            _ => Lease {
                expires_at: Some(now + Duration::seconds(extension_secs as i64)),
                ..current.clone()
            },
        })
    }

    /// Remove, whoever owns it, the lease on the issue among the stored lease
    /// records that `issue` names (admin operation), and return it. The issue
    /// store is not consulted, so a lease on a deleted issue stays evictable.
    ///
    /// # Errors
    ///
    /// No lease record matches `issue`, several do, or the matched lease has
    /// expired; an expired lease's eviction is persisted before the error.
    pub fn force_evict_lease(&self, issue: &IssueIdPrefix, reason: &str) -> Result<Lease> {
        let mut lock = self.lock()?;
        let issue_id = lock.resolve_leased_issue(issue)?;
        let mut evicted = None;
        lock.apply(&issue_id, |current, _| {
            evicted = current.cloned();
            match current {
                Some(_) => Ok(LeaseChange::Remove(reason.to_string())),
                None => Err(no_active_lease(&issue_id)),
            }
        })?;
        evicted.ok_or_else(|| no_active_lease(&issue_id))
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
            let lease = Lease {
                last_beat,
                ..make_lease(ttl_secs, expires_at)
            };
            assert_eq!(lease.is_active(now), expected, "{lease:?}");
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
        for lease in &leases {
            crate::test_lease_store::plant_lease(&coordinator.paths.shared_jit, lease).unwrap();
        }
        leases
    }

    /// Every file under the control plane outside `locks/`, by path, with its
    /// bytes.
    fn control_plane_files(coordinator: &ClaimCoordinator) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, files: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.ends_with("locks") {
                    continue;
                } else if path.is_dir() {
                    walk(&path, files);
                } else {
                    files.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        let mut files = Vec::new();
        walk(&coordinator.paths.shared_jit, &mut files);
        files.sort();
        files
    }

    #[test]
    fn test_read_leases_returns_every_stored_lease_without_locking_or_writing() {
        let temp = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp);
        let leases = stored_read_leases(&coordinator, Utc::now());
        let _lock = hold_claims_lock(&temp);
        let before = control_plane_files(&coordinator);

        assert_eq!(read_leases(&coordinator.paths).unwrap(), leases);
        assert_eq!(control_plane_files(&coordinator), before);
    }

    #[test]
    fn test_read_leases_and_lease_lock_list_skip_names_that_are_not_lease_records() {
        let temp = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp);
        let leases = stored_read_leases(&coordinator, Utc::now());
        let dir = WorktreePaths::leases_dir(&coordinator.paths.shared_jit);
        let issue = crate::test_utils::issue_id(12);
        for name in [
            WorktreePaths::lease_stage_path(&coordinator.paths.shared_jit, &issue)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            format!("{}.json", issue.as_str().to_uppercase()),
            "notes.json".to_string(),
            issue.to_string(),
        ] {
            fs::write(dir.join(name), "not a lease record").unwrap();
        }
        fs::create_dir(dir.join("nested")).unwrap();

        assert_eq!(read_leases(&coordinator.paths).unwrap(), leases);
        assert_eq!(coordinator.lock().unwrap().list().unwrap(), leases);
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
    fn test_read_leases_missing_store_is_empty_and_malformed_record_is_a_parse_error() {
        use crate::test_lease_store::{corrupt_lease, LeaseCorruption};

        let temp = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(temp.path());
        let issue = crate::test_utils::issue_id(13);
        assert!(read_leases(&paths).unwrap().is_empty());
        corrupt_lease(&paths.shared_jit, &issue, LeaseCorruption::MalformedJson).unwrap();

        for error in [
            read_leases(&paths).unwrap_err(),
            read_active_lease(&paths, &issue, Utc::now()).unwrap_err(),
        ] {
            assert!(
                error.to_string().contains("Failed to parse lease record")
                    && error.chain().any(|cause| cause.is::<serde_json::Error>()),
                "{error:#}"
            );
        }
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
                r#""acquire","lease_id":"{lease_id}","issue_id":"{issue_id}","agent_id":"agent:valid","worktree_id":"wt:main","branch":null,"ttl_secs":0,"acquired_at":"2026-01-06T20:00:00Z","expires_at":null,"last_beat":"2026-01-06T20:00:00Z"}}"#
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
    fn test_lease_lock_apply_stores_each_lease_as_its_issue_record_only() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let shared_jit = &coordinator.paths.shared_jit;
        let limits = ClaimAcquireLimits::new(100, 100);
        let (kept, ()) = acquire(&coordinator, &crate::test_utils::issue_id(21), 600, limits);
        let (evicted, ()) = acquire(&coordinator, &crate::test_utils::issue_id(22), 600, limits);
        coordinator
            .force_evict_lease(&IssueIdPrefix::from(&evicted.issue_id), "test")
            .unwrap();

        let records = fs::read_dir(WorktreePaths::leases_dir(shared_jit))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(
            records,
            vec![WorktreePaths::lease_path(shared_jit, &kept.issue_id)]
        );
        let stored: Lease = serde_json::from_slice(&fs::read(&records[0]).unwrap()).unwrap();
        assert_eq!(stored, kept);
        assert!(!shared_jit.join("claims.index.json").exists());
    }

    #[test]
    fn test_lease_lock_apply_takes_sequence_numbers_from_claims_seq() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let seq_path = coordinator.paths.shared_jit.join("claims.seq");
        let issue_id = crate::test_utils::issue_id(20);
        acquire(&coordinator, &issue_id, 1, ClaimAcquireLimits::new(1, 1));
        fs::write(&seq_path, "41").unwrap();
        clock.set(base + Duration::seconds(2));

        // Evicting the expired lease and acquiring anew log two records.
        acquire(&coordinator, &issue_id, 600, ClaimAcquireLimits::new(1, 1));

        let seqs = fs::read_to_string(temp_dir.path().join(".git/jit/claims.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<ClaimLogEntry>(line).unwrap().seq)
            .collect::<Vec<_>>();
        assert_eq!(seqs, vec![1, 42, 43]);
        assert_eq!(fs::read_to_string(&seq_path).unwrap(), "43");
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
            let error = coordinator.renew_lease(&expired.issue_id, 600).unwrap_err();
            assert!(error.to_string().contains("not found"), "{error}");
        }

        assert_evicted_once(&temp_dir, &coordinator, &expired);
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

    #[test]
    fn test_acquire_claim_synchronized_reads_other_leases_only_for_the_indefinite_limit_check() {
        use crate::test_lease_store::{corrupt_lease, LeaseCorruption};

        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let limits = ClaimAcquireLimits::new(10, 10);
        corrupt_lease(
            &coordinator.paths.shared_jit,
            &crate::test_utils::issue_id(28),
            LeaseCorruption::MalformedJson,
        )
        .unwrap();
        let acquire_with_ttl = |issue, ttl_secs| {
            coordinator.acquire_claim_synchronized(
                &crate::test_utils::issue_id(issue),
                ttl_secs,
                Some("test lease"),
                limits,
                |_| Ok(()),
            )
        };

        assert!(acquire_with_ttl(29, 600).is_ok());
        let error = acquire_with_ttl(30, 0).unwrap_err();
        assert!(
            error.to_string().contains("Failed to parse lease record"),
            "{error}"
        );
    }

    #[test]
    fn test_evict_lapsed_logs_one_record_per_lease_and_counts_each_cause_apart() {
        use crate::storage::clock::FixedClock;

        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let temp_dir = TempDir::new().unwrap();
        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let limits = ClaimAcquireLimits::new(10, 10);
        let (silent, ()) = acquire(&coordinator, &crate::test_utils::issue_id(41), 0, limits);
        let (expired, ()) = acquire(&coordinator, &crate::test_utils::issue_id(42), 1, limits);
        clock.set(base + Duration::seconds(threshold + 1));
        let (current, ()) = acquire(&coordinator, &crate::test_utils::issue_id(43), 0, limits);
        let logged_before = logged_operations(&temp_dir).len();

        let lapsed = coordinator.evict_lapsed().unwrap();

        assert_eq!(
            lapsed,
            LapsedLeases {
                expired: 1,
                reclaimed: 1
            }
        );
        assert_eq!(read_leases(&coordinator.paths).unwrap(), vec![current]);
        let appended = logged_operations(&temp_dir).split_off(logged_before);
        assert_eq!(appended.len(), 2, "one record per lapsed lease");
        assert!(appended.iter().any(
            |op| matches!(op, ClaimOp::Reclaim { lease_id, .. } if *lease_id == silent.lease_id)
        ));
        assert!(appended.iter().any(
            |op| matches!(op, ClaimOp::AutoEvict { lease_id, .. } if *lease_id == expired.lease_id)
        ));
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
        let renewed = coordinator.renew_lease(&lease.issue_id, 600).unwrap();

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
        let renewed = coordinator.renew_lease(&lease.issue_id, 0).unwrap();

        // Heartbeat should be updated
        assert!(renewed.last_beat > original_heartbeat);
        assert_eq!(renewed.ttl_secs, 0);
        assert!(renewed.expires_at.is_none());
    }

    #[test]
    fn test_renew_lease_of_an_issue_without_a_lease_fails_as_not_found_naming_the_issue() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let issue_id = crate::test_utils::issue_id(7);

        let error = coordinator.renew_lease(&issue_id, 600).unwrap_err();

        assert!(error.is::<errors::LeaseNotFoundError>(), "{error}");
        assert!(error.to_string().contains(issue_id.as_str()), "{error}");
    }

    #[test]
    fn test_renew_lease_and_heartbeat_by_another_agent_fail_as_not_owned_and_keep_the_lease() {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let owner = coordinator_for(&temp_dir, "agent:owner", clock.clone());
        let (lease, ()) = acquire(
            &owner,
            &crate::test_utils::issue_id(28),
            600,
            ClaimAcquireLimits::new(100, 100),
        );
        let stranger = coordinator_for(&temp_dir, "agent:stranger", clock);

        let errors = [
            stranger.renew_lease(&lease.issue_id, 600).unwrap_err(),
            stranger.heartbeat(&lease.issue_id).unwrap_err(),
        ];

        errors.iter().for_each(|error| {
            assert!(
                error.to_string().contains("owned by agent:owner"),
                "{error}"
            )
        });
        assert_eq!(read_leases(&owner.paths).unwrap(), vec![lease]);
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
        let evicted = admin_coordinator
            .force_evict_lease(&IssueIdPrefix::from(&lease.issue_id), "stale after crash")
            .unwrap();
        assert_eq!(evicted, lease);

        // Verify lease is gone
        assert!(read_leases(&coordinator1.paths).unwrap().is_empty());

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
    fn test_force_evict_lease_of_a_prefix_without_a_lease_record_fails_as_not_found_naming_it() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let prefix: IssueIdPrefix = "deadbeef".parse().unwrap();

        let error = coordinator.force_evict_lease(&prefix, "test").unwrap_err();

        assert!(error.is::<errors::LeaseNotFoundError>(), "{error}");
        assert!(error.to_string().contains("Issue deadbeef "), "{error}");
    }

    #[test]
    fn test_force_evict_lease_resolves_its_prefix_among_lease_record_names() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let [kept, evicted] = [0xabcd_0001, 0xabcd_0002].map(|seed| {
            acquire(
                &coordinator,
                &crate::test_utils::issue_id(seed),
                600,
                ClaimAcquireLimits::new(100, 100),
            )
            .0
        });
        let prefix = |text: &str| text.parse::<IssueIdPrefix>().unwrap();

        let ambiguous = coordinator
            .force_evict_lease(&prefix("abcd"), "test")
            .unwrap_err();
        let removed = coordinator
            .force_evict_lease(&prefix(&evicted.issue_id.as_str()[..8]), "test")
            .unwrap();

        assert!(
            ambiguous.is::<crate::storage::AmbiguousIdError>(),
            "{ambiguous}"
        );
        assert_eq!(removed, evicted);
        assert_eq!(read_leases(&coordinator.paths).unwrap(), vec![kept]);
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
        coordinator.renew_lease(&lease.issue_id, 600).unwrap();
        coordinator
            .force_evict_lease(&IssueIdPrefix::from(&lease.issue_id), "test release")
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
        let updated = coordinator.heartbeat(&lease.issue_id).unwrap();

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

        coordinator.heartbeat(&lease.issue_id).unwrap();

        // Check audit log contains heartbeat operation
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        let content = fs::read_to_string(log_path).unwrap();
        assert!(
            content.contains("heartbeat"),
            "Audit log should contain heartbeat operation"
        );
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
        let guard = ClaimsMutationGuard::hold(coordinator.lock().unwrap());
        let stranger: Assignee = "agent:stranger".parse().unwrap();

        assert!(guard
            .conflicting_lease(&leased, Some(&stranger))
            .unwrap()
            .is_some());
        assert!(guard
            .conflicting_lease(&other, Some(&stranger))
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_claims_mutation_guard_conflicting_lease_anonymous_writer_conflicts_with_every_active_lease(
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let (lease, ()) = acquire(
            &coordinator,
            &crate::test_utils::issue_id(50),
            600,
            ClaimAcquireLimits::new(1, 1),
        );
        let guard = ClaimsMutationGuard::hold(coordinator.lock().unwrap());

        assert!(guard
            .conflicting_lease(&lease.issue_id, Some(&lease.agent_id))
            .unwrap()
            .is_none());
        assert!(guard
            .conflicting_lease(&lease.issue_id, None)
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_claims_mutation_guard_conflicting_lease_inactive_lease_conflicts_with_nobody() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, expired, stale) = lease_lock_fixture(&temp_dir);

        let guard = ClaimsMutationGuard::hold(coordinator.lock().unwrap());

        let stranger: Assignee = "agent:stranger".parse().unwrap();
        for lease in [&expired, &stale] {
            for agent in [Some(&stranger), None] {
                assert!(guard
                    .conflicting_lease(&lease.issue_id, agent)
                    .unwrap()
                    .is_none());
            }
        }
    }

    #[test]
    fn test_claims_mutation_guard_hold_keeps_the_lease_lock_until_dropped() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);
        let lock_path = temp_dir.path().join(".git/jit/locks/claims.lock");
        let contender = FileLocker::new(EXPIRING_LOCK_WAIT);

        let guard = ClaimsMutationGuard::hold(coordinator.lock().unwrap());
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_none());
        drop(guard);
        assert!(contender.try_lock_exclusive(&lock_path).unwrap().is_some());
    }

    /// A coordinator on a fixed clock with a finite lease on issue 60 that has
    /// expired and an indefinite lease on issue 61 that has gone stale.
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
        clock.set(base + Duration::seconds(threshold + 1));
        (coordinator, clock, expired, stale)
    }

    #[test]
    fn test_lease_lock_loads_lists_and_applies_without_the_claims_index_or_log_history() {
        let temp_dir = TempDir::new().unwrap();
        let (coordinator, _clock, expired, stale) = lease_lock_fixture(&temp_dir);
        let shared_jit = &coordinator.paths.shared_jit;
        // Neither file decodes, so reading either one fails the operation.
        fs::write(shared_jit.join("claims.jsonl"), "not a claim record\n").unwrap();
        fs::write(shared_jit.join("claims.index.json"), "not an index").unwrap();

        let mut lock = coordinator.lock().unwrap();

        assert_eq!(lock.list().unwrap().len(), 2);
        assert!(lock.load(&expired.issue_id).unwrap().is_some());
        lock.apply(&stale.issue_id, |_, _| {
            Ok(LeaseChange::Remove("operator request".to_string()))
        })
        .unwrap();
        assert_eq!(lock.load(&stale.issue_id).unwrap(), None);
        assert!(read_leases(&coordinator.paths)
            .unwrap()
            .iter()
            .all(|lease| lease.lease_id != stale.lease_id));
        assert_eq!(
            fs::read(shared_jit.join("claims.index.json")).unwrap(),
            b"not an index"
        );
    }

    #[test]
    fn test_lease_lock_load_and_list_read_every_stored_lease_including_inactive_ones() {
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
        let before = control_plane_files(&coordinator);
        let mut lock = coordinator.lock().unwrap();

        let kept = lock
            .apply(&stale.issue_id, |current, _| {
                assert_eq!(current.map(|l| &l.lease_id), Some(&stale.lease_id));
                Ok(LeaseChange::Keep)
            })
            .unwrap();

        assert_eq!(kept.map(|l| l.lease_id), Some(stale.lease_id));
        assert_eq!(control_plane_files(&coordinator), before);
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
        let before = control_plane_files(&coordinator);
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

        assert_eq!(control_plane_files(&coordinator), before);
    }
}

// Include property-based tests
#[cfg(test)]
#[path = "claim_coordinator_proptests.rs"]
mod proptests;

#[cfg(test)]
#[path = "claim_coordinator_interruption_tests.rs"]
mod interruption_tests;
