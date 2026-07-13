//! Injectable clock abstraction for time-dependent storage logic.
//!
//! Lease expiry and heartbeat freshness need "the current time", but reading it
//! directly from the system clock makes the surrounding logic untestable without
//! real wall-clock delays: a test must `sleep` for a TTL to elapse and then race
//! the assertion against build load. This module lets those code paths obtain the
//! current time from a [`Clock`] so tests can drive time deterministically.
//!
//! Production code uses [`SystemClock`], which delegates to [`chrono::Utc::now`],
//! so behavior is identical to reading the system clock directly.

use chrono::{DateTime, Utc};

/// Source of the current time for expiry and freshness computations.
///
/// Implementors return "now" from [`now`](Clock::now). Production uses
/// [`SystemClock`]; tests can supply a fixed or advanceable clock so that
/// TTL/staleness boundaries are exercised without real delays.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// Return the current time as a UTC timestamp.
    fn now(&self) -> DateTime<Utc>;
}

/// Production [`Clock`] that reads the real system clock.
///
/// Delegates to [`chrono::Utc::now`], so code routed through a `SystemClock`
/// behaves exactly as if it called `Utc::now()` directly.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Test-only clock whose "now" is set explicitly and can be advanced.
    ///
    /// Backed by an atomic millisecond counter so it can be shared across
    /// threads (e.g. a background heartbeat thread) while a test moves time
    /// forward deterministically.
    #[derive(Debug)]
    pub(crate) struct FixedClock {
        millis: AtomicI64,
    }

    impl FixedClock {
        /// Create a clock reporting `instant` as the current time.
        pub(crate) fn new(instant: DateTime<Utc>) -> Self {
            Self {
                millis: AtomicI64::new(instant.timestamp_millis()),
            }
        }

        /// Overwrite the reported time.
        pub(crate) fn set(&self, instant: DateTime<Utc>) {
            self.millis
                .store(instant.timestamp_millis(), Ordering::SeqCst);
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            DateTime::from_timestamp_millis(self.millis.load(Ordering::SeqCst))
                .expect("FixedClock millis in range")
        }
    }
}

#[cfg(test)]
pub(crate) use test_support::FixedClock;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_system_clock_is_monotonic() {
        let clock = SystemClock;
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }

    #[test]
    fn test_fixed_clock_reports_and_advances() {
        let base = Utc::now();
        let clock = FixedClock::new(base);
        assert_eq!(clock.now().timestamp_millis(), base.timestamp_millis());

        let later = base + Duration::seconds(90);
        clock.set(later);
        assert_eq!(clock.now().timestamp_millis(), later.timestamp_millis());
    }
}
