//! Time controller for testing - allows controlling time progression in tests.

use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

/// Controller for managing time in tests.
///
/// Allows tests to:
/// - Set a fixed current time
/// - Advance time by a specific duration
/// - Skip all pending timers
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::TimeController;
/// use std::time::Duration;
///
/// let time = TimeController::new();
/// time.set_current_time_millis(1000);
/// assert_eq!(time.current_time_millis(), 1000);
///
/// time.advance(Duration::from_secs(5));
/// assert_eq!(time.current_time_millis(), 6000);
/// ```
#[derive(Debug, Clone)]
pub struct TimeController {
    inner: Arc<TimeControllerInner>,
}

#[derive(Debug)]
struct TimeControllerInner {
    current_time_millis: RwLock<i64>,
    pending_timers: RwLock<Vec<PendingTimer>>,
}

#[derive(Debug, Clone)]
struct PendingTimer {
    timer_id: String,
    fire_at_millis: i64,
}

impl Default for TimeController {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeController {
    /// Create a new time controller with the current system time.
    pub fn new() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self::with_initial_time(now)
    }

    /// Create a new time controller with a specific initial time.
    pub fn with_initial_time(initial_time_millis: i64) -> Self {
        Self {
            inner: Arc::new(TimeControllerInner {
                current_time_millis: RwLock::new(initial_time_millis),
                pending_timers: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Get the current time in milliseconds.
    pub fn current_time_millis(&self) -> i64 {
        *self.inner.current_time_millis.read()
    }

    /// Set the current time to a specific value.
    pub fn set_current_time_millis(&self, time_millis: i64) {
        *self.inner.current_time_millis.write() = time_millis;
    }

    /// Advance time by the given duration.
    pub fn advance(&self, duration: Duration) -> Vec<String> {
        let advance_millis = duration.as_millis() as i64;
        let mut current = self.inner.current_time_millis.write();
        *current += advance_millis;
        let new_time = *current;
        drop(current);

        // Fire any timers that should have fired
        self.fire_timers_up_to(new_time)
    }

    /// Advance time to fire the next pending timer.
    /// Returns the timer ID that was fired, or None if no timers are pending.
    pub fn advance_to_next_timer(&self) -> Option<String> {
        let timers = self.inner.pending_timers.read();
        let next_timer = timers.iter().min_by_key(|t| t.fire_at_millis).cloned();
        drop(timers);

        if let Some(timer) = next_timer {
            self.set_current_time_millis(timer.fire_at_millis);
            let fired = self.fire_timers_up_to(timer.fire_at_millis);
            fired.into_iter().next()
        } else {
            None
        }
    }

    /// Skip all pending timers by advancing time past all of them.
    /// Returns the IDs of all fired timers.
    pub fn skip_all_timers(&self) -> Vec<String> {
        let timers = self.inner.pending_timers.read();
        let max_time = timers.iter().map(|t| t.fire_at_millis).max();
        drop(timers);

        if let Some(max) = max_time {
            self.set_current_time_millis(max);
            self.fire_timers_up_to(max)
        } else {
            Vec::new()
        }
    }

    /// Register a timer to fire at a specific time.
    pub fn register_timer(&self, timer_id: &str, fire_at_millis: i64) {
        let mut timers = self.inner.pending_timers.write();
        timers.push(PendingTimer {
            timer_id: timer_id.to_string(),
            fire_at_millis,
        });
    }

    /// Register a timer to fire after a duration from now.
    pub fn register_timer_after(&self, timer_id: &str, duration: Duration) {
        let fire_at = self.current_time_millis() + duration.as_millis() as i64;
        self.register_timer(timer_id, fire_at);
    }

    /// Get all pending timer IDs.
    pub fn pending_timer_ids(&self) -> Vec<String> {
        self.inner
            .pending_timers
            .read()
            .iter()
            .map(|t| t.timer_id.clone())
            .collect()
    }

    /// Check if a specific timer is pending.
    pub fn is_timer_pending(&self, timer_id: &str) -> bool {
        self.inner
            .pending_timers
            .read()
            .iter()
            .any(|t| t.timer_id == timer_id)
    }

    /// Cancel a pending timer.
    pub fn cancel_timer(&self, timer_id: &str) -> bool {
        let mut timers = self.inner.pending_timers.write();
        let initial_len = timers.len();
        timers.retain(|t| t.timer_id != timer_id);
        timers.len() < initial_len
    }

    /// Fire all timers up to the given time and return their IDs.
    fn fire_timers_up_to(&self, time_millis: i64) -> Vec<String> {
        let mut timers = self.inner.pending_timers.write();
        let (fired, remaining): (Vec<_>, Vec<_>) = timers
            .drain(..)
            .partition(|t| t.fire_at_millis <= time_millis);

        *timers = remaining;
        fired.into_iter().map(|t| t.timer_id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_controller_new() {
        let tc = TimeController::new();
        assert!(tc.current_time_millis() > 0);
    }

    #[test]
    fn test_time_controller_with_initial_time() {
        let tc = TimeController::with_initial_time(1000);
        assert_eq!(tc.current_time_millis(), 1000);
    }

    #[test]
    fn test_time_controller_set_time() {
        let tc = TimeController::new();
        tc.set_current_time_millis(5000);
        assert_eq!(tc.current_time_millis(), 5000);
    }

    #[test]
    fn test_time_controller_advance() {
        let tc = TimeController::with_initial_time(1000);
        tc.advance(Duration::from_secs(5));
        assert_eq!(tc.current_time_millis(), 6000);
    }

    #[test]
    fn test_time_controller_register_timer() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-1", 2000);
        assert!(tc.is_timer_pending("timer-1"));
        assert!(!tc.is_timer_pending("timer-2"));
    }

    #[test]
    fn test_time_controller_register_timer_after() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer_after("timer-1", Duration::from_secs(5));
        assert!(tc.is_timer_pending("timer-1"));

        let timers = tc.inner.pending_timers.read();
        assert_eq!(timers[0].fire_at_millis, 6000);
    }

    #[test]
    fn test_time_controller_advance_fires_timers() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-1", 2000);
        tc.register_timer("timer-2", 3000);

        let fired = tc.advance(Duration::from_millis(1500));
        assert_eq!(fired, vec!["timer-1"]);
        assert!(!tc.is_timer_pending("timer-1"));
        assert!(tc.is_timer_pending("timer-2"));
    }

    #[test]
    fn test_time_controller_advance_to_next_timer() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-1", 5000);
        tc.register_timer("timer-2", 3000);

        let fired = tc.advance_to_next_timer();
        assert_eq!(fired, Some("timer-2".to_string()));
        assert_eq!(tc.current_time_millis(), 3000);
        assert!(!tc.is_timer_pending("timer-2"));
        assert!(tc.is_timer_pending("timer-1"));
    }

    #[test]
    fn test_time_controller_skip_all_timers() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-1", 2000);
        tc.register_timer("timer-2", 5000);
        tc.register_timer("timer-3", 3000);

        let fired = tc.skip_all_timers();
        assert_eq!(fired.len(), 3);
        assert_eq!(tc.current_time_millis(), 5000);
        assert!(tc.pending_timer_ids().is_empty());
    }

    #[test]
    fn test_time_controller_cancel_timer() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-1", 2000);

        assert!(tc.cancel_timer("timer-1"));
        assert!(!tc.is_timer_pending("timer-1"));
        assert!(!tc.cancel_timer("timer-1")); // Already cancelled
    }

    #[test]
    fn test_time_controller_pending_timer_ids() {
        let tc = TimeController::with_initial_time(1000);
        tc.register_timer("timer-a", 2000);
        tc.register_timer("timer-b", 3000);

        let ids = tc.pending_timer_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"timer-a".to_string()));
        assert!(ids.contains(&"timer-b".to_string()));
    }

    #[test]
    fn test_time_controller_default() {
        let tc = TimeController::default();
        assert!(tc.current_time_millis() > 0);
    }

    #[test]
    fn test_time_controller_clone() {
        let tc1 = TimeController::with_initial_time(1000);
        let tc2 = tc1.clone();

        tc1.set_current_time_millis(2000);
        assert_eq!(tc2.current_time_millis(), 2000); // Same underlying state
    }
}
