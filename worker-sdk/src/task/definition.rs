//! TaskDefinition trait

use crate::common::version::SemanticVersion;
use crate::error::Result;
use crate::task::context::TaskContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;

/// Generate JSON Schema from a type that implements JsonSchema.
/// This is the Rust equivalent of Kotlin's `JsonSchemaGenerator.generateSchema<T>()`.
pub fn generate_schema<T: JsonSchema>() -> Value {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

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

/// Definition of a task with typed input and output.
///
/// Input and output types must implement `JsonSchema` to enable automatic
/// schema generation (like Kotlin's JsonSchemaGenerator).
#[async_trait]
pub trait TaskDefinition: Send + Sync {
    /// Input type for the task (must derive JsonSchema for auto-generation)
    type Input: Serialize + DeserializeOwned + JsonSchema + Send;
    /// Output type for the task (must derive JsonSchema for auto-generation)
    type Output: Serialize + DeserializeOwned + JsonSchema + Send;

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

    /// JSON Schema for task input validation.
    /// Default: auto-generated from Input type using schemars.
    fn input_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Input>())
    }

    /// JSON Schema for task output validation.
    /// Default: auto-generated from Output type using schemars.
    fn output_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Output>())
    }
}

/// Type alias for dynamic task input/output
pub type DynamicTaskInput = Map<String, Value>;
pub type DynamicTaskOutput = Map<String, Value>;

/// Helper trait for implementing dynamic (untyped) tasks.
///
/// Since DynamicTask uses `Map<String, Value>` for input/output,
/// you must manually provide schemas via `input_schema()` and `output_schema()`.
/// For typed tasks, use `TaskDefinition` directly to get auto-generated schemas.
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

    /// JSON Schema for task input validation.
    /// Must be provided manually for DynamicTask since input is untyped.
    fn input_schema(&self) -> Option<Value> {
        None
    }

    /// JSON Schema for task output validation.
    /// Must be provided manually for DynamicTask since output is untyped.
    fn output_schema(&self) -> Option<Value> {
        None
    }
}

// Implement TaskDefinition for any DynamicTask
// Schemas are delegated to DynamicTask's manual methods (not auto-generated)
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

    // Override to use manual schemas from DynamicTask instead of auto-generating
    fn input_schema(&self) -> Option<Value> {
        DynamicTask::input_schema(self)
    }

    fn output_schema(&self) -> Option<Value> {
        DynamicTask::output_schema(self)
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
