//! Lazy task future types for agent execution
//!
//! This module provides future types that enable workflow-like parallel patterns
//! for agent task scheduling. Unlike immediate RPC-based scheduling, these futures
//! are lazy and only execute when collected and awaited via `ctx.join_all()`.
//!
//! ## Design
//!
//! The lazy future pattern enables:
//! - No RPC on creation (task is just recorded)
//! - Deterministic task IDs (assigned at creation time)
//! - Natural parallel patterns with `join_all`
//! - Efficient batching of task submissions
//!
//! ## Example
//!
//! ```rust,ignore
//! // Schedule multiple tasks lazily
//! let f1 = ctx.schedule_task("task-a", input_a);
//! let f2 = ctx.schedule_task("task-b", input_b);
//!
//! // Tasks are batched and submitted together when awaited
//! let results = ctx.join_all(vec![f1, f2]).await?;
//! ```

use serde_json::Value;
use uuid::Uuid;

/// Raw future representing a scheduled agent task.
///
/// Unlike `schedule_task_handle().await`, this:
/// - Does not make an RPC when created
/// - Assigns a deterministic task ID
/// - Only executes when collected and awaited via `ctx.join_all()`
///
/// # Example
///
/// ```rust,ignore
/// let f1 = ctx.schedule_task("task-a", input_a);
/// let f2 = ctx.schedule_task("task-b", input_b);
/// let results = ctx.join_all(vec![f1, f2]).await?;
/// ```
#[derive(Debug, Clone)]
pub struct AgentTaskFutureRaw {
    /// The deterministic task ID
    pub task_id: Uuid,
    /// The task kind/type
    pub kind: String,
    /// The task input
    pub input: Value,
}

impl AgentTaskFutureRaw {
    /// Create a new lazy task future.
    ///
    /// # Arguments
    ///
    /// * `task_id` - Deterministic task ID (typically generated via `ctx.deterministic_uuid()`)
    /// * `kind` - Task kind that matches a registered task definition
    /// * `input` - Task input as JSON
    pub fn new(task_id: Uuid, kind: String, input: Value) -> Self {
        Self {
            task_id,
            kind,
            input,
        }
    }

    /// Get the task ID.
    pub fn task_id(&self) -> Uuid {
        self.task_id
    }

    /// Get the task kind.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Get the task input.
    pub fn input(&self) -> &Value {
        &self.input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_agent_task_future_raw_new() {
        let task_id = Uuid::new_v4();
        let kind = "analyze".to_string();
        let input = json!({"data": [1, 2, 3]});

        let future = AgentTaskFutureRaw::new(task_id, kind.clone(), input.clone());

        assert_eq!(future.task_id(), task_id);
        assert_eq!(future.kind(), "analyze");
        assert_eq!(future.input(), &input);
    }

    #[test]
    fn test_agent_task_future_raw_clone() {
        let task_id = Uuid::new_v4();
        let future =
            AgentTaskFutureRaw::new(task_id, "process".to_string(), json!({"key": "value"}));

        let cloned = future.clone();

        assert_eq!(cloned.task_id(), future.task_id());
        assert_eq!(cloned.kind(), future.kind());
        assert_eq!(cloned.input(), future.input());
    }

    #[test]
    fn test_agent_task_future_raw_debug() {
        let task_id = Uuid::nil();
        let future = AgentTaskFutureRaw::new(task_id, "test".to_string(), json!(null));

        let debug_str = format!("{:?}", future);

        assert!(debug_str.contains("AgentTaskFutureRaw"));
        assert!(debug_str.contains("task_id"));
        assert!(debug_str.contains("kind"));
    }
}
