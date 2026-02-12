//! Task executor trait for pluggable task execution
//!
//! This module defines the [`TaskExecutor`] trait which provides an abstraction
//! for how tasks are executed within an agent. There are two execution modes:
//!
//! - **Remote execution** (default): Tasks are scheduled via storage and executed
//!   by external workers. This is the standard durable execution model.
//!
//! - **Local execution** (Phase 4): Tasks can be executed in-process for specific
//!   task kinds. This enables performance optimizations for lightweight operations.
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::{TaskExecutor, RemoteTaskExecutor};
//!
//! // Default remote executor - tasks are scheduled to external workers
//! let executor = RemoteTaskExecutor::new();
//!
//! // Check if a task kind supports local execution
//! if executor.supports_local("send-email") {
//!     // Execute locally (only for custom executors)
//!     let result = executor.execute(task_id, "send-email", input).await?;
//! }
//! ```

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::error::FlovynError;

/// Result type for executor operations
pub type ExecutorResult<T> = Result<T, FlovynError>;

/// Trait for pluggable task execution within agents
///
/// Implementations determine how tasks are executed:
/// - Remote: Via storage scheduling (default, durable)
/// - Local: In-process for specific task kinds (future)
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute a task with the given parameters
    ///
    /// # Arguments
    ///
    /// * `task_id` - Unique identifier for the task
    /// * `kind` - The task kind/type being executed
    /// * `input` - JSON input for the task
    ///
    /// # Returns
    ///
    /// The task result as JSON, or an error if execution failed
    async fn execute(&self, task_id: Uuid, kind: &str, input: Value) -> ExecutorResult<Value>;

    /// Check if this executor supports local execution for the given task kind
    ///
    /// Returns `true` if the task can be executed in-process, `false` if it
    /// must be scheduled remotely.
    fn supports_local(&self, kind: &str) -> bool;
}

/// Default executor that delegates all tasks to remote workers via storage
///
/// This is the standard execution model where tasks are:
/// 1. Scheduled via the storage layer
/// 2. Picked up by external task workers
/// 3. Results returned through the storage layer
///
/// Direct execution via this executor always fails because tasks must go
/// through the storage scheduling path for durability.
pub struct RemoteTaskExecutor;

impl RemoteTaskExecutor {
    /// Create a new remote task executor
    pub fn new() -> Self {
        Self
    }
}

impl Default for RemoteTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TaskExecutor for RemoteTaskExecutor {
    async fn execute(&self, _task_id: Uuid, _kind: &str, _input: Value) -> ExecutorResult<Value> {
        Err(FlovynError::Other(
            "Remote tasks are executed via storage scheduling, not direct execution".to_string(),
        ))
    }

    fn supports_local(&self, _kind: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_executor_supports_local_returns_false() {
        let executor = RemoteTaskExecutor::new();
        assert!(!executor.supports_local("any-task"));
        assert!(!executor.supports_local(""));
        assert!(!executor.supports_local("send-email"));
    }

    #[test]
    fn test_remote_executor_default() {
        let executor = RemoteTaskExecutor::default();
        assert!(!executor.supports_local("test"));
    }

    #[tokio::test]
    async fn test_remote_executor_execute_returns_error() {
        let executor = RemoteTaskExecutor::new();
        let task_id = Uuid::new_v4();
        let result = executor
            .execute(task_id, "test-task", serde_json::json!({}))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("storage scheduling"));
    }
}
