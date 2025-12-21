//! Reconnection strategy types.

use serde::{Deserialize, Serialize};

/// Strategy for handling reconnection after connection loss.
///
/// Note: The `Custom` variant from the SDK is not included here as it
/// requires an async trait which is language-specific.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReconnectionStrategy {
    /// No automatic reconnection.
    None,

    /// Fixed delay between attempts.
    Fixed {
        /// Delay between attempts in milliseconds.
        delay_ms: u64,
        /// Maximum number of attempts (None = infinite).
        max_attempts: Option<u32>,
    },

    /// Exponential backoff between attempts.
    ExponentialBackoff {
        /// Initial delay before first retry in milliseconds.
        initial_delay_ms: u64,
        /// Maximum delay between attempts in milliseconds.
        max_delay_ms: u64,
        /// Multiplier for each subsequent attempt.
        multiplier: f64,
        /// Maximum number of attempts (None = infinite).
        max_attempts: Option<u32>,
    },
}

impl Default for ReconnectionStrategy {
    fn default() -> Self {
        Self::ExponentialBackoff {
            initial_delay_ms: 1000,
            max_delay_ms: 60_000,
            multiplier: 2.0,
            max_attempts: None,
        }
    }
}

impl ReconnectionStrategy {
    /// Creates a no reconnection strategy.
    pub fn none() -> Self {
        Self::None
    }

    /// Creates a fixed delay strategy.
    pub fn fixed(delay_ms: u64) -> Self {
        Self::Fixed {
            delay_ms,
            max_attempts: None,
        }
    }

    /// Creates a fixed delay strategy with max attempts.
    pub fn fixed_with_max(delay_ms: u64, max_attempts: u32) -> Self {
        Self::Fixed {
            delay_ms,
            max_attempts: Some(max_attempts),
        }
    }

    /// Creates an exponential backoff strategy with default settings.
    pub fn exponential_backoff() -> Self {
        Self::default()
    }

    /// Creates an exponential backoff strategy with custom settings.
    pub fn exponential_backoff_custom(
        initial_delay_ms: u64,
        max_delay_ms: u64,
        multiplier: f64,
        max_attempts: Option<u32>,
    ) -> Self {
        Self::ExponentialBackoff {
            initial_delay_ms,
            max_delay_ms,
            multiplier,
            max_attempts,
        }
    }

    /// Calculates the delay for the given attempt number.
    ///
    /// Returns `None` if max attempts have been reached or strategy is None.
    pub fn calculate_delay_ms(&self, attempt: u32) -> Option<u64> {
        match self {
            Self::None => None,
            Self::Fixed {
                delay_ms,
                max_attempts,
            } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        return None;
                    }
                }
                Some(*delay_ms)
            }
            Self::ExponentialBackoff {
                initial_delay_ms,
                max_delay_ms,
                multiplier,
                max_attempts,
            } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        return None;
                    }
                }
                let delay = (*initial_delay_ms as f64) * multiplier.powi(attempt as i32);
                Some((delay as u64).min(*max_delay_ms))
            }
        }
    }

    /// Check if this strategy allows reconnection.
    pub fn allows_reconnection(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Check if max attempts have been reached.
    pub fn is_exhausted(&self, attempt: u32) -> bool {
        match self {
            Self::None => true,
            Self::Fixed { max_attempts, .. } | Self::ExponentialBackoff { max_attempts, .. } => {
                max_attempts.map(|max| attempt >= max).unwrap_or(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnection_strategy_default() {
        let strategy = ReconnectionStrategy::default();
        match strategy {
            ReconnectionStrategy::ExponentialBackoff {
                initial_delay_ms,
                max_delay_ms,
                multiplier,
                max_attempts,
            } => {
                assert_eq!(initial_delay_ms, 1000);
                assert_eq!(max_delay_ms, 60_000);
                assert_eq!(multiplier, 2.0);
                assert!(max_attempts.is_none());
            }
            _ => panic!("Expected ExponentialBackoff"),
        }
    }

    #[test]
    fn test_fixed_strategy() {
        let strategy = ReconnectionStrategy::fixed(5000);
        assert_eq!(strategy.calculate_delay_ms(0), Some(5000));
        assert_eq!(strategy.calculate_delay_ms(100), Some(5000));
    }

    #[test]
    fn test_fixed_strategy_with_max() {
        let strategy = ReconnectionStrategy::fixed_with_max(5000, 3);
        assert_eq!(strategy.calculate_delay_ms(0), Some(5000));
        assert_eq!(strategy.calculate_delay_ms(2), Some(5000));
        assert_eq!(strategy.calculate_delay_ms(3), None);
        assert_eq!(strategy.calculate_delay_ms(4), None);
    }

    #[test]
    fn test_exponential_backoff() {
        let strategy = ReconnectionStrategy::ExponentialBackoff {
            initial_delay_ms: 100,
            max_delay_ms: 10_000,
            multiplier: 2.0,
            max_attempts: None,
        };

        // Attempt 0: 100 * 2^0 = 100
        assert_eq!(strategy.calculate_delay_ms(0), Some(100));

        // Attempt 1: 100 * 2^1 = 200
        assert_eq!(strategy.calculate_delay_ms(1), Some(200));

        // Attempt 2: 100 * 2^2 = 400
        assert_eq!(strategy.calculate_delay_ms(2), Some(400));

        // Attempt 10: 100 * 2^10 = 102400, but capped at 10000
        assert_eq!(strategy.calculate_delay_ms(10), Some(10_000));
    }

    #[test]
    fn test_exponential_backoff_with_max_attempts() {
        let strategy = ReconnectionStrategy::ExponentialBackoff {
            initial_delay_ms: 100,
            max_delay_ms: 10_000,
            multiplier: 2.0,
            max_attempts: Some(5),
        };

        assert!(strategy.calculate_delay_ms(0).is_some());
        assert!(strategy.calculate_delay_ms(4).is_some());
        assert!(strategy.calculate_delay_ms(5).is_none());
    }

    #[test]
    fn test_none_strategy() {
        let strategy = ReconnectionStrategy::None;
        assert!(strategy.calculate_delay_ms(0).is_none());
        assert!(!strategy.allows_reconnection());
    }

    #[test]
    fn test_allows_reconnection() {
        assert!(!ReconnectionStrategy::none().allows_reconnection());
        assert!(ReconnectionStrategy::fixed(1000).allows_reconnection());
        assert!(ReconnectionStrategy::exponential_backoff().allows_reconnection());
    }

    #[test]
    fn test_is_exhausted() {
        let strategy = ReconnectionStrategy::fixed_with_max(1000, 3);
        assert!(!strategy.is_exhausted(0));
        assert!(!strategy.is_exhausted(2));
        assert!(strategy.is_exhausted(3));
        assert!(strategy.is_exhausted(4));

        let infinite = ReconnectionStrategy::fixed(1000);
        assert!(!infinite.is_exhausted(100));
    }

    #[test]
    fn test_strategy_serde() {
        let strategy = ReconnectionStrategy::fixed_with_max(5000, 10);
        let json = serde_json::to_string(&strategy).unwrap();
        let parsed: ReconnectionStrategy = serde_json::from_str(&json).unwrap();

        match parsed {
            ReconnectionStrategy::Fixed {
                delay_ms,
                max_attempts,
            } => {
                assert_eq!(delay_ms, 5000);
                assert_eq!(max_attempts, Some(10));
            }
            _ => panic!("Expected Fixed"),
        }
    }
}
