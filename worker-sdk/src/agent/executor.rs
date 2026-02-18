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

#[cfg(feature = "local")]
use std::collections::HashMap;

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

/// Trait for task functions that can be executed locally.
///
/// Implementations provide the actual task logic. Register implementations
/// with a [`LocalTaskExecutor`] to enable in-process task execution.
#[cfg(feature = "local")]
#[async_trait]
pub trait TaskFn: Send + Sync {
    /// Execute the task with the given input.
    async fn execute(&self, input: Value) -> ExecutorResult<Value>;
}

/// Executor that runs tasks in-process using a registry of task functions.
///
/// For local agent mode, tasks are executed directly without remote scheduling.
/// Register task functions by kind, then the agent's `join_all` / `select_ok`
/// calls will execute matching tasks locally.
///
/// # Example
///
/// ```rust,ignore
/// use flovyn_worker_sdk::agent::executor::LocalTaskExecutor;
///
/// let mut executor = LocalTaskExecutor::new();
/// executor.register("echo", |input| async move { Ok(input) });
/// ```
#[cfg(feature = "local")]
pub struct LocalTaskExecutor {
    registry: HashMap<String, Box<dyn TaskFn>>,
}

#[cfg(feature = "local")]
impl LocalTaskExecutor {
    /// Create a new empty local task executor.
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
        }
    }

    /// Register a task function for a given kind.
    ///
    /// If a task with the same kind is already registered, it will be replaced.
    pub fn register(&mut self, kind: &str, task: impl TaskFn + 'static) -> &mut Self {
        self.registry.insert(kind.to_string(), Box::new(task));
        self
    }
}

#[cfg(feature = "local")]
impl Default for LocalTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "local")]
#[async_trait]
impl TaskExecutor for LocalTaskExecutor {
    async fn execute(&self, _task_id: Uuid, kind: &str, input: Value) -> ExecutorResult<Value> {
        let task_fn = self.registry.get(kind).ok_or_else(|| {
            FlovynError::Other(format!("No local task registered for kind: {kind}"))
        })?;

        task_fn.execute(input).await
    }

    fn supports_local(&self, kind: &str) -> bool {
        self.registry.contains_key(kind)
    }
}

/// Implement `TaskFn` for async closures via boxed futures.
///
/// This allows registering closures as task functions:
/// ```rust,ignore
/// executor.register("echo", FnTask(|input| async move { Ok(input) }));
/// ```
#[cfg(feature = "local")]
pub struct FnTask<F>(pub F);

#[cfg(feature = "local")]
#[async_trait]
impl<F, Fut> TaskFn for FnTask<F>
where
    F: Fn(Value) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ExecutorResult<Value>> + Send,
{
    async fn execute(&self, input: Value) -> ExecutorResult<Value> {
        (self.0)(input).await
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
        let executor = RemoteTaskExecutor;
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

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_local_executor_register_and_execute() {
        let mut executor = LocalTaskExecutor::new();
        executor.register("echo", FnTask(|input| async move { Ok(input) }));

        let task_id = Uuid::new_v4();
        let input = serde_json::json!({"message": "hello"});
        let result = executor
            .execute(task_id, "echo", input.clone())
            .await
            .unwrap();
        assert_eq!(result, input);
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_local_executor_unknown_task() {
        let executor = LocalTaskExecutor::new();
        let task_id = Uuid::new_v4();
        let result = executor
            .execute(task_id, "nonexistent", serde_json::json!({}))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[cfg(feature = "local")]
    #[test]
    fn test_local_executor_supports_local() {
        let mut executor = LocalTaskExecutor::new();
        assert!(!executor.supports_local("echo"));

        executor.register("echo", FnTask(|input| async move { Ok(input) }));
        assert!(executor.supports_local("echo"));
        assert!(!executor.supports_local("other"));
    }
}
