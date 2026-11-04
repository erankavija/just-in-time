//! Concurrent profile-lifecycle coverage over the real file-backed repository
//! session and publication boundary.

use super::profile_publication_tests::{
    executor_with_interruptions, prepare_changed_profile, selector, LifecycleState,
    PACKAGE_LOCATION,
};
use jit::profile::ProfileApplicationStatus;
use jit::storage::{MutationInterruptionInjector, MutationPoint, RedoPoint};
use jit::test_cli::JitCli;
use jit::test_utils::file_executor;
use serde_json::Value;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Default)]
struct PauseState {
    reached: bool,
    released: bool,
}

/// Pause one real publication at the moment its record application begins.
///
/// The repository mutation session already holds the repository-wide lock at
/// this point, while none of the planned target or audit bytes have
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

impl MutationInterruptionInjector for PublicationPause {
    fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
        if point != &MutationPoint::Redo(RedoPoint::BeforeApply) {
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
    let repository = JitCli::initialised().unwrap();
    let prior = prepare_changed_profile(&repository);
    let serial_repository = JitCli::initialised().unwrap();
    prepare_changed_profile(&serial_repository);
    jit::test_utils::apply_selected_packages(
        &file_executor(serial_repository.root()).unwrap(),
        &[selector()],
    )
    .unwrap();
    let serial_new = LifecycleState::capture(serial_repository.root());
    let prior_event_count = std::fs::read_to_string(repository.root().join(".jit/events.jsonl"))
        .unwrap()
        .lines()
        .count();
    let pause = Arc::new(PublicationPause::default());
    let winner_path = repository.root().to_path_buf();
    let winner_pause = Arc::clone(&pause);
    let winner = std::thread::spawn(move || {
        jit::test_utils::apply_selected_packages(
            &executor_with_interruptions(&winner_path, winner_pause),
            &[selector()],
        )
    });

    pause.wait_until_reached();
    let contender = repository
        .command_with_env(
            ["profile", "apply", "--profile", PACKAGE_LOCATION, "--json"],
            [("JIT_LOCK_TIMEOUT", "1")],
        )
        .output()
        .expect("run the contending lifecycle process");
    let while_winner_paused = LifecycleState::capture(repository.root());

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
    assert_eq!(
        contention["error"]["code"],
        jit::output::ErrorCode::LockTimeout.as_str()
    );
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

    let after_winner = LifecycleState::capture(repository.root());
    assert!(
        after_winner.is_semantically_equivalent_to(&serial_new),
        "the concurrent outcomes must equal the complete serial winner state"
    );
    let retried = jit::test_utils::apply_selected_packages(
        &file_executor(repository.root()).unwrap(),
        &[selector()],
    )
    .unwrap();
    assert_eq!(
        retried.requested().unwrap().status,
        ProfileApplicationStatus::Unchanged,
        "a retry must plan against the winner's published state"
    );
    assert_eq!(
        LifecycleState::capture(repository.root()),
        after_winner,
        "the replanned no-op must not republish stale targets or audit bytes"
    );

    assert_eq!(
        std::fs::read_to_string(
            repository
                .root()
                .join("docs/lifecycle-interruption/retained.txt")
        )
        .unwrap(),
        "retained content from 2.0.0\n",
        "the winner published the changed package"
    );
    assert_eq!(
        std::fs::read_to_string(repository.root().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        prior_event_count + 1,
        "exactly the winning lifecycle publication appends an audit event"
    );
}
