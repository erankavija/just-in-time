//! Interruption matrix over [`LeaseLock::apply`]'s publication steps.

use super::*;
use crate::storage::clock::FixedClock;
use std::sync::Mutex;
use std::time::Duration as StdDuration;
use tempfile::TempDir;

const HOLDER: &str = "agent:holder";

/// Records every point an operation checks, in order.
#[derive(Default)]
struct RecordedPoints(Mutex<Vec<LeasePoint>>);

impl LeaseInterruptionInjector for RecordedPoints {
    fn check(&self, point: &LeasePoint) -> std::io::Result<()> {
        self.0.lock().unwrap().push(point.clone());
        Ok(())
    }
}

/// Interrupts an operation at one named point, so exactly the steps before it ran.
struct InterruptAt(LeasePoint);

impl LeaseInterruptionInjector for InterruptAt {
    fn check(&self, point: &LeasePoint) -> std::io::Result<()> {
        match *point == self.0 {
            true => Err(std::io::Error::other(format!("interrupted at {point:?}"))),
            false => Ok(()),
        }
    }
}

fn leased_issue() -> IssueId {
    crate::test_utils::issue_id(80)
}

/// `agent`'s coordinator over the control plane in `temp`, checking its points
/// against `injector`.
fn actor(
    temp: &TempDir,
    agent: &str,
    clock: &Arc<FixedClock>,
    injector: Arc<dyn LeaseInterruptionInjector>,
) -> ClaimCoordinator {
    ClaimCoordinator::new(
        crate::test_utils::primary_checkout_paths(temp.path()),
        FileLocker::new(StdDuration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        )),
        format!("wt:{}", agent.replace(':', "-")).parse().unwrap(),
        agent.parse().unwrap(),
    )
    .with_fsync(false)
    .with_clock(clock.clone())
    .with_interruptions(injector)
}

fn uninterrupted(temp: &TempDir, agent: &str, clock: &Arc<FixedClock>) -> ClaimCoordinator {
    actor(temp, agent, clock, Arc::new(NoLeaseInterruptions))
}

fn acquire_leased_issue(coordinator: &ClaimCoordinator, ttl_secs: u64) -> Result<Lease> {
    coordinator
        .acquire_claim_synchronized(
            &leased_issue(),
            ttl_secs,
            Some("test lease"),
            ClaimAcquireLimits::new(1, 1),
            |_| Ok(()),
        )
        .map(|(lease, ())| lease)
}

/// One lease operation over the prior state it starts from.
struct Case {
    name: &'static str,
    /// Builds the prior state through uninterrupted operations.
    prior: fn(&TempDir, &Arc<FixedClock>),
    agent: &'static str,
    operation: fn(&ClaimCoordinator) -> Result<()>,
}

fn no_lease(_: &TempDir, _: &Arc<FixedClock>) {}

/// `HOLDER`'s finite lease, ten seconds into its term.
fn held_lease(temp: &TempDir, clock: &Arc<FixedClock>) {
    acquire_leased_issue(&uninterrupted(temp, HOLDER, clock), 600).unwrap();
    clock.set(clock.now() + Duration::seconds(10));
}

/// Another agent's lease, past its expiry.
fn expired_lease(temp: &TempDir, clock: &Arc<FixedClock>) {
    acquire_leased_issue(&uninterrupted(temp, "agent:other", clock), 1).unwrap();
    clock.set(clock.now() + Duration::seconds(2));
}

fn cases() -> [Case; 6] {
    [
        Case {
            name: "acquire",
            prior: no_lease,
            agent: HOLDER,
            operation: |c| acquire_leased_issue(c, 600).map(drop),
        },
        Case {
            name: "acquire over an expired lease",
            prior: expired_lease,
            agent: HOLDER,
            operation: |c| acquire_leased_issue(c, 600).map(drop),
        },
        Case {
            name: "renew",
            prior: held_lease,
            agent: HOLDER,
            operation: |c| c.renew_lease(&leased_issue(), 600).map(drop),
        },
        Case {
            name: "heartbeat",
            prior: held_lease,
            agent: HOLDER,
            operation: |c| c.heartbeat(&leased_issue()).map(drop),
        },
        Case {
            name: "release",
            prior: held_lease,
            agent: HOLDER,
            operation: |c| {
                c.lock()?
                    .apply(&leased_issue(), |_, _| {
                        Ok(LeaseChange::Remove("released".to_string()))
                    })
                    .map(drop)
            },
        },
        Case {
            name: "force-evict",
            prior: held_lease,
            agent: "admin:alice",
            operation: |c| {
                c.force_evict_lease(&IssueIdPrefix::from(&leased_issue()), "admin")
                    .map(drop)
            },
        },
    ]
}

/// The stored state around one run of a case.
struct Run {
    temp: TempDir,
    clock: Arc<FixedClock>,
    prior_lease: Option<Lease>,
    prior_history: String,
    outcome: Result<()>,
}

impl Run {
    fn start(
        case: &Case,
        base: DateTime<Utc>,
        injector: Arc<dyn LeaseInterruptionInjector>,
    ) -> Self {
        let temp = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock::new(base));
        (case.prior)(&temp, &clock);
        let (prior_lease, prior_history) = (stored_lease(&temp), history(&temp));
        let outcome = (case.operation)(&actor(&temp, case.agent, &clock, injector));
        Self {
            temp,
            clock,
            prior_lease,
            prior_history,
            outcome,
        }
    }
}

fn shared_jit(temp: &TempDir) -> std::path::PathBuf {
    crate::test_utils::primary_checkout_paths(temp.path()).shared_jit
}

fn stored_lease(temp: &TempDir) -> Option<Lease> {
    read_lease_record(&shared_jit(temp), &leased_issue()).unwrap()
}

fn history(temp: &TempDir) -> String {
    fs::read_to_string(shared_jit(temp).join("claims.jsonl")).unwrap_or_default()
}

/// A lease with its random id masked, so runs of one case compare.
fn masked(lease: Option<Lease>) -> Option<Lease> {
    lease.map(|lease| Lease {
        lease_id: "00000000-0000-4000-8000-000000000000".parse().unwrap(),
        ..lease
    })
}

fn logged_seqs(temp: &TempDir) -> Vec<u64> {
    history(temp)
        .lines()
        .map(|line| serde_json::from_str::<ClaimLogEntry>(line).unwrap().seq)
        .collect()
}

#[test]
fn test_lease_lock_apply_interrupted_at_any_step_leaves_the_prior_or_the_next_lease() {
    let base = Utc::now();
    for case in cases() {
        let recorded = Arc::new(RecordedPoints::default());
        let reference = Run::start(&case, base, recorded.clone());
        reference.outcome.unwrap();
        let next = masked(stored_lease(&reference.temp));
        let operation_lines = history(&reference.temp)[reference.prior_history.len()..]
            .lines()
            .count();
        assert_ne!(masked(reference.prior_lease.clone()), next, "{}", case.name);
        let points = recorded.0.lock().unwrap().clone();
        let commit = points
            .iter()
            .position(|point| *point == LeasePoint::BeforeCommit)
            .unwrap_or_else(|| panic!("{} commits a lease record", case.name));

        for (index, point) in points.iter().enumerate() {
            let label = format!("{} interrupted at {point:?}", case.name);
            let run = Run::start(&case, base, Arc::new(InterruptAt(point.clone())));
            assert!(run.outcome.is_err(), "{label}");

            let committed = index > commit;
            let expected = match committed {
                true => next.clone(),
                false => masked(run.prior_lease.clone()),
            };
            assert_eq!(masked(stored_lease(&run.temp)), expected, "{label}");
            let logged = history(&run.temp);
            let appended = logged
                .strip_prefix(run.prior_history.as_str())
                .unwrap_or_else(|| panic!("{label}: history kept its prior lines"))
                .lines()
                .count();
            // History never records an uncommitted change, and lacks at most
            // the interrupted operation's lines.
            let logged_lines = match committed {
                true => vec![0, operation_lines],
                false => vec![0],
            };
            assert!(logged_lines.contains(&appended), "{label}");

            // `HOLDER` holds the lease in every case's prior or next state, or
            // may take it from an expired or absent one.
            let followed = acquire_leased_issue(&uninterrupted(&run.temp, HOLDER, &run.clock), 600)
                .unwrap_or_else(|error| panic!("{label}: the next operation failed: {error:#}"));
            assert_eq!(stored_lease(&run.temp), Some(followed), "{label}");
            assert!(
                logged_seqs(&run.temp)
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]),
                "{label}: history sequence numbers stay increasing"
            );
        }
    }
}

#[test]
fn test_acquire_claim_synchronized_retried_past_the_commit_point_finds_its_lease_logging_nothing() {
    for point in [
        LeasePoint::BeforeSequence,
        LeasePoint::BeforeHistory,
        LeasePoint::AfterHistory,
    ] {
        let temp = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let interrupted = actor(&temp, HOLDER, &clock, Arc::new(InterruptAt(point.clone())));
        assert!(
            acquire_leased_issue(&interrupted, 600).is_err(),
            "{point:?}"
        );
        let committed = stored_lease(&temp).unwrap();
        let logged = history(&temp);

        let retried = acquire_leased_issue(&uninterrupted(&temp, HOLDER, &clock), 600).unwrap();

        assert_eq!(retried, committed, "{point:?}");
        assert_eq!(history(&temp), logged, "{point:?}: the retry logs nothing");
    }
}

#[test]
fn test_lease_lock_apply_failing_before_the_commit_leaves_the_prior_lease_and_no_stage() {
    let temp = TempDir::new().unwrap();
    let clock = Arc::new(FixedClock::new(Utc::now()));
    held_lease(&temp, &clock);
    let prior = stored_lease(&temp);
    let interrupted = actor(
        &temp,
        HOLDER,
        &clock,
        Arc::new(InterruptAt(LeasePoint::BeforeCommit)),
    );

    assert!(interrupted.renew_lease(&leased_issue(), 600).is_err());

    assert_eq!(stored_lease(&temp), prior);
    let leases = fs::read_dir(WorktreePaths::leases_dir(&shared_jit(&temp)))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        leases,
        [
            WorktreePaths::lease_path(&shared_jit(&temp), &leased_issue())
                .file_name()
                .unwrap()
                .to_owned()
        ],
        "the failed publication left its stage beside the lease record"
    );
}
