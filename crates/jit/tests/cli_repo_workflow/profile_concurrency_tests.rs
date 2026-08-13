//! Concurrent profile-lifecycle coverage over the real file-backed repository
//! session and transaction boundary.

use super::profile_interruption_tests::{
    changed_profile_fixture, executor_with_failures, ordinary_executor, selector, LifecycleState,
    PACKAGE_LOCATION,
};
use jit::profile::ProfileApplicationStatus;
use jit::storage::{TransactionFailureInjector, TransactionFailurePoint};
use serde_json::Value;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Default)]
struct PauseState {
    reached: bool,
    released: bool,
}

/// Pause one real publication immediately before transaction-control creation.
///
/// The repository mutation session already holds the repository-wide lock at
/// this point, while none of the planned target, ownership, or audit bytes have
/// been published. The condition variable makes both edges of the overlap
/// explicit; elapsed time is not used to decide when the contender has arrived.
#[derive(Debug, Default)]
struct PublicationPause {
    state: Mutex<PauseState>,
    changed: Condvar,
}

impl PublicationPause {
    fn wait_until_reached(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.reached {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }
}

impl TransactionFailureInjector for PublicationPause {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point != &TransactionFailurePoint::RepositoryBeforeControlCreation {
            return Ok(());
        }

        let mut state = self.state.lock().unwrap();
        state.reached = true;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
        Ok(())
    }
}

#[test]
fn test_concurrent_profile_apply_contender_never_publishes_a_stale_capture() {
    let (repository, prior) = changed_profile_fixture();
    let (serial_repository, _) = changed_profile_fixture();
    ordinary_executor(serial_repository.path())
        .apply_profile(&[selector()])
        .unwrap();
    let serial_new = LifecycleState::capture(serial_repository.path());
    let prior_event_count = std::fs::read_to_string(repository.path().join(".jit/events.jsonl"))
        .unwrap()
        .lines()
        .count();
    let pause = Arc::new(PublicationPause::default());
    let winner_path = repository.path().to_path_buf();
    let winner_pause = Arc::clone(&pause);
    let winner = std::thread::spawn(move || {
        executor_with_failures(&winner_path, winner_pause).apply_profile(&[selector()])
    });

    pause.wait_until_reached();
    let contender = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args([
            "profile",
            "apply",
            "--profile",
            &format!("path:{PACKAGE_LOCATION}"),
            "--json",
        ])
        .env("JIT_LOCK_TIMEOUT", "1")
        .current_dir(repository.path())
        .output()
        .expect("run the contending lifecycle process");
    let while_winner_paused = LifecycleState::capture(repository.path());

    // Always release and join the winner before making assertions, so a failed
    // assertion cannot strand a lock-holding test thread.
    pause.release();
    let winner_result = winner.join().expect("winner thread must not panic");

    assert!(!contender.status.success(), "{contender:?}");
    let contention: Value = serde_json::from_slice(&contender.stdout).unwrap_or_else(|error| {
        panic!(
            "contender did not emit JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&contender.stdout),
            String::from_utf8_lossy(&contender.stderr)
        )
    });
    let contention_message = contention["error"]["message"]
        .as_str()
        .expect("contention failure carries a message");
    assert_eq!(contention["error"]["code"], "GENERIC_ERROR");
    assert!(contention_message.contains("Lock timeout"), "{contention}");
    assert!(
        contention_message.contains("exclusive lock"),
        "{contention}"
    );
    assert!(
        contention_message.contains(".jit-bootstrap.lock"),
        "{contention}"
    );

    assert_eq!(
        while_winner_paused, prior,
        "the contending lifecycle process must not expose or publish a partial winner state"
    );
    assert_eq!(
        winner_result.unwrap().requested().unwrap().status,
        ProfileApplicationStatus::Applied
    );

    let after_winner = LifecycleState::capture(repository.path());
    assert!(
        after_winner.is_semantically_equivalent_to(&serial_new),
        "the concurrent outcomes must equal the complete serial winner state"
    );
    let retried = ordinary_executor(repository.path())
        .apply_profile(&[selector()])
        .unwrap();
    assert_eq!(
        retried.requested().unwrap().status,
        ProfileApplicationStatus::Unchanged,
        "a retry must plan against the winner's published state"
    );
    assert_eq!(
        LifecycleState::capture(repository.path()),
        after_winner,
        "the replanned no-op must not republish stale ownership, targets, or audit bytes"
    );

    let record: Value = serde_json::from_slice(
        &std::fs::read(
            repository
                .path()
                .join(".jit/profiles/lifecycle-interruption.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["version"], "2.0.0");
    assert_eq!(
        std::fs::read_to_string(repository.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        prior_event_count + 1,
        "exactly the winning lifecycle publication appends an audit event"
    );
}
