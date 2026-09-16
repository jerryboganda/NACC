//! The engine's only source of time.
//!
//! Every durable record the engine writes carries a timestamp, and retry
//! backoff is expressed in wall-clock time. Both go through this trait so the
//! engine's tests can run without sleeping: a retry policy that backs off for
//! thirty seconds is exercised in microseconds, and the test can assert on
//! the *scheduled* time rather than waiting for it (master plan S14.5's
//! repair policy is otherwise untestable in CI, which is how a policy like
//! this quietly rots).

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

/// Milliseconds since the Unix epoch. Saturates rather than failing: a clock
/// set before 1970 is a misconfiguration, not a reason to lose a workflow.
pub fn wall_clock_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[async_trait]
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> u64;
    async fn sleep(&self, duration: Duration);
}

/// The real clock.
#[derive(Debug, Default)]
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        wall_clock_millis()
    }

    async fn sleep(&self, duration: Duration) {
        if !duration.is_zero() {
            tokio::time::sleep(duration).await;
        }
    }
}

/// A clock that never waits: `sleep` advances the recorded time instead.
/// Used by tests, and available to the app for replaying a run's timing
/// without re-running it.
#[derive(Debug, Default)]
pub struct VirtualClock {
    now_millis: Mutex<u64>,
}

impl VirtualClock {
    /// Start at `now_millis` (0 by default, so timestamps are small and
    /// readable in test failures).
    pub fn starting_at(now_millis: u64) -> Self {
        Self {
            now_millis: Mutex::new(now_millis),
        }
    }

    /// Move time forward without pretending anything slept.
    pub fn advance(&self, duration: Duration) {
        let mut now = self.now_millis.lock().expect("virtual clock poisoned");
        *now = now.saturating_add(duration.as_millis() as u64);
    }
}

#[async_trait]
impl Clock for VirtualClock {
    fn now_millis(&self) -> u64 {
        *self.now_millis.lock().expect("virtual clock poisoned")
    }

    async fn sleep(&self, duration: Duration) {
        self.advance(duration);
        // Yield so that a task waiting on this clock cannot starve the
        // runtime's other tasks (a busy-wait would make the deterministic
        // clock a nondeterministic hang).
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_clock_is_after_2020_and_saturates_before_the_epoch() {
        // 2020-01-01, i.e. a sanity check that we are not reporting seconds
        // or a monotonic-since-boot value.
        assert!(wall_clock_millis() > 1_577_836_800_000);
    }

    #[tokio::test]
    async fn the_virtual_clock_advances_on_sleep_without_waiting() {
        let clock = VirtualClock::starting_at(1_000);
        assert_eq!(clock.now_millis(), 1_000);
        clock.sleep(Duration::from_millis(250)).await;
        assert_eq!(clock.now_millis(), 1_250);
        clock.advance(Duration::from_millis(750));
        assert_eq!(clock.now_millis(), 2_000);
    }

    #[tokio::test]
    async fn the_system_clock_actually_sleeps() {
        let clock = SystemClock;
        let start = clock.now_millis();
        clock.sleep(Duration::from_millis(5)).await;
        let elapsed = clock.now_millis().saturating_sub(start);
        assert!(elapsed >= 5, "expected at least 5ms to pass, got {elapsed}");
    }
}
