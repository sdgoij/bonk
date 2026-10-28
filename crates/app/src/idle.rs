//! Idle lock: drop the keys when the user walks away.
//!
//! The timer is a plain value with a clock passed in, so its behaviour is
//! testable without a window or a nap. The UI only decides when to call it.

use std::time::{Duration, Instant};

/// Default idle limit in minutes.
pub const DEFAULT_IDLE_MINUTES: u64 = 15;
/// Shortest limit the environment variable can ask for.
pub const MIN_IDLE_MINUTES: u64 = 1;
/// Longest limit the environment variable can ask for: a day.
pub const MAX_IDLE_MINUTES: u64 = 24 * 60;

/// How long the workspace may sit untouched before the keys are dropped.
///
/// Fire-once by construction: after `due` returns true it stays disarmed until
/// activity re-arms it, so a locked screen cannot keep re-locking.
#[derive(Clone, Debug)]
pub struct IdleLock {
    timeout: Duration,
    last_activity: Instant,
    armed: bool,
}

impl IdleLock {
    /// Starts armed with `timeout` and the clock at `now`.
    pub fn new(timeout: Duration, now: Instant) -> Self {
        Self {
            timeout,
            last_activity: now,
            armed: true,
        }
    }

    /// The configured limit.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Records activity, restarting the clock and re-arming the lock.
    pub fn touch(&mut self, now: Instant) {
        self.last_activity = now;
        self.armed = true;
    }

    /// True when the limit has passed since the last activity.
    ///
    /// Saturating on purpose: a clock that appears to move backwards must not
    /// panic in the middle of a frame.
    pub fn due(&mut self, now: Instant) -> bool {
        if !self.armed || now.saturating_duration_since(self.last_activity) < self.timeout {
            return false;
        }
        self.armed = false;
        true
    }
}

/// Reads the limit from `BONK_IDLE_MINUTES`, falling back to the default.
///
/// Out-of-range and unparsable values are clamped rather than refused: a typo in
/// an environment variable must not leave the workspace unlocked forever.
pub fn timeout_from(value: Option<&str>) -> Duration {
    let minutes = value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_IDLE_MINUTES)
        .clamp(MIN_IDLE_MINUTES, MAX_IDLE_MINUTES);
    Duration::from_secs(minutes * 60)
}

/// The limit this process should use.
pub fn configured_timeout() -> Duration {
    timeout_from(std::env::var("BONK_IDLE_MINUTES").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn the_lock_fires_once_after_the_limit() {
        let start = Instant::now();
        let mut idle = IdleLock::new(Duration::from_secs(15 * 60), start);

        assert!(!idle.due(start + Duration::from_secs(14 * 60)));
        assert!(idle.due(start + Duration::from_secs(15 * 60)));
        assert!(
            !idle.due(start + Duration::from_secs(16 * 60)),
            "the lock fires once, not on every frame"
        );
    }

    #[test]
    fn activity_rearms_the_lock() {
        let start = Instant::now();
        let mut idle = IdleLock::new(Duration::from_secs(15 * 60), start);
        assert!(idle.due(start + Duration::from_secs(16 * 60)));

        idle.touch(start + Duration::from_secs(20 * 60));
        assert!(!idle.due(start + Duration::from_secs(34 * 60)));
        assert!(idle.due(start + Duration::from_secs(35 * 60)));
    }

    #[test]
    fn a_backwards_clock_does_not_panic() {
        let start = Instant::now();
        let mut idle = IdleLock::new(MINUTE, start + Duration::from_secs(60));
        assert!(!idle.due(start));
    }

    #[test]
    fn the_environment_value_is_clamped_rather_than_trusted() {
        assert_eq!(timeout_from(None), Duration::from_secs(DEFAULT_IDLE_MINUTES * 60));
        assert_eq!(timeout_from(Some("1")), MINUTE);
        assert_eq!(timeout_from(Some("0")), MINUTE, "zero would mean never");
        assert_eq!(timeout_from(Some("999999")), Duration::from_secs(MAX_IDLE_MINUTES * 60));
        assert_eq!(
            timeout_from(Some("fifteen")),
            Duration::from_secs(DEFAULT_IDLE_MINUTES * 60)
        );
        assert_eq!(timeout_from(Some(" 30 ")), Duration::from_secs(30 * 60));
    }
}
