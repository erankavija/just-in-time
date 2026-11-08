//! The `process` module is the single owner of PID conversion and process liveness. Its guard
//! prevents sentinel or lossy values from reaching `kill(2)`, and its probe keeps permission-limited
//! live processes from being treated as stale.

/// Converts a PID only when it is positive and representable by `kill(2)`'s `i32` interface.
pub fn checked_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|pid| *pid > 0)
}

/// Converts a safe PID to the negative process-group ID used by `kill(2)`; PID 1 is refused
/// because negating it would target every process owned by the caller.
pub fn process_group_target(pid: u32) -> Option<i32> {
    checked_pid(pid).filter(|pid| *pid > 1).map(|pid| -pid)
}

/// Returns whether a PID should be treated as live. A guard refusal or `ESRCH` is dead; success,
/// `EPERM`, and any other errno are alive. On non-Unix platforms, an accepted PID is alive. In
/// particular, a lock or server held by another user's live process must not be treated as stale.
pub(crate) fn is_alive(pid: u32) -> bool {
    checked_pid(pid).is_some_and(probe)
}

/// Returns whether any process belongs to the group `pgid` leads, under the same errno policy as
/// `is_alive`. A guard refusal (pgid 0, 1, or unrepresentable) is dead.
pub fn is_group_alive(pgid: u32) -> bool {
    process_group_target(pgid).is_some_and(probe)
}

/// The module's only signal-zero probe over an already-guarded `kill(2)` target.
fn probe(target: i32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        probe_result_is_alive(kill(Pid::from_raw(target), None))
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        true
    }
}

#[cfg(unix)]
fn probe_result_is_alive(result: Result<(), nix::errno::Errno>) -> bool {
    match result {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(nix::errno::Errno::ESRCH) => false,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{checked_pid, is_alive, is_group_alive, process_group_target};

    #[cfg(unix)]
    use super::probe_result_is_alive;

    #[test]
    fn test_is_alive_reports_current_process_alive() {
        assert!(is_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_alive_reports_dead_for_exited_process() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn 'true'");
        let pid = child.id();
        child.wait().expect("failed to wait on child");

        assert!(!is_alive(pid));
    }

    #[test]
    fn test_is_alive_rejects_guard_refused_pids() {
        assert!(!is_alive(0));
        assert!(!is_alive(u32::MAX));
    }

    #[test]
    fn test_is_group_alive_rejects_guard_refused_pgids() {
        assert!(!is_group_alive(0));
        assert!(!is_group_alive(1));
        assert!(!is_group_alive(u32::MAX));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_group_alive_reports_own_group_alive() {
        use nix::unistd::getpgrp;
        assert!(is_group_alive(getpgrp().as_raw() as u32));
    }

    #[test]
    fn test_checked_pid_rejects_zero_and_overflow() {
        assert_eq!(checked_pid(0), None);
        assert_eq!(checked_pid(u32::MAX), None);
    }

    #[test]
    fn test_process_group_target_rejects_unsafe_pids() {
        assert_eq!(process_group_target(0), None);
        assert_eq!(process_group_target(1), None);
        assert_eq!(process_group_target(u32::MAX), None);
    }

    #[test]
    fn test_process_group_target_converts_pid_to_negative_group_id() {
        assert_eq!(process_group_target(42), Some(-42));
    }

    #[cfg(unix)]
    #[test]
    fn test_probe_treats_permission_denied_as_alive() {
        assert!(probe_result_is_alive(Err(nix::errno::Errno::EPERM)));
    }

    #[cfg(unix)]
    #[test]
    fn test_probe_treats_no_such_process_as_dead() {
        assert!(!probe_result_is_alive(Err(nix::errno::Errno::ESRCH)));
    }
}
