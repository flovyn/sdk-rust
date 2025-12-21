//! Task execution utilities - language-agnostic execution logic
//!
//! This module provides core types and utilities for task execution that can be
//! shared across different language SDKs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Task metadata extracted from a task definition.
///
/// This represents the static metadata about a task type,
/// independent of any specific execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMetadata {
    /// Unique task kind identifier (e.g., "process-image", "send-email")
    pub kind: String,
    /// Human-readable name for display purposes
    pub name: String,
    /// Optional description of what the task does
    pub description: Option<String>,
    /// Semantic version of the task implementation
    pub version: Option<String>,
    /// Tags for categorization and filtering
    pub tags: Vec<String>,
    /// Timeout in seconds (None means use executor default)
    pub timeout_seconds: Option<u32>,
    /// Whether the task supports graceful cancellation
    pub cancellable: bool,
    /// Heartbeat timeout in seconds (None means use executor default)
    pub heartbeat_timeout_seconds: Option<u32>,
}

impl TaskMetadata {
    /// Create a new TaskMetadata with required fields only
    pub fn new(kind: impl Into<String>) -> Self {
        let kind = kind.into();
        Self {
            kind: kind.clone(),
            name: kind,
            description: None,
            version: None,
            tags: Vec::new(),
            timeout_seconds: None,
            cancellable: true,
            heartbeat_timeout_seconds: None,
        }
    }

    /// Set the human-readable name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set the timeout in seconds
    pub fn with_timeout_seconds(mut self, timeout: u32) -> Self {
        self.timeout_seconds = Some(timeout);
        self
    }

    /// Set whether the task is cancellable
    pub fn with_cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }

    /// Set the heartbeat timeout in seconds
    pub fn with_heartbeat_timeout_seconds(mut self, timeout: u32) -> Self {
        self.heartbeat_timeout_seconds = Some(timeout);
        self
    }
}

/// Result of task execution.
///
/// This enum represents all possible outcomes of a task execution,
/// independent of the language runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskExecutionResult {
    /// Task completed successfully with an output value
    Completed { output: Value },
    /// Task failed with an error
    Failed {
        /// Human-readable error message
        error_message: String,
        /// Machine-readable error type for categorization
        error_type: Option<String>,
        /// Whether the task can be retried
        is_retryable: bool,
    },
    /// Task was cancelled (either by user or system)
    Cancelled,
    /// Task exceeded its timeout
    TimedOut,
}

impl TaskExecutionResult {
    /// Create a successful completion result
    pub fn completed(output: Value) -> Self {
        Self::Completed { output }
    }

    /// Create a failed result
    pub fn failed(error_message: impl Into<String>, is_retryable: bool) -> Self {
        Self::Failed {
            error_message: error_message.into(),
            error_type: None,
            is_retryable,
        }
    }

    /// Create a failed result with an error type
    pub fn failed_with_type(
        error_message: impl Into<String>,
        error_type: impl Into<String>,
        is_retryable: bool,
    ) -> Self {
        Self::Failed {
            error_message: error_message.into(),
            error_type: Some(error_type.into()),
            is_retryable,
        }
    }

    /// Check if this is a successful completion
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    /// Check if this is a failure
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// Check if the task can be retried
    pub fn can_retry(&self) -> bool {
        match self {
            Self::Failed { is_retryable, .. } => *is_retryable,
            Self::TimedOut => true, // Timeouts are typically retryable
            _ => false,
        }
    }
}

/// Configuration for task execution.
///
/// These are default values used when a task doesn't specify its own configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutorConfig {
    /// Default timeout for task execution in milliseconds
    pub default_timeout_ms: u64,
    /// Default heartbeat interval in milliseconds
    pub default_heartbeat_interval_ms: u64,
}

impl Default for TaskExecutorConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 300_000,           // 5 minutes
            default_heartbeat_interval_ms: 30_000, // 30 seconds
        }
    }
}

impl TaskExecutorConfig {
    /// Get the default timeout as a Duration
    pub fn default_timeout(&self) -> Duration {
        Duration::from_millis(self.default_timeout_ms)
    }

    /// Get the default heartbeat interval as a Duration
    pub fn default_heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.default_heartbeat_interval_ms)
    }
}

/// Configuration for exponential backoff retry logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackoffConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Maximum backoff duration in milliseconds
    pub max_backoff_ms: u64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000, // 1 second
            backoff_multiplier: 2.0,
            max_backoff_ms: 60_000, // 60 seconds
        }
    }
}

impl BackoffConfig {
    /// Get initial backoff as Duration
    pub fn initial_backoff(&self) -> Duration {
        Duration::from_millis(self.initial_backoff_ms)
    }

    /// Get max backoff as Duration
    pub fn max_backoff(&self) -> Duration {
        Duration::from_millis(self.max_backoff_ms)
    }
}

/// Calculate backoff duration for a retry attempt.
///
/// Uses exponential backoff with jitter, capped at the maximum backoff.
///
/// # Arguments
/// * `config` - The backoff configuration
/// * `attempt` - The current attempt number (1-based)
///
/// # Returns
/// The duration to wait before the next retry attempt
pub fn calculate_backoff(config: &BackoffConfig, attempt: u32) -> Duration {
    let base_ms = config.initial_backoff_ms as f64;
    let multiplier = config
        .backoff_multiplier
        .powi(attempt.saturating_sub(1) as i32);
    let backoff_ms = base_ms * multiplier;
    let backoff = Duration::from_millis(backoff_ms as u64);
    std::cmp::min(backoff, config.max_backoff())
}

/// Check if a task should be retried based on configuration and attempt count.
///
/// # Arguments
/// * `config` - The backoff configuration
/// * `attempt` - The current attempt number (1-based)
/// * `is_retryable` - Whether the error is retryable
///
/// # Returns
/// `true` if the task should be retried, `false` otherwise
pub fn should_retry(config: &BackoffConfig, attempt: u32, is_retryable: bool) -> bool {
    is_retryable && attempt < config.max_retries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ==================== TaskMetadata Tests ====================

    #[test]
    fn test_task_metadata_new() {
        let meta = TaskMetadata::new("my-task");
        assert_eq!(meta.kind, "my-task");
        assert_eq!(meta.name, "my-task"); // Defaults to kind
        assert!(meta.description.is_none());
        assert!(meta.version.is_none());
        assert!(meta.tags.is_empty());
        assert!(meta.timeout_seconds.is_none());
        assert!(meta.cancellable);
        assert!(meta.heartbeat_timeout_seconds.is_none());
    }

    #[test]
    fn test_task_metadata_builder() {
        let meta = TaskMetadata::new("process-image")
            .with_name("Process Image")
            .with_description("Processes images with various transformations")
            .with_version("1.0.0")
            .with_tags(vec!["image".to_string(), "processing".to_string()])
            .with_timeout_seconds(300)
            .with_cancellable(false)
            .with_heartbeat_timeout_seconds(60);

        assert_eq!(meta.kind, "process-image");
        assert_eq!(meta.name, "Process Image");
        assert_eq!(
            meta.description,
            Some("Processes images with various transformations".to_string())
        );
        assert_eq!(meta.version, Some("1.0.0".to_string()));
        assert_eq!(meta.tags, vec!["image", "processing"]);
        assert_eq!(meta.timeout_seconds, Some(300));
        assert!(!meta.cancellable);
        assert_eq!(meta.heartbeat_timeout_seconds, Some(60));
    }

    #[test]
    fn test_task_metadata_serde() {
        let meta = TaskMetadata::new("test-task")
            .with_name("Test Task")
            .with_version("1.0.0");

        let json = serde_json::to_string(&meta).unwrap();
        let parsed: TaskMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.kind, meta.kind);
        assert_eq!(parsed.name, meta.name);
        assert_eq!(parsed.version, meta.version);
    }

    // ==================== TaskExecutionResult Tests ====================

    #[test]
    fn test_task_execution_result_completed() {
        let result = TaskExecutionResult::completed(json!({"status": "ok"}));
        assert!(result.is_completed());
        assert!(!result.is_failed());
        assert!(!result.can_retry());
    }

    #[test]
    fn test_task_execution_result_failed() {
        let result = TaskExecutionResult::failed("Something went wrong", true);
        assert!(!result.is_completed());
        assert!(result.is_failed());
        assert!(result.can_retry());
    }

    #[test]
    fn test_task_execution_result_failed_not_retryable() {
        let result = TaskExecutionResult::failed("Invalid input", false);
        assert!(!result.can_retry());
    }

    #[test]
    fn test_task_execution_result_failed_with_type() {
        let result = TaskExecutionResult::failed_with_type("Network error", "NETWORK_ERROR", true);

        match result {
            TaskExecutionResult::Failed {
                error_message,
                error_type,
                is_retryable,
            } => {
                assert_eq!(error_message, "Network error");
                assert_eq!(error_type, Some("NETWORK_ERROR".to_string()));
                assert!(is_retryable);
            }
            _ => panic!("Expected Failed variant"),
        }
    }

    #[test]
    fn test_task_execution_result_cancelled() {
        let result = TaskExecutionResult::Cancelled;
        assert!(!result.is_completed());
        assert!(!result.is_failed());
        assert!(!result.can_retry());
    }

    #[test]
    fn test_task_execution_result_timed_out() {
        let result = TaskExecutionResult::TimedOut;
        assert!(!result.is_completed());
        assert!(!result.is_failed());
        assert!(result.can_retry()); // Timeouts are retryable
    }

    #[test]
    fn test_task_execution_result_serde() {
        let result = TaskExecutionResult::completed(json!({"count": 42}));
        let json = serde_json::to_string(&result).unwrap();
        let parsed: TaskExecutionResult = serde_json::from_str(&json).unwrap();

        match parsed {
            TaskExecutionResult::Completed { output } => {
                assert_eq!(output, json!({"count": 42}));
            }
            _ => panic!("Expected Completed variant"),
        }
    }

    // ==================== TaskExecutorConfig Tests ====================

    #[test]
    fn test_task_executor_config_default() {
        let config = TaskExecutorConfig::default();
        assert_eq!(config.default_timeout_ms, 300_000);
        assert_eq!(config.default_heartbeat_interval_ms, 30_000);
        assert_eq!(config.default_timeout(), Duration::from_secs(300));
        assert_eq!(config.default_heartbeat_interval(), Duration::from_secs(30));
    }

    // ==================== BackoffConfig Tests ====================

    #[test]
    fn test_backoff_config_default() {
        let config = BackoffConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff_ms, 1000);
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.max_backoff_ms, 60_000);
    }

    #[test]
    fn test_calculate_backoff_first_attempt() {
        let config = BackoffConfig::default();
        let backoff = calculate_backoff(&config, 1);
        assert_eq!(backoff, Duration::from_secs(1));
    }

    #[test]
    fn test_calculate_backoff_second_attempt() {
        let config = BackoffConfig::default();
        let backoff = calculate_backoff(&config, 2);
        assert_eq!(backoff, Duration::from_secs(2));
    }

    #[test]
    fn test_calculate_backoff_third_attempt() {
        let config = BackoffConfig::default();
        let backoff = calculate_backoff(&config, 3);
        assert_eq!(backoff, Duration::from_secs(4));
    }

    #[test]
    fn test_calculate_backoff_capped_at_max() {
        let config = BackoffConfig {
            max_retries: 10,
            initial_backoff_ms: 10_000, // 10 seconds
            backoff_multiplier: 2.0,
            max_backoff_ms: 60_000, // 60 seconds max
        };
        let backoff = calculate_backoff(&config, 10);
        assert_eq!(backoff, Duration::from_secs(60)); // Capped at 60 seconds
    }

    #[test]
    fn test_should_retry_true() {
        let config = BackoffConfig::default();
        assert!(should_retry(&config, 1, true));
        assert!(should_retry(&config, 2, true));
    }

    #[test]
    fn test_should_retry_false_max_reached() {
        let config = BackoffConfig::default(); // max_retries is 3
        assert!(!should_retry(&config, 3, true));
        assert!(!should_retry(&config, 4, true));
    }

    #[test]
    fn test_should_retry_false_not_retryable() {
        let config = BackoffConfig::default();
        assert!(!should_retry(&config, 1, false));
    }
}
