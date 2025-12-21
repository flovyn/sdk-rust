//! TaskDefinition trait

use crate::common::version::SemanticVersion;
use crate::error::Result;
use crate::task::context::TaskContext;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;

/// Retry configuration for task execution
#[derive(Debug, Clone, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial backoff duration
    pub initial_backoff: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Maximum backoff duration
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// Definition of a task with typed input and output
#[async_trait]
pub trait TaskDefinition: Send + Sync {
    /// Input type for the task
    type Input: Serialize + DeserializeOwned + Send;
    /// Output type for the task
    type Output: Serialize + DeserializeOwned + Send;

    /// Unique identifier for this task type
    fn kind(&self) -> &str;

    /// Execute the task with the given input and context
    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output>;

    /// Human-readable name for the task (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this task definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the task
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for task execution (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this task can be cancelled
    fn cancellable(&self) -> bool {
        false
    }

    /// Tags for categorizing the task
    fn tags(&self) -> Vec<String> {
        vec![]
    }

    /// Retry configuration for this task
    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    /// Heartbeat timeout in seconds (None = use default)
    fn heartbeat_timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this task uses streaming for real-time event delivery.
    ///
    /// When true, the task can call streaming methods like `ctx.stream_token()`,
    /// `ctx.stream_progress()`, etc. to send ephemeral events to connected clients.
    ///
    /// Default: false
    fn uses_streaming(&self) -> bool {
        false
    }
}

/// Type alias for dynamic task input/output
pub type DynamicTaskInput = Map<String, Value>;
pub type DynamicTaskOutput = Map<String, Value>;

/// Helper trait for implementing dynamic tasks
#[async_trait]
pub trait DynamicTask: Send + Sync {
    /// Unique identifier for this task type
    fn kind(&self) -> &str;

    /// Execute the task with dynamic input/output
    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput>;

    /// Human-readable name for the task (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this task definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the task
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for task execution (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this task can be cancelled
    fn cancellable(&self) -> bool {
        false
    }

    /// Tags for categorizing the task
    fn tags(&self) -> Vec<String> {
        vec![]
    }

    /// Retry configuration for this task
    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    /// Heartbeat timeout in seconds (None = use default)
    fn heartbeat_timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this task uses streaming for real-time event delivery.
    ///
    /// Default: false
    fn uses_streaming(&self) -> bool {
        false
    }
}

// Implement TaskDefinition for any DynamicTask
#[async_trait]
impl<T: DynamicTask> TaskDefinition for T {
    type Input = DynamicTaskInput;
    type Output = DynamicTaskOutput;

    fn kind(&self) -> &str {
        DynamicTask::kind(self)
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        DynamicTask::execute(self, input, ctx).await
    }

    fn name(&self) -> &str {
        DynamicTask::name(self)
    }

    fn version(&self) -> SemanticVersion {
        DynamicTask::version(self)
    }

    fn description(&self) -> Option<&str> {
        DynamicTask::description(self)
    }

    fn timeout_seconds(&self) -> Option<u32> {
        DynamicTask::timeout_seconds(self)
    }

    fn cancellable(&self) -> bool {
        DynamicTask::cancellable(self)
    }

    fn tags(&self) -> Vec<String> {
        DynamicTask::tags(self)
    }

    fn retry_config(&self) -> RetryConfig {
        DynamicTask::retry_config(self)
    }

    fn heartbeat_timeout_seconds(&self) -> Option<u32> {
        DynamicTask::heartbeat_timeout_seconds(self)
    }

    fn uses_streaming(&self) -> bool {
        DynamicTask::uses_streaming(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_secs(1));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.max_backoff, Duration::from_secs(60));
    }

    #[test]
    fn test_retry_config_custom() {
        let config = RetryConfig {
            max_retries: 5,
            initial_backoff: Duration::from_millis(500),
            backoff_multiplier: 1.5,
            max_backoff: Duration::from_secs(30),
        };
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_backoff, Duration::from_millis(500));
        assert_eq!(config.backoff_multiplier, 1.5);
        assert_eq!(config.max_backoff, Duration::from_secs(30));
    }

    #[test]
    fn test_retry_config_equality() {
        let config1 = RetryConfig::default();
        let config2 = RetryConfig::default();
        assert_eq!(config1, config2);
    }

    // Test struct for DynamicTask
    struct TestDynamicTask;

    #[async_trait]
    impl DynamicTask for TestDynamicTask {
        fn kind(&self) -> &str {
            "test-task"
        }

        async fn execute(
            &self,
            mut input: DynamicTaskInput,
            _ctx: &dyn TaskContext,
        ) -> Result<DynamicTaskOutput> {
            input.insert("processed".to_string(), Value::Bool(true));
            Ok(input)
        }

        fn version(&self) -> SemanticVersion {
            SemanticVersion::new(2, 0, 0)
        }

        fn description(&self) -> Option<&str> {
            Some("A test task")
        }

        fn timeout_seconds(&self) -> Option<u32> {
            Some(30)
        }

        fn cancellable(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_dynamic_task_kind() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::kind(&task), "test-task");
    }

    #[test]
    fn test_dynamic_task_name_defaults_to_kind() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::name(&task), DynamicTask::kind(&task));
    }

    #[test]
    fn test_dynamic_task_version() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::version(&task), SemanticVersion::new(2, 0, 0));
    }

    #[test]
    fn test_dynamic_task_description() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::description(&task), Some("A test task"));
    }

    #[test]
    fn test_dynamic_task_timeout_seconds() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::timeout_seconds(&task), Some(30));
    }

    #[test]
    fn test_dynamic_task_cancellable() {
        let task = TestDynamicTask;
        assert!(DynamicTask::cancellable(&task));
    }

    #[test]
    fn test_dynamic_task_tags_default_empty() {
        let task = TestDynamicTask;
        assert!(DynamicTask::tags(&task).is_empty());
    }

    #[test]
    fn test_dynamic_task_retry_config_default() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::retry_config(&task), RetryConfig::default());
    }

    #[test]
    fn test_dynamic_task_heartbeat_timeout_default() {
        let task = TestDynamicTask;
        assert_eq!(DynamicTask::heartbeat_timeout_seconds(&task), None);
    }

    // Test that TaskDefinition is implemented for DynamicTask
    #[test]
    fn test_task_definition_impl() {
        let task = TestDynamicTask;
        let def: &dyn TaskDefinition<Input = DynamicTaskInput, Output = DynamicTaskOutput> = &task;
        assert_eq!(def.kind(), "test-task");
        assert_eq!(def.version(), SemanticVersion::new(2, 0, 0));
    }
}
