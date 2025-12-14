//! WorkflowContext trait definition

use crate::error::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

/// Options for scheduling a task
#[derive(Debug, Clone, Default)]
pub struct ScheduleTaskOptions {
    /// Priority in seconds (lower = higher priority)
    pub priority_seconds: Option<i32>,
    /// Task timeout override
    pub timeout: Option<Duration>,
    /// Task queue override
    pub queue: Option<String>,
    /// Maximum retry attempts
    pub max_retries: Option<u32>,
}

/// A deterministic random number generator
pub trait DeterministicRandom: Send + Sync {
    /// Generate a random i32 in the range [min, max)
    fn next_int(&self, min: i32, max: i32) -> i32;

    /// Generate a random i64 in the range [min, max)
    fn next_long(&self, min: i64, max: i64) -> i64;

    /// Generate a random f64 in the range [0, 1)
    fn next_double(&self) -> f64;

    /// Generate a random bool
    fn next_bool(&self) -> bool;
}

/// Context for workflow execution providing deterministic APIs and side effect management.
///
/// This trait uses `Value` types for object-safety. For typed APIs, use the extension
/// methods provided by `WorkflowContextExt`.
#[async_trait]
pub trait WorkflowContext: Send + Sync {
    // === Identifiers ===

    /// Get the unique ID of this workflow execution
    fn workflow_execution_id(&self) -> Uuid;

    /// Get the tenant ID for this workflow
    fn tenant_id(&self) -> Uuid;

    /// Get the raw workflow input as JSON Value
    fn input_raw(&self) -> &Value;

    // === Deterministic APIs (recorded/replayed) ===

    /// Get the current time in milliseconds (deterministic - same on replay)
    fn current_time_millis(&self) -> i64;

    /// Generate a deterministic UUID (same on replay)
    fn random_uuid(&self) -> Uuid;

    /// Get a deterministic random number generator (same sequence on replay)
    fn random(&self) -> &dyn DeterministicRandom;

    // === Side Effects (cached via event sourcing) ===

    /// Execute a side effect and cache the result (raw Value version).
    /// On replay, returns the cached result without re-executing.
    async fn run_raw(&self, name: &str, result: Value) -> Result<Value>;

    // === Task Scheduling ===

    /// Schedule a task and wait for its completion (raw Value version)
    async fn schedule_raw(&self, task_type: &str, input: Value) -> Result<Value>;

    /// Schedule a task with custom options (raw Value version)
    async fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> Result<Value>;

    // === State Management ===

    /// Get a value from workflow state (raw Value version)
    async fn get_raw(&self, key: &str) -> Result<Option<Value>>;

    /// Set a value in workflow state (raw Value version)
    async fn set_raw(&self, key: &str, value: Value) -> Result<()>;

    /// Clear a specific key from workflow state
    async fn clear(&self, key: &str) -> Result<()>;

    /// Clear all workflow state
    async fn clear_all(&self) -> Result<()>;

    /// Get all keys in workflow state
    async fn state_keys(&self) -> Result<Vec<String>>;

    // === Timers ===

    /// Sleep for the specified duration (durable - survives restarts)
    async fn sleep(&self, duration: Duration) -> Result<()>;

    // === Promises (Signals) ===

    /// Create a durable promise that can be resolved externally (raw Value version)
    async fn promise_raw(&self, name: &str) -> Result<Value>;

    /// Create a durable promise with a timeout (raw Value version)
    async fn promise_with_timeout_raw(&self, name: &str, timeout: Duration) -> Result<Value>;

    // === Child Workflows ===

    /// Schedule a child workflow and wait for its completion (raw Value version)
    async fn schedule_workflow_raw(&self, name: &str, kind: &str, input: Value) -> Result<Value>;

    // === Cancellation ===

    /// Check if cancellation has been requested
    fn is_cancellation_requested(&self) -> bool;

    /// Check for cancellation and return error if cancelled
    async fn check_cancellation(&self) -> Result<()>;
}

/// Extension trait for typed workflow context operations.
/// These methods provide type-safe wrappers around the raw Value methods.
pub trait WorkflowContextExt: WorkflowContext {
    /// Get the workflow input as the specified type
    fn input<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.input_raw().clone())
            .map_err(crate::error::FlovynError::Serialization)
    }

    /// Get a value from workflow state
    fn get_typed<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<Option<T>>> + Send
    where
        Self: Sync,
    {
        async move {
            match self.get_raw(key).await? {
                Some(v) => serde_json::from_value(v)
                    .map(Some)
                    .map_err(crate::error::FlovynError::Serialization),
                None => Ok(None),
            }
        }
    }

    /// Set a value in workflow state
    fn set_typed<T: serde::Serialize + Send>(
        &self,
        key: &str,
        value: T,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sync,
    {
        async move {
            let v =
                serde_json::to_value(value).map_err(crate::error::FlovynError::Serialization)?;
            self.set_raw(key, v).await
        }
    }
}

// Implement WorkflowContextExt for all types that implement WorkflowContext
impl<T: WorkflowContext + ?Sized> WorkflowContextExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_task_options_default() {
        let options = ScheduleTaskOptions::default();
        assert!(options.priority_seconds.is_none());
        assert!(options.timeout.is_none());
    }

    #[test]
    fn test_schedule_task_options_with_values() {
        let options = ScheduleTaskOptions {
            priority_seconds: Some(60),
            timeout: Some(Duration::from_secs(300)),
            queue: Some("custom-queue".to_string()),
            max_retries: Some(5),
        };
        assert_eq!(options.priority_seconds, Some(60));
        assert_eq!(options.timeout, Some(Duration::from_secs(300)));
        assert_eq!(options.queue, Some("custom-queue".to_string()));
        assert_eq!(options.max_retries, Some(5));
    }
}
