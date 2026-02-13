//! Agent Task Types and Utilities
//!
//! Provides types for working with parallel task execution patterns.
//!
//! ## Overview
//!
//! Agents can schedule multiple tasks and wait for them using methods on AgentContext:
//! - `ctx.schedule_task_lazy()` - Schedule a task lazily (no immediate RPC)
//! - `ctx.join_all(futures)` - Wait for ALL tasks to complete
//! - `ctx.select_ok(futures)` - Wait for first successful task
//!
//! ## Example
//!
//! ```rust,ignore
//! // Schedule tasks lazily (no RPC call yet)
//! let f1 = ctx.schedule_task_lazy("process-item", json!({"item": "a"}));
//! let f2 = ctx.schedule_task_lazy("process-item", json!({"item": "b"}));
//! let f3 = ctx.schedule_task_lazy("process-item", json!({"item": "c"}));
//!
//! // Wait for all to complete (batched RPC, then await)
//! let results = ctx.join_all(vec![f1, f2, f3]).await?;
//!
//! // Or wait for first success (batched RPC, then race)
//! let (result, remaining) = ctx.select_ok(vec![f1, f2]).await?;
//! ```

use crate::error::{FlovynError, Result};
use serde_json::Value;

/// Outcome of a task execution.
///
/// Used when you need to inspect individual task results after
/// using patterns like `join_all` or `select_ok`.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    /// Task completed successfully
    Completed(Value),
    /// Task failed with an error
    Failed(String),
    /// Task was cancelled
    Cancelled,
    /// Task is still pending/running (shouldn't appear in final results)
    Pending,
}

impl TaskOutcome {
    /// Check if the task completed successfully
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed(_))
    }

    /// Check if the task failed
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Check if the task was cancelled
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Check if the task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// Convert to Result, returning error for non-completed outcomes
    pub fn into_result(self) -> Result<Value> {
        match self {
            Self::Completed(v) => Ok(v),
            Self::Failed(e) => Err(FlovynError::TaskFailed(e)),
            Self::Cancelled => Err(FlovynError::TaskFailed("Task was cancelled".to_string())),
            Self::Pending => Err(FlovynError::TaskFailed("Task is still pending".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_outcome() {
        let completed = TaskOutcome::Completed(serde_json::json!({"result": 42}));
        assert!(completed.is_completed());
        assert!(completed.is_terminal());

        let failed = TaskOutcome::Failed("error".to_string());
        assert!(failed.is_failed());
        assert!(failed.is_terminal());

        let cancelled = TaskOutcome::Cancelled;
        assert!(cancelled.is_cancelled());
        assert!(cancelled.is_terminal());

        let pending = TaskOutcome::Pending;
        assert!(!pending.is_terminal());
    }
}
