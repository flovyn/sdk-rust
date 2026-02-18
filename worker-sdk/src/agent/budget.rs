//! Budget tracking and enforcement for agent executions.
//!
//! Tracks cumulative token usage and enforces budget limits. When the budget
//! is exceeded, the agent's cancellation flag is set.

use crate::agent::child::Budget;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tracks cumulative token usage against a budget.
pub struct BudgetTracker {
    /// Maximum total tokens allowed
    max_tokens: Option<u64>,
    /// Maximum cost in USD (stored as microdollars for atomic operations)
    max_cost_microdollars: Option<u64>,
    /// Accumulated input tokens
    input_tokens: AtomicU64,
    /// Accumulated output tokens
    output_tokens: AtomicU64,
}

impl BudgetTracker {
    /// Create a new budget tracker from budget limits.
    pub fn new(budget: &Budget) -> Self {
        Self {
            max_tokens: budget.max_tokens,
            max_cost_microdollars: budget.max_cost_usd.map(|usd| (usd * 1_000_000.0) as u64),
            input_tokens: AtomicU64::new(0),
            output_tokens: AtomicU64::new(0),
        }
    }

    /// Create a tracker with no limits (unlimited budget).
    pub fn unlimited() -> Self {
        Self {
            max_tokens: None,
            max_cost_microdollars: None,
            input_tokens: AtomicU64::new(0),
            output_tokens: AtomicU64::new(0),
        }
    }

    /// Record token usage from an LLM call.
    pub fn record_usage(&self, input_tokens: u64, output_tokens: u64) {
        self.input_tokens.fetch_add(input_tokens, Ordering::Relaxed);
        self.output_tokens
            .fetch_add(output_tokens, Ordering::Relaxed);
    }

    /// Get total tokens consumed so far.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.load(Ordering::Relaxed) + self.output_tokens.load(Ordering::Relaxed)
    }

    /// Check if the budget has been exceeded.
    pub fn is_exceeded(&self) -> bool {
        if let Some(max) = self.max_tokens {
            if self.total_tokens() >= max {
                return true;
            }
        }
        false
    }

    /// Get the remaining budget, or None if unlimited.
    pub fn remaining(&self) -> Option<Budget> {
        if self.max_tokens.is_none() && self.max_cost_microdollars.is_none() {
            return None;
        }

        let remaining_tokens = self
            .max_tokens
            .map(|max| max.saturating_sub(self.total_tokens()));

        let remaining_cost_usd = self.max_cost_microdollars.map(|max| {
            // Simple approximation: cost scales linearly with tokens
            // Real implementation would use per-model pricing
            let used_fraction = if let Some(max_tokens) = self.max_tokens {
                if max_tokens > 0 {
                    self.total_tokens() as f64 / max_tokens as f64
                } else {
                    1.0
                }
            } else {
                0.0
            };
            let max_usd = max as f64 / 1_000_000.0;
            max_usd * (1.0 - used_fraction).max(0.0)
        });

        Some(Budget {
            max_tokens: remaining_tokens,
            max_cost_usd: remaining_cost_usd,
            max_tokens_per_call: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_tracks_usage() {
        let budget = Budget {
            max_tokens: Some(1000),
            max_cost_usd: None,
            max_tokens_per_call: None,
        };
        let tracker = BudgetTracker::new(&budget);

        tracker.record_usage(100, 50);
        assert_eq!(tracker.total_tokens(), 150);
        assert!(!tracker.is_exceeded());

        tracker.record_usage(400, 200);
        assert_eq!(tracker.total_tokens(), 750);
        assert!(!tracker.is_exceeded());
    }

    #[test]
    fn test_budget_enforcement_triggers_cancellation() {
        let budget = Budget {
            max_tokens: Some(500),
            max_cost_usd: None,
            max_tokens_per_call: None,
        };
        let tracker = BudgetTracker::new(&budget);

        tracker.record_usage(200, 100);
        assert!(!tracker.is_exceeded());

        tracker.record_usage(100, 150);
        assert!(tracker.is_exceeded()); // 550 > 500
    }

    #[test]
    fn test_unlimited_budget() {
        let tracker = BudgetTracker::unlimited();
        tracker.record_usage(1_000_000, 1_000_000);
        assert!(!tracker.is_exceeded());
        assert!(tracker.remaining().is_none());
    }

    #[test]
    fn test_remaining_budget() {
        let budget = Budget {
            max_tokens: Some(1000),
            max_cost_usd: None,
            max_tokens_per_call: None,
        };
        let tracker = BudgetTracker::new(&budget);

        tracker.record_usage(300, 200);
        let remaining = tracker.remaining().unwrap();
        assert_eq!(remaining.max_tokens, Some(500));
    }
}
