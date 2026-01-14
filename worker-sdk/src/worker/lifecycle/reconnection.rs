//! Reconnection strategy and policy types.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Strategy for handling reconnection after connection loss.
#[derive(Clone)]
pub enum ReconnectionStrategy {
    /// No automatic reconnection.
    None,

    /// Fixed delay between attempts.
    Fixed {
        /// Delay between attempts.
        delay: Duration,
        /// Maximum number of attempts (None = infinite).
        max_attempts: Option<u32>,
    },

    /// Exponential backoff between attempts.
    ExponentialBackoff {
        /// Initial delay before first retry.
        initial_delay: Duration,
        /// Maximum delay between attempts.
        max_delay: Duration,
        /// Multiplier for each subsequent attempt.
        multiplier: f64,
        /// Maximum number of attempts (None = infinite).
        max_attempts: Option<u32>,
    },

    /// Custom strategy using a policy trait.
    Custom(Arc<dyn ReconnectionPolicy>),
}

impl Default for ReconnectionStrategy {
    fn default() -> Self {
        Self::ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: None,
        }
    }
}

impl std::fmt::Debug for ReconnectionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "ReconnectionStrategy::None"),
            Self::Fixed {
                delay,
                max_attempts,
            } => f
                .debug_struct("ReconnectionStrategy::Fixed")
                .field("delay", delay)
                .field("max_attempts", max_attempts)
                .finish(),
            Self::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                max_attempts,
            } => f
                .debug_struct("ReconnectionStrategy::ExponentialBackoff")
                .field("initial_delay", initial_delay)
                .field("max_delay", max_delay)
                .field("multiplier", multiplier)
                .field("max_attempts", max_attempts)
                .finish(),
            Self::Custom(_) => write!(f, "ReconnectionStrategy::Custom(...)"),
        }
    }
}

impl ReconnectionStrategy {
    /// Creates a fixed delay strategy.
    pub fn fixed(delay: Duration) -> Self {
        Self::Fixed {
            delay,
            max_attempts: None,
        }
    }

    /// Creates a fixed delay strategy with max attempts.
    pub fn fixed_with_max(delay: Duration, max_attempts: u32) -> Self {
        Self::Fixed {
            delay,
            max_attempts: Some(max_attempts),
        }
    }

    /// Creates an exponential backoff strategy with default settings.
    pub fn exponential_backoff() -> Self {
        Self::default()
    }

    /// Calculates the delay for the given attempt number.
    ///
    /// Returns `None` if max attempts have been reached.
    pub fn calculate_delay(&self, attempt: u32) -> Option<Duration> {
        match self {
            Self::None => None,
            Self::Fixed {
                delay,
                max_attempts,
            } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        return None;
                    }
                }
                Some(*delay)
            }
            Self::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                max_attempts,
            } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        return None;
                    }
                }
                let delay = initial_delay.mul_f64(multiplier.powi(attempt as i32));
                Some(delay.min(*max_delay))
            }
            Self::Custom(_) => {
                // Custom strategies use the policy trait directly
                None
            }
        }
    }
}

/// Custom reconnection policy for fine-grained control.
#[async_trait]
pub trait ReconnectionPolicy: Send + Sync {
    /// Calculate the delay before the next reconnection attempt.
    ///
    /// Returns `None` to stop reconnecting.
    async fn next_delay(&self, attempt: u32, last_error: &str) -> Option<Duration>;

    /// Called when reconnection succeeds.
    async fn on_reconnected(&self);

    /// Reset the policy state.
    async fn reset(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnection_strategy_default() {
        let strategy = ReconnectionStrategy::default();
        match strategy {
            ReconnectionStrategy::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                max_attempts,
            } => {
                assert_eq!(initial_delay, Duration::from_secs(1));
                assert_eq!(max_delay, Duration::from_secs(60));
                assert_eq!(multiplier, 2.0);
                assert!(max_attempts.is_none());
            }
            _ => panic!("Expected ExponentialBackoff"),
        }
    }

    #[test]
    fn test_fixed_strategy() {
        let strategy = ReconnectionStrategy::fixed(Duration::from_secs(5));
        assert_eq!(strategy.calculate_delay(0), Some(Duration::from_secs(5)));
        assert_eq!(strategy.calculate_delay(100), Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_fixed_strategy_with_max() {
        let strategy = ReconnectionStrategy::fixed_with_max(Duration::from_secs(5), 3);
        assert_eq!(strategy.calculate_delay(0), Some(Duration::from_secs(5)));
        assert_eq!(strategy.calculate_delay(2), Some(Duration::from_secs(5)));
        assert_eq!(strategy.calculate_delay(3), None);
        assert_eq!(strategy.calculate_delay(4), None);
    }

    #[test]
    fn test_exponential_backoff() {
        let strategy = ReconnectionStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            max_attempts: None,
        };

        // Attempt 0: 100ms * 2^0 = 100ms
        assert_eq!(
            strategy.calculate_delay(0),
            Some(Duration::from_millis(100))
        );

        // Attempt 1: 100ms * 2^1 = 200ms
        assert_eq!(
            strategy.calculate_delay(1),
            Some(Duration::from_millis(200))
        );

        // Attempt 2: 100ms * 2^2 = 400ms
        assert_eq!(
            strategy.calculate_delay(2),
            Some(Duration::from_millis(400))
        );

        // Attempt 10: 100ms * 2^10 = 102400ms, but capped at 10s
        assert_eq!(strategy.calculate_delay(10), Some(Duration::from_secs(10)));
    }

    #[test]
    fn test_exponential_backoff_with_max_attempts() {
        let strategy = ReconnectionStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            max_attempts: Some(5),
        };

        assert!(strategy.calculate_delay(0).is_some());
        assert!(strategy.calculate_delay(4).is_some());
        assert!(strategy.calculate_delay(5).is_none());
    }

    #[test]
    fn test_none_strategy() {
        let strategy = ReconnectionStrategy::None;
        assert!(strategy.calculate_delay(0).is_none());
    }

    #[test]
    fn test_strategy_debug() {
        let strategy = ReconnectionStrategy::None;
        assert_eq!(format!("{:?}", strategy), "ReconnectionStrategy::None");

        let strategy = ReconnectionStrategy::fixed(Duration::from_secs(5));
        let debug = format!("{:?}", strategy);
        assert!(debug.contains("Fixed"));
        assert!(debug.contains("5s"));
    }
}
